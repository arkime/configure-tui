//! Everything the wizard collects from the admin. Filled in step by step; read
//! by the templating (native) and docker generators at Apply time.

#[derive(Debug, Clone, Default)]
pub struct Answers {
    /// Semicolon-separated interface list, matching Arkime's `interface=` value.
    pub interfaces: String,
    /// OpenSearch/Elasticsearch server URL.
    pub elasticsearch: String,
    /// Optional ES user (empty = no basic auth).
    pub es_user: String,
    /// Optional ES password (only meaningful when `es_user` is set).
    pub es_password: String,
    /// S2S / encryption secret (`passwordSecret`). Required for capture/viewer.
    pub s2s_password: String,
    /// Whether to stand up a local demo OpenSearch/Elasticsearch. In native mode
    /// this triggers a package install; in docker mode it adds a compose service.
    pub install_demo_es: bool,
    /// Whether to download GeoIP files (native mode only).
    pub download_geoip: bool,
}

impl Answers {
    /// Default ES URL used when the admin leaves the field blank, matching bash.
    pub const DEFAULT_ES_URL: &'static str = "https://localhost:9200";

    pub fn has_es_user(&self) -> bool {
        !self.es_user.is_empty()
    }

    /// Resolve the ES URL, applying the bash default for an empty entry.
    pub fn elasticsearch_or_default(&self) -> &str {
        if self.elasticsearch.is_empty() {
            Self::DEFAULT_ES_URL
        } else {
            &self.elasticsearch
        }
    }
}
