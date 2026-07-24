//! Host→container bind mounts offered in Docker mode. Each mount can be toggled
//! on/off and its **host path edited**; the container path is fixed (that's the
//! part we understand). Custom host paths round-trip when loading a compose.

use crate::domain::{Component, Components};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MountKind {
    /// Config dir, shared by every Arkime service.
    Etc,
    /// PCAP storage (capture writes, viewer reads).
    Pcap,
    /// MaxMind GeoIP database dir.
    GeoIpData,
    /// geoipupdate config file.
    GeoIpConf,
}

impl MountKind {
    pub const ALL: [MountKind; 4] = [
        MountKind::Etc,
        MountKind::Pcap,
        MountKind::GeoIpData,
        MountKind::GeoIpConf,
    ];

    pub fn index(self) -> usize {
        self as usize
    }

    /// Short label for the row (what this mount is for).
    pub fn label(self) -> &'static str {
        match self {
            MountKind::Etc => "etc",
            MountKind::Pcap => "raw",
            MountKind::GeoIpData => "geoip",
            MountKind::GeoIpConf => "geoip.conf",
        }
    }

    /// Default host path (editable in the wizard).
    pub fn default_host(self) -> &'static str {
        match self {
            MountKind::Etc => "/arkime/etc",
            MountKind::Pcap => "/arkime/raw",
            MountKind::GeoIpData => "/arkime/maxmind",
            MountKind::GeoIpConf => "./GeoIP.conf",
        }
    }

    /// Fixed container path (what we understand — not editable).
    pub fn container(self) -> &'static str {
        match self {
            MountKind::Etc => "/opt/arkime/etc",
            MountKind::Pcap => "/opt/arkime/raw",
            MountKind::GeoIpData => "/var/lib/GeoIP",
            MountKind::GeoIpConf => "/etc/GeoIP.conf",
        }
    }

    /// Whether this mount is worth offering, given the selected components.
    pub fn relevant(self, components: &Components) -> bool {
        match self {
            MountKind::Etc => components.any(),
            MountKind::Pcap | MountKind::GeoIpData | MountKind::GeoIpConf => {
                components.capture || components.viewer
            }
        }
    }

    /// Whether this mount attaches to a given component's service.
    pub fn attaches_to(self, component: Component) -> bool {
        match self {
            MountKind::Etc => true,
            MountKind::Pcap | MountKind::GeoIpData | MountKind::GeoIpConf => {
                matches!(component, Component::Capture | Component::Viewer)
            }
        }
    }

    /// If `vol` (a `host:container` string) targets this mount's container, pull
    /// out its host part. Used to round-trip a loaded compose.
    pub fn host_of(self, vol: &str) -> Option<String> {
        vol.strip_suffix(self.container())
            .and_then(|p| p.strip_suffix(':'))
            .map(|h| h.to_string())
    }
}

/// Per-mount enable flags + editable host paths, indexed by `MountKind::index`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MountSelection {
    enabled: [bool; 4],
    hosts: [String; 4],
}

impl Default for MountSelection {
    fn default() -> Self {
        MountSelection {
            enabled: [true; 4],
            hosts: MountKind::ALL.map(|k| k.default_host().to_string()),
        }
    }
}

impl MountSelection {
    pub fn is_enabled(&self, kind: MountKind) -> bool {
        self.enabled[kind.index()]
    }

    pub fn set_enabled(&mut self, kind: MountKind, on: bool) {
        self.enabled[kind.index()] = on;
    }

    pub fn toggle(&mut self, kind: MountKind) {
        let slot = &mut self.enabled[kind.index()];
        *slot = !*slot;
    }

    pub fn host(&self, kind: MountKind) -> &str {
        &self.hosts[kind.index()]
    }

    pub fn set_host(&mut self, kind: MountKind, host: String) {
        self.hosts[kind.index()] = host;
    }

    /// Mutable host, for inline editing.
    pub fn host_mut(&mut self, kind: MountKind) -> &mut String {
        &mut self.hosts[kind.index()]
    }

    /// The `host:container` spec for a mount (using the current host path).
    pub fn spec(&self, kind: MountKind) -> String {
        format!("{}:{}", self.host(kind), kind.container())
    }

    /// Relevant mounts for the current components, in display order.
    pub fn relevant_kinds(components: &Components) -> Vec<MountKind> {
        MountKind::ALL
            .into_iter()
            .filter(|k| k.relevant(components))
            .collect()
    }

    /// The `host:container` specs to attach to `component`'s service.
    pub fn specs_for(&self, component: Component, components: &Components) -> Vec<String> {
        MountKind::ALL
            .into_iter()
            .filter(|k| k.relevant(components) && self.is_enabled(*k) && k.attaches_to(component))
            .map(|k| self.spec(k))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caps() -> Components {
        Components {
            capture: true,
            viewer: true,
            ..Default::default()
        }
    }

    #[test]
    fn capture_gets_all_four_mounts_by_default() {
        let sel = MountSelection::default();
        let specs = sel.specs_for(Component::Capture, &caps());
        assert_eq!(
            specs,
            vec![
                "/arkime/etc:/opt/arkime/etc",
                "/arkime/raw:/opt/arkime/raw",
                "/arkime/maxmind:/var/lib/GeoIP",
                "./GeoIP.conf:/etc/GeoIP.conf",
            ]
        );
    }

    #[test]
    fn edited_host_shows_in_spec() {
        let mut sel = MountSelection::default();
        sel.set_host(MountKind::Pcap, "/data/pcap".into());
        assert_eq!(sel.spec(MountKind::Pcap), "/data/pcap:/opt/arkime/raw");
    }

    #[test]
    fn host_of_parses_loaded_volume() {
        assert_eq!(
            MountKind::Pcap.host_of("/data/pcap:/opt/arkime/raw"),
            Some("/data/pcap".to_string())
        );
        assert_eq!(MountKind::Pcap.host_of("/x:/other"), None);
    }

    #[test]
    fn wise_only_sees_etc_and_nothing_geo() {
        let wise = Components {
            wise: true,
            ..Default::default()
        };
        assert_eq!(MountSelection::relevant_kinds(&wise), vec![MountKind::Etc]);
    }

    #[test]
    fn toggling_off_removes_the_mount() {
        let mut sel = MountSelection::default();
        sel.toggle(MountKind::Pcap);
        let specs = sel.specs_for(Component::Capture, &caps());
        assert!(!specs.iter().any(|s| s.contains("/opt/arkime/raw")));
        assert!(specs.iter().any(|s| s.contains("/opt/arkime/etc")));
    }
}
