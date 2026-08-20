//! Google Cloud Translation v2 con clave de API.

use super::{agent, describe_http_error, Translator};
use crate::config::Translate as TranslateCfg;
use crate::text::unescape_html;
use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::time::Duration;

const ENDPOINT: &str = "https://translation.googleapis.com/language/translate/v2";

pub struct Google {
    api_key: String,
    endpoint: String,
    source: String,
    target: String,
    agent: ureq::Agent,
}

pub fn request_body(text: &str, source: &str, target: &str) -> Value {
    let mut body = json!({
        "q": text,
        "target": target,
        "format": "text",
    });
    if !source.trim().is_empty() && source != "auto" {
        body["source"] = json!(source);
    }
    body
}

pub fn parse_response(v: &Value) -> Result<String> {
    let bruto = v
        .get("data")
        .and_then(|d| d.get("translations"))
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .and_then(|t| t.get("translatedText"))
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("respuesta de Google inesperada: {v}"))?;
    // Google escapa entidades HTML incluso con format=text.
    Ok(unescape_html(bruto))
}

impl Google {
    pub fn new(cfg: &TranslateCfg, timeout: Duration) -> Self {
        let endpoint = if cfg.endpoint.trim().is_empty() {
            ENDPOINT.to_string()
        } else {
            cfg.endpoint.trim().to_string()
        };
        Self {
            api_key: cfg.api_key.trim().to_string(),
            endpoint,
            source: cfg.source_lang.clone(),
            target: cfg.target_lang.clone(),
            agent: agent(timeout),
        }
    }
}

impl Translator for Google {
    fn translate(&self, text: &str) -> Result<String> {
        let body = request_body(text, &self.source, &self.target);
        let resp = self
            .agent
            .post(&self.endpoint)
            .query("key", &self.api_key)
            .set("Content-Type", "application/json")
            .send_json(body)
            .map_err(|e| describe_http_error("Google Translate", e))?;
        let v: Value = resp.into_json()?;
        parse_response(&v)
    }

    fn name(&self) -> &'static str {
        "Google Translate"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn el_cuerpo_pide_texto_plano() {
        let b = request_body("こんにちは", "ja", "es");
        assert_eq!(b["q"], "こんにちは");
        assert_eq!(b["source"], "ja");
        assert_eq!(b["target"], "es");
        assert_eq!(b["format"], "text");
    }

    #[test]
    fn con_auto_no_se_manda_source() {
        assert!(request_body("hola", "auto", "ja").get("source").is_none());
    }

    #[test]
    fn lee_y_desescapa_la_respuesta() {
        let v: Value = serde_json::from_str(
            r#"{"data":{"translations":[{"translatedText":"¿Qu&#39;e tal? a &amp; b"}]}}"#,
        )
        .unwrap();
        assert_eq!(parse_response(&v).unwrap(), "¿Qu'e tal? a & b");
    }

    #[test]
    fn un_error_de_google_no_hace_panic() {
        let v: Value =
            serde_json::from_str(r#"{"error":{"code":403,"message":"API key not valid"}}"#)
                .unwrap();
        assert!(parse_response(&v).is_err());
    }
}
