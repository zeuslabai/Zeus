//! Error Classification
//!
//! Classifies errors to determine retry strategy

use zeus_core::Error;

/// Error classification for determining retry strategy
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorClass {
    /// Transient network errors - retry with backoff
    Transient,
    /// Rate limit errors - rotate profile
    RateLimit,
    /// Billing/quota errors - rotate profile with long cooldown
    Billing,
    /// Authentication errors - rotate profile
    Auth,
    /// Context window overflow - compact and retry
    ContextOverflow,
    /// Provider unavailable - rotate profile
    ProviderUnavailable,
    /// Fatal errors - do not retry
    Fatal,
}

/// Classify an error to determine the appropriate response
pub fn classify_error(error: &Error) -> ErrorClass {
    let msg = error.to_string().to_lowercase();

    // Rate limiting
    if msg.contains("rate limit") || msg.contains("429") || msg.contains("too many requests") {
        return ErrorClass::RateLimit;
    }

    // Billing/quota
    if msg.contains("insufficient")
        || msg.contains("quota")
        || msg.contains("402")
        || msg.contains("billing")
    {
        return ErrorClass::Billing;
    }

    // Authentication
    if msg.contains("401")
        || msg.contains("403")
        || msg.contains("unauthorized")
        || msg.contains("authentication")
    {
        return ErrorClass::Auth;
    }

    // Context overflow
    if msg.contains("context")
        && (msg.contains("too long") || msg.contains("exceeds") || msg.contains("maximum"))
    {
        return ErrorClass::ContextOverflow;
    }

    // Provider unavailable
    if msg.contains("503") || msg.contains("unavailable") || msg.contains("service") {
        return ErrorClass::ProviderUnavailable;
    }

    // Transient network errors
    if msg.contains("timeout") || msg.contains("connection") || msg.contains("network") {
        return ErrorClass::Transient;
    }

    // Default to fatal
    ErrorClass::Fatal
}

/// Prometheus-specific error type
pub type PrometheusError = Error;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rate_limit_classification() {
        let err = Error::llm("Error 429: Rate limit exceeded");
        assert_eq!(classify_error(&err), ErrorClass::RateLimit);
    }

    #[test]
    fn test_billing_classification() {
        let err = Error::llm("Error 402: Insufficient credits");
        assert_eq!(classify_error(&err), ErrorClass::Billing);
    }

    #[test]
    fn test_context_classification() {
        let err = Error::llm("Context length exceeds maximum");
        assert_eq!(classify_error(&err), ErrorClass::ContextOverflow);
    }

    #[test]
    fn test_network_classification() {
        let err = Error::Network("Connection timeout".to_string());
        assert_eq!(classify_error(&err), ErrorClass::Transient);
    }
}
