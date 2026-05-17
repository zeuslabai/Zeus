//! Network Discovery & Node Pairing
//!
//! mDNS-based service discovery for Zeus gateway. Client devices (iOS, Android,
//! Desktop) discover the gateway on the local network via `_zeus._tcp` service.
//! Pairing uses a 6-digit code exchange over the discovered connection.

use std::collections::HashMap;
#[cfg(test)]
use std::net::Ipv4Addr;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

/// Service type for mDNS advertisement
pub const SERVICE_TYPE: &str = "_zeus._tcp";

/// Default gateway port
pub const DEFAULT_PORT: u16 = 3001;

/// Pairing code expiry time
const PAIRING_CODE_EXPIRY: Duration = Duration::from_secs(300); // 5 minutes

/// A discovered Zeus node on the network
#[derive(Debug, Clone)]
pub struct DiscoveredNode {
    /// Unique instance name (e.g., "zeus-gateway-abc123")
    pub instance_name: String,
    /// Hostname
    pub hostname: String,
    /// IP addresses
    pub addresses: Vec<IpAddr>,
    /// Port
    pub port: u16,
    /// Service properties (version, capabilities, etc.)
    pub properties: HashMap<String, String>,
    /// When this node was discovered
    pub discovered_at: Instant,
    /// When we last saw this node
    pub last_seen: Instant,
    /// Whether this node is paired
    pub paired: bool,
}

impl DiscoveredNode {
    /// Get the primary socket address
    pub fn primary_addr(&self) -> Option<SocketAddr> {
        self.addresses
            .first()
            .map(|ip| SocketAddr::new(*ip, self.port))
    }

    /// Get the API base URL
    pub fn api_url(&self) -> Option<String> {
        self.primary_addr().map(|addr| format!("http://{}", addr))
    }

    /// Check if the node is still considered alive (seen within last 2 minutes)
    pub fn is_alive(&self) -> bool {
        self.last_seen.elapsed() < Duration::from_secs(120)
    }

    /// Get the Zeus version from properties
    pub fn version(&self) -> Option<&str> {
        self.properties.get("version").map(|s| s.as_str())
    }
}

/// A pending pairing session
#[derive(Debug, Clone)]
pub struct PairingSession {
    /// 6-digit pairing code
    pub code: String,
    /// The device requesting pairing
    pub device_name: String,
    /// Device type (ios, android, desktop, cli)
    pub device_type: String,
    /// When the session was created
    pub created_at: Instant,
    /// Whether pairing was completed
    pub completed: bool,
}

impl PairingSession {
    pub fn is_expired(&self) -> bool {
        self.created_at.elapsed() > PAIRING_CODE_EXPIRY
    }
}

/// Gateway advertisement configuration
#[derive(Debug, Clone)]
pub struct AdvertiseConfig {
    /// Instance name (must be unique on network)
    pub instance_name: String,
    /// Port to advertise
    pub port: u16,
    /// Additional TXT record properties
    pub properties: HashMap<String, String>,
}

impl Default for AdvertiseConfig {
    fn default() -> Self {
        let mut properties = HashMap::new();
        properties.insert("version".into(), env!("CARGO_PKG_VERSION").into());
        properties.insert("api".into(), "rest".into());

        Self {
            instance_name: format!("zeus-gateway-{}", &generate_instance_id()),
            port: DEFAULT_PORT,
            properties,
        }
    }
}

/// Generate a short unique instance ID
fn generate_instance_id() -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    let hostname = hostname::get()
        .map(|h| h.to_string_lossy().to_string())
        .unwrap_or_else(|_| "unknown".into());
    hostname.hash(&mut hasher);
    std::process::id().hash(&mut hasher);
    format!("{:08x}", hasher.finish() & 0xFFFF_FFFF)
}

/// Node discovery manager
///
/// Handles both advertising (gateway side) and browsing (client side).
pub struct DiscoveryManager {
    /// Known nodes on the network
    nodes: Arc<RwLock<HashMap<String, DiscoveredNode>>>,
    /// Active pairing sessions
    pairing_sessions: Arc<RwLock<HashMap<String, PairingSession>>>,
    /// Paired device tokens
    paired_devices: Arc<RwLock<HashMap<String, PairedDevice>>>,
    /// Whether we are advertising
    advertising: Arc<RwLock<bool>>,
    /// Whether we are browsing
    browsing: Arc<RwLock<bool>>,
    /// Advertisement config
    config: AdvertiseConfig,
}

/// A paired device record
#[derive(Debug, Clone)]
pub struct PairedDevice {
    pub device_name: String,
    pub device_type: String,
    pub token: String,
    pub paired_at: Instant,
    pub last_connected: Option<Instant>,
}

