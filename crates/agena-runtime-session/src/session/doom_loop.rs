//! Doom-loop detection for session runs.
//!
//! Watches the recent assistant tool invocations on a session and trips when
//! the *same* `(tool, input)` pair has been issued more than the configured
//! threshold in immediate succession. Repeating the identical call rarely
//! produces a different answer — the only thing it does produce is wasted
//! tokens — so we abort the run instead of continuing forever.

use agena_domain::{DoomLoopHit, DoomLoopPolicy, ToolInvocation};
use agena_storage::store::{Part, PartRole};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::session::store::typed_content_from_value;
use agena_runtime_contracts::part_content::{TypedContent, operation_from_tool_call};

static SIGNATURE_SERIALIZATION_FAILURE_SEQUENCE: AtomicU64 = AtomicU64::new(1);

/// Walk `parts` from the tail forward and detect a run of identical
/// assistant tool invocations. Only assistant-role `Operation` parts
/// participate; a non-tool part inside an already-signature-carrying run
/// breaks the chain, as does an assistant run with no tool call at all.
pub fn detect(parts: &[Part], policy: DoomLoopPolicy) -> Option<DoomLoopHit> {
    if !policy.is_enabled() {
        return None;
    }

    let mut latest_signature: Option<(String, String)> = None;
    let mut run_len: u8 = 0;
    let mut saw_tool_in_run = false;

    'parts: for part in parts.iter().rev() {
        if part.role != PartRole::Assistant {
            break;
        }
        if part.is_run_marker() {
            // Run boundary: an assistant run whose parts carried no tool call
            // breaks the chain (mirrors the v1 per-message `saw_tool_in_message`
            // break).
            if !saw_tool_in_run {
                break;
            }
            saw_tool_in_run = false;
            continue;
        }
        let Ok(TypedContent::ToolCall(tool_call)) =
            typed_content_from_value(&part.kind, &part.content)
        else {
            if latest_signature.is_some() {
                break 'parts;
            }
            continue;
        };
        saw_tool_in_run = true;
        let exec = operation_from_tool_call(&tool_call);
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
