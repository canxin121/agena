//! Doom-loop detection for session runs.
//!
//! Watches settled assistant tool invocations in the current user task.
//! Repeated calls with the same input and result indicate that the model is
//! not making progress, including when it alternates between help lookups or
//! starts a continuation run.

use agena_domain::{DoomLoopHit, DoomLoopPolicy, ToolInvocation};
use agena_storage::store::{Part, PartRole};
use std::sync::atomic::Ordering;

use crate::session::store::typed_content_from_value;
use agena_runtime_contracts::part_content::{TypedContent, operation_from_tool_call};
use portable_atomic::AtomicU64;

static SIGNATURE_SERIALIZATION_FAILURE_SEQUENCE: AtomicU64 = AtomicU64::new(1);

/// Help lookups can alternate (A, B, A, B, A), so inspect a short window of
/// them across assistant continuation runs. For all other tools, require
/// consecutive matching invocations in one run. A recovery marker starts a
/// fresh detection window so previous calls cannot trigger the same recovery
/// again. Text, reasoning, and compaction markers do not break a sequence.
pub fn detect(parts: &[Part], policy: DoomLoopPolicy) -> Option<DoomLoopHit> {
    if !policy.is_enabled() {
        return None;
    }

    const RECENT_HELP_CALL_WINDOW: usize = 12;
    let mut latest_signature: Option<(String, String)> = None;
    let mut latest_run_id = None::<Option<i64>>;
    let mut latest_output = None;
    let mut repeat_count: u8 = 0;
    let mut calls_seen = 0usize;

    for part in parts.iter().rev() {
        if part.role != PartRole::Assistant {
            break;
        }
        // A completed assistant answer is progress. Historical tool calls
        // before it must not start another recovery on the stop path.
        if latest_signature.is_none() && part.kind == "text" {
            break;
        }
        if part.is_run_marker() {
            if part
                .content
                .get("doom_loop_recovery")
                .and_then(serde_json::Value::as_bool)
                == Some(true)
            {
                break;
            }
            if matches!(
                part.content
                    .get("run_kind")
                    .and_then(serde_json::Value::as_str),
                Some("compaction" | "continue")
            ) {
                continue;
            }
            break;
        }
        let Ok(TypedContent::ToolCall(tool_call)) =
            typed_content_from_value(&part.kind, &part.content)
        else {
            continue;
        };
        if !part.state.is_terminal() {
            break;
        }
        if latest_run_id.is_none() {
            latest_run_id = Some(part.run_id);
        }
        let exec = operation_from_tool_call(&tool_call);
        let signature = signature_of(exec.invocation());
        calls_seen += 1;
        if latest_signature.is_none() {
            latest_signature = Some(signature.clone());
            latest_output = Some((part.content.get("output"), part.content.get("error")));
        }
        let latest_is_help = latest_signature.as_ref().is_some_and(|(name, _)| {
            matches!(name.as_str(), "tools_help" | "tools_list" | "tools_search")
        });
        if !latest_is_help && Some(part.run_id) != latest_run_id {
            break;
        }
        if !latest_is_help && latest_signature.as_ref() != Some(&signature) {
            break;
        }
        if latest_signature.as_ref() == Some(&signature) {
            if latest_output != Some((part.content.get("output"), part.content.get("error"))) {
                break;
            }
            repeat_count = repeat_count.saturating_add(1);
        }
        if repeat_count >= policy.repeat_threshold {
            let label = latest_signature
                .as_ref()
                .map(|(name, _)| name.clone())
                .unwrap_or_else(|| "<unknown>".to_string());
            return Some(DoomLoopHit {
                tool_label: label,
                repeat_count,
            });
        }
        if calls_seen >= RECENT_HELP_CALL_WINDOW {
            break;
        }
    }

    None
}

fn signature_of(invocation: &ToolInvocation) -> (String, String) {
    let ToolInvocation { name, input, .. } = invocation;
    let input = match serde_json::to_string(input) {
        Ok(input) => input,
        Err(error) => {
            let sequence = SIGNATURE_SERIALIZATION_FAILURE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            tracing::error!(
                tool_name = name,
                sequence,
                diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                    "serialize tool invocation for doom-loop detection",
                    &error,
                ),
                "doom-loop detection assigned a one-use signature after serialization failure"
            );
            format!("__agena_doom_loop_serialization_failure_{sequence}")
        }
    };
    (name.clone(), input)
}

#[cfg(test)]
mod tests {
    use super::*;
    use agena_domain::{RawOutput, StructuredObject, TimeRange};
    use agena_runtime_contracts::{part::OperationPart, part_content::tool_call_from_operation};
    use agena_storage::store::{PartState, PartVisibility};

