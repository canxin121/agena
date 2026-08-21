//! Activity v2 runtime handler (design 07 §5/§6, 08 §3).
//!
//! New-subsystem skeleton that consumes a tool's [`ToolActivityEvent`] stream,
//! keeps the in-memory live view, and produces unified wire events
//! ([`ActivityLiveEvent`]) that TUI and Web consume identically. Persistence
//! wiring lands later; this module is deliberately independent of the legacy
//! checkpoint/projection machinery.

use agena_domain::{ActivityId, ActivityState, RawOutput, RenderDelta, ViewBlock};
use agena_tool::{ToolActivityEvent, ToolActivityResult};

/// Converged activity kinds (07 §4.1): the only nine live variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivityKind {
    Operation,
    Resource,
    SkillReference,
    Reasoning,
    TextArtifact,
    TextSegment,
    Interaction,
    Error,
    Notice,
}

/// v2 activity state node: the single durable shape (title/summary/facts).
/// Views (model/human) are projections of `raw_output`, never stored.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActivityStateNode {
    pub activity_id: ActivityId,
    pub kind: ActivityKind,
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub summary: String,
    pub state: ActivityState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_output: Option<RawOutput>,
}

/// Unified live wire event: one shape for TUI and Web (07 §5.2).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ActivityLiveEvent {
    DetailDelta {
        activity_id: ActivityId,
        #[serde(flatten)]
        delta: RenderDelta,
    },
    TitleChanged {
        activity_id: ActivityId,
        title: String,
    },
    SummaryChanged {
        activity_id: ActivityId,
        summary: String,
    },
    StateChanged {
        activity_id: ActivityId,
        state: ActivityState,
    },
    Upserted {
        node: Box<ActivityStateNode>,
    },
    Removed {
        activity_id: ActivityId,
    },
}

/// In-memory live activity state machine (07 §6).
///
/// - [`ToolActivityEvent::Render`] → merge into `live_blocks` + emit `DetailDelta`;
/// - [`ToolActivityEvent::Title`] → tool takes over the title (auto `· Ns` stops);
/// - terminal assembly happens in [`ActivityHandler::finish`].
pub struct ActivityHandler {
    pub activity_id: ActivityId,
    pub kind: ActivityKind,
    title: String,
    base_title: String,
    tool_author_title: bool,
    summary: String,
    state: ActivityState,
    live_blocks: Vec<ViewBlock>,
    attachments: Vec<agena_domain::AttachmentItem>,
    metadata: std::collections::BTreeMap<String, serde_json::Value>,
}

impl std::fmt::Debug for ActivityHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActivityHandler")
            .field("activity_id", &self.activity_id)
            .field("kind", &self.kind)
            .field("title", &self.title)
            .field("state", &self.state)
            .field("live_blocks", &self.live_blocks.len())
            .finish()
    }
}

impl ActivityHandler {
    pub fn begin(
        activity_id: ActivityId,
        kind: ActivityKind,
        initial_title: impl Into<String>,
    ) -> Self {
        let title = initial_title.into();
        Self {
            activity_id,
            kind,
            base_title: title.clone(),
            title,
            tool_author_title: false,
            summary: String::new(),
            state: ActivityState::Pending,
            live_blocks: Vec::new(),
            attachments: Vec::new(),
            metadata: Default::default(),
        }
    }

