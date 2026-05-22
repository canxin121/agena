use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AttachmentKind {
    Image,
    Audio,
    Video,
    Pdf,
    File,
}

impl AttachmentKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Image => "image",
            Self::Audio => "audio",
            Self::Video => "video",
            Self::Pdf => "pdf",
            Self::File => "file",
        }
    }

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
            .and_then(|value| {
                value
                    .rsplit(['/', '\\'])
                    .next()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
            })
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
        match self {
            Self::Url { url } | Self::DataUrl { url } => {
                let trimmed = url.trim();
                if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed)
                }
            }
            Self::Base64 { .. } => Some("base64"),
            Self::FileId { file_id } => {
                let trimmed = file_id.trim();
                if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed)
                }
            }
            Self::LocalPath { path } => {
                let trimmed = path.trim();
                if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed)
                }
            }
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
        if let Some(filename) = self.filename.as_ref() {
            let trimmed = filename.trim();
            if !trimmed.is_empty() {
                return trimmed.to_owned();
            }
        }

        if let Some(title) = self.title.as_ref() {
            let trimmed = title.trim();
            if !trimmed.is_empty() {
                return trimmed.to_owned();
            }
        }

        if let Some(hint) = self.source.summary_hint() {
            let trimmed = hint.trim();
            if !trimmed.is_empty() {
                return trimmed.to_owned();
            }
        }

        self.kind.as_str().to_owned()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct AttachmentPart {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<AttachmentItem>,
}
