//! Qwen Coding subscription backend constants.
//!
//! Qwen Coding is Alibaba/Qwen's dedicated flat-rate Coding Plan API. It is
//! separate from the existing Qwen OAuth/standard DashScope path and exposes an
//! OpenAI-compatible Chat Completions surface.
//!
//! Contract from #418:
//! - Base URL: https://coding.dashscope.aliyuncs.com/v1
//! - Endpoint: /chat/completions
//! - Auth: Authorization: Bearer <API_KEY>
//! - Key source: QWEN_CODING_API_KEY / [credentials.qwen-coding]

/// OpenAI-compatible base URL for Qwen Coding subscription inference.
pub const QWEN_CODING_BASE_URL: &str = "https://coding.dashscope.aliyuncs.com/v1";

/// Environment variable used for the dedicated Qwen Coding Plan API key.
pub const QWEN_CODING_API_KEY_ENV: &str = "QWEN_CODING_API_KEY";

/// Qwen Max Preview model id accepted by the Qwen Coding API.
pub const QWEN3_8_MAX_PREVIEW_MODEL: &str = "qwen3.8-max-preview";

/// Qwen Coder model id accepted by the Qwen Coding API.
pub const QWEN3_CODER_MODEL: &str = "qwen3-coder";