    fn marker(part_id: i64, run_kind: &str) -> Part {
        Part {
            part_id,
            kind: "run".to_owned(),
            role: PartRole::Assistant,
            state: PartState::Completed,
            content: serde_json::json!({ "run_kind": run_kind }),
            summary: None,
            visibility: PartVisibility::Both,
            parent_part_id: None,
            run_id: None,
            origin_session_id: 1,
            revision: 0,
            started_at_ms: 1,
            finished_at_ms: Some(2),
            created_at_ms: part_id,
            updated_at_ms: part_id,
            provider_state: None,
        }
    }

    fn help_call(part_id: i64, target: &str, output: &str) -> Part {
        help_call_in_run(part_id, 1, target, output)
    }

    fn help_call_in_run(part_id: i64, run_id: i64, target: &str, output: &str) -> Part {
        let invocation = ToolInvocation::new(
            "tools_help",
            StructuredObject::try_from(serde_json::json!({ "tool": target }))
                .expect("structured input"),
        );
        let operation = OperationPart::completed(
            part_id,
            invocation,
            RawOutput::text(output),
            TimeRange::default(),
        );
        Part {
            part_id,
            kind: "tool_call".to_owned(),
            role: PartRole::Assistant,
            state: PartState::Completed,
            content: tool_call_from_operation(&operation).as_value(),
            summary: None,
            visibility: PartVisibility::Both,
            parent_part_id: None,
            run_id: Some(run_id),
            origin_session_id: 1,
            revision: 0,
            started_at_ms: 1,
            finished_at_ms: Some(2),
            created_at_ms: part_id,
            updated_at_ms: part_id,
            provider_state: None,
        }
    }

    #[test]
    fn alternating_identical_help_results_cross_a_compaction_marker() {
        let parts = vec![
            marker(1, "continue"),
            help_call(2, "session.model", "model docs"),
            help_call(3, "session.rename", "rename docs"),
            marker(4, "compaction"),
            help_call(5, "session.model", "model docs"),
            help_call(6, "session.rename", "rename docs"),
            help_call(7, "session.model", "model docs"),
        ];
        let hit = detect(&parts, DoomLoopPolicy::default()).expect("repeated help loop");
        assert_eq!(hit.tool_label, "tools_help");
        assert_eq!(hit.repeat_count, 3);
    }

    #[test]
    fn changed_help_result_does_not_count_as_the_same_loop() {
        let parts = vec![
            marker(1, "continue"),
            help_call(2, "session.model", "old docs"),
            help_call(3, "session.rename", "rename docs"),
            help_call(4, "session.model", "new docs"),
        ];
        assert!(detect(&parts, DoomLoopPolicy::default()).is_none());
    }

    #[test]
    fn repeated_help_across_continuation_runs_is_detected_once() {
        let mut recovery = marker(7, "continue");
        recovery.content["doom_loop_recovery"] = serde_json::Value::Bool(true);
        let before_recovery = vec![
            marker(1, "continue"),
            help_call_in_run(2, 1, "session.model", "model docs"),
            help_call_in_run(3, 1, "session.rename", "rename docs"),
            marker(4, "continue"),
            help_call_in_run(5, 4, "session.model", "model docs"),
            help_call_in_run(6, 4, "session.rename", "rename docs"),
            help_call_in_run(7, 4, "session.model", "model docs"),
        ];
        let hit = detect(&before_recovery, DoomLoopPolicy::default())
            .expect("help repeated across assistant runs");
        assert_eq!(hit.repeat_count, 3);

        let mut after_recovery = before_recovery[..6].to_vec();
        after_recovery.push(recovery);
        after_recovery.push(help_call_in_run(8, 7, "session.model", "model docs"));
        assert!(detect(&after_recovery, DoomLoopPolicy::default()).is_none());
    }

    #[test]
    fn final_answer_closes_the_help_loop_window() {
        let mut answer = marker(8, "continue");
        answer.kind = "text".to_owned();
        answer.run_id = Some(1);
        answer.content = serde_json::json!({ "text": "Here is the answer." });
        let parts = vec![
            marker(1, "continue"),
            help_call(2, "session.model", "model docs"),
            help_call(3, "session.rename", "rename docs"),
            help_call(4, "session.model", "model docs"),
            help_call(5, "session.rename", "rename docs"),
            help_call(6, "session.model", "model docs"),
            answer,
        ];
        assert!(detect(&parts, DoomLoopPolicy::default()).is_none());
    }
}
