//! FASTA-backed sequence store that loads sequences into memory.

use std::collections::HashMap;
use std::io::BufReader;
use std::path::Path;

use md5::{Digest, Md5};
use noodles_fasta as fasta;
use noodles_fasta::fai;
use refget_digest::sha512t24u;
use refget_model::{Alias, SequenceMetadata};
use serde::{Deserialize, Serialize};

use crate::{SequenceStore, StoreError, StoreResult, extract_subsequence};

/// A sequence store that holds all sequences in memory, loaded from indexed FASTA files.
///
/// Supports loading multiple FASTA files incrementally via [`add_fasta`](Self::add_fasta).
/// Digest computation is skipped when a fresh `.refget.json` cache exists.
pub struct FastaSequenceStore {
    /// Map from digest (MD5 hex, `SQ.`-prefixed, or bare sha512t24u) to record index.
    digest_index: HashMap<String, usize>,
    /// Loaded records.
    records: Vec<RecordData>,
}

struct RecordData {
    _name: String,
    sequence: Vec<u8>,
    metadata: SequenceMetadata,
}

/// Summary info for each sequence loaded from a FASTA file, useful for
/// building SeqCol objects.
#[derive(Debug, Clone)]
pub struct FastaSequenceSummary {
    pub name: String,
    pub length: u64,
    pub md5: String,
    pub sha512t24u: String,
    pub circular: bool,
}

/// Pre-computed digest cache for a FASTA file.
///
/// Written as `{fasta_path}.refget.json`. When present and up-to-date,
/// the server skips digest computation at startup.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DigestCache {
    pub sequences: Vec<CachedDigest>,
}

/// A single sequence's pre-computed digests.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedDigest {
    pub name: String,
    pub length: u64,
    pub md5: String,
    /// GA4GH digest with `SQ.` prefix.
    pub sha512t24u: String,
    /// Whether this sequence is circular.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub circular: bool,
}

impl CachedDigest {
    /// Build a `SequenceMetadata` from this cached digest.
    pub fn to_metadata(&self) -> SequenceMetadata {
        SequenceMetadata {
            md5: self.md5.clone(),
            sha512t24u: self.sha512t24u.clone(),
            length: self.length,
            aliases: vec![Alias {
                naming_authority: "refseq".to_string(),
                value: self.name.clone(),
            }],
            circular: self.circular,
        }
    }

    /// Build a `FastaSequenceSummary` from this cached digest.
    pub fn to_summary(&self) -> FastaSequenceSummary {
        FastaSequenceSummary {
            name: self.name.clone(),
            length: self.length,
            md5: self.md5.clone(),
            sha512t24u: self.sha512t24u.clone(),
            circular: self.circular,
        }
    }
}

impl DigestCache {
    /// Compute digests for all sequences in a FASTA file.
    pub fn from_fasta<P: AsRef<Path>>(path: P) -> StoreResult<Self> {
        let path = path.as_ref();
        let index = read_fai_index(path)?;
        let index_records: &[fai::Record] = index.as_ref();

        let file = std::fs::File::open(path)?;
        let mut reader = fasta::io::Reader::new(BufReader::new(file));
        let mut sequences = Vec::new();

        for (idx, result) in reader.records().enumerate() {
            let record = result
                .map_err(|e| StoreError::Fasta(format!("Failed to read FASTA record: {e}")))?;

            let name = std::str::from_utf8(record.name())
                .map_err(|e| StoreError::Fasta(format!("Invalid UTF-8 in sequence name: {e}")))?
                .to_string();
            let seq_bytes: Vec<u8> = record
                .sequence()
                .as_ref()
                .iter()
                .copied()
                .filter(|b| !b.is_ascii_whitespace())
                .map(|b| b.to_ascii_uppercase())
                .collect();

            let length = seq_bytes.len() as u64;

            if let Some(fai_record) = index_records.get(idx) {
                let expected_length = fai_record.length() as usize;
                if expected_length != seq_bytes.len() {
                    return Err(StoreError::Fasta(format!(
                        "Sequence length mismatch for {name}: index says {expected_length}, got {}",
                        seq_bytes.len()
                    )));
                }
            }

            let md5 = format!("{:x}", Md5::digest(&seq_bytes));
            let sha512t24u = format!("SQ.{}", sha512t24u(&seq_bytes));

            sequences.push(CachedDigest { name, length, md5, sha512t24u, circular: false });
        }

        Ok(Self { sequences })
    }

