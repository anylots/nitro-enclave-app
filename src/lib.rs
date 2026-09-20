#![deny(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

//! In-enclave signer for Sotto settlement batches.
//!
//! Replays a batch and signs the resulting transition with a key derived
//! inside the enclave that never leaves it. See `spec/enclave.md`.

/// API types for enclave requests and responses.
pub mod api;
/// Enclave signing and batch verification logic.
pub mod enclave;
/// Nitro Secure Module access and development stubs.
pub mod nsm;
/// Vsock server for handling enclave requests.
pub mod serve;
/// Length-prefixed wire protocol for enclave requests and responses.
pub mod wire;

pub use serve::{EnclaveServer, ServeConfig};
