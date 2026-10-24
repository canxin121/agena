use std::collections::BTreeMap;

use crate::part::AttachmentItem;
use agena_domain::ToolOutput;

use super::ToolPayloadOutput;
use agena_tool::ApplyPatchExecution;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
/// View of a tool execution result.
pub struct ToolExecutionView {
    pub title: String,
    pub summary: String,
    pub output_text: String,
    pub metadata: BTreeMap<String, String>,
    pub attachments: Vec<AttachmentItem>,
}

impl ToolExecutionView {
    pub fn simple(
        title: impl Into<String>,
        summary: impl Into<String>,
        output_text: impl Into<String>,
    ) -> Self {
        Self {
            title: agena_tool::normalize_tool_title(title.into()),
            summary: agena_tool::normalize_tool_summary(summary.into()),
            output_text: output_text.into(),
            metadata: BTreeMap::new(),
            attachments: Vec::new(),
        }
    }

    /// Project the core presentation view into the runtime-neutral tool
    /// execution contract. Runtime-private attachments intentionally stay on this type.
    pub fn summary(&self) -> agena_tool::ToolExecutionSummary {
        agena_tool::ToolExecutionSummary {
            title: self.title.clone(),
            summary: self.summary.clone(),
            output_text: self.output_text.clone(),
            payload: None,
            metadata: self.metadata.clone(),
            attachments: self
                .attachments
                .iter()
                .map(|attachment| agena_tool::ToolAttachmentSummary {
                    kind: attachment.kind.as_ref().to_owned(),
                    mime: attachment.mime.clone(),
                    label: attachment.summary_label(),
                    size_bytes: attachment.size_bytes,
                    source_hint: attachment.source.summary_hint().map(ToOwned::to_owned),
                })
                .collect(),
        }
    }

    pub fn raw_output(&self, output: &ToolOutput) -> agena_domain::RawOutput {
        let mut payload: serde_json::Map<String, serde_json::Value> = output
            .payload
            .fields
            .iter()
            .map(|field| {
                (
                    field.name.clone(),
                    serde_json::Value::from(field.value.clone()),
                )
            })
            .collect();
        // Presentation metadata is string-valued. Keeping it out of the
        // result's field namespace also preserves typed defaults that serde
        // omits, such as Read::truncated=false. Tool-owned metadata wins.
        if !self.metadata.is_empty()
            && let serde_json::Value::Object(metadata) = payload
                .entry("metadata".to_owned())
                .or_insert_with(|| serde_json::Value::Object(Default::default()))
        {
            for (key, value) in &self.metadata {
                metadata
                    .entry(key.clone())
                    .or_insert_with(|| serde_json::Value::String(value.clone()));
            }
        }
        if output.payload.is_empty() && !self.output_text.is_empty() {
            payload
                .entry("text".to_owned())
                .or_insert_with(|| serde_json::Value::String(self.output_text.clone()));
        }
        agena_domain::RawOutput::from_parts(
            (!payload.is_empty()).then_some(serde_json::Value::Object(payload)),
            self.output_text.clone(),
            self.attachments.clone(),
            output.managed_outputs.clone(),
            output.truncated,
        )
    }

    /// Apply presentation fields returned by a runtime/plugin boundary.
    /// Concrete attachments remain owned by the core execution view.
    pub fn apply_neutral_fields(
        &mut self,
        title: String,
        summary: String,
        output_text: String,
        metadata: impl IntoIterator<Item = (String, String)>,
    ) {
        self.title = agena_tool::normalize_tool_title(title);
        self.summary = agena_tool::normalize_tool_summary(summary);
        self.output_text = output_text;
        self.metadata.extend(metadata);
    }

    pub fn set_neutral_output(&mut self, output_text: String) {
        self.output_text = output_text;
    }

    pub fn set_title(&mut self, title: impl Into<String>) {
        self.title = agena_tool::normalize_tool_title(title.into());
    }

    pub fn normalize_presentation(&mut self) {
        self.title = agena_tool::normalize_tool_title(&self.title);
        self.summary = agena_tool::normalize_tool_summary(&self.summary);
    }

    pub fn insert_neutral_metadata(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.metadata.insert(key.into(), value.into());
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Result of a payload-based tool execution.
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

    /// Project the executor result into the runtime-neutral summary contract.
    pub fn summary(&self) -> agena_tool::ToolExecutionSummary {
        let mut summary = self.view.summary();
        summary.payload = self.output.clone().into_tool_output().to_json_payload();
        summary
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Result of a tool invocation execution.
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
        let output = value.output.into_tool_output();
        Self {
            output,
            view: value.view,
            apply_patch: value.apply_patch,
        }
    }
}

impl ToolInvocationExecution {
    /// Project the executor result into the runtime-neutral summary contract.
    pub fn summary(&self) -> agena_tool::ToolExecutionSummary {
        let mut summary = self.view.summary();
        summary.payload = self.output.to_json_payload();
        summary
    }
}
