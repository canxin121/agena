//! Temporary, read-only questions with an execution independent of the parent.

use crate::{Application, ApplicationError};
use agena_api::resource::{BtwAnswer, BtwRequest};
use agena_domain::{
    ComposerDocument, ComposerNode, ConversationMode, ExecutionLifecycle, ExecutionOutcome,
    ExecutionPhase,
};
use std::{collections::BTreeMap, time::Duration};
use tokio::sync::mpsc;
const MAX_QUESTION_BYTES: usize = 16 * 1024;
const MAX_ANSWER_BYTES: usize = 256 * 1024;
static BTW_CAPACITY: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(4);

impl Application {
    /// The returned channel owns the question. Dropping it cancels only the
    /// temporary execution. No request is submitted to the parent session.
    pub async fn start_btw(
        &self,
        parent_id: i64,
        mut request: BtwRequest,
    ) -> Result<mpsc::Receiver<BtwAnswer>, ApplicationError> {
        request.question = request.question.trim().to_owned();
        if request.question.is_empty() || request.question.len() > MAX_QUESTION_BYTES {
            return Err(ApplicationError::bad_request(
                "A BTW question must contain 1–16384 bytes of text.",
            ));
        }
        let services = self.session_execution_services()?;
        if self.service().get_session(parent_id).await?.is_none() {
            return Err(ApplicationError::not_found(
                "The parent session was not found.",
            ));
        }
        let permit = BTW_CAPACITY.try_acquire().map_err(|_| {
            ApplicationError::service_unavailable(
                "Too many temporary questions are running. Please wait for one to finish.",
            )
        })?;
        let (tx, rx) = mpsc::channel(2);
        let application = self.clone();
        tokio::spawn(async move {
            let _permit = permit;
            let outcome = services
                .commands
                .fork_session(agena_runtime::SessionForkRequest {
                    session_id: parent_id,
                    at_message_id: None,
                    title: Some(format!(
                        "btw: {}",
                        request.question.chars().take(60).collect::<String>()
                    )),
                    conversation_mode: Some(ConversationMode::Btw),
                    expected_version: None,
                })
                .await;
            let child_id = match outcome {
                Ok(outcome) => outcome.session_id,
                Err(error) => {
                    let _ = tx
                        .send(BtwAnswer {
                            done: true,
                            error: Some(error.to_string()),
                            ..Default::default()
                        })
                        .await;
                    return;
                }
            };
            let result = application.run_btw(child_id, request, &tx).await;
            if let Err(error) = result {
                let _ = tx
                    .send(BtwAnswer {
                        done: true,
                        error: Some(error.to_string()),
                        ..Default::default()
                    })
                    .await;
            }
            // This owner outlives the HTTP stream, so disconnect, window close,
            // failure, and success all take the same cleanup path.
            application.dispose_btw(child_id).await;
        });
        Ok(rx)
    }

