//! WhatsApp Cloud API tools
//!
//! Provides tools for interacting with the WhatsApp Business Cloud API.
//! Each tool accepts an optional `token` parameter, falling back to the
//! `WHATSAPP_TOKEN` environment variable, and an optional `phone_number_id`
//! parameter, falling back to `WHATSAPP_PHONE_NUMBER_ID`.

use crate::TalosTool;
use async_trait::async_trait;
use serde_json::{Value, json};
use zeus_core::{Error, Result, ToolSchema};

const WHATSAPP_API: &str = "https://graph.facebook.com/v18.0";

/// Get access token from args or environment
fn get_token(args: &Value) -> Result<String> {
    if let Some(token) = args.get("token").and_then(|v| v.as_str()) {
        return Ok(token.to_string());
    }
    std::env::var("WHATSAPP_TOKEN").map_err(|_| {
        Error::Tool("Missing 'token' parameter and WHATSAPP_TOKEN env var not set".to_string())
    })
}

/// Get phone number ID from args or environment
fn get_phone_number_id(args: &Value) -> Result<String> {
    if let Some(id) = args.get("phone_number_id").and_then(|v| v.as_str()) {
        return Ok(id.to_string());
    }
    std::env::var("WHATSAPP_PHONE_NUMBER_ID").map_err(|_| {
        Error::Tool(
            "Missing 'phone_number_id' parameter and WHATSAPP_PHONE_NUMBER_ID env var not set"
                .to_string(),
        )
    })
}

/// Make a WhatsApp Cloud API POST request
async fn whatsapp_post(token: &str, phone_number_id: &str, body: &Value) -> Result<Value> {
    let url = format!("{}/{}/messages", WHATSAPP_API, phone_number_id);
    let client = reqwest::Client::new();

    let response = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", token))
        .header("Content-Type", "application/json")
        .json(body)
        .send()
        .await
        .map_err(|e| Error::Tool(format!("WhatsApp API request failed: {}", e)))?;

    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|e| Error::Tool(format!("Failed to read response: {}", e)))?;

    if !status.is_success() {
        return Err(Error::Tool(format!(
            "WhatsApp API error {}: {}",
            status, text
        )));
    }

    serde_json::from_str(&text).map_err(|e| {
        Error::Tool(format!(
            "Invalid JSON: {} (body: {})",
            e,
            &text[..zeus_core::floor_char_boundary(&text, 200)]
        ))
    })
}

/// Make a WhatsApp Cloud API GET request
async fn whatsapp_get(token: &str, phone_number_id: &str, path: &str) -> Result<Value> {
    let url = format!("{}/{}/{}", WHATSAPP_API, phone_number_id, path);
    let client = reqwest::Client::new();

    let response = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .map_err(|e| Error::Tool(format!("WhatsApp API request failed: {}", e)))?;

    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|e| Error::Tool(format!("Failed to read response: {}", e)))?;

    if !status.is_success() {
        return Err(Error::Tool(format!(
            "WhatsApp API error {}: {}",
            status, text
        )));
    }

    serde_json::from_str(&text).map_err(|e| {
        Error::Tool(format!(
            "Invalid JSON: {} (body: {})",
            e,
            &text[..zeus_core::floor_char_boundary(&text, 200)]
        ))
    })
}

/// Extract the first message ID from a WhatsApp API response
fn extract_message_id(result: &Value) -> &str {
    result
        .get("messages")
        .and_then(|arr| arr.get(0))
        .and_then(|msg| msg.get("id"))
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
}

// ---------------------------------------------------------------------------
// 1. WhatsAppSendMessageTool
// ---------------------------------------------------------------------------

/// Send a text message via WhatsApp Cloud API
pub struct WhatsAppSendMessageTool;

