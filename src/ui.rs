//! Rendering. `view` dispatches by the active wizard step; each screen is a
//! small render helper. No state is mutated here.

use crate::app::App;
use crate::domain::{plugins, Component, Deployment, MountSelection};
use crate::log::Level;
use crate::steps::WizardStep;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Frame;

const ACCENT: Color = Color::Cyan;

pub fn view(app: &App, f: &mut Frame) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // header
            Constraint::Min(5),    // body
            Constraint::Length(3), // footer
        ])
        .split(f.area());

    render_header(app, f, chunks[0]);
    render_body(app, f, chunks[1]);
    render_footer(app, f, chunks[2]);

    if app.editor.is_some() {
        render_editor(app, f);
    }
}

fn render_header(app: &App, f: &mut Frame, area: Rect) {
    let dep = match app.deployment {
        Some(Deployment::Native) => "native",
        Some(Deployment::Docker) => "docker",
        None => "-",
    };
    let title = Line::from(vec![
        Span::styled(
            " Arkime Setup ",
            Style::default()
                .fg(Color::Black)
                .bg(ACCENT)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!("  {}", step_title(app.step))),
        Span::styled(
            format!("   [{dep} · {:?}]", app.platform.os),
            Style::default().fg(Color::DarkGray),
        ),
    ]);
    f.render_widget(
        Paragraph::new(title).block(Block::default().borders(Borders::BOTTOM)),
        area,
    );
}

fn step_title(step: WizardStep) -> &'static str {
    match step {
        WizardStep::StartSelect => "Start",
        WizardStep::LoadPath => "Load file",
        WizardStep::PrefixSelect => "Service prefix",
        WizardStep::ComponentsSelect => "Components",
        WizardStep::Interfaces => "Interfaces",
        WizardStep::Elasticsearch => "OpenSearch / Elasticsearch",
        WizardStep::S2sPassword => "Encryption password",
        WizardStep::ViewerUploads => "Viewer uploads",
        WizardStep::ViewerPlugins => "Viewer plugins",
        WizardStep::Plugins => "Capture plugins",
        WizardStep::WiseUrl => "WISE URL",
        WizardStep::DockerMounts => "Docker mounts",
        WizardStep::GeoIp => "GeoIP",
        WizardStep::AdminSetup => "Database & admin",
        WizardStep::Review => "Review",
        WizardStep::Progress => "Applying",
        WizardStep::Done => "Done",
    }
}

fn render_body(app: &App, f: &mut Frame, area: Rect) {
    let block = Block::default().borders(Borders::NONE);
    let inner = block.inner(area);
    f.render_widget(block, area);
    match app.step {
        WizardStep::StartSelect => render_start(app, f, inner),
        WizardStep::LoadPath => render_load_path(app, f, inner),
        WizardStep::PrefixSelect => render_prefix_select(app, f, inner),
        WizardStep::ComponentsSelect => render_components(app, f, inner),
        WizardStep::Interfaces => render_interfaces(app, f, inner),
        WizardStep::Elasticsearch => render_elasticsearch(app, f, inner),
        WizardStep::S2sPassword => render_s2s(app, f, inner),
        WizardStep::ViewerUploads => render_viewer_uploads(app, f, inner),
        WizardStep::ViewerPlugins => render_viewer_plugins(app, f, inner),
        WizardStep::Plugins => render_plugins(app, f, inner),
        WizardStep::WiseUrl => render_wise_url(app, f, inner),
        WizardStep::DockerMounts => render_docker_mounts(app, f, inner),
        WizardStep::GeoIp => render_geoip(app, f, inner),
        WizardStep::AdminSetup => render_admin_setup(app, f, inner),
        WizardStep::Review => render_review(app, f, inner),
        WizardStep::Progress => render_progress(app, f, inner),
        WizardStep::Done => {}
    }
}

fn selectable(label: &str, selected: bool) -> Line<'static> {
    let marker = if selected { "▶ " } else { "  " };
    let style = if selected {
        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };
    Line::from(Span::styled(format!("{marker}{label}"), style))
}

