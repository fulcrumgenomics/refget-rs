//! Handlers for the refget Sequences v2.0.0 API.

use axum::Router;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Deserialize;

use crate::RefgetState;

/// GA4GH refget v2 content type for JSON responses.
const REFGET_JSON_CONTENT_TYPE: &str = "application/vnd.ga4gh.refget.v2.0.0+json";

/// GA4GH refget v2 content type for sequence (plain text) responses.
const REFGET_PLAIN_CONTENT_TYPE: &str = "text/vnd.ga4gh.refget.v2.0.0+plain";

/// Acceptable fallback media types for JSON endpoints.
const JSON_FALLBACKS: &[&str] = &["application/json"];

/// Acceptable fallback media types for plain text endpoints.
const PLAIN_FALLBACKS: &[&str] = &["text/plain"];

pub fn router(state: RefgetState) -> Router {
    Router::new()
        .route("/sequence/service-info", get(service_info))
        .route("/sequence/{digest}", get(get_sequence))
        .route("/sequence/{digest}/metadata", get(get_metadata))
        .with_state(state)
}

/// Check the Accept header and return 406 if the client doesn't accept the
/// given content type or any of the provided fallbacks.
fn check_accept(headers: &HeaderMap, content_type: &str, fallbacks: &[&str]) -> Option<Response> {
    let accept = headers.get("accept")?;
    let accept_str = accept.to_str().ok()?;
    for media_type in accept_str.split(',') {
        let media_type = media_type.split(';').next().unwrap_or("").trim();
        if media_type == "*/*" || media_type == content_type || fallbacks.contains(&media_type) {
            return None;
        }
    }
    Some((StatusCode::NOT_ACCEPTABLE, "Not Acceptable").into_response())
}

/// Normalize a digest identifier for store lookup.
///
/// Produces candidates in priority order:
/// 1. Raw identifier as-is
/// 2. Strip `SQ.` prefix (ga4gh without prefix)
/// 3. Strip `ga4gh:` namespace prefix
/// 4. Strip `md5:` namespace prefix
/// 5. Convert trunc512 (48-char hex) to sha512t24u (base64url)
/// 6. Case-fold to lowercase (for case-insensitive MD5 hex)
fn normalize_candidates(digest: &str) -> Vec<String> {
    let mut candidates = vec![digest.to_string()];

    // Strip SQ. prefix
    if let Some(stripped) = digest.strip_prefix("SQ.") {
        candidates.push(stripped.to_string());
    }

    // Strip ga4gh: namespace prefix → "ga4gh:SQ.xxx" → "SQ.xxx" and "xxx"
    if let Some(stripped) = digest.strip_prefix("ga4gh:") {
        candidates.push(stripped.to_string());
        if let Some(bare) = stripped.strip_prefix("SQ.") {
            candidates.push(bare.to_string());
        }
    }

    // Strip md5: namespace prefix
    if let Some(stripped) = digest.strip_prefix("md5:") {
        candidates.push(stripped.to_string());
    }

    // trunc512: 48-char hex → decode to 24 bytes → base64url = sha512t24u
    if digest.len() == 48
        && digest.chars().all(|c| c.is_ascii_hexdigit())
        && let Ok(bytes) = hex_decode(digest)
    {
        let b64 = URL_SAFE_NO_PAD.encode(&bytes);
        candidates.push(format!("SQ.{b64}"));
        candidates.push(b64);
    }

    // Case-fold: lowercase for case-insensitive MD5 hex matching
    let lower = digest.to_ascii_lowercase();
    if lower != digest {
        candidates.push(lower);
    }

    candidates
}

/// Decode a hex string to bytes.
fn hex_decode(hex: &str) -> Result<Vec<u8>, ()> {
    if !hex.len().is_multiple_of(2) {
        return Err(());
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).map_err(|_| ()))
        .collect()
}

