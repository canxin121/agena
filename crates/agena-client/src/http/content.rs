//! Cursor-based source reads. This protocol is identical for every consumer.

use super::*;
use agena_domain::{ContentCursor, ContentId, ContentPage};

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
    /// Resolve a bounded text projection without changing the canonical Part.
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
