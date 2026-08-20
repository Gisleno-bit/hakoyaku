//! Atajos de teclado globales.
//!
//! Sin esto, la unica forma de parar el programa es salir del juego, buscar la
//! consola y pulsar Ctrl+C. Con esto se controla desde dentro de la partida.
//!
//!   F9              pausar / reanudar
//!   F10             releer ahora (fuerza una traduccion aunque no haya cambiado)
//!   Ctrl+Shift+Q    salir
//!
//! `RegisterHotKey` entrega los mensajes al hilo que registro los atajos, asi
//! que hay un hilo propio con su bucle de mensajes que solo hace esto y avisa
//! al pipeline por variables atomicas.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Un atajo ya traducido a lo que espera `RegisterHotKey`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Atajo {
    /// MOD_ALT=1, MOD_CONTROL=2, MOD_SHIFT=4, MOD_WIN=8.
    pub modificadores: u32,
    /// Codigo de tecla virtual de Windows.
    pub tecla: u32,
}

/// Traduce `"ctrl+space"` o `"alt+h"` en modificadores y codigo de tecla.
///
/// Los atajos se configuran por texto en el TOML y no se fijan en el codigo
/// porque cualquier tecla que se elija va a chocar con algun juego: unos usan
/// las F para guardar partida, otros el espacio para avanzar. La eleccion tiene
/// que ser del usuario.
pub fn parsear(texto: &str) -> Option<Atajo> {
    let limpio = texto.trim().to_lowercase();
    if limpio.is_empty() || limpio == "none" || limpio == "ninguno" {
        return None;
    }

    let mut modificadores = 0u32;
    let mut tecla = None;

    for parte in limpio.split('+').map(str::trim).filter(|p| !p.is_empty()) {
        match parte {
            "ctrl" | "control" => modificadores |= 2,
            "shift" | "mayus" => modificadores |= 4,
            "alt" => modificadores |= 1,
            "win" | "windows" | "meta" => modificadores |= 8,
            otro => {
                if tecla.is_some() {
                    return None; // dos teclas, solo se admite una
                }
                tecla = codigo_de_tecla(otro);
                tecla?;
            }
        }
    }

    tecla.map(|t| Atajo { modificadores, tecla: t })
}

/// Nombre de tecla -> codigo virtual de Windows.
fn codigo_de_tecla(nombre: &str) -> Option<u32> {
    // Letra suelta: 'a' -> 0x41. Los codigos coinciden con el ASCII en mayuscula.
    if nombre.len() == 1 {
        let c = nombre.chars().next()?;
        if c.is_ascii_alphabetic() {
            return Some(c.to_ascii_uppercase() as u32);
        }
        if c.is_ascii_digit() {
            return Some(c as u32);
        }
    }

    // Teclas de funcion: f1 = 0x70 ... f24.
    if let Some(n) = nombre.strip_prefix('f') {
        if let Ok(num) = n.parse::<u32>() {
            if (1..=24).contains(&num) {
                return Some(0x6F + num);
            }
        }
    }

    Some(match nombre {
        "space" | "espacio" => 0x20,
        "tab" | "tabulador" => 0x09,
        "esc" | "escape" => 0x1B,
        "enter" | "intro" | "return" => 0x0D,
        "insert" | "ins" => 0x2D,
        "delete" | "del" | "supr" => 0x2E,
        "home" | "inicio" => 0x24,
        "end" | "fin" => 0x23,
        "pageup" | "pgup" | "repag" => 0x21,
        "pagedown" | "pgdn" | "avpag" => 0x22,
        "left" | "izquierda" => 0x25,
        "up" | "arriba" => 0x26,
        "right" | "derecha" => 0x27,
        "down" | "abajo" => 0x28,
        // La tecla del acento grave / ordinal, encima del tabulador.
        "tilde" | "grave" | "backtick" => 0xC0,
        "menos" | "minus" => 0xBD,
        "mas" | "plus" => 0xBB,
        _ => return None,
    })
}

/// Estado compartido entre el hilo de atajos y el pipeline.
#[derive(Debug, Default)]
pub struct Control {
    pausado: AtomicBool,
    relectura: AtomicBool,
    salir: AtomicBool,
    oculto: AtomicBool,
}

impl Control {
    pub fn nuevo() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn esta_pausado(&self) -> bool {
        self.pausado.load(Ordering::Relaxed)
    }

    /// Cambia el estado y devuelve el nuevo.
    pub fn alternar_pausa(&self) -> bool {
        !self.pausado.fetch_xor(true, Ordering::Relaxed)
    }

