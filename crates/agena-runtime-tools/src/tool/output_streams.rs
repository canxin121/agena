//! Typed, kernel-owned capture for tool and independent activity output.
//!
//! A terminal has separate raw-log and screen sources. Streaming plugin tools
//! have one ordered mixed source, so a screen patch never uses a cursor
//! advanced by an unrelated log event.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU8, Ordering},
};

use agena_domain::{ContentInput, ContentKind, ContentRef, ProcessStatus, ProcessSummary};
use agena_storage::kernel::{
    ActivityHandle, BackgroundActivityKind, BackgroundActivityOutcome, BackgroundActivityStart,
    DocumentOutput, EventCursor, KernelError, LogOutput, MixedOutput, RequestId, TerminalOutput,
    TextOutput, ToolHandle,
};

#[derive(Clone, Default)]
pub struct ToolOutputStreams {
    text: Option<TextOutput>,
    log: Option<LogOutput>,
    document: Option<DocumentOutput>,
    terminal: Option<TerminalOutput>,
    mixed: Option<MixedOutput>,
    activity: Option<ActivityHandle>,
    // A failed launch wait can end only an intent not yet handed to a registry.
    // This small gate prevents that failure racing a late native worker launch.
    external_launch: Arc<AtomicU8>,
    capture_error: Arc<Mutex<Option<String>>>,
}

impl std::fmt::Debug for ToolOutputStreams {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolOutputStreams")
            .field("sources", &self.references())
            .field("activity", &self.activity.as_ref().map(ActivityHandle::id))
            .finish()
    }
}

impl PartialEq for ToolOutputStreams {
    fn eq(&self, other: &Self) -> bool {
        self.references() == other.references()
            && self.activity.as_ref().map(ActivityHandle::id)
                == other.activity.as_ref().map(ActivityHandle::id)
    }
}
impl Eq for ToolOutputStreams {}

fn request(owner: impl std::fmt::Display, label: &str) -> RequestId {
    let digest = blake3::hash(format!("agena/tool-output/{owner}/{label}").as_bytes());
    let mut bytes = [0; 16];
    bytes.copy_from_slice(&digest.as_bytes()[..16]);
    RequestId::from(uuid::Uuid::from_bytes(bytes))
}

impl ToolOutputStreams {
    /// Opens all required source identities before any external effect. Fixed
    /// request identities make retries safe after a partial open or lost reply.
    pub async fn open(tool: &ToolHandle, kind: ContentKind) -> Result<Self, KernelError> {
        let mut output = Self::default();
        let id = tool.id().get();
        if kind == ContentKind::Text {
            output.text = Some(tool.begin_text(request(id, "text")).await?.value);
        }
        if matches!(kind, ContentKind::Log | ContentKind::Terminal) {
            output.log = Some(tool.begin_log(request(id, "log")).await?.value);
        }
        if kind == ContentKind::Structured {
            output.document = Some(tool.begin_document(request(id, "document")).await?.value);
        }
        if kind == ContentKind::Document {
            output.mixed = Some(tool.begin_mixed(request(id, "mixed")).await?.value);
        }
        if kind == ContentKind::Terminal {
            output.terminal = Some(tool.begin_terminal(request(id, "terminal")).await?.value);
        }
        Ok(output)
    }

    /// The launch intent and activity-owned sources commit before the caller
    /// starts a process, PTY, or WebSocket. Parent completion cannot seal them.
    pub async fn begin_activity(
        tool: &ToolHandle,
        activity_kind: BackgroundActivityKind,
        label: String,
        kind: ContentKind,
    ) -> Result<Self, KernelError> {
        let activity = tool
            .begin_activity(
                request(tool.id().get(), "activity"),
                BackgroundActivityStart {
                    kind: activity_kind,
                    label,
                },
            )
            .await?
            .value;
        let mut output = Self {
            activity: Some(activity.clone()),
            ..Self::default()
        };
        let id = activity.id();
        let opened = async {
            if kind == ContentKind::Text {
                output.text = Some(activity.begin_text(request(id, "text")).await?.value);
            }
            if matches!(kind, ContentKind::Log | ContentKind::Terminal) {
                output.log = Some(activity.begin_log(request(id, "log")).await?.value);
            }
            if kind == ContentKind::Structured {
                output.document = Some(
                    activity
                        .begin_document(request(id, "document"))
                        .await?
                        .value,
                );
            }
            if kind == ContentKind::Document {
                output.mixed = Some(activity.begin_mixed(request(id, "mixed")).await?.value);
            }
            if kind == ContentKind::Terminal {
                output.terminal = Some(
                    activity
                        .begin_terminal(request(id, "terminal"))
                        .await?
                        .value,
                );
            }
            Ok::<(), KernelError>(())
        }
        .await;
        if let Err(error) = opened {
            let pending = output.clone();
            let diagnostic =
                format!("background capture setup failed before external launch: {error}");
            tokio::spawn(async move {
                pending.fail_activity(diagnostic).await;
            });
            return Err(error);
        }
        Ok(output)
    }

