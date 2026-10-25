//! Event-triggered title maintenance. No naming obligations enter the main turn.
use super::*;
use futures_util::FutureExt;
use serde::Deserialize;

const PROMPT: &str = include_str!("title_prompt.txt");
const CONTEXT_BYTES: usize = 16 * 1024;

#[derive(Default)]
pub(super) struct TitleWork {
    running: bool,
    pending: bool,
}

struct Worker {
    dispatch: Arc<SessionDispatch>,
    active: bool,
}
impl Drop for Worker {
    fn drop(&mut self) {
        if self.active {
            self.dispatch
                .title
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .running = false;
        }
    }
}

#[derive(Debug, PartialEq, Eq, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum Decision {
    Keep {},
    Rename { title: String },
}

impl SessionManager {
    pub(super) fn schedule_title(&self, session: SessionId) {
        if self.shutdown.is_cancelled() || !self.state.load().config.auto_title.enabled {
            return;
        }
        let dispatch = self.session_dispatch(session);
        let mut work = dispatch.title.lock().unwrap_or_else(|p| p.into_inner());
        work.pending = true;
        if work.running {
            return;
        }
        work.running = true;
        drop(work);
        let manager = self.clone();
        tokio::spawn(async move {
            let mut worker = Worker {
                dispatch,
                active: true,
            };
            let _permit = tokio::select! {
                biased;
                _ = manager.shutdown.cancelled() => return,
                permit = manager.auxiliary_tasks.clone().acquire_owned() => match permit {
                    Ok(permit) => permit,
                    Err(_) => return,
                },
            };
            loop {
                {
                    let mut work = worker
                        .dispatch
                        .title
                        .lock()
                        .unwrap_or_else(|p| p.into_inner());
                    if !work.pending || manager.shutdown.is_cancelled() {
                        // Clear while holding the same lock used by scheduling.
                        // Dropping the worker here cannot race a new worker.
                        work.running = false;
                        worker.active = false;
                        break;
                    }
                    work.pending = false;
                }
                match std::panic::AssertUnwindSafe(manager.update_auto_title(session))
                    .catch_unwind()
                    .await
                {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => {
                        tracing::debug!(session_id=session.get(), %error, "automatic title deferred")
                    }
                    Err(_) => {
                        tracing::warn!(
                            session_id = session.get(),
                            "automatic title worker panicked"
                        );
                        break;
                    }
                }
            }
        });
    }

