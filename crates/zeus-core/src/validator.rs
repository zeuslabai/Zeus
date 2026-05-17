//! Configuration Validator — comprehensive startup validation.
//!
//! Validates Zeus configuration at startup with detailed error reporting:
//! - **Model format**: Validates "provider/model" format and known providers
//! - **Path validation**: Checks workspace, sessions, and subsystem paths exist or are creatable
//! - **API key checks**: Verifies required environment variables for configured providers
//! - **Port conflicts**: Detects duplicate port bindings
//! - **Value ranges**: Validates numeric config values are within acceptable ranges
//! - **Subsystem coherence**: Checks subsystem configs reference valid dependencies

use std::collections::{HashMap, HashSet};
use std::path::Path;

// ============================================================================
// Types
// ============================================================================

/// Severity level of a validation finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// Informational — not a problem, just a note.
    Info,
    /// Warning — works but may cause issues.
    Warning,
    /// Error — will prevent correct operation.
    Error,
}

impl std::fmt::Display for Severity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Severity::Info => write!(f, "INFO"),
            Severity::Warning => write!(f, "WARN"),
            Severity::Error => write!(f, "ERROR"),
        }
    }
}

/// A single validation finding.
#[derive(Debug, Clone)]
pub struct ValidationFinding {
    /// Severity of the finding.
    pub severity: Severity,
    /// Config field or section this relates to.
    pub field: String,
    /// Human-readable description of the issue.
    pub message: String,
    /// Suggested fix, if any.
    pub suggestion: Option<String>,
}

/// Overall validation result.
#[derive(Debug, Clone)]
pub struct ValidationReport {
    /// All findings.
    pub findings: Vec<ValidationFinding>,
    /// Whether validation passed (no errors).
    pub passed: bool,
    /// Count of errors.
    pub error_count: usize,
    /// Count of warnings.
    pub warning_count: usize,
    /// Count of info items.
    pub info_count: usize,
}

impl ValidationReport {
    /// Get findings filtered by severity.
    pub fn by_severity(&self, severity: Severity) -> Vec<&ValidationFinding> {
        self.findings
            .iter()
            .filter(|f| f.severity == severity)
            .collect()
    }

    /// Get a human-readable summary string.
    pub fn summary(&self) -> String {
        let status = if self.passed { "PASSED" } else { "FAILED" };
        format!(
            "Validation {}: {} errors, {} warnings, {} info",
            status, self.error_count, self.warning_count, self.info_count
        )
    }
}

// ============================================================================
// Known Providers
// ============================================================================

/// Known LLM providers with their required env vars.
fn known_providers() -> HashMap<&'static str, &'static str> {
    let mut m = HashMap::new();
    m.insert("anthropic", "ANTHROPIC_API_KEY");
    m.insert("openai", "OPENAI_API_KEY");
    m.insert("openrouter", "OPENROUTER_API_KEY");
    m.insert("google", "GOOGLE_API_KEY");
    m.insert("groq", "GROQ_API_KEY");
    m.insert("mistral", "MISTRAL_API_KEY");
    m.insert("together", "TOGETHER_API_KEY");
    m.insert("fireworks", "FIREWORKS_API_KEY");
    m.insert("azure", "AZURE_OPENAI_API_KEY");
    m.insert("bedrock", "AWS_ACCESS_KEY_ID");
    m.insert("ollama", ""); // No API key needed
    m
}

// ============================================================================
// Validator
// ============================================================================

/// Configuration validator.
pub struct ConfigValidator {
    findings: Vec<ValidationFinding>,
}

impl ConfigValidator {
    /// Create a new validator.
    pub fn new() -> Self {
        Self {
            findings: Vec::new(),
        }
    }

    /// Add a finding.
    fn add(&mut self, severity: Severity, field: &str, message: &str, suggestion: Option<&str>) {
        self.findings.push(ValidationFinding {
            severity,
            field: field.to_string(),
            message: message.to_string(),
            suggestion: suggestion.map(String::from),
        });
    }

