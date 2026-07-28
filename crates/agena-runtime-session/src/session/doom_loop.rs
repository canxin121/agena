//! Doom-loop detection for session runs.
//!
//! Watches the recent assistant tool invocations on a session and trips when
//! the *same* `(tool, input)` pair has been issued more than the configured
//! threshold in immediate succession. Repeating the identical call rarely
//! produces a different answer — the only thing it does produce is wasted
//! tokens — so we abort the run instead of continuing forever.

use crate::message::{Message, PartContent};
use agena_domain::{DoomLoopHit, DoomLoopPolicy, Role, ToolInvocation};

/// Walk `messages` from the tail forward and detect a run of identical
/// assistant tool invocations.
pub fn detect(messages: &[Message], policy: DoomLoopPolicy) -> Option<DoomLoopHit> {
    if !policy.is_enabled() {
        return None;
    }

    let mut latest_signature: Option<(String, String)> = None;
    let mut run_len: u8 = 0;

    'messages: for message in messages.iter().rev() {
        if message.is_activity() {
            continue;
        }
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
    use crate::message::{ActivityPart, OperationPart};
    use agena_domain::ToolOutput;
    use agena_domain::{StructuredObject, TimeRange};

    fn operation_message(role: Role) -> Message {
        let operation = OperationPart::completed(
            1,
            ToolInvocation::new("test.repeat", StructuredObject::default()),
            "done",
            Vec::new(),
            Vec::new(),
            ToolOutput::default(),
            TimeRange::default(),
        );
        Message::prompt_parts(role, vec![PartContent::Operation(operation)])
    }

    fn activity_message() -> Message {
        Message::prompt_parts(
            Role::System,
            vec![PartContent::Activity(ActivityPart::execution(
                agena_domain::ExecutionId::new(),
                agena_domain::ExecutionSource::Compaction,
                1,
            ))],
        )
    }

    #[test]
    fn activity_is_transparent_to_doom_loop_detection() {
        let messages = vec![
            operation_message(Role::Assistant),
            activity_message(),
            operation_message(Role::Assistant),
            operation_message(Role::Assistant),
        ];

        assert_eq!(
            detect(&messages, DoomLoopPolicy::default()),
            Some(DoomLoopHit {
                tool_label: "test.repeat".to_owned(),
                repeat_count: 3,
            }),
        );
    }
}
