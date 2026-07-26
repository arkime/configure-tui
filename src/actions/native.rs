//! Native-deployment system actions: create dirs, write limits.d, download
//! GeoIP / demo ES, and enable/start services. The `.ini` files themselves are
//! written from the in-memory documents by the caller — this handles only the
//! non-file side effects.

use crate::actions::system::SystemOps;
use crate::domain::{
    Answers, BuildConfig, Component, Components, EsBackend, Os, Platform, ServiceManagerKind,
};
use crate::log::{Level, LogLine};
use anyhow::Result;
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

    // 2b. Optional single-node datastore (demo, not for production).
    if answers.es_backend.is_some() && (components.capture || components.viewer) {
        install_backend(answers.es_backend, ops, log);
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
        // Inits a fresh cluster or upgrades an existing one, never prompting;
        // --ifneeded no-ops when already current.
        match ops.run(
            &db.to_string_lossy(),
            &[es, "initorupgradenoprompt", "--ifneeded"],
        ) {
            Ok(()) => log.push(LogLine::new(
                Level::Info,
                "Initialized/upgraded the database".into(),
            )),
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
                "  Initialize/upgrade the DB:  {} {es} initorupgradenoprompt --ifneeded",
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

/// Demo single-node install (NOT for production). Elasticsearch uses the OSS
/// 7.10.2 package (no security); OpenSearch uses a 2.x bundle with the security
/// plugin disabled and single-node discovery appended to its config.
const ES_VERSION: &str = "7.10.2";
const OPENSEARCH_VERSION: &str = "2.19.1";

fn install_backend(backend: EsBackend, ops: &dyn SystemOps, log: &mut Vec<LogLine>) {
    let is_rpm =
        Path::new("/etc/redhat-release").exists() || Path::new("/etc/system-release").exists();
    let result = match backend {
        EsBackend::Elasticsearch => install_elasticsearch(is_rpm, ops, log),
        EsBackend::OpenSearch => install_opensearch(is_rpm, ops, log),
        EsBackend::None => return,
    };
    match result {
        Ok(()) => log.push(LogLine::new(
            Level::Info,
            format!("Installed single-node {} (demo)", backend.short()),
        )),
        Err(e) => log.push(LogLine::new(
            Level::Warn,
            format!("{} install: {e}", backend.short()),
        )),
    }
}

fn install_elasticsearch(is_rpm: bool, ops: &dyn SystemOps, log: &mut Vec<LogLine>) -> Result<()> {
    let (arch_rpm, arch_deb) = match std::env::consts::ARCH {
        "x86_64" => ("x86_64", "amd64"),
        "aarch64" => ("aarch64", "arm64"),
        other => {
            log.push(LogLine::new(
                Level::Warn,
                format!("unsupported arch {other}"),
            ));
            return Ok(());
        }
    };
    let base = "https://artifacts.elastic.co/downloads/elasticsearch";
    if is_rpm {
        ops.run(
            "yum",
            &[
                "install",
                "-y",
                &format!("{base}/elasticsearch-oss-{ES_VERSION}-{arch_rpm}.rpm"),
            ],
        )
    } else {
        let file = format!("elasticsearch-oss-{ES_VERSION}-{arch_deb}.deb");
        ops.run("curl", &["-sSfLO", &format!("{base}/{file}")])
            .and_then(|_| ops.run("dpkg", &["-i", &file]))
            .and_then(|_| ops.run("rm", &["-f", &file]))
    }
}

fn install_opensearch(is_rpm: bool, ops: &dyn SystemOps, log: &mut Vec<LogLine>) -> Result<()> {
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        other => {
            log.push(LogLine::new(
                Level::Warn,
                format!("unsupported arch {other}"),
            ));
            return Ok(());
        }
    };
    let base = "https://artifacts.opensearch.org/releases/bundle/opensearch";
    let ext = if is_rpm { "rpm" } else { "deb" };
    let file = format!("opensearch-{OPENSEARCH_VERSION}-linux-{arch}.{ext}");
    let url = format!("{base}/{OPENSEARCH_VERSION}/{file}");
    // Disable the security demo config during install, then force single-node +
    // security-disabled in opensearch.yml.
    let install = if is_rpm {
        format!("DISABLE_INSTALL_DEMO_CONFIG=true yum install -y {url}")
    } else {
        format!(
            "curl -sSfLO {url} && DISABLE_INSTALL_DEMO_CONFIG=true dpkg -i {file}; rm -f {file}"
        )
    };
    ops.run("sh", &["-c", &install]).and_then(|_| {
        ops.run(
            "sh",
            &[
                "-c",
                "printf 'discovery.type: single-node\\nplugins.security.disabled: true\\n' \
                 >> /etc/opensearch/opensearch.yml",
            ],
        )
    })
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
