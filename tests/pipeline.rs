//! Test de integracion del pipeline completo, con mocks en las cuatro puntas.
//!
//! No abre ventanas, no toca la pantalla y no sale a la red, asi que corre
//! igual en Windows que en el CI de Linux.

use hakoyaku::capture::ScriptedCapturer;
use hakoyaku::config::{Backend, Config, Region};
use hakoyaku::frame::Frame;
use hakoyaku::ocr::{apilar, HashKeyedOcr, TextRecognizer};
use hakoyaku::overlay::{Presenter, Rgb};
use hakoyaku::pipeline::{Pipeline, Step};
use hakoyaku::translate::Translator;

use std::sync::{Arc, Mutex};

/// Traductor que cuenta cuantas veces le han llamado y pone un prefijo.
#[derive(Clone, Default)]
struct TraductorContador {
    llamadas: Arc<Mutex<Vec<String>>>,
    fallar: bool,
}

impl Translator for TraductorContador {
    fn translate(&self, text: &str) -> anyhow::Result<String> {
        self.llamadas.lock().unwrap().push(text.to_string());
        if self.fallar {
            anyhow::bail!("la API esta caida");
        }
        Ok(format!("ES:{text}"))
    }
    fn name(&self) -> &'static str {
        "contador"
    }
}

/// Una recolocacion del recuadro: donde se puso y con que color de fondo.
type Colocacion = (hakoyaku::config::Region, Option<Rgb>);

#[derive(Clone, Default)]
struct PresentadorEspia {
    eventos: Arc<Mutex<Vec<(String, String)>>>,
    colocaciones: Arc<Mutex<Vec<Colocacion>>>,
}

impl Presenter for PresentadorEspia {
    fn place_over(
        &self,
        rect: hakoyaku::config::Region,
        background: Option<Rgb>,
    ) -> anyhow::Result<()> {
        self.colocaciones.lock().unwrap().push((rect, background));
        Ok(())
    }

    fn show(&self, original: &str, translation: &str) -> anyhow::Result<()> {
        self.eventos.lock().unwrap().push((original.into(), translation.into()));
        Ok(())
    }
    fn clear(&self) -> anyhow::Result<()> {
        self.eventos.lock().unwrap().push(("<clear>".into(), String::new()));
        Ok(())
    }
}

/// Fabrica frames claramente distinguibles: mismo tamano, brillo distinto.
///
/// Antes cambiaban un solo pixel, pero el detector compara medias por bloque
/// con tolerancia —para poder ignorar particulas y fondos animados—, asi que un
/// pixel suelto ya no cuenta como pantalla nueva. Y eso es exactamente lo que
/// se quiere en el juego real.
fn frame(marca: u8) -> Frame {
    let v = marca.saturating_mul(60);
    Frame::new(4, 4, vec![v; 4 * 4 * 4]).unwrap()
}

fn config() -> Config {
    Config {
        region: Region { x: 0, y: 0, width: 4, height: 4 },
        capture: hakoyaku::config::Capture {
            stable_frames: 2,
            upscale: 1,
            preprocess: hakoyaku::config::Preprocess::Off,
            // Sin enfriamiento: los tests dan varias vueltas seguidas y en la
            // vida real entre una y otra pasan cientos de milisegundos.
            cooldown_ms: 0,
            ..Default::default()
        },
        ocr: hakoyaku::config::Ocr { min_chars: 3, ..Default::default() },
        translate: hakoyaku::config::Translate { backend: Backend::None, ..Default::default() },
        ..Default::default()
    }
}

fn ocr_para(pares: &[(&Frame, Vec<&str>)]) -> Box<dyn TextRecognizer> {
    Box::new(HashKeyedOcr {
        tabla: pares.iter().map(|(f, l)| (f.fingerprint(), apilar(l))).collect(),
    })
}

