//! Tool activity protocol (design 07 §5.1 / 08 §2).
//!
//! Provider-independent contract between a tool and the runtime:
//! - [`ToolActivityEvent`]: realtime increments a tool pushes while streaming;
//! - [`ToolActivityResult`]: terminal facts (single [`RawOutput`]);
//! - [`ToolHumanRenderer`]: optional render function the tool owns; when absent
//!   the runtime falls back to rendering the raw output directly.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use agena_domain::{ArtifactRef, RawOutput, RenderDelta, ToolPresentationSection, ViewBlock};

/// Realtime event a tool pushes during execution (07 §5.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToolActivityEvent {
    /// Realtime render increment: the "human view" delta.
    Render(RenderDelta),
    /// Realtime title (tool takes over; runtime stops auto `· Ns` suffix).
    Title {
        #[serde(default, skip_serializing_if = "String::is_empty")]
        title: String,
    },
    /// Optional status suffix (e.g. ` · scanning`, ` · 3/5`).
    TitleSuffix {
        #[serde(default, skip_serializing_if = "String::is_empty")]
        suffix: String,
    },
    /// Realtime summary.
    Summary {
        #[serde(default, skip_serializing_if = "String::is_empty")]
        summary: String,
    },
    /// Collapsible section (aggregated at end).
    Section(ToolPresentationSection),
    /// Attachment fact.
    Attachment(ArtifactRef),
    /// Metadata fact.
    Metadata { key: String, value: String },
}

/// Terminal facts returned by a tool (08 §2). All fields optional except
/// `raw_output`; defaults preserve current runtime behavior (Golden).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolActivityResult {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    pub raw_output: RawOutput,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sections: Vec<ToolPresentationSection>,
}

impl ToolActivityResult {
    pub fn raw(raw_output: RawOutput) -> Self {
        Self {
            raw_output,
            ..Self::default()
        }
    }

    /// Compose the durable title at the shared boundary. Uses the tool-provided
    /// title when present; otherwise falls back to the composed tool title
    /// (caller supplies name + invocation summary).
    pub fn durable_title(
        &self,
        tool_name: &str,
        fallback_summary: impl AsRef<str>,
    ) -> String {
        match self.title.as_deref() {
            Some(title) if !title.trim().is_empty() => {
                crate::compose_tool_title(tool_name, title)
            }
            _ => crate::compose_tool_title(tool_name, fallback_summary),
        }
    }
}

/// Context handed to a tool's human render function.
///
/// Kept deliberately small: rendering must be a deterministic, side-effect-free
/// function. Workspace reads are allowed through `read_managed`-style access at
/// the runtime boundary; the protocol only carries the path so a plugin render
/// hook can ask for file contents over IPC.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RenderContext {
    #[serde(default)]
    pub workspace_root: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
}

/// Error returned by a tool's human render function. The runtime catches any
/// error and falls back to rendering the raw output directly.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RenderError {
    #[error("tool does not provide a human render function")]
    Fallback,
    #[error("human render failed: {0}")]
    Failed(String),
}

impl RenderError {
    pub fn fallback() -> Self {
        Self::Fallback
    }
}

/// Tool-owned human rendering contract (07 §4.3 / 08 §2).
///
/// When a tool registers a renderer, the runtime calls it on detail requests;
/// when it does not, the runtime renders the raw output directly (the same
/// content the model sees).
pub trait ToolHumanRenderer: Send + Sync {
    fn render_human(
        &self,
        ctx: &RenderContext,
        raw: &RawOutput,
    ) -> Result<Vec<ViewBlock>, RenderError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use agena_domain::{
        CommandOutputStream, DeltaMode, FileChangeKind, FileChangeRecord, RenderDelta,
    };
    use serde_json::json;

    #[test]
    fn tool_activity_event_serde_roundtrips() {
        let events = vec![
            ToolActivityEvent::Title { title: "cargo test".into() },
            ToolActivityEvent::TitleSuffix { suffix: " · scanning".into() },
            ToolActivityEvent::Summary { summary: "2 passed".into() },
            ToolActivityEvent::Render(RenderDelta::append(
                "out",
                ViewBlock::log("out", CommandOutputStream::Stdout, "ok\n"),
            )),
            ToolActivityEvent::Section(ToolPresentationSection {
                title: "stdout".into(),
                text: "ok".into(),
            }),
            ToolActivityEvent::Attachment(ArtifactRef {
                uri: "file:///a".into(),
                mime: "text/plain".into(),
                name: None,
                size_bytes: None,
                sha256: None,
            }),
            ToolActivityEvent::Metadata {
                key: "k".into(),
                value: "v".into(),
            },
        ];
        for event in events {
            let encoded = serde_json::to_string(&event).unwrap();
            let decoded: ToolActivityEvent = serde_json::from_str(&encoded).unwrap();
            assert_eq!(decoded, event);
        }
    }

    #[test]
    fn render_delta_keeps_serde_shape() {
        let delta = RenderDelta {
            block_id: None,
            mode: DeltaMode::default(),
            view: ViewBlock::Text {
                id: None,
                text: "hi".into(),
            },
        };
        let encoded = serde_json::to_string(&delta).unwrap();
        assert_eq!(
            encoded,
            r#"{"mode":"new","view":{"type":"text","text":"hi"}}"#
        );
    }

    #[test]
    fn durable_title_uses_tool_title_or_fallback() {
        let result = ToolActivityResult {
            title: Some("cargo test".into()),
            ..ToolActivityResult::raw(RawOutput::text("ok"))
        };
        assert_eq!(result.durable_title("shell.run", "cargo test"), "shell.run · cargo test");

        let no_title = ToolActivityResult::raw(RawOutput::text("ok"));
        assert_eq!(
            no_title.durable_title("shell.run", "cargo test"),
            "shell.run · cargo test"
        );
    }

    #[test]
    fn tool_activity_result_serde_with_payload() {
        let result = ToolActivityResult {
            title: Some("t".into()),
            summary: Some("s".into()),
            raw_output: RawOutput {
                payload: Some(json!({ "exit_code": 0 })),
                text: "ok".into(),
                ..RawOutput::default()
            },
            sections: vec![ToolPresentationSection {
                title: "x".into(),
                text: "y".into(),
            }],
        };
        let encoded = serde_json::to_string(&result).unwrap();
        let decoded: ToolActivityResult = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, result);
    }

    #[test]
    fn file_change_view_roundtrips() {
        let block = ViewBlock::FileChanges {
            id: None,
            changes: vec![FileChangeRecord {
                path: "a.rs".into(),
                kind: FileChangeKind::Updated,
                from_path: None,
            }],
        };
        let encoded = serde_json::to_string(&block).unwrap();
        let decoded: ViewBlock = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, block);
    }
}