fn render_start(app: &App, f: &mut Frame, area: Rect) {
    let modes = crate::domain::StartMode::available(app.platform.os);
    let mut lines = vec![
        Line::from("How do you want to configure Arkime?"),
        Line::from(""),
    ];
    for (i, m) in modes.iter().enumerate() {
        lines.push(selectable(m.label(), i == app.cursor));
    }
    if app.platform.os == crate::domain::Os::MacOs {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "(native modes are unavailable on macOS — docker only)",
            Style::default().fg(Color::DarkGray),
        )));
    }
    f.render_widget(Paragraph::new(lines), area);
}

fn render_load_path(app: &App, f: &mut Frame, area: Rect) {
    let what = match app.deployment {
        Some(Deployment::Docker) => "Path to the docker-compose.yml to load and update:",
        _ => "Path to the etc directory containing the .ini files to load:",
    };
    let lines = vec![
        Line::from(what),
        Line::from(Span::styled(
            "Everything we don't understand is preserved on write-back.",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(""),
        field_line("path", app.fields.load_path.value(), true),
    ];
    f.render_widget(Paragraph::new(lines), area);
}

fn render_prefix_select(app: &App, f: &mut Frame, area: Rect) {
    if app.prefix_adding {
        let lines = vec![
            Line::from("New service-name prefix (e.g. `arkime2-`; blank = no prefix):"),
            Line::from(Span::styled(
                "It's prepended to the service names — arkime2-capture, arkime2-viewer, …",
                Style::default().fg(Color::DarkGray),
            )),
            Line::from(""),
            field_line("prefix", app.fields.prefix.value(), true),
        ];
        f.render_widget(Paragraph::new(lines), area);
        return;
    }

    let mut lines = vec![
        Line::from("Service prefix to manage (each prefix is a separate deployment)."),
        Line::from(Span::styled(
            "Enter select · a add · d delete — other prefixes are left untouched.",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(""),
    ];
    for (i, p) in app.detected_prefixes.iter().enumerate() {
        let shown = if p.is_empty() {
            "(no prefix)".to_string()
        } else {
            format!("{p}*")
        };
        lines.push(selectable(&shown, i == app.cursor));
    }
    f.render_widget(Paragraph::new(lines), area);
}

fn render_components(app: &App, f: &mut Frame, area: Rect) {
    let mut lines = vec![
        Line::from("Toggle the components to configure (space), then Enter."),
        Line::from(""),
    ];
    for (i, c) in Component::ALL.iter().enumerate() {
        let checked = if app.components.contains(*c) {
            "[x]"
        } else {
            "[ ]"
        };
        let focused = i == app.cursor;
        let marker = if focused { "▶ " } else { "  " };
        let style = if focused {
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        lines.push(Line::from(Span::styled(
            format!("{marker}{checked} {}", c.label()),
            style,
        )));
    }
    f.render_widget(Paragraph::new(lines), area);
}

fn field_line(label: &str, value: &str, focused: bool) -> Line<'static> {
    let cursor = if focused { "_" } else { "" };
    let val_style = if focused {
        Style::default().fg(ACCENT)
    } else {
        Style::default()
    };
    Line::from(vec![
        Span::styled(format!("{label:>14}: "), Style::default().fg(Color::Gray)),
        Span::styled(format!("{value}{cursor}"), val_style),
    ])
}

fn render_interfaces(app: &App, f: &mut Frame, area: Rect) {
    if app.interface_advanced {
        let hint = if app.detected_interfaces.is_empty() {
            "No interfaces detected — type them manually.".to_string()
        } else {
            "Advanced mode — Tab returns to the checkbox list.".to_string()
        };
        let lines = vec![
            Line::from("Interface(s) to monitor, ';'-separated."),
            Line::from(Span::styled(hint, Style::default().fg(Color::DarkGray))),
            Line::from(""),
            field_line("interfaces", app.fields.interface.value(), true),
        ];
        f.render_widget(Paragraph::new(lines), area);
        return;
    }

    let mut lines = vec![
        Line::from("Select the interface(s) to monitor (space to toggle)."),
        Line::from(""),
    ];
    for (i, name) in app.detected_interfaces.iter().enumerate() {
        let checked = if app.interface_checked[i] {
            "[x]"
        } else {
            "[ ]"
        };
        let focused = i == app.cursor;
        let marker = if focused { "▶ " } else { "  " };
        let style = if focused {
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        lines.push(Line::from(Span::styled(
            format!("{marker}{checked} {name}"),
            style,
        )));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Press 'a' for advanced mode (type interfaces by hand).",
        Style::default().fg(Color::DarkGray),
    )));
    f.render_widget(Paragraph::new(lines), area);
}

fn mask(value: &str) -> String {
    "*".repeat(value.chars().count())
}

fn render_elasticsearch(app: &App, f: &mut Frame, area: Rect) {
    let demo = if app.answers.install_demo_es {
        "[x]"
    } else {
        "[ ]"
    };
    let is_docker = app.deployment == Some(Deployment::Docker);
    let toggle_label = if is_docker {
        "run a single-node Elasticsearch we configure (space)"
    } else {
        "install a local demo Elasticsearch (space)"
    };
    let mut lines = vec![
        Line::from("OpenSearch/Elasticsearch connection. Tab/↑↓ between fields."),
        Line::from(""),
        field_line("URL", app.fields.es_url.value(), app.es_focus == 0),
        field_line(
            "user (optional)",
            app.fields.es_user.value(),
            app.es_focus == 1,
        ),
        field_line(
            "password",
            &mask(app.fields.es_password.value()),
            app.es_focus == 2,
        ),
        {
            let focused = app.es_focus == 3;
            let style = if focused {
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            Line::from(Span::styled(
                format!("{}{demo} {toggle_label}", if focused { "▶ " } else { "  " }),
                style,
            ))
        },
    ];
    // Docker single-node ES: ask for the host data directory (a compose volume).
    if is_docker && app.answers.install_demo_es {
        lines.push(field_line(
            "data dir",
            app.fields.es_data.value(),
            app.es_focus == 4,
        ));
    }
    f.render_widget(Paragraph::new(lines), area);
}

fn render_s2s(app: &App, f: &mut Frame, area: Rect) {
    let lines = vec![
        Line::from("Password to encrypt S2S and other secrets (no spaces)."),
        Line::from(""),
        field_line("password", &mask(app.fields.s2s.value()), true),
    ];
    f.render_widget(Paragraph::new(lines), area);
}

fn render_plugins(app: &App, f: &mut Frame, area: Rect) {
    if app.plugin_advanced {
        let lines = vec![
            Line::from("Capture plugins to load, ';'-separated."),
            Line::from(Span::styled(
                "Advanced mode — Tab returns to the checkbox list.",
                Style::default().fg(Color::DarkGray),
            )),
            Line::from(""),
            field_line("plugins", app.fields.plugins.value(), true),
        ];
        f.render_widget(Paragraph::new(lines), area);
        return;
    }

    let mut lines = vec![
        Line::from("Select capture plugins to load (space to toggle)."),
        Line::from(""),
    ];
    for (i, name) in plugins::KNOWN_PLUGINS.iter().enumerate() {
        let checked = if app.plugin_checked[i] { "[x]" } else { "[ ]" };
        let locked = *name == plugins::WISE_PLUGIN && app.components.wise;
        let focused = i == app.cursor;
        let marker = if focused { "▶ " } else { "  " };
        let suffix = if locked { "  (required by wise)" } else { "" };
        let style = if focused {
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
        } else if locked {
            Style::default().fg(Color::DarkGray)
        } else {
            Style::default()
        };
        lines.push(Line::from(Span::styled(
            format!("{marker}{checked} {name}{suffix}"),
            style,
        )));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Press 'a' to type a custom plugin list.",
        Style::default().fg(Color::DarkGray),
    )));
    f.render_widget(Paragraph::new(lines), area);
}

fn render_viewer_plugins(app: &App, f: &mut Frame, area: Rect) {
    if app.viewer_plugin_advanced {
        let lines = vec![
            Line::from("Viewer plugins to load, ';'-separated."),
            Line::from(Span::styled(
                "Advanced mode — Tab returns to the checkbox list.",
                Style::default().fg(Color::DarkGray),
            )),
            Line::from(""),
            field_line("viewerPlugins", app.fields.viewer_plugins.value(), true),
        ];
        f.render_widget(Paragraph::new(lines), area);
        return;
    }

    let mut lines = vec![
        Line::from("Select viewer plugins to load (space to toggle)."),
        Line::from(""),
    ];
    for (i, name) in plugins::KNOWN_VIEWER_PLUGINS.iter().enumerate() {
        let checked = if app.viewer_plugin_checked[i] {
            "[x]"
        } else {
            "[ ]"
        };
        lines.push(selectable(&format!("{checked} {name}"), i == app.cursor));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Press 'a' to type a custom viewer-plugin list.",
        Style::default().fg(Color::DarkGray),
    )));
    f.render_widget(Paragraph::new(lines), area);
}

