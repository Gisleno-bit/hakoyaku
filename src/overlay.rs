//! El recuadro con la traduccion.
//!
//! En Windows es una ventana `WS_EX_LAYERED | WS_EX_TRANSPARENT`: siempre
//! encima, semitransparente y sin recibir clics, asi que el raton la atraviesa
//! y el juego no se entera de que existe.
//!
//! El calculo de donde ponerla (`place`) esta fuera de la parte Win32 a
//! proposito: es la logica con mas casos raros y asi se puede testear.

use crate::config::{Overlay as OverlayCfg, Position, Region};
use anyhow::Result;

/// Un color, en el orden natural (rojo, verde, azul).
///
/// Existe para que las firmas no se llenen de `(u8, u8, u8)` anonimos, que no
/// dicen si el primer canal es el rojo o el azul.
pub type Rgb = (u8, u8, u8);

pub trait Presenter: Send {
    /// Muestra el par original/traduccion. `original` puede ir vacio.
    fn show(&self, original: &str, translation: &str) -> Result<()>;
    /// Oculta o vacia el recuadro.
    fn clear(&self) -> Result<()>;
    /// Modo in-place: recoloca el recuadro justo encima del texto original y le
    /// da el color de fondo de la caja del juego. En modo panel no hace nada.
    fn place_over(&self, _rect: Region, _background: Option<Rgb>) -> Result<()> {
        Ok(())
    }
}

/// Decide el rectangulo del overlay a partir de la region vigilada.
///
/// `screen` es (x, y, ancho, alto) del escritorio virtual.
pub fn place(region: Region, cfg: &OverlayCfg, screen: (i32, i32, i32, i32)) -> Region {
    let (sx, sy, sw, sh) = screen;
    let w = cfg.width.min(sw.max(1) as u32);
    let h = cfg.height.min(sh.max(1) as u32);
    let m = cfg.margin;

    let candidato = |p: Position| -> Region {
        let (x, y) = match p {
            Position::Right => (region.right() + m, region.y),
            Position::Left => (region.x - m - w as i32, region.y),
            Position::Above => (region.x, region.y - m - h as i32),
            Position::Below => (region.x, region.bottom() + m),
            Position::Custom | Position::Auto => (cfg.x, cfg.y),
        };
        Region { x, y, width: w, height: h }
    };

    let cabe = |r: &Region| r.x >= sx && r.y >= sy && r.right() <= sx + sw && r.bottom() <= sy + sh;

    let elegido = match cfg.position {
        Position::Custom => candidato(Position::Custom),
        Position::Auto => [Position::Right, Position::Above, Position::Below, Position::Left]
            .into_iter()
            .map(candidato)
            .find(cabe)
            .unwrap_or_else(|| candidato(Position::Right)),
        otra => candidato(otra),
    };

    // Aunque no quepa del todo, que al menos quede dentro de la pantalla.
    Region {
        x: elegido.x.clamp(sx, (sx + sw - w as i32).max(sx)),
        y: elegido.y.clamp(sy, (sy + sh - h as i32).max(sy)),
        width: w,
        height: h,
    }
}

/// Salida por consola: sirve para depurar y para usar el programa sin ventana.
pub struct ConsolePresenter;

impl Presenter for ConsolePresenter {
    fn show(&self, original: &str, translation: &str) -> Result<()> {
        if original.is_empty() {
            println!("\n{translation}");
        } else {
            println!("\n[{original}]\n{translation}");
        }
        Ok(())
    }
    fn clear(&self) -> Result<()> {
        Ok(())
    }
}

/// Presenter que solo guarda lo ultimo mostrado. Para tests.
#[derive(Default)]
pub struct RecordingPresenter {
    pub eventos: std::sync::Mutex<Vec<(String, String)>>,
}

impl RecordingPresenter {
    pub fn eventos(&self) -> Vec<(String, String)> {
        self.eventos.lock().unwrap().clone()
    }
}

impl Presenter for &RecordingPresenter {
    fn show(&self, original: &str, translation: &str) -> Result<()> {
        self.eventos.lock().unwrap().push((original.to_string(), translation.to_string()));
        Ok(())
    }
    fn clear(&self) -> Result<()> {
        self.eventos.lock().unwrap().push((String::new(), String::new()));
        Ok(())
    }
}

