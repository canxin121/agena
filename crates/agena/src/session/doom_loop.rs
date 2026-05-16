//! Doom-loop detection for session turns.
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
            "doom-loop detected: tool `{}` was invoked with the same input {} times in a row; aborting turn",
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

    for message in messages.iter().rev() {
        if message.role != Role::Assistant {
            continue;
        }
        for part in message.parts.iter().rev() {
            let Some(PartContent::Operation(exec)) = part.content.as_ref() else {
                continue;
            };
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
        ExecutionStatus, OperationPart, PartContent, StructuredField, StructuredObject,
        StructuredValue, TimeRange, ToolInvocation, ToolOutput,
    };
    use crate::role::Role;
    use chrono::Utc;

    fn invocation(name: &str, value: &str) -> ToolInvocation {
        let input = StructuredObject {
            fields: vec![StructuredField {
                name: "v".to_string(),
                value: StructuredValue::Text {
                    value: value.to_string(),
                },
            }],
        };
        ToolInvocation {
            name: name.to_string(),
            plugin_name: None,
            input,
        }
    }

    fn assistant_with_calls(message_id: i64, calls: Vec<ToolInvocation>) -> Message {
        let parts: Vec<PartContent> = calls
            .into_iter()
            .enumerate()
            .map(|(idx, inv)| {
                PartContent::Operation(OperationPart::completed(
                    message_id * 1000 + idx as i64 + 1,
                    inv,
                    String::new(),
                    Vec::new(),
                    Vec::new(),
                    ToolOutput::default(),
                    TimeRange::default(),
                ))
            })
            .collect();

        let mut message = Message::prompt_parts(Role::Assistant, parts);
        message.id = message_id;
        let now = Utc::now();
        for (idx, part) in message.parts.iter_mut().enumerate() {
            part.id = message_id * 1000 + idx as i64 + 1;
            part.message_id = message_id;
            part.created_at = now;
            part.status = ExecutionStatus::Completed;
        }
        message
    }

    #[test]
    fn detects_three_in_a_row() {
        let calls = vec![
            invocation("bash", "ls"),
            invocation("bash", "ls"),
            invocation("bash", "ls"),
        ];
        let messages = vec![assistant_with_calls(1, calls)];
        let hit = detect(&messages, DoomLoopPolicy::default()).expect("doom loop");
        assert_eq!(hit.tool_label, "bash");
        assert_eq!(hit.repeat_count, 3);
    }

    #[test]
    fn ignores_when_inputs_differ() {
        let calls = vec![
            invocation("bash", "ls"),
            invocation("bash", "pwd"),
            invocation("bash", "ls"),
        ];
        let messages = vec![assistant_with_calls(1, calls)];
        assert!(detect(&messages, DoomLoopPolicy::default()).is_none());
    }

    #[test]
    fn disabled_policy_never_trips() {
        let calls = vec![
            invocation("bash", "ls"),
            invocation("bash", "ls"),
            invocation("bash", "ls"),
            invocation("bash", "ls"),
        ];
        let messages = vec![assistant_with_calls(1, calls)];
        assert!(detect(&messages, DoomLoopPolicy::disabled()).is_none());
    }

    #[test]
    fn run_resets_on_different_tool() {
        let calls = vec![
            invocation("bash", "ls"),
            invocation("read", "ls"),
            invocation("bash", "ls"),
            invocation("bash", "ls"),
        ];
        let messages = vec![assistant_with_calls(1, calls)];
        let hit = detect(
            &messages,
            DoomLoopPolicy {
                repeat_threshold: 3,
            },
        );
        assert!(
            hit.is_none(),
            "only 2 trailing identical calls, threshold is 3"
        );
    }

    #[test]
    fn detects_across_messages() {
        let calls_a = vec![invocation("bash", "ls"), invocation("bash", "ls")];
        let calls_b = vec![invocation("bash", "ls")];
        let messages = vec![
            assistant_with_calls(1, calls_a),
            assistant_with_calls(2, calls_b),
        ];
        let hit = detect(&messages, DoomLoopPolicy::default()).expect("doom loop across msgs");
        assert_eq!(hit.tool_label, "bash");
        assert!(hit.repeat_count >= 3);
    }
}
