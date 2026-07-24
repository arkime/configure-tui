//! Native-deployment apply step: template `.ini` files, create dirs, write
//! limits.d, and enable/start services via the platform's service manager.
//! Mirrors the bash `Configure` default/wise/cont3xt flows.

use crate::actions::system::SystemOps;
use crate::config::substitute::{inject_basic_auth, render, BasicAuthEncoding, Substitutions};
use crate::config::templates::{load_sample, SampleKind};
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

/// Run the full native flow. Never returns Err: each step logs its own outcome
/// so one failed service does not abort the rest (matching the forgiving bash
/// behavior). The returned log drives the Progress screen.
pub fn apply(
    ops: &dyn SystemOps,
    build: &BuildConfig,
    platform: Platform,
    components: &Components,
    answers: &Answers,
    basic_auth: BasicAuthEncoding,
) -> Vec<LogLine> {
    let mut log = Vec::new();
    let etc = build.etc_dir();

    // 1. Config files for the enabled components.
    if components.capture || components.viewer {
        write_ini(
            ops,
            &etc,
            SampleKind::Config,
            build,
            answers,
            basic_auth,
            &mut log,
        );
    }
    if components.wise {
        write_ini(
            ops,
            &etc,
            SampleKind::Wise,
            build,
            answers,
            basic_auth,
            &mut log,
        );
    }
    if components.cont3xt {
        write_ini(
            ops,
            &etc,
            SampleKind::Cont3xt,
            build,
            answers,
            basic_auth,
            &mut log,
        );
    }
    // Parliament has no templated .ini in the bash flow — service only.

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
        install_demo_es(ops, &mut log);
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

    // 5. Services.
    for c in components.enabled() {
        enable_start_service(ops, platform, c, &mut log);
    }

    log.push(LogLine::new(
        Level::Info,
        format!(
            "Done. Continue with the remaining steps in {}/README.txt",
            build.install_dir.display()
        ),
    ));
    log
}

fn write_ini(
    ops: &dyn SystemOps,
    etc: &Path,
    kind: SampleKind,
    build: &BuildConfig,
    answers: &Answers,
    basic_auth: BasicAuthEncoding,
    log: &mut Vec<LogLine>,
) {
    let sample = load_sample(etc, kind);
    let install_dir = build.install_dir.to_string_lossy();
    let mut rendered = render(
        &sample,
        &Substitutions {
            interface: &answers.interfaces,
            elasticsearch: answers.elasticsearch_or_default(),
            password: &answers.s2s_password,
            install_dir: &install_dir,
        },
    );
    if answers.has_es_user() {
        inject_basic_auth(
            &mut rendered,
            &answers.es_user,
            &answers.es_password,
            basic_auth,
        );
    }

    let out = etc.join(format!("{}.ini", kind.base()));
    match ops.write_new(&out, &rendered) {
        Ok(true) => log.push(LogLine::new(
            Level::Info,
            format!("Not overwriting existing {}", out.display()),
        )),
        Ok(false) => log.push(LogLine::new(
            Level::Info,
            format!("Wrote {}", out.display()),
        )),
        Err(e) => log.push(LogLine::new(
            Level::Error,
            format!("writing {}: {e}", out.display()),
        )),
    }
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

        let _ = apply(
            &ops,
            &build(),
            platform_systemd(),
            &components,
            &answers,
            BasicAuthEncoding::Plaintext,
        );

        let recorded = ops.ops();
        // config.ini written (once), logs+raw dirs created, both services started.
        assert!(recorded
            .iter()
            .any(|o| matches!(o, Op::WriteNew { path, .. } if path.ends_with("config.ini"))));
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

        let _ = apply(
            &ops,
            &build(),
            platform,
            &components,
            &answers,
            BasicAuthEncoding::Plaintext,
        );

        let recorded = ops.ops();
        assert!(recorded.iter().any(|o| matches!(o, Op::Run { program, args } if program == "sysrc" && args == &["arkimecapture_enable=YES"])));
        assert!(recorded.iter().any(|o| matches!(o, Op::Run { program, args } if program == "service" && args == &["arkimecapture", "start"])));
    }
}
