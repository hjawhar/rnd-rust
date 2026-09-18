//! Cache module — Redis-only modules for cross-service state.
//!
//! All in-memory DashMap caches live in `state.rs` (AppState).
//! Only Redis-backed modules remain here.

pub mod price_cache;
pub mod volume_cache;
