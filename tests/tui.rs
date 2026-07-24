//! View-layer tests driving the real render path through ratatui's TestBackend.

use arkime_setup::app::App;
use arkime_setup::domain::{BuildConfig, Deployment, Os, Platform, ServiceManagerKind};
use arkime_setup::steps::WizardStep;
use arkime_setup::ui;
use ratatui::backend::TestBackend;
use ratatui::Terminal;
use tui_input::Input;

fn platform() -> Platform {
    Platform {
        os: Os::Linux,
        service_manager: ServiceManagerKind::Systemd,
    }
}

fn app() -> App {
    App::new(
        BuildConfig {
            name: "arkime".into(),
            install_dir: "/opt/arkime".into(),
        },
        platform(),
    )
}

/// Render the current view and flatten the buffer to a string.
fn render(app: &App) -> String {
    let backend = TestBackend::new(90, 24);
    let mut terminal = Terminal::new(backend).unwrap();
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

#[test]
fn s2s_password_is_masked_never_plaintext() {
    let mut app = app();
    app.deployment = Some(Deployment::Native);
    app.components.capture = true;
    app.fields.s2s = Input::new("hunter2secret".into());
    app.step = WizardStep::S2sPassword;

    let screen = render(&app);
    assert!(screen.contains("*************"), "expected masked password");
    assert!(
        !screen.contains("hunter2secret"),
        "plaintext password leaked into the view"
    );
}

#[test]
fn startup_screen_shows_the_four_modes() {
    let app = app();
    let screen = render(&app);
    assert!(screen.contains("Docker"));
    assert!(screen.contains("Run on machine"));
    assert!(screen.contains("create"));
    assert!(screen.contains("load"));
}

#[test]
fn review_lists_selected_components_and_masks_password() {
    let mut app = app();
    app.deployment = Some(Deployment::Native);
    app.components.capture = true;
    app.components.viewer = true;
    app.answers.interfaces = "eth0;eth1".into();
    app.answers.elasticsearch = "https://os:9200".into();
    app.answers.s2s_password = "topsecret".into();
    app.step = WizardStep::Review;

    let screen = render(&app);
    assert!(screen.contains("capture"));
    assert!(screen.contains("viewer"));
    assert!(screen.contains("eth0;eth1"));
    assert!(!screen.contains("topsecret"), "review leaked the password");
}
