//! URL validation and SSRF protection
//!
//! Validates URLs to prevent Server-Side Request Forgery (SSRF) attacks
//! by blocking requests to private/internal IP ranges and localhost.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum UrlValidationError {
    #[error("Invalid URL format: {0}")]
    InvalidFormat(String),
    #[error("URL scheme must be http or https, got: {0}")]
    InvalidScheme(String),
    #[error("Access to private/internal IP addresses is forbidden: {0}")]
    PrivateIpForbidden(String),
    #[error("Access to localhost is forbidden")]
    LocalhostForbidden,
    #[error("URL must have a valid host")]
    MissingHost,
    #[error("Failed to resolve hostname: {0}")]
    DnsResolutionFailed(String),
}

/// Validates a URL for SSRF protection
///
/// Blocks:
/// - Non-HTTP/HTTPS schemes
/// - Private IP ranges (RFC 1918): 10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16
/// - Localhost: 127.0.0.0/8, ::1
/// - Link-local: 169.254.0.0/16, fe80::/10
/// - Loopback IPv6: ::1/128
/// - All zeros: 0.0.0.0
///
/// # Example
/// ```
/// use zeus_api::url_validator::validate_url;
///
/// // Valid external URL
/// assert!(validate_url("https://example.com/api").is_ok());
///
/// // Invalid: private IP
/// assert!(validate_url("http://192.168.1.1/admin").is_err());
/// assert!(validate_url("http://10.0.0.1/metadata").is_err());
///
/// // Invalid: localhost
/// assert!(validate_url("http://localhost:8080/internal").is_err());
/// assert!(validate_url("http://127.0.0.1/secret").is_err());
/// ```
pub fn validate_url(url: &str) -> Result<reqwest::Url, UrlValidationError> {
    // Parse URL
    let parsed =
        reqwest::Url::parse(url).map_err(|e| UrlValidationError::InvalidFormat(e.to_string()))?;

    // Check scheme (must be http or https)
    match parsed.scheme() {
        "http" | "https" => {}
        scheme => {
            return Err(UrlValidationError::InvalidScheme(scheme.to_string()));
        }
    }

    // Get host
    let host = parsed.host_str().ok_or(UrlValidationError::MissingHost)?;

    // Strip brackets from IPv6 addresses
    let host_clean = if host.starts_with('[') && host.ends_with(']') {
        &host[1..host.len() - 1]
    } else {
        host
    };

    // Try to parse as IP address
    if let Ok(ip) = host_clean.parse::<IpAddr>() {
        validate_ip(ip)?;
    } else {
        // Not an IP, treat as hostname
        validate_hostname(host_clean)?;
    }

    Ok(parsed)
}

/// Validates an IP address is not private/internal
fn validate_ip(ip: IpAddr) -> Result<(), UrlValidationError> {
    match ip {
        IpAddr::V4(ipv4) => validate_ipv4(ipv4),
        IpAddr::V6(ipv6) => validate_ipv6(ipv6),
    }
}

/// Validates IPv4 address is not private/internal
fn validate_ipv4(ip: Ipv4Addr) -> Result<(), UrlValidationError> {
    let octets = ip.octets();

    // 0.0.0.0/8 - Current network
    if octets[0] == 0 {
        return Err(UrlValidationError::PrivateIpForbidden(ip.to_string()));
    }

    // 10.0.0.0/8 - Private network
    if octets[0] == 10 {
        return Err(UrlValidationError::PrivateIpForbidden(ip.to_string()));
    }

    // 127.0.0.0/8 - Loopback
    if octets[0] == 127 {
        return Err(UrlValidationError::LocalhostForbidden);
    }

    // 169.254.0.0/16 - Link-local
    if octets[0] == 169 && octets[1] == 254 {
        return Err(UrlValidationError::PrivateIpForbidden(ip.to_string()));
    }

    // 172.16.0.0/12 - Private network
    if octets[0] == 172 && (octets[1] >= 16 && octets[1] <= 31) {
        return Err(UrlValidationError::PrivateIpForbidden(ip.to_string()));
    }

    // 192.168.0.0/16 - Private network
    if octets[0] == 192 && octets[1] == 168 {
        return Err(UrlValidationError::PrivateIpForbidden(ip.to_string()));
    }

    // 224.0.0.0/4 - Multicast
    if octets[0] >= 224 && octets[0] <= 239 {
        return Err(UrlValidationError::PrivateIpForbidden(ip.to_string()));
    }

    // 240.0.0.0/4 - Reserved
    if octets[0] >= 240 {
        return Err(UrlValidationError::PrivateIpForbidden(ip.to_string()));
    }

    Ok(())
}

