use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use hakoyaku::assistant::{self, Accion};
use hakoyaku::config::{Backend, Config};
use hakoyaku::{capture, hotkeys, ocr, outline, overlay, picker, pipeline, target, text};
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(
    name = "hakoyaku",
    version,
    about = "Traduce en tiempo real el texto de un recuadro de la pantalla y lo muestra al lado",
    long_about = None
)]
struct Cli {
    /// Fichero de configuracion.
    #[arg(short, long, default_value = "hakoyaku.toml", global = true)]
    config: PathBuf,

    /// Mas detalle en los mensajes (-v info, -vv debug).
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    verbose: u8,

    #[command(subcommand)]
    comando: Comando,
}

#[derive(Subcommand)]
enum Comando {
    /// Crea un hakoyaku.toml de ejemplo.
    Init {
        /// Sobrescribir si ya existe.
        #[arg(long)]
        force: bool,
    },
    /// Marca en pantalla la region a vigilar y la guarda en el fichero.
    Pick,
    /// Lista los idiomas de OCR que tiene instalados Windows.
    Langs,
    /// Captura la region una vez, la guarda como BMP y ensena lo que lee el OCR.
    Dump {
        /// Donde dejar la imagen.
        #[arg(long, default_value = "hakoyaku-dump.bmp")]
        out: PathBuf,
        /// Guardar tambien la imagen sin preprocesar.
        #[arg(long)]
        raw: bool,
    },
    /// Dibuja el marco de la region sobre la pantalla para comprobar que
    /// encuadra bien la caja de dialogo. Se cierra con Enter.
    Region,
    /// Arranca el traductor. Se para con Ctrl+C.
    Run {
        /// Escribir en la consola en vez de abrir el recuadro flotante.
        #[arg(long)]
        console: bool,
        /// Idioma destino para esta sesion, sin tocar el fichero. Ej: es, en.
        #[arg(long)]
        lang: Option<String>,
    },
}

fn main() {
    // Doble clic en el .exe = sin argumentos. Sin esto, clap imprime la ayuda,
    // el proceso termina y Windows cierra la consola de golpe: desde fuera
    // parece que el programa no arranca.
    let sin_argumentos = std::env::args().count() <= 1;

    let resultado = if sin_argumentos { modo_asistente() } else { modo_cli() };

    if let Err(e) = resultado {
        eprintln!("\nError: {e:#}");
        if sin_argumentos {
            esperar_enter();
        }
        std::process::exit(1);
    }

    if sin_argumentos {
        esperar_enter();
    }
}

fn esperar_enter() {
    use std::io::Write;
    print!("\nPulsa Enter para cerrar...");
    std::io::stdout().flush().ok();
    let mut b = String::new();
    std::io::stdin().read_line(&mut b).ok();
}

/// Menu interactivo. Diagnostica que falta y deja arreglarlo sin tocar el TOML.
fn modo_asistente() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn"))
        .format_timestamp(None)
        .init();
    capture::enable_dpi_awareness();

    let ruta = ruta_por_defecto();

    if !ruta.exists() {
        std::fs::write(&ruta, PLANTILLA)?;
        println!("He creado {} con los valores por defecto.\n", ruta.display());
    }

    match assistant::ejecutar(&ruta)? {
        Some(Accion::VerMarco) => region(&ruta),
        Some(Accion::Traducir) => run(&ruta, false, None),
        Some(Accion::ProbarOcr) => dump(&ruta, Path::new("hakoyaku-dump.bmp"), true),
        None => Ok(()),
    }
}

/// El fichero se busca junto al ejecutable, no en el directorio de trabajo:
/// al abrir con doble clic desde el escritorio, ese directorio puede ser
/// cualquier cosa.
fn ruta_por_defecto() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("hakoyaku.toml")))
        .unwrap_or_else(|| PathBuf::from("hakoyaku.toml"))
}

fn modo_cli() -> Result<()> {
    let cli = Cli::parse();

    let nivel = match cli.verbose {
        0 => "warn",
        1 => "info",
        _ => "debug",
    };
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(nivel))
        .format_timestamp(None)
        .init();

    match cli.comando {
        Comando::Init { force } => init(&cli.config, force),
        Comando::Pick => pick(&cli.config),
        Comando::Langs => langs(),
        Comando::Region => region(&cli.config),
        Comando::Dump { out, raw } => dump(&cli.config, &out, raw),
        Comando::Run { console, lang } => run(&cli.config, console, lang),
    }
}

fn cargar(ruta: &Path) -> Result<Config> {
    if !ruta.exists() {
        anyhow::bail!("no encuentro {}. Ejecuta `hakoyaku init` para crearlo.", ruta.display());
    }
    let mut cfg = Config::load(ruta)?;
    cfg.apply_env_overrides();
    Ok(cfg)
}

