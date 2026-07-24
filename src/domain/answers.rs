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
    /// Whether to stand up a single-node Elasticsearch we configure. In native
    /// mode this triggers a package install; in docker mode it adds a compose
    /// service (with a bind-mounted data dir — see `es_data_dir`).
    pub install_demo_es: bool,
    /// Host data directory for the docker single-node Elasticsearch (compose
    /// volume, not an env var). Only used when `install_demo_es` in docker mode.
    pub es_data_dir: String,
    /// Whether to download GeoIP files (native mode only).
    pub download_geoip: bool,
    /// `;`-separated capture plugin list (already finalized, incl. wise.so when
    /// the wise component is enabled). Empty means none.
    pub plugins: String,
    /// External WISE service URL, set only when the wise.so plugin is enabled
    /// without deploying the wise component locally. Empty means unset.
    pub wise_url: String,
}

impl Answers {
    /// Default ES URL used when the admin leaves the field blank, matching bash.
    pub const DEFAULT_ES_URL: &'static str = "https://localhost:9200";

    /// Default WISE URL suggested when configuring an external WISE service.
    pub const DEFAULT_WISE_URL: &'static str = "http://127.0.0.1:8081";

    /// Default host data dir for the docker single-node Elasticsearch.
    pub const DEFAULT_ES_DATA_DIR: &'static str = "/arkime/esdata";

    /// URL the arkime containers use to reach the single-node ES (host net,
    /// security disabled).
    pub const SINGLE_NODE_ES_URL: &'static str = "http://localhost:9200";

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