    async fn run_btw(
        &self,
        session_id: i64,
        request: BtwRequest,
        tx: &mpsc::Sender<BtwAnswer>,
    ) -> Result<(), ApplicationError> {
        if tx.is_closed() {
            return Ok(());
        }
        let document_text = request.question.clone();
        let document = ComposerDocument(vec![ComposerNode::Text {
            text: request.question,
        }]);
        let run =
            crate::session::session_user_run_request(self, session_id, request.options, document)
                .await?;
        let services = self.session_execution_services()?;
        services
            .commands
            .submit_user_run(run)
            .await
            .map_err(|error| ApplicationError::from_failure(error.failure))?;
        let question = document_text;
        if tx
            .send(BtwAnswer {
                question: Some(question.clone()),
                ..Default::default()
            })
            .await
            .is_err()
        {
            return Ok(());
        }
        let mut tick = tokio::time::interval(Duration::from_millis(100));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let timeout = tokio::time::sleep(Duration::from_secs(600));
        tokio::pin!(timeout);
        let mut last_text = String::new();
        let mut sources =
            BTreeMap::<agena_domain::ContentId, (agena_domain::ContentCursor, String, bool)>::new();
        let mut source_order = Vec::new();
        loop {
            tokio::select! {
                _ = tx.closed() => return Ok(()),
                _ = &mut timeout => return Err(ApplicationError::bad_request("The BTW question timed out. Please try a narrower question.")),
                _ = tick.tick() => {}
            }
            let lifecycle = services
                .execution_control
                .active_execution(session_id)
                .await;
            let entries = self
                .read_parts(agena_api::queries::ReadPartsParams {
                    session_id,
                    limit: Some(128),
                    ..Default::default()
                })
                .await?;
            let ids = entries
                .parts
                .iter()
                .filter(|part| {
                    part.origin_session_id == session_id
                        && part.role == "assistant"
                        && part.kind == "text"
                })
                .flat_map(|part| {
                    part.content
                        .get("resources")
                        .and_then(serde_json::Value::as_array)
                        .into_iter()
                        .flatten()
                })
                .filter_map(|value| {
                    serde_json::from_value::<agena_domain::ContentRef>(value.clone()).ok()
                })
                .filter(|source| source.kind == agena_domain::ContentKind::Text)
                .map(|source| source.resource_id)
                .collect::<Vec<_>>();
            let mut source_bytes = sources
                .values()
                .map(|(_, text, _)| text.len())
                .sum::<usize>();
            let mut source_oversized = false;
            for id in ids {
                if sources.get(&id).is_some_and(|(_, _, complete)| *complete) {
                    continue;
                }
                if let std::collections::btree_map::Entry::Vacant(entry) = sources.entry(id) {
                    source_order.push(id);
                    let descriptor = self.watch_content(session_id, id).await?.resource;
                    entry.insert((
                        agena_domain::ContentCursor {
                            sequence: 0,
                            ..descriptor.cursor
                        },
                        String::new(),
                        false,
                    ));
                }
                let source = sources.get_mut(&id).expect("registered answer source");
                loop {
                    let page = self
                        .read_content(session_id, id, Some(source.0), 64 * 1024)
                        .await?;
                    if page.gap {
                        return Err(ApplicationError::internal(
                            "The BTW answer content was not retained.",
                        ));
                    }
                    for chunk in page.chunks {
                        if let agena_domain::ContentPayload::Text { text } = chunk.payload {
                            source_bytes = source_bytes.saturating_add(text.len());
                            if source_bytes > MAX_ANSWER_BYTES {
                                source_oversized = true;
                                break;
                            }
                            source.1.push_str(&text);
                        }
                    }
                    source.0 = page.next_cursor;
                    source.2 =
                        page.resource.state != agena_domain::ContentState::Active && !page.has_more;
                    if !page.has_more || source_oversized {
                        break;
                    }
                }
                if source_oversized {
                    break;
                }
            }
            let text = source_order
                .iter()
                .filter_map(|id| sources.get(id))
                .map(|(_, text, _)| text.as_str())
                .collect::<Vec<_>>()
                .join("\n\n");
            let oversized = source_oversized || text.len() > MAX_ANSWER_BYTES;
            let (done, error) = if oversized {
                (
                    true,
                    Some(
                        "The BTW answer exceeded its size limit. Please ask a narrower question."
                            .to_owned(),
                    ),
                )
            } else {
                match lifecycle {
                    Some(ExecutionLifecycle::Active { phase: ExecutionPhase::AwaitingInteraction, .. }) => (true, Some("This query needs additional approval. Please continue it in /side or the main conversation.".to_owned())),
                    Some(ExecutionLifecycle::Active { .. }) => (false, None),
                    Some(ExecutionLifecycle::Terminal { outcome: ExecutionOutcome::Failed { failure }, .. }) => (true, Some(failure.user.fallback)),
                    Some(ExecutionLifecycle::Terminal { outcome: ExecutionOutcome::Cancelled, .. }) => (true, Some("The BTW question was cancelled.".to_owned())),
                    _ => (true, None),
                }
            };
            if done {
                let _ = tx
                    .send(BtwAnswer {
                        question: Some(question.clone()),
                        text,
                        done,
                        error,
                    })
                    .await;
                return Ok(());
            }
            if text != last_text {
                match tx.try_send(BtwAnswer {
                    question: Some(question.clone()),
                    text: text.clone(),
                    done: false,
                    error: None,
                }) {
                    Ok(()) => last_text = text,
                    Err(mpsc::error::TrySendError::Full(_)) => {}
                    Err(mpsc::error::TrySendError::Closed(_)) => return Ok(()),
                }
            }
        }
    }

    async fn dispose_btw(&self, session_id: i64) {
        let Ok(services) = self.session_execution_services() else {
            return;
        };
        if let Err(error) = self.cancel_run(session_id, None).await {
            tracing::warn!(session_id, %error, "failed to cancel temporary BTW execution");
        }
        // Cancellation may be acknowledged before a tool/provider has unwound.
        // Keep storage alive until the execution owner has stopped writing.
        let stopped = tokio::time::timeout(Duration::from_secs(30), async {
            while matches!(
                services
                    .execution_control
                    .active_execution(session_id)
                    .await,
                Some(ExecutionLifecycle::Active { .. })
            ) {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await
        .is_ok();
        if !stopped {
            tracing::warn!(
                session_id,
                "temporary BTW execution is still stopping; startup recovery will remove it"
            );
            return;
        }
        if let Err(error) = self.service().delete_session(session_id).await {
            tracing::warn!(session_id,%error,"failed to remove temporary BTW conversation");
        }
    }
}
