//! In-memory sequence store implementation.

use std::collections::HashMap;

use refget_model::SequenceMetadata;

use crate::{SequenceStore, StoreResult, extract_subsequence};

/// An in-memory sequence store suitable for testing and small datasets.
pub struct InMemorySequenceStore {
    /// Map from digest (MD5 or sha512t24u) to (metadata, sequence bytes).
    index: HashMap<String, (SequenceMetadata, Vec<u8>)>,
}

impl InMemorySequenceStore {
    /// Create a new empty store.
    pub fn new() -> Self {
        Self { index: HashMap::new() }
    }

    /// Add a sequence to the store.
    pub fn add(&mut self, metadata: SequenceMetadata, sequence: Vec<u8>) {
        self.index.insert(metadata.md5.clone(), (metadata.clone(), sequence.clone()));
        self.index.insert(metadata.sha512t24u.clone(), (metadata, sequence));
    }
}

impl Default for InMemorySequenceStore {
    fn default() -> Self {
        Self::new()
    }
}

impl SequenceStore for InMemorySequenceStore {
    fn get_sequence(
        &self,
        digest: &str,
        start: Option<u64>,
        end: Option<u64>,
    ) -> StoreResult<Option<Vec<u8>>> {
        let Some((_, seq)) = self.index.get(digest) else {
            return Ok(None);
        };
        Ok(Some(extract_subsequence(seq, start, end)))
    }

    fn get_metadata(&self, digest: &str) -> StoreResult<Option<SequenceMetadata>> {
        Ok(self.index.get(digest).map(|(meta, _)| meta.clone()))
    }

    fn get_length(&self, digest: &str) -> StoreResult<Option<u64>> {
        Ok(self.index.get(digest).map(|(meta, _)| meta.length))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use refget_model::SequenceMetadata;

    fn test_metadata() -> SequenceMetadata {
        SequenceMetadata {
            md5: "abc123".to_string(),
            sha512t24u: "SQ.xyz789".to_string(),
            length: 4,
            aliases: vec![],
            circular: false,
        }
    }

    #[test]
    fn test_add_and_get() {
        let mut store = InMemorySequenceStore::new();
        store.add(test_metadata(), b"ACGT".to_vec());

        let seq = store.get_sequence("abc123", None, None).unwrap().unwrap();
        assert_eq!(seq, b"ACGT");

        let seq = store.get_sequence("SQ.xyz789", None, None).unwrap().unwrap();
        assert_eq!(seq, b"ACGT");
    }

    #[test]
    fn test_subsequence() {
        let mut store = InMemorySequenceStore::new();
        store.add(test_metadata(), b"ACGT".to_vec());

        let seq = store.get_sequence("abc123", Some(1), Some(3)).unwrap().unwrap();
        assert_eq!(seq, b"CG");
    }

    #[test]
    fn test_not_found() {
        let store = InMemorySequenceStore::new();
        assert!(store.get_sequence("missing", None, None).unwrap().is_none());
    }

    #[test]
    fn test_metadata_lookup() {
        let mut store = InMemorySequenceStore::new();
        let meta = test_metadata();
        store.add(meta.clone(), b"ACGT".to_vec());

        let found = store.get_metadata("abc123").unwrap().unwrap();
        assert_eq!(found, meta);
    }

    #[test]
    fn test_get_sequence_start_at_length_returns_empty() {
        let mut store = InMemorySequenceStore::new();
        store.add(test_metadata(), b"ACGT".to_vec());

        let seq = store.get_sequence("abc123", Some(4), None).unwrap().unwrap();
        assert!(seq.is_empty());
    }

    #[test]
    fn test_get_sequence_start_beyond_length_returns_empty() {
        let mut store = InMemorySequenceStore::new();
        store.add(test_metadata(), b"ACGT".to_vec());

        let seq = store.get_sequence("abc123", Some(100), None).unwrap().unwrap();
        assert!(seq.is_empty());
    }

    #[test]
    fn test_get_sequence_end_beyond_length_clamps() {
        let mut store = InMemorySequenceStore::new();
        store.add(test_metadata(), b"ACGT".to_vec());

        let seq = store.get_sequence("abc123", Some(2), Some(100)).unwrap().unwrap();
        assert_eq!(seq, b"GT");
    }

    #[test]
    fn test_get_metadata_non_existent_returns_none() {
        let store = InMemorySequenceStore::new();
        let result = store.get_metadata("no_such_digest").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_add_same_sequence_twice_overwrites() {
        let mut store = InMemorySequenceStore::new();
        let meta = test_metadata();
        store.add(meta.clone(), b"ACGT".to_vec());
        store.add(meta.clone(), b"TTTT".to_vec());

        let seq = store.get_sequence("abc123", None, None).unwrap().unwrap();
        assert_eq!(seq, b"TTTT");

        let seq = store.get_sequence("SQ.xyz789", None, None).unwrap().unwrap();
        assert_eq!(seq, b"TTTT");
    }
}