    async fn update_auto_title(&self, session: SessionId) -> Result<(), AppError> {
        let state = self.state.load_full();
        if !state.config.auto_title.enabled {
            return Ok(());
        }
        let control = self
            .kernel()
            .reader
            .control_snapshot(session, 1)
            .await
            .map_err(kernel_error)?;
        let title = &control.configuration.title_state;
        let count = control.summary.user_message_count;
        if control.configuration.relation_kind.is_subagent()
            || !due(title, count, Utc::now().timestamp_millis(), &state.config.auto_title)
            // Only the initial title runs alongside the first response. Later
            // checks use settled conversation context, including queued inputs.
            || ((title.source != SessionTitleSource::Default || count > 1)
                && (control.summary.running_executions > 0 || control.summary.queued_executions > 0))
        {
            return Ok(());
        }
        let metadata = control.metadata.clone();
        let initial = title.source == SessionTitleSource::Default;
        drop(control);
        let first = self
            .kernel()
            .reader
            .history_after(session, None, 1)
            .await
            .map_err(kernel_error)?;
        let mut last_user = first
            .entries
            .first()
            .filter(|entry| matches!(entry.payload, EntryPayload::User { .. }))
            .map(|entry| (entry.id, entry.revision));
        let initial_prompt = first
            .entries
            .first()
            .and_then(|entry| match &entry.payload {
                EntryPayload::User { document } => Some(document_text(document, 2048)),
                _ => None,
            });
        drop(first);
        let page = self
            .kernel()
            .reader
            .message_history(session, None, 8, 8)
            .await
            .map_err(kernel_error)?;
        if let Some(entry) = page
            .entries
            .iter()
            .filter(|entry| matches!(entry.payload, EntryPayload::User { .. }))
            .max_by_key(|entry| entry.id)
        {
            last_user = Some((entry.id, entry.revision));
        }
        let mut recent = Vec::new();
        let mut replies = std::collections::HashSet::new();
        let mut remaining = CONTEXT_BYTES;
        // Select the latest text even when a response contains many tool calls.
        for entry in page.entries.iter().rev() {
            if remaining == 0 {
                break;
            }
            let (role, text) = match &entry.payload {
                EntryPayload::User { document } => {
                    ("user", document_text(document, remaining.min(2048)))
                }
                EntryPayload::Text { content_id } => {
                    if !replies.insert(entry.message_id) {
                        continue;
                    }
                    let range = self
                        .kernel()
                        .reader
                        .text_range(session, *content_id, None, remaining.min(2048))
                        .await
                        .map_err(kernel_error)?;
                    (
                        "assistant",
                        String::from_utf8_lossy(&range.bytes).into_owned(),
                    )
                }
                EntryPayload::Compaction { summary, .. } => {
                    let range = self
                        .kernel()
                        .reader
                        .text_range(session, *summary, None, remaining.min(2048))
                        .await
                        .map_err(kernel_error)?;
                    (
                        "checkpoint",
                        String::from_utf8_lossy(&range.bytes).into_owned(),
                    )
                }
                _ => continue,
            };
            if text.trim().is_empty() {
                continue;
            }
            remaining = remaining.saturating_sub(text.len());
            recent.push(serde_json::json!({"role":role,"text":text}));
        }
        recent.reverse();
        drop(page);
        let Some((last_user_message, last_user_revision)) = last_user else {
            return Ok(());
        };
        if recent.is_empty()
            && initial_prompt
                .as_deref()
                .is_none_or(|text| text.trim().is_empty())
        {
            return Ok(());
        }
        let prompt = serde_json::json!({
            "current_title":metadata.title,
            "initial_naming":initial,
            "first_user_message":initial_prompt,
            "recent_conversation":recent,
        })
        .to_string();
        let response = self
            .complete_auxiliary_with_state(
                &state,
                metadata.selection.as_ref(),
                PROMPT.into(),
                prompt,
                2048,
            )
            .await?;
        let decision = parse_decision(&response.text)?;
        // A reload may disable naming or replace its model while this request
        // is in flight. Let the next event use the newly published configuration.
        if self.shutdown.is_cancelled() || !Arc::ptr_eq(&state, &self.state.load_full()) {
            return Ok(());
        }
        let title = match decision {
            Decision::Keep {} if initial => {
                return Err(AppError::Internal("title model kept a placeholder".into()));
            }
            Decision::Keep {} => None,
            Decision::Rename { title } => Some(title),
        };
        let request = RequestId::new();
        let change = MetadataChange {
            automatic_title: Some(AutomaticTitleUpdate {
                expected_metadata_version: metadata.metadata_version,
                expected_membership_generation: metadata.membership_generation,
                user_message_count: count,
                last_user_message,
                last_user_revision,
                title,
            }),
            ..Default::default()
        };
        match persist(|| {
            self.kernel()
                .sessions
                .update_metadata(request, session, change.clone())
        })
        .await
        {
            Ok(_) | Err(KernelError::Conflict(_)) | Err(KernelError::NotFound(_)) => Ok(()),
            Err(error) => Err(kernel_error(error)),
        }
    }
}

fn due(
    state: &SessionTitleState,
    messages: u64,
    now: i64,
    config: &agena_runtime_config::SessionAutoTitleConfig,
) -> bool {
    if !config.enabled || messages == 0 || state.source == SessionTitleSource::Manual {
        return false;
    }
    state.source == SessionTitleSource::Default
        || (!state.calibrated && messages >= 3)
        || (state.calibrated
            && messages.saturating_sub(state.checked_user_messages) >= config.message_interval
            && u64::try_from(now.saturating_sub(state.checked_at_ms)).unwrap_or(0)
                >= config.cooldown_seconds.saturating_mul(1000))
}

fn document_text(document: &ComposerDocument, max: usize) -> String {
    let mut text = String::new();
    for node in &document.0 {
        if let ComposerNode::Text { text: part } = node {
            if text.len() >= max {
                break;
            }
            let mut end = part.len().min(max - text.len());
            while !part.is_char_boundary(end) {
                end -= 1;
            }
            text.push_str(&part[..end]);
            if text.len() < max {
                text.push('\n');
            }
        }
    }
    text
}

