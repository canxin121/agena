//! Doom-loop detection for session runs.
//!
//! Watches the recent assistant tool invocations on a session and trips when
//! the *same* `(tool, input)` pair has been issued more than the configured
//! threshold in immediate succession. Repeating the identical call rarely
//! produces a different answer — the only thing it does produce is wasted
//! tokens — so we abort the run instead of continuing forever.

use serde::{Deserialize, Serialize};

use crate::message::{Message, PartContent, ToolInvocation};
use crate::role::Role;

/// Configuration for [`detect`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DoomLoopPolicy {
    /// Number of immediately-consecutive identical tool calls that constitute
    /// a doom loop. Values below 2 disable the check.
    pub repeat_threshold: u8,
}

impl Default for DoomLoopPolicy {
    fn default() -> Self {
        Self {
            repeat_threshold: 3,
        }
    }
}

impl DoomLoopPolicy {
    pub const fn disabled() -> Self {
        Self {
            repeat_threshold: 0,
        }
    }

    pub const fn is_enabled(&self) -> bool {
        self.repeat_threshold >= 2
    }
}

/// Description of a detected doom loop, suitable for surfacing as an error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DoomLoopHit {
    pub tool_label: String,
    pub repeat_count: u8,
}

impl DoomLoopHit {
    pub fn message(&self) -> String {
        format!(
            "doom-loop detected: tool `{}` was invoked with the same input {} times in a row; aborting run",
            self.tool_label, self.repeat_count
        )
    }
}

/// Walk `messages` from the tail forward and detect a run of identical
/// assistant tool invocations.
pub fn detect(messages: &[Message], policy: DoomLoopPolicy) -> Option<DoomLoopHit> {
    if !policy.is_enabled() {
        return None;
    }

    let mut latest_signature: Option<(String, String)> = None;
    let mut run_len: u8 = 0;

    'messages: for message in messages.iter().rev() {
        if message.role != Role::Assistant {
            break;
        }
        let mut saw_tool_in_message = false;
        for part in message.parts.iter().rev() {
            let Some(PartContent::Operation(exec)) = part.content.as_ref() else {
                if latest_signature.is_some() {
                    break 'messages;
                }
                continue;
            };
            saw_tool_in_message = true;
            let signature = signature_of(exec.invocation());
            match &latest_signature {
                Some(prev) if prev == &signature => {
                    run_len = run_len.saturating_add(1);
                }
                _ => {
                    latest_signature = Some(signature);
                    run_len = 1;
                }
            }
            if run_len >= policy.repeat_threshold {
                let label = latest_signature
                    .as_ref()
                    .map(|(name, _)| name.clone())
                    .unwrap_or_else(|| "<unknown>".to_string());
                return Some(DoomLoopHit {
                    tool_label: label,
                    repeat_count: run_len,
                });
            }
        }
        if !saw_tool_in_message {
            break;
        }
    }

    None
}

fn signature_of(invocation: &ToolInvocation) -> (String, String) {
    let ToolInvocation { name, input, .. } = invocation;
    (
        name.clone(),
        serde_json::to_string(input).unwrap_or_default(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{
        ExecutionStatus, MessagePart, OperationPart, PartContent, StructuredObject, TimeRange,
        ToolInvocation,
    };
    use chrono::Utc;
    use serde_json::json;

    fn assistant_tool(name: &str, input: serde_json::Value) -> Message {
        let created_at = Utc::now();
        let invocation = ToolInvocation::new(
            name,
            StructuredObject::try_from(input).expect("tool input should be an object"),
        );
        let mut message = Message {
            id: 0,
            role: Role::Assistant,
            state: ExecutionStatus::Completed,
            parts: vec![MessagePart::with_content(
                1,
                0,
                created_at,
                ExecutionStatus::Failed,
                PartContent::Operation(OperationPart::failed(
                    1,
                    invocation,
                    "failed",
                    "failed",
                    Vec::new(),
                    Vec::new(),
                    Default::default(),
                    TimeRange::default(),
                )),
            )],
            created_at,
            metadata: Default::default(),
            provider_state: None,
            usage: None,
        };
        message.parts[0].operation_id = Some("call".to_string());
        message
    }

    #[test]
    fn detects_tail_repeated_tool_calls() {
        let messages = vec![
            assistant_tool("tools", json!({})),
            assistant_tool("tools", json!({})),
            assistant_tool("tools", json!({})),
        ];

        let hit = detect(
            messages.as_slice(),
            DoomLoopPolicy {
                repeat_threshold: 3,
            },
        )
        .expect("repeated tail calls should trip doom-loop");

        assert_eq!(hit.tool_label, "tools");
        assert_eq!(hit.repeat_count, 3);
    }

    #[test]
    fn user_message_breaks_old_repeated_tool_calls() {
        let messages = vec![
            assistant_tool("tools", json!({})),
            assistant_tool("tools", json!({})),
            assistant_tool("tools", json!({})),
            Message::prompt_text(Role::User, "try again"),
        ];

        assert!(
            detect(
                messages.as_slice(),
                DoomLoopPolicy {
                    repeat_threshold: 3
                },
            )
            .is_none()
        );
    }

    #[test]
    fn assistant_text_breaks_repeated_tool_calls() {
        let messages = vec![
            assistant_tool("tools", json!({})),
            assistant_tool("tools", json!({})),
            Message::prompt_text(Role::Assistant, "I will answer without tools."),
            assistant_tool("tools", json!({})),
        ];

        assert!(
            detect(
                messages.as_slice(),
                DoomLoopPolicy {
                    repeat_threshold: 3
                },
            )
            .is_none()
        );
    }
}
