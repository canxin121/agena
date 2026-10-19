//! Intent-based session capabilities. Business callers receive only the ports
//! they need; backend composition lives in `crate::backend::kernel`.
//!
//! Owned read values have no generic save/JSON-patch path. Executor handles
//! bind session, execution, generation and service epoch, and cannot be built
//! or deserialized by a producer.

use std::sync::Arc;

pub use agena_domain::session_kernel::*;
use agena_domain::{
    ComposerDocument, ContentId, ExecutionId, ExecutionSelection, KernelEpoch, PermissionMode,
};
use serde::{Deserialize, Serialize};

use crate::backend::kernel::{Command, KernelBackend, Mutation, Scope};

pub use crate::backend::kernel::KernelPorts as SessionKernel;
mod import;
mod lifecycle;
pub use lifecycle::*;

mod blob;
mod history_views;
mod prompt;
mod read;
pub use history_views::{
    MessageHistoryPage, MessageHistorySummary, OwnedToolCursor, OwnedToolPage,
};
mod snapshot;
pub use prompt::{PromptEntry, PromptReplay, PromptTool, PromptView};
pub use read::{
    ReadBudget, ReadBudgetSnapshot, ReadLease, ReadOwner, ReadSource, ReadView, modeled_read_bytes,
};
pub use snapshot::{
    ExportReport, HistorySnapshot, HistorySnapshotInfo, SnapshotCursor, SnapshotEntry, SnapshotPage,
};
mod activity;
mod loss;
pub use loss::{CaptureLoss, CaptureLossCounters, CaptureLossCursor, CaptureLossPage};
mod events;
pub use activity::{
    ActiveActivityCursor, ActiveActivityPage, ActivityHandle, BackgroundActivityKind,
    BackgroundActivityOutcome, BackgroundActivityStart, BackgroundActivityView,
};
pub use blob::{BlobDescriptor, BlobRange, BlobReader, PreparedBlob};
mod output;
pub use events::{
    DocumentOutput, EventCursor, EventKind, EventPage, EventPayload, EventProjection, EventRecord,
    EventSnapshot, LogOutput, MixedOutput, TerminalOutput,
};
pub(crate) mod watch;
pub use output::{
    CaptureEnd, ContentDescriptor, ContentRetention, ContentStreamView, ContentWatch, TextCursor,
    TextOutput, TextRange,
};
pub use watch::{
    ChangeKind, KernelChange, KernelWatch, SessionListChange, SessionListState, WatchError,
};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum KernelError {
    #[error("invalid command: {0}")]
    Invalid(String),
    #[error("{0} does not exist in this scope")]
    NotFound(String),
    #[error("the operation belongs to a different session or attempt")]
    Ownership,
    #[error("attempt generation has been superseded")]
    StaleAttempt,
    #[error("conflicting state or command identity: {0}")]
    Conflict(String),
    #[error("execution {0} has pending guidance to consume before successful completion")]
    PendingDelivery(ExecutionId),
    #[error("kernel capacity exhausted; operation was not admitted")]
    Busy,
    #[error("command was not committed: {0}")]
    Storage(String),
    #[error("commit outcome is unknown; retry request {request_id}: {diagnostic}")]
    CommitUnknown {
        request_id: RequestId,
        diagnostic: String,
    },
    #[error("kernel has shut down")]
    Closed,
    #[error("content cursor is no longer valid; read a new content snapshot")]
    ContentReset,
    #[error("content {content} retains positions from {retained_start}; requested {requested}")]
    ContentGap {
        content: ContentId,
        requested: u64,
        retained_start: u64,
    },
}

impl From<TransitionError> for KernelError {
    fn from(error: TransitionError) -> Self {
        match error {
            TransitionError::StaleAttempt => Self::StaleAttempt,
            other => Self::Conflict(other.to_string()),
        }
    }
}

pub type Result<T> = std::result::Result<T, KernelError>;

/// A client-generated durable idempotency identity. Retrying a different
/// payload with the same identity conflicts, rather than replaying by accident.
pub type RequestId = agena_domain::CommandId;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionMetadata {
    pub id: SessionId,
    pub workspace_id: i64,
    pub title: String,
    pub favorite: bool,
    pub pinned: bool,
    pub selection: Option<ExecutionSelection>,
    pub metadata_version: MetadataVersion,
    pub membership_generation: MembershipGeneration,
}

