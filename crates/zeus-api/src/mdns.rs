//! mDNS Gateway Discovery
//!
//! Broadcasts the Zeus gateway service on the local network using mDNS
//! (multicast DNS, RFC 6762) and discovers other Zeus instances.
//!
//! Service type: `_zeus-gateway._tcp.local`
//!
//! Uses raw UDP multicast sockets — no external crate needed.
//! mDNS multicast group: 224.0.0.251:5353

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::Arc;
use tokio::net::UdpSocket;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// mDNS multicast address (RFC 6762).
const MDNS_ADDR: Ipv4Addr = Ipv4Addr::new(224, 0, 0, 251);
const MDNS_PORT: u16 = 5353;

/// Zeus mDNS service type.
const SERVICE_TYPE: &str = "_zeus-gateway._tcp.local";

/// A discovered Zeus peer on the local network.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZeusPeer {
    /// Peer instance name (e.g., "zeus-macmini")
    pub instance_name: String,
    /// IP address of the peer
    pub address: String,
    /// API port of the peer
    pub port: u16,
    /// When this peer was last seen (RFC 3339)
    pub last_seen: String,
    /// Additional metadata from the TXT record
    #[serde(default)]
    pub metadata: HashMap<String, String>,
}

/// mDNS discovery manager for Zeus gateway instances.
///
/// Provides two capabilities:
/// 1. **Broadcasting**: Announces this Zeus instance on the local network
/// 2. **Discovery**: Finds other Zeus instances via mDNS queries
pub struct MdnsDiscovery {
    /// This instance's name
    instance_name: String,
    /// This instance's API port
    port: u16,
    /// Discovered peers (thread-safe)
    peers: Arc<RwLock<HashMap<String, ZeusPeer>>>,
    /// Whether broadcast is active
    broadcasting: Arc<RwLock<bool>>,
}

impl MdnsDiscovery {
    /// Create a new mDNS discovery manager.
    ///
    /// # Arguments
    /// * `instance_name` — Human-readable instance name (e.g., hostname)
    /// * `port` — The API port this instance is listening on
    pub fn new(instance_name: String, port: u16) -> Self {
        Self {
            instance_name,
            port,
            peers: Arc::new(RwLock::new(HashMap::new())),
            broadcasting: Arc::new(RwLock::new(false)),
        }
    }

    /// Start broadcasting this Zeus instance on the local network.
    ///
    /// Sends an mDNS announcement every 30 seconds on the multicast group.
    /// Runs in the background until the discovery manager is dropped.
    pub async fn start_broadcast(&self) -> Result<(), String> {
        let already = *self.broadcasting.read().await;
        if already {
            return Err("Broadcast already active".to_string());
        }

        let socket = bind_mdns_socket().await?;
        let packet = build_announcement(&self.instance_name, self.port);
        let broadcasting = self.broadcasting.clone();
        let peers = self.peers.clone();
        let instance_name = self.instance_name.clone();
        let port = self.port;

        *broadcasting.write().await = true;

        tokio::spawn(async move {
            let dest = SocketAddr::V4(SocketAddrV4::new(MDNS_ADDR, MDNS_PORT));
            let mut buf = [0u8; 1500];

            info!(
                "mDNS broadcast started: {} ({}:{}) on {}",
                instance_name, SERVICE_TYPE, port, dest
            );

            loop {
                // Send announcement
                if let Err(e) = socket.send_to(&packet, dest).await {
                    warn!("mDNS broadcast send failed: {}", e);
                }

                // Listen for responses/announcements from others for 30 seconds
                let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
                loop {
                    let timeout = deadline.saturating_duration_since(tokio::time::Instant::now());
                    if timeout.is_zero() {
                        break;
                    }

                    match tokio::time::timeout(timeout, socket.recv_from(&mut buf)).await {
                        Ok(Ok((len, addr))) => {
                            if let Some(peer) = parse_announcement(&buf[..len], addr) {
                                // Don't add ourselves
                                if peer.instance_name != instance_name || peer.port != port {
                                    debug!(
                                        "mDNS discovered peer: {} at {}:{}",
                                        peer.instance_name, peer.address, peer.port
                                    );
                                    peers
                                        .write()
                                        .await
                                        .insert(format!("{}:{}", peer.address, peer.port), peer);
                                }
                            }
                        }
                        Ok(Err(e)) => {
                            debug!("mDNS recv error: {}", e);
                        }
                        Err(_) => break, // Timeout
                    }
                }

                // Check if we should stop
                if !*broadcasting.read().await {
                    info!("mDNS broadcast stopped");
                    break;
                }
            }
        });

        Ok(())
    }