    /// Validate a model string format ("provider/model").
    pub fn validate_model(&mut self, model: &str) {
        if model.is_empty() {
            self.add(
                Severity::Error,
                "model",
                "Model string is empty",
                Some("Set model = \"ollama/llama3.2\" in config.toml"),
            );
            return;
        }

        let parts: Vec<&str> = model.splitn(2, '/').collect();
        if parts.len() < 2 {
            self.add(
                Severity::Warning,
                "model",
                &format!("Model '{}' missing provider prefix", model),
                Some("Use 'provider/model' format, e.g., 'anthropic/claude-sonnet-4-6'"),
            );
            return;
        }

        let provider = parts[0];
        let providers = known_providers();

        if !providers.contains_key(provider) {
            self.add(
                Severity::Warning,
                "model",
                &format!("Unknown provider '{}' in model string", provider),
                Some(&format!(
                    "Known providers: {}",
                    providers.keys().cloned().collect::<Vec<_>>().join(", ")
                )),
            );
        }
    }

    /// Validate that a required environment variable is set.
    pub fn validate_env_var(&mut self, field: &str, var_name: &str) {
        if var_name.is_empty() {
            return; // No env var needed (e.g., ollama)
        }
        if std::env::var(var_name).is_err() {
            self.add(
                Severity::Warning,
                field,
                &format!("Environment variable {} not set", var_name),
                Some(&format!("export {}=your-api-key", var_name)),
            );
        }
    }

    /// Validate API key for a model's provider.
    pub fn validate_model_api_key(&mut self, model: &str) {
        let parts: Vec<&str> = model.splitn(2, '/').collect();
        if parts.len() < 2 {
            return;
        }

        let provider = parts[0];
        let providers = known_providers();

        if let Some(env_var) = providers.get(provider) {
            self.validate_env_var(&format!("model({})", provider), env_var);
        }
    }

    /// Validate a path exists or its parent is creatable.
    pub fn validate_path(&mut self, field: &str, path: &Path, must_exist: bool) {
        if must_exist && !path.exists() {
            self.add(
                Severity::Error,
                field,
                &format!("Path does not exist: {}", path.display()),
                Some(&format!("Create it with: mkdir -p {}", path.display())),
            );
        } else if !must_exist {
            // Check parent directory exists
            if let Some(parent) = path.parent()
                && !parent.as_os_str().is_empty()
                && !parent.exists()
            {
                self.add(
                    Severity::Warning,
                    field,
                    &format!("Parent directory does not exist: {}", parent.display()),
                    Some(&format!(
                        "Will be created automatically or run: mkdir -p {}",
                        parent.display()
                    )),
                );
            }
        }
    }

    /// Validate a numeric value is within range.
    pub fn validate_range(&mut self, field: &str, value: usize, min: usize, max: usize) {
        if value < min || value > max {
            self.add(
                Severity::Error,
                field,
                &format!("Value {} is outside range [{}, {}]", value, min, max),
                Some(&format!("Set to a value between {} and {}", min, max)),
            );
        }
    }

    /// Validate a port number.
    pub fn validate_port(&mut self, field: &str, port: u16) {
        if port == 0 {
            self.add(
                Severity::Error,
                field,
                "Port cannot be 0",
                Some("Use a port between 1024 and 65535"),
            );
        } else if port < 1024 {
            self.add(
                Severity::Warning,
                field,
                &format!("Port {} is a privileged port (requires root)", port),
                Some("Use a port >= 1024 unless running as root"),
            );
        }
    }

    /// Check for duplicate ports across configs.
    pub fn validate_no_duplicate_ports(&mut self, ports: &[(&str, u16)]) {
        let mut seen: HashMap<u16, &str> = HashMap::new();
        for (field, port) in ports {
            if *port == 0 {
                continue;
            }
            if let Some(other_field) = seen.get(port) {
                self.add(
                    Severity::Error,
                    field,
                    &format!("Port {} conflicts with {}", port, other_field),
                    Some("Each service must use a unique port"),
                );
            } else {
                seen.insert(*port, field);
            }
        }
    }

