//! Wire types for the host ↔ enclave protocol, and the digest that is signed.

use alloy_primitives::{Address, B256};
use serde::{Deserialize, Serialize};

/// Protocol version. Bump on any change to the wire types or to the digest.
///
/// The enclave image is measured by PCR0, so the host and the enclave can never
/// be upgraded atomically. An explicit version turns that skew into a clear
/// error instead of a misread frame.
pub(crate) const PROTOCOL_VERSION: u16 = 1;

/// A request from the host.
///
/// There is deliberately no variant that signs caller-supplied bytes; see the
/// crate-level invariants.
#[derive(Debug, Serialize, Deserialize)]
pub enum EnclaveCall {
    /// Reports the current signer public key. No parameters.
    ///
    /// Identity probe for the cold path: the host learns which key this boot
    /// derived — and can compare it against the registered signer — before
    /// paying for an attestation round. Infallible: the key exists from
    /// `Enclave::new`.
    SignerPublicKey,
    /// Cold path: NSM attestation over the signer public key.
    ///
    /// `nonce` is supplied by the registrar. The attestation document carries a
    /// timestamp, but a timestamp alone lets a host replay a cached document
    /// from an image that has since been revoked; only a caller-chosen nonce
    /// closes that window.
    Attest {
        /// Registrar-chosen challenge.
        nonce: Option<B256>,
    },
    /// Verify a batch and sign the resulting transition.
    ///
    /// Boxed because the payload reaches gigabytes and would otherwise set the
    /// stack size of the whole enum.
    Verify(Box<VerifyCall>),
}

/// Payload of [`EnclaveCall::Verify`].
#[derive(Debug, Serialize, Deserialize)]
pub struct VerifyCall {
    /// Block and pre-state witness data for the batch.
    pub input: Vec<u8>,
}

/// A response from the enclave.
#[derive(Debug, Serialize, Deserialize)]
pub enum Reply {
    /// Answer to [`EnclaveCall::SignerPublicKey`].
    Signer(Signer),
    /// Answer to [`EnclaveCall::Attest`].
    Attested(Attested),
    /// Answer to [`EnclaveCall::Verify`].
    Verified(Verified),
    /// The call could not be served. Details are logged inside the enclave; the
    /// message returned to the host is deliberately coarse.
    Failed {
        /// Stable, machine-readable category.
        code: ErrorCode,
        /// Human-readable detail.
        message: String,
    },
}

/// The current signer public key, as [`EnclaveCall::SignerPublicKey`] reports it.
#[derive(Debug, Serialize, Deserialize)]
pub struct Signer {
    /// 65-byte uncompressed EC point, as embedded in the attestation document.
    pub public_key: Vec<u8>,
}

/// Signer identity plus the attestation that binds it to the enclave image.
#[derive(Debug, Serialize, Deserialize)]
pub struct Attested {
    /// `keccak256(public_key[1..])[12..]`, as the registry derives it.
    pub signer: Address,
    /// 65-byte uncompressed EC point, as embedded in the document.
    pub public_key: Vec<u8>,
    /// Raw `COSE_Sign1` attestation document.
    pub document: Vec<u8>,
}

/// A verified batch and its signature.

#[derive(Debug, Serialize, Deserialize)]
pub struct Verified {
    /// Output of [`sotto_settlement_executor_client::verify`].
    pub statement_hash: B256,
    /// 65-byte `r ‖ s ‖ v`, `v ∈ {27, 28}`, ready for `ecrecover`.
    pub signature: Vec<u8>,
}

/// Stable error categories. Append-only: alerting depends on these values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErrorCode {
    /// Bad magic, unsupported version, oversized frame, or a decode failure.
    Protocol,
    /// Verification of a batch failed.
    VerificationFailed,
    /// NSM is unavailable or rejected the request.
    Nsm,
    /// The executor rejected the batch, or panicked while replaying it.
    Execution,
    /// Anything else. Already redacted.
    Internal,
}
