//! The datastore Arkime talks to: an existing/external cluster, or a single-node
//! OpenSearch or Elasticsearch we stand up (a compose service in docker, a demo
//! package install in native).

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EsBackend {
    /// Use an existing/external cluster at the entered URL.
    #[default]
    None,
    OpenSearch,
    Elasticsearch,
}

impl EsBackend {
    pub const ALL: [EsBackend; 3] = [
        EsBackend::None,
        EsBackend::OpenSearch,
        EsBackend::Elasticsearch,
    ];

    /// Whether we stand up a single-node backend (vs use an external one).
    pub fn is_some(self) -> bool {
        self != EsBackend::None
    }

    /// Cycle None -> OpenSearch -> Elasticsearch -> None.
    pub fn cycle(self) -> EsBackend {
        match self {
            EsBackend::None => EsBackend::OpenSearch,
            EsBackend::OpenSearch => EsBackend::Elasticsearch,
            EsBackend::Elasticsearch => EsBackend::None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            EsBackend::None => "none — use an existing/external cluster",
            EsBackend::OpenSearch => "OpenSearch — run a single-node one",
            EsBackend::Elasticsearch => "Elasticsearch — run a single-node one",
        }
    }

    pub fn short(self) -> &'static str {
        match self {
            EsBackend::None => "none",
            EsBackend::OpenSearch => "opensearch",
            EsBackend::Elasticsearch => "elasticsearch",
        }
    }

    /// The compose service name (also the container_name).
    pub fn service_name(self) -> Option<&'static str> {
        match self {
            EsBackend::None => None,
            EsBackend::OpenSearch => Some("opensearch"),
            EsBackend::Elasticsearch => Some("elasticsearch"),
        }
    }

    /// The container data directory to bind-mount.
    pub fn data_path(self) -> Option<&'static str> {
        match self {
            EsBackend::None => None,
            EsBackend::OpenSearch => Some("/usr/share/opensearch/data"),
            EsBackend::Elasticsearch => Some("/usr/share/elasticsearch/data"),
        }
    }

    /// Recover the backend from a compose service name.
    pub fn from_service(name: &str) -> EsBackend {
        match name {
            "opensearch" => EsBackend::OpenSearch,
            "elasticsearch" => EsBackend::Elasticsearch,
            _ => EsBackend::None,
        }
    }
}
