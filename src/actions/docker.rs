//! Docker-deployment generator. Emits a `docker-compose.yml` plus an
//! `arkime.env` file of `ARKIME__*` variables (the `docker.sh` convention).
//!
//! Deliberately writes NO `.ini` files — docker mode configures Arkime purely
//! through environment variables consumed by the containers.

use crate::actions::system::SystemOps;
use crate::config::substitute::{basic_auth_value, BasicAuthEncoding};
use crate::domain::{Answers, Component, Components};
use crate::log::{Level, LogLine};
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::Path;

/// Container images, overridable later; sensible defaults for now.
pub struct Images {
    pub arkime: String,
    pub opensearch: String,
}

impl Default for Images {
    fn default() -> Self {
        Images {
            arkime: "arkime/arkime:latest".into(),
            opensearch: "opensearchproject/opensearch:2".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedDocker {
    pub compose_yaml: String,
    pub env_file: String,
}

#[derive(Serialize)]
struct Compose {
    services: BTreeMap<String, Service>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    volumes: BTreeMap<String, BTreeMap<String, String>>,
}

#[derive(Serialize, Default)]
struct Service {
    image: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    command: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    network_mode: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    cap_add: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    env_file: Vec<String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    environment: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    volumes: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    ports: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    depends_on: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    restart: Option<String>,
}

const ENV_FILE: &str = "arkime.env";

/// Build the compose + env text for the current selections.
pub fn generate(
    components: &Components,
    answers: &Answers,
    images: &Images,
    basic_auth: BasicAuthEncoding,
) -> GeneratedDocker {
    let mut services: BTreeMap<String, Service> = BTreeMap::new();
    let mut volumes: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();

    // Demo backend -> add an opensearch service; other components depend on it.
    let es_depends: Vec<String> = if answers.install_demo_es {
        services.insert("opensearch".into(), opensearch_service(images));
        volumes.insert("osdata".into(), BTreeMap::new());
        vec!["opensearch".into()]
    } else {
        Vec::new()
    };

    for c in components.enabled() {
        let (name, svc) = arkime_service(c, images, &es_depends);
        services.insert(name, svc);
    }

    let compose = Compose { services, volumes };
    let compose_yaml =
        serde_yml::to_string(&compose).unwrap_or_else(|e| format!("# serialization error: {e}\n"));

    GeneratedDocker {
        compose_yaml,
        env_file: env_file(components, answers, basic_auth),
    }
}

fn arkime_service(c: Component, images: &Images, es_depends: &[String]) -> (String, Service) {
    let mut svc = Service {
        image: images.arkime.clone(),
        command: Some(c.label().to_string()),
        env_file: vec![ENV_FILE.into()],
        depends_on: es_depends.to_vec(),
        restart: Some("unless-stopped".into()),
        ..Default::default()
    };

    match c {
        Component::Capture => {
            // Capture needs host networking + raw-socket capabilities.
            svc.network_mode = Some("host".into());
            svc.cap_add = vec!["NET_ADMIN".into(), "NET_RAW".into()];
            svc.volumes = vec![
                "./raw:/opt/arkime/raw".into(),
                "./logs:/opt/arkime/logs".into(),
            ];
        }
        Component::Viewer => svc.ports = vec!["8005:8005".into()],
        Component::Wise => svc.ports = vec!["8081:8081".into()],
        Component::Parliament => svc.ports = vec!["8008:8008".into()],
        Component::Cont3xt => svc.ports = vec!["3218:3218".into()],
    }

    (format!("arkime-{}", c.label()), svc)
}

fn opensearch_service(images: &Images) -> Service {
    let mut environment = BTreeMap::new();
    environment.insert("discovery.type".into(), "single-node".into());
    environment.insert("bootstrap.memory_lock".into(), "true".into());
    environment.insert("DISABLE_SECURITY_PLUGIN".into(), "true".into());
    Service {
        image: images.opensearch.clone(),
        environment,
        volumes: vec!["osdata:/usr/share/opensearch/data".into()],
        ports: vec!["9200:9200".into()],
        restart: Some("unless-stopped".into()),
        ..Default::default()
    }
}

/// The `ARKIME__*` env file. Keys are emitted in sorted order for deterministic
/// output.
fn env_file(components: &Components, answers: &Answers, basic_auth: BasicAuthEncoding) -> String {
    let mut vars: BTreeMap<String, String> = BTreeMap::new();

    if components.needs_elasticsearch() {
        // When standing up the demo backend, point at the compose service.
        let es = if answers.install_demo_es {
            "http://opensearch:9200".to_string()
        } else {
            answers.elasticsearch_or_default().to_string()
        };
        vars.insert("ARKIME__elasticsearch".into(), es);
        if answers.has_es_user() {
            vars.insert(
                "ARKIME__elasticsearchBasicAuth".into(),
                basic_auth_value(&answers.es_user, &answers.es_password, basic_auth),
            );
        }
    }
    if components.needs_interfaces() {
        vars.insert("ARKIME__interface".into(), answers.interfaces.clone());
    }
    if components.needs_s2s_password() {
        vars.insert(
            "ARKIME__passwordSecret".into(),
            answers.s2s_password.clone(),
        );
    }

    let mut out = String::new();
    for (k, v) in vars {
        out.push_str(&format!("{k}={v}\n"));
    }
    out
}

/// Write the generated files into `out_dir`, logging each. Docker mode never
/// writes `.ini`.
pub fn apply(ops: &dyn SystemOps, out_dir: &Path, generated: &GeneratedDocker) -> Vec<LogLine> {
    let mut log = Vec::new();
    let compose_path = out_dir.join("docker-compose.yml");
    let env_path = out_dir.join(ENV_FILE);

    match ops.write_file(&compose_path, &generated.compose_yaml, 0o644) {
        Ok(()) => log.push(LogLine::new(
            Level::Info,
            format!("Wrote {}", compose_path.display()),
        )),
        Err(e) => log.push(LogLine::new(Level::Error, format!("writing compose: {e}"))),
    }
    match ops.write_file(&env_path, &generated.env_file, 0o600) {
        Ok(()) => log.push(LogLine::new(
            Level::Info,
            format!("Wrote {}", env_path.display()),
        )),
        Err(e) => log.push(LogLine::new(Level::Error, format!("writing env: {e}"))),
    }
    log.push(LogLine::new(
        Level::Info,
        format!("Run `docker compose up -d` from {}", out_dir.display()),
    ));
    log
}

#[cfg(test)]
mod tests {
    use super::*;

    fn answers() -> Answers {
        Answers {
            interfaces: "eth0".into(),
            elasticsearch: "https://os:9200".into(),
            es_user: "admin".into(),
            es_password: "pass".into(),
            s2s_password: "secret".into(),
            ..Default::default()
        }
    }

    #[test]
    fn capture_service_has_host_net_and_caps() {
        let components = Components {
            capture: true,
            ..Default::default()
        };
        let g = generate(
            &components,
            &answers(),
            &Images::default(),
            BasicAuthEncoding::Plaintext,
        );
        assert!(g.compose_yaml.contains("arkime-capture:"));
        assert!(g.compose_yaml.contains("network_mode: host"));
        assert!(g.compose_yaml.contains("NET_ADMIN"));
        assert!(g.compose_yaml.contains("NET_RAW"));
        // No .ini anywhere in docker output.
        assert!(!g.compose_yaml.contains(".ini"));
    }

    #[test]
    fn env_file_has_expected_arkime_vars() {
        let components = Components {
            capture: true,
            viewer: true,
            ..Default::default()
        };
        let g = generate(
            &components,
            &answers(),
            &Images::default(),
            BasicAuthEncoding::Plaintext,
        );
        assert_eq!(
            g.env_file,
            "ARKIME__elasticsearch=https://os:9200\n\
             ARKIME__elasticsearchBasicAuth=admin:pass\n\
             ARKIME__interface=eth0\n\
             ARKIME__passwordSecret=secret\n"
        );
    }

    #[test]
    fn demo_es_adds_opensearch_service_and_points_env_at_it() {
        let components = Components {
            viewer: true,
            ..Default::default()
        };
        let a = Answers {
            install_demo_es: true,
            ..answers()
        };
        let g = generate(
            &components,
            &a,
            &Images::default(),
            BasicAuthEncoding::Plaintext,
        );
        assert!(g.compose_yaml.contains("opensearch:"));
        assert!(g.compose_yaml.contains("osdata"));
        assert!(g
            .env_file
            .contains("ARKIME__elasticsearch=http://opensearch:9200"));
    }

    #[test]
    fn base64_basic_auth_in_env() {
        let components = Components {
            viewer: true,
            ..Default::default()
        };
        let g = generate(
            &components,
            &answers(),
            &Images::default(),
            BasicAuthEncoding::Base64,
        );
        // base64("admin:pass")
        assert!(g
            .env_file
            .contains("ARKIME__elasticsearchBasicAuth=YWRtaW46cGFzcw=="));
    }
}
