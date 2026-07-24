//! The four startup choices: docker vs native, crossed with new vs load.

use crate::domain::{Deployment, Os};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartMode {
    DockerNew,
    DockerLoad,
    NativeNew,
    NativeLoad,
}

impl StartMode {
    pub const ALL: [StartMode; 4] = [
        StartMode::DockerNew,
        StartMode::DockerLoad,
        StartMode::NativeNew,
        StartMode::NativeLoad,
    ];

    /// Modes offered on a given OS. macOS is docker-only (native needs
    /// Linux/FreeBSD + systemd/rc.d + root).
    pub fn available(os: Os) -> Vec<StartMode> {
        match os {
            Os::MacOs => vec![StartMode::DockerNew, StartMode::DockerLoad],
            _ => StartMode::ALL.to_vec(),
        }
    }

    pub fn deployment(self) -> Deployment {
        match self {
            StartMode::DockerNew | StartMode::DockerLoad => Deployment::Docker,
            StartMode::NativeNew | StartMode::NativeLoad => Deployment::Native,
        }
    }

    pub fn is_load(self) -> bool {
        matches!(self, StartMode::DockerLoad | StartMode::NativeLoad)
    }

    pub fn label(self) -> &'static str {
        match self {
            StartMode::DockerNew => "Docker — create a new docker-compose",
            StartMode::DockerLoad => "Docker — load an existing docker-compose",
            StartMode::NativeNew => "Run on machine — create new ini files",
            StartMode::NativeLoad => "Run on machine — load existing ini files",
        }
    }
}
