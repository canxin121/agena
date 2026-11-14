//! Byte-bounded text projections over membership-authorized kernel reads.

use super::{PluginError, plugin_error};
use agena_domain::{
    ContentCursor, ContentEpoch, ContentKind, ContentRange, ContentResource, ContentState,
    ContentTextPage, ContentTextPosition, ContentTextSlice,
};
use agena_session_api::content::ReadContentTextParams;
use agena_storage::kernel::{
    CaptureState, ContentStreamView, EventCursor, EventPayload, KernelError, KernelReader,
    SessionId, TextCursor,
};

pub(super) async fn read_content_text(
    reader: &KernelReader,
    params: ReadContentTextParams,
) -> Result<ContentTextPage, PluginError> {
    let session = SessionId::try_from(params.session_id)
        .map_err(|error| PluginError::invalid_params(error.to_string()))?;
    let watch = reader
        .watch_content(session, params.resource_id)
        .await
        .map_err(plugin_error)?;
    let head = watch.head().map_err(plugin_error)?;
    let position = params
        .position
        .filter(|position| ContentEpoch::from(position.after.epoch) == head.epoch);
    let budget = params.max_bytes.clamp(4, 1024 * 1024);
    if head.kind == ContentKind::Text {
        let cursor = position.map(|position| TextCursor {
            content: head.id,
            epoch: head.epoch,
            offset: position
                .after
                .sequence
                .saturating_add(position.offset as u64),
        });
        let mut gap = false;
        let mut result = reader.text_range(session, head.id, cursor, budget).await;
        for _ in 0..2 {
            match &result {
                Err(KernelError::ContentGap { retained_start, .. }) => {
                    gap = true;
                    let offset = *retained_start;
                    result = reader
                        .text_range(
                            session,
                            head.id,
                            Some(TextCursor {
                                content: head.id,
                                epoch: watch.head().map_err(plugin_error)?.epoch,
                                offset,
                            }),
                            budget,
                        )
                        .await;
                }
                Err(KernelError::ContentReset)
                    if cursor.is_some_and(|cursor| {
                        watch.head().is_ok_and(|head| head.epoch != cursor.epoch)
                    }) =>
                {
                    result = reader.text_range(session, head.id, None, budget).await;
                }
                _ => break,
            }
        }
        let page = result.map_err(plugin_error)?;
        // A retained tail can begin inside a scalar; those bytes belong to the
        // reported gap. A requested offset inside a scalar is an invalid cursor.
        let skipped = if gap {
            page.bytes
                .iter()
                .take_while(|byte| (**byte & 0xc0) == 0x80)
                .count()
        } else {
            0
        };
        let body = &page.bytes[skipped..];
        let length = match std::str::from_utf8(body) {
            Ok(_) => body.len(),
            Err(error) if error.error_len().is_none() => error.valid_up_to(),
            Err(error) => return Err(PluginError::invalid_params_error(&error)),
        };
        let text = String::from_utf8(body[..length].to_vec()).map_err(plugin_error)?;
        let next = ContentCursor {
            epoch: page.stream.epoch.into(),
            sequence: page.start.saturating_add((skipped + length) as u64),
        };
        let slices = if text.is_empty() {
            Vec::new()
        } else {
            vec![ContentTextSlice {
                cursor: next,
                captured_at_ms: 0,
                offset: 0,
                stream: None,
                text,
            }]
        };
        return Ok(ContentTextPage {
            inline_ui: None,
            resource: content_resource(params.session_id, &page.stream, watch.is_snapshot()),
            slices,
            next_position: ContentTextPosition {
                after: next,
                offset: 0,
            },
            has_more: page.has_more || length < body.len(),
            gap,
        });
    }

    let cursor = position.map(|position| EventCursor {
        content: head.id,
        epoch: head.epoch,
        sequence: position.after.sequence,
    });
    let mut gap = false;
    let mut result = reader.event_page(session, head.id, cursor, 64).await;
    for _ in 0..2 {
        match &result {
            Err(KernelError::ContentGap { retained_start, .. }) => {
                gap = true;
                let sequence = *retained_start;
                result = reader
                    .event_page(
                        session,
                        head.id,
                        Some(EventCursor {
                            content: head.id,
                            epoch: watch.head().map_err(plugin_error)?.epoch,
                            sequence,
                        }),
                        64,
                    )
                    .await;
            }
            Err(KernelError::ContentReset)
                if cursor.is_some_and(|cursor| {
                    watch.head().is_ok_and(|head| head.epoch != cursor.epoch)
                }) =>
            {
                result = reader.event_page(session, head.id, None, 64).await;
            }
            _ => break,
        }
    }
    let page = result.map_err(plugin_error)?;
    let epoch = page.stream.epoch.into();
    let mut next = ContentTextPosition {
        after: ContentCursor {
            epoch,
            sequence: position
                .filter(|position| position.after.epoch == epoch)
                .map_or(page.stream.retained_start, |position| {
                    position.after.sequence.max(page.stream.retained_start)
                }),
        },
        offset: 0,
    };
    let mut slices = Vec::new();
    let mut bytes = 0usize;
    for (index, record) in page.records.iter().enumerate() {
        let cursor = ContentCursor {
            epoch,
            sequence: record.sequence,
        };
        let (stream, text) = match &record.payload {
            EventPayload::Log { channel, text } => (Some(channel.clone()), Some(text.as_str())),
            EventPayload::Mixed { input, .. } => match &**input {
                agena_domain::ContentInput::Text { text } => (None, Some(text.as_str())),
                agena_domain::ContentInput::Log { stream, text } => {
                    (Some(stream.clone()), Some(text.as_str()))
                }
                _ => (None, None),
            },
            _ => (None, None),
        };
        if let Some(text) = text {
            let start = if index == 0 {
                position
                    .filter(|position| {
                        position.after.epoch == epoch
                            && position.after.sequence.saturating_add(1) == record.sequence
                    })
                    .map_or(0, |position| position.offset)
                    .min(text.len())
            } else {
                0
            };
            if !text.is_char_boundary(start) {
                return Err(PluginError::invalid_params(
                    "the text offset is not a UTF-8 boundary",
                ));
            }
            let mut end = text
                .len()
                .min(start.saturating_add(budget.saturating_sub(bytes)));
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            if end > start || start == text.len() {
                slices.push(ContentTextSlice {
                    cursor,
                    captured_at_ms: record.timestamp_ms,
                    offset: start,
                    stream,
                    text: text[start..end].to_owned(),
                });
                bytes += end - start;
            }
            if end < text.len() {
                next = ContentTextPosition {
                    after: ContentCursor {
                        epoch,
                        sequence: record.sequence.saturating_sub(1),
                    },
                    offset: end,
                };
                return Ok(ContentTextPage {
                    inline_ui: None,
                    resource: content_resource(
                        params.session_id,
                        &page.stream,
                        watch.is_snapshot(),
                    ),
                    slices,
                    next_position: next,
                    has_more: true,
                    gap,
                });
            }
        }
        // Structured and terminal records advance the cursor without inventing text.
        next = ContentTextPosition {
            after: cursor,
            offset: 0,
        };
    }
    Ok(ContentTextPage {
        inline_ui: None,
        resource: content_resource(params.session_id, &page.stream, watch.is_snapshot()),
        slices,
        next_position: next,
        has_more: page.has_more,
        gap,
    })
}