    /// Stop the broadcast loop.
    pub async fn stop_broadcast(&self) {
        *self.broadcasting.write().await = false;
    }

    /// Whether broadcast is currently active.
    pub async fn is_broadcasting(&self) -> bool {
        *self.broadcasting.read().await
    }

    /// Discover Zeus peers on the local network (one-shot query).
    ///
    /// Sends an mDNS query and waits up to `timeout_ms` for responses.
    /// Returns all discovered peers.
    pub async fn discover_peers(&self, timeout_ms: u64) -> Vec<ZeusPeer> {
        let socket = match bind_mdns_socket().await {
            Ok(s) => s,
            Err(e) => {
                warn!("mDNS discover_peers: failed to bind socket: {}", e);
                return Vec::new();
            }
        };

        let query = build_query();
        let dest = SocketAddr::V4(SocketAddrV4::new(MDNS_ADDR, MDNS_PORT));

        if let Err(e) = socket.send_to(&query, dest).await {
            warn!("mDNS query send failed: {}", e);
            return Vec::new();
        }

        let mut discovered: HashMap<String, ZeusPeer> = HashMap::new();
        let mut buf = [0u8; 1500];
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);

        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                break;
            }

            match tokio::time::timeout(remaining, socket.recv_from(&mut buf)).await {
                Ok(Ok((len, addr))) => {
                    if let Some(peer) = parse_announcement(&buf[..len], addr)
                        && (peer.instance_name != self.instance_name || peer.port != self.port)
                    {
                        discovered.insert(format!("{}:{}", peer.address, peer.port), peer);
                    }
                }
                Ok(Err(_)) => continue,
                Err(_) => break, // Timeout
            }
        }

        // Merge into persistent peers map
        {
            let mut peers = self.peers.write().await;
            for (k, v) in &discovered {
                peers.insert(k.clone(), v.clone());
            }
        }

        discovered.into_values().collect()
    }

    /// Get all known peers (from broadcast listener + previous discoveries).
    pub async fn known_peers(&self) -> Vec<ZeusPeer> {
        self.peers.read().await.values().cloned().collect()
    }

    /// Clear stale peers older than `max_age_secs`.
    pub async fn prune_stale(&self, max_age_secs: i64) {
        let now = chrono::Utc::now();
        let mut peers = self.peers.write().await;
        peers.retain(|_, peer| {
            if let Ok(last) = chrono::DateTime::parse_from_rfc3339(&peer.last_seen) {
                (now - last.with_timezone(&chrono::Utc)).num_seconds() < max_age_secs
            } else {
                false
            }
        });
    }
}

// ============================================================================
// Packet construction & parsing
// ============================================================================

/// Bind a UDP socket to the mDNS multicast group.
///
/// Uses `std::net::UdpSocket` for setup (SO_REUSEADDR, multicast join),
/// then converts to `tokio::net::UdpSocket` for async I/O.
async fn bind_mdns_socket() -> Result<UdpSocket, String> {
    // Use a non-standard port (0) for sending queries — avoids needing
    // SO_REUSEPORT on systems where mDNS responders already hold :5353.
    // For the broadcast loop we try 5353 first, fall back to ephemeral.
    let std_sock = std::net::UdpSocket::bind("0.0.0.0:0")
        .or_else(|_| std::net::UdpSocket::bind("0.0.0.0:5353"))
        .map_err(|e| format!("Failed to bind UDP socket: {e}"))?;

    std_sock
        .join_multicast_v4(&MDNS_ADDR, &Ipv4Addr::UNSPECIFIED)
        .map_err(|e| format!("Failed to join mDNS multicast: {e}"))?;

    std_sock
        .set_nonblocking(true)
        .map_err(|e| format!("set_nonblocking: {e}"))?;

    UdpSocket::from_std(std_sock).map_err(|e| format!("Failed to convert to tokio socket: {e}"))
}