/// Compact facts for a session listing. Counts are maintained with the
/// underlying commands; reading a summary never aggregates the transcript.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionSummaryView {
    pub state: agena_domain::SessionStateKind,
    pub parent_session_id: Option<SessionId>,
    pub root_session_id: SessionId,
    pub depth: u32,
    pub relation_kind: agena_domain::SessionRelationKind,
    pub task_id: Option<String>,
    pub subtask_status: Option<agena_domain::SubtaskStatus>,
    pub child_session_count: u64,
    pub id: SessionId,
    pub workspace_id: i64,
    pub title: String,
    pub favorite: bool,
    pub pinned: bool,
    pub metadata_version: MetadataVersion,
    pub membership_generation: MembershipGeneration,
    pub entry_count: u64,
    pub user_message_count: u64,
    pub queued_executions: u64,
    pub running_executions: u64,
    pub last_message_at_ms: Option<i64>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

/// Keyset position in one workspace, ordered by (updated_at_ms, id) descending.
/// A listing is live: changes between pages can move a session to an earlier page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SessionSummaryCursor {
    pub workspace_id: i64,
    pub updated_at_ms: i64,
    pub id: SessionId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionSummaryPage {
    pub total: Option<u64>,
    pub workspace_id: i64,
    pub sessions: Vec<SessionSummaryView>,
    pub next: Option<SessionSummaryCursor>,
    pub has_more: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct MetadataChange {
    pub title: Option<String>,
    pub favorite: Option<bool>,
    pub pinned: Option<bool>,
}

/// Field-level selection changes are merged against the current value inside
/// the command transaction; detached selections cannot replace newer fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SelectionChange {
    Model(Option<agena_domain::ModelRef>),
    ThinkingMode(Option<String>),
    SpeedMode(Option<String>),
    Verbosity(Option<String>),
    ParallelToolCalls(Option<bool>),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExecutionView {
    pub id: ExecutionId,
    pub session_id: SessionId,
    pub message_id: EntryId,
    pub generation: AttemptGeneration,
    pub phase: ExecutionPhase,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EntryPayload {
    User {
        document: ComposerDocument,
    },
    Text {
        content_id: ContentId,
    },
    Reasoning {
        content_id: ContentId,
    },
    Tool {
        tool_id: ToolId,
    },
    Interaction {
        interaction_id: InteractionId,
    },
    Notice {
        text: String,
    },
    Compaction {
        summary: ContentId,
        through: EntryId,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EntryView {
    pub id: EntryId,
    pub origin_session_id: SessionId,
    pub message_id: EntryId,
    pub execution_id: ExecutionId,
    pub generation: AttemptGeneration,
    pub revision: EntryRevision,
    pub created_at_ms: i64,
    pub response_entry_id: Option<EntryId>,
    pub execution_phase: ExecutionPhase,
    /// Capture state of the entry's own content stream. `Open` is the only
    /// state that proves the model is still writing this text/reasoning part;
    /// a running attempt alone does not.
    #[serde(default)]
    pub content_state: Option<CaptureState>,
    pub payload: EntryPayload,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct HistoryCursor {
    pub session: SessionId,
    pub membership: MembershipGeneration,
    pub segment_sequence: i64,
    pub ordinal: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct HistoryPage {
    pub metadata: SessionMetadata,
    pub entries: Vec<EntryView>,
    pub next: Option<HistoryCursor>,
    pub has_more: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ActiveExecutionCursor {
    pub session: SessionId,
    pub after: ExecutionId,
}

/// A bounded page of work owned by this session. Inherited history never
/// makes another session's execution claimable or active here.
#[derive(Debug, Clone, Serialize)]
pub struct ActiveExecutionPage {
    pub executions: Vec<ExecutionView>,
    pub next: Option<ActiveExecutionCursor>,
    pub has_more: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PendingInteractionCursor {
    pub session: SessionId,
    pub after: InteractionId,
}

#[derive(Debug, Clone, Serialize)]
pub struct PendingInteractionPage {
    pub interactions: Vec<InteractionView>,
    pub next: Option<PendingInteractionCursor>,
    pub has_more: bool,
}

/// Bounded control facts from one committed read snapshot. Transcript/content
/// pages are independent reads and never delay a command's control projection.
#[derive(Debug, Clone, Serialize)]
pub struct SessionControlSnapshot {
    pub control_version: i64,
    pub metadata: SessionMetadata,
    pub summary: SessionSummaryView,
    pub configuration: SessionConfiguration,
    pub pending_interactions: PendingInteractionPage,
    pub active_executions: ActiveExecutionPage,
    #[serde(serialize_with = "serialize_control_execution")]
    pub active_execution: Option<agena_domain::ExecutionLifecycle>,
}

fn serialize_control_execution<S: serde::Serializer>(
    value: &Option<agena_domain::ExecutionLifecycle>,
    serializer: S,
) -> std::result::Result<S::Ok, S::Error> {
    #[derive(Serialize)]
    #[serde(tag = "state", rename_all = "snake_case")]
    enum Borrowed<'a> {
        Active {
            execution_id: &'a ExecutionId,
            phase: &'a agena_domain::ExecutionPhase,
        },
        Terminal {
            execution_id: &'a ExecutionId,
            outcome: &'a agena_domain::ExecutionOutcome,
        },
    }
    value
        .as_ref()
        .map(|execution| match execution {
            agena_domain::ExecutionLifecycle::Active {
                execution_id,
                phase,
            } => Borrowed::Active {
                execution_id,
                phase,
            },
            agena_domain::ExecutionLifecycle::Terminal {
                execution_id,
                outcome,
            } => Borrowed::Terminal {
                execution_id,
                outcome,
            },
        })
        .serialize(serializer)
}

/// Exact outcome of this command, captured in its transaction. The position
/// is not obtained by re-reading metadata after another writer has run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommitReceipt<T> {
    pub position: CommitPosition,
    pub value: T,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WatchCursor {
    pub session: SessionId,
    pub epoch: KernelEpoch,
    pub position: CommitPosition,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum InteractionRequest {
    Permission {
        action_key: String,
        description: String,
    },
    Input {
        request: agena_domain::UserInputRequest,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RememberPermission {
    pub mode: PermissionMode,
    pub scope: agena_domain::PermissionScope,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolStart {
    pub call_id: String,
    pub name: String,
    pub input: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolResult {
    pub phase: ToolPhase,
    pub value: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolView {
    pub entry_id: EntryId,
    pub id: ToolId,
    pub execution_kind: ToolExecutionKind,
    pub round_id: RoundId,
    pub phase: ToolPhase,
    pub call: ToolStart,
    pub result: Option<serde_json::Value>,
    pub revision: EntryRevision,
    pub outputs: Vec<ContentDescriptor>,
    pub activities: Vec<agena_domain::ActivityId>,
    pub interactions: Vec<InteractionId>,
    pub blobs: Vec<BlobDescriptor>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InteractionView {
    pub created_at_ms: i64,
    pub decided_at_ms: Option<i64>,
    pub id: InteractionId,
    pub tool_id: ToolId,
    pub request: InteractionRequest,
    pub decision: Option<InteractionDecision>,
    pub remembered: Option<RememberPermission>,
    pub revision: EntryRevision,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderRoundView {
    pub execution_id: ExecutionId,
    #[serde(default)]
    pub created_at_ms: i64,
    pub id: RoundId,
    pub phase: ProviderRoundPhase,
    pub inputs: Vec<EntryId>,
    pub provider_state: Option<serde_json::Value>,
}

pub(crate) struct KernelInner {
    pub backend: Arc<dyn KernelBackend>,
    pub epoch: KernelEpoch,
}

/// User-facing mutations. This capability cannot claim work, mint an output
/// writer, finish a tool, or access content maintenance.
#[derive(Clone)]
pub struct SessionCommands(pub(crate) Arc<KernelInner>);

/// Scheduler/executor-only entry point. Issued separately during composition.
#[derive(Clone)]
pub struct ExecutorCommands(pub(crate) Arc<KernelInner>);

#[derive(Clone)]
pub struct KernelReader(pub(crate) Arc<KernelInner>);

#[derive(Clone)]
pub struct AttemptHandle {
    pub(crate) inner: Arc<KernelInner>,
    pub(crate) scope: Scope,
}

#[derive(Clone)]
pub struct RoundHandle {
    pub(crate) attempt: AttemptHandle,
    pub(crate) id: RoundId,
}

#[derive(Clone)]
pub struct ToolHandle {
    pub(crate) round: RoundHandle,
    pub(crate) id: ToolId,
}

/// A stable transcript identity while the provider assembles arguments. It
/// cannot execute, request permission or publish a result before finalization.
#[derive(Clone)]
pub struct ToolDraftHandle(ToolHandle);

impl ToolDraftHandle {
    pub fn id(&self) -> ToolId {
        self.0.id
    }
    pub async fn finalize(
        &self,
        request: RequestId,
        input: serde_json::Value,
    ) -> Result<CommitReceipt<ToolHandle>> {
        let result = self
            .0
            .round
            .attempt
            .inner
            .command(
                request,
                Command::FinalizeTool {
                    scope: self.0.round.attempt.scope,
                    tool: self.0.id,
                    input,
                },
            )
            .await?;
        match result.value {
            Mutation::Tool(id) if id == self.0.id => Ok(CommitReceipt {
                position: result.position,
                value: self.0.clone(),
            }),
            _ => Err(unexpected()),
        }
    }
}

impl KernelInner {
    async fn command(
        &self,
        request_id: RequestId,
        command: Command,
    ) -> Result<CommitReceipt<Mutation>> {
        self.backend.commit(request_id, command).await
    }
}

fn unexpected() -> KernelError {
    KernelError::Storage("backend returned an incompatible command result".into())
}

impl SessionCommands {
    pub async fn create(
        &self,
        request: RequestId,
        workspace_id: i64,
        title: String,
    ) -> Result<CommitReceipt<SessionMetadata>> {
        let result = self
            .0
            .command(
                request,
                Command::CreateSession {
                    workspace_id,
                    title,
                },
            )
            .await?;
        match result.value {
            Mutation::Session(value) => Ok(CommitReceipt {
                position: result.position,
                value: *value,
            }),
            _ => Err(unexpected()),
        }
    }
    pub async fn update_metadata(
        &self,
        request: RequestId,
        session: SessionId,
        change: MetadataChange,
    ) -> Result<CommitReceipt<SessionMetadata>> {
        let result = self
            .0
            .command(request, Command::Metadata { session, change })
            .await?;
        match result.value {
            Mutation::Session(value) => Ok(CommitReceipt {
                position: result.position,
                value: *value,
            }),
            _ => Err(unexpected()),
        }
    }
    pub async fn update_selection(
        &self,
        request: RequestId,
        session: SessionId,
        change: SelectionChange,
    ) -> Result<CommitReceipt<SessionMetadata>> {
        let result = self
            .0
            .command(request, Command::Selection { session, change })
            .await?;
        match result.value {
            Mutation::Session(value) => Ok(CommitReceipt {
                position: result.position,
                value: *value,
            }),
            _ => Err(unexpected()),
        }
    }
    pub async fn send(
        &self,
        request: RequestId,
        session: SessionId,
        document: ComposerDocument,
    ) -> Result<CommitReceipt<ExecutionView>> {
        let result = self
            .0
            .command(
                request,
                Command::Send {
                    session,
                    document,
                    blobs: Vec::new(),
                },
            )
            .await?;
        execution_receipt(result)
    }
    pub async fn cancel(
        &self,
        request: RequestId,
        session: SessionId,
        execution: ExecutionId,
    ) -> Result<CommitReceipt<ExecutionView>> {
        execution_receipt(
            self.0
                .command(request, Command::Cancel { session, execution })
                .await?,
        )
    }
    pub async fn retry(
        &self,
        request: RequestId,
        session: SessionId,
        execution: ExecutionId,
    ) -> Result<CommitReceipt<ExecutionView>> {
        execution_receipt(
            self.0
                .command(request, Command::Retry { session, execution })
                .await?,
        )
    }
    pub async fn reply(
        &self,
        request: RequestId,
        session: SessionId,
        interaction: InteractionId,
        decision: InteractionDecision,
        remember: Option<RememberPermission>,
    ) -> Result<CommitReceipt<InteractionDecision>> {
        let result = self
            .0
            .command(
                request,
                Command::Reply {
                    session,
                    interaction,
                    decision,
                    remember,
                },
            )
            .await?;
        match result.value {
            Mutation::Decision(value) => Ok(CommitReceipt {
                position: result.position,
                value,
            }),
            _ => Err(unexpected()),
        }
    }
}

fn execution_receipt(result: CommitReceipt<Mutation>) -> Result<CommitReceipt<ExecutionView>> {
    match result.value {
        Mutation::Execution(value) => Ok(CommitReceipt {
            position: result.position,
            value,
        }),
        _ => Err(unexpected()),
    }
}

impl ExecutorCommands {
    pub async fn claim(
        &self,
        request: RequestId,
        session: SessionId,
        execution: ExecutionId,
    ) -> Result<CommitReceipt<AttemptHandle>> {
        let result = execution_receipt(
            self.0
                .command(
                    request,
                    Command::Claim {
                        session,
                        execution,
                        epoch: self.0.epoch,
                    },
                )
                .await?,
        )?;
        let scope = Scope {
            session,
            execution,
            generation: result.value.generation,
            epoch: self.0.epoch,
        };
        Ok(CommitReceipt {
            position: result.position,
            value: AttemptHandle {
                inner: self.0.clone(),
                scope,
            },
        })
    }
}

impl AttemptHandle {
    pub fn execution_id(&self) -> ExecutionId {
        self.scope.execution
    }
    pub fn generation(&self) -> AttemptGeneration {
        self.scope.generation
    }
    pub async fn begin_round(
        &self,
        request: RequestId,
        inputs: Vec<EntryId>,
    ) -> Result<CommitReceipt<RoundHandle>> {
        let result = self
            .inner
            .command(
                request,
                Command::BeginRound {
                    scope: self.scope,
                    inputs,
                },
            )
            .await?;
        match result.value {
            Mutation::Round(id) => Ok(CommitReceipt {
                position: result.position,
                value: RoundHandle {
                    attempt: self.clone(),
                    id,
                },
            }),
            _ => Err(unexpected()),
        }
    }
    /// Capture the complete current model window in transcript order, using
    /// the latest committed checkpoint and excluding failed/compaction output.
    /// An oversized window fails admission; it is never silently shortened.
    pub async fn begin_current_round(
        &self,
        request: RequestId,
    ) -> Result<CommitReceipt<RoundHandle>> {
        self.round_command(request, Command::BeginCurrentRound { scope: self.scope })
            .await
    }

    /// Capture a bounded prefix of the current window for incremental
    /// compaction. A successful checkpoint can cover only this captured prefix.
    pub async fn begin_compaction(
        &self,
        request: RequestId,
        max_entries: usize,
    ) -> Result<CommitReceipt<RoundHandle>> {
        self.round_command(
            request,
            Command::BeginCompactionRound {
                scope: self.scope,
                max_entries,
            },
        )
        .await
    }

    async fn round_command(
        &self,
        request: RequestId,
        command: Command,
    ) -> Result<CommitReceipt<RoundHandle>> {
        let receipt = self.inner.command(request, command).await?;
        match receipt.value {
            Mutation::Round(id) => Ok(CommitReceipt {
                position: receipt.position,
                value: RoundHandle {
                    attempt: self.clone(),
                    id,
                },
            }),
            _ => Err(unexpected()),
        }
    }
    /// Decide the normal stop boundary atomically with delivery publication.
    /// A ready decision fences guidance until this attempt begins a new round.
    pub async fn prepare_stop(
        &self,
        request: RequestId,
        conditions: StopConditions,
    ) -> Result<CommitReceipt<StopDecision>> {
        let receipt = self
            .inner
            .command(
                request,
                Command::PrepareStop {
                    scope: self.scope,
                    conditions,
                },
            )
            .await?;
        match receipt.value {
            Mutation::StopDecision(value) => Ok(CommitReceipt {
                position: receipt.position,
                value,
            }),
            _ => Err(unexpected()),
        }
    }
    pub async fn finish(
        &self,
        request: RequestId,
        phase: ExecutionPhase,
    ) -> Result<CommitReceipt<ExecutionView>> {
        execution_receipt(
            self.inner
                .command(
                    request,
                    Command::Finish {
                        scope: self.scope,
                        phase,
                    },
                )
                .await?,
        )
    }
}

impl RoundHandle {
    pub fn id(&self) -> RoundId {
        self.id
    }
    pub fn session_id(&self) -> SessionId {
        self.attempt.scope.session
    }
    pub fn execution_id(&self) -> ExecutionId {
        self.attempt.execution_id()
    }
    pub fn generation(&self) -> AttemptGeneration {
        self.attempt.generation()
    }
    /// Atomically seal the summary, finish this compaction round, append its
    /// visible checkpoint and advance the model boundary. Failed, cancelled
    /// or unchanged rounds never advance it. Appended suffix entries survive.
    pub async fn commit_compaction(
        &self,
        request: RequestId,
        summary: ContentId,
        through: EntryId,
        provider_state: Option<serde_json::Value>,
    ) -> Result<CommitReceipt<EntryId>> {
        let receipt = self
            .attempt
            .inner
            .command(
                request,
                Command::CommitCompaction {
                    scope: self.attempt.scope,
                    round: self.id,
                    summary,
                    through,
                    provider_state,
                },
            )
            .await?;
        match receipt.value {
            Mutation::Checkpoint(id) => Ok(CommitReceipt {
                position: receipt.position,
                value: id,
            }),
            _ => Err(unexpected()),
        }
    }
    pub async fn begin_tool(
        &self,
        request: RequestId,
        tool: ToolStart,
    ) -> Result<CommitReceipt<ToolHandle>> {
        self.begin_tool_kind(request, tool, ToolExecutionKind::Runtime, false)
            .await
    }
    /// Record a provider-reported operation before waiting for its terminal
    /// item. This handle does not enqueue local tool execution or permission.
    pub async fn begin_provider_tool(
        &self,
        request: RequestId,
        tool: ToolStart,
    ) -> Result<CommitReceipt<ToolHandle>> {
        self.begin_tool_kind(request, tool, ToolExecutionKind::ProviderHosted, false)
            .await
    }
    async fn begin_tool_kind(
        &self,
        request: RequestId,
        tool: ToolStart,
        execution_kind: ToolExecutionKind,
        preparing: bool,
    ) -> Result<CommitReceipt<ToolHandle>> {
        let result = self
            .attempt
            .inner
            .command(
                request,
                Command::BeginTool {
                    scope: self.attempt.scope,
                    round: self.id,
                    tool,
                    execution_kind,
                    preparing,
                },
            )
            .await?;
        match result.value {
            Mutation::Tool(id) => Ok(CommitReceipt {
                position: result.position,
                value: ToolHandle {
                    round: self.clone(),
                    id,
                },
            }),
            _ => Err(unexpected()),
        }
    }
    pub async fn begin_tool_draft(
        &self,
        request: RequestId,
        call_id: String,
        name: String,
    ) -> Result<CommitReceipt<ToolDraftHandle>> {
        let result = self
            .begin_tool_kind(
                request,
                ToolStart {
                    call_id,
                    name,
                    input: serde_json::Value::Null,
                },
                ToolExecutionKind::Runtime,
                true,
            )
            .await?;
        Ok(CommitReceipt {
            position: result.position,
            value: ToolDraftHandle(result.value),
        })
    }
    pub async fn finish(
        &self,
        request: RequestId,
        provider_state: Option<serde_json::Value>,
        succeeded: bool,
    ) -> Result<CommitReceipt<RoundId>> {
        let result = self
            .attempt
            .inner
            .command(
                request,
                Command::FinishRound {
                    scope: self.attempt.scope,
                    round: self.id,
                    provider_state,
                    succeeded,
                },
            )
            .await?;
        match result.value {
            Mutation::Round(value) => Ok(CommitReceipt {
                position: result.position,
                value,
            }),
            _ => Err(unexpected()),
        }
    }
}

impl ToolHandle {
    pub fn id(&self) -> ToolId {
        self.id
    }
    pub async fn request_interaction(
        &self,
        request: RequestId,
        interaction: InteractionRequest,
    ) -> Result<CommitReceipt<InteractionId>> {
        let result = self
            .round
            .attempt
            .inner
            .command(
                request,
                Command::RequestInteraction {
                    scope: self.round.attempt.scope,
                    tool: self.id,
                    interaction,
                },
            )
            .await?;
        match result.value {
            Mutation::Interaction(value) => Ok(CommitReceipt {
                position: result.position,
                value,
            }),
            _ => Err(unexpected()),
        }
    }
    pub async fn complete(
        &self,
        request: RequestId,
        result: ToolResult,
    ) -> Result<CommitReceipt<ToolId>> {
        let result = self
            .round
            .attempt
            .inner
            .command(
                request,
                Command::CompleteTool {
                    scope: self.round.attempt.scope,
                    tool: self.id,
                    result,
                    blobs: Vec::new(),
                },
            )
            .await?;
        match result.value {
            Mutation::Tool(value) => Ok(CommitReceipt {
                position: result.position,
                value,
            }),
            _ => Err(unexpected()),
        }
    }
}

impl KernelReader {
    /// Read session control facts together, with at most 128 pending requests
    /// and active executions. Warm reads share one accounting lease without
    /// SQL; the returned view never pins a transaction or admission permit.
    pub async fn control_snapshot(
        &self,
        session: SessionId,
        limit: usize,
    ) -> Result<ReadView<SessionControlSnapshot>> {
        self.0.backend.control_snapshot(session, limit).await
    }
    /// Read the immutable input facts and committed content boundaries captured
    /// when this provider round began. This does not select a new live window.
    pub async fn prompt(&self, session: SessionId, round: RoundId) -> Result<ReadView<PromptView>> {
        self.0.backend.prompt(session, round).await
    }
    pub async fn summaries(
        &self,
        workspace_id: i64,
        before: Option<SessionSummaryCursor>,
        limit: usize,
    ) -> Result<ReadView<SessionSummaryPage>> {
        self.0.backend.summaries(workspace_id, before, limit).await
    }
    pub async fn summary(&self, session: SessionId) -> Result<ReadView<SessionSummaryView>> {
        self.0.backend.summary(session).await
    }
    pub async fn pending_interactions(
        &self,
        session: SessionId,
        after: Option<PendingInteractionCursor>,
        limit: usize,
    ) -> Result<ReadView<PendingInteractionPage>> {
        self.0
            .backend
            .pending_interactions(session, after, limit)
            .await
    }
    pub async fn entry(&self, session: SessionId, entry: EntryId) -> Result<ReadView<EntryView>> {
        self.0.backend.entry(session, entry).await
    }
    pub async fn active_executions(
        &self,
        session: SessionId,
        after: Option<ActiveExecutionCursor>,
        limit: usize,
    ) -> Result<ReadView<ActiveExecutionPage>> {
        self.0
            .backend
            .active_executions(session, after, limit)
            .await
    }
    pub async fn tool(&self, session: SessionId, tool: ToolId) -> Result<ReadView<ToolView>> {
        self.0.backend.tool(session, tool).await
    }
    pub async fn interaction(
        &self,
        session: SessionId,
        interaction: InteractionId,
    ) -> Result<ReadView<InteractionView>> {
        self.0.backend.interaction(session, interaction).await
    }
    pub async fn round(
        &self,
        session: SessionId,
        round: RoundId,
    ) -> Result<ReadView<ProviderRoundView>> {
        self.0.backend.round(session, round).await
    }
    pub async fn failed_round(
        &self,
        session: SessionId,
        execution: ExecutionId,
        generation: AttemptGeneration,
    ) -> Result<ReadView<Option<ProviderRoundView>>> {
        self.0
            .backend
            .failed_round(session, execution, generation)
            .await
    }
    /// Subscribe before reading a snapshot. The returned head/replay and live
    /// receiver are installed under one feed lock, closing the read/watch gap.
    pub fn watch(&self, session: SessionId, after: Option<WatchCursor>) -> Result<KernelWatch> {
        self.0.backend.watch(session, after)
    }
    pub async fn metadata(&self, session: SessionId) -> Result<ReadView<SessionMetadata>> {
        self.0.backend.metadata(session).await
    }
    /// Monotonic version of durable session control changes. Coherent control
    /// projections use `control_snapshot`; metadata's optimistic concurrency
    /// version remains independent.
    pub async fn control_version(&self, session: SessionId) -> Result<ReadView<i64>> {
        self.0.backend.control_version(session).await
    }
    pub async fn execution(
        &self,
        session: SessionId,
        execution: ExecutionId,
    ) -> Result<ReadView<ExecutionView>> {
        self.0.backend.execution(session, execution).await
    }
    pub async fn history(
        &self,
        session: SessionId,
        before: Option<HistoryCursor>,
        limit: usize,
    ) -> Result<ReadView<HistoryPage>> {
        self.0.backend.history(session, before, limit).await
    }
}
