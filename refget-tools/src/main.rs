//! CLI tool for computing refget digests from FASTA files.

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use refget_model::SeqCol;
use refget_store::{DigestCache, FastaSequenceStore, collect_fasta_files};

#[derive(Parser)]
#[command(name = "refget-tools", about = "Compute GA4GH refget digests from FASTA files")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Compute per-sequence digests from a FASTA file.
    /// Outputs TSV: name, length, md5, sha512t24u.
    DigestFasta {
        /// Path to the indexed FASTA file.
        fasta: PathBuf,
    },
    /// Compute the SeqCol Level 0 and Level 1 digests for a FASTA file.
    DigestCollection {
        /// Path to the indexed FASTA file.
        fasta: PathBuf,
    },
    /// Pre-compute digest cache for one or more FASTA files.
    ///
    /// Writes a `.refget.json` sidecar next to each FASTA containing
    /// pre-computed MD5 and sha512t24u digests. The server will use this
    /// cache at startup to skip digest computation.
    Cache {
        /// Paths to indexed FASTA files (or directories containing them).
        #[arg(required = true, num_args = 1..)]
        fasta: Vec<PathBuf>,
    },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::DigestFasta { fasta } => digest_fasta(&fasta),
        Commands::DigestCollection { fasta } => digest_collection(&fasta),
        Commands::Cache { fasta } => cache_fastas(&fasta),
    }
}

fn cache_fastas(paths: &[PathBuf]) -> anyhow::Result<()> {
    let fasta_files = collect_fasta_files(paths)?;
    if fasta_files.is_empty() {
        anyhow::bail!("No FASTA files found in the provided paths");
    }

    for fasta_path in &fasta_files {
        let cache = DigestCache::from_fasta(fasta_path)?;
        let cache_path = cache.write(fasta_path)?;
        println!("Cached {} sequence digests to {}", cache.sequences.len(), cache_path.display());
    }

    Ok(())
}

fn digest_fasta(fasta_path: &PathBuf) -> anyhow::Result<()> {
    let (_store, summaries) = FastaSequenceStore::from_fasta(fasta_path)?;

    println!("name\tlength\tmd5\tsha512t24u");
    for s in &summaries {
        println!("{}\t{}\t{}\t{}", s.name, s.length, s.md5, s.sha512t24u);
    }

    Ok(())
}

fn digest_collection(fasta_path: &PathBuf) -> anyhow::Result<()> {
    let (_store, summaries) = FastaSequenceStore::from_fasta(fasta_path)?;

    let col = SeqCol {
        names: summaries.iter().map(|s| s.name.clone()).collect(),
        lengths: summaries.iter().map(|s| s.length).collect(),
        sequences: summaries.iter().map(|s| s.sha512t24u.clone()).collect(),
        sorted_name_length_pairs: None,
    };

    let level0 = col.digest();
    let level1 = col.to_level1();

    println!("Level 0 digest: {level0}");
    println!();
    println!("Level 1 digests:");
    println!("  names:                     {}", level1.names);
    println!("  lengths:                   {}", level1.lengths);
    println!("  sequences:                 {}", level1.sequences);
    if let Some(snlp) = &level1.sorted_name_length_pairs {
        println!("  sorted_name_length_pairs:  {snlp}");
    }

    Ok(())
}
