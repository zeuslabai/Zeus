//! Credential vault — server-side secret store for skill credential injection.
//!
//! Credentials are stored in the OS keychain (macOS Keychain / Linux Secret Service)
//! with a plaintext `[credentials]` config-based fallback for headless/FreeBSD servers.
//!
//! Values are **never** logged, never returned via REST (names only), and never
//! injected into LLM context — they are resolved only at `Command::spawn()` time
//! inside `SkillManager::execute()`.

use crate::Keychain;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use zeus_core::Result;

/// Server-side credential vault for skill API key injection.
///
/// Keys are injected as environment variables into skill subprocesses only —
/// they never appear in LLM context, system prompts, tool args, or audit logs.
///
/// ## Storage priority
/// 1. **OS keychain** (macOS Keychain / Linux Secret Service) — preferred
/// 2. **Config plaintext** (`[credentials]` section in config.toml) — fallback for
///    FreeBSD and headless servers where keychain is unavailable
///
/// ## OpenClaw skill format
/// ```yaml
/// metadata:
///   openclaw:
///     requires:
///       env: [GITHUB_TOKEN, OPENAI_API_KEY]
///     primaryEnv: GITHUB_TOKEN
/// ```
pub struct CredentialVault {
    /// OS keychain — None on unsupported platforms (FreeBSD, etc.)
    keychain: Option<Keychain>,
    /// Read-only plaintext fallback from `[credentials]` config section.
    config_store: HashMap<String, String>,
    /// In-memory index of names stored in the keychain (values never stored here).
    keychain_index: Arc<RwLock<HashSet<String>>>,
    /// Path for persisting the keychain name index (~/.zeus/credential_names.json).
    index_path: PathBuf,
    /// File-based credential store for platforms without OS keychain (FreeBSD, etc.).
    /// Values stored in `~/.zeus/credentials.json` with restricted permissions.
    file_store: Arc<RwLock<HashMap<String, String>>>,
    /// Path to the file-based credential store.
    file_store_path: PathBuf,
}

impl CredentialVault {
    /// Create a new `CredentialVault`.
    ///
    /// - `config_credentials`: plaintext fallback from `[credentials]` config section.
    /// - `index_dir`: directory for persisting the keychain name index file.
    pub fn new(config_credentials: HashMap<String, String>, index_dir: PathBuf) -> Self {
        let keychain = Keychain::new("zeus-credentials").ok();
        let index_path = index_dir.join("credential_names.json");
        let keychain_index = Self::load_index(&index_path);
        let file_store_path = index_dir.join("credentials.json");
        let file_store = Self::load_file_store(&file_store_path);
        Self {
            keychain,
            config_store: config_credentials,
            keychain_index: Arc::new(RwLock::new(keychain_index)),
            index_path,
            file_store: Arc::new(RwLock::new(file_store)),
            file_store_path,
        }
    }

