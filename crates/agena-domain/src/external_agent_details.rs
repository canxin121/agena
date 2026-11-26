use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{
    ActivityId, ActivityPayload, ComposerActivity, ComposerDocument, ComposerNode,
    ResourceActivity, ResourceDelivery, ResourceKind, ResourceReference,
};

/// External Agent facts that may change independently of the prompt text.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExternalAgentDetails {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost: Option<ExternalAgentCost>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub notices: Vec<ExternalAgentNotice>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub compactions: Vec<ExternalAgentCompaction>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub subagents: BTreeMap<String, ExternalAgentSubagent>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub messages: Vec<ExternalAgentMessage>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub suggestions: Vec<ExternalAgentSuggestion>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub terminals: Vec<ExternalAgentTerminal>,
}

impl ExternalAgentDetails {
    pub fn is_empty(&self) -> bool {
        self.cost.is_none()
            && self.notices.is_empty()
            && self.compactions.is_empty()
            && self.subagents.is_empty()
            && self.messages.is_empty()
            && self.suggestions.is_empty()
            && self.terminals.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalAgentCost {
    pub amount: serde_json::Number,
    pub currency: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalAgentNotice {
    pub severity: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalAgentCompaction {
    pub id: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExternalAgentSubagent {
    pub parent_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub additional_directories: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub messages: Vec<ExternalAgentMessage>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExternalAgentMessage {
    pub id: String,
    pub role: String,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sender_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recipient_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalAgentSuggestion {
    pub prompt: Vec<ExternalAgentSuggestionBlock>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ExternalAgentSuggestionBlock {
    Text {
        text: String,
    },
    ResourceLink {
        uri: String,
        name: String,
        #[serde(rename = "mimeType", default, skip_serializing_if = "Option::is_none")]
        mime_type: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        size: Option<u64>,
    },
}

impl ExternalAgentSuggestion {
    pub fn display_text(&self) -> String {
        use std::fmt::Write as _;

        let mut display = String::new();
        for block in &self.prompt {
            match block {
                ExternalAgentSuggestionBlock::Text { text } => display.push_str(text),
                ExternalAgentSuggestionBlock::ResourceLink { uri, name, .. } => {
                    if !display.is_empty() && !display.ends_with('\n') {
                        display.push('\n');
                    }
                    let _ = writeln!(display, "{name} · {uri}");
                }
            }
        }
        display
    }

    pub fn composer_document(&self) -> ComposerDocument {
        ComposerDocument(
            self.prompt
                .iter()
                .map(|block| match block {
                    ExternalAgentSuggestionBlock::Text { text } => {
                        ComposerNode::Text { text: text.clone() }
                    }
                    ExternalAgentSuggestionBlock::ResourceLink {
                        uri,
                        name,
                        mime_type,
                        size,
                    } => ComposerNode::activity(ComposerActivity {
                        id: ActivityId::new(),
                        payload: ActivityPayload::Resource(ResourceActivity {
                            delivery: ResourceDelivery::Reference,
                            kind: ResourceKind::Url,
                            reference: ResourceReference::Url { url: uri.clone() },
                            name: name.clone(),
                            media_type: mime_type.clone(),
                            size_bytes: *size,
                            width: None,
                            height: None,
                            duration_ms: None,
                            page_count: None,
                        }),
                        provenance: Default::default(),
                    }),
                })
                .collect(),
        )
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExternalAgentTerminal {
    pub id: String,
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rows: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cols: Option<u16>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub output: String,
    pub exited: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signal: Option<String>,
}

/// A cancellation offered by the ACP connection that currently owns this work.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalAgentWorkItem {
    pub cancel_id: uuid::Uuid,
    pub kind: ExternalAgentWorkKind,
    pub session_id: String,
    pub target_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExternalAgentWorkKind {
    ToolCall,
    Terminal,
    Subagent,
    AsyncTask,
}