    /// Ocultar la traduccion es distinto de pausar: pausar deja lo ultimo en
    /// pantalla y deja de mirar; ocultar quita el recuadro para ver el juego
    /// limpio, y al volver se relee para que reaparezca al momento.
    pub fn esta_oculto(&self) -> bool {
        self.oculto.load(Ordering::Relaxed)
    }

    pub fn alternar_oculto(&self) -> bool {
        !self.oculto.fetch_xor(true, Ordering::Relaxed)
    }

    pub fn hay_que_salir(&self) -> bool {
        self.salir.load(Ordering::Relaxed)
    }

    pub fn pedir_salida(&self) {
        self.salir.store(true, Ordering::Relaxed);
    }

    pub fn pedir_relectura(&self) {
        self.relectura.store(true, Ordering::Relaxed);
    }

    /// Mira si hay relectura pendiente sin consumirla.
    pub fn hay_relectura(&self) -> bool {
        self.relectura.load(Ordering::Relaxed)
    }

    /// Consume la peticion de relectura: devuelve `true` una sola vez.
    pub fn tomar_relectura(&self) -> bool {
        self.relectura.swap(false, Ordering::Relaxed)
    }
}

/// Los cuatro atajos, ya traducidos. `None` = ese atajo esta desactivado.
#[derive(Debug, Clone, Copy, Default)]
pub struct Atajos {
    pub ocultar: Option<Atajo>,
    pub pausar: Option<Atajo>,
    pub releer: Option<Atajo>,
    pub salir: Option<Atajo>,
}

impl Atajos {
    pub fn desde_config(cfg: &crate::config::Hotkeys) -> Self {
        Self {
            ocultar: parsear(&cfg.toggle_overlay),
            pausar: parsear(&cfg.pause),
            releer: parsear(&cfg.reread),
            salir: parsear(&cfg.quit),
        }
    }
}

/// Arranca el hilo que escucha los atajos. En sistemas sin soporte no hace
/// nada y el programa sigue funcionando igual, solo que sin atajos.
pub fn escuchar(control: Arc<Control>, atajos: Atajos) {
    #[cfg(windows)]
    {
        win::escuchar(control, atajos);
    }
    #[cfg(not(windows))]
    {
        let _ = (control, atajos);
    }
}

#[cfg(windows)]
mod win {
    use super::*;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::Input::KeyboardAndMouse::{RegisterHotKey, HOT_KEY_MODIFIERS};
    use windows::Win32::UI::WindowsAndMessaging::{GetMessageW, MSG, WM_HOTKEY};

    const ID_OCULTAR: i32 = 1;
    const ID_PAUSA: i32 = 2;
    const ID_RELEER: i32 = 3;
    const ID_SALIR: i32 = 4;

    /// MOD_NOREPEAT: sin esto, dejar la tecla pulsada dispara el atajo decenas
    /// de veces por segundo.
    const SIN_REPETIR: u32 = 0x4000;

    unsafe fn registrar(id: i32, atajo: Option<Atajo>) -> bool {
        match atajo {
            Some(a) => {
                RegisterHotKey(None, id, HOT_KEY_MODIFIERS(a.modificadores | SIN_REPETIR), a.tecla)
                    .is_ok()
            }
            None => false,
        }
    }