fn render_wise_url(app: &App, f: &mut Frame, area: Rect) {
    let lines = vec![
        Line::from("A WISE plugin is enabled but the wise service isn't deployed here."),
        Line::from(Span::styled(
            "Enter the URL of the external WISE service to query.",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(""),
        field_line("WISE URL", app.fields.wise_url.value(), true),
    ];
    f.render_widget(Paragraph::new(lines), area);
}

fn render_docker_mounts(app: &App, f: &mut Frame, area: Rect) {
    let relevant = MountSelection::relevant_kinds(&app.components);
    let mut lines = vec![
        Line::from("Host mounts — edit the host path, space toggles a mount on/off."),
        Line::from(Span::styled(
            "(the container path on the right is fixed)",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(""),
    ];
    for (i, kind) in relevant.iter().enumerate() {
        let focused = i == app.cursor;
        let enabled = app.docker_mounts.is_enabled(*kind);
        let checked = if enabled { "[x]" } else { "[ ]" };
        let marker = if focused { "▶ " } else { "  " };
        let host = app.docker_mounts.host(*kind);
        let cursor = if focused { "_" } else { "" };
        let host_style = if focused {
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
        } else if enabled {
            Style::default()
        } else {
            Style::default().fg(Color::DarkGray)
        };
        lines.push(Line::from(vec![
            Span::styled(
                format!("{marker}{checked} {:<11} ", kind.label()),
                if focused {
                    Style::default().fg(ACCENT)
                } else {
                    Style::default().fg(Color::Gray)
                },
            ),
            Span::styled(format!("{host}{cursor}"), host_style),
            Span::styled(
                format!(" → {}", kind.container()),
                Style::default().fg(Color::DarkGray),
            ),
        ]));
    }
    f.render_widget(Paragraph::new(lines), area);
}

/// A single always-focused checkbox line for the boolean-toggle screens.
fn checkbox_line(label: &str, checked: bool) -> Line<'static> {
    let mark = if checked { "[x]" } else { "[ ]" };
    Line::from(Span::styled(
        format!("▶ {mark} {label}"),
        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
    ))
}

fn render_viewer_uploads(app: &App, f: &mut Frame, area: Rect) {
    let lines = vec![
        Line::from("Viewer uploads (space to toggle)."),
        Line::from(""),
        checkbox_line(
            "Allow PCAP uploads through the viewer UI",
            app.answers.enable_uploads,
        ),
        Line::from(Span::styled(
            "Sets uploadCommand so operators can upload capture files.",
            Style::default().fg(Color::DarkGray),
        )),
    ];
    f.render_widget(Paragraph::new(lines), area);
}

fn render_admin_setup(app: &App, f: &mut Frame, area: Rect) {
    let toggle = |on: bool, label: &str, focused: bool| -> Line<'static> {
        let mark = if on { "[x]" } else { "[ ]" };
        let marker = if focused { "▶ " } else { "  " };
        let style = if focused {
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        Line::from(Span::styled(format!("{marker}{mark} {label}"), style))
    };
    let mut lines = vec![
        Line::from("Post-setup actions (Tab/↑↓ between rows, space toggles)."),
        Line::from(""),
        toggle(
            app.answers.init_db,
            "Initialize the database (db.pl init --ifneeded)",
            app.admin_focus == 0,
        ),
        toggle(
            app.answers.create_admin,
            "Create an admin user",
            app.admin_focus == 1,
        ),
    ];
    if app.answers.create_admin {
        lines.push(field_line(
            "user",
            app.fields.admin_user.value(),
            app.admin_focus == 2,
        ));
        lines.push(field_line(
            "password",
            &mask(app.fields.admin_password.value()),
            app.admin_focus == 3,
        ));
    }
    f.render_widget(Paragraph::new(lines), area);
}

fn render_geoip(app: &App, f: &mut Frame, area: Rect) {
    let mut lines = vec![
        Line::from("GeoIP — MaxMind account + license (blank to skip GeoIP.conf)."),
        Line::from(Span::styled(
            "Free account: https://arkime.com/faq#maxmind",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(""),
        field_line(
            "account ID",
            app.fields.maxmind_account.value(),
            app.geoip_focus == 0,
        ),
        field_line(
            "license key",
            app.fields.maxmind_key.value(),
            app.geoip_focus == 1,
        ),
    ];
    // Native runs the download now; docker's container does it on start.
    if app.deployment == Some(Deployment::Native) {
        let checked = if app.answers.download_geoip {
            "[x]"
        } else {
            "[ ]"
        };
        let marker = if app.geoip_focus == 2 { "▶ " } else { "  " };
        let style = if app.geoip_focus == 2 {
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        lines.push(Line::from(Span::styled(
            format!("{marker}{checked} Download GeoIP files now (space)"),
            style,
        )));
    }
    f.render_widget(Paragraph::new(lines), area);
}

fn render_review(app: &App, f: &mut Frame, area: Rect) {
    let comps: Vec<&str> = app.components.enabled().map(|c| c.label()).collect();
    let mut lines = vec![
        Line::from(Span::styled(
            "Review — Enter to apply, ← to go back",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        kv(
            "Deployment",
            app.deployment.map(|d| d.label()).unwrap_or("-"),
        ),
        kv("Components", &comps.join(", ")),
    ];
    if app.components.needs_interfaces() {
        lines.push(kv("Interfaces", &app.answers.interfaces));
    }
    if app.components.needs_elasticsearch() {
        let is_docker = app.deployment == Some(Deployment::Docker);
        let single_node = app.answers.install_demo_es;
        // With our single-node ES the containers use localhost.
        let es_url = if is_docker && single_node {
            crate::domain::Answers::SINGLE_NODE_ES_URL
        } else {
            app.answers.elasticsearch_or_default()
        };
        lines.push(kv("Elasticsearch", es_url));
        if app.answers.has_es_user() && !single_node {
            lines.push(kv("ES user", &app.answers.es_user));
        }
        if is_docker {
            lines.push(kv(
                "Single-node ES",
                if single_node {
                    "yes (we configure it)"
                } else {
                    "no"
                },
            ));
            if single_node {
                lines.push(kv("ES data dir", &app.answers.es_data_dir));
            }
        } else {
            lines.push(kv("Demo ES", if single_node { "yes" } else { "no" }));
        }
    }
    if app.components.needs_s2s_password() {
        lines.push(kv("Encryption pw", &mask(&app.answers.s2s_password)));
    }
    if app.components.viewer {
        lines.push(kv(
            "Viewer uploads",
            if app.answers.enable_uploads {
                "yes"
            } else {
                "no"
            },
        ));
        let vp = if app.answers.viewer_plugins.is_empty() {
            "(none)"
        } else {
            &app.answers.viewer_plugins
        };
        lines.push(kv("Viewer plugins", vp));
    }
    if app.components.capture {
        let plugins = if app.answers.plugins.is_empty() {
            "(none)"
        } else {
            &app.answers.plugins
        };
        lines.push(kv("Plugins", plugins));
        if !app.answers.wise_url.is_empty() {
            lines.push(kv("WISE URL", &app.answers.wise_url));
        }
    }
    if app.components.capture || app.components.viewer {
        let geo = if !app.answers.maxmind_account.is_empty() {
            "GeoIP.conf (MaxMind creds set)"
        } else {
            "skipped"
        };
        lines.push(kv("GeoIP", geo));
    }
    if app.deployment == Some(Deployment::Native)
        && (app.components.capture || app.components.viewer)
    {
        if app.answers.init_db {
            lines.push(kv("Initialize DB", "yes (init --ifneeded)"));
        }
        if app.answers.create_admin {
            lines.push(kv("Admin user", &app.answers.admin_user));
        }
    }
    if app.deployment == Some(Deployment::Docker) {
        let mounts: Vec<String> = MountSelection::relevant_kinds(&app.components)
            .into_iter()
            .filter(|k| app.docker_mounts.is_enabled(*k))
            .map(|k| app.docker_mounts.spec(k))
            .collect();
        let text = if mounts.is_empty() {
            "(none)".to_string()
        } else {
            mounts.join(", ")
        };
        lines.push(kv("Mounts", &text));

        let prefix = if app.service_prefix.is_empty() {
            "(none)"
        } else {
            &app.service_prefix
        };
        lines.push(kv("Service prefix", prefix));
        if !app.other_prefixes.is_empty() {
            lines.push(kv("Untouched prefixes", &app.other_prefixes.join(", ")));
        }
    }
    // Native apply needs root; warn before the user tries.
    if app.deployment == Some(Deployment::Native) && !app.is_root {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "⚠ Native setup requires root — re-run with sudo, or pick Docker.",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        )));
    }
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
}

fn kv(k: &str, v: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{k:>16}: "), Style::default().fg(Color::Gray)),
        Span::raw(v.to_string()),
    ])
}

fn render_progress(app: &App, f: &mut Frame, area: Rect) {
    let mut lines: Vec<Line> = app
        .log
        .iter()
        .map(|l| {
            let color = match l.level {
                Level::Info => Color::Green,
                Level::Warn => Color::Yellow,
                Level::Error => Color::Red,
            };
            Line::from(vec![
                Span::styled("• ", Style::default().fg(color)),
                Span::raw(l.text.clone()),
            ])
        })
        .collect();
    if app.applied {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "Finished — press Enter or q to exit.",
            Style::default().add_modifier(Modifier::BOLD),
        )));
    }
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
}

