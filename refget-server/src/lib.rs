//! Axum router library for GA4GH refget Sequences v2.0.0 and Sequence Collections v1.0.0.

mod handlers;
mod state;

pub use state::{RefgetConfig, RefgetState, ServiceInfoConfig};

use axum::Router;

/// Create the combined refget router with both sequences and sequence collections endpoints.
pub fn refget_router(state: RefgetState) -> Router {
    Router::new().merge(sequences_router(state.clone())).merge(seqcol_router(state))
}

/// Create the sequences-only router (Sequences v2.0.0 endpoints).
pub fn sequences_router(state: RefgetState) -> Router {
    handlers::sequences::router(state)
}

/// Create the sequence collections-only router (Sequence Collections v1.0.0 endpoints).
pub fn seqcol_router(state: RefgetState) -> Router {
    handlers::seqcol::router(state)
}
