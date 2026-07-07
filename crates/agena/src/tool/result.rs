use std::collections::BTreeMap;

use crate::message::{AttachmentItem, ToolOutput};

use super::{ApplyPatchExecution, ToolPayloadOutput};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ToolExecutionView {
    pub title: String,
    pub output_text: String,
    pub metadata: BTreeMap<String, String>,
    pub attachments: Vec<AttachmentItem>,
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
pub struct ToolPayloadExecution {
    pub output: ToolPayloadOutput,
    pub view: ToolExecutionView,
    pub apply_patch: Option<ApplyPatchExecution>,
}

impl ToolPayloadExecution {
    pub fn new(output: ToolPayloadOutput, view: ToolExecutionView) -> Self {
        Self {
            output,
            view,
            apply_patch: None,
        }
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
}

impl From<ToolPayloadExecution> for ToolInvocationExecution {
    fn from(value: ToolPayloadExecution) -> Self {
        Self {
            output: value.output.into_tool_output(),
            view: value.view,
            apply_patch: value.apply_patch,
        }
    }
}
