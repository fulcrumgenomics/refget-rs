//! GA4GH service-info types.

use serde::{Deserialize, Serialize};

/// GA4GH service type descriptor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceType {
    pub group: String,
    pub artifact: String,
    pub version: String,
}

/// GA4GH service-info response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    #[serde(rename = "type")]
    pub service_type: ServiceType,
    pub version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub organization: Option<serde_json::Value>,
}

/// Extended service-info for the refget Sequences API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SequenceServiceInfo {
    #[serde(flatten)]
    pub service: ServiceInfo,
    pub refget: RefgetServiceDetails,
}

/// Details specific to refget service-info.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefgetServiceDetails {
    pub circular_supported: bool,
    pub algorithms: Vec<String>,
    pub identifier_types: Vec<String>,
}
