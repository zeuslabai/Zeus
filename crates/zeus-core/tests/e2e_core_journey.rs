//! End-to-end tests for core Zeus types and user journeys.
//!
//! Tests the full lifecycle of:
//! 1. Config loading, saving, modification
//! 2. Message construction with tool calls and results
//! 3. Tool schema validation
//! 4. Auth profile management
//! 5. Workspace template generation

use tempfile::tempdir;
use zeus_core::*;

// ============================================================================
// Journey 1: Config lifecycle — create, modify, save, reload
// ============================================================================

#[test]
fn journey_config_lifecycle() {
    let dir = tempdir().expect("should create temp dir");
    let config_path = dir.path().join("config.toml");

    // Step 1: Create default config
    let mut config = Config::default();
    assert!(!config.onboarding_complete); // fresh installs run onboarding

    // Step 2: Customize config (model is "provider/model" format string)
    config.model = "anthropic/claude-sonnet-4-20250514".to_string();
    config.onboarding_complete = true;
    config.tui.vim_mode = true;
    config.auth.use_oauth = false;

    // Step 3: Save to disk
    let toml_str = toml::to_string_pretty(&config).expect("should serialize");
    std::fs::write(&config_path, &toml_str).expect("should write file");

    // Step 4: Reload and verify
    let loaded_str = std::fs::read_to_string(&config_path).expect("should read file");
    let loaded: Config = toml::from_str(&loaded_str).expect("should parse successfully");
    assert!(loaded.onboarding_complete);
    assert_eq!(loaded.model, "anthropic/claude-sonnet-4-20250514");
    assert!(loaded.tui.vim_mode);

    // Step 5: Modify and re-save
    let mut modified = loaded;
    modified.model = "anthropic/claude-opus-4-20250514".to_string();
    let toml_str2 = toml::to_string_pretty(&modified).expect("should serialize");
    std::fs::write(&config_path, &toml_str2).expect("should write file");

    let reloaded_str = std::fs::read_to_string(&config_path).expect("should read file");
    let reloaded: Config = toml::from_str(&reloaded_str).expect("should parse successfully");
    assert_eq!(reloaded.model, "anthropic/claude-opus-4-20250514");
}

// ============================================================================
// Journey 2: Message construction pipeline
// ============================================================================

#[test]
fn journey_message_pipeline() {
    // Step 1: User sends a message
    let user_msg = Message::user("Read the file at src/main.rs");
    assert_eq!(user_msg.role, Role::User);
    assert_eq!(user_msg.content, "Read the file at src/main.rs");
    assert!(user_msg.tool_calls.is_empty());

    // Step 2: Assistant responds with tool call
    let tool_call = ToolCall {
        id: "tc_001".to_string(),
        name: "read_file".to_string(),
        arguments: serde_json::json!({"path": "src/main.rs"}),
    };
    let assistant_msg =
        Message::assistant("I'll read that file for you.").with_tool_calls(vec![tool_call.clone()]);
    assert_eq!(assistant_msg.role, Role::Assistant);
    assert_eq!(assistant_msg.tool_calls.len(), 1);
    assert_eq!(assistant_msg.tool_calls[0].name, "read_file");

    // Step 3: Tool returns result (uses call_id, success, output)
    let tool_result = ToolResult {
        call_id: "tc_001".to_string(),
        success: true,
        output: "fn main() { println!(\"Hello\"); }".to_string(),
    };
    assert!(tool_result.success);
    assert_eq!(tool_result.call_id, "tc_001");

    // Step 4: Create a Tool message using the helper
    let tool_msg = Message::tool("tc_001", true, "fn main() { println!(\"Hello\"); }");
    assert_eq!(tool_msg.role, Role::Tool);
    assert_eq!(tool_msg.tool_results.len(), 1);
    assert_eq!(tool_msg.tool_results[0].call_id, "tc_001");
    assert!(tool_msg.tool_results[0].success);

    // Step 5: Serialize the full conversation
    let messages = vec![user_msg, assistant_msg, tool_msg];
    let json = serde_json::to_string(&messages).expect("should serialize to JSON");
    let parsed: Vec<Message> = serde_json::from_str(&json).expect("should parse successfully");
    assert_eq!(parsed.len(), 3);
    assert_eq!(parsed[0].role, Role::User);
    assert_eq!(parsed[1].tool_calls.len(), 1);
    assert_eq!(parsed[2].role, Role::Tool);

    // Step 6: Verify tool result roundtrip
    let result_json = serde_json::to_string(&tool_result).expect("should serialize to JSON");
    let parsed_result: ToolResult =
        serde_json::from_str(&result_json).expect("should parse successfully");
    assert!(parsed_result.success);
    assert_eq!(parsed_result.call_id, "tc_001");
}

