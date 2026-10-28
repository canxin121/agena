//! Typed, cancellable live inline UI snapshots. Authentication uses the same
//! client transport as every other session resource.
use super::*;
use agena_domain::InlineSnapshot;

pub struct InlineUiSubscription {
    rx: mpsc::Receiver<Result<InlineSnapshot, ClientError>>,
    task: JoinHandle<()>,
}
impl InlineUiSubscription {
    pub async fn recv(&mut self) -> Option<Result<InlineSnapshot, ClientError>> {
        self.rx.recv().await
    }
}
impl Drop for InlineUiSubscription {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl AgenaClient {
    pub async fn read_inline_ui(
        &self,
        session: i64,
        reference: &str,
    ) -> Result<InlineSnapshot, ClientError> {
        if agena_domain::inline_reference_tool_id(reference).is_none() {
            return Err(ClientError::Protocol("invalid inline UI reference".into()));
        }
        let snapshot: InlineSnapshot = self
            .get_json(&format!(
                "/api/v1/sessions/{session}/inline-ui/{reference}/live"
            ))
            .await?;
        if snapshot.reference != reference {
            return Err(ClientError::Protocol("inline UI reference mismatch".into()));
        }
        snapshot.validate().map_err(ClientError::Protocol)?;
        Ok(snapshot)
    }
    pub async fn stream_inline_ui(
        &self,
        session: i64,
        reference: &str,
    ) -> Result<InlineUiSubscription, ClientError> {
        if agena_domain::inline_reference_tool_id(reference).is_none() {
            return Err(ClientError::Protocol("invalid inline UI reference".into()));
        }
        let response = tokio::time::timeout(
            Duration::from_secs(15),
            self.send_request(
                reqwest::Method::GET,
                self.endpoint(&format!(
                    "/api/v1/sessions/{session}/inline-ui/{reference}/stream"
                )),
                None,
                Some("text/event-stream"),
            ),
        )
        .await
        .map_err(|_| ClientError::Transport("inline UI handshake timed out".into()))??;
        let status = response.status();
        if !status.is_success() {
            let body =
                read_response_text_bounded(response, MAX_ERROR_RESPONSE_BYTES, "inline UI error")
                    .await?;
            if let Ok(error) = serde_json::from_str::<agena_api::error::ApiError>(&body) {
                return Err(ClientError::Api(error));
            }
            return Err(ClientError::Transport(format!(
                "inline UI stream failed ({status})"
            )));
        }
        let (tx, rx) = mpsc::channel(4);
        let reference = reference.to_owned();
        let reader = StreamReader::new(response.bytes_stream().map_err(std::io::Error::other));
        let mut frames = FramedRead::new(reader, SseDecoder::<String>::with_max_size(256 * 1024));
        let task = tokio::spawn(async move {
            let mut revision = 0;
            loop {
                let event = match tokio::time::timeout(Duration::from_secs(45), frames.next()).await
                {
                    Ok(Some(Ok(SseFrame::Event(event)))) => event,
                    Ok(Some(Ok(SseFrame::Comment(_) | SseFrame::Retry(_)))) => continue,
                    Ok(None) => return,
                    Ok(Some(Err(error))) => {
                        let _ = tx
                            .send(Err(ClientError::Protocol(format!(
                                "invalid inline UI event: {error}"
                            ))))
                            .await;
                        return;
                    }
                    Err(_) => {
                        let _ = tx
                            .send(Err(ClientError::Transport(
                                "inline UI stream stalled".into(),
                            )))
                            .await;
                        return;
                    }
                };
                if event.name.as_ref() == "inline_ui_error" {
                    let error = serde_json::from_str(&event.data)
                        .map(ClientError::Api)
                        .unwrap_or_else(ClientError::Decode);
                    let _ = tx.send(Err(error)).await;
                    return;
                }
                if event.name.as_ref() != "inline_ui" {
                    continue;
                }
                let snapshot = serde_json::from_str::<InlineSnapshot>(&event.data)
                    .map_err(ClientError::Decode)
                    .and_then(|snapshot| {
                        if snapshot.reference != reference {
                            return Err(ClientError::Protocol(
                                "inline UI reference mismatch".into(),
                            ));
                        }
                        snapshot.validate().map_err(ClientError::Protocol)?;
                        Ok(snapshot)
                    });
                match snapshot {
                    Ok(snapshot) if snapshot.revision > revision => {
                        revision = snapshot.revision;
                        if tx.send(Ok(snapshot)).await.is_err() {
                            return;
                        }
                    }
                    Ok(_) => {}
                    Err(error) => {
                        let _ = tx.send(Err(error)).await;
                        return;
                    }
                }
            }
        });
        Ok(InlineUiSubscription { rx, task })
    }
}
