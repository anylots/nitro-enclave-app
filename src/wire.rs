//! Length-prefixed bincode framing over vsock.
//!
//! ```text
//! ┌────────┬─────────┬──────────┬────────┬──────────────┐
//! │ magic  │ version │ reserved │ length │   payload    │
//! │  4 B   │  2 B BE │   2 B    │ 8 B BE │  length B    │
//! └────────┴─────────┴──────────┴────────┴──────────────┘
//! ```

use std::io;

use serde::{Serialize, de::DeserializeOwned};
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};

use crate::api::PROTOCOL_VERSION;

const MAGIC: [u8; 4] = *b"SOTT";
const HEADER: usize = 16;

/// Cap on a single `write()` to vsock.
///
/// Linux ≥ 6.17 (`virtio_vsock` commit `6693731487a8`) allocates non-linear
/// SKBs for packets above `PAGE_ALLOC_COSTLY_ORDER` (~32 KiB on x86). Some
/// hypervisors do not reassemble those multi-descriptor TX packets correctly,
/// which corrupts data **silently** — it surfaces as an occasional decode
/// failure, not an error. Staying under the threshold sidesteps it.
///
/// See <https://github.com/cloud-hypervisor/cloud-hypervisor/issues/7672>.
pub const MAX_WRITE_CHUNK: usize = 28 * 1024;

/// Granularity in which a payload is allocated while being read.
const READ_CHUNK: usize = 4 * 1024 * 1024;

