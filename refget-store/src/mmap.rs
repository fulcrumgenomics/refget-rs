//! Memory-mapped FASTA sequence store for disk-backed serving.
//!
//! Uses `memmap2` to memory-map FASTA files and the `.fai` index for
//! random access. Requires a pre-computed `.refget.json` digest cache.

use std::collections::HashMap;
use std::path::Path;

use memmap2::Mmap;
use noodles_fasta::fai;
use refget_model::SequenceMetadata;

use crate::fasta::{DigestCache, FastaSequenceSummary, index_digests, read_fai_index};
use crate::{SequenceStore, StoreError, StoreResult};

/// A sequence store backed by memory-mapped FASTA files.
///
/// Only the digest index and metadata are held in RAM. Sequence bytes are
/// read from the memory-mapped file on each request, with the OS managing
/// page caching.
///
/// Requires a `.refget.json` digest cache for each FASTA file. Use
/// `refget-tools cache` to generate them.
pub struct MmapSequenceStore {
    /// Map from digest to record info index.
    digest_index: HashMap<String, usize>,
    /// Per-sequence metadata and FAI location info.
    records: Vec<MmapRecordInfo>,
    /// Memory-mapped FASTA files, kept alive for the store's lifetime.
    mmaps: Vec<Mmap>,
}

struct MmapRecordInfo {
    mmap_idx: usize,
    metadata: SequenceMetadata,
    /// Byte offset of first base in the FASTA file.
    fai_offset: u64,
    /// Number of sequence bases per line.
    fai_line_bases: u64,
    /// Number of bytes per line (bases + newline).
    fai_line_width: u64,
}

impl MmapSequenceStore {
    /// Create an empty store.
    pub fn new() -> Self {
        Self { digest_index: HashMap::new(), records: Vec::new(), mmaps: Vec::new() }
    }

    /// Mark sequences with matching names as circular.
    pub fn mark_circular(&mut self, circular_names: &[String]) {
        for record in &mut self.records {
            if circular_names.iter().any(|n| n.as_str() == record.metadata.aliases[0].value) {
                record.metadata.circular = true;
            }
        }
    }

    /// Add a FASTA file to the store via memory mapping.
    ///
    /// Requires a fresh `.refget.json` digest cache. Returns an error if
    /// the cache is missing or stale.
    pub fn add_fasta<P: AsRef<Path>>(&mut self, path: P) -> StoreResult<Vec<FastaSequenceSummary>> {
        let path = path.as_ref();

        // Digest cache is mandatory for mmap mode
        let cache = DigestCache::load_if_fresh(path).ok_or_else(|| {
            StoreError::Fasta(format!(
                "Disk mode requires a fresh .refget.json cache for {}. \
                 Run `refget-tools cache {}` first.",
                path.display(),
                path.display()
            ))
        })?;

        // Read the FAI index
        let index = read_fai_index(path)?;
        let fai_records: &[fai::Record] = index.as_ref();

        if cache.sequences.len() != fai_records.len() {
            return Err(StoreError::Fasta(format!(
                "Digest cache has {} entries but FAI index has {} for {}",
                cache.sequences.len(),
                fai_records.len(),
                path.display()
            )));
        }

        // Memory-map the FASTA file.
        // SAFETY: We require that the FASTA file is not modified while the
        // server is running. This is a reasonable constraint for a read-only
        // reference server.
        let file = std::fs::File::open(path)?;
        let mmap = unsafe { Mmap::map(&file) }
            .map_err(|e| StoreError::Fasta(format!("Failed to mmap {}: {e}", path.display())))?;
        let mmap_idx = self.mmaps.len();
        self.mmaps.push(mmap);

        let mut summaries = Vec::new();

        for (cached, fai_rec) in cache.sequences.iter().zip(fai_records.iter()) {
            // Verify FAI length matches cache
            if fai_rec.length() != cached.length {
                return Err(StoreError::Fasta(format!(
                    "FAI/cache length mismatch for {}: FAI says {}, cache says {}",
                    cached.name,
                    fai_rec.length(),
                    cached.length
                )));
            }

            let metadata = cached.to_metadata();
            summaries.push(cached.to_summary());

            let record_idx = self.records.len();
            index_digests(&mut self.digest_index, cached, record_idx);

            self.records.push(MmapRecordInfo {
                mmap_idx,
                metadata,
                fai_offset: fai_rec.offset(),
                fai_line_bases: fai_rec.line_bases(),
                fai_line_width: fai_rec.line_width(),
            });
        }

        Ok(summaries)
    }