fn init(ruta: &Path, force: bool) -> Result<()> {
    if ruta.exists() && !force {
        anyhow::bail!("{} ya existe. Usa --force si quieres machacarlo.", ruta.display());
    }
    std::fs::write(ruta, PLANTILLA)
        .with_context(|| format!("no se pudo escribir {}", ruta.display()))?;

    println!("Creado {}.\n", ruta.display());
    println!("Siguientes pasos:");
    println!("  1. hakoyaku langs           (comprueba que tienes el OCR de japones)");
    println!("  2. hakoyaku pick            (marca el recuadro de dialogo del juego)");
    println!("  3. pon tu clave en [translate] o en la variable HAKOYAKU_API_KEY");
    println!("  4. hakoyaku dump            (comprueba que el OCR lee bien)");
    println!("  5. hakoyaku run");
    Ok(())
}

fn pick(ruta: &Path) -> Result<()> {
    let region = picker::pick_region()?;

    let mut cfg =
        if ruta.exists() { Config::load(ruta).unwrap_or_default() } else { Config::default() };
    cfg.region = region;
    cfg.save(ruta)?;

    println!("\nGuardado en {}.", ruta.display());
    println!("Comprueba que se lee bien con: hakoyaku dump");
    Ok(())
}

fn langs() -> Result<()> {
    let idiomas = ocr::available_languages()?;
    if idiomas.is_empty() {
        println!("Windows no tiene ningun motor de OCR instalado.");
    } else {
        println!("Idiomas de OCR disponibles:");
        for i in &idiomas {
            println!("  {i}");
        }
    }
    println!(
        "\nPara anadir uno: Configuracion > Hora e idioma > Idioma y region >\n\
         Anadir idioma > (elige) > Opciones > Reconocimiento optico de caracteres."
    );
    Ok(())
}

fn region(ruta: &Path) -> Result<()> {
    let cfg = cargar(ruta)?;
    capture::enable_dpi_awareness();

    let color = hakoyaku::config::parse_color(&cfg.overlay.region_color)?;
    let region = target::resolver(&cfg)?;
    let marco = outline::create(region, color, cfg.overlay.region_thickness)?;

    println!(
        "Marco dibujado en {},{} de {}x{}.\n\n\
         Mira la pantalla: el recuadro deberia encuadrar la caja de dialogo, un\n\
         poco holgado pero sin meter dentro relojes, barras de vida ni nada que\n\
         se mueva solo.\n\n\
         Pulsa Enter para cerrarlo.",
        region.x, region.y, region.width, region.height
    );

    let mut _basura = String::new();
    std::io::stdin().read_line(&mut _basura).ok();
    marco.hide()?;
    Ok(())
}

fn dump(ruta: &Path, salida: &Path, guardar_raw: bool) -> Result<()> {
    let cfg = cargar(ruta)?;
    capture::enable_dpi_awareness();

    let zona = if cfg.cursor.follow { zona_bajo_el_raton(&cfg)? } else { target::resolver(&cfg)? };

    let mut capturador = capture::create()?;
    let bruto = capturador.capture(zona)?;
    let preparado = pipeline::preprocess(&bruto, &cfg);

    std::fs::write(salida, preparado.to_bmp())
        .with_context(|| format!("no se pudo escribir {}", salida.display()))?;
    println!(
        "Imagen que ve el OCR -> {} ({}x{})",
        salida.display(),
        preparado.width,
        preparado.height
    );

    if guardar_raw {
        let raw = salida.with_extension("raw.bmp");
        std::fs::write(&raw, bruto.to_bmp())?;
        println!("Captura sin tocar     -> {}", raw.display());
    }

    let motor = ocr::create(&cfg.ocr.language)?;
    let lineas = motor.recognize(&preparado)?;

    println!("\nLineas en crudo ({}):", lineas.len());
    for l in &lineas {
        match l.rect {
            Some(r) => println!("  |{}|  caja {}x{} en {},{}", l.text, r.width, r.height, r.x, r.y),
            None => println!("  |{}|  (sin caja)", l.text),
        }
    }

    let limpio = text::clean_ocr_lines(&hakoyaku::ocr::textos(&lineas));
    println!("\nTexto limpio:\n  {limpio}");

    let vale = text::is_worth_translating(&limpio, cfg.ocr.min_chars, cfg.ocr.require_cjk);
    println!(
        "\n¿Se traduciria? {}",
        if vale { "si" } else { "no (se descarta por los filtros de [ocr])" }
    );

    if vale && cfg.translate.backend != Backend::None {
        let traductor = hakoyaku::translate::build(&cfg.translate)?;
        match traductor.translate(&limpio) {
            Ok(t) => println!("\n{} dice:\n  {t}", traductor.name()),
            Err(e) => println!("\nFallo al traducir: {e:#}"),
        }
    }

    Ok(())
}