/// Build a simplified mDNS announcement packet.
///
/// This is a minimal DNS response packet containing:
/// - Header (ID=0, QR=1 response, 1 answer)
/// - Answer: SERVICE_TYPE TXT record with instance name and port
fn build_announcement(instance_name: &str, port: u16) -> Vec<u8> {
    // We use a TXT record approach: the payload contains key=value pairs
    // that identify this Zeus instance. This is simpler than full SRV+A records
    // and still discoverable by our own parser.
    let txt_data = format!(
        "zeus=1\nname={}\nport={}\ntime={}",
        instance_name,
        port,
        chrono::Utc::now().to_rfc3339()
    );

    let mut packet = Vec::with_capacity(256);

    // DNS Header (12 bytes)
    packet.extend_from_slice(&[0x00, 0x00]); // Transaction ID
    packet.extend_from_slice(&[0x84, 0x00]); // Flags: QR=1, AA=1 (authoritative response)
    packet.extend_from_slice(&[0x00, 0x00]); // Questions: 0
    packet.extend_from_slice(&[0x00, 0x01]); // Answers: 1
    packet.extend_from_slice(&[0x00, 0x00]); // Authority: 0
    packet.extend_from_slice(&[0x00, 0x00]); // Additional: 0

    // Answer: name (SERVICE_TYPE encoded)
    encode_dns_name(&mut packet, SERVICE_TYPE);

    // Type TXT (0x0010), Class IN (0x0001)
    packet.extend_from_slice(&[0x00, 0x10]); // TXT
    packet.extend_from_slice(&[0x00, 0x01]); // IN

    // TTL: 120 seconds
    packet.extend_from_slice(&120u32.to_be_bytes());

    // RDATA: TXT record (length-prefixed strings)
    let txt_bytes = txt_data.as_bytes();
    let rdlen = 1 + txt_bytes.len(); // 1 byte length prefix + data
    packet.extend_from_slice(&(rdlen as u16).to_be_bytes());
    packet.push(txt_bytes.len() as u8); // TXT string length
    packet.extend_from_slice(txt_bytes);

    packet
}

/// Build a minimal mDNS query for Zeus services.
fn build_query() -> Vec<u8> {
    let mut packet = Vec::with_capacity(128);

    // DNS Header
    packet.extend_from_slice(&[0x00, 0x00]); // Transaction ID
    packet.extend_from_slice(&[0x00, 0x00]); // Flags: standard query
    packet.extend_from_slice(&[0x00, 0x01]); // Questions: 1
    packet.extend_from_slice(&[0x00, 0x00]); // Answers: 0
    packet.extend_from_slice(&[0x00, 0x00]); // Authority: 0
    packet.extend_from_slice(&[0x00, 0x00]); // Additional: 0

    // Question: SERVICE_TYPE
    encode_dns_name(&mut packet, SERVICE_TYPE);
    packet.extend_from_slice(&[0x00, 0x10]); // QTYPE: TXT
    packet.extend_from_slice(&[0x00, 0x01]); // QCLASS: IN

    packet
}

/// Encode a dotted domain name into DNS wire format.
fn encode_dns_name(buf: &mut Vec<u8>, name: &str) {
    for label in name.split('.') {
        let bytes = label.as_bytes();
        buf.push(bytes.len() as u8);
        buf.extend_from_slice(bytes);
    }
    buf.push(0); // Terminator
}

