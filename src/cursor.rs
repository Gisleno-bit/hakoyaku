//! Modo "sigue al raton".
//!
//! En vez de vigilar un rectangulo fijo, se mira **donde apunta el cursor** y se
//! detecta la caja que hay debajo. Encaja con como se juega de verdad: en una
//! novela visual el raton esta casi siempre sobre el cuadro de dialogo, porque
//! es donde se hace clic para avanzar.
//!
//! Ventajas sobre la region fija: no hay que marcar nada, funciona aunque el
//! juego use varios cuadros en sitios distintos (dialogo, nombre del personaje,
//! menu), y si el juego cambia de resolucion sigue valiendo.

use crate::config::Region;

/// Rectangulo donde buscar la caja, centrado en el cursor y recortado a los
/// limites de la zona util.
///
/// Se busca en una ventana acotada y no en toda la pantalla por una razon de
/// coste: esto se ejecuta varias veces por segundo.
pub fn area_de_busqueda(cursor: (i32, i32), limites: Region, ancho: u32, alto: u32) -> Region {
    let ancho = (ancho.max(64)).min(limites.width.max(64));
    let alto = (alto.max(48)).min(limites.height.max(48));

    let x = (cursor.0 - ancho as i32 / 2)
        .max(limites.x)
        .min((limites.right() - ancho as i32).max(limites.x));
    let y = (cursor.1 - alto as i32 / 2)
        .max(limites.y)
        .min((limites.bottom() - alto as i32).max(limites.y));

    Region { x, y, width: ancho, height: alto }
}

/// Porcentaje del area de `a` que tambien esta dentro de `b` (0-100).
pub fn solapamiento(a: Region, b: Region) -> u32 {
    let ancho = (a.right().min(b.right()) - a.x.max(b.x)).max(0) as u64;
    let alto = (a.bottom().min(b.bottom()) - a.y.max(b.y)).max(0) as u64;
    let comun = ancho * alto;
    let area_a = a.width as u64 * a.height as u64;

    if area_a == 0 {
        return 0;
    }
    (comun * 100 / area_a) as u32
}

/// Decide si la caja recien detectada es "la misma de antes".
///
/// La deteccion de bordes baila unos pixeles entre fotogramas segun donde caiga
/// el cursor y que letras haya debajo. Sin esto, cada temblor moveria el
/// recuadro de la traduccion y quedaria nervioso.
pub fn es_la_misma(anterior: Option<Region>, nueva: Region, minimo: u32) -> bool {
    match anterior {
        Some(a) => solapamiento(a, nueva) >= minimo && solapamiento(nueva, a) >= minimo,
        None => false,
    }
}

/// `true` si la caja detectada es sospechosa: casi toda el area de busqueda.
///
/// Suele significar que el cursor estaba sobre el fondo del juego, no sobre una
/// caja, y la deteccion se ha extendido hasta el borde.
pub fn es_demasiado_grande(caja: Region, busqueda: Region) -> bool {
    let area_caja = caja.width as u64 * caja.height as u64;
    let area_busqueda = (busqueda.width as u64 * busqueda.height as u64).max(1);
    area_caja * 100 / area_busqueda > 92
}

/// Detecta la caja bajo el punto probando tolerancias de menor a mayor.
///
/// Se prueba primero ajustado y se va aflojando. Con la referencia adaptativa,
/// la tolerancia solo tiene que cubrir el cambio de un pixel al siguiente, que
/// en un degradado es de una o dos unidades. Una tolerancia grande es peor de
/// lo que parece: si el fondo del juego es casi tan oscuro como la parte oscura
/// del degradado del cuadro, el barrido cruza el borde sin enterarse y se
/// expande por toda la pantalla.
///
/// Probar en escalera evita tener que acertar el numero a mano.
pub fn detectar(
    frame: &crate::frame::Frame,
    punto: (u32, u32),
    busqueda: Region,
    tolerancia_max: u8,
) -> Option<Region> {
    let mut candidatas: Vec<u8> =
        [5u8, 8, 12, 18, 26].into_iter().filter(|t| *t <= tolerancia_max.max(5)).collect();
    if candidatas.is_empty() {
        candidatas.push(tolerancia_max.max(5));
    }

    for t in candidatas {
        if let Some(caja) = frame.caja_desde_punto(punto.0, punto.1, t) {
            if !es_demasiado_grande(caja, busqueda) {
                return Some(caja);
            }
        }
    }
    None
}

