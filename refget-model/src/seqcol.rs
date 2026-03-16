//! Sequence Collection types and algorithms for the refget Sequence Collections API.

use std::collections::{BTreeMap, HashSet};

use refget_digest::{digest_json, sha512t24u};
use serde::{Deserialize, Serialize};

/// The level of detail for a sequence collection response.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum Level {
    /// Level 0: a single digest for the entire collection.
    Zero,
    /// Level 1: per-attribute digests.
    One,
    /// Level 2: full attribute arrays.
    Two,
}

impl Level {
    /// Parse a level from an integer (0, 1, or 2).
    pub fn from_int(n: u8) -> Option<Self> {
        match n {
            0 => Some(Self::Zero),
            1 => Some(Self::One),
            2 => Some(Self::Two),
            _ => None,
        }
    }
}

/// A Level 2 sequence collection: the full attribute arrays.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeqCol {
    /// Sequence names (e.g. "chr1", "chr2", ...).
    pub names: Vec<String>,
    /// Sequence lengths.
    pub lengths: Vec<u64>,
    /// GA4GH sha512t24u digests of each sequence.
    pub sequences: Vec<String>,
    /// Optional: sorted name-length pairs digest.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sorted_name_length_pairs: Option<Vec<String>>,
}

/// Level 1 representation: per-attribute digests.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeqColLevel1 {
    pub names: String,
    pub lengths: String,
    pub sequences: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sorted_name_length_pairs: Option<String>,
}

impl SeqCol {
    /// Validate that all arrays have the same length.
    pub fn validate(&self) -> Result<(), SeqColError> {
        let n = self.names.len();
        if self.lengths.len() != n {
            return Err(SeqColError::MismatchedArrayLengths {
                expected: n,
                attribute: "lengths".to_string(),
                actual: self.lengths.len(),
            });
        }
        if self.sequences.len() != n {
            return Err(SeqColError::MismatchedArrayLengths {
                expected: n,
                attribute: "sequences".to_string(),
                actual: self.sequences.len(),
            });
        }
        Ok(())
    }

    /// Compute the Level 0 digest (the single digest for the entire collection).
    ///
    /// This is computed from the inherent attributes (names, lengths, sequences)
    /// by computing per-attribute digests, building a JSON object of those digests,
    /// canonicalizing it with JCS, and hashing with sha512t24u.
    pub fn digest(&self) -> String {
        let level1 = self.to_level1_inherent();
        let obj = serde_json::json!({
            "lengths": level1.lengths,
            "names": level1.names,
            "sequences": level1.sequences,
        });
        digest_json(&obj)
    }

    /// Compute Level 1: per-attribute digests.
    pub fn to_level1(&self) -> SeqColLevel1 {
        let mut level1 = self.to_level1_inherent();
        level1.sorted_name_length_pairs =
            Some(digest_string_array(&self.sorted_name_length_pairs()));
        level1
    }

    /// Compute Level 1 for inherent attributes only.
    fn to_level1_inherent(&self) -> SeqColLevel1 {
        SeqColLevel1 {
            names: digest_string_array(&self.names),
            lengths: digest_u64_array(&self.lengths),
            sequences: digest_string_array(&self.sequences),
            sorted_name_length_pairs: None,
        }
    }

    /// Compute sorted name-length pairs as an array of strings.
    ///
    /// Each element is the sha512t24u of `name:length`, sorted lexicographically.
    pub fn sorted_name_length_pairs(&self) -> Vec<String> {
        let mut pairs = self.name_length_pairs();
        pairs.sort();
        pairs
    }

    /// Compute name-length pairs (unsorted) as an array of digests.
    pub fn name_length_pairs(&self) -> Vec<String> {
        self.names
            .iter()
            .zip(self.lengths.iter())
            .map(|(name, length)| sha512t24u(format!("{name}:{length}").as_bytes()))
            .collect()
    }

    /// Return the collection as a JSON value at the specified level.
    pub fn to_json(&self, level: Level) -> serde_json::Value {
        match level {
            Level::Zero => serde_json::Value::String(self.digest()),
            Level::One => serde_json::to_value(self.to_level1()).unwrap(),
            Level::Two => {
                let mut col = self.clone();
                col.sorted_name_length_pairs = Some(self.sorted_name_length_pairs());
                serde_json::to_value(col).unwrap()
            }
        }
    }
}

