//! Storage traits and implementations for refget sequences and sequence collections.

pub mod fasta;
mod memory;
mod mmap;
mod seqcol_store;

pub use fasta::{DigestCache, FastaSequenceStore, FastaSequenceSummary};
pub use memory::InMemorySequenceStore;
pub use mmap::MmapSequenceStore;
pub use seqcol_store::InMemorySeqColStore;

use std::path::{Path, PathBuf};

use refget_model::SequenceMetadata;
use serde::{Deserialize, Serialize};

/// Errors from store operations.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("FASTA index error: {0}")]
    Fasta(String),
    #[error("Sequence not found: {0}")]
    NotFound(String),
}

/// Result type for store operations.
pub type StoreResult<T> = Result<T, StoreError>;

/// Extract a subsequence from `seq` given optional 0-based half-open `start`/`end`.
/// Clamps `end` to the sequence length. Returns empty if `start >= seq.len()`.
pub(crate) fn extract_subsequence(seq: &[u8], start: Option<u64>, end: Option<u64>) -> Vec<u8> {
    let start = start.unwrap_or(0) as usize;
    let end = end.unwrap_or(seq.len() as u64) as usize;
    let end = end.min(seq.len());
    if start >= seq.len() {
        return vec![];
    }
    seq[start..end].to_vec()
}

/// Trait for retrieving sequences and their metadata.
pub trait SequenceStore: Send + Sync {
    /// Retrieve sequence bases by digest (MD5 or sha512t24u).
    /// Supports optional start/end for subsequence retrieval (0-based, half-open).
    fn get_sequence(
        &self,
        digest: &str,
        start: Option<u64>,
        end: Option<u64>,
    ) -> StoreResult<Option<Vec<u8>>>;

    /// Retrieve metadata for a sequence by digest.
    fn get_metadata(&self, digest: &str) -> StoreResult<Option<SequenceMetadata>>;

    /// Retrieve the length of a sequence by digest.
    fn get_length(&self, digest: &str) -> StoreResult<Option<u64>>;
}

/// Result from listing collections.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListResult {
    pub items: Vec<ListItem>,
    pub total: usize,
    pub page: usize,
    pub page_size: usize,
}

/// A single item in a collection listing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListItem {
    pub digest: String,
}

/// Trait for storing and retrieving sequence collections.
pub trait SeqColStore: Send + Sync {
    /// Get a collection by its Level 0 digest.
    fn get_collection(&self, digest: &str) -> Option<&refget_model::SeqCol>;

    /// List collections with optional attribute-based filters, paginated.
    fn list_collections(
        &self,
        filters: &[(String, String)],
        page: usize,
        page_size: usize,
    ) -> ListResult;

    /// Get a single attribute array by attribute name and its digest.
    fn get_attribute(&self, name: &str, digest: &str) -> Option<serde_json::Value>;

    /// Return the total number of collections.
    fn count(&self) -> usize;
}

/// Collect FASTA files from a list of paths (files or directories).
///
/// Directories are searched non-recursively for files with FASTA extensions
/// (`.fa`, `.fasta`, `.fna`, `.fas`). Results are sorted by path.
pub fn collect_fasta_files(paths: &[PathBuf]) -> StoreResult<Vec<PathBuf>> {
    let mut files = Vec::new();
    for path in paths {
        if path.is_dir() {
            let entries = std::fs::read_dir(path)?;
            for entry in entries {
                let p = entry?.path();
                if is_fasta_file(&p) {
                    files.push(p);
                }
            }
        } else if path.is_file() {
            files.push(path.clone());
        } else {
            return Err(StoreError::Fasta(format!("Path does not exist: {}", path.display())));
        }
    }
    files.sort();
    Ok(files)
}

/// Check if a path has a FASTA file extension.
pub fn is_fasta_file(path: &Path) -> bool {
    matches!(path.extension().and_then(|e| e.to_str()), Some("fa" | "fasta" | "fna" | "fas"))
}
