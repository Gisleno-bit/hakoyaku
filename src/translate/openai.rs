//! Cualquier API con formato OpenAI `/v1/chat/completions`: Ollama y LM Studio
//! en local, o la propia OpenAI.
//!
//! Para dialogo de videojuego suele dar la traduccion mas natural de los cuatro
//! backends, porque puedes decirle en el prompt que es un juego y que no te
//! ponga notas del traductor. A cambio va mas lento.

use super::{agent, describe_http_error, Translator};
use crate::config::Translate as TranslateCfg;
use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::time::Duration;

const ENDPOINT: &str = "http://localhost:11434/v1/chat/completions";

pub struct OpenAiCompatible {
    endpoint: String,
    api_key: String,
    model: String,
    source: String,
    target: String,
    agent: ureq::Agent,
}

/// Acepta `http://host:11434`, `.../v1` o la ruta completa.
pub fn normalize_endpoint(raw: &str) -> String {
    let e = raw.trim().trim_end_matches('/');
    if e.is_empty() {
        return ENDPOINT.to_string();
    }
    if e.ends_with("/chat/completions") {
        e.to_string()
    } else if e.ends_with("/v1") {
        format!("{e}/chat/completions")
    } else {
        format!("{e}/v1/chat/completions")
    }
}

pub fn system_prompt(source: &str, target: &str) -> String {
    format!(
        "Traduces texto extraido por OCR de la pantalla de un videojuego, de {source} a {target}.\n\
         Reglas:\n\
         - Devuelve UNICAMENTE la traduccion. Sin comillas, sin notas, sin explicaciones.\n\
         - Manten el registro y el tono del original (narracion, dialogo, menu).\n\
         - Conserva los saltos de linea del original.\n\
         - Los nombres propios se dejan como estan salvo que tengan traduccion asentada.\n\
         - Si el texto llega cortado o con errores de OCR, traduce lo que se entienda \
           sin inventar contenido que no este."
    )
}

pub fn request_body(text: &str, model: &str, source: &str, target: &str) -> Value {
    json!({
        "model": model,
        "temperature": 0.2,
        "stream": false,
        "messages": [
            { "role": "system", "content": system_prompt(source, target) },
            { "role": "user", "content": text },
        ],
    })
}

/// Los modelos pequenos tienden a envolver la respuesta en comillas o a poner
/// "Traduccion:" delante. Se lo quitamos.
pub fn clean_model_output(raw: &str) -> String {
    let mut s = raw.trim();

    for prefijo in ["Traduccion:", "Traducción:", "Translation:", "翻訳:"] {
        if let Some(resto) = s.strip_prefix(prefijo) {
            s = resto.trim_start();
        }
    }

    let chars: Vec<char> = s.chars().collect();
    if chars.len() >= 2 {
        let primero = chars[0];
        let ultimo = chars[chars.len() - 1];
        let emparejadas =
            matches!((primero, ultimo), ('"', '"') | ('\'', '\'') | ('「', '」') | ('“', '”'));
        if emparejadas {
            return chars[1..chars.len() - 1].iter().collect::<String>().trim().to_string();
        }
    }

    s.to_string()
}

pub fn parse_response(v: &Value) -> Result<String> {
    let contenido = v
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("respuesta del modelo inesperada: {v}"))?;
    Ok(clean_model_output(contenido))
}

impl OpenAiCompatible {
    pub fn new(cfg: &TranslateCfg, timeout: Duration) -> Self {
        Self {
            endpoint: normalize_endpoint(&cfg.endpoint),
            api_key: cfg.api_key.trim().to_string(),
            model: if cfg.model.trim().is_empty() {
                "qwen2.5:7b".into()
            } else {
                cfg.model.clone()
            },
            source: cfg.source_lang.clone(),
            target: cfg.target_lang.clone(),
            agent: agent(timeout),
        }
    }
}

impl Translator for OpenAiCompatible {
    fn translate(&self, text: &str) -> Result<String> {
        let body = request_body(text, &self.model, &self.source, &self.target);
        let mut req = self.agent.post(&self.endpoint).set("Content-Type", "application/json");
        if !self.api_key.is_empty() {
            req = req.set("Authorization", &format!("Bearer {}", self.api_key));
        }
        let resp = req.send_json(body).map_err(|e| describe_http_error("el modelo", e))?;
        let v: Value = resp.into_json()?;
        parse_response(&v)
    }

    fn name(&self) -> &'static str {
        "API compatible con OpenAI"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completa_el_endpoint_desde_cualquier_forma() {
        assert_eq!(normalize_endpoint(""), "http://localhost:11434/v1/chat/completions");
        assert_eq!(normalize_endpoint("http://pc:1234"), "http://pc:1234/v1/chat/completions");
        assert_eq!(normalize_endpoint("http://pc:1234/v1"), "http://pc:1234/v1/chat/completions");
        assert_eq!(
            normalize_endpoint("http://pc:1234/v1/chat/completions"),
            "http://pc:1234/v1/chat/completions"
        );
    }

    #[test]
    fn el_cuerpo_lleva_modelo_y_dos_mensajes() {
        let b = request_body("こんにちは", "gemma3:12b", "japones", "castellano");
        assert_eq!(b["model"], "gemma3:12b");
        assert_eq!(b["stream"], false);
        assert_eq!(b["messages"][0]["role"], "system");
        assert_eq!(b["messages"][1]["role"], "user");
        assert_eq!(b["messages"][1]["content"], "こんにちは");
        assert!(b["messages"][0]["content"].as_str().unwrap().contains("castellano"));
    }

    #[test]
    fn quita_prefijos_habituales() {
        assert_eq!(clean_model_output("Traduccion: Hola"), "Hola");
        assert_eq!(clean_model_output("Translation: Hello"), "Hello");
        assert_eq!(clean_model_output("  Hola  "), "Hola");
    }

    #[test]
    fn quita_comillas_emparejadas() {
        assert_eq!(clean_model_output("\"Hola mundo\""), "Hola mundo");
        assert_eq!(clean_model_output("「こんにちは」"), "こんにちは");
    }

    #[test]
    fn no_toca_comillas_desemparejadas() {
        assert_eq!(clean_model_output("\"Hola"), "\"Hola");
        assert_eq!(clean_model_output("El dijo \"vale\" y se fue"), "El dijo \"vale\" y se fue");
    }

    #[test]
    fn lee_la_respuesta_del_modelo() {
        let v: Value = serde_json::from_str(
            r#"{"choices":[{"message":{"role":"assistant","content":"Sales a un espacio extraño."}}]}"#,
        )
        .unwrap();
        assert_eq!(parse_response(&v).unwrap(), "Sales a un espacio extraño.");
    }

    #[test]
    fn una_respuesta_sin_choices_da_error() {
        let v: Value = serde_json::from_str(r#"{"error":{"message":"model not found"}}"#).unwrap();
        assert!(parse_response(&v).is_err());
    }
}
