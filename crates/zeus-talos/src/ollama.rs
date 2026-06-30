//! Ollama tools — HTTP API wrappers for a local Ollama daemon.
//!
//! Talks to `http://localhost:11434` by default (overridable via the
//! `OLLAMA_HOST` env var). Provides:
//! - `ollama_pull`  — pull a model
//! - `ollama_list`  — list local models
//! - `ollama_rm`    — delete a local model
//! - `ollama_show`  — show model details (modelfile, parameters, template)
//! - `ollama_ps`    — list currently-loaded (running) models
//!
//! Reference: https://github.com/ollama/ollama/blob/main/docs/api.md

use crate::TalosTool;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::time::Duration;
use zeus_core::{Error, Result, ToolSchema};

const DEFAULT_HOST: &str = "http://localhost:11434";
const PULL_TIMEOUT_SECS: u64 = 60 * 60; // 1 hour — pulls can be huge
const QUICK_TIMEOUT_SECS: u64 = 30;

fn host() -> String {
    std::env::var("OLLAMA_HOST")
        .ok()
        .filter(|s| !s.is_empty())
        .map(|h| {
            // accept "host:port" without scheme for compatibility with ollama's own conventions
            if h.starts_with("http://") || h.starts_with("https://") {
                h
            } else {
                format!("http://{}", h)
            }
        })
        .unwrap_or_else(|| DEFAULT_HOST.to_string())
}

/// Validate a model name. Ollama accepts forms like `llama3.2`, `qwen3:7b`,
/// `registry.example/foo/bar:tag`. We allow alnum + `_-./:`.
fn validate_model(name: &str) -> Result<()> {
    if name.is_empty() {
        return Err(Error::Tool("model name must not be empty".into()));
    }
    if name.len() > 256 {
        return Err(Error::Tool("model name too long".into()));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | ':' | '/'))
    {
        return Err(Error::Tool(
            "model name contains invalid characters".into(),
        ));
    }
    Ok(())
}

fn client(timeout_secs: u64) -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(timeout_secs))
        .build()
        .map_err(|e| Error::Tool(format!("failed to build http client: {}", e)))
}

async fn json_request(
    method: reqwest::Method,
    path: &str,
    body: Option<Value>,
    timeout_secs: u64,
) -> Result<Value> {
    let url = format!("{}{}", host(), path);
    let c = client(timeout_secs)?;
    let mut req = c.request(method, &url);
    if let Some(b) = body {
        req = req.json(&b);
    }
    let resp = req
        .send()
        .await
        .map_err(|e| Error::Tool(format!("ollama request failed ({}): {}", url, e)))?;

    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| Error::Tool(format!("failed to read ollama response: {}", e)))?;

    if !status.is_success() {
        return Err(Error::Tool(format!(
            "ollama returned {} for {}: {}",
            status,
            path,
            text.trim()
        )));
    }

    if text.trim().is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_str(&text)
        .map_err(|e| Error::Tool(format!("invalid JSON from ollama: {} (body: {})", e, text)))
}

// ---------- ollama_pull ----------

/// Pull a model from the Ollama registry.
pub struct OllamaPullTool;

#[async_trait]
impl TalosTool for OllamaPullTool {
    fn name(&self) -> &'static str {
        "ollama_pull"
    }
    fn description(&self) -> &'static str {
        "Pull (download) a model into the local Ollama daemon"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("model", "string", "Model name, e.g. 'llama3.2' or 'qwen3:7b'", true)
            .with_param(
                "insecure",
                "boolean",
                "Allow insecure connections to the registry (default false)",
                false,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let model = args
            .get("model")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing model".into()))?;
        validate_model(model)?;

        let insecure = args
            .get("insecure")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        // Use stream=false so the API blocks until the pull completes and returns
        // a single status JSON. Avoids needing to parse the streaming NDJSON.
        let body = json!({
            "model": model,
            "insecure": insecure,
            "stream": false,
        });
        let v = json_request(
            reqwest::Method::POST,
            "/api/pull",
            Some(body),
            PULL_TIMEOUT_SECS,
        )
        .await?;

        let status = v
            .get("status")
            .and_then(|s| s.as_str())
            .unwrap_or("completed");
        Ok(format!("pulled {}: {}", model, status))
    }
}

