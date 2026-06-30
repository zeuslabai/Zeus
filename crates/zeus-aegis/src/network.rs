//! Network filtering and allowlisting
//!
//! Provides network access control for Zeus operations.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::net::{IpAddr, SocketAddr, ToSocketAddrs};
use zeus_core::{Error, Result};

/// URI schemes that are always blocked regardless of allowlist (SSRF protection)
const BLOCKED_SCHEMES: &[&str] = &["file", "gopher", "dict", "ftp", "ldap", "ldaps", "sftp", "tftp"];

/// Network filter for controlling outbound connections
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkFilter {
    /// Enabled state
    enabled: bool,
    /// Allowed hostnames/domains
    allowed_hosts: HashSet<String>,
    /// Allowed IP ranges (CIDR-like patterns)
    allowed_ips: HashSet<String>,
    /// Allowed ports
    allowed_ports: HashSet<u16>,
    /// Blocked hostnames/domains
    blocked_hosts: HashSet<String>,
}

impl NetworkFilter {
    /// Create a new network filter
    pub fn new() -> Self {
        Self {
            enabled: false,
            allowed_hosts: HashSet::new(),
            allowed_ips: HashSet::new(),
            allowed_ports: HashSet::new(),
            blocked_hosts: HashSet::new(),
        }
    }

    /// Create a filter that allows all traffic
    pub fn allow_all() -> Self {
        let mut filter = Self::new();
        filter.allowed_hosts.insert("*".to_string());
        filter.allowed_ips.insert("*".to_string());
        filter
    }

    /// Create a filter with default allowed hosts (LLM providers, etc.)
    pub fn default_allowlist() -> Self {
        let mut filter = Self::new();
        filter.enabled = true;

        // LLM API providers
        filter.allow_host("api.anthropic.com");
        filter.allow_host("api.openai.com");
        filter.allow_host("generativelanguage.googleapis.com");

        // Common messaging APIs
        filter.allow_host("api.telegram.org");
        filter.allow_host("discord.com");
        filter.allow_host("*.discord.com");
        filter.allow_host("slack.com");
        filter.allow_host("*.slack.com");

        // Local network
        filter.allow_ip("127.0.0.1");
        filter.allow_ip("::1");

        // Common ports
        filter.allow_port(80);
        filter.allow_port(443);

        filter
    }

    /// Enable the filter
    pub fn enable(&mut self) {
        self.enabled = true;
    }

    /// Disable the filter
    pub fn disable(&mut self) {
        self.enabled = false;
    }

    /// Check if the filter is enabled
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Add an allowed hostname
    pub fn allow_host(&mut self, host: &str) {
        self.allowed_hosts.insert(host.to_string());
    }

    /// Add an allowed IP
    pub fn allow_ip(&mut self, ip: &str) {
        self.allowed_ips.insert(ip.to_string());
    }

    /// Add an allowed port
    pub fn allow_port(&mut self, port: u16) {
        self.allowed_ports.insert(port);
    }

    /// Block a hostname
    pub fn block_host(&mut self, host: &str) {
        self.blocked_hosts.insert(host.to_string());
    }

    /// Check if a hostname is allowed
    pub fn check_host(&self, host: &str) -> Result<()> {
        if !self.enabled {
            return Ok(());
        }

        // Check blocked list first
        if self.is_blocked_host(host) {
            return Err(Error::Security(format!(
                "Host '{}' is blocked by network filter",
                host
            )));
        }

        // Check allowed list
        if self.is_allowed_host(host) {
            return Ok(());
        }

        Err(Error::Security(format!(
            "Host '{}' is not in network allowlist",
            host
        )))
    }

    /// Check if an IP is allowed
    pub fn check_ip(&self, ip: &IpAddr) -> Result<()> {
        if !self.enabled {
            return Ok(());
        }

        let ip_str = ip.to_string();

        if self.is_allowed_ip(&ip_str) {
            return Ok(());
        }

        Err(Error::Security(format!(
            "IP '{}' is not in network allowlist",
            ip
        )))
    }

    /// Check if a socket address is allowed
    pub fn check_socket(&self, addr: &SocketAddr) -> Result<()> {
        self.check_ip(&addr.ip())?;

        if !self.enabled {
            return Ok(());
        }

        // Check port if port allowlist is not empty
        if !self.allowed_ports.is_empty() && !self.allowed_ports.contains(&addr.port()) {
            return Err(Error::Security(format!(
                "Port {} is not in network allowlist",
                addr.port()
            )));
        }

        Ok(())
    }

