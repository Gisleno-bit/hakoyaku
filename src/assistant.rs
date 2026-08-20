//! Lo que pasa cuando abres `hakoyaku.exe` con doble clic.
//!
//! Sin esto, el ejecutable arranca sin argumentos, clap imprime la ayuda y la
//! ventana se cierra de golpe: desde fuera parece que el programa no funciona.
//!
//! El asistente hace tres cosas: diagnostica que falta, ofrece un menu para
//! arreglarlo, y **nunca cierra la ventana sin avisar**.

use crate::config::{Backend, Config};
use anyhow::Result;
use std::io::Write;
use std::path::Path;

/// Estado de cada requisito, para pintar el diagnostico.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Chequeo {
    Bien(String),
    Aviso(String),
    Mal(String),
}

impl Chequeo {
    pub fn simbolo(&self) -> &'static str {
        match self {
            Chequeo::Bien(_) => "[ok]",
            Chequeo::Aviso(_) => "[ ?]",
            Chequeo::Mal(_) => "[!!]",
        }
    }

    pub fn mensaje(&self) -> &str {
        match self {
            Chequeo::Bien(m) | Chequeo::Aviso(m) | Chequeo::Mal(m) => m,
        }
    }

    pub fn es_bloqueante(&self) -> bool {
        matches!(self, Chequeo::Mal(_))
    }
}

/// Revisa la configuracion y dice que falta, en orden de importancia.
///
/// Es una funcion pura sobre `Config` e `idiomas`, asi que se puede testear sin
/// tocar la pantalla ni el registro de Windows.
pub fn diagnostico(cfg: Option<&Config>, idiomas: &[String]) -> Vec<Chequeo> {
    let mut out = Vec::new();

    let cfg = match cfg {
        Some(c) => c,
        None => {
            out.push(Chequeo::Mal("No hay hakoyaku.toml todavia".into()));
            return out;
        }
    };
    out.push(Chequeo::Bien("Configuracion encontrada".into()));

    // Idioma de OCR
    let pedido = cfg.ocr.language.to_lowercase();
    let hay = idiomas.iter().any(|d| {
        let d = d.to_lowercase();
        d == pedido || d.starts_with(&format!("{pedido}-")) || pedido.starts_with(&format!("{d}-"))
    });
    if hay {
        out.push(Chequeo::Bien(format!("OCR de '{}' instalado", cfg.ocr.language)));
    } else if idiomas.is_empty() {
        out.push(Chequeo::Mal(
            "Windows no tiene ningun motor de OCR instalado (opcion 3 del menu)".into(),
        ));
    } else {
        out.push(Chequeo::Mal(format!(
            "Falta el OCR de '{}'. Instalados: {}",
            cfg.ocr.language,
            idiomas.join(", ")
        )));
    }

    // Aplicacion anclada
    if cfg.anclado() {
        out.push(Chequeo::Bien(format!(
            "Anclado a la ventana '{}'",
            cfg.target.window_title.trim()
        )));
    } else {
        out.push(Chequeo::Aviso(
            "Sin aplicacion elegida: la region va en coordenadas de pantalla y se \
             descuadra si mueves el juego (opcion 1)"
                .into(),
        ));
    }

    // Region
    if cfg.cursor.follow {
        out.push(Chequeo::Bien(
            "Modo sigue-al-raton: se traduce la caja que senales con el cursor".into(),
        ));
    } else if cfg.region.width == 0 || cfg.region.height == 0 {
        out.push(Chequeo::Mal("Falta marcar la region a vigilar (opcion 1)".into()));
    } else {
        let descripcion = format!(
            "Region: {},{} de {}x{}",
            cfg.region.x, cfg.region.y, cfg.region.width, cfg.region.height
        );
        if region_demasiado_grande(cfg.region.width, cfg.region.height) {
            out.push(Chequeo::Aviso(format!(
                "{descripcion} — parece la ventana entera, no un cuadro de dialogo. \
                 Va lento y mezcla textos de sitios distintos. Marca solo la caja (opcion 1)."
            )));
        } else {
            out.push(Chequeo::Bien(descripcion));
        }
    }

    // Traductor
    match cfg.translate.backend {
        Backend::None => out.push(Chequeo::Aviso(
            "El traductor esta en 'none': solo veras el texto sin traducir".into(),
        )),
        Backend::Deepl | Backend::Google if cfg.translate.api_key.trim().is_empty() => {
            out.push(Chequeo::Mal(format!(
                "El backend {:?} necesita una clave de API (opcion 4)",
                cfg.translate.backend
            )));
        }
        b => out.push(Chequeo::Bien(format!("Traductor {:?} -> {}", b, cfg.translate.target_lang))),
    }

    out
}

