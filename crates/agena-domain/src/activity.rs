use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{
    ActivityId, ExecutionFailureKind, ExecutionId, FileChangeRecord, PermissionReply,
    PermissionRequest, ProcessSummary, PromptCompactionActivity, ReasoningPart, ResponseId,
    ResponseSegmentId, RunId, SearchResultItem, SubtaskStatus, TodoItem, ToolCallId,
    ToolInvocation, ToolOutput, TurnId, UserInputReply, UserInputRequest,
};

/// The only two kinds of content that can appear in a turn or response.
///
/// Text intentionally stays a primitive. Every structured or stateful value,
/// including resources and Skill references, is an [`ActivityNode`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentNode {
    Text { segment: TextSegment },
    Activity { activity: Box<ActivityNode> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ContentIdentity {
    Text(ResponseSegmentId),
    Activity(ActivityId),
}

impl ContentNode {
    pub fn text(text: impl Into<String>) -> Self {
        Self::text_at(ResponseSegmentId::new(), text, 0, 0)
    }

    pub fn text_at(
        id: ResponseSegmentId,
        text: impl Into<String>,
        position: u32,
        revision_seq: i64,
    ) -> Self {
        Self::Text {
            segment: TextSegment {
                id,
                text: text.into(),
                position: ContentPosition { index: position },
                revision_seq,
            },
        }
    }

    pub fn activity(activity: ActivityNode) -> Self {
        Self::Activity {
            activity: Box::new(activity),
        }
    }

    pub const fn position(&self) -> u32 {
        match self {
            Self::Text { segment } => segment.position.index,
            Self::Activity { activity } => activity.position.index,
        }
    }

    pub const fn revision_seq(&self) -> i64 {
        match self {
            Self::Text { segment } => segment.revision_seq,
            Self::Activity { activity } => activity.revision_seq,
        }
    }

    pub fn set_position(&mut self, position: u32) {
        match self {
            Self::Text { segment } => segment.position.index = position,
            Self::Activity { activity } => activity.position.index = position,
        }
    }

    pub fn same_identity(&self, other: &Self) -> bool {
        self.identity() == other.identity()
    }

    const fn identity(&self) -> ContentIdentity {
        match self {
            Self::Text { segment } => ContentIdentity::Text(segment.id),
            Self::Activity { activity } => ContentIdentity::Activity(activity.id),
        }
    }
}

/// A stable, independently updatable primitive text segment.
///
/// Text is intentionally not an Activity, but it still needs identity and a
/// revision so streamed updates and durable snapshots converge without
/// timestamp, message-adjacency, or role-based guesses.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TextSegment {
    pub id: ResponseSegmentId,
    pub text: String,
    pub position: ContentPosition,
    pub revision_seq: i64,
}

/// An ordered document. Vector order is semantic and is preserved all the way
/// from the composer to provider projection and transcript rendering.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(transparent)]
pub struct ContentDocument(pub Vec<ContentNode>);

impl ContentDocument {
    pub fn new(nodes: Vec<ContentNode>) -> Self {
        Self(nodes)
    }

    pub fn nodes(&self) -> &[ContentNode] {
        self.0.as_slice()
    }

    pub fn push(&mut self, node: ContentNode) {
        self.0.push(node);
    }

    pub fn is_empty(&self) -> bool {
        self.0.iter().all(|node| match node {
            ContentNode::Text { segment } => segment.text.trim().is_empty(),
            ContentNode::Activity { .. } => false,
        })
    }

    /// User-authored text only. Render labels for activities never leak into
    /// the body passed to hooks or providers.
    pub fn text(&self) -> String {
        self.0
            .iter()
            .filter_map(|node| match node {
                ContentNode::Text { segment } => Some(segment.text.as_str()),
                ContentNode::Activity { .. } => None,
            })
            .collect()
    }

