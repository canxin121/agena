//! Protocol-level media contracts, independent of catalog/model overrides.
//! A model capability cannot make its selected protocol accept another shape.

use crate::ProviderError;
use agena_domain::{AttachmentItem, AttachmentKind, AttachmentSource};

#[derive(Clone, Copy, Debug)]
pub enum MediaProtocol {
    OpenAiResponses,
    OpenAiChat,
    Anthropic,
    Gemini,
}

fn inline_size(item: &AttachmentItem) -> usize {
    match &item.source {
        AttachmentSource::Base64 { data } | AttachmentSource::ProviderData { data, .. } => {
            data.len()
        }
        AttachmentSource::DataUrl { url } => url.len(),
        _ => 0,
    }
}

pub fn validate(protocol: MediaProtocol, items: &[AttachmentItem]) -> Result<(), ProviderError> {
    let mut inline_total = 0usize;
    for item in items {
        // A local path is a reference, not an attempt to send media bytes.
        // An unavailable attachment carries no bytes at all: it is delivered
        // as an explicit textual notice, so no protocol rule about media
        // shapes or payload budgets can apply to it. Rejecting one would turn
        // a conversation that merely lost its bytes into an unusable one.
        if matches!(
            item.source,
            AttachmentSource::LocalPath { .. } | AttachmentSource::Unavailable { .. }
        ) {
            continue;
        }
        let mime = item.mime.trim().to_ascii_lowercase();
        let encoded_size = inline_size(item);
        inline_total = inline_total.saturating_add(encoded_size);
        let unsupported = |reason: &str| {
            ProviderError::Config(format!(
                "Cannot deliver {} ({mime}) through {protocol:?}: {reason}. No media content was sent. Use fs.document for extracted document text, a supported model/protocol, or explicitly convert the media first.",
                item.summary_label(),
            ))
        };
        let image_mime = matches!(
            mime.as_str(),
            "image/png" | "image/jpeg" | "image/webp" | "image/gif"
        );
        if item.kind == AttachmentKind::Pdf && mime != "application/pdf" {
            return Err(unsupported("PDF content requires application/pdf MIME"));
        }
        match &item.source {
            AttachmentSource::Base64 { data } | AttachmentSource::ProviderData { data, .. }
                if data.trim().is_empty() =>
            {
                return Err(unsupported("inline media bytes are empty"));
            }
            AttachmentSource::DataUrl { url } => {
                if !url
                    .trim()
                    .split_once(',')
                    .is_some_and(|(header, _)| header.ends_with(";base64"))
                {
                    return Err(unsupported("media data URL requires base64 encoding"));
                }
                let Some((embedded_mime, _)) = super::wire_message::base64_with_mime(item) else {
                    return Err(unsupported("invalid or empty base64 data URL"));
                };
                if !embedded_mime.eq_ignore_ascii_case(&mime) {
                    return Err(unsupported(
                        "data URL MIME disagrees with the attachment MIME",
                    ));
                }
            }
            _ => {}
        }
        if item.kind == AttachmentKind::Image
            && matches!(item.source, AttachmentSource::FileId { .. })
        {
            return Err(unsupported(
                "this adapter's image path requires an image URL or inline bytes",
            ));
        }
        match protocol {
            MediaProtocol::OpenAiResponses => match item.kind {
                AttachmentKind::Audio | AttachmentKind::Video => {
                    return Err(unsupported(
                        "Responses input_file is not native audio/video input; audio requires an audio-capable Chat model and video requires a video-capable protocol",
                    ));
                }
                AttachmentKind::Image if !image_mime => {
                    return Err(unsupported("unsupported image format"));
                }
                _ => {}
            },
            MediaProtocol::OpenAiChat => match item.kind {
                AttachmentKind::Video => {
                    return Err(unsupported("Chat Completions has no native video input"));
                }
                AttachmentKind::Audio => {
                    if !matches!(
                        mime.as_str(),
                        "audio/wav" | "audio/x-wav" | "audio/mpeg" | "audio/mp3"
                    ) {
                        return Err(unsupported("input_audio accepts WAV or MP3 only"));
                    }
                    if encoded_size == 0 {
                        return Err(unsupported(
                            "input_audio needs inline audio bytes; read the local file with fs.read_media",
                        ));
                    }
                }
                AttachmentKind::Image if !image_mime => {
                    return Err(unsupported("unsupported image format"));
                }
                AttachmentKind::Pdf if matches!(item.source, AttachmentSource::Url { .. }) => {
                    return Err(unsupported(
                        "Chat PDF inputs require file_data or file_id, not file_url",
                    ));
                }
                AttachmentKind::File
                    if !mime.starts_with("text/")
                        && !matches!(
                            mime.as_str(),
                            "application/json"
                                | "application/xml"
                                | "application/yaml"
                                | "application/javascript"
                        ) =>
                {
                    return Err(unsupported(
                        "Chat file content parts accept PDF only; extract other documents as text first",
                    ));
                }
                _ => {}
            },
            MediaProtocol::Anthropic => {
                if matches!(
                    item.source,
                    AttachmentSource::Url { .. } | AttachmentSource::FileId { .. }
                ) {
                    return Err(unsupported(
                        "this adapter's Messages media path requires inline bytes; use fs.read_media for local files",
                    ));
                }
                match item.kind {
                    AttachmentKind::Audio | AttachmentKind::Video => {
                        return Err(unsupported(
                            "Messages has no native audio/video content block",
                        ));
                    }
                    AttachmentKind::Image => {
                        if !image_mime {
                            return Err(unsupported("unsupported image format"));
                        }
                        if encoded_size > 10_000_000 {
                            return Err(unsupported(
                                "image exceeds the 10 MB base64-encoded limit",
                            ));
                        }
                        if item.width.is_some_and(|v| v > 8000)
                            || item.height.is_some_and(|v| v > 8000)
                        {
                            return Err(unsupported("image dimensions exceed 8000 pixels"));
                        }
                    }
                    AttachmentKind::File
                        if !mime.starts_with("text/")
                            && !matches!(
                                mime.as_str(),
                                "application/json" | "application/xml" | "application/yaml"
                            ) =>
                    {
                        return Err(unsupported("document input requires PDF or extracted text"));
                    }
                    _ => {}
                }
            }
            MediaProtocol::Gemini => {
                let supported = match item.kind {
                    AttachmentKind::Image => image_mime && mime != "image/gif",
                    AttachmentKind::Pdf => mime == "application/pdf",
                    AttachmentKind::File => {
                        mime.starts_with("text/")
                            || matches!(
                                mime.as_str(),
                                "application/json" | "application/xml" | "application/yaml"
                            )
                    }
                    AttachmentKind::Audio => matches!(
                        mime.as_str(),
                        "audio/wav"
                            | "audio/x-wav"
                            | "audio/mpeg"
                            | "audio/mp3"
                            | "audio/aiff"
                            | "audio/aac"
                            | "audio/ogg"
                            | "audio/flac"
                            | "audio/x-flac"
                            | "audio/mp4"
                            | "audio/m4a"
                            | "audio/webm"
                    ),
                    AttachmentKind::Video => matches!(
                        mime.as_str(),
                        "video/mp4"
                            | "video/webm"
                            | "video/mpeg"
                            | "video/quicktime"
                            | "video/avi"
                            | "video/x-flv"
                            | "video/mpg"
                            | "video/wmv"
                            | "video/3gpp"
                    ),
                };
                if !supported {
                    return Err(unsupported(
                        "MIME type is not supported by this media input path",
                    ));
                }
                if matches!(
                    item.source,
                    AttachmentSource::FileId { .. } | AttachmentSource::Url { .. }
                ) {
                    return Err(unsupported(
                        "this adapter's media path requires inline bytes; use fs.read_media for local files",
                    ));
                }
            }
        }
    }
    // Conservative payload budgets include base64 expansion. Larger Gemini
    // media should use Files API; do not silently exceed older 20 MB endpoints.
    let budget = match protocol {
        MediaProtocol::Anthropic => 31_000_000,
        MediaProtocol::Gemini => 19_000_000,
        MediaProtocol::OpenAiResponses | MediaProtocol::OpenAiChat => 49_000_000,
    };
    if inline_total > budget {
        return Err(ProviderError::Config(format!(
            "{protocol:?} inline media exceeds the {budget}-byte encoded payload budget; select smaller files or convert/clip the media before reading"
        )));
    }
    Ok(())
}