pub fn create(rect: Region, cfg: &OverlayCfg) -> Result<Box<dyn Presenter>> {
    #[cfg(windows)]
    {
        Ok(Box::new(win::WindowPresenter::spawn(rect, cfg)?))
    }
    #[cfg(not(windows))]
    {
        let _ = (rect, cfg);
        Ok(Box::new(ConsolePresenter))
    }
}

#[cfg(windows)]
mod win {
    use super::*;
    use crate::config::parse_color;
    use anyhow::{anyhow, Context};
    use std::sync::mpsc;
    use std::sync::{Mutex, OnceLock};
    use windows::core::{w, PCWSTR};
    use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, TRUE, WPARAM};
    use windows::Win32::Graphics::Gdi::HDC;
    use windows::Win32::Graphics::Gdi::{
        BeginPaint, CreateFontW, CreateSolidBrush, DeleteObject, DrawTextW, EndPaint, FillRect,
        InvalidateRect, SelectObject, SetBkMode, SetTextColor, DT_CALCRECT, DT_NOPREFIX, DT_TOP,
        DT_WORDBREAK, PAINTSTRUCT, TRANSPARENT,
    };
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DispatchMessageW, GetClientRect, GetMessageW, LoadCursorW,
        MoveWindow, PostMessageW, PostQuitMessage, RegisterClassExW, SetLayeredWindowAttributes,
        SetWindowDisplayAffinity, SetWindowPos, ShowWindow, TranslateMessage, CS_HREDRAW,
        CS_VREDRAW, HTTRANSPARENT, HWND_TOPMOST, IDC_ARROW, LWA_ALPHA, MSG, SWP_NOACTIVATE,
        SW_HIDE, SW_SHOWNOACTIVATE, WDA_EXCLUDEFROMCAPTURE, WM_APP, WM_DESTROY, WM_NCHITTEST,
        WM_PAINT, WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
        WS_EX_TRANSPARENT, WS_POPUP, WS_VISIBLE,
    };

    const WM_REDIBUJA: u32 = WM_APP + 1;
    const PADDING: i32 = 14;
    const SEPARACION: i32 = 8;

    struct Contenido {
        original: String,
        traduccion: String,
        /// Color con el que tapar el original. `None` = el del tema.
        fondo: Option<COLORREF>,
    }

    struct Estilo {
        fuente: Vec<u16>,
        tamano: i32,
        color_texto: COLORREF,
        color_original: COLORREF,
        color_fondo: COLORREF,
        mostrar_original: bool,
        reposo: Vec<u16>,
        /// En modo in-place el recuadro se pega al texto y no ensena el
        /// mensaje de reposo: seria un cartel flotando encima del juego.
        inplace: bool,
        minimo: i32,
    }

    // Solo hay un overlay en todo el proceso, asi que un estatico es mas simple
    // (y mas seguro) que pasear punteros por GWLP_USERDATA.
    static CONTENIDO: OnceLock<Mutex<Contenido>> = OnceLock::new();
    static ESTILO: OnceLock<Estilo> = OnceLock::new();

    /// COLORREF es 0x00BBGGRR, al reves de lo que uno escribiria.
    fn colorref(rgb: Rgb) -> COLORREF {
        COLORREF(rgb.0 as u32 | ((rgb.1 as u32) << 8) | ((rgb.2 as u32) << 16))
    }

    fn atenuar(rgb: Rgb) -> Rgb {
        (
            (rgb.0 as u32 * 55 / 100) as u8,
            (rgb.1 as u32 * 55 / 100) as u8,
            (rgb.2 as u32 * 55 / 100) as u8,
        )
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    pub struct WindowPresenter {
        /// El HWND no es `Send`, pero `PostMessageW` si es seguro entre hilos,
        /// asi que guardamos el puntero como entero y lo reconstruimos al usarlo.
        hwnd: usize,
    }

    impl WindowPresenter {
        pub fn spawn(rect: Region, cfg: &OverlayCfg) -> Result<Self> {
            let texto = parse_color(&cfg.text_color)?;
            let fondo = parse_color(&cfg.background_color)?;

            ESTILO
                .set(Estilo {
                    fuente: wide(&cfg.font),
                    tamano: cfg.font_size.max(8),
                    color_texto: colorref(texto),
                    color_original: colorref(atenuar(texto)),
                    color_fondo: colorref(fondo),
                    mostrar_original: cfg.show_original,
                    reposo: cfg.idle_text.encode_utf16().collect(),
                    inplace: cfg.mode == crate::config::Mode::Inplace,
                    minimo: cfg.min_font_size.max(6),
                })
                .map_err(|_| anyhow!("el overlay ya estaba creado"))?;

            CONTENIDO
                .set(Mutex::new(Contenido {
                    original: String::new(),
                    traduccion: String::new(),
                    fondo: None,
                }))
                .map_err(|_| anyhow!("el overlay ya estaba creado"))?;

            let opacidad = cfg.opacity;
            let (tx, rx) = mpsc::channel::<Result<usize, String>>();

            std::thread::Builder::new().name("hakoyaku-overlay".into()).spawn(move || {
                let creada = unsafe { crear_ventana(rect, opacidad) };
                match creada {
                    Ok(hwnd) => {
                        let _ = tx.send(Ok(hwnd.0 as usize));
                        unsafe { bucle_mensajes() };
                    }
                    Err(e) => {
                        let _ = tx.send(Err(e.to_string()));
                    }
                }
            })?;

            let hwnd = rx
                .recv_timeout(std::time::Duration::from_secs(10))
                .context("el hilo del overlay no respondio")?
                .map_err(|e| anyhow!("no se pudo crear la ventana del overlay: {e}"))?;

            Ok(Self { hwnd })
        }

        fn repintar(&self) {
            unsafe {
                let _ = PostMessageW(
                    HWND(self.hwnd as *mut std::ffi::c_void),
                    WM_REDIBUJA,
                    WPARAM(0),
                    LPARAM(0),
                );
            }
        }
    }

    impl Presenter for WindowPresenter {
        fn show(&self, original: &str, translation: &str) -> Result<()> {
            if let Some(c) = CONTENIDO.get() {
                let mut g = c.lock().map_err(|_| anyhow!("estado del overlay corrupto"))?;
                g.original = original.to_string();
                g.traduccion = translation.to_string();
            }
            self.repintar();
            Ok(())
        }

        fn clear(&self) -> Result<()> {
            let inplace = ESTILO.get().map(|e| e.inplace).unwrap_or(false);
            if inplace {
                // Sin texto no hay nada que tapar: se esconde entera para no
                // dejar un rectangulo de color encima del juego.
                unsafe {
                    let _ = ShowWindow(HWND(self.hwnd as *mut std::ffi::c_void), SW_HIDE);
                }
                return Ok(());
            }
            self.show("", "")
        }

        fn place_over(&self, rect: Region, background: Option<Rgb>) -> Result<()> {
            if let Some(c) = CONTENIDO.get() {
                let mut g = c.lock().map_err(|_| anyhow!("estado del overlay corrupto"))?;
                g.fondo = background.map(colorref);
            }
            unsafe {
                let hwnd = HWND(self.hwnd as *mut std::ffi::c_void);
                let _ =
                    MoveWindow(hwnd, rect.x, rect.y, rect.width as i32, rect.height as i32, TRUE);
                let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            }
            Ok(())
        }
    }

    unsafe fn crear_ventana(rect: Region, opacidad: u8) -> Result<HWND> {
        let instancia = GetModuleHandleW(PCWSTR::null()).context("GetModuleHandleW fallo")?;
        let clase = w!("HakoyakuOverlayClass");

        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wndproc),
            hInstance: instancia.into(),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            lpszClassName: clase,
            ..Default::default()
        };

        // Devuelve 0 si falla, pero tambien si la clase ya estaba registrada de
        // una ejecucion anterior en el mismo proceso; no lo tratamos como error.
        RegisterClassExW(&wc);

        let hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            clase,
            w!("hakoyaku"),
            WS_POPUP | WS_VISIBLE,
            rect.x,
            rect.y,
            rect.width as i32,
            rect.height as i32,
            None,
            None,
            instancia,
            None,
        )
        .context("CreateWindowExW fallo")?;

        SetLayeredWindowAttributes(hwnd, COLORREF(0), opacidad, LWA_ALPHA)
            .context("SetLayeredWindowAttributes fallo")?;

        // LA LINEA MAS IMPORTANTE DEL FICHERO.
        //
        // El recuadro se dibuja justo encima de la region que vigilamos, asi
        // que sin esto la siguiente captura recoge nuestro propio texto en
        // castellano: cambia la huella, se dispara el OCR, se descarta por no
        // llevar japones, se oculta el recuadro, reaparece el japones de
        // debajo... y vuelta a empezar. Un bucle infinito con parpadeo.
        //
        // WDA_EXCLUDEFROMCAPTURE hace que la ventana sea invisible para
        // cualquier API de captura de pantalla, incluida la nuestra. El usuario
        // la ve; BitBlt no.
        if SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE).is_err() {
            log::warn!(
                "esta version de Windows no permite excluir el overlay de la captura; \
                 si ves parpadeo, sube capture.cooldown_ms"
            );
        }

        let _ = SetWindowPos(
            hwnd,
            HWND_TOPMOST,
            rect.x,
            rect.y,
            rect.width as i32,
            rect.height as i32,
            SWP_NOACTIVATE,
        );
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);

        Ok(hwnd)
    }

    unsafe fn bucle_mensajes() {
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }

    extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
        unsafe {
            match msg {
                WM_REDIBUJA => {
                    let _ = InvalidateRect(hwnd, None, TRUE);
                    LRESULT(0)
                }
                WM_PAINT => {
                    pintar(hwnd);
                    LRESULT(0)
                }
                // Que el raton atraviese la ventana aunque el estilo fallase.
                WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
                WM_DESTROY => {
                    PostQuitMessage(0);
                    LRESULT(0)
                }
                _ => DefWindowProcW(hwnd, msg, wp, lp),
            }
        }
    }

    /// Mayor tamano de letra con el que `texto` cabe dentro de `hueco`.
    ///
    /// Se mide con `DT_CALCRECT`, que calcula sin pintar nada, y se baja de dos
    /// en dos puntos. Son pocas vueltas y solo ocurre cuando cambia la frase.
    unsafe fn ajustar_tamano(hdc: HDC, texto: &str, hueco: &RECT, estilo: &Estilo) -> i32 {
        let ancho = hueco.right - hueco.left;
        let alto = hueco.bottom - hueco.top;
        if ancho <= 0 || alto <= 0 {
            return estilo.minimo;
        }

        let mut buf: Vec<u16> = texto.encode_utf16().collect();
        let mut tamano = estilo.tamano;

        while tamano > estilo.minimo {
            let fuente = CreateFontW(
                -tamano,
                0,
                0,
                0,
                400,
                0,
                0,
                0,
                1,
                0,
                0,
                5,
                0,
                PCWSTR(estilo.fuente.as_ptr()),
            );
            let anterior = SelectObject(hdc, fuente);

            let mut medida = RECT { left: 0, top: 0, right: ancho, bottom: 0 };
            let necesario =
                DrawTextW(hdc, &mut buf, &mut medida, DT_CALCRECT | DT_WORDBREAK | DT_NOPREFIX);

            SelectObject(hdc, anterior);
            let _ = DeleteObject(fuente);

            if necesario <= alto {
                return tamano;
            }
            tamano -= 2;
        }

        estilo.minimo
    }

    unsafe fn pintar(hwnd: HWND) {
        let mut ps = PAINTSTRUCT::default();
        let hdc = BeginPaint(hwnd, &mut ps);
        if hdc.is_invalid() {
            return;
        }

        let estilo = match ESTILO.get() {
            Some(e) => e,
            None => {
                let _ = EndPaint(hwnd, &ps);
                return;
            }
        };

        let mut cliente = RECT::default();
        let _ = GetClientRect(hwnd, &mut cliente);

        let (original, traduccion, fondo) = match CONTENIDO.get().and_then(|c| c.lock().ok()) {
            Some(g) => (g.original.clone(), g.traduccion.clone(), g.fondo),
            None => (String::new(), String::new(), None),
        };

        // En modo in-place el parche va del color de la caja del juego, para que
        // no se note el remiendo. Si no hay muestra, el color del tema.
        let brocha = CreateSolidBrush(fondo.unwrap_or(estilo.color_fondo));
        FillRect(hdc, &cliente, brocha);
        let _ = DeleteObject(brocha);

        let mut area = RECT {
            left: cliente.left + PADDING,
            top: cliente.top + PADDING,
            right: cliente.right - PADDING,
            bottom: cliente.bottom - PADDING,
        };

        // Buscar el mayor tamano de letra con el que la traduccion quepa entera
        // en el hueco. Sin esto, una frase larga en castellano (que ocupa mas
        // que el japones original) se saldria del parche.
        let tamano = if estilo.inplace && !traduccion.is_empty() {
            ajustar_tamano(hdc, &traduccion, &area, estilo)
        } else {
            estilo.tamano
        };

        let fuente = CreateFontW(
            -tamano, // negativo = altura de caracter, no de celda
            0,
            0,
            0,
            400, // FW_NORMAL
            0,   // cursiva
            0,   // subrayado
            0,   // tachado
            1,   // DEFAULT_CHARSET
            0,   // OUT_DEFAULT_PRECIS
            0,   // CLIP_DEFAULT_PRECIS
            5,   // CLEARTYPE_QUALITY
            0,   // DEFAULT_PITCH | FF_DONTCARE
            PCWSTR(estilo.fuente.as_ptr()),
        );
        let fuente_anterior = SelectObject(hdc, fuente);
        SetBkMode(hdc, TRANSPARENT);

        if estilo.mostrar_original && !original.is_empty() {
            let mut buf = original.encode_utf16().collect::<Vec<u16>>();
            let mut medida = area;
            let alto =
                DrawTextW(hdc, &mut buf, &mut medida, DT_CALCRECT | DT_WORDBREAK | DT_NOPREFIX);

            SetTextColor(hdc, estilo.color_original);
            let mut destino = RECT { bottom: area.top + alto, ..area };
            DrawTextW(hdc, &mut buf, &mut destino, DT_WORDBREAK | DT_NOPREFIX | DT_TOP);

            area.top += alto + SEPARACION;
        }

        if traduccion.is_empty() && estilo.inplace {
            // En modo in-place, vacio es vacio: la ventana ya esta oculta.
        } else if traduccion.is_empty() {
            // Sin esto el recuadro se queda en negro y no hay forma de
            // distinguir "esperando" de "se ha colgado".
            let mut buf = estilo.reposo.clone();
            SetTextColor(hdc, estilo.color_original);
            DrawTextW(hdc, &mut buf, &mut area, DT_WORDBREAK | DT_NOPREFIX | DT_TOP);
        } else {
            let mut buf = traduccion.encode_utf16().collect::<Vec<u16>>();
            SetTextColor(hdc, estilo.color_texto);
            DrawTextW(hdc, &mut buf, &mut area, DT_WORDBREAK | DT_NOPREFIX | DT_TOP);
        }

        SelectObject(hdc, fuente_anterior);
        let _ = DeleteObject(fuente);
        let _ = EndPaint(hwnd, &ps);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PANTALLA: (i32, i32, i32, i32) = (0, 0, 1920, 1080);

    fn cfg() -> OverlayCfg {
        OverlayCfg { width: 400, height: 200, margin: 10, ..Default::default() }
    }

    fn caja_dialogo() -> Region {
        // Un cuadro de dialogo tipico: abajo y ancho.
        Region { x: 100, y: 800, width: 1000, height: 150 }
    }

    #[test]
    fn a_la_derecha_pega_el_overlay_al_borde_de_la_region() {
        let mut c = cfg();
        c.position = Position::Right;
        let r = place(caja_dialogo(), &c, PANTALLA);
        assert_eq!(r.x, 1110); // 100 + 1000 + 10
        assert_eq!(r.y, 800);
        assert_eq!((r.width, r.height), (400, 200));
    }

    #[test]
    fn encima_resta_margen_y_altura() {
        let mut c = cfg();
        c.position = Position::Above;
        let r = place(caja_dialogo(), &c, PANTALLA);
        assert_eq!(r.x, 100);
        assert_eq!(r.y, 590); // 800 - 10 - 200
    }

    #[test]
    fn debajo_parte_del_final_de_la_region() {
        let mut c = cfg();
        c.position = Position::Below;
        let r = place(Region { x: 100, y: 100, width: 400, height: 100 }, &c, PANTALLA);
        assert_eq!(r.y, 210); // 100 + 100 + 10
    }

    #[test]
    fn a_la_izquierda_resta_margen_y_ancho() {
        let mut c = cfg();
        c.position = Position::Left;
        let r = place(Region { x: 600, y: 100, width: 400, height: 100 }, &c, PANTALLA);
        assert_eq!(r.x, 190); // 600 - 10 - 400
    }

    #[test]
    fn custom_usa_las_coordenadas_del_fichero() {
        let mut c = cfg();
        c.position = Position::Custom;
        c.x = 42;
        c.y = 77;
        let r = place(caja_dialogo(), &c, PANTALLA);
        assert_eq!((r.x, r.y), (42, 77));
    }

    #[test]
    fn auto_prefiere_la_derecha_cuando_cabe() {
        let mut c = cfg();
        c.position = Position::Auto;
        let region = Region { x: 100, y: 400, width: 500, height: 150 };
        let r = place(region, &c, PANTALLA);
        assert_eq!(r.x, 610);
        assert_eq!(r.y, 400);
    }

    #[test]
    fn auto_sube_arriba_si_no_hay_sitio_a_la_derecha() {
        let mut c = cfg();
        c.position = Position::Auto;
        // Caja ancha pegada al borde derecho: a la derecha no cabe.
        let region = Region { x: 100, y: 800, width: 1750, height: 150 };
        let r = place(region, &c, PANTALLA);
        assert_eq!(r.x, 100);
        assert_eq!(r.y, 590);
    }

    #[test]
    fn auto_baja_si_arriba_tampoco_cabe() {
        let mut c = cfg();
        c.position = Position::Auto;
        // Caja ancha y pegada arriba: ni derecha ni arriba.
        let region = Region { x: 100, y: 0, width: 1750, height: 150 };
        let r = place(region, &c, PANTALLA);
        assert_eq!(r.y, 160); // 0 + 150 + 10
    }

    #[test]
    fn nunca_se_sale_de_la_pantalla() {
        let mut c = cfg();
        c.position = Position::Right;
        // Region pegada al borde derecho: forzamos que se salga y se recorte.
        let region = Region { x: 1800, y: 1000, width: 100, height: 60 };
        let r = place(region, &c, PANTALLA);
        assert!(r.x >= 0 && r.y >= 0, "{r:?}");
        assert!(r.right() <= 1920, "se sale por la derecha: {r:?}");
        assert!(r.bottom() <= 1080, "se sale por abajo: {r:?}");
    }

    #[test]
    fn un_overlay_mas_grande_que_la_pantalla_se_recorta() {
        let mut c = cfg();
        c.width = 4000;
        c.height = 3000;
        let r = place(caja_dialogo(), &c, PANTALLA);
        assert_eq!((r.width, r.height), (1920, 1080));
        assert_eq!((r.x, r.y), (0, 0));
    }

    #[test]
    fn respeta_un_escritorio_virtual_con_origen_negativo() {
        // Segundo monitor a la izquierda del principal.
        let pantalla = (-1920, 0, 3840, 1080);
        let mut c = cfg();
        c.position = Position::Left;
        let r = place(Region { x: -1800, y: 100, width: 200, height: 100 }, &c, pantalla);
        assert!(r.x >= -1920, "{r:?}");
    }

    #[test]
    fn el_presenter_de_pruebas_registra_lo_que_se_muestra() {
        let p = RecordingPresenter::default();
        (&p).show("原文", "original").unwrap();
        (&p).clear().unwrap();
        assert_eq!(
            p.eventos(),
            vec![("原文".to_string(), "original".to_string()), (String::new(), String::new())]
        );
    }
}
