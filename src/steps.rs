//! The wizard step sequence and its conditional branching.
//!
//! `required_steps` is the single source of truth: given the deployment,
//! component set, and platform, it produces the ordered list of steps to show.
//! `next`/`prev` simply walk that list, so a disabled component's questions
//! never appear and there is one place to reason about the flow.

use crate::domain::{Components, Deployment};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WizardStep {
    /// FIRST screen — docker/native × new/load. Everything branches on it.
    StartSelect,
    /// Path to the file being loaded (load modes only).
    LoadPath,
    /// Multi-select toggles for capture/viewer/wise/parliament/cont3xt.
    ComponentsSelect,
    /// Interfaces to monitor (capture only).
    Interfaces,
    /// ES URL/user/password + demo-ES toggle.
    Elasticsearch,
    /// S2S / encryption secret.
    S2sPassword,
    /// Viewer upload toggle (viewer only).
    ViewerUploads,
    /// Capture plugin selection (capture only).
    Plugins,
    /// External WISE URL (only when wise.so is enabled without the wise
    /// component).
    WiseUrl,
    /// GeoIP download prompt (native + capture only).
    GeoIp,
    /// Suggested host bind mounts (docker only).
    DockerMounts,
    /// Summary + confirm.
    Review,
    /// Applying actions, live log.
    Progress,
    /// Terminal state.
    Done,
}

/// Compute the ordered steps for the current selections.
///
/// `deployment` is `None` only before the first screen is answered; in that
/// case we still return the full skeleton so `next`/`prev` have something to
/// walk. Component-dependent steps are filtered once components are known.
pub fn required_steps(
    deployment: Option<Deployment>,
    is_load: bool,
    components: &Components,
    wise_url_needed: bool,
) -> Vec<WizardStep> {
    let mut steps = vec![WizardStep::StartSelect];
    if is_load {
        steps.push(WizardStep::LoadPath);
    }
    steps.push(WizardStep::ComponentsSelect);

    if components.needs_interfaces() {
        steps.push(WizardStep::Interfaces);
    }
    if components.needs_elasticsearch() {
        steps.push(WizardStep::Elasticsearch);
    }
    if components.needs_s2s_password() {
        steps.push(WizardStep::S2sPassword);
    }
    // Viewer-only: offer PCAP uploads.
    if components.viewer {
        steps.push(WizardStep::ViewerUploads);
    }
    // Plugins are loaded by capture, in both deployments.
    if components.capture {
        steps.push(WizardStep::Plugins);
    }
    // External WISE endpoint: wise.so enabled but no local wise component.
    if wise_url_needed {
        steps.push(WizardStep::WiseUrl);
    }
    // GeoIP is a native-only action, and only relevant when capturing.
    if deployment == Some(Deployment::Native) && components.capture {
        steps.push(WizardStep::GeoIp);
    }
    // Suggested bind mounts only apply to the docker deployment.
    if deployment == Some(Deployment::Docker) && components.any() {
        steps.push(WizardStep::DockerMounts);
    }

    steps.push(WizardStep::Review);
    steps.push(WizardStep::Progress);
    steps.push(WizardStep::Done);
    steps
}

/// Step after `current`, honoring the active selections. Returns `current` if it
/// is the last step (defensive; callers normally stop at `Done`).
pub fn next(
    current: WizardStep,
    deployment: Option<Deployment>,
    is_load: bool,
    components: &Components,
    wise_url_needed: bool,
) -> WizardStep {
    let steps = required_steps(deployment, is_load, components, wise_url_needed);
    match steps.iter().position(|&s| s == current) {
        Some(i) if i + 1 < steps.len() => steps[i + 1],
        // `current` may not be in the list if selections changed under it; fall
        // back to the first step at/after it, else stay put.
        _ => current,
    }
}