/// `true` si la region parece la ventana del juego entera en vez de su cuadro
/// de dialogo.
///
/// Un cuadro de dialogo es ancho y bajo. Cuando la altura pasa de unos 350
/// pixeles, o el area se acerca a la de una ventana completa, casi siempre
/// significa que se han marcado las esquinas del juego y no las del texto.
pub fn region_demasiado_grande(ancho: u32, alto: u32) -> bool {
    alto > 350 || (ancho as u64 * alto as u64) > 600_000
}

/// `true` si todo lo imprescindible esta listo para traducir.
pub fn listo(chequeos: &[Chequeo]) -> bool {
    !chequeos.iter().any(Chequeo::es_bloqueante)
}

fn leer_linea(prompt: &str) -> String {
    print!("{prompt}");
    std::io::stdout().flush().ok();
    let mut buf = String::new();
    std::io::stdin().read_line(&mut buf).ok();
    buf.trim().to_string()
}

fn pausa() {
    println!();
    leer_linea("Pulsa Enter para continuar...");
}

pub const BANNER: &str = r#"
  ┌──────────────────────────────────────────────┐
  │   hakoyaku · traductor de pantalla en vivo    │
  └──────────────────────────────────────────────┘
"#;

/// Bucle principal del asistente. Devuelve que quiere hacer el usuario.
pub fn ejecutar(ruta: &Path) -> Result<Option<Accion>> {
    println!("{BANNER}");

    loop {
        let cfg = if ruta.exists() {
            match Config::load_lenient(ruta) {
                Ok(mut c) => {
                    c.apply_env_overrides();
                    Some(c)
                }
                Err(e) => {
                    println!("El fichero {} tiene un problema:\n  {e:#}\n", ruta.display());
                    None
                }
            }
        } else {
            None
        };

        let idiomas = crate::ocr::available_languages().unwrap_or_default();
        let chequeos = diagnostico(cfg.as_ref(), &idiomas);

        println!("Estado:");
        for c in &chequeos {
            println!("  {} {}", c.simbolo(), c.mensaje());
        }
        println!();

        if listo(&chequeos) {
            println!("Todo listo. Con la opcion 5 empieza a traducir.\n");
        }

        println!("  1) Elegir la aplicacion (el juego) y marcar la region");
        println!("  2) Ver el marco sobre la pantalla (comprobar encuadre)");
        println!("  3) Ver los idiomas de OCR instalados");
        println!("  4) Elegir traductor e introducir la clave");
        println!("  5) EMPEZAR A TRADUCIR");
        println!("  6) Probar el OCR sin traducir (diagnostico)");
        println!(
            "  7) Modo SIGUE AL RATON  [{}]",
            if cfg.as_ref().is_some_and(|c| c.cursor.follow) { "activado" } else { "desactivado" }
        );
        println!("  0) Salir");
        println!();

        let opcion = leer_linea("Opcion: ");
        println!();

        match opcion.as_str() {
            "1" => {
                let mut c = cfg.unwrap_or_default();
                if let Err(e) = elegir_ventana_y_region(&mut c) {
                    println!("{e:#}");
                } else if let Err(e) = c.save(ruta) {
                    println!("No se pudo guardar: {e:#}");
                } else {
                    println!("Guardado.");
                }
                pausa();
            }
            "2" => return Ok(Some(Accion::VerMarco)),
            "3" => {
                if idiomas.is_empty() {
                    println!("Windows no tiene ningun motor de OCR instalado.");
                } else {
                    println!("Idiomas de OCR disponibles:");
                    for i in &idiomas {
                        println!("  {i}");
                    }
                }
                println!(
                    "\nPara anadir japones: Configuracion > Hora e idioma > Idioma y region >\n\
                     Anadir idioma > 日本語 > Opciones > Reconocimiento optico de caracteres."
                );
                pausa();
            }
            "4" => {
                let mut c = cfg.unwrap_or_default();
                configurar_traductor(&mut c);
                if let Err(e) = c.save(ruta) {
                    println!("No se pudo guardar: {e:#}");
                } else {
                    println!("Guardado.");
                }
                pausa();
            }
            "5" => return Ok(Some(Accion::Traducir)),
            "6" => return Ok(Some(Accion::ProbarOcr)),
            "7" => {
                let mut c = cfg.unwrap_or_default();
                c.cursor.follow = !c.cursor.follow;
                if c.cursor.follow {
                    println!(
                        "Modo sigue-al-raton ACTIVADO.\n\n\
                         Se traduce la caja que estes senalando con el cursor: botones,\n\
                         opciones, menus.\n\n\
                         IMPORTANTE: marca ademas la region del cuadro de dialogo\n\
                         principal con la opcion 1. Ese cuadro suele ser una banda\n\
                         semitransparente sin borde, imposible de detectar por bordes,\n\
                         pero siempre esta en el mismo sitio. Cuando no haya caja bajo el\n\
                         raton se usara esa region.\n\n\
                         Con las dos cosas puestas tienes el dialogo siempre traducido y\n\
                         los botones cuando los senales."
                    );
                } else {
                    println!("Modo sigue-al-raton desactivado: se vuelve a la region marcada.");
                }
                if let Err(e) = c.save(ruta) {
                    println!("No se pudo guardar: {e:#}");
                }
                pausa();
            }
            "0" | "" => return Ok(None),
            _ => println!("No entiendo '{opcion}'.\n"),
        }
    }
}

