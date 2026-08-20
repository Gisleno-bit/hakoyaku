//! El bucle: mirar la pantalla -> detectar cambio -> OCR -> traducir -> mostrar.
//!
//! Esta escrito contra los traits (`ScreenCapturer`, `TextRecognizer`,
//! `Translator`, `Presenter`), no contra las implementaciones. Por eso el test
//! de integracion puede montar un pipeline completo con mocks y comprobar el
//! comportamiento sin Windows, sin pantalla y sin red.

use crate::cache::TranslationCache;
use crate::capture::ScreenCapturer;
use crate::config::{Config, Cover, Mode, Preprocess};
use crate::cursor;
use crate::frame::{bloques_distintos, Frame};
use crate::hotkeys::Control;
use crate::ocr::{self, TextRecognizer, TextRect};
use crate::overlay::{Presenter, Rgb};
use crate::target;
use crate::text;
use crate::translate::Translator;
use anyhow::Result;
use std::time::{Duration, Instant};

/// Umbral de luminancia por debajo del cual consideramos que la caja es oscura.
const LUMA_OSCURA: u8 = 110;

/// Rejilla con la que se compara la pantalla.
///
/// 32x16 sobre una caja de dialogo tipica da bloques de unas decenas de
/// pixeles: suficiente para que una linea de texto nueva mueva varios, y para
/// que una particula de fondo no mueva ninguno.
pub const REJILLA: (u32, u32) = (32, 16);

/// Decide cuando merece la pena gastar una pasada de OCR.
///
/// Dos problemas que resuelve:
///
/// - el texto que se escribe letra a letra: sin esto traduciriamos "Sale a un
///   esp" y luego "Sale a un espacio", pagando dos veces por media frase;
/// - el ruido: sombras, cursores que parpadean, el propio raton por encima.
#[derive(Debug, Default)]
pub struct StabilityDetector {
    ultima: Option<Vec<u8>>,
    repeticiones: u32,
    ya_procesada: Option<Vec<u8>>,
}

impl StabilityDetector {
    pub fn new() -> Self {
        Self::default()
    }

    /// `true` si este fotograma debe pasar por OCR.
    ///
    /// La comparacion es con tolerancia, no exacta: un fondo con particulas o
    /// un degradado que respira no deben impedir que la pantalla se considere
    /// quieta, o no se leeria nunca.
    pub fn observe(
        &mut self,
        firma: &[u8],
        estables_necesarios: u32,
        tolerancia: u8,
        bloques: usize,
    ) -> bool {
        let necesarios = estables_necesarios.max(1);
        let minimo = bloques.max(1);

        let igual_que_antes =
            self.ultima.as_ref().is_some_and(|u| bloques_distintos(u, firma, tolerancia) < minimo);

        if igual_que_antes {
            self.repeticiones += 1;
        } else {
            self.ultima = Some(firma.to_vec());
            self.repeticiones = 1;
        }

        if self.repeticiones != necesarios {
            return false;
        }

        let ya_vista = self
            .ya_procesada
            .as_ref()
            .is_some_and(|p| bloques_distintos(p, firma, tolerancia) < minimo);

        if ya_vista {
            false
        } else {
            self.ya_procesada = Some(firma.to_vec());
            true
        }
    }

    /// Olvida lo ya procesado para forzar una relectura.
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

/// Prepara el fotograma para el OCR segun la configuracion.
pub fn preprocess(frame: &Frame, cfg: &Config) -> Frame {
    let escalado = frame.upscale_nearest(cfg.capture.upscale.max(1));

    match cfg.capture.preprocess {
        Preprocess::Off => escalado,
        Preprocess::Invert => escalado.invert(),
        Preprocess::Binarize => {
            let oscura = escalado.mean_luma() < LUMA_OSCURA;
            escalado.binarize(cfg.capture.binarize_threshold, oscura)
        }
        Preprocess::Auto => {
            if escalado.mean_luma() < LUMA_OSCURA {
                escalado.invert()
            } else {
                escalado
            }
        }
    }
}

/// Que ha pasado en una vuelta del bucle.
///
/// `EsperandoVentana` es informativo: el juego esta minimizado, cerrado, o
/// simplemente no tiene el foco.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// La pantalla no ha cambiado, o aun no lleva suficientes lecturas iguales.
    SinCambios,
    /// Cambio, pero el OCR no saco nada aprovechable.
    Descartado(String),
    /// Se ha mostrado una traduccion.
    Mostrado { original: String, traduccion: String, cacheado: bool },
    /// La ventana del juego no esta disponible ahora mismo.
    EsperandoVentana(String),
}

