//! Bounded attachment responses. No provider paths or MIME hints are trusted.
use axum::{
    body::{Body, Bytes},
    http::{HeaderMap, Method, StatusCode, header},
    response::Response,
};

#[derive(Debug, PartialEq, Eq)]
enum ByteRange {
    Full,
    Partial(usize, usize), // half-open
    Unsatisfiable,
}

// One byte range is enough for browser media seeking and download resume.
// Unsupported/malformed range sets are ignored, as allowed by RFC 9110.
fn byte_range(value: &str, length: usize) -> ByteRange {
    let Some(value) = value.trim().strip_prefix("bytes=") else {
        return ByteRange::Full;
    };
    if value.len() > 256 || value.contains(',') {
        return ByteRange::Full;
    }
    let Some((start, end)) = value.trim().split_once('-') else {
        return ByteRange::Full;
    };
    fn number(s: &str) -> Option<u64> {
        if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        Some(s.bytes().fold(0u64, |n, b| {
            n.saturating_mul(10).saturating_add(u64::from(b - b'0'))
        }))
    }
    if start.is_empty() {
        let Some(suffix) = number(end) else {
            return ByteRange::Full;
        };
        if suffix == 0 || length == 0 {
            return ByteRange::Unsatisfiable;
        }
        return ByteRange::Partial(
            length.saturating_sub(suffix.min(length as u64) as usize),
            length,
        );
    }
    let Some(start) = number(start) else {
        return ByteRange::Full;
    };
    let end = if end.is_empty() {
        u64::MAX
    } else {
        let Some(end) = number(end) else {
            return ByteRange::Full;
        };
        end
    };
    if end < start {
        return ByteRange::Full;
    }
    if start >= length as u64 {
        return ByteRange::Unsatisfiable;
    }
    ByteRange::Partial(
        start as usize,
        end.saturating_add(1).min(length as u64) as usize,
    )
}

fn content_disposition(filename: &str, inline: bool) -> String {
    let filename = filename.rsplit(['/', '\\']).next().unwrap_or("attachment");
    let fallback: String = filename
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ' ') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let fallback = if fallback.trim().is_empty() {
        "attachment"
    } else {
        fallback.as_str()
    };
    let mut encoded = String::new();
    for b in filename.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_' | b'~') {
            encoded.push(char::from(b));
        } else {
            use std::fmt::Write as _;
            let _ = write!(encoded, "%{b:02X}");
        }
    }
    format!(
        "{}; filename=\"{fallback}\"; filename*=UTF-8''{encoded}",
        if inline { "inline" } else { "attachment" }
    )
}

fn preview_mime(bytes: &[u8]) -> Option<&'static str> {
    let mime = infer::get(bytes)?.mime_type();
    // Never execute HTML, XML or SVG from a same-origin attachment URL.
    (matches!(
        mime,
        "image/png"
            | "image/jpeg"
            | "image/gif"
            | "image/webp"
            | "image/avif"
            | "image/bmp"
            | "image/tiff"
            | "image/x-icon"
            | "application/pdf"
    ) || mime.starts_with("audio/")
        || mime.starts_with("video/"))
    .then_some(mime)
}