#[async_trait]
impl TalosTool for WhatsAppSendMessageTool {
    fn name(&self) -> &'static str {
        "whatsapp_send_message"
    }
    fn description(&self) -> &'static str {
        "Send a text message via WhatsApp Cloud API"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "to",
                "string",
                "Recipient phone number in E.164 format (e.g. 15551234567)",
                true,
            )
            .with_param("text", "string", "Message text body", true)
            .with_param(
                "token",
                "string",
                "WhatsApp access token (or set WHATSAPP_TOKEN env var)",
                false,
            )
            .with_param(
                "phone_number_id",
                "string",
                "WhatsApp phone number ID (or set WHATSAPP_PHONE_NUMBER_ID env var)",
                false,
            )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let token = get_token(&args)?;
        let phone_number_id = get_phone_number_id(&args)?;

        let to = args
            .get("to")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'to' parameter".to_string()))?;

        let text = args
            .get("text")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'text' parameter".to_string()))?;

        let body = json!({
            "messaging_product": "whatsapp",
            "to": to,
            "type": "text",
            "text": { "body": text }
        });

        let result = whatsapp_post(&token, &phone_number_id, &body).await?;
        let msg_id = extract_message_id(&result);
        Ok(format!("Message sent to {} (id: {})", to, msg_id))
    }
}

// ---------------------------------------------------------------------------
// 2. WhatsAppSendImageTool
// ---------------------------------------------------------------------------

/// Send an image via WhatsApp Cloud API
pub struct WhatsAppSendImageTool;

#[async_trait]
impl TalosTool for WhatsAppSendImageTool {
    fn name(&self) -> &'static str {
        "whatsapp_send_image"
    }
    fn description(&self) -> &'static str {
        "Send an image via WhatsApp Cloud API"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "to",
                "string",
                "Recipient phone number in E.164 format (e.g. 15551234567)",
                true,
            )
            .with_param(
                "image_url",
                "string",
                "Publicly accessible URL of the image",
                true,
            )
            .with_param("caption", "string", "Optional image caption", false)
            .with_param(
                "token",
                "string",
                "WhatsApp access token (or set WHATSAPP_TOKEN env var)",
                false,
            )
            .with_param(
                "phone_number_id",
                "string",
                "WhatsApp phone number ID (or set WHATSAPP_PHONE_NUMBER_ID env var)",
                false,
            )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let token = get_token(&args)?;
        let phone_number_id = get_phone_number_id(&args)?;

        let to = args
            .get("to")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'to' parameter".to_string()))?;

        let image_url = args
            .get("image_url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'image_url' parameter".to_string()))?;

        let mut image_obj = json!({ "link": image_url });
        if let Some(caption) = args.get("caption").and_then(|v| v.as_str()) {
            image_obj["caption"] = json!(caption);
        }

        let body = json!({
            "messaging_product": "whatsapp",
            "to": to,
            "type": "image",
            "image": image_obj
        });

        let result = whatsapp_post(&token, &phone_number_id, &body).await?;
        let msg_id = extract_message_id(&result);
        Ok(format!("Image sent to {} (id: {})", to, msg_id))
    }
}

// ---------------------------------------------------------------------------
// 3. WhatsAppSendDocumentTool
// ---------------------------------------------------------------------------

/// Send a document via WhatsApp Cloud API
pub struct WhatsAppSendDocumentTool;

#[async_trait]
impl TalosTool for WhatsAppSendDocumentTool {
    fn name(&self) -> &'static str {
        "whatsapp_send_document"
    }
    fn description(&self) -> &'static str {
        "Send a document via WhatsApp Cloud API"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "to",
                "string",
                "Recipient phone number in E.164 format (e.g. 15551234567)",
                true,
            )
            .with_param(
                "document_url",
                "string",
                "Publicly accessible URL of the document",
                true,
            )
            .with_param(
                "filename",
                "string",
                "Display filename shown to recipient (optional)",
                false,
            )
            .with_param("caption", "string", "Optional document caption", false)
            .with_param(
                "token",
                "string",
                "WhatsApp access token (or set WHATSAPP_TOKEN env var)",
                false,
            )
            .with_param(
                "phone_number_id",
                "string",
                "WhatsApp phone number ID (or set WHATSAPP_PHONE_NUMBER_ID env var)",
                false,
            )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let token = get_token(&args)?;
        let phone_number_id = get_phone_number_id(&args)?;

        let to = args
            .get("to")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'to' parameter".to_string()))?;

        let document_url = args
            .get("document_url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'document_url' parameter".to_string()))?;

        let mut document_obj = json!({ "link": document_url });
        if let Some(filename) = args.get("filename").and_then(|v| v.as_str()) {
            document_obj["filename"] = json!(filename);
        }
        if let Some(caption) = args.get("caption").and_then(|v| v.as_str()) {
            document_obj["caption"] = json!(caption);
        }

        let body = json!({
            "messaging_product": "whatsapp",
            "to": to,
            "type": "document",
            "document": document_obj
        });

        let result = whatsapp_post(&token, &phone_number_id, &body).await?;
        let msg_id = extract_message_id(&result);
        Ok(format!("Document sent to {} (id: {})", to, msg_id))
    }
}