    /// Extract bases from a memory-mapped FASTA using FAI offset math.
    ///
    /// Processes whole lines at a time to avoid per-base division/modulo.
    fn extract_bases(&self, info: &MmapRecordInfo, start: u64, end: u64) -> Vec<u8> {
        let mmap = &self.mmaps[info.mmap_idx];
        let len = (end - start) as usize;
        let mut result = Vec::with_capacity(len);
        let mut pos = start;

        while pos < end {
            let line_idx = pos / info.fai_line_bases;
            let col = pos % info.fai_line_bases;
            let line_start = info.fai_offset + line_idx * info.fai_line_width;
            // How many bases we can read from this line
            let remaining_on_line = info.fai_line_bases - col;
            let to_read = remaining_on_line.min(end - pos) as usize;
            let byte_start = (line_start + col) as usize;

            for &b in &mmap[byte_start..byte_start + to_read] {
                result.push(b.to_ascii_uppercase());
            }
            pos += to_read as u64;
        }

        result
    }
}

impl Default for MmapSequenceStore {
    fn default() -> Self {
        Self::new()
    }
}

impl SequenceStore for MmapSequenceStore {
    fn get_sequence(
        &self,
        digest: &str,
        start: Option<u64>,
        end: Option<u64>,
    ) -> StoreResult<Option<Vec<u8>>> {
        let Some(&record_idx) = self.digest_index.get(digest) else {
            return Ok(None);
        };
        let info = &self.records[record_idx];
        let length = info.metadata.length;

        let start = start.unwrap_or(0);
        let end = end.unwrap_or(length).min(length);

        if start >= length {
            return Ok(Some(vec![]));
        }

        Ok(Some(self.extract_bases(info, start, end)))
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

    /// Write a test FASTA with a multi-line sequence and generate its cache.
    fn write_test_fasta_with_cache(dir: &TempDir) -> std::path::PathBuf {
        let fasta_path = dir.path().join("test.fa");
        let mut f = std::fs::File::create(&fasta_path).unwrap();
        // seq1: 8 bases across 2 lines of 4
        writeln!(f, ">seq1").unwrap();
        writeln!(f, "ACGT").unwrap();
        writeln!(f, "NNNN").unwrap();
        // seq2: 4 bases on 1 line
        writeln!(f, ">seq2").unwrap();
        writeln!(f, "TTTT").unwrap();

        // FAI: name, length, offset, line_bases, line_width
        let fai_path = dir.path().join("test.fa.fai");
        let mut fai = std::fs::File::create(&fai_path).unwrap();
        writeln!(fai, "seq1\t8\t6\t4\t5").unwrap();
        writeln!(fai, "seq2\t4\t22\t4\t5").unwrap();

        // Generate digest cache
        let cache = DigestCache::from_fasta(&fasta_path).unwrap();
        cache.write(&fasta_path).unwrap();

        fasta_path
    }

    #[test]
    fn test_mmap_load_and_retrieve() {
        let dir = TempDir::new().unwrap();
        let fasta_path = write_test_fasta_with_cache(&dir);

        let mut store = MmapSequenceStore::new();
        let summaries = store.add_fasta(&fasta_path).unwrap();
        assert_eq!(summaries.len(), 2);

        let seq = store.get_sequence(&summaries[0].sha512t24u, None, None).unwrap().unwrap();
        assert_eq!(seq, b"ACGTNNNN");

        let seq = store.get_sequence(&summaries[1].sha512t24u, None, None).unwrap().unwrap();
        assert_eq!(seq, b"TTTT");
    }

    #[test]
    fn test_mmap_subsequence() {
        let dir = TempDir::new().unwrap();
        let fasta_path = write_test_fasta_with_cache(&dir);

        let mut store = MmapSequenceStore::new();
        let summaries = store.add_fasta(&fasta_path).unwrap();

        // Subsequence within first line
        let seq = store.get_sequence(&summaries[0].sha512t24u, Some(1), Some(3)).unwrap().unwrap();
        assert_eq!(seq, b"CG");

        // Subsequence spanning line boundary
        let seq = store.get_sequence(&summaries[0].sha512t24u, Some(2), Some(6)).unwrap().unwrap();
        assert_eq!(seq, b"GTNN");
    }

    #[test]
    fn test_mmap_metadata() {
        let dir = TempDir::new().unwrap();
        let fasta_path = write_test_fasta_with_cache(&dir);

        let mut store = MmapSequenceStore::new();
        let summaries = store.add_fasta(&fasta_path).unwrap();

        let meta = store.get_metadata(&summaries[0].sha512t24u).unwrap().unwrap();
        assert_eq!(meta.length, 8);
        assert!(meta.sha512t24u.starts_with("SQ."));
    }

    #[test]
    fn test_mmap_get_length() {
        let dir = TempDir::new().unwrap();
        let fasta_path = write_test_fasta_with_cache(&dir);

        let mut store = MmapSequenceStore::new();
        let summaries = store.add_fasta(&fasta_path).unwrap();

        assert_eq!(store.get_length(&summaries[0].sha512t24u).unwrap(), Some(8));
        assert_eq!(store.get_length(&summaries[1].sha512t24u).unwrap(), Some(4));
        assert_eq!(store.get_length("nonexistent").unwrap(), None);
    }

    #[test]
    fn test_mmap_not_found() {
        let store = MmapSequenceStore::new();
        assert!(store.get_sequence("missing", None, None).unwrap().is_none());
        assert!(store.get_metadata("missing").unwrap().is_none());
    }

    #[test]
    fn test_mmap_start_beyond_length() {
        let dir = TempDir::new().unwrap();
        let fasta_path = write_test_fasta_with_cache(&dir);

        let mut store = MmapSequenceStore::new();
        let summaries = store.add_fasta(&fasta_path).unwrap();

        let seq = store.get_sequence(&summaries[0].sha512t24u, Some(100), None).unwrap().unwrap();
        assert!(seq.is_empty());
    }

    #[test]
    fn test_mmap_requires_cache() {
        let dir = TempDir::new().unwrap();
        let fasta_path = dir.path().join("nocache.fa");
        let mut f = std::fs::File::create(&fasta_path).unwrap();
        writeln!(f, ">seq1\nACGT").unwrap();
        let mut fai = std::fs::File::create(dir.path().join("nocache.fa.fai")).unwrap();
        writeln!(fai, "seq1\t4\t6\t4\t5").unwrap();

        let mut store = MmapSequenceStore::new();
        let err = store.add_fasta(&fasta_path).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("requires a fresh .refget.json"), "Unexpected: {msg}");
    }

