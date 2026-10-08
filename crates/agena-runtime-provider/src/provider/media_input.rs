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
        if matches!(item.source, AttachmentSource::LocalPath { .. }) {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn item(kind: &str, mime: &str) -> AttachmentItem {
        serde_json::from_value(serde_json::json!({
            "kind": kind, "mime": mime, "source": {"source": "base64", "data": "Zml4dHVyZQ=="}
        }))
        .unwrap()
    }

    #[test]
    fn media_protocols_accept_only_the_shapes_their_adapters_deliver() {
        let image = item("image", "image/png");
        let pdf = item("pdf", "application/pdf");
        let wav = item("audio", "audio/wav");
        let flac = item("audio", "audio/flac");
        let video = item("video", "video/mp4");
        for protocol in [
            MediaProtocol::OpenAiResponses,
            MediaProtocol::OpenAiChat,
            MediaProtocol::Anthropic,
            MediaProtocol::Gemini,
        ] {
            validate(protocol, &[image.clone(), pdf.clone()]).unwrap();
        }
        for protocol in [MediaProtocol::OpenAiResponses, MediaProtocol::Anthropic] {
            assert!(validate(protocol, std::slice::from_ref(&wav)).is_err());
            assert!(validate(protocol, std::slice::from_ref(&video)).is_err());
        }
        validate(
            MediaProtocol::OpenAiChat,
            &[wav.clone(), item("audio", "audio/mpeg")],
        )
        .unwrap();
        assert!(validate(MediaProtocol::OpenAiChat, std::slice::from_ref(&flac)).is_err());
        assert!(validate(MediaProtocol::OpenAiChat, std::slice::from_ref(&video)).is_err());
        validate(MediaProtocol::Gemini, &[wav, flac, video]).unwrap();
        let mut reference = image.clone();
        reference.source = AttachmentSource::LocalPath {
            path: "missing.png".into(),
        };
        validate(MediaProtocol::Anthropic, &[reference]).unwrap();
        let mut url_pdf = pdf;
        url_pdf.source = AttachmentSource::Url {
            url: "https://example.test/input.pdf".into(),
        };
        validate(
            MediaProtocol::OpenAiResponses,
            std::slice::from_ref(&url_pdf),
        )
        .unwrap();
        for protocol in [
            MediaProtocol::OpenAiChat,
            MediaProtocol::Anthropic,
            MediaProtocol::Gemini,
        ] {
            assert!(validate(protocol, std::slice::from_ref(&url_pdf)).is_err());
        }
        assert!(
            validate(
                MediaProtocol::OpenAiChat,
                &[item("file", "application/zip")]
            )
            .is_err()
        );
        assert!(
            validate(
                MediaProtocol::OpenAiResponses,
                &[item("image", "image/heic")]
            )
            .is_err()
        );
    }

    #[test]
    fn encoded_payload_budgets_include_all_media_results() {
        let mut oversized = item("image", "image/png");
        oversized.source = AttachmentSource::Base64 {
            data: "A".repeat(10_000_004),
        };
        assert!(
            validate(MediaProtocol::Anthropic, &[oversized])
                .unwrap_err()
                .to_string()
                .contains("10 MB")
        );
        let mut part = item("pdf", "application/pdf");
        // A raw 7.2 MB file expands to 9.6 MB; two files exceed the conservative
        // Gemini inline budget even though both raw files fit preparation.
        part.source = AttachmentSource::Base64 {
            data: "A".repeat(9_600_000),
        };
        let error = validate(MediaProtocol::Gemini, &[part.clone(), part]).unwrap_err();
        assert!(error.to_string().contains("encoded payload budget"));
        let mut part = item("pdf", "application/pdf");
        part.source = AttachmentSource::Base64 {
            data: "A".repeat(25_000_000),
        };
        assert!(validate(MediaProtocol::OpenAiResponses, &[part.clone(), part]).is_err());
    }

    #[test]
    fn invalid_mime_and_empty_media_fail_instead_of_becoming_metadata_hints() {
        let mut image = item("image", "image/png");
        image.source = AttachmentSource::DataUrl {
            url: "data:image/jpeg;base64,Zml4dHVyZQ==".into(),
        };
        assert!(validate(MediaProtocol::OpenAiChat, std::slice::from_ref(&image)).is_err());
        image.source = AttachmentSource::Base64 { data: "  ".into() };
        assert!(validate(MediaProtocol::Gemini, std::slice::from_ref(&image)).is_err());
        assert!(validate(MediaProtocol::Anthropic, &[item("pdf", "image/png")]).is_err());
        image.width = Some(8001);
        image.source = AttachmentSource::Base64 {
            data: "Zml4dHVyZQ==".into(),
        };
        assert!(validate(MediaProtocol::Anthropic, &[image]).is_err());
    }
}