    /// Return the standard cache path for a given FASTA file.
    pub fn cache_path_for<P: AsRef<Path>>(fasta_path: P) -> std::path::PathBuf {
        let p = fasta_path.as_ref();
        let ext = p
            .extension()
            .map(|e| format!("{}.refget.json", e.to_string_lossy()))
            .unwrap_or_else(|| "refget.json".to_string());
        p.with_extension(ext)
    }

    /// Write the cache to its standard path.
    pub fn write<P: AsRef<Path>>(&self, fasta_path: P) -> StoreResult<std::path::PathBuf> {
        let cache_path = Self::cache_path_for(&fasta_path);
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| StoreError::Fasta(format!("Failed to serialize digest cache: {e}")))?;
        std::fs::write(&cache_path, json)?;
        Ok(cache_path)
    }

    /// Try to load a cache file for the given FASTA, returning `None` if
    /// the cache is missing or stale (older than the FASTA or its index).
    pub fn load_if_fresh<P: AsRef<Path>>(fasta_path: P) -> Option<Self> {
        let fasta_path = fasta_path.as_ref();
        let cache_path = Self::cache_path_for(fasta_path);
        let cache_meta = std::fs::metadata(&cache_path).ok()?;
        let cache_mtime = cache_meta.modified().ok()?;

        // Cache must be newer than both the FASTA and the .fai
        let fasta_mtime = std::fs::metadata(fasta_path).ok()?.modified().ok()?;
        if cache_mtime < fasta_mtime {
            return None;
        }
        let fai_path = fai_path_for(fasta_path);
        if let Ok(fai_meta) = std::fs::metadata(&fai_path)
            && let Ok(fai_mtime) = fai_meta.modified()
            && cache_mtime < fai_mtime
        {
            return None;
        }

        let data = std::fs::read_to_string(&cache_path).ok()?;
        serde_json::from_str(&data).ok()
    }
}

/// Return the .fai index path for a given FASTA path.
pub(crate) fn fai_path_for(path: &Path) -> std::path::PathBuf {
    path.with_extension(
        path.extension()
            .map(|e| format!("{}.fai", e.to_string_lossy()))
            .unwrap_or_else(|| "fai".to_string()),
    )
}

/// Read and parse a `.fai` index file.
pub(crate) fn read_fai_index(path: &Path) -> StoreResult<fai::Index> {
    let fai_path = fai_path_for(path);
    if !fai_path.exists() {
        return Err(StoreError::Fasta(format!("FASTA index not found: {}", fai_path.display())));
    }
    let fai_reader = BufReader::new(std::fs::File::open(&fai_path)?);
    fai::Reader::new(fai_reader)
        .read_index()
        .map_err(|e| StoreError::Fasta(format!("Failed to read FASTA index: {e}")))
}

/// Index a sequence by its digests (md5, SQ.-prefixed, and bare sha512t24u).
pub(crate) fn index_digests(
    digest_index: &mut HashMap<String, usize>,
    cached: &CachedDigest,
    record_idx: usize,
) {
    digest_index.insert(cached.md5.clone(), record_idx);
    digest_index.insert(cached.sha512t24u.clone(), record_idx);
    let bare_sha = cached.sha512t24u.strip_prefix("SQ.").unwrap_or(&cached.sha512t24u).to_string();
    digest_index.insert(bare_sha, record_idx);
}

impl FastaSequenceStore {
    /// Create an empty store.
    pub fn new() -> Self {
        Self { digest_index: HashMap::new(), records: Vec::new() }
    }

    /// Load a FASTA file into this store, appending to existing data.
    ///
    /// Uses a digest cache if fresh, otherwise computes digests on the fly.
    /// Returns summaries for the sequences loaded from this file.
    pub fn add_fasta<P: AsRef<Path>>(&mut self, path: P) -> StoreResult<Vec<FastaSequenceSummary>> {
        let path = path.as_ref();
        let cache = DigestCache::load_if_fresh(path);
        let computed;
        let cache = match &cache {
            Some(c) => c,
            None => {
                computed = DigestCache::from_fasta(path)?;
                &computed
            }
        };
        self.add_fasta_with_cache(path, cache)
    }

