//! API key authentication middleware
//!
//! Supports `X-Zeus-Api-Key` header authentication alongside Bearer tokens.
//! API keys are loaded from `ZEUS_API_KEYS` (comma-separated) or
//! `ZEUS_API_KEY` (single key) environment variables.

use std::sync::Arc;

/// Validated set of API keys for constant-time lookup.
#[derive(Debug, Clone)]
pub struct ApiKeyValidator {
    keys: Arc<Vec<String>>,
}

impl ApiKeyValidator {
    /// Create a new validator with the given keys.
    pub fn new(keys: Vec<String>) -> Self {
        let keys: Vec<String> = keys.into_iter().filter(|k| !k.is_empty()).collect();
        Self {
            keys: Arc::new(keys),
        }
    }

    /// Load API keys from environment variables.
    ///
    /// Checks `ZEUS_API_KEYS` (comma-separated) first, then falls back
    /// to `ZEUS_API_KEY` (single key).
    pub fn from_env() -> Self {
        let keys = if let Ok(keys_str) = std::env::var("ZEUS_API_KEYS") {
            keys_str
                .split(',')
                .map(|k| k.trim().to_string())
                .filter(|k| !k.is_empty())
                .collect()
        } else if let Ok(key) = std::env::var("ZEUS_API_KEY") {
            if key.is_empty() {
                Vec::new()
            } else {
                vec![key]
            }
        } else {
            Vec::new()
        };
        Self::new(keys)
    }

    /// Check if the given key is valid (constant-time comparison).
    ///
    /// Iterates ALL keys without short-circuiting so timing does not reveal
    /// which key matched or how many keys precede the matching one.
    pub fn validate(&self, candidate: &str) -> bool {
        if self.keys.is_empty() {
            return false;
        }
        let mut result = false;
        for key in self.keys.iter() {
            result |= constant_time_eq(candidate, key);
        }
        result
    }

    /// Returns true if at least one API key is configured.
    pub fn has_keys(&self) -> bool {
        !self.keys.is_empty()
    }

    /// Number of configured API keys.
    pub fn key_count(&self) -> usize {
        self.keys.len()
    }
}

/// Constant-time string comparison to prevent timing attacks.
pub(crate) fn constant_time_eq(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.as_bytes()
        .iter()
        .zip(b.as_bytes().iter())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    // Mutex to serialize tests that manipulate environment variables
    static ENV_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn test_api_key_validator_basic() {
        let validator = ApiKeyValidator::new(vec![
            "key-alpha-1234567890".to_string(),
            "key-beta-0987654321".to_string(),
        ]);

        assert!(validator.validate("key-alpha-1234567890"));
        assert!(validator.validate("key-beta-0987654321"));
        assert!(!validator.validate("wrong-key"));
        assert!(!validator.validate(""));
        assert!(validator.has_keys());
        assert_eq!(validator.key_count(), 2);
    }

    #[test]
    fn test_api_key_validator_empty() {
        let validator = ApiKeyValidator::new(vec![]);
        assert!(!validator.validate("any-key"));
        assert!(!validator.has_keys());
        assert_eq!(validator.key_count(), 0);
    }

    #[test]
    fn test_api_key_validator_filters_empty_strings() {
        let validator = ApiKeyValidator::new(vec![
            "".to_string(),
            "valid-key-12345678".to_string(),
            "".to_string(),
        ]);
        assert_eq!(validator.key_count(), 1);
        assert!(validator.validate("valid-key-12345678"));
    }

    #[test]
    fn test_constant_time_eq() {
        assert!(constant_time_eq("hello", "hello"));
        assert!(!constant_time_eq("hello", "world"));
        assert!(!constant_time_eq("hello", "hell"));
        assert!(!constant_time_eq("short", "longer-string"));
        assert!(constant_time_eq("", ""));
    }

    #[test]
    fn test_api_key_validator_single_key() {
        let validator = ApiKeyValidator::new(vec!["only-key-123456789".to_string()]);
        assert!(validator.validate("only-key-123456789"));
        assert!(!validator.validate("not-the-key"));
        assert_eq!(validator.key_count(), 1);
    }

    #[test]
    fn test_from_env_with_zeus_api_keys() {
        let _guard = ENV_MUTEX
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        // SAFETY: Test-only code; serialized by ENV_MUTEX
        unsafe {
            std::env::set_var("ZEUS_API_KEYS", "key-a-1234,key-b-5678,key-c-9012");
            std::env::remove_var("ZEUS_API_KEY");
        }

        let validator = ApiKeyValidator::from_env();
        assert_eq!(validator.key_count(), 3);
        assert!(validator.validate("key-a-1234"));
        assert!(validator.validate("key-b-5678"));
        assert!(validator.validate("key-c-9012"));

        // Clean up
        unsafe {
            std::env::remove_var("ZEUS_API_KEYS");
        }
    }

    #[test]
    fn test_from_env_with_zeus_api_key() {
        let _guard = ENV_MUTEX
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        // SAFETY: Test-only code; serialized by ENV_MUTEX
        unsafe {
            std::env::remove_var("ZEUS_API_KEYS");
            std::env::set_var("ZEUS_API_KEY", "single-key-123");
        }

        let validator = ApiKeyValidator::from_env();
        assert_eq!(validator.key_count(), 1);
        assert!(validator.validate("single-key-123"));

        // Clean up
        unsafe {
            std::env::remove_var("ZEUS_API_KEY");
        }
    }

    #[test]
    fn test_from_env_no_keys() {
        let _guard = ENV_MUTEX
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        // SAFETY: Test-only code; serialized by ENV_MUTEX
        unsafe {
            std::env::remove_var("ZEUS_API_KEYS");
            std::env::remove_var("ZEUS_API_KEY");
        }

        let validator = ApiKeyValidator::from_env();
        assert_eq!(validator.key_count(), 0);
        assert!(!validator.has_keys());
    }
}