    pub fn upsert(&mut self, mut incoming: ContentNode) {
        if let Some(current) = self
            .0
            .iter_mut()
            .find(|current| current.same_identity(&incoming))
        {
            if incoming.revision_seq() >= current.revision_seq() {
                // Ownership and position are immutable after insertion. A
                // live producer does not need to rediscover the response-wide
                // position merely to update a node.
                incoming.set_position(current.position());
                *current = incoming;
            }
            return;
        }

        // Patches are reduced in the event stream's monotonic sequence. New
        // live nodes therefore append to the owner's content order; durable
        // snapshots carry their already-canonical position.
        let next_position = self
            .0
            .iter()
            .map(ContentNode::position)
            .max()
            .map_or(0, |position| position.saturating_add(1));
        incoming.set_position(next_position);
        self.0.push(incoming);
        self.0.sort_by_key(ContentNode::position);
    }

    pub fn merge_from(&mut self, incoming: ContentDocument) {
        let mut canonical_identities = Vec::with_capacity(incoming.0.len());
        for node in incoming.0 {
            let identity = node.identity();
            canonical_identities.push(identity);
            if let Some(current) = self
                .0
                .iter_mut()
                .find(|current| current.identity() == identity)
            {
                let canonical_position = node.position();
                if node.revision_seq() >= current.revision_seq() {
                    *current = node;
                }
                // Position is immutable canonical ownership metadata, not
                // revisioned streamed content. A durable snapshot may correct
                // a temporary append position even when the local text or
                // Activity revision is newer.
                current.set_position(canonical_position);
            } else {
                self.0.push(node);
            }
        }

        // Durable nodes win position ties over local-only live nodes. Such a
        // tie means the live node was appended before the durable projection
        // revealed an earlier canonical sibling. Stable sorting preserves the
        // relative event order of all remaining live-only nodes.
        self.0.sort_by(|left, right| {
            left.position().cmp(&right.position()).then_with(|| {
                let left_canonical = canonical_identities.contains(&left.identity());
                let right_canonical = canonical_identities.contains(&right.identity());
                right_canonical.cmp(&left_canonical)
            })
        });
        for (position, node) in self.0.iter_mut().enumerate() {
            node.set_position(u32::try_from(position).unwrap_or(u32::MAX));
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ActivityOwner {
    TurnInput { turn_id: TurnId },
    Response { response_id: ResponseId },
    Activity { parent_activity_id: ActivityId },
    Session { session_id: i64 },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActivityActor {
    User,
    Assistant,
    Runtime,
    Tool,
    Plugin,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContentPosition {
    /// Zero-based position inside the owner's ordered content sequence.
    pub index: u32,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActivityLifecycle {
    pub started_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at_ms: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ActivityProvenance {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ActivityNode {
    pub id: ActivityId,
    pub owner: ActivityOwner,
    pub actor: ActivityActor,
    pub payload: ActivityPayload,
    pub state: ActivityState,
    pub position: ContentPosition,
    pub revision_seq: i64,
    pub lifecycle: ActivityLifecycle,
    #[serde(default)]
    pub provenance: ActivityProvenance,
}

impl ActivityNode {
    pub fn new(
        id: ActivityId,
        owner: ActivityOwner,
        actor: ActivityActor,
        payload: ActivityPayload,
        state: ActivityState,
        position: u32,
        started_at_ms: i64,
    ) -> Self {
        Self {
            id,
            owner,
            actor,
            payload,
            state,
            position: ContentPosition { index: position },
            revision_seq: 0,
            lifecycle: ActivityLifecycle {
                started_at_ms,
                finished_at_ms: state.is_terminal().then_some(started_at_ms),
            },
            provenance: ActivityProvenance::default(),
        }
    }

    pub fn transition(&mut self, next: ActivityState, at_ms: i64, revision_seq: i64) {
        self.state = next;
        self.revision_seq = revision_seq;
        self.lifecycle.finished_at_ms = next.is_terminal().then_some(at_ms);
    }
}

/// Exhaustive first-party structured content. Custom activities remain typed
/// by a registered schema name and version; arbitrary clients cannot opt
/// themselves into provider visibility.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "activity_type", rename_all = "snake_case")]
pub enum ActivityPayload {
    Resource(ResourceActivity),
    SkillReference(SkillReferenceActivity),
    SkillExecution(SkillExecutionActivity),
    TextArtifact(TextArtifactActivity),
    Reasoning(ReasoningActivity),
    Operation(OperationActivity),
    Interaction(InteractionActivity),
    Progress(ProgressActivity),
    Checklist(ChecklistActivity),
    Search(SearchActivity),
    FileChanges(FileChangesActivity),
    NestedTask(NestedTaskActivity),
    Maintenance(MaintenanceActivity),
    Error(ErrorActivity),
    Custom(CustomActivity),
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
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
pub struct SkillExecutionActivity {
    pub skill_name: String,
    pub execution_id: ExecutionId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_activity_id: Option<ActivityId>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TextArtifactActivity {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ReasoningActivity {
    pub content: ReasoningPart,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct OperationActivity {
    pub call_id: ToolCallId,
    pub invocation: ToolInvocation,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub summary: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub model_output_text: String,
    #[serde(default, skip_serializing_if = "ToolOutput::is_empty")]
    pub details: ToolOutput,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resource_activity_ids: Vec<ActivityId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<OperationActivityError>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct OperationActivityError {
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "interaction_type", rename_all = "snake_case")]
pub enum InteractionActivity {
    Permission {
        request: PermissionRequest,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reply: Option<PermissionReply>,
    },
    UserInput {
        request: UserInputRequest,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reply: Option<UserInputReply>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProgressActivity {
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub detail: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ChecklistActivity {
    pub items: Vec<TodoItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SearchActivity {
    pub query: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub results: Vec<SearchResultItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FileChangesActivity {
    pub changes: Vec<FileChangeRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct NestedTaskActivity {
    pub task_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<i64>,
    pub status: SubtaskStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "maintenance_type", rename_all = "snake_case")]
pub enum MaintenanceActivity {
    Compaction {
        execution_id: ExecutionId,
        activity: PromptCompactionActivity,
    },
    Process {
        process: ProcessSummary,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ErrorActivity {
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_kind: Option<ExecutionFailureKind>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CustomActivity {
    pub schema: String,
    pub schema_version: u32,
    pub data: serde_json::Value,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub presentation: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ResponseStatus {
    #[default]
    Pending,
    InProgress,
    Completed,
    Failed,
    Cancelled,
}

impl ResponseStatus {
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ResponseSnapshot {
    pub id: ResponseId,
    pub turn_id: TurnId,
    pub execution_id: ExecutionId,
    pub status: ResponseStatus,
    pub content: ContentDocument,
    pub revision_seq: i64,
    pub created_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TurnSnapshot {
    pub id: TurnId,
    pub session_id: i64,
    pub sequence: i64,
    pub input: ContentDocument,
    pub response: ResponseSnapshot,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TranscriptSnapshot {
    pub session_id: i64,
    pub seq_session: i64,
    pub turns: Vec<TurnSnapshot>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub session_activities: Vec<ActivityNode>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TranscriptPatch {
    TurnOpened {
        seq_session: i64,
        turn: TurnSnapshot,
    },
    ResponseUpdated {
        seq_session: i64,
        response: ResponseSnapshot,
    },
    ContentUpserted {
        seq_session: i64,
        owner: ActivityOwner,
        node: ContentNode,
    },
}

impl TranscriptSnapshot {
    pub fn apply(&mut self, patch: TranscriptPatch) {
        let seq_session = match &patch {
            TranscriptPatch::TurnOpened { seq_session, .. }
            | TranscriptPatch::ResponseUpdated { seq_session, .. }
            | TranscriptPatch::ContentUpserted { seq_session, .. } => *seq_session,
        };
        if seq_session <= self.seq_session {
            return;
        }
        if let TranscriptPatch::ContentUpserted {
            owner,
            node: ContentNode::Activity { activity },
            ..
        } = &patch
            && activity.owner != *owner
        {
            return;
        }
        match patch {
            TranscriptPatch::TurnOpened { turn, .. } => {
                if let Some(existing) = self.turns.iter_mut().find(|item| item.id == turn.id) {
                    merge_matching_turn(existing, turn);
                } else {
                    self.turns.push(turn);
                    self.turns.sort_by_key(|turn| turn.sequence);
                }
            }
            TranscriptPatch::ResponseUpdated { mut response, .. } => {
                if let Some(turn) = self
                    .turns
                    .iter_mut()
                    .find(|turn| turn.id == response.turn_id)
                    && turn.response.id == response.id
                    && turn.response.execution_id == response.execution_id
                    && response.revision_seq >= turn.response.revision_seq
                {
                    let mut content = std::mem::take(&mut turn.response.content);
                    content.merge_from(std::mem::take(&mut response.content));
                    response.content = content;
                    turn.response = response;
                }
            }
            TranscriptPatch::ContentUpserted { owner, node, .. } => match owner {
                ActivityOwner::TurnInput { turn_id } => {
                    if let Some(turn) = self.turns.iter_mut().find(|turn| turn.id == turn_id) {
                        turn.input.upsert(node);
                    }
                }
                ActivityOwner::Response { response_id } => {
                    if let Some(turn) = self
                        .turns
                        .iter_mut()
                        .find(|turn| turn.response.id == response_id)
                    {
                        turn.response.content.upsert(node);
                    }
                }
                ActivityOwner::Session { session_id } if session_id == self.session_id => {
                    if let ContentNode::Activity { activity } = node {
                        upsert_activity(&mut self.session_activities, *activity);
                    }
                }
                ActivityOwner::Activity { .. } | ActivityOwner::Session { .. } => {}
            },
        }
        self.seq_session = seq_session;
    }

    /// Reconcile a durable snapshot with newer live content already reduced
    /// locally. Identity and revision decide every replacement; timestamps do
    /// not participate.
    pub fn merge(&mut self, incoming: TranscriptSnapshot) {
        if self.session_id != incoming.session_id {
            *self = incoming;
            return;
        }

        for incoming_turn in incoming.turns {
            if let Some(current) = self
                .turns
                .iter_mut()
                .find(|turn| turn.id == incoming_turn.id)
            {
                merge_matching_turn(current, incoming_turn);
            } else {
                self.turns.push(incoming_turn);
            }
        }
        self.turns.sort_by_key(|turn| turn.sequence);
        for activity in incoming.session_activities {
            upsert_activity(&mut self.session_activities, activity);
        }
        self.seq_session = self.seq_session.max(incoming.seq_session);
    }
}

/// Merge two observations of the same turn without ever replacing the
/// identity-bearing turn envelope or discarding content reduced locally.
///
/// A `TurnOpened` patch and a durable refresh are merely two sources of the
/// same projection. Keeping their reconciliation here prevents either source
/// from becoming a special overwrite path. Response metadata follows the
/// response revision, while every content node follows its own stable identity
/// and revision.
fn merge_matching_turn(current: &mut TurnSnapshot, mut incoming: TurnSnapshot) {
    if current.response.id != incoming.response.id
        || current.response.execution_id != incoming.response.execution_id
        || current.response.turn_id != incoming.response.turn_id
    {
        return;
    }

    current.input.merge_from(incoming.input);
    if incoming.response.revision_seq >= current.response.revision_seq {
        let mut content = std::mem::take(&mut current.response.content);
        content.merge_from(std::mem::take(&mut incoming.response.content));
        current.response = incoming.response;
        current.response.content = content;
    } else {
        current
            .response
            .content
            .merge_from(incoming.response.content);
    }
}

fn upsert_activity(activities: &mut Vec<ActivityNode>, activity: ActivityNode) {
    if let Some(current) = activities.iter_mut().find(|item| item.id == activity.id) {
        if activity.revision_seq >= current.revision_seq {
            *current = activity;
        }
    } else {
        activities.push(activity);
        activities.sort_by_key(|item| item.position.index);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
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
pub struct ComposerActivity {
    pub id: ActivityId,
    pub payload: ActivityPayload,
    #[serde(default)]
    pub provenance: ActivityProvenance,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(transparent)]
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

    pub fn into_turn_input(self, turn_id: TurnId, started_at_ms: i64) -> ContentDocument {
        ContentDocument::new(
            self.0
                .into_iter()
                .enumerate()
                .map(|(index, node)| match node {
                    ComposerNode::Text { text } => {
                        ContentNode::text_at(ResponseSegmentId::new(), text, index as u32, 0)
                    }
                    ComposerNode::Activity { activity } => ContentNode::activity(ActivityNode {
                        id: activity.id,
                        owner: ActivityOwner::TurnInput { turn_id },
                        actor: ActivityActor::User,
                        payload: activity.payload,
                        state: ActivityState::Completed,
                        position: ContentPosition {
                            index: index as u32,
                        },
                        revision_seq: 0,
                        lifecycle: ActivityLifecycle {
                            started_at_ms,
                            finished_at_ms: Some(started_at_ms),
                        },
                        provenance: activity.provenance,
                    }),
                })
                .collect(),
        )
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CancellationResult {
    CancellationRequested,
    AlreadyTerminal,
    NotFound,
    ExecutionMismatch,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ExecutionTarget {
    pub session_id: i64,
    pub execution_id: ExecutionId,
    pub response_id: ResponseId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<RunId>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn skill(name: &str) -> ComposerActivity {
        ComposerActivity {
            id: ActivityId::new(),
            payload: ActivityPayload::SkillReference(SkillReferenceActivity {
                name: name.to_owned(),
                description: String::new(),
                instructions: "Do the work.".to_owned(),
                content_hash: "sha256:test".to_owned(),
                source: "test".to_owned(),
                aliases: Vec::new(),
            }),
            provenance: ActivityProvenance::default(),
        }
    }

    #[test]
    fn composer_document_preserves_interleaving_without_leaking_labels_into_text() {
        let activity = skill("review");
        let id = activity.id;
        let document = ComposerDocument(vec![
            ComposerNode::Text { text: "hi ".into() },
            ComposerNode::activity(activity),
            ComposerNode::Text {
                text: " there".into(),
            },
        ]);

        assert_eq!(document.text(), "hi  there");
        assert_eq!(document.activity_ids().collect::<Vec<_>>(), vec![id]);

        let turn_id = TurnId::new();
        let input = document.into_turn_input(turn_id, 7);
        assert!(matches!(
            &input.0[1],
            ContentNode::Activity { activity }
                if activity.id == id
                    && activity.owner == ActivityOwner::TurnInput { turn_id }
                    && activity.position.index == 1
        ));
    }

    #[test]
    fn snapshot_reducer_uses_identity_and_revision_not_timestamps() {
        let turn_id = TurnId::new();
        let response_id = ResponseId::new();
        let execution_id = ExecutionId::new();
        let response = ResponseSnapshot {
            id: response_id,
            turn_id,
            execution_id,
            status: ResponseStatus::InProgress,
            content: ContentDocument::default(),
            revision_seq: 1,
            created_at_ms: 10,
            finished_at_ms: None,
        };
        let mut snapshot = TranscriptSnapshot {
            session_id: 1,
            seq_session: 0,
            turns: vec![TurnSnapshot {
                id: turn_id,
                session_id: 1,
                sequence: 1,
                input: ContentDocument::default(),
                response,
                created_at_ms: 10,
            }],
            session_activities: Vec::new(),
        };
        snapshot.apply(TranscriptPatch::ResponseUpdated {
            seq_session: 2,
            response: ResponseSnapshot {
                id: response_id,
                turn_id,
                execution_id,
                status: ResponseStatus::Cancelled,
                content: ContentDocument::default(),
                revision_seq: 2,
                created_at_ms: 10,
                finished_at_ms: Some(5),
            },
        });
        assert_eq!(snapshot.turns[0].response.status, ResponseStatus::Cancelled);
    }

    #[test]
    fn snapshot_replay_converges_by_turn_response_and_segment_identity() {
        let turn_id = TurnId::new();
        let response_id = ResponseId::new();
        let execution_id = ExecutionId::new();
        let segment_id = ResponseSegmentId::new();
        let opened = TurnSnapshot {
            id: turn_id,
            session_id: 7,
            sequence: 1,
            input: ContentDocument::new(vec![ContentNode::text_at(
                ResponseSegmentId::new(),
                "question",
                0,
                1,
            )]),
            response: ResponseSnapshot {
                id: response_id,
                turn_id,
                execution_id,
                status: ResponseStatus::InProgress,
                content: ContentDocument::default(),
                revision_seq: 1,
                created_at_ms: 100,
                finished_at_ms: None,
            },
            created_at_ms: 100,
        };
        let patches = vec![
            TranscriptPatch::TurnOpened {
                seq_session: 1,
                turn: opened.clone(),
            },
            TranscriptPatch::ContentUpserted {
                seq_session: 2,
                owner: ActivityOwner::Response { response_id },
                node: ContentNode::text_at(segment_id, "partial", 99, 2),
            },
            TranscriptPatch::ContentUpserted {
                seq_session: 3,
                owner: ActivityOwner::Response { response_id },
                node: ContentNode::text_at(segment_id, "partial answer", 0, 3),
            },
            TranscriptPatch::ResponseUpdated {
                seq_session: 4,
                response: ResponseSnapshot {
                    id: response_id,
                    turn_id,
                    execution_id,
                    status: ResponseStatus::Cancelled,
                    content: ContentDocument::new(vec![ContentNode::text_at(
                        segment_id,
                        "partial answer",
                        0,
                        3,
                    )]),
                    revision_seq: 4,
                    // Deliberately earlier than creation: timestamps do not
                    // decide ordering or ownership.
                    created_at_ms: 100,
                    finished_at_ms: Some(50),
                },
            },
        ];
        let mut replayed = TranscriptSnapshot {
            session_id: 7,
            ..Default::default()
        };
        for patch in patches {
            replayed.apply(patch);
        }

        let expected = TranscriptSnapshot {
            session_id: 7,
            seq_session: 4,
            turns: vec![TurnSnapshot {
                response: ResponseSnapshot {
                    id: response_id,
                    turn_id,
                    execution_id,
                    status: ResponseStatus::Cancelled,
                    content: ContentDocument::new(vec![ContentNode::text_at(
                        segment_id,
                        "partial answer",
                        0,
                        3,
                    )]),
                    revision_seq: 4,
                    created_at_ms: 100,
                    finished_at_ms: Some(50),
                },
                ..opened
            }],
            session_activities: Vec::new(),
        };
        assert_eq!(replayed, expected);
    }

    #[test]
    fn separate_cancelled_responses_never_overwrite_each_other() {
        let mut snapshot = TranscriptSnapshot {
            session_id: 9,
            ..Default::default()
        };
        let mut response_ids = Vec::new();
        for sequence in 1..=2 {
            let turn_id = TurnId::new();
            let response_id = ResponseId::new();
            response_ids.push(response_id);
            let execution_id = ExecutionId::new();
            snapshot.apply(TranscriptPatch::TurnOpened {
                seq_session: sequence * 2 - 1,
                turn: TurnSnapshot {
                    id: turn_id,
                    session_id: 9,
                    sequence,
                    input: ContentDocument::new(vec![ContentNode::text(format!(
                        "question {sequence}"
                    ))]),
                    response: ResponseSnapshot {
                        id: response_id,
                        turn_id,
                        execution_id,
                        status: ResponseStatus::InProgress,
                        content: ContentDocument::default(),
                        revision_seq: sequence * 2 - 1,
                        created_at_ms: 100 - sequence,
                        finished_at_ms: None,
                    },
                    created_at_ms: 100 - sequence,
                },
            });
            snapshot.apply(TranscriptPatch::ResponseUpdated {
                seq_session: sequence * 2,
                response: ResponseSnapshot {
                    id: response_id,
                    turn_id,
                    execution_id,
                    status: ResponseStatus::Cancelled,
                    content: ContentDocument::default(),
                    revision_seq: sequence * 2,
                    created_at_ms: 100 - sequence,
                    finished_at_ms: Some(sequence),
                },
            });
        }
        assert_eq!(snapshot.turns.len(), 2);
        assert_eq!(snapshot.turns[0].response.id, response_ids[0]);
        assert_eq!(snapshot.turns[1].response.id, response_ids[1]);
        assert!(
            snapshot
                .turns
                .iter()
                .all(|turn| turn.response.status == ResponseStatus::Cancelled)
        );
    }

    #[test]
    fn repeated_turn_opened_merges_without_erasing_live_content() {
        let turn_id = TurnId::new();
        let response_id = ResponseId::new();
        let execution_id = ExecutionId::new();
        let question_id = ResponseSegmentId::new();
        let followup_id = ResponseSegmentId::new();
        let answer_id = ResponseSegmentId::new();
        let opened = TurnSnapshot {
            id: turn_id,
            session_id: 11,
            sequence: 1,
            input: ContentDocument::new(vec![ContentNode::text_at(question_id, "question", 0, 1)]),
            response: ResponseSnapshot {
                id: response_id,
                turn_id,
                execution_id,
                status: ResponseStatus::InProgress,
                content: ContentDocument::default(),
                revision_seq: 1,
                created_at_ms: 10,
                finished_at_ms: None,
            },
            created_at_ms: 10,
        };
        let mut snapshot = TranscriptSnapshot {
            session_id: 11,
            ..Default::default()
        };
        snapshot.apply(TranscriptPatch::TurnOpened {
            seq_session: 1,
            turn: opened.clone(),
        });
        snapshot.apply(TranscriptPatch::ContentUpserted {
            seq_session: 2,
            owner: ActivityOwner::Response { response_id },
            node: ContentNode::text_at(answer_id, "live answer", 0, 2),
        });
        snapshot.apply(TranscriptPatch::ContentUpserted {
            seq_session: 3,
            owner: ActivityOwner::TurnInput { turn_id },
            node: ContentNode::text_at(followup_id, " plus context", 1, 3),
        });

        snapshot.apply(TranscriptPatch::TurnOpened {
            seq_session: 4,
            turn: TurnSnapshot {
                response: ResponseSnapshot {
                    status: ResponseStatus::Completed,
                    revision_seq: 4,
                    finished_at_ms: Some(20),
                    ..opened.response
                },
                ..opened
            },
        });

        let turn = &snapshot.turns[0];
        assert_eq!(turn.input.text(), "question plus context");
        assert_eq!(turn.response.content.text(), "live answer");
        assert_eq!(turn.response.status, ResponseStatus::Completed);
        assert_eq!(snapshot.seq_session, 4);
    }

    #[test]
    fn durable_refresh_and_live_projection_converge_per_node_revision() {
        let turn_id = TurnId::new();
        let response_id = ResponseId::new();
        let execution_id = ExecutionId::new();
        let segment_id = ResponseSegmentId::new();
        let mut snapshot = TranscriptSnapshot {
            session_id: 12,
            seq_session: 5,
            turns: vec![TurnSnapshot {
                id: turn_id,
                session_id: 12,
                sequence: 1,
                input: ContentDocument::default(),
                response: ResponseSnapshot {
                    id: response_id,
                    turn_id,
                    execution_id,
                    status: ResponseStatus::InProgress,
                    content: ContentDocument::new(vec![ContentNode::text_at(
                        segment_id,
                        "newer live text",
                        0,
                        5,
                    )]),
                    revision_seq: 5,
                    created_at_ms: 1,
                    finished_at_ms: None,
                },
                created_at_ms: 1,
            }],
            session_activities: Vec::new(),
        };
        let refreshed_turn = |response_revision, segment_revision, text, status| TurnSnapshot {
            id: turn_id,
            session_id: 12,
            sequence: 1,
            input: ContentDocument::default(),
            response: ResponseSnapshot {
                id: response_id,
                turn_id,
                execution_id,
                status,
                content: ContentDocument::new(vec![ContentNode::text_at(
                    segment_id,
                    text,
                    0,
                    segment_revision,
                )]),
                revision_seq: response_revision,
                created_at_ms: 1,
                finished_at_ms: status.is_terminal().then_some(response_revision),
            },
            created_at_ms: 1,
        };

        snapshot.merge(TranscriptSnapshot {
            session_id: 12,
            seq_session: 6,
            turns: vec![refreshed_turn(
                6,
                4,
                "stale durable text",
                ResponseStatus::Cancelled,
            )],
            session_activities: Vec::new(),
        });
        assert_eq!(snapshot.turns[0].response.status, ResponseStatus::Cancelled);
        assert_eq!(snapshot.turns[0].response.content.text(), "newer live text");

        snapshot.merge(TranscriptSnapshot {
            session_id: 12,
            seq_session: 7,
            turns: vec![refreshed_turn(
                7,
                7,
                "terminal durable text",
                ResponseStatus::Completed,
            )],
            session_activities: Vec::new(),
        });
        assert_eq!(snapshot.turns[0].response.status, ResponseStatus::Completed);
        assert_eq!(
            snapshot.turns[0].response.content.text(),
            "terminal durable text"
        );
    }

    #[test]
    fn durable_merge_restores_canonical_order_without_downgrading_live_content() {
        let first_id = ResponseSegmentId::new();
        let second_id = ResponseSegmentId::new();
        let third_id = ResponseSegmentId::new();
        let mut document =
            ContentDocument::new(vec![ContentNode::text_at(third_id, "newer third", 0, 5)]);

        document.merge_from(ContentDocument::new(vec![
            ContentNode::text_at(first_id, "first ", 0, 3),
            ContentNode::text_at(second_id, "second ", 1, 3),
            ContentNode::text_at(third_id, "stale third", 2, 3),
        ]));

        assert_eq!(document.text(), "first second newer third");
        assert_eq!(
            document
                .nodes()
                .iter()
                .map(ContentNode::position)
                .collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
        assert!(matches!(
            &document.nodes()[2],
            ContentNode::Text { segment }
                if segment.id == third_id
                    && segment.text == "newer third"
                    && segment.revision_seq == 5
        ));
    }

    #[test]
    fn mismatched_activity_owner_is_rejected_without_consuming_sequence() {
        let turn_id = TurnId::new();
        let response_id = ResponseId::new();
        let execution_id = ExecutionId::new();
        let mut snapshot = TranscriptSnapshot {
            session_id: 13,
            seq_session: 1,
            turns: vec![TurnSnapshot {
                id: turn_id,
                session_id: 13,
                sequence: 1,
                input: ContentDocument::default(),
                response: ResponseSnapshot {
                    id: response_id,
                    turn_id,
                    execution_id,
                    status: ResponseStatus::InProgress,
                    content: ContentDocument::default(),
                    revision_seq: 1,
                    created_at_ms: 1,
                    finished_at_ms: None,
                },
                created_at_ms: 1,
            }],
            session_activities: Vec::new(),
        };
        let activity = ActivityNode {
            id: ActivityId::new(),
            owner: ActivityOwner::Response { response_id },
            actor: ActivityActor::Assistant,
            state: ActivityState::Completed,
            position: ContentPosition { index: 0 },
            revision_seq: 2,
            lifecycle: ActivityLifecycle::default(),
            payload: ActivityPayload::Error(ErrorActivity {
                code: "test".to_owned(),
                message: "must not be inserted".to_owned(),
                failure_kind: None,
            }),
            provenance: ActivityProvenance::default(),
        };

        snapshot.apply(TranscriptPatch::ContentUpserted {
            seq_session: 2,
            owner: ActivityOwner::TurnInput { turn_id },
            node: ContentNode::activity(activity),
        });

        assert_eq!(snapshot.seq_session, 1);
        assert!(snapshot.turns[0].input.nodes().is_empty());
        assert!(snapshot.turns[0].response.content.nodes().is_empty());
    }

    #[test]
    fn response_update_requires_exact_turn_response_and_execution_identity() {
        let turn_id = TurnId::new();
        let response_id = ResponseId::new();
        let execution_id = ExecutionId::new();
        let mut snapshot = TranscriptSnapshot {
            session_id: 14,
            seq_session: 1,
            turns: vec![TurnSnapshot {
                id: turn_id,
                session_id: 14,
                sequence: 1,
                input: ContentDocument::default(),
                response: ResponseSnapshot {
                    id: response_id,
                    turn_id,
                    execution_id,
                    status: ResponseStatus::InProgress,
                    content: ContentDocument::new(vec![ContentNode::text("correct response")]),
                    revision_seq: 1,
                    created_at_ms: 1,
                    finished_at_ms: None,
                },
                created_at_ms: 1,
            }],
            session_activities: Vec::new(),
        };

        snapshot.apply(TranscriptPatch::ResponseUpdated {
            seq_session: 2,
            response: ResponseSnapshot {
                id: response_id,
                turn_id,
                execution_id: ExecutionId::new(),
                status: ResponseStatus::Cancelled,
                content: ContentDocument::default(),
                revision_seq: 2,
                created_at_ms: 1,
                finished_at_ms: Some(2),
            },
        });

        assert_eq!(snapshot.turns[0].response.execution_id, execution_id);
        assert_eq!(
            snapshot.turns[0].response.status,
            ResponseStatus::InProgress
        );
        assert_eq!(
            snapshot.turns[0].response.content.text(),
            "correct response"
        );
    }
}