fn render_footer(app: &App, f: &mut Frame, area: Rect) {
    // Every screen (once files exist) offers the editor via ^E.
    let edit = if app.docs.is_empty() {
        ""
    } else if app.step == WizardStep::Review {
        " · ^E edit · ^D diff changes"
    } else {
        " · ^E edit · ^D diff"
    };
    let help = match app.step {
        WizardStep::StartSelect => "↑↓ choose · →/Enter select · Esc quit",
        WizardStep::LoadPath => "type path · Enter load · Esc back",
        WizardStep::PrefixSelect => {
            if app.prefix_adding {
                "type prefix · Enter add · Esc cancel"
            } else {
                "↑↓ choose · Enter select · a add · d delete · ←/Esc back"
            }
        }
        WizardStep::ComponentsSelect => "↑↓ move · space toggle · →/Enter next · ←/Esc back",
        WizardStep::Interfaces => {
            if app.interface_advanced {
                "type · Tab checkboxes · Enter next · Esc back"
            } else {
                "↑↓ move · space toggle · a advanced · →/Enter next · ←/Esc back"
            }
        }
        WizardStep::S2sPassword => "type · Enter next · Esc back",
        WizardStep::Plugins => {
            if app.plugin_advanced {
                "type · Tab checkboxes · Enter next · Esc back"
            } else {
                "↑↓ move · space toggle · a custom · →/Enter next · ←/Esc back"
            }
        }
        WizardStep::ViewerPlugins => {
            if app.viewer_plugin_advanced {
                "type · Tab checkboxes · Enter next · Esc back"
            } else {
                "↑↓ move · space toggle · a custom · →/Enter next · ←/Esc back"
            }
        }
        WizardStep::WiseUrl => "type · Enter next · Esc back",
        WizardStep::DockerMounts => {
            "↑↓ row · type host path · space toggle · Enter next · Esc back"
        }
        WizardStep::Elasticsearch => "Tab/↑↓ field · type · space (demo) · Enter next · Esc back",
        WizardStep::ViewerUploads => "space toggle · →/Enter next · ←/Esc back",
        WizardStep::GeoIp => "Tab/↑↓ field · type · space (download) · Enter next · Esc back",
        WizardStep::AdminSetup => "Tab/↑↓ row · space toggle · type · Enter next · Esc back",
        WizardStep::Review => "→/Enter apply · ←/Esc back",
        WizardStep::Progress => {
            if app.applied {
                "Enter/Esc exit"
            } else {
                "applying…"
            }
        }
        WizardStep::Done => "",
    };
    let line = if let Some(err) = &app.error {
        Line::from(Span::styled(err.clone(), Style::default().fg(Color::Red)))
    } else {
        Line::from(Span::styled(
            format!("{help}{edit}"),
            Style::default().fg(Color::DarkGray),
        ))
    };
    f.render_widget(
        Paragraph::new(line).block(Block::default().borders(Borders::TOP)),
        area,
    );
}