/// Compare two sequence collections and produce a comparison result.
pub fn compare(a: &SeqCol, b: &SeqCol) -> ComparisonResult {
    let a_digest = a.digest();
    let b_digest = b.digest();
    // Attribute comparison: both collections have the same inherent attributes
    let a_and_b: Vec<String> = INHERENT_ATTRIBUTES.iter().map(|s| (*s).to_string()).collect();
    let a_only: Vec<String> = vec![];
    let b_only: Vec<String> = vec![];

    // For shared attributes, compute element-level comparison
    let mut array_elements = BTreeMap::new();
    for attr in &a_and_b {
        let (a_vals, b_vals) = get_attribute_strings(a, b, attr);
        let a_set: HashSet<&str> = a_vals.iter().map(String::as_str).collect();
        let b_set: HashSet<&str> = b_vals.iter().map(String::as_str).collect();

        let total_a = a_vals.len();
        let total_b = b_vals.len();
        let a_and_b_count = a_set.intersection(&b_set).count();
        let a_only_count = a_set.difference(&b_set).count();
        let b_only_count = b_set.difference(&a_set).count();
        let order = if a_vals == b_vals { OrderResult::Match } else { OrderResult::Differ };

        array_elements.insert(
            attr.clone(),
            ArrayElementComparison {
                total_a,
                total_b,
                a_and_b: a_and_b_count,
                a_only: a_only_count,
                b_only: b_only_count,
                order,
            },
        );
    }

    ComparisonResult {
        digests: DigestComparison { a: a_digest, b: b_digest },
        attributes: AttributeComparison { a_only, b_only, a_and_b },
        array_elements,
    }
}

/// The three inherent attributes of a sequence collection.
const INHERENT_ATTRIBUTES: &[&str] = &["names", "lengths", "sequences"];

/// Get string representations of attribute values for comparison.
fn get_attribute_strings(a: &SeqCol, b: &SeqCol, attr: &str) -> (Vec<String>, Vec<String>) {
    match attr {
        "names" => (a.names.clone(), b.names.clone()),
        "lengths" => (
            a.lengths.iter().map(|v| v.to_string()).collect(),
            b.lengths.iter().map(|v| v.to_string()).collect(),
        ),
        "sequences" => (a.sequences.clone(), b.sequences.clone()),
        _ => (vec![], vec![]),
    }
}

/// The result of comparing two sequence collections.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComparisonResult {
    pub digests: DigestComparison,
    pub attributes: AttributeComparison,
    pub array_elements: BTreeMap<String, ArrayElementComparison>,
}

/// Digest information for both collections in a comparison.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DigestComparison {
    pub a: String,
    pub b: String,
}

/// Which attributes exist in each collection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttributeComparison {
    pub a_only: Vec<String>,
    pub b_only: Vec<String>,
    pub a_and_b: Vec<String>,
}

/// Element-level comparison for a single attribute.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArrayElementComparison {
    pub total_a: usize,
    pub total_b: usize,
    pub a_and_b: usize,
    pub a_only: usize,
    pub b_only: usize,
    pub order: OrderResult,
}

/// Whether the element order matches between two arrays.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OrderResult {
    Match,
    Differ,
}

/// Errors from sequence collection operations.
#[derive(Debug, thiserror::Error)]
pub enum SeqColError {
    #[error("Array length mismatch: {attribute} has {actual} elements, expected {expected}")]
    MismatchedArrayLengths { expected: usize, attribute: String, actual: usize },
}

/// Compute the sha512t24u digest of an array of strings, by converting to a JSON
/// array and hashing the canonicalized form.
fn digest_string_array(values: &[String]) -> String {
    let json_array: Vec<serde_json::Value> =
        values.iter().map(|v| serde_json::Value::String(v.clone())).collect();
    let json = serde_json::Value::Array(json_array);
    digest_json(&json)
}