fn parse_decision(text: &str) -> Result<Decision, AppError> {
    let text = text.trim();
    let text = text
        .strip_prefix("```json")
        .or_else(|| text.strip_prefix("```"))
        .and_then(|text| text.strip_suffix("```"))
        .map(str::trim)
        .unwrap_or(text);
    let mut decision: Decision = serde_json::from_str(text)
        .map_err(|_| AppError::Internal("title model returned invalid JSON".into()))?;
    if let Decision::Rename { title } = &mut decision {
        *title = title.trim().to_owned();
        if title.is_empty()
            || title.chars().count() > 80
            || title.chars().any(|c| {
                c.is_control()
                    || matches!(c, '\u{061c}' | '\u{200b}'..='\u{200f}' | '\u{2028}'..='\u{202e}' | '\u{2060}'..='\u{206f}' | '\u{feff}')
            })
        {
            return Err(AppError::Internal(
                "title model returned an invalid title".into(),
            ));
        }
    }
    Ok(decision)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_calibration_and_periodic_checks_have_distinct_schedules() {
        let config = agena_runtime_config::SessionAutoTitleConfig::default();
        let mut state = SessionTitleState::automatic();
        assert!(!due(&state, 0, 1000, &config));
        assert!(due(&state, 1, 1000, &config));
        state.source = SessionTitleSource::Automatic;
        state.checked_user_messages = 1;
        state.checked_at_ms = 1000;
        assert!(!due(&state, 2, 5000, &config));
        assert!(due(&state, 3, 5000, &config));
        state.calibrated = true;
        state.checked_user_messages = 3;
        assert!(!due(&state, 7, 500_000, &config));
        assert!(!due(&state, 8, 300_999, &config));
        assert!(due(&state, 8, 301_000, &config));
        assert!(!due(&state, 8, 0, &config));
        state.source = SessionTitleSource::Manual;
        assert!(!due(&state, 100, 900_000, &config));
    }

    #[test]
    fn disabled_naming_and_zero_cooldown_respect_policy() {
        let mut config = agena_runtime_config::SessionAutoTitleConfig {
            enabled: false,
            ..Default::default()
        };
        assert!(!due(&SessionTitleState::automatic(), 1, 1000, &config));
        config.enabled = true;
        config.cooldown_seconds = 0;
        let state = SessionTitleState {
            source: SessionTitleSource::Automatic,
            checked_user_messages: 3,
            checked_at_ms: 1000,
            calibrated: true,
        };
        assert!(due(&state, 8, 1000, &config));
        config.message_interval = 1;
        let mut uncalibrated = state;
        uncalibrated.checked_user_messages = 1;
        uncalibrated.calibrated = false;
        assert!(!due(&uncalibrated, 2, 1000, &config));
        assert!(due(&uncalibrated, 3, 1000, &config));
    }

    #[test]
    fn title_output_rejects_bad_json_empty_multiline_and_invisible_names() {
        for text in [
            "not JSON",
            r#"{"action":"rename","title":" "}"#,
            r#"{"action":"rename","title":"a\nb"}"#,
            r#"{"action":"rename","title":"a\u202eb"}"#,
            r#"{"action":"rename","title":"a\u2028b"}"#,
            r#"{"action":"rename","title":"a\u2060b"}"#,
            r#"{"action":"keep","extra":true}"#,
        ] {
            assert!(parse_decision(text).is_err(), "accepted {text}");
        }
        let long = serde_json::json!({"action":"rename","title":"名".repeat(81)}).to_string();
        assert!(parse_decision(&long).is_err());
        assert_eq!(
            parse_decision("```json\n{\"action\":\"keep\"}\n```").unwrap(),
            Decision::Keep {}
        );
        assert_eq!(
            parse_decision(r#"{"action":"rename","title":" OAuth 回调配置 "}"#).unwrap(),
            Decision::Rename {
                title: "OAuth 回调配置".into()
            }
        );
    }

    #[test]
    fn user_context_is_bounded_without_splitting_utf8() {
        let document = ComposerDocument(vec![ComposerNode::Text {
            text: "会话".repeat(10_000),
        }]);
        let text = document_text(&document, 8);
        assert!(text.len() <= 8);
        assert_eq!(text.trim(), "会话");
    }
}