#[test]
fn traduce_una_frase_y_la_muestra() {
    let a = frame(1);
    let capturador = ScriptedCapturer::new(vec![a.clone(), a.clone(), a.clone()]);
    let ocr = ocr_para(&[(&a, vec!["積 雪 の 様 に みっしり と"])]);
    let traductor = TraductorContador::default();
    let vista = PresentadorEspia::default();

    let mut p = Pipeline::new(
        config(),
        Box::new(capturador),
        ocr,
        Box::new(traductor.clone()),
        Box::new(vista.clone()),
    );

    // Primera vuelta: aun no hay dos lecturas iguales.
    assert_eq!(p.step().unwrap(), Step::SinCambios);

    // Segunda: se estabiliza y dispara.
    match p.step().unwrap() {
        Step::Mostrado { original, traduccion, cacheado } => {
            assert_eq!(original, "積雪の様にみっしりと");
            assert_eq!(traduccion, "ES:積雪の様にみっしりと");
            assert!(!cacheado);
        }
        otro => panic!("se esperaba Mostrado, llego {otro:?}"),
    }

    assert_eq!(traductor.llamadas.lock().unwrap().len(), 1);
    assert_eq!(vista.eventos.lock().unwrap().len(), 1);
}

#[test]
fn no_traduce_dos_veces_la_misma_pantalla() {
    let a = frame(1);
    let capturador = ScriptedCapturer::new(vec![a.clone()]);
    let ocr = ocr_para(&[(&a, vec!["こんにちは"])]);
    let traductor = TraductorContador::default();

    let mut p = Pipeline::new(
        config(),
        Box::new(capturador),
        ocr,
        Box::new(traductor.clone()),
        Box::new(PresentadorEspia::default()),
    );

    for _ in 0..12 {
        p.step().unwrap();
    }

    assert_eq!(
        traductor.llamadas.lock().unwrap().len(),
        1,
        "la pantalla no cambio, no deberia haber mas de una llamada"
    );
}

#[test]
fn la_cache_evita_repetir_al_volver_a_una_frase() {
    let a = frame(1);
    let b = frame(2);
    // A estable, B estable, A otra vez.
    let capturador = ScriptedCapturer::new(vec![
        a.clone(),
        a.clone(),
        b.clone(),
        b.clone(),
        a.clone(),
        a.clone(),
    ]);
    let ocr = ocr_para(&[(&a, vec!["おはよう"]), (&b, vec!["こんばんは"])]);
    let traductor = TraductorContador::default();

    let mut p = Pipeline::new(
        config(),
        Box::new(capturador),
        ocr,
        Box::new(traductor.clone()),
        Box::new(PresentadorEspia::default()),
    );

    let mut mostrados = Vec::new();
    for _ in 0..6 {
        if let Step::Mostrado { traduccion, cacheado, .. } = p.step().unwrap() {
            mostrados.push((traduccion, cacheado));
        }
    }

    assert_eq!(mostrados.len(), 3, "se esperaban tres pantallas mostradas: {mostrados:?}");
    assert_eq!(mostrados[2].0, "ES:おはよう");
    assert!(mostrados[2].1, "la tercera deberia venir de la cache");
    assert_eq!(
        traductor.llamadas.lock().unwrap().len(),
        2,
        "solo dos frases distintas -> dos llamadas"
    );

    let (aciertos, _) = p.cache_stats();
    assert_eq!(aciertos, 1);
}

