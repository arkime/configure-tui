//! Network interface enumeration, replacing the bash `ip -o link` listing.
//!
//! Linux: read `/sys/class/net` (zero deps). FreeBSD: shell out to
//! `ifconfig -l`. Loopback is filtered out. Best-effort — a failure just yields
//! an empty list and the user types the interface manually.

use crate::domain::Os;

pub fn detect(os: Os) -> Vec<String> {
    let mut names = match os {
        Os::Linux => from_sys_class_net(),
        Os::FreeBsd => from_ifconfig(),
        _ => Vec::new(),
    };
    names.retain(|n| n != "lo" && !n.starts_with("lo"));
    names.sort();
    names.dedup();
    names
}

fn from_sys_class_net() -> Vec<String> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir("/sys/class/net") {
        for e in entries.flatten() {
            if let Some(name) = e.file_name().to_str() {
                out.push(name.to_string());
            }
        }
    }
    out
}

fn from_ifconfig() -> Vec<String> {
    std::process::Command::new("ifconfig")
        .arg("-l")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .split_whitespace()
                .map(|s| s.to_string())
                .collect()
        })
        .unwrap_or_default()
}