    pub fn state(&self) -> ActivityState {
        self.state
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn live_blocks(&self) -> &[ViewBlock] {
        &self.live_blocks
    }

    /// Apply one tool event; returns the wire events to broadcast (07 §6.1).
    pub fn apply_event(&mut self, event: ToolActivityEvent) -> Vec<ActivityLiveEvent> {
        let mut emitted = Vec::new();
        match event {
            ToolActivityEvent::Render(delta) => {
                self.merge_delta(&delta);
                emitted.push(ActivityLiveEvent::DetailDelta {
                    activity_id: self.activity_id,
                    delta,
                });
            }
            ToolActivityEvent::Title { title } => {
                self.tool_author_title = true;
                self.base_title = title.clone();
                self.title = title.clone();
                emitted.push(ActivityLiveEvent::TitleChanged {
                    activity_id: self.activity_id,
                    title,
                });
            }
            ToolActivityEvent::TitleSuffix { suffix } => {
                self.tool_author_title = true;
                self.title = format!("{}{}", self.title, suffix);
                emitted.push(ActivityLiveEvent::TitleChanged {
                    activity_id: self.activity_id,
                    title: self.title.clone(),
                });
            }
            ToolActivityEvent::Summary { summary } => {
                self.summary = summary.clone();
                emitted.push(ActivityLiveEvent::SummaryChanged {
                    activity_id: self.activity_id,
                    summary,
                });
            }
            ToolActivityEvent::Attachment(artifact) => {
                self.attachments.push(artifact.into());
            }
            ToolActivityEvent::Metadata { key, value } => {
                self.metadata.insert(key, serde_json::json!(value));
            }
        }
        emitted
    }

    /// Automatic elapsed-time title refresh: only when the tool has not taken
    /// over the title (07 §5.1). Returns the title change event to broadcast.
    pub fn refresh_elapsed_title(&mut self, elapsed_secs: u64) -> Option<ActivityLiveEvent> {
        if self.tool_author_title || elapsed_secs == 0 {
            return None;
        }
        let title = format!("{} · {}s", self.base_title, elapsed_secs);
        self.title = title.clone();
        Some(ActivityLiveEvent::TitleChanged {
            activity_id: self.activity_id,
            title,
        })
    }

    /// Assemble the terminal state node. `raw_output` comes from the tool's
    /// terminal result (or the accumulated facts when none was provided).
    pub fn finish(
        &mut self,
        result: ToolActivityResult,
        state: ActivityState,
    ) -> ActivityStateNode {
        self.state = state;
        if let Some(title) = result.title.filter(|t| !t.trim().is_empty()) {
            self.title = title;
        }
        if let Some(summary) = result.summary.filter(|s| !s.trim().is_empty()) {
            self.summary = summary;
        }
        let mut raw_output = result.raw_output;
        if raw_output.is_empty() {
            // Accumulated facts from streaming events.
            raw_output = RawOutput {
                attachments: std::mem::take(&mut self.attachments),
                metadata: std::mem::take(&mut self.metadata),
                ..RawOutput::default()
            };
        } else if !self.attachments.is_empty() {
            raw_output
                .attachments
                .extend(std::mem::take(&mut self.attachments));
        }
        ActivityStateNode {
            activity_id: self.activity_id,
            kind: self.kind,
            title: self.title.clone(),
            summary: self.summary.clone(),
            state,
            raw_output: Some(raw_output),
        }
    }

    fn merge_delta(&mut self, delta: &RenderDelta) {
        let block_id = delta.block_id.as_deref().or_else(|| delta.view.block_id());
        match delta.mode {
            agena_domain::DeltaMode::New => {
                self.live_blocks.push(delta.view.clone());
            }
            agena_domain::DeltaMode::Append => {
                if let Some(id) = block_id
                    && let Some(block) = self
                        .live_blocks
                        .iter_mut()
                        .find(|b| b.block_id() == Some(id))
                {
                    append_to_block(block, &delta.view);
                    return;
                }
                self.live_blocks.push(delta.view.clone());
            }
            agena_domain::DeltaMode::Replace => {
                if let Some(id) = block_id
                    && let Some(block) = self
                        .live_blocks
                        .iter_mut()
                        .find(|b| b.block_id() == Some(id))
                {
                    *block = delta.view.clone();
                    return;
                }
                self.live_blocks.push(delta.view.clone());
            }
        }
    }
}

fn append_to_block(target: &mut ViewBlock, incoming: &ViewBlock) {
    let append_text = match incoming {
        ViewBlock::Text { text, .. } | ViewBlock::Markdown { text, .. } => Some(text.as_str()),
        ViewBlock::Log { text, .. } => Some(text.as_str()),
        _ => None,
    };
    if let Some(text) = append_text {
        match target {
            ViewBlock::Text { text: t, .. }
            | ViewBlock::Markdown { text: t, .. }
            | ViewBlock::Log { text: t, .. } => t.push_str(text),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agena_domain::{CommandOutputStream, DeltaMode};

    fn handler() -> ActivityHandler {
        ActivityHandler::begin(
            agena_domain::ActivityId::new(),
            ActivityKind::Operation,
            "shell.run",
        )
    }

    #[test]
    fn render_deltas_merge_into_live_blocks() {
        let mut h = handler();
        let events = h.apply_event(ToolActivityEvent::Render(RenderDelta::new(ViewBlock::log(
            "out",
            CommandOutputStream::Stdout,
            "a\n",
        ))));
        assert_eq!(events.len(), 1);
        let events = h.apply_event(ToolActivityEvent::Render(RenderDelta::append(
            "out",
            ViewBlock::log("out", CommandOutputStream::Stdout, "b\n"),
        )));
        assert_eq!(events.len(), 1);
        assert_eq!(h.live_blocks.len(), 1);
        match &h.live_blocks[0] {
            ViewBlock::Log { text, .. } => assert_eq!(text, "a\nb\n"),
            other => panic!("expected log, got {other:?}"),
        }
    }

    #[test]
    fn title_takeover_stops_auto_elapsed() {
        let mut h = handler();
        h.apply_event(ToolActivityEvent::Title {
            title: "cargo test".into(),
        });
        assert!(h.refresh_elapsed_title(5).is_none());
        assert_eq!(h.title(), "cargo test");
    }

    #[test]
    fn auto_elapsed_title_when_tool_did_not_take_over() {
        let mut h = handler();
        let event = h.refresh_elapsed_title(12).expect("elapsed refresh");
        match event {
            ActivityLiveEvent::TitleChanged { title, .. } => {
                assert_eq!(title, "shell.run · 12s");
            }
            other => panic!("unexpected {other:?}"),
        }
        assert!(h.refresh_elapsed_title(0).is_none());
    }

    #[test]
    fn finish_assembles_terminal_node_with_facts() {
        let mut h = handler();
        h.apply_event(ToolActivityEvent::Render(RenderDelta::new(ViewBlock::log(
            "out",
            CommandOutputStream::Stdout,
            "done\n",
        ))));
        let node = h.finish(
            ToolActivityResult::raw(RawOutput {
                payload: Some(serde_json::json!({ "exit_code": 0 })),
                text: "done\n".into(),
                ..RawOutput::default()
            }),
            ActivityState::Completed,
        );
        assert_eq!(node.state, ActivityState::Completed);
        assert_eq!(node.title, "shell.run");
        assert_eq!(node.raw_output.as_ref().unwrap().text, "done\n");
    }

    #[test]
    fn live_event_wire_shape_roundtrips() {
        let event = ActivityLiveEvent::DetailDelta {
            activity_id: agena_domain::ActivityId::new(),
            delta: RenderDelta {
                block_id: None,
                mode: DeltaMode::default(),
                view: ViewBlock::Text {
                    id: None,
                    text: "hi".into(),
                },
            },
        };
        let encoded = serde_json::to_string(&event).unwrap();
        let decoded: ActivityLiveEvent = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, event);
    }
}
