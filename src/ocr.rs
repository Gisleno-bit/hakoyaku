//! Reconocimiento de texto.
//!
//! La implementacion real usa `Windows.Media.Ocr`, el motor que ya viene dentro
//! de Windows 10 y 11. Ventajas frente a Tesseract: no hay que instalar nada
//! aparte, funciona sin conexion, tarda unos pocos milisegundos y con japones
//! acierta bastante mas.
//!
//! Requisito: tener instalado el paquete de idioma correspondiente en
//! Configuracion > Hora e idioma > Idioma y region.

use crate::config::Region;
use crate::frame::Frame;
use anyhow::Result;

/// Caja de una linea de texto, en coordenadas del frame que se paso al OCR
/// (es decir, ya con el escalado aplicado).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl TextRect {
    pub fn right(&self) -> i32 {
        self.x + self.width
    }
    pub fn bottom(&self) -> i32 {
        self.y + self.height
    }

    /// Caja que engloba a otras dos.
    pub fn merge(&self, otra: &TextRect) -> TextRect {
        let x = self.x.min(otra.x);
        let y = self.y.min(otra.y);
        TextRect {
            x,
            y,
            width: self.right().max(otra.right()) - x,
            height: self.bottom().max(otra.bottom()) - y,
        }
    }

    /// Caja que engloba a todas las lineas. `None` si no hay ninguna con caja.
    pub fn envolvente(lineas: &[TextLine]) -> Option<TextRect> {
        lineas.iter().filter_map(|l| l.rect).reduce(|a, b| a.merge(&b))
    }

    /// Traduce la caja a coordenadas de pantalla: deshace el escalado del
    /// preprocesado y suma el origen de la region vigilada.
    pub fn a_pantalla(&self, upscale: u32, region: Region, margen: i32) -> Region {
        let f = upscale.max(1) as i32;
        let x = region.x + self.x / f - margen;
        let y = region.y + self.y / f - margen;
        Region {
            x,
            y,
            width: (self.width / f + margen * 2).max(1) as u32,
            height: (self.height / f + margen * 2).max(1) as u32,
        }
    }
}

/// Una linea reconocida: su texto y donde estaba.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextLine {
    pub text: String,
    /// `None` si el motor no da coordenadas.
    pub rect: Option<TextRect>,
}

impl TextLine {
    pub fn sin_caja(text: impl Into<String>) -> Self {
        Self { text: text.into(), rect: None }
    }
}

/// Solo los textos, para la limpieza que hace el modulo `text`.
pub fn textos(lineas: &[TextLine]) -> Vec<String> {
    lineas.iter().map(|l| l.text.clone()).collect()
}

pub trait TextRecognizer: Send {
    /// Devuelve las lineas detectadas, en crudo y sin limpiar.
    fn recognize(&self, frame: &Frame) -> Result<Vec<TextLine>>;
    fn language(&self) -> &str;
}

pub fn create(language: &str) -> Result<Box<dyn TextRecognizer>> {
    #[cfg(windows)]
    {
        Ok(Box::new(win::WindowsOcr::new(language)?))
    }
    #[cfg(not(windows))]
    {
        let _ = language;
        anyhow::bail!(
            "el OCR nativo solo existe en Windows; en otros sistemas puedes \
             ejecutar los tests pero no `hakoyaku run`"
        )
    }
}

/// Idiomas para los que Windows tiene motor de OCR instalado.
pub fn available_languages() -> Result<Vec<String>> {
    #[cfg(windows)]
    {
        win::available_languages()
    }
    #[cfg(not(windows))]
    {
        Ok(Vec::new())
    }
}

/// OCR de mentira: devuelve siempre las mismas lineas. Solo para tests.
pub struct FixedOcr {
    pub lines: Vec<TextLine>,
    pub lang: String,
}

impl FixedOcr {
    pub fn new(lines: &[&str]) -> Self {
        Self { lines: apilar(lines), lang: "ja".into() }
    }
}

/// Convierte textos sueltos en lineas con cajas apiladas, como las devolveria
/// un OCR de verdad. Solo para mocks y tests.
pub fn apilar(lines: &[&str]) -> Vec<TextLine> {
    lines
        .iter()
        .enumerate()
        .map(|(i, t)| TextLine {
            text: t.to_string(),
            rect: Some(TextRect {
                x: 10,
                y: 10 + i as i32 * 30,
                width: 20 * t.chars().count() as i32,
                height: 26,
            }),
        })
        .collect()
}

