use alloy_primitives::{Address, B256};
use alloy_signer_local::PrivateKeySigner;
use tracing::{info, warn};

use crate::{
    api::{Attested, EnclaveCall, ErrorCode, Reply, Signer},
    nsm::{Nsm, NsmError},
};

/// An enclave that derives a signing key from hardware entropy and attests to it.
#[derive(Debug)]
pub struct Enclave<N: Nsm> {
    nsm: N,
    signer: PrivateKeySigner,
}

impl<N: Nsm> Enclave<N> {
    /// Derives a fresh signer key from hardware entropy.
    ///
    /// The key exists only in memory for the life of the process. A restart
    /// therefore yields a new address that must be re-registered; that cost is
    /// what buys the guarantee that no copy of the key exists anywhere.
    pub fn new(nsm: N) -> Result<Self, crate::nsm::NsmError> {
        let mut secret = [0u8; 32];
        nsm.fill_random(&mut secret)?;
        let signer = PrivateKeySigner::from_slice(&secret)
            .map_err(|error| NsmError::Entropy(error.to_string()))?;
        secret.fill(0);

        info!(signer = %signer.address(),  "enclave signer derived");
        Ok(Self { nsm, signer })
    }

    /// The signer address, as the registry derives it from the public key.
    pub const fn signer(&self) -> Address {
        self.signer.address()
    }

    /// Produces an attestation document over the signer public key.
    ///
    /// `user_data` commits the settlement config, so registration proves not
    /// just *that* the key lives in a trusted image but *which* chain and portal
    /// that image will sign for. Without it, an enclave built for a testnet
    /// portal could be registered against a mainnet registry.
    pub fn attestation(
        &self,
        user_data: Option<Vec<u8>>,
        nonce: Option<B256>,
    ) -> Result<Attested, NsmError> {
        let public_key = self.public_key();
        let document = self.nsm.attest(public_key.clone(), user_data, nonce)?;

        Ok(Attested {
            signer: self.signer.address(),
            public_key,
            document,
        })
    }

    /// Serves one call. Never panics on bad input; failures become [`Reply::Failed`].
    pub fn dispatch(&self, call: EnclaveCall) -> Reply {
        match call {
            EnclaveCall::SignerPublicKey => Reply::Signer(Signer {
                public_key: self.public_key(),
            }),
            EnclaveCall::Attest { nonce } => match self.attestation(None, nonce) {
                Ok(attested) => Reply::Attested(attested),
                Err(error) => {
                    warn!(%error, "attestation failed");
                    Reply::Failed {
                        code: ErrorCode::Nsm,
                        message: error.to_string(),
                    }
                }
            },
            EnclaveCall::Verify(_call) => {
                warn!("Verify not support");
                Reply::Failed {
                    code: ErrorCode::Nsm,
                    message: "Verify not support".to_string(),
                }
            }
        }
    }

    /// 65-byte uncompressed EC point.
    pub fn public_key(&self) -> Vec<u8> {
        self.signer
            .credential()
            .verifying_key()
            .to_encoded_point(false)
            .as_bytes()
            .to_vec()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {

    use alloy_primitives::keccak256;

    use super::*;
    use crate::nsm::DevNsm;

    /// The reported key must derive `enclave.signer()` through the registry's
    /// formula `keccak256(public_key[1..])[12..]`, or the chain would register
    /// a different address than the one that actually signs.
    #[test]
    fn signer_public_key_derives_the_signer_address() {
        let enclave = Enclave::new(DevNsm).unwrap();
        let Reply::Signer(signer) = enclave.dispatch(EnclaveCall::SignerPublicKey) else {
            panic!("expected Reply::Signer");
        };

        assert_eq!(signer.public_key.len(), 65);
        assert_eq!(signer.public_key[0], 0x04);
        assert_eq!(
            &keccak256(&signer.public_key[1..])[12..],
            enclave.signer().as_slice()
        );
    }
}