impl DiscoveryManager {
    pub fn new(config: AdvertiseConfig) -> Self {
        Self {
            nodes: Arc::new(RwLock::new(HashMap::new())),
            pairing_sessions: Arc::new(RwLock::new(HashMap::new())),
            paired_devices: Arc::new(RwLock::new(HashMap::new())),
            advertising: Arc::new(RwLock::new(false)),
            browsing: Arc::new(RwLock::new(false)),
            config,
        }
    }

    pub fn with_defaults() -> Self {
        Self::new(AdvertiseConfig::default())
    }

    /// Start advertising this gateway on the network via mDNS.
    ///
    /// Uses the `_zeus._tcp` service type. Other Zeus clients can discover
    /// this instance by browsing for the same service type.
    pub async fn start_advertising(&self) -> Result<(), DiscoveryError> {
        let mut advertising = self.advertising.write().await;
        if *advertising {
            return Ok(()); // Already advertising
        }

        // In a real implementation, this would use mdns-sd or similar crate
        // to register a DNS-SD service. For now we record that we intend to advertise.
        *advertising = true;
        tracing::info!(
            instance = %self.config.instance_name,
            port = self.config.port,
            "Started mDNS advertisement for {}",
            SERVICE_TYPE,
        );
        Ok(())
    }

    /// Stop advertising
    pub async fn stop_advertising(&self) {
        let mut advertising = self.advertising.write().await;
        *advertising = false;
        tracing::info!("Stopped mDNS advertisement");
    }

    /// Start browsing for Zeus gateways on the network.
    pub async fn start_browsing(&self) -> Result<(), DiscoveryError> {
        let mut browsing = self.browsing.write().await;
        if *browsing {
            return Ok(());
        }
        *browsing = true;
        tracing::info!("Started mDNS browsing for {}", SERVICE_TYPE);
        Ok(())
    }

    /// Stop browsing
    pub async fn stop_browsing(&self) {
        let mut browsing = self.browsing.write().await;
        *browsing = false;
    }

    /// Register a discovered node (called by mDNS browser callback)
    pub async fn register_node(&self, node: DiscoveredNode) {
        let mut nodes = self.nodes.write().await;
        nodes.insert(node.instance_name.clone(), node);
    }

    /// Get all discovered nodes
    pub async fn discovered_nodes(&self) -> Vec<DiscoveredNode> {
        let nodes = self.nodes.read().await;
        nodes.values().filter(|n| n.is_alive()).cloned().collect()
    }

    /// Get a specific node by instance name
    pub async fn get_node(&self, instance_name: &str) -> Option<DiscoveredNode> {
        let nodes = self.nodes.read().await;
        nodes.get(instance_name).cloned()
    }

    /// Remove stale nodes (not seen for > 2 minutes)
    pub async fn prune_stale_nodes(&self) -> usize {
        let mut nodes = self.nodes.write().await;
        let before = nodes.len();
        nodes.retain(|_, n| n.is_alive());
        before - nodes.len()
    }

    /// Generate a pairing code for a new device
    pub async fn create_pairing_session(&self, device_name: &str, device_type: &str) -> String {
        let code = generate_pairing_code();
        let session = PairingSession {
            code: code.clone(),
            device_name: device_name.into(),
            device_type: device_type.into(),
            created_at: Instant::now(),
            completed: false,
        };

        let mut sessions = self.pairing_sessions.write().await;
        // Clean expired sessions
        sessions.retain(|_, s| !s.is_expired());
        sessions.insert(code.clone(), session);

        code
    }

    /// Verify a pairing code and complete pairing
    pub async fn complete_pairing(&self, code: &str) -> Result<PairedDevice, DiscoveryError> {
        let mut sessions = self.pairing_sessions.write().await;

        let session = sessions
            .get_mut(code)
            .ok_or(DiscoveryError::InvalidPairingCode)?;

        if session.is_expired() {
            sessions.remove(code);
            return Err(DiscoveryError::PairingExpired);
        }

        if session.completed {
            return Err(DiscoveryError::PairingAlreadyUsed);
        }

        session.completed = true;

        let token = generate_device_token();
        let device = PairedDevice {
            device_name: session.device_name.clone(),
            device_type: session.device_type.clone(),
            token: token.clone(),
            paired_at: Instant::now(),
            last_connected: None,
        };

        let mut devices = self.paired_devices.write().await;
        devices.insert(token, device.clone());

        Ok(device)
    }

    /// Validate a device token
    pub async fn validate_token(&self, token: &str) -> bool {
        let devices = self.paired_devices.read().await;
        devices.contains_key(token)
    }

