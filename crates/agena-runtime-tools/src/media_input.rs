//! Shared, network-free input preparation. Reading bytes is not authorization
//! to send them. Callers own read/egress permissions and bind the result to a
//! concrete provider route before placing it in a model request.
use agena_domain::{AttachmentItem, AttachmentKind, AttachmentSource};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};

pub const MAX_MEDIA_BYTES: usize = 20 * 1024 * 1024;
pub const MAX_MEDIA_BATCH_BYTES: usize = 40 * 1024 * 1024;
pub const MAX_MEDIA_INPUTS: usize = 8;
pub const MAX_IMAGE_PIXELS: u64 = 40_000_000;
pub const MAX_TEXT_BYTES: usize = 1024 * 1024;
#[derive(Debug, Clone, Serialize)]
pub struct PreparedMedia {
    pub filename: String,
    pub mime: String,
    pub kind: AttachmentKind,
    pub size_bytes: u64,
    pub sha256: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
    #[serde(skip)]
    pub bytes: Vec<u8>,
}
impl PreparedMedia {
    pub fn attachment(&self, route: Option<&str>) -> AttachmentItem {
        let data = STANDARD.encode(&self.bytes);
        AttachmentItem {
            kind: self.kind,
            mime: self.mime.clone(),
            source: match route {
                Some(route) => AttachmentSource::ProviderData {
                    route: route.into(),
                    data,
                },
                None => AttachmentSource::Base64 { data },
            },
            filename: Some(self.filename.clone()),
            title: None,
            size_bytes: Some(self.size_bytes),
            sha256: Some(self.sha256.clone()),
            width: self.width,
            height: self.height,
            duration_ms: None,
            page_count: None,
        }
    }
}
pub fn read_local(
    root: &Path,
    path: &str,
    expected_sha256: Option<&str>,
) -> Result<PreparedMedia, String> {
    if path.trim().is_empty() {
        return Err("media input path is empty".into());
    }
    let target = if Path::new(path).is_absolute() {
        PathBuf::from(path)
    } else {
        root.join(path)
    };
    // Caller must authorize the canonical target. Refuse final symlinks and
    // special files, and read/validate the same open descriptor throughout.
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options
        .open(&target)
        .map_err(|error| format!("cannot open media input: {error}"))?;
    let before = file.metadata().map_err(|e| e.to_string())?;
    if !before.is_file() {
        return Err("media input must be a regular file, not a directory/device/pipe".into());
    }
    if before.len() == 0 || before.len() > MAX_MEDIA_BYTES as u64 {
        return Err("media input must be nonempty and at most 20 MiB".into());
    }
    let mut bytes = Vec::with_capacity(before.len() as usize);
    (&file)
        .take(MAX_MEDIA_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    let after = file.metadata().map_err(|e| e.to_string())?;
    if before.len() != after.len()
        || before.modified().ok() != after.modified().ok()
        || bytes.len() as u64 != after.len()
    {
        return Err("media input changed while reading; select it again".into());
    }
    let filename = target
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("input");
    let prepared = from_bytes(filename, bytes, None)?;
    if expected_sha256.is_some_and(|hash| !hash.eq_ignore_ascii_case(&prepared.sha256)) {
        return Err("media input revision changed; no content was sent".into());
    }
    Ok(prepared)
}
pub fn from_bytes(
    filename: &str,
    bytes: Vec<u8>,
    declared_mime: Option<&str>,
) -> Result<PreparedMedia, String> {
    if bytes.is_empty() || bytes.len() > MAX_MEDIA_BYTES {
        return Err("media input must be nonempty and at most 20 MiB".into());
    }
    let filename = Path::new(filename)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("input")
        .chars()
        .filter(|c| !c.is_control())
        .take(240)
        .collect::<String>();
    let raster = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        Some("image/webp")
    } else {
        None
    };
    let (kind, mime, width, height) = if let Some(mime) = raster {
        let size = imagesize::blob_size(&bytes)
            .map_err(|_| "image has invalid or incomplete dimensions")?;
        if size.width == 0
            || size.height == 0
            || size.width > 16384
            || size.height > 16384
            || size.width as u64 * size.height as u64 > MAX_IMAGE_PIXELS
        {
            return Err("image dimensions exceed the 40 megapixel/16384 pixel safety limit".into());
        }
        // Header dimensions are a cheap first gate, not proof that the image
        // decodes. Bound decompression too, then drop decoded pixels before
        // retaining only the original, unmodified input bytes.
        let mut reader = image::ImageReader::new(std::io::Cursor::new(&bytes))
            .with_guessed_format()
            .map_err(|_| "invalid image format")?;
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(16384);
        limits.max_image_height = Some(16384);
        limits.max_alloc = Some(MAX_IMAGE_PIXELS * 4);
        reader.limits(limits);
        let decoded = reader
            .decode()
            .map_err(|_| "image content is corrupt or exceeds decoded-memory limits")?;
        if decoded.width() as usize != size.width || decoded.height() as usize != size.height {
            return Err("image dimensions changed during decoding".into());
        }
        drop(decoded);
        (
            AttachmentKind::Image,
            mime.to_owned(),
            Some(size.width as u32),
            Some(size.height as u32),
        )
    } else if bytes.starts_with(b"%PDF-") {
        (AttachmentKind::Pdf, "application/pdf".into(), None, None)
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WAVE") {
        (AttachmentKind::Audio, "audio/wav".into(), None, None)
    } else if bytes.starts_with(b"ID3")
        || bytes.len() > 1 && bytes[0] == 0xff && bytes[1] & 0xe0 == 0xe0
    {
        (AttachmentKind::Audio, "audio/mpeg".into(), None, None)
    } else if bytes.starts_with(b"OggS") {
        (AttachmentKind::Audio, "audio/ogg".into(), None, None)
    } else if bytes.get(4..8) == Some(b"ftyp") {
        (AttachmentKind::Video, "video/mp4".into(), None, None)
    } else if bytes.starts_with(b"\x1a\x45\xdf\xa3") {
        (AttachmentKind::Video, "video/webm".into(), None, None)
    } else {
        let name = filename.to_ascii_lowercase();
        if [
            ".png", ".jpg", ".jpeg", ".gif", ".webp", ".svg", ".bmp", ".pdf", ".mp3", ".wav",
            ".mp4",
        ]
        .iter()
        .any(|ext| name.ends_with(ext))
        {
            return Err("media bytes do not match a supported image/document format; filename extensions are not proof".into());
        }
        if bytes.len() > MAX_TEXT_BYTES {
            return Err("text model inputs must be at most 1 MiB; select a smaller file".into());
        }
        let text = std::str::from_utf8(&bytes)
            .map_err(|_| "unsupported binary input; use an explicitly supported media format")?;
        if text.contains('\0') {
            return Err("binary NUL bytes are not valid text model inputs".into());
        }
        (AttachmentKind::File, "text/plain".into(), None, None)
    };
    if let Some(declared) = declared_mime
        .map(|m| m.split(';').next().unwrap_or(m).trim())
        .filter(|m| !m.is_empty() && *m != "application/octet-stream")
    {
        let normalized = if declared == "image/jpg" {
            "image/jpeg"
        } else {
            declared
        };
        if normalized != mime
            && !(kind == AttachmentKind::File
                && (normalized.starts_with("text/")
                    || matches!(
                        normalized,
                        "application/json" | "application/xml" | "application/yaml"
                    )))
        {
            return Err(format!(
                "declared MIME {declared} does not match actual content {mime}"
            ));
        }
    }
    Ok(PreparedMedia {
        filename,
        mime,
        kind,
        size_bytes: bytes.len() as u64,
        sha256: hex::encode(Sha256::digest(&bytes)),
        width,
        height,
        bytes,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGMQVDL+DwACFAFmBODefwAAAABJRU5ErkJggg==";
    #[test]
    fn snapshots_validate_content_and_keep_provider_binding() {
        let media = from_bytes(
            "clipboard.png",
            STANDARD.decode(PNG).unwrap(),
            Some("image/png"),
        )
        .unwrap();
        assert_eq!(media.width, Some(1));
        let attachment = media.attachment(Some("provider/model"));
        assert!(
            matches!(attachment.source,AttachmentSource::ProviderData{ref route,..} if route=="provider/model")
        );
        assert!(from_bytes("fake.png", b"not an image".to_vec(), None).is_err());
        assert!(
            from_bytes(
                "valid.png",
                STANDARD.decode(PNG).unwrap(),
                Some("image/jpeg")
            )
            .is_err()
        );
    }
    #[test]
    fn bounded_reads_reject_stale_content_and_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("input.txt"), "fixture").unwrap();
        let input = read_local(dir.path(), "input.txt", None).unwrap();
        assert!(read_local(dir.path(), "input.txt", Some(&input.sha256)).is_ok());
        assert!(read_local(dir.path(), "input.txt", Some("stale")).is_err());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(dir.path().join("input.txt"), dir.path().join("link"))
                .unwrap();
            assert!(read_local(dir.path(), "link", None).is_err());
        }
    }
}