    /// Validate a URL format.
    pub fn validate_url(&mut self, field: &str, url: &str) {
        if url.is_empty() {
            self.add(Severity::Error, field, "URL is empty", None);
            return;
        }
        if !url.starts_with("http://") && !url.starts_with("https://") {
            self.add(
                Severity::Warning,
                field,
                &format!("URL '{}' missing http(s):// prefix", url),
                Some("URLs should start with http:// or https://"),
            );
        }
    }

    /// Validate that fallback models don't include the primary model.
    pub fn validate_fallback_models(&mut self, primary: &str, fallbacks: &[String]) {
        if fallbacks.is_empty() {
            return;
        }

        let mut seen: HashSet<String> = HashSet::new();
        seen.insert(primary.to_string());

        for (i, model) in fallbacks.iter().enumerate() {
            if !seen.insert(model.clone()) {
                self.add(
                    Severity::Warning,
                    &format!("fallback_models[{}]", i),
                    &format!("Duplicate model '{}' in fallback list", model),
                    Some("Each fallback model should be unique"),
                );
            }
            // Also validate format
            self.validate_model(model);
        }
    }

    /// Validate thinking level value.
    pub fn validate_thinking_level(&mut self, level: &str) {
        let valid = ["low", "medium", "high", "xhigh"];
        if !valid.contains(&level) {
            self.add(
                Severity::Error,
                "thinking_level",
                &format!(
                    "Invalid thinking level '{}', must be one of: {}",
                    level,
                    valid.join(", ")
                ),
                None,
            );
        }
    }

    /// Produce the final validation report.
    pub fn report(self) -> ValidationReport {
        let error_count = self
            .findings
            .iter()
            .filter(|f| f.severity == Severity::Error)
            .count();
        let warning_count = self
            .findings
            .iter()
            .filter(|f| f.severity == Severity::Warning)
            .count();
        let info_count = self
            .findings
            .iter()
            .filter(|f| f.severity == Severity::Info)
            .count();
        let passed = error_count == 0;

        ValidationReport {
            findings: self.findings,
            passed,
            error_count,
            warning_count,
            info_count,
        }
    }
}

impl Default for ConfigValidator {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_new_validator_empty() {
        let v = ConfigValidator::new();
        let report = v.report();
        assert!(report.passed);
        assert_eq!(report.error_count, 0);
        assert_eq!(report.warning_count, 0);
        assert_eq!(report.findings.len(), 0);
    }

    #[test]
    fn test_validate_model_valid() {
        let mut v = ConfigValidator::new();
        v.validate_model("ollama/llama3.2");
        let report = v.report();
        assert!(report.passed);
        assert_eq!(report.error_count, 0);
    }

    #[test]
    fn test_validate_model_all_providers() {
        for provider in [
            "anthropic",
            "openai",
            "ollama",
            "openrouter",
            "google",
            "groq",
            "mistral",
            "together",
            "fireworks",
            "azure",
            "bedrock",
        ] {
            let mut v = ConfigValidator::new();
            v.validate_model(&format!("{}/test-model", provider));
            let report = v.report();
            // Known providers should produce no warnings about provider
            assert_eq!(
                report
                    .findings
                    .iter()
                    .filter(|f| f.message.contains("Unknown provider"))
                    .count(),
                0
            );
        }
    }

    #[test]
    fn test_validate_model_empty() {
        let mut v = ConfigValidator::new();
        v.validate_model("");
        let report = v.report();
        assert!(!report.passed);
        assert_eq!(report.error_count, 1);
    }

    #[test]
    fn test_validate_model_no_provider() {
        let mut v = ConfigValidator::new();
        v.validate_model("llama3.2");
        let report = v.report();
        assert_eq!(report.warning_count, 1);
        assert!(
            report.findings[0]
                .message
                .contains("missing provider prefix")
        );
    }