/// Full-screen editor overlay with a tab per document.
fn render_editor(app: &App, f: &mut Frame) {
    let ed = app.editor.as_ref().unwrap();
    let area = f.area();
    // Clear behind the overlay.
    f.render_widget(ratatui::widgets::Clear, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(area);

    // Tab bar.
    let mode = if ed.diff { " DIFF " } else { " EDIT " };
    let mut spans = vec![Span::styled(
        mode,
        Style::default()
            .fg(Color::Black)
            .bg(ACCENT)
            .add_modifier(Modifier::BOLD),
    )];
    for (i, d) in app.docs.iter().enumerate() {
        let sel = i == ed.tab;
        let style = if sel {
            Style::default()
                .fg(ACCENT)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
        } else {
            Style::default().fg(Color::Gray)
        };
        spans.push(Span::raw(" "));
        spans.push(Span::styled(d.kind.filename().to_string(), style));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), chunks[0]);

    // Body: a diff (original vs current) or the editable buffer.
    if ed.diff {
        let original = app
            .docs
            .get(ed.tab)
            .map(|d| d.original.as_str())
            .unwrap_or("");
        let current = ed
            .areas
            .get(ed.tab)
            .map(|a| a.lines().join("\n"))
            .unwrap_or_default();
        f.render_widget(
            Paragraph::new(diff_lines(original, &current)).wrap(Wrap { trim: false }),
            chunks[1],
        );
    } else if let Some(area_w) = ed.areas.get(ed.tab) {
        f.render_widget(area_w, chunks[1]);
    }

    let hint = if ed.diff {
        "Tab/Shift-Tab switch file · ^D back to edit · Esc/^E done"
    } else {
        "Tab/Shift-Tab switch file · edit freely · ^D diff · Esc/^E done (syncs to wizard)"
    };
    f.render_widget(
        Paragraph::new(Span::styled(hint, Style::default().fg(Color::DarkGray)))
            .block(Block::default().borders(Borders::TOP)),
        chunks[2],
    );
}

/// Unified line diff (original vs current) for the editor's diff view.
fn diff_lines(original: &str, current: &str) -> Vec<Line<'static>> {
    use similar::{ChangeTag, TextDiff};
    let diff = TextDiff::from_lines(original, current);
    let mut out = Vec::new();
    let mut changed = false;
    for change in diff.iter_all_changes() {
        let (sign, style) = match change.tag() {
            ChangeTag::Delete => ("-", Style::default().fg(Color::Red)),
            ChangeTag::Insert => ("+", Style::default().fg(Color::Green)),
            ChangeTag::Equal => (" ", Style::default().fg(Color::DarkGray)),
        };
        if change.tag() != ChangeTag::Equal {
            changed = true;
        }
        let text = change.value().trim_end_matches('\n').to_string();
        out.push(Line::from(Span::styled(format!("{sign}{text}"), style)));
    }
    if !changed {
        out.push(Line::from(Span::styled(
            "(no changes from the original)",
            Style::default().fg(Color::DarkGray),
        )));
    }
    out
}
