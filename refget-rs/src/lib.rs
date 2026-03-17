//! GA4GH refget Sequences v2.0.0 and Sequence Collections v1.0.0 in Rust.
//!
//! This is an umbrella crate that re-exports the individual refget-rs workspace crates.
//! Enable features to pull in the components you need:
//!
//! - **`digest`** — SHA-512/24 and JSON canonicalization ([`digest`])
//! - **`model`** — Domain types for sequences and collections ([`model`])
//! - **`store`** — Storage backends: in-memory, FASTA, memory-mapped ([`store`])
//! - **`server`** — Axum router for serving the refget API ([`server`])
//! - **`client`** — HTTP client for calling refget servers ([`client`])
//!
//! By default, `digest` and `model` are enabled.

#[cfg(feature = "client")]
pub use refget_client as client;

#[cfg(feature = "digest")]
pub use refget_digest as digest;

#[cfg(feature = "model")]
pub use refget_model as model;

#[cfg(feature = "server")]
pub use refget_server as server;

#[cfg(feature = "store")]
pub use refget_store as store;