#[test]
fn en_modo_panel_al_cerrarse_la_caja_se_limpia_el_overlay() {
    let con_texto = frame(1);
    let vacio = frame(2);
    let capturador = ScriptedCapturer::new(vec![
        con_texto.clone(),
        con_texto.clone(),
        vacio.clone(),
        vacio.clone(),
    ]);
    // El segundo frame devuelve algo demasiado corto: la caja se ha cerrado.
    let ocr = ocr_para(&[(&con_texto, vec!["こんにちは"]), (&vacio, vec!["・"])]);
    let vista = PresentadorEspia::default();

    let mut cfg = config();
    cfg.overlay.mode = hakoyaku::config::Mode::Panel;

    let mut p = Pipeline::new(
        cfg,
        Box::new(capturador),
        ocr,
        Box::new(TraductorContador::default()),
        Box::new(vista.clone()),
    );

    let pasos: Vec<Step> = (0..4).map(|_| p.step().unwrap()).collect();

    assert!(matches!(pasos[1], Step::Mostrado { .. }), "{:?}", pasos[1]);
    assert!(matches!(pasos[3], Step::Descartado(_)), "{:?}", pasos[3]);

    let eventos = vista.eventos.lock().unwrap().clone();
    assert_eq!(eventos.len(), 2);
    assert_eq!(eventos[1].0, "<clear>", "al cerrarse la caja hay que vaciar el recuadro");
}

/// El bug que hacia parpadear el overlay sin parar.
///
/// En modo in-place el recuadro se dibuja encima de la region vigilada, asi que
/// la siguiente captura recoge el texto en castellano. Al no llevar japones se
/// descarta — y si ahi borrasemos el overlay, reapareceria el japones de debajo
/// y volveria a traducirse. Bucle infinito. La traduccion tiene que quedarse
/// quieta.
#[test]
fn en_modo_inplace_leer_la_propia_traduccion_no_borra_nada() {
    let con_japones = frame(1);
    let con_castellano = frame(2);
    let capturador = ScriptedCapturer::new(vec![
        con_japones.clone(),
        con_japones.clone(),
        con_castellano.clone(),
        con_castellano.clone(),
        con_castellano.clone(),
        con_castellano.clone(),
    ]);
    // El segundo frame simula lo que ve la captura cuando nuestro propio
    // recuadro esta encima: texto latino, sin un solo caracter japones.
    let ocr =
        ocr_para(&[(&con_japones, vec!["こんにちは"]), (&con_castellano, vec!["Hola que tal"])]);
    let vista = PresentadorEspia::default();

    let mut cfg = config();
    cfg.overlay.mode = hakoyaku::config::Mode::Inplace;

    let mut p = Pipeline::new(
        cfg,
        Box::new(capturador),
        ocr,
        Box::new(TraductorContador::default()),
        Box::new(vista.clone()),
    );

    for _ in 0..6 {
        p.step().unwrap();
    }

    let eventos = vista.eventos.lock().unwrap().clone();
    assert_eq!(eventos.len(), 1, "solo debe mostrarse una vez: {eventos:?}");
    assert!(
        !eventos.iter().any(|(o, _)| o == "<clear>"),
        "en in-place no se borra nunca al leerse a si mismo: {eventos:?}"
    );
}

#[test]
fn el_enfriamiento_bloquea_las_relecturas_inmediatas() {
    let a = frame(1);
    let b = frame(2);
    let capturador = ScriptedCapturer::new(vec![a.clone(), a.clone(), b.clone(), b.clone()]);
    let ocr = ocr_para(&[(&a, vec!["こんにちは"]), (&b, vec!["さようなら"])]);
    let traductor = TraductorContador::default();

    let mut cfg = config();
    cfg.capture.cooldown_ms = 60_000; // un minuto: nada mas puede pasar

    let mut p = Pipeline::new(
        cfg,
        Box::new(capturador),
        ocr,
        Box::new(traductor.clone()),
        Box::new(PresentadorEspia::default()),
    );

    for _ in 0..4 {
        p.step().unwrap();
    }

    assert_eq!(
        traductor.llamadas.lock().unwrap().len(),
        1,
        "tras traducir una vez, el enfriamiento tapa el resto"
    );
}

