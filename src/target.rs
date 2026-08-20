//! Anclar el traductor a una aplicacion concreta.
//!
//! Sin esto la region va en coordenadas absolutas de pantalla, asi que en
//! cuanto mueves la ventana del juego —o la maximizas, o cambias de monitor—
//! el programa sigue mirando el sitio de antes y lee cualquier cosa.
//!
//! Con una ventana elegida, la region se guarda **relativa al area de cliente**
//! de esa ventana. Muevas el juego donde lo muevas, el recuadro le sigue.
//!
//! De propina: si el juego no esta en primer plano, no se traduce nada. Eso
//! evita el efecto mas desconcertante de todos, que es ver al programa
//! traduciendo el navegador que tienes detras.

use crate::config::Region;
use anyhow::Result;

/// Una ventana de escritorio candidata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ventana {
    /// Puntero de la ventana como entero: `HWND` no es `Send`.
    pub hwnd: usize,
    pub titulo: String,
    /// Area util (sin bordes ni barra de titulo), en coordenadas de pantalla.
    pub cliente: Region,
}

/// `true` si `titulo` contiene `fragmento`, ignorando mayusculas.
///
/// Se compara por fragmento y no por igualdad porque los juegos meten en el
/// titulo cosas que cambian solas: contadores de FPS, la version, el modo de
/// pantalla. `サキュバスプリズン-乳夢帰還-V1.00` no es un identificador estable,
/// pero `サキュバスプリズン` si.
pub fn coincide(titulo: &str, fragmento: &str) -> bool {
    let f = fragmento.trim();
    if f.is_empty() {
        return false;
    }
    titulo.to_lowercase().contains(&f.to_lowercase())
}

/// Elige la mejor ventana de entre las que coinciden.
///
/// Se prefiere la de titulo mas corto: si el fragmento es "Prison" y hay una
/// ventana del juego y otra del navegador titulada "Prison - Guia - Chrome",
/// la del juego casi siempre tiene el titulo mas breve.
pub fn elegir<'a>(ventanas: &'a [Ventana], fragmento: &str) -> Option<&'a Ventana> {
    ventanas
        .iter()
        .filter(|v| coincide(&v.titulo, fragmento))
        .min_by_key(|v| v.titulo.chars().count())
}

/// Convierte una region en coordenadas de pantalla a coordenadas relativas al
/// area de cliente de la ventana.
pub fn a_relativa(absoluta: Region, cliente: Region) -> Region {
    Region {
        x: absoluta.x - cliente.x,
        y: absoluta.y - cliente.y,
        width: absoluta.width,
        height: absoluta.height,
    }
}

/// El camino de vuelta: region relativa + ventana actual -> coordenadas de
/// pantalla de ahora mismo.
pub fn a_absoluta(relativa: Region, cliente: Region) -> Region {
    Region {
        x: cliente.x + relativa.x,
        y: cliente.y + relativa.y,
        width: relativa.width,
        height: relativa.height,
    }
}

/// Recorta la region para que no se salga del area de cliente.
///
/// Si el jugador redimensiona la ventana a algo mas pequeno que cuando marco la
/// region, capturar fuera del cliente devolveria basura.
pub fn recortar(region: Region, cliente: Region) -> Option<Region> {
    let x = region.x.max(cliente.x);
    let y = region.y.max(cliente.y);
    let derecha = region.right().min(cliente.right());
    let abajo = region.bottom().min(cliente.bottom());

    if derecha - x < 8 || abajo - y < 8 {
        return None;
    }
    Some(Region { x, y, width: (derecha - x) as u32, height: (abajo - y) as u32 })
}

/// Donde hay que mirar en este instante, segun la configuracion.
///
/// **Es la unica funcion que debe traducir `cfg.region` a coordenadas de
/// pantalla.** Antes cada sitio lo hacia por su cuenta y el marco y el volcado
/// se olvidaban del anclaje, asi que con una region relativa acababan pintando
/// arriba a la izquierda.
pub fn resolver(cfg: &crate::config::Config) -> Result<Region> {
    if !cfg.anclado() {
        return Ok(cfg.region);
    }

    let titulo = cfg.target.window_title.trim();
    let ventana = buscar(titulo)?.ok_or_else(|| {
        anyhow::anyhow!("no encuentro la ventana '{titulo}'. ¿Esta el juego abierto?")
    })?;

    let absoluta = a_absoluta(cfg.region, ventana.cliente);
    recortar(absoluta, ventana.cliente)
        .ok_or_else(|| anyhow::anyhow!("la region marcada se ha quedado fuera de la ventana"))
}