/// Step before `current`. Returns `current` if it is the first step.
pub fn prev(
    current: WizardStep,
    deployment: Option<Deployment>,
    is_load: bool,
    components: &Components,
    wise_url_needed: bool,
) -> WizardStep {
    let steps = required_steps(deployment, is_load, components, wise_url_needed);
    match steps.iter().position(|&s| s == current) {
        Some(i) if i > 0 => steps[i - 1],
        _ => current,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Components;

    fn caps() -> Components {
        Components {
            capture: true,
            viewer: true,
            ..Default::default()
        }
    }

    #[test]
    fn native_capture_viewer_full_flow() {
        let steps = required_steps(Some(Deployment::Native), false, &caps(), false);
        assert_eq!(
            steps,
            vec![
                WizardStep::StartSelect,
                WizardStep::ComponentsSelect,
                WizardStep::Interfaces,
                WizardStep::Elasticsearch,
                WizardStep::S2sPassword,
                WizardStep::ViewerUploads,
                WizardStep::Plugins,
                WizardStep::GeoIp,
                WizardStep::Review,
                WizardStep::Progress,
                WizardStep::Done,
            ]
        );
    }

    #[test]
    fn docker_skips_geoip() {
        let steps = required_steps(Some(Deployment::Docker), false, &caps(), false);
        assert!(!steps.contains(&WizardStep::GeoIp));
        assert!(steps.contains(&WizardStep::Interfaces));
        assert!(steps.contains(&WizardStep::DockerMounts));
        assert!(steps.contains(&WizardStep::Plugins));
    }

    #[test]
    fn wise_url_step_appears_only_when_needed() {
        // capture with wise.so but no wise component -> WiseUrl step.
        let cap = Components {
            capture: true,
            ..Default::default()
        };
        assert!(required_steps(Some(Deployment::Native), false, &cap, true)
            .contains(&WizardStep::WiseUrl));
        assert!(
            !required_steps(Some(Deployment::Native), false, &cap, false)
                .contains(&WizardStep::WiseUrl)
        );
    }

    #[test]
    fn wise_only_skips_interfaces_and_s2s_and_geoip() {
        let wise = Components {
            wise: true,
            ..Default::default()
        };
        let steps = required_steps(Some(Deployment::Native), false, &wise, false);
        assert!(!steps.contains(&WizardStep::Interfaces));
        assert!(!steps.contains(&WizardStep::S2sPassword));
        assert!(!steps.contains(&WizardStep::GeoIp));
        // Wise still needs no ES here (not in needs_elasticsearch set).
        assert!(!steps.contains(&WizardStep::Elasticsearch));
    }

    #[test]
    fn cont3xt_needs_es_and_s2s_but_not_interfaces() {
        let c = Components {
            cont3xt: true,
            ..Default::default()
        };
        let steps = required_steps(Some(Deployment::Native), false, &c, false);
        assert!(steps.contains(&WizardStep::Elasticsearch));
        assert!(steps.contains(&WizardStep::S2sPassword));
        assert!(!steps.contains(&WizardStep::Interfaces));
        assert!(!steps.contains(&WizardStep::GeoIp)); // no capture
    }

    #[test]
    fn next_and_prev_are_inverse_across_flow() {
        let d = Some(Deployment::Native);
        let c = caps();
        let steps = required_steps(d, false, &c, true);
        for win in steps.windows(2) {
            assert_eq!(next(win[0], d, false, &c, true), win[1]);
            assert_eq!(prev(win[1], d, false, &c, true), win[0]);
        }
        // Ends are clamped.
        assert_eq!(
            prev(WizardStep::StartSelect, d, false, &c, true),
            WizardStep::StartSelect
        );
        assert_eq!(next(WizardStep::Done, d, false, &c, true), WizardStep::Done);
    }

    #[test]
    fn every_flow_terminates_at_done_via_review() {
        for dep in [Deployment::Native, Deployment::Docker] {
            for mask in 1u8..32 {
                let c = Components {
                    capture: mask & 1 != 0,
                    viewer: mask & 2 != 0,
                    wise: mask & 4 != 0,
                    parliament: mask & 8 != 0,
                    cont3xt: mask & 16 != 0,
                };
                let steps = required_steps(Some(dep), false, &c, false);
                let review = steps.iter().position(|&s| s == WizardStep::Review).unwrap();
                let progress = steps
                    .iter()
                    .position(|&s| s == WizardStep::Progress)
                    .unwrap();
                let done = steps.iter().position(|&s| s == WizardStep::Done).unwrap();
                assert!(review < progress && progress < done);
                assert_eq!(*steps.last().unwrap(), WizardStep::Done);
            }
        }
    }
}
