//! The single place that bounds model-facing tool output.
//!
//! Every tool result that reaches a model passes through here, after the
//! owning plugin has had its say about how the raw result is rendered. What
//! arrives is untrusted in size: a plugin renderer, a generic payload
//! projection, or a historical record replayed from the transcript can all be
//! arbitrarily long. The two constants below are therefore the only output
//! budget in the product - a tool declares nothing about its own length.
//!
//! Oversized output is not discarded. The whole text is written under the
//! workspace's managed state directory and the model is given the path, so it
//! can read the remainder with the ordinary file tools it already has.

use std::path::{Path, PathBuf};

use agena_provider::{CompletionInputPart, CompletionInputRun};
use agena_runtime_tools::tool::{
    bounded_model_output_preview, line_count, model_output_exceeds_boundary,
};

/// Model-facing tool results must be small enough that a run of noisy commands
/// cannot consume the whole context window.
pub(crate) const MAX_MODEL_TOOL_OUTPUT_LINES: usize = 2_000;
pub(crate) const MAX_MODEL_TOOL_OUTPUT_BYTES: usize = 50 * 1024;

static PROMPT_WORKERS: agena_async::BlockingPool = agena_async::BlockingPool::new(2);

pub(crate) async fn bound_model_tool_outputs_async(
    mut turns: Vec<CompletionInputRun>,
    workspace_root: Option<PathBuf>,
    session_id: i64,
) -> Result<Vec<CompletionInputRun>, crate::AppError> {
    PROMPT_WORKERS
        .run(move || {
            bound_model_tool_outputs(&mut turns, workspace_root.as_deref(), session_id);
            turns
        })
        .await
        .map_err(|error| crate::AppError::Internal(format!("prompt output worker failed: {error}")))
}

/// Bound every non-empty tool result the model is about to see.
///
/// A result within budget is left byte-for-byte untouched. A result over
/// budget keeps its head and its tail - the parts that normally carry setup
/// and final diagnostics - and its full text is spilled to
/// `<managed state>/tool_output/<session>/<sha256>.txt`.
///
/// The spill is best effort. If the file cannot be written the result is
/// still bounded and the marker says so, so an unreadable state directory can
/// never leak an unbounded result into the prompt.
pub(crate) fn bound_model_tool_outputs(
    turns: &mut [CompletionInputRun],
    workspace_root: Option<&Path>,
    session_id: i64,
) {
    bound_with_spill_path(turns, |text| {
        workspace_root
            .map(|root| agena_runtime_tools::tool_output_spill_path(root, session_id, text))
    });
}

/// The bounding itself, with the spill destination injected so a caller can
/// describe a write that is expected to fail.
fn bound_with_spill_path(
    turns: &mut [CompletionInputRun],
    spill_path: impl Fn(&str) -> Option<PathBuf>,
) {
    for part in turns.iter_mut().flat_map(|turn| turn.parts.iter_mut()) {
        let CompletionInputPart::ToolResult { output_json, .. } = part else {
            continue;
        };
        if output_json.is_empty()
            || !model_output_exceeds_boundary(
                output_json,
                MAX_MODEL_TOOL_OUTPUT_LINES,
                MAX_MODEL_TOOL_OUTPUT_BYTES,
            )
        {
            continue;
        }
        let complete = output_json.clone();
        let marker = format!(
            "... output truncated ({} lines, {} bytes) ...{}",
            line_count(complete.as_str()),
            complete.len(),
            footer_for(spill_path(complete.as_str()), complete.as_str()),
        );
        *output_json = bounded_model_output_preview(
            complete.as_str(),
            marker.as_str(),
            MAX_MODEL_TOOL_OUTPUT_LINES,
            MAX_MODEL_TOOL_OUTPUT_BYTES,
        );
    }
}

/// Save the full text and describe where it went.
///
/// The path is budgeted by the caller like any other marker text, so a long
/// path consumes preview bytes instead of pushing the preview over budget.
fn footer_for(path: Option<PathBuf>, text: &str) -> String {
    let Some(path) = path else {
        return "\n\nFull output could not be saved; only this preview remains.".to_owned();
    };
    if let Err(error) = spill(&path, text) {
        return format!("\n\nFull output could not be saved ({error}); only this preview remains.");
    }
    format!(
        "\n\nFull output saved to {}\nRead it with fs.read (offset/limit) or search it with fs.grep.",
        path.to_string_lossy(),
    )
}

