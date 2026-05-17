//! Network tools

use crate::TalosTool;
use async_trait::async_trait;
use serde_json::{Value, json};
use std::process::Command;
use zeus_core::{Error, Result, ToolSchema};

/// Ping a host
pub struct PingTool;

#[async_trait]
impl TalosTool for PingTool {
    fn name(&self) -> &'static str {
        "ping"
    }
    fn description(&self) -> &'static str {
        "Ping a host to check connectivity"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("host", "string", "Host to ping", true)
            .with_param("count", "integer", "Number of pings (default 3)", false)
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let host = args
            .get("host")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing host".to_string()))?;

        let count = args.get("count").and_then(|v| v.as_u64()).unwrap_or(3);

        let output = Command::new("ping")
            .arg("-c")
            .arg(count.to_string())
            .arg(host)
            .output()
            .map_err(|e| Error::Tool(format!("Failed to execute ping: {}", e)))?;

        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).to_string())
        } else {
            Err(Error::Tool(format!(
                "Ping failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )))
        }
    }
}

/// DNS lookup
pub struct DnsTool;

#[async_trait]
impl TalosTool for DnsTool {
    fn name(&self) -> &'static str {
        "dns_lookup"
    }
    fn description(&self) -> &'static str {
        "Perform DNS lookup for a domain"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("domain", "string", "Domain to look up", true)
            .with_param("type", "string", "Record type (A, AAAA, MX, etc.)", false)
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let domain = args
            .get("domain")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing domain".to_string()))?;

        let record_type = args.get("type").and_then(|v| v.as_str()).unwrap_or("A");

        let output = Command::new("dig")
            .arg("+short")
            .arg("-t")
            .arg(record_type)
            .arg(domain)
            .output()
            .map_err(|e| Error::Tool(format!("Failed to execute dig: {}", e)))?;

        let output_str = String::from_utf8_lossy(&output.stdout);
        let records: Vec<String> = output_str.trim().lines().map(|s| s.to_string()).collect();

        let result = json!({
            "domain": domain,
            "type": record_type,
            "records": records,
        });

        Ok(serde_json::to_string_pretty(&result)?)
    }
}

/// Check if a port is open
pub struct PortCheckTool;

#[async_trait]
impl TalosTool for PortCheckTool {
    fn name(&self) -> &'static str {
        "port_check"
    }
    fn description(&self) -> &'static str {
        "Check if a port is open on a host"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("host", "string", "Host to check", true)
            .with_param("port", "integer", "Port number", true)
            .with_param(
                "timeout",
                "integer",
                "Timeout in seconds (default 5)",
                false,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let host = args
            .get("host")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing host".to_string()))?;

        let port = args
            .get("port")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| Error::Tool("Missing port".to_string()))?;

        let timeout = args.get("timeout").and_then(|v| v.as_u64()).unwrap_or(5);

        use std::net::{TcpStream, ToSocketAddrs};
        use std::time::Duration;

        let addr = format!("{}:{}", host, port);
        let socket_addr = addr
            .to_socket_addrs()
            .map_err(|e| Error::Tool(format!("Invalid address: {}", e)))?
            .next()
            .ok_or_else(|| Error::Tool("Could not resolve address".to_string()))?;

        let is_open =
            TcpStream::connect_timeout(&socket_addr, Duration::from_secs(timeout)).is_ok();

        let result = json!({
            "host": host,
            "port": port,
            "is_open": is_open,
            "status": if is_open { "open" } else { "closed" },
        });

        Ok(serde_json::to_string_pretty(&result)?)
    }
}

// ── Extended network tools ───────────────────────────────────────────

/// Get network interface information
pub struct NetworkInfoTool;

#[async_trait]
impl TalosTool for NetworkInfoTool {
    fn name(&self) -> &'static str {
        "network_info"
    }
    fn description(&self) -> &'static str {
        "Get network interface information (IP addresses, interfaces, status)"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
    }

    async fn execute(&self, _args: Value) -> Result<String> {
        let output = tokio::process::Command::new("ifconfig")
            .output()
            .await
            .map_err(|e| Error::Tool(format!("Failed: {}", e)))?;
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }
}

/// List network services
pub struct NetworkServicesTool;

#[async_trait]
impl TalosTool for NetworkServicesTool {
    fn name(&self) -> &'static str {
        "network_services"
    }
    fn description(&self) -> &'static str {
        "List all network services (Wi-Fi, Ethernet, etc.)"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
    }

    async fn execute(&self, _args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            let output = tokio::process::Command::new("networksetup")
                .arg("-listallnetworkservices")
                .output()
                .await
                .map_err(|e| Error::Tool(format!("Failed: {}", e)))?;
            let text = String::from_utf8_lossy(&output.stdout);
            let services: Vec<&str> = text.lines().skip(1).collect();
            Ok(serde_json::to_string_pretty(
                &json!({ "services": services }),
            )?)
        }
        #[cfg(not(target_os = "macos"))]
        Ok("network_services only available on macOS".to_string())
    }
}