// ---------- ollama_list ----------

/// List locally-installed models.
pub struct OllamaListTool;

#[async_trait]
impl TalosTool for OllamaListTool {
    fn name(&self) -> &'static str {
        "ollama_list"
    }
    fn description(&self) -> &'static str {
        "List models installed in the local Ollama daemon"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
    }

    async fn execute(&self, _args: Value) -> Result<String> {
        let v = json_request(reqwest::Method::GET, "/api/tags", None, QUICK_TIMEOUT_SECS).await?;

        let empty: Vec<Value> = vec![];
        let models = v.get("models").and_then(|m| m.as_array()).unwrap_or(&empty);

        if models.is_empty() {
            return Ok("(no models installed)".into());
        }

        let mut out = String::new();
        out.push_str("NAME\tSIZE\tMODIFIED\tDIGEST\n");
        for m in models {
            let name = m.get("name").and_then(|v| v.as_str()).unwrap_or("?");
            let size = m.get("size").and_then(|v| v.as_u64()).unwrap_or(0);
            let modified = m
                .get("modified_at")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let digest = m
                .get("digest")
                .and_then(|v| v.as_str())
                .map(|d| {
                    let end = d.len().min(12);
                    &d[..end]
                })
                .unwrap_or("?");
            out.push_str(&format!(
                "{}\t{}\t{}\t{}\n",
                name,
                human_size(size),
                modified,
                digest
            ));
        }
        Ok(out)
    }
}

fn human_size(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut v = bytes as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    format!("{:.1} {}", v, UNITS[i])
}

// ---------- ollama_rm ----------

/// Delete a local model.
pub struct OllamaRmTool;

#[async_trait]
impl TalosTool for OllamaRmTool {
    fn name(&self) -> &'static str {
        "ollama_rm"
    }
    fn description(&self) -> &'static str {
        "Delete a model from the local Ollama daemon"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "model",
            "string",
            "Model name to delete (e.g. 'llama3.2:7b')",
            true,
        )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let model = args
            .get("model")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing model".into()))?;
        validate_model(model)?;

        let body = json!({ "model": model });
        json_request(
            reqwest::Method::DELETE,
            "/api/delete",
            Some(body),
            QUICK_TIMEOUT_SECS,
        )
        .await?;
        Ok(format!("deleted {}", model))
    }
}

// ---------- ollama_show ----------

/// Show details of a model (modelfile, parameters, template, license).
pub struct OllamaShowTool;

#[async_trait]
impl TalosTool for OllamaShowTool {
    fn name(&self) -> &'static str {
        "ollama_show"
    }
    fn description(&self) -> &'static str {
        "Show metadata for a local model (parameters, template, modelfile)"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("model", "string", "Model name", true)
            .with_param(
                "verbose",
                "boolean",
                "Include full modelfile and template (default false)",
                false,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let model = args
            .get("model")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing model".into()))?;
        validate_model(model)?;
        let verbose = args
            .get("verbose")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let body = json!({ "model": model, "verbose": verbose });
        let v = json_request(
            reqwest::Method::POST,
            "/api/show",
            Some(body),
            QUICK_TIMEOUT_SECS,
        )
        .await?;

        // The show endpoint returns a fairly verbose object. Pretty-print it
        // for human consumption — verbose flag already controls payload size.
        serde_json::to_string_pretty(&v)
            .map_err(|e| Error::Tool(format!("failed to serialize show response: {}", e)))
    }
}

// ---------- ollama_ps ----------

/// List currently-running (loaded into memory) models.
pub struct OllamaPsTool;

