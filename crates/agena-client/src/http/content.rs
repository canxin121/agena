//! Cursor-based source reads. This protocol is identical for every consumer.

use super::*;
use agena_domain::{ContentCursor, ContentId, ContentPage};
use tokio::io::{AsyncWrite, AsyncWriteExt as _};

const COMPLETE_TEXT_PAGE_BYTES: usize = 64 * 1024;

#[cfg(test)]
mod tests;

pub struct ContentSubscription {
    rx: mpsc::Receiver<Result<ContentPage, ClientError>>,
    task: JoinHandle<()>,
}

impl ContentSubscription {
    pub async fn recv(&mut self) -> Option<Result<ContentPage, ClientError>> {
        self.rx.recv().await
    }
}

impl Drop for ContentSubscription {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl AgenaClient {
    /// Write the complete native Text projection of a Part to a caller-owned sink.
    ///
    /// Text resources are concatenated in reference order, followed by the
    /// Part's inline `content.text`, matching [`Self::part_text`]. Each source
    /// fixes its epoch and UTF-8 byte head from its first HTTP page; later
    /// appends remain outside that source's snapshot. Other resource kinds
    /// require their own typed projection and are skipped.
    ///
    /// Reads use pages of at most 64 KiB and have no aggregate body limit.
    /// Callers may explicitly collect into a `Vec<u8>` for a clipboard, or
    /// stream into a file for export. Retention gaps, epoch resets, malformed
    /// byte cursors, premature EOF and lack of progress return an error rather
    /// than successful truncated text. Cancellation or any failure leaves the
    /// already written prefix in the sink. The caller owns flushing, closing
    /// and discarding partial output.
    pub async fn write_part_text<W: AsyncWrite + Unpin + Send + ?Sized>(
        &self,
        session_id: i64,
        part: &agena_api::part::PartResource,
        target: &mut W,
    ) -> Result<(), ClientError> {
        if let Some(values) = part.content.get("resources") {
            let values = values.as_array().ok_or_else(|| {
                ClientError::Protocol("part content resources are not an array".into())
            })?;
            for value in values {
                let reference: agena_domain::ContentRef =
                    serde_json::from_value(value.clone()).map_err(ClientError::Decode)?;
                if reference.kind != agena_domain::ContentKind::Text {
                    continue;
                }
                self.write_native_text(
                    session_id,
                    reference.resource_id,
                    Some(part.part_id),
                    target,
                )
                .await?;
            }
        }
        if let Some(text) = part.content.get("text").and_then(serde_json::Value::as_str) {
            target.write_all(text.as_bytes()).await.map_err(|error| {
                ClientError::Transport(format!("write complete part text: {error}"))
            })?;
        }
        Ok(())
    }