/// Compute the sha512t24u digest of an array of u64 values.
fn digest_u64_array(values: &[u64]) -> String {
    let json_array: Vec<serde_json::Value> = values.iter().map(|v| serde_json::json!(v)).collect();
    let json = serde_json::Value::Array(json_array);
    digest_json(&json)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn example_seqcol() -> SeqCol {
        SeqCol {
            names: vec!["chr1".to_string(), "chr2".to_string()],
            lengths: vec![248956422, 242193529],
            sequences: vec![
                "SQ.IIB53T8CNeJJdUqzn1V4W1SqtRA".to_string(),
                "SQ.v7noePfnNpK8ghYXEqZ9NukMXW0".to_string(),
            ],
            sorted_name_length_pairs: None,
        }
    }

    #[test]
    fn test_validate_ok() {
        let col = example_seqcol();
        assert!(col.validate().is_ok());
    }

    #[test]
    fn test_validate_mismatched_lengths() {
        let mut col = example_seqcol();
        col.lengths.push(100);
        assert!(col.validate().is_err());
    }

    #[test]
    fn test_digest_deterministic() {
        let col = example_seqcol();
        let d1 = col.digest();
        let d2 = col.digest();
        assert_eq!(d1, d2);
        assert_eq!(d1.len(), 32);
    }

    #[test]
    fn test_level1() {
        let col = example_seqcol();
        let level1 = col.to_level1();
        assert_eq!(level1.names.len(), 32);
        assert_eq!(level1.lengths.len(), 32);
        assert_eq!(level1.sequences.len(), 32);
        assert!(level1.sorted_name_length_pairs.is_some());
    }

    #[test]
    fn test_sorted_name_length_pairs() {
        let col = example_seqcol();
        let pairs = col.sorted_name_length_pairs();
        assert_eq!(pairs.len(), 2);
        // Each pair is a sha512t24u digest
        for p in &pairs {
            assert_eq!(p.len(), 32);
        }
        // Must be sorted
        assert!(pairs[0] <= pairs[1]);
    }

    #[test]
    fn test_compare_identical() {
        let col = example_seqcol();
        let result = compare(&col, &col);
        assert_eq!(result.digests.a, result.digests.b);
        assert!(result.attributes.a_only.is_empty());
        assert!(result.attributes.b_only.is_empty());
        assert_eq!(result.attributes.a_and_b.len(), 3);
        for elem in result.array_elements.values() {
            assert_eq!(elem.a_only, 0);
            assert_eq!(elem.b_only, 0);
            assert_eq!(elem.order, OrderResult::Match);
        }
    }

    #[test]
    fn test_compare_different() {
        let a = example_seqcol();
        let mut b = example_seqcol();
        b.names[0] = "chrX".to_string();
        let result = compare(&a, &b);
        assert_ne!(result.digests.a, result.digests.b);
        let names_cmp = result.array_elements.get("names").unwrap();
        assert_eq!(names_cmp.a_only, 1);
        assert_eq!(names_cmp.b_only, 1);
    }

    #[test]
    fn test_to_json_levels() {
        let col = example_seqcol();
        let l0 = col.to_json(Level::Zero);
        assert!(l0.is_string());
        let l1 = col.to_json(Level::One);
        assert!(l1.is_object());
        let l2 = col.to_json(Level::Two);
        assert!(l2.is_object());
        assert!(l2.get("names").unwrap().is_array());
    }

    // --- Level::from_int invalid values ---

    #[test]
    fn test_level_from_int_invalid_3() {
        assert!(Level::from_int(3).is_none());
    }

    #[test]
    fn test_level_from_int_invalid_255() {
        assert!(Level::from_int(255).is_none());
    }

    // --- SeqCol::validate with empty arrays ---

    fn empty_seqcol() -> SeqCol {
        SeqCol { names: vec![], lengths: vec![], sequences: vec![], sorted_name_length_pairs: None }
    }

    #[test]
    fn test_validate_all_empty_ok() {
        let col = empty_seqcol();
        assert!(col.validate().is_ok());
    }

    #[test]
    fn test_validate_sequences_length_mismatch() {
        let mut col = example_seqcol();
        col.sequences.push("SQ.extra".to_string());
        let err = col.validate().unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("sequences"), "error should mention 'sequences': {msg}");
    }

    // --- name_length_pairs output ---

    #[test]
    fn test_name_length_pairs_length_and_digest_size() {
        let col = example_seqcol();
        let pairs = col.name_length_pairs();
        assert_eq!(pairs.len(), 2);
        for p in &pairs {
            assert_eq!(p.len(), 32, "each name-length pair digest should be 32 chars");
        }
    }

    // --- compare: completely different collections ---

    #[test]
    fn test_compare_no_overlap() {
        let a = example_seqcol();
        let b = SeqCol {
            names: vec!["chrX".to_string(), "chrY".to_string()],
            lengths: vec![1000, 2000],
            sequences: vec![
                "SQ.aaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
                "SQ.bbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string(),
            ],
            sorted_name_length_pairs: None,
        };
        let result = compare(&a, &b);
        assert_ne!(result.digests.a, result.digests.b);
        for elem in result.array_elements.values() {
            assert_eq!(elem.a_and_b, 0, "no elements should overlap");
            assert_eq!(elem.a_only, elem.total_a);
            assert_eq!(elem.b_only, elem.total_b);
        }
    }

    // --- compare: different-length collections ---

    #[test]
    fn test_compare_different_lengths() {
        let a = example_seqcol();
        let b = SeqCol {
            names: vec!["chr1".to_string()],
            lengths: vec![248956422],
            sequences: vec!["SQ.IIB53T8CNeJJdUqzn1V4W1SqtRA".to_string()],
            sorted_name_length_pairs: None,
        };
        let result = compare(&a, &b);
        let names_cmp = result.array_elements.get("names").unwrap();
        assert_eq!(names_cmp.total_a, 2);
        assert_eq!(names_cmp.total_b, 1);
        assert_eq!(names_cmp.a_and_b, 1);
        assert_eq!(names_cmp.a_only, 1);
        assert_eq!(names_cmp.b_only, 0);
    }

    // --- compare: same elements, different order ---

    #[test]
    fn test_compare_same_elements_different_order() {
        let a = example_seqcol();
        let b = SeqCol {
            names: vec!["chr2".to_string(), "chr1".to_string()],
            lengths: vec![242193529, 248956422],
            sequences: vec![
                "SQ.v7noePfnNpK8ghYXEqZ9NukMXW0".to_string(),
                "SQ.IIB53T8CNeJJdUqzn1V4W1SqtRA".to_string(),
            ],
            sorted_name_length_pairs: None,
        };
        let result = compare(&a, &b);
        // Digests differ because order matters for the level-0 digest
        assert_ne!(result.digests.a, result.digests.b);
        for elem in result.array_elements.values() {
            assert_eq!(elem.order, OrderResult::Differ, "order should differ");
            assert_eq!(elem.a_and_b, elem.total_a, "all elements of a should be in b");
            assert_eq!(elem.a_and_b, elem.total_b, "all elements of b should be in a");
            assert_eq!(elem.a_only, 0);
            assert_eq!(elem.b_only, 0);
        }
    }

    // --- to_json Level::Zero returns a JSON string ---

    #[test]
    fn test_to_json_level_zero_is_string() {
        let col = example_seqcol();
        let json = col.to_json(Level::Zero);
        assert!(json.is_string(), "Level::Zero JSON should be a string");
        assert_eq!(json.as_str().unwrap().len(), 32, "Level::Zero digest should be 32 chars");
    }

    // --- to_json Level::Two includes sorted_name_length_pairs ---

    #[test]
    fn test_to_json_level_two_has_sorted_name_length_pairs() {
        let col = example_seqcol();
        let json = col.to_json(Level::Two);
        let snlp = json.get("sorted_name_length_pairs");
        assert!(snlp.is_some(), "Level::Two should include sorted_name_length_pairs");
        assert!(snlp.unwrap().is_array());
    }

    // --- empty collections produce valid 32-char digests (exercises digest_string_array / digest_u64_array) ---

    #[test]
    fn test_empty_collection_digests_are_valid() {
        let col = empty_seqcol();
        // digest exercises digest_string_array (names, sequences) and digest_u64_array (lengths)
        let d = col.digest();
        assert_eq!(d.len(), 32, "digest of empty collection should be 32 chars");

        let level1 = col.to_level1();
        assert_eq!(level1.names.len(), 32);
        assert_eq!(level1.lengths.len(), 32);
        assert_eq!(level1.sequences.len(), 32);
        // names and sequences are both empty string arrays, so their digests should be equal
        assert_eq!(level1.names, level1.sequences);
    }

    // --- single-element SeqCol: validate, digest, level1 ---

    #[test]
    fn test_single_element_seqcol() {
        let col = SeqCol {
            names: vec!["chrM".to_string()],
            lengths: vec![16569],
            sequences: vec!["SQ.someDigest_chrM_placeholder00".to_string()],
            sorted_name_length_pairs: None,
        };
        assert!(col.validate().is_ok());

        let d = col.digest();
        assert_eq!(d.len(), 32);

        let level1 = col.to_level1();
        assert_eq!(level1.names.len(), 32);
        assert_eq!(level1.lengths.len(), 32);
        assert_eq!(level1.sequences.len(), 32);
        assert!(level1.sorted_name_length_pairs.is_some());
        assert_eq!(level1.sorted_name_length_pairs.unwrap().len(), 32);
    }
}