/// Lista las ventanas visibles con titulo. Solo en Windows.
pub fn listar() -> Result<Vec<Ventana>> {
    #[cfg(windows)]
    {
        win::listar()
    }
    #[cfg(not(windows))]
    {
        Ok(Vec::new())
    }
}

/// Busca la ventana que corresponde al fragmento guardado en la configuracion.
pub fn buscar(fragmento: &str) -> Result<Option<Ventana>> {
    let ventanas = listar()?;
    Ok(elegir(&ventanas, fragmento).cloned())
}

/// Ventana de nivel superior que hay **dibujada** en ese punto.
///
/// Comprobar solo que el cursor cae dentro del rectangulo del juego no basta:
/// si tienes el explorador de archivos encima, el punto sigue estando dentro
/// del rectangulo pero lo que se ve —y lo que captura BitBlt— es el explorador.
pub fn ventana_en(punto: (i32, i32)) -> Option<usize> {
    #[cfg(windows)]
    {
        win::ventana_en(punto)
    }
    #[cfg(not(windows))]
    {
        let _ = punto;
        None
    }
}

/// La ventana que hay dibujada en `punto`, si pertenece al juego.
///
/// Se resuelve por titulo y no por HWND porque un juego puede tener varias
/// ventanas de nivel superior; cualquiera de ellas vale mientras sea suya.
pub fn ventana_bajo_el_cursor(punto: (i32, i32), fragmento: &str) -> Option<Ventana> {
    let hwnd = ventana_en(punto)?;
    let ventanas = listar().ok()?;
    ventanas.into_iter().find(|v| v.hwnd == hwnd && coincide(&v.titulo, fragmento))
}

/// `true` si esa ventana es la que tiene el foco.
pub fn tiene_el_foco(hwnd: usize) -> bool {
    #[cfg(windows)]
    {
        win::tiene_el_foco(hwnd)
    }
    #[cfg(not(windows))]
    {
        let _ = hwnd;
        true
    }
}

