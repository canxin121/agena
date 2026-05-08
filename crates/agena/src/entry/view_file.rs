use std::fs;
use std::path::Path;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use mime_guess::MimeGuess;

use crate::message::{
    AttachmentKind, AttachmentSource, FirstPartyToolOutput, ToolAttachment, ViewFileToolInput,
};

use super::{FirstPartyExecution, ToolError, ToolExecutionView, ToolExecutor};

const MAX_FILE_BYTES: usize = 50 * 1024 * 1024;

pub(super) fn execute(
    executor: &ToolExecutor,
    input: &ViewFileToolInput,
) -> Result<FirstPartyExecution, ToolError> {
    let target = executor.resolve_target_path(&input.path);
    executor.ensure_read_permission(&target)?;

    if !target.exists() {
        return Err(ToolError::InvalidInput(format!(
            "view_file target does not exist: {}",
            input.path
        )));
    }

    if !target.is_file() {
        return Err(ToolError::InvalidInput(format!(
            "view_file target is not a file: {}",
            input.path
        )));
    }

    let bytes = fs::read(&target)?;
    if bytes.is_empty() {
        return Err(ToolError::InvalidInput(format!(
            "view_file target is empty: {}",
            input.path
        )));
    }

    if bytes.len() > MAX_FILE_BYTES {
        return Err(ToolError::InvalidInput(format!(
            "view_file supports files up to {} bytes: {}",
            MAX_FILE_BYTES, input.path
        )));
    }

    let display_path = executor.display_path(&target);
    let filename = target
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_owned)
        .unwrap_or_else(|| display_path.clone());
    let mime = detect_mime(&target, &bytes);
    let kind = AttachmentKind::detect(mime.as_str(), Some(filename.as_str()));
    let (width, height) = match kind {
        AttachmentKind::Image => detect_dimensions(&bytes),
        _ => (None, None),
    };

    let output = FirstPartyToolOutput::ViewFile {
        path: display_path.clone(),
        kind,
        mime: mime.clone(),
        size_bytes: bytes.len() as u64,
        filename: Some(filename.clone()),
        width,
        height,
        duration_ms: None,
        page_count: None,
    };

    let mut view = ToolExecutionView::simple(
        format!("View file {}", display_path),
        render_summary(
            display_path.as_str(),
            kind,
            mime.as_str(),
            bytes.len(),
            width,
            height,
        ),
    );
    view.metadata
        .insert("kind".to_string(), kind.as_str().to_string());
    view.metadata
        .insert("path".to_string(), display_path.clone());
    view.metadata
        .insert("filename".to_string(), filename.clone());
    view.metadata.insert("mime".to_string(), mime.clone());
    view.metadata
        .insert("size_bytes".to_string(), bytes.len().to_string());
    if let Some(width) = width {
        view.metadata.insert("width".to_string(), width.to_string());
    }
    if let Some(height) = height {
        view.metadata
            .insert("height".to_string(), height.to_string());
    }
    view.attachments.push(ToolAttachment {
        kind,
        mime,
        source: AttachmentSource::Base64 {
            data: STANDARD.encode(&bytes),
        },
        filename: Some(filename),
        title: None,
        size_bytes: Some(bytes.len() as u64),
        sha256: None,
        width,
        height,
        duration_ms: None,
        page_count: None,
    });

    Ok(FirstPartyExecution::new(output, view))
}

fn render_summary(
    path: &str,
    kind: AttachmentKind,
    mime: &str,
    size_bytes: usize,
    width: Option<u32>,
    height: Option<u32>,
) -> String {
    match (kind, width, height) {
        (AttachmentKind::Image, Some(width), Some(height)) => {
            format!("Attached image file {path} ({width}x{height}, {mime}, {size_bytes} bytes).")
        }
        (AttachmentKind::Pdf, _, _) => {
            format!("Attached document file {path} ({mime}, {size_bytes} bytes).")
        }
        _ => format!(
            "Attached {} file {path} ({mime}, {size_bytes} bytes).",
            kind.as_str()
        ),
    }
}

fn detect_dimensions(bytes: &[u8]) -> (Option<u32>, Option<u32>) {
    match imagesize::blob_size(bytes) {
        Ok(size) => {
            let width = u32::try_from(size.width).ok();
            let height = u32::try_from(size.height).ok();
            (width, height)
        }
        Err(_) => (None, None),
    }
}

fn detect_mime(path: &Path, bytes: &[u8]) -> String {
    if looks_like_png(bytes) {
        return "image/png".to_string();
    }
    if looks_like_jpeg(bytes) {
        return "image/jpeg".to_string();
    }
    if looks_like_gif(bytes) {
        return "image/gif".to_string();
    }
    if looks_like_webp(bytes) {
        return "image/webp".to_string();
    }
    if looks_like_bmp(bytes) {
        return "image/bmp".to_string();
    }
    if looks_like_svg(bytes) {
        return "image/svg+xml".to_string();
    }
    if looks_like_pdf(bytes) {
        return "application/pdf".to_string();
    }
    if looks_like_utf8_text(bytes) {
        return MimeGuess::from_path(path)
            .first_raw()
            .filter(|mime| {
                mime.starts_with("text/")
                    || matches!(
                        *mime,
                        "application/json"
                            | "application/xml"
                            | "application/yaml"
                            | "application/x-yaml"
                            | "application/javascript"
                    )
            })
            .map(str::to_owned)
            .unwrap_or_else(|| "text/plain".to_string());
    }

    MimeGuess::from_path(path)
        .first_raw()
        .map(str::to_owned)
        .unwrap_or_else(|| "application/octet-stream".to_string())
}

fn looks_like_png(bytes: &[u8]) -> bool {
    bytes.starts_with(b"\x89PNG\r\n\x1a\n")
}

fn looks_like_jpeg(bytes: &[u8]) -> bool {
    bytes.len() >= 3 && bytes[0] == 0xff && bytes[1] == 0xd8 && bytes[2] == 0xff
}

fn looks_like_gif(bytes: &[u8]) -> bool {
    bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a")
}

fn looks_like_webp(bytes: &[u8]) -> bool {
    bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP"
}

fn looks_like_bmp(bytes: &[u8]) -> bool {
    bytes.starts_with(b"BM")
}

fn looks_like_svg(bytes: &[u8]) -> bool {
    let prefix_len = bytes.len().min(2048);
    let sample = String::from_utf8_lossy(&bytes[..prefix_len]);
    let trimmed = sample.trim_start_matches('\u{feff}').trim_start();
    let lowered = trimmed.to_ascii_lowercase();
    lowered.starts_with("<svg") || lowered.starts_with("<?xml") && lowered.contains("<svg")
}

fn looks_like_pdf(bytes: &[u8]) -> bool {
    bytes.starts_with(b"%PDF-")
}

fn looks_like_utf8_text(bytes: &[u8]) -> bool {
    if bytes.contains(&0) {
        return false;
    }

    std::str::from_utf8(bytes).is_ok()
}
