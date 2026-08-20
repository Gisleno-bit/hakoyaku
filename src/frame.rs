//! Representacion de un fotograma capturado y el preprocesado previo al OCR.
//!
//! Todo aqui es aritmetica sobre un `Vec<u8>`: se compila y se testea igual en
//! Windows que en Linux. La captura real vive en `capture.rs`.

use crate::config::Region;
use anyhow::{bail, Result};

/// Fotograma en BGRA8 (el orden nativo de GDI y de `SoftwareBitmap::Bgra8`).
#[derive(Clone, PartialEq, Eq)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    /// `width * height * 4` bytes, de arriba a abajo.
    pub data: Vec<u8>,
}

impl std::fmt::Debug for Frame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Frame({}x{}, {} bytes)", self.width, self.height, self.data.len())
    }
}

impl Frame {
    pub fn new(width: u32, height: u32, data: Vec<u8>) -> Result<Self> {
        let esperado = width as usize * height as usize * 4;
        if data.len() != esperado {
            bail!(
                "buffer de {} bytes para un frame de {}x{} (se esperaban {})",
                data.len(),
                width,
                height,
                esperado
            );
        }
        Ok(Self { width, height, data })
    }

    /// Frame en negro, util para tests y para inicializar estado.
    pub fn blank(width: u32, height: u32) -> Self {
        Self { width, height, data: vec![0u8; width as usize * height as usize * 4] }
    }

    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * self.width + x) * 4) as usize;
        [self.data[i], self.data[i + 1], self.data[i + 2], self.data[i + 3]]
    }

    /// Huella del contenido. Se usa para saber si el cuadro de dialogo ha
    /// cambiado sin gastar una llamada al OCR.
    ///
    /// FNV-1a de 64 bits: no es criptografico, pero es rapido y determinista,
    /// que es justo lo que hace falta aqui.
    pub fn fingerprint(&self) -> u64 {
        const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
        const PRIME: u64 = 0x1000_0000_01b3;

        let mut h = OFFSET;
        h ^= self.width as u64;
        h = h.wrapping_mul(PRIME);
        h ^= self.height as u64;
        h = h.wrapping_mul(PRIME);

        // Solo canales BGR: ignorar alfa evita falsos cambios con ventanas
        // que componen con transparencia.
        for px in self.data.chunks_exact(4) {
            h ^= px[0] as u64;
            h = h.wrapping_mul(PRIME);
            h ^= px[1] as u64;
            h = h.wrapping_mul(PRIME);
            h ^= px[2] as u64;
            h = h.wrapping_mul(PRIME);
        }
        h
    }

    /// Firma perceptual: luminancia media de una rejilla de bloques.
    ///
    /// El `fingerprint` exacto no sirve para detectar "ha cambiado el dialogo"
    /// en un juego con fondo animado: cuatro particulas cayendo ya cambian el
    /// hash, la pantalla no se estabiliza nunca y no se lee jamas. Comparando
    /// medias por bloque, el ruido de fondo se promedia y desaparece, mientras
    /// que un cambio de texto mueve varios bloques de golpe.
    pub fn signature(&self, cols: u32, rows: u32) -> Vec<u8> {
        let cols = cols.max(1).min(self.width.max(1));
        let rows = rows.max(1).min(self.height.max(1));
        let mut out = Vec::with_capacity((cols * rows) as usize);

        for by in 0..rows {
            let y0 = by * self.height / rows;
            let y1 = ((by + 1) * self.height / rows).max(y0 + 1).min(self.height);
            for bx in 0..cols {
                let x0 = bx * self.width / cols;
                let x1 = ((bx + 1) * self.width / cols).max(x0 + 1).min(self.width);

                let mut suma: u64 = 0;
                let mut n: u64 = 0;
                for y in y0..y1 {
                    let fila = (y * self.width) as usize * 4;
                    for x in x0..x1 {
                        let i = fila + x as usize * 4;
                        suma += (self.data[i + 2] as u64 * 299
                            + self.data[i + 1] as u64 * 587
                            + self.data[i] as u64 * 114)
                            / 1000;
                        n += 1;
                    }
                }
                out.push(suma.checked_div(n).unwrap_or(0) as u8);
            }
        }

        out
    }

    /// Detecta los bordes de la caja de dialogo a partir de un punto de dentro.
    ///
    /// Marcar dos esquinas con precision es incomodo. Pinchar en medio del
    /// cuadro y que el programa averigue donde acaba es mucho mas natural.
    ///
    /// El metodo es **adaptativo**, no de color plano: al avanzar, la referencia
    /// se va mezclando con lo que se encuentra. Un degradado cambia poco a poco
    /// y la referencia lo sigue; el borde del cuadro es un salto brusco y ahi se
    /// para. Con una tolerancia fija sobre el color del punto inicial, un boton
    /// con fondo degradado sobre un fondo oscuro es indetectable: o se corta a
    /// mitad del degradado, o se escapa al fondo del juego.
    ///
    /// Para arriba y abajo no se mira un pixel sino la **mediana** de la fila:
    /// asi las letras, que son minoria, no cortan la expansion.
    ///
    /// `(px, py)` van en coordenadas de este fotograma.
    pub fn caja_desde_punto(&self, px: u32, py: u32, tolerancia: u8) -> Option<Region> {
        if px >= self.width || py >= self.height || self.width < 8 || self.height < 8 {
            return None;
        }

        // El punto puede caer encima de una letra, que es justo donde apunta
        // cualquiera. Se estima el color de fondo de la caja con la mediana del
        // vecindario (robusta: las letras son minoria) y se busca el pixel de
        // fondo mas cercano para empezar desde ahi.
        let fondo = self.fondo_local(px, py, 14);
        let (sx, sy) = self.buscar_semilla(px, py, fondo, tolerancia)?;

        let (izq, der) = self.expandir_horizontal(sx, sy, tolerancia);
        let (arriba, abajo) = self.expandir_vertical(sy, izq, der, tolerancia);
        let (izq, der) = self.refinar_horizontal(izq, der, arriba, abajo, tolerancia);

        let ancho = der - izq + 1;
        let alto = abajo - arriba + 1;
        if ancho < 24 || alto < 12 {
            return None;
        }

        Some(Region { x: izq as i32, y: arriba as i32, width: ancho, height: alto })
    }

    /// Luminancia de fondo estimada alrededor de un punto.
    ///
    /// Mediana y no media: si apuntas a una letra blanca sobre caja oscura, la
    /// media saldria gris y no se pareceria a ninguno de los dos. La mediana se
    /// queda con el color mayoritario, que es el de la caja.
    fn fondo_local(&self, px: u32, py: u32, radio: u32) -> u8 {
        let x0 = px.saturating_sub(radio);
        let x1 = (px + radio).min(self.width - 1);
        let y0 = py.saturating_sub(radio);
        let y1 = (py + radio).min(self.height - 1);

        let mut v: Vec<u8> = Vec::new();
        for y in (y0..=y1).step_by(2) {
            for x in (x0..=x1).step_by(2) {
                v.push(self.luma(x, y));
            }
        }
        if v.is_empty() {
            return self.luma(px, py);
        }
        v.sort_unstable();
        v[v.len() / 2]
    }

    /// Pixel de fondo mas cercano al punto, buscando en anillos concentricos.
    ///
    /// Si apuntas al centro de un kanji, el punto de partida esta sobre tinta;
    /// desde ahi cualquier expansion se para en el primer pixel. Hay que salir
    /// primero de la letra.
    fn buscar_semilla(&self, px: u32, py: u32, fondo: u8, tolerancia: u8) -> Option<(u32, u32)> {
        if self.luma(px, py).abs_diff(fondo) <= tolerancia {
            return Some((px, py));
        }

        // 60 px cubre de sobra el trazo de una letra grande.
        for radio in 1..=60u32 {
            for (dx, dy) in [
                (radio as i32, 0i32),
                (-(radio as i32), 0),
                (0, radio as i32),
                (0, -(radio as i32)),
            ] {
                let x = px as i32 + dx;
                let y = py as i32 + dy;
                if x < 0 || y < 0 || x as u32 >= self.width || y as u32 >= self.height {
                    continue;
                }
                let (x, y) = (x as u32, y as u32);
                if self.luma(x, y).abs_diff(fondo) <= tolerancia {
                    return Some((x, y));
                }
            }
        }
        None
    }

    fn luma(&self, x: u32, y: u32) -> u8 {
        let p = self.pixel(x, y);
        ((p[2] as u32 * 299 + p[1] as u32 * 587 + p[0] as u32 * 114) / 1000) as u8
    }

    /// Mediana de luminancia de un tramo de fila. Se muestrea de 4 en 4: con mil
    /// pixeles por fila y cientos de filas, mirarlos todos no compensa.
    fn mediana_fila(&self, y: u32, x0: u32, x1: u32) -> u8 {
        let mut v: Vec<u8> =
            (x0..=x1.min(self.width - 1)).step_by(4).map(|x| self.luma(x, y)).collect();
        if v.is_empty() {
            return 0;
        }
        v.sort_unstable();
        v[v.len() / 2]
    }

    /// Lo mismo por columnas.
    fn mediana_columna(&self, x: u32, y0: u32, y1: u32) -> u8 {
        let mut v: Vec<u8> =
            (y0..=y1.min(self.height - 1)).step_by(4).map(|y| self.luma(x, y)).collect();
        if v.is_empty() {
            return 0;
        }
        v.sort_unstable();
        v[v.len() / 2]
    }

    /// Mezcla la referencia con el valor nuevo. Peso bajo para el nuevo: la
    /// referencia sigue el degradado sin dejarse arrastrar por una letra.
    fn adaptar(referencia: u8, nuevo: u8) -> u8 {
        ((referencia as u32 * 7 + nuevo as u32) / 8) as u8
    }

    /// Ancho maximo de "cosa que no es fondo" que se puede cruzar sin dar la
    /// caja por terminada. Una letra grande cabe; el borde del cuadro, seguido
    /// del fondo del juego, no.
    const SALTO_MAX: u32 = 90;

    fn expandir_horizontal(&self, px: u32, py: u32, tolerancia: u8) -> (u32, u32) {
        let mut izq = px;
        let mut ultimo_bueno = px;
        let mut referencia = self.luma(px, py);
        let mut fallos = 0u32;
        while izq > 0 && fallos < Self::SALTO_MAX {
            izq -= 1;
            let v = self.luma(izq, py);
            if v.abs_diff(referencia) > tolerancia {
                fallos += 1;
            } else {
                fallos = 0;
                ultimo_bueno = izq;
                referencia = Self::adaptar(referencia, v);
            }
        }
        let izq = ultimo_bueno;

        let mut der = px;
        ultimo_bueno = px;
        referencia = self.luma(px, py);
        fallos = 0;
        while der + 1 < self.width && fallos < Self::SALTO_MAX {
            der += 1;
            let v = self.luma(der, py);
            if v.abs_diff(referencia) > tolerancia {
                fallos += 1;
            } else {
                fallos = 0;
                ultimo_bueno = der;
                referencia = Self::adaptar(referencia, v);
            }
        }

        (izq, ultimo_bueno)
    }

    fn expandir_vertical(&self, py: u32, izq: u32, der: u32, tolerancia: u8) -> (u32, u32) {
        let mut arriba = py;
        let mut referencia = self.mediana_fila(py, izq, der);
        while arriba > 0 {
            let v = self.mediana_fila(arriba - 1, izq, der);
            if v.abs_diff(referencia) > tolerancia {
                break;
            }
            referencia = Self::adaptar(referencia, v);
            arriba -= 1;
        }

        let mut abajo = py;
        referencia = self.mediana_fila(py, izq, der);
        while abajo + 1 < self.height {
            let v = self.mediana_fila(abajo + 1, izq, der);
            if v.abs_diff(referencia) > tolerancia {
                break;
            }
            referencia = Self::adaptar(referencia, v);
            abajo += 1;
        }

        (arriba, abajo)
    }

    /// Segunda pasada a lo ancho, ya con la altura conocida: recupera los
    /// extremos que la primera pasada se dejo por tropezar con una letra.
    fn refinar_horizontal(
        &self,
        izq: u32,
        der: u32,
        arriba: u32,
        abajo: u32,
        tolerancia: u8,
    ) -> (u32, u32) {
        let mut i = izq;
        let mut referencia = self.mediana_columna(izq, arriba, abajo);
        while i > 0 {
            let v = self.mediana_columna(i - 1, arriba, abajo);
            if v.abs_diff(referencia) > tolerancia {
                break;
            }
            referencia = Self::adaptar(referencia, v);
            i -= 1;
        }

        let mut d = der;
        referencia = self.mediana_columna(der, arriba, abajo);
        while d + 1 < self.width {
            let v = self.mediana_columna(d + 1, arriba, abajo);
            if v.abs_diff(referencia) > tolerancia {
                break;
            }
            referencia = Self::adaptar(referencia, v);
            d += 1;
        }

        (i, d)
    }

    /// Escalado por vecino mas cercano.
    ///
    /// El OCR de Windows falla con tipografias pequenas o pixel-art; duplicar o
    /// triplicar el tamano sube muchisimo el acierto y es barato. Nearest en vez
    /// de bilineal a proposito: no queremos suavizar los bordes del pixel-art.
    pub fn upscale_nearest(&self, factor: u32) -> Frame {
        if factor <= 1 {
            return self.clone();
        }
        let nw = self.width * factor;
        let nh = self.height * factor;
        let mut out = vec![0u8; nw as usize * nh as usize * 4];

        for y in 0..nh {
            let sy = y / factor;
            let fila_origen = (sy * self.width) as usize * 4;
            let fila_destino = (y * nw) as usize * 4;
            for x in 0..nw {
                let sx = x / factor;
                let si = fila_origen + sx as usize * 4;
                let di = fila_destino + x as usize * 4;
                out[di..di + 4].copy_from_slice(&self.data[si..si + 4]);
            }
        }

        Frame { width: nw, height: nh, data: out }
    }

    /// Binariza a blanco y negro con un umbral de luminancia.
    ///
    /// `invert` sirve para el caso habitual en juegos: texto claro sobre caja
    /// oscura. El OCR acierta mas con texto oscuro sobre fondo claro.
    pub fn binarize(&self, threshold: u8, invert: bool) -> Frame {
        let mut out = self.data.clone();

        for px in out.chunks_exact_mut(4) {
            // Coeficientes ITU-R BT.601 en enteros para no depender de floats.
            let luma = (px[2] as u32 * 299 + px[1] as u32 * 587 + px[0] as u32 * 114) / 1000;
            let mut claro = luma as u8 >= threshold;
            if invert {
                claro = !claro;
            }
            let v = if claro { 255 } else { 0 };
            px[0] = v;
            px[1] = v;
            px[2] = v;
            px[3] = 255;
        }

        Frame { width: self.width, height: self.height, data: out }
    }

    /// Invierte los tres canales de color y deja el alfa opaco.
    ///
    /// Es la transformacion mas suave que existe para el caso tipico de juego
    /// (texto claro sobre caja oscura): a diferencia de binarizar, no se pierde
    /// el suavizado de los bordes de las letras.
    pub fn invert(&self) -> Frame {
        let mut out = self.data.clone();
        for px in out.chunks_exact_mut(4) {
            px[0] = 255 - px[0];
            px[1] = 255 - px[1];
            px[2] = 255 - px[2];
            px[3] = 255;
        }
        Frame { width: self.width, height: self.height, data: out }
    }

    /// Color dominante del fotograma, cuantizado a 5 bits por canal.
    ///
    /// Sirve para pintar el parche que tapa el texto original del mismo color
    /// que la caja de dialogo del juego. Se usa la moda y no la media porque la
    /// media de "caja azul oscura + letras blancas" da un gris que no se parece
    /// a ninguno de los dos.
    pub fn dominant_color(&self) -> crate::overlay::Rgb {
        use std::collections::HashMap;
        let mut cuenta: HashMap<crate::overlay::Rgb, u32> = HashMap::new();

        for px in self.data.chunks_exact(4) {
            let clave = (px[2] >> 3, px[1] >> 3, px[0] >> 3);
            *cuenta.entry(clave).or_insert(0) += 1;
        }

        match cuenta.into_iter().max_by_key(|(_, n)| *n) {
            // Se devuelve el centro del cubo de color, no su esquina.
            Some(((r, g, b), _)) => ((r << 3) | 4, (g << 3) | 4, (b << 3) | 4),
            None => (0, 0, 0),
        }
    }

    /// Luminancia media (0-255). Se usa para decidir automaticamente si el texto
    /// va claro sobre oscuro o al reves.
    pub fn mean_luma(&self) -> u8 {
        if self.data.is_empty() {
            return 0;
        }
        let total: u64 = self
            .data
            .chunks_exact(4)
            .map(|px| (px[2] as u64 * 299 + px[1] as u64 * 587 + px[0] as u64 * 114) / 1000)
            .sum();
        (total / (self.data.len() / 4) as u64) as u8
    }

    /// Serializa a BMP de 32 bits sin compresion.
    ///
    /// Es formato suficiente para `hakoyaku dump`, que existe para que puedas
    /// mirar con tus ojos exactamente lo que esta viendo el OCR.
    pub fn to_bmp(&self) -> Vec<u8> {
        let pixel_bytes = self.data.len();
        let file_size = 14 + 40 + pixel_bytes;
        let mut out = Vec::with_capacity(file_size);

        // BITMAPFILEHEADER
        out.extend_from_slice(b"BM");
        out.extend_from_slice(&(file_size as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes()); // reservado
        out.extend_from_slice(&54u32.to_le_bytes()); // offset a los pixeles

        // BITMAPINFOHEADER
        out.extend_from_slice(&40u32.to_le_bytes());
        out.extend_from_slice(&(self.width as i32).to_le_bytes());
        out.extend_from_slice(&(self.height as i32).to_le_bytes()); // positivo = bottom-up
        out.extend_from_slice(&1u16.to_le_bytes()); // planos
        out.extend_from_slice(&32u16.to_le_bytes()); // bits por pixel
        out.extend_from_slice(&0u32.to_le_bytes()); // BI_RGB
        out.extend_from_slice(&(pixel_bytes as u32).to_le_bytes());
        out.extend_from_slice(&2835i32.to_le_bytes()); // ~72 ppp
        out.extend_from_slice(&2835i32.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());

        // Pixeles: el BMP bottom-up guarda la ultima fila primero.
        let stride = self.width as usize * 4;
        for y in (0..self.height as usize).rev() {
            out.extend_from_slice(&self.data[y * stride..(y + 1) * stride]);
        }

        out
    }
}