impl TextRecognizer for FixedOcr {
    fn recognize(&self, _frame: &Frame) -> Result<Vec<TextLine>> {
        Ok(self.lines.clone())
    }
    fn language(&self) -> &str {
        &self.lang
    }
}

/// OCR de mentira que devuelve un texto distinto segun la huella del frame.
/// Sirve para comprobar que el pipeline reacciona a los cambios de pantalla.
pub struct HashKeyedOcr {
    pub tabla: Vec<(u64, Vec<TextLine>)>,
}

impl TextRecognizer for HashKeyedOcr {
    fn recognize(&self, frame: &Frame) -> Result<Vec<TextLine>> {
        let h = frame.fingerprint();
        Ok(self.tabla.iter().find(|(k, _)| *k == h).map(|(_, v)| v.clone()).unwrap_or_default())
    }
    fn language(&self) -> &str {
        "ja"
    }
}

#[cfg(windows)]
mod win {
    use super::*;
    use anyhow::{bail, Context};
    use windows::core::{Interface, HSTRING};
    use windows::Globalization::Language;
    use windows::Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap};
    use windows::Media::Ocr::OcrEngine;
    use windows::Storage::Streams::Buffer;
    use windows::Win32::System::WinRT::{IBufferByteAccess, RoInitialize, RO_INIT_MULTITHREADED};

    /// WinRT hay que inicializarlo una vez por hilo. Llamarlo de mas es
    /// inofensivo, asi que no complicamos con `Once`.
    fn init_winrt() {
        unsafe {
            let _ = RoInitialize(RO_INIT_MULTITHREADED);
        }
    }

    pub fn available_languages() -> Result<Vec<String>> {
        init_winrt();
        let langs = OcrEngine::AvailableRecognizerLanguages()
            .context("no se pudo consultar los idiomas de OCR instalados")?;
        let mut out = Vec::new();
        for l in langs {
            if let Ok(tag) = l.LanguageTag() {
                out.push(tag.to_string());
            }
        }
        Ok(out)
    }

    pub struct WindowsOcr {
        engine: OcrEngine,
        lang: String,
        max_dim: u32,
    }

    impl WindowsOcr {
        pub fn new(tag: &str) -> Result<Self> {
            init_winrt();

            let disponibles = available_languages().unwrap_or_default();
            let hay_coincidencia = disponibles.iter().any(|d| {
                d.eq_ignore_ascii_case(tag)
                    || d.to_lowercase().starts_with(&format!("{}-", tag.to_lowercase()))
                    || tag.to_lowercase().starts_with(&format!("{}-", d.to_lowercase()))
            });

            if !hay_coincidencia {
                bail!(
                    "Windows no tiene motor de OCR para '{tag}'.\n\
                     Instalados ahora mismo: {}\n\
                     Para anadirlo: Configuracion > Hora e idioma > Idioma y region > \
                     Anadir idioma > (elige el idioma) > Opciones > Reconocimiento optico \
                     de caracteres.",
                    if disponibles.is_empty() {
                        "ninguno".to_string()
                    } else {
                        disponibles.join(", ")
                    }
                );
            }

            let language = Language::CreateLanguage(&HSTRING::from(tag))
                .with_context(|| format!("'{tag}' no es una etiqueta de idioma valida"))?;
            let engine = OcrEngine::TryCreateFromLanguage(&language)
                .with_context(|| format!("no se pudo crear el motor de OCR para '{tag}'"))?;
            let max_dim = OcrEngine::MaxImageDimension().unwrap_or(10_000);

            Ok(Self { engine, lang: tag.to_string(), max_dim })
        }
    }

    impl TextRecognizer for WindowsOcr {
        fn recognize(&self, frame: &Frame) -> Result<Vec<TextLine>> {
            if frame.width > self.max_dim || frame.height > self.max_dim {
                bail!(
                    "la imagen ({}x{}) supera el maximo del motor de OCR ({}). \
                     Baja capture.upscale o reduce la region.",
                    frame.width,
                    frame.height,
                    self.max_dim
                );
            }

            let bitmap = software_bitmap_desde(frame)?;
            let resultado = self
                .engine
                .RecognizeAsync(&bitmap)
                .context("RecognizeAsync fallo")?
                .get()
                .context("el OCR no devolvio resultado")?;

            let mut lineas = Vec::new();
            for linea in resultado.Lines()? {
                let t = linea.Text()?.to_string();
                if t.trim().is_empty() {
                    continue;
                }
                // La caja de la linea es la union de las de sus palabras. Es lo
                // que permite tapar el texto original y pintar encima.
                let mut caja: Option<TextRect> = None;
                if let Ok(palabras) = linea.Words() {
                    for palabra in palabras {
                        if let Ok(r) = palabra.BoundingRect() {
                            let actual = TextRect {
                                x: r.X as i32,
                                y: r.Y as i32,
                                width: r.Width as i32,
                                height: r.Height as i32,
                            };
                            caja = Some(match caja {
                                Some(c) => c.merge(&actual),
                                None => actual,
                            });
                        }
                    }
                }
                lineas.push(TextLine { text: t, rect: caja });
            }
            Ok(lineas)
        }

        fn language(&self) -> &str {
            &self.lang
        }
    }

    /// Copia el buffer BGRA a un `SoftwareBitmap` de WinRT.
    fn software_bitmap_desde(frame: &Frame) -> Result<SoftwareBitmap> {
        let len = frame.data.len();
        let buffer = Buffer::Create(len as u32).context("no se pudo reservar el buffer WinRT")?;
        buffer.SetLength(len as u32)?;

        let acceso: IBufferByteAccess = buffer.cast()?;
        unsafe {
            let destino = acceso.Buffer()?;
            std::ptr::copy_nonoverlapping(frame.data.as_ptr(), destino, len);
        }

        SoftwareBitmap::CreateCopyFromBuffer(
            &buffer,
            BitmapPixelFormat::Bgra8,
            frame.width as i32,
            frame.height as i32,
        )
        .context("no se pudo construir el SoftwareBitmap")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn el_ocr_fijo_devuelve_lo_que_le_das() {
        let o = FixedOcr::new(&["積 雪 の 様 に", "白 い 絨 毯"]);
        let lineas = o.recognize(&Frame::blank(4, 4)).unwrap();
        assert_eq!(lineas.len(), 2);
        assert_eq!(lineas[0].text, "積 雪 の 様 に");
        assert!(lineas[0].rect.is_some());
        assert_eq!(o.language(), "ja");
    }

    #[test]
    fn dos_cajas_se_funden_en_la_que_las_engloba() {
        let a = TextRect { x: 10, y: 10, width: 100, height: 20 };
        let b = TextRect { x: 50, y: 40, width: 100, height: 20 };
        assert_eq!(a.merge(&b), TextRect { x: 10, y: 10, width: 140, height: 50 });
        assert_eq!(a.merge(&b), b.merge(&a), "fundir debe ser conmutativo");
    }

    #[test]
    fn la_envolvente_cubre_todas_las_lineas() {
        let lineas = apilar(&["ab", "cd", "ef"]);
        let e = TextRect::envolvente(&lineas).unwrap();
        assert_eq!(e.x, 10);
        assert_eq!(e.y, 10);
        assert_eq!(e.bottom(), 96); // 10 + 2*30 + 26
    }

    #[test]
    fn sin_cajas_no_hay_envolvente() {
        let lineas = vec![TextLine::sin_caja("hola"), TextLine::sin_caja("adios")];
        assert!(TextRect::envolvente(&lineas).is_none());
    }

    #[test]
    fn la_caja_vuelve_a_coordenadas_de_pantalla() {
        // El OCR trabajo sobre una imagen escalada x2 de una region en 100,400.
        let caja = TextRect { x: 40, y: 20, width: 400, height: 60 };
        let region = Region { x: 100, y: 400, width: 500, height: 100 };
        let r = caja.a_pantalla(2, region, 0);
        assert_eq!(r, Region { x: 120, y: 410, width: 200, height: 30 });
    }

    #[test]
    fn el_margen_agranda_la_caja_por_los_cuatro_lados() {
        let caja = TextRect { x: 0, y: 0, width: 100, height: 40 };
        let region = Region { x: 0, y: 0, width: 200, height: 100 };
        let r = caja.a_pantalla(1, region, 5);
        assert_eq!(r, Region { x: -5, y: -5, width: 110, height: 50 });
    }

    #[test]
    fn el_ocr_por_hash_distingue_frames() {
        let a = Frame::blank(2, 2);
        let mut b = Frame::blank(2, 2);
        b.data[0] = 99;

        let o = HashKeyedOcr {
            tabla: vec![
                (a.fingerprint(), apilar(&["primero"])),
                (b.fingerprint(), apilar(&["segundo"])),
            ],
        };
        assert_eq!(o.recognize(&a).unwrap()[0].text, "primero");
        assert_eq!(o.recognize(&b).unwrap()[0].text, "segundo");

        let mut c = Frame::blank(2, 2);
        c.data[4] = 1;
        assert!(o.recognize(&c).unwrap().is_empty());
    }
}
