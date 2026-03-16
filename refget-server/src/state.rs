//! Shared application state for the refget server.

use std::sync::Arc;

use refget_store::{SeqColStore, SequenceStore};
use serde::{Deserialize, Serialize};

/// Configuration for the refget server.
///
/// Loaded from a YAML config file via `--config` or constructed with defaults.
///
/// # Example YAML
///
/// ```yaml
/// # Required refget protocol settings
/// circular_supported: true
/// algorithms:
///   - md5
///   - ga4gh
/// subsequence_limit: 0  # 0 = no limit
///
/// # Sequences to treat as circular (by FASTA name)
/// circular_sequences:
///   - NC_001422.1
///   - chrM
///
/// # Optional GA4GH service-info fields
/// service_info:
///   organization:
///     name: "My Organization"
///     url: "https://example.org"
///   contact_url: "mailto:admin@example.org"
///   documentation_url: "https://example.org/docs"
///   environment: "production"
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RefgetConfig {
    /// Whether circular sequence retrieval is supported.
    pub circular_supported: bool,
    /// Supported hash algorithms.
    pub algorithms: Vec<String>,
    /// Maximum length of a subsequence request (0 = no limit).
    pub subsequence_limit: u64,
    /// Sequence names that should be treated as circular.
    pub circular_sequences: Vec<String>,
    /// Optional GA4GH service-info fields.
    pub service_info: ServiceInfoConfig,
}

/// Optional GA4GH service-info metadata fields.
///
/// When set, these are included in the `/sequence/service-info` response.
/// All fields are optional; unset fields are omitted from the response.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ServiceInfoConfig {
    /// Organization that runs this service.
    pub organization: Option<OrganizationConfig>,
    /// URL to contact the service operator.
    pub contact_url: Option<String>,
    /// URL to documentation for this service.
    pub documentation_url: Option<String>,
    /// Deployment environment (e.g., "production", "staging").
    pub environment: Option<String>,
}

/// Organization metadata for service-info.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrganizationConfig {
    /// Organization name.
    pub name: String,
    /// Organization URL.
    pub url: String,
}

impl Default for RefgetConfig {
    fn default() -> Self {
        Self {
            circular_supported: true,
            algorithms: vec!["md5".to_string(), "ga4gh".to_string(), "trunc512".to_string()],
            subsequence_limit: 0,
            circular_sequences: vec![],
            service_info: ServiceInfoConfig::default(),
        }
    }
}

/// Shared application state passed to all handlers.
#[derive(Clone)]
pub struct RefgetState {
    pub sequence_store: Arc<dyn SequenceStore>,
    pub seqcol_store: Arc<dyn SeqColStore>,
    pub config: RefgetConfig,
}