    /// Check if a URL is allowed.
    ///
    /// Scheme blocking and DNS pre-resolution are only active when the filter is
    /// enabled (`self.enabled`). Callers using a raw `NetworkFilter::new()` must
    /// call `enable()` for SSRF protections to take effect.
    pub fn check_url(&self, url: &str) -> Result<()> {
        if !self.enabled {
            return Ok(());
        }

        // Block dangerous URI schemes regardless of allowlist
        let scheme = url.split("://").next().unwrap_or("").to_lowercase();
        if BLOCKED_SCHEMES.contains(&scheme.as_str()) {
            return Err(Error::Security(format!(
                "URI scheme '{}://' is blocked (SSRF protection)",
                scheme
            )));
        }

        // Parse URL to extract host, handling userinfo (user:pass@host)
        let authority = if url.starts_with("http://") || url.starts_with("https://") {
            url.split("://").nth(1).and_then(|s| s.split('/').next())
        } else {
            url.split('/').next()
        };

        // Strip userinfo if present (e.g., "user:pass@host" → "host")
        let host_with_port = authority.map(|a| {
            if let Some(at_pos) = a.rfind('@') {
                &a[at_pos + 1..]
            } else {
                a
            }
        });

        // Strip port if present
        let host = host_with_port.map(|hp| {
            // Handle IPv6 addresses in brackets [::1]:8080
            if hp.starts_with('[') {
                hp.split(']').next().unwrap_or(hp).trim_start_matches('[')
            } else {
                hp.split(':').next().unwrap_or(hp)
            }
        });

        match host {
            Some(h) if !h.is_empty() => {
                self.check_host(h)?;
                self.check_ssrf_dns(h)
            }
            _ => Err(Error::Security(format!(
                "Could not parse host from URL: {}",
                url
            ))),
        }
    }

    /// Returns true if the IP falls within a private, loopback, or link-local range.
    fn is_private_ip(ip: &IpAddr) -> bool {
        match ip {
            IpAddr::V4(v4) => {
                let o = v4.octets();
                o[0] == 127                                        // 127.0.0.0/8 loopback
                || o[0] == 10                                      // 10.0.0.0/8
                || (o[0] == 172 && (16..=31).contains(&o[1]))     // 172.16.0.0/12
                || (o[0] == 192 && o[1] == 168)                   // 192.168.0.0/16
                || (o[0] == 169 && o[1] == 254)                   // 169.254.0.0/16 link-local
                || o[0] == 0                                       // 0.0.0.0/8
            }
            IpAddr::V6(v6) => {
                let s = v6.segments();
                v6.is_loopback()                                   // ::1
                || (s[0] & 0xfe00) == 0xfc00                      // fc00::/7 unique-local
                || (s[0] & 0xffc0) == 0xfe80                      // fe80::/10 link-local
            }
        }
    }

    /// Returns true only for exact (non-wildcard) allowlist entries.
    /// Used to gate SSRF DNS bypass — wildcard entries must not skip the private-IP check.
    fn is_explicitly_allowed_host(&self, host: &str) -> bool {
        self.allowed_hosts.contains(&host.to_lowercase())
    }

    /// Resolve `host` and reject if any address falls in a private IP range.
    /// Skipped only when the host is an exact entry in the allowlist (intentional local access).
    fn check_ssrf_dns(&self, host: &str) -> Result<()> {
        if self.is_explicitly_allowed_host(host) {
            return Ok(());
        }

        let addrs = (host, 80u16)
            .to_socket_addrs()
            .map_err(|e| Error::Security(format!("DNS resolution failed for '{}': {}", host, e)))?;

        for addr in addrs {
            let ip = addr.ip();
            if Self::is_private_ip(&ip) {
                return Err(Error::Security(format!(
                    "SSRF blocked: '{}' resolves to private IP {}",
                    host, ip
                )));
            }
        }

        Ok(())
    }

    /// Check if a host is blocked
    fn is_blocked_host(&self, host: &str) -> bool {
        let host_lower = host.to_lowercase();

        for blocked in &self.blocked_hosts {
            if Self::host_matches(blocked, &host_lower) {
                return true;
            }
        }

        false
    }

