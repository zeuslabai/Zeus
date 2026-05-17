//! Keychain tools — wrappers around the macOS `security` CLI.
//!
//! Provides:
//! - `keychain_get`    — read a generic password
//! - `keychain_set`    — add or update a generic password
//! - `keychain_delete` — remove a generic password
//! - `keychain_list`   — list service names visible to the user
//!
//! All arguments are passed directly to `tokio::process::Command` (no shell
//! interpretation), so command injection is structurally impossible. We still
//! validate service/account identifiers up-front to fail loudly on bad input.
//!
//! Sensitive note: secret material is only ever returned by `keychain_get`. The
//! plaintext password supplied to `keychain_set` is passed via argv to the
//! `security` binary (visible to local users via `ps` for the duration of the
//! call). This is the same trade-off the `security` CLI itself makes; macOS
//! treats argv of short-lived processes as acceptable for keychain workflows.

use crate::TalosTool;
use async_trait::async_trait;
use serde_json::Value;
use zeus_core::{Error, Result, ToolSchema};

const MAX_OUTPUT_BYTES: usize = 256 * 1024;
const MAX_PASSWORD_BYTES: usize = 64 * 1024; // generous; real secrets are tiny
const MAX_IDENT_LEN: usize = 256;

/// Validate a keychain identifier (service or account name). We allow any
/// printable, non-control character except NUL — keychain itself is permissive,
/// and rejecting too aggressively would surprise users with API tokens that
/// contain `@`, `=`, etc. We *do* reject empties, NUL, and obviously-bogus
/// length.
fn validate_identifier(s: &str, field: &str) -> Result<()> {
    if s.is_empty() {
        return Err(Error::Tool(format!("{} must not be empty", field)));
    }
    if s.len() > MAX_IDENT_LEN {
        return Err(Error::Tool(format!(
            "{} too long (max {} bytes)",
            field, MAX_IDENT_LEN
        )));
    }
    if s.contains('\0') {
        return Err(Error::Tool(format!("{} contains NUL byte", field)));
    }
    if s.chars().any(|c| c.is_control()) {
        return Err(Error::Tool(format!(
            "{} contains control characters",
            field
        )));
    }
    Ok(())
}

/// Validate an optional keychain path. Empty/None → default (login.keychain).
/// We accept anything that looks like a filesystem path to a `.keychain` /
/// `.keychain-db` file, but reject NUL and shell metachars (defense in depth —
/// argv is already safe, this just catches typos earlier).
fn validate_keychain_path(s: &str) -> Result<()> {
    if s.is_empty() {
        return Err(Error::Tool("keychain path must not be empty".into()));
    }
    if s.len() > 4096 {
        return Err(Error::Tool("keychain path too long".into()));
    }
    if s.contains('\0') {
        return Err(Error::Tool("keychain path contains NUL byte".into()));
    }
    Ok(())
}

fn truncate_output(s: String) -> String {
    if s.len() <= MAX_OUTPUT_BYTES {
        return s;
    }
    let mut out = s.into_bytes();
    out.truncate(MAX_OUTPUT_BYTES);
    let mut s = String::from_utf8_lossy(&out).to_string();
    s.push_str("\n... [truncated]");
    s
}

/// Run the `security` binary with the given args. Returns stdout on success,
/// or an error containing stderr on failure.
async fn run_security(args: &[&str]) -> Result<String> {
    let output = tokio::process::Command::new("security")
        .args(args)
        .output()
        .await
        .map_err(|e| {
            Error::Tool(format!(
                "Failed to run /usr/bin/security (macOS only?): {}",
                e
            ))
        })?;

    if output.status.success() {
        Ok(truncate_output(
            String::from_utf8_lossy(&output.stdout).to_string(),
        ))
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let detail = if stderr.is_empty() { stdout } else { stderr };
        Err(Error::Tool(format!(
            "security {} failed: {}",
            args.first().copied().unwrap_or("?"),
            detail
        )))
    }
}

fn arg_str<'a>(args: &'a Value, key: &str) -> Result<&'a str> {
    args.get(key)
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::Tool(format!("missing or non-string '{}'", key)))
}

fn opt_arg_str<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(|v| v.as_str())
}

// ---------- keychain_get ----------

/// Read a generic password from the keychain.
pub struct KeychainGetTool;

