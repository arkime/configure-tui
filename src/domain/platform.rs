//! OS + service-manager detection. Extends the old bash checks (refuse macOS,
//! require root) with explicit FreeBSD handling so native mode can use rc.d
//! instead of systemd.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    Linux,
    FreeBsd,
    MacOs,
    Other,
}

impl Os {
    pub fn detect() -> Os {
        match std::env::consts::OS {
            "linux" => Os::Linux,
            "freebsd" => Os::FreeBsd,
            "macos" => Os::MacOs,
            _ => Os::Other,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceManagerKind {
    /// Linux with systemd present.
    Systemd,
    /// FreeBSD rc.d (`sysrc` + `service`).
    FreeBsdRc,
    /// No init integration available; the tool writes config but does not
    /// enable/start services.
    None,
}

#[derive(Debug, Clone, Copy)]
pub struct Platform {
    pub os: Os,
    pub service_manager: ServiceManagerKind,
}

impl Platform {
    pub fn detect() -> Platform {
        let os = Os::detect();
        Platform {
            os,
            service_manager: detect_service_manager(os),
        }
    }
}

fn detect_service_manager(os: Os) -> ServiceManagerKind {
    match os {
        // Match the bash gate: systemd dir present and systemctl executable.
        Os::Linux
            if std::path::Path::new("/etc/systemd").is_dir() && is_executable("/bin/systemctl") =>
        {
            ServiceManagerKind::Systemd
        }
        Os::FreeBsd => ServiceManagerKind::FreeBsdRc,
        _ => ServiceManagerKind::None,
    }
}

fn is_executable(path: &str) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path)
            .map(|m| m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        std::path::Path::new(path).exists()
    }
}