    /// Write a native Text source or the ordered text of a Log source.
    ///
    /// The first page fixes the source epoch and head. Native Text heads count
    /// UTF-8 bytes; Log heads count records, and the `/text` position retains
    /// the byte offset within a partially read record. Reads use 64 KiB pages
    /// and stop after that initial head, including the whole final Log record.
    /// There is no aggregate output limit and later appends are excluded.
    ///
    /// Structured, Document and Terminal sources require a typed state
    /// projection and return an explicit error. Their records cannot be
    /// exported by treating an empty `/text` page as a complete document or
    /// screen. Callers retain partial output on cancellation or any read/write
    /// error and own sink flushing and closing, as in [`Self::write_part_text`].
    pub async fn write_content_text<W: AsyncWrite + Unpin + Send + ?Sized>(
        &self,
        session_id: i64,
        source: &agena_domain::ContentRef,
        target: &mut W,
    ) -> Result<(), ClientError> {
        if source.kind == agena_domain::ContentKind::Text {
            return self
                .write_native_text(session_id, source.resource_id, None, target)
                .await;
        }
        if source.kind != agena_domain::ContentKind::Log {
            return Err(ClientError::Protocol(
                "this content source requires a typed document or terminal projection".into(),
            ));
        }
        let mut page = self
            .read_content_text(agena_api::content::ReadContentTextParams {
                session_id,
                resource_id: source.resource_id,
                position: None,
                max_bytes: COMPLETE_TEXT_PAGE_BYTES,
            })
            .await?;
        let boundary = page.resource.cursor;
        let mut position = agena_domain::ContentTextPosition {
            after: ContentCursor {
                sequence: 0,
                ..boundary
            },
            offset: 0,
        };
        loop {
            if page.resource.resource_id != source.resource_id
                || page.resource.owner_session_id != session_id
                || page.resource.kind != source.kind
            {
                return Err(ClientError::Protocol(
                    "complete content text received a different content source".into(),
                ));
            }
            if page.resource.cursor.epoch != boundary.epoch
                || page.next_position.after.epoch != boundary.epoch
            {
                return Err(ClientError::Protocol(
                    "complete content text source epoch reset during transfer".into(),
                ));
            }
            if page.gap {
                return Err(ClientError::Protocol(
                    "complete content text encountered a content retention gap".into(),
                ));
            }
            if page.resource.cursor.sequence < boundary.sequence
                || page.next_position.after.sequence > page.resource.cursor.sequence
                || (page.next_position.after.sequence == page.resource.cursor.sequence
                    && page.next_position.offset != 0)
            {
                return Err(ClientError::Protocol(
                    "complete content text has an invalid source head or record position".into(),
                ));
            }
            let mut expected = position;
            let mut bytes = 0usize;
            for slice in &page.slices {
                if slice.cursor.epoch != boundary.epoch
                    || expected.after.sequence.checked_add(1) != Some(slice.cursor.sequence)
                    || slice.cursor.sequence > page.resource.cursor.sequence
                    || slice.offset != expected.offset
                {
                    return Err(ClientError::Protocol(
                        "complete content text record positions are not contiguous".into(),
                    ));
                }
                bytes = bytes.checked_add(slice.text.len()).ok_or_else(|| {
                    ClientError::Protocol("complete content text page length overflowed".into())
                })?;
                if bytes > COMPLETE_TEXT_PAGE_BYTES {
                    return Err(ClientError::Protocol(
                        "complete content text exceeded its requested page byte budget".into(),
                    ));
                }
                if slice.cursor.sequence <= page.next_position.after.sequence {
                    expected = agena_domain::ContentTextPosition {
                        after: slice.cursor,
                        offset: 0,
                    };
                } else {
                    expected.offset =
                        expected
                            .offset
                            .checked_add(slice.text.len())
                            .ok_or_else(|| {
                                ClientError::Protocol(
                                    "complete content text record offset overflowed".into(),
                                )
                            })?;
                }
            }
            if expected != page.next_position {
                return Err(ClientError::Protocol(
                    "complete content text page has an invalid continuation position".into(),
                ));
            }
            let advanced = expected.after.sequence > position.after.sequence
                || (expected.after.sequence == position.after.sequence
                    && expected.offset > position.offset);
            if !advanced && position.after.sequence < boundary.sequence {
                return Err(ClientError::Protocol(
                    "complete content text read made no progress".into(),
                ));
            }
            if !page.has_more && expected.after.sequence < boundary.sequence {
                return Err(ClientError::Protocol(
                    "complete content text ended before its initial source head".into(),
                ));
            }
            for slice in &page.slices {
                if slice.cursor.sequence > boundary.sequence {
                    break;
                }
                target
                    .write_all(slice.text.as_bytes())
                    .await
                    .map_err(|error| {
                        ClientError::Transport(format!("write complete content text: {error}"))
                    })?;
            }
            if expected.after.sequence >= boundary.sequence {
                break;
            }
            position = page.next_position;
            page = self
                .read_content_text(agena_api::content::ReadContentTextParams {
                    session_id,
                    resource_id: source.resource_id,
                    position: Some(position),
                    max_bytes: COMPLETE_TEXT_PAGE_BYTES,
                })
                .await?;
        }
        Ok(())
    }