// ============================================================================
// Journey 3: Tool schema definition and validation
// ============================================================================

#[test]
fn journey_tool_schema() {
    // Step 1: Define a tool using constructor
    let tool = ToolSchema::new("execute_shell", "Execute a shell command")
        .with_param("command", "string", "The shell command to execute", true)
        .with_param("timeout", "integer", "Timeout in seconds", false);

    assert_eq!(tool.name, "execute_shell");
    assert_eq!(tool.description, "Execute a shell command");

    // Step 2: Serialize and deserialize
    let json = serde_json::to_string_pretty(&tool).expect("should serialize");
    let parsed: ToolSchema = serde_json::from_str(&json).expect("should parse successfully");
    assert_eq!(parsed.name, "execute_shell");
    assert!(json.contains("command"));

    // Step 3: Verify parameter structure
    let params = &parsed.parameters;
    assert_eq!(params["type"], "object");
    assert!(params["properties"]["command"].is_object());

    // Step 4: Raw construction
    let raw_tool = ToolSchema {
        name: "read_file".to_string(),
        description: "Read a file".to_string(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "File path"}
            },
            "required": ["path"]
        }),
    };
    let raw_json = serde_json::to_string(&raw_tool).expect("should serialize to JSON");
    let raw_parsed: ToolSchema =
        serde_json::from_str(&raw_json).expect("should parse successfully");
    assert_eq!(raw_parsed.parameters["required"][0], "path");
}

// ============================================================================
// Journey 4: Provider enum roundtrip
// ============================================================================

#[test]
fn journey_provider_roundtrip() {
    // Step 1: All providers should serialize/deserialize correctly
    let providers = [
        Provider::Anthropic,
        Provider::OpenAI,
        Provider::Ollama,
        Provider::OpenRouter,
        Provider::Google,
        Provider::Groq,
        Provider::Mistral,
        Provider::Together,
        Provider::Fireworks,
        Provider::Azure,
        Provider::Bedrock,
    ];

    for provider in &providers {
        let json = serde_json::to_string(provider).expect("should serialize to JSON");
        let parsed: Provider = serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(
            *provider, parsed,
            "Provider roundtrip failed for {:?}",
            provider
        );
    }

    // Step 2: Verify env key mapping
    assert_eq!(Provider::Anthropic.env_key(), "ANTHROPIC_API_KEY");
    assert_eq!(Provider::OpenAI.env_key(), "OPENAI_API_KEY");
    assert_eq!(Provider::Ollama.env_key(), "OLLAMA_HOST");

    // Step 3: Verify name mapping
    assert_eq!(Provider::Anthropic.name(), "anthropic");
    assert_eq!(Provider::Google.name(), "google");
    assert_eq!(Provider::Bedrock.name(), "bedrock");

    // Step 4: Config with auth settings
    let mut config = Config::default();
    config.model = "anthropic/claude-sonnet-4-20250514".to_string();
    config.auth.use_oauth = true;

    let toml_str = toml::to_string_pretty(&config).expect("should serialize");
    assert!(toml_str.contains("use_oauth"));
}

// ============================================================================
// Journey 5: Auth profile management
// ============================================================================

