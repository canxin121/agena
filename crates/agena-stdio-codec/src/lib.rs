//! Bounded LSP-style `Content-Length` framing for stdio protocols.
//!
//! Header parsing is delegated to `httparse`; buffering and backpressure are
//! delegated to `tokio_util::codec`. Consumers serialize their own payloads so
//! this crate stays independent of any particular JSON-RPC model.

use std::io;

use bytes::{Buf as _, BufMut as _, Bytes, BytesMut};
use tokio_util::codec::{Decoder, Encoder};

const MAX_HEADER_BYTES: usize = 8 * 1024;
const MAX_HEADERS: usize = 32;

#[derive(Debug, thiserror::Error)]
pub enum ContentLengthCodecError {
    #[error("stdio framing io error: {0}")]
    Io(#[from] io::Error),
    #[error("stdio frame header is malformed: {0}")]
    MalformedHeader(#[from] httparse::Error),
    #[error("stdio frame header exceeds {MAX_HEADER_BYTES} bytes")]
    HeaderTooLarge,
    #[error("stdio frame is missing Content-Length")]
    MissingContentLength,
    #[error("stdio frame contains more than one Content-Length header")]
    DuplicateContentLength,
    #[error("stdio frame has an invalid Content-Length value")]
    InvalidContentLength,
    #[error("stdio frame body is {actual} bytes; limit is {maximum} bytes")]
    FrameTooLarge { actual: usize, maximum: usize },
}

/// A bounded decoder and encoder for LSP-style Content-Length frames.
#[derive(Debug, Clone)]
pub struct ContentLengthCodec {
    max_frame_bytes: usize,
}

impl ContentLengthCodec {
    pub fn new(max_frame_bytes: usize) -> Self {
        assert!(max_frame_bytes > 0, "frame limit must be non-zero");
        Self { max_frame_bytes }
    }

    pub fn max_frame_bytes(&self) -> usize {
        self.max_frame_bytes
    }

    fn ensure_frame_size(&self, actual: usize) -> Result<(), ContentLengthCodecError> {
        if actual > self.max_frame_bytes {
            return Err(ContentLengthCodecError::FrameTooLarge {
                actual,
                maximum: self.max_frame_bytes,
            });
        }
        Ok(())
    }
}

impl Decoder for ContentLengthCodec {
    type Item = Bytes;
    type Error = ContentLengthCodecError;

    fn decode(&mut self, source: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        let mut parsed_headers = [httparse::EMPTY_HEADER; MAX_HEADERS];
        let (header_bytes, headers) = match httparse::parse_headers(source, &mut parsed_headers)? {
            httparse::Status::Complete(parsed) => parsed,
            httparse::Status::Partial => {
                if source.len() > MAX_HEADER_BYTES {
                    return Err(ContentLengthCodecError::HeaderTooLarge);
                }
                return Ok(None);
            }
        };
        if header_bytes > MAX_HEADER_BYTES {
            return Err(ContentLengthCodecError::HeaderTooLarge);
        }

        let mut content_length = None;
        for header in headers {
            if !header.name.eq_ignore_ascii_case("Content-Length") {
                continue;
            }
            if content_length.is_some() {
                return Err(ContentLengthCodecError::DuplicateContentLength);
            }
            let value = std::str::from_utf8(header.value)
                .map_err(|_| ContentLengthCodecError::InvalidContentLength)?;
            content_length = Some(
                value
                    .trim()
                    .parse::<usize>()
                    .map_err(|_| ContentLengthCodecError::InvalidContentLength)?,
            );
        }

        let content_length = content_length.ok_or(ContentLengthCodecError::MissingContentLength)?;
        self.ensure_frame_size(content_length)?;
        let frame_bytes = header_bytes
            .checked_add(content_length)
            .ok_or(ContentLengthCodecError::InvalidContentLength)?;
        if source.len() < frame_bytes {
            source.reserve(frame_bytes - source.len());
            return Ok(None);
        }

        source.advance(header_bytes);
        Ok(Some(source.split_to(content_length).freeze()))
    }
}

impl Encoder<Bytes> for ContentLengthCodec {
    type Error = ContentLengthCodecError;

    fn encode(&mut self, body: Bytes, destination: &mut BytesMut) -> Result<(), Self::Error> {
        self.ensure_frame_size(body.len())?;
        let header = format!("Content-Length: {}\r\n\r\n", body.len());
        destination.reserve(header.len() + body.len());
        destination.put_slice(header.as_bytes());
        destination.put_slice(&body);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{ContentLengthCodec, ContentLengthCodecError};
    use bytes::{Bytes, BytesMut};
    use tokio_util::codec::{Decoder as _, Encoder as _};

    #[test]
    fn partial_and_back_to_back_frames_decode() {
        let mut codec = ContentLengthCodec::new(128);
        let mut encoded = BytesMut::new();
        codec
            .encode(Bytes::from_static(br#"{"id":1}"#), &mut encoded)
            .expect("encode first frame");
        codec
            .encode(Bytes::from_static(br#"{"id":2}"#), &mut encoded)
            .expect("encode second frame");

        let split = 10;
        let tail = encoded.split_off(split);
        assert!(
            codec
                .decode(&mut encoded)
                .expect("partial decode")
                .is_none()
        );
        encoded.extend_from_slice(&tail);
        assert_eq!(
            codec.decode(&mut encoded).expect("first decode").as_deref(),
            Some(br#"{"id":1}"#.as_slice())
        );
        assert_eq!(
            codec
                .decode(&mut encoded)
                .expect("second decode")
                .as_deref(),
            Some(br#"{"id":2}"#.as_slice())
        );
    }

    #[test]
    fn malformed_and_oversized_frames_are_rejected_before_body_allocation() {
        let mut codec = ContentLengthCodec::new(8);
        let mut duplicate =
            BytesMut::from("Content-Length: 1\r\nContent-Length: 1\r\n\r\nx".as_bytes());
        assert!(matches!(
            codec.decode(&mut duplicate),
            Err(ContentLengthCodecError::DuplicateContentLength)
        ));

        let mut oversized = BytesMut::from("Content-Length: 999999\r\n\r\n".as_bytes());
        assert!(matches!(
            codec.decode(&mut oversized),
            Err(ContentLengthCodecError::FrameTooLarge { .. })
        ));
    }
}
