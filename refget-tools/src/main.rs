//! CLI tool for computing refget digests from FASTA files and fetching from refget servers.

use std::io::Write;
use std::path::PathBuf;

use clap::{Parser, Subcommand};
use refget_client::RefgetClientBlocking;
use refget_store::{
    DigestCache, FastaSequenceStore, SeqColCache, SidecarCache, collect_fasta_files,
};

#[derive(Parser)]
#[command(
    name = "refget-tools",
    about = "GA4GH refget digest computation, cache generation, and remote server queries"
)]
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
    /// Fetch a sequence from a remote refget server and print to stdout.
    FetchSequence {
        /// Base URL of the refget server (e.g. http://localhost:8080).
        url: String,
        /// Sequence digest (MD5 or ga4gh).
        digest: String,
        /// Start position (0-based, inclusive).
        #[arg(long)]
        start: Option<u64>,
        /// End position (0-based, exclusive).
        #[arg(long)]
        end: Option<u64>,
    },
    /// Fetch sequence metadata from a remote refget server.
    FetchMetadata {
        /// Base URL of the refget server (e.g. http://localhost:8080).
        url: String,
        /// Sequence digest (MD5 or ga4gh).
        digest: String,
    },
    /// Fetch service-info from a remote refget server.
    FetchServiceInfo {
        /// Base URL of the refget server (e.g. http://localhost:8080).
        url: String,
    },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::DigestFasta { fasta } => digest_fasta(&fasta),
        Commands::DigestCollection { fasta } => digest_collection(&fasta),
        Commands::Cache { fasta } => cache_fastas(&fasta),
        Commands::FetchSequence { url, digest, start, end } => {
            fetch_sequence(&url, &digest, start, end)
        }
        Commands::FetchMetadata { url, digest } => fetch_metadata(&url, &digest),
        Commands::FetchServiceInfo { url } => fetch_service_info(&url),
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

        let summaries: Vec<_> = cache.sequences.iter().map(|cd| cd.to_summary()).collect();
        let seqcol_cache = SeqColCache::from_summaries(&summaries);
        let seqcol_path = seqcol_cache.write(fasta_path)?;
        println!("Cached SeqCol to {}", seqcol_path.display());
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
    let col = SeqColCache::from_summaries(&summaries).collection;

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

fn fetch_sequence(
    url: &str,
    digest: &str,
    start: Option<u64>,
    end: Option<u64>,
) -> anyhow::Result<()> {
    let client = RefgetClientBlocking::new(url)?;
    let seq = client.get_sequence(digest, start, end)?;

    match seq {
        Some(bytes) => {
            std::io::stdout().write_all(&bytes)?;
            // Add trailing newline if the output doesn't end with one
            if !bytes.ends_with(b"\n") {
                println!();
            }
        }
        None => anyhow::bail!("Sequence not found: {digest}"),
    }

    Ok(())
}

fn fetch_metadata(url: &str, digest: &str) -> anyhow::Result<()> {
    let client = RefgetClientBlocking::new(url)?;
    match client.get_metadata(digest)? {
        Some(meta) => {
            println!("{}", serde_json::to_string_pretty(&meta)?);
        }
        None => anyhow::bail!("Sequence not found: {digest}"),
    }
    Ok(())
}

fn fetch_service_info(url: &str) -> anyhow::Result<()> {
    let client = RefgetClientBlocking::new(url)?;
    let info = client.get_sequence_service_info()?;
    println!("{}", serde_json::to_string_pretty(&info)?);
    Ok(())
}