// ---------------------------------------------------------------------------
// 4. WhatsAppSendTemplateTool
// ---------------------------------------------------------------------------

/// Send a template message via WhatsApp Cloud API
pub struct WhatsAppSendTemplateTool;

#[async_trait]
impl TalosTool for WhatsAppSendTemplateTool {
    fn name(&self) -> &'static str {
        "whatsapp_send_template"
    }
    fn description(&self) -> &'static str {
        "Send a pre-approved template message via WhatsApp Cloud API"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "to",
                "string",
                "Recipient phone number in E.164 format (e.g. 15551234567)",
                true,
            )
            .with_param(
                "template_name",
                "string",
                "Name of the pre-approved WhatsApp message template",
                true,
            )
            .with_param(
                "language",
                "string",
                "Template language code (default: en_US)",
                false,
            )
            .with_param(
                "token",
                "string",
                "WhatsApp access token (or set WHATSAPP_TOKEN env var)",
                false,
            )
            .with_param(
                "phone_number_id",
                "string",
                "WhatsApp phone number ID (or set WHATSAPP_PHONE_NUMBER_ID env var)",
                false,
            )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let token = get_token(&args)?;
        let phone_number_id = get_phone_number_id(&args)?;

        let to = args
            .get("to")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'to' parameter".to_string()))?;

        let template_name = args
            .get("template_name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'template_name' parameter".to_string()))?;

        let language = args
            .get("language")
            .and_then(|v| v.as_str())
            .unwrap_or("en_US");

        let body = json!({
            "messaging_product": "whatsapp",
            "to": to,
            "type": "template",
            "template": {
                "name": template_name,
                "language": { "code": language }
            }
        });

        let result = whatsapp_post(&token, &phone_number_id, &body).await?;
        let msg_id = extract_message_id(&result);
        Ok(format!(
            "Template '{}' sent to {} (id: {})",
            template_name, to, msg_id
        ))
    }
}

// ---------------------------------------------------------------------------
// 5. WhatsAppGetProfileTool
// ---------------------------------------------------------------------------

/// Get the WhatsApp Business profile
pub struct WhatsAppGetProfileTool;

