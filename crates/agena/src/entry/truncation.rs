use crate::message::FirstPartyToolOutput;

use super::result::FirstPartyExecution;

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

    pub fn apply(&self, mut execution: FirstPartyExecution) -> FirstPartyExecution {
        execution.view.output_text =
            truncate_text(&execution.view.output_text, self.policy.max_chars);

        match &mut execution.output {
            FirstPartyToolOutput::Bash { output, .. }
            | FirstPartyToolOutput::PowerShell { output, .. } => {
                if let Some(text) = output.as_mut() {
                    *text = truncate_text(text, self.policy.max_chars);
                }
            }
            FirstPartyToolOutput::Read { preview, .. } => {
                if let Some(text) = preview.as_mut() {
                    *text = truncate_text(text, self.policy.max_chars);
                }
            }
            FirstPartyToolOutput::ViewFile { .. } => {}
            FirstPartyToolOutput::ApplyPatch {
                inverse_patch,
                diff,
                ..
            } => {
                *inverse_patch = truncate_text(inverse_patch, self.policy.max_chars);
                *diff = truncate_text(diff, self.policy.max_chars);
            }
            FirstPartyToolOutput::Glob { .. }
            | FirstPartyToolOutput::Grep { .. }
            | FirstPartyToolOutput::Task { .. }
            | FirstPartyToolOutput::ToolSearch { .. }
            | FirstPartyToolOutput::TodoWrite { .. }
            | FirstPartyToolOutput::AskUser { .. }
            | FirstPartyToolOutput::Monitor { .. }
            | FirstPartyToolOutput::WebFetch { .. }
            | FirstPartyToolOutput::WebSearch { .. }
            | FirstPartyToolOutput::EnterPlanMode { .. }
            | FirstPartyToolOutput::ExitPlanMode { .. }
            | FirstPartyToolOutput::EnterWorktree { .. }
            | FirstPartyToolOutput::ExitWorktree { .. }
            | FirstPartyToolOutput::CronCreate { .. }
            | FirstPartyToolOutput::CronList { .. }
            | FirstPartyToolOutput::CronDelete { .. }
            | FirstPartyToolOutput::ScheduleWakeup { .. }
            | FirstPartyToolOutput::LspDefinition { .. }
            | FirstPartyToolOutput::LspReferences { .. }
            | FirstPartyToolOutput::LspHover { .. }
            | FirstPartyToolOutput::LspDiagnostics { .. }
            | FirstPartyToolOutput::NotebookEdit { .. } => {}
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
