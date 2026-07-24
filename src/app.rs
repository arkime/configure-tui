//! The wizard Model + update logic and the terminal run loop.

use crate::actions::docker::{self, Images};
use crate::actions::native;
use crate::actions::system::RealOps;
use crate::config::substitute::BasicAuthEncoding;
use crate::domain::{
    plugins, Answers, BuildConfig, Component, Components, Deployment, MountSelection, Platform,
};
use crate::interfaces;
use crate::log::LogLine;
use crate::steps::{self, WizardStep};
use crate::ui;
use anyhow::Result;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind};
use ratatui::backend::Backend;
use ratatui::Terminal;
use tui_input::backend::crossterm::EventHandler;
use tui_input::Input;

/// Text fields the wizard edits. Kept as distinct buffers so moving between
/// screens preserves what was typed.
#[derive(Default)]
pub struct Fields {
    pub interface: Input,
    pub es_url: Input,
    pub es_user: Input,
    pub es_password: Input,
    pub s2s: Input,
    pub plugins: Input,
    pub wise_url: Input,
}

pub struct App {
    pub build: BuildConfig,
    pub platform: Platform,
    pub basic_auth: BasicAuthEncoding,

    pub deployment: Option<Deployment>,
    pub components: Components,
    pub answers: Answers,

    pub step: WizardStep,
    pub detected_interfaces: Vec<String>,

    pub fields: Fields,
    /// Cursor for select-style screens (deployment, components, interface list).
    pub cursor: usize,
    /// Sub-field focus for the multi-field Elasticsearch screen (0..=3).
    pub es_focus: usize,

    /// Per-detected-interface checkbox state, parallel to `detected_interfaces`.
    pub interface_checked: Vec<bool>,
    /// When true, the Interfaces screen shows a free-text field instead of the
    /// checkbox list (advanced mode).
    pub interface_advanced: bool,

    /// Per-plugin checkbox state, parallel to `plugins::KNOWN_PLUGINS`.
    pub plugin_checked: Vec<bool>,
    /// When true, the Plugins screen shows a free-text field.
    pub plugin_advanced: bool,

    /// Suggested docker bind-mount toggles.
    pub docker_mounts: MountSelection,

    /// Whether the process is running as root. Native apply needs it; docker
    /// does not.
    pub is_root: bool,

    pub log: Vec<LogLine>,
    pub applied: bool,
    pub error: Option<String>,
    pub should_quit: bool,
}

impl App {
    pub fn new(build: BuildConfig, platform: Platform) -> Self {
        let detected = interfaces::detect(platform.os);
        let mut fields = Fields::default();
        // Advanced-mode free-text prefill (bash's eth1 default when none found).
        let iface_default = if detected.is_empty() {
            "eth1".to_string()
        } else {
            detected.join(";")
        };
        fields.interface = Input::new(iface_default);
        fields.es_url = Input::new(Answers::DEFAULT_ES_URL.to_string());
        fields.wise_url = Input::new(Answers::DEFAULT_WISE_URL.to_string());

        // Pre-check the first detected interface (common single-NIC case). With
        // nothing detected there is nothing to check, so start in advanced mode.
        let mut interface_checked = vec![false; detected.len()];
        if let Some(first) = interface_checked.first_mut() {
            *first = true;
        }
        let interface_advanced = detected.is_empty();

        App {
            build,
            platform,
            basic_auth: BasicAuthEncoding::default(),
            deployment: None,
            components: Components::default(),
            answers: Answers {
                download_geoip: true,
                ..Default::default()
            },
            step: WizardStep::DeploymentSelect,
            detected_interfaces: detected,
            fields,
            cursor: 0,
            es_focus: 0,
            interface_checked,
            interface_advanced,
            plugin_checked: vec![false; plugins::KNOWN_PLUGINS.len()],
            plugin_advanced: false,
            docker_mounts: MountSelection::default(),
            is_root: crate::guards::is_root(),
            log: Vec::new(),
            applied: false,
            error: None,
            should_quit: false,
        }
    }

    fn advance(&mut self) {
        let w = self.wise_url_needed();
        self.step = steps::next(self.step, self.deployment, &self.components, w);
        self.on_enter_step();
    }

    fn retreat(&mut self) {
        let w = self.wise_url_needed();
        self.step = steps::prev(self.step, self.deployment, &self.components, w);
        self.on_enter_step();
    }