/// Cuantos bloques de dos firmas difieren mas de `tolerancia`.
///
/// Firmas de distinto tamano se consideran totalmente distintas.
pub fn bloques_distintos(a: &[u8], b: &[u8], tolerancia: u8) -> usize {
    if a.len() != b.len() {
        return a.len().max(b.len());
    }
    a.iter().zip(b.iter()).filter(|(x, y)| x.abs_diff(**y) > tolerancia).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame_2x2() -> Frame {
        // BGRA: rojo, verde, azul, blanco
        let data = vec![
            0, 0, 255, 255, //
            0, 255, 0, 255, //
            255, 0, 0, 255, //
            255, 255, 255, 255,
        ];
        Frame::new(2, 2, data).unwrap()
    }

    #[test]
    fn rechaza_buffer_de_tamano_incorrecto() {
        assert!(Frame::new(2, 2, vec![0; 10]).is_err());
        assert!(Frame::new(2, 2, vec![0; 16]).is_ok());
    }

    /// Pinta una "caja de dialogo": rectangulo claro sobre fondo oscuro, con
    /// unas cuantas rayas oscuras dentro haciendo de texto.
    fn con_caja(ancho: u32, alto: u32, caja: Region, con_texto: bool) -> Frame {
        let mut f = Frame::blank(ancho, alto);
        for y in caja.y as u32..(caja.y as u32 + caja.height) {
            for x in caja.x as u32..(caja.x as u32 + caja.width) {
                let i = ((y * ancho + x) * 4) as usize;
                f.data[i] = 200;
                f.data[i + 1] = 200;
                f.data[i + 2] = 200;
                f.data[i + 3] = 255;
            }
        }
        if con_texto {
            // Dos renglones de "letras" que ocupan un tercio del ancho.
            for renglon in 0..2u32 {
                let y = caja.y as u32 + 8 + renglon * 10;
                for x in (caja.x as u32 + 5..caja.x as u32 + caja.width - 5).step_by(3) {
                    for dy in 0..4 {
                        let i = (((y + dy) * ancho + x) * 4) as usize;
                        f.data[i] = 20;
                        f.data[i + 1] = 20;
                        f.data[i + 2] = 20;
                    }
                }
            }
        }
        f
    }

    /// El caso que fallaba en el juego real: apuntar justo encima de una letra.
    /// Es lo natural —senalas el texto que quieres traducir— y hacia que la
    /// deteccion muriera en el primer pixel.
    #[test]
    fn detecta_la_caja_aunque_apuntes_a_una_letra() {
        let caja = Region { x: 20, y: 30, width: 140, height: 46 };
        let f = con_caja(220, 130, caja, true);

        // (35, 38) cae sobre uno de los trazos oscuros del "texto".
        let sobre_letra = f.caja_desde_punto(35, 38, 20);
        assert!(sobre_letra.is_some(), "apuntar a una letra no puede romper la deteccion");

        let d = sobre_letra.unwrap();
        assert!(d.width >= caja.width - 12, "ancho: {d:?} vs {caja:?}");
        assert!(d.height >= caja.height - 12, "alto: {d:?} vs {caja:?}");
    }

    #[test]
    fn detecta_una_caja_limpia_desde_un_punto_de_dentro() {
        let caja = Region { x: 20, y: 30, width: 100, height: 40 };
        let f = con_caja(200, 120, caja, false);
        let d = f.caja_desde_punto(70, 50, 20).unwrap();
        assert_eq!(d, caja);
    }

    #[test]
    fn detecta_la_caja_aunque_tenga_texto_encima() {
        let caja = Region { x: 20, y: 30, width: 120, height: 46 };
        let f = con_caja(200, 120, caja, true);
        let d = f.caja_desde_punto(80, 33, 20).unwrap();

        // Con texto dentro los bordes bailan un poco; basta con que encuadre.
        assert!((d.x - caja.x).abs() <= 4, "izquierda: {d:?}");
        assert!((d.y - caja.y).abs() <= 4, "arriba: {d:?}");
        assert!(d.width >= caja.width - 8 && d.width <= caja.width + 4, "ancho: {d:?}");
        assert!(d.height >= caja.height - 8, "alto: {d:?}");
    }

    #[test]
    fn un_punto_fuera_del_frame_no_detecta_nada() {
        let f = con_caja(200, 120, Region { x: 20, y: 30, width: 100, height: 40 }, false);
        assert!(f.caja_desde_punto(500, 50, 20).is_none());
        assert!(f.caja_desde_punto(70, 900, 20).is_none());
    }

    /// Limitacion conocida y aceptada: si la "caja" es mas pequena que el
    /// vecindario que se mira para estimar el fondo, la mediana sale del fondo
    /// que la rodea y lo que se detecta es ese fondo, no la caja.
    ///
    /// Con cuadros de dialogo de verdad no ocurre —son mucho mayores— y a
    /// cambio se gana poder apuntar directamente a las letras. Lo que
    /// detecta esto en el pipeline es `cursor::es_demasiado_grande`, que
    /// descarta las cajas que ocupan casi toda la zona de busqueda.
    #[test]
    fn una_caja_mas_pequena_que_el_vecindario_detecta_el_fondo() {
        let f = con_caja(200, 120, Region { x: 20, y: 30, width: 10, height: 6 }, false);
        let d = f.caja_desde_punto(24, 32, 20).unwrap();
        assert!(
            d.width > 100 && d.height > 60,
            "deberia haber detectado el fondo entero, no la cajita: {d:?}"
        );
    }

    /// El caso que rompia el detector anterior: un boton con fondo en degradado
    /// sobre un fondo de juego casi igual de oscuro, separados por un borde
    /// claro. Con tolerancia de color plano no hay forma; siguiendo el
    /// degradado y parando en el salto, si.
    #[test]
    fn detecta_un_boton_con_degradado_sobre_fondo_oscuro() {
        let (ancho, alto) = (240u32, 120u32);
        let caja = Region { x: 30, y: 30, width: 160, height: 50 };
        let mut f = Frame::blank(ancho, alto);

        // Fondo del juego: casi negro.
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
                    // Borde claro, como el naranja de los botones del juego.
                    f.data[i] = 60;
                    f.data[i + 1] = 170;
                    f.data[i + 2] = 210;
                } else {
                    // Degradado verde oscuro: de 30 a 70 de arriba abajo.
                    let t = (y - caja.y as u32) * 40 / caja.height;
                    f.data[i] = 25 + t as u8;
                    f.data[i + 1] = 45 + t as u8;
                    f.data[i + 2] = 20;
                }
            }
        }

        let d = f.caja_desde_punto(110, 55, 24).unwrap();
        assert!((d.x - caja.x).abs() <= 3, "izquierda mal: {d:?} vs {caja:?}");
        assert!((d.y - caja.y).abs() <= 3, "arriba mal: {d:?} vs {caja:?}");
        assert!(
            d.width >= caja.width - 6 && d.width <= caja.width + 6,
            "ancho mal: {d:?} vs {caja:?}"
        );
        assert!(
            d.height >= caja.height - 6 && d.height <= caja.height + 6,
            "alto mal: {d:?} vs {caja:?}"
        );
    }

    #[test]
    fn en_un_frame_uniforme_la_caja_es_el_frame_entero() {
        let f = Frame::new(64, 32, vec![120; 64 * 32 * 4]).unwrap();
        let d = f.caja_desde_punto(30, 16, 10).unwrap();
        assert_eq!(d, Region { x: 0, y: 0, width: 64, height: 32 });
    }

    #[test]
    fn la_firma_tiene_un_valor_por_bloque() {
        let f = Frame::blank(8, 4);
        assert_eq!(f.signature(4, 2).len(), 8);
        assert_eq!(f.signature(1, 1).len(), 1);
    }

    #[test]
    fn la_firma_no_pide_mas_bloques_que_pixeles() {
        let f = Frame::blank(2, 2);
        assert_eq!(f.signature(50, 50).len(), 4);
    }

    #[test]
    fn la_firma_promedia_el_brillo_del_bloque() {
        // Mitad negra, mitad blanca, en un solo bloque -> gris medio.
        let mut data = vec![0u8; 4 * 4];
        for px in data.chunks_exact_mut(4).skip(2) {
            px[0] = 255;
            px[1] = 255;
            px[2] = 255;
        }
        let f = Frame::new(4, 1, data).unwrap();
        let v = f.signature(1, 1)[0];
        assert!((100..=155).contains(&v), "gris medio esperado, salio {v}");
    }

    #[test]
    fn el_ruido_de_fondo_no_cuenta_como_cambio() {
        // Una particula de dos pixeles sobre un fondo grande: al promediar por
        // bloques, el brillo del bloque apenas se mueve.
        let a = Frame::blank(64, 64);
        let mut b = a.clone();
        for i in 0..2 {
            let px = i * 4;
            b.data[px] = 255;
            b.data[px + 1] = 255;
            b.data[px + 2] = 255;
        }
        // 8x8 bloques sobre 64x64 = bloques de 64 pixeles, proporcion parecida
        // a la real (una region de 1000x760 con la rejilla por defecto da
        // bloques de unos 3000 pixeles).
        let distintos = bloques_distintos(&a.signature(8, 8), &b.signature(8, 8), 10);
        assert_eq!(distintos, 0, "dos pixeles no deberian mover ningun bloque");
        // Pero el hash exacto si cambia: por eso no vale para esto.
        assert_ne!(a.fingerprint(), b.fingerprint());
    }

    #[test]
    fn un_cambio_de_texto_si_mueve_varios_bloques() {
        let a = Frame::blank(64, 64);
        let mut b = a.clone();
        // Una banda blanca ancha, como una linea de texto nueva.
        for y in 20..30u32 {
            for x in 0..64u32 {
                let i = ((y * 64 + x) * 4) as usize;
                b.data[i] = 255;
                b.data[i + 1] = 255;
                b.data[i + 2] = 255;
            }
        }
        let distintos = bloques_distintos(&a.signature(8, 8), &b.signature(8, 8), 10);
        assert!(distintos >= 8, "una linea entera deberia mover muchos bloques, movio {distintos}");
    }

    #[test]
    fn firmas_de_distinto_tamano_son_totalmente_distintas() {
        assert_eq!(bloques_distintos(&[1, 2, 3], &[1, 2], 0), 3);
    }

    #[test]
    fn el_fingerprint_es_estable_e_ignora_alfa() {
        let a = frame_2x2();
        let mut b = a.clone();
        assert_eq!(a.fingerprint(), b.fingerprint());

        b.data[3] = 0; // solo cambia el alfa del primer pixel
        assert_eq!(a.fingerprint(), b.fingerprint());

        b.data[0] = 7; // ahora cambia el canal azul
        assert_ne!(a.fingerprint(), b.fingerprint());
    }

    #[test]
    fn el_fingerprint_distingue_dimensiones() {
        let a = Frame::blank(4, 1);
        let b = Frame::blank(1, 4);
        assert_ne!(a.fingerprint(), b.fingerprint());
    }

    #[test]
    fn el_escalado_duplica_dimensiones_y_replica_pixeles() {
        let a = frame_2x2();
        let b = a.upscale_nearest(2);
        assert_eq!((b.width, b.height), (4, 4));
        // El pixel (0,0) original ocupa ahora el bloque (0..2, 0..2).
        assert_eq!(b.pixel(0, 0), a.pixel(0, 0));
        assert_eq!(b.pixel(1, 1), a.pixel(0, 0));
        assert_eq!(b.pixel(2, 0), a.pixel(1, 0));
        assert_eq!(b.pixel(0, 2), a.pixel(0, 1));
    }

    #[test]
    fn el_factor_uno_no_toca_nada() {
        let a = frame_2x2();
        assert_eq!(a.upscale_nearest(1), a);
        assert_eq!(a.upscale_nearest(0), a);
    }

    #[test]
    fn binariza_con_umbral() {
        let a = frame_2x2();
        let b = a.binarize(128, false);
        // El blanco queda blanco; el azul puro (luma 29) queda negro.
        assert_eq!(b.pixel(1, 1), [255, 255, 255, 255]);
        assert_eq!(b.pixel(0, 1), [0, 0, 0, 255]);
    }

    #[test]
    fn binarizar_invertido_da_el_complemento() {
        let a = frame_2x2();
        let normal = a.binarize(128, false);
        let invertido = a.binarize(128, true);
        for (n, i) in normal.data.chunks_exact(4).zip(invertido.data.chunks_exact(4)) {
            assert_eq!(n[0], 255 - i[0]);
        }
    }

    #[test]
    fn invertir_dos_veces_devuelve_el_original() {
        let a = frame_2x2();
        assert_eq!(a.invert().invert(), a);
    }

    #[test]
    fn invertir_cambia_negro_por_blanco() {
        let negro = Frame::new(1, 1, vec![0, 0, 0, 255]).unwrap();
        assert_eq!(negro.invert().pixel(0, 0), [255, 255, 255, 255]);
    }

    #[test]
    fn el_color_dominante_ignora_la_minoria() {
        // Nueve pixeles de "caja azul oscura" y uno de "letra blanca".
        let mut data = Vec::new();
        for _ in 0..9 {
            data.extend_from_slice(&[64, 16, 16, 255]); // BGRA -> azul oscuro
        }
        data.extend_from_slice(&[255, 255, 255, 255]);
        let f = Frame::new(10, 1, data).unwrap();

        let (r, g, b) = f.dominant_color();
        assert!(b > r && b > g, "deberia salir azulado, salio {r},{g},{b}");
        assert!(r < 40 && g < 40, "deberia salir oscuro, salio {r},{g},{b}");
    }

    #[test]
    fn el_color_dominante_de_un_frame_uniforme_es_ese_color() {
        let f = Frame::new(2, 1, vec![10, 20, 30, 255, 10, 20, 30, 255]).unwrap();
        let (r, g, b) = f.dominant_color();
        // Cuantizado a 5 bits, asi que se admite un error de +-4.
        assert!(
            (r as i32 - 30).abs() <= 4 && (g as i32 - 20).abs() <= 4 && (b as i32 - 10).abs() <= 4,
            "salio {r},{g},{b}"
        );
    }

    #[test]
    fn luma_media_de_negro_y_blanco() {
        assert_eq!(Frame::blank(4, 4).mean_luma(), 0);
        let blanco = Frame::new(1, 1, vec![255, 255, 255, 255]).unwrap();
        assert_eq!(blanco.mean_luma(), 255);
    }

    #[test]
    fn el_bmp_tiene_cabecera_y_tamano_correctos() {
        let a = frame_2x2();
        let bmp = a.to_bmp();
        assert_eq!(&bmp[0..2], b"BM");
        assert_eq!(bmp.len(), 14 + 40 + 16);
        assert_eq!(u32::from_le_bytes(bmp[2..6].try_into().unwrap()), bmp.len() as u32);
        assert_eq!(u32::from_le_bytes(bmp[10..14].try_into().unwrap()), 54);
        assert_eq!(i32::from_le_bytes(bmp[18..22].try_into().unwrap()), 2);
        assert_eq!(i32::from_le_bytes(bmp[22..26].try_into().unwrap()), 2);
        assert_eq!(u16::from_le_bytes(bmp[28..30].try_into().unwrap()), 32);
    }

    #[test]
    fn el_bmp_guarda_las_filas_de_abajo_arriba() {
        let a = frame_2x2();
        let bmp = a.to_bmp();
        // La primera fila del fichero debe ser la ultima del frame.
        assert_eq!(&bmp[54..62], &a.data[8..16]);
        assert_eq!(&bmp[62..70], &a.data[0..8]);
    }
}
