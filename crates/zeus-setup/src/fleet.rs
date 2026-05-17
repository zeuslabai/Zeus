//! Fleet configuration parser (fleet.conf)

use anyhow::Result;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct FleetNode {
    pub name: String,
    pub ip: String,
    pub os: String,
    pub user: String,
    pub comment: String,
}

impl FleetNode {
    pub fn is_macos(&self) -> bool {
        self.os.eq_ignore_ascii_case("darwin") || self.os.eq_ignore_ascii_case("macos")
    }

    pub fn is_freebsd(&self) -> bool {
        self.os.eq_ignore_ascii_case("freebsd")
    }

    pub fn ssh_target(&self) -> String {
        format!("{}@{}", self.user, self.ip)
    }
}

impl std::fmt::Display for FleetNode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.comment.is_empty() {
            write!(f, "{} ({}) [{}]", self.name, self.ip, self.os)
        } else {
            write!(
                f,
                "{} ({}) [{}] — {}",
                self.name, self.ip, self.os, self.comment
            )
        }
    }
}

/// Load fleet configuration from standard locations.
///
/// Search order:
/// 1. `~/.zeus/fleet.conf`
/// 2. `<project_root>/scripts/fleet.conf`
/// 3. `<project_root>/scripts/fleet.conf.example`
pub fn load_fleet_conf(project_root: Option<&std::path::Path>) -> Result<Vec<FleetNode>> {
    let candidates = [
        dirs::home_dir().map(|h| h.join(".zeus/fleet.conf")),
        project_root.map(|r| r.join("scripts/fleet.conf")),
        project_root.map(|r| r.join("scripts/fleet.conf.example")),
    ];

    for candidate in candidates.into_iter().flatten() {
        if candidate.exists() {
            return parse_fleet_conf(&candidate);
        }
    }

    Ok(Vec::new())
}

/// Parse a fleet.conf file.
///
/// Format: `NAME IP OS USER # optional comment`
/// - OS: darwin, freebsd, linux (defaults to "darwin")
/// - USER: SSH user (defaults to "mike")
///
/// Lines starting with `#` are skipped.
fn parse_fleet_conf(path: &PathBuf) -> Result<Vec<FleetNode>> {
    let content = std::fs::read_to_string(path)?;
    let mut nodes = Vec::new();

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        // Split line into fields and optional comment
        let (fields, comment) = if let Some(idx) = line.find('#') {
            (&line[..idx], line[idx + 1..].trim().to_string())
        } else {
            (line, String::new())
        };

        let parts: Vec<&str> = fields.split_whitespace().collect();
        if parts.len() >= 2 {
            nodes.push(FleetNode {
                name: parts[0].to_string(),
                ip: parts[1].to_string(),
                os: parts.get(2).unwrap_or(&"darwin").to_string(),
                user: parts.get(3).unwrap_or(&"mike").to_string(),
                comment,
            });
        }
    }

    Ok(nodes)
}

/// Look up an IP by fleet shortname (e.g. ".100" -> "192.168.1.100")
pub fn fleet_ip(nodes: &[FleetNode], name: &str) -> Option<String> {
    nodes.iter().find(|n| n.name == name).map(|n| n.ip.clone())
}

/// Look up a name by IP
pub fn fleet_name(nodes: &[FleetNode], ip: &str) -> Option<String> {
    nodes.iter().find(|n| n.ip == ip).map(|n| n.name.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_fleet_line() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fleet.conf");
        std::fs::write(
            &path,
            ".100 192.168.1.100 darwin mike  # Mac Mini M5\n\
             .224 192.168.1.224 freebsd mike  # FreeBSD\n\
             # comment line\n\
             \n\
             .107 192.168.1.107\n",
        )
        .unwrap();

        let nodes = parse_fleet_conf(&path).unwrap();
        assert_eq!(nodes.len(), 3);
        assert_eq!(nodes[0].name, ".100");
        assert_eq!(nodes[0].ip, "192.168.1.100");
        assert_eq!(nodes[0].os, "darwin");
        assert_eq!(nodes[0].comment, "Mac Mini M5");
        assert!(nodes[1].is_freebsd());
        assert_eq!(nodes[2].name, ".107");
        assert_eq!(nodes[2].os, "darwin"); // default
    }

    #[test]
    fn test_fleet_lookup() {
        let nodes = vec![
            FleetNode {
                name: ".100".into(),
                ip: "192.168.1.100".into(),
                os: "darwin".into(),
                user: "mike".into(),
                comment: String::new(),
            },
            FleetNode {
                name: ".224".into(),
                ip: "192.168.1.224".into(),
                os: "freebsd".into(),
                user: "mike".into(),
                comment: String::new(),
            },
        ];

        assert_eq!(fleet_ip(&nodes, ".100"), Some("192.168.1.100".into()));
        assert_eq!(fleet_ip(&nodes, ".999"), None);
        assert_eq!(fleet_name(&nodes, "192.168.1.224"), Some(".224".into()));
        assert!(nodes[0].is_macos());
        assert!(nodes[1].is_freebsd());
    }
}