    /// The external WISE URL is only asked for when the wise.so plugin is
    /// enabled but the wise component is NOT being deployed locally.
    fn wise_url_needed(&self) -> bool {
        self.components.capture
            && !self.components.wise
            && self
                .answers
                .plugins
                .split(';')
                .any(|p| p.trim() == plugins::WISE_PLUGIN)
    }

    /// Reset per-screen transient cursor state when a screen becomes active.
    fn on_enter_step(&mut self) {
        self.error = None;
        match self.step {
            WizardStep::DeploymentSelect => {
                self.cursor = match self.deployment {
                    Some(Deployment::Docker) => 1,
                    _ => 0,
                };
            }
            WizardStep::ComponentsSelect => self.cursor = 0,
            WizardStep::Interfaces => self.cursor = 0,
            WizardStep::Elasticsearch => self.es_focus = 0,
            WizardStep::Plugins => {
                self.cursor = 0;
                // The wise component forces the wise plugin on.
                if self.components.wise {
                    if let Some(i) = plugins::KNOWN_PLUGINS
                        .iter()
                        .position(|&p| p == plugins::WISE_PLUGIN)
                    {
                        self.plugin_checked[i] = true;
                    }
                }
            }
            WizardStep::DockerMounts => self.cursor = 0,
            _ => {}
        }
    }

    /// Handle a key. Dispatches by the active step.
    pub fn on_key(&mut self, key: KeyEvent) {
        if key.kind != KeyEventKind::Press {
            return;
        }
        // Global quit.
        if matches!(key.code, KeyCode::Esc) && self.step != WizardStep::Progress {
            self.should_quit = true;
            return;
        }
        match self.step {
            WizardStep::DeploymentSelect => self.key_deployment(key),
            WizardStep::ComponentsSelect => self.key_components(key),
            WizardStep::Interfaces => self.key_interfaces(key),
            WizardStep::Elasticsearch => self.key_elasticsearch(key),
            WizardStep::S2sPassword => self.key_s2s(key),
            WizardStep::Plugins => self.key_plugins(key),
            WizardStep::WiseUrl => self.key_wise_url(key),
            WizardStep::DockerMounts => self.key_docker_mounts(key),
            WizardStep::GeoIp => self.key_geoip(key),
            WizardStep::Review => self.key_review(key),
            WizardStep::Progress => self.key_progress(key),
            WizardStep::Done => self.should_quit = true,
        }
    }