    async fn write_native_text<W: AsyncWrite + Unpin + Send + ?Sized>(
        &self,
        session_id: i64,
        id: ContentId,
        part_id: Option<i64>,
        target: &mut W,
    ) -> Result<(), ClientError> {
        let mut page = self
            .read_content(session_id, id, None, COMPLETE_TEXT_PAGE_BYTES)
            .await?;
        let boundary = page.resource.cursor;
        let mut cursor = ContentCursor {
            sequence: 0,
            ..boundary
        };
        loop {
            if page.resource.resource_id != id
                || page.resource.owner_session_id != session_id
                || part_id.is_some_and(|part_id| page.resource.part_id != part_id)
                || page.resource.kind != agena_domain::ContentKind::Text
            {
                return Err(ClientError::Protocol(
                    "complete part text received a different content source".into(),
                ));
            }
            if page.resource.cursor.epoch != boundary.epoch
                || page.next_cursor.epoch != boundary.epoch
            {
                return Err(ClientError::Protocol(
                    "complete part text source epoch reset during transfer".into(),
                ));
            }
            if page.gap {
                return Err(ClientError::Protocol(
                    "complete part text encountered a content retention gap".into(),
                ));
            }
            if page.resource.cursor.sequence < boundary.sequence {
                return Err(ClientError::Protocol(
                    "complete part text source head moved backwards".into(),
                ));
            }
            let mut end = cursor.sequence;
            for chunk in &page.chunks {
                let agena_domain::ContentPayload::Text { text } = &chunk.payload else {
                    return Err(ClientError::Protocol(
                        "native Text source returned a non-text payload".into(),
                    ));
                };
                if chunk.cursor.epoch != boundary.epoch
                    || chunk.cursor.sequence.checked_sub(text.len() as u64) != Some(end)
                {
                    return Err(ClientError::Protocol(
                        "complete part text byte cursors are not contiguous".into(),
                    ));
                }
                end = chunk.cursor.sequence;
            }
            if page.next_cursor.sequence != end || end > page.resource.cursor.sequence {
                return Err(ClientError::Protocol(
                    "complete part text page has an invalid byte cursor".into(),
                ));
            }
            if end.saturating_sub(cursor.sequence) > COMPLETE_TEXT_PAGE_BYTES as u64 {
                return Err(ClientError::Protocol(
                    "complete part text exceeded its requested page byte budget".into(),
                ));
            }
            if end <= cursor.sequence && cursor.sequence < boundary.sequence {
                return Err(ClientError::Protocol(
                    "complete part text read made no progress".into(),
                ));
            }
            if !page.has_more && end < boundary.sequence {
                return Err(ClientError::Protocol(
                    "complete part text ended before its initial source head".into(),
                ));
            }
            let mut remaining = boundary.sequence - cursor.sequence;
            for chunk in &page.chunks {
                if remaining == 0 {
                    break;
                }
                if let agena_domain::ContentPayload::Text { text } = &chunk.payload {
                    let length = remaining.min(text.len() as u64) as usize;
                    if !text.is_char_boundary(length) {
                        return Err(ClientError::Protocol(
                            "complete part text source head is not a UTF-8 boundary".into(),
                        ));
                    }
                    target
                        .write_all(&text.as_bytes()[..length])
                        .await
                        .map_err(|error| {
                            ClientError::Transport(format!("write complete part text: {error}"))
                        })?;
                    remaining -= length as u64;
                }
            }
            if end >= boundary.sequence {
                break;
            }
            cursor = page.next_cursor;
            page = self
                .read_content(
                    session_id,
                    id,
                    Some(cursor),
                    (boundary.sequence - cursor.sequence).min(COMPLETE_TEXT_PAGE_BYTES as u64)
                        as usize,
                )
                .await?;
        }
        Ok(())
    }

    /// Resolve a text preview without changing the canonical Part.
    /// Source reads clamp `max_bytes` to 1 byte through 8 MiB and may return a
    /// truncation marker. Use [`Self::write_part_text`] for complete copy/export.
    pub async fn part_text(
        &self,
        session_id: i64,
        part: &agena_api::part::PartResource,
        max_bytes: usize,
    ) -> Result<String, ClientError> {
        let mut body = String::new();
        let limit = max_bytes.clamp(1, 8 * 1024 * 1024);
        if let Some(values) = part
            .content
            .get("resources")
            .and_then(serde_json::Value::as_array)
        {
            for value in values {
                let reference: agena_domain::ContentRef =
                    serde_json::from_value(value.clone()).map_err(ClientError::Decode)?;
                if reference.kind != agena_domain::ContentKind::Text {
                    continue;
                }
                let first = self
                    .read_content(session_id, reference.resource_id, None, 1)
                    .await?;
                let mut cursor = ContentCursor {
                    sequence: 0,
                    ..first.resource.cursor
                };
                loop {
                    let page = self
                        .read_content(
                            session_id,
                            reference.resource_id,
                            Some(cursor),
                            limit.saturating_sub(body.len()).clamp(1, 64 * 1024),
                        )
                        .await?;
                    if page.gap {
                        body.push_str("\n[Content retention gap]\n");
                    }
                    for chunk in page.chunks {
                        if let agena_domain::ContentPayload::Text { text } = chunk.payload {
                            let mut end = text.len().min(limit.saturating_sub(body.len()));
                            while !text.is_char_boundary(end) {
                                end -= 1;
                            }
                            body.push_str(&text[..end]);
                            if end < text.len() {
                                body.push_str("\n[Content projection truncated]\n");
                                return Ok(body);
                            }
                        }
                    }
                    if !page.has_more {
                        break;
                    }
                    if page.next_cursor == cursor || body.len() >= limit {
                        body.push_str("\n[Content projection truncated]\n");
                        return Ok(body);
                    }
                    cursor = page.next_cursor;
                }
            }
        }
        if let Some(text) = part.content.get("text").and_then(serde_json::Value::as_str) {
            body.push_str(text);
        }
        Ok(body)
    }

