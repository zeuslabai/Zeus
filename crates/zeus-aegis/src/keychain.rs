//! OS keychain integration

use zeus_core::{Error, Result};

/// Keychain abstraction for secret storage
pub struct Keychain {
    #[allow(dead_code)]
    service: String,
}

impl Keychain {
    /// Create a new keychain instance
    pub fn new(service: &str) -> Result<Self> {
        Ok(Self {
            service: service.to_string(),
        })
    }

    /// Get a secret from the keychain
    #[cfg(target_os = "macos")]
    pub async fn get(&self, key: &str) -> Result<Option<String>> {
        use security_framework::passwords::get_generic_password;

        match get_generic_password(&self.service, key) {
            Ok(bytes) => {
                let value = String::from_utf8(bytes.to_vec())
                    .map_err(|e| Error::Security(format!("Invalid UTF-8 in keychain: {}", e)))?;
                Ok(Some(value))
            }
            Err(e) if e.code() == -25300 => Ok(None), // errSecItemNotFound
            Err(e) => Err(Error::Security(format!("Keychain error: {}", e))),
        }
    }

    /// Get a secret from the keychain (Linux)
    #[cfg(target_os = "linux")]
    pub async fn get(&self, key: &str) -> Result<Option<String>> {
        use secret_service::EncryptionType;
        use secret_service::SecretService;
        use std::collections::HashMap;

        let ss = SecretService::connect(EncryptionType::Dh)
            .await
            .map_err(|e| Error::Security(format!("Secret service error: {}", e)))?;

        let collection = ss
            .get_default_collection()
            .await
            .map_err(|e| Error::Security(format!("Secret service error: {}", e)))?;

        let attributes: HashMap<&str, &str> =
            HashMap::from([("service", self.service.as_str()), ("key", key)]);

        let items = collection
            .search_items(attributes)
            .await
            .map_err(|e| Error::Security(format!("Secret service error: {}", e)))?;

        if let Some(item) = items.first() {
            let secret = item
                .get_secret()
                .await
                .map_err(|e| Error::Security(format!("Secret service error: {}", e)))?;
            let value = String::from_utf8(secret)
                .map_err(|e| Error::Security(format!("Invalid UTF-8: {}", e)))?;
            Ok(Some(value))
        } else {
            Ok(None)
        }
    }

    /// Fallback for other platforms
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    pub async fn get(&self, _key: &str) -> Result<Option<String>> {
        Err(Error::Security(
            "Keychain not supported on this platform".into(),
        ))
    }

    /// Set a secret in the keychain
    #[cfg(target_os = "macos")]
    pub async fn set(&self, key: &str, value: &str) -> Result<()> {
        use security_framework::passwords::{delete_generic_password, set_generic_password};

        // Delete existing item first (ignore errors)
        let _ = delete_generic_password(&self.service, key);

        set_generic_password(&self.service, key, value.as_bytes())
            .map_err(|e| Error::Security(format!("Keychain error: {}", e)))
    }

    /// Set a secret in the keychain (Linux)
    #[cfg(target_os = "linux")]
    pub async fn set(&self, key: &str, value: &str) -> Result<()> {
        use secret_service::EncryptionType;
        use secret_service::SecretService;
        use std::collections::HashMap;

        let ss = SecretService::connect(EncryptionType::Dh)
            .await
            .map_err(|e| Error::Security(format!("Secret service error: {}", e)))?;

        let collection = ss
            .get_default_collection()
            .await
            .map_err(|e| Error::Security(format!("Secret service error: {}", e)))?;

        let attributes: HashMap<&str, &str> =
            HashMap::from([("service", self.service.as_str()), ("key", key)]);

        collection
            .create_item(
                &format!("zeus:{}", key),
                attributes,
                value.as_bytes(),
                true, // replace
                "text/plain",
            )
            .await
            .map_err(|e| Error::Security(format!("Secret service error: {}", e)))?;

        Ok(())
    }

    /// Fallback for other platforms
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    pub async fn set(&self, _key: &str, _value: &str) -> Result<()> {
        Err(Error::Security(
            "Keychain not supported on this platform".into(),
        ))
    }

    /// Delete a secret from the keychain
    #[cfg(target_os = "macos")]
    pub async fn delete(&self, key: &str) -> Result<()> {
        use security_framework::passwords::delete_generic_password;

        delete_generic_password(&self.service, key)
            .map_err(|e| Error::Security(format!("Keychain error: {}", e)))
    }

    /// Delete a secret from the keychain (Linux)
    #[cfg(target_os = "linux")]
    pub async fn delete(&self, key: &str) -> Result<()> {
        use secret_service::EncryptionType;
        use secret_service::SecretService;
        use std::collections::HashMap;

        let ss = SecretService::connect(EncryptionType::Dh)
            .await
            .map_err(|e| Error::Security(format!("Secret service error: {}", e)))?;

        let collection = ss
            .get_default_collection()
            .await
            .map_err(|e| Error::Security(format!("Secret service error: {}", e)))?;

        let attributes: HashMap<&str, &str> =
            HashMap::from([("service", self.service.as_str()), ("key", key)]);

        let items = collection
            .search_items(attributes)
            .await
            .map_err(|e| Error::Security(format!("Secret service error: {}", e)))?;

        for item in items {
            item.delete()
                .await
                .map_err(|e| Error::Security(format!("Secret service error: {}", e)))?;
        }

        Ok(())
    }

    /// Fallback for other platforms
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    pub async fn delete(&self, _key: &str) -> Result<()> {
        Err(Error::Security(
            "Keychain not supported on this platform".into(),
        ))
    }
}
