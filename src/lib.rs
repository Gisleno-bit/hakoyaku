//! hakoyaku — traduce en tiempo real el texto que aparece en un recuadro de la
//! pantalla y lo muestra al lado.
//!
//! La logica esta separada en dos capas a proposito:
//!
//! - Lo portable (`text`, `frame`, `cache`, `config`, `pipeline`, y la
//!   colocacion del overlay) no depende de Windows y se testea en cualquier
//!   sistema, incluido el CI de Linux.
//! - Lo especifico de Windows (captura por GDI, `Windows.Media.Ocr`, la ventana
//!   layered) vive detras de traits y de `#[cfg(windows)]`.
//!
//! Gracias a eso el test de integracion puede montar un pipeline entero con
//! mocks y comprobar el comportamiento sin abrir una sola ventana.

pub mod assistant;
pub mod cache;
pub mod capture;
pub mod config;
pub mod cursor;
pub mod frame;
pub mod hotkeys;
pub mod ocr;
pub mod outline;
pub mod overlay;
pub mod picker;
pub mod pipeline;
pub mod target;
pub mod text;
pub mod translate;
