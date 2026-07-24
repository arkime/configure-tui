//! Native (systemd/rc.d + `.ini` files) vs Docker (compose + `ARKIME__*` env).
//! This is the very first choice in the wizard; everything downstream branches
//! on it.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Deployment {
    Native,
    Docker,
}

impl Deployment {
    pub fn label(self) -> &'static str {
        match self {
            Deployment::Native => "Native (systemd / rc.d, writes config.ini)",
            Deployment::Docker => "Docker (docker-compose.yml + ARKIME__* env)",
        }
    }
}
