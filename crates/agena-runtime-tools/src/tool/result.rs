use std::collections::BTreeMap;

use crate::part::AttachmentItem;
use agena_domain::ToolOutput;
use agena_tool::ToolPresentationSection;

use super::ToolPayloadOutput;
use agena_tool::ApplyPatchExecution;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
/// View of a tool execution result.
pub struct ToolExecutionView {
    pub title: String,
    pub summary: String,
    pub output_text: String,
    pub sections: Vec<ToolPresentationSection>,
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
            sections: Vec::new(),
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
            sections: self.sections.clone(),
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

#[cfg(test)]
mod tests {
    use super::ToolExecutionView;
    use agena_tool::ToolPresentationSection;

    #[test]
    fn view_projects_stable_fields_into_tool_contract() {
        let mut view = ToolExecutionView::simple("title", "one-line summary", "output");
        view.sections.push(ToolPresentationSection {
            title: "Files".to_owned(),
            text: "README.md".to_owned(),
        });
        view.metadata
            .insert("path".to_owned(), "README.md".to_owned());
        let summary = view.summary();
        assert_eq!(summary.title, "title");
        assert_eq!(summary.summary, "one-line summary");
        assert_eq!(summary.output_text, "output");
        assert_eq!(summary.sections[0].title, "Files");
        assert_eq!(summary.metadata["path"], "README.md");
    }

    #[test]
    fn view_applies_neutral_fields_without_replacing_core_attachments() {
        let mut view = ToolExecutionView::simple("old", "old summary", "old output");
        view.metadata
            .insert("existing".to_owned(), "yes".to_owned());
        view.apply_neutral_fields(
            "new".to_owned(),
            "new summary".to_owned(),
            "new output".to_owned(),
            [("hook".to_owned(), "applied".to_owned())],
        );
        assert_eq!(view.title, "new");
        assert_eq!(view.summary, "new summary");
        assert_eq!(view.output_text, "new output");
        assert_eq!(view.metadata["existing"], "yes");
        assert_eq!(view.metadata["hook"], "applied");
        assert!(view.attachments.is_empty());
    }

    #[test]
    fn execution_results_expose_the_same_neutral_summary() {
        let view = ToolExecutionView::simple("title", "summary", "output");
        let payload = super::ToolPayloadExecution {
            output: super::ToolPayloadOutput::Read {
                preview: None,
                truncated: false,
                loaded_paths: Vec::new(),
                attachment: None,
            },
            view: view.clone(),
            apply_patch: None,
        };
        assert_eq!(payload.summary(), view.summary());
        let invocation = super::ToolInvocationExecution {
            output: agena_domain::ToolOutput::default(),
            view: view.clone(),
            apply_patch: None,
        };
        assert_eq!(invocation.summary(), view.summary());
    }

    #[test]
    fn view_projects_attachment_metadata_without_sdk_source_types() {
        let view = ToolExecutionView {
            attachments: vec![crate::part::AttachmentItem {
                kind: crate::part::AttachmentKind::File,
                mime: "text/plain".to_owned(),
                source: crate::part::AttachmentSource::LocalPath {
                    path: "README.md".to_owned(),
                },
                filename: Some("README.md".to_owned()),
                title: None,
                size_bytes: Some(12),
                sha256: None,
                width: None,
                height: None,
                duration_ms: None,
                page_count: None,
            }],
            ..ToolExecutionView::simple("title", "summary", "output")
        };
        let attachment = &view.summary().attachments[0];
        assert_eq!(attachment.kind, "file");
        assert_eq!(attachment.label, "README.md");
        assert_eq!(attachment.source_hint.as_deref(), Some("README.md"));
        assert_eq!(attachment.size_bytes, Some(12));
    }
}
