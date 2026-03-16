//! Standalone GA4GH refget server binary.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use clap::{Parser, ValueEnum};
use refget_model::SeqCol;
use refget_server::{RefgetConfig, RefgetState, refget_router};
use refget_store::{
    FastaSequenceStore, InMemorySeqColStore, MmapSequenceStore, SequenceStore, collect_fasta_files,
};
use tower_http::cors::CorsLayer;
use tracing::info;

/// How to serve sequence data.
#[derive(Debug, Clone, Copy, ValueEnum)]
enum StoreMode {
    /// Load all sequences into RAM at startup (fast reads, high memory).
    Memory,
    /// Memory-map FASTA files and read from disk on demand (low memory).
    /// Requires pre-computed digest caches (run `refget-tools cache` first).
    Disk,
}

#[derive(Parser)]
#[command(name = "refget-server", about = "GA4GH refget reference server")]
struct Args {
    /// FASTA files or directories containing FASTA files to serve.
    #[arg(long, required = true, num_args = 1..)]
    fasta: Vec<PathBuf>,

    /// Path to YAML configuration file.
    #[arg(long)]
    config: Option<PathBuf>,

    /// Storage mode: `memory` loads sequences into RAM, `disk` memory-maps files.
    #[arg(long, default_value = "memory")]
    mode: StoreMode,

    /// Port to listen on.
    #[arg(long, default_value = "8080")]
    port: u16,

    /// Address to bind to.
    #[arg(long, default_value = "0.0.0.0")]
    address: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let args = Args::parse();

    // Load config from YAML file or use defaults
    let config = match &args.config {
        Some(path) => {
            info!("Loading config from {}", path.display());
            let contents = std::fs::read_to_string(path)?;
            serde_yaml::from_str(&contents)?
        }
        None => RefgetConfig::default(),
    };

    // Collect all FASTA files
    let fasta_files = collect_fasta_files(&args.fasta)?;
    if fasta_files.is_empty() {
        anyhow::bail!("No FASTA files found in the provided paths");
    }

    info!("Loading {} FASTA file(s) in {:?} mode...", fasta_files.len(), args.mode);

    // Load sequences + build SeqCol store
    let (sequence_store, seqcol_store) = match args.mode {
        StoreMode::Memory => load_memory_mode(&fasta_files)?,
        StoreMode::Disk => load_disk_mode(&fasta_files)?,
    };

    let state = RefgetState { sequence_store, seqcol_store, config };

    let app = refget_router(state).layer(CorsLayer::permissive());
    let addr: SocketAddr = format!("{}:{}", args.address, args.port).parse()?;
    info!("Starting refget server on {addr}");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

/// Load all FASTAs into RAM using `FastaSequenceStore` directly (no copy).
fn load_memory_mode(
    fasta_files: &[PathBuf],
) -> anyhow::Result<(Arc<dyn SequenceStore>, Arc<dyn refget_store::SeqColStore>)> {
    let mut store = FastaSequenceStore::new();
    let mut seqcol_store = InMemorySeqColStore::new();

    for fasta_path in fasta_files {
        info!("  Loading {}", fasta_path.display());
        let summaries = store.add_fasta(fasta_path)?;
        add_seqcol(&mut seqcol_store, &summaries);
    }

    Ok((Arc::new(store), Arc::new(seqcol_store)))
}

/// Memory-map all FASTAs for disk-backed serving.
fn load_disk_mode(
    fasta_files: &[PathBuf],
) -> anyhow::Result<(Arc<dyn SequenceStore>, Arc<dyn refget_store::SeqColStore>)> {
    let mut store = MmapSequenceStore::new();
    let mut seqcol_store = InMemorySeqColStore::new();

    for fasta_path in fasta_files {
        info!("  Mapping {}", fasta_path.display());
        let summaries = store.add_fasta(fasta_path)?;
        add_seqcol(&mut seqcol_store, &summaries);
    }

    Ok((Arc::new(store), Arc::new(seqcol_store)))
}

/// Build a SeqCol from summaries and add it to the store.
fn add_seqcol(
    seqcol_store: &mut InMemorySeqColStore,
    summaries: &[refget_store::fasta::FastaSequenceSummary],
) {
    let col = SeqCol {
        names: summaries.iter().map(|s| s.name.clone()).collect(),
        lengths: summaries.iter().map(|s| s.length).collect(),
        sequences: summaries.iter().map(|s| s.sha512t24u.clone()).collect(),
        sorted_name_length_pairs: None,
    };
    let digest = col.digest();
    info!("    {} sequences, collection digest: {}", summaries.len(), digest);
    seqcol_store.add(col);
}