pub struct Pipeline {
    pub cfg: Config,
    capturer: Box<dyn ScreenCapturer>,
    ocr: Box<dyn TextRecognizer>,
    translator: Box<dyn Translator>,
    presenter: Box<dyn Presenter>,
    detector: StabilityDetector,
    cache: TranslationCache,
    ultimo_texto: String,
    /// Cuando se mostro la ultima traduccion, para el periodo de enfriamiento.
    ultima_vez: Option<Instant>,
    control: Option<std::sync::Arc<Control>>,
    /// Region en coordenadas de pantalla usada en la ultima vuelta.
    region_vigente: crate::config::Region,
    /// Ultima caja detectada bajo el cursor, para que no tiemble.
    caja_bajo_cursor: Option<crate::config::Region>,
    /// La region de esta vuelta viene de senalar con el raton (y no de la
    /// region fija de reserva).
    region_por_cursor: bool,
    /// Donde estaba el raton la ultima vez que se detecto una caja.
    ultimo_punto: Option<(i32, i32)>,
    ultima_deteccion: Option<Instant>,
    /// (ms de OCR, ms de traduccion) de la ultima frase. Se ensena al usuario
    /// para que sepa donde se le esta yendo el tiempo en vez de suponerlo.
    ultimos_tiempos: (u128, u128),
    /// Capturador auxiliar para la busqueda alrededor del cursor.
    buscador: Option<Box<dyn ScreenCapturer>>,
}

impl Pipeline {
    pub fn new(
        cfg: Config,
        capturer: Box<dyn ScreenCapturer>,
        ocr: Box<dyn TextRecognizer>,
        translator: Box<dyn Translator>,
        presenter: Box<dyn Presenter>,
    ) -> Self {
        let cache = TranslationCache::new(cfg.translate.cache_size);
        let region_vigente = cfg.region;
        Self {
            region_vigente,
            caja_bajo_cursor: None,
            region_por_cursor: false,
            ultimo_punto: None,
            ultima_deteccion: None,
            ultimos_tiempos: (0, 0),
            buscador: None,
            cfg,
            capturer,
            ocr,
            translator,
            presenter,
            detector: StabilityDetector::new(),
            cache,
            ultimo_texto: String::new(),
            ultima_vez: None,
            control: None,
        }
    }

    /// Conecta los atajos de teclado globales.
    pub fn con_control(mut self, control: std::sync::Arc<Control>) -> Self {
        self.control = Some(control);
        self
    }