/// Lo que el asistente pide hacer al volver a `main`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Accion {
    VerMarco,
    Traducir,
    ProbarOcr,
}

/// Elige la ventana del juego y marca la region dentro de ella.
///
/// La region se guarda **relativa** a la ventana, asi que a partir de ahi puedes
/// mover o redimensionar el juego sin volver a marcarla.
fn elegir_ventana_y_region(cfg: &mut Config) -> Result<()> {
    let ventanas = crate::target::listar()?;

    if ventanas.is_empty() {
        println!("No encuentro ninguna ventana. ¿Esta el juego abierto?");
        return Ok(());
    }

    println!("Ventanas abiertas:\n");
    for (i, v) in ventanas.iter().enumerate() {
        println!("  {:>2}) {}  [{}x{}]", i + 1, v.titulo, v.cliente.width, v.cliente.height);
    }
    println!("   0) Sin anclar (coordenadas de pantalla, como antes)\n");

    let eleccion = leer_linea("Cual es el juego: ");
    let indice: usize = eleccion.parse().unwrap_or(usize::MAX);

    let ventana = if indice == 0 {
        cfg.target.window_title.clear();
        None
    } else {
        match ventanas.get(indice.wrapping_sub(1)) {
            Some(v) => {
                cfg.target.window_title = titulo_estable(&v.titulo);
                println!("\nAnclado a: {}", cfg.target.window_title);
                Some(v)
            }
            None => {
                println!("No entiendo '{eleccion}'.");
                return Ok(());
            }
        }
    };

    println!();
    println!("Como quieres marcar el cuadro de dialogo:");
    println!("  1) Pinchando DENTRO de la caja (yo detecto los bordes)   <- recomendado");
    println!("  2) Marcando las dos esquinas a mano");
    println!();

    let absoluta = if leer_linea("Metodo [1]: ") == "2" {
        crate::picker::pick_region()?
    } else {
        detectar_por_clic(ventana.map(|v| v.cliente))?
    };

    cfg.region = match ventana {
        Some(v) => {
            let r = crate::target::a_relativa(absoluta, v.cliente);
            println!("Region relativa a la ventana: {},{} de {}x{}", r.x, r.y, r.width, r.height);
            r
        }
        None => absoluta,
    };

    Ok(())
}

/// Detecta la caja de dialogo a partir de un clic dentro de ella.
///
/// Se captura la zona (la ventana del juego, o la pantalla entera si no hay
/// anclaje), se localiza el punto dentro de esa captura y se buscan los bordes.
fn detectar_por_clic(cliente: Option<crate::config::Region>) -> Result<crate::config::Region> {
    let zona = match cliente {
        Some(c) => c,
        None => {
            let (x, y, w, h) = crate::capture::virtual_screen();
            crate::config::Region { x, y, width: w as u32, height: h as u32 }
        }
    };

    println!("Pon el juego delante con un dialogo visible.");
    let (px, py) = crate::picker::pick_point(
        "Pincha en medio del cuadro de texto, en una zona sin letras, y pulsa F8... ",
    )?;

    if px < zona.x || py < zona.y || px >= zona.right() || py >= zona.bottom() {
        anyhow::bail!("ese punto esta fuera de la ventana del juego");
    }

    let mut capturador = crate::capture::create()?;
    let frame = capturador.capture(zona)?;

    let relativo = ((px - zona.x) as u32, (py - zona.y) as u32);
    match frame.caja_desde_punto(relativo.0, relativo.1, 24) {
        Some(caja) => {
            let absoluta = crate::config::Region {
                x: zona.x + caja.x,
                y: zona.y + caja.y,
                width: caja.width,
                height: caja.height,
            };
            println!(
                "\nCaja detectada: {},{} de {}x{}",
                absoluta.x, absoluta.y, absoluta.width, absoluta.height
            );
            println!("Comprueba el encuadre con la opcion 2 del menu.");
            Ok(absoluta)
        }
        None => anyhow::bail!(
            "no he sabido encontrar los bordes desde ahi. Prueba a pinchar mas al centro \
             de la caja, o usa el metodo 2 (dos esquinas)."
        ),
    }
}

