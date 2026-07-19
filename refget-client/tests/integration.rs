//! Integration tests for the refget client.
//!
//! Spins up an in-process axum server on a random port and tests the client against it.

use std::sync::Arc;

use refget_digest::{md5_hex, sha512t24u};
use refget_model::{Alias, SeqCol, SequenceMetadata};
use refget_server::{RefgetConfig, RefgetState, refget_router};
use refget_store::{InMemorySeqColStore, InMemorySequenceStore};

/// Build test state with known sequences and collections.
fn test_state() -> (RefgetState, TestData) {
    let mut seq_store = InMemorySequenceStore::new();

    // Sequence 1: ACGTACGTAC
    let seq1 = b"ACGTACGTAC";
    let md5_1 = md5_hex(seq1);
    let sha_1 = sha512t24u(seq1);
    let ga4gh_1 = format!("SQ.{sha_1}");
    seq_store.add(
        SequenceMetadata {
            md5: md5_1.clone(),
            sha512t24u: ga4gh_1.clone(),
            length: seq1.len() as u64,
            aliases: vec![Alias {
                naming_authority: "insdc".to_string(),
                value: "test_seq_1".to_string(),
            }],
            circular: false,
        },
        seq1.to_vec(),
    );

    // Sequence 2: NNNNNNNN
    let seq2 = b"NNNNNNNN";
    let md5_2 = md5_hex(seq2);
    let sha_2 = sha512t24u(seq2);
    seq_store.add(
        SequenceMetadata {
            md5: md5_2.clone(),
            sha512t24u: format!("SQ.{sha_2}"),
            length: seq2.len() as u64,
            aliases: vec![],
            circular: false,
        },
        seq2.to_vec(),
    );

    // SeqCol with both sequences
    let col = SeqCol {
        names: vec!["chr1".to_string(), "chr2".to_string()],
        lengths: vec![seq1.len() as u64, seq2.len() as u64],
        sequences: vec![ga4gh_1.clone(), format!("SQ.{sha_2}")],
        sorted_name_length_pairs: None,
    };
    let col_digest = col.digest();

    let mut seqcol_store = InMemorySeqColStore::new();
    seqcol_store.add(col.clone());

    let state = RefgetState {
        sequence_store: Arc::new(seq_store),
        seqcol_store: Arc::new(seqcol_store),
        config: RefgetConfig::default(),
    };

    let data = TestData { md5_1, ga4gh_1, md5_2, col_digest, col };

    (state, data)
}

#[allow(dead_code)]
struct TestData {
    md5_1: String,
    ga4gh_1: String,
    md5_2: String,
    col_digest: String,
    col: SeqCol,
}

/// Start a server on a random port and return the base URL.
async fn start_server(state: RefgetState) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let router = refget_router(state);
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    format!("http://127.0.0.1:{port}")
}

// --- Async client tests ---

mod async_tests {
    use super::*;
    use refget_client::RefgetClient;

    #[tokio::test]
    async fn test_get_sequence_by_md5() {
        let (state, data) = test_state();
        let base_url = start_server(state).await;
        let client = RefgetClient::new(&base_url).unwrap();

        let seq = client.get_sequence(&data.md5_1, None, None).await.unwrap();
        assert_eq!(seq.as_deref(), Some(b"ACGTACGTAC".as_slice()));
    }

    #[tokio::test]
    async fn test_get_sequence_by_ga4gh() {
        let (state, data) = test_state();
        let base_url = start_server(state).await;
        let client = RefgetClient::new(&base_url).unwrap();

        let seq = client.get_sequence(&data.ga4gh_1, None, None).await.unwrap();
        assert_eq!(seq.as_deref(), Some(b"ACGTACGTAC".as_slice()));
    }

    #[tokio::test]
    async fn test_get_subsequence() {
        let (state, data) = test_state();
        let base_url = start_server(state).await;
        let client = RefgetClient::new(&base_url).unwrap();

        let seq = client.get_sequence(&data.md5_1, Some(2), Some(6)).await.unwrap();
        assert_eq!(seq.as_deref(), Some(b"GTAC".as_slice()));
    }

