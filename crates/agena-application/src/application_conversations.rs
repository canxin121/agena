//! Temporary, read-only questions with an execution independent of the parent.

use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::Duration,
};

use agena_api::resource::{BtwAnswer, BtwRequest};
use agena_domain::{
    ComposerDocument, ComposerNode, ConversationMode, ExecutionLifecycle, ExecutionOutcome,
    ExecutionPhase,
};
use agena_storage::store::{PartRole, SessionChange};
use tokio::sync::mpsc;

use crate::{Application, ApplicationError};

const MAX_QUESTION_BYTES: usize = 16 * 1024;
const MAX_ANSWER_BYTES: usize = 256 * 1024;
static BTW_CAPACITY: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(4);

#[derive(Default)]
struct AnswerProjection {
    parts: BTreeMap<(i64, i64), String>,
    bytes: usize,
    oversized: bool,
}

impl AnswerProjection {
    fn apply(&mut self, change: SessionChange, session_id: i64) {
        match change {
            SessionChange::PartAdded { part, .. } | SessionChange::PartUpdated { part, .. }
                if part.origin_session_id == session_id
                    && part.role == PartRole::Assistant
                    && part.kind == "text"
                    && part.visibility.visible_to_user() =>
            {
                let key = (part.created_at_ms, part.part_id);
                let text = part
                    .content
                    .get("text")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default();
                let previous = self.parts.get(&key).map_or(0, String::len);
                let bytes = self
                    .bytes
                    .saturating_sub(previous)
                    .saturating_add(text.len());
                if bytes > MAX_ANSWER_BYTES {
                    self.oversized = true;
                    return;
                }
                self.bytes = bytes;
                self.parts.insert(key, text.to_owned());
            }
            SessionChange::PartRemoved { part_id, .. } => {
                self.parts.retain(|(_, id), _| *id != part_id);
                self.bytes = self.parts.values().map(String::len).sum();
            }
            _ => {}
        }
    }

    fn text(&self) -> String {
        self.parts
            .values()
            .filter(|text| !text.is_empty())
            .cloned()
            .collect::<Vec<_>>()
            .join("\n\n")
    }
}

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
        let store = self.session_store_facade()?;
        let projection = Arc::new(Mutex::new(AnswerProjection::default()));
        let observer = Arc::clone(&projection);
        let _subscription = store.subscribe(
            session_id,
            Arc::new(move |change| {
                observer
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .apply(change, session_id);
            }),
        );
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
        let mut tick = tokio::time::interval(Duration::from_millis(100));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let timeout = tokio::time::sleep(Duration::from_secs(600));
        tokio::pin!(timeout);
        let mut last_text = String::new();
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
            let (text, oversized) = {
                let projection = projection.lock().unwrap_or_else(|error| error.into_inner());
                (projection.text(), projection.oversized)
            };
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
                let _ = tx.send(BtwAnswer { text, done, error }).await;
                return Ok(());
            }
            if text != last_text {
                match tx.try_send(BtwAnswer {
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
        if let Ok(store) = self.session_store_facade()
            && let Err(error) = store.delete(session_id).await
        {
            tracing::warn!(session_id, %error, "failed to remove temporary BTW conversation");
        }
    }
}
