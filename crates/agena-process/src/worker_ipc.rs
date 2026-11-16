//! Bounded binary framing. Credentials and payload bytes travel over anonymous
//! pipes, never command-line arguments or temporary request files.
use serde::{Serialize, de::DeserializeOwned};
use std::io;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub(crate) const CHUNK: usize = 64 * 1024;
const MAX_HEADER: usize = 512 * 1024;
const ERROR: u32 = u32::MAX;

pub(crate) async fn write_json<W: AsyncWrite + Unpin, T: Serialize>(
    writer: &mut W,
    value: &T,
) -> io::Result<()> {
    let bytes = serde_json::to_vec(value).map_err(io::Error::other)?;
    if bytes.len() > MAX_HEADER {
        return Err(io::Error::new(
            io::ErrorKind::FileTooLarge,
            "worker header exceeds 512 KiB",
        ));
    }
    writer.write_u32(bytes.len() as u32).await?;
    writer.write_all(&bytes).await?;
    writer.flush().await
}

pub(crate) async fn read_json<R: AsyncRead + Unpin, T: DeserializeOwned>(
    reader: &mut R,
) -> io::Result<T> {
    let length = reader.read_u32().await? as usize;
    if length > MAX_HEADER {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "worker header exceeds 512 KiB",
        ));
    }
    let mut bytes = vec![0; length];
    reader.read_exact(&mut bytes).await?;
    serde_json::from_slice(&bytes)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

pub(crate) async fn write_chunk<W: AsyncWrite + Unpin>(
    writer: &mut W,
    bytes: &[u8],
) -> io::Result<()> {
    for chunk in bytes.chunks(CHUNK) {
        writer.write_u32(chunk.len() as u32).await?;
        writer.write_all(chunk).await?;
    }
    writer.flush().await
}

pub(crate) async fn finish<W: AsyncWrite + Unpin>(writer: &mut W) -> io::Result<()> {
    writer.write_u32(0).await?;
    writer.flush().await
}

pub(crate) async fn fail<W: AsyncWrite + Unpin>(
    writer: &mut W,
    error: &io::Error,
) -> io::Result<()> {
    writer.write_u32(ERROR).await?;
    write_json(writer, &error.to_string()).await
}

pub(crate) async fn read_chunk<R: AsyncRead + Unpin>(
    reader: &mut R,
) -> io::Result<Option<bytes::Bytes>> {
    let length = reader.read_u32().await?;
    if length == ERROR {
        let message: String = read_json(reader).await?;
        return Err(io::Error::other(message));
    }
    if length == 0 {
        return Ok(None);
    }
    if length as usize > CHUNK {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid worker body frame",
        ));
    }
    let mut bytes = vec![0; length as usize];
    reader.read_exact(&mut bytes).await?;
    Ok(Some(bytes.into()))
}
