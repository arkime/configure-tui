//! Dev aid: render a wizard screen and the generated docker artifacts to stdout,
//! so the output can be eyeballed on any platform (the real binary refuses to
//! run on macOS). `cargo run --example preview`.

use arkime_setup::app::App;
use arkime_setup::config::substitute::BasicAuthEncoding;
use arkime_setup::docset::{render_compose, render_env, Images};
use arkime_setup::domain::{
    Answers, BuildConfig, Components, Deployment, MountSelection, Os, Platform, ServiceManagerKind,
};
use arkime_setup::steps::WizardStep;
use arkime_setup::ui;
use ratatui::backend::TestBackend;
use ratatui::Terminal;

fn render(app: &App) -> String {
    let mut terminal = Terminal::new(TestBackend::new(90, 16)).unwrap();
    terminal.draw(|f| ui::view(app, f)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    let area = *buffer.area();
    let mut out = String::new();
    for y in 0..area.height {
        for x in 0..area.width {
            out.push_str(buffer[(x, y)].symbol());
        }
        out.push('\n');
    }
    out
}

fn main() {
    let mut app = App::new(
        BuildConfig {
            name: "arkime".into(),
            install_dir: "/opt/arkime".into(),
        },
        Platform {
            os: Os::Linux,
            service_manager: ServiceManagerKind::Systemd,
        },
    );
    println!("=== Start screen (4 modes) ===\n{}", render(&app));

    // Prefix-select screen (choose / add / delete).
    app.deployment = Some(Deployment::Docker);
    app.is_load = true;
    app.detected_prefixes = vec!["".into(), "arkime1-".into(), "arkime2-".into()];
    app.cursor = 2;
    app.step = WizardStep::PrefixSelect;
    println!("=== Prefix select screen ===\n{}", render(&app));

    // Prefix add mode.
    app.prefix_adding = true;
    app.fields.prefix = tui_input::Input::new("arkime3-".into());
    println!("=== Prefix add mode (a) ===\n{}", render(&app));
    app.prefix_adding = false;

    app.deployment = None;
    app.is_load = false;
    app.detected_prefixes.clear();

    app.components.capture = true;
    app.components.viewer = true;
    app.step = WizardStep::ComponentsSelect;
    println!("=== Components screen ===\n{}", render(&app));

    // Force some detected NICs so the checkbox list shows (host detection is
    // empty on non-Linux).
    app.detected_interfaces = vec!["eth0".into(), "eth1".into(), "ens5".into()];
    app.interface_checked = vec![true, false, false];
    app.interface_advanced = false;
    app.cursor = 1;
    app.step = WizardStep::Interfaces;
    println!("=== Interfaces screen (checkboxes) ===\n{}", render(&app));

    app.deployment = Some(Deployment::Native);
    app.answers.interfaces = "eth0;eth1".into();
    app.answers.elasticsearch = "https://os:9200".into();
    app.answers.s2s_password = "secret".into();
    app.step = WizardStep::Review;
    println!("=== Review screen ===\n{}", render(&app));

    let components = Components {
        capture: true,
        viewer: true,
        ..Default::default()
    };
    let answers = Answers {
        interfaces: "eth0;eth1".into(),
        es_user: "admin".into(),
        es_password: "pass".into(),
        s2s_password: "secret".into(),
        install_demo_es: true,
        es_data_dir: "/esdata".into(),
        plugins: "wise.so;ja4plus.amd64.so;entropy.so".into(),
        ..Default::default()
    };
    let compose = render_compose(
        "",
        arkime_setup::docset::DEFAULT_PREFIX,
        &components,
        &answers,
        &MountSelection::default(),
        &Images::default(),
    );
    let env = render_env("", &answers, BasicAuthEncoding::Plaintext);
    println!("=== docker-compose.yml (docker mode, demo ES) ===\n{compose}");
    println!("=== arkime.env ===\n{env}");

    // Plugins screen (wise component forces wise.so on).
    app.components.wise = true;
    app.plugin_checked = vec![true, true, false, false];
    app.cursor = 1;
    app.step = WizardStep::Plugins;
    println!("=== Plugins screen ===\n{}", render(&app));

    // WISE URL screen (wise.so enabled, no wise component).
    app.components.wise = false;
    app.step = WizardStep::WiseUrl;
    println!("=== WISE URL screen ===\n{}", render(&app));

    // Docker mounts screen.
    app.deployment = Some(Deployment::Docker);
    app.components.wise = true;
    app.cursor = 0;
    app.step = WizardStep::DockerMounts;
    println!("=== Docker mounts screen ===\n{}", render(&app));

    // Editor overlay (file tabs; Tab cycles).
    app.components = Components {
        capture: true,
        viewer: true,
        ..Default::default()
    };
    app.answers.interfaces = "eth0;eth1".into();
    app.answers.s2s_password = "secret".into();
    app.open_editor();
    println!(
        "=== Editor overlay (docker-compose.yml tab) ===\n{}",
        render(&app)
    );

    // Diff view: pretend the compose was loaded with a bridge network, so the
    // change to host networking shows up as a diff.
    let bridged = app.docs[0]
        .text
        .replace("network_mode: host", "network_mode: bridge");
    app.docs[0].original = bridged;
    app.editor.as_mut().unwrap().diff = true;
    println!("=== Editor diff view (^D) ===\n{}", render(&app));
}
