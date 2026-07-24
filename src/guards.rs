//! Pre-flight checks run before the TUI starts.
//!
//! - macOS is allowed but **docker-only** (native needs Linux/FreeBSD +
//!   systemd/rc.d + root); the startup screen hides native there.
//! - Root is NOT required to start — docker only writes files. The root
//!   requirement is deferred to apply time for the native deployment (see
//!   `app::run_apply`).

use crate::domain::{Os, Platform};

pub enum GuardOutcome {
    Ok(Platform),
    /// Fatal: print the message and exit non-zero without starting the TUI.
    Refuse(String),
}

/// Validate the operating system. Does not check root — see the module note.
pub fn preflight() -> GuardOutcome {
    let platform = Platform::detect();

    match platform.os {
        Os::Other => GuardOutcome::Refuse(
            "Unsupported operating system. Only Linux, FreeBSD, and macOS (docker only) \
             are supported."
                .to_string(),
        ),
        Os::Linux | Os::FreeBsd | Os::MacOs => GuardOutcome::Ok(platform),
    }
}

#[cfg(unix)]
pub fn is_root() -> bool {
    nix::unistd::Uid::effective().is_root()
}

#[cfg(not(unix))]
pub fn is_root() -> bool {
    false
}
