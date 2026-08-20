//! Carga y validacion del fichero `hakoyaku.toml`.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub hotkeys: Hotkeys,
    pub target: Target,
    pub cursor: Cursor,
    pub region: Region,
    pub capture: Capture,
    pub ocr: Ocr,
    pub translate: Translate,
    pub overlay: Overlay,
}

/// Atajos de teclado globales, en texto: "ctrl+space", "alt+h", "f9"...
///
/// Van por configuracion y no fijos en el codigo porque cualquier tecla choca
/// con algun juego: unos usan las F para guardar partida, otros el espacio para
/// avanzar el dialogo. Dejar un campo vacio desactiva ese atajo.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct Hotkeys {
    /// Quitar y devolver la traduccion de la pantalla.
    pub toggle_overlay: String,
    /// Dejar de mirar (lo ultimo se queda en pantalla).
    pub pause: String,
    /// Releer ahora.
    pub reread: String,
    pub quit: String,
}

impl Default for Hotkeys {
    fn default() -> Self {
        Self {
            // Con modificador a proposito: una tecla suelta se la comeria el
            // juego, o se disparararia sola al escribir.
            toggle_overlay: "ctrl+space".into(),
            pause: "ctrl+alt+p".into(),
            reread: "ctrl+alt+r".into(),
            quit: "ctrl+shift+q".into(),
        }
    }
}

/// A que aplicacion nos anclamos.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct Target {
    /// Fragmento del titulo de la ventana del juego. Vacio = pantalla completa
    /// con coordenadas absolutas (el comportamiento de siempre).
    pub window_title: String,
    /// No traducir mientras el juego no tenga el foco. Evita que el programa se
    /// ponga a leer el navegador que tienes detras.
    pub only_when_focused: bool,
}

impl Default for Target {
    fn default() -> Self {
        Self { window_title: String::new(), only_when_focused: true }
    }
}

/// Modo "sigue al raton": en vez de una region fija, se traduce la caja que
/// haya bajo el cursor en cada momento.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct Cursor {
    pub follow: bool,
    /// Tamano de la zona donde se busca la caja alrededor del cursor. Mas
    /// grande = detecta cajas mas anchas, pero cuesta mas por vuelta.
    pub search_width: u32,
    pub search_height: u32,
    /// Cuanto tienen que solaparse dos detecciones seguidas (0-100) para
    /// considerarlas la misma caja y no mover el recuadro.
    pub stickiness: u32,
    /// Tolerancia de color al buscar los bordes de la caja.
    pub edge_tolerance: u8,
}

impl Default for Cursor {
    fn default() -> Self {
        Self {
            follow: false,
            search_width: 1100,
            search_height: 500,
            stickiness: 85,
            edge_tolerance: 18,
        }
    }
}