/// Try a store lookup with digest normalization.
/// Returns `Ok(Some(value))` on hit, `Ok(None)` for not found, `Err(response)` on store error.
fn lookup_normalized<T>(
    digest: &str,
    mut f: impl FnMut(&str) -> refget_store::StoreResult<Option<T>>,
) -> Result<Option<T>, Box<Response>> {
    for candidate in normalize_candidates(digest) {
        match f(&candidate) {
            Ok(Some(val)) => return Ok(Some(val)),
            Ok(None) => continue,
            Err(_) => return Err(Box::new(StatusCode::INTERNAL_SERVER_ERROR.into_response())),
        }
    }
    Ok(None)
}

async fn service_info(State(state): State<RefgetState>, headers: HeaderMap) -> Response {
    if let Some(resp) = check_accept(&headers, REFGET_JSON_CONTENT_TYPE, JSON_FALLBACKS) {
        return resp;
    }

    let mut info = serde_json::json!({
        "id": "org.ga4gh.refget",
        "name": "refget-rs",
        "type": {
            "group": "org.ga4gh",
            "artifact": "refget",
            "version": "2.0.0"
        },
        "description": "GA4GH refget Sequences v2.0.0",
        "version": env!("CARGO_PKG_VERSION"),
        "refget": {
            "circular_supported": state.config.circular_supported,
            "algorithms": state.config.algorithms,
            "identifier_types": ["ga4gh", "md5"],
            "subsequence_limit": state.config.subsequence_limit,
        }
    });

    // Add optional service-info fields when configured
    let si = &state.config.service_info;
    if let Some(org) = &si.organization {
        info["organization"] = serde_json::json!({"name": org.name, "url": org.url});
    }
    if let Some(url) = &si.contact_url {
        info["contactUrl"] = serde_json::Value::String(url.clone());
    }
    if let Some(url) = &si.documentation_url {
        info["documentationUrl"] = serde_json::Value::String(url.clone());
    }
    if let Some(env) = &si.environment {
        info["environment"] = serde_json::Value::String(env.clone());
    }

    (
        StatusCode::OK,
        [("content-type", REFGET_JSON_CONTENT_TYPE)],
        serde_json::to_string(&info).unwrap(),
    )
        .into_response()
}

#[derive(Deserialize)]
struct SubsequenceParams {
    start: Option<u64>,
    end: Option<u64>,
}

async fn get_sequence(
    State(state): State<RefgetState>,
    Path(digest): Path<String>,
    Query(params): Query<SubsequenceParams>,
    headers: HeaderMap,
) -> Response {
    if let Some(resp) = check_accept(&headers, REFGET_PLAIN_CONTENT_TYPE, PLAIN_FALLBACKS) {
        return resp;
    }

    let has_query_params = params.start.is_some() || params.end.is_some();
    let has_range_header = headers.get("range").is_some();

    // Reject combined Range header + query params
    if has_query_params && has_range_header {
        return (
            StatusCode::BAD_REQUEST,
            "Cannot combine Range header with start/end query params",
        )
            .into_response();
    }

    // Parse Range header if present and no query params
    let (start, end, used_range_header) = if !has_query_params && has_range_header {
        let (s, e) = parse_range_header(&headers);
        (s, e, true)
    } else {
        (params.start, params.end, false)
    };

    // Validate start/end: start > end on non-circular server → 501
    if let (Some(s), Some(e)) = (start, end)
        && s > e
    {
        return (StatusCode::NOT_IMPLEMENTED, "Circular sequences not supported").into_response();
    }

    // Look up sequence length for bounds checking
    let length = match lookup_normalized(&digest, |d| state.sequence_store.get_length(d)) {
        Ok(Some(len)) => len,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(resp) => return *resp,
    };

    // Bounds validation
    if let Some(s) = start
        && s >= length
    {
        return (StatusCode::RANGE_NOT_SATISFIABLE, "start >= sequence length").into_response();
    }
    // For query params, end > length is an error (400).
    // For Range header, clamping is handled by the store.
    if let Some(e) = end
        && !used_range_header
        && e > length
    {
        return (StatusCode::BAD_REQUEST, "end > sequence length").into_response();
    }

    // Enforce subsequence_limit (0 = no limit)
    let limit = state.config.subsequence_limit;
    if limit > 0 {
        let req_start = start.unwrap_or(0);
        let req_end = end.unwrap_or(length);
        if req_end - req_start > limit {
            return (StatusCode::RANGE_NOT_SATISFIABLE, "Subsequence exceeds limit")
                .into_response();
        }
    }

    match lookup_normalized(&digest, |d| state.sequence_store.get_sequence(d, start, end)) {
        Ok(Some(seq)) => {
            if used_range_header {
                // Range header → 206 Partial Content
                (StatusCode::PARTIAL_CONTENT, [("content-type", REFGET_PLAIN_CONTENT_TYPE)], seq)
                    .into_response()
            } else if start.is_some() || end.is_some() {
                // Query params → 200 OK with Accept-Ranges: none
                (
                    StatusCode::OK,
                    [("content-type", REFGET_PLAIN_CONTENT_TYPE), ("accept-ranges", "none")],
                    seq,
                )
                    .into_response()
            } else {
                // Full sequence → 200 OK
                (StatusCode::OK, [("content-type", REFGET_PLAIN_CONTENT_TYPE)], seq).into_response()
            }
        }
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(resp) => *resp,
    }
}