    /// Update last-connected time for a device
    pub async fn touch_device(&self, token: &str) {
        let mut devices = self.paired_devices.write().await;
        if let Some(device) = devices.get_mut(token) {
            device.last_connected = Some(Instant::now());
        }
    }

    /// List all paired devices
    pub async fn paired_devices(&self) -> Vec<PairedDevice> {
        let devices = self.paired_devices.read().await;
        devices.values().cloned().collect()
    }

    /// Revoke a device token
    pub async fn revoke_device(&self, token: &str) -> bool {
        let mut devices = self.paired_devices.write().await;
        devices.remove(token).is_some()
    }

    /// Check if advertising is active
    pub async fn is_advertising(&self) -> bool {
        *self.advertising.read().await
    }

    /// Check if browsing is active
    pub async fn is_browsing(&self) -> bool {
        *self.browsing.read().await
    }
}

/// Generate a 6-digit pairing code
fn generate_pairing_code() -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    Instant::now().hash(&mut hasher);
    std::thread::current().id().hash(&mut hasher);
    let n = hasher.finish() % 1_000_000;
    format!("{:06}", n)
}

/// Generate a device token (64 hex chars)
fn generate_device_token() -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    Instant::now().hash(&mut hasher);
    std::process::id().hash(&mut hasher);
    let h1 = hasher.finish();
    std::thread::current().id().hash(&mut hasher);
    let h2 = hasher.finish();
    format!(
        "{:016x}{:016x}{:016x}{:016x}",
        h1,
        h2,
        h1 ^ h2,
        h2.wrapping_mul(31)
    )
}

/// Discovery errors
#[derive(Debug, thiserror::Error)]
pub enum DiscoveryError {
    #[error("Invalid pairing code")]
    InvalidPairingCode,
    #[error("Pairing code expired")]
    PairingExpired,
    #[error("Pairing code already used")]
    PairingAlreadyUsed,
    #[error("mDNS error: {0}")]
    Mdns(String),
    #[error("Network error: {0}")]
    Network(String),
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_advertise_config_default() {
        let config = AdvertiseConfig::default();
        assert!(config.instance_name.starts_with("zeus-gateway-"));
        assert_eq!(config.port, DEFAULT_PORT);
        assert!(config.properties.contains_key("version"));
    }

    #[test]
    fn test_generate_pairing_code() {
        let code = generate_pairing_code();
        assert_eq!(code.len(), 6);
        assert!(code.chars().all(|c| c.is_ascii_digit()));
    }

    #[test]
    fn test_generate_device_token() {
        let token = generate_device_token();
        assert_eq!(token.len(), 64);
        assert!(token.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_discovered_node_api_url() {
        let node = DiscoveredNode {
            instance_name: "test".into(),
            hostname: "zeus.local".into(),
            addresses: vec![IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100))],
            port: 3001,
            properties: HashMap::new(),
            discovered_at: Instant::now(),
            last_seen: Instant::now(),
            paired: false,
        };
        assert_eq!(node.api_url(), Some("http://192.168.1.100:3001".into()));
        assert!(node.is_alive());
    }

    #[test]
    fn test_discovered_node_no_addresses() {
        let node = DiscoveredNode {
            instance_name: "empty".into(),
            hostname: "x".into(),
            addresses: vec![],
            port: 3001,
            properties: HashMap::new(),
            discovered_at: Instant::now(),
            last_seen: Instant::now(),
            paired: false,
        };
        assert!(node.primary_addr().is_none());
        assert!(node.api_url().is_none());
    }

    #[test]
    fn test_pairing_session_expiry() {
        let session = PairingSession {
            code: "123456".into(),
            device_name: "iPhone".into(),
            device_type: "ios".into(),
            created_at: Instant::now(),
            completed: false,
        };
        assert!(!session.is_expired());
    }

    #[tokio::test]
    async fn test_discovery_manager_new() {
        let mgr = DiscoveryManager::with_defaults();
        assert!(!mgr.is_advertising().await);
        assert!(!mgr.is_browsing().await);
        assert!(mgr.discovered_nodes().await.is_empty());
    }

    #[tokio::test]
    async fn test_start_stop_advertising() {
        let mgr = DiscoveryManager::with_defaults();
        mgr.start_advertising()
            .await
            .expect("async operation should succeed");
        assert!(mgr.is_advertising().await);
        mgr.stop_advertising().await;
        assert!(!mgr.is_advertising().await);
    }

    #[tokio::test]
    async fn test_start_stop_browsing() {
        let mgr = DiscoveryManager::with_defaults();
        mgr.start_browsing()
            .await
            .expect("async operation should succeed");
        assert!(mgr.is_browsing().await);
        mgr.stop_browsing().await;
        assert!(!mgr.is_browsing().await);
    }

