//! Pre-flight checks run before the TUI starts, mirroring and extending the bash
//! `Configure` guards: refuse macOS, detect FreeBSD, require root.

use crate::domain::{Os, Platform};

pub enum GuardOutcome {
    Ok(Platform),
    /// Fatal: print the message and exit non-zero without starting the TUI.
    Refuse(String),
}

/// Validate the environment. `require_root` is honored only on Linux/FreeBSD;
/// on other systems we never reach here because the OS check refuses first.
pub fn preflight(require_root: bool) -> GuardOutcome {
    let platform = Platform::detect();

    match platform.os {
        Os::MacOs => {
            return GuardOutcome::Refuse(
                "This tool does not run on macOS. Create the config files by hand \
                 (see the Arkime docs)."
                    .to_string(),
            );
        }
        Os::Other => {
            return GuardOutcome::Refuse(
                "Unsupported operating system. Only Linux and FreeBSD are supported.".to_string(),
            );
        }
        Os::Linux | Os::FreeBsd => {}
    }

    if require_root && !is_root() {
        return GuardOutcome::Refuse(
            "This tool must be run as root (it writes system config and manages services)."
                .to_string(),
        );
    }

    GuardOutcome::Ok(platform)
}

#[cfg(unix)]
pub fn is_root() -> bool {
    nix::unistd::Uid::effective().is_root()
}

#[cfg(not(unix))]
pub fn is_root() -> bool {
    false
}
