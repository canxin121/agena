use std::{
    borrow::Cow,
    io::{self, Read, Write},
};

use flate2::{Compression, bufread::ZlibDecoder, write::ZlibEncoder};

const MIN_COMPRESSED_BYTES: usize = 512;
const MAX_COMPRESSED_BYTES: usize = 32 * 1024 * 1024;
const RAW: u8 = 0;
const ZLIB: u8 = 1;

pub(crate) fn pack(raw: &[u8]) -> io::Result<Vec<u8>> {
    pack_with_prefix(&[], raw)
}

pub(crate) fn pack_with_prefix(prefix: &[u8], raw: &[u8]) -> io::Result<Vec<u8>> {
    if (MIN_COMPRESSED_BYTES..=MAX_COMPRESSED_BYTES).contains(&raw.len()) {
        let mut header = Vec::with_capacity(prefix.len() + 5);
        header.extend_from_slice(prefix);
        header.push(ZLIB);
        header.extend_from_slice(&(raw.len() as u32).to_le_bytes());
        let mut encoder = ZlibEncoder::new(header, Compression::fast());
        encoder.write_all(raw)?;
        let compressed = encoder.finish()?;
        if compressed.len() - prefix.len() <= raw.len() {
            return Ok(compressed);
        }
    }
    let mut payload = Vec::with_capacity(prefix.len() + raw.len() + 1);
    payload.extend_from_slice(prefix);
    payload.push(RAW);
    payload.extend_from_slice(raw);
    Ok(payload)
}

pub(crate) fn unpack(payload: &[u8]) -> io::Result<Cow<'_, [u8]>> {
    let invalid = || io::Error::new(io::ErrorKind::InvalidData, "invalid receipt payload");
    match payload.split_first() {
        Some((&RAW, raw)) => Ok(Cow::Borrowed(raw)),
        Some((&ZLIB, data)) => {
            let (size, compressed) = data.split_first_chunk().ok_or_else(invalid)?;
            let size = u32::from_le_bytes(*size) as usize;
            if !(MIN_COMPRESSED_BYTES..=MAX_COMPRESSED_BYTES).contains(&size) {
                return Err(invalid());
            }
            let mut decoded = Vec::new();
            let mut decoder = ZlibDecoder::new(compressed).take(size as u64 + 1);
            decoder.read_to_end(&mut decoded)?;
            if decoded.len() != size || !decoder.get_ref().get_ref().is_empty() {
                return Err(invalid());
            }
            Ok(Cow::Owned(decoded))
        }
        _ => Err(invalid()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn receipt_payload_round_trips_exact_bytes_and_rejects_bad_lengths() {
        let short = br#"{"value": "small"}"#;
        let raw = pack(short).unwrap();
        assert_eq!(unpack(&raw).unwrap(), Cow::Borrowed(short.as_slice()));

        let long = format!(r#"{{"value": "{}"}}"#, "对话".repeat(20_000));
        let mut compressed = pack(long.as_bytes()).unwrap();
        assert_eq!(compressed[0], ZLIB);
        assert!(compressed.len() < long.len() / 2);
        assert_eq!(unpack(&compressed).unwrap(), long.as_bytes());
        let digest = [0x42; 32];
        let receipt = pack_with_prefix(&digest, long.as_bytes()).unwrap();
        assert_eq!(&receipt[..digest.len()], &digest);
        assert_eq!(&receipt[digest.len()..], &compressed);

        let size = MIN_COMPRESSED_BYTES as u32;
        compressed[1..5].copy_from_slice(&size.to_le_bytes());
        assert!(unpack(&compressed).is_err());
    }
}