    /// Una vuelta completa. No duerme: de eso se encarga `run`.
    pub fn step(&mut self) -> Result<Step> {
        // F10 fuerza una relectura. Se comprueba ANTES de resolver la region
        // pero solo se consume despues, cuando ya se sabe que la vuelta va a
        // llegar a alguna parte: si se consumiera aqui y luego resultara que el
        // raton esta fuera de la ventana, la pulsacion se perderia en el vacio
        // y desde fuera pareceria que F10 no hace nada.
        let pendiente = self.control.as_ref().is_some_and(|c| c.hay_relectura());

        // Periodo de enfriamiento: recien puesta una traduccion no tiene sentido
        // volver a mirar. Sin esto, cualquier animacion de fondo dispara una
        // relectura por segundo.
        if let Some(t) = self.ultima_vez {
            if t.elapsed() < Duration::from_millis(self.cfg.capture.cooldown_ms) {
                return Ok(Step::SinCambios);
            }
        }

        let region = match self.region_actual() {
            Ok(r) => r,
            Err(motivo) => return Ok(Step::EsperandoVentana(motivo)),
        };
        self.region_vigente = region;

        // Ya hay region: ahora si se consume la peticion de relectura.
        let forzar = pendiente && self.control.as_ref().is_some_and(|c| c.tomar_relectura());
        if forzar {
            self.detector.reset();
            self.ultima_vez = None;
            self.ultimo_texto.clear();
        }

        let frame = self.capturer.capture(region)?;
        let firma = frame.signature(REJILLA.0, REJILLA.1);

        // Con F10 se salta el detector entero. Antes solo se reseteaba, asi que
        // en un juego con fondo animado seguia sin leer nunca: la pantalla no
        // llegaba a estabilizarse. Forzar tiene que ser forzar.
        if !forzar
            && !self.detector.observe(
                &firma,
                self.cfg.capture.stable_frames,
                self.cfg.capture.change_tolerance,
                self.cfg.capture.change_blocks,
            )
        {
            return Ok(Step::SinCambios);
        }

        let reloj = Instant::now();
        let preparado = preprocess(&frame, &self.cfg);
        let lineas = self.ocr.recognize(&preparado)?;
        let ms_ocr = reloj.elapsed().as_millis();
        let texto = text::clean_ocr_lines(&ocr::textos(&lineas));

        // Si el usuario ha senalado una caja con el raton, ha dicho
        // explicitamente que quiere ESO traducido. Filtrar por longitud ahi es
        // absurdo: los botones ponen "はい" (2 caracteres) o "僕" (1), y con el
        // minimo general de 4 no se traducirian nunca.
        let minimo = if self.region_por_cursor { 1 } else { self.cfg.ocr.min_chars };

        if !text::is_worth_translating(&texto, minimo, self.cfg.ocr.require_cjk) {
            // En modo in-place NO se borra. Si se borrase, desapareceria el
            // parche, volveria a verse el japones de debajo, se traduciria otra
            // vez, se pintaria encima... y a parpadear. La traduccion se queda
            // quieta hasta que aparezca texto nuevo de verdad.
            if self.cfg.overlay.mode != Mode::Inplace && !self.ultimo_texto.is_empty() {
                self.presenter.clear()?;
                self.ultimo_texto.clear();
            }
            return Ok(Step::Descartado(texto));
        }

        if texto == self.ultimo_texto && !forzar {
            return Ok(Step::SinCambios);
        }

        let reloj = Instant::now();
        let (traduccion, cacheado) = match self.cache.get(&texto) {
            Some(t) => (t.to_string(), true),
            None => {
                let t = self.translator.translate(&texto)?;
                self.cache.insert(texto.clone(), t.clone());
                (t, false)
            }
        };
        let ms_traduccion = reloj.elapsed().as_millis();
        self.ultimos_tiempos = (ms_ocr, ms_traduccion);

        // Modo in-place: colocar el recuadro justo encima del texto original,
        // del color de la caja del juego, para que parezca que el juego esta
        // traducido y no que hay un cartel pegado encima.
        if self.cfg.overlay.mode == Mode::Inplace {
            let destino = match self.cfg.overlay.inplace_cover {
                // La caja entera: el parche cubre toda la bandeja de dialogo,
                // que es lo que hace que parezca el texto original sustituido y
                // no un recorte pegado encima. Ademas deja sitio de sobra, que
                // hace falta porque el castellano ocupa mas que el japones.
                Cover::Box => Some(self.region_vigente),
                // Solo donde habia letras.
                Cover::Text => TextRect::envolvente(&lineas).map(|c| {
                    c.a_pantalla(
                        self.cfg.capture.upscale,
                        self.region_vigente,
                        self.cfg.overlay.inplace_padding,
                    )
                }),
            };
            if let Some(destino) = destino {
                self.presenter.place_over(destino, self.fondo_para(&frame))?;
            }
        }

        let original = if self.cfg.overlay.show_original { texto.as_str() } else { "" };
        self.presenter.show(original, &traduccion)?;
        self.ultimo_texto = texto.clone();
        self.ultima_vez = Some(Instant::now());

        Ok(Step::Mostrado { original: texto, traduccion, cacheado })
    }

