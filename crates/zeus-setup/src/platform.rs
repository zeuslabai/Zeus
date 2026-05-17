//! Platform detection (OS, architecture, CPU count)

use anyhow::{Result, bail};

#[derive(Debug, Clone, PartialEq)]
pub enum Os {
    MacOS,
    Linux,
    FreeBSD,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Arch {
    X86_64,
    Aarch64,
}

#[derive(Debug, Clone)]
pub struct Platform {
    pub os: Os,
    pub arch: Arch,
    pub triple: String,
    pub cpu_count: usize,
}

impl Platform {
    pub fn detect() -> Result<Self> {
        let os = match std::env::consts::OS {
            "macos" => Os::MacOS,
            "linux" => Os::Linux,
            "freebsd" => Os::FreeBSD,
            other => bail!("Unsupported OS: {other}"),
        };

        let arch = match std::env::consts::ARCH {
            "x86_64" => Arch::X86_64,
            "aarch64" => Arch::Aarch64,
            other => bail!("Unsupported architecture: {other}"),
        };

        let triple = format!(
            "{}-{}",
            match &arch {
                Arch::X86_64 => "x86_64",
                Arch::Aarch64 => "aarch64",
            },
            match &os {
                Os::MacOS => "apple-darwin",
                Os::Linux => "unknown-linux-gnu",
                Os::FreeBSD => "unknown-freebsd",
            }
        );

        let cpu_count = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4);

        Ok(Self {
            os,
            arch,
            triple,
            cpu_count,
        })
    }

    pub fn is_macos(&self) -> bool {
        self.os == Os::MacOS
    }

    pub fn is_freebsd(&self) -> bool {
        self.os == Os::FreeBSD
    }

    pub fn is_linux(&self) -> bool {
        self.os == Os::Linux
    }
}

impl std::fmt::Display for Os {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Os::MacOS => write!(f, "macOS"),
            Os::Linux => write!(f, "Linux"),
            Os::FreeBSD => write!(f, "FreeBSD"),
        }
    }
}

impl std::fmt::Display for Arch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Arch::X86_64 => write!(f, "x86_64"),
            Arch::Aarch64 => write!(f, "aarch64"),
        }
    }
}

impl std::fmt::Display for Platform {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({}) [{}]", self.os, self.arch, self.triple)
    }
}
