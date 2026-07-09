use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout};
use tokio::time::{timeout, Duration};
use tracing::{debug, warn};

use crate::config::LspServerConfig;

pub struct LspSession {
    pub name: String,
    _child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: i64,
}

impl LspSession {
    pub async fn spawn(name: &str, config: &LspServerConfig) -> Result<Self> {
        let mut cmd = tokio::process::Command::new(&config.command);
        cmd.args(&config.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());

        if let Some(cwd) = &config.cwd {
            cmd.current_dir(cwd);
        }
        for (k, v) in &config.env {
            cmd.env(k, v);
        }

        let mut child = cmd.spawn()?;
        let stdin = child.stdin.take().ok_or_else(|| anyhow!("no stdin"))?;
        let stdout = child.stdout.take().ok_or_else(|| anyhow!("no stdout"))?;

        let mut session = Self {
            name: name.to_string(),
            _child: child,
            stdin,
            stdout: BufReader::new(stdout),
            next_id: 1,
        };

        session.initialize().await?;
        Ok(session)
    }

    async fn initialize(&mut self) -> Result<()> {
        let init_params = json!({
            "processId": std::process::id(),
            "clientInfo": { "name": "zeus-lsp", "version": env!("CARGO_PKG_VERSION") },
            "rootUri": null,
            "capabilities": {
                "textDocument": {
                    "hover": { "contentFormat": ["plaintext"] },
                    "definition": {},
                    "references": {}
                }
            }
        });

        self.request("initialize", init_params).await?;
        self.notify("initialized", json!({})).await?;
        Ok(())
    }

    pub async fn request(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;

        let msg = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        });

        self.send(&msg).await?;

        // Wait up to 10s for response
        timeout(Duration::from_secs(10), self.recv_response(id))
            .await
            .map_err(|_| anyhow!("LSP request timed out: {}", method))?
    }

    async fn notify(&mut self, method: &str, params: Value) -> Result<()> {
        let msg = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params
        });
        self.send(&msg).await
    }

    async fn send(&mut self, msg: &Value) -> Result<()> {
        let body = serde_json::to_string(msg)?;
        let header = format!("Content-Length: {}\r\n\r\n", body.len());
        self.stdin.write_all(header.as_bytes()).await?;
        self.stdin.write_all(body.as_bytes()).await?;
        self.stdin.flush().await?;
        debug!("LSP → {}: {}", self.name, body);
        Ok(())
    }

    async fn recv_response(&mut self, expected_id: i64) -> Result<Value> {
        loop {
            // Read Content-Length header
            let mut header = String::new();
            let mut content_length: usize = 0;

            loop {
                header.clear();
                self.stdout.read_line(&mut header).await?;
                let trimmed = header.trim();
                if trimmed.is_empty() {
                    break;
                }
                if let Some(val) = trimmed.strip_prefix("Content-Length: ") {
                    content_length = val.parse()?;
                }
            }

            if content_length == 0 {
                warn!("LSP {} sent empty response", self.name);
                continue;
            }

            let mut body = vec![0u8; content_length];
            use tokio::io::AsyncReadExt;
            self.stdout.read_exact(&mut body).await?;
            let msg: Value = serde_json::from_slice(&body)?;
            debug!("LSP ← {}: {}", self.name, msg);

            // Skip notifications (no "id")
            if msg.get("id").is_none() {
                continue;
            }

            let id = msg["id"].as_i64().unwrap_or(-1);
            if id != expected_id {
                warn!("LSP {} unexpected id {} (expected {})", self.name, id, expected_id);
                continue;
            }

            if let Some(err) = msg.get("error") {
                return Err(anyhow!("LSP error: {}", err));
            }

            return Ok(msg["result"].clone());
        }
    }

    pub async fn shutdown(&mut self) -> Result<()> {
        self.request("shutdown", json!(null)).await?;
        self.notify("exit", json!(null)).await?;
        Ok(())
    }
}