/// Validates IPv6 address is not private/internal
fn validate_ipv6(ip: Ipv6Addr) -> Result<(), UrlValidationError> {
    // ::1 - Loopback
    if ip.is_loopback() {
        return Err(UrlValidationError::LocalhostForbidden);
    }

    // fe80::/10 - Link-local
    if (ip.segments()[0] & 0xffc0) == 0xfe80 {
        return Err(UrlValidationError::PrivateIpForbidden(ip.to_string()));
    }

    // fc00::/7 - Unique local address (ULA)
    if (ip.segments()[0] & 0xfe00) == 0xfc00 {
        return Err(UrlValidationError::PrivateIpForbidden(ip.to_string()));
    }

    // :: - Unspecified
    if ip.is_unspecified() {
        return Err(UrlValidationError::PrivateIpForbidden(ip.to_string()));
    }

    Ok(())
}

/// Validates hostname is not a known dangerous pattern
fn validate_hostname(host: &str) -> Result<(), UrlValidationError> {
    let host_lower = host.to_lowercase();

    // Block localhost variations
    if host_lower == "localhost"
        || host_lower.ends_with(".localhost")
        || host_lower == "[::]"
        || host_lower == "[::1]"
    {
        return Err(UrlValidationError::LocalhostForbidden);
    }

    // Block internal TLDs
    let internal_tlds = [".local", ".internal", ".private", ".corp", ".home", ".lan"];
    for tld in &internal_tlds {
        if host_lower.ends_with(tld) {
            return Err(UrlValidationError::PrivateIpForbidden(format!(
                "hostname ends with internal TLD: {}",
                tld
            )));
        }
    }

    // Block metadata service endpoints (cloud providers)
    let metadata_hosts = [
        "169.254.169.254",          // AWS, Azure, GCP metadata
        "metadata.google.internal", // GCP
        "metadata",                 // Generic
    ];
    for metadata in &metadata_hosts {
        if host_lower == *metadata || host_lower.ends_with(&format!(".{}", metadata)) {
            return Err(UrlValidationError::PrivateIpForbidden(format!(
                "metadata service endpoint forbidden: {}",
                metadata
            )));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_urls() {
        assert!(validate_url("https://example.com").is_ok());
        assert!(validate_url("http://example.com/path").is_ok());
        assert!(validate_url("https://api.github.com/repos").is_ok());
        assert!(validate_url("https://8.8.8.8").is_ok()); // Public DNS
        assert!(validate_url("https://1.1.1.1").is_ok()); // Cloudflare DNS
    }

    #[test]
    fn test_invalid_scheme() {
        assert!(validate_url("ftp://example.com").is_err());
        assert!(validate_url("file:///etc/passwd").is_err());
        assert!(validate_url("gopher://example.com").is_err());
    }

    #[test]
    fn test_localhost_blocked() {
        assert!(validate_url("http://localhost").is_err());
        assert!(validate_url("http://localhost:8080").is_err());
        assert!(validate_url("http://127.0.0.1").is_err());
        assert!(validate_url("http://127.0.0.2").is_err());
        assert!(validate_url("http://127.255.255.255").is_err());
        assert!(validate_url("http://[::1]").is_err());
    }

    #[test]
    fn test_private_ipv4_blocked() {
        // 10.0.0.0/8
        assert!(validate_url("http://10.0.0.1").is_err());
        assert!(validate_url("http://10.255.255.255").is_err());

        // 172.16.0.0/12
        assert!(validate_url("http://172.16.0.1").is_err());
        assert!(validate_url("http://172.31.255.255").is_err());

        // 192.168.0.0/16
        assert!(validate_url("http://192.168.0.1").is_err());
        assert!(validate_url("http://192.168.255.255").is_err());
    }

    #[test]
    fn test_link_local_blocked() {
        // 169.254.0.0/16
        assert!(validate_url("http://169.254.0.1").is_err());
        assert!(validate_url("http://169.254.169.254").is_err()); // AWS metadata
    }

    #[test]
    fn test_multicast_reserved_blocked() {
        assert!(validate_url("http://224.0.0.1").is_err()); // Multicast
        assert!(validate_url("http://255.255.255.255").is_err()); // Broadcast/Reserved
    }

    #[test]
    fn test_zero_ip_blocked() {
        assert!(validate_url("http://0.0.0.0").is_err());
    }

    #[test]
    fn test_internal_hostname_blocked() {
        assert!(validate_url("http://server.local").is_err());
        assert!(validate_url("http://server.internal").is_err());
        assert!(validate_url("http://server.private").is_err());
        assert!(validate_url("http://server.corp").is_err());
        assert!(validate_url("http://metadata.google.internal").is_err());
    }

    #[test]
    fn test_ipv6_loopback_blocked() {
        assert!(validate_url("http://[::1]").is_err());
        assert!(validate_url("http://[::1]:8080").is_err());
    }

    #[test]
    fn test_ipv6_link_local_blocked() {
        assert!(validate_url("http://[fe80::1]").is_err());
        assert!(validate_url("http://[fe80::dead:beef]").is_err());
    }

    #[test]
    fn test_ipv6_ula_blocked() {
        assert!(validate_url("http://[fc00::1]").is_err());
        assert!(validate_url("http://[fd00::1]").is_err());
    }

    #[test]
    fn test_invalid_format() {
        assert!(validate_url("not a url").is_err());
        assert!(validate_url("").is_err());
        assert!(validate_url("://example.com").is_err());
    }
}
