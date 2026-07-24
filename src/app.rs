//! The wizard Model + update logic and the terminal run loop.

use crate::actions::docker::{self, Images};
use crate::actions::native;
use crate::actions::system::RealOps;
use crate::config::substitute::BasicAuthEncoding;
use crate::domain::{Answers, BuildConfig, Component, Components, Deployment, Platform};
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
    /// Cursor for select-style screens (deployment, components).
    pub cursor: usize,
    /// Sub-field focus for the multi-field Elasticsearch screen (0..=3).
    pub es_focus: usize,

    pub log: Vec<LogLine>,
    pub applied: bool,
    pub error: Option<String>,
    pub should_quit: bool,
}

impl App {
    pub fn new(build: BuildConfig, platform: Platform) -> Self {
        let detected = interfaces::detect(platform.os);
        let mut fields = Fields::default();
        // Prefill interface with detected list (or bash's eth1 default).
        let iface_default = if detected.is_empty() {
            "eth1".to_string()
        } else {
            detected.join(";")
        };
        fields.interface = Input::new(iface_default);
        fields.es_url = Input::new(Answers::DEFAULT_ES_URL.to_string());

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
            log: Vec::new(),
            applied: false,
            error: None,
            should_quit: false,
        }
    }

    fn advance(&mut self) {
        self.step = steps::next(self.step, self.deployment, &self.components);
        self.on_enter_step();
    }

    fn retreat(&mut self) {
        self.step = steps::prev(self.step, self.deployment, &self.components);
        self.on_enter_step();
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
            WizardStep::Elasticsearch => self.es_focus = 0,
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
            WizardStep::Interfaces => self.key_text(key, |a| a.step_from_interfaces()),
            WizardStep::Elasticsearch => self.key_elasticsearch(key),
            WizardStep::S2sPassword => self.key_s2s(key),
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

    /// Shared handler for single-text-field screens.
    fn key_text(&mut self, key: KeyEvent, on_enter: fn(&mut App)) {
        match key.code {
            KeyCode::Enter => on_enter(self),
            KeyCode::Backspace if self.field_is_empty_at_start() => self.retreat(),
            _ => {
                self.active_input().handle_event(&Event::Key(key));
            }
        }
    }

    fn field_is_empty_at_start(&self) -> bool {
        false // Backspace edits text; use Left to go back instead.
    }

    fn active_input(&mut self) -> &mut Input {
        match self.step {
            WizardStep::Interfaces => &mut self.fields.interface,
            WizardStep::S2sPassword => &mut self.fields.s2s,
            WizardStep::Elasticsearch => match self.es_focus {
                0 => &mut self.fields.es_url,
                1 => &mut self.fields.es_user,
                _ => &mut self.fields.es_password,
            },
            _ => &mut self.fields.interface,
        }
    }

    fn step_from_interfaces(&mut self) {
        self.answers.interfaces = self.fields.interface.value().trim().to_string();
        self.advance();
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
                    &Images::default(),
                    self.basic_auth,
                );
                let out_dir = std::env::current_dir().unwrap_or_else(|_| ".".into());
                docker::apply(&ops, &out_dir, &generated)
            }
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

        // Interfaces: clear prefill, type our own.
        a.fields.interface = Input::default();
        typ(&mut a, "eth9");
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
        assert_eq!(a.step, WizardStep::GeoIp);

        // GeoIP default is yes; proceed to Review.
        press(&mut a, KeyCode::Enter);
        assert_eq!(a.step, WizardStep::Review);
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
        // Wise needs neither interfaces, ES, nor S2S -> straight to Review.
        assert_eq!(a.step, WizardStep::Review);
    }
}
