const ZEUS_GATEWAY_URL_ENV: &str = "ZEUS_GATEWAY_URL";

/// Optional gateway target selected outside saved config.
///
/// Used by `zeus tui --port ...` / `--gateway-url ...` so the production TUI
/// can point at another gateway without editing `config.toml`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GatewayTargetOverride {
    pub port: Option<u16>,
    pub gateway_url: Option<String>,
}

impl GatewayTargetOverride {
    pub fn from_cli(port: Option<u16>, gateway_url: Option<String>) -> Option<Self> {
        if port.is_some()
            || gateway_url
                .as_ref()
                .is_some_and(|url| !url.trim().is_empty())
        {
            Some(Self { port, gateway_url })
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedGatewayTarget {
    pub base_url: String,
    pub display_host: String,
    pub display_port: u16,
}

/// Resolve the gateway endpoint used by all production TUI API clients.
///
/// Precedence: `--gateway-url`, `--port`, `ZEUS_GATEWAY_URL`, config/default.
/// A config bind address of `0.0.0.0` is normalized to loopback because it is
/// not a client connect address.
pub fn resolve_gateway_target(
    config: &zeus_core::Config,
    override_target: Option<&GatewayTargetOverride>,
) -> ResolvedGatewayTarget {
    let env_gateway_url = std::env::var(ZEUS_GATEWAY_URL_ENV).ok();
    resolve_gateway_target_with_env(config, override_target, env_gateway_url.as_deref())
}

pub fn resolve_gateway_target_with_env(
    config: &zeus_core::Config,
    override_target: Option<&GatewayTargetOverride>,
    env_gateway_url: Option<&str>,
) -> ResolvedGatewayTarget {
    let (config_host, config_port) = config_gateway_host_port(config);

    if let Some(url) = override_target
        .and_then(|target| target.gateway_url.as_deref())
        .filter(|url| !url.trim().is_empty())
    {
        return target_from_url(url);
    }

    if let Some(port) = override_target.and_then(|target| target.port) {
        return ResolvedGatewayTarget {
            base_url: format!("http://{config_host}:{port}"),
            display_host: config_host,
            display_port: port,
        };
    }

    if let Some(url) = env_gateway_url.filter(|url| !url.trim().is_empty()) {
        return target_from_url(url);
    }

    ResolvedGatewayTarget {
        base_url: format!("http://{config_host}:{config_port}"),
        display_host: config_host,
        display_port: config_port,
    }
}

fn config_gateway_host_port(config: &zeus_core::Config) -> (String, u16) {
    config
        .gateway
        .as_ref()
        .map(|gateway| {
            let host = if gateway.host == "0.0.0.0" {
                "127.0.0.1".to_string()
            } else {
                gateway.host.clone()
            };
            (host, gateway.port)
        })
        .unwrap_or_else(|| ("localhost".to_string(), 8080))
}

fn target_from_url(raw_url: &str) -> ResolvedGatewayTarget {
    let base_url = normalize_gateway_url(raw_url);
    let parsed = reqwest::Url::parse(&base_url).ok();
    let display_host = parsed
        .as_ref()
        .and_then(|url| url.host_str())
        .unwrap_or("")
        .to_string();
    let display_port = parsed
        .as_ref()
        .and_then(|url| url.port_or_known_default())
        .unwrap_or(80);

    ResolvedGatewayTarget {
        base_url,
        display_host,
        display_port,
    }
}

fn normalize_gateway_url(raw_url: &str) -> String {
    let trimmed = raw_url.trim().trim_end_matches('/');
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        trimmed.to_string()
    } else {
        format!("http://{trimmed}")
    }
}

#[cfg(test)]
mod tests {
    use super::{GatewayTargetOverride, resolve_gateway_target_with_env};
    use zeus_core::{Config, GatewayConfig};

    fn config_with_gateway(host: &str, port: u16) -> Config {
        Config {
            gateway: Some(GatewayConfig {
                host: host.to_string(),
                port,
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[test]
    fn config_gateway_is_default_target() {
        let config = config_with_gateway("10.0.0.5", 8181);
        let target = resolve_gateway_target_with_env(&config, None, None);

        assert_eq!(target.base_url, "http://10.0.0.5:8181");
        assert_eq!(target.display_host, "10.0.0.5");
        assert_eq!(target.display_port, 8181);
    }

    #[test]
    fn config_bind_all_host_is_normalized_to_loopback() {
        let config = config_with_gateway("0.0.0.0", 8080);
        let target = resolve_gateway_target_with_env(&config, None, None);

        assert_eq!(target.base_url, "http://127.0.0.1:8080");
        assert_eq!(target.display_host, "127.0.0.1");
        assert_eq!(target.display_port, 8080);
    }

    #[test]
    fn env_gateway_url_overrides_config_without_persisting() {
        let config = config_with_gateway("10.0.0.5", 8181);
        let target = resolve_gateway_target_with_env(
            &config,
            None,
            Some("https://remote.example:9443/zeus/"),
        );

        assert_eq!(target.base_url, "https://remote.example:9443/zeus");
        assert_eq!(target.display_host, "remote.example");
        assert_eq!(target.display_port, 9443);
    }

    #[test]
    fn cli_port_overrides_env_and_uses_config_host() {
        let config = config_with_gateway("0.0.0.0", 8080);
        let cli = GatewayTargetOverride {
            port: Some(9090),
            gateway_url: None,
        };
        let target =
            resolve_gateway_target_with_env(&config, Some(&cli), Some("http://env.example:7777"));

        assert_eq!(target.base_url, "http://127.0.0.1:9090");
        assert_eq!(target.display_host, "127.0.0.1");
        assert_eq!(target.display_port, 9090);
    }

    #[test]
    fn cli_gateway_url_overrides_cli_port_and_env() {
        let config = config_with_gateway("10.0.0.5", 8181);
        let cli = GatewayTargetOverride {
            port: Some(9090),
            gateway_url: Some("remote.example:9999".to_string()),
        };
        let target =
            resolve_gateway_target_with_env(&config, Some(&cli), Some("http://env.example:7777"));

        assert_eq!(target.base_url, "http://remote.example:9999");
        assert_eq!(target.display_host, "remote.example");
        assert_eq!(target.display_port, 9999);
    }

    #[test]
    fn https_gateway_url_without_port_displays_443() {
        let config = Config::default();
        let target = resolve_gateway_target_with_env(&config, None, Some("https://gw.example"));

        assert_eq!(target.base_url, "https://gw.example");
        assert_eq!(target.display_host, "gw.example");
        assert_eq!(target.display_port, 443);
    }
}