/// Write once per distinct payload: the path is content-addressed, so a replay
/// of the same result in a later round finds the file already there.
fn spill(path: &Path, text: &str) -> std::io::Result<()> {
    if path.is_file() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    agena_runtime_tools::atomic_write_file(path, text.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SESSION: i64 = 7;

    fn result_part(text: &str) -> CompletionInputRun {
        CompletionInputRun {
            role: agena_domain::Role::Tool,
            parts: vec![CompletionInputPart::ToolResult {
                tool_call_id: "call-1".to_owned(),
                function: agena_provider::ModelToolFunction::new("shell_run"),
                arguments_json: String::new(),
                status: agena_provider::CompletionInputToolResultStatus::Completed,
                output_json: text.to_owned(),
            }],
            provider_state: Default::default(),
        }
    }

    fn only_output(run: &CompletionInputRun) -> &str {
        run.parts
            .iter()
            .find_map(|part| match part {
                CompletionInputPart::ToolResult { output_json, .. } => Some(output_json.as_str()),
                _ => None,
            })
            .unwrap()
    }

    fn long_output() -> String {
        (0..5_000)
            .map(|index| {
                if index == 2_500 {
                    "UNIQUE_MIDDLE_FAILURE".to_owned()
                } else {
                    format!("line {index}: noisy command output")
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn a_result_within_budget_is_untouched() {
        let mut turns = vec![result_part("short output")];
        bound_model_tool_outputs(&mut turns, None, SESSION);
        assert_eq!(only_output(&turns[0]), "short output");
    }

    #[test]
    fn an_empty_result_stays_empty() {
        let mut turns = vec![result_part("")];
        bound_model_tool_outputs(&mut turns, None, SESSION);
        assert_eq!(only_output(&turns[0]), "");
    }

    #[test]
    fn an_oversized_result_is_bounded_and_spilled() {
        let root = tempfile::tempdir().unwrap();
        let text = long_output();
        let mut turns = vec![result_part(&text)];
        bound_model_tool_outputs(&mut turns, Some(root.path()), SESSION);
        let bounded = only_output(&turns[0]);

        assert!(line_count(bounded) <= MAX_MODEL_TOOL_OUTPUT_LINES);
        assert!(bounded.len() <= MAX_MODEL_TOOL_OUTPUT_BYTES);
        assert!(!bounded.contains("UNIQUE_MIDDLE_FAILURE"));
        assert!(bounded.contains("line 0: noisy command output"));
        assert!(bounded.contains("line 4999: noisy command output"));

        let path = agena_runtime_tools::tool_output_spill_path(root.path(), SESSION, &text);
        assert!(path.is_file());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
        assert!(bounded.contains(path.to_string_lossy().as_ref()));
        assert!(
            path.parent()
                .is_some_and(|parent| parent.ends_with(SESSION.to_string()))
        );
    }

    #[test]
    fn the_footer_is_inside_the_budget() {
        let root = tempfile::tempdir().unwrap();
        let mut turns = vec![result_part(&long_output())];
        bound_model_tool_outputs(&mut turns, Some(root.path()), SESSION);
        let bounded = only_output(&turns[0]);
        assert!(bounded.len() <= MAX_MODEL_TOOL_OUTPUT_BYTES);
        assert!(bounded.contains("Full output saved to"));
    }

    #[test]
    fn the_same_payload_is_stored_once() {
        let root = tempfile::tempdir().unwrap();
        let text = long_output();
        let path = agena_runtime_tools::tool_output_spill_path(root.path(), SESSION, &text);
        for _ in 0..2 {
            let mut turns = vec![result_part(&text)];
            bound_model_tool_outputs(&mut turns, Some(root.path()), SESSION);
        }
        assert!(path.is_file());
        let entries = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn a_failed_spill_still_bounds_the_result() {
        let root = tempfile::tempdir().unwrap();
        // A directory in place of the spill file makes the write fail after
        // the parent directory has already been created.
        let blocked = root.path().join("blocked.txt");
        std::fs::create_dir_all(&blocked).unwrap();
        let mut turns = vec![result_part(&long_output())];
        bound_with_spill_path(&mut turns, |_| Some(blocked.clone()));
        let bounded = only_output(&turns[0]);
        assert!(line_count(bounded) <= MAX_MODEL_TOOL_OUTPUT_LINES);
        assert!(bounded.len() <= MAX_MODEL_TOOL_OUTPUT_BYTES);
        assert!(bounded.contains("could not be saved"));
    }

    #[test]
    fn a_missing_workspace_root_still_bounds_the_result() {
        let mut turns = vec![result_part(&long_output())];
        bound_model_tool_outputs(&mut turns, None, SESSION);
        let bounded = only_output(&turns[0]);
        assert!(bounded.len() <= MAX_MODEL_TOOL_OUTPUT_BYTES);
        assert!(bounded.contains("could not be saved"));
    }
}
