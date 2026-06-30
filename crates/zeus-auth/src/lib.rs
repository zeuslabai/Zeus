//! zeus-auth — OAuth PKCE authentication flows.
//!
//! Implements Authorization Code + PKCE for OAuth providers
//! (OpenAI, Anthropic, Google/Gemini). Browser-based login with local callback server.
//! Also supports importing existing Gemini CLI credentials from `~/.gemini/`.
//!
//! # Usage
//! ```rust,ignore
//! use zeus_auth::flow::{OAuthProvider, run_oauth_flow};
//!
//! let provider = OAuthProvider::openai("your-client-id");
//! let tokens = run_oauth_flow(&provider).await?;
//! println!("Access token: {}", tokens.access_token);
//! ```

pub mod callback;
pub mod flow;
pub mod pkce;

pub use flow::{
    OAuthProvider, TokenResponse, run_oauth_flow, refresh_token,
    GeminiCliCredentials, import_gemini_cli_credentials, refresh_gemini_cli_token,
    extract_gemini_cli_credentials, discover_google_project, fetch_google_user_email,
    DeviceCodeProvider, DeviceCodeResponse, run_device_code_flow,
};
pub use pkce::PkceChallenge;