#[test]
fn journey_auth_profiles() {
    // Step 1: Create a profile manager
    let mut manager = AuthProfileManager::new();

    // Step 2: Add profiles (provider is String, not enum)
    let anthropic = AuthProfile {
        id: "prof-anthropic-1".to_string(),
        name: "Primary Anthropic".to_string(),
        provider: "anthropic".to_string(),
        api_key: "sk-ant-test-key-123".to_string(),
        model: Some("claude-sonnet-4-20250514".to_string()),
        max_rpm: 60,
        max_tokens_per_day: Some(1_000_000),
        tokens_used_today: std::sync::atomic::AtomicU64::new(0),
        enabled: true,
        priority: 1,
        cooldown_secs: 30,
        last_rate_limit: std::sync::Mutex::new(None),
    };

    let ollama = AuthProfile {
        id: "prof-ollama-1".to_string(),
        name: "Local Ollama".to_string(),
        provider: "ollama".to_string(),
        api_key: String::new(),
        model: Some("llama3:70b".to_string()),
        max_rpm: 120,
        max_tokens_per_day: None,
        tokens_used_today: std::sync::atomic::AtomicU64::new(0),
        enabled: true,
        priority: 2,
        cooldown_secs: 0,
        last_rate_limit: std::sync::Mutex::new(None),
    };

    manager.add(anthropic);
    manager.add(ollama);

    // Step 3: Verify sorted by priority
    let all = manager.all();
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].priority, 1);
    assert_eq!(all[0].provider, "anthropic");
    assert_eq!(all[1].priority, 2);

    // Step 4: Get available profile for provider
    let available = manager.get_available("anthropic");
    assert!(available.is_some());
    assert_eq!(
        available.expect("operation should succeed").id,
        "prof-anthropic-1"
    );

    // Step 5: Get by ID
    let by_id = manager.get("prof-ollama-1");
    assert!(by_id.is_some());
    assert_eq!(
        by_id.expect("operation should succeed").name,
        "Local Ollama"
    );

    // Step 6: Rate limit the primary
    let primary = manager.get("prof-anthropic-1").expect("key should exist");
    primary.mark_rate_limited();
    assert!(!primary.is_available()); // Should be in cooldown

    // Step 7: Record token usage
    let ollama_prof = manager.get("prof-ollama-1").expect("key should exist");
    ollama_prof.record_usage(5000);
    assert!(ollama_prof.is_available()); // No daily limit set

    // Step 8: Fallback — anthropic rate-limited, get ollama
    let fallback = manager.get_available("ollama");
    assert!(fallback.is_some());
    assert_eq!(
        fallback.expect("operation should succeed").provider,
        "ollama"
    );
}

// ============================================================================
// Journey 6: Error handling chain
// ============================================================================

#[test]
fn journey_error_handling() {
    // Step 1: Create various error types
    let errors: Vec<Error> = vec![
        Error::Config("missing API key".into()),
        Error::Llm("rate limited".into()),
        Error::Tool("file not found".into()),
        Error::Memory("database locked".into()),
        Error::Database("connection timeout".into()),
        Error::Network("connection refused".into()),
        Error::Timeout("request timed out".into()),
        Error::RateLimited("429 too many requests".into()),
        Error::Security("unauthorized access".into()),
        Error::NotFound("resource not found".into()),
    ];

    // Step 2: Verify display
    for err in &errors {
        let msg = err.to_string();
        assert!(!msg.is_empty(), "Error should have a message");
    }

    // Step 3: Check retryable — only Network/Timeout/RateLimited
    assert!(!Error::Config("x".into()).is_retryable());
    assert!(!Error::Llm("x".into()).is_retryable());
    assert!(!Error::Tool("x".into()).is_retryable());
    assert!(Error::Network("x".into()).is_retryable());
    assert!(Error::Timeout("x".into()).is_retryable());
    assert!(Error::RateLimited("x".into()).is_retryable());

    // Step 4: Error conversion to Result
    let result: Result<()> = Err(Error::Config("test".into()));
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.to_string().contains("test"));

    // Step 5: Builder-style constructors
    let e1 = Error::config("missing key");
    assert!(e1.to_string().contains("missing key"));
    let e2 = Error::llm("model not found");
    assert!(e2.to_string().contains("model not found"));
    let e3 = Error::not_found("file.txt");
    assert!(e3.to_string().contains("file.txt"));
}