/// Los botones de una novela visual ponen "はい" o "僕": uno o dos caracteres.
/// Con el minimo general de 4 no se traducirian nunca, y sin embargo son
/// justamente lo que el usuario esta senalando a proposito.
#[test]
fn el_minimo_de_caracteres_no_se_aplica_a_lo_que_senalas() {
    let a = frame(1);
    let capturador = ScriptedCapturer::new(vec![a.clone(), a.clone()]);
    let ocr = ocr_para(&[(&a, vec!["はい"])]);
    let traductor = TraductorContador::default();

    let mut cfg = config();
    cfg.ocr.min_chars = 4; // dos caracteres no llegarian
    cfg.cursor.follow = false; // sin raton, se aplica el minimo

    let mut p = Pipeline::new(
        cfg,
        Box::new(capturador),
        ocr,
        Box::new(traductor.clone()),
        Box::new(PresentadorEspia::default()),
    );

    p.step().unwrap();
    p.step().unwrap();
    assert!(
        traductor.llamadas.lock().unwrap().is_empty(),
        "con region fija, dos caracteres se descartan"
    );
}

#[test]
fn un_fallo_de_la_api_se_propaga_como_error() {
    let a = frame(1);
    let capturador = ScriptedCapturer::new(vec![a.clone(), a.clone()]);
    let ocr = ocr_para(&[(&a, vec!["こんにちは"])]);
    let traductor = TraductorContador { fallar: true, ..Default::default() };
    let vista = PresentadorEspia::default();

    let mut p = Pipeline::new(
        config(),
        Box::new(capturador),
        ocr,
        Box::new(traductor),
        Box::new(vista.clone()),
    );

    assert_eq!(p.step().unwrap(), Step::SinCambios);
    assert!(p.step().is_err());
    assert!(vista.eventos.lock().unwrap().is_empty(), "no se muestra nada si falla la traduccion");
}

#[test]
fn en_modo_inplace_se_coloca_el_recuadro_sobre_el_texto() {
    let a = frame(1);
    let capturador = ScriptedCapturer::new(vec![a.clone(), a.clone()]);
    let ocr = ocr_para(&[(&a, vec!["こんにちは"])]);
    let vista = PresentadorEspia::default();

    let mut cfg = config();
    cfg.overlay.mode = hakoyaku::config::Mode::Inplace;
    cfg.region = Region { x: 300, y: 700, width: 4, height: 4 };

    let mut p = Pipeline::new(
        cfg,
        Box::new(capturador),
        ocr,
        Box::new(TraductorContador::default()),
        Box::new(vista.clone()),
    );

    p.step().unwrap();
    p.step().unwrap();

    let colocaciones = vista.colocaciones.lock().unwrap().clone();
    assert_eq!(colocaciones.len(), 1, "deberia haberse colocado una vez");
    let (rect, fondo) = colocaciones[0];
    assert!(rect.x >= 290 && rect.y >= 690, "cerca del origen de la region: {rect:?}");
    assert!(rect.width > 0 && rect.height > 0);
    assert!(fondo.is_some(), "en modo auto hay que muestrear el color de la caja");
}

/// Por defecto el parche cubre la bandeja entera, no solo el hueco que ocupaban
/// las letras: es lo que hace que parezca el texto original sustituido.
#[test]
fn por_defecto_el_parche_cubre_la_caja_entera() {
    let a = frame(1);
    let capturador = ScriptedCapturer::new(vec![a.clone(), a.clone()]);
    let ocr = ocr_para(&[(&a, vec!["こんにちは"])]);
    let vista = PresentadorEspia::default();

    let mut cfg = config();
    cfg.overlay.mode = hakoyaku::config::Mode::Inplace;
    cfg.region = Region { x: 300, y: 700, width: 4, height: 4 };

    let mut p = Pipeline::new(
        cfg,
        Box::new(capturador),
        ocr,
        Box::new(TraductorContador::default()),
        Box::new(vista.clone()),
    );

    p.step().unwrap();
    p.step().unwrap();

    let (rect, _) = vista.colocaciones.lock().unwrap()[0];
    assert_eq!(
        rect,
        Region { x: 300, y: 700, width: 4, height: 4 },
        "deberia taparse la region entera"
    );
}