    /// Check if a host is allowed
    fn is_allowed_host(&self, host: &str) -> bool {
        let host_lower = host.to_lowercase();

        for allowed in &self.allowed_hosts {
            if Self::host_matches(allowed, &host_lower) {
                return true;
            }
        }

        false
    }

    /// Check if an IP is allowed
    fn is_allowed_ip(&self, ip: &str) -> bool {
        if self.allowed_ips.contains("*") {
            return true;
        }

        self.allowed_ips.contains(ip)
    }

    /// Check if a pattern matches a host
    fn host_matches(pattern: &str, host: &str) -> bool {
        let pattern_lower = pattern.to_lowercase();

        // Wildcard all
        if pattern_lower == "*" {
            return true;
        }

        // Wildcard subdomain (*.example.com)
        if let Some(suffix) = pattern_lower.strip_prefix("*.") {
            return host == suffix || host.ends_with(&format!(".{}", suffix));
        }

        // Exact match
        pattern_lower == host
    }
}

impl Default for NetworkFilter {
    fn default() -> Self {
        Self::allow_all()
    }
}

/// Network statistics
#[derive(Debug, Default, Clone)]
pub struct NetworkStats {
    /// Total requests checked
    pub total_checks: u64,
    /// Requests allowed
    pub allowed: u64,
    /// Requests blocked
    pub blocked: u64,
    /// Unique hosts seen
    pub unique_hosts: HashSet<String>,
}

impl NetworkStats {
    /// Record an allowed request
    pub fn record_allowed(&mut self, host: &str) {
        self.total_checks += 1;
        self.allowed += 1;
        self.unique_hosts.insert(host.to_string());
    }