/// Framing failures.
#[derive(Debug, thiserror::Error)]
pub enum WireError {
    /// The peer is not speaking this protocol.
    #[error("bad frame magic {0:02x?}")]
    Magic([u8; 4]),
    /// Host and enclave disagree on the protocol version.
    #[error("unsupported protocol version {got}; this image speaks {PROTOCOL_VERSION}")]
    Version {
        /// Version advertised by the peer.
        got: u16,
    },
    /// The declared length exceeds the configured cap.
    #[error("frame of {len} bytes exceeds the {max} byte limit")]
    TooLarge {
        /// Length the peer declared.
        len: u64,
        /// Configured cap.
        max: usize,
    },
    /// The payload could not be allocated.
    #[error("cannot allocate {0} bytes")]
    Alloc(usize),
    /// The peer closed mid-frame.
    #[error("frame truncated")]
    Truncated,
    /// The payload is not valid bincode for the expected type.
    #[error("decode failed: {0}")]
    Decode(String),
    /// Transport failure.
    #[error("io: {0}")]
    Io(#[source] io::Error),
}

/// Validates a header and returns the declared payload length.
fn parse_header(header: &[u8; HEADER]) -> Result<u64, WireError> {
    let magic = header[0..4].try_into().map_err(|_| WireError::Truncated)?;
    if magic != MAGIC {
        return Err(WireError::Magic(magic));
    }
    let version = u16::from_be_bytes(header[4..6].try_into().map_err(|_| WireError::Truncated)?);
    if version != PROTOCOL_VERSION {
        return Err(WireError::Version { got: version });
    }
    Ok(u64::from_be_bytes(header[8..16].try_into().map_err(|_| WireError::Truncated)?))
}

/// Builds the header for a payload of `length` bytes.
fn build_header(length: u64) -> [u8; HEADER] {
    let mut header = [0u8; HEADER];
    header[0..4].copy_from_slice(&MAGIC);
    header[4..6].copy_from_slice(&PROTOCOL_VERSION.to_be_bytes());
    header[8..16].copy_from_slice(&length.to_be_bytes());
    header
}

/// Maps a mid-payload EOF to `Truncated`; anything else stays an `Io` error.
fn truncated(error: io::Error) -> WireError {
    match error.kind() {
        io::ErrorKind::UnexpectedEof => WireError::Truncated,
        _ => WireError::Io(error),
    }
}

/// Reads one frame.
///
/// `Ok(None)` means the peer closed cleanly on a frame boundary, which is a
/// normal end of connection and not an error.
pub async fn read<R, T>(reader: &mut R, max_len: usize) -> Result<Option<T>, WireError>
where
    R: AsyncRead + Unpin,
    T: DeserializeOwned,
{
    let mut header = [0u8; HEADER];
    if let Err(error) = reader.read_exact(&mut header).await {
        return match error.kind() {
            io::ErrorKind::UnexpectedEof => Ok(None),
            _ => Err(WireError::Io(error)),
        };
    }

    let declared = parse_header(&header)?;
    // Check the cap before allocating anything. The host is untrusted, so
    // `vec![0; declared]` would be a free out-of-memory abort.
    let len = usize::try_from(declared)
        .ok()
        .filter(|len| *len <= max_len)
        .ok_or(WireError::TooLarge { len: declared, max: max_len })?;

    let payload = read_payload(reader, len).await?;
    bincode::serde::decode_from_slice(&payload, bincode::config::standard())
        .map(|(value, _)| Some(value))
        .map_err(|error| WireError::Decode(error.to_string()))
}

/// Reads `len` bytes, growing the buffer one [`READ_CHUNK`] at a time so that a
/// peer which declares a huge frame and then stops sending wastes one chunk,
/// not the whole declared length.
async fn read_payload<R>(reader: &mut R, len: usize) -> Result<Vec<u8>, WireError>
where
    R: AsyncRead + Unpin,
{
    let mut payload = Vec::new();
    while payload.len() < len {
        let chunk = READ_CHUNK.min(len - payload.len());
        // `resize` aborts on allocation failure; reserve first so it becomes
        // an error the caller can report instead.
        payload.try_reserve(chunk).map_err(|_| WireError::Alloc(chunk))?;
        let start = payload.len();
        payload.resize(start + chunk, 0);
        let Some(target) = payload.get_mut(start..) else {
            return Err(WireError::Io(io::Error::other("payload buffer invariant violated")));
        };
        reader.read_exact(target).await.map_err(truncated)?;
    }
    Ok(payload)
}

/// Writes one frame, returning the payload size.
pub async fn write<W, T>(writer: &mut W, value: &T, max_len: usize) -> Result<usize, WireError>
where
    W: AsyncWrite + Unpin,
    T: Serialize + ?Sized,
{
    let payload = bincode::serde::encode_to_vec(value, bincode::config::standard())
        .map_err(|error| WireError::Decode(error.to_string()))?;
    if payload.len() > max_len {
        return Err(WireError::TooLarge { len: payload.len() as u64, max: max_len });
    }

    writer.write_all(&build_header(payload.len() as u64)).await.map_err(WireError::Io)?;
    for chunk in payload.chunks(MAX_WRITE_CHUNK) {
        writer.write_all(chunk).await.map_err(WireError::Io)?;
    }
    writer.flush().await.map_err(WireError::Io)?;
    Ok(payload.len())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn oversized_length_is_rejected_before_allocating() {
        let mut frame = Vec::new();
        frame.extend_from_slice(&MAGIC);
        frame.extend_from_slice(&PROTOCOL_VERSION.to_be_bytes());
        frame.extend_from_slice(&[0, 0]);
        frame.extend_from_slice(&u64::MAX.to_be_bytes());

        let error = read::<_, u64>(&mut frame.as_slice(), 1024).await.unwrap_err();
        assert!(matches!(error, WireError::TooLarge { .. }));
    }

    #[tokio::test]
    async fn clean_eof_is_not_an_error() {
        assert!(read::<_, u64>(&mut [].as_slice(), 1024).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn round_trip() {
        let mut buffer = Vec::new();
        write(&mut buffer, &7u64, 1024).await.unwrap();
        assert_eq!(read::<_, u64>(&mut buffer.as_slice(), 1024).await.unwrap(), Some(7));
    }
}
