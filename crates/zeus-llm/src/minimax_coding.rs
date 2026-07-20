//! MiniMax Coding subscription backend constants.
//!
//! MiniMax Coding is MiniMax's flat-rate Token Plan API. It is separate from
//! the existing MiniMax PAYG/OAuth path and mirrors that arm's Anthropic
//! Messages request surface.
//!
//! Contract from #415:
//! - Base URL: https://api.minimax.io/anthropic
//! - Endpoint: /v1/messages
//! - Auth: Authorization: Bearer <API_KEY>
//! - Key source: MINIMAX_CODING_API_KEY / [credentials.minimax-coding]

/// Anthropic-compatible base URL for MiniMax Token Plan subscription inference.
pub const MINIMAX_CODING_BASE_URL: &str = "https://api.minimax.io/anthropic";

/// Environment variable used for the dedicated MiniMax Token Plan API key.
pub const MINIMAX_CODING_API_KEY_ENV: &str = "MINIMAX_CODING_API_KEY";

/// MiniMax M2.5 model id accepted by the MiniMax Token Plan API.
pub const MINIMAX_M2_5_MODEL: &str = "MiniMax-M2.5";

/// MiniMax M3 model id accepted by the MiniMax Token Plan API.
pub const MINIMAX_M3_MODEL: &str = "MiniMax-M3";

/// MiniMax M2.7 model id accepted by the MiniMax Token Plan API.
pub const MINIMAX_M2_7_MODEL: &str = "MiniMax-M2.7";