    /// Bucle infinito. Se sale con Ctrl+C.
    ///
    /// Un error de traduccion (se cayo la red, la API dio 429) no tumba el
    /// programa: se registra y se sigue mirando la pantalla. Un error de
    /// captura si es fatal, porque significa que algo va mal de verdad.
    pub fn run(&mut self) -> Result<()> {
        let espera = Duration::from_millis(self.cfg.capture.poll_ms);
        let mut fallos_seguidos = 0u32;
        // Se imprime solo cuando cambia el motivo, no en cada vuelta: si no,
        // seria una cascada. Pero se imprime, que es lo importante: quedarse
        // mirando una consola muda sin saber por que no pasa nada es lo peor
        // que puede hacer un programa asi.
        let mut ultimo_motivo = String::new();

        let mut estaba_oculto = false;

        loop {
            if let Some(c) = &self.control {
                if c.hay_que_salir() {
                    println!("\nHasta luego.");
                    return Ok(());
                }

                let oculto = c.esta_oculto();
                if oculto != estaba_oculto {
                    estaba_oculto = oculto;
                    if oculto {
                        // Quitar el recuadro de en medio para ver el juego limpio.
                        let _ = self.presenter.clear();
                        self.ultimo_texto.clear();
                    } else {
                        // Al volver, releer para que la traduccion reaparezca al
                        // momento en vez de esperar al siguiente dialogo.
                        c.pedir_relectura();
                    }
                }
                if oculto || c.esta_pausado() {
                    std::thread::sleep(espera);
                    continue;
                }
            }

            match self.step() {
                Ok(Step::Mostrado { original, traduccion, cacheado }) => {
                    fallos_seguidos = 0;
                    ultimo_motivo.clear();
                    let (ocr_ms, tr_ms) = self.ultimos_tiempos;
                    let marca = if cacheado {
                        format!("cache, ocr {ocr_ms}ms")
                    } else {
                        format!("ocr {ocr_ms}ms + traducir {tr_ms}ms")
                    };
                    println!("[{marca}] {original}\n        -> {traduccion}");
                }
                Ok(Step::EsperandoVentana(motivo)) => {
                    fallos_seguidos = 0;
                    if motivo != ultimo_motivo {
                        println!("[esperando] {motivo}");
                        ultimo_motivo = motivo;
                    }
                }
                Ok(Step::Descartado(t)) => {
                    fallos_seguidos = 0;
                    let motivo = if t.trim().is_empty() {
                        "no se ha leido texto en la zona".to_string()
                    } else {
                        format!("texto descartado por los filtros: {t}")
                    };
                    if motivo != ultimo_motivo {
                        println!("[sin traducir] {motivo}");
                        ultimo_motivo = motivo;
                    }
                }
                Ok(_) => {
                    fallos_seguidos = 0;
                }
                Err(e) => {
                    fallos_seguidos += 1;
                    log::warn!("fallo en la vuelta {fallos_seguidos}: {e:#}");
                    if fallos_seguidos >= 10 {
                        return Err(e.context("10 fallos seguidos; se aborta"));
                    }
                    // Backoff para no machacar una API caida.
                    std::thread::sleep(espera * 4);
                    continue;
                }
            }
            std::thread::sleep(espera);
        }
    }

    /// Donde hay que mirar en este instante.
    ///
    /// Sin anclaje, la region del fichero tal cual. Con anclaje, se localiza la
    /// ventana del juego y la region se traslada a donde este ahora — que es lo
    /// que permite mover o redimensionar el juego sin volver a marcarla.
    fn region_actual(&mut self) -> std::result::Result<crate::config::Region, String> {
        if self.cfg.cursor.follow {
            return self.caja_del_cursor();
        }

        if self.cfg.anclado() && self.cfg.target.only_when_focused {
            let titulo = self.cfg.target.window_title.trim();
            match target::buscar(titulo) {
                Ok(Some(v)) if !target::tiene_el_foco(v.hwnd) => {
                    return Err(format!("'{}' no tiene el foco", v.titulo));
                }
                Ok(None) => return Err(format!("no encuentro la ventana '{titulo}'")),
                Err(e) => return Err(format!("no se pudo buscar la ventana: {e}")),
                _ => {}
            }
        }

        target::resolver(&self.cfg).map_err(|e| e.to_string())
    }

