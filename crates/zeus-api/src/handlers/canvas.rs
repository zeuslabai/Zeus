//! Live Canvas / Action-to-UI (A2UI) Rendering System
//!
//! Converts structured action descriptions into UI component trees.
//! POST /v1/canvas/render accepts a JSON action and returns renderable
//! UI components (buttons, forms, cards, tables, etc.).

use axum::{Json, extract::State, http::StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::SharedState;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Supported UI component types
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ComponentType {
    Text,
    Button,
    Form,
    Table,
    Card,
    CodeBlock,
    Image,
    ChartPlaceholder,
}

/// A single UI component returned by the renderer
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CanvasComponent {
    /// Unique id within the render response
    pub id: String,
    /// Component kind
    #[serde(rename = "type")]
    pub component_type: ComponentType,
    /// Human-readable label / title
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Primary content (text body, code, image URL, etc.)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    /// Key-value properties specific to the component type
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub props: Value,
    /// Ordered child components (for cards / forms)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<CanvasComponent>,
}

/// Description of the action the UI should represent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CanvasAction {
    /// Short verb describing the action (e.g. "confirm_delete", "show_status")
    pub action: String,
    /// Target entity or resource
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    /// Arbitrary parameters the renderer can use
    #[serde(default)]
    pub params: Value,
}

/// Inbound render request
#[derive(Debug, Deserialize)]
pub struct RenderRequest {
    /// The action to render
    pub action: CanvasAction,
    /// Optional layout hint ("compact", "full", "minimal")
    #[serde(default = "default_layout")]
    pub layout: String,
}

fn default_layout() -> String {
    "full".to_string()
}

