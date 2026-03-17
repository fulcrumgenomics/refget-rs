//! Handlers for the refget Sequence Collections v1.0.0 API.

use axum::Router;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use refget_model::{Level, SeqCol, compare};
use serde::Deserialize;

use super::json_error;
use crate::RefgetState;

pub fn router(state: RefgetState) -> Router {
    Router::new()
        .route("/service-info", get(service_info))
        .route("/collection/{digest}", get(get_collection))
        .route("/comparison/{digest1}/{digest2}", get(compare_collections))
        .route("/comparison/{digest1}", post(compare_with_post))
        .route("/list/collection", get(list_collections))
        .route("/attribute/collection/{attr}/{digest}", get(get_attribute))
        .with_state(state)
}

async fn service_info(State(state): State<RefgetState>) -> impl IntoResponse {
    let info = serde_json::json!({
        "id": "org.ga4gh.seqcol",
        "name": "refget-rs seqcol",
        "type": {
            "group": "org.ga4gh",
            "artifact": "seqcol",
            "version": "1.0.0"
        },
        "description": "GA4GH Sequence Collections v1.0.0",
        "version": env!("CARGO_PKG_VERSION"),
        "seqcol": {
            "schema": {
                "names": { "type": "array", "collated": true },
                "lengths": { "type": "array", "collated": true },
                "sequences": { "type": "array", "collated": true },
                "sorted_name_length_pairs": { "type": "array", "collated": false }
            },
            "total_collections": state.seqcol_store.count()
        }
    });
    axum::Json(info)
}

#[derive(Deserialize)]
struct CollectionParams {
    level: Option<u8>,
}

async fn get_collection(
    State(state): State<RefgetState>,
    Path(digest): Path<String>,
    Query(params): Query<CollectionParams>,
) -> Response {
    let level = params.level.and_then(Level::from_int).unwrap_or(Level::Two);

    match state.seqcol_store.get_collection(&digest) {
        Some(col) => axum::Json(col.to_json(level)).into_response(),
        None => json_error(StatusCode::NOT_FOUND, "Collection not found"),
    }
}

async fn compare_collections(
    State(state): State<RefgetState>,
    Path((digest1, digest2)): Path<(String, String)>,
) -> Response {
    let a = state.seqcol_store.get_collection(&digest1);
    let b = state.seqcol_store.get_collection(&digest2);

    match (a, b) {
        (Some(a), Some(b)) => axum::Json(compare(a, b)).into_response(),
        (None, None) => json_error(StatusCode::NOT_FOUND, "Collections not found"),
        (None, _) => json_error(StatusCode::NOT_FOUND, format!("Collection not found: {digest1}")),
        (_, None) => json_error(StatusCode::NOT_FOUND, format!("Collection not found: {digest2}")),
    }
}

async fn compare_with_post(
    State(state): State<RefgetState>,
    Path(digest1): Path<String>,
    axum::Json(body): axum::Json<SeqCol>,
) -> Response {
    match state.seqcol_store.get_collection(&digest1) {
        Some(a) => axum::Json(compare(a, &body)).into_response(),
        None => json_error(StatusCode::NOT_FOUND, "Collection not found"),
    }
}

#[derive(Deserialize)]
struct ListParams {
    page: Option<usize>,
    page_size: Option<usize>,
    // Attribute filters are passed as query params like `names=<digest>`
    names: Option<String>,
    lengths: Option<String>,
    sequences: Option<String>,
}

async fn list_collections(
    State(state): State<RefgetState>,
    Query(params): Query<ListParams>,
) -> impl IntoResponse {
    let page = params.page.unwrap_or(0);
    let page_size = params.page_size.unwrap_or(50);

    let mut filters = Vec::new();
    if let Some(d) = params.names {
        filters.push(("names".to_string(), d));
    }
    if let Some(d) = params.lengths {
        filters.push(("lengths".to_string(), d));
    }
    if let Some(d) = params.sequences {
        filters.push(("sequences".to_string(), d));
    }

    let result = state.seqcol_store.list_collections(&filters, page, page_size);
    axum::Json(result)
}