async fn get_metadata(
    State(state): State<RefgetState>,
    Path(digest): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Some(resp) = check_accept(&headers, REFGET_JSON_CONTENT_TYPE, JSON_FALLBACKS) {
        return resp;
    }

    match lookup_normalized(&digest, |d| state.sequence_store.get_metadata(d)) {
        Ok(Some(meta)) => {
            let response = serde_json::json!({
                "metadata": {
                    "md5": meta.md5,
                    "ga4gh": meta.sha512t24u,
                    "length": meta.length,
                    "aliases": meta.aliases,
                }
            });
            (
                StatusCode::OK,
                [("content-type", REFGET_JSON_CONTENT_TYPE)],
                serde_json::to_string(&response).unwrap(),
            )
                .into_response()
        }
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(resp) => *resp,
    }
}

/// Parse a Range header of the form `bytes=start-end`.
fn parse_range_header(headers: &HeaderMap) -> (Option<u64>, Option<u64>) {
    if let Some(range) = headers.get("range")
        && let Ok(range_str) = range.to_str()
        && let Some(bytes_range) = range_str.strip_prefix("bytes=")
    {
        let parts: Vec<&str> = bytes_range.splitn(2, '-').collect();
        if parts.len() == 2 {
            let start = parts[0].parse::<u64>().ok();
            let end = parts[1].parse::<u64>().ok().map(|e| e + 1); // HTTP Range is inclusive
            return (start, end);
        }
    }
    (None, None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use refget_model::SequenceMetadata;
    use refget_store::{InMemorySeqColStore, InMemorySequenceStore};
    use std::sync::Arc;
    use tower::ServiceExt;

    use crate::{RefgetConfig, RefgetState};

    fn test_metadata() -> SequenceMetadata {
        SequenceMetadata {
            md5: "abc123".to_string(),
            sha512t24u: "SQ.xyz789".to_string(),
            length: 10,
            aliases: vec![],
        }
    }

    fn test_state() -> RefgetState {
        RefgetState {
            sequence_store: Arc::new(InMemorySequenceStore::new()),
            seqcol_store: Arc::new(InMemorySeqColStore::new()),
            config: RefgetConfig::default(),
        }
    }

    fn test_state_with_sequence() -> RefgetState {
        let mut seq_store = InMemorySequenceStore::new();
        seq_store.add(test_metadata(), b"ACGTACGTAC".to_vec());
        RefgetState {
            sequence_store: Arc::new(seq_store),
            seqcol_store: Arc::new(InMemorySeqColStore::new()),
            config: RefgetConfig::default(),
        }
    }

    #[tokio::test]
    async fn test_service_info() {
        let app = router(test_state());
        let req = Request::builder().uri("/sequence/service-info").body(Body::empty()).unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let content_type = resp.headers().get("content-type").unwrap().to_str().unwrap();
        assert_eq!(content_type, REFGET_JSON_CONTENT_TYPE);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let refget = json.get("refget").expect("response must contain 'refget' key");
        assert_eq!(refget["subsequence_limit"], 0);
    }

    #[tokio::test]
    async fn test_get_sequence() {
        let state = test_state_with_sequence();
        let app = router(state);
        let req = Request::builder().uri("/sequence/SQ.xyz789").body(Body::empty()).unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let content_type = resp.headers().get("content-type").unwrap().to_str().unwrap();
        assert_eq!(content_type, REFGET_PLAIN_CONTENT_TYPE);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(&body[..], b"ACGTACGTAC");
    }

    #[tokio::test]
    async fn test_get_sequence_not_found() {
        let app = router(test_state());
        let req = Request::builder().uri("/sequence/nonexistent").body(Body::empty()).unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_get_sequence_with_query_params() {
        let state = test_state_with_sequence();
        let app = router(state);
        let req = Request::builder()
            .uri("/sequence/SQ.xyz789?start=2&end=6")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        // Query param subsequences return 200, not 206
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.headers().get("accept-ranges").unwrap().to_str().unwrap(), "none");

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(&body[..], b"GTAC");
    }

    #[tokio::test]
    async fn test_get_sequence_with_range_header() {
        let state = test_state_with_sequence();
        let app = router(state);
        let req = Request::builder()
            .uri("/sequence/SQ.xyz789")
            .header("range", "bytes=2-5")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        // Range header → 206
        assert_eq!(resp.status(), StatusCode::PARTIAL_CONTENT);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(&body[..], b"GTAC");
    }

    #[tokio::test]
    async fn test_get_sequence_invalid_range() {
        let state = test_state_with_sequence();
        let app = router(state);
        let req = Request::builder()
            .uri("/sequence/SQ.xyz789?start=5&end=3")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_IMPLEMENTED);
    }

    #[tokio::test]
    async fn test_get_metadata() {
        let state = test_state_with_sequence();
        let app = router(state);
        let req =
            Request::builder().uri("/sequence/SQ.xyz789/metadata").body(Body::empty()).unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let content_type = resp.headers().get("content-type").unwrap().to_str().unwrap();
        assert_eq!(content_type, REFGET_JSON_CONTENT_TYPE);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let metadata = json.get("metadata").expect("must have 'metadata' key");
        assert_eq!(metadata["md5"], "abc123");
        assert_eq!(metadata["ga4gh"], "SQ.xyz789");
        assert_eq!(metadata["length"], 10);
    }

    #[tokio::test]
    async fn test_get_metadata_not_found() {
        let app = router(test_state());
        let req =
            Request::builder().uri("/sequence/nonexistent/metadata").body(Body::empty()).unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn test_parse_range_header_valid() {
        let mut headers = HeaderMap::new();
        headers.insert("range", "bytes=10-19".parse().unwrap());
        let (start, end) = parse_range_header(&headers);
        assert_eq!(start, Some(10));
        assert_eq!(end, Some(20));
    }

    #[test]
    fn test_parse_range_header_invalid_format() {
        let mut headers = HeaderMap::new();
        headers.insert("range", "invalid".parse().unwrap());
        let (start, end) = parse_range_header(&headers);
        assert_eq!(start, None);
        assert_eq!(end, None);
    }

    #[test]
    fn test_parse_range_header_missing() {
        let headers = HeaderMap::new();
        let (start, end) = parse_range_header(&headers);
        assert_eq!(start, None);
        assert_eq!(end, None);
    }

    // --- GA4GH Compliance Integration Tests ---

    fn compliance_test_state() -> RefgetState {
        use md5::{Digest, Md5};
        use refget_digest::sha512t24u;
        use refget_model::Alias;

        let mut seq_store = InMemorySequenceStore::new();

        // Canonical test vector: ACGT
        let seq = b"ACGT";
        let md5_hex = format!("{:x}", Md5::digest(seq));
        let sha_digest = sha512t24u(seq);
        let ga4gh_digest = format!("SQ.{sha_digest}");

        seq_store.add(
            SequenceMetadata {
                md5: md5_hex.clone(),
                sha512t24u: ga4gh_digest.clone(),
                length: seq.len() as u64,
                aliases: vec![Alias {
                    naming_authority: "insdc".to_string(),
                    value: "test_seq".to_string(),
                }],
            },
            seq.to_vec(),
        );

        RefgetState {
            sequence_store: Arc::new(seq_store),
            seqcol_store: Arc::new(InMemorySeqColStore::new()),
            config: RefgetConfig::default(),
        }
    }

    #[tokio::test]
    async fn test_compliance_service_info() {
        let app = router(compliance_test_state());
        let req = Request::builder().uri("/sequence/service-info").body(Body::empty()).unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let content_type = resp.headers().get("content-type").unwrap().to_str().unwrap();
        assert_eq!(content_type, REFGET_JSON_CONTENT_TYPE);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let refget = &json["refget"];
        assert_eq!(refget["circular_supported"], false);
        assert!(refget["algorithms"].as_array().unwrap().len() >= 2);
        assert_eq!(refget["subsequence_limit"], 0);
        assert!(refget["identifier_types"].as_array().is_some());
    }

    #[tokio::test]
    async fn test_compliance_service_info_with_optional_fields() {
        use crate::state::{OrganizationConfig, ServiceInfoConfig};

        let mut state = compliance_test_state();
        state.config.service_info = ServiceInfoConfig {
            organization: Some(OrganizationConfig {
                name: "Test Org".to_string(),
                url: "https://example.org".to_string(),
            }),
            contact_url: Some("mailto:admin@example.org".to_string()),
            documentation_url: Some("https://example.org/docs".to_string()),
            environment: Some("test".to_string()),
        };

        let app = router(state);
        let req = Request::builder().uri("/sequence/service-info").body(Body::empty()).unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["organization"]["name"], "Test Org");
        assert_eq!(json["organization"]["url"], "https://example.org");
        assert_eq!(json["contactUrl"], "mailto:admin@example.org");
        assert_eq!(json["documentationUrl"], "https://example.org/docs");
        assert_eq!(json["environment"], "test");
    }

    #[tokio::test]
    async fn test_compliance_get_sequence_by_ga4gh() {
        let app = router(compliance_test_state());
        let req = Request::builder()
            .uri("/sequence/SQ.aKF498dAxcJAqme6QYQ7EZ07-fiw8Kw2")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(&body[..], b"ACGT");
    }

    #[tokio::test]
    async fn test_compliance_get_sequence_by_md5() {
        use md5::{Digest, Md5};

        let md5_hex = format!("{:x}", Md5::digest(b"ACGT"));
        let app = router(compliance_test_state());
        let req =
            Request::builder().uri(format!("/sequence/{md5_hex}")).body(Body::empty()).unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(&body[..], b"ACGT");
    }

    #[tokio::test]
    async fn test_compliance_get_sequence_by_md5_uppercase() {
        use md5::{Digest, Md5};

        let md5_hex = format!("{:X}", Md5::digest(b"ACGT"));
        let app = router(compliance_test_state());
        let req =
            Request::builder().uri(format!("/sequence/{md5_hex}")).body(Body::empty()).unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(&body[..], b"ACGT");
    }

    #[tokio::test]
    async fn test_compliance_get_sequence_by_namespaced_ga4gh() {
        let app = router(compliance_test_state());
        let req = Request::builder()
            .uri("/sequence/ga4gh:SQ.aKF498dAxcJAqme6QYQ7EZ07-fiw8Kw2")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(&body[..], b"ACGT");
    }

    #[tokio::test]
    async fn test_compliance_get_sequence_by_namespaced_md5() {
        use md5::{Digest, Md5};

        let md5_hex = format!("{:x}", Md5::digest(b"ACGT"));
        let app = router(compliance_test_state());
        let req =
            Request::builder().uri(format!("/sequence/md5:{md5_hex}")).body(Body::empty()).unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(&body[..], b"ACGT");
    }

    #[tokio::test]
    async fn test_compliance_get_sequence_by_trunc512() {
        use sha2::{Digest, Sha512};

        // Compute trunc512: first 24 bytes of SHA-512, hex-encoded (48 chars)
        let hash = Sha512::digest(b"ACGT");
        let trunc512: String = hash[..24].iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(trunc512.len(), 48);

        let app = router(compliance_test_state());
        let req =
            Request::builder().uri(format!("/sequence/{trunc512}")).body(Body::empty()).unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(&body[..], b"ACGT");
    }

    #[tokio::test]
    async fn test_compliance_subsequence_start_end() {
        let app = router(compliance_test_state());
        let req = Request::builder()
            .uri("/sequence/SQ.aKF498dAxcJAqme6QYQ7EZ07-fiw8Kw2?start=1&end=3")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        // Query params → 200, not 206
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.headers().get("accept-ranges").unwrap().to_str().unwrap(), "none");

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(&body[..], b"CG");
    }

    #[tokio::test]
    async fn test_compliance_subsequence_range_header() {
        let app = router(compliance_test_state());
        let req = Request::builder()
            .uri("/sequence/SQ.aKF498dAxcJAqme6QYQ7EZ07-fiw8Kw2")
            .header("range", "bytes=1-2")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        // Range header → 206
        assert_eq!(resp.status(), StatusCode::PARTIAL_CONTENT);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(&body[..], b"CG");
    }

    #[tokio::test]
    async fn test_compliance_start_equals_end() {
        let app = router(compliance_test_state());
        let req = Request::builder()
            .uri("/sequence/SQ.aKF498dAxcJAqme6QYQ7EZ07-fiw8Kw2?start=2&end=2")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        assert!(body.is_empty());
    }

    #[tokio::test]
    async fn test_compliance_start_greater_end_501() {
        let app = router(compliance_test_state());
        let req = Request::builder()
            .uri("/sequence/SQ.aKF498dAxcJAqme6QYQ7EZ07-fiw8Kw2?start=3&end=1")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_IMPLEMENTED);
    }

    #[tokio::test]
    async fn test_compliance_start_beyond_length_416() {
        let app = router(compliance_test_state());
        let req = Request::builder()
            .uri("/sequence/SQ.aKF498dAxcJAqme6QYQ7EZ07-fiw8Kw2?start=100")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::RANGE_NOT_SATISFIABLE);
    }

    #[tokio::test]
    async fn test_compliance_end_beyond_length_400() {
        let app = router(compliance_test_state());
        let req = Request::builder()
            .uri("/sequence/SQ.aKF498dAxcJAqme6QYQ7EZ07-fiw8Kw2?end=100")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_compliance_range_plus_query_400() {
        let app = router(compliance_test_state());
        let req = Request::builder()
            .uri("/sequence/SQ.aKF498dAxcJAqme6QYQ7EZ07-fiw8Kw2?start=0&end=2")
            .header("range", "bytes=0-1")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_compliance_accept_not_acceptable() {
        let app = router(compliance_test_state());
        let req = Request::builder()
            .uri("/sequence/SQ.aKF498dAxcJAqme6QYQ7EZ07-fiw8Kw2")
            .header("accept", "text/xml")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_ACCEPTABLE);
    }

    #[tokio::test]
    async fn test_compliance_accept_wildcard() {
        let app = router(compliance_test_state());
        let req = Request::builder()
            .uri("/sequence/SQ.aKF498dAxcJAqme6QYQ7EZ07-fiw8Kw2")
            .header("accept", "*/*")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_compliance_accept_text_plain_fallback() {
        let app = router(compliance_test_state());
        let req = Request::builder()
            .uri("/sequence/SQ.aKF498dAxcJAqme6QYQ7EZ07-fiw8Kw2")
            .header("accept", "text/plain")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_compliance_accept_json_fallback_for_metadata() {
        let app = router(compliance_test_state());
        let req = Request::builder()
            .uri("/sequence/SQ.aKF498dAxcJAqme6QYQ7EZ07-fiw8Kw2/metadata")
            .header("accept", "application/json")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_compliance_accept_json_fallback_for_service_info() {
        let app = router(compliance_test_state());
        let req = Request::builder()
            .uri("/sequence/service-info")
            .header("accept", "application/json")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_compliance_metadata_content_type() {
        let app = router(compliance_test_state());
        let req = Request::builder()
            .uri("/sequence/SQ.aKF498dAxcJAqme6QYQ7EZ07-fiw8Kw2/metadata")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let content_type = resp.headers().get("content-type").unwrap().to_str().unwrap();
        assert_eq!(content_type, REFGET_JSON_CONTENT_TYPE);
    }

    #[tokio::test]
    async fn test_compliance_metadata_shape() {
        let app = router(compliance_test_state());
        let req = Request::builder()
            .uri("/sequence/SQ.aKF498dAxcJAqme6QYQ7EZ07-fiw8Kw2/metadata")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let metadata = json.get("metadata").expect("must have 'metadata' key");
        assert!(metadata["md5"].is_string());
        assert!(metadata["ga4gh"].as_str().unwrap().starts_with("SQ."));
        assert_eq!(metadata["length"], 4);
        assert!(metadata["aliases"].is_array());
    }

    #[tokio::test]
    async fn test_compliance_not_found_404() {
        let app = router(compliance_test_state());
        let req = Request::builder()
            .uri("/sequence/SQ.nonexistentdigest000000000000")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_compliance_subsequence_limit_enforced() {
        let mut state = compliance_test_state();
        state.config.subsequence_limit = 2;

        let app = router(state);
        // Request 3 bases with limit of 2 → 416
        let req = Request::builder()
            .uri("/sequence/SQ.aKF498dAxcJAqme6QYQ7EZ07-fiw8Kw2?start=0&end=3")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::RANGE_NOT_SATISFIABLE);
    }

    #[tokio::test]
    async fn test_compliance_subsequence_limit_within() {
        let mut state = compliance_test_state();
        state.config.subsequence_limit = 2;

        let app = router(state);
        // Request 2 bases with limit of 2 → OK
        let req = Request::builder()
            .uri("/sequence/SQ.aKF498dAxcJAqme6QYQ7EZ07-fiw8Kw2?start=0&end=2")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(&body[..], b"AC");
    }

    #[test]
    fn test_hex_decode() {
        assert_eq!(hex_decode(""), Ok(vec![]));
        assert_eq!(hex_decode("00ff"), Ok(vec![0x00, 0xff]));
        assert_eq!(hex_decode("0F"), Ok(vec![0x0f]));
        assert!(hex_decode("0").is_err());
        assert!(hex_decode("zz").is_err());
    }

    #[test]
    fn test_normalize_candidates() {
        // Basic SQ. prefix
        let candidates = normalize_candidates("SQ.abc123");
        assert!(candidates.contains(&"SQ.abc123".to_string()));
        assert!(candidates.contains(&"abc123".to_string()));

        // ga4gh: namespace
        let candidates = normalize_candidates("ga4gh:SQ.abc123");
        assert!(candidates.contains(&"SQ.abc123".to_string()));
        assert!(candidates.contains(&"abc123".to_string()));

        // md5: namespace
        let candidates = normalize_candidates("md5:abc123");
        assert!(candidates.contains(&"abc123".to_string()));

        // Case folding
        let candidates = normalize_candidates("ABC123");
        assert!(candidates.contains(&"abc123".to_string()));
    }
}
