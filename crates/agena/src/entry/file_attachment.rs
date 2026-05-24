use std::fs;
use std::path::Path;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use mime_guess::MimeGuess;

use crate::entry::payload::ReadAttachmentOutput;
use crate::message::{AttachmentItem, AttachmentKind, AttachmentSource};

use super::{ToolError, ToolExecutionView, ToolExecutor, ToolPayloadExecution, ToolPayloadOutput};

const MAX_FILE_BYTES: usize = 50 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
struct PreparedFileAttachment {
    path: String,
    kind: AttachmentKind,
    mime: String,
    size_bytes: u64,
    filename: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    duration_ms: Option<u64>,
    page_count: Option<u32>,
    summary: String,
    attachment: AttachmentItem,
}

impl PreparedFileAttachment {
    fn read_attachment_output(&self) -> ReadAttachmentOutput {
        ReadAttachmentOutput {
            path: self.path.clone(),
            kind: self.kind,
            mime: self.mime.clone(),
            size_bytes: self.size_bytes,
            filename: self.filename.clone(),
            width: self.width,
            height: self.height,
            duration_ms: self.duration_ms,
            page_count: self.page_count,
        }
    }

    fn into_attachment(self) -> AttachmentItem {
        self.attachment
    }
}

pub(super) fn execute_for_read_attachment(
    executor: &ToolExecutor,
    path: &str,
) -> Result<ToolPayloadExecution, ToolError> {
    let prepared = prepare_file_attachment(executor, path)?;
    let output = ToolPayloadOutput::Read {
        preview: None,
        truncated: None,
        loaded_paths: vec![prepared.path.clone()],
        attachment: Some(prepared.read_attachment_output()),
    };
    let mut view =
        ToolExecutionView::simple(format!("Read {}", prepared.path), prepared.summary.clone());
    populate_view_metadata(&mut view, &prepared);
    view.attachments.push(prepared.into_attachment());
    Ok(ToolPayloadExecution::new(output, view))
}

pub(super) fn should_attach_in_read_auto(path: &Path, bytes: &[u8]) -> bool {
    let mime = detect_mime(path, bytes);
    let filename = path.file_name().and_then(|name| name.to_str());
    let kind = AttachmentKind::detect(mime.as_str(), filename);
    kind != AttachmentKind::File || !looks_like_utf8_text(bytes)
}

fn prepare_file_attachment(
    executor: &ToolExecutor,
    input_path: &str,
) -> Result<PreparedFileAttachment, ToolError> {
    let target = executor.resolve_target_path(input_path);
    executor.ensure_read_permission(&target)?;

    if !target.exists() {
        return Err(ToolError::InvalidInput(format!(
            "file attachment target does not exist: {}",
            input_path
        )));
    }

    if !target.is_file() {
        return Err(ToolError::InvalidInput(format!(
            "file attachment target is not a file: {}",
            input_path
        )));
    }

    let bytes = fs::read(&target)?;
    if bytes.is_empty() {
        return Err(ToolError::InvalidInput(format!(
            "file attachment target is empty: {}",
            input_path
        )));
    }

    if bytes.len() > MAX_FILE_BYTES {
        return Err(ToolError::InvalidInput(format!(
            "file attachment mode supports files up to {} bytes: {}",
            MAX_FILE_BYTES, input_path
        )));
    }

    Ok(build_file_attachment(executor, &target, &bytes))
}

fn build_file_attachment(
    executor: &ToolExecutor,
    target: &Path,
    bytes: &[u8],
) -> PreparedFileAttachment {
    let display_path = executor.display_path(target);
    let filename = target
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_owned)
        .unwrap_or_else(|| display_path.clone());
    let mime = detect_mime(target, bytes);
    let kind = AttachmentKind::detect(mime.as_str(), Some(filename.as_str()));
    let (width, height) = match kind {
        AttachmentKind::Image => detect_dimensions(bytes),
        _ => (None, None),
    };

    let summary = render_summary(
        display_path.as_str(),
        kind,
        mime.as_str(),
        bytes.len(),
        width,
        height,
    );

    PreparedFileAttachment {
        path: display_path,
        kind,
        mime: mime.clone(),
        size_bytes: bytes.len() as u64,
        filename: Some(filename.clone()),
        width,
        height,
        duration_ms: None,
        page_count: None,
        summary,
        attachment: AttachmentItem {
            kind,
            mime,
            source: AttachmentSource::Base64 {
                data: STANDARD.encode(bytes),
            },
            filename: Some(filename),
            title: None,
            size_bytes: Some(bytes.len() as u64),
            sha256: None,
            width,
            height,
            duration_ms: None,
            page_count: None,
        },
    }
}

fn populate_view_metadata(view: &mut ToolExecutionView, prepared: &PreparedFileAttachment) {
    view.metadata
        .insert("kind".to_string(), prepared.kind.as_str().to_string());
    view.metadata
        .insert("path".to_string(), prepared.path.clone());
    if let Some(filename) = prepared.filename.as_ref() {
        view.metadata
            .insert("filename".to_string(), filename.clone());
    }
    view.metadata
        .insert("mime".to_string(), prepared.mime.clone());
    view.metadata
        .insert("size_bytes".to_string(), prepared.size_bytes.to_string());
    if let Some(width) = prepared.width {
        view.metadata.insert("width".to_string(), width.to_string());
    }
    if let Some(height) = prepared.height {
        view.metadata
            .insert("height".to_string(), height.to_string());
    }
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

fn detect_mime(path: &Path, _bytes: &[u8]) -> String {
    let guess = MimeGuess::from_path(path);
    guess
        .first_raw()
        .unwrap_or("application/octet-stream")
        .to_string()
}

fn looks_like_utf8_text(bytes: &[u8]) -> bool {
    std::str::from_utf8(bytes)
        .map(|text| {
            text.chars()
                .all(|ch| ch == '\n' || ch == '\r' || ch == '\t' || !ch.is_control())
        })
        .unwrap_or(false)
}
