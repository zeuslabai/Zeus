use anyhow::Result;
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::info;

use crate::session::LspSession;

/// A single LSP-backed tool (hover, definition, or references)
pub struct LspTool {
    pub name: String,
    pub description: String,
    pub method: &'static str,
    pub session: Arc<Mutex<LspSession>>,
}

impl LspTool {
    pub fn schema(&self) -> Value {
        json!({
            "name": self.name,
            "description": self.description,
            "parameters": {
                "type": "object",
                "properties": {
                    "uri": {
                        "type": "string",
                        "description": "File URI, e.g. file:///path/to/file.rs"
                    },
                    "line": {
                        "type": "integer",
                        "description": "Zero-based line number"
                    },
                    "character": {
                        "type": "integer",
                        "description": "Zero-based character offset"
                    }
                },
                "required": ["uri", "line", "character"]
            }
        })
    }

    pub async fn execute(&self, args: &Value) -> Result<Value> {
        let uri = args["uri"].as_str().unwrap_or("").to_string();
        let line = args["line"].as_u64().unwrap_or(0);
        let character = args["character"].as_u64().unwrap_or(0);

        let params = json!({
            "textDocument": { "uri": uri },
            "position": { "line": line, "character": character }
        });

        // References needs includeDeclaration
        let params = if self.method == "textDocument/references" {
            let mut p = params;
            p["context"] = json!({ "includeDeclaration": true });
            p
        } else {
            params
        };

        let mut session = self.session.lock().await;
        let result = session.request(self.method, params).await?;
        info!("LSP tool {} result: {}", self.name, result);
        Ok(result)
    }
}

/// Build all tools for a named LSP session
pub fn build_lsp_tools(server_name: &str, session: Arc<Mutex<LspSession>>) -> Vec<LspTool> {
    let slug = server_name.replace('-', "_").replace('.', "_");

    vec![
        LspTool {
            name: format!("lsp_hover_{}", slug),
            description: format!("Get hover info (type, docs) from {} at a position", server_name),
            method: "textDocument/hover",
            session: session.clone(),
        },
        LspTool {
            name: format!("lsp_definition_{}", slug),
            description: format!("Go to definition using {} — returns file URI + position", server_name),
            method: "textDocument/definition",
            session: session.clone(),
        },
        LspTool {
            name: format!("lsp_references_{}", slug),
            description: format!("Find all references using {} — returns list of locations", server_name),
            method: "textDocument/references",
            session,
        },
    ]
}