/// Try to parse an mDNS announcement packet from another Zeus instance.
fn parse_announcement(data: &[u8], addr: SocketAddr) -> Option<ZeusPeer> {
    // Minimum valid DNS packet is 12 bytes header
    if data.len() < 12 {
        return None;
    }

    // Check flags: QR=1 (response)
    let flags = u16::from_be_bytes([data[2], data[3]]);
    if flags & 0x8000 == 0 {
        return None; // Not a response
    }

    // Answer count is at bytes [6,7] in the DNS header
    let answer_count = u16::from_be_bytes([data[6], data[7]]);
    if answer_count == 0 {
        return None;
    }

    // Search for our TXT payload in the packet body
    // Look for "zeus=1" marker in the raw bytes
    let body = std::str::from_utf8(data).ok();
    let raw_str = if let Some(s) = body {
        s.to_string()
    } else {
        // Try to find the TXT data portion (after DNS headers + name)
        // The TXT data starts after the length prefix byte
        let mut txt_data = String::new();
        for window in data.windows(6) {
            if window == b"zeus=1" {
                // Found the marker — extract the surrounding TXT record
                let start = data.windows(6).position(|w| w == b"zeus=1").unwrap_or(0);
                // The TXT string starts 1 byte before "zeus=1" (the length prefix)
                // but we want the data itself
                if start > 0 {
                    let txt_len = data[start - 1] as usize;
                    if start + txt_len <= data.len() {
                        txt_data =
                            String::from_utf8_lossy(&data[start..start + txt_len]).to_string();
                    }
                }
                break;
            }
        }
        if txt_data.is_empty() {
            return None;
        }
        txt_data
    };

    // Parse key=value pairs from the TXT data
    let mut metadata: HashMap<String, String> = HashMap::new();
    for line in raw_str.split('\n') {
        if let Some((k, v)) = line.split_once('=') {
            let key = k.trim().to_string();
            let val = v.trim().to_string();
            // Only keep printable ASCII key-value pairs
            if key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') && !key.is_empty() {
                metadata.insert(key, val);
            }
        }
    }

    // Must have the zeus=1 marker
    if metadata.get("zeus").map(|v| v.as_str()) != Some("1") {
        return None;
    }

    let instance_name = metadata
        .remove("name")
        .unwrap_or_else(|| addr.ip().to_string());
    let port: u16 = metadata
        .remove("port")
        .and_then(|p| p.parse().ok())
        .unwrap_or(8080);
    let last_seen = metadata
        .remove("time")
        .unwrap_or_else(|| chrono::Utc::now().to_rfc3339());

    Some(ZeusPeer {
        instance_name,
        address: addr.ip().to_string(),
        port,
        last_seen,
        metadata,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mdns_discovery_new() {
        let d = MdnsDiscovery::new("test-zeus".to_string(), 8080);
        assert_eq!(d.instance_name, "test-zeus");
        assert_eq!(d.port, 8080);
    }

    #[test]
    fn test_build_announcement_not_empty() {
        let pkt = build_announcement("my-zeus", 8080);
        assert!(pkt.len() > 12); // At least header
        // Check it starts with DNS header
        assert_eq!(pkt[2], 0x84); // QR=1, AA=1
        assert_eq!(pkt[3], 0x00);
    }

    #[test]
    fn test_build_query_not_empty() {
        let pkt = build_query();
        assert!(pkt.len() > 12);
        // Standard query flags
        assert_eq!(pkt[2], 0x00);
        assert_eq!(pkt[3], 0x00);
        // 1 question
        assert_eq!(pkt[4], 0x00);
        assert_eq!(pkt[5], 0x01);
    }

    #[test]
    fn test_encode_dns_name() {
        let mut buf = Vec::new();
        encode_dns_name(&mut buf, "_zeus._tcp.local");
        // Should be: 5 "_zeus" 4 "_tcp" 5 "local" 0
        assert_eq!(buf[0], 5); // "_zeus" length
        assert_eq!(&buf[1..6], b"_zeus");
        assert_eq!(buf[6], 4); // "_tcp" length
        assert_eq!(&buf[7..11], b"_tcp");
        assert_eq!(buf[11], 5); // "local" length
        assert_eq!(&buf[12..17], b"local");
        assert_eq!(buf[17], 0); // terminator
    }

    #[test]
    fn test_parse_announcement_roundtrip() {
        let pkt = build_announcement("test-instance", 9090);
        let addr: SocketAddr = "192.168.1.42:5353".parse().unwrap();
        let peer = parse_announcement(&pkt, addr);
        assert!(peer.is_some(), "Should parse our own announcement");
        let peer = peer.unwrap();
        assert_eq!(peer.instance_name, "test-instance");
        assert_eq!(peer.port, 9090);
        assert_eq!(peer.address, "192.168.1.42");
    }

    #[test]
    fn test_parse_announcement_too_short() {
        let peer = parse_announcement(&[0u8; 6], "127.0.0.1:5353".parse().unwrap());
        assert!(peer.is_none());
    }

    #[test]
    fn test_parse_announcement_query_not_response() {
        let query = build_query();
        let peer = parse_announcement(&query, "127.0.0.1:5353".parse().unwrap());
        assert!(
            peer.is_none(),
            "Should not parse a query as an announcement"
        );
    }

    #[test]
    fn test_parse_announcement_no_zeus_marker() {
        // Craft a fake response without zeus=1
        let mut pkt = Vec::new();
        pkt.extend_from_slice(&[0x00, 0x00]); // ID
        pkt.extend_from_slice(&[0x84, 0x00]); // Flags: response
        pkt.extend_from_slice(&[0x00, 0x00]); // Questions
        pkt.extend_from_slice(&[0x00, 0x01]); // Answers: 1
        pkt.extend_from_slice(&[0x00, 0x00]); // Authority
        pkt.extend_from_slice(&[0x00, 0x00]); // Additional
        pkt.extend_from_slice(b"\x05other\x04data\x00");
        let peer = parse_announcement(&pkt, "127.0.0.1:5353".parse().unwrap());
        assert!(peer.is_none());
    }

    #[test]
    fn test_zeus_peer_serialization() {
        let peer = ZeusPeer {
            instance_name: "zeus-mac".to_string(),
            address: "192.168.1.100".to_string(),
            port: 8080,
            last_seen: "2026-02-18T12:00:00Z".to_string(),
            metadata: HashMap::new(),
        };
        let json = serde_json::to_string(&peer).unwrap();
        let parsed: ZeusPeer = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.instance_name, "zeus-mac");
        assert_eq!(parsed.port, 8080);
    }

    #[tokio::test]
    async fn test_known_peers_empty() {
        let d = MdnsDiscovery::new("test".to_string(), 8080);
        let peers = d.known_peers().await;
        assert!(peers.is_empty());
    }

    #[tokio::test]
    async fn test_is_broadcasting_default() {
        let d = MdnsDiscovery::new("test".to_string(), 8080);
        assert!(!d.is_broadcasting().await);
    }

    #[tokio::test]
    async fn test_prune_stale_empty() {
        let d = MdnsDiscovery::new("test".to_string(), 8080);
        d.prune_stale(60).await;
        assert!(d.known_peers().await.is_empty());
    }

    #[tokio::test]
    async fn test_prune_stale_removes_old() {
        let d = MdnsDiscovery::new("test".to_string(), 8080);
        // Manually insert a stale peer
        {
            let old_time = (chrono::Utc::now() - chrono::Duration::seconds(120)).to_rfc3339();
            let mut peers = d.peers.write().await;
            peers.insert(
                "10.0.0.1:8080".to_string(),
                ZeusPeer {
                    instance_name: "old-zeus".to_string(),
                    address: "10.0.0.1".to_string(),
                    port: 8080,
                    last_seen: old_time,
                    metadata: HashMap::new(),
                },
            );
        }
        assert_eq!(d.known_peers().await.len(), 1);
        d.prune_stale(60).await; // Max age 60s, peer is 120s old
        assert!(d.known_peers().await.is_empty());
    }

    #[test]
    fn test_service_type() {
        assert_eq!(SERVICE_TYPE, "_zeus-gateway._tcp.local");
    }

    #[test]
    fn test_mdns_constants() {
        assert_eq!(MDNS_ADDR, Ipv4Addr::new(224, 0, 0, 251));
        assert_eq!(MDNS_PORT, 5353);
    }
}
