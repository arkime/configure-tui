//! In-memory documents and their two-way sync with the wizard.
//!
//! Each output file is held as editable text. `render_*` merges the wizard's
//! understood fields into a document while preserving everything we don't
//! understand (unknown ini keys, unknown env vars, unknown compose services /
//! keys). `parse_*` reads the understood fields back out, so hand-edits in the
//! (e) editor flow back into the wizard. Nothing is written to disk until the
//! final apply step.

use crate::config::substitute::{
    basic_auth_value, get_ini_key, render, set_ini_key, set_ini_key_opt, BasicAuthEncoding,
    Substitutions,
};
use crate::config::templates::{load_sample, SampleKind};
use crate::domain::mounts::MountKind;
use crate::domain::{Answers, Component, Components, MountSelection};
use serde_yml::{Mapping, Value};
use std::path::{Path, PathBuf};

/// Container images used when materializing docker services.
pub struct Images {
    pub arkime: String,
    pub elasticsearch: String,
}

impl Default for Images {
    fn default() -> Self {
        Images {
            arkime: "arkime/arkime:latest".into(),
            elasticsearch: "elasticsearch:8.19.19".into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocKind {
    ConfigIni,
    WiseIni,
    Cont3xtIni,
    Compose,
    Env,
}

impl DocKind {
    pub fn filename(self) -> &'static str {
        match self {
            DocKind::ConfigIni => "config.ini",
            DocKind::WiseIni => "wise.ini",
            DocKind::Cont3xtIni => "cont3xt.ini",
            DocKind::Compose => "docker-compose.yml",
            DocKind::Env => "arkime.env",
        }
    }

    fn sample(self) -> Option<SampleKind> {
        match self {
            DocKind::ConfigIni => Some(SampleKind::Config),
            DocKind::WiseIni => Some(SampleKind::Wise),
            DocKind::Cont3xtIni => Some(SampleKind::Cont3xt),
            _ => None,
        }
    }
}

/// One in-memory output file.
#[derive(Debug, Clone)]
pub struct Document {
    pub kind: DocKind,
    /// Where it will be written (native: <etc>/x.ini; docker: <out_dir>/x).
    pub path: PathBuf,
    pub text: String,
    /// The text at load/creation time — the baseline for the editor's diff view.
    pub original: String,
}

const ENV_FILE: &str = "arkime.env";

// ---------------------------------------------------------------------------
// INI
// ---------------------------------------------------------------------------

/// Fresh ini base: the sample with ARKIME_INSTALL_DIR substituted and field
/// placeholders blanked, ready for `render_ini` to fill.
pub fn fresh_ini_base(kind: DocKind, etc_dir: &Path, install_dir: &str) -> String {
    let sample_kind = kind.sample().expect("ini kind");
    let sample = load_sample(etc_dir, sample_kind);
    render(
        &sample,
        &Substitutions {
            interface: "",
            elasticsearch: "",
            password: "",
            install_dir,
        },
    )
}

/// Merge understood fields into an ini document, preserving unknown keys.
pub fn render_ini(
    kind: DocKind,
    base: &str,
    answers: &Answers,
    basic_auth: BasicAuthEncoding,
) -> String {
    let mut t = base.to_string();
    let auth = if answers.has_es_user() {
        Some(basic_auth_value(
            &answers.es_user,
            &answers.es_password,
            basic_auth,
        ))
    } else {
        None
    };
    match kind {
        DocKind::ConfigIni => {
            t = set_ini_key(&t, "interface", &answers.interfaces);
            t = set_ini_key(&t, "elasticsearch", answers.elasticsearch_or_default());
            t = set_ini_key(&t, "passwordSecret", &answers.s2s_password);
            if !answers.plugins.is_empty() {
                t = set_ini_key(&t, "plugins", &answers.plugins);
            }
            if !answers.wise_url.is_empty() {
                t = set_ini_key(&t, "wiseURL", &answers.wise_url);
            }
            if answers.enable_uploads {
                t = set_ini_key(&t, "uploadCommand", Answers::UPLOAD_COMMAND);
            }
            if let Some(a) = &auth {
                t = set_ini_key(&t, "elasticsearchBasicAuth", a);
            }
        }
        DocKind::WiseIni | DocKind::Cont3xtIni => {
            // Only touch keys already present; never append unrelated keys.
            t = set_ini_key_opt(
                &t,
                "elasticsearch",
                answers.elasticsearch_or_default(),
                false,
            );
            t = set_ini_key_opt(&t, "passwordSecret", &answers.s2s_password, false);
            if let Some(a) = &auth {
                t = set_ini_key_opt(&t, "elasticsearchBasicAuth", a, false);
            }
        }
        _ => {}
    }
    t
}

/// Read understood fields out of an ini document into `answers`.
pub fn parse_ini(kind: DocKind, text: &str, answers: &mut Answers) {
    if kind == DocKind::ConfigIni {
        if let Some(v) = get_ini_key(text, "interface") {
            answers.interfaces = v;
        }
        if let Some(v) = get_ini_key(text, "plugins") {
            answers.plugins = v;
        }
        if let Some(v) = get_ini_key(text, "wiseURL") {
            answers.wise_url = v;
        }
        answers.enable_uploads = get_ini_key(text, "uploadCommand").is_some();
    }
    if let Some(v) = get_ini_key(text, "elasticsearch") {
        answers.elasticsearch = v;
    }
    if let Some(v) = get_ini_key(text, "passwordSecret") {
        answers.s2s_password = v;
    }
    if let Some(v) = get_ini_key(text, "elasticsearchBasicAuth") {
        if let Some((user, pass)) = v.split_once(':') {
            answers.es_user = user.to_string();
            answers.es_password = pass.to_string();
        }
    }
}

// ---------------------------------------------------------------------------
// ENV (docker)
// ---------------------------------------------------------------------------

/// A getter that produces an env var's value from the answers (or None to omit).
type EnvGetter = fn(&Answers, BasicAuthEncoding) -> Option<String>;

fn env_pairs() -> [(&'static str, EnvGetter); 8] {
    [
        // Always drop privileges to `nobody` in the containers.
        ("ARKIME__dropUser", |_, _| Some("nobody".to_string())),
        ("ARKIME__uploadCommand", |a, _| {
            a.enable_uploads
                .then(|| Answers::UPLOAD_COMMAND.to_string())
        }),
        ("ARKIME__interface", |a, _| {
            (!a.interfaces.is_empty()).then(|| a.interfaces.clone())
        }),
        ("ARKIME__elasticsearch", |a, _| {
            // When we run the single-node ES, point at it (host net, no TLS).
            if a.install_demo_es {
                Some(Answers::SINGLE_NODE_ES_URL.to_string())
            } else {
                Some(a.elasticsearch_or_default().to_string())
            }
        }),
        ("ARKIME__elasticsearchBasicAuth", |a, enc| {
            a.has_es_user()
                .then(|| basic_auth_value(&a.es_user, &a.es_password, enc))
        }),
        ("ARKIME__passwordSecret", |a, _| {
            (!a.s2s_password.is_empty()).then(|| a.s2s_password.clone())
        }),
        ("ARKIME__plugins", |a, _| {
            (!a.plugins.is_empty()).then(|| a.plugins.clone())
        }),
        ("ARKIME__wiseURL", |a, _| {
            (!a.wise_url.is_empty()).then(|| a.wise_url.clone())
        }),
    ]
}

/// Merge understood `ARKIME__*` vars into an env file, preserving unknown vars.
pub fn render_env(base: &str, answers: &Answers, basic_auth: BasicAuthEncoding) -> String {
    let mut t = base.to_string();
    for (key, get) in env_pairs() {
        t = set_env_key(&t, key, get(answers, basic_auth).as_deref());
    }
    t
}

/// Read understood `ARKIME__*` vars back into `answers`.
pub fn parse_env(text: &str, answers: &mut Answers) {
    if let Some(v) = get_env_key(text, "ARKIME__interface") {
        answers.interfaces = v;
    }
    if let Some(v) = get_env_key(text, "ARKIME__elasticsearch") {
        answers.elasticsearch = v;
    }
    if let Some(v) = get_env_key(text, "ARKIME__passwordSecret") {
        answers.s2s_password = v;
    }
    if let Some(v) = get_env_key(text, "ARKIME__plugins") {
        answers.plugins = v;
    }
    if let Some(v) = get_env_key(text, "ARKIME__wiseURL") {
        answers.wise_url = v;
    }
    answers.enable_uploads = get_env_key(text, "ARKIME__uploadCommand").is_some();
    if let Some(v) = get_env_key(text, "ARKIME__elasticsearchBasicAuth") {
        if let Some((user, pass)) = v.split_once(':') {
            answers.es_user = user.to_string();
            answers.es_password = pass.to_string();
        }
    }
}

fn get_env_key(text: &str, key: &str) -> Option<String> {
    let needle = format!("{key}=");
    for line in text.lines() {
        let bare = line.trim_start();
        if bare.starts_with('#') {
            continue;
        }
        if let Some(rest) = bare.strip_prefix(&needle) {
            return Some(rest.trim().to_string());
        }
    }
    None
}

/// Set (`Some`), or remove (`None`), a `key=value` env line. Unknown lines and
/// comments are preserved in place.
fn set_env_key(text: &str, key: &str, value: Option<&str>) -> String {
    let needle = format!("{key}=");
    let mut lines: Vec<String> = Vec::new();
    let mut done = false;
    for line in text.lines() {
        let bare = line.trim_start();
        if !bare.starts_with('#') && bare.starts_with(&needle) {
            done = true;
            if let Some(v) = value {
                lines.push(format!("{key}={v}"));
            }
            // value == None -> drop the line (remove).
        } else {
            lines.push(line.to_string());
        }
    }
    let mut out = lines.join("\n");
    if !out.is_empty() {
        out.push('\n');
    }
    if !done {
        if let Some(v) = value {
            out.push_str(&format!("{key}={v}\n"));
        }
    }
    out
}

// ---------------------------------------------------------------------------
// COMPOSE (docker)
// ---------------------------------------------------------------------------

/// The default service-name prefix (`arkime-viewer`, `arkime-capture`, …).
pub const DEFAULT_PREFIX: &str = "arkime-";

fn arkime_service_name(prefix: &str, c: Component) -> String {
    format!("{prefix}{}", c.label())
}

/// What we learned about the arkime service naming in a loaded compose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrefixDetection {
    /// The prefix we'll manage (the one used by the most services).
    pub prefix: String,
    /// Any other prefixes also present — left untouched.
    pub others: Vec<String>,
}

/// Detect the prefix(es) the arkime services use. A service name counts if it
/// ends with a component label after an empty / `-` / `_` separator (so
/// `viewer`, `arkime-viewer`, `arkime6-viewer` all match, but `otherwise`
/// doesn't). Returns the dominant prefix plus any others found.
pub fn detect_prefix(text: &str) -> Option<PrefixDetection> {
    let root: Value = serde_yml::from_str(text).ok()?;
    let services = root.get("services").and_then(|v| v.as_mapping())?;

    let mut counts: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for key in services.keys() {
        let name = match key.as_str() {
            Some(n) => n,
            None => continue,
        };
        for c in Component::ALL {
            if let Some(stem) = name.strip_suffix(c.label()) {
                if stem.is_empty() || stem.ends_with('-') || stem.ends_with('_') {
                    *counts.entry(stem.to_string()).or_default() += 1;
                    break;
                }
            }
        }
    }
    if counts.is_empty() {
        return None;
    }
    // Dominant = most services (ties broken by BTreeMap order for determinism).
    let prefix = counts
        .iter()
        .max_by_key(|(_, n)| **n)
        .map(|(p, _)| p.clone())
        .unwrap();
    let others = counts.keys().filter(|p| **p != prefix).cloned().collect();
    Some(PrefixDetection { prefix, others })
}

/// Remove every arkime service using `prefix` from a compose document,
/// preserving everything else. Used to delete a prefix set.
pub fn remove_prefix_services(text: &str, prefix: &str) -> String {
    let mut root: Value = match serde_yml::from_str(text) {
        Ok(v) => v,
        Err(_) => return text.to_string(),
    };
    if let Some(services) = root.get_mut("services").and_then(|v| v.as_mapping_mut()) {
        for c in Component::ALL {
            services.remove(Value::from(arkime_service_name(prefix, c)));
        }
    }
    serde_yml::to_string(&root).unwrap_or_else(|_| text.to_string())
}

/// Merge our services into a compose document, preserving unknown services and
/// top-level keys. We own the five `arkime-*` services (their standard fields)
/// and add an `elasticsearch` service only when single-node mode is on and none
/// exists.
pub fn render_compose(
    base: &str,
    prefix: &str,
    components: &Components,
    answers: &Answers,
    mounts: &MountSelection,
    images: &Images,
) -> String {
    let mut root: Value = if base.trim().is_empty() {
        Value::Mapping(Mapping::new())
    } else {
        serde_yml::from_str(base).unwrap_or_else(|_| Value::Mapping(Mapping::new()))
    };
    if !root.is_mapping() {
        root = Value::Mapping(Mapping::new());
    }
    let root_map = root.as_mapping_mut().unwrap();

    let services = root_map
        .entry(Value::from("services"))
        .or_insert_with(|| Value::Mapping(Mapping::new()));
    let services = match services.as_mapping_mut() {
        Some(m) => m,
        None => {
            *services = Value::Mapping(Mapping::new());
            services.as_mapping_mut().unwrap()
        }
    };

    let es_depends = answers.install_demo_es || services.contains_key(Value::from("elasticsearch"));

    for c in Component::ALL {
        let key = Value::from(arkime_service_name(prefix, c));
        if components.contains(c) {
            let mut svc = services
                .get(&key)
                .and_then(|v| v.as_mapping().cloned())
                .unwrap_or_default();
            fill_arkime_service(&mut svc, c, images, es_depends, mounts, components);
            services.insert(key, Value::Mapping(svc));
        } else {
            services.remove(&key);
        }
    }

    // Single-node Elasticsearch we configure (added if not already present).
    if answers.install_demo_es && !services.contains_key(Value::from("elasticsearch")) {
        let data_dir = if answers.es_data_dir.is_empty() {
            Answers::DEFAULT_ES_DATA_DIR
        } else {
            &answers.es_data_dir
        };
        services.insert(
            Value::from("elasticsearch"),
            elasticsearch_service(images, data_dir),
        );
    }

    serde_yml::to_string(&root).unwrap_or_else(|e| format!("# serialization error: {e}\n"))
}

fn fill_arkime_service(
    svc: &mut Mapping,
    c: Component,
    images: &Images,
    es_depends: bool,
    mounts: &MountSelection,
    components: &Components,
) {
    set_str(svc, "image", &images.arkime);
    set_str(svc, "command", c.label());
    set_seq(svc, "env_file", &[ENV_FILE.to_string()]);
    set_str(svc, "restart", "unless-stopped");

    let binds = mounts.specs_for(c, components);
    if binds.is_empty() {
        svc.remove(Value::from("volumes"));
    } else {
        set_seq(svc, "volumes", &binds);
    }

    if es_depends {
        set_seq(svc, "depends_on", &["elasticsearch".to_string()]);
    } else {
        svc.remove(Value::from("depends_on"));
    }

    match c {
        Component::Capture => {
            set_str(svc, "network_mode", "host");
            set_seq(
                svc,
                "cap_add",
                &["NET_ADMIN".to_string(), "NET_RAW".to_string()],
            );
            svc.remove(Value::from("ports"));
        }
        // Viewer runs on host networking too so it can reach a host-net ES on
        // localhost:9200 (no port mapping needed with host net).
        Component::Viewer => {
            set_str(svc, "network_mode", "host");
            svc.remove(Value::from("cap_add"));
            svc.remove(Value::from("ports"));
        }
        Component::Wise => port(svc, "8081:8081"),
        Component::Parliament => port(svc, "8008:8008"),
        Component::Cont3xt => port(svc, "3218:3218"),
    }
}

/// A single-node Elasticsearch service, matching the shape Arkime deployments
/// use: host networking, security disabled, memlock unlimited, and a
/// bind-mounted data dir. Heap is intentionally NOT pinned — modern ES
/// auto-sizes the JVM heap from the container's memory.
fn elasticsearch_service(images: &Images, data_dir: &str) -> Value {
    let mut m = Mapping::new();
    set_str(&mut m, "container_name", "elasticsearch");
    set_str(&mut m, "image", &images.elasticsearch);
    set_str(&mut m, "network_mode", "host");
    set_seq(
        &mut m,
        "environment",
        &[
            "discovery.type=single-node".to_string(),
            "xpack.security.enabled=false".to_string(),
            "xpack.security.enrollment.enabled=false".to_string(),
            "xpack.security.transport.ssl.enabled=false".to_string(),
            "xpack.security.http.ssl.enabled=false".to_string(),
        ],
    );
    // ulimits: { memlock: { soft: -1, hard: -1 } }
    let mut memlock = Mapping::new();
    memlock.insert(Value::from("soft"), Value::from(-1));
    memlock.insert(Value::from("hard"), Value::from(-1));
    let mut ulimits = Mapping::new();
    ulimits.insert(Value::from("memlock"), Value::Mapping(memlock));
    m.insert(Value::from("ulimits"), Value::Mapping(ulimits));

    set_seq(
        &mut m,
        "volumes",
        &[format!("{data_dir}:/usr/share/elasticsearch/data")],
    );
    set_str(&mut m, "restart", "unless-stopped");
    Value::Mapping(m)
}

fn set_str(map: &mut Mapping, key: &str, value: &str) {
    map.insert(Value::from(key), Value::from(value));
}

fn set_seq(map: &mut Mapping, key: &str, items: &[String]) {
    let seq = items.iter().map(|s| Value::from(s.clone())).collect();
    map.insert(Value::from(key), Value::Sequence(seq));
}

fn port(svc: &mut Mapping, p: &str) {
    svc.remove(Value::from("network_mode"));
    svc.remove(Value::from("cap_add"));
    set_seq(svc, "ports", &[p.to_string()]);
}

/// Read understood structure out of a compose document: which of our services
/// exist (-> components), demo backend presence, and capture/viewer mounts.
pub fn parse_compose(
    text: &str,
    prefix: &str,
    components: &mut Components,
    answers: &mut Answers,
    mounts: &mut MountSelection,
) {
    let root: Value = match serde_yml::from_str(text) {
        Ok(v) => v,
        Err(_) => return,
    };
    let services = match root.get("services").and_then(|v| v.as_mapping()) {
        Some(m) => m,
        None => return,
    };

    for c in Component::ALL {
        let present = services.contains_key(Value::from(arkime_service_name(prefix, c)));
        set_component(components, c, present);
    }
    answers.install_demo_es = services.contains_key(Value::from("elasticsearch"));
    // Recover the ES data dir from its bind mount.
    if let Some(es) = services
        .get(Value::from("elasticsearch"))
        .and_then(|v| v.as_mapping())
    {
        if let Some(host) = es
            .get(Value::from("volumes"))
            .and_then(|v| v.as_sequence())
            .and_then(|s| s.iter().filter_map(|v| v.as_str()).next())
            .and_then(|v| v.strip_suffix(":/usr/share/elasticsearch/data"))
        {
            answers.es_data_dir = host.to_string();
        }
    }

    // Mounts from whichever of capture/viewer exists.
    let svc = services
        .get(Value::from(arkime_service_name(prefix, Component::Capture)))
        .or_else(|| services.get(Value::from(arkime_service_name(prefix, Component::Viewer))))
        .and_then(|v| v.as_mapping());
    if let Some(svc) = svc {
        let vols: Vec<String> = svc
            .get(Value::from("volumes"))
            .and_then(|v| v.as_sequence())
            .map(|s| {
                s.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        for kind in MountKind::ALL {
            if let Some(host) = vols.iter().find_map(|v| kind.host_of(v)) {
                mounts.set_enabled(kind, true);
                mounts.set_host(kind, host);
            } else {
                mounts.set_enabled(kind, false);
            }
        }
    }
}

fn set_component(components: &mut Components, c: Component, on: bool) {
    if components.contains(c) != on {
        components.toggle(c);
    }
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
            plugins: "wise.so".into(),
            ..Default::default()
        }
    }

    #[test]
    fn ini_preserves_unknown_keys() {
        let base =
            "elasticsearch=OLD\ncustomThing=keepme\n# comment\ninterface=OLD\npasswordSecret=OLD\n";
        let out = render_ini(
            DocKind::ConfigIni,
            base,
            &answers(),
            BasicAuthEncoding::Plaintext,
        );
        assert!(out.contains("customThing=keepme"));
        assert!(out.contains("# comment"));
        assert!(out.contains("interface=eth0"));
        assert!(out.contains("elasticsearch=https://os:9200"));
        assert!(out.contains("elasticsearchBasicAuth=admin:pass"));
    }

    #[test]
    fn ini_round_trips_into_answers() {
        let text = "interface=eth5;eth6\nelasticsearch=https://prod:9200\npasswordSecret=zzz\nelasticsearchBasicAuth=u:p\nplugins=entropy.so\n";
        let mut a = Answers::default();
        parse_ini(DocKind::ConfigIni, text, &mut a);
        assert_eq!(a.interfaces, "eth5;eth6");
        assert_eq!(a.elasticsearch, "https://prod:9200");
        assert_eq!(a.s2s_password, "zzz");
        assert_eq!(a.es_user, "u");
        assert_eq!(a.es_password, "p");
        assert_eq!(a.plugins, "entropy.so");
    }

    #[test]
    fn uploads_toggle_sets_upload_command() {
        let mut a = answers();
        a.enable_uploads = true;
        let env = render_env("", &a, BasicAuthEncoding::Plaintext);
        assert!(env.contains(
            "ARKIME__uploadCommand=/opt/arkime/bin/capture --copy -n {NODE} -r {TMPFILE} {TAGS}"
        ));
        let ini = render_ini(
            DocKind::ConfigIni,
            "passwordSecret=x\n",
            &a,
            BasicAuthEncoding::Plaintext,
        );
        assert!(ini.contains("uploadCommand=/opt/arkime/bin/capture"));

        a.enable_uploads = false;
        assert!(!render_env("", &a, BasicAuthEncoding::Plaintext).contains("uploadCommand"));
    }

    #[test]
    fn env_always_drops_to_nobody() {
        let out = render_env("", &Answers::default(), BasicAuthEncoding::Plaintext);
        assert!(out.contains("ARKIME__dropUser=nobody"));
    }

    #[test]
    fn env_preserves_unknown_vars() {
        let base = "ARKIME__elasticsearch=OLD\nMY_CUSTOM=keep\n";
        let out = render_env(base, &answers(), BasicAuthEncoding::Plaintext);
        assert!(out.contains("MY_CUSTOM=keep"));
        assert!(out.contains("ARKIME__elasticsearch=https://os:9200"));
        assert!(out.contains("ARKIME__interface=eth0"));
    }

    #[test]
    fn env_removes_key_when_value_cleared() {
        let base = "ARKIME__plugins=old.so\nKEEP=1\n";
        let mut a = answers();
        a.plugins = String::new();
        let out = render_env(base, &a, BasicAuthEncoding::Plaintext);
        assert!(!out.contains("ARKIME__plugins"));
        assert!(out.contains("KEEP=1"));
    }

    #[test]
    fn compose_preserves_unknown_service_and_owns_ours() {
        let base = "services:\n  myextra:\n    image: busybox\n";
        let mut components = Components {
            capture: true,
            ..Default::default()
        };
        let out = render_compose(
            base,
            DEFAULT_PREFIX,
            &components,
            &answers(),
            &MountSelection::default(),
            &Images::default(),
        );
        assert!(out.contains("myextra"));
        assert!(out.contains("arkime-capture"));
        assert!(out.contains("network_mode: host"));

        // Round-trip: parse back detects our service.
        let mut parsed = Components::default();
        let mut a = Answers::default();
        let mut m = MountSelection::default();
        parse_compose(&out, DEFAULT_PREFIX, &mut parsed, &mut a, &mut m);
        assert!(parsed.capture);
        // Toggling capture off removes only our service.
        components.capture = false;
        let out2 = render_compose(
            &out,
            DEFAULT_PREFIX,
            &components,
            &answers(),
            &MountSelection::default(),
            &Images::default(),
        );
        assert!(out2.contains("myextra"));
        assert!(!out2.contains("arkime-capture"));
    }

    #[test]
    fn single_node_es_service_and_data_dir_round_trip() {
        let components = Components {
            capture: true,
            ..Default::default()
        };
        let a = Answers {
            install_demo_es: true,
            es_data_dir: "/esdata".into(),
            ..answers()
        };
        let out = render_compose(
            &out_default(&components, &a),
            DEFAULT_PREFIX,
            &components,
            &a,
            &MountSelection::default(),
            &Images::default(),
        );
        assert!(out.contains("container_name: elasticsearch"));
        assert!(out.contains("image: elasticsearch:8.19.19"));
        assert!(out.contains("network_mode: host"));
        assert!(out.contains("discovery.type=single-node"));
        assert!(out.contains("xpack.security.enabled=false"));
        assert!(out.contains("/esdata:/usr/share/elasticsearch/data"));
        assert!(out.contains("memlock"));
        // Heap is auto-sized — we don't pin ES_JAVA_OPTS.
        assert!(!out.contains("ES_JAVA_OPTS"));

        // Env points the containers at the single-node ES over localhost.
        let env = render_env("", &a, BasicAuthEncoding::Plaintext);
        assert!(env.contains("ARKIME__elasticsearch=http://localhost:9200"));

        // Round-trip: parse recovers the flag + data dir.
        let mut c = Components::default();
        let mut ans = Answers::default();
        let mut m = MountSelection::default();
        parse_compose(&out, DEFAULT_PREFIX, &mut c, &mut ans, &mut m);
        assert!(ans.install_demo_es);
        assert_eq!(ans.es_data_dir, "/esdata");
    }

    #[test]
    fn viewer_runs_on_host_networking() {
        let components = Components {
            viewer: true,
            ..Default::default()
        };
        let out = out_default(&components, &answers());
        // The viewer service uses host net and no published ports.
        let viewer = out.split("arkime-viewer:").nth(1).unwrap();
        assert!(viewer.contains("network_mode: host"));
        assert!(!viewer.contains("ports:"));
    }

    #[test]
    fn detect_prefix_handles_variants() {
        let two = "services:\n  arkime-viewer:\n    image: x\n  arkime-capture:\n    image: y\n";
        let d = detect_prefix(two).unwrap();
        assert_eq!(d.prefix, "arkime-");
        assert!(d.others.is_empty());

        let bare = "services:\n  viewer:\n    image: x\n";
        assert_eq!(detect_prefix(bare).unwrap().prefix, "");

        let custom = "services:\n  arkime6-viewer:\n    image: x\n";
        assert_eq!(detect_prefix(custom).unwrap().prefix, "arkime6-");

        // Two prefixes: dominant wins, the other is reported.
        let mixed = "services:\n  arkime-viewer:\n    image: x\n  arkime-capture:\n    image: y\n  arkime6-wise:\n    image: z\n";
        let d = detect_prefix(mixed).unwrap();
        assert_eq!(d.prefix, "arkime-");
        assert_eq!(d.others, vec!["arkime6-".to_string()]);

        assert!(detect_prefix("services:\n  postgres:\n    image: p\n").is_none());
    }

    #[test]
    fn remove_prefix_services_strips_only_that_prefix() {
        let base = "services:\n  arkime-capture:\n    image: x\n  arkime2-capture:\n    image: y\n  other:\n    image: z\n";
        let out = remove_prefix_services(base, "arkime2-");
        assert!(!out.contains("arkime2-capture"));
        assert!(out.contains("arkime-capture"));
        assert!(out.contains("other"));
    }

    #[test]
    fn render_respects_prefix_and_leaves_others_untouched() {
        let base = "services:\n  arkime6-viewer:\n    image: keep-me\n";
        let components = Components {
            viewer: true,
            ..Default::default()
        };
        // We manage the "arkime-" set; the arkime6- service must survive.
        let out = render_compose(
            base,
            "arkime-",
            &components,
            &answers(),
            &MountSelection::default(),
            &Images::default(),
        );
        assert!(out.contains("arkime6-viewer"));
        assert!(out.contains("keep-me"));
        assert!(out.contains("arkime-viewer:"));

        // Parsing with the same prefix sees our service, not the other one.
        let mut c = Components::default();
        let mut a = Answers::default();
        let mut m = MountSelection::default();
        parse_compose(&out, "arkime-", &mut c, &mut a, &mut m);
        assert!(c.viewer);
    }

    fn out_default(components: &Components, a: &Answers) -> String {
        render_compose(
            "",
            DEFAULT_PREFIX,
            components,
            a,
            &MountSelection::default(),
            &Images::default(),
        )
    }
}
