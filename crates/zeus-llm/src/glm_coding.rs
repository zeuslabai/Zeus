//! GLM Coding subscription backend constants.
//!
//! GLM Coding is z.ai/GLM's dedicated flat-rate Coding Plan API. It is
//! separate from PAYG GLM/Zai and exposes an OpenAI-compatible Chat
//! Completions surface.
//!
//! Contract from #412/#413 investigation:
//! - Base URL: https://api.z.ai/api/coding/paas/v4
//! - Endpoint: /chat/completions
//! - Auth: Authorization: Bearer <API_KEY>
//! - Key source: GLM_CODING_API_KEY / [credentials.glm-coding]

/// OpenAI-compatible base URL for GLM Coding subscription inference.
pub const GLM_CODING_BASE_URL: &str = "https://api.z.ai/api/coding/paas/v4";

/// Environment variable used for the dedicated GLM Coding Plan API key.
pub const GLM_CODING_API_KEY_ENV: &str = "GLM_CODING_API_KEY";

/// GLM-5.2 model id accepted by the GLM Coding API.
pub const GLM_5_2_MODEL: &str = "glm-5.2";

/// GLM-5 Turbo model id accepted by the GLM Coding API.
pub const GLM_5_TURBO_MODEL: &str = "glm-5-turbo";

/// GLM-4.7 model id accepted by the GLM Coding API.
pub const GLM_4_7_MODEL: &str = "glm-4.7";
