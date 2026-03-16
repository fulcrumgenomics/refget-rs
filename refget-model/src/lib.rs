//! Domain types for GA4GH refget Sequences v2.0.0 and Sequence Collections v1.0.0.

mod seqcol;
mod sequence;
mod service_info;

pub use seqcol::{
    ArrayElementComparison, AttributeComparison, ComparisonResult, Level, SeqCol, SeqColLevel1,
    compare,
};
pub use sequence::{Alias, SequenceMetadata};
pub use service_info::{SequenceServiceInfo, ServiceInfo, ServiceType};