    #[test]
    fn test_validate_model_unknown_provider() {
        let mut v = ConfigValidator::new();
        v.validate_model("unknown_provider/model");
        let report = v.report();
        assert_eq!(report.warning_count, 1);
        assert!(report.findings[0].message.contains("Unknown provider"));
    }

    #[test]
    fn test_validate_path_exists() {
        let mut v = ConfigValidator::new();
        v.validate_path("workspace", Path::new("/tmp"), true);
        let report = v.report();
        assert!(report.passed);
    }

    #[test]
    fn test_validate_path_missing_must_exist() {
        let mut v = ConfigValidator::new();
        v.validate_path("workspace", Path::new("/nonexistent/path/xyz"), true);
        let report = v.report();
        assert!(!report.passed);
        assert_eq!(report.error_count, 1);
    }

    #[test]
    fn test_validate_path_parent_missing() {
        let mut v = ConfigValidator::new();
        v.validate_path(
            "db_path",
            Path::new("/nonexistent_parent_dir_xyz/file.db"),
            false,
        );
        let report = v.report();
        assert_eq!(report.warning_count, 1);
    }

    #[test]
    fn test_validate_range_valid() {
        let mut v = ConfigValidator::new();
        v.validate_range("max_iterations", 20, 1, 100);
        let report = v.report();
        assert!(report.passed);
    }

    #[test]
    fn test_validate_range_too_low() {
        let mut v = ConfigValidator::new();
        v.validate_range("max_iterations", 0, 1, 100);
        let report = v.report();
        assert!(!report.passed);
        assert!(report.findings[0].message.contains("outside range"));
    }

    #[test]
    fn test_validate_range_too_high() {
        let mut v = ConfigValidator::new();
        v.validate_range("max_iterations", 200, 1, 100);
        let report = v.report();
        assert!(!report.passed);
    }

    #[test]
    fn test_validate_port_valid() {
        let mut v = ConfigValidator::new();
        v.validate_port("gateway.port", 3000);
        let report = v.report();
        assert!(report.passed);
    }

    #[test]
    fn test_validate_port_zero() {
        let mut v = ConfigValidator::new();
        v.validate_port("gateway.port", 0);
        let report = v.report();
        assert!(!report.passed);
    }

    #[test]
    fn test_validate_port_privileged() {
        let mut v = ConfigValidator::new();
        v.validate_port("gateway.port", 80);
        let report = v.report();
        assert_eq!(report.warning_count, 1);
        assert!(report.findings[0].message.contains("privileged"));
    }

    #[test]
    fn test_validate_no_duplicate_ports() {
        let mut v = ConfigValidator::new();
        v.validate_no_duplicate_ports(&[("api", 3000), ("metrics", 9090)]);
        let report = v.report();
        assert!(report.passed);
    }

    #[test]
    fn test_validate_duplicate_ports() {
        let mut v = ConfigValidator::new();
        v.validate_no_duplicate_ports(&[("api", 3000), ("metrics", 3000)]);
        let report = v.report();
        assert!(!report.passed);
        assert!(report.findings[0].message.contains("conflicts"));
    }

    #[test]
    fn test_validate_url_valid() {
        let mut v = ConfigValidator::new();
        v.validate_url("ollama.url", "http://localhost:11434");
        let report = v.report();
        assert!(report.passed);
    }

    #[test]
    fn test_validate_url_https() {
        let mut v = ConfigValidator::new();
        v.validate_url("api_url", "https://api.example.com");
        let report = v.report();
        assert!(report.passed);
    }

    #[test]
    fn test_validate_url_empty() {
        let mut v = ConfigValidator::new();
        v.validate_url("ollama.url", "");
        let report = v.report();
        assert!(!report.passed);
    }

    #[test]
    fn test_validate_url_no_scheme() {
        let mut v = ConfigValidator::new();
        v.validate_url("ollama.url", "localhost:11434");
        let report = v.report();
        assert_eq!(report.warning_count, 1);
    }