/// Outbound render response
#[derive(Debug, Serialize)]
pub struct RenderResponse {
    /// Whether rendering succeeded
    pub ok: bool,
    /// Root-level components
    pub components: Vec<CanvasComponent>,
    /// The original action echoed back
    pub action: CanvasAction,
    /// Layout that was applied
    pub layout: String,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_id(prefix: &str, idx: usize) -> String {
    format!("{prefix}_{idx}")
}

/// Render a "confirm" style action → card with description + confirm/cancel buttons
fn render_confirm(action: &CanvasAction) -> Vec<CanvasComponent> {
    let target = action.target.as_deref().unwrap_or("item");
    let message = action
        .params
        .get("message")
        .and_then(|v| v.as_str())
        .unwrap_or("Are you sure?");

    vec![CanvasComponent {
        id: make_id("card", 0),
        component_type: ComponentType::Card,
        label: Some(format!("Confirm: {}", action.action)),
        content: None,
        props: json!({}),
        children: vec![
            CanvasComponent {
                id: make_id("text", 0),
                component_type: ComponentType::Text,
                label: None,
                content: Some(format!("{message} (target: {target})")),
                props: json!({}),
                children: vec![],
            },
            CanvasComponent {
                id: make_id("btn", 0),
                component_type: ComponentType::Button,
                label: Some("Confirm".to_string()),
                content: None,
                props: json!({"variant": "primary", "action": action.action}),
                children: vec![],
            },
            CanvasComponent {
                id: make_id("btn", 1),
                component_type: ComponentType::Button,
                label: Some("Cancel".to_string()),
                content: None,
                props: json!({"variant": "secondary", "action": "cancel"}),
                children: vec![],
            },
        ],
    }]
}

/// Render a "show" action → card with text content
fn render_show(action: &CanvasAction) -> Vec<CanvasComponent> {
    let target = action.target.as_deref().unwrap_or("unknown");
    let body = action
        .params
        .get("body")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    vec![CanvasComponent {
        id: make_id("card", 0),
        component_type: ComponentType::Card,
        label: Some(format!("Details: {target}")),
        content: None,
        props: json!({}),
        children: vec![CanvasComponent {
            id: make_id("text", 0),
            component_type: ComponentType::Text,
            label: None,
            content: Some(body.to_string()),
            props: json!({}),
            children: vec![],
        }],
    }]
}

/// Render a "form" action → form with fields from params.fields
fn render_form(action: &CanvasAction) -> Vec<CanvasComponent> {
    let title = action.target.as_deref().unwrap_or("Form");
    let fields = action
        .params
        .get("fields")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut children: Vec<CanvasComponent> = fields
        .iter()
        .enumerate()
        .map(|(i, f)| {
            let name = f.get("name").and_then(|v| v.as_str()).unwrap_or("field");
            let field_type = f.get("type").and_then(|v| v.as_str()).unwrap_or("text");
            CanvasComponent {
                id: make_id("field", i),
                component_type: ComponentType::Text,
                label: Some(name.to_string()),
                content: None,
                props: json!({"input_type": field_type, "required": f.get("required").and_then(|v| v.as_bool()).unwrap_or(false)}),
                children: vec![],
            }
        })
        .collect();

    children.push(CanvasComponent {
        id: make_id("btn", 0),
        component_type: ComponentType::Button,
        label: Some("Submit".to_string()),
        content: None,
        props: json!({"variant": "primary", "action": "submit"}),
        children: vec![],
    });

    vec![CanvasComponent {
        id: make_id("form", 0),
        component_type: ComponentType::Form,
        label: Some(title.to_string()),
        content: None,
        props: json!({}),
        children,
    }]
}

/// Render a "table" action → table component from params.columns + params.rows
fn render_table(action: &CanvasAction) -> Vec<CanvasComponent> {
    let columns = action.params.get("columns").cloned().unwrap_or(json!([]));
    let rows = action.params.get("rows").cloned().unwrap_or(json!([]));
    let title = action.target.as_deref().unwrap_or("Table");

    vec![CanvasComponent {
        id: make_id("table", 0),
        component_type: ComponentType::Table,
        label: Some(title.to_string()),
        content: None,
        props: json!({"columns": columns, "rows": rows}),
        children: vec![],
    }]
}

/// Render a "code" action → code block
fn render_code(action: &CanvasAction) -> Vec<CanvasComponent> {
    let language = action
        .params
        .get("language")
        .and_then(|v| v.as_str())
        .unwrap_or("text");
    let code = action
        .params
        .get("code")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    vec![CanvasComponent {
        id: make_id("code", 0),
        component_type: ComponentType::CodeBlock,
        label: Some(language.to_string()),
        content: Some(code.to_string()),
        props: json!({"language": language}),
        children: vec![],
    }]
}

/// Render an "image" action → image component
fn render_image(action: &CanvasAction) -> Vec<CanvasComponent> {
    let url = action
        .params
        .get("url")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let alt = action
        .params
        .get("alt")
        .and_then(|v| v.as_str())
        .unwrap_or("image");

    vec![CanvasComponent {
        id: make_id("img", 0),
        component_type: ComponentType::Image,
        label: Some(alt.to_string()),
        content: Some(url.to_string()),
        props: json!({"alt": alt}),
        children: vec![],
    }]
}

/// Render a "chart" action → chart placeholder
fn render_chart(action: &CanvasAction) -> Vec<CanvasComponent> {
    let chart_type = action
        .params
        .get("chart_type")
        .and_then(|v| v.as_str())
        .unwrap_or("bar");
    let title = action.target.as_deref().unwrap_or("Chart");

    vec![CanvasComponent {
        id: make_id("chart", 0),
        component_type: ComponentType::ChartPlaceholder,
        label: Some(title.to_string()),
        content: None,
        props: json!({"chart_type": chart_type, "data": action.params.get("data").cloned().unwrap_or(json!([]))}),
        children: vec![],
    }]
}

/// Core render dispatch — maps action verbs to component trees
fn render_action(action: &CanvasAction) -> Vec<CanvasComponent> {
    let verb = action.action.to_lowercase();

    if verb.starts_with("confirm") || verb.starts_with("approve") || verb.starts_with("delete") {
        render_confirm(action)
    } else if verb.starts_with("show") || verb.starts_with("view") || verb.starts_with("detail") {
        render_show(action)
    } else if verb.starts_with("form") || verb.starts_with("create") || verb.starts_with("edit") {
        render_form(action)
    } else if verb.starts_with("table") || verb.starts_with("list") {
        render_table(action)
    } else if verb.starts_with("code") || verb.starts_with("snippet") {
        render_code(action)
    } else if verb.starts_with("image") || verb.starts_with("photo") {
        render_image(action)
    } else if verb.starts_with("chart") || verb.starts_with("graph") || verb.starts_with("plot") {
        render_chart(action)
    } else {
        // Fallback: text card
        vec![CanvasComponent {
            id: make_id("text", 0),
            component_type: ComponentType::Text,
            label: Some(action.action.clone()),
            content: action.target.clone(),
            props: json!({"params": action.params}),
            children: vec![],
        }]
    }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// POST /v1/canvas/render — Render an action into UI components
pub async fn canvas_render(
    State(_state): State<SharedState>,
    Json(req): Json<RenderRequest>,
) -> Result<Json<RenderResponse>, (StatusCode, Json<Value>)> {
    if req.action.action.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "action.action must not be empty"})),
        ));
    }

    let components = render_action(&req.action);

    Ok(Json(RenderResponse {
        ok: true,
        components,
        action: req.action,
        layout: req.layout,
    }))
}