// ============================================================================
// Journey 7: Message truncation and large content
// ============================================================================

#[test]
fn journey_large_content_handling() {
    // Step 1: Create a very large message
    let large_content = "x".repeat(100_000);
    let msg = Message::user(&large_content);
    assert_eq!(msg.content.len(), 100_000);

    // Step 2: Truncate
    let truncated = truncate_str(&large_content, 1000);
    assert!(truncated.len() <= 1000);

    // Step 3: Truncation preserves valid UTF-8
    let unicode = "Hello 🌍 world!";
    let trunc = truncate_str(unicode, 8);
    // Should not split the emoji — valid UTF-8
    assert!(trunc.len() <= 8);
    let _ = trunc.to_string();

    // Step 4: Empty and small strings
    assert_eq!(truncate_str("", 100), "");
    assert_eq!(truncate_str("short", 100), "short");
    assert_eq!(truncate_str("short", 5), "short");

    // Step 5: Exact boundary
    assert_eq!(truncate_str("abc", 3), "abc");
    assert_eq!(truncate_str("abc", 2).len(), 2);
}

// ============================================================================
// Journey 8: Workspace template generation
// ============================================================================

#[test]
fn journey_workspace_template() {
    // Step 1: Get built-in templates
    let templates = WorkspaceTemplate::builtins();
    assert!(!templates.is_empty());

    // Step 2: Verify each template has required fields
    for template in &templates {
        assert!(!template.id.is_empty(), "Template must have an id");
        assert!(!template.name.is_empty(), "Template must have a name");
        assert!(
            !template.description.is_empty(),
            "Template must have a description"
        );
        assert!(
            !template.category.is_empty(),
            "Template must have a category"
        );
    }

    // Step 3: Check rust-project template specifically
    let rust_tmpl = templates.iter().find(|t| t.id == "rust-project");
    assert!(rust_tmpl.is_some(), "Should have a rust-project template");
    let rust_tmpl = rust_tmpl.expect("operation should succeed");
    assert_eq!(rust_tmpl.category, "development");
    assert!(rust_tmpl.files.contains_key("memory/project.md"));

    // Step 4: Serialize/deserialize roundtrip
    let json = serde_json::to_string_pretty(&rust_tmpl).expect("should serialize");
    let parsed: WorkspaceTemplate = serde_json::from_str(&json).expect("should parse successfully");
    assert_eq!(parsed.id, rust_tmpl.id);
    assert_eq!(parsed.files.len(), rust_tmpl.files.len());
}

// ============================================================================
// Journey 9: Role serialization consistency
// ============================================================================

#[test]
fn journey_role_consistency() {
    // All roles should roundtrip through serde
    let roles = [Role::User, Role::Assistant, Role::System, Role::Tool];
    for role in &roles {
        let json = serde_json::to_string(role).expect("should serialize to JSON");
        let parsed: Role = serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(*role, parsed);
    }

    // Verify lowercase serialization
    assert_eq!(
        serde_json::to_string(&Role::User).expect("should serialize to JSON"),
        "\"user\""
    );
    assert_eq!(
        serde_json::to_string(&Role::Assistant).expect("should serialize to JSON"),
        "\"assistant\""
    );
    assert_eq!(
        serde_json::to_string(&Role::System).expect("should serialize to JSON"),
        "\"system\""
    );
    assert_eq!(
        serde_json::to_string(&Role::Tool).expect("should serialize to JSON"),
        "\"tool\""
    );

    // Messages preserve roles
    let msgs = vec![
        Message::user("hello"),
        Message::assistant("hi"),
        Message::system("context"),
        Message::tool("tc_001", true, "result"),
    ];
    for msg in &msgs {
        let json = serde_json::to_string(msg).expect("should serialize to JSON");
        let parsed: Message = serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(msg.role, parsed.role);
        assert_eq!(msg.content, parsed.content);
    }
}