    fn key_deployment(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.cursor = 0,
            KeyCode::Down | KeyCode::Char('j') => self.cursor = 1,
            KeyCode::Enter => {
                self.deployment = Some(if self.cursor == 0 {
                    Deployment::Native
                } else {
                    Deployment::Docker
                });
                self.advance();
            }
            _ => {}
        }
    }

    fn key_components(&mut self, key: KeyEvent) {
        let n = Component::ALL.len();
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.cursor = (self.cursor + n - 1) % n;
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.cursor = (self.cursor + 1) % n;
            }
            KeyCode::Char(' ') => {
                self.components.toggle(Component::ALL[self.cursor]);
            }
            KeyCode::Enter => {
                if self.components.any() {
                    self.advance();
                } else {
                    self.error = Some("Select at least one component (space to toggle).".into());
                }
            }
            KeyCode::Left | KeyCode::Backspace => self.retreat(),
            _ => {}
        }
    }

    fn active_input(&mut self) -> &mut Input {
        match self.step {
            WizardStep::S2sPassword => &mut self.fields.s2s,
            WizardStep::Elasticsearch => match self.es_focus {
                0 => &mut self.fields.es_url,
                1 => &mut self.fields.es_user,
                _ => &mut self.fields.es_password,
            },
            _ => &mut self.fields.interface,
        }
    }

    /// Interfaces screen: a checkbox list of detected NICs, or an advanced
    /// free-text field toggled with Tab / 'a'.
    fn key_interfaces(&mut self, key: KeyEvent) {
        if self.interface_advanced {
            match key.code {
                KeyCode::Enter => self.commit_interfaces(),
                // Only offer "back to checkboxes" when there is something to show.
                KeyCode::Tab if !self.detected_interfaces.is_empty() => {
                    self.interface_advanced = false;
                }
                _ => {
                    self.fields.interface.handle_event(&Event::Key(key));
                }
            }
            return;
        }

        let n = self.detected_interfaces.len();
        match key.code {
            KeyCode::Up | KeyCode::Char('k') if n > 0 => {
                self.cursor = (self.cursor + n - 1) % n;
            }
            KeyCode::Down | KeyCode::Char('j') if n > 0 => {
                self.cursor = (self.cursor + 1) % n;
            }
            KeyCode::Char(' ') if n > 0 => {
                self.interface_checked[self.cursor] = !self.interface_checked[self.cursor];
            }
            KeyCode::Char('a') | KeyCode::Tab => {
                // Seed the advanced field with the current checkbox selection so
                // the user edits from where they are.
                self.fields.interface = Input::new(self.checked_interfaces().join(";"));
                self.interface_advanced = true;
            }
            KeyCode::Enter => self.commit_interfaces(),
            KeyCode::Left | KeyCode::Backspace => self.retreat(),
            _ => {}
        }
    }

    fn checked_interfaces(&self) -> Vec<String> {
        self.detected_interfaces
            .iter()
            .zip(&self.interface_checked)
            .filter(|(_, &checked)| checked)
            .map(|(name, _)| name.clone())
            .collect()
    }

    /// Resolve the interface answer from whichever mode is active and advance,
    /// or set an error if the selection is empty.
    fn commit_interfaces(&mut self) {
        let value = if self.interface_advanced {
            self.fields.interface.value().trim().to_string()
        } else {
            self.checked_interfaces().join(";")
        };
        if value.is_empty() {
            self.error = Some("Select an interface (space), or press 'a' to type manually.".into());
        } else {
            self.answers.interfaces = value;
            self.advance();
        }
    }

    /// Plugins screen: checkbox list of known plugins, or advanced free-text.
    /// The wise plugin is locked on whenever the wise component is enabled.
    fn key_plugins(&mut self, key: KeyEvent) {
        if self.plugin_advanced {
            match key.code {
                KeyCode::Enter => self.commit_plugins(),
                KeyCode::Tab => self.plugin_advanced = false,
                _ => {
                    self.fields.plugins.handle_event(&Event::Key(key));
                }
            }
            return;
        }

        let n = plugins::KNOWN_PLUGINS.len();
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.cursor = (self.cursor + n - 1) % n,
            KeyCode::Down | KeyCode::Char('j') => self.cursor = (self.cursor + 1) % n,
            KeyCode::Char(' ') => {
                let is_wise = plugins::KNOWN_PLUGINS[self.cursor] == plugins::WISE_PLUGIN;
                if is_wise && self.components.wise {
                    self.error = Some("wise.so is required by the wise component.".into());
                } else {
                    self.plugin_checked[self.cursor] = !self.plugin_checked[self.cursor];
                }
            }
            KeyCode::Char('a') | KeyCode::Tab => {
                self.fields.plugins = Input::new(self.checked_plugins().join(";"));
                self.plugin_advanced = true;
            }
            KeyCode::Enter => self.commit_plugins(),
            KeyCode::Left | KeyCode::Backspace => self.retreat(),
            _ => {}
        }
    }

    fn checked_plugins(&self) -> Vec<String> {
        plugins::KNOWN_PLUGINS
            .iter()
            .zip(&self.plugin_checked)
            .filter(|(_, &checked)| checked)
            .map(|(name, _)| name.to_string())
            .collect()
    }

    fn commit_plugins(&mut self) {
        let raw = if self.plugin_advanced {
            self.fields.plugins.value().to_string()
        } else {
            self.checked_plugins().join(";")
        };
        // finalize() forces wise.so on when the wise component is enabled.
        self.answers.plugins = plugins::finalize(&raw, self.components.wise);
        self.advance();
    }

    fn key_wise_url(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Enter => {
                self.answers.wise_url = self.fields.wise_url.value().trim().to_string();
                self.advance();
            }
            KeyCode::Left => self.retreat(),
            _ => {
                self.fields.wise_url.handle_event(&Event::Key(key));
            }
        }
    }

    /// Docker mounts screen: toggle the suggested host bind mounts relevant to
    /// the selected components.
    fn key_docker_mounts(&mut self, key: KeyEvent) {
        let relevant = MountSelection::relevant_kinds(&self.components);
        let n = relevant.len();
        match key.code {
            KeyCode::Up | KeyCode::Char('k') if n > 0 => self.cursor = (self.cursor + n - 1) % n,
            KeyCode::Down | KeyCode::Char('j') if n > 0 => self.cursor = (self.cursor + 1) % n,
            KeyCode::Char(' ') if n > 0 => self.docker_mounts.toggle(relevant[self.cursor]),
            KeyCode::Enter => self.advance(),
            KeyCode::Left | KeyCode::Backspace => self.retreat(),
            _ => {}
        }
    }

    fn key_elasticsearch(&mut self, key: KeyEvent) {
        // Fields: 0 url, 1 user, 2 password, 3 demo-es toggle.
        match key.code {
            KeyCode::Up => self.es_focus = self.es_focus.saturating_sub(1),
            KeyCode::Down | KeyCode::Tab => self.es_focus = (self.es_focus + 1).min(3),
            KeyCode::Left => self.retreat(),
            KeyCode::Char(' ') if self.es_focus == 3 => {
                self.answers.install_demo_es = !self.answers.install_demo_es;
            }
            KeyCode::Enter => {
                self.answers.elasticsearch = self.fields.es_url.value().trim().to_string();
                self.answers.es_user = self.fields.es_user.value().trim().to_string();
                self.answers.es_password = self.fields.es_password.value().to_string();
                self.advance();
            }
            _ if self.es_focus < 3 => {
                self.active_input().handle_event(&Event::Key(key));
            }
            _ => {}
        }
    }

    fn key_s2s(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Enter => {
                let v = self.fields.s2s.value().to_string();
                if v.is_empty() {
                    self.error = Some("Password required (it encrypts S2S and secrets).".into());
                } else if v.contains(' ') {
                    self.error = Some("Password must not contain spaces.".into());
                } else {
                    self.answers.s2s_password = v;
                    self.advance();
                }
            }
            KeyCode::Left => self.retreat(),
            _ => {
                self.active_input().handle_event(&Event::Key(key));
            }
        }
    }

    fn key_geoip(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => self.answers.download_geoip = true,
            KeyCode::Char('n') | KeyCode::Char('N') => self.answers.download_geoip = false,
            KeyCode::Char(' ') => self.answers.download_geoip = !self.answers.download_geoip,
            KeyCode::Enter => self.advance(),
            KeyCode::Left => self.retreat(),
            _ => {}
        }
    }

    fn key_review(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Enter => {
                self.step = WizardStep::Progress;
                self.run_apply();
            }
            KeyCode::Left | KeyCode::Backspace => self.retreat(),
            _ => {}
        }
    }

    fn key_progress(&mut self, key: KeyEvent) {
        if self.applied && matches!(key.code, KeyCode::Enter | KeyCode::Char('q') | KeyCode::Esc) {
            self.should_quit = true;
        }
    }

    /// Execute the side-effecting apply for the chosen deployment.
    fn run_apply(&mut self) {
        let ops = RealOps;
        self.log = match self.deployment {
            Some(Deployment::Docker) => {
                let generated = docker::generate(
                    &self.components,
                    &self.answers,
                    &self.docker_mounts,
                    &Images::default(),
                    self.basic_auth,
                );
                let out_dir = std::env::current_dir().unwrap_or_else(|_| ".".into());
                docker::apply(&ops, &out_dir, &generated)
            }
            // Native writes system config and manages services — needs root.
            _ if !self.is_root => vec![LogLine::new(
                crate::log::Level::Error,
                "Native setup must run as root. Re-run with sudo, or go back and \
                 choose Docker (which needs no root)."
                    .into(),
            )],
            _ => native::apply(
                &ops,
                &self.build,
                self.platform,
                &self.components,
                &self.answers,
                self.basic_auth,
            ),
        };
        self.applied = true;
    }
}

