//! The wizard Model + update logic and the terminal run loop.

use crate::actions::native;
use crate::actions::system::RealOps;
use crate::config::substitute::BasicAuthEncoding;
use crate::docset::{self, DocKind, Document, Images};
use crate::domain::{
    plugins, Answers, BuildConfig, Component, Components, Deployment, MountSelection, Platform,
    StartMode,
};
use crate::interfaces;
use crate::log::LogLine;
use crate::steps::{self, WizardStep};
use crate::ui;
use anyhow::Result;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::backend::Backend;
use ratatui::Terminal;
use std::path::PathBuf;
use tui_input::backend::crossterm::EventHandler;
use tui_input::Input;
use tui_textarea::TextArea;

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
    pub es_data: Input,
    pub load_path: Input,
    pub prefix: Input,
}

/// The full-file editor overlay: one text buffer per document, Tab-cycled.
pub struct Editor {
    pub tab: usize,
    pub areas: Vec<TextArea<'static>>,
    /// When true the active tab shows a diff (original vs current) instead of
    /// the editable buffer.
    pub diff: bool,
}

pub struct App {
    pub build: BuildConfig,
    pub platform: Platform,
    pub basic_auth: BasicAuthEncoding,

    pub start_mode: Option<StartMode>,
    pub deployment: Option<Deployment>,
    pub is_load: bool,
    pub components: Components,
    pub answers: Answers,

    /// In-memory output files (config.ini/…, or docker-compose.yml + arkime.env).
    /// The single source of truth for on-disk content; written only at apply.
    pub docs: Vec<Document>,
    /// Directory docker files are written to (native ini paths come from build).
    pub out_dir: PathBuf,
    /// Full-file editor overlay, when open.
    pub editor: Option<Editor>,

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
    /// Compose service-name prefix we manage (e.g. `arkime-`). Detected on load.
    pub service_prefix: String,
    /// All prefixes found in a loaded compose (dominant first) — the choices on
    /// the PrefixSelect screen when there is more than one.
    pub detected_prefixes: Vec<String>,
    /// Prefixes we are NOT managing (everything in `detected_prefixes` except the
    /// chosen one) — left untouched.
    pub other_prefixes: Vec<String>,
    /// True while typing a new prefix name on the PrefixSelect screen.
    pub prefix_adding: bool,

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
        fields.es_data = Input::new(Answers::DEFAULT_ES_DATA_DIR.to_string());

        // Pre-check the first detected interface (common single-NIC case). With
        // nothing detected there is nothing to check, so start in advanced mode.
        let mut interface_checked = vec![false; detected.len()];
        if let Some(first) = interface_checked.first_mut() {
            *first = true;
        }
        let interface_advanced = detected.is_empty();

        let out_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