pub(super) fn attachment_response(
    filename: &str,
    bytes: Vec<u8>,
    preview: bool,
    request: &HeaderMap,
    method: &Method,
) -> Response {
    let mime = preview.then(|| preview_mime(&bytes)).flatten();
    let length = bytes.len();
    let range = if method == Method::GET && !request.contains_key(header::IF_RANGE) {
        request
            .get(header::RANGE)
            .and_then(|h| h.to_str().ok())
            .map(|h| byte_range(h, length))
            .unwrap_or(ByteRange::Full)
    } else {
        ByteRange::Full
    };
    let mut builder = Response::builder()
        .header(
            header::CONTENT_TYPE,
            mime.unwrap_or("application/octet-stream"),
        )
        .header(
            header::CONTENT_DISPOSITION,
            content_disposition(filename, mime.is_some()),
        )
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::CACHE_CONTROL, "private, no-store")
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
        .header("content-security-policy", "sandbox; default-src 'none'");
    let bytes = Bytes::from(bytes);
    let body = match range {
        ByteRange::Full => {
            builder = builder.header(header::CONTENT_LENGTH, length);
            if method == Method::HEAD {
                Body::empty()
            } else {
                Body::from(bytes)
            }
        }
        ByteRange::Partial(start, end) => {
            builder = builder
                .status(StatusCode::PARTIAL_CONTENT)
                .header(header::CONTENT_LENGTH, end - start)
                .header(
                    header::CONTENT_RANGE,
                    format!("bytes {start}-{}/{length}", end - 1),
                );
            Body::from(bytes.slice(start..end))
        }
        ByteRange::Unsatisfiable => {
            builder = builder
                .status(StatusCode::RANGE_NOT_SATISFIABLE)
                .header(header::CONTENT_LENGTH, 0)
                .header(header::CONTENT_RANGE, format!("bytes */{length}"));
            Body::empty()
        }
    };
    builder
        .body(body)
        .expect("attachment headers contain only encoded/constant ASCII")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn single_ranges_are_bounded_and_overflow_safe() {
        for (value, expected) in [
            ("bytes=0-4", ByteRange::Partial(0, 5)),
            ("bytes=5-", ByteRange::Partial(5, 10)),
            ("bytes=-3", ByteRange::Partial(7, 10)),
            (
                "bytes=-999999999999999999999999999",
                ByteRange::Partial(0, 10),
            ),
            (
                "bytes=0-999999999999999999999999999",
                ByteRange::Partial(0, 10),
            ),
            ("bytes=10-", ByteRange::Unsatisfiable),
            ("bytes=-0", ByteRange::Unsatisfiable),
            (
                "bytes=999999999999999999999999999-",
                ByteRange::Unsatisfiable,
            ),
            ("bytes=5-3", ByteRange::Full),
            ("bytes=0-2,4-6", ByteRange::Full),
            ("bytes=x-5", ByteRange::Full),
            ("other=0-5", ByteRange::Full),
        ] {
            assert_eq!(byte_range(value, 10), expected, "{value}");
        }
        assert_eq!(byte_range("bytes=0-", 0), ByteRange::Unsatisfiable);
    }
    #[test]
    fn utf8_filename_is_preserved_without_header_injection() {
        let value = content_disposition("向日葵\r\n\".png", true);
        assert!(value.starts_with("inline;"));
        assert!(value.contains("filename*=UTF-8''%E5%90%91"));
        assert!(!value.contains('\r') && !value.contains('\n'));
        assert!(!content_disposition("../foo.png", false).contains("../"));
    }
    #[tokio::test]
    async fn headers_partial_download_and_head_are_consistent() {
        let mut headers = HeaderMap::new();
        headers.insert(header::RANGE, "bytes=2-4".parse().unwrap());
        let response = attachment_response(
            "文件.txt",
            b"0123456789".to_vec(),
            true,
            &headers,
            &Method::GET,
        );
        assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(response.headers()[header::CONTENT_RANGE], "bytes 2-4/10");
        assert_eq!(response.headers()[header::CONTENT_LENGTH], "3");
        assert_eq!(
            response.headers()[header::CONTENT_TYPE],
            "application/octet-stream"
        );
        assert_eq!(
            axum::body::to_bytes(response.into_body(), 100)
                .await
                .unwrap(),
            "234"
        );
        let response = attachment_response("foo", vec![0; 10], true, &headers, &Method::HEAD);
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CONTENT_LENGTH], "10");
        assert!(
            axum::body::to_bytes(response.into_body(), 100)
                .await
                .unwrap()
                .is_empty()
        );
    }
    #[test]
    fn only_recognized_passive_media_is_inline() {
        let png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR";
        assert_eq!(preview_mime(png), Some("image/png"));
        assert_eq!(
            preview_mime(b"<svg xmlns='http://www.w3.org/2000/svg'><script/></svg>"),
            None
        );
        let response = attachment_response(
            "evil.png",
            b"<html>script</html>".to_vec(),
            true,
            &HeaderMap::new(),
            &Method::GET,
        );
        assert!(
            response.headers()[header::CONTENT_DISPOSITION]
                .to_str()
                .unwrap()
                .starts_with("attachment;")
        );
    }
}