    #[tokio::test]
    async fn test_register_and_list_nodes() {
        let mgr = DiscoveryManager::with_defaults();
        let node = DiscoveredNode {
            instance_name: "gateway-1".into(),
            hostname: "zeus.local".into(),
            addresses: vec![IpAddr::V4(Ipv4Addr::new(192, 168, 1, 221))],
            port: 3001,
            properties: HashMap::new(),
            discovered_at: Instant::now(),
            last_seen: Instant::now(),
            paired: false,
        };
        mgr.register_node(node).await;
        let nodes = mgr.discovered_nodes().await;
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].hostname, "zeus.local");
    }

    #[tokio::test]
    async fn test_get_node_by_name() {
        let mgr = DiscoveryManager::with_defaults();
        let node = DiscoveredNode {
            instance_name: "gw-abc".into(),
            hostname: "h".into(),
            addresses: vec![],
            port: 3001,
            properties: HashMap::new(),
            discovered_at: Instant::now(),
            last_seen: Instant::now(),
            paired: false,
        };
        mgr.register_node(node).await;
        assert!(mgr.get_node("gw-abc").await.is_some());
        assert!(mgr.get_node("nonexistent").await.is_none());
    }

    #[tokio::test]
    async fn test_pairing_flow() {
        let mgr = DiscoveryManager::with_defaults();

        // Create pairing session
        let code = mgr.create_pairing_session("iPhone 15", "ios").await;
        assert_eq!(code.len(), 6);

        // Complete pairing
        let device = mgr
            .complete_pairing(&code)
            .await
            .expect("async operation should succeed");
        assert_eq!(device.device_name, "iPhone 15");
        assert_eq!(device.device_type, "ios");
        assert_eq!(device.token.len(), 64);

        // Validate token
        assert!(mgr.validate_token(&device.token).await);
        assert!(!mgr.validate_token("bad-token").await);
    }

    #[tokio::test]
    async fn test_pairing_invalid_code() {
        let mgr = DiscoveryManager::with_defaults();
        let result = mgr.complete_pairing("000000").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_pairing_already_used() {
        let mgr = DiscoveryManager::with_defaults();
        let code = mgr.create_pairing_session("Device", "android").await;
        mgr.complete_pairing(&code)
            .await
            .expect("async operation should succeed");
        let result = mgr.complete_pairing(&code).await;
        assert!(matches!(result, Err(DiscoveryError::PairingAlreadyUsed)));
    }

    #[tokio::test]
    async fn test_revoke_device() {
        let mgr = DiscoveryManager::with_defaults();
        let code = mgr.create_pairing_session("Test", "cli").await;
        let device = mgr
            .complete_pairing(&code)
            .await
            .expect("async operation should succeed");
        assert!(mgr.validate_token(&device.token).await);
        assert!(mgr.revoke_device(&device.token).await);
        assert!(!mgr.validate_token(&device.token).await);
    }

    #[tokio::test]
    async fn test_touch_device() {
        let mgr = DiscoveryManager::with_defaults();
        let code = mgr.create_pairing_session("Test", "desktop").await;
        let device = mgr
            .complete_pairing(&code)
            .await
            .expect("async operation should succeed");
        mgr.touch_device(&device.token).await;

        let devices = mgr.paired_devices().await;
        assert_eq!(devices.len(), 1);
        assert!(devices[0].last_connected.is_some());
    }

    #[tokio::test]
    async fn test_list_paired_devices() {
        let mgr = DiscoveryManager::with_defaults();

        let code1 = mgr.create_pairing_session("iPhone", "ios").await;
        let code2 = mgr.create_pairing_session("Pixel", "android").await;
        mgr.complete_pairing(&code1)
            .await
            .expect("async operation should succeed");
        mgr.complete_pairing(&code2)
            .await
            .expect("async operation should succeed");

        let devices = mgr.paired_devices().await;
        assert_eq!(devices.len(), 2);
    }

    #[test]
    fn test_service_type_constant() {
        assert_eq!(SERVICE_TYPE, "_zeus._tcp");
    }

    #[test]
    fn test_node_version() {
        let mut props = HashMap::new();
        props.insert("version".into(), "0.1.0".into());
        let node = DiscoveredNode {
            instance_name: "n".into(),
            hostname: "h".into(),
            addresses: vec![],
            port: 3001,
            properties: props,
            discovered_at: Instant::now(),
            last_seen: Instant::now(),
            paired: false,
        };
        assert_eq!(node.version(), Some("0.1.0"));
    }
}
