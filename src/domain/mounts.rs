//! Suggested host→container bind mounts offered in Docker mode. Each is
//! toggleable in the wizard (defaulting on) and, when enabled, attached to the
//! relevant component services in the generated `docker-compose.yml`.

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

    pub fn host(self) -> &'static str {
        match self {
            MountKind::Etc => "/arkime/etc",
            MountKind::Pcap => "/arkime/pcap",
            MountKind::GeoIpData => "/arkime/maxmind",
            MountKind::GeoIpConf => "./GeoIP.conf",
        }
    }

    pub fn container(self) -> &'static str {
        match self {
            MountKind::Etc => "/opt/arkime/etc",
            MountKind::Pcap => "/opt/arkime/raw",
            MountKind::GeoIpData => "/var/lib/GeoIP",
            MountKind::GeoIpConf => "/etc/GeoIP.conf",
        }
    }

    /// `host:container` string for a compose `volumes:` entry.
    pub fn spec(self) -> String {
        format!("{}:{}", self.host(), self.container())
    }

    /// Whether this mount is worth offering, given the selected components.
    pub fn relevant(self, components: &Components) -> bool {
        match self {
            // Every Arkime service reads its config from etc.
            MountKind::Etc => components.any(),
            // Only capture/viewer touch pcap and geo data.
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
}

/// Per-mount enable flags, indexed by `MountKind::index`. All on by default.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MountSelection([bool; 4]);

impl Default for MountSelection {
    fn default() -> Self {
        MountSelection([true; 4])
    }
}

impl MountSelection {
    pub fn is_enabled(&self, kind: MountKind) -> bool {
        self.0[kind.index()]
    }

    pub fn toggle(&mut self, kind: MountKind) {
        let slot = &mut self.0[kind.index()];
        *slot = !*slot;
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
            .map(|k| k.spec())
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
                "/arkime/pcap:/opt/arkime/raw",
                "/arkime/maxmind:/var/lib/GeoIP",
                "./GeoIP.conf:/etc/GeoIP.conf",
            ]
        );
    }

    #[test]
    fn wise_only_sees_etc_and_nothing_geo() {
        let wise = Components {
            wise: true,
            ..Default::default()
        };
        assert_eq!(MountSelection::relevant_kinds(&wise), vec![MountKind::Etc]);
        let sel = MountSelection::default();
        assert_eq!(
            sel.specs_for(Component::Wise, &wise),
            vec!["/arkime/etc:/opt/arkime/etc"]
        );
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
