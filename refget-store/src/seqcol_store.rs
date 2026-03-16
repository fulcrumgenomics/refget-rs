//! In-memory sequence collection store.

use std::collections::HashMap;

use refget_digest::digest_json;
use refget_model::SeqCol;

use crate::{ListItem, ListResult, SeqColStore};

/// An in-memory store of sequence collections, indexed by Level 0 digest
/// and by per-attribute digests.
pub struct InMemorySeqColStore {
    /// Map from Level 0 digest to SeqCol.
    collections: HashMap<String, SeqCol>,
    /// Map from (attribute_name, attribute_digest) to list of collection digests.
    attribute_index: HashMap<(String, String), Vec<String>>,
    /// Map from (attribute_name, attribute_digest) to the attribute JSON value.
    attribute_values: HashMap<(String, String), serde_json::Value>,
    /// Ordered list of all collection digests.
    digests: Vec<String>,
}

impl InMemorySeqColStore {
    /// Create a new empty store.
    pub fn new() -> Self {
        Self {
            collections: HashMap::new(),
            attribute_index: HashMap::new(),
            attribute_values: HashMap::new(),
            digests: Vec::new(),
        }
    }

    /// Add a sequence collection to the store. Computes and indexes all attribute digests.
    pub fn add(&mut self, col: SeqCol) {
        let digest = col.digest();

        // Index inherent attributes
        self.index_attribute("names", &col.names, &digest);
        self.index_attribute_u64("lengths", &col.lengths, &digest);
        self.index_attribute("sequences", &col.sequences, &digest);

        // Index computed attributes
        let snlp = col.sorted_name_length_pairs();
        self.index_attribute("sorted_name_length_pairs", &snlp, &digest);

        let nlp = col.name_length_pairs();
        self.index_attribute("name_length_pairs", &nlp, &digest);

        self.digests.push(digest.clone());
        self.collections.insert(digest, col);
    }

    fn index_attribute(&mut self, name: &str, values: &[String], collection_digest: &str) {
        let json_array: Vec<serde_json::Value> =
            values.iter().map(|v| serde_json::Value::String(v.clone())).collect();
        let json = serde_json::Value::Array(json_array.clone());
        let attr_digest = digest_json(&json);

        let key = (name.to_string(), attr_digest.clone());
        self.attribute_index.entry(key.clone()).or_default().push(collection_digest.to_string());
        self.attribute_values.entry(key).or_insert(serde_json::Value::Array(json_array));
    }

    fn index_attribute_u64(&mut self, name: &str, values: &[u64], collection_digest: &str) {
        let json_array: Vec<serde_json::Value> =
            values.iter().map(|v| serde_json::json!(v)).collect();
        let json = serde_json::Value::Array(json_array.clone());
        let attr_digest = digest_json(&json);

        let key = (name.to_string(), attr_digest.clone());
        self.attribute_index.entry(key.clone()).or_default().push(collection_digest.to_string());
        self.attribute_values.entry(key).or_insert(serde_json::Value::Array(json_array));
    }
}

impl Default for InMemorySeqColStore {
    fn default() -> Self {
        Self::new()
    }
}

impl SeqColStore for InMemorySeqColStore {
    fn get_collection(&self, digest: &str) -> Option<&SeqCol> {
        self.collections.get(digest)
    }

    fn list_collections(
        &self,
        filters: &[(String, String)],
        page: usize,
        page_size: usize,
    ) -> ListResult {
        // Start with all digests, then narrow by filters
        let mut matching: Option<Vec<&str>> = None;

        for (attr_name, attr_digest) in filters {
            let key = (attr_name.clone(), attr_digest.clone());
            if let Some(collection_digests) = self.attribute_index.get(&key) {
                let set: Vec<&str> = collection_digests.iter().map(String::as_str).collect();
                matching = Some(match matching {
                    None => set,
                    Some(prev) => prev.into_iter().filter(|d| set.contains(d)).collect(),
                });
            } else {
                return ListResult { items: vec![], total: 0, page, page_size };
            }
        }

        let all_digests: Vec<&str> = match matching {
            Some(m) => m,
            None => self.digests.iter().map(String::as_str).collect(),
        };

        let total = all_digests.len();
        let start = page * page_size;
        let items: Vec<ListItem> = all_digests
            .into_iter()
            .skip(start)
            .take(page_size)
            .map(|d| ListItem { digest: d.to_string() })
            .collect();

        ListResult { items, total, page, page_size }
    }