    /// Load a FASTA file using pre-computed digests from a cache.
    fn add_fasta_with_cache(
        &mut self,
        path: &Path,
        cache: &DigestCache,
    ) -> StoreResult<Vec<FastaSequenceSummary>> {
        let file = std::fs::File::open(path)?;
        let mut reader = fasta::io::Reader::new(BufReader::new(file));
        let mut summaries = Vec::new();

        for (idx, result) in reader.records().enumerate() {
            let record = result
                .map_err(|e| StoreError::Fasta(format!("Failed to read FASTA record: {e}")))?;

            let seq_bytes: Vec<u8> = record
                .sequence()
                .as_ref()
                .iter()
                .copied()
                .filter(|b| !b.is_ascii_whitespace())
                .map(|b| b.to_ascii_uppercase())
                .collect();

            let cached = cache.sequences.get(idx).ok_or_else(|| {
                StoreError::Fasta(format!(
                    "Digest cache has {} entries but FASTA has more sequences",
                    cache.sequences.len()
                ))
            })?;

            if cached.length != seq_bytes.len() as u64 {
                return Err(StoreError::Fasta(format!(
                    "Cache length mismatch for {}: cache says {}, got {}",
                    cached.name,
                    cached.length,
                    seq_bytes.len()
                )));
            }

            let metadata = cached.to_metadata();
            summaries.push(cached.to_summary());

            let record_idx = self.records.len();
            index_digests(&mut self.digest_index, cached, record_idx);
            self.records.push(RecordData {
                _name: cached.name.clone(),
                sequence: seq_bytes,
                metadata,
            });
        }

        Ok(summaries)
    }

    /// Mark sequences with matching names as circular.
    pub fn mark_circular(&mut self, circular_names: &[String]) {
        for record in &mut self.records {
            if circular_names.iter().any(|n| n == &record._name) {
                record.metadata.circular = true;
            }
        }
    }

    /// Convenience: load a single FASTA file and return the store + summaries.
    pub fn from_fasta<P: AsRef<Path>>(path: P) -> StoreResult<(Self, Vec<FastaSequenceSummary>)> {
        let mut store = Self::new();
        let summaries = store.add_fasta(path)?;
        Ok((store, summaries))
    }
}

impl Default for FastaSequenceStore {
    fn default() -> Self {
        Self::new()
    }
}

impl SequenceStore for FastaSequenceStore {
    fn get_sequence(
        &self,
        digest: &str,
        start: Option<u64>,
        end: Option<u64>,
    ) -> StoreResult<Option<Vec<u8>>> {
        let Some(&record_idx) = self.digest_index.get(digest) else {
            return Ok(None);
        };
        let seq = &self.records[record_idx].sequence;
        Ok(Some(extract_subsequence(seq, start, end)))
    }

    fn get_metadata(&self, digest: &str) -> StoreResult<Option<SequenceMetadata>> {
        Ok(self.digest_index.get(digest).map(|&idx| self.records[idx].metadata.clone()))
    }