/// List network locations
pub struct NetworkLocationsTool;

#[async_trait]
impl TalosTool for NetworkLocationsTool {
    fn name(&self) -> &'static str {
        "network_locations"
    }
    fn description(&self) -> &'static str {
        "List all network locations"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
    }

    async fn execute(&self, _args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            let output = tokio::process::Command::new("networksetup")
                .arg("-listlocations")
                .output()
                .await
                .map_err(|e| Error::Tool(format!("Failed: {}", e)))?;
            let text = String::from_utf8_lossy(&output.stdout);
            let locations: Vec<&str> = text.lines().collect();
            Ok(serde_json::to_string_pretty(
                &json!({ "locations": locations }),
            )?)
        }
        #[cfg(not(target_os = "macos"))]
        Ok("network_locations only available on macOS".to_string())
    }
}

/// Get current network location
pub struct NetworkCurrentLocationTool;

#[async_trait]
impl TalosTool for NetworkCurrentLocationTool {
    fn name(&self) -> &'static str {
        "network_current_location"
    }
    fn description(&self) -> &'static str {
        "Get the current active network location"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
    }

    async fn execute(&self, _args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            let output = tokio::process::Command::new("networksetup")
                .arg("-getcurrentlocation")
                .output()
                .await
                .map_err(|e| Error::Tool(format!("Failed: {}", e)))?;
            Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
        }
        #[cfg(not(target_os = "macos"))]
        Ok("network_current_location only available on macOS".to_string())
    }
}

/// Switch network location
pub struct NetworkSwitchLocationTool;

#[async_trait]
impl TalosTool for NetworkSwitchLocationTool {
    fn name(&self) -> &'static str {
        "network_switch_location"
    }
    fn description(&self) -> &'static str {
        "Switch to a different network location"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "location",
            "string",
            "Network location name to switch to",
            true,
        )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let location = args
            .get("location")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("'location' is required".to_string()))?;
        #[cfg(target_os = "macos")]
        {
            let output = tokio::process::Command::new("networksetup")
                .args(["-switchtolocation", location])
                .output()
                .await
                .map_err(|e| Error::Tool(format!("Failed: {}", e)))?;
            if output.status.success() {
                Ok(format!("Switched to location '{}'", location))
            } else {
                Err(Error::Tool(
                    String::from_utf8_lossy(&output.stderr).to_string(),
                ))
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = location;
            Ok("network_switch_location only available on macOS".to_string())
        }
    }
}

/// Set DNS servers
pub struct DnsSetTool;

#[async_trait]
impl TalosTool for DnsSetTool {
    fn name(&self) -> &'static str {
        "dns_set"
    }
    fn description(&self) -> &'static str {
        "Set DNS servers for a network service"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "service",
                "string",
                "Network service name (e.g. 'Wi-Fi')",
                true,
            )
            .with_param(
                "servers",
                "string",
                "Space-separated DNS server IPs (e.g. '8.8.8.8 8.8.4.4')",
                true,
            )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let service = args
            .get("service")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("'service' is required".to_string()))?;
        let servers = args
            .get("servers")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("'servers' is required".to_string()))?;
        #[cfg(target_os = "macos")]
        {
            let mut cmd = tokio::process::Command::new("networksetup");
            cmd.arg("-setdnsservers").arg(service);
            for s in servers.split_whitespace() {
                cmd.arg(s);
            }
            let output = cmd
                .output()
                .await
                .map_err(|e| Error::Tool(format!("Failed: {}", e)))?;
            if output.status.success() {
                Ok(format!("DNS set to {} for {}", servers, service))
            } else {
                Err(Error::Tool(
                    String::from_utf8_lossy(&output.stderr).to_string(),
                ))
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (service, servers);
            Ok("dns_set only available on macOS".to_string())
        }
    }
}

/// Reset DNS to DHCP
pub struct DnsResetTool;

#[async_trait]
impl TalosTool for DnsResetTool {
    fn name(&self) -> &'static str {
        "dns_reset"
    }
    fn description(&self) -> &'static str {
        "Reset DNS servers to automatic/DHCP"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "service",
            "string",
            "Network service name (e.g. 'Wi-Fi')",
            true,
        )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let service = args
            .get("service")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("'service' is required".to_string()))?;
        #[cfg(target_os = "macos")]
        {
            let output = tokio::process::Command::new("networksetup")
                .args(["-setdnsservers", service, "empty"])
                .output()
                .await
                .map_err(|e| Error::Tool(format!("Failed: {}", e)))?;
            if output.status.success() {
                Ok(format!("DNS reset for {}", service))
            } else {
                Err(Error::Tool(
                    String::from_utf8_lossy(&output.stderr).to_string(),
                ))
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = service;
            Ok("dns_reset only available on macOS".to_string())
        }
    }
}

