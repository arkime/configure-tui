//! Native-deployment system actions: create dirs, write limits.d, download
//! GeoIP / demo ES, and enable/start services. The `.ini` files themselves are
//! written from the in-memory documents by the caller — this handles only the
//! non-file side effects.

use crate::actions::system::SystemOps;
use crate::domain::{
    Answers, BuildConfig, Component, Components, Os, Platform, ServiceManagerKind,
};
use crate::log::{Level, LogLine};
use std::path::Path;

/// The limits.d contents, verbatim from Configure lines 231-234.
const LIMITS_CONF: &str = "\
nobody  -       core    unlimited
root    -       core    unlimited
nobody  -       memlock    unlimited
root    -       memlock    unlimited
";

/// Run the native side effects (everything except writing the `.ini` files).
/// Never returns Err: each step logs its own outcome so one failure does not
/// abort the rest.
pub fn system_actions(
    ops: &dyn SystemOps,
    build: &BuildConfig,
    platform: Platform,
    components: &Components,
    answers: &Answers,
    log: &mut Vec<LogLine>,
) {
    // 2. Recreate data dirs (only meaningful when capturing/viewing).
    if components.capture || components.viewer {
        for sub in ["logs", "raw"] {
            let dir = build.install_dir.join(sub);
            match ops.mkdir_mode_chown(&dir, 0o700, "nobody") {
                Ok(()) => log.push(LogLine::new(
                    Level::Info,
                    format!("Ensured {}", dir.display()),
                )),
                Err(e) => log.push(LogLine::new(
                    Level::Warn,
                    format!("mkdir {}: {e}", dir.display()),
                )),
            }
        }
    }

    // 2b. Optional local demo OpenSearch/Elasticsearch (bash lines 205-225).
    if answers.install_demo_es && (components.capture || components.viewer) {
        install_demo_es(ops, log);
    }

    // 3. limits.d (Linux only, and only if the dir exists — bash line 228).
    if platform.os == Os::Linux && Path::new("/etc/security/limits.d").is_dir() {
        let path = Path::new("/etc/security/limits.d/99-arkime.conf");
        match ops.write_new(path, LIMITS_CONF) {
            Ok(true) => log.push(LogLine::new(
                Level::Info,
                "limits.d/99-arkime.conf already present".into(),
            )),
            Ok(false) => log.push(LogLine::new(
                Level::Info,
                "Installed limits.d/99-arkime.conf".into(),
            )),
            Err(e) => log.push(LogLine::new(Level::Warn, format!("limits.d: {e}"))),
        }
    }

    // 4. GeoIP (bash lines 246-248).
    if answers.download_geoip && (components.capture || components.viewer) {
        let script = build.install_dir.join("bin/arkime_update_geo.sh");
        match ops.run(&script.to_string_lossy(), &[]) {
            Ok(()) => log.push(LogLine::new(Level::Info, "Downloaded GEO files".into())),
            Err(e) => log.push(LogLine::new(Level::Warn, format!("GEO update: {e}"))),
        }
    }

    // 5. Database init + admin user (before starting services).
    let db = build.install_dir.join("db/db.pl");
    let add_user = build.install_dir.join("bin/arkime_add_user.sh");
    let es = answers.elasticsearch_or_default();
    if answers.init_db {
        // db.pl init prompts for the word INIT; feed it.
        let cmd = format!("echo INIT | {} {} init", db.to_string_lossy(), es);
        match ops.run("sh", &["-c", &cmd]) {
            Ok(()) => log.push(LogLine::new(Level::Info, "Initialized the database".into())),
            Err(e) => log.push(LogLine::new(Level::Warn, format!("db init: {e}"))),
        }
    }
    if answers.create_admin && !answers.admin_user.is_empty() {
        let r = ops.run(
            &add_user.to_string_lossy(),
            &[
                &answers.admin_user,
                &answers.admin_user,
                &answers.admin_password,
                "--admin",
            ],
        );
        match r {
            Ok(()) => log.push(LogLine::new(
                Level::Info,
                format!("Created admin user '{}'", answers.admin_user),
            )),
            Err(e) => log.push(LogLine::new(Level::Warn, format!("add user: {e}"))),
        }
    }

    // 6. Services.
    for c in components.enabled() {
        enable_start_service(ops, platform, c, log);
    }

    // Next steps for anything not done automatically.
    log.push(LogLine::new(Level::Info, "Done. Next steps:".into()));
    if !answers.init_db {
        log.push(LogLine::new(
            Level::Info,
            format!(
                "  Initialize the DB once:  {} {es} init",
                db.to_string_lossy()
            ),
        ));
    }
    if !answers.create_admin {
        log.push(LogLine::new(
            Level::Info,
            format!(
                "  Add a user:  {} <user> <name> <pass> --admin",
                add_user.to_string_lossy()
            ),
        ));
    }
    log.push(LogLine::new(
        Level::Info,
        "  Then open the viewer (default http://<host>:8005).".into(),
    ));
}