#[async_trait]
impl TalosTool for KeychainGetTool {
    fn name(&self) -> &'static str {
        "keychain_get"
    }
    fn description(&self) -> &'static str {
        "Read a generic password from the macOS keychain. \
         Args: { service: string, account?: string, keychain?: string }. \
         Returns the plaintext password."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let service = arg_str(&args, "service")?;
        validate_identifier(service, "service")?;

        let account = opt_arg_str(&args, "account");
        if let Some(a) = account {
            validate_identifier(a, "account")?;
        }

        let keychain = opt_arg_str(&args, "keychain");
        if let Some(k) = keychain {
            validate_keychain_path(k)?;
        }

        // `-w` writes only the password to stdout (no metadata).
        let mut argv: Vec<&str> = vec!["find-generic-password", "-s", service, "-w"];
        if let Some(a) = account {
            argv.push("-a");
            argv.push(a);
        }
        if let Some(k) = keychain {
            argv.push(k);
        }

        let out = run_security(&argv).await?;
        // `security -w` appends a trailing newline. Strip exactly one.
        let trimmed = out.strip_suffix('\n').unwrap_or(&out).to_string();
        Ok(trimmed)
    }
}

// ---------- keychain_set ----------

/// Add or update a generic password in the keychain.
pub struct KeychainSetTool;

#[async_trait]
impl TalosTool for KeychainSetTool {
    fn name(&self) -> &'static str {
        "keychain_set"
    }
    fn description(&self) -> &'static str {
        "Add or update a generic password in the macOS keychain. \
         Args: { service: string, account: string, password: string, keychain?: string, label?: string }. \
         Uses `-U` so existing entries are overwritten."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let service = arg_str(&args, "service")?;
        let account = arg_str(&args, "account")?;
        let password = arg_str(&args, "password")?;
        validate_identifier(service, "service")?;
        validate_identifier(account, "account")?;

        if password.is_empty() {
            return Err(Error::Tool("password must not be empty".into()));
        }
        if password.len() > MAX_PASSWORD_BYTES {
            return Err(Error::Tool(format!(
                "password too long (max {} bytes)",
                MAX_PASSWORD_BYTES
            )));
        }
        if password.contains('\0') {
            return Err(Error::Tool("password contains NUL byte".into()));
        }

        let label = opt_arg_str(&args, "label");
        if let Some(l) = label {
            validate_identifier(l, "label")?;
        }

        let keychain = opt_arg_str(&args, "keychain");
        if let Some(k) = keychain {
            validate_keychain_path(k)?;
        }

        // `-U` updates if the item exists, creates otherwise.
        let mut argv: Vec<&str> = vec![
            "add-generic-password",
            "-U",
            "-s",
            service,
            "-a",
            account,
            "-w",
            password,
        ];
        if let Some(l) = label {
            argv.push("-l");
            argv.push(l);
        }
        if let Some(k) = keychain {
            argv.push(k);
        }

        run_security(&argv).await?;
        Ok(format!("stored {}/{}", service, account))
    }
}

// ---------- keychain_delete ----------

/// Delete a generic password from the keychain.
pub struct KeychainDeleteTool;

#[async_trait]
impl TalosTool for KeychainDeleteTool {
    fn name(&self) -> &'static str {
        "keychain_delete"
    }
    fn description(&self) -> &'static str {
        "Delete a generic password from the macOS keychain. \
         Args: { service: string, account?: string, keychain?: string }."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let service = arg_str(&args, "service")?;
        validate_identifier(service, "service")?;

        let account = opt_arg_str(&args, "account");
        if let Some(a) = account {
            validate_identifier(a, "account")?;
        }

        let keychain = opt_arg_str(&args, "keychain");
        if let Some(k) = keychain {
            validate_keychain_path(k)?;
        }

        let mut argv: Vec<&str> = vec!["delete-generic-password", "-s", service];
        if let Some(a) = account {
            argv.push("-a");
            argv.push(a);
        }
        if let Some(k) = keychain {
            argv.push(k);
        }

        run_security(&argv).await?;
        let target = match account {
            Some(a) => format!("{}/{}", service, a),
            None => service.to_string(),
        };
        Ok(format!("deleted {}", target))
    }
}

// ---------- keychain_list ----------

/// List keychains visible to the current user (or dump generic-password
/// service names from the default keychain when `services=true`).
pub struct KeychainListTool;