#[cfg(windows)]
mod win {
    use super::*;
    use std::cell::RefCell;
    use windows::Win32::Foundation::{BOOL, HWND, LPARAM, POINT, RECT, TRUE};
    use windows::Win32::Graphics::Gdi::ClientToScreen;
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetAncestor, GetClientRect, GetForegroundWindow, GetWindowTextLengthW,
        GetWindowTextW, IsIconic, IsWindowVisible, WindowFromPoint, GA_ROOT,
    };

    thread_local! {
        static RECOGIDAS: RefCell<Vec<Ventana>> = const { RefCell::new(Vec::new()) };
    }

    pub fn ventana_en(punto: (i32, i32)) -> Option<usize> {
        unsafe {
            let h = WindowFromPoint(POINT { x: punto.0, y: punto.1 });
            if h.0.is_null() {
                return None;
            }
            // WindowFromPoint devuelve el control concreto (un boton, un panel);
            // hay que subir hasta la ventana de nivel superior para comparar.
            let raiz = GetAncestor(h, GA_ROOT);
            Some(if raiz.0.is_null() { h.0 as usize } else { raiz.0 as usize })
        }
    }

    pub fn tiene_el_foco(hwnd: usize) -> bool {
        unsafe { GetForegroundWindow().0 as usize == hwnd }
    }

    /// Area de cliente en coordenadas de pantalla.
    unsafe fn cliente_en_pantalla(hwnd: HWND) -> Option<Region> {
        let mut r = RECT::default();
        GetClientRect(hwnd, &mut r).ok()?;

        let mut origen = POINT { x: 0, y: 0 };
        if !ClientToScreen(hwnd, &mut origen).as_bool() {
            return None;
        }

        let ancho = r.right - r.left;
        let alto = r.bottom - r.top;
        if ancho <= 0 || alto <= 0 {
            return None;
        }
        Some(Region { x: origen.x, y: origen.y, width: ancho as u32, height: alto as u32 })
    }

    unsafe extern "system" fn recoger(hwnd: HWND, _lp: LPARAM) -> BOOL {
        if !IsWindowVisible(hwnd).as_bool() || IsIconic(hwnd).as_bool() {
            return TRUE;
        }

        let largo = GetWindowTextLengthW(hwnd);
        if largo <= 0 {
            return TRUE;
        }

        let mut buf = vec![0u16; largo as usize + 1];
        let escrito = GetWindowTextW(hwnd, &mut buf);
        if escrito <= 0 {
            return TRUE;
        }
        let titulo = String::from_utf16_lossy(&buf[..escrito as usize]);

        if let Some(cliente) = cliente_en_pantalla(hwnd) {
            // Descarta ventanas de utilidad: nadie traduce un recuadro de 50px.
            if cliente.width >= 200 && cliente.height >= 150 {
                RECOGIDAS.with(|v| {
                    v.borrow_mut().push(Ventana { hwnd: hwnd.0 as usize, titulo, cliente })
                });
            }
        }
        TRUE
    }

    pub fn listar() -> Result<Vec<Ventana>> {
        RECOGIDAS.with(|v| v.borrow_mut().clear());
        unsafe {
            EnumWindows(Some(recoger), LPARAM(0)).ok();
        }
        Ok(RECOGIDAS.with(|v| v.borrow().clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ventana(titulo: &str, cliente: Region) -> Ventana {
        Ventana { hwnd: 1, titulo: titulo.into(), cliente }
    }

    const CLIENTE: Region = Region { x: 458, y: 151, width: 1021, height: 761 };

    #[test]
    fn coincide_por_fragmento_e_ignora_mayusculas() {
        assert!(coincide("サキュバスプリズン-乳夢帰還-V1.00", "サキュバスプリズン"));
        assert!(coincide("My Game [FPS60]", "my game"));
        assert!(coincide("MY GAME", "game"));
        assert!(!coincide("Otra cosa", "game"));
    }

    #[test]
    fn un_fragmento_vacio_no_coincide_con_nada() {
        assert!(!coincide("cualquier cosa", ""));
        assert!(!coincide("cualquier cosa", "   "));
    }

    #[test]
    fn elige_el_titulo_mas_corto_entre_los_que_coinciden() {
        let vs = vec![
            ventana("Prison - Guia completa - Chrome", CLIENTE),
            ventana("Prison V1.00", CLIENTE),
            ventana("Bloc de notas", CLIENTE),
        ];
        assert_eq!(elegir(&vs, "prison").unwrap().titulo, "Prison V1.00");
    }

    #[test]
    fn sin_coincidencias_no_elige_nada() {
        let vs = vec![ventana("Bloc de notas", CLIENTE)];
        assert!(elegir(&vs, "prison").is_none());
    }

    #[test]
    fn ida_y_vuelta_entre_relativa_y_absoluta() {
        let absoluta = Region { x: 900, y: 500, width: 820, height: 145 };
        let relativa = a_relativa(absoluta, CLIENTE);
        assert_eq!(relativa, Region { x: 442, y: 349, width: 820, height: 145 });
        assert_eq!(a_absoluta(relativa, CLIENTE), absoluta);
    }

    #[test]
    fn al_mover_la_ventana_la_region_la_sigue() {
        let absoluta = Region { x: 900, y: 500, width: 820, height: 145 };
        let relativa = a_relativa(absoluta, CLIENTE);

        // El jugador arrastra el juego 300 px a la derecha y 100 hacia abajo.
        let movida = Region { x: 758, y: 251, ..CLIENTE };
        let nueva = a_absoluta(relativa, movida);

        assert_eq!(nueva.x, absoluta.x + 300);
        assert_eq!(nueva.y, absoluta.y + 100);
        assert_eq!((nueva.width, nueva.height), (820, 145));
    }

    #[test]
    fn la_region_se_recorta_al_area_de_cliente() {
        // Ventana encogida: la region se sale por la derecha.
        let cliente = Region { x: 0, y: 0, width: 500, height: 400 };
        let region = Region { x: 100, y: 100, width: 800, height: 100 };
        let r = recortar(region, cliente).unwrap();
        assert_eq!(r, Region { x: 100, y: 100, width: 400, height: 100 });
    }

    #[test]
    fn una_region_que_ya_cabe_no_se_toca() {
        let region = Region { x: 500, y: 300, width: 800, height: 145 };
        assert_eq!(recortar(region, CLIENTE).unwrap(), region);
    }

    #[test]
    fn si_no_queda_nada_visible_no_hay_region() {
        let cliente = Region { x: 0, y: 0, width: 500, height: 400 };
        let fuera = Region { x: 900, y: 900, width: 100, height: 100 };
        assert!(recortar(fuera, cliente).is_none());
    }
}