    pub fn escuchar(control: Arc<Control>, atajos: Atajos) {
        std::thread::Builder::new()
            .name("hakoyaku-atajos".into())
            .spawn(move || unsafe {
                let mut registrados = 0;
                for (id, atajo) in [
                    (ID_OCULTAR, atajos.ocultar),
                    (ID_PAUSA, atajos.pausar),
                    (ID_RELEER, atajos.releer),
                    (ID_SALIR, atajos.salir),
                ] {
                    if registrar(id, atajo) {
                        registrados += 1;
                    } else if atajo.is_some() {
                        log::warn!("no se pudo registrar un atajo: ya lo usa otro programa");
                    }
                }

                if registrados == 0 {
                    println!(
                        "\nAviso: no se pudo registrar ningun atajo. Otro programa los esta\n\
                         usando. Cambia las teclas en la seccion [hotkeys] del fichero de\n\
                         configuracion, o sal con Ctrl+C en esta ventana."
                    );
                    return;
                }

                let mut msg = MSG::default();
                while GetMessageW(&mut msg, HWND::default(), 0, 0).as_bool() {
                    if msg.message != WM_HOTKEY {
                        continue;
                    }
                    match msg.wParam.0 as i32 {
                        ID_OCULTAR => {
                            let oculto = control.alternar_oculto();
                            println!(
                                "\n[{}]",
                                if oculto { "traduccion oculta" } else { "traduccion visible" }
                            );
                        }
                        ID_PAUSA => {
                            let pausado = control.alternar_pausa();
                            println!("\n[{}]", if pausado { "PAUSADO" } else { "reanudado" });
                        }
                        ID_RELEER => {
                            control.pedir_relectura();
                            println!("\n[releyendo…]");
                        }
                        ID_SALIR => {
                            control.pedir_salida();
                            return;
                        }
                        _ => {}
                    }
                }
            })
            .ok();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parsea_una_letra_con_modificador() {
        let a = parsear("ctrl+space").unwrap();
        assert_eq!(a.modificadores, 2);
        assert_eq!(a.tecla, 0x20);

        let a = parsear("alt+h").unwrap();
        assert_eq!(a.modificadores, 1);
        assert_eq!(a.tecla, 'H' as u32);
    }

    #[test]
    fn acepta_varios_modificadores_y_da_igual_el_orden() {
        let a = parsear("ctrl+shift+q").unwrap();
        assert_eq!(a.modificadores, 2 | 4);
        assert_eq!(parsear("shift+ctrl+q").unwrap(), a);
        assert_eq!(
            parsear("  CTRL + SHIFT + Q  ").unwrap(),
            a,
            "sin distinguir mayusculas ni espacios"
        );
    }

    #[test]
    fn parsea_teclas_de_funcion() {
        assert_eq!(parsear("f1").unwrap().tecla, 0x70);
        assert_eq!(parsear("f9").unwrap().tecla, 0x78);
        assert_eq!(parsear("f12").unwrap().tecla, 0x7B);
        assert!(parsear("f99").is_none());
    }

    #[test]
    fn parsea_nombres_en_espanol_y_en_ingles() {
        assert_eq!(parsear("espacio").unwrap().tecla, parsear("space").unwrap().tecla);
        assert_eq!(parsear("supr").unwrap().tecla, parsear("delete").unwrap().tecla);
        assert_eq!(parsear("intro").unwrap().tecla, parsear("enter").unwrap().tecla);
    }

    #[test]
    fn una_tecla_sin_modificador_tambien_vale() {
        let a = parsear("tilde").unwrap();
        assert_eq!(a.modificadores, 0);
        assert_eq!(a.tecla, 0xC0);
    }

    #[test]
    fn desactivar_un_atajo() {
        assert!(parsear("").is_none());
        assert!(parsear("   ").is_none());
        assert!(parsear("none").is_none());
        assert!(parsear("ninguno").is_none());
    }

    #[test]
    fn rechaza_lo_que_no_entiende() {
        assert!(parsear("ctrl+banana").is_none());
        assert!(parsear("ctrl").is_none(), "un modificador solo no es un atajo");
        assert!(parsear("a+b").is_none(), "dos teclas no");
    }

    #[test]
    fn ocultar_alterna_y_es_independiente_de_pausar() {
        let c = Control::nuevo();
        assert!(!c.esta_oculto());
        assert!(c.alternar_oculto());
        assert!(c.esta_oculto());
        assert!(!c.esta_pausado(), "ocultar no debe pausar");
        assert!(!c.alternar_oculto());
        assert!(!c.esta_oculto());
    }

    #[test]
    fn empieza_en_marcha() {
        let c = Control::nuevo();
        assert!(!c.esta_pausado());
        assert!(!c.hay_que_salir());
    }

    #[test]
    fn la_pausa_alterna() {
        let c = Control::nuevo();
        assert!(c.alternar_pausa(), "la primera pulsacion pausa");
        assert!(c.esta_pausado());
        assert!(!c.alternar_pausa(), "la segunda reanuda");
        assert!(!c.esta_pausado());
    }

    #[test]
    fn la_relectura_se_consume_una_sola_vez() {
        let c = Control::nuevo();
        assert!(!c.tomar_relectura());
        c.pedir_relectura();
        assert!(c.tomar_relectura());
        assert!(!c.tomar_relectura(), "no debe repetirse sin volver a pedirla");
    }

    #[test]
    fn se_puede_mirar_la_relectura_sin_gastarla() {
        let c = Control::nuevo();
        c.pedir_relectura();
        assert!(c.hay_relectura());
        assert!(c.hay_relectura(), "mirar no debe consumir");
        assert!(c.tomar_relectura());
        assert!(!c.hay_relectura(), "tomar si consume");
    }

    #[test]
    fn la_salida_no_se_deshace() {
        let c = Control::nuevo();
        c.pedir_salida();
        assert!(c.hay_que_salir());
        assert!(c.hay_que_salir());
    }

    #[test]
    fn el_control_se_puede_compartir_entre_hilos() {
        let c = Control::nuevo();
        let otro = Arc::clone(&c);
        std::thread::spawn(move || otro.pedir_salida()).join().unwrap();
        assert!(c.hay_que_salir());
    }
}
