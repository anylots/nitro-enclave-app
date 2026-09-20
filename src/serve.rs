#![cfg(target_os = "linux")]

use std::{sync::Arc, time::Duration};

use tokio::time::timeout;
use tokio_vsock::VsockStream;
use tracing::{debug, error, info, warn};

use crate::{
    api::{EnclaveCall, ErrorCode, Reply},
    enclave::Enclave,
    nsm::NitroNsm,
    wire,
};

/// Fixed vsock port the enclave listens on.
pub const VSOCK_PORT: u32 = 8000;

/// Allows large batch witnesses containing trie proofs and bytecode pools while bounding the
/// allocation an untrusted frame length can request.
pub const DEFAULT_MAX_REQUEST_BYTES: usize = 512 * 1024 * 1024;

/// Default read timeout for a single request frame. Must cover a gigabyte-scale `Settle` payload.
pub const DEFAULT_READ_TIMEOUT: Duration = Duration::from_secs(300);

/// Default write timeout for a single reply frame. Replies are KB-scale (an
/// attestation document or a signature), so this stays much tighter than
/// [`DEFAULT_READ_TIMEOUT`]: a host that stops reading must not hold the
/// serial accept loop hostage.
pub const DEFAULT_WRITE_TIMEOUT: Duration = Duration::from_secs(30);

/// Listener configuration.
#[derive(Debug, Clone)]
pub struct ServeConfig {
    /// vsock port.
    pub port: u32,
    /// Cap on a single frame.
    ///
    /// Budget enclave memory at roughly 2.5× this value: the encoded payload and
    /// the decoded [`crate::SettleCall`] are live at the same time, and enclave
    /// memory is reserved at launch and cannot be swapped.
    pub max_frame_len: usize,
    /// Deadline for reading one complete request frame. Must cover a
    /// gigabyte-scale `Settle` payload.
    pub read_timeout: Duration,
    /// Deadline for writing one complete reply frame.
    pub write_timeout: Duration,
}

impl Default for ServeConfig {
    fn default() -> Self {
        Self {
            port: VSOCK_PORT,
            max_frame_len: DEFAULT_MAX_REQUEST_BYTES,
            read_timeout: DEFAULT_READ_TIMEOUT,
            write_timeout: DEFAULT_WRITE_TIMEOUT,
        }
    }
}

/// A server that listens for vsock connections and dispatches them to the enclave.
#[derive(Debug)]
pub struct EnclaveServer {
    enclave: Arc<Enclave<NitroNsm>>,
    config: ServeConfig,
}

impl EnclaveServer {
    /// Creates a new enclave server with the given configuration.
    pub fn new(config: ServeConfig) -> eyre::Result<Self> {
        let nsm = NitroNsm::open()?;
        let enclave = Arc::new(Enclave::new(nsm)?);
        Ok(Self { enclave, config })
    }

    /// Starts the server and listens for incoming connections.
    pub async fn serve(&self) -> eyre::Result<()> {
        use tokio_vsock::{VMADDR_CID_ANY, VsockAddr, VsockListener};

        let listener = VsockListener::bind(VsockAddr::new(VMADDR_CID_ANY, VSOCK_PORT))?;
        info!(cid = VMADDR_CID_ANY, port = VSOCK_PORT, "enclave listening");

        loop {
            let (stream, peer) = match listener.accept().await {
                Ok((stream, peer)) => (stream, peer),
                Err(error) => {
                    error!(%error, "failed to accept vsock connection");
                    continue;
                }
            };
            debug!(cid = peer.cid(), port = peer.port(), "accepted connection");

            if let Err(e) = self.handle_connection(stream).await {
                warn!(
                    error = %e,
                    cid = peer.cid(),
                    port = peer.port(),
                    "connection failed"
                );
            }
        }
    }

    // Handles an incoming vsock connection.
    async fn handle_connection(&self, mut stream: VsockStream) -> eyre::Result<()> {
        let call: EnclaveCall =
            timeout(self.config.read_timeout, wire::read(&mut stream, self.config.max_frame_len))
                .await??
                .ok_or_else(|| eyre::eyre!("No call received"))?;

        let worker = Arc::clone(&self.enclave);
        let reply = match tokio::task::spawn_blocking(move || worker.dispatch(call)).await {
            Ok(reply) => reply,
            Err(error) => {
                error!(?error, "dispatch task failed");
                Reply::Failed { code: ErrorCode::Execution, message: "call failed".to_owned() }
            }
        };
        timeout(
            self.config.write_timeout,
            wire::write(&mut stream, &reply, self.config.max_frame_len),
        )
        .await??;
        Ok(())
    }
}