    #[test]
    fn test_validate_fallback_models() {
        let mut v = ConfigValidator::new();
        v.validate_fallback_models(
            "anthropic/claude-3",
            &["openai/gpt-4o".into(), "ollama/llama3.2".into()],
        );
        let report = v.report();
        // No duplicates, all valid format — may have warnings about missing API keys
        let dup_warnings = report
            .findings
            .iter()
            .filter(|f| f.message.contains("Duplicate"))
            .count();
        assert_eq!(dup_warnings, 0);
    }

    #[test]
    fn test_validate_fallback_models_duplicate() {
        let mut v = ConfigValidator::new();
        v.validate_fallback_models(
            "anthropic/claude-3",
            &[
                "openai/gpt-4o".into(),
                "anthropic/claude-3".into(), // Same as primary
            ],
        );
        let report = v.report();
        let dup_warnings = report
            .findings
            .iter()
            .filter(|f| f.message.contains("Duplicate"))
            .count();
        assert_eq!(dup_warnings, 1);
    }

    #[test]
    fn test_validate_thinking_level_valid() {
        for level in ["low", "medium", "high", "xhigh"] {
            let mut v = ConfigValidator::new();
            v.validate_thinking_level(level);
            let report = v.report();
            assert!(report.passed);
        }
    }

    #[test]
    fn test_validate_thinking_level_invalid() {
        let mut v = ConfigValidator::new();
        v.validate_thinking_level("ultra");
        let report = v.report();
        assert!(!report.passed);
    }

    #[test]
    fn test_severity_ordering() {
        assert!(Severity::Info < Severity::Warning);
        assert!(Severity::Warning < Severity::Error);
    }

    #[test]
    fn test_severity_display() {
        assert_eq!(Severity::Info.to_string(), "INFO");
        assert_eq!(Severity::Warning.to_string(), "WARN");
        assert_eq!(Severity::Error.to_string(), "ERROR");
    }

    #[test]
    fn test_report_by_severity() {
        let mut v = ConfigValidator::new();
        v.validate_model(""); // Error
        v.validate_model("unknown/model"); // Warning
        v.validate_port("test", 3000); // No finding (valid)
        let report = v.report();
        assert_eq!(report.by_severity(Severity::Error).len(), 1);
        assert_eq!(report.by_severity(Severity::Warning).len(), 1);
    }

    #[test]
    fn test_report_summary() {
        let mut v = ConfigValidator::new();
        v.validate_model("ollama/llama3.2");
        let report = v.report();
        let summary = report.summary();
        assert!(summary.contains("PASSED"));

        let mut v2 = ConfigValidator::new();
        v2.validate_model("");
        let report2 = v2.report();
        assert!(report2.summary().contains("FAILED"));
    }

    #[test]
    fn test_multiple_validations() {
        let mut v = ConfigValidator::new();
        v.validate_model("ollama/llama3.2");
        v.validate_port("api", 3000);
        v.validate_range("max_iterations", 20, 1, 100);
        v.validate_path("workspace", Path::new("/tmp"), true);
        v.validate_url("ollama.url", "http://localhost:11434");

        let report = v.report();
        assert!(report.passed);
        assert_eq!(report.error_count, 0);
    }

    #[test]
    fn test_findings_have_suggestions() {
        let mut v = ConfigValidator::new();
        v.validate_model("");
        let report = v.report();
        assert!(report.findings[0].suggestion.is_some());
    }

    #[test]
    fn test_env_var_missing() {
        // Use a var name that definitely doesn't exist
        let mut v = ConfigValidator::new();
        v.validate_env_var("test", "ZEUS_NONEXISTENT_TEST_VAR_12345");
        let report = v.report();
        assert_eq!(report.warning_count, 1);
    }

    #[test]
    fn test_env_var_empty_name() {
        let mut v = ConfigValidator::new();
        v.validate_env_var("test", "");
        let report = v.report();
        // Empty env var name = no check needed (like ollama)
        assert_eq!(report.findings.len(), 0);
    }
}