#[test]
fn con_cover_text_el_parche_se_cine_a_las_letras() {
    let a = frame(1);
    let capturador = ScriptedCapturer::new(vec![a.clone(), a.clone()]);
    let ocr = ocr_para(&[(&a, vec!["こんにちは"])]);
    let vista = PresentadorEspia::default();

    let mut cfg = config();
    cfg.overlay.mode = hakoyaku::config::Mode::Inplace;
    cfg.overlay.inplace_cover = hakoyaku::config::Cover::Text;
    cfg.region = Region { x: 300, y: 700, width: 4, height: 4 };

    let mut p = Pipeline::new(
        cfg,
        Box::new(capturador),
        ocr,
        Box::new(TraductorContador::default()),
        Box::new(vista.clone()),
    );

    p.step().unwrap();
    p.step().unwrap();

    let (rect, _) = vista.colocaciones.lock().unwrap()[0];
    assert_ne!(rect, Region { x: 300, y: 700, width: 4, height: 4 });
    assert!(rect.width > 0 && rect.height > 0);
}

#[test]
fn en_modo_panel_no_se_recoloca_nada() {
    let a = frame(1);
    let capturador = ScriptedCapturer::new(vec![a.clone(), a.clone()]);
    let ocr = ocr_para(&[(&a, vec!["こんにちは"])]);
    let vista = PresentadorEspia::default();

    let mut cfg = config();
    cfg.overlay.mode = hakoyaku::config::Mode::Panel;

    let mut p = Pipeline::new(
        cfg,
        Box::new(capturador),
        ocr,
        Box::new(TraductorContador::default()),
        Box::new(vista.clone()),
    );

    p.step().unwrap();
    p.step().unwrap();

    assert!(vista.colocaciones.lock().unwrap().is_empty());
    assert_eq!(vista.eventos.lock().unwrap().len(), 1, "pero si se muestra la traduccion");
}

#[test]
fn con_show_original_se_manda_tambien_el_japones() {
    let a = frame(1);
    let capturador = ScriptedCapturer::new(vec![a.clone(), a.clone()]);
    let ocr = ocr_para(&[(&a, vec!["こんにちは"])]);
    let vista = PresentadorEspia::default();

    let mut cfg = config();
    cfg.overlay.show_original = true;

    let mut p = Pipeline::new(
        cfg,
        Box::new(capturador),
        ocr,
        Box::new(TraductorContador::default()),
        Box::new(vista.clone()),
    );

    p.step().unwrap();
    p.step().unwrap();

    let eventos = vista.eventos.lock().unwrap().clone();
    assert_eq!(eventos[0].0, "こんにちは");
    assert_eq!(eventos[0].1, "ES:こんにちは");
}

#[test]
fn el_texto_a_medias_no_se_traduce() {
    // Simula el efecto maquina de escribir: tres pantallas distintas seguidas y
    // solo la ultima se queda quieta.
    let p1 = frame(1);
    let p2 = frame(2);
    let p3 = frame(3);
    let capturador =
        ScriptedCapturer::new(vec![p1.clone(), p2.clone(), p3.clone(), p3.clone(), p3.clone()]);
    let ocr = ocr_para(&[
        (&p1, vec!["積雪"]),
        (&p2, vec!["積雪の様に"]),
        (&p3, vec!["積雪の様にみっしりと"]),
    ]);
    let traductor = TraductorContador::default();

    let mut cfg = config();
    cfg.capture.stable_frames = 2;

    let mut p = Pipeline::new(
        cfg,
        Box::new(capturador),
        ocr,
        Box::new(traductor.clone()),
        Box::new(PresentadorEspia::default()),
    );

    for _ in 0..5 {
        p.step().unwrap();
    }

    let llamadas = traductor.llamadas.lock().unwrap().clone();
    assert_eq!(
        llamadas,
        vec!["積雪の様にみっしりと".to_string()],
        "solo debe traducirse la frase completa, no los estados intermedios"
    );
}
