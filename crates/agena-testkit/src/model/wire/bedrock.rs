use anyhow::{Context, Result, ensure};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use bytes::Bytes;
use serde_json::{Value, json};

/// AWS EventStream uses CRC32 (IEEE), a big-endian prelude, typed headers, and
/// a checksum over the entire message except its final checksum field.
/// https://smithy.io/2.0/aws/amazon-eventstream.html
pub(in crate::model) fn bedrock_frame(name: &str, data: &Value, exception: bool) -> Result<Bytes> {
    ensure!(!name.is_empty(), "AWS event type must not be empty");
    let mut headers = Vec::new();
    string_header(
        &mut headers,
        ":message-type",
        if exception { "exception" } else { "event" },
    )?;
    string_header(
        &mut headers,
        if exception {
            ":exception-type"
        } else {
            ":event-type"
        },
        name,
    )?;
    string_header(&mut headers, ":content-type", "application/json")?;
    let payload = serde_json::to_vec(data)?;
    frame(&headers, &payload)
}

pub(in crate::model) fn bedrock_error_frame(code: &str, message: &str) -> Result<Bytes> {
    ensure!(!code.is_empty(), "AWS error code must not be empty");
    let mut headers = Vec::new();
    string_header(&mut headers, ":message-type", "error")?;
    string_header(&mut headers, ":error-code", code)?;
    string_header(&mut headers, ":error-message", message)?;
    frame(&headers, &[])
}

fn frame(headers: &[u8], payload: &[u8]) -> Result<Bytes> {
    let total = headers
        .len()
        .checked_add(payload.len())
        .and_then(|n| n.checked_add(16))
        .context("AWS frame length overflow")?;
    let total = u32::try_from(total).context("AWS frame exceeds u32 length")?;
    let mut frame = Vec::with_capacity(total as usize);
    frame.extend_from_slice(&total.to_be_bytes());
    frame.extend_from_slice(&u32::try_from(headers.len())?.to_be_bytes());
    frame.extend_from_slice(&crc32(&frame).to_be_bytes());
    frame.extend_from_slice(headers);
    frame.extend_from_slice(payload);
    frame.extend_from_slice(&crc32(&frame).to_be_bytes());
    Ok(Bytes::from(frame))
}

pub(in crate::model) fn anthropic_frame(data: &Value) -> Result<Bytes> {
    bedrock_frame(
        "chunk",
        &json!({"bytes": STANDARD.encode(serde_json::to_vec(data)?)}),
        false,
    )
}

fn string_header(headers: &mut Vec<u8>, name: &str, value: &str) -> Result<()> {
    headers.push(u8::try_from(name.len()).context("AWS header name too long")?);
    headers.extend_from_slice(name.as_bytes());
    headers.push(7); // String header value.
    headers.extend_from_slice(
        &u16::try_from(value.len())
            .context("AWS header value too long")?
            .to_be_bytes(),
    );
    headers.extend_from_slice(value.as_bytes());
    Ok(())
}

pub(super) fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0_u32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & 0_u32.wrapping_sub(crc & 1));
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ieee_crc_uses_the_standard_check_vector() {
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
        assert_eq!(crc32(b""), 0);
    }

    #[test]
    fn frame_matches_an_independent_python_zlib_vector() {
        // This fixed vector was computed using struct.pack and zlib.crc32, not
        // the fixture encoder. It catches endian, framing, and checksum drift.
        let expected = "0000007a00000051ca2c46650d3a6d6573736167652d747970650700056576656e740b3a6576656e742d7479706507000b6d65737361676553746f700d3a636f6e74656e742d747970650700106170706c69636174696f6e2f6a736f6e7b2273746f70526561736f6e223a22656e645f7475726e227d3dc33bfa";
        let frame = bedrock_frame("messageStop", &json!({"stopReason":"end_turn"}), false).unwrap();
        let actual = frame
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        assert_eq!(actual, expected);
    }
}