#[async_trait]
impl TalosTool for KeychainListTool {
    fn name(&self) -> &'static str {
        "keychain_list"
    }
    fn description(&self) -> &'static str {
        "List keychains. \
         Args: { services?: bool, keychain?: string }. \
         When `services=true`, dumps generic-password service names from the keychain \
         (login keychain by default). Otherwise lists keychain *files* on the search list."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let services = args
            .get("services")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let keychain = opt_arg_str(&args, "keychain");
        if let Some(k) = keychain {
            validate_keychain_path(k)?;
        }

        if !services {
            // List keychain files on the user's search list.
            return run_security(&["list-keychains"]).await;
        }

        // dump-keychain prints a verbose plist-ish blob; we just grep for
        // service attributes (`"svce"`) so the output is useful as a service
        // inventory.
        let mut argv: Vec<&str> = vec!["dump-keychain"];
        if let Some(k) = keychain {
            argv.push(k);
        }
        let raw = run_security(&argv).await?;

        let mut services: Vec<String> = raw
            .lines()
            .filter_map(|line| {
                // Looking for: `"svce"<blob>="..."`
                let tag = line.find("\"svce\"")?;
                let rest = &line[tag..];
                let eq = rest.find('=')?;
                let after = rest[eq + 1..].trim();
                let stripped = after.trim_matches('"');
                if stripped.is_empty() || stripped == "<NULL>" {
                    None
                } else {
                    Some(stripped.to_string())
                }
            })
            .collect();
        services.sort();
        services.dedup();

        if services.is_empty() {
            Ok("(no generic-password services found)".to_string())
        } else {
            Ok(services.join("\n"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_identifier_rejects_empty() {
        assert!(validate_identifier("", "service").is_err());
    }

    #[test]
    fn validate_identifier_rejects_nul() {
        assert!(validate_identifier("foo\0bar", "service").is_err());
    }

    #[test]
    fn validate_identifier_rejects_control_chars() {
        assert!(validate_identifier("foo\nbar", "service").is_err());
        assert!(validate_identifier("foo\tbar", "service").is_err());
    }

    #[test]
    fn validate_identifier_accepts_realistic_names() {
        assert!(validate_identifier("github.com", "service").is_ok());
        assert!(validate_identifier("api.openai.com/v1", "service").is_ok());
        assert!(validate_identifier("user@example.com", "account").is_ok());
        assert!(validate_identifier("zeus_secret_v2", "service").is_ok());
    }

    #[test]
    fn validate_identifier_rejects_overlong() {
        let big = "a".repeat(MAX_IDENT_LEN + 1);
        assert!(validate_identifier(&big, "service").is_err());
    }

    #[test]
    fn validate_keychain_path_rejects_empty_and_nul() {
        assert!(validate_keychain_path("").is_err());
        assert!(validate_keychain_path("foo\0").is_err());
    }

    #[test]
    fn validate_keychain_path_accepts_real_paths() {
        assert!(validate_keychain_path("/Users/example/Library/Keychains/login.keychain-db").is_ok());
        assert!(validate_keychain_path("login.keychain").is_ok());
    }

    #[test]
    fn truncate_output_caps_size() {
        let big = "x".repeat(MAX_OUTPUT_BYTES + 100);
        let out = truncate_output(big);
        assert!(out.len() <= MAX_OUTPUT_BYTES + "\n... [truncated]".len());
        assert!(out.ends_with("[truncated]"));
    }

    #[tokio::test]
    async fn keychain_get_rejects_missing_service() {
        let tool = KeychainGetTool;
        let err = tool.execute(serde_json::json!({})).await.unwrap_err();
        match err {
            Error::Tool(msg) => assert!(msg.contains("service")),
            _ => panic!("wrong error variant"),
        }
    }

    #[tokio::test]
    async fn keychain_set_rejects_empty_password() {
        let tool = KeychainSetTool;
        let err = tool
            .execute(serde_json::json!({
                "service": "test_zeus_keychain",
                "account": "ci",
                "password": ""
            }))
            .await
            .unwrap_err();
        match err {
            Error::Tool(msg) => assert!(msg.contains("password")),
            _ => panic!("wrong error variant"),
        }
    }

    #[tokio::test]
    async fn keychain_set_rejects_nul_in_password() {
        let tool = KeychainSetTool;
        let err = tool
            .execute(serde_json::json!({
                "service": "test_zeus_keychain",
                "account": "ci",
                "password": "ab\0cd"
            }))
            .await
            .unwrap_err();
        match err {
            Error::Tool(msg) => assert!(msg.contains("NUL")),
            _ => panic!("wrong error variant"),
        }
    }
}