    #[test]
    fn test_mmap_multiple_fastas() {
        let dir = TempDir::new().unwrap();

        let fa1 = dir.path().join("a.fa");
        let mut f = std::fs::File::create(&fa1).unwrap();
        writeln!(f, ">s1\nAAAA").unwrap();
        let mut fai = std::fs::File::create(dir.path().join("a.fa.fai")).unwrap();
        writeln!(fai, "s1\t4\t4\t4\t5").unwrap();
        DigestCache::from_fasta(&fa1).unwrap().write(&fa1).unwrap();

        let fa2 = dir.path().join("b.fa");
        let mut f = std::fs::File::create(&fa2).unwrap();
        writeln!(f, ">s2\nCCCC").unwrap();
        let mut fai = std::fs::File::create(dir.path().join("b.fa.fai")).unwrap();
        writeln!(fai, "s2\t4\t4\t4\t5").unwrap();
        DigestCache::from_fasta(&fa2).unwrap().write(&fa2).unwrap();

        let mut store = MmapSequenceStore::new();
        let s1 = store.add_fasta(&fa1).unwrap();
        let s2 = store.add_fasta(&fa2).unwrap();

        let seq1 = store.get_sequence(&s1[0].sha512t24u, None, None).unwrap().unwrap();
        assert_eq!(seq1, b"AAAA");
        let seq2 = store.get_sequence(&s2[0].sha512t24u, None, None).unwrap().unwrap();
        assert_eq!(seq2, b"CCCC");
    }

    #[test]
    fn test_mmap_lowercase_uppercased() {
        let dir = TempDir::new().unwrap();
        let fasta_path = dir.path().join("lower.fa");
        let mut f = std::fs::File::create(&fasta_path).unwrap();
        writeln!(f, ">seq1\nacgt").unwrap();
        let mut fai = std::fs::File::create(dir.path().join("lower.fa.fai")).unwrap();
        writeln!(fai, "seq1\t4\t6\t4\t5").unwrap();

        DigestCache::from_fasta(&fasta_path).unwrap().write(&fasta_path).unwrap();

        let mut store = MmapSequenceStore::new();
        let summaries = store.add_fasta(&fasta_path).unwrap();

        let seq = store.get_sequence(&summaries[0].sha512t24u, None, None).unwrap().unwrap();
        assert_eq!(seq, b"ACGT");
    }

    #[test]
    fn test_mmap_matches_memory_store() {
        let dir = TempDir::new().unwrap();
        let fasta_path = write_test_fasta_with_cache(&dir);

        // Load with both stores
        let (mem_store, mem_summaries) =
            crate::fasta::FastaSequenceStore::from_fasta(&fasta_path).unwrap();
        let mut mmap_store = MmapSequenceStore::new();
        let mmap_summaries = mmap_store.add_fasta(&fasta_path).unwrap();

        // Summaries should match
        assert_eq!(mem_summaries.len(), mmap_summaries.len());
        for (a, b) in mem_summaries.iter().zip(mmap_summaries.iter()) {
            assert_eq!(a.name, b.name);
            assert_eq!(a.length, b.length);
            assert_eq!(a.md5, b.md5);
            assert_eq!(a.sha512t24u, b.sha512t24u);
        }

        // Full sequences should match
        for s in &mem_summaries {
            let mem_seq = mem_store.get_sequence(&s.sha512t24u, None, None).unwrap().unwrap();
            let mmap_seq = mmap_store.get_sequence(&s.sha512t24u, None, None).unwrap().unwrap();
            assert_eq!(mem_seq, mmap_seq, "Mismatch for {}", s.name);
        }
    }
}
