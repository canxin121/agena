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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct AttachmentPart {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<AttachmentItem>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_label_prefers_filename_then_title_then_source() {
        let mut attachment = AttachmentItem {
            kind: AttachmentKind::Image,
            mime: "image/png".to_owned(),
            source: AttachmentSource::Url {
                url: "https://example.com/a.png".to_owned(),
            },
            filename: Some("cat.png".to_owned()),
            title: Some("cat".to_owned()),
            size_bytes: None,
            sha256: None,
            width: None,
            height: None,
            duration_ms: None,
            page_count: None,
        };
        assert_eq!(attachment.summary_label(), "cat.png");

        attachment.filename = Some("   ".to_owned());
        assert_eq!(attachment.summary_label(), "cat");

        attachment.title = Some(" ".to_owned());
        assert_eq!(attachment.summary_label(), "https://example.com/a.png");
    }

    #[test]
    fn summary_label_falls_back_to_kind() {
        let attachment = AttachmentItem {
            kind: AttachmentKind::Pdf,
            mime: String::new(),
            source: AttachmentSource::FileId {
                file_id: String::new(),
            },
            filename: None,
            title: None,
            size_bytes: None,
            sha256: None,
            width: None,
            height: None,
            duration_ms: None,
            page_count: None,
        };

        assert_eq!(attachment.summary_label(), "pdf");
    }
}