/// Rectangulo de pantalla que se vigila, en pixeles fisicos.
///
/// Si `[target] window_title` esta puesto, se interpreta **relativo al area de
/// cliente de esa ventana**; si no, en coordenadas absolutas de pantalla.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct Region {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl Region {
    pub fn right(&self) -> i32 {
        self.x + self.width as i32
    }
    pub fn bottom(&self) -> i32 {
        self.y + self.height as i32
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct Capture {
    /// Cada cuanto se mira la pantalla.
    pub poll_ms: u64,
    /// Cuantas lecturas seguidas identicas hacen falta antes de mandar al OCR.
    /// Con 2 o 3 se evita traducir el texto a medias mientras se escribe solo.
    pub stable_frames: u32,
    /// Multiplicador de tamano antes del OCR (1 = desactivado).
    pub upscale: u32,
    /// Tras mostrar una traduccion, no se vuelve a mirar la pantalla durante
    /// este tiempo. Evita el parpadeo y ahorra llamadas a la API.
    pub cooldown_ms: u64,
    /// Cuanto puede variar el brillo de un bloque (0-255) sin contar como
    /// cambio. Subelo si el juego tiene fondo animado.
    pub change_tolerance: u8,
    /// Cuantos bloques tienen que cambiar para considerar que hay pantalla
    /// nueva. Subelo si se dispara con animaciones; bajalo si no reacciona.
    pub change_blocks: usize,
    pub preprocess: Preprocess,
    pub binarize_threshold: u8,
}

impl Default for Capture {
    fn default() -> Self {
        Self {
            poll_ms: 90,
            stable_frames: 2,
            upscale: 2,
            cooldown_ms: 150,
            change_tolerance: 10,
            change_blocks: 4,
            preprocess: Preprocess::Auto,
            binarize_threshold: 128,
        }
    }
}

/// Que se le hace a la imagen antes de mandarla al OCR.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Preprocess {
    /// Tal cual sale de la pantalla.
    Off,
    /// Invierte los colores. El OCR acierta mas con texto oscuro sobre claro,
    /// y en los juegos casi siempre es al reves.
    Invert,
    /// Blanco y negro puro con umbral. Va muy bien con pixel-art y con cajas
    /// semitransparentes sobre fondos con textura; destroza las fuentes
    /// suavizadas.
    Binarize,
    /// Invierte solo si la region es oscura. Es la opcion segura.
    #[default]
    Auto,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct Ocr {
    /// Etiqueta BCP-47: "ja", "ja-JP", "zh-Hans", "ko", "en"...
    pub language: String,
    /// Numero minimo de caracteres visibles para molestarse en traducir.
    pub min_chars: usize,
    /// Ignorar lo que no lleve ningun caracter CJK (util con idioma origen ja).
    pub require_cjk: bool,
}

impl Default for Ocr {
    fn default() -> Self {
        // 2 y no 4: en los menus japoneses abundan opciones de dos caracteres
        // —はい (si), 見る (ver), 戻る (volver)— y con el minimo en 4 se
        // descartaban justo las que mas falta hacen.
        Self { language: "ja".into(), min_chars: 2, require_cjk: true }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Backend {
    #[default]
    Deepl,
    Google,
    /// LibreTranslate, propio o publico.
    Libre,
    /// Cualquier API compatible con OpenAI: Ollama, LM Studio, OpenAI...
    Openai,
    /// No traduce: devuelve el original. Para probar la captura y el OCR.
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct Translate {
    pub backend: Backend,
    pub source_lang: String,
    pub target_lang: String,
    pub api_key: String,
    /// Solo para `libre` y `openai`. Vacio = valor por defecto del backend.
    pub endpoint: String,
    /// Solo para `openai`.
    pub model: String,
    /// Frases distintas que se recuerdan para no pagar dos veces por la misma.
    pub cache_size: usize,
    pub timeout_secs: u64,
}

impl Default for Translate {
    fn default() -> Self {
        Self {
            backend: Backend::Deepl,
            source_lang: "ja".into(),
            target_lang: "es".into(),
            api_key: String::new(),
            endpoint: String::new(),
            model: "qwen2.5:7b".into(),
            cache_size: 500,
            timeout_secs: 12,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Position {
    /// A la derecha si cabe; si no, encima; si no, debajo.
    #[default]
    Auto,
    Right,
    Left,
    Above,
    Below,
    /// Usa `overlay.x` y `overlay.y` tal cual.
    Custom,
}

/// Como se ensena la traduccion.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// Encima del texto original, tapandolo. Es lo que hace que parezca que el
    /// juego esta traducido.
    #[default]
    Inplace,
    /// En un recuadro aparte, al lado de la region.
    Panel,
}

/// Que se tapa en modo in-place.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Cover {
    /// La caja de dialogo entera. Queda como si el juego estuviera traducido, y
    /// deja sitio de sobra para la traduccion.
    #[default]
    Box,
    /// Solo el rectangulo que ocupaban las letras.
    Text,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct Overlay {
    pub mode: Mode,
    pub inplace_cover: Cover,
    /// Pixeles que se anaden por cada lado al parche que tapa el original.
    pub inplace_padding: i32,
    /// "auto" toma el color de la propia caja del juego; tambien vale #RRGGBB.
    pub inplace_background: String,
    /// Por debajo de este tamano de letra no se encoge mas, aunque no quepa.
    pub min_font_size: i32,
    pub position: Position,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub margin: i32,
    pub font: String,
    pub font_size: i32,
    /// 0 = invisible, 255 = opaco.
    pub opacity: u8,
    pub text_color: String,
    pub background_color: String,
    /// Mostrar tambien el texto japones original encima de la traduccion.
    pub show_original: bool,
    /// Dibujar un marco alrededor de la region vigilada. Es la unica forma de
    /// ver de un vistazo si el programa esta mirando donde debe.
    pub show_region: bool,
    pub region_color: String,
    pub region_thickness: i32,
    /// Lo que pone el recuadro cuando no hay nada que traducir. Sirve de senal
    /// de vida: si esto se ve, el programa esta corriendo.
    pub idle_text: String,
}

impl Default for Overlay {
    fn default() -> Self {
        Self {
            mode: Mode::Inplace,
            inplace_cover: Cover::Box,
            inplace_padding: 4,
            inplace_background: "auto".into(),
            min_font_size: 9,
            position: Position::Auto,
            x: 0,
            y: 0,
            width: 520,
            height: 220,
            margin: 12,
            font: "Yu Gothic UI".into(),
            font_size: 20,
            opacity: 235,
            text_color: "#F2F2F2".into(),
            background_color: "#12121A".into(),
            show_original: false,
            show_region: true,
            region_color: "#FF8A3D".into(),
            region_thickness: 2,
            idle_text: "hakoyaku · esperando texto…".into(),
        }
    }
}

/// Convierte `#RRGGBB` (o `RRGGBB`) en la tripleta (r, g, b).
pub fn parse_color(s: &str) -> Result<crate::overlay::Rgb> {
    let h = s.trim().trim_start_matches('#');
    if h.len() != 6 || !h.chars().all(|c| c.is_ascii_hexdigit()) {
        bail!("color invalido '{s}': se esperaba formato #RRGGBB");
    }
    Ok((
        u8::from_str_radix(&h[0..2], 16)?,
        u8::from_str_radix(&h[2..4], 16)?,
        u8::from_str_radix(&h[4..6], 16)?,
    ))
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("no se pudo leer {}", path.display()))?;
        let cfg: Config = toml::from_str(&raw)
            .with_context(|| format!("{} no es un TOML valido para hakoyaku", path.display()))?;
        cfg.validate()?;
        Ok(cfg)
    }

    /// Carga sin validar.
    ///
    /// El asistente necesita esto: su trabajo es precisamente explicar que
    /// falta, y no puede hacerlo si `load` rechaza el fichero por lo mismo que
    /// tiene que diagnosticar.
    pub fn load_lenient(path: &Path) -> Result<Self> {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("no se pudo leer {}", path.display()))?;
        toml::from_str(&raw)
            .with_context(|| format!("{} no es un TOML valido para hakoyaku", path.display()))
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let raw = toml::to_string_pretty(self)?;
        std::fs::write(path, raw)
            .with_context(|| format!("no se pudo escribir {}", path.display()))?;
        Ok(())
    }

    /// `true` si la region va anclada a una ventana concreta.
    pub fn anclado(&self) -> bool {
        !self.target.window_title.trim().is_empty()
    }

    pub fn validate(&self) -> Result<()> {
        if self.cursor.stickiness > 100 {
            bail!("cursor.stickiness = {} debe estar entre 0 y 100", self.cursor.stickiness);
        }
        // Con seguimiento del raton no hace falta region marcada.
        if self.cursor.follow {
            return self.validar_resto();
        }
        if self.region.width == 0 || self.region.height == 0 {
            bail!(
                "la region a capturar esta vacia ({}x{}). Ejecuta `hakoyaku pick` para marcarla.",
                self.region.width,
                self.region.height
            );
        }
        if self.capture.poll_ms < 30 {
            bail!("capture.poll_ms = {} es demasiado agresivo; usa 30 o mas", self.capture.poll_ms);
        }
        self.validar_resto()
    }

    fn validar_resto(&self) -> Result<()> {
        if self.capture.change_blocks == 0 {
            bail!("capture.change_blocks = 0 haria saltar la lectura con cualquier pixel");
        }
        if self.capture.upscale > 6 {
            bail!(
                "capture.upscale = {} disparara el uso de RAM; el maximo util es 6",
                self.capture.upscale
            );
        }
        if self.ocr.language.trim().is_empty() {
            bail!("ocr.language no puede estar vacio");
        }
        if self.overlay.width == 0 || self.overlay.height == 0 {
            bail!("el overlay tiene tamano cero");
        }
        for (nombre, valor) in [
            ("toggle_overlay", &self.hotkeys.toggle_overlay),
            ("pause", &self.hotkeys.pause),
            ("reread", &self.hotkeys.reread),
            ("quit", &self.hotkeys.quit),
        ] {
            if !valor.trim().is_empty() && crate::hotkeys::parsear(valor).is_none() {
                bail!(
                    "hotkeys.{nombre} = \"{valor}\" no se entiende. Ejemplos: \
                     \"ctrl+space\", \"alt+h\", \"f9\", \"ctrl+shift+q\". \
                     Dejalo vacio para desactivarlo."
                );
            }
        }

        parse_color(&self.overlay.text_color)?;
        parse_color(&self.overlay.background_color)?;
        parse_color(&self.overlay.region_color)?;
        let fondo = self.overlay.inplace_background.trim();
        if !fondo.eq_ignore_ascii_case("auto") {
            parse_color(fondo)?;
        }
        if self.overlay.min_font_size < 6 {
            bail!("overlay.min_font_size = {} es ilegible", self.overlay.min_font_size);
        }
        if !(1..=12).contains(&self.overlay.region_thickness) {
            bail!(
                "overlay.region_thickness = {} fuera de rango; usa entre 1 y 12",
                self.overlay.region_thickness
            );
        }

        match self.translate.backend {
            Backend::Deepl | Backend::Google if self.translate.api_key.trim().is_empty() => {
                bail!(
                    "el backend {:?} necesita translate.api_key (o la variable de entorno HAKOYAKU_API_KEY)",
                    self.translate.backend
                );
            }
            _ => {}
        }
        Ok(())
    }

    /// Permite sobrescribir la clave desde el entorno para no dejarla en el
    /// fichero (que es lo que acabaras subiendo a GitHub por accidente).
    pub fn apply_env_overrides(&mut self) {
        if let Ok(k) = std::env::var("HAKOYAKU_API_KEY") {
            if !k.trim().is_empty() {
                self.translate.api_key = k;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config_minima() -> Config {
        Config {
            region: Region { x: 10, y: 20, width: 800, height: 120 },
            translate: Translate { backend: Backend::None, ..Default::default() },
            ..Default::default()
        }
    }

    #[test]
    fn un_toml_vacio_da_los_valores_por_defecto() {
        let c: Config = toml::from_str("").unwrap();
        assert_eq!(c.capture.poll_ms, 90);
        assert_eq!(c.capture.stable_frames, 2);
        assert_eq!(c.capture.cooldown_ms, 150);
        assert_eq!(c.capture.change_tolerance, 10);
        assert_eq!(c.capture.change_blocks, 4);
        assert_eq!(c.ocr.language, "ja");
        assert_eq!(c.translate.target_lang, "es");
        assert_eq!(c.overlay.position, Position::Auto);
    }

    #[test]
    fn parsea_un_toml_completo() {
        let raw = r#"
            [region]
            x = 100
            y = 430
            width = 780
            height = 100

            [capture]
            poll_ms = 250
            stable_frames = 3
            upscale = 3
            preprocess = "binarize"

            [ocr]
            language = "ja-JP"
            min_chars = 6

            [translate]
            backend = "openai"
            target_lang = "en"
            model = "gemma3:12b"

            [overlay]
            position = "below"
            show_original = true
        "#;
        let c: Config = toml::from_str(raw).unwrap();
        assert_eq!(c.region, Region { x: 100, y: 430, width: 780, height: 100 });
        assert_eq!(c.capture.poll_ms, 250);
        assert_eq!(c.capture.preprocess, Preprocess::Binarize);
        assert_eq!(c.capture.binarize_threshold, 128); // no puesto -> defecto
        assert_eq!(c.ocr.language, "ja-JP");
        assert_eq!(c.ocr.min_chars, 6);
        assert_eq!(c.translate.backend, Backend::Openai);
        assert_eq!(c.translate.model, "gemma3:12b");
        assert_eq!(c.overlay.position, Position::Below);
        assert!(c.overlay.show_original);
    }

    #[test]
    fn una_clave_desconocida_es_un_error_y_no_un_silencio() {
        let raw = "[captura]\npoll_ms = 100\n";
        assert!(toml::from_str::<Config>(raw).is_err());
    }

    #[test]
    fn ida_y_vuelta_por_toml() {
        let c = config_minima();
        let raw = toml::to_string_pretty(&c).unwrap();
        assert_eq!(toml::from_str::<Config>(&raw).unwrap(), c);
    }

    #[test]
    fn la_carga_indulgente_acepta_lo_que_validate_rechaza() {
        let dir = std::env::temp_dir().join(format!("hakoyaku-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("sin-clave.toml");
        std::fs::write(
            &f,
            "[region]\nwidth = 100\nheight = 50\n\n[translate]\nbackend = \"deepl\"\n",
        )
        .unwrap();

        // load() se queja de la clave que falta; load_lenient() no.
        assert!(Config::load(&f).is_err());
        let c = Config::load_lenient(&f).unwrap();
        assert_eq!(c.region.width, 100);
        assert!(c.validate().is_err(), "pero el problema sigue ahi para diagnosticarlo");

        std::fs::remove_file(&f).ok();
    }

    #[test]
    fn la_region_vacia_no_valida() {
        let mut c = config_minima();
        c.region.width = 0;
        assert!(c.validate().is_err());
    }

    #[test]
    fn deepl_sin_clave_no_valida() {
        let mut c = config_minima();
        c.translate.backend = Backend::Deepl;
        c.translate.api_key = "  ".into();
        assert!(c.validate().is_err());

        c.translate.api_key = "abc:fx".into();
        assert!(c.validate().is_ok());
    }

    #[test]
    fn los_backends_locales_no_piden_clave() {
        let mut c = config_minima();
        for b in [Backend::Libre, Backend::Openai, Backend::None] {
            c.translate.backend = b;
            c.translate.api_key = String::new();
            assert!(c.validate().is_ok(), "{b:?} deberia validar sin clave");
        }
    }

    #[test]
    fn rechaza_polling_absurdo() {
        let mut c = config_minima();
        c.capture.poll_ms = 5;
        assert!(c.validate().is_err());
    }

    #[test]
    fn colores_validos_e_invalidos() {
        assert_eq!(parse_color("#FF8000").unwrap(), (255, 128, 0));
        assert_eq!(parse_color("ff8000").unwrap(), (255, 128, 0));
        assert!(parse_color("#FFF").is_err());
        assert!(parse_color("azul").is_err());
        assert!(parse_color("#GGGGGG").is_err());
    }

    /// El fichero de ejemplo es el que copia `hakoyaku init`, y `deny_unknown_fields`
    /// hace que una sola errata ahi rompa el arranque. Mejor que salte aqui.
    #[test]
    fn los_atajos_por_defecto_evitan_las_teclas_de_funcion() {
        let h = Hotkeys::default();
        for a in [&h.toggle_overlay, &h.pause, &h.reread, &h.quit] {
            assert!(crate::hotkeys::parsear(a).is_some(), "'{a}' deberia parsear");
            assert!(a.contains('+'), "'{a}' deberia llevar modificador");
        }
    }

    #[test]
    fn un_atajo_sin_sentido_no_valida() {
        let mut c = config_minima();
        c.hotkeys.pause = "ctrl+banana".into();
        let e = c.validate().unwrap_err().to_string();
        assert!(e.contains("hotkeys.pause"), "{e}");
    }

    #[test]
    fn un_atajo_vacio_esta_permitido_y_lo_desactiva() {
        let mut c = config_minima();
        c.hotkeys.reread = String::new();
        assert!(c.validate().is_ok());
    }

    #[test]
    fn con_seguimiento_del_raton_no_hace_falta_region() {
        let mut c = config_minima();
        c.region = Region::default();
        assert!(c.validate().is_err(), "sin region y sin seguimiento no vale");

        c.cursor.follow = true;
        assert!(c.validate().is_ok(), "con seguimiento la region sobra");
    }

    #[test]
    fn una_adherencia_imposible_no_valida() {
        let mut c = config_minima();
        c.cursor.stickiness = 150;
        assert!(c.validate().is_err());
    }

    #[test]
    fn sin_titulo_de_ventana_no_hay_anclaje() {
        let mut c = config_minima();
        assert!(!c.anclado());
        c.target.window_title = "   ".into();
        assert!(!c.anclado());
        c.target.window_title = "Prison".into();
        assert!(c.anclado());
    }

    #[test]
    fn el_toml_de_ejemplo_parsea_y_valida() {
        let raw = include_str!("../hakoyaku.example.toml");
        let c: Config = toml::from_str(raw).expect("hakoyaku.example.toml no parsea");
        assert!(c.region.width > 0 && c.region.height > 0);
        // El ejemplo trae backend deepl sin clave, asi que validate() debe
        // quejarse justamente de eso y de nada mas.
        let e = c.validate().unwrap_err().to_string();
        assert!(e.contains("api_key"), "error inesperado: {e}");
    }

    #[test]
    fn el_modo_por_defecto_es_encima_del_original() {
        assert_eq!(Overlay::default().mode, Mode::Inplace);
        assert_eq!(Overlay::default().inplace_background, "auto");
        assert_eq!(Overlay::default().inplace_cover, Cover::Box);
    }

    #[test]
    fn inplace_background_acepta_auto_o_un_color() {
        let mut c = config_minima();
        c.overlay.inplace_background = "auto".into();
        assert!(c.validate().is_ok());
        c.overlay.inplace_background = "AUTO".into();
        assert!(c.validate().is_ok());
        c.overlay.inplace_background = "#101820".into();
        assert!(c.validate().is_ok());
        c.overlay.inplace_background = "azulito".into();
        assert!(c.validate().is_err());
    }

    #[test]
    fn una_letra_minima_ilegible_no_valida() {
        let mut c = config_minima();
        c.overlay.min_font_size = 3;
        assert!(c.validate().is_err());
    }

    #[test]
    fn el_marco_de_region_viene_activado_por_defecto() {
        let c = Overlay::default();
        assert!(c.show_region, "sin marco no hay forma de saber si funciona");
        assert_eq!(c.region_thickness, 2);
        assert!(!c.idle_text.is_empty(), "el texto de reposo es la senal de vida");
    }

    #[test]
    fn un_grosor_de_marco_absurdo_no_valida() {
        let mut c = config_minima();
        c.overlay.region_thickness = 0;
        assert!(c.validate().is_err());
        c.overlay.region_thickness = 40;
        assert!(c.validate().is_err());
        c.overlay.region_thickness = 3;
        assert!(c.validate().is_ok());
    }

    #[test]
    fn un_color_de_marco_invalido_no_valida() {
        let mut c = config_minima();
        c.overlay.region_color = "naranja".into();
        assert!(c.validate().is_err());
    }

    #[test]
    fn los_bordes_de_la_region_se_calculan_bien() {
        let r = Region { x: 100, y: 50, width: 300, height: 80 };
        assert_eq!(r.right(), 400);
        assert_eq!(r.bottom(), 130);
    }
}
