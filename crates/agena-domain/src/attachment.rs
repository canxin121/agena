//! Raw attachment facts shared by tools, plugins, providers, and renderers.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::ArtifactRef;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AttachmentKind {
    Image,
    Audio,
    Video,
    Pdf,
    File,
}

impl AsRef<str> for AttachmentKind {
    fn as_ref(&self) -> &str {
        match self {
            Self::Image => "image",
            Self::Audio => "audio",
            Self::Video => "video",
            Self::Pdf => "pdf",
            Self::File => "file",
        }
    }
}

impl fmt::Display for AttachmentKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_ref())
    }
}

impl AttachmentKind {
    pub fn detect(mime: &str, hint: Option<&str>) -> Self {
        let normalized_mime = mime.trim().to_ascii_lowercase();
        if normalized_mime.starts_with("image/") {
            return Self::Image;
        }
        if normalized_mime.starts_with("audio/") {
            return Self::Audio;
        }
        if normalized_mime.starts_with("video/") {
            return Self::Video;
        }
        if normalized_mime == "application/pdf" {
            return Self::Pdf;
        }

        let normalized_hint = hint
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .and_then(|value| value.rsplit(['/', '\\']).next())
            .map(str::to_ascii_lowercase);
        if let Some(hint) = normalized_hint.as_deref() {
            if [".png", ".jpg", ".jpeg", ".gif", ".webp", ".svg", ".bmp"]
                .iter()
                .any(|suffix| hint.ends_with(suffix))
            {
                return Self::Image;
            }
            if [".mp3", ".wav", ".m4a", ".ogg", ".flac"]
                .iter()
                .any(|suffix| hint.ends_with(suffix))
            {
                return Self::Audio;
            }
            if [".mp4", ".mov", ".webm", ".avi", ".mkv"]
                .iter()
                .any(|suffix| hint.ends_with(suffix))
            {
                return Self::Video;
            }
            if hint.ends_with(".pdf") {
                return Self::Pdf;
            }
        }
        Self::File
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum AttachmentSource {
    Url {
        url: String,
    },
    DataUrl {
        url: String,
    },
    Base64 {
        data: String,
    },
    /// Explicit user input, immutable bytes scoped to the selected model route.
    ProviderData {
        route: String,
        data: String,
    },
    FileId {
        file_id: String,
    },
    LocalPath {
        path: String,
    },
    /// The bytes are gone: a provider-hosted id from a route that is no longer
    /// selected, an expired upload, or a blob that no longer belongs to this
    /// conversation. History keeps the fact that something was attached, the
    /// model receives a text placeholder instead, and the session stays
    /// usable. This variant is what a failed hydration degrades to — it is
    /// never produced by an explicit user selection.
    Unavailable {
        #[serde(default, skip_serializing_if = "String::is_empty")]
        reason: String,
    },
}

impl AttachmentSource {
    pub fn summary_hint(&self) -> Option<&str> {
        let value = match self {
            Self::Url { url } | Self::DataUrl { url } => url,
            Self::Base64 { .. } => return Some("base64"),
            Self::ProviderData { .. } => return Some("provider-bound media"),
            Self::FileId { file_id } => file_id,
            Self::LocalPath { path } => path,
            Self::Unavailable { .. } => return None,
        };
        (!value.trim().is_empty()).then_some(value.as_str())
    }
    pub fn is_unavailable(&self) -> bool {
        matches!(self, Self::Unavailable { .. })
    }
    /// The textual substitute a model receives in place of bytes that can no
    /// longer be delivered. It is deliberately explicit: a model must never
    /// believe it saw content it did not receive.
    pub fn unavailable_notice(label: &str, reason: &str) -> String {
        let reason = reason.trim();
        if reason.is_empty() {
            format!(
                "[{label} is unavailable in this conversation; it was attached through a different provider or model. Attach the file again to use it here.]"
            )
        } else {
            format!("[{label} is unavailable: {reason}. Attach the file again to use it here.]")
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AttachmentItem {
    pub kind: AttachmentKind,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub mime: String,
    pub source: AttachmentSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_count: Option<u32>,
}

impl AttachmentItem {
    pub fn summary_label(&self) -> String {
        self.filename
            .as_deref()
            .or(self.title.as_deref())
            .or_else(|| self.source.summary_hint())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| self.kind.to_string())
    }
    /// Mark this attachment's bytes as no longer deliverable. Identity and
    /// display metadata are kept so the conversation still reads correctly;
    /// the checksum and size describe bytes that are no longer present, so
    /// they are cleared with them.
    pub fn degrade(&mut self, reason: impl Into<String>) {
        if self.source.is_unavailable() {
            return;
        }
        self.source = AttachmentSource::Unavailable {
            reason: reason.into(),
        };
        self.sha256 = None;
        self.size_bytes = None;
    }
}

impl From<ArtifactRef> for AttachmentItem {
    fn from(artifact: ArtifactRef) -> Self {
        let source = if artifact.uri.starts_with("data:") {
            AttachmentSource::DataUrl {
                url: artifact.uri.clone(),
            }
        } else if let Some(path) = artifact.uri.strip_prefix("file://") {
            AttachmentSource::LocalPath {
                path: path.to_owned(),
            }
        } else {
            AttachmentSource::Url {
                url: artifact.uri.clone(),
            }
        };
        Self {
            kind: AttachmentKind::detect(&artifact.mime, artifact.name.as_deref()),
            mime: artifact.mime,
            source,
            filename: artifact.name,
            title: None,
            size_bytes: artifact.size_bytes,
            sha256: artifact.sha256,
            width: None,
            height: None,
            duration_ms: None,
            page_count: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct AttachmentPart {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<AttachmentItem>,
}

/// A portable attachment basename. Preserve human Unicode names, remove path
/// and control syntax, and bound UTF-8 bytes (not characters) so a UUID prefix
/// still fits ordinary filesystem component limits.
pub fn safe_attachment_filename(value: &str) -> String {
    const MAX_BYTES: usize = 200;
    let base = value.rsplit(['/', '\\']).next().unwrap_or_default().trim();
    if base.is_empty() || matches!(base, "." | "..") {
        return String::new();
    }
    let name: String = base
        .chars()
        .map(|ch| {
            if ch.is_control()
                || matches!(ch, '<' | '>' | ':' | '"' | '|' | '?' | '*'
            | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
            {
                '_'
            } else {
                ch
            }
        })
        .collect();
    let mut name = name.trim_matches(' ').trim_end_matches('.').to_owned();
    if name.is_empty() {
        return String::new();
    }
    let stem = name
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    if matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && matches!(stem.as_bytes()[3], b'1'..=b'9'))
    {
        name.insert(0, '_');
    }
    if name.len() > MAX_BYTES {
        let extension = name
            .rsplit_once('.')
            .filter(|(stem, ext)| !stem.is_empty() && !ext.is_empty() && ext.len() <= 32)
            .map(|(_, ext)| format!(".{ext}"))
            .unwrap_or_default();
        let mut end = MAX_BYTES - extension.len();
        while !name.is_char_boundary(end) {
            end -= 1;
        }
        name.truncate(end);
        name.push_str(&extension);
    }
    name
}

#[cfg(test)]
mod filename_tests {
    use super::safe_attachment_filename as safe;
    #[test]
    fn unicode_names_are_not_destroyed_and_limits_count_bytes() {
        assert_eq!(safe("目录/向日葵 报告.pdf"), "向日葵 报告.pdf");
        assert_eq!(safe(r"C:\temp\向日葵.png"), "向日葵.png");
        let name = safe(&format!("{}.png", "向日葵".repeat(100)));
        assert!(name.len() <= 200 && name.ends_with(".png"));
        assert!(name.is_char_boundary(name.len()));
    }
    #[test]
    fn filenames_cannot_traverse_inject_or_name_devices() {
        for value in ["", ".", "..", "/tmp/..", "dir/"] {
            assert!(safe(value).is_empty(), "{value}");
        }
        assert_eq!(safe("../../file.txt"), "file.txt");
        assert_eq!(safe("file\r\n\".txt"), "file___.txt");
        assert_eq!(safe("NUL.txt"), "_NUL.txt");
        assert_eq!(safe("LPT1"), "_LPT1");
        assert_eq!(safe("file.png. "), "file.png");
    }
}
