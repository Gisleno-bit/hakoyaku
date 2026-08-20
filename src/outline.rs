//! Marco que dibuja el contorno de la region vigilada por encima del juego.
//!
//! Existe por una razon muy concreta: sin el, `hakoyaku run` no da ninguna
//! senal de estar mirando el sitio correcto. Con el ves de un vistazo si el
//! rectangulo encuadra la caja de dialogo o se te ha ido medio centimetro.
//!
//! Truco: la ventana se rellena entera de un color raro y se declara ese color
//! como transparente (`LWA_COLORKEY`). Solo queda visible lo que se pinta
//! encima con otro color, o sea el borde. El interior se ve como si no hubiera
//! ventana.

use crate::config::Region;
use anyhow::Result;

/// Un marco en pantalla. Al soltarlo, la ventana se cierra.
pub trait RegionMarker: Send {
    /// Cierra el marco.
    fn hide(&self) -> Result<()>;
}

/// Marco que no hace nada, para plataformas sin implementacion.
pub struct NoMarker;

impl RegionMarker for NoMarker {
    fn hide(&self) -> Result<()> {
        Ok(())
    }
}

pub fn create(
    region: Region,
    color: crate::overlay::Rgb,
    grosor: i32,
) -> Result<Box<dyn RegionMarker>> {
    #[cfg(windows)]
    {
        Ok(Box::new(win::OutlineWindow::spawn(region, color, grosor.clamp(1, 12))?))
    }
    #[cfg(not(windows))]
    {
        let _ = (region, color, grosor);
        Ok(Box::new(NoMarker))
    }
}

