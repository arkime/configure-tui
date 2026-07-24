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
        WizardStep::DeploymentSelect => "Deployment",
        WizardStep::ComponentsSelect => "Components",
        WizardStep::Interfaces => "Interfaces",
        WizardStep::Elasticsearch => "OpenSearch / Elasticsearch",
        WizardStep::S2sPassword => "Encryption password",
        WizardStep::Plugins => "Capture plugins",
        WizardStep::WiseUrl => "WISE URL",
        WizardStep::DockerMounts => "Docker mounts",
        WizardStep::GeoIp => "GeoIP",
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
        WizardStep::DeploymentSelect => render_deployment(app, f, inner),
        WizardStep::ComponentsSelect => render_components(app, f, inner),
        WizardStep::Interfaces => render_interfaces(app, f, inner),
        WizardStep::Elasticsearch => render_elasticsearch(app, f, inner),
        WizardStep::S2sPassword => render_s2s(app, f, inner),
        WizardStep::Plugins => render_plugins(app, f, inner),
        WizardStep::WiseUrl => render_wise_url(app, f, inner),
        WizardStep::DockerMounts => render_docker_mounts(app, f, inner),
        WizardStep::GeoIp => render_geoip(app, f, inner),
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

fn render_deployment(app: &App, f: &mut Frame, area: Rect) {
    let lines = vec![
        Line::from("How do you want to run Arkime?"),
        Line::from(""),
        selectable(Deployment::Native.label(), app.cursor == 0),
        selectable(Deployment::Docker.label(), app.cursor == 1),
    ];
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
    let lines = vec![
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
                format!(
                    "{}{demo} install local demo OpenSearch (space)",
                    if focused { "▶ " } else { "  " }
                ),
                style,
            ))
        },
    ];
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

fn render_wise_url(app: &App, f: &mut Frame, area: Rect) {
    let lines = vec![
        Line::from("wise.so is enabled but the wise component isn't deployed here."),
        Line::from(Span::styled(
            "Enter the URL of the external WISE service capture should query.",
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
        Line::from("Suggested host mounts (space to toggle). Host paths on the left."),
        Line::from(""),
    ];
    for (i, kind) in relevant.iter().enumerate() {
        let checked = if app.docker_mounts.is_enabled(*kind) {
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
            format!("{marker}{checked} {}", kind.spec()),
            style,
        )));
    }
    f.render_widget(Paragraph::new(lines), area);
}

fn render_geoip(app: &App, f: &mut Frame, area: Rect) {
    let choice = if app.answers.download_geoip {
        "yes"
    } else {
        "no"
    };
    let lines = vec![
        Line::from("Download GeoIP files? (needs a MaxMind account)"),
        Line::from(Span::styled(
            "https://arkime.com/faq#maxmind",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(""),
        Line::from(vec![
            Span::raw("Choice: "),
            Span::styled(
                choice,
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "   (y/n or space to toggle)",
                Style::default().fg(Color::DarkGray),
            ),
        ]),
    ];
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
        lines.push(kv("Elasticsearch", app.answers.elasticsearch_or_default()));
        if app.answers.has_es_user() {
            lines.push(kv("ES user", &app.answers.es_user));
        }
        lines.push(kv(
            "Demo OpenSearch",
            if app.answers.install_demo_es {
                "yes"
            } else {
                "no"
            },
        ));
    }
    if app.components.needs_s2s_password() {
        lines.push(kv("Encryption pw", &mask(&app.answers.s2s_password)));
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
    if app.deployment == Some(Deployment::Native) && app.components.capture {
        lines.push(kv(
            "GeoIP",
            if app.answers.download_geoip {
                "yes"
            } else {
                "no"
            },
        ));
    }
    if app.deployment == Some(Deployment::Docker) {
        let mounts: Vec<String> = MountSelection::relevant_kinds(&app.components)
            .into_iter()
            .filter(|k| app.docker_mounts.is_enabled(*k))
            .map(|k| k.spec())
            .collect();
        let text = if mounts.is_empty() {
            "(none)".to_string()
        } else {
            mounts.join(", ")
        };
        lines.push(kv("Mounts", &text));
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
    let help = match app.step {
        WizardStep::DeploymentSelect => "↑↓ choose · Enter select · Esc quit",
        WizardStep::ComponentsSelect => "↑↓ move · space toggle · Enter next · ← back · Esc quit",
        WizardStep::Interfaces => {
            if app.interface_advanced {
                "type · Tab checkboxes · Enter next · Esc quit"
            } else {
                "↑↓ move · space toggle · a advanced · Enter next · ← back"
            }
        }
        WizardStep::S2sPassword => "type · Enter next · ← back · Esc quit",
        WizardStep::Plugins => {
            if app.plugin_advanced {
                "type · Tab checkboxes · Enter next · Esc quit"
            } else {
                "↑↓ move · space toggle · a custom · Enter next · ← back"
            }
        }
        WizardStep::WiseUrl => "type · Enter next · ← back · Esc quit",
        WizardStep::DockerMounts => "↑↓ move · space toggle · Enter next · ← back",
        WizardStep::Elasticsearch => "Tab/↑↓ field · type · space (demo) · Enter next · ← back",
        WizardStep::GeoIp => "y/n · Enter next · ← back · Esc quit",
        WizardStep::Review => "Enter apply · ← back · Esc quit",
        WizardStep::Progress => {
            if app.applied {
                "Enter/q exit"
            } else {
                "applying…"
            }
        }
        WizardStep::Done => "",
    };
    let line = if let Some(err) = &app.error {
        Line::from(Span::styled(err.clone(), Style::default().fg(Color::Red)))
    } else {
        Line::from(Span::styled(help, Style::default().fg(Color::DarkGray)))
    };
    f.render_widget(
        Paragraph::new(line).block(Block::default().borders(Borders::TOP)),
        area,
    );
}
