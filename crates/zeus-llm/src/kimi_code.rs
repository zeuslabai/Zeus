//! Kimi Code subscription backend constants.
//!
//! Kimi Code is Moonshot/Kimi's membership/subscription API. It is separate
//! from Moonshot PAYG and exposes an OpenAI-compatible Chat Completions surface.
//!
//! Contract from Kimi Code docs:
//! - Base URL: https://api.kimi.com/coding/v1
//! - Endpoint: /chat/completions
//! - Auth: Authorization: Bearer <API_KEY>
//! - Key source: KIMI_CODE_API_KEY / [credentials.kimi-code]

/// OpenAI-compatible base URL for Kimi Code subscription inference.
pub const KIMI_CODE_BASE_URL: &str = "https://api.kimi.com/coding/v1";

/// Environment variable used for the manually-created Kimi Code API key.
pub const KIMI_CODE_API_KEY_ENV: &str = "KIMI_CODE_API_KEY";

/// Kimi K3 model id accepted by the Kimi Code API.
pub const KIMI_K3_MODEL: &str = "k3";

/// Kimi K2.7 Code model id accepted by the Kimi Code API.
pub const KIMI_FOR_CODING_MODEL: &str = "kimi-for-coding";

/// Kimi K2.7 HighSpeed model id accepted by the Kimi Code API.
pub const KIMI_FOR_CODING_HIGHSPEED_MODEL: &str = "kimi-for-coding-highspeed";