    fn get_length(&self, digest: &str) -> StoreResult<Option<u64>> {
        Ok(self.digest_index.get(digest).map(|&idx| self.records[idx].metadata.length))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    fn write_test_fasta(dir: &TempDir) -> std::path::PathBuf {
        let fasta_path = dir.path().join("test.fa");
        let mut f = std::fs::File::create(&fasta_path).unwrap();
        writeln!(f, ">seq1").unwrap();
        writeln!(f, "ACGTACGT").unwrap();
        writeln!(f, ">seq2").unwrap();
        writeln!(f, "NNNN").unwrap();

        let fai_path = dir.path().join("test.fa.fai");
        let mut fai = std::fs::File::create(&fai_path).unwrap();
        writeln!(fai, "seq1\t8\t6\t8\t9").unwrap();
        writeln!(fai, "seq2\t4\t21\t4\t5").unwrap();

        fasta_path
    }

    #[test]
    fn test_load_fasta() {
        let dir = TempDir::new().unwrap();
        let fasta_path = write_test_fasta(&dir);

        let (store, summaries) = FastaSequenceStore::from_fasta(&fasta_path).unwrap();
        assert_eq!(summaries.len(), 2);
        assert_eq!(summaries[0].name, "seq1");
        assert_eq!(summaries[0].length, 8);
        assert_eq!(summaries[1].name, "seq2");
        assert_eq!(summaries[1].length, 4);

        let seq = store.get_sequence(&summaries[0].sha512t24u, None, None).unwrap().unwrap();
        assert_eq!(seq, b"ACGTACGT");

        let seq = store.get_sequence(&summaries[0].md5, None, None).unwrap().unwrap();
        assert_eq!(seq, b"ACGTACGT");
    }

    #[test]
    fn test_add_multiple_fastas() {
        let dir = TempDir::new().unwrap();

        // First FASTA
        let fa1 = dir.path().join("a.fa");
        let mut f = std::fs::File::create(&fa1).unwrap();
        writeln!(f, ">seq1\nACGT").unwrap();
        let mut fai = std::fs::File::create(dir.path().join("a.fa.fai")).unwrap();
        writeln!(fai, "seq1\t4\t6\t4\t5").unwrap();

        // Second FASTA
        let fa2 = dir.path().join("b.fa");
        let mut f = std::fs::File::create(&fa2).unwrap();
        writeln!(f, ">seq2\nTTTT").unwrap();
        let mut fai = std::fs::File::create(dir.path().join("b.fa.fai")).unwrap();
        writeln!(fai, "seq2\t4\t6\t4\t5").unwrap();

        let mut store = FastaSequenceStore::new();
        let s1 = store.add_fasta(&fa1).unwrap();
        let s2 = store.add_fasta(&fa2).unwrap();

        assert_eq!(s1.len(), 1);
        assert_eq!(s2.len(), 1);

        let seq1 = store.get_sequence(&s1[0].sha512t24u, None, None).unwrap().unwrap();
        assert_eq!(seq1, b"ACGT");
        let seq2 = store.get_sequence(&s2[0].sha512t24u, None, None).unwrap().unwrap();
        assert_eq!(seq2, b"TTTT");
    }

    #[test]
    fn test_subsequence() {
        let dir = TempDir::new().unwrap();
        let fasta_path = write_test_fasta(&dir);

        let (store, summaries) = FastaSequenceStore::from_fasta(&fasta_path).unwrap();
        let seq = store.get_sequence(&summaries[0].sha512t24u, Some(2), Some(5)).unwrap().unwrap();
        assert_eq!(seq, b"GTA");
    }

    #[test]
    fn test_metadata() {
        let dir = TempDir::new().unwrap();
        let fasta_path = write_test_fasta(&dir);

        let (store, summaries) = FastaSequenceStore::from_fasta(&fasta_path).unwrap();
        let meta = store.get_metadata(&summaries[0].sha512t24u).unwrap().unwrap();
        assert_eq!(meta.length, 8);
        assert_eq!(meta.sha512t24u, summaries[0].sha512t24u);
        assert!(meta.sha512t24u.starts_with("SQ."), "ga4gh digest must have SQ. prefix");
        assert_eq!(meta.md5, summaries[0].md5);
    }

    #[test]
    fn test_from_fasta_missing_fai_errors() {
        let dir = TempDir::new().unwrap();
        let fasta_path = dir.path().join("no_index.fa");
        let mut f = std::fs::File::create(&fasta_path).unwrap();
        writeln!(f, ">seq1").unwrap();
        writeln!(f, "ACGT").unwrap();

        let result = FastaSequenceStore::from_fasta(&fasta_path);
        let err_msg = format!("{}", result.err().expect("Expected an error for missing .fai file"));
        assert!(err_msg.contains("FASTA index not found"), "Unexpected error: {err_msg}");
    }

    #[test]
    fn test_from_fasta_empty_fasta() {
        let dir = TempDir::new().unwrap();
        let fasta_path = dir.path().join("empty.fa");
        std::fs::File::create(&fasta_path).unwrap();
        std::fs::File::create(dir.path().join("empty.fa.fai")).unwrap();

        let (store, summaries) = FastaSequenceStore::from_fasta(&fasta_path).unwrap();
        assert!(summaries.is_empty());
        assert!(store.get_sequence("anything", None, None).unwrap().is_none());
    }

    #[test]
    fn test_get_sequence_non_existent_digest_returns_none() {
        let dir = TempDir::new().unwrap();
        let fasta_path = write_test_fasta(&dir);

        let (store, _) = FastaSequenceStore::from_fasta(&fasta_path).unwrap();
        assert!(store.get_sequence("no_such_digest", None, None).unwrap().is_none());
    }

    #[test]
    fn test_get_sequence_start_beyond_length_returns_empty() {
        let dir = TempDir::new().unwrap();
        let fasta_path = write_test_fasta(&dir);

        let (store, summaries) = FastaSequenceStore::from_fasta(&fasta_path).unwrap();
        let seq = store.get_sequence(&summaries[0].sha512t24u, Some(100), None).unwrap().unwrap();
        assert!(seq.is_empty());
    }

    #[test]
    fn test_lowercase_sequences_are_uppercased() {
        let dir = TempDir::new().unwrap();
        let fasta_path = dir.path().join("lower.fa");
        let mut f = std::fs::File::create(&fasta_path).unwrap();
        writeln!(f, ">seq1").unwrap();
        writeln!(f, "acgtACGT").unwrap();

        let fai_path = dir.path().join("lower.fa.fai");
        let mut fai = std::fs::File::create(&fai_path).unwrap();
        writeln!(fai, "seq1\t8\t6\t8\t9").unwrap();

        let (store, summaries) = FastaSequenceStore::from_fasta(&fasta_path).unwrap();
        let seq = store.get_sequence(&summaries[0].sha512t24u, None, None).unwrap().unwrap();
        assert_eq!(seq, b"ACGTACGT");
    }

    #[test]
    fn test_digest_cache_write_and_load() {
        let dir = TempDir::new().unwrap();
        let fasta_path = write_test_fasta(&dir);

        let cache = DigestCache::from_fasta(&fasta_path).unwrap();
        assert_eq!(cache.sequences.len(), 2);
        let cache_path = cache.write(&fasta_path).unwrap();
        assert!(cache_path.exists());
        assert!(cache_path.to_string_lossy().ends_with(".refget.json"));

        let loaded = DigestCache::load_if_fresh(&fasta_path).unwrap();
        assert_eq!(loaded.sequences.len(), 2);
        assert_eq!(loaded.sequences[0].name, "seq1");
        assert!(loaded.sequences[0].sha512t24u.starts_with("SQ."));
    }

    #[test]
    fn test_from_fasta_with_cache_matches_without() {
        let dir = TempDir::new().unwrap();
        let fasta_path = write_test_fasta(&dir);

        let (_, summaries_no_cache) = FastaSequenceStore::from_fasta(&fasta_path).unwrap();

        let cache = DigestCache::from_fasta(&fasta_path).unwrap();
        cache.write(&fasta_path).unwrap();
        let (store, summaries_cached) = FastaSequenceStore::from_fasta(&fasta_path).unwrap();

        assert_eq!(summaries_no_cache.len(), summaries_cached.len());
        for (a, b) in summaries_no_cache.iter().zip(summaries_cached.iter()) {
            assert_eq!(a.name, b.name);
            assert_eq!(a.length, b.length);
            assert_eq!(a.md5, b.md5);
            assert_eq!(a.sha512t24u, b.sha512t24u);
        }

        let seq = store.get_sequence(&summaries_cached[0].sha512t24u, None, None).unwrap().unwrap();
        assert_eq!(seq, b"ACGTACGT");
    }

    #[test]
    fn test_stale_cache_is_ignored() {
        let dir = TempDir::new().unwrap();
        let fasta_path = write_test_fasta(&dir);

        let cache = DigestCache::from_fasta(&fasta_path).unwrap();
        cache.write(&fasta_path).unwrap();

        std::thread::sleep(std::time::Duration::from_millis(50));
        let mut f = std::fs::OpenOptions::new().append(true).open(&fasta_path).unwrap();
        writeln!(f, ">seq3").unwrap();
        writeln!(f, "TTTT").unwrap();

        assert!(DigestCache::load_if_fresh(&fasta_path).is_none());
    }
}