#[async_trait]
impl TalosTool for WhatsAppGetProfileTool {
    fn name(&self) -> &'static str {
        "whatsapp_get_profile"
    }
    fn description(&self) -> &'static str {
        "Get the WhatsApp Business profile for the configured phone number"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "token",
                "string",
                "WhatsApp access token (or set WHATSAPP_TOKEN env var)",
                false,
            )
            .with_param(
                "phone_number_id",
                "string",
                "WhatsApp phone number ID (or set WHATSAPP_PHONE_NUMBER_ID env var)",
                false,
            )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let token = get_token(&args)?;
        let phone_number_id = get_phone_number_id(&args)?;

        let result = whatsapp_get(&token, &phone_number_id, "whatsapp_business_profile").await?;

        // The API returns { "data": [ { ...profile fields... } ] }
        let profile = result
            .get("data")
            .and_then(|arr| arr.get(0))
            .unwrap_or(&result);

        let about = profile
            .get("about")
            .and_then(|v| v.as_str())
            .unwrap_or("(not set)");
        let address = profile
            .get("address")
            .and_then(|v| v.as_str())
            .unwrap_or("(not set)");
        let description = profile
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("(not set)");
        let email = profile
            .get("email")
            .and_then(|v| v.as_str())
            .unwrap_or("(not set)");
        let vertical = profile
            .get("vertical")
            .and_then(|v| v.as_str())
            .unwrap_or("(not set)");
        let websites = profile
            .get("websites")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|u| u.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_else(|| "(not set)".to_string());

        let mut output = format!(
            "WhatsApp Business Profile (phone_number_id: {}):\n",
            phone_number_id
        );
        output.push_str(&format!("  About: {}\n", about));
        output.push_str(&format!("  Address: {}\n", address));
        output.push_str(&format!("  Description: {}\n", description));
        output.push_str(&format!("  Email: {}\n", email));
        output.push_str(&format!("  Industry: {}\n", vertical));
        output.push_str(&format!("  Websites: {}", websites));

        Ok(output)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // Serialize tests that mutate WHATSAPP_TOKEN / WHATSAPP_PHONE_NUMBER_ID env vars.
    static WHATSAPP_ENV_LOCK: Mutex<()> = Mutex::new(());

    // Helper: collect required parameter names from a schema
    fn required_params(schema: &ToolSchema) -> Vec<&str> {
        schema
            .parameters
            .get("required")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_default()
    }

    // Helper: collect all property names from a schema
    fn property_names(schema: &ToolSchema) -> Vec<String> {
        schema
            .parameters
            .get("properties")
            .and_then(|v| v.as_object())
            .map(|obj| obj.keys().cloned().collect())
            .unwrap_or_default()
    }

    #[test]
    fn test_send_message_schema() {
        let tool = WhatsAppSendMessageTool;
        let schema = tool.schema();

        assert_eq!(schema.name, "whatsapp_send_message");
        assert!(!schema.description.is_empty());

        let required = required_params(&schema);
        assert!(required.contains(&"to"), "to must be required");
        assert!(required.contains(&"text"), "text must be required");
        assert!(!required.contains(&"token"), "token must be optional");
        assert!(
            !required.contains(&"phone_number_id"),
            "phone_number_id must be optional"
        );

        let props = property_names(&schema);
        assert!(props.contains(&"to".to_string()));
        assert!(props.contains(&"text".to_string()));
        assert!(props.contains(&"token".to_string()));
        assert!(props.contains(&"phone_number_id".to_string()));
    }

    #[test]
    fn test_send_image_schema() {
        let tool = WhatsAppSendImageTool;
        let schema = tool.schema();

        assert_eq!(schema.name, "whatsapp_send_image");

        let required = required_params(&schema);
        assert!(required.contains(&"to"), "to must be required");
        assert!(
            required.contains(&"image_url"),
            "image_url must be required"
        );
        assert!(!required.contains(&"caption"), "caption must be optional");
        assert!(!required.contains(&"token"), "token must be optional");
        assert!(
            !required.contains(&"phone_number_id"),
            "phone_number_id must be optional"
        );

        let props = property_names(&schema);
        assert!(props.contains(&"caption".to_string()));
        assert!(props.contains(&"image_url".to_string()));
    }

    #[test]
    fn test_send_document_schema() {
        let tool = WhatsAppSendDocumentTool;
        let schema = tool.schema();

        assert_eq!(schema.name, "whatsapp_send_document");

        let required = required_params(&schema);
        assert!(required.contains(&"to"), "to must be required");
        assert!(
            required.contains(&"document_url"),
            "document_url must be required"
        );
        assert!(!required.contains(&"filename"), "filename must be optional");
        assert!(!required.contains(&"caption"), "caption must be optional");
        assert!(!required.contains(&"token"), "token must be optional");
        assert!(
            !required.contains(&"phone_number_id"),
            "phone_number_id must be optional"
        );

        let props = property_names(&schema);
        assert!(props.contains(&"document_url".to_string()));
        assert!(props.contains(&"filename".to_string()));
        assert!(props.contains(&"caption".to_string()));
    }

    #[test]
    fn test_send_template_schema() {
        let tool = WhatsAppSendTemplateTool;
        let schema = tool.schema();

        assert_eq!(schema.name, "whatsapp_send_template");

        let required = required_params(&schema);
        assert!(required.contains(&"to"), "to must be required");
        assert!(
            required.contains(&"template_name"),
            "template_name must be required"
        );
        assert!(!required.contains(&"language"), "language must be optional");
        assert!(!required.contains(&"token"), "token must be optional");
        assert!(
            !required.contains(&"phone_number_id"),
            "phone_number_id must be optional"
        );

        let props = property_names(&schema);
        assert!(props.contains(&"template_name".to_string()));
        assert!(props.contains(&"language".to_string()));
    }

    #[test]
    fn test_get_profile_schema() {
        let tool = WhatsAppGetProfileTool;
        let schema = tool.schema();

        assert_eq!(schema.name, "whatsapp_get_profile");

        let required = required_params(&schema);
        assert!(
            required.is_empty()
                || (!required.contains(&"token") && !required.contains(&"phone_number_id")),
            "get_profile should have no required params; token and phone_number_id must be optional"
        );

        let props = property_names(&schema);
        assert!(props.contains(&"token".to_string()));
        assert!(props.contains(&"phone_number_id".to_string()));
    }

    #[test]
    fn test_get_token_from_args() {
        let args = json!({ "token": "EAAtest123" });
        let token = get_token(&args).expect("should succeed from args");
        assert_eq!(token, "EAAtest123");
    }

    #[test]
    fn test_get_phone_number_id_from_args() {
        let args = json!({ "phone_number_id": "123456789" });
        let id = get_phone_number_id(&args).expect("should succeed from args");
        assert_eq!(id, "123456789");
    }

    #[test]
    fn test_get_token_missing_returns_error() {
        let _guard = WHATSAPP_ENV_LOCK.lock().unwrap();
        unsafe {
            std::env::remove_var("WHATSAPP_TOKEN");
        }
        let args = json!({});
        let result = get_token(&args);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("WHATSAPP_TOKEN"),
            "error should mention env var name: {}",
            msg
        );
    }

    #[test]
    fn test_get_phone_number_id_missing_returns_error() {
        let _guard = WHATSAPP_ENV_LOCK.lock().unwrap();
        unsafe {
            std::env::remove_var("WHATSAPP_PHONE_NUMBER_ID");
        }
        let args = json!({});
        let result = get_phone_number_id(&args);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("WHATSAPP_PHONE_NUMBER_ID"),
            "error should mention env var name: {}",
            msg
        );
    }

    #[test]
    fn test_get_token_from_env() {
        let _guard = WHATSAPP_ENV_LOCK.lock().unwrap();
        unsafe {
            std::env::set_var("WHATSAPP_TOKEN", "env-token-abc");
        }
        let args = json!({});
        let token = get_token(&args).expect("should succeed from env");
        assert_eq!(token, "env-token-abc");
        unsafe {
            std::env::remove_var("WHATSAPP_TOKEN");
        }
    }

    #[test]
    fn test_get_phone_number_id_from_env() {
        let _guard = WHATSAPP_ENV_LOCK.lock().unwrap();
        unsafe {
            std::env::set_var("WHATSAPP_PHONE_NUMBER_ID", "987654321");
        }
        let args = json!({});
        let id = get_phone_number_id(&args).expect("should succeed from env");
        assert_eq!(id, "987654321");
        unsafe {
            std::env::remove_var("WHATSAPP_PHONE_NUMBER_ID");
        }
    }

    #[test]
    fn test_tool_names_are_unique() {
        let names = [
            WhatsAppSendMessageTool.name(),
            WhatsAppSendImageTool.name(),
            WhatsAppSendDocumentTool.name(),
            WhatsAppSendTemplateTool.name(),
            WhatsAppGetProfileTool.name(),
        ];
        let mut seen = std::collections::HashSet::new();
        for name in &names {
            assert!(seen.insert(*name), "duplicate tool name: {}", name);
        }
    }
}
