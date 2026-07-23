use super::{ToolPayloadOutput, result::ToolPayloadExecution};
use agena_tool::ToolOutputTruncationPolicy;

#[derive(Debug, Clone, Copy)]
pub(crate) struct ToolOutputTruncator {
    policy: ToolOutputTruncationPolicy,
}

impl Default for ToolOutputTruncator {
    fn default() -> Self {
        Self::new(ToolOutputTruncationPolicy::default())
    }
}

impl ToolOutputTruncator {
    pub(crate) fn new(policy: ToolOutputTruncationPolicy) -> Self {
        Self { policy }
    }

    pub(crate) fn apply(&self, mut execution: ToolPayloadExecution) -> ToolPayloadExecution {
        if self.policy.max_chars == usize::MAX {
            return execution;
        }

        let output_text = agena_runtime::truncate_tool_output_text(
            &execution.view.output_text,
            self.policy.max_chars,
        );
        execution.view.set_neutral_output(output_text);

        match &mut execution.output {
            ToolPayloadOutput::Shell { output, .. } => {
                if let Some(text) = output.as_mut() {
                    *text = agena_runtime::truncate_tool_output_text(text, self.policy.max_chars);
                }
            }
            ToolPayloadOutput::Read { preview, .. } => {
                if let Some(text) = preview.as_mut() {
                    *text = agena_runtime::truncate_tool_output_text(text, self.policy.max_chars);
                }
            }
            ToolPayloadOutput::ApplyPatch {
                inverse_patch,
                diff,
                ..
            } => {
                *inverse_patch =
                    agena_runtime::truncate_tool_output_text(inverse_patch, self.policy.max_chars);
                *diff = agena_runtime::truncate_tool_output_text(diff, self.policy.max_chars);
            }
            ToolPayloadOutput::Glob { .. }
            | ToolPayloadOutput::Grep { .. }
            | ToolPayloadOutput::Task { .. }
            | ToolPayloadOutput::ToolSearch { .. }
            | ToolPayloadOutput::AskUser { .. }
            | ToolPayloadOutput::WebFetch { .. }
            | ToolPayloadOutput::WebSearch { .. }
            | ToolPayloadOutput::EnterSnapshot { .. }
            | ToolPayloadOutput::ExitSnapshot { .. }
            | ToolPayloadOutput::CronCreate { .. }
            | ToolPayloadOutput::CronList { .. }
            | ToolPayloadOutput::CronDelete { .. }
            | ToolPayloadOutput::ScheduleWakeup { .. }
            | ToolPayloadOutput::LspDefinition { .. }
            | ToolPayloadOutput::LspReferences { .. }
            | ToolPayloadOutput::LspHover { .. }
            | ToolPayloadOutput::LspDiagnostics { .. } => {}
        }

        execution
    }
}
