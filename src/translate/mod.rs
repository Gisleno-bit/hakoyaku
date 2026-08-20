//! Backends de traduccion.
//!
//! Cada backend separa tres cosas:
//!   1. construir el cuerpo de la peticion  (puro -> testeable)
//!   2. mandarla por HTTP                   (no testeable sin red)
//!   3. interpretar la respuesta            (puro -> testeable)
//!
//! Asi el 80% de la logica que puede tener bugs se testea sin tocar la red.

pub mod deepl;
pub mod google;
pub mod libre;
pub mod openai;

use crate::config::{Backend, Translate as TranslateCfg};
use anyhow::Result;
use std::time::Duration;

pub trait Translator: Send {
    fn translate(&self, text: &str) -> Result<String>;
    fn name(&self) -> &'static str;
}

/// Backend que devuelve el texto tal cual. Sirve para comprobar que la captura
/// y el OCR funcionan antes de gastar un solo caracter de cuota.
pub struct Passthrough;

impl Translator for Passthrough {
    fn translate(&self, text: &str) -> Result<String> {
        Ok(text.to_string())
    }
    fn name(&self) -> &'static str {
        "none (sin traducir)"
    }
}

pub fn build(cfg: &TranslateCfg) -> Result<Box<dyn Translator>> {
    let timeout = Duration::from_secs(cfg.timeout_secs.max(1));
    Ok(match cfg.backend {
        Backend::Deepl => Box::new(deepl::DeepL::new(cfg, timeout)),
        Backend::Google => Box::new(google::Google::new(cfg, timeout)),
        Backend::Libre => Box::new(libre::Libre::new(cfg, timeout)),
        Backend::Openai => Box::new(openai::OpenAiCompatible::new(cfg, timeout)),
        Backend::None => Box::new(Passthrough),
    })
}

/// Agente HTTP compartido por los backends.
pub(crate) fn agent(timeout: Duration) -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_read(timeout)
        .timeout_write(timeout)
        .timeout_connect(Duration::from_secs(8))
        .user_agent(concat!("hakoyaku/", env!("CARGO_PKG_VERSION")))
        .build()
}

/// Convierte un error de `ureq` en algo que se entienda en el terminal.
pub(crate) fn describe_http_error(backend: &str, e: ureq::Error) -> anyhow::Error {
    match e {
        ureq::Error::Status(code, resp) => {
            let cuerpo = resp.into_string().unwrap_or_default();
            let pista = match code {
                401 | 403 => " (revisa translate.api_key)",
                429 => " (has llegado al limite de peticiones; sube capture.poll_ms)",
                456 => " (cuota de caracteres agotada este mes)",
                _ => "",
            };
            anyhow::anyhow!("{backend} respondio HTTP {code}{pista}: {}", cuerpo.trim())
        }
        otro => anyhow::anyhow!("no se pudo contactar con {backend}: {otro}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passthrough_devuelve_lo_mismo() {
        let t = Passthrough;
        assert_eq!(t.translate("積雪の様に").unwrap(), "積雪の様に");
    }

    #[test]
    fn la_factory_cubre_todos_los_backends() {
        let mut cfg = TranslateCfg { api_key: "clave".into(), ..Default::default() };
        for b in [Backend::Deepl, Backend::Google, Backend::Libre, Backend::Openai, Backend::None] {
            cfg.backend = b;
            assert!(build(&cfg).is_ok(), "fallo construyendo {b:?}");
        }
    }
}