/// Para el diagnostico en modo raton: pide un clic y devuelve la caja detectada.
fn zona_bajo_el_raton(cfg: &hakoyaku::config::Config) -> Result<hakoyaku::config::Region> {
    use hakoyaku::cursor;

    let limites = if cfg.anclado() {
        match hakoyaku::target::buscar(cfg.target.window_title.trim())? {
            Some(v) => v.cliente,
            None => anyhow::bail!("no encuentro la ventana '{}'", cfg.target.window_title.trim()),
        }
    } else {
        let (x, y, w, h) = capture::virtual_screen();
        hakoyaku::config::Region { x, y, width: w as u32, height: h as u32 }
    };

    let punto =
        picker::pick_point("Pon el raton sobre la caja que quieres traducir y pulsa F8... ")?;

    let busqueda =
        cursor::area_de_busqueda(punto, limites, cfg.cursor.search_width, cfg.cursor.search_height);

    let mut cap = capture::create()?;
    let frame = cap.capture(busqueda)?;
    let rel = ((punto.0 - busqueda.x) as u32, (punto.1 - busqueda.y) as u32);

    match cursor::detectar(&frame, rel, busqueda, cfg.cursor.edge_tolerance) {
        Some(caja) => {
            let abs = hakoyaku::config::Region {
                x: busqueda.x + caja.x,
                y: busqueda.y + caja.y,
                width: caja.width,
                height: caja.height,
            };
            println!(
                "Caja detectada bajo el raton: {},{} de {}x{}\n",
                abs.x, abs.y, abs.width, abs.height
            );
            Ok(abs)
        }
        None => anyhow::bail!(
            "no he encontrado ninguna caja desde ahi.\n\
             Prueba a subir cursor.edge_tolerance (ahora {}) o a apuntar mas al centro.",
            cfg.cursor.edge_tolerance
        ),
    }
}

fn atajo_o_guion(s: &str) -> String {
    if s.trim().is_empty() {
        "(desactivado)".into()
    } else {
        s.trim().to_string()
    }
}

fn run(ruta: &Path, consola: bool, lang: Option<String>) -> Result<()> {
    let mut cfg = cargar(ruta)?;
    if let Some(l) = lang {
        cfg.translate.target_lang = l;
        cfg.validate()?;
    }
    capture::enable_dpi_awareness();

    let capturador = capture::create()?;
    let motor = ocr::create(&cfg.ocr.language)?;
    let traductor = hakoyaku::translate::build(&cfg.translate)?;

    // El marco se guarda en una variable que vive hasta el final del proceso:
    // si se soltara aqui, la ventana se cerraria al instante.
    // El marco solo tiene sentido con region fija: en modo raton la caja cambia
    // en cada vuelta y un rectangulo quieto solo confundiria.
    let _marco = if cfg.overlay.show_region && !consola && !cfg.cursor.follow {
        let color = hakoyaku::config::parse_color(&cfg.overlay.region_color)?;
        Some(outline::create(target::resolver(&cfg)?, color, cfg.overlay.region_thickness)?)
    } else {
        None
    };

    let presentador: Box<dyn overlay::Presenter> = if consola {
        Box::new(overlay::ConsolePresenter)
    } else {
        let sitio =
            overlay::place(target::resolver(&cfg)?, &cfg.overlay, capture::virtual_screen());
        log::info!("overlay en {},{} de {}x{}", sitio.x, sitio.y, sitio.width, sitio.height);
        let p = overlay::create(sitio, &cfg.overlay)?;
        // Pintarlo ya con el texto de reposo: asi se ve que esta vivo antes
        // incluso de que aparezca la primera frase.
        p.clear()?;
        p
    };

    let que_mira = if cfg.cursor.follow {
        "sigue al raton (la caja que senales con el cursor)".to_string()
    } else {
        format!(
            "region {},{} de {}x{}",
            cfg.region.x, cfg.region.y, cfg.region.width, cfg.region.height
        )
    };

    println!(
        "hakoyaku en marcha.\n  mira:      {}\n  ocr:       {}\n  traductor: {}\n\n  {}  ocultar / mostrar la traduccion\n  {}  pausar\n  {}  releer ahora\n  {}  salir",
        que_mira,
        motor.language(),
        traductor.name(),
        atajo_o_guion(&cfg.hotkeys.toggle_overlay),
        atajo_o_guion(&cfg.hotkeys.pause),
        atajo_o_guion(&cfg.hotkeys.reread),
        atajo_o_guion(&cfg.hotkeys.quit)
    );

    let control = hotkeys::Control::nuevo();
    let atajos = hotkeys::Atajos::desde_config(&cfg.hotkeys);
    hotkeys::escuchar(std::sync::Arc::clone(&control), atajos);

    let mut p = pipeline::Pipeline::new(cfg, capturador, motor, traductor, presentador)
        .con_control(control);
    p.run()
}

const PLANTILLA: &str = include_str!("../hakoyaku.example.toml");