    /// Busca la caja de dialogo que hay ahora mismo bajo el raton.
    ///
    /// Se captura una zona acotada alrededor del cursor —no la pantalla entera,
    /// que costaria demasiado varias veces por segundo— y se detectan los bordes
    /// desde el punto exacto donde apunta.
    fn caja_del_cursor(&mut self) -> std::result::Result<crate::config::Region, String> {
        let punto = cursor::posicion().ok_or("no se pudo leer la posicion del raton")?;

        // Los limites son la ventana del juego si hay anclaje, o el escritorio.
        let limites = if self.cfg.anclado() {
            let titulo = self.cfg.target.window_title.trim();
            match target::buscar(titulo) {
                // No se mira el foco a proposito: en modo raton, senalar ya es
                // senal de intencion, y exigir foco convierte a la consola en
                // una trampa. Lo que si se comprueba es que el juego sea lo que
                // esta dibujado bajo el cursor, porque si tienes otra ventana
                // encima el punto cae dentro del rectangulo del juego pero lo
                // que se captura es la otra ventana.
                // La ventana concreta que devolvio `buscar` no se usa: lo que
                // vale es la que este dibujada bajo el cursor, que se resuelve
                // justo debajo.
                Ok(Some(_)) => {
                    // Muchos juegos abren mas de una ventana de nivel superior
                    // (la de render y una de informacion). Comparar contra la
                    // que eligio `buscar` fallaba cuando el raton estaba sobre
                    // la otra, aunque fuera del mismo juego. Lo que importa es
                    // que lo dibujado bajo el cursor pertenezca al juego, asi
                    // que se comprueba contra el titulo, no contra un HWND
                    // concreto — y se usa esa ventana, que es la que se ve.
                    match target::ventana_bajo_el_cursor(punto, titulo) {
                        Some(real) => real.cliente,
                        None => return Err("hay otra ventana encima del juego".into()),
                    }
                }
                Ok(None) => return Err(format!("no encuentro la ventana '{titulo}'")),
                Err(e) => return Err(format!("no se pudo buscar la ventana: {e}")),
            }
        } else {
            let (x, y, w, h) = crate::capture::virtual_screen();
            crate::config::Region { x, y, width: w as u32, height: h as u32 }
        };

        if punto.0 < limites.x
            || punto.1 < limites.y
            || punto.0 >= limites.right()
            || punto.1 >= limites.bottom()
        {
            return Err("el raton esta fuera de la ventana del juego".into());
        }

        // Si el raton no se ha movido y ya se sabe donde esta la caja, no hace
        // falta volver a capturar la zona de busqueda ni a barrer bordes: esa
        // es la parte cara de la vuelta. Cada dos segundos se rehace de todos
        // modos, por si el juego ha cambiado de cuadro sin que se mueva nada.
        let quieto = self.ultimo_punto == Some(punto);
        let reciente =
            self.ultima_deteccion.is_some_and(|t| t.elapsed() < Duration::from_millis(2000));

        if quieto && reciente {
            if let Some(caja) = self.caja_bajo_cursor {
                self.region_por_cursor = true;
                return Ok(caja);
            }
        }
        self.ultimo_punto = Some(punto);
        self.ultima_deteccion = Some(Instant::now());

        let busqueda = cursor::area_de_busqueda(
            punto,
            limites,
            self.cfg.cursor.search_width,
            self.cfg.cursor.search_height,
        );

        if self.buscador.is_none() {
            self.buscador = Some(crate::capture::create().map_err(|e| e.to_string())?);
        }
        let frame = self
            .buscador
            .as_mut()
            .unwrap()
            .capture(busqueda)
            .map_err(|e| format!("no se pudo capturar alrededor del raton: {e}"))?;

        let relativo = ((punto.0 - busqueda.x) as u32, (punto.1 - busqueda.y) as u32);
        let caja =
            match cursor::detectar(&frame, relativo, busqueda, self.cfg.cursor.edge_tolerance) {
                Some(c) => c,
                // Sin caja bajo el raton no hay que rendirse: el cuadro de dialogo
                // principal suele ser una banda semitransparente sin borde, con el
                // fondo del juego transparentandose, y ahi la deteccion por bordes
                // no tiene nada a lo que agarrarse. Pero ese cuadro siempre esta en
                // el mismo sitio, asi que si hay una region marcada se usa esa.
                None => {
                    if self.cfg.region.width > 0 && self.cfg.region.height > 0 {
                        self.region_por_cursor = false;
                        return target::resolver(&self.cfg).map_err(|e| e.to_string());
                    }
                    return Err(
                        "no hay caja bajo el raton y no hay region marcada de reserva".into()
                    );
                }
            };

        let absoluta = crate::config::Region {
            x: busqueda.x + caja.x,
            y: busqueda.y + caja.y,
            width: caja.width,
            height: caja.height,
        };

        // Si es practicamente la misma de antes, se reutiliza la anterior: asi
        // el recuadro de la traduccion no baila con cada temblor de la
        // deteccion.
        if cursor::es_la_misma(self.caja_bajo_cursor, absoluta, self.cfg.cursor.stickiness) {
            self.region_por_cursor = true;
            return Ok(self.caja_bajo_cursor.unwrap());
        }

        self.region_por_cursor = true;
        self.caja_bajo_cursor = Some(absoluta);
        Ok(absoluta)
    }