    fn get_attribute(&self, name: &str, digest: &str) -> Option<serde_json::Value> {
        let key = (name.to_string(), digest.to_string());
        self.attribute_values.get(&key).cloned()
    }

    fn count(&self) -> usize {
        self.collections.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use refget_model::SeqCol;

    fn example_col() -> SeqCol {
        SeqCol {
            names: vec!["chr1".to_string(), "chr2".to_string()],
            lengths: vec![100, 200],
            sequences: vec!["SQ.abc".to_string(), "SQ.def".to_string()],
            sorted_name_length_pairs: None,
        }
    }

    #[test]
    fn test_add_and_get() {
        let mut store = InMemorySeqColStore::new();
        let col = example_col();
        let digest = col.digest();
        store.add(col.clone());

        let found = store.get_collection(&digest).unwrap();
        assert_eq!(found.names, col.names);
    }

    #[test]
    fn test_list_no_filters() {
        let mut store = InMemorySeqColStore::new();
        store.add(example_col());
        let result = store.list_collections(&[], 0, 10);
        assert_eq!(result.total, 1);
        assert_eq!(result.items.len(), 1);
    }

    #[test]
    fn test_count() {
        let mut store = InMemorySeqColStore::new();
        assert_eq!(store.count(), 0);
        store.add(example_col());
        assert_eq!(store.count(), 1);
    }

    #[test]
    fn test_get_attribute() {
        let mut store = InMemorySeqColStore::new();
        let col = example_col();
        let level1 = col.to_level1();
        store.add(col);

        let names = store.get_attribute("names", &level1.names);
        assert!(names.is_some());
        let names_arr = names.unwrap();
        assert!(names_arr.is_array());
        assert_eq!(names_arr.as_array().unwrap().len(), 2);
    }

    fn make_col(name: &str) -> SeqCol {
        SeqCol {
            names: vec![name.to_string()],
            lengths: vec![42],
            sequences: vec![format!("SQ.{name}")],
            sorted_name_length_pairs: None,
        }
    }

    #[test]
    fn test_list_collections_pagination() {
        let mut store = InMemorySeqColStore::new();
        let col_a = make_col("a");
        let col_b = make_col("b");
        let col_c = make_col("c");
        let digest_b = col_b.digest();
        store.add(col_a);
        store.add(col_b);
        store.add(col_c);

        // page_size=1, page=1 should return the second collection
        let result = store.list_collections(&[], 1, 1);
        assert_eq!(result.total, 3);
        assert_eq!(result.items.len(), 1);
        assert_eq!(result.items[0].digest, digest_b);
    }

    #[test]
    fn test_list_collections_filter_matches_nothing() {
        let mut store = InMemorySeqColStore::new();
        store.add(example_col());

        let filters = vec![("names".to_string(), "nonexistent_digest".to_string())];
        let result = store.list_collections(&filters, 0, 10);
        assert_eq!(result.total, 0);
        assert!(result.items.is_empty());
    }

    #[test]
    fn test_list_collections_page_beyond_available() {
        let mut store = InMemorySeqColStore::new();
        store.add(example_col());

        let result = store.list_collections(&[], 100, 10);
        assert!(result.items.is_empty());
        assert_eq!(result.total, 1);
    }

    #[test]
    fn test_get_attribute_invalid_name_returns_none() {
        let mut store = InMemorySeqColStore::new();
        let col = example_col();
        let level1 = col.to_level1();
        store.add(col);

        // Use a valid digest but an invalid attribute name
        let result = store.get_attribute("not_a_real_attribute", &level1.names);
        assert!(result.is_none());
    }

    #[test]
    fn test_get_attribute_invalid_digest_returns_none() {
        let mut store = InMemorySeqColStore::new();
        store.add(example_col());

        let result = store.get_attribute("names", "bogus_digest");
        assert!(result.is_none());
    }

    #[test]
    fn test_get_collection_non_existent_returns_none() {
        let store = InMemorySeqColStore::new();
        assert!(store.get_collection("no_such_digest").is_none());
    }
}
