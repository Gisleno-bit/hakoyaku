//! LibreTranslate. La opcion sin cuentas ni tarjetas: lo levantas en Docker en
//! tu propio PC y no sale nada a internet. La calidad con japones es peor que
//! DeepL, pero es gratis y offline.

use super::{agent, describe_http_error, Translator};
use crate::config::Translate as TranslateCfg;
use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::time::Duration;

const ENDPOINT: &str = "http://localhost:5000/translate";

pub struct Libre {
    endpoint: String,
    api_key: String,
    source: String,
    target: String,
    agent: ureq::Agent,
}

/// Acepta tanto `http://host:5000` como `http://host:5000/translate`.
pub fn normalize_endpoint(raw: &str) -> String {
    let e = raw.trim().trim_end_matches('/');
    if e.is_empty() {
        return ENDPOINT.to_string();
    }
    if e.ends_with("/translate") {
        e.to_string()
    } else {
        format!("{e}/translate")
    }
}

pub fn request_body(text: &str, source: &str, target: &str, api_key: &str) -> Value {
    let origen = if source.trim().is_empty() { "auto" } else { source };
    let mut body = json!({
        "q": text,
        "source": origen,
        "target": target,
        "format": "text",
    });
    if !api_key.trim().is_empty() {
        body["api_key"] = json!(api_key.trim());
    }
    body
}

pub fn parse_response(v: &Value) -> Result<String> {
    if let Some(t) = v.get("translatedText").and_then(Value::as_str) {
        return Ok(t.to_string());
    }
    if let Some(e) = v.get("error").and_then(Value::as_str) {
        return Err(anyhow!("LibreTranslate: {e}"));
    }
    Err(anyhow!("respuesta de LibreTranslate inesperada: {v}"))
}

impl Libre {
    pub fn new(cfg: &TranslateCfg, timeout: Duration) -> Self {
        Self {
            endpoint: normalize_endpoint(&cfg.endpoint),
            api_key: cfg.api_key.clone(),
            source: cfg.source_lang.clone(),
            target: cfg.target_lang.clone(),
            agent: agent(timeout),
        }
    }
}

impl Translator for Libre {
    fn translate(&self, text: &str) -> Result<String> {
        let body = request_body(text, &self.source, &self.target, &self.api_key);
        let resp = self
            .agent
            .post(&self.endpoint)
            .set("Content-Type", "application/json")
            .send_json(body)
            .map_err(|e| describe_http_error("LibreTranslate", e))?;
        let v: Value = resp.into_json()?;
        parse_response(&v)
    }

    fn name(&self) -> &'static str {
        "LibreTranslate"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completa_el_endpoint() {
        assert_eq!(normalize_endpoint(""), "http://localhost:5000/translate");
        assert_eq!(normalize_endpoint("http://pc:5000"), "http://pc:5000/translate");
        assert_eq!(normalize_endpoint("http://pc:5000/"), "http://pc:5000/translate");
        assert_eq!(normalize_endpoint("http://pc:5000/translate"), "http://pc:5000/translate");
    }

    #[test]
    fn sin_clave_no_se_manda_el_campo() {
        let b = request_body("あ", "ja", "es", "");
        assert!(b.get("api_key").is_none());

        let b = request_body("あ", "ja", "es", "secreto");
        assert_eq!(b["api_key"], "secreto");
    }

    #[test]
    fn el_origen_vacio_se_convierte_en_auto() {
        assert_eq!(request_body("a", "", "es", "")["source"], "auto");
    }

    #[test]
    fn lee_la_respuesta() {
        let v: Value = serde_json::from_str(r#"{"translatedText":"Hola"}"#).unwrap();
        assert_eq!(parse_response(&v).unwrap(), "Hola");
    }

    #[test]
    fn propaga_el_error_del_servidor() {
        let v: Value = serde_json::from_str(r#"{"error":"Invalid target language"}"#).unwrap();
        let e = parse_response(&v).unwrap_err().to_string();
        assert!(e.contains("Invalid target language"), "{e}");
    }
}