/// GET /v1/canvas/components — List supported component types
pub async fn canvas_components() -> Json<Value> {
    Json(json!({
        "component_types": [
            "text", "button", "form", "table",
            "card", "code_block", "image", "chart_placeholder"
        ],
        "action_prefixes": {
            "confirm/approve/delete": "Confirmation card with buttons",
            "show/view/detail": "Detail card with text",
            "form/create/edit": "Form with input fields",
            "table/list": "Data table",
            "code/snippet": "Code block",
            "image/photo": "Image display",
            "chart/graph/plot": "Chart placeholder"
        }
    }))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_confirm_action() {
        let action = CanvasAction {
            action: "confirm_delete".to_string(),
            target: Some("agent-007".to_string()),
            params: json!({"message": "Delete this agent?"}),
        };
        let components = render_action(&action);
        assert_eq!(components.len(), 1);
        assert_eq!(components[0].component_type, ComponentType::Card);
        assert_eq!(components[0].children.len(), 3); // text + confirm + cancel
        assert_eq!(
            components[0].children[1].component_type,
            ComponentType::Button
        );
        assert_eq!(components[0].children[1].label.as_deref(), Some("Confirm"));
        assert_eq!(components[0].children[2].label.as_deref(), Some("Cancel"));
    }

    #[test]
    fn test_render_show_action() {
        let action = CanvasAction {
            action: "show_status".to_string(),
            target: Some("server-1".to_string()),
            params: json!({"body": "All systems operational"}),
        };
        let components = render_action(&action);
        assert_eq!(components.len(), 1);
        assert_eq!(components[0].component_type, ComponentType::Card);
        assert!(components[0].label.as_ref().unwrap().contains("server-1"));
        let text = &components[0].children[0];
        assert_eq!(text.content.as_deref(), Some("All systems operational"));
    }

    #[test]
    fn test_render_form_action() {
        let action = CanvasAction {
            action: "create_agent".to_string(),
            target: Some("New Agent".to_string()),
            params: json!({"fields": [
                {"name": "name", "type": "text", "required": true},
                {"name": "model", "type": "select"}
            ]}),
        };
        let components = render_action(&action);
        assert_eq!(components.len(), 1);
        assert_eq!(components[0].component_type, ComponentType::Form);
        // 2 fields + 1 submit button
        assert_eq!(components[0].children.len(), 3);
        assert_eq!(
            components[0].children[2].component_type,
            ComponentType::Button
        );
    }

    #[test]
    fn test_render_table_action() {
        let action = CanvasAction {
            action: "list_agents".to_string(),
            target: Some("Agents".to_string()),
            params: json!({
                "columns": ["id", "name", "status"],
                "rows": [["1", "Zeus", "active"], ["2", "Athena", "idle"]]
            }),
        };
        let components = render_action(&action);
        assert_eq!(components.len(), 1);
        assert_eq!(components[0].component_type, ComponentType::Table);
        assert!(components[0].props.get("columns").is_some());
        assert!(components[0].props.get("rows").is_some());
    }

    #[test]
    fn test_render_code_action() {
        let action = CanvasAction {
            action: "code_review".to_string(),
            target: None,
            params: json!({"language": "rust", "code": "fn main() {}"}),
        };
        let components = render_action(&action);
        assert_eq!(components.len(), 1);
        assert_eq!(components[0].component_type, ComponentType::CodeBlock);
        assert_eq!(components[0].content.as_deref(), Some("fn main() {}"));
    }

    #[test]
    fn test_render_image_action() {
        let action = CanvasAction {
            action: "image_preview".to_string(),
            target: None,
            params: json!({"url": "https://example.com/img.png", "alt": "Screenshot"}),
        };
        let components = render_action(&action);
        assert_eq!(components.len(), 1);
        assert_eq!(components[0].component_type, ComponentType::Image);
        assert_eq!(
            components[0].content.as_deref(),
            Some("https://example.com/img.png")
        );
    }

    #[test]
    fn test_render_chart_action() {
        let action = CanvasAction {
            action: "chart_costs".to_string(),
            target: Some("Monthly Costs".to_string()),
            params: json!({"chart_type": "line", "data": [10, 20, 15]}),
        };
        let components = render_action(&action);
        assert_eq!(components.len(), 1);
        assert_eq!(
            components[0].component_type,
            ComponentType::ChartPlaceholder
        );
        assert_eq!(components[0].label.as_deref(), Some("Monthly Costs"));
    }

    #[test]
    fn test_render_unknown_falls_back_to_text() {
        let action = CanvasAction {
            action: "unknown_verb".to_string(),
            target: Some("something".to_string()),
            params: json!({}),
        };
        let components = render_action(&action);
        assert_eq!(components.len(), 1);
        assert_eq!(components[0].component_type, ComponentType::Text);
    }

    #[test]
    fn test_component_ids_unique() {
        let action = CanvasAction {
            action: "confirm_deploy".to_string(),
            target: Some("prod".to_string()),
            params: json!({}),
        };
        let components = render_action(&action);
        let card = &components[0];
        let ids: Vec<&str> = std::iter::once(card.id.as_str())
            .chain(card.children.iter().map(|c| c.id.as_str()))
            .collect();
        let unique: std::collections::HashSet<&&str> = ids.iter().collect();
        assert_eq!(ids.len(), unique.len(), "All component IDs must be unique");
    }

    #[test]
    fn test_render_response_serialization() {
        let action = CanvasAction {
            action: "show_info".to_string(),
            target: Some("test".to_string()),
            params: json!({}),
        };
        let resp = RenderResponse {
            ok: true,
            components: render_action(&action),
            action,
            layout: "full".to_string(),
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["ok"], true);
        assert!(json["components"].is_array());
        assert_eq!(json["layout"], "full");
    }
}