#[cfg(windows)]
mod win {
    use super::*;
    use anyhow::{anyhow, Context};
    use std::sync::mpsc;
    use std::sync::{Mutex, OnceLock};
    use windows::core::{w, PCWSTR};
    use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
    use windows::Win32::Graphics::Gdi::{
        BeginPaint, CreateSolidBrush, DeleteObject, EndPaint, FillRect, FrameRect, PAINTSTRUCT,
    };
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect,
        GetMessageW, PostQuitMessage, RegisterClassExW, SetLayeredWindowAttributes,
        SetWindowDisplayAffinity, SetWindowPos, ShowWindow, TranslateMessage, HTTRANSPARENT,
        HWND_TOPMOST, LWA_COLORKEY, MSG, SWP_NOACTIVATE, SW_SHOWNOACTIVATE, WDA_EXCLUDEFROMCAPTURE,
        WM_DESTROY, WM_NCHITTEST, WM_PAINT, WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE,
        WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP, WS_VISIBLE,
    };

    /// Color que se declara transparente. Un valor improbable a proposito: si
    /// coincidiera con el del borde, el borde tambien desapareceria.
    const CLAVE_TRANSPARENTE: u32 = 0x00_01_02_03 & 0x00FF_FFFF;

    struct Estilo {
        borde: COLORREF,
        grosor: i32,
    }

    static ESTILO: OnceLock<Mutex<Estilo>> = OnceLock::new();

    fn colorref(rgb: crate::overlay::Rgb) -> COLORREF {
        COLORREF(rgb.0 as u32 | ((rgb.1 as u32) << 8) | ((rgb.2 as u32) << 16))
    }

    pub struct OutlineWindow {
        hwnd: usize,
    }

    impl OutlineWindow {
        pub fn spawn(region: Region, color: crate::overlay::Rgb, grosor: i32) -> Result<Self> {
            let estilo = Estilo { borde: colorref(color), grosor };
            match ESTILO.get() {
                Some(m) => {
                    *m.lock().map_err(|_| anyhow!("estado del marco corrupto"))? = estilo;
                }
                None => {
                    ESTILO.set(Mutex::new(estilo)).map_err(|_| anyhow!("marco ya inicializado"))?;
                }
            }

            let (tx, rx) = mpsc::channel::<std::result::Result<usize, String>>();

            std::thread::Builder::new().name("hakoyaku-marco".into()).spawn(move || {
                match unsafe { crear(region) } {
                    Ok(hwnd) => {
                        let _ = tx.send(Ok(hwnd.0 as usize));
                        unsafe { bucle() };
                    }
                    Err(e) => {
                        let _ = tx.send(Err(e.to_string()));
                    }
                }
            })?;

            let hwnd = rx
                .recv_timeout(std::time::Duration::from_secs(10))
                .context("el hilo del marco no respondio")?
                .map_err(|e| anyhow!("no se pudo dibujar el marco de la region: {e}"))?;

            Ok(Self { hwnd })
        }
    }

    impl RegionMarker for OutlineWindow {
        fn hide(&self) -> Result<()> {
            unsafe {
                let _ = DestroyWindow(HWND(self.hwnd as *mut std::ffi::c_void));
            }
            Ok(())
        }
    }

    unsafe fn crear(region: Region) -> Result<HWND> {
        let instancia = GetModuleHandleW(PCWSTR::null()).context("GetModuleHandleW fallo")?;
        let clase = w!("HakoyakuOutlineClass");

        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wndproc),
            hInstance: instancia.into(),
            lpszClassName: clase,
            ..Default::default()
        };
        RegisterClassExW(&wc);

        // El marco se dibuja por fuera de la region para no tapar ni un pixel
        // del texto que hay que leer.
        let grosor = ESTILO.get().and_then(|m| m.lock().ok()).map(|e| e.grosor).unwrap_or(2);

        let hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            clase,
            w!("hakoyaku region"),
            WS_POPUP | WS_VISIBLE,
            region.x - grosor,
            region.y - grosor,
            region.width as i32 + grosor * 2,
            region.height as i32 + grosor * 2,
            None,
            None,
            instancia,
            None,
        )
        .context("CreateWindowExW fallo")?;

        // Todo lo que se pinte con CLAVE_TRANSPARENTE desaparece.
        SetLayeredWindowAttributes(hwnd, COLORREF(CLAVE_TRANSPARENTE), 255, LWA_COLORKEY)
            .context("SetLayeredWindowAttributes fallo")?;

        // El marco rodea la region vigilada: si la captura lo viera, sus bordes
        // formarian parte de la huella y el naranja acabaria en el OCR.
        let _ = SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE);

        let _ = SetWindowPos(
            hwnd,
            HWND_TOPMOST,
            region.x - grosor,
            region.y - grosor,
            region.width as i32 + grosor * 2,
            region.height as i32 + grosor * 2,
            SWP_NOACTIVATE,
        );
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);

        Ok(hwnd)
    }

    unsafe fn bucle() {
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }

    extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
        unsafe {
            match msg {
                WM_PAINT => {
                    pintar(hwnd);
                    LRESULT(0)
                }
                WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
                WM_DESTROY => {
                    PostQuitMessage(0);
                    LRESULT(0)
                }
                _ => DefWindowProcW(hwnd, msg, wp, lp),
            }
        }
    }

    unsafe fn pintar(hwnd: HWND) {
        let mut ps = PAINTSTRUCT::default();
        let hdc = BeginPaint(hwnd, &mut ps);
        if hdc.is_invalid() {
            return;
        }

        let (borde, grosor) = match ESTILO.get().and_then(|m| m.lock().ok()) {
            Some(e) => (e.borde, e.grosor),
            None => {
                let _ = EndPaint(hwnd, &ps);
                return;
            }
        };

        let mut cliente = RECT::default();
        let _ = GetClientRect(hwnd, &mut cliente);

        // Fondo entero con el color clave: se vuelve invisible.
        let hueco = CreateSolidBrush(COLORREF(CLAVE_TRANSPARENTE));
        FillRect(hdc, &cliente, hueco);
        let _ = DeleteObject(hueco);

        // El borde, capa a capa. FrameRect dibuja un pixel de grosor.
        let pincel = CreateSolidBrush(borde);
        for i in 0..grosor {
            let r = RECT {
                left: cliente.left + i,
                top: cliente.top + i,
                right: cliente.right - i,
                bottom: cliente.bottom - i,
            };
            if r.right <= r.left || r.bottom <= r.top {
                break;
            }
            FrameRect(hdc, &r, pincel);
        }
        let _ = DeleteObject(pincel);

        let _ = EndPaint(hwnd, &ps);
    }
}
