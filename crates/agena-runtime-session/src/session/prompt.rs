//! Bound model-facing projections without creating a second content archive.
//! Source bodies stay in canonical resources; explicit content.read pages
//! retrieve retained bytes beyond the preview, including within one record.

use agena_provider::{CompletionInputPart, CompletionInputRun};
use agena_runtime_tools::tool::{
    bounded_model_output_preview, line_count, model_output_exceeds_boundary,
};

pub(crate) const MAX_MODEL_TOOL_OUTPUT_LINES: usize = 2_000;
pub(crate) const MAX_MODEL_TOOL_OUTPUT_BYTES: usize = 50 * 1024;
static PROMPT_WORKERS: agena_async::BlockingPool = agena_async::BlockingPool::new(2);

pub(crate) async fn bound_model_tool_outputs_async(
    mut turns: Vec<CompletionInputRun>,
) -> Result<Vec<CompletionInputRun>, crate::AppError> {
    PROMPT_WORKERS
        .run(move || {
            bound_model_tool_outputs(&mut turns);
            turns
        })
        .await
        .map_err(|error| crate::AppError::Internal(format!("prompt output worker failed: {error}")))
}

pub(crate) fn bound_model_tool_outputs(turns: &mut [CompletionInputRun]) {
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
        let complete = std::mem::take(output_json);
        let marker = format!(
            "... model projection truncated ({} lines, {} bytes); use content.read with the source resource references below to read retained bytes ...",
            line_count(&complete),
            complete.len()
        );
        *output_json = bounded_model_output_preview(
            &complete,
            &marker,
            MAX_MODEL_TOOL_OUTPUT_LINES,
            MAX_MODEL_TOOL_OUTPUT_BYTES,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn result_part(text: &str) -> CompletionInputRun {
        CompletionInputRun {
            role: agena_domain::Role::Tool,
            parts: vec![CompletionInputPart::ToolResult {
                attachments: Vec::new(),
                tool_call_id: "call-1".into(),
                function: agena_provider::ModelToolFunction::new("shell_run"),
                arguments_json: String::new(),
                status: agena_provider::CompletionInputToolResultStatus::Completed,
                output_json: text.into(),
            }],
            provider_state: Default::default(),
        }
    }
    fn only_output(run: &CompletionInputRun) -> &str {
        let CompletionInputPart::ToolResult { output_json, .. } = &run.parts[0] else {
            panic!("tool result");
        };
        output_json
    }
    #[test]
    fn short_and_empty_results_are_untouched() {
        let mut turns = vec![result_part("short output"), result_part("")];
        bound_model_tool_outputs(&mut turns);
        assert_eq!(only_output(&turns[0]), "short output");
        assert_eq!(only_output(&turns[1]), "");
    }
    #[test]
    fn oversized_projection_keeps_diagnostics_and_resource_references_inside_the_budget() {
        let mut text = (0..5000)
            .map(|i| format!("line {i}: noisy command output"))
            .collect::<Vec<_>>()
            .join("\n");
        text.push_str(
            "\n<agena_content_resources>{resource_id: canonical-id}</agena_content_resources>",
        );
        let mut turns = vec![result_part(&text)];
        bound_model_tool_outputs(&mut turns);
        let bounded = only_output(&turns[0]);
        assert!(bounded.len() <= MAX_MODEL_TOOL_OUTPUT_BYTES);
        assert!(line_count(bounded) <= MAX_MODEL_TOOL_OUTPUT_LINES);
        assert!(bounded.contains("line 0:"));
        assert!(bounded.contains("line 4999:"));
        assert!(!bounded.contains("line 2500:"));
        assert!(bounded.contains("content.read"));
        assert!(bounded.contains("resource_id: canonical-id"));
    }
    #[test]
    fn utf8_and_a_long_single_line_are_bounded_without_breaking_characters() {
        let mut turns = vec![result_part(&"中文🙂".repeat(20000))];
        bound_model_tool_outputs(&mut turns);
        assert!(only_output(&turns[0]).len() <= MAX_MODEL_TOOL_OUTPUT_BYTES);
        assert!(only_output(&turns[0]).contains("truncated"));
    }
}