/// Posicion actual del cursor, en pixeles fisicos.
pub fn posicion() -> Option<(i32, i32)> {
    #[cfg(windows)]
    {
        win::posicion()
    }
    #[cfg(not(windows))]
    {
        None
    }
}

#[cfg(windows)]
mod win {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

    pub fn posicion() -> Option<(i32, i32)> {
        let mut p = POINT::default();
        unsafe { GetCursorPos(&mut p).ok()? };
        Some((p.x, p.y))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PANTALLA: Region = Region { x: 0, y: 0, width: 1920, height: 1080 };

    #[test]
    fn el_area_se_centra_en_el_cursor() {
        let a = area_de_busqueda((960, 540), PANTALLA, 800, 400);
        assert_eq!(a, Region { x: 560, y: 340, width: 800, height: 400 });
    }

    #[test]
    fn el_area_no_se_sale_por_los_bordes() {
        let esquina = area_de_busqueda((10, 10), PANTALLA, 800, 400);
        assert_eq!((esquina.x, esquina.y), (0, 0));

        let otra = area_de_busqueda((1910, 1070), PANTALLA, 800, 400);
        assert_eq!(otra.right(), 1920);
        assert_eq!(otra.bottom(), 1080);
    }

    #[test]
    fn el_area_nunca_es_mayor_que_los_limites() {
        let pequena = Region { x: 100, y: 100, width: 300, height: 200 };
        let a = area_de_busqueda((250, 200), pequena, 4000, 4000);
        assert_eq!((a.width, a.height), (300, 200));
        assert_eq!((a.x, a.y), (100, 100));
    }

    #[test]
    fn funciona_con_monitores_a_la_izquierda() {
        let virtual_ = Region { x: -1920, y: 0, width: 3840, height: 1080 };
        let a = area_de_busqueda((-1900, 500), virtual_, 800, 400);
        assert!(a.x >= -1920, "{a:?}");
    }

    #[test]
    fn dos_rectangulos_identicos_solapan_del_todo() {
        let r = Region { x: 10, y: 10, width: 100, height: 50 };
        assert_eq!(solapamiento(r, r), 100);
    }

    #[test]
    fn dos_rectangulos_separados_no_solapan() {
        let a = Region { x: 0, y: 0, width: 50, height: 50 };
        let b = Region { x: 500, y: 500, width: 50, height: 50 };
        assert_eq!(solapamiento(a, b), 0);
    }

    #[test]
    fn el_solapamiento_es_relativo_al_primero() {
        let grande = Region { x: 0, y: 0, width: 100, height: 100 };
        let pequeno = Region { x: 0, y: 0, width: 50, height: 100 };
        assert_eq!(solapamiento(pequeno, grande), 100, "el pequeno cabe entero en el grande");
        assert_eq!(
            solapamiento(grande, pequeno),
            50,
            "solo la mitad del grande esta en el pequeno"
        );
    }

    #[test]
    fn un_temblor_de_pocos_pixeles_es_la_misma_caja() {
        let antes = Region { x: 100, y: 400, width: 800, height: 150 };
        let ahora = Region { x: 103, y: 398, width: 796, height: 152 };
        assert!(es_la_misma(Some(antes), ahora, 85));
    }

    #[test]
    fn una_caja_distinta_no_es_la_misma() {
        let dialogo = Region { x: 100, y: 400, width: 800, height: 150 };
        let nombre = Region { x: 100, y: 330, width: 200, height: 50 };
        assert!(!es_la_misma(Some(dialogo), nombre, 85));
    }

    #[test]
    fn sin_caja_anterior_nunca_es_la_misma() {
        assert!(!es_la_misma(None, Region { x: 0, y: 0, width: 10, height: 10 }, 85));
    }

    /// Reproduce el boton del juego real: degradado verde de 25 a 70 sobre un
    /// fondo casi negro (8), con borde claro.
    ///
    /// Con tolerancia 24 el barrido se escapa: |8 - 25| = 17 cae dentro de la
    /// tolerancia, asi que cruza el borde y se expande por toda la pantalla.
    /// La escalera prueba primero valores ajustados y acierta sin que el
    /// usuario tenga que tocar nada.
    #[test]
    fn la_escalera_acierta_donde_una_tolerancia_alta_se_escapa() {
        use crate::frame::Frame;
        let (ancho, alto) = (400u32, 300u32);
        let caja = Region { x: 60, y: 100, width: 260, height: 70 };
        let mut f = Frame::blank(ancho, alto);

        for px in f.data.chunks_exact_mut(4) {
            px[0] = 8;
            px[1] = 8;
            px[2] = 8;
            px[3] = 255;
        }
        for y in caja.y as u32..(caja.y as u32 + caja.height) {
            for x in caja.x as u32..(caja.x as u32 + caja.width) {
                let i = ((y * ancho + x) * 4) as usize;
                let borde = y == caja.y as u32
                    || y == caja.y as u32 + caja.height - 1
                    || x == caja.x as u32
                    || x == caja.x as u32 + caja.width - 1;
                if borde {
                    f.data[i] = 60;
                    f.data[i + 1] = 170;
                    f.data[i + 2] = 210;
                } else {
                    // Gris para que la luminancia sea exactamente 20..34, que
                    // es el rango medido en los botones del juego real. El
                    // fondo esta en 8: solo 12 de diferencia con la parte mas
                    // oscura del degradado.
                    let t = ((y - caja.y as u32) * 14 / caja.height) as u8;
                    f.data[i] = 20 + t;
                    f.data[i + 1] = 20 + t;
                    f.data[i + 2] = 20 + t;
                }
            }
        }

        let busqueda = Region { x: 0, y: 0, width: ancho, height: alto };

        // Con la tolerancia alta de antes, el barrido cruza el borde del boton
        // sin enterarse y se desparrama por el fondo del juego.
        let suelta = f.caja_desde_punto(190, 135, 26).unwrap();
        assert!(
            suelta.width > caja.width + 80,
            "con tolerancia alta deberia escaparse a lo ancho: {suelta:?} vs {caja:?}"
        );

        // La escalera prueba valores mas ajustados y da con el boton.
        let d = detectar(&f, (190, 135), busqueda, 26).expect("la escalera deberia encontrarlo");
        assert!((d.x - caja.x).abs() <= 6, "izquierda: {d:?} vs {caja:?}");
        assert!(d.width >= caja.width - 12 && d.width <= caja.width + 12, "ancho: {d:?}");
        assert!(d.height >= caja.height - 12 && d.height <= caja.height + 12, "alto: {d:?}");
    }

    #[test]
    fn la_escalera_de_tolerancias_respeta_el_maximo() {
        // Un frame uniforme detecta siempre el frame entero, que es "demasiado
        // grande", asi que ninguna tolerancia sirve y devuelve None sin colgarse.
        let f = crate::frame::Frame::new(64, 64, vec![120; 64 * 64 * 4]).unwrap();
        let busqueda = Region { x: 0, y: 0, width: 64, height: 64 };
        assert!(detectar(&f, (32, 32), busqueda, 26).is_none());
    }

    #[test]
    fn una_caja_que_llena_la_busqueda_es_sospechosa() {
        let busqueda = Region { x: 0, y: 0, width: 800, height: 400 };
        let todo = Region { x: 0, y: 0, width: 800, height: 400 };
        assert!(es_demasiado_grande(todo, busqueda));

        let caja = Region { x: 50, y: 100, width: 700, height: 150 };
        assert!(!es_demasiado_grande(caja, busqueda));
    }
}
