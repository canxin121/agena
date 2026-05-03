use std::collections::BTreeMap;

use crate::message::{BuiltinToolOutput, ToolAttachment, ToolOutput};

use super::ApplyPatchExecution;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ToolExecutionView {
    pub title: String,
    pub output_text: String,
    pub metadata: BTreeMap<String, String>,
    pub attachments: Vec<ToolAttachment>,
}

impl ToolExecutionView {
    pub fn simple(title: impl Into<String>, output_text: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            output_text: output_text.into(),
            metadata: BTreeMap::new(),
            attachments: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuiltinExecution {
    pub output: BuiltinToolOutput,
    pub view: ToolExecutionView,
    pub apply_patch: Option<ApplyPatchExecution>,
}

impl BuiltinExecution {
    pub fn new(output: BuiltinToolOutput, view: ToolExecutionView) -> Self {
        Self {
            output,
            view,
            apply_patch: None,
        }
    }

    pub fn with_apply_patch(mut self, execution: ApplyPatchExecution) -> Self {
        self.apply_patch = Some(execution);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolInvocationExecution {
    pub output: ToolOutput,
    pub view: ToolExecutionView,
    pub apply_patch: Option<ApplyPatchExecution>,
}

impl ToolInvocationExecution {
    pub fn new(output: ToolOutput, view: ToolExecutionView) -> Self {
        Self {
            output,
            view,
            apply_patch: None,
        }
    }

    pub fn with_apply_patch(mut self, execution: ApplyPatchExecution) -> Self {
        self.apply_patch = Some(execution);
        self
    }
}

impl From<BuiltinExecution> for ToolInvocationExecution {
    fn from(value: BuiltinExecution) -> Self {
        Self {
            output: ToolOutput::Custom {
                output: value.output.into_custom_output(),
            },
            view: value.view,
            apply_patch: value.apply_patch,
        }
    }
}
