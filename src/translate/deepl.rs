//! DeepL. Es el que mejor sale para japones -> castellano y tiene plan gratuito
//! de 500.000 caracteres al mes, que con la cache da para muchisimo juego.

use super::{agent, describe_http_error, Translator};
use crate::config::Translate as TranslateCfg;
use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::time::Duration;

pub struct DeepL {
    api_key: String,
    endpoint: String,
    source: String,
    target: String,
    agent: ureq::Agent,
}

/// Las claves del plan gratuito acaban en `:fx` y van a otro host.
pub fn endpoint_for_key(api_key: &str, override_endpoint: &str) -> String {
    if !override_endpoint.trim().is_empty() {
        return override_endpoint.trim().to_string();
    }
    if api_key.trim().ends_with(":fx") {
        "https://api-free.deepl.com/v2/translate".into()
    } else {
        "https://api.deepl.com/v2/translate".into()
    }
}

pub fn request_body(text: &str, source: &str, target: &str) -> Value {
    let mut body = json!({
        "text": [text],
        "target_lang": target.to_uppercase(),
        // El texto de juego viene con saltos de linea por como cabe en la caja,
        // no por como esta escrito. Que DeepL los ignore mejora la traduccion.
        "split_sentences": "nonewlines",
        "preserve_formatting": false,
    });
    if !source.trim().is_empty() && source != "auto" {
        body["source_lang"] = json!(source.to_uppercase());
    }
    body
}

pub fn parse_response(v: &Value) -> Result<String> {
    v.get("translations")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .and_then(|t| t.get("text"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| anyhow!("respuesta de DeepL inesperada: {v}"))
}

impl DeepL {
    pub fn new(cfg: &TranslateCfg, timeout: Duration) -> Self {
        Self {
            endpoint: endpoint_for_key(&cfg.api_key, &cfg.endpoint),
            api_key: cfg.api_key.trim().to_string(),
            source: cfg.source_lang.clone(),
            target: cfg.target_lang.clone(),
            agent: agent(timeout),
        }
    }
}

impl Translator for DeepL {
    fn translate(&self, text: &str) -> Result<String> {
        let body = request_body(text, &self.source, &self.target);
        let resp = self
            .agent
            .post(&self.endpoint)
            .set("Authorization", &format!("DeepL-Auth-Key {}", self.api_key))
            .set("Content-Type", "application/json")
            .send_json(body)
            .map_err(|e| describe_http_error("DeepL", e))?;
        let v: Value = resp.into_json()?;
        parse_response(&v)
    }

    fn name(&self) -> &'static str {
        "DeepL"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn las_claves_free_van_al_host_free() {
        assert_eq!(endpoint_for_key("aaaa-bbbb:fx", ""), "https://api-free.deepl.com/v2/translate");
        assert_eq!(endpoint_for_key("aaaa-bbbb", ""), "https://api.deepl.com/v2/translate");
    }

    #[test]
    fn el_endpoint_manual_gana() {
        assert_eq!(
            endpoint_for_key("x:fx", "http://localhost:8080/v2/translate"),
            "http://localhost:8080/v2/translate"
        );
    }

    #[test]
    fn el_cuerpo_lleva_los_idiomas_en_mayusculas() {
        let b = request_body("こんにちは", "ja", "es");
        assert_eq!(b["text"][0], "こんにちは");
        assert_eq!(b["source_lang"], "JA");
        assert_eq!(b["target_lang"], "ES");
        assert_eq!(b["split_sentences"], "nonewlines");
    }

    #[test]
    fn con_origen_auto_no_se_manda_source_lang() {
        let b = request_body("hola", "auto", "en");
        assert!(b.get("source_lang").is_none());

        let b = request_body("hola", "", "en");
        assert!(b.get("source_lang").is_none());
    }

    #[test]
    fn lee_la_traduccion_de_la_respuesta() {
        let v: Value = serde_json::from_str(
            r#"{"translations":[{"detected_source_language":"JA","text":"Hola mundo"}]}"#,
        )
        .unwrap();
        assert_eq!(parse_response(&v).unwrap(), "Hola mundo");
    }

    #[test]
    fn una_respuesta_rara_da_error_en_vez_de_panic() {
        let v: Value = serde_json::from_str(r#"{"message":"Wrong endpoint"}"#).unwrap();
        assert!(parse_response(&v).is_err());

        let v: Value = serde_json::from_str(r#"{"translations":[]}"#).unwrap();
        assert!(parse_response(&v).is_err());
    }
}
