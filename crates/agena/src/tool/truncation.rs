use crate::message::BuiltinToolOutput;

use super::result::BuiltinExecution;

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

    pub fn apply(&self, mut execution: BuiltinExecution) -> BuiltinExecution {
        execution.view.output_text =
            truncate_text(&execution.view.output_text, self.policy.max_chars);

        match &mut execution.output {
            BuiltinToolOutput::Bash { output, .. } => {
                if let Some(text) = output.as_mut() {
                    *text = truncate_text(text, self.policy.max_chars);
                }
            }
            BuiltinToolOutput::Read { preview, .. } => {
                if let Some(text) = preview.as_mut() {
                    *text = truncate_text(text, self.policy.max_chars);
                }
            }
            BuiltinToolOutput::ViewFile { .. } => {}
            BuiltinToolOutput::ApplyPatch { inverse_patch, .. } => {
                *inverse_patch = truncate_text(inverse_patch, self.policy.max_chars);
            }
            BuiltinToolOutput::Glob { .. }
            | BuiltinToolOutput::Grep { .. }
            | BuiltinToolOutput::Task { .. }
            | BuiltinToolOutput::ToolSearch { .. }
            | BuiltinToolOutput::TodoWrite { .. }
            | BuiltinToolOutput::AskUser { .. }
            | BuiltinToolOutput::Monitor { .. }
            | BuiltinToolOutput::WebFetch { .. }
            | BuiltinToolOutput::WebSearch { .. } => {}
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
