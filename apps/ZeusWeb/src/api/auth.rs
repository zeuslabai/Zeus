// Auth, credentials, OAuth, security

use super::*;

pub async fn fetch_auth_status() -> Result<AuthStatusResponse, String> {
    fetch_json("/v1/auth/status").await
}

pub async fn auth_login() -> Result<AuthLoginResponse, String> {
    post_json("/v1/auth/login", &serde_json::json!({})).await
}

pub async fn auth_login_provider(provider: &str, redirect_uri: &str, state: &str, code_verifier: &str) -> Result<AuthLoginResponse, String> {
    post_json("/v1/auth/login", &serde_json::json!({
        "provider": provider,
        "redirect_uri": redirect_uri,
        "state": state,
        "code_verifier": code_verifier,
    })).await
}

pub async fn auth_token(token: &str) -> Result<AuthTokenResponse, String> {
    post_json("/v1/auth/token", &serde_json::json!({ "token": token })).await
}

pub async fn auth_logout() -> Result<AuthTokenResponse, String> {
    post_json("/v1/auth/logout", &serde_json::json!({})).await
}

/// Store an API key in CredentialVault via POST /v1/credentials (S54 Track A).
/// Previously routed to POST /v1/auth/token → OAuthManager; now goes to
/// CredentialVault → keychain / config.credentials fallback.
pub async fn auth_save_credentials(provider: &str, api_key: &str) -> Result<AuthCallbackResponse, String> {
    let key_name = provider_to_key_name(provider);
    if key_name == "UNKNOWN_API_KEY" {
        return Err(format!(
            "Unknown provider '{}': cannot determine credential name. \
             Add it to provider_to_key_name().",
            provider
        ));
    }
    post_json("/v1/credentials", &serde_json::json!({
        "name": key_name,
        "value": api_key,
    })).await
}

/// Store an OAuth setup token (sk-ant-oat01-...) via /v1/auth/token
pub async fn auth_store_oauth_token(token: &str) -> Result<AuthCallbackResponse, String> {
    post_json("/v1/auth/token", &serde_json::json!({
        "token": token,
    })).await
}

pub async fn auth_oauth_callback(code: &str, code_verifier: &str, provider: &str, redirect_uri: &str) -> Result<AuthCallbackResponse, String> {
    post_json("/v1/auth/token", &serde_json::json!({
        "code": code,
        "code_verifier": code_verifier,
        "provider": provider,
        "redirect_uri": redirect_uri,
    })).await
}

pub async fn store_credential(name: &str, value: &str) -> Result<MsgResponse, String> {
    post_json("/v1/credentials", &serde_json::json!({ "name": name, "value": value })).await
}

pub async fn update_permissions(perms: &GlobalPerms) -> Result<MsgResponse, String> {
    put_json("/v1/security/permissions", perms).await
}

// Security

pub async fn fetch_keys() -> Result<KeysResponse, String> {
    fetch_json("/v1/security/keys").await
}

pub async fn fetch_permissions() -> Result<PermissionsResponse, String> {
    fetch_json("/v1/security/permissions").await
}

pub async fn fetch_threats() -> Result<ThreatsResponse, String> {
    fetch_json("/v1/security/threats").await
}

pub async fn fetch_allowlist() -> Result<AllowlistResponse, String> {
    fetch_json("/v1/security/allowlist").await
}

pub async fn update_allowlist(commands: &[String]) -> Result<MsgResponse, String> {
    put_json("/v1/security/allowlist", &serde_json::json!({ "allowlist": commands })).await
}

pub async fn fetch_audit_log() -> Result<SecurityAuditResponse, String> {
    fetch_json("/v1/security/audit").await
}

pub async fn rotate_api_key() -> Result<MsgResponse, String> {
    post_json("/v1/security/rotate-key", &serde_json::json!({})).await
}

pub async fn fetch_rotation_status() -> Result<RotationStatusResponse, String> {
    fetch_json("/v1/security/rotation-status").await
}

// Anthropic OAuth

pub async fn fetch_anthropic_oauth_status() -> Result<AnthropicOAuthStatus, String> {
    fetch_json("/v1/auth/anthropic/status").await
}