    fn content_url(
        &self,
        session_id: i64,
        id: ContentId,
        stream: bool,
        after: Option<ContentCursor>,
        max_bytes: usize,
    ) -> url::Url {
        let suffix = if stream { "/stream" } else { "" };
        let mut url = self.endpoint(&format!(
            "/api/v1/sessions/{session_id}/content/{id}{suffix}"
        ));
        let mut query = url.query_pairs_mut();
        query.append_pair("max_bytes", &max_bytes.clamp(1, 1024 * 1024).to_string());
        if let Some(cursor) = after {
            query.append_pair("epoch", &cursor.epoch.to_string());
            query.append_pair("after", &cursor.sequence.to_string());
        }
        drop(query);
        url
    }

    pub async fn read_content(
        &self,
        session_id: i64,
        id: ContentId,
        after: Option<ContentCursor>,
        max_bytes: usize,
    ) -> Result<ContentPage, ClientError> {
        let response = self
            .send_request(
                reqwest::Method::GET,
                self.content_url(session_id, id, false, after, max_bytes),
                None,
                None,
            )
            .await?;
        self.parse_json(response).await
    }

    pub async fn read_content_text(
        &self,
        params: agena_api::content::ReadContentTextParams,
    ) -> Result<agena_domain::ContentTextPage, ClientError> {
        let mut url = self.endpoint(&format!(
            "/api/v1/sessions/{}/content/{}/text",
            params.session_id, params.resource_id
        ));
        {
            let mut query = url.query_pairs_mut();
            query.append_pair("max_bytes", &params.max_bytes.to_string());
            if let Some(position) = params.position {
                query.append_pair("epoch", &position.after.epoch.to_string());
                query.append_pair("after", &position.after.sequence.to_string());
                query.append_pair("offset", &position.offset.to_string());
            }
        }
        let response = self
            .send_request(reqwest::Method::GET, url, None, None)
            .await?;
        self.parse_json(response).await
    }

    /// Establish before bootstrap; subsequent pages advance the returned
    /// cursor. Dropping this handle cancels its connection immediately.
    pub async fn stream_content(
        &self,
        session_id: i64,
        id: ContentId,
        after: Option<ContentCursor>,
        max_bytes: usize,
    ) -> Result<ContentSubscription, ClientError> {
        let response = tokio::time::timeout(
            Duration::from_secs(15),
            self.send_request(
                reqwest::Method::GET,
                self.content_url(session_id, id, true, after, max_bytes),
                None,
                Some("text/event-stream"),
            ),
        )
        .await
        .map_err(|_| ClientError::Transport("content handshake timed out".into()))??;
        let status = response.status();
        if !status.is_success() {
            let body = read_response_text_bounded(
                response,
                MAX_ERROR_RESPONSE_BYTES,
                "content stream error",
            )
            .await?;
            if let Ok(error) = serde_json::from_str::<agena_api::error::ApiError>(&body) {
                return Err(ClientError::Api(error));
            }
            return Err(ClientError::Transport(format!(
                "content stream failed ({status})"
            )));
        }
        let (tx, rx) = mpsc::channel(8);
        let reader = StreamReader::new(response.bytes_stream().map_err(std::io::Error::other));
        let mut frames = FramedRead::new(
            reader,
            SseDecoder::<String>::with_max_size(MAX_SSE_EVENT_BYTES),
        );
        let task = tokio::spawn(async move {
            loop {
                let frame = tokio::select! {
                    _ = tx.closed() => return,
                    frame = tokio::time::timeout(Duration::from_secs(60), frames.next()) => frame,
                };
                let event = match frame {
                    Ok(Some(Ok(SseFrame::Event(event)))) => event,
                    Ok(Some(Ok(SseFrame::Comment(_) | SseFrame::Retry(_)))) => continue,
                    Ok(None) => return,
                    Ok(Some(Err(error))) => {
                        let _ = tx
                            .send(Err(ClientError::Protocol(format!(
                                "invalid content event: {error}"
                            ))))
                            .await;
                        return;
                    }
                    Err(_) => {
                        let _ = tx
                            .send(Err(ClientError::Transport("content stream stalled".into())))
                            .await;
                        return;
                    }
                };
                let page = match event.name.as_ref() {
                    "content" => serde_json::from_str(&event.data).map_err(ClientError::Decode),
                    "content_error" => {
                        let error = serde_json::from_str(&event.data)
                            .map(ClientError::Api)
                            .unwrap_or_else(ClientError::Decode);
                        let _ = tx.send(Err(error)).await;
                        return;
                    }
                    _ => continue,
                };
                let invalid = page.is_err();
                if tx.send(page).await.is_err() || invalid {
                    return;
                }
            }
        });
        Ok(ContentSubscription { rx, task })
    }
}