    /// Load name index from disk (names only — no values).
    fn load_index(path: &PathBuf) -> HashSet<String> {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str::<HashSet<String>>(&s).ok())
            .unwrap_or_default()
    }

    /// Persist name index to disk.
    fn save_index(&self) {
        if let Ok(index) = self.keychain_index.read()
            && let Ok(json) = serde_json::to_string(&*index)
        {
            let _ = std::fs::write(&self.index_path, json);
        }
    }

    /// Load file-based credential store from disk.
    fn load_file_store(path: &PathBuf) -> HashMap<String, String> {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str::<HashMap<String, String>>(&s).ok())
            .unwrap_or_default()
    }

    /// Persist file-based credential store to disk with restricted permissions.
    fn save_file_store(&self) {
        if let Ok(store) = self.file_store.read()
            && let Ok(json) = serde_json::to_string_pretty(&*store)
        {
            let _ = std::fs::write(&self.file_store_path, &json);
            // Restrict permissions to owner-only (chmod 600)
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(
                    &self.file_store_path,
                    std::fs::Permissions::from_mode(0o600),
                );
            }
        }
    }

    /// Store a credential in the OS keychain.
    ///
    /// Returns an error on platforms without keychain support, directing the user
    /// to add the credential to `[credentials]` in config.toml instead.
    pub async fn store(&self, name: &str, value: &str) -> Result<()> {
        // Try OS keychain first
        if let Some(ref kc) = self.keychain {
            match kc.set(name, value).await {
                Ok(()) => {
                    if let Ok(mut index) = self.keychain_index.write() {
                        index.insert(name.to_string());
                    }
                    self.save_index();
                    return Ok(());
                }
                Err(_) => {
                    // Keychain failed (e.g. FreeBSD compiled with macOS cfg but no actual keychain)
                    // Fall through to file-based store
                }
            }
        }

        // Fallback: file-based credential store (~/.zeus/credentials.json, chmod 600)
        if let Ok(mut store) = self.file_store.write() {
            store.insert(name.to_string(), value.to_string());
        }
        self.save_file_store();
        Ok(())
    }

    /// Resolve a single credential value.
    ///
    /// Lookup order:
    /// 1. OS keychain
    /// 2. `[credentials]` config section
    /// 3. `None` (not found)
    pub async fn resolve(&self, name: &str) -> Result<Option<String>> {
        // 1. OS keychain
        if let Some(ref kc) = self.keychain
            && let Ok(Some(val)) = kc.get(name).await
        {
            return Ok(Some(val));
        }
        // 2. File-based credential store
        if let Ok(store) = self.file_store.read()
            && let Some(val) = store.get(name)
        {
            return Ok(Some(val.clone()));
        }
        // 3. Config plaintext fallback
        if let Some(val) = self.config_store.get(name) {
            return Ok(Some(val.clone()));
        }
        Ok(None)
    }

    /// Resolve multiple credential names in one call.
    ///
    /// Missing names are silently skipped (skill may have the env var already set,
    /// or the tool handles missing credentials itself).
    ///
    /// Returns a `HashMap<name, value>` for all resolved credentials.
    pub async fn resolve_map(&self, names: &[String]) -> Result<HashMap<String, String>> {
        let mut map = HashMap::with_capacity(names.len());
        for name in names {
            if let Ok(Some(val)) = self.resolve(name).await {
                map.insert(name.clone(), val);
            }
        }
        Ok(map)
    }

    /// Delete a credential from the OS keychain.
    ///
    /// No-op equivalent for config-based credentials: returns an error directing
    /// the user to remove the entry from config.toml manually.
    pub async fn delete(&self, name: &str) -> Result<()> {
        // Try OS keychain first
        if let Some(ref kc) = self.keychain {
            match kc.delete(name).await {
                Ok(()) => {
                    if let Ok(mut index) = self.keychain_index.write() {
                        index.remove(name);
                    }
                    self.save_index();
                    return Ok(());
                }
                Err(_) => {
                    // Fall through to file-based store
                }
            }
        }

        // Fallback: remove from file-based store
        if let Ok(mut store) = self.file_store.write() {
            store.remove(name);
        }
        self.save_file_store();
        Ok(())
    }

    /// List all known credential names. **Never returns values.**
    ///
    /// Returns the union of keychain-indexed names and config store keys,
    /// sorted alphabetically.
    pub fn list(&self) -> Vec<String> {
        let mut names: HashSet<String> = self.config_store.keys().cloned().collect();
        if let Ok(index) = self.keychain_index.read() {
            names.extend(index.iter().cloned());
        }
        if let Ok(store) = self.file_store.read() {
            names.extend(store.keys().cloned());
        }
        let mut sorted: Vec<String> = names.into_iter().collect();
        sorted.sort();
        sorted
    }

    /// Whether the OS keychain is available on this platform.
    pub fn has_keychain(&self) -> bool {
        self.keychain.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_vault(config: HashMap<String, String>) -> CredentialVault {
        let tmp = std::env::temp_dir().join(format!(
            "zeus_vault_test_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        CredentialVault::new(config, tmp)
    }

    #[tokio::test]
    async fn test_file_store_fallback() {
        let tmp = std::env::temp_dir().join(format!(
            "zeus_vault_file_test_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::create_dir_all(&tmp).unwrap();

        // Create vault, store via file fallback (keychain may or may not work)
        let vault = CredentialVault::new(HashMap::new(), tmp.clone());

        // Write directly to file store to test resolve
        if let Ok(mut store) = vault.file_store.write() {
            store.insert("TEST_FILE_KEY".to_string(), "file_value".to_string());
        }
        vault.save_file_store();

        // Resolve should find it
        let val = vault.resolve("TEST_FILE_KEY").await.unwrap();
        assert_eq!(val, Some("file_value".to_string()));

        // Reload from disk
        let vault2 = CredentialVault::new(HashMap::new(), tmp);
        let val2 = vault2.resolve("TEST_FILE_KEY").await.unwrap();
        assert_eq!(val2, Some("file_value".to_string()));

        // List should include it
        let names = vault2.list();
        assert!(names.contains(&"TEST_FILE_KEY".to_string()));
    }

    #[tokio::test]
    async fn test_resolve_from_config_store() {
        let mut cfg = HashMap::new();
        cfg.insert("GITHUB_TOKEN".to_string(), "ghp_test123".to_string());
        let vault = make_vault(cfg);

        let val = vault.resolve("GITHUB_TOKEN").await.unwrap();
        assert_eq!(val, Some("ghp_test123".to_string()));
    }

    #[tokio::test]
    async fn test_resolve_missing_returns_none() {
        let vault = make_vault(HashMap::new());
        let val = vault.resolve("NONEXISTENT_CRED_XYZ").await.unwrap();
        assert!(val.is_none());
    }

    #[tokio::test]
    async fn test_resolve_map_partial() {
        let mut cfg = HashMap::new();
        cfg.insert("KEY_A".to_string(), "value_a".to_string());
        cfg.insert("KEY_B".to_string(), "value_b".to_string());
        let vault = make_vault(cfg);

        let map = vault
            .resolve_map(&[
                "KEY_A".to_string(),
                "KEY_B".to_string(),
                "KEY_MISSING".to_string(),
            ])
            .await
            .unwrap();

        assert_eq!(map.get("KEY_A").unwrap(), "value_a");
        assert_eq!(map.get("KEY_B").unwrap(), "value_b");
        assert!(!map.contains_key("KEY_MISSING"));
    }

    #[tokio::test]
    async fn test_resolve_map_empty_names() {
        let vault = make_vault(HashMap::new());
        let map = vault.resolve_map(&[]).await.unwrap();
        assert!(map.is_empty());
    }

    #[test]
    fn test_list_returns_config_names() {
        let mut cfg = HashMap::new();
        cfg.insert("OPENAI_API_KEY".to_string(), "sk-xxx".to_string());
        cfg.insert("GITHUB_TOKEN".to_string(), "ghp_xxx".to_string());
        let vault = make_vault(cfg);

        let names = vault.list();
        assert!(names.contains(&"OPENAI_API_KEY".to_string()));
        assert!(names.contains(&"GITHUB_TOKEN".to_string()));
        // Values must never appear in list output
        assert!(
            !names
                .iter()
                .any(|n| n.contains("sk-") || n.contains("ghp_"))
        );
    }

    #[test]
    fn test_list_sorted() {
        let mut cfg = HashMap::new();
        cfg.insert("Z_KEY".to_string(), "z".to_string());
        cfg.insert("A_KEY".to_string(), "a".to_string());
        cfg.insert("M_KEY".to_string(), "m".to_string());
        let vault = make_vault(cfg);

        let names = vault.list();
        assert_eq!(names, vec!["A_KEY", "M_KEY", "Z_KEY"]);
    }

    #[test]
    fn test_list_empty() {
        let vault = make_vault(HashMap::new());
        assert!(vault.list().is_empty());
    }
}