/// Set IP to DHCP
pub struct IpSetDhcpTool;

#[async_trait]
impl TalosTool for IpSetDhcpTool {
    fn name(&self) -> &'static str {
        "ip_set_dhcp"
    }
    fn description(&self) -> &'static str {
        "Set a network service to use DHCP for IP address"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "service",
            "string",
            "Network service name (e.g. 'Wi-Fi')",
            true,
        )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let service = args
            .get("service")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("'service' is required".to_string()))?;
        #[cfg(target_os = "macos")]
        {
            let output = tokio::process::Command::new("networksetup")
                .args(["-setdhcp", service])
                .output()
                .await
                .map_err(|e| Error::Tool(format!("Failed: {}", e)))?;
            if output.status.success() {
                Ok(format!("IP set to DHCP for {}", service))
            } else {
                Err(Error::Tool(
                    String::from_utf8_lossy(&output.stderr).to_string(),
                ))
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = service;
            Ok("ip_set_dhcp only available on macOS".to_string())
        }
    }
}

/// Set manual IP address
pub struct IpSetManualTool;

#[async_trait]
impl TalosTool for IpSetManualTool {
    fn name(&self) -> &'static str {
        "ip_set_manual"
    }
    fn description(&self) -> &'static str {
        "Set a manual IP address for a network service"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("service", "string", "Network service name", true)
            .with_param("ip", "string", "IP address", true)
            .with_param("subnet", "string", "Subnet mask", true)
            .with_param("router", "string", "Router/gateway IP", true)
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let service = args
            .get("service")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("'service' is required".to_string()))?;
        let ip = args
            .get("ip")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("'ip' is required".to_string()))?;
        let subnet = args
            .get("subnet")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("'subnet' is required".to_string()))?;
        let router = args
            .get("router")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("'router' is required".to_string()))?;
        #[cfg(target_os = "macos")]
        {
            let output = tokio::process::Command::new("networksetup")
                .args(["-setmanual", service, ip, subnet, router])
                .output()
                .await
                .map_err(|e| Error::Tool(format!("Failed: {}", e)))?;
            if output.status.success() {
                Ok(format!("IP set to {} for {}", ip, service))
            } else {
                Err(Error::Tool(
                    String::from_utf8_lossy(&output.stderr).to_string(),
                ))
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (service, ip, subnet, router);
            Ok("ip_set_manual only available on macOS".to_string())
        }
    }
}

/// Set HTTP/HTTPS proxy
pub struct ProxySetTool;

#[async_trait]
impl TalosTool for ProxySetTool {
    fn name(&self) -> &'static str {
        "proxy_set"
    }
    fn description(&self) -> &'static str {
        "Set HTTP/HTTPS proxy for a network service"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("service", "string", "Network service name", true)
            .with_param("host", "string", "Proxy host", true)
            .with_param("port", "integer", "Proxy port", true)
            .with_param("https", "boolean", "Set HTTPS proxy (default false)", false)
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let service = args
            .get("service")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("'service' is required".to_string()))?;
        let host = args
            .get("host")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("'host' is required".to_string()))?;
        let port = args
            .get("port")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| Error::Tool("'port' is required".to_string()))?;
        let https = args.get("https").and_then(|v| v.as_bool()).unwrap_or(false);
        #[cfg(target_os = "macos")]
        {
            let flag = if https {
                "-setsecurewebproxy"
            } else {
                "-setwebproxy"
            };
            let output = tokio::process::Command::new("networksetup")
                .args([flag, service, host, &port.to_string()])
                .output()
                .await
                .map_err(|e| Error::Tool(format!("Failed: {}", e)))?;
            if output.status.success() {
                Ok(format!("Proxy set to {}:{} for {}", host, port, service))
            } else {
                Err(Error::Tool(
                    String::from_utf8_lossy(&output.stderr).to_string(),
                ))
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (service, host, port, https);
            Ok("proxy_set only available on macOS".to_string())
        }
    }
}

/// Disable proxy
pub struct ProxyDisableTool;

#[async_trait]
impl TalosTool for ProxyDisableTool {
    fn name(&self) -> &'static str {
        "proxy_disable"
    }
    fn description(&self) -> &'static str {
        "Disable HTTP and HTTPS proxies for a network service"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "service",
            "string",
            "Network service name",
            true,
        )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let service = args
            .get("service")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("'service' is required".to_string()))?;
        #[cfg(target_os = "macos")]
        {
            let _ = tokio::process::Command::new("networksetup")
                .args(["-setwebproxystate", service, "off"])
                .output()
                .await;
            let _ = tokio::process::Command::new("networksetup")
                .args(["-setsecurewebproxystate", service, "off"])
                .output()
                .await;
            Ok(format!("Proxies disabled for {}", service))
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = service;
            Ok("proxy_disable only available on macOS".to_string())
        }
    }
}
