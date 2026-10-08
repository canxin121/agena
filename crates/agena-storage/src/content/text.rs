//! Bounded projections with resumable offsets inside a source record.

use agena_domain::{ContentPayload, ContentTextPage, ContentTextPosition, ContentTextSlice};

use super::{ContentCursor, ContentHub, ContentId, StoreError};

// A byte budget alone does not bound the JSON envelope for tiny records. Also
// bound non-text records so a diagnostic read cannot scan arbitrary history.
const MAX_TEXT_PAGE_RECORDS: usize = 32;

impl ContentHub {
    pub async fn read_text_page(
        &self,
        id: ContentId,
        position: Option<ContentTextPosition>,
        max_bytes: usize,
    ) -> Result<ContentTextPage, StoreError> {
        if !(4..=1024 * 1024).contains(&max_bytes) {
            return Err(StoreError::Constraint(
                "text read budget must be 4–1048576 bytes".into(),
            ));
        }
        let resource = self.describe(id).await?;
        let start = position.unwrap_or(ContentTextPosition {
            after: ContentCursor {
                sequence: 0,
                ..resource.cursor
            },
            offset: 0,
        });
        let page = self.read(id, Some(start.after), max_bytes).await?;
        let mut next = start;
        let mut slices = vec![];
        let mut remaining = max_bytes;
        let mut consumed_page = true;
        for (index, chunk) in page.chunks.iter().enumerate() {
            if index == MAX_TEXT_PAGE_RECORDS {
                consumed_page = false;
                break;
            }
            let offset = if chunk.cursor.sequence == start.after.sequence.saturating_add(1) {
                start.offset
            } else {
                if start.offset > 0 && next == start {
                    return Err(StoreError::Conflict(
                        "the partially read record is no longer retained".into(),
                    ));
                }
                0
            };
            if let Some(text) = chunk.payload.text_content() {
                if offset > text.len() || !text.is_char_boundary(offset) {
                    return Err(StoreError::Constraint(
                        "text offset must be within a record at a UTF-8 boundary".into(),
                    ));
                }
                let mut end = text.len().min(offset.saturating_add(remaining));
                while !text.is_char_boundary(end) {
                    end -= 1;
                }
                if end > offset {
                    slices.push(ContentTextSlice {
                        cursor: chunk.cursor,
                        captured_at_ms: chunk.captured_at_ms,
                        offset,
                        stream: match &chunk.payload {
                            ContentPayload::Log { stream, .. } => Some(stream.clone()),
                            _ => None,
                        },
                        text: text[offset..end].to_owned(),
                    });
                    remaining -= end - offset;
                }
                if end < text.len() {
                    next = ContentTextPosition {
                        after: ContentCursor {
                            sequence: chunk.cursor.sequence - 1,
                            ..chunk.cursor
                        },
                        offset: end,
                    };
                    consumed_page = false;
                    break;
                }
            } else if offset != 0 {
                return Err(StoreError::Constraint(
                    "non-text records do not accept a text offset".into(),
                ));
            }
            next = ContentTextPosition {
                after: chunk.cursor,
                offset: 0,
            };
        }
        if page.chunks.is_empty() && start.offset > 0 {
            return Err(StoreError::Conflict(
                "the partially read record is unavailable".into(),
            ));
        }
        if consumed_page {
            next = ContentTextPosition {
                after: page.next_cursor,
                offset: 0,
            };
        }
        Ok(ContentTextPage {
            resource: page.resource,
            slices,
            next_position: next,
            has_more: !consumed_page || page.has_more,
            gap: page.gap,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agena_domain::{CommandOutputStream, ContentInput, ContentKind, ContentState};

    #[tokio::test]
    async fn resumes_inside_one_large_record_without_losing_unicode_or_whitespace() {
        let hub = ContentHub::in_memory();
        let writer = hub.open(1, 2, ContentKind::Text).await.unwrap();
        let text = format!("  {}\n\n", "中文🙂 ".repeat(4000));
        writer.append_text(&text).await.unwrap();
        let id = writer
            .finalize(ContentState::Complete)
            .await
            .unwrap()
            .resource_id;
        let mut position = None;
        let mut restored = String::new();
        loop {
            let page = hub.read_text_page(id, position, 257).await.unwrap();
            assert!(!page.gap);
            assert!(
                page.slices
                    .iter()
                    .map(|slice| slice.text.len())
                    .sum::<usize>()
                    <= 257
            );
            for slice in page.slices {
                restored.push_str(&slice.text);
            }
            if !page.has_more {
                break;
            }
            assert_ne!(position, Some(page.next_position));
            position = Some(page.next_position);
        }
        assert_eq!(restored, text);
        assert!(
            hub.read_text_page(
                id,
                Some(ContentTextPosition {
                    after: ContentCursor {
                        sequence: 0,
                        epoch: id.0
                    },
                    offset: 3,
                }),
                256
            )
            .await
            .is_err(),
            "reject offsets inside a Chinese character"
        );
    }

    #[tokio::test]
    async fn preserves_log_channels_and_rejects_offsets_for_non_text_records() {
        let hub = ContentHub::in_memory();
        let writer = hub.open(1, 2, ContentKind::Log).await.unwrap();
        for (stream, text) in [
            (CommandOutputStream::Stdout, "  first\n"),
            (CommandOutputStream::Stderr, "error\n\n"),
        ] {
            writer
                .append(ContentInput::Log {
                    stream,
                    text: text.into(),
                })
                .await
                .unwrap();
        }
        let resource = writer.finalize(ContentState::Complete).await.unwrap();
        let page = hub
            .read_text_page(resource.resource_id, None, 4096)
            .await
            .unwrap();
        assert_eq!(page.slices.len(), 2);
        assert_eq!(page.slices[0].stream, Some(CommandOutputStream::Stdout));
        assert_eq!(page.slices[1].stream, Some(CommandOutputStream::Stderr));
        assert_eq!(page.slices[1].text, "error\n\n");
        assert!(!page.has_more);
        assert!(
            hub.read_text_page(
                resource.resource_id,
                Some(ContentTextPosition {
                    after: resource.cursor,
                    offset: 1
                }),
                4096
            )
            .await
            .is_err()
        );
    }

    #[tokio::test]
    async fn tiny_records_have_bounded_envelopes_and_resume_without_skipping() {
        let hub = ContentHub::in_memory();
        let writer = hub.open(1, 2, ContentKind::Log).await.unwrap();
        for index in 0..257 {
            writer
                .append(ContentInput::Log {
                    stream: if index % 2 == 0 {
                        CommandOutputStream::Stdout
                    } else {
                        CommandOutputStream::Stderr
                    },
                    text: "\0".into(),
                })
                .await
                .unwrap();
        }
        let resource = writer.finalize(ContentState::Complete).await.unwrap();
        let mut position = None;
        let mut sequences = Vec::new();
        loop {
            let page = hub
                .read_text_page(resource.resource_id, position, 4096)
                .await
                .unwrap();
            assert!(page.slices.len() <= MAX_TEXT_PAGE_RECORDS);
            assert!(serde_json::to_vec(&page).unwrap().len() < 12 * 1024);
            sequences.extend(page.slices.iter().map(|slice| slice.cursor.sequence));
            if !page.has_more {
                break;
            }
            assert_ne!(position, Some(page.next_position));
            position = Some(page.next_position);
        }
        assert_eq!(sequences, (1..=257).collect::<Vec<_>>());
    }
}
