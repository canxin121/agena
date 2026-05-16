use crate::message::BundledToolOutput;

use super::result::BundledExecution;

const DEFAULT_OUTPUT_LIMIT: usize = 16 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolOutputTruncationPolicy {
    pub max_chars: usize,
}

impl Default for ToolOutputTruncationPolicy {
    fn default() -> Self {
        Self {
            max_chars: DEFAULT_OUTPUT_LIMIT,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ToolOutputTruncator {
    policy: ToolOutputTruncationPolicy,
}

impl Default for ToolOutputTruncator {
    fn default() -> Self {
        Self::new(ToolOutputTruncationPolicy::default())
    }
}

impl ToolOutputTruncator {
    pub fn new(policy: ToolOutputTruncationPolicy) -> Self {
        Self { policy }
    }

    pub fn apply(&self, mut execution: BundledExecution) -> BundledExecution {
        execution.view.output_text =
            truncate_text(&execution.view.output_text, self.policy.max_chars);

        match &mut execution.output {
            BundledToolOutput::Bash { output, .. }
            | BundledToolOutput::PowerShell { output, .. } => {
                if let Some(text) = output.as_mut() {
                    *text = truncate_text(text, self.policy.max_chars);
                }
            }
            BundledToolOutput::Read { preview, .. } => {
                if let Some(text) = preview.as_mut() {
                    *text = truncate_text(text, self.policy.max_chars);
                }
            }
            BundledToolOutput::ViewFile { .. } => {}
            BundledToolOutput::ApplyPatch {
                inverse_patch,
                diff,
                ..
            } => {
                *inverse_patch = truncate_text(inverse_patch, self.policy.max_chars);
                *diff = truncate_text(diff, self.policy.max_chars);
            }
            BundledToolOutput::Glob { .. }
            | BundledToolOutput::Grep { .. }
            | BundledToolOutput::Task { .. }
            | BundledToolOutput::ToolSearch { .. }
            | BundledToolOutput::TodoWrite { .. }
            | BundledToolOutput::AskUser { .. }
            | BundledToolOutput::Monitor { .. }
            | BundledToolOutput::WebFetch { .. }
            | BundledToolOutput::WebSearch { .. }
            | BundledToolOutput::EnterPlanMode { .. }
            | BundledToolOutput::ExitPlanMode { .. }
            | BundledToolOutput::EnterWorktree { .. }
            | BundledToolOutput::ExitWorktree { .. }
            | BundledToolOutput::CronCreate { .. }
            | BundledToolOutput::CronList { .. }
            | BundledToolOutput::CronDelete { .. }
            | BundledToolOutput::ScheduleWakeup { .. }
            | BundledToolOutput::LspDefinition { .. }
            | BundledToolOutput::LspReferences { .. }
            | BundledToolOutput::LspHover { .. }
            | BundledToolOutput::LspDiagnostics { .. }
            | BundledToolOutput::NotebookEdit { .. } => {}
        }

        execution
    }
}

fn truncate_text(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let truncated = value.chars().take(max_chars).collect::<String>();
    format!("{truncated}\n\n[truncated to {max_chars} chars]")
}