        App {
            build,
            platform,
            basic_auth: BasicAuthEncoding::default(),
            start_mode: None,
            deployment: None,
            is_load: false,
            components: Components::default(),
            answers: Answers {
                download_geoip: true,
                ..Default::default()
            },
            docs: Vec::new(),
            out_dir,
            editor: None,
            step: WizardStep::StartSelect,
            detected_interfaces: detected,
            fields,
            cursor: 0,
            es_focus: 0,
            interface_checked,
            interface_advanced,
            plugin_checked: vec![false; plugins::KNOWN_PLUGINS.len()],
            plugin_advanced: false,
            docker_mounts: MountSelection::default(),
            service_prefix: docset::DEFAULT_PREFIX.to_string(),
            detected_prefixes: Vec::new(),
            other_prefixes: Vec::new(),
            prefix_adding: false,
            is_root: crate::guards::is_root(),
            log: Vec::new(),
            applied: false,
            error: None,
            should_quit: false,
        }
    }

    fn advance(&mut self) {
        let w = self.wise_url_needed();
        let sp = self.wants_prefix_step();
        self.step = steps::next(
            self.step,
            self.deployment,
            self.is_load,
            sp,
            &self.components,
            w,
        );
        self.sync_docs();
        self.on_enter_step();
    }

    fn retreat(&mut self) {
        let w = self.wise_url_needed();
        let sp = self.wants_prefix_step();
        self.step = steps::prev(
            self.step,
            self.deployment,
            self.is_load,
            sp,
            &self.components,
            w,
        );
        self.sync_docs();
        self.on_enter_step();
    }

    /// The prefix-management screen is shown for every docker flow (new or
    /// load), so you can choose / add / delete the service-name prefix set.
    fn wants_prefix_step(&self) -> bool {
        self.deployment == Some(Deployment::Docker)
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
            WizardStep::StartSelect => self.cursor = 0,
            WizardStep::LoadPath => {
                // Default load path per deployment.
                if self.fields.load_path.value().is_empty() {
                    let default = match self.deployment {
                        Some(Deployment::Docker) => "docker-compose.yml".to_string(),
                        _ => self.build.etc_dir().to_string_lossy().to_string(),
                    };
                    self.fields.load_path = Input::new(default);
                }
            }
            WizardStep::PrefixSelect => {
                self.prefix_adding = false;
                // New docker files start with just the default prefix to manage.
                if self.detected_prefixes.is_empty() {
                    self.detected_prefixes = vec![self.service_prefix.clone()];
                }
                // Highlight the currently-chosen prefix.
                self.cursor = self
                    .detected_prefixes
                    .iter()
                    .position(|p| *p == self.service_prefix)
                    .unwrap_or(0);
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
        // The editor overlay swallows all input while open.
        if self.editor.is_some() {
            self.editor_key(key);
            return;
        }
        // Ctrl+E opens the editor anywhere; plain 'e' on non-typing screens.
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let ctrl_e = key.code == KeyCode::Char('e') && ctrl;
        let plain_e = matches!(key.code, KeyCode::Char('e')) && !self.screen_is_text_input();
        if ctrl_e || plain_e {
            self.open_editor();
            return;
        }
        // Ctrl+D jumps straight to the diff view (e.g. from Review, to see what
        // will change before writing).
        if key.code == KeyCode::Char('d') && ctrl {
            self.open_editor();
            if let Some(ed) = &mut self.editor {
                ed.diff = true;
            }
            return;
        }
        // Esc cancels prefix-add mode; otherwise it goes back a screen.
        if key.code == KeyCode::Esc {
            if self.step == WizardStep::PrefixSelect && self.prefix_adding {
                self.prefix_adding = false;
            } else {
                self.back();
            }
            return;
        }
        // On non-typing screens, Left/Right also navigate (← back, → forward).
        // On typing screens they move the field cursor instead.
        if !self.screen_is_text_input() {
            match key.code {
                KeyCode::Left => {
                    self.back();
                    return;
                }
                KeyCode::Right => {
                    self.dispatch(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                    return;
                }
                _ => {}
            }
        }
        self.dispatch(key);
    }

    /// Go back one screen (or quit on the first screen / after apply).
    fn back(&mut self) {
        match self.step {
            WizardStep::StartSelect => self.should_quit = true,
            WizardStep::Progress | WizardStep::Done => {
                if self.applied {
                    self.should_quit = true;
                }
            }
            _ => self.retreat(),
        }
    }

    /// Route a key to the active screen's handler.
    fn dispatch(&mut self, key: KeyEvent) {
        match self.step {
            WizardStep::StartSelect => self.key_start(key),
            WizardStep::LoadPath => self.key_load_path(key),
            WizardStep::PrefixSelect => self.key_prefix_select(key),
            WizardStep::ComponentsSelect => self.key_components(key),
            WizardStep::Interfaces => self.key_interfaces(key),
            WizardStep::Elasticsearch => self.key_elasticsearch(key),
            WizardStep::S2sPassword => self.key_s2s(key),
            WizardStep::ViewerUploads => self.key_viewer_uploads(key),
            WizardStep::Plugins => self.key_plugins(key),
            WizardStep::WiseUrl => self.key_wise_url(key),
            WizardStep::DockerMounts => self.key_docker_mounts(key),
            WizardStep::GeoIp => self.key_geoip(key),
            WizardStep::Review => self.key_review(key),
            WizardStep::Progress => self.key_progress(key),
            WizardStep::Done => self.should_quit = true,
        }
    }

    /// Screens where typing occurs (so plain 'e' is a character, not the editor).
    fn screen_is_text_input(&self) -> bool {
        match self.step {
            WizardStep::LoadPath
            | WizardStep::Elasticsearch
            | WizardStep::S2sPassword
            | WizardStep::WiseUrl
            | WizardStep::DockerMounts => true,
            WizardStep::Interfaces => self.interface_advanced,
            WizardStep::Plugins => self.plugin_advanced,
            WizardStep::PrefixSelect => self.prefix_adding,
            _ => false,
        }
    }

    fn available_modes(&self) -> Vec<StartMode> {
        StartMode::available(self.platform.os)
    }

    fn key_start(&mut self, key: KeyEvent) {
        let modes = self.available_modes();
        let n = modes.len();
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.cursor = (self.cursor + n - 1) % n,
            KeyCode::Down | KeyCode::Char('j') => self.cursor = (self.cursor + 1) % n,
            KeyCode::Enter => {
                let mode = modes[self.cursor];
                self.start_mode = Some(mode);
                self.deployment = Some(mode.deployment());
                self.is_load = mode.is_load();
                self.advance();
            }
            _ => {}
        }
    }

    fn key_load_path(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Enter => match self.load_from_path() {
                Ok(()) => self.advance(),
                Err(e) => self.error = Some(format!("Load failed: {e}")),
            },
            _ => {
                self.fields.load_path.handle_event(&Event::Key(key));
            }
        }
    }

    /// Choose / add / delete which arkime prefix set to manage.
    fn key_prefix_select(&mut self, key: KeyEvent) {
        // Add mode: typing a new prefix name.
        if self.prefix_adding {
            match key.code {
                KeyCode::Enter => self.confirm_add_prefix(),
                _ => {
                    self.fields.prefix.handle_event(&Event::Key(key));
                }
            }
            return;
        }

        let n = self.detected_prefixes.len();
        match key.code {
            KeyCode::Up | KeyCode::Char('k') if n > 0 => self.cursor = (self.cursor + n - 1) % n,
            KeyCode::Down | KeyCode::Char('j') if n > 0 => self.cursor = (self.cursor + 1) % n,
            KeyCode::Char('a') => {
                self.fields.prefix = Input::default();
                self.prefix_adding = true;
            }
            KeyCode::Char('d') if n > 0 => self.delete_prefix(self.cursor),
            KeyCode::Enter if n > 0 => {
                self.select_prefix(self.cursor);
                self.reparse_for_prefix();
                self.advance();
            }
            _ => {}
        }
    }

    /// Make the prefix at `idx` the managed one and recompute the untouched set.
    fn select_prefix(&mut self, idx: usize) {
        self.service_prefix = self.detected_prefixes[idx].clone();
        self.other_prefixes = self
            .detected_prefixes
            .iter()
            .filter(|p| **p != self.service_prefix)
            .cloned()
            .collect();
    }

    /// Confirm the typed new prefix: add it (if new) and select it. A brand-new
    /// prefix has no services yet, so components start empty for it.
    fn confirm_add_prefix(&mut self) {
        self.prefix_adding = false;
        let p = self.fields.prefix.value().trim().to_string();
        if !self.detected_prefixes.contains(&p) {
            self.detected_prefixes.push(p.clone());
        }
        self.cursor = self
            .detected_prefixes
            .iter()
            .position(|x| *x == p)
            .unwrap_or(0);
        self.select_prefix(self.cursor);
        self.components = Components::default();
        self.sync_fields_from_answers();
    }

    /// Delete a prefix set: strip its services from the compose and drop it from
    /// the list. Keeps at least the default prefix around.
    fn delete_prefix(&mut self, idx: usize) {
        let removed = self.detected_prefixes.remove(idx);
        for d in self.docs.iter_mut().filter(|d| d.kind == DocKind::Compose) {
            d.text = docset::remove_prefix_services(&d.text, &removed);
        }
        if self.detected_prefixes.is_empty() {
            self.detected_prefixes
                .push(docset::DEFAULT_PREFIX.to_string());
        }
        self.cursor = self.cursor.min(self.detected_prefixes.len() - 1);
        if self.service_prefix == removed {
            self.select_prefix(self.cursor);
            self.reparse_for_prefix();
        }
    }

    /// Re-read the loaded compose under the chosen prefix so components, mounts
    /// and ES reflect that deployment's services.
    fn reparse_for_prefix(&mut self) {
        let (text, prefix) = match self.docs.iter().find(|d| d.kind == DocKind::Compose) {
            Some(d) => (d.text.clone(), self.service_prefix.clone()),
            None => return,
        };
        self.components = Components::default();
        self.docker_mounts = MountSelection::default();
        docset::parse_compose(
            &text,
            &prefix,
            &mut self.components,
            &mut self.answers,
            &mut self.docker_mounts,
        );
        self.sync_fields_from_answers();
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
                2 => &mut self.fields.es_password,
                _ => &mut self.fields.es_data, // field 4
            },
            _ => &mut self.fields.interface,
        }
    }

    /// Whether the ES screen shows the extra "data dir" field (docker + we run
    /// the single-node ES).
    fn es_shows_data_dir(&self) -> bool {
        self.deployment == Some(Deployment::Docker) && self.answers.install_demo_es
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
            _ => {
                self.fields.wise_url.handle_event(&Event::Key(key));
            }
        }
    }

    /// Docker mounts screen: toggle the suggested host bind mounts relevant to
    /// the selected components.
    fn key_docker_mounts(&mut self, key: KeyEvent) {
        // Rows are editable host paths, so ↑↓ (not j/k) move between them and
        // typed characters edit the focused mount's host path.
        let relevant = MountSelection::relevant_kinds(&self.components);
        let n = relevant.len();
        if n == 0 {
            if key.code == KeyCode::Enter {
                self.advance();
            }
            return;
        }
        let kind = relevant[self.cursor.min(n - 1)];
        match key.code {
            KeyCode::Up => self.cursor = (self.cursor + n - 1) % n,
            KeyCode::Down => self.cursor = (self.cursor + 1) % n,
            KeyCode::Char(' ') => self.docker_mounts.toggle(kind),
            KeyCode::Char(c) => self.docker_mounts.host_mut(kind).push(c),
            KeyCode::Backspace => {
                self.docker_mounts.host_mut(kind).pop();
            }
            KeyCode::Enter => self.advance(),
            _ => {}
        }
    }

    fn key_elasticsearch(&mut self, key: KeyEvent) {
        // Fields: 0 url, 1 user, 2 password, 3 single-node toggle, 4 data dir
        // (only shown in docker when the single-node ES is on).
        let max_focus = if self.es_shows_data_dir() { 4 } else { 3 };
        match key.code {
            KeyCode::Up => self.es_focus = self.es_focus.saturating_sub(1),
            KeyCode::Down | KeyCode::Tab => self.es_focus = (self.es_focus + 1).min(max_focus),
            KeyCode::Char(' ') if self.es_focus == 3 => {
                self.answers.install_demo_es = !self.answers.install_demo_es;
                if !self.es_shows_data_dir() && self.es_focus > 3 {
                    self.es_focus = 3;
                }
            }
            KeyCode::Enter => {
                self.answers.elasticsearch = self.fields.es_url.value().trim().to_string();
                self.answers.es_user = self.fields.es_user.value().trim().to_string();
                self.answers.es_password = self.fields.es_password.value().to_string();
                self.answers.es_data_dir = self.fields.es_data.value().trim().to_string();
                self.advance();
            }
            // Fields 0/1/2/4 are text; 3 is the toggle.
            _ if self.es_focus != 3 => {
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
            _ => {
                self.active_input().handle_event(&Event::Key(key));
            }
        }
    }

    fn key_geoip(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char(' ') => self.answers.download_geoip = !self.answers.download_geoip,
            KeyCode::Enter => self.advance(),
            _ => {}
        }
    }

    fn key_viewer_uploads(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char(' ') => self.answers.enable_uploads = !self.answers.enable_uploads,
            KeyCode::Enter => self.advance(),
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

    // --- documents -------------------------------------------------------

    /// Reconcile the document set to the current deployment/components and
    /// re-merge understood fields into each (preserving unknown content).
    fn sync_docs(&mut self) {
        let dep = match self.deployment {
            Some(d) => d,
            None => return,
        };
        let desired = self.desired_doc_kinds(dep);

        // New mode: drop docs no longer wanted. Load mode: keep loaded docs.
        if !self.is_load {
            self.docs.retain(|d| desired.contains(&d.kind));
        }
        for kind in &desired {
            if !self.docs.iter().any(|d| d.kind == *kind) {
                let (path, base) = self.new_doc(*kind);
                self.docs.push(Document {
                    kind: *kind,
                    path,
                    original: base.clone(),
                    text: base,
                });
            }
        }

        // Re-merge understood fields (locals avoid borrowing self twice).
        let answers = self.answers.clone();
        let components = self.components;
        let mounts = self.docker_mounts.clone();
        let ba = self.basic_auth;
        let prefix = self.service_prefix.clone();
        let images = Images::default();
        for d in &mut self.docs {
            d.text = match d.kind {
                DocKind::Compose => docset::render_compose(
                    &d.text,
                    &prefix,
                    &components,
                    &answers,
                    &mounts,
                    &images,
                ),
                DocKind::Env => docset::render_env(&d.text, &answers, ba),
                ini => docset::render_ini(ini, &d.text, &answers, ba),
            };
        }
    }

    fn desired_doc_kinds(&self, dep: Deployment) -> Vec<DocKind> {
        match dep {
            Deployment::Docker => vec![DocKind::Compose, DocKind::Env],
            Deployment::Native => {
                let mut v = Vec::new();
                if self.components.capture || self.components.viewer {
                    v.push(DocKind::ConfigIni);
                }
                if self.components.wise {
                    v.push(DocKind::WiseIni);
                }
                if self.components.cont3xt {
                    v.push(DocKind::Cont3xtIni);
                }
                v
            }
        }
    }

    fn new_doc(&self, kind: DocKind) -> (PathBuf, String) {
        match kind {
            DocKind::Compose | DocKind::Env => (self.out_dir.join(kind.filename()), String::new()),
            ini => {
                let etc = self.build.etc_dir();
                let install = self.build.install_dir.to_string_lossy();
                let base = docset::fresh_ini_base(ini, &etc, &install);
                (etc.join(kind.filename()), base)
            }
        }
    }

    // --- load ------------------------------------------------------------

    /// The directory containing `path`, resolving a bare filename (no directory
    /// component) to the current working directory rather than an empty path.
    fn dir_of(path: &std::path::Path) -> PathBuf {
        match path.parent() {
            Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
            _ => std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        }
    }

    /// Read the file(s) at the load path into documents and prefill the wizard.
    fn load_from_path(&mut self) -> std::io::Result<()> {
        let raw = self.fields.load_path.value().trim().to_string();
        let path = PathBuf::from(&raw);
        self.docs.clear();

        match self.deployment {
            Some(Deployment::Docker) => {
                let text = std::fs::read_to_string(&path)?;
                self.out_dir = Self::dir_of(&path);
                // Detect the service prefix (arkime-/none/arkime6-/…) and manage
                // only that set; other prefixes are preserved untouched.
                match docset::detect_prefix(&text) {
                    Some(det) => {
                        // Dominant first, then the others — the PrefixSelect list.
                        let mut all = vec![det.prefix.clone()];
                        all.extend(det.others.clone());
                        self.detected_prefixes = all;
                        self.service_prefix = det.prefix;
                        self.other_prefixes = det.others;
                    }
                    None => {
                        self.detected_prefixes.clear();
                        self.service_prefix = docset::DEFAULT_PREFIX.to_string();
                        self.other_prefixes.clear();
                    }
                }
                let prefix = self.service_prefix.clone();
                docset::parse_compose(
                    &text,
                    &prefix,
                    &mut self.components,
                    &mut self.answers,
                    &mut self.docker_mounts,
                );
                // Write back to an absolute path under out_dir.
                let compose_name = path
                    .file_name()
                    .map(PathBuf::from)
                    .unwrap_or_else(|| PathBuf::from(DocKind::Compose.filename()));
                self.docs.push(Document {
                    kind: DocKind::Compose,
                    path: self.out_dir.join(compose_name),
                    original: text.clone(),
                    text,
                });
                // Sibling env file, if present.
                let env_path = self.out_dir.join(DocKind::Env.filename());
                let env_text = std::fs::read_to_string(&env_path).unwrap_or_default();
                docset::parse_env(&env_text, &mut self.answers);
                self.docs.push(Document {
                    kind: DocKind::Env,
                    path: env_path,
                    original: env_text.clone(),
                    text: env_text,
                });
            }
            _ => {
                // Native: treat the path as the etc dir (or a file's dir).
                let dir = if path.is_dir() {
                    path.clone()
                } else {
                    Self::dir_of(&path)
                };
                let mut any = false;
                for (kind, comp) in [
                    (DocKind::ConfigIni, None),
                    (DocKind::WiseIni, Some(Component::Wise)),
                    (DocKind::Cont3xtIni, Some(Component::Cont3xt)),
                ] {
                    let p = dir.join(kind.filename());
                    if let Ok(text) = std::fs::read_to_string(&p) {
                        any = true;
                        docset::parse_ini(kind, &text, &mut self.answers);
                        if kind == DocKind::ConfigIni {
                            self.components.capture = true;
                            self.components.viewer = true;
                        }
                        if let Some(c) = comp {
                            if !self.components.contains(c) {
                                self.components.toggle(c);
                            }
                        }
                        self.docs.push(Document {
                            kind,
                            path: p,
                            original: text.clone(),
                            text,
                        });
                    }
                }
                if !any {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::NotFound,
                        format!("no ini files found in {}", dir.display()),
                    ));
                }
            }
        }
        self.sync_fields_from_answers();
        Ok(())
    }

    /// Push understood answer values back into the editable input fields and
    /// checkbox state (after a load or an editor edit).
    fn sync_fields_from_answers(&mut self) {
        self.fields.es_url = Input::new(self.answers.elasticsearch_or_default().to_string());
        self.fields.es_user = Input::new(self.answers.es_user.clone());
        self.fields.es_password = Input::new(self.answers.es_password.clone());
        self.fields.s2s = Input::new(self.answers.s2s_password.clone());
        self.fields.plugins = Input::new(self.answers.plugins.clone());
        if !self.answers.es_data_dir.is_empty() {
            self.fields.es_data = Input::new(self.answers.es_data_dir.clone());
        }
        if !self.answers.wise_url.is_empty() {
            self.fields.wise_url = Input::new(self.answers.wise_url.clone());
        }
        self.fields.interface = Input::new(self.answers.interfaces.clone());

        // Interface checkboxes: check detected NICs present in the answer; if the
        // answer names something we didn't detect, fall back to advanced.
        let wanted: Vec<&str> = self.answers.interfaces.split(';').map(str::trim).collect();
        for (i, name) in self.detected_interfaces.iter().enumerate() {
            self.interface_checked[i] = wanted.contains(&name.as_str());
        }
        if !self.answers.interfaces.is_empty()
            && wanted
                .iter()
                .any(|w| !w.is_empty() && !self.detected_interfaces.iter().any(|d| d == w))
        {
            self.interface_advanced = true;
        }

        // Plugin checkboxes similarly.
        let plugs: Vec<&str> = self.answers.plugins.split(';').map(str::trim).collect();
        for (i, name) in plugins::KNOWN_PLUGINS.iter().enumerate() {
            self.plugin_checked[i] = plugs.contains(name);
        }
        if !self.answers.plugins.is_empty()
            && plugs
                .iter()
                .any(|p| !p.is_empty() && !plugins::KNOWN_PLUGINS.contains(p))
        {
            self.plugin_advanced = true;
        }
    }

    // --- editor ----------------------------------------------------------

    pub fn open_editor(&mut self) {
        self.sync_docs();
        if self.docs.is_empty() {
            self.error = Some("No files to edit yet.".into());
            return;
        }
        let areas = self
            .docs
            .iter()
            .map(|d| TextArea::from(d.text.lines().collect::<Vec<_>>()))
            .collect();
        self.editor = Some(Editor {
            tab: 0,
            areas,
            diff: false,
        });
    }

    fn editor_key(&mut self, key: KeyEvent) {
        let ed = self.editor.as_mut().unwrap();
        let n = ed.areas.len();
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let ctrl_e = key.code == KeyCode::Char('e') && ctrl;
        let ctrl_d = key.code == KeyCode::Char('d') && ctrl;
        match key.code {
            KeyCode::Esc => self.close_editor(),
            _ if ctrl_e => self.close_editor(),
            _ if ctrl_d => ed.diff = !ed.diff,
            KeyCode::Tab => ed.tab = (ed.tab + 1) % n,
            KeyCode::BackTab => ed.tab = (ed.tab + n - 1) % n,
            // The diff view is read-only.
            _ if ed.diff => {}
            _ => {
                ed.areas[ed.tab].input(key);
            }
        }
    }

    /// Commit editor buffers back into the documents, then re-parse understood
    /// fields into the wizard (two-way sync, last write wins).
    fn close_editor(&mut self) {
        if let Some(ed) = self.editor.take() {
            for (d, area) in self.docs.iter_mut().zip(ed.areas.iter()) {
                d.text = area.lines().join("\n");
                if !d.text.ends_with('\n') {
                    d.text.push('\n');
                }
            }
            // Parse understood fields back out of the (possibly edited) docs.
            let mut components = self.components;
            let prefix = self.service_prefix.clone();
            for d in &self.docs {
                match d.kind {
                    DocKind::Compose => docset::parse_compose(
                        &d.text,
                        &prefix,
                        &mut components,
                        &mut self.answers,
                        &mut self.docker_mounts,
                    ),
                    DocKind::Env => docset::parse_env(&d.text, &mut self.answers),
                    ini => docset::parse_ini(ini, &d.text, &mut self.answers),
                }
            }
            self.components = components;
            self.sync_fields_from_answers();
            self.sync_docs();
        }
    }

    // --- apply -----------------------------------------------------------

    /// Write the in-memory documents to disk and run native system actions.
    fn run_apply(&mut self) {
        self.sync_docs();
        let ops = RealOps;
        let mut log = Vec::new();

        // Native writes system config + manages services — needs root.
        if self.deployment == Some(Deployment::Native) && !self.is_root {
            self.log = vec![LogLine::new(
                crate::log::Level::Error,
                "Native setup must run as root. Re-run with sudo, or go back and \
                 choose Docker (which needs no root)."
                    .into(),
            )];
            self.applied = true;
            return;
        }

        // Write every document. Load mode overwrites (backing up first); new
        // mode won't clobber an existing file.
        use crate::actions::system::SystemOps;
        for d in &self.docs {
            let mode = if matches!(d.kind, DocKind::Env) {
                0o600
            } else {
                0o644
            };
            let outcome = if self.is_load {
                // Snapshot the existing file before overwriting it.
                if let Ok(Some(bak)) = ops.backup(&d.path) {
                    log.push(LogLine::new(
                        crate::log::Level::Info,
                        format!("Backed up to {}", bak.display()),
                    ));
                }
                ops.write_file(&d.path, &d.text, mode).map(|_| false)
            } else {
                ops.write_new(&d.path, &d.text)
            };
            match outcome {
                Ok(true) => log.push(LogLine::new(
                    crate::log::Level::Info,
                    format!("Kept existing {}", d.path.display()),
                )),
                Ok(false) => log.push(LogLine::new(
                    crate::log::Level::Info,
                    format!("Wrote {}", d.path.display()),
                )),
                Err(e) => log.push(LogLine::new(
                    crate::log::Level::Error,
                    format!("writing {}: {e}", d.path.display()),
                )),
            }
        }

        match self.deployment {
            Some(Deployment::Docker) => {
                log.push(LogLine::new(
                    crate::log::Level::Info,
                    "Files written. Nothing is running yet — start Arkime with:".into(),
                ));
                log.push(LogLine::new(
                    crate::log::Level::Info,
                    format!("    cd {} && docker compose up -d", self.out_dir.display()),
                ));
            }
            _ => {
                native::system_actions(
                    &ops,
                    &self.build,
                    self.platform,
                    &self.components,
                    &self.answers,
                    &mut log,
                );
            }
        }

        self.log = log;
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
        // Start: "Run on machine — create new" (index 2 on Linux) -> Enter.
        a.cursor = 2;
        press(&mut a, KeyCode::Enter);
        assert_eq!(a.deployment, Some(Deployment::Native));
        assert!(!a.is_load);
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
        assert_eq!(a.step, WizardStep::ViewerUploads);

        // Viewer uploads: enable (space), then proceed.
        press(&mut a, KeyCode::Char(' '));
        press(&mut a, KeyCode::Enter);
        assert!(a.answers.enable_uploads);
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
    fn arrows_navigate_on_non_typing_screens() {
        let mut a = app();
        a.on_enter_step();
        a.cursor = 0; // Docker — create new
                      // Right = forward (same as Enter).
        press(&mut a, KeyCode::Right);
        assert_eq!(a.deployment, Some(Deployment::Docker));
        assert_eq!(a.step, WizardStep::PrefixSelect);
        // Left = back.
        press(&mut a, KeyCode::Left);
        assert_eq!(a.step, WizardStep::StartSelect);
    }

    #[test]
    fn arrows_do_not_navigate_while_typing() {
        let mut a = app();
        a.step = WizardStep::S2sPassword;
        typ(&mut a, "abc");
        // Left moves the field cursor, it does NOT go back a screen.
        press(&mut a, KeyCode::Left);
        assert_eq!(a.step, WizardStep::S2sPassword);
    }

    #[test]
    fn components_requires_at_least_one() {
        let mut a = app();
        a.on_enter_step();
        press(&mut a, KeyCode::Enter); // start (Docker new) -> prefix
        press(&mut a, KeyCode::Enter); // accept default prefix -> components
        press(&mut a, KeyCode::Enter); // no components selected
        assert_eq!(a.step, WizardStep::ComponentsSelect);
        assert!(a.error.is_some());
    }

    #[test]
    fn docker_wise_only_skips_interface_and_geoip_steps() {
        let mut a = app();
        a.on_enter_step();
        a.cursor = 0; // Docker — create new
        press(&mut a, KeyCode::Enter);
        assert_eq!(a.deployment, Some(Deployment::Docker));
        // Docker shows the prefix screen first; accept the default.
        assert_eq!(a.step, WizardStep::PrefixSelect);
        press(&mut a, KeyCode::Enter);
        assert_eq!(a.step, WizardStep::ComponentsSelect);

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

    #[test]
    fn docker_mount_host_is_editable_and_reaches_compose() {
        let mut a = app();
        a.deployment = Some(Deployment::Docker);
        a.components = Components {
            capture: true,
            ..Default::default()
        };
        a.step = WizardStep::DockerMounts;
        a.on_enter_step(); // cursor 0 = etc
        press(&mut a, KeyCode::Down); // -> raw (Pcap)
        typ(&mut a, "-x"); // append to the raw host path
        assert_eq!(
            a.docker_mounts.host(crate::domain::MountKind::Pcap),
            "/arkime/raw-x"
        );

        a.sync_docs();
        let compose = a.docs.iter().find(|d| d.kind == DocKind::Compose).unwrap();
        assert!(compose.text.contains("/arkime/raw-x:/opt/arkime/raw"));
    }

    #[test]
    fn editor_edit_syncs_back_into_wizard() {
        let mut a = app();
        a.deployment = Some(Deployment::Docker);
        a.components = Components {
            capture: true,
            ..Default::default()
        };
        a.open_editor();
        let env_idx = a.docs.iter().position(|d| d.kind == DocKind::Env).unwrap();
        // Hand-edit the env file in the editor.
        a.editor.as_mut().unwrap().areas[env_idx] = TextArea::new(vec![
            "ARKIME__interface=zzz9".to_string(),
            "ARKIME__passwordSecret=fromedit".to_string(),
            "MY_UNKNOWN=keep".to_string(),
        ]);
        a.close_editor();

        // Edits flowed back into the wizard...
        assert_eq!(a.answers.interfaces, "zzz9");
        assert_eq!(a.answers.s2s_password, "fromedit");
        // ...and the unknown var is preserved in the re-synced document.
        let env = a.docs.iter().find(|d| d.kind == DocKind::Env).unwrap();
        assert!(env.text.contains("MY_UNKNOWN=keep"));
    }

    #[test]
    fn multi_prefix_compose_prompts_and_reparses() {
        let dir = tempfile::tempdir().unwrap();
        let compose = "services:\n  arkime1-viewer:\n    image: arkime/arkime\n  arkime2-capture:\n    image: arkime/arkime\n  wise:\n    image: arkime/arkime\n";
        let path = dir.path().join("docker-compose.yml");
        std::fs::write(&path, compose).unwrap();

        let mut a = app();
        a.deployment = Some(Deployment::Docker);
        a.is_load = true;
        a.step = WizardStep::LoadPath;
        a.fields.load_path = Input::new(path.to_string_lossy().to_string());
        a.load_from_path().unwrap();

        // Three prefixes detected: "", arkime1-, arkime2-.
        assert_eq!(a.detected_prefixes.len(), 3);
        assert!(a.wants_prefix_step());

        // Advancing from LoadPath lands on the prefix chooser.
        a.advance();
        assert_eq!(a.step, WizardStep::PrefixSelect);

        // Choose arkime2- and confirm the component set re-parses to its services.
        a.cursor = a
            .detected_prefixes
            .iter()
            .position(|p| p == "arkime2-")
            .unwrap();
        press(&mut a, KeyCode::Enter);
        assert_eq!(a.service_prefix, "arkime2-");
        assert!(a.components.capture);
        assert!(!a.components.viewer);
        assert!(!a.components.wise);
        assert_eq!(a.step, WizardStep::ComponentsSelect);
    }

    #[test]
    fn prefix_add_and_delete() {
        let mut a = app();
        a.deployment = Some(Deployment::Docker);
        a.step = WizardStep::PrefixSelect;
        a.on_enter_step();
        assert_eq!(a.detected_prefixes, vec!["arkime-".to_string()]);

        // Add a new prefix.
        press(&mut a, KeyCode::Char('a'));
        assert!(a.prefix_adding);
        typ(&mut a, "arkime2-");
        press(&mut a, KeyCode::Enter);
        assert!(!a.prefix_adding);
        assert!(a.detected_prefixes.contains(&"arkime2-".to_string()));
        assert_eq!(a.service_prefix, "arkime2-");

        // Delete it again.
        a.cursor = a
            .detected_prefixes
            .iter()
            .position(|p| p == "arkime2-")
            .unwrap();
        press(&mut a, KeyCode::Char('d'));
        assert!(!a.detected_prefixes.contains(&"arkime2-".to_string()));
        assert_eq!(a.detected_prefixes, vec!["arkime-".to_string()]);
    }

    #[test]
    fn dir_of_resolves_bare_filename_to_cwd() {
        // A bare filename has an empty parent — resolve it to an absolute CWD,
        // not an empty path (which produced a blank "cd  && docker compose up").
        let d = App::dir_of(std::path::Path::new("docker-compose.yml"));
        assert!(!d.as_os_str().is_empty());
        assert!(d.is_absolute());
        // A real directory component is kept as-is.
        assert_eq!(
            App::dir_of(std::path::Path::new("/srv/arkime/docker-compose.yml")),
            std::path::Path::new("/srv/arkime")
        );
    }

    #[test]
    fn native_load_prefills_and_preserves_unknown() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("config.ini"),
            "interface=eth7\nelasticsearch=https://loaded:9200\npasswordSecret=loadedpw\ncustomKey=keepme\n",
        )
        .unwrap();

        let mut a = app();
        a.deployment = Some(Deployment::Native);
        a.is_load = true;
        a.fields.load_path = Input::new(dir.path().to_string_lossy().to_string());
        a.load_from_path().unwrap();

        // Prefilled from the file.
        assert!(a.components.capture && a.components.viewer);
        assert_eq!(a.answers.interfaces, "eth7");
        assert_eq!(a.answers.elasticsearch, "https://loaded:9200");
        assert_eq!(a.answers.s2s_password, "loadedpw");

        // Changing a value and re-syncing preserves the unknown key.
        a.answers.interfaces = "eth0;eth1".into();
        a.sync_docs();
        let cfg = a
            .docs
            .iter()
            .find(|d| d.kind == DocKind::ConfigIni)
            .unwrap();
        assert!(cfg.text.contains("customKey=keepme"));
        assert!(cfg.text.contains("interface=eth0;eth1"));
    }
}