    /// Color con el que tapar el texto original.
    ///
    /// Se muestrea del propio fotograma sin preprocesar: es el unico que tiene
    /// los colores reales del juego. Si el usuario fijo un color a mano, manda
    /// el suyo.
    fn fondo_para(&self, frame: &Frame) -> Option<Rgb> {
        let ajuste = self.cfg.overlay.inplace_background.trim();
        if ajuste.eq_ignore_ascii_case("auto") {
            Some(frame.dominant_color())
        } else {
            crate::config::parse_color(ajuste).ok()
        }
    }

    pub fn cache_stats(&self) -> (u64, u64) {
        self.cache.stats()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Region;

    /// Firma de un solo bloque, para leer los tests con comodidad.
    fn f(v: u8) -> Vec<u8> {
        vec![v]
    }

    #[test]
    fn no_dispara_hasta_alcanzar_las_lecturas_estables() {
        let mut d = StabilityDetector::new();
        assert!(!d.observe(&f(10), 3, 0, 1));
        assert!(!d.observe(&f(10), 3, 0, 1));
        assert!(d.observe(&f(10), 3, 0, 1));
    }

    #[test]
    fn no_repite_el_disparo_mientras_no_cambie() {
        let mut d = StabilityDetector::new();
        d.observe(&f(10), 2, 0, 1);
        assert!(d.observe(&f(10), 2, 0, 1));
        assert!(!d.observe(&f(10), 2, 0, 1));
        assert!(!d.observe(&f(10), 2, 0, 1));
    }

    #[test]
    fn un_cambio_a_medias_reinicia_la_cuenta() {
        let mut d = StabilityDetector::new();
        assert!(!d.observe(&f(10), 3, 0, 1));
        assert!(!d.observe(&f(10), 3, 0, 1));
        assert!(!d.observe(&f(90), 3, 0, 1)); // el texto seguia escribiendose
        assert!(!d.observe(&f(90), 3, 0, 1));
        assert!(d.observe(&f(90), 3, 0, 1));
    }

    #[test]
    fn con_un_solo_frame_estable_dispara_ya() {
        let mut d = StabilityDetector::new();
        assert!(d.observe(&f(70), 1, 0, 1));
        assert!(!d.observe(&f(70), 1, 0, 1));
        assert!(d.observe(&f(180), 1, 0, 1));
    }

    #[test]
    fn cero_frames_estables_se_trata_como_uno() {
        let mut d = StabilityDetector::new();
        assert!(d.observe(&f(70), 0, 0, 1));
    }

    #[test]
    fn volver_a_una_pantalla_anterior_si_dispara() {
        let mut d = StabilityDetector::new();
        assert!(d.observe(&f(10), 1, 0, 1));
        assert!(d.observe(&f(90), 1, 0, 1));
        assert!(d.observe(&f(10), 1, 0, 1)); // se cerro y se reabrio la caja
    }

    #[test]
    fn el_reset_permite_releer_lo_mismo() {
        let mut d = StabilityDetector::new();
        assert!(d.observe(&f(50), 1, 0, 1));
        assert!(!d.observe(&f(50), 1, 0, 1));
        d.reset();
        assert!(d.observe(&f(50), 1, 0, 1));
    }

    /// El caso que hacia que en una sala con particulas cayendo no se leyera
    /// nunca: la pantalla cambiaba un poquito en cada fotograma y no llegaba a
    /// acumular las lecturas estables que hacen falta.
    #[test]
    fn el_fondo_animado_no_impide_estabilizarse() {
        let mut d = StabilityDetector::new();
        // Ocho bloques que tiemblan +-3 de brillo entre fotogramas.
        let a = vec![100, 100, 100, 100, 100, 100, 100, 100];
        let b = vec![103, 98, 101, 97, 102, 100, 99, 101];
        let c = vec![98, 102, 99, 103, 100, 101, 102, 98];

        assert!(!d.observe(&a, 3, 10, 4));
        assert!(!d.observe(&b, 3, 10, 4));
        assert!(d.observe(&c, 3, 10, 4), "el temblor de fondo no deberia contar como cambio");
    }

    #[test]
    fn un_dialogo_nuevo_si_rompe_la_estabilidad() {
        let mut d = StabilityDetector::new();
        let quieto = vec![100; 8];
        let con_texto = vec![100, 100, 20, 20, 20, 20, 100, 100];

        assert!(!d.observe(&quieto, 2, 10, 4));
        assert!(d.observe(&quieto, 2, 10, 4));
        // Cuatro bloques cambian de golpe: eso es texto nuevo.
        assert!(!d.observe(&con_texto, 2, 10, 4));
        assert!(d.observe(&con_texto, 2, 10, 4));
    }

    fn cfg_con(preprocesado: Preprocess, upscale: u32) -> Config {
        Config {
            region: Region { x: 0, y: 0, width: 2, height: 2 },
            capture: crate::config::Capture {
                preprocess: preprocesado,
                upscale,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn el_preprocesado_off_solo_escala() {
        let f = Frame::blank(2, 2);
        let out = preprocess(&f, &cfg_con(Preprocess::Off, 3));
        assert_eq!((out.width, out.height), (6, 6));
        assert_eq!(out.pixel(0, 0), [0, 0, 0, 0]);
    }

    #[test]
    fn el_preprocesado_auto_invierte_las_cajas_oscuras() {
        let oscuro = Frame::blank(2, 2); // negro puro
        let out = preprocess(&oscuro, &cfg_con(Preprocess::Auto, 1));
        assert_eq!(out.pixel(0, 0), [255, 255, 255, 255]);
    }

    #[test]
    fn el_preprocesado_auto_no_toca_las_cajas_claras() {
        let claro = Frame::new(1, 1, vec![250, 250, 250, 255]).unwrap();
        let out = preprocess(&claro, &cfg_con(Preprocess::Auto, 1));
        assert_eq!(out.pixel(0, 0), [250, 250, 250, 255]);
    }

    #[test]
    fn el_preprocesado_invert_invierte_siempre() {
        let claro = Frame::new(1, 1, vec![250, 250, 250, 255]).unwrap();
        let out = preprocess(&claro, &cfg_con(Preprocess::Invert, 1));
        assert_eq!(out.pixel(0, 0), [5, 5, 5, 255]);
    }

    #[test]
    fn el_preprocesado_binarize_da_blanco_o_negro() {
        let gris = Frame::new(2, 1, vec![200, 200, 200, 255, 20, 20, 20, 255]).unwrap();
        let out = preprocess(&gris, &cfg_con(Preprocess::Binarize, 1));
        for px in out.data.chunks_exact(4) {
            assert!(px[0] == 0 || px[0] == 255, "{px:?}");
        }
    }

    #[test]
    fn la_caja_del_ocr_acaba_donde_esta_el_texto_en_pantalla() {
        // Region en 300,700; el OCR trabajo sobre una imagen al triple.
        let region = Region { x: 300, y: 700, width: 600, height: 120 };
        let lineas = crate::ocr::apilar(&["ab", "cd"]);
        let caja = TextRect::envolvente(&lineas).unwrap();
        let destino = caja.a_pantalla(3, region, 4);

        assert!(destino.x >= region.x - 4, "{destino:?}");
        assert!(destino.y >= region.y - 4, "{destino:?}");
        assert!(destino.width > 0 && destino.height > 0);
    }

    #[test]
    fn el_escalado_cero_no_rompe() {
        let f = Frame::blank(2, 2);
        let out = preprocess(&f, &cfg_con(Preprocess::Off, 0));
        assert_eq!((out.width, out.height), (2, 2));
    }
}
