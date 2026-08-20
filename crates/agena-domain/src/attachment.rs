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
    Url { url: String },
    DataUrl { url: String },
    Base64 { data: String },
    FileId { file_id: String },
    LocalPath { path: String },
}

impl AttachmentSource {
    pub fn summary_hint(&self) -> Option<&str> {
        let value = match self {
            Self::Url { url } | Self::DataUrl { url } => url,
            Self::Base64 { .. } => return Some("base64"),
            Self::FileId { file_id } => file_id,
            Self::LocalPath { path } => path,
        };
        (!value.trim().is_empty()).then_some(value.as_str())
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
