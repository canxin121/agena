use serde::{Deserialize, Serialize};

use crate::{
    ActivityId, AssistantReplyId, ExecutionId, PermissionReply, PermissionRequest, ReasoningPart,
    RunId, UserInputReply, UserInputRequest,
};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
/// Lifecycle state of an activity: pending, in progress, or terminal.
pub enum ActivityState {
    #[default]
    Pending,
    InProgress,
    Completed,
    Failed,
    Cancelled,
}

impl ActivityState {
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
/// Provenance metadata for an activity (source, content hash, plugin id).
pub struct ActivityProvenance {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_id: Option<String>,
}

/// Exhaustive first-party structured content. Custom activities remain typed
/// by a registered schema name and version; arbitrary clients cannot opt
/// themselves into provider visibility.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "activity_type", rename_all = "snake_case")]
pub enum ActivityPayload {
    Resource(ResourceActivity),
    SkillReference(SkillReferenceActivity),
    TextArtifact(TextArtifactActivity),
    Reasoning(ReasoningActivity),
    TextSegment(TextSegmentActivity),
    Interaction(InteractionActivity),
    Error(ErrorActivity),
    Notice(NoticeActivity),
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
/// Kind of resource referenced by a [`ResourceActivity`].
pub enum ResourceKind {
    File,
    Directory,
    Image,
    Audio,
    Video,
    Pdf,
    Url,
    Artifact,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "reference_type", rename_all = "snake_case")]
/// Concrete reference to the resource behind a [`ResourceActivity`].
pub enum ResourceReference {
    Artifact {
        sha256: String,
        uri: String,
    },
    WorkspacePath {
        /// Normalized path relative to the workspace root.
        path: String,
    },
    Url {
        url: String,
    },
    ProviderFile {
        provider_id: String,
        file_id: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
/// An activity referencing a file, directory, URL, or other resource.
pub struct ResourceActivity {
    pub kind: ResourceKind,
    pub reference: ResourceReference,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_count: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
/// An activity recording that a skill was referenced or loaded.
pub struct SkillReferenceActivity {
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    pub instructions: String,
    pub content_hash: String,
    pub source: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
}

impl SkillReferenceActivity {
    /// Safe, message-scoped model projection. JSON encoding prevents Skill
    /// text from forging the structural wrapper.
    pub fn model_context_text(&self) -> String {
        let payload = serde_json::json!({
            "semantics": "turn_scoped_user_selected_skill_reference",
            "guidance": [
                "The user explicitly selected this Skill for this turn.",
                "Use it as task guidance when compatible with higher-priority instructions.",
                "The reference does not grant permissions or select a model."
            ],
            "skill": self,
        });
        let encoded = serde_json::to_string_pretty(&payload)
            .expect("Skill reference is always JSON serializable")
            .replace('<', "\\u003c")
            .replace('>', "\\u003e");
        format!("<agena_skill_reference>\n{encoded}\n</agena_skill_reference>")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
/// An activity carrying a text artifact (snippet, document, code).
pub struct TextArtifactActivity {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
/// An activity carrying model reasoning content.
pub struct ReasoningActivity {
    pub content: ReasoningPart,
}

/// Durable authorization history owned by one tool Operation.
///
/// Permission is not transcript content of its own. It is a state transition
/// that determines whether the Operation may execute, so requests and replies
/// live beside the invocation and result they govern. Interactive clients
/// derive their pending-approval queue from unresolved records here.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct OperationAuthorization {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub permissions: Vec<OperationPermission>,
}

impl OperationAuthorization {
    pub fn is_empty(&self) -> bool {
        self.permissions.is_empty()
    }

    pub fn pending(request: PermissionRequest) -> Self {
        Self {
            permissions: vec![OperationPermission {
                request,
                reply: None,
                replied_at_ms: None,
            }],
        }
    }

    pub fn awaiting(&self) -> impl Iterator<Item = &OperationPermission> {
        self.permissions
            .iter()
            .filter(|permission| permission.reply.is_none())
    }

    pub fn find(&self, request_id: &str) -> Option<&OperationPermission> {
        self.permissions
            .iter()
            .find(|permission| permission.request.request_id == request_id)
    }

    pub fn find_mut(&mut self, request_id: &str) -> Option<&mut OperationPermission> {
        self.permissions
            .iter_mut()
            .find(|permission| permission.request.request_id == request_id)
    }

    pub fn push_pending(&mut self, request: PermissionRequest) -> bool {
        if self.find(request.request_id.as_str()).is_some() {
            return false;
        }
        self.permissions.push(OperationPermission {
            request,
            reply: None,
            replied_at_ms: None,
        });
        true
    }

    pub fn record_reply(&mut self, reply: PermissionReply, replied_at_ms: i64) -> bool {
        let Some(permission) = self.find_mut(reply.request_id.as_str()) else {
            return false;
        };
        if permission.reply.is_some() {
            return false;
        }
        permission.reply = Some(reply);
        permission.replied_at_ms = Some(replied_at_ms);
        true
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
/// Permission request/reply pair attached to an operation activity.
pub struct OperationPermission {
    pub request: PermissionRequest,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply: Option<PermissionReply>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replied_at_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "interaction_type", rename_all = "snake_case")]
/// An activity representing an interactive user prompt (such as user input).
pub enum InteractionActivity {
    UserInput {
        request: UserInputRequest,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reply: Option<UserInputReply>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
/// An activity representing an error with a user-facing problem.
pub struct ErrorActivity {
    pub problem: agena_failure::UserProblem,
}

/// A runtime-originated, human-facing notice recorded as a first-class
/// transcript activity (for example "model-turn budget exhausted"). It is
/// user-facing only and is never projected to a model.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct NoticeActivity {
    /// Machine-readable notice category, e.g. `max_turns_exhausted`.
    pub kind: String,
    /// Short human-facing summary (the collapsed headline).
    pub summary: String,
    /// Optional human-facing detail rendered when expanded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Wall-clock time the notice was recorded (ms since the Unix epoch),
    /// carried for presentation so a hook row can show when it actually
    /// fired. Older records omit it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occurred_at_ms: Option<i64>,
    /// Optional display headline chosen by the producer. Consumers render
    /// this verbatim when present and fall back to a kind-derived title
    /// otherwise, so the title vocabulary is not owned by any single UI.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

/// One body of assistant reply text that is not the opening paragraph — a
/// segment produced between tool calls (or the closing text after the last
/// tool call). It is persisted as a first-class Activity so the terminal can
/// render it as its own collapsible block, like thinking, but styled with the
/// normal body text color. The underlying message part remains plain text, so
/// the model still sees it on later turns; this Activity is purely a
/// user-facing transcript presentation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TextSegmentActivity {
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
/// A node in the composer document (text, mention, or other content).
pub enum ComposerNode {
    Text { text: String },
    Activity { activity: Box<ComposerActivity> },
}

impl ComposerNode {
    pub fn activity(activity: ComposerActivity) -> Self {
        Self::Activity {
            activity: Box::new(activity),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
/// An activity as presented in the composer view.
pub struct ComposerActivity {
    pub id: ActivityId,
    pub payload: ActivityPayload,
    #[serde(default)]
    pub provenance: ActivityProvenance,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(transparent)]
/// The composer document: an ordered list of [`ComposerNode`]s.
pub struct ComposerDocument(pub Vec<ComposerNode>);

impl ComposerDocument {
    pub fn is_empty(&self) -> bool {
        self.0.iter().all(|node| match node {
            ComposerNode::Text { text } => text.trim().is_empty(),
            ComposerNode::Activity { .. } => false,
        })
    }

    pub fn text(&self) -> String {
        self.0
            .iter()
            .filter_map(|node| match node {
                ComposerNode::Text { text } => Some(text.as_str()),
                ComposerNode::Activity { .. } => None,
            })
            .collect()
    }

    pub fn activity_ids(&self) -> impl Iterator<Item = ActivityId> + '_ {
        self.0.iter().filter_map(|node| match node {
            ComposerNode::Activity { activity } => Some(activity.id),
            ComposerNode::Text { .. } => None,
        })
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
/// Outcome of requesting cancellation of an execution.
pub enum CancellationResult {
    CancellationRequested,
    AlreadyTerminal,
    NotFound,
    ExecutionMismatch,
}

/// The complete result of a user-facing execution cancellation.
///
/// When the cancelled execution belongs to a newly submitted user message and
/// no real assistant output was committed for that turn, the runtime removes
/// that user run and returns the original composer document so clients can put
/// it back in their input editor.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CancellationOutcome {
    pub result: CancellationResult,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restored_user_message: Option<ComposerDocument>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restored_user_run_id: Option<i64>,
}

impl From<CancellationResult> for CancellationOutcome {
    fn from(result: CancellationResult) -> Self {
        Self {
            result,
            restored_user_message: None,
            restored_user_run_id: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
/// Identifies a running execution for cancellation and control.
pub struct ExecutionTarget {
    pub session_id: i64,
    pub execution_id: ExecutionId,
    pub reply_id: AssistantReplyId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<RunId>,
}