#[async_trait]
impl TalosTool for OllamaPsTool {
    fn name(&self) -> &'static str {
        "ollama_ps"
    }
    fn description(&self) -> &'static str {
        "List models currently loaded in memory by the Ollama daemon"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
    }

    async fn execute(&self, _args: Value) -> Result<String> {
        let v = json_request(reqwest::Method::GET, "/api/ps", None, QUICK_TIMEOUT_SECS).await?;

        let empty: Vec<Value> = vec![];
        let models = v.get("models").and_then(|m| m.as_array()).unwrap_or(&empty);

        if models.is_empty() {
            return Ok("(no models currently loaded)".into());
        }

        let mut out = String::new();
        out.push_str("NAME\tSIZE\tEXPIRES\tDIGEST\n");
        for m in models {
            let name = m.get("name").and_then(|v| v.as_str()).unwrap_or("?");
            let size = m.get("size").and_then(|v| v.as_u64()).unwrap_or(0);
            let expires = m
                .get("expires_at")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let digest = m
                .get("digest")
                .and_then(|v| v.as_str())
                .map(|d| {
                    let end = d.len().min(12);
                    &d[..end]
                })
                .unwrap_or("?");
            out.push_str(&format!(
                "{}\t{}\t{}\t{}\n",
                name,
                human_size(size),
                expires,
                digest
            ));
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_model_names() {
        assert!(validate_model("llama3.2").is_ok());
        assert!(validate_model("qwen3:7b").is_ok());
        assert!(validate_model("registry.io/library/foo:bar").is_ok());
        assert!(validate_model("").is_err());
        assert!(validate_model("bad name").is_err());
        assert!(validate_model("bad;rm").is_err());
        assert!(validate_model("bad$(x)").is_err());
    }

    // Pure helper extracted for testability without mutating process env
    // (parallel tests + edition-2024 unsafe set_var make env mutation flaky).
    fn host_from(env_val: Option<&str>) -> String {
        env_val
            .filter(|s| !s.is_empty())
            .map(|h| {
                if h.starts_with("http://") || h.starts_with("https://") {
                    h.to_string()
                } else {
                    format!("http://{}", h)
                }
            })
            .unwrap_or_else(|| DEFAULT_HOST.to_string())
    }

    #[test]
    fn host_defaults_to_localhost() {
        assert_eq!(host_from(None), "http://localhost:11434");
        assert_eq!(host_from(Some("")), "http://localhost:11434");
    }

    #[test]
    fn host_adds_scheme_if_missing() {
        assert_eq!(host_from(Some("remote:11434")), "http://remote:11434");
        assert_eq!(
            host_from(Some("http://x.example:11434")),
            "http://x.example:11434"
        );
        assert_eq!(
            host_from(Some("https://x.example:11434")),
            "https://x.example:11434"
        );
    }

    #[test]
    fn human_size_formats() {
        assert_eq!(human_size(0), "0.0 B");
        assert_eq!(human_size(1024), "1.0 KB");
        assert_eq!(human_size(1024 * 1024), "1.0 MB");
        assert_eq!(human_size(2u64 * 1024 * 1024 * 1024), "2.0 GB");
    }

    #[test]
    fn pull_schema_requires_model() {
        let s = OllamaPullTool.schema();
        assert_eq!(s.name, "ollama_pull");
        let required = s
            .parameters
            .get("required")
            .and_then(|r| r.as_array())
            .expect("required array");
        assert!(required.iter().any(|v| v.as_str() == Some("model")));
    }

    #[test]
    fn list_and_ps_have_no_required_args() {
        for name in ["ollama_list", "ollama_ps"] {
            let s = if name == "ollama_list" {
                OllamaListTool.schema()
            } else {
                OllamaPsTool.schema()
            };
            let required = s
                .parameters
                .get("required")
                .and_then(|r| r.as_array())
                .expect("required array");
            assert!(required.is_empty(), "{} should have no required args", name);
        }
    }
}