    #[tokio::test]
    async fn test_get_metadata() {
        let (state, data) = test_state();
        let base_url = start_server(state).await;
        let client = RefgetClient::new(&base_url).unwrap();

        let meta = client.get_metadata(&data.md5_1).await.unwrap().unwrap();
        assert_eq!(meta.md5, data.md5_1);
        assert_eq!(meta.sha512t24u, data.ga4gh_1);
        assert_eq!(meta.length, 10);
        assert_eq!(meta.aliases.len(), 1);
        assert_eq!(meta.aliases[0].naming_authority, "insdc");
    }

    #[tokio::test]
    async fn test_not_found_returns_none() {
        let (state, _data) = test_state();
        let base_url = start_server(state).await;
        let client = RefgetClient::new(&base_url).unwrap();

        assert!(client.get_sequence("nonexistent", None, None).await.unwrap().is_none());
        assert!(client.get_metadata("nonexistent").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_service_info() {
        let (state, _data) = test_state();
        let base_url = start_server(state).await;
        let client = RefgetClient::new(&base_url).unwrap();

        let info = client.get_sequence_service_info().await.unwrap();
        assert!(info.refget.circular_supported);
        assert!(!info.refget.algorithms.is_empty());
        assert!(!info.refget.identifier_types.is_empty());
    }

    #[tokio::test]
    async fn test_collection_level0() {
        let (state, data) = test_state();
        let base_url = start_server(state).await;
        let client = RefgetClient::new(&base_url).unwrap();

        let digest = client.get_collection_level0(&data.col_digest).await.unwrap().unwrap();
        assert_eq!(digest.len(), 32);
    }

    #[tokio::test]
    async fn test_collection_level1() {
        let (state, data) = test_state();
        let base_url = start_server(state).await;
        let client = RefgetClient::new(&base_url).unwrap();

        let level1 = client.get_collection_level1(&data.col_digest).await.unwrap().unwrap();
        assert_eq!(level1.names.len(), 32);
        assert_eq!(level1.lengths.len(), 32);
        assert_eq!(level1.sequences.len(), 32);
    }

    #[tokio::test]
    async fn test_collection_level2() {
        let (state, data) = test_state();
        let base_url = start_server(state).await;
        let client = RefgetClient::new(&base_url).unwrap();

        let col = client.get_collection_level2(&data.col_digest).await.unwrap().unwrap();
        assert_eq!(col.names.len(), 2);
        assert_eq!(col.names[0], "chr1");
        assert_eq!(col.lengths[0], 10);
    }

    #[tokio::test]
    async fn test_collection_not_found() {
        let (state, _data) = test_state();
        let base_url = start_server(state).await;
        let client = RefgetClient::new(&base_url).unwrap();

        assert!(client.get_collection_level2("nonexistent").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_compare_collections() {
        let (state, data) = test_state();
        let base_url = start_server(state).await;
        let client = RefgetClient::new(&base_url).unwrap();

        let result = client.compare_collections(&data.col_digest, &data.col_digest).await.unwrap();
        assert_eq!(result.digests.a, result.digests.b);
        assert_eq!(result.attributes.a_and_b.len(), 3);
    }

    #[tokio::test]
    async fn test_compare_collection_with_post() {
        let (state, data) = test_state();
        let base_url = start_server(state).await;
        let client = RefgetClient::new(&base_url).unwrap();

        let other = SeqCol {
            names: vec!["chrX".to_string()],
            lengths: vec![500],
            sequences: vec!["SQ.other".to_string()],
            sorted_name_length_pairs: None,
        };
        let result = client.compare_collection_with(&data.col_digest, &other).await.unwrap();
        assert_eq!(result.digests.a, data.col_digest);
        assert_ne!(result.digests.a, result.digests.b);
    }

    #[tokio::test]
    async fn test_list_collections() {
        let (state, _data) = test_state();
        let base_url = start_server(state).await;
        let client = RefgetClient::new(&base_url).unwrap();

        let result = client.list_collections(&[], 0, 50).await.unwrap();
        assert_eq!(result["total"], 1);
        assert_eq!(result["items"].as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn test_get_attribute() {
        let (state, data) = test_state();
        let base_url = start_server(state).await;
        let client = RefgetClient::new(&base_url).unwrap();

        let level1 = data.col.to_level1();
        let names = client.get_attribute("names", &level1.names).await.unwrap().unwrap();
        assert!(names.is_array());
        assert_eq!(names.as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn test_seqcol_service_info() {
        let (state, _data) = test_state();
        let base_url = start_server(state).await;
        let client = RefgetClient::new(&base_url).unwrap();

        let info = client.get_seqcol_service_info().await.unwrap();
        assert!(info.get("seqcol").is_some());
    }

    #[tokio::test]
    async fn test_get_sequence_start_only() {
        let (state, data) = test_state();
        let base_url = start_server(state).await;
        let client = RefgetClient::new(&base_url).unwrap();

        // start=7, no end → should return bytes from position 7 to the end ("TAC")
        let seq = client.get_sequence(&data.md5_1, Some(7), None).await.unwrap();
        assert_eq!(seq.as_deref(), Some(b"TAC".as_slice()));
    }

    #[tokio::test]
    async fn test_get_sequence_end_only() {
        let (state, data) = test_state();
        let base_url = start_server(state).await;
        let client = RefgetClient::new(&base_url).unwrap();

        // no start, end=3 → should return first 3 bytes ("ACG")
        let seq = client.get_sequence(&data.md5_1, None, Some(3)).await.unwrap();
        assert_eq!(seq.as_deref(), Some(b"ACG".as_slice()));
    }

    #[tokio::test]
    async fn test_get_sequence_empty_range() {
        let (state, data) = test_state();
        let base_url = start_server(state).await;
        let client = RefgetClient::new(&base_url).unwrap();

        // start=2, end=2 → empty result
        let seq = client.get_sequence(&data.md5_1, Some(2), Some(2)).await.unwrap();
        assert_eq!(seq.as_deref(), Some(b"".as_slice()));
    }

    #[tokio::test]
    async fn test_service_info_fields() {
        let (state, _data) = test_state();
        let base_url = start_server(state).await;
        let client = RefgetClient::new(&base_url).unwrap();

        let info = client.get_sequence_service_info().await.unwrap();
        assert_eq!(info.refget.subsequence_limit, 0);
        assert!(info.refget.supported_api_versions.contains(&"2.0.0".to_string()));
        assert!(info.refget.identifier_types.contains(&"ga4gh".to_string()));
        assert!(info.refget.identifier_types.contains(&"md5".to_string()));
    }

    #[tokio::test]
    async fn test_trailing_slash_stripped() {
        let (state, data) = test_state();
        let base_url = start_server(state).await;
        let client = RefgetClient::new(&format!("{base_url}/")).unwrap();

        let seq = client.get_sequence(&data.md5_1, None, None).await.unwrap();
        assert_eq!(seq.as_deref(), Some(b"ACGTACGTAC".as_slice()));
    }

    #[tokio::test]
    async fn test_invalid_url() {
        assert!(RefgetClient::new("").is_err());
    }
}

// --- Blocking client tests ---

mod blocking_tests {
    use super::*;
    use refget_client::RefgetClientBlocking;

    #[tokio::test]
    async fn test_blocking_get_sequence() {
        let (state, data) = test_state();
        let base_url = start_server(state).await;

        // Run the blocking client in a separate thread to avoid blocking the tokio runtime
        let md5 = data.md5_1.clone();
        let result = tokio::task::spawn_blocking(move || {
            let client = RefgetClientBlocking::new(&base_url).unwrap();
            client.get_sequence(&md5, None, None).unwrap()
        })
        .await
        .unwrap();

        assert_eq!(result.as_deref(), Some(b"ACGTACGTAC".as_slice()));
    }

    #[tokio::test]
    async fn test_blocking_get_subsequence() {
        let (state, data) = test_state();
        let base_url = start_server(state).await;

        let md5 = data.md5_1.clone();
        let result = tokio::task::spawn_blocking(move || {
            let client = RefgetClientBlocking::new(&base_url).unwrap();
            client.get_sequence(&md5, Some(2), Some(6)).unwrap()
        })
        .await
        .unwrap();

        assert_eq!(result.as_deref(), Some(b"GTAC".as_slice()));
    }

    #[tokio::test]
    async fn test_blocking_get_metadata() {
        let (state, data) = test_state();
        let base_url = start_server(state).await;

        let md5 = data.md5_1.clone();
        let ga4gh = data.ga4gh_1.clone();
        let meta = tokio::task::spawn_blocking(move || {
            let client = RefgetClientBlocking::new(&base_url).unwrap();
            client.get_metadata(&md5).unwrap().unwrap()
        })
        .await
        .unwrap();

        assert_eq!(meta.md5, data.md5_1);
        assert_eq!(meta.sha512t24u, ga4gh);
        assert_eq!(meta.length, 10);
    }

    #[tokio::test]
    async fn test_blocking_service_info() {
        let (state, _data) = test_state();
        let base_url = start_server(state).await;

        let info = tokio::task::spawn_blocking(move || {
            let client = RefgetClientBlocking::new(&base_url).unwrap();
            client.get_sequence_service_info().unwrap()
        })
        .await
        .unwrap();

        assert!(info.refget.circular_supported);
    }

    #[tokio::test]
    async fn test_blocking_not_found() {
        let (state, _data) = test_state();
        let base_url = start_server(state).await;

        let result = tokio::task::spawn_blocking(move || {
            let client = RefgetClientBlocking::new(&base_url).unwrap();
            client.get_sequence("nonexistent", None, None).unwrap()
        })
        .await
        .unwrap();

        assert!(result.is_none());
    }

    #[test]
    fn test_blocking_invalid_url() {
        assert!(RefgetClientBlocking::new("").is_err());
    }

    #[tokio::test]
    async fn test_blocking_collection_level2() {
        let (state, data) = test_state();
        let base_url = start_server(state).await;

        let digest = data.col_digest.clone();
        let col = tokio::task::spawn_blocking(move || {
            let client = RefgetClientBlocking::new(&base_url).unwrap();
            client.get_collection_level2(&digest).unwrap().unwrap()
        })
        .await
        .unwrap();

        assert_eq!(col.names.len(), 2);
        assert_eq!(col.names[0], "chr1");
    }
}

// --- RemoteSequenceStore tests ---

#[cfg(feature = "store")]
mod store_tests {
    use super::*;
    use refget_client::RemoteSequenceStore;
    use refget_store::SequenceStore;

    #[tokio::test]
    async fn test_remote_store_get_sequence() {
        let (state, data) = test_state();
        let base_url = start_server(state).await;

        let md5 = data.md5_1.clone();
        let result = tokio::task::spawn_blocking(move || {
            let store = RemoteSequenceStore::new(&base_url).unwrap();
            store.get_sequence(&md5, None, None).unwrap()
        })
        .await
        .unwrap();

        assert_eq!(result.as_deref(), Some(b"ACGTACGTAC".as_slice()));
    }

    #[tokio::test]
    async fn test_remote_store_get_subsequence() {
        let (state, data) = test_state();
        let base_url = start_server(state).await;

        let md5 = data.md5_1.clone();
        let result = tokio::task::spawn_blocking(move || {
            let store = RemoteSequenceStore::new(&base_url).unwrap();
            store.get_sequence(&md5, Some(2), Some(6)).unwrap()
        })
        .await
        .unwrap();

        assert_eq!(result.as_deref(), Some(b"GTAC".as_slice()));
    }

    #[tokio::test]
    async fn test_remote_store_get_metadata() {
        let (state, data) = test_state();
        let base_url = start_server(state).await;

        let md5 = data.md5_1.clone();
        let meta = tokio::task::spawn_blocking(move || {
            let store = RemoteSequenceStore::new(&base_url).unwrap();
            store.get_metadata(&md5).unwrap().unwrap()
        })
        .await
        .unwrap();

        assert_eq!(meta.md5, data.md5_1);
        assert_eq!(meta.length, 10);
    }

    #[tokio::test]
    async fn test_remote_store_get_length() {
        let (state, data) = test_state();
        let base_url = start_server(state).await;

        let md5 = data.md5_1.clone();
        let length = tokio::task::spawn_blocking(move || {
            let store = RemoteSequenceStore::new(&base_url).unwrap();
            store.get_length(&md5).unwrap().unwrap()
        })
        .await
        .unwrap();

        assert_eq!(length, 10);
    }

    #[tokio::test]
    async fn test_remote_store_not_found() {
        let (state, _data) = test_state();
        let base_url = start_server(state).await;

        let result = tokio::task::spawn_blocking(move || {
            let store = RemoteSequenceStore::new(&base_url).unwrap();
            store.get_sequence("nonexistent", None, None).unwrap()
        })
        .await
        .unwrap();

        assert!(result.is_none());
    }
}