/// Recorta el titulo hasta su parte mas identificativa.
///
/// Los juegos meten en el titulo cosas que cambian solas —contadores de FPS,
/// numero de version, modo de pantalla—, asi que guardar el titulo entero haria
/// que dejase de encontrarse en cuanto cambiara un numero.
///
/// Se parte por los separadores habituales y se coge el trozo mas largo, no el
/// primero: hay juegos cuyo titulo empieza por `[ScreenFPS60]`, donde el primer
/// trozo esta vacio y el nombre viene despues.
pub fn titulo_estable(titulo: &str) -> String {
    let mejor = titulo
        .split(['[', ']', '(', ')', '-', '|'])
        .map(str::trim)
        .filter(|t| t.chars().count() >= 3)
        .max_by_key(|t| t.chars().count());

    match mejor {
        Some(t) => t.to_string(),
        None => titulo.trim().to_string(),
    }
}

fn configurar_traductor(cfg: &mut Config) {
    println!("Traductores disponibles:");
    println!("  1) DeepL      - el mejor con japones. 500.000 caracteres/mes gratis.");
    println!("  2) Google     - Cloud Translation, de pago.");
    println!("  3) Ollama     - gratis y en tu PC, pero mas lento.");
    println!("  4) Libre      - LibreTranslate en Docker, gratis y sin conexion.");
    println!("  5) Ninguno    - solo ensena lo que lee el OCR.");
    println!();

    match leer_linea("Cual: ").as_str() {
        "1" => cfg.translate.backend = Backend::Deepl,
        "2" => cfg.translate.backend = Backend::Google,
        "3" => cfg.translate.backend = Backend::Openai,
        "4" => cfg.translate.backend = Backend::Libre,
        "5" => cfg.translate.backend = Backend::None,
        _ => {
            println!("Sin cambios.");
            return;
        }
    }

    if matches!(cfg.translate.backend, Backend::Deepl | Backend::Google) {
        println!("\nPega la clave de API (se guarda en hakoyaku.toml).");
        let k = leer_linea("Clave: ");
        if !k.is_empty() {
            cfg.translate.api_key = k;
        }
    }

    let idioma = leer_linea("\nIdioma destino [es / en] (Enter para dejar el actual): ");
    if !idioma.is_empty() {
        cfg.translate.target_lang = idioma.to_lowercase();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Region;

    fn cfg_completa() -> Config {
        Config {
            region: Region { x: 10, y: 10, width: 800, height: 100 },
            translate: crate::config::Translate {
                backend: Backend::Deepl,
                api_key: "clave:fx".into(),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn sin_configuracion_solo_avisa_de_eso() {
        let d = diagnostico(None, &[]);
        assert_eq!(d.len(), 1);
        assert!(d[0].es_bloqueante());
        assert!(!listo(&d));
    }

    #[test]
    fn una_configuracion_completa_esta_lista() {
        let d = diagnostico(Some(&cfg_completa()), &["ja".to_string()]);
        assert!(listo(&d), "{d:?}");
        assert!(d.iter().all(|c| !c.es_bloqueante()));
    }

    #[test]
    fn el_titulo_se_recorta_quitando_version_y_adornos() {
        assert_eq!(titulo_estable("サキュバスプリズン-乳夢帰還-V1.00"), "サキュバスプリズン");
        assert_eq!(titulo_estable("Mi Juego (v1.2)"), "Mi Juego");
        assert_eq!(titulo_estable("Mi Juego"), "Mi Juego");
    }

    #[test]
    fn un_titulo_que_empieza_por_corchete_no_se_queda_en_vacio() {
        // Caso real: hay juegos cuyo titulo abre con el contador de FPS.
        assert_eq!(titulo_estable("[ScreenFPS60] Nombre Del Juego"), "Nombre Del Juego");
    }

    #[test]
    fn si_no_queda_ningun_trozo_util_se_guarda_el_titulo_entero() {
        assert_eq!(titulo_estable("a-b"), "a-b");
        assert_eq!(titulo_estable("  x  "), "x");
    }

    #[test]
    fn sin_anclaje_se_avisa() {
        let d = diagnostico(Some(&cfg_completa()), &["ja".to_string()]);
        assert!(d.iter().any(|c| matches!(c, Chequeo::Aviso(m) if m.contains("mueves el juego"))));
        assert!(listo(&d), "es un aviso, no un impedimento");
    }

    #[test]
    fn con_anclaje_se_informa_de_la_ventana() {
        let mut c = cfg_completa();
        c.target.window_title = "Prison".into();
        let d = diagnostico(Some(&c), &["ja".to_string()]);
        assert!(d.iter().any(|x| matches!(x, Chequeo::Bien(m) if m.contains("Prison"))));
    }

    #[test]
    fn una_caja_de_dialogo_normal_no_da_aviso() {
        assert!(!region_demasiado_grande(820, 145));
        assert!(!region_demasiado_grande(1578, 300));
    }

    #[test]
    fn la_ventana_entera_del_juego_si_da_aviso() {
        assert!(region_demasiado_grande(1021, 761));
        assert!(region_demasiado_grande(1920, 1080));
    }

    #[test]
    fn una_region_enorme_avisa_pero_no_bloquea() {
        let mut c = cfg_completa();
        c.region.width = 1021;
        c.region.height = 761;
        let d = diagnostico(Some(&c), &["ja".to_string()]);
        assert!(listo(&d), "es un aviso, no un impedimento");
        assert!(d
            .iter()
            .any(|x| matches!(x, Chequeo::Aviso(m) if m.contains("cuadro de dialogo"))));
    }

    #[test]
    fn la_region_sin_marcar_bloquea() {
        let mut c = cfg_completa();
        c.region.width = 0;
        let d = diagnostico(Some(&c), &["ja".to_string()]);
        assert!(!listo(&d));
        assert!(d.iter().any(|c| c.es_bloqueante() && c.mensaje().contains("region")));
    }

    #[test]
    fn con_sigue_al_raton_la_region_deja_de_bloquear() {
        let mut c = cfg_completa();
        c.region.width = 0;
        c.cursor.follow = true;
        let d = diagnostico(Some(&c), &["ja".to_string()]);
        assert!(listo(&d), "{d:?}");
        assert!(d.iter().any(|x| x.mensaje().contains("sigue-al-raton")));
    }

    #[test]
    fn falta_de_ocr_bloquea_y_dice_cuales_hay() {
        let d = diagnostico(Some(&cfg_completa()), &["en-US".to_string()]);
        assert!(!listo(&d));
        let m = d.iter().find(|c| c.es_bloqueante()).unwrap().mensaje().to_string();
        assert!(m.contains("en-US"), "deberia listar lo instalado: {m}");
    }

    #[test]
    fn ja_jp_cuenta_como_ja() {
        let d = diagnostico(Some(&cfg_completa()), &["ja-JP".to_string()]);
        assert!(listo(&d), "{d:?}");
    }

    #[test]
    fn deepl_sin_clave_bloquea() {
        let mut c = cfg_completa();
        c.translate.api_key = String::new();
        let d = diagnostico(Some(&c), &["ja".to_string()]);
        assert!(!listo(&d));
    }

    #[test]
    fn el_backend_none_avisa_pero_no_bloquea() {
        let mut c = cfg_completa();
        c.translate.backend = Backend::None;
        c.translate.api_key = String::new();
        let d = diagnostico(Some(&c), &["ja".to_string()]);
        assert!(listo(&d), "poder probar sin clave es justo el objetivo de 'none'");
        assert!(d.iter().any(|c| matches!(c, Chequeo::Aviso(_))));
    }

    #[test]
    fn ollama_no_pide_clave() {
        let mut c = cfg_completa();
        c.translate.backend = Backend::Openai;
        c.translate.api_key = String::new();
        let d = diagnostico(Some(&c), &["ja".to_string()]);
        assert!(listo(&d), "{d:?}");
    }

    #[test]
    fn los_simbolos_distinguen_los_tres_estados() {
        assert_eq!(Chequeo::Bien("x".into()).simbolo(), "[ok]");
        assert_eq!(Chequeo::Aviso("x".into()).simbolo(), "[ ?]");
        assert_eq!(Chequeo::Mal("x".into()).simbolo(), "[!!]");
    }
}
