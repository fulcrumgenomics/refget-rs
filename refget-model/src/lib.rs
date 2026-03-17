//! Domain types for GA4GH refget Sequences v2.0.0 and Sequence Collections v1.0.0.

mod seqcol;
mod sequence;
mod service_info;

pub use seqcol::{
    ArrayElementComparison, AttributeComparison, ComparisonResult, Level, SeqCol, SeqColLevel1,
    compare,
};
pub use sequence::{Alias, SequenceMetadata};
pub use service_info::{RefgetServiceDetails, SequenceServiceInfo, ServiceInfo, ServiceType};

use serde::{Deserialize, Serialize};

/// A structured JSON error response body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorResponse {
    /// The HTTP status code.
    pub status_code: u16,
    /// A human-readable error message.
    pub message: String,
}
