//! Pre-flight checks run before the TUI starts: refuse macOS, detect FreeBSD.
//!
//! Root is NOT required to start — docker mode only writes compose/env files to
//! a directory and needs no privileges. The root requirement is deferred to
//! apply time and enforced only for the native deployment (see `app::run_apply`).

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
        Os::MacOs => GuardOutcome::Refuse(
            "This tool does not run on macOS. Create the config files by hand \
             (see the Arkime docs)."
                .to_string(),
        ),
        Os::Other => GuardOutcome::Refuse(
            "Unsupported operating system. Only Linux and FreeBSD are supported.".to_string(),
        ),
        Os::Linux | Os::FreeBsd => GuardOutcome::Ok(platform),
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