fn content_resource(
    session_id: i64,
    head: &ContentStreamView,
    is_snapshot: bool,
) -> ContentResource {
    ContentResource {
        resource_id: head.id,
        owner_session_id: session_id,
        part_id: head.entry.get(),
        kind: head.kind,
        state: match head.state {
            CaptureState::Open if !is_snapshot => ContentState::Active,
            CaptureState::Open | CaptureState::Complete => ContentState::Complete,
            CaptureState::Interrupted => ContentState::Interrupted,
        },
        cursor: ContentCursor {
            epoch: head.epoch.into(),
            sequence: head.accepted,
        },
        committed_cursor: ContentCursor {
            epoch: head.epoch.into(),
            sequence: head.committed,
        },
        total_bytes: (head.kind == ContentKind::Text).then_some(head.accepted),
        dropped_bytes: head.capture_loss.bytes,
        dropped_packets: head.capture_loss.packets,
        committed_dropped_bytes: head.capture_loss.committed_bytes,
        committed_dropped_packets: head.capture_loss.committed_packets,
        retained_ranges: if head.accepted > head.retained_start {
            vec![ContentRange {
                first: head.retained_start.saturating_add(1),
                last: head.accepted,
            }]
        } else {
            Vec::new()
        },
        capture_error: head.capture_error.clone(),
    }
}
