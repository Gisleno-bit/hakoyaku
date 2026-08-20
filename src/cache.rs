//! Cache de traducciones.
//!
//! En un juego relees el mismo menu cien veces, y vuelves sobre el mismo
//! dialogo cada vez que el cuadro parpadea. Sin cache eso son cien llamadas a
//! la API por la misma frase. Con cache es una.
//!
//! Politica FIFO en vez de LRU a proposito: es la mitad de codigo, no necesita
//! `unsafe` ni una lista doblemente enlazada, y para este patron de uso (texto
//! que aparece en rafagas y no vuelve) el acierto es practicamente el mismo.

use std::collections::{HashMap, VecDeque};

#[derive(Debug)]
pub struct TranslationCache {
    map: HashMap<String, String>,
    orden: VecDeque<String>,
    capacidad: usize,
    aciertos: u64,
    fallos: u64,
}

impl TranslationCache {
    pub fn new(capacidad: usize) -> Self {
        Self {
            map: HashMap::new(),
            orden: VecDeque::new(),
            capacidad: capacidad.max(1),
            aciertos: 0,
            fallos: 0,
        }
    }

    pub fn get(&mut self, clave: &str) -> Option<&str> {
        match self.map.get(clave) {
            Some(v) => {
                self.aciertos += 1;
                Some(v.as_str())
            }
            None => {
                self.fallos += 1;
                None
            }
        }
    }

    pub fn insert(&mut self, clave: String, valor: String) {
        // Un solo lookup: si ya estaba, se actualiza en sitio y el orden no cambia.
        if let Some(hueco) = self.map.get_mut(&clave) {
            *hueco = valor;
            return;
        }
        while self.map.len() >= self.capacidad {
            match self.orden.pop_front() {
                Some(vieja) => {
                    self.map.remove(&vieja);
                }
                None => break,
            }
        }
        self.orden.push_back(clave.clone());
        self.map.insert(clave, valor);
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// (aciertos, fallos) desde que arranco el programa.
    pub fn stats(&self) -> (u64, u64) {
        (self.aciertos, self.fallos)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guarda_y_recupera() {
        let mut c = TranslationCache::new(4);
        assert!(c.get("こんにちは").is_none());
        c.insert("こんにちは".into(), "hola".into());
        assert_eq!(c.get("こんにちは"), Some("hola"));
    }

    #[test]
    fn expulsa_la_entrada_mas_antigua_al_llenarse() {
        let mut c = TranslationCache::new(2);
        c.insert("a".into(), "1".into());
        c.insert("b".into(), "2".into());
        c.insert("c".into(), "3".into());

        assert_eq!(c.len(), 2);
        assert!(c.get("a").is_none());
        assert_eq!(c.get("b"), Some("2"));
        assert_eq!(c.get("c"), Some("3"));
    }

    #[test]
    fn reinsertar_actualiza_sin_crecer() {
        let mut c = TranslationCache::new(2);
        c.insert("a".into(), "1".into());
        c.insert("a".into(), "1-corregido".into());
        assert_eq!(c.len(), 1);
        assert_eq!(c.get("a"), Some("1-corregido"));
    }

    #[test]
    fn la_capacidad_cero_se_trata_como_uno() {
        let mut c = TranslationCache::new(0);
        c.insert("a".into(), "1".into());
        c.insert("b".into(), "2".into());
        assert_eq!(c.len(), 1);
        assert_eq!(c.get("b"), Some("2"));
    }

    #[test]
    fn cuenta_aciertos_y_fallos() {
        let mut c = TranslationCache::new(4);
        c.insert("a".into(), "1".into());
        c.get("a");
        c.get("a");
        c.get("z");
        assert_eq!(c.stats(), (2, 1));
    }

    #[test]
    fn empieza_vacia() {
        let c = TranslationCache::new(8);
        assert!(c.is_empty());
        assert_eq!(c.len(), 0);
    }
}
