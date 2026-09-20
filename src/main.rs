#[cfg(target_os = "linux")]
use nitro_enclave_app::{EnclaveServer, ServeConfig};

#[cfg(target_os = "linux")]
#[tokio::main]
pub async fn main() -> eyre::Result<()> {
    // Initialize the enclave server.
    let enclave_server = EnclaveServer::new(ServeConfig::default())?;
    // Start the server and wait for it to finish.
    enclave_server.serve().await?;
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn main() {
    panic!("sotto-settlement-tee-enclave only supports Linux (AWS Nitro Enclaves)");
}