/// Run the TUI to completion on the given terminal.
pub fn run<B: Backend>(terminal: &mut Terminal<B>, mut app: App) -> Result<()> {
    app.on_enter_step();
    while !app.should_quit {
        terminal.draw(|f| ui::view(&app, f))?;
        if let Event::Key(key) = crossterm::event::read()? {
            app.on_key(key);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Os, ServiceManagerKind};
    use crossterm::event::{KeyCode, KeyModifiers};

    fn app() -> App {
        App::new(
            BuildConfig {
                name: "arkime".into(),
                install_dir: "/opt/arkime".into(),
            },
            Platform {
                os: Os::Linux,
                service_manager: ServiceManagerKind::Systemd,
            },
        )
    }

    fn press(app: &mut App, code: KeyCode) {
        app.on_key(KeyEvent::new(code, KeyModifiers::NONE));
    }

    fn typ(app: &mut App, s: &str) {
        for ch in s.chars() {
            press(app, KeyCode::Char(ch));
        }
    }

    #[test]
    fn native_capture_viewer_drives_to_review_with_answers() {
        let mut a = app();
        a.on_enter_step();
        // Deployment: Native (cursor 0) -> Enter.
        press(&mut a, KeyCode::Enter);
        assert_eq!(a.deployment, Some(Deployment::Native));
        assert_eq!(a.step, WizardStep::ComponentsSelect);

        // Components: toggle capture (cursor 0) + viewer (cursor 1).
        press(&mut a, KeyCode::Char(' ')); // capture
        press(&mut a, KeyCode::Down);
        press(&mut a, KeyCode::Char(' ')); // viewer
        press(&mut a, KeyCode::Enter);
        assert!(a.components.capture && a.components.viewer);
        assert_eq!(a.step, WizardStep::Interfaces);

        // Interfaces: detection is host-dependent, so set a known checkbox state.
        a.detected_interfaces = vec!["eth9".into(), "eth8".into()];
        a.interface_checked = vec![true, false];
        a.interface_advanced = false;
        a.cursor = 0;
        press(&mut a, KeyCode::Enter);
        assert_eq!(a.answers.interfaces, "eth9");
        assert_eq!(a.step, WizardStep::Elasticsearch);

        // Elasticsearch: keep default URL, add a user, then move to demo toggle.
        press(&mut a, KeyCode::Down); // -> user field
        typ(&mut a, "admin");
        press(&mut a, KeyCode::Enter);
        assert_eq!(a.answers.es_user, "admin");
        assert_eq!(a.step, WizardStep::S2sPassword);

        // Empty password is rejected.
        press(&mut a, KeyCode::Enter);
        assert_eq!(a.step, WizardStep::S2sPassword);
        assert!(a.error.is_some());
        typ(&mut a, "s3cret");
        press(&mut a, KeyCode::Enter);
        assert_eq!(a.answers.s2s_password, "s3cret");
        assert_eq!(a.step, WizardStep::Plugins);

        // Plugins: leave none selected, proceed.
        press(&mut a, KeyCode::Enter);
        assert_eq!(a.answers.plugins, "");
        assert_eq!(a.step, WizardStep::GeoIp);

        // GeoIP default is yes; proceed to Review.
        press(&mut a, KeyCode::Enter);
        assert_eq!(a.step, WizardStep::Review);
    }

    #[test]
    fn interface_checkboxes_toggle_and_join() {
        let mut a = app();
        a.detected_interfaces = vec!["eth0".into(), "eth1".into(), "eth2".into()];
        a.interface_checked = vec![false, false, false];
        a.interface_advanced = false;
        a.step = WizardStep::Interfaces;
        a.cursor = 0;

        press(&mut a, KeyCode::Char(' ')); // check eth0
        press(&mut a, KeyCode::Down);
        press(&mut a, KeyCode::Down);
        press(&mut a, KeyCode::Char(' ')); // check eth2
        press(&mut a, KeyCode::Enter);

        assert_eq!(a.answers.interfaces, "eth0;eth2");
    }

    #[test]
    fn interface_empty_selection_errors() {
        let mut a = app();
        a.detected_interfaces = vec!["eth0".into()];
        a.interface_checked = vec![false];
        a.interface_advanced = false;
        a.step = WizardStep::Interfaces;
        press(&mut a, KeyCode::Enter);
        assert_eq!(a.step, WizardStep::Interfaces);
        assert!(a.error.is_some());
    }

    #[test]
    fn interface_advanced_mode_lets_you_type() {
        let mut a = app();
        a.detected_interfaces = vec!["eth0".into()];
        a.interface_checked = vec![true];
        a.interface_advanced = false;
        a.step = WizardStep::Interfaces;

        press(&mut a, KeyCode::Char('a')); // switch to advanced
        assert!(a.interface_advanced);
        // Seeded from the checked selection, then append a second interface.
        typ(&mut a, ";bond0");
        press(&mut a, KeyCode::Enter);
        assert_eq!(a.answers.interfaces, "eth0;bond0");
    }

    #[test]
    fn components_requires_at_least_one() {
        let mut a = app();
        a.on_enter_step();
        press(&mut a, KeyCode::Enter); // deployment -> components
        press(&mut a, KeyCode::Enter); // no components selected
        assert_eq!(a.step, WizardStep::ComponentsSelect);
        assert!(a.error.is_some());
    }

    #[test]
    fn docker_wise_only_skips_interface_and_geoip_steps() {
        let mut a = app();
        a.on_enter_step();
        press(&mut a, KeyCode::Down); // deployment cursor -> Docker
        press(&mut a, KeyCode::Enter);
        assert_eq!(a.deployment, Some(Deployment::Docker));

        // Toggle wise (index 2).
        press(&mut a, KeyCode::Down);
        press(&mut a, KeyCode::Down);
        press(&mut a, KeyCode::Char(' '));
        press(&mut a, KeyCode::Enter);
        assert!(a.components.wise);
        // Wise needs no interfaces/ES/S2S/plugins, but docker still offers mounts.
        assert_eq!(a.step, WizardStep::DockerMounts);
        press(&mut a, KeyCode::Enter);
        assert_eq!(a.step, WizardStep::Review);
    }

    #[test]
    fn wise_component_forces_wise_plugin() {
        let mut a = app();
        a.components = Components {
            capture: true,
            wise: true,
            ..Default::default()
        };
        a.step = WizardStep::Plugins;
        a.on_enter_step();
        // wise.so pre-checked and locked; committing keeps it.
        a.commit_plugins();
        assert_eq!(a.answers.plugins, "wise.so");
    }

    #[test]
    fn plugin_checkboxes_join_selection() {
        let mut a = app();
        a.components = Components {
            capture: true,
            ..Default::default()
        };
        a.step = WizardStep::Plugins;
        a.on_enter_step();
        // Toggle ja4plus (index 1) and entropy (index 2).
        a.cursor = 1;
        press(&mut a, KeyCode::Char(' '));
        a.cursor = 2;
        press(&mut a, KeyCode::Char(' '));
        press(&mut a, KeyCode::Enter);
        assert_eq!(a.answers.plugins, "ja4plus.amd64.so;entropy.so");
    }

    #[test]
    fn wise_plugin_without_component_asks_for_wise_url() {
        let mut a = app();
        a.components = Components {
            capture: true,
            ..Default::default()
        };
        a.step = WizardStep::Plugins;
        a.on_enter_step();
        // Check wise.so (index 0) and proceed.
        a.cursor = 0;
        press(&mut a, KeyCode::Char(' '));
        press(&mut a, KeyCode::Enter);
        assert_eq!(a.answers.plugins, "wise.so");
        assert_eq!(a.step, WizardStep::WiseUrl);

        // Default is prefilled; accept it.
        press(&mut a, KeyCode::Enter);
        assert_eq!(a.answers.wise_url, Answers::DEFAULT_WISE_URL);
    }

    #[test]
    fn wise_component_does_not_ask_for_wise_url() {
        let mut a = app();
        a.components = Components {
            capture: true,
            wise: true,
            ..Default::default()
        };
        a.step = WizardStep::Plugins;
        a.on_enter_step();
        press(&mut a, KeyCode::Enter); // wise.so forced on
        assert_eq!(a.answers.plugins, "wise.so");
        // Local wise deployment -> no external URL step.
        assert_ne!(a.step, WizardStep::WiseUrl);
    }

    #[test]
    fn native_apply_without_root_is_blocked() {
        let mut a = app();
        a.deployment = Some(Deployment::Native);
        a.is_root = false;
        a.components = Components {
            capture: true,
            ..Default::default()
        };
        // Returns an error log and touches nothing (early-returns before RealOps).
        a.run_apply();
        assert!(a.applied);
        assert!(a
            .log
            .iter()
            .any(|l| l.level == crate::log::Level::Error && l.text.contains("root")));
    }

    #[test]
    fn docker_mounts_toggle_persists() {
        let mut a = app();
        a.deployment = Some(Deployment::Docker);
        a.components = Components {
            capture: true,
            ..Default::default()
        };
        a.step = WizardStep::DockerMounts;
        a.on_enter_step();
        // Toggle the first relevant mount (etc) off.
        press(&mut a, KeyCode::Char(' '));
        let etc = crate::domain::MountKind::Etc;
        assert!(!a.docker_mounts.is_enabled(etc));
    }
}