async fn get_attribute(
    State(state): State<RefgetState>,
    Path((attr, digest)): Path<(String, String)>,
) -> Response {
    match state.seqcol_store.get_attribute(&attr, &digest) {
        Some(value) => axum::Json(value).into_response(),
        None => json_error(StatusCode::NOT_FOUND, "Attribute not found"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use refget_store::{InMemorySeqColStore, InMemorySequenceStore};
    use std::sync::Arc;
    use tower::ServiceExt;

    use crate::{RefgetConfig, RefgetState};

    fn test_state() -> RefgetState {
        let mut seqcol_store = InMemorySeqColStore::new();
        seqcol_store.add(SeqCol {
            names: vec!["chr1".to_string()],
            lengths: vec![100],
            sequences: vec!["SQ.abc".to_string()],
            sorted_name_length_pairs: None,
        });

        RefgetState {
            sequence_store: Arc::new(InMemorySequenceStore::new()),
            seqcol_store: Arc::new(seqcol_store),
            config: RefgetConfig::default(),
        }
    }

    #[tokio::test]
    async fn test_service_info() {
        let app = router(test_state());
        let req = Request::builder().uri("/service-info").body(Body::empty()).unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_collection_not_found() {
        let app = router(test_state());
        let req = Request::builder().uri("/collection/nonexistent").body(Body::empty()).unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_get_collection() {
        let state = test_state();
        let col = SeqCol {
            names: vec!["chr1".to_string()],
            lengths: vec![100],
            sequences: vec!["SQ.abc".to_string()],
            sorted_name_length_pairs: None,
        };
        let digest = col.digest();

        let app = router(state);
        let req =
            Request::builder().uri(format!("/collection/{digest}")).body(Body::empty()).unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json.get("names").is_some());
    }

    #[tokio::test]
    async fn test_list_collections() {
        let app = router(test_state());
        let req = Request::builder().uri("/list/collection").body(Body::empty()).unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["total"], 1);
    }

    fn two_collection_state() -> (RefgetState, String, String) {
        let col_a = SeqCol {
            names: vec!["chr1".to_string()],
            lengths: vec![100],
            sequences: vec!["SQ.abc".to_string()],
            sorted_name_length_pairs: None,
        };
        let col_b = SeqCol {
            names: vec!["chr2".to_string()],
            lengths: vec![200],
            sequences: vec!["SQ.def".to_string()],
            sorted_name_length_pairs: None,
        };
        let digest_a = col_a.digest();
        let digest_b = col_b.digest();

        let mut seqcol_store = InMemorySeqColStore::new();
        seqcol_store.add(col_a);
        seqcol_store.add(col_b);

        let state = RefgetState {
            sequence_store: Arc::new(InMemorySequenceStore::new()),
            seqcol_store: Arc::new(seqcol_store),
            config: RefgetConfig::default(),
        };
        (state, digest_a, digest_b)
    }

    #[tokio::test]
    async fn test_compare_collections() {
        let (state, digest_a, digest_b) = two_collection_state();
        let app = router(state);
        let req = Request::builder()
            .uri(format!("/comparison/{digest_a}/{digest_b}"))
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json.get("digests").is_some());
        assert!(json.get("attributes").is_some());
        assert!(json.get("array_elements").is_some());
        assert_eq!(json["digests"]["a"], digest_a);
        assert_eq!(json["digests"]["b"], digest_b);
    }

    #[tokio::test]
    async fn test_compare_not_found() {
        let (state, digest_a, _) = two_collection_state();
        let app = router(state);
        let req = Request::builder()
            .uri(format!("/comparison/{digest_a}/nonexistent"))
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_compare_with_post() {
        let (state, digest_a, _) = two_collection_state();
        let col_c = SeqCol {
            names: vec!["chr3".to_string()],
            lengths: vec![300],
            sequences: vec!["SQ.ghi".to_string()],
            sorted_name_length_pairs: None,
        };
        let body_json = serde_json::to_string(&col_c).unwrap();

        let app = router(state);
        let req = Request::builder()
            .method("POST")
            .uri(format!("/comparison/{digest_a}"))
            .header("content-type", "application/json")
            .body(Body::from(body_json))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json.get("digests").is_some());
        assert_eq!(json["digests"]["a"], digest_a);
    }

    #[tokio::test]
    async fn test_get_attribute() {
        let state = test_state();
        // Get the level1 digests for the collection added in test_state
        let col = SeqCol {
            names: vec!["chr1".to_string()],
            lengths: vec![100],
            sequences: vec!["SQ.abc".to_string()],
            sorted_name_length_pairs: None,
        };
        let level1 = col.to_level1();

        let app = router(state);
        let req = Request::builder()
            .uri(format!("/attribute/collection/names/{}", level1.names))
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json.is_array());
        assert_eq!(json.as_array().unwrap().len(), 1);
        assert_eq!(json[0], "chr1");
    }

    #[tokio::test]
    async fn test_get_attribute_not_found() {
        let app = router(test_state());
        let req = Request::builder()
            .uri("/attribute/collection/names/nonexistent_digest")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_get_collection_levels() {
        let state = test_state();
        let col = SeqCol {
            names: vec!["chr1".to_string()],
            lengths: vec![100],
            sequences: vec!["SQ.abc".to_string()],
            sorted_name_length_pairs: None,
        };
        let digest = col.digest();

        // Level 0: returns a single digest string
        let app = router(state.clone());
        let req = Request::builder()
            .uri(format!("/collection/{digest}?level=0"))
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json.is_string(), "level 0 should return a string digest");

        // Level 1: returns an object with per-attribute digests (strings)
        let app = router(state.clone());
        let req = Request::builder()
            .uri(format!("/collection/{digest}?level=1"))
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json.is_object(), "level 1 should return an object");
        assert!(json["names"].is_string(), "level 1 names should be a digest string");
        assert!(json["lengths"].is_string(), "level 1 lengths should be a digest string");

        // Level 2: returns an object with full arrays
        let app = router(state);
        let req = Request::builder()
            .uri(format!("/collection/{digest}?level=2"))
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json.is_object(), "level 2 should return an object");
        assert!(json["names"].is_array(), "level 2 names should be an array");
        assert!(json["lengths"].is_array(), "level 2 lengths should be an array");
    }

    #[tokio::test]
    async fn test_list_with_pagination() {
        let (state, _, _) = two_collection_state();

        // Page 0 with page_size=1 should return 1 item out of 2 total
        let app = router(state.clone());
        let req = Request::builder()
            .uri("/list/collection?page=0&page_size=1")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["total"], 2);
        assert_eq!(json["items"].as_array().unwrap().len(), 1);
        assert_eq!(json["page"], 0);
        assert_eq!(json["page_size"], 1);

        // Page 1 with page_size=1 should return the other item
        let app = router(state.clone());
        let req = Request::builder()
            .uri("/list/collection?page=1&page_size=1")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["total"], 2);
        assert_eq!(json["items"].as_array().unwrap().len(), 1);
        assert_eq!(json["page"], 1);

        // Page 2 with page_size=1 should return 0 items (past the end)
        let app = router(state);
        let req = Request::builder()
            .uri("/list/collection?page=2&page_size=1")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["total"], 2);
        assert_eq!(json["items"].as_array().unwrap().len(), 0);
    }
}
