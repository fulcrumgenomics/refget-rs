//! Sequence metadata types for the refget Sequences API.

use serde::{Deserialize, Serialize};

/// A naming authority alias for a sequence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Alias {
    /// The naming authority (e.g. "insdc", "ensembl").
    pub naming_authority: String,
    /// The identifier value within that authority.
    pub value: String,
}

/// Metadata for a single sequence, as returned by the `/sequence/{digest}/metadata` endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SequenceMetadata {
    /// MD5 hex digest of the sequence.
    pub md5: String,
    /// GA4GH sha512t24u digest of the sequence.
    #[serde(rename = "ga4gh")]
    pub sha512t24u: String,
    /// Length of the sequence in bases.
    pub length: u64,
    /// Known aliases for this sequence.
    pub aliases: Vec<Alias>,
}
