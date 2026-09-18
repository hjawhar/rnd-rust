//! Test harness for the IRC suite.
//!
//! Phase 0 ships the trait scaffolding for the four boundary abstractions
//! the rest of the suite injects: time, persistence, outbound email, DNSBL.
//! Production wires these to real implementations; tests use the in-memory
//! defaults exposed here.
//!
//! Real method surfaces are added per phase as the corresponding feature
//! lands (see `PLAN.md` §12).

#![deny(missing_docs)]

pub mod clock;
pub mod dnsbl;
pub mod smtp;
pub mod store;
