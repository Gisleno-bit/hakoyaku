//! Captura de una region de la pantalla.
//!
//! El trait es portable; la implementacion real usa GDI (`BitBlt`) y solo se
//! compila en Windows. El mock de abajo permite testear todo el pipeline en
//! cualquier sistema.

use crate::config::Region;
use crate::frame::Frame;
use anyhow::Result;

pub trait ScreenCapturer: Send {
    fn capture(&mut self, region: Region) -> Result<Frame>;
}

/// Limites del escritorio virtual (todos los monitores juntos), en pixeles
/// fisicos: (x, y, ancho, alto).
pub fn virtual_screen() -> (i32, i32, i32, i32) {
    #[cfg(windows)]
    {
        win::virtual_screen()
    }
    #[cfg(not(windows))]
    {
        (0, 0, 1920, 1080)
    }
}

/// Marca el proceso como consciente del escalado de pantalla.
///
/// Sin esto, en un monitor al 125% o al 150% (lo normal en Windows 11) las
/// coordenadas que marcas con el raton no coinciden con las que captura GDI, y
/// acabas capturando un trozo desplazado de la pantalla.
pub fn enable_dpi_awareness() {
    #[cfg(windows)]
    {
        win::enable_dpi_awareness();
    }
}

pub fn create() -> Result<Box<dyn ScreenCapturer>> {
    #[cfg(windows)]
    {
        Ok(Box::new(win::GdiCapturer::new()))
    }
    #[cfg(not(windows))]
    {
        anyhow::bail!(
            "la captura de pantalla solo esta implementada para Windows; \
             en otros sistemas puedes ejecutar los tests pero no `hakoyaku run`"
        )
    }
}

/// Capturador de mentira que va devolviendo una lista de frames preparados.
/// Repite el ultimo indefinidamente. Solo se usa en tests.
pub struct ScriptedCapturer {
    frames: Vec<Frame>,
    idx: usize,
    pub llamadas: usize,
}

impl ScriptedCapturer {
    pub fn new(frames: Vec<Frame>) -> Self {
        assert!(!frames.is_empty(), "ScriptedCapturer necesita al menos un frame");
        Self { frames, idx: 0, llamadas: 0 }
    }
}

impl ScreenCapturer for ScriptedCapturer {
    fn capture(&mut self, _region: Region) -> Result<Frame> {
        self.llamadas += 1;
        let f = self.frames[self.idx.min(self.frames.len() - 1)].clone();
        self.idx += 1;
        Ok(f)
    }
}

#[cfg(windows)]
mod win {
    use super::*;
    use anyhow::{bail, Context};
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Graphics::Gdi::{
        BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC,
        GetDIBits, ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, HDC,
        SRCCOPY,
    };
    use windows::Win32::UI::HiDpi::{
        SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
        SM_YVIRTUALSCREEN,
    };

    pub fn enable_dpi_awareness() {
        unsafe {
            let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        }
    }

    pub fn virtual_screen() -> (i32, i32, i32, i32) {
        unsafe {
            (
                GetSystemMetrics(SM_XVIRTUALSCREEN),
                GetSystemMetrics(SM_YVIRTUALSCREEN),
                GetSystemMetrics(SM_CXVIRTUALSCREEN),
                GetSystemMetrics(SM_CYVIRTUALSCREEN),
            )
        }
    }

    /// Reutiliza el buffer entre capturas para no pedir memoria 5 veces por
    /// segundo.
    pub struct GdiCapturer {
        buf: Vec<u8>,
    }

    impl Default for GdiCapturer {
        fn default() -> Self {
            Self::new()
        }
    }

    impl GdiCapturer {
        pub fn new() -> Self {
            Self { buf: Vec::new() }
        }
    }

    impl ScreenCapturer for GdiCapturer {
        fn capture(&mut self, region: Region) -> Result<Frame> {
            let w = region.width as i32;
            let h = region.height as i32;
            if w <= 0 || h <= 0 {
                bail!("region de captura vacia");
            }

            let bytes = w as usize * h as usize * 4;
            if self.buf.len() != bytes {
                self.buf.resize(bytes, 0);
            }

            unsafe {
                // HWND nulo = el escritorio entero, en coordenadas virtuales.
                let pantalla: HDC = GetDC(HWND::default());
                if pantalla.is_invalid() {
                    bail!("GetDC fallo: no se pudo abrir el contexto de la pantalla");
                }

                // A partir de aqui hay que liberar recursos en todos los caminos,
                // asi que el trabajo real va en una funcion aparte y aqui solo
                // se limpia.
                let resultado = capturar_en(pantalla, region, w, h, &mut self.buf);

                ReleaseDC(HWND::default(), pantalla);
                resultado?;
            }

            Frame::new(region.width, region.height, self.buf.clone())
        }
    }

    unsafe fn capturar_en(
        pantalla: HDC,
        region: Region,
        w: i32,
        h: i32,
        buf: &mut [u8],
    ) -> Result<()> {
        let mem_hdc = CreateCompatibleDC(pantalla);
        if mem_hdc.is_invalid() {
            bail!("CreateCompatibleDC fallo");
        }

        let bitmap = CreateCompatibleBitmap(pantalla, w, h);
        if bitmap.is_invalid() {
            let _ = DeleteDC(mem_hdc);
            bail!("CreateCompatibleBitmap fallo para {w}x{h}");
        }

        let anterior = SelectObject(mem_hdc, bitmap);

        let copia = BitBlt(mem_hdc, 0, 0, w, h, pantalla, region.x, region.y, SRCCOPY)
            .context("BitBlt fallo al copiar la region de pantalla");

        let leido = if copia.is_ok() {
            let mut info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: w,
                    // Negativo = filas de arriba abajo, que es como quiere `Frame`.
                    biHeight: -h,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: 0, // BI_RGB
                    biSizeImage: 0,
                    biXPelsPerMeter: 0,
                    biYPelsPerMeter: 0,
                    biClrUsed: 0,
                    biClrImportant: 0,
                },
                ..Default::default()
            };

            GetDIBits(
                mem_hdc,
                bitmap,
                0,
                h as u32,
                Some(buf.as_mut_ptr() as *mut std::ffi::c_void),
                &mut info,
                DIB_RGB_COLORS,
            )
        } else {
            0
        };

        SelectObject(mem_hdc, anterior);
        let _ = DeleteObject(bitmap);
        let _ = DeleteDC(mem_hdc);

        copia?;
        if leido == 0 {
            bail!("GetDIBits no devolvio ninguna fila");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn region() -> Region {
        Region { x: 0, y: 0, width: 2, height: 2 }
    }

    #[test]
    fn el_capturador_de_prueba_avanza_por_los_frames() {
        let a = Frame::blank(2, 2);
        let mut b = Frame::blank(2, 2);
        b.data[0] = 200;

        let mut c = ScriptedCapturer::new(vec![a.clone(), b.clone()]);
        assert_eq!(c.capture(region()).unwrap(), a);
        assert_eq!(c.capture(region()).unwrap(), b);
        // Al acabarse la lista repite el ultimo.
        assert_eq!(c.capture(region()).unwrap(), b);
        assert_eq!(c.llamadas, 3);
    }

    #[test]
    fn la_pantalla_virtual_tiene_tamano_positivo() {
        let (_, _, w, h) = virtual_screen();
        assert!(w > 0 && h > 0);
    }
}