fn enable_start_service(
    ops: &dyn SystemOps,
    platform: Platform,
    c: Component,
    log: &mut Vec<LogLine>,
) {
    let svc = c.service_name();
    match platform.service_manager {
        ServiceManagerKind::Systemd => {
            let r = ops
                .run("systemctl", &["enable", svc])
                .and_then(|_| ops.run("systemctl", &["start", svc]));
            report_service(log, svc, r);
        }
        ServiceManagerKind::FreeBsdRc => {
            let r = ops
                .run("sysrc", &[&format!("{svc}_enable=YES")])
                .and_then(|_| ops.run("service", &[svc, "start"]));
            report_service(log, svc, r);
        }
        ServiceManagerKind::None => {
            log.push(LogLine::new(
                Level::Warn,
                format!("No service manager detected; not starting {svc}"),
            ));
        }
    }
}

fn report_service(log: &mut Vec<LogLine>, svc: &str, r: anyhow::Result<()>) {
    match r {
        Ok(()) => log.push(LogLine::new(
            Level::Info,
            format!("Enabled + started {svc}"),
        )),
        Err(e) => log.push(LogLine::new(Level::Warn, format!("{svc}: {e}"))),
    }
}

/// Demo OSS Elasticsearch install, mirroring bash lines 205-225: RPM distros use
/// `yum install <url>`; others `curl` the `.deb` and `dpkg -i`. NOT for
/// production — the prompt says so.
const ES_VERSION: &str = "7.10.2";

fn install_demo_es(ops: &dyn SystemOps, log: &mut Vec<LogLine>) {
    let (arch_rpm, arch_deb) = match std::env::consts::ARCH {
        "x86_64" => ("x86_64", "amd64"),
        "aarch64" => ("aarch64", "arm64"),
        other => {
            log.push(LogLine::new(
                Level::Warn,
                format!("Demo ES: unsupported arch {other}, skipping"),
            ));
            return;
        }
    };

    let base = "https://artifacts.elastic.co/downloads/elasticsearch";
    let is_rpm =
        Path::new("/etc/redhat-release").exists() || Path::new("/etc/system-release").exists();

    log.push(LogLine::new(
        Level::Info,
        "Installing demo OSS Elasticsearch (not for production)".into(),
    ));

    let result = if is_rpm {
        let url = format!("{base}/elasticsearch-oss-{ES_VERSION}-{arch_rpm}.rpm");
        ops.run("yum", &["install", "-y", &url])
    } else {
        let file = format!("elasticsearch-oss-{ES_VERSION}-{arch_deb}.deb");
        let url = format!("{base}/{file}");
        ops.run("curl", &["-sSfLO", &url])
            .and_then(|_| ops.run("dpkg", &["-i", &file]))
            .and_then(|_| ops.run("rm", &["-f", &file]))
    };

    match result {
        Ok(()) => log.push(LogLine::new(
            Level::Info,
            "Demo Elasticsearch installed".into(),
        )),
        Err(e) => log.push(LogLine::new(Level::Warn, format!("Demo ES install: {e}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::system::{Op, RecordingOps};
    use crate::domain::Os;

    fn platform_systemd() -> Platform {
        Platform {
            os: Os::Linux,
            service_manager: ServiceManagerKind::Systemd,
        }
    }

    fn build() -> BuildConfig {
        BuildConfig {
            name: "arkime".into(),
            install_dir: "/opt/arkime".into(),
        }
    }

    #[test]
    fn capture_viewer_writes_config_dirs_and_services() {
        let ops = RecordingOps::new();
        let components = Components {
            capture: true,
            viewer: true,
            ..Default::default()
        };
        let answers = Answers {
            interfaces: "eth0".into(),
            elasticsearch: "https://os:9200".into(),
            s2s_password: "secret".into(),
            ..Default::default()
        };

        let mut log = Vec::new();
        system_actions(
            &ops,
            &build(),
            platform_systemd(),
            &components,
            &answers,
            &mut log,
        );

        let recorded = ops.ops();
        // logs+raw dirs created, both services started (ini files are written by
        // the caller from documents, not here).
        assert!(recorded.iter().any(|o| matches!(o, Op::Mkdir { path, mode, owner } if path.ends_with("logs") && *mode == 0o700 && owner == "nobody")));
        assert!(recorded
            .iter()
            .any(|o| matches!(o, Op::Mkdir { path, .. } if path.ends_with("raw"))));
        assert!(recorded.iter().any(|o| matches!(o, Op::Run { program, args } if program == "systemctl" && args == &["enable", "arkimecapture"])));
        assert!(recorded.iter().any(|o| matches!(o, Op::Run { program, args } if program == "systemctl" && args == &["enable", "arkimeviewer"])));
    }

    #[test]
    fn freebsd_uses_rc_service_management() {
        let ops = RecordingOps::new();
        let components = Components {
            capture: true,
            ..Default::default()
        };
        let answers = Answers {
            interfaces: "em0".into(),
            s2s_password: "x".into(),
            ..Default::default()
        };
        let platform = Platform {
            os: Os::FreeBsd,
            service_manager: ServiceManagerKind::FreeBsdRc,
        };

        let mut log = Vec::new();
        system_actions(&ops, &build(), platform, &components, &answers, &mut log);

        let recorded = ops.ops();
        assert!(recorded.iter().any(|o| matches!(o, Op::Run { program, args } if program == "sysrc" && args == &["arkimecapture_enable=YES"])));
        assert!(recorded.iter().any(|o| matches!(o, Op::Run { program, args } if program == "service" && args == &["arkimecapture", "start"])));
    }
}