    /// Record a blocked request
    pub fn record_blocked(&mut self, host: &str) {
        self.total_checks += 1;
        self.blocked += 1;
        self.unique_hosts.insert(host.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filter_disabled() {
        let filter = NetworkFilter::new();
        assert!(filter.check_host("anything.com").is_ok());
    }

    #[test]
    fn test_filter_explicit_allow() {
        let mut filter = NetworkFilter::new();
        filter.enable();
        filter.allow_host("api.example.com");

        assert!(filter.check_host("api.example.com").is_ok());
        assert!(filter.check_host("evil.com").is_err());
    }

    #[test]
    fn test_filter_wildcard_subdomain() {
        let mut filter = NetworkFilter::new();
        filter.enable();
        filter.allow_host("*.example.com");

        assert!(filter.check_host("api.example.com").is_ok());
        assert!(filter.check_host("example.com").is_ok());
        assert!(filter.check_host("sub.api.example.com").is_ok());
        assert!(filter.check_host("evil.com").is_err());
    }

    #[test]
    fn test_filter_blocked_takes_precedence() {
        let mut filter = NetworkFilter::new();
        filter.enable();
        filter.allow_host("*");
        filter.block_host("evil.com");

        assert!(filter.check_host("good.com").is_ok());
        assert!(filter.check_host("evil.com").is_err());
    }

    #[test]
    fn test_url_parsing() {
        let mut filter = NetworkFilter::new();
        filter.enable();
        filter.allow_host("api.example.com");

        assert!(filter.check_url("https://api.example.com/v1/test").is_ok());
        assert!(filter.check_url("http://api.example.com:8080/test").is_ok());
        assert!(filter.check_url("https://evil.com/test").is_err());
    }

    #[test]
    fn test_url_userinfo_bypass() {
        let mut filter = NetworkFilter::new();
        filter.enable();
        filter.allow_host("api.example.com");

        // URLs with userinfo should extract the actual host, not the username
        assert!(filter.check_url("https://user:pass@evil.com/path").is_err());
        assert!(
            filter
                .check_url("https://user:pass@api.example.com/path")
                .is_ok()
        );
        assert!(
            filter
                .check_url("https://evil@api.example.com/path")
                .is_ok()
        );
    }

    #[test]
    fn test_port_filtering() {
        let mut filter = NetworkFilter::new();
        filter.enable();
        filter.allow_ip("127.0.0.1");
        filter.allow_port(443);
        filter.allow_port(80);

        let addr_443: SocketAddr = "127.0.0.1:443".parse().expect("should parse successfully");
        let addr_8080: SocketAddr = "127.0.0.1:8080".parse().expect("should parse successfully");

        assert!(filter.check_socket(&addr_443).is_ok());
        assert!(filter.check_socket(&addr_8080).is_err());
    }

    #[test]
    fn test_default_allowlist() {
        let filter = NetworkFilter::default_allowlist();

        assert!(filter.check_host("api.anthropic.com").is_ok());
        assert!(filter.check_host("api.openai.com").is_ok());
        assert!(filter.check_host("random-site.com").is_err());
    }

    // ── SSRF: scheme blocking ─────────────────────────────────────────────

    #[test]
    fn test_scheme_block_file() {
        let filter = NetworkFilter::default_allowlist();
        let err = filter.check_url("file:///etc/passwd").unwrap_err();
        assert!(err.to_string().contains("file"), "expected 'file' in: {err}");
    }

    #[test]
    fn test_scheme_block_gopher() {
        let filter = NetworkFilter::default_allowlist();
        assert!(filter.check_url("gopher://evil.com/1exploit").is_err());
    }

    #[test]
    fn test_scheme_block_dict() {
        let filter = NetworkFilter::default_allowlist();
        assert!(filter.check_url("dict://evil.com:11111/d:password").is_err());
    }

    #[test]
    fn test_scheme_http_still_allowed() {
        let mut filter = NetworkFilter::new();
        filter.enable();
        filter.allow_host("example.com");
        assert!(filter.check_url("http://example.com/path").is_ok());
        assert!(filter.check_url("https://example.com/path").is_ok());
    }

    // ── SSRF: private IP detection ────────────────────────────────────────

    #[test]
    fn test_private_ip_loopback_v4() {
        let ip: IpAddr = "127.0.0.1".parse().unwrap();
        assert!(NetworkFilter::is_private_ip(&ip));
    }

    #[test]
    fn test_private_ip_rfc1918_10() {
        let ip: IpAddr = "10.0.0.1".parse().unwrap();
        assert!(NetworkFilter::is_private_ip(&ip));
    }

    #[test]
    fn test_private_ip_rfc1918_172() {
        for second in 16u8..=31 {
            let ip: IpAddr = format!("172.{}.0.1", second).parse().unwrap();
            assert!(NetworkFilter::is_private_ip(&ip), "172.{second}.0.1 should be private");
        }
        let pub_ip: IpAddr = "172.32.0.1".parse().unwrap();
        assert!(!NetworkFilter::is_private_ip(&pub_ip));
    }

    #[test]
    fn test_private_ip_rfc1918_192() {
        let ip: IpAddr = "192.168.1.100".parse().unwrap();
        assert!(NetworkFilter::is_private_ip(&ip));
        let pub_ip: IpAddr = "192.169.1.1".parse().unwrap();
        assert!(!NetworkFilter::is_private_ip(&pub_ip));
    }

    #[test]
    fn test_private_ip_loopback_v6() {
        let ip: IpAddr = "::1".parse().unwrap();
        assert!(NetworkFilter::is_private_ip(&ip));
    }

    #[test]
    fn test_public_ip_not_private() {
        for addr in &["1.1.1.1", "8.8.8.8", "93.184.216.34"] {
            let ip: IpAddr = addr.parse().unwrap();
            assert!(!NetworkFilter::is_private_ip(&ip), "{addr} should be public");
        }
    }

    #[test]
    fn test_ssrf_dns_localhost_blocked() {
        let mut filter = NetworkFilter::new();
        filter.enable();
        filter.allow_host("*");
        // localhost resolves to 127.0.0.1 / ::1 — must be rejected
        assert!(filter.check_ssrf_dns("localhost").is_err());
    }

    #[test]
    fn test_ssrf_allowlist_bypasses_dns_check() {
        let mut filter = NetworkFilter::new();
        filter.enable();
        filter.allow_host("localhost");
        // explicitly allowlisted — DNS check is skipped
        assert!(filter.check_ssrf_dns("localhost").is_ok());
    }

    #[test]
    fn test_network_stats() {
        let mut stats = NetworkStats::default();

        stats.record_allowed("api.example.com");
        stats.record_blocked("evil.com");

        assert_eq!(stats.total_checks, 2);
        assert_eq!(stats.allowed, 1);
        assert_eq!(stats.blocked, 1);
        assert_eq!(stats.unique_hosts.len(), 2);
    }
}
