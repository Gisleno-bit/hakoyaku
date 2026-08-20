//! Marcar en pantalla la region a vigilar.
//!
//! En vez de dibujar un rectangulo arrastrando (que exige una ventana a
//! pantalla completa por encima del juego, y eso pelea con los juegos en
//! pantalla completa), se marcan las dos esquinas con una tecla. Es mas tosco
//! de contar y muchisimo mas fiable de usar: funciona con el juego delante.

use crate::config::Region;
use anyhow::Result;

/// Construye la region a partir de dos esquinas cualesquiera, en cualquier
/// orden. Devuelve `None` si el rectangulo es degenerado.
pub fn region_from_corners(a: (i32, i32), b: (i32, i32)) -> Option<Region> {
    let x = a.0.min(b.0);
    let y = a.1.min(b.1);
    let ancho = (a.0 - b.0).unsigned_abs();
    let alto = (a.1 - b.1).unsigned_abs();

    if ancho < 8 || alto < 8 {
        return None;
    }
    Some(Region { x, y, width: ancho, height: alto })
}

/// Espera a que el usuario pinche un punto con F8 y devuelve sus coordenadas.
pub fn pick_point(mensaje: &str) -> Result<(i32, i32)> {
    #[cfg(windows)]
    {
        win::pick_point(mensaje)
    }
    #[cfg(not(windows))]
    {
        let _ = mensaje;
        anyhow::bail!("marcar puntos necesita Windows")
    }
}

/// Guia al usuario para marcar las dos esquinas. Solo en Windows.
pub fn pick_region() -> Result<Region> {
    #[cfg(windows)]
    {
        win::pick_region()
    }
    #[cfg(not(windows))]
    {
        anyhow::bail!("`hakoyaku pick` necesita Windows; edita la seccion [region] a mano")
    }
}

#[cfg(windows)]
mod win {
    use super::*;
    use anyhow::bail;
    use std::io::Write;
    use std::time::Duration;
    use windows::Win32::Foundation::POINT;
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_ESCAPE, VK_F8};
    use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

    fn pulsada(vk: u16) -> bool {
        // El bit alto indica "esta pulsada ahora mismo".
        unsafe { (GetAsyncKeyState(vk as i32) as u16 & 0x8000) != 0 }
    }

    fn cursor() -> Result<(i32, i32)> {
        let mut p = POINT::default();
        unsafe { GetCursorPos(&mut p)? };
        Ok((p.x, p.y))
    }

    /// Espera a que se pulse y se suelte F8, y devuelve donde estaba el raton al
    /// pulsarla. Esc aborta.
    fn esperar_marca() -> Result<(i32, i32)> {
        // Si F8 venia pulsada de la marca anterior, esperamos a que se suelte.
        while pulsada(VK_F8.0) {
            std::thread::sleep(Duration::from_millis(15));
        }

        loop {
            if pulsada(VK_ESCAPE.0) {
                bail!("seleccion cancelada");
            }
            if pulsada(VK_F8.0) {
                let punto = cursor()?;
                while pulsada(VK_F8.0) {
                    std::thread::sleep(Duration::from_millis(15));
                }
                return Ok(punto);
            }
            std::thread::sleep(Duration::from_millis(15));
        }
    }

    pub fn pick_point(mensaje: &str) -> Result<(i32, i32)> {
        crate::capture::enable_dpi_awareness();
        print!("{mensaje}");
        std::io::stdout().flush().ok();
        let p = esperar_marca()?;
        println!("marcado en {},{}", p.0, p.1);
        Ok(p)
    }

    pub fn pick_region() -> Result<Region> {
        crate::capture::enable_dpi_awareness();

        println!("Vas a marcar el recuadro de texto del juego con dos esquinas.");
        println!(
            "Deja esta ventana de fondo y pon el juego delante; las teclas funcionan igual.\n"
        );
        print!("  1) Pon el raton en la esquina SUPERIOR IZQUIERDA y pulsa F8 (Esc cancela)... ");
        std::io::stdout().flush().ok();

        let a = esperar_marca()?;
        println!("marcado en {},{}", a.0, a.1);

        print!("  2) Ahora en la esquina INFERIOR DERECHA y pulsa F8... ");
        std::io::stdout().flush().ok();

        let b = esperar_marca()?;
        println!("marcado en {},{}", b.0, b.1);

        match region_from_corners(a, b) {
            Some(r) => {
                println!("\nRegion: x={} y={} ancho={} alto={}", r.x, r.y, r.width, r.height);
                Ok(r)
            }
            None => bail!(
                "las dos esquinas estan demasiado juntas ({}x{} px). Vuelve a intentarlo.",
                (a.0 - b.0).abs(),
                (a.1 - b.1).abs()
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn construye_la_region_desde_las_esquinas() {
        let r = region_from_corners((100, 400), (900, 550)).unwrap();
        assert_eq!(r, Region { x: 100, y: 400, width: 800, height: 150 });
    }

    #[test]
    fn el_orden_de_las_esquinas_da_igual() {
        let a = region_from_corners((100, 400), (900, 550)).unwrap();
        let b = region_from_corners((900, 550), (100, 400)).unwrap();
        let c = region_from_corners((900, 400), (100, 550)).unwrap();
        assert_eq!(a, b);
        assert_eq!(a, c);
    }

    #[test]
    fn funciona_con_coordenadas_negativas() {
        // Monitor secundario a la izquierda del principal.
        let r = region_from_corners((-1800, 100), (-1000, 300)).unwrap();
        assert_eq!(r, Region { x: -1800, y: 100, width: 800, height: 200 });
    }

    #[test]
    fn rechaza_rectangulos_degenerados() {
        assert!(region_from_corners((100, 100), (100, 100)).is_none());
        assert!(region_from_corners((100, 100), (103, 400)).is_none());
        assert!(region_from_corners((100, 100), (400, 102)).is_none());
    }
}