    pub fn references(&self) -> Vec<ContentRef> {
        let mut sources = Vec::with_capacity(4);
        if let Some(source) = &self.text {
            sources.push(ContentRef {
                resource_id: source.id(),
                kind: ContentKind::Text,
            });
        }
        if let Some(source) = &self.log {
            sources.push(ContentRef {
                resource_id: source.id(),
                kind: ContentKind::Log,
            });
        }
        if let Some(source) = &self.document {
            sources.push(ContentRef {
                resource_id: source.id(),
                kind: ContentKind::Structured,
            });
        }
        if let Some(source) = &self.terminal {
            sources.push(ContentRef {
                resource_id: source.id(),
                kind: ContentKind::Terminal,
            });
        }
        if let Some(source) = &self.mixed {
            sources.push(ContentRef {
                resource_id: source.id(),
                kind: ContentKind::Document,
            });
        }
        sources
    }

    pub fn reference(&self) -> ContentRef {
        if let Some(source) = &self.terminal {
            return ContentRef {
                resource_id: source.id(),
                kind: ContentKind::Terminal,
            };
        }
        self.references()
            .into_iter()
            .next()
            .expect("output contains a typed source")
    }

    pub fn capture_diagnostic(&self) -> Option<String> {
        self.capture_error
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn register_external_launch(&self) -> Result<(), KernelError> {
        match self
            .external_launch
            .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
        {
            Ok(_) => Ok(()),
            Err(_) => Err(KernelError::Conflict(
                "background launch intent was abandoned before external launch".into(),
            )),
        }
    }

    pub fn record_loss(&self, kind: ContentKind, bytes: usize, reason: &str) {
        let mut end = reason.len().min(512);
        while !reason.is_char_boundary(end) {
            end -= 1;
        }
        let reason = &reason[..end];
        let result = if let Some(source) = &self.mixed {
            Some(source.record_loss(bytes as u64, reason))
        } else {
            match kind {
                ContentKind::Text => self
                    .text
                    .as_ref()
                    .map(|source| source.record_loss(bytes as u64, reason)),
                ContentKind::Log => self
                    .log
                    .as_ref()
                    .map(|source| source.record_loss(bytes as u64, reason)),
                ContentKind::Structured | ContentKind::Document => self
                    .document
                    .as_ref()
                    .map(|source| source.record_loss(bytes as u64, reason)),
                ContentKind::Terminal => self
                    .terminal
                    .as_ref()
                    .map(|source| source.record_loss(bytes as u64, reason)),
            }
        };
        let mut diagnostic = self
            .capture_error
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if diagnostic.is_none() {
            *diagnostic = Some(match result {
                Some(Ok(())) => reason.to_owned(),
                Some(Err(error)) => format!("{reason}; capture-loss accounting failed: {error}"),
                None => format!("{reason}; no {kind:?} capture source"),
            });
        }
    }

    pub fn try_log(
        &self,
        channel: agena_domain::CommandOutputStream,
        text: &str,
    ) -> Result<EventCursor, KernelError> {
        self.log
            .as_ref()
            .ok_or_else(|| KernelError::Invalid("tool has no log source".into()))?
            .try_append(channel, text)
    }

    pub fn try_snapshot(
        &self,
        screen: agena_domain::TerminalSnapshot,
    ) -> Result<EventCursor, KernelError> {
        self.terminal
            .as_ref()
            .ok_or_else(|| KernelError::Invalid("tool has no terminal source".into()))?
            .try_snapshot(screen)
    }

    pub fn try_patch(
        &self,
        base: EventCursor,
        screen: agena_domain::TerminalSnapshot,
        rows: Vec<u16>,
    ) -> Result<EventCursor, KernelError> {
        self.terminal
            .as_ref()
            .ok_or_else(|| KernelError::Invalid("tool has no terminal source".into()))?
            .try_patch(base, screen, rows)
    }

    /// Plugin SDK input is routed into the matching source without synthesizing
    /// legacy cursors. Terminal driver patches use try_patch's native cursor.
    pub async fn append(&self, input: ContentInput) -> Result<(), KernelError> {
        if let Some(source) = &self.mixed {
            source.append(input).await?;
            return Ok(());
        }
        match input {
            ContentInput::Text { text } => {
                self.text
                    .as_ref()
                    .ok_or_else(|| KernelError::Invalid("tool has no text source".into()))?
                    .append(&text)
                    .await?;
            }
            ContentInput::Log { stream, text } => {
                self.log
                    .as_ref()
                    .ok_or_else(|| KernelError::Invalid("tool has no log source".into()))?
                    .append(stream, &text)
                    .await?;
            }
            ContentInput::Structured { event } => {
                self.document
                    .as_ref()
                    .ok_or_else(|| KernelError::Invalid("tool has no document source".into()))?
                    .apply(event)
                    .await?;
            }
            ContentInput::Terminal { screen } => {
                self.terminal
                    .as_ref()
                    .ok_or_else(|| KernelError::Invalid("tool has no terminal source".into()))?
                    .snapshot(screen)
                    .await?;
            }
            ContentInput::TerminalPatch { .. } => {
                return Err(KernelError::Invalid(
                    "terminal patches require the native terminal capability cursor".into(),
                ));
            }
        }
        Ok(())
    }

    /// The observed process result is saved repeatedly with one identity; the
    /// external process is never reissued due to a persistence failure.
    pub async fn finish_activity(&self, summary: &ProcessSummary) {
        let Some(activity) = &self.activity else {
            return;
        };
        let outcome = match summary.status {
            ProcessStatus::Running => return,
            ProcessStatus::Exited => BackgroundActivityOutcome::Succeeded {
                value: serde_json::json!({"process_id": summary.process_id, "exit_code": summary.exit_code, "completion_reason": summary.completion_reason}),
            },
            ProcessStatus::Stopped => BackgroundActivityOutcome::Stopped,
            ProcessStatus::Failed | ProcessStatus::TimedOut => BackgroundActivityOutcome::Failed {
                diagnostic: format!(
                    "process {} ended with exit code {:?}: {}",
                    summary.process_id,
                    summary.exit_code,
                    summary
                        .completion_reason
                        .as_deref()
                        .unwrap_or("background process failed")
                ),
            },
        };
        self.persist_activity_outcome(activity, outcome).await;
    }

    pub(crate) async fn fail_activity(&self, diagnostic: String) {
        if let Some(activity) = &self.activity
            && self
                .external_launch
                .compare_exchange(0, 2, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
        {
            self.persist_activity_outcome(
                activity,
                BackgroundActivityOutcome::Interrupted { diagnostic },
            )
            .await;
        }
    }

    async fn persist_activity_outcome(
        &self,
        activity: &ActivityHandle,
        outcome: BackgroundActivityOutcome,
    ) {
        let request = request(activity.id(), "end");
        let mut backoff = std::time::Duration::from_millis(20);
        loop {
            match activity.end(request, outcome.clone()).await {
                Ok(_) => return,
                Err(
                    KernelError::Busy | KernelError::Storage(_) | KernelError::CommitUnknown { .. },
                ) => {
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(std::time::Duration::from_secs(2));
                }
                Err(error) => {
                    tracing::error!(%error, activity = %activity.id(), "background activity outcome could not be committed");
                    return;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel_fixture::{complete, fixture};
    use agena_domain::{CommandOutputStream, DocumentMutation, ViewBlock};
    use agena_storage::kernel::{CaptureState, EventPayload};

    fn screen(text: &str) -> agena_domain::TerminalSnapshot {
        agena_domain::TerminalSnapshot {
            rows: 2,
            cols: 80,
            cursor_row: 0,
            cursor_col: 0,
            cursor_visible: true,
            alternate_screen: false,
            bracketed_paste: false,
            application_cursor: false,
            cells: vec![agena_domain::TerminalCellRun {
                row: 0,
                col: 0,
                text: text.into(),
                attributes: Default::default(),
            }],
        }
    }

    #[tokio::test]
    async fn mixed_plugin_payloads_retain_type_and_original_interleave() {
        let fixture = fixture().await;
        let output = ToolOutputStreams::open(&fixture.tool, ContentKind::Document)
            .await
            .unwrap();
        let inputs = [
            ContentInput::Text {
                text: "  first\r\n🙂".into(),
            },
            ContentInput::Log {
                stream: CommandOutputStream::Stderr,
                text: "\u{1b}[31mwarning".into(),
            },
            ContentInput::Structured {
                event: DocumentMutation::Insert {
                    after: None,
                    block: ViewBlock::Text {
                        id: Some("body".into()),
                        text: "document".into(),
                    },
                },
            },
            ContentInput::Text {
                text: "last\n\n".into(),
            },
        ];
        for input in &inputs {
            output.append(input.clone()).await.unwrap();
        }
        complete(&fixture.tool).await;
        let id = output.reference().resource_id;
        let page = fixture
            .runtime
            .ports
            .reader
            .event_page(fixture.session, id, None, 16)
            .await
            .unwrap();
        let observed = page
            .records
            .iter()
            .map(|record| match &record.payload {
                EventPayload::Mixed { input, .. } => (**input).clone(),
                other => panic!("unexpected mixed source event {other:?}"),
            })
            .collect::<Vec<_>>();
        assert_eq!(observed, inputs);
        assert_eq!(page.stream.state, CaptureState::Complete);
        assert_eq!(output.references().len(), 1);
        drop(page);
        let snapshot = fixture
            .runtime
            .ports
            .reader
            .event_snapshot(fixture.session, id)
            .await
            .unwrap();
        assert_eq!(snapshot.document.as_ref().unwrap().blocks.len(), 1);
        drop(snapshot);
        fixture.runtime.ports.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn terminal_log_admission_does_not_advance_the_screen_patch_base() {
        let fixture = fixture().await;
        let output = ToolOutputStreams::open(&fixture.tool, ContentKind::Terminal)
            .await
            .unwrap();
        let base = output.try_snapshot(screen("before")).unwrap();
        output
            .try_log(CommandOutputStream::Stdout, "raw ANSI\r\n")
            .unwrap();
        let end = output.try_patch(base, screen("after"), vec![0]).unwrap();
        assert_eq!(end.sequence, base.sequence + 1);
        complete(&fixture.tool).await;
        let snapshot = fixture
            .runtime
            .ports
            .reader
            .event_snapshot(fixture.session, output.reference().resource_id)
            .await
            .unwrap();
        assert_eq!(snapshot.terminal.as_deref(), Some(&screen("after")));
        let refs = output.references();
        assert_eq!(
            refs.iter()
                .map(|reference| reference.kind)
                .collect::<Vec<_>>(),
            [ContentKind::Log, ContentKind::Terminal]
        );
        let raw = fixture
            .runtime
            .ports
            .reader
            .event_page(fixture.session, refs[0].resource_id, None, 8)
            .await
            .unwrap();
        assert!(
            matches!(&raw.records[0].payload, EventPayload::Log { text, .. } if text == "raw ANSI\r\n")
        );
        drop((snapshot, raw));
        fixture.runtime.ports.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn activity_output_survives_its_launch_tool_and_seals_with_its_own_outcome() {
        let fixture = fixture().await;
        let output = ToolOutputStreams::begin_activity(
            &fixture.tool,
            BackgroundActivityKind::Process,
            "independent process".into(),
            ContentKind::Log,
        )
        .await
        .unwrap();
        output.register_external_launch().unwrap();
        output
            .try_log(CommandOutputStream::Stdout, "before")
            .unwrap();
        complete(&fixture.tool).await;
        output
            .try_log(CommandOutputStream::Stderr, "after parent")
            .unwrap();
        let id = output.reference().resource_id;
        let watch = fixture
            .runtime
            .ports
            .reader
            .watch_content(fixture.session, id)
            .await
            .unwrap();
        assert_eq!(watch.head().unwrap().state, CaptureState::Open);
        output
            .activity
            .as_ref()
            .unwrap()
            .end(RequestId::new(), BackgroundActivityOutcome::Stopped)
            .await
            .unwrap();
        let page = fixture
            .runtime
            .ports
            .reader
            .event_page(fixture.session, id, None, 8)
            .await
            .unwrap();
        assert_eq!(page.records.len(), 2);
        assert_eq!(page.stream.state, CaptureState::Complete);
        assert!(output.try_log(CommandOutputStream::Stdout, "late").is_err());
        drop((page, watch));
        fixture.runtime.ports.shutdown().await.unwrap();
    }
}
