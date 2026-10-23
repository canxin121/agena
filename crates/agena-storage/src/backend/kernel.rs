//! Infrastructure port. Product composition passes separately scoped
//! `SessionCommands`, `ExecutorCommands` and `KernelReader` to consumers.

use std::sync::Arc;

use agena_domain::{
    ActivityId, BlobId, CommandId, ComposerDocument, ContentEpoch, ContentId, ExecutionId,
    KernelEpoch,
};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

pub use crate::kernel::watch::ChangeFeed;
use crate::kernel::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scope {
    pub session: SessionId,
    pub execution: ExecutionId,
    pub generation: AttemptGeneration,
    pub epoch: KernelEpoch,
}

/// Independent background authority. This lease carries no attempt-control
/// capability and survives completion/cancellation/retry of its launch attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivityScope {
    pub id: ActivityId,
    pub session: SessionId,
    pub epoch: KernelEpoch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OutputScope {
    Attempt(Scope),
    Activity(ActivityScope),
}

impl OutputScope {
    pub fn session(self) -> SessionId {
        match self {
            Self::Attempt(scope) => scope.session,
            Self::Activity(scope) => scope.session,
        }
    }

    pub fn check(self, other: Self) -> Result<()> {
        if let (Self::Attempt(a), Self::Attempt(b)) = (self, other)
            && a.generation != b.generation
        {
            return Err(KernelError::StaleAttempt);
        }
        if self != other {
            return Err(KernelError::Ownership);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Command {
    RecordCompaction {
        scope: Scope,
        policy_key: String,
        outcome: CompactionPolicyOutcome,
    },
    PresentInteraction {
        session: SessionId,
        interaction: InteractionId,
    },
    CreateConfigured {
        workspace_id: i64,
        title: String,
        configuration: SessionConfiguration,
    },
    Configure {
        session: SessionId,
        change: SessionConfigurationChange,
    },
    Fork {
        session: SessionId,
        through: EntryId,
        rewind: bool,
        conversation_mode: Option<agena_domain::ConversationMode>,
    },
    Withdraw {
        session: SessionId,
        message: EntryId,
    },
    DeleteSession {
        session: SessionId,
    },
    Continue {
        session: SessionId,
        message: EntryId,
    },
    Notice {
        scope: Scope,
        text: String,
    },
    Deliver {
        session: SessionId,
        business_key: String,
        payload: serde_json::Value,
    },
    DeliverToExecution {
        session: SessionId,
        execution: ExecutionId,
        business_key: String,
        payload: serde_json::Value,
    },
    ResumeDelivery {
        session: SessionId,
        delivery: CommandId,
        message: EntryId,
        text: String,
    },
    ConsumeDelivery {
        session: SessionId,
        delivery: CommandId,
        message: EntryId,
        text: String,
    },
    CreateSession {
        workspace_id: i64,
        title: String,
    },
    Metadata {
        session: SessionId,
        change: MetadataChange,
    },
    Selection {
        session: SessionId,
        change: SelectionChange,
    },
    Send {
        session: SessionId,
        document: ComposerDocument,
        blobs: Vec<BlobLease>,
    },
    Claim {
        session: SessionId,
        execution: ExecutionId,
        epoch: KernelEpoch,
    },
    ClaimNext {
        session: SessionId,
        execution: ExecutionId,
        epoch: KernelEpoch,
    },
    Cancel {
        session: SessionId,
        execution: ExecutionId,
    },
    CancelQueued {
        session: SessionId,
        execution: ExecutionId,
    },
    ClearQueued {
        session: SessionId,
    },
    Retry {
        session: SessionId,
        execution: ExecutionId,
    },
    BeginRound {
        scope: Scope,
        inputs: Vec<EntryId>,
    },
    BeginCurrentRound {
        scope: Scope,
    },
    BeginCompactionRound {
        scope: Scope,
        max_entries: usize,
    },
    CommitCompaction {
        scope: Scope,
        round: RoundId,
        summary: ContentId,
        through: EntryId,
        provider_state: Option<serde_json::Value>,
    },
    FinishRound {
        scope: Scope,
        round: RoundId,
        provider_state: Option<serde_json::Value>,
        succeeded: bool,
    },
    BeginTool {
        scope: Scope,
        round: RoundId,
        tool: ToolStart,
        execution_kind: ToolExecutionKind,
        preparing: bool,
    },
    FinalizeTool {
        scope: Scope,
        tool: ToolId,
        input: serde_json::Value,
    },
    RequestInteraction {
        scope: Scope,
        tool: ToolId,
        interaction: InteractionRequest,
    },
    Reply {
        session: SessionId,
        interaction: InteractionId,
        decision: InteractionDecision,
        remember: Option<RememberPermission>,
    },
    CompleteTool {
        scope: Scope,
        tool: ToolId,
        result: ToolResult,
        blobs: Vec<BlobLease>,
    },
    Finish {
        scope: Scope,
        phase: ExecutionPhase,
    },
    PrepareStop {
        scope: Scope,
        conditions: StopConditions,
    },
    BeginActivity {
        scope: Scope,
        tool: ToolId,
        activity: BackgroundActivityStart,
    },
    EndActivity {
        scope: ActivityScope,
        outcome: BackgroundActivityOutcome,
    },
    OpenActivityOutput {
        scope: ActivityScope,
        kind: agena_domain::ContentKind,
    },
    OpenText {
        scope: Scope,
        round: RoundId,
        tool: Option<ToolId>,
        reasoning: bool,
    },
    OpenEvents {
        scope: Scope,
        round: RoundId,
        tool: ToolId,
        kind: EventKind,
    },
    SealContent {
        scope: OutputScope,
        content: ContentId,
        epoch: ContentEpoch,
        ending: CaptureEnd,
    },
    PrepareBlob {
        session: SessionId,
        expected_bytes: u64,
    },
    PinBlob {
        session: SessionId,
        blob: BlobId,
        epoch: KernelEpoch,
        preparation: bool,
    },
    PinSnapshotBlob {
        snapshot: HistorySnapshotLease,
        position: SnapshotCursor,
        blob: BlobId,
    },
    WriteBlob {
        lease: BlobLease,
        offset: u64,
        bytes: Vec<u8>,
    },
    ReadyBlob {
        lease: BlobLease,
        sha256: [u8; 32],
    },
    PruneContent {
        session: SessionId,
        content: ContentId,
        before: u64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlobLease {
    pub id: BlobId,
    pub session: SessionId,
    pub token: CommandId,
    pub epoch: KernelEpoch,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlobRecord {
    pub id: BlobId,
    pub byte_len: u64,
    pub sha256: Option<[u8; 32]>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlobPin {
    pub lease: BlobLease,
    pub record: BlobRecord,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BlobGc {
    pub retired: u64,
    pub chunks: u64,
    pub blobs: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistorySnapshotLease {
    pub token: CommandId,
    pub session: SessionId,
    pub generation: MembershipGeneration,
    pub epoch: KernelEpoch,
}

pub struct HistorySnapshotOpened {
    pub lease: HistorySnapshotLease,
    pub info: ReadView<HistorySnapshotInfo>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HistorySnapshotGc {
    pub roots: usize,
    pub references: usize,
    pub segments: usize,
}

/// Infrastructure-only maintenance. Product callers receive the separately
/// issued session/executor/reader capabilities.
#[derive(Clone)]
pub struct KernelMaintenance(Arc<dyn KernelBackend>);
impl KernelMaintenance {
    pub async fn collect_history_snapshots(&self) -> Result<HistorySnapshotGc> {
        self.0.collect_history_snapshots().await
    }
    pub fn invalidate_read_cache(&self) {
        self.0.invalidate_read_cache();
    }
    pub fn new(backend: Arc<dyn KernelBackend>) -> Self {
        Self(backend)
    }
    pub async fn collect_blobs(&self) -> Result<BlobGc> {
        self.0.collect_blobs().await
    }
    /// Retention is an infrastructure policy. One command establishes the
    /// logical gap and reclaims a bounded batch. Further reclamation uses a new
    /// request identity; retrying one request preserves its original receipt.
    pub async fn prune_content(
        &self,
        request: RequestId,
        session: SessionId,
        content: ContentId,
        before: u64,
    ) -> Result<CommitReceipt<ContentRetention>> {
        let receipt = self
            .0
            .commit(
                request,
                Command::PruneContent {
                    session,
                    content,
                    before,
                },
            )
            .await?;
        match receipt.value {
            Mutation::Retention(value) => Ok(CommitReceipt {
                position: receipt.position,
                value,
            }),
            _ => Err(KernelError::Storage("incompatible retention result".into())),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Mutation {
    PromptWindowStatus(PromptWindowStatus),
    InteractionView(Box<InteractionView>),
    Configuration(Box<SessionConfiguration>),
    Rewind(Box<RewindResult>),
    Delivery(DeliveryView),
    Session(Box<SessionMetadata>),
    Execution(ExecutionView),
    ClearedQueued(u64),
    StopDecision(StopDecision),
    Round(RoundId),
    Checkpoint(EntryId),
    Tool(ToolId),
    Activity(ActivityId),
    Interaction(InteractionId),
    Decision(InteractionDecision),
    Content(ContentStreamView),
    BlobRecord(BlobRecord),
    BlobPin(Box<BlobPin>),
    BlobOffset(u64),
    BlobReady(BlobDescriptor),
    Retention(ContentRetention),
}

pub enum EventInput {
    Mixed(Box<agena_domain::ContentInput>),
    Log {
        channel: agena_domain::CommandOutputStream,
        text: String,
    },
    Document(Box<agena_domain::DocumentMutation>),
    Terminal(Box<agena_domain::TerminalSnapshot>),
    TerminalPatch {
        base: EventCursor,
        screen: Box<agena_domain::TerminalSnapshot>,
        rows_changed: Vec<u16>,
    },
}

#[async_trait]
pub trait KernelBackend: Send + Sync {
    async fn prompt_window_status(
        &self,
        session: SessionId,
    ) -> Result<ReadView<PromptWindowStatus>>;

    async fn background_activities(
        &self,
        session: Option<SessionId>,
        after: Option<ActivityId>,
        active_only: bool,
        limit: usize,
    ) -> Result<ReadView<Vec<BackgroundActivityView>>>;
    async fn history_after(
        &self,
        session: SessionId,
        after_entry: Option<EntryId>,
        limit: usize,
    ) -> Result<ReadView<HistoryPage>>;
    async fn find_task(
        &self,
        parent: SessionId,
        task_id: &str,
    ) -> Result<ReadView<Option<SessionId>>>;

    async fn import_archive(
        &self,
        request: RequestId,
        workspace_id: i64,
        path: &std::path::Path,
    ) -> Result<CommitReceipt<SessionMetadata>>;
    async fn latest_round(&self, session: SessionId)
    -> Result<ReadView<Option<ProviderRoundView>>>;
    /// Most recent completed round that retained provider usage. A later
    /// failed round can finish without a measurement; it must not hide the
    /// context size measured by the previous provider request.
    async fn latest_usage_round(
        &self,
        session: SessionId,
    ) -> Result<ReadView<Option<ProviderRoundView>>>;
    /// Most recent failed round that retained a provider-state failure for an
    /// execution generation. Used to reconstruct a user-visible error for a
    /// transcript after the failure has become durable.
    async fn failed_round(
        &self,
        session: SessionId,
        execution: ExecutionId,
        generation: AttemptGeneration,
    ) -> Result<ReadView<Option<ProviderRoundView>>>;
    async fn configuration(&self, session: SessionId) -> Result<ReadView<SessionConfiguration>>;
    async fn query_summaries(&self, query: SummaryQuery) -> Result<ReadView<SessionSummaryPage>>;
    async fn workspace_statistics(
        &self,
        workspace_ids: &[i64],
    ) -> Result<ReadView<std::collections::BTreeMap<i64, WorkspaceStatistics>>>;
    async fn rounds(
        &self,
        session: SessionId,
        after: Option<RoundId>,
        limit: usize,
    ) -> Result<ReadView<Vec<ProviderRoundView>>>;
    fn watch_all(&self, after: Option<WatchCursor>) -> Result<KernelWatch>;
    async fn pending_deliveries(
        &self,
        session: SessionId,
        after: Option<CommandId>,
        limit: usize,
    ) -> Result<ReadView<Vec<DeliveryView>>>;
    async fn export_jsonl(
        &self,
        lease: HistorySnapshotLease,
        info: &ReadView<HistorySnapshotInfo>,
        target: &mut (dyn tokio::io::AsyncWrite + Unpin + Send),
    ) -> Result<ExportReport>;
    async fn open_history_snapshot(
        &self,
        session: SessionId,
        epoch: KernelEpoch,
    ) -> Result<HistorySnapshotOpened>;
    fn retain_history_snapshot(&self, lease: HistorySnapshotLease) -> Result<()>;
    fn release_history_snapshot(&self, lease: HistorySnapshotLease);
    async fn collect_history_snapshots(&self) -> Result<HistorySnapshotGc>;
    async fn snapshot_page(
        &self,
        lease: HistorySnapshotLease,
        after: Option<SnapshotCursor>,
        limit: usize,
    ) -> Result<ReadView<SnapshotPage>>;
    async fn prompt(&self, session: SessionId, round: RoundId) -> Result<ReadView<PromptView>>;
    async fn text_range_at(
        &self,
        session: SessionId,
        boundary: &ContentStreamView,
        after: Option<TextCursor>,
        limit: usize,
    ) -> Result<ReadView<TextRange>>;
    async fn event_page_at(
        &self,
        session: SessionId,
        boundary: &ContentStreamView,
        after: Option<EventCursor>,
        limit: usize,
    ) -> Result<ReadView<EventPage>>;
    async fn event_snapshot_at(
        &self,
        session: SessionId,
        boundary: &ContentStreamView,
    ) -> Result<ReadView<EventSnapshot>>;
    async fn summaries(
        &self,
        workspace_id: i64,
        before: Option<SessionSummaryCursor>,
        limit: usize,
    ) -> Result<ReadView<SessionSummaryPage>>;
    async fn summary(&self, session: SessionId) -> Result<ReadView<SessionSummaryView>>;
    async fn control_snapshot(
        &self,
        session: SessionId,
        limit: usize,
    ) -> Result<ReadView<SessionControlSnapshot>>;
    async fn pending_interactions(
        &self,
        session: SessionId,
        after: Option<PendingInteractionCursor>,
        limit: usize,
    ) -> Result<ReadView<PendingInteractionPage>>;
    async fn entry(&self, session: SessionId, entry: EntryId) -> Result<ReadView<EntryView>>;
    async fn owned_tool_entries(
        &self,
        session: SessionId,
        after: Option<OwnedToolCursor>,
        limit: usize,
    ) -> Result<ReadView<OwnedToolPage>>;
    async fn active_executions(
        &self,
        session: SessionId,
        after: Option<ActiveExecutionCursor>,
        limit: usize,
    ) -> Result<ReadView<ActiveExecutionPage>>;
    fn invalidate_read_cache(&self);
    fn admit_blob_upload(&self) -> Result<tokio::sync::OwnedSemaphorePermit>;
    fn retain_blob(&self, lease: BlobLease) -> Result<()>;
    fn release_blob(&self, lease: BlobLease);
    async fn collect_blobs(&self) -> Result<BlobGc>;
    async fn blob_range(
        &self,
        lease: BlobLease,
        offset: u64,
        limit: usize,
    ) -> Result<ReadView<BlobRange>>;
    async fn entry_blobs(
        &self,
        session: SessionId,
        entry: EntryId,
    ) -> Result<ReadView<Vec<BlobDescriptor>>>;
    async fn append_event(
        &self,
        scope: OutputScope,
        content: ContentId,
        epoch: ContentEpoch,
        timestamp_ms: Option<i64>,
        input: EventInput,
    ) -> Result<EventCursor>;
    /// Memory-only admission for OS pipe/PTY readers. Busy admits nothing;
    /// success returns the canonical accepted cursor immediately.
    fn try_append_event(
        &self,
        scope: OutputScope,
        content: ContentId,
        epoch: ContentEpoch,
        timestamp_ms: Option<i64>,
        input: EventInput,
    ) -> Result<EventCursor>;
    /// Record input deliberately discarded by a pipe/PTY producer. This is a
    /// short memory operation, never a persistence wait. Report each discarded
    /// packet once, including non-text packets whose byte count is zero.
    fn record_capture_loss(
        &self,
        scope: OutputScope,
        content: ContentId,
        epoch: ContentEpoch,
        bytes: u64,
        diagnostic: &str,
    ) -> Result<()>;
    async fn capture_losses(
        &self,
        session: SessionId,
        content: ContentId,
        after: Option<CaptureLossCursor>,
        limit: usize,
    ) -> Result<ReadView<CaptureLossPage>>;
    async fn event_page(
        &self,
        session: SessionId,
        content: ContentId,
        after: Option<EventCursor>,
        limit: usize,
    ) -> Result<ReadView<EventPage>>;
    async fn event_snapshot(
        &self,
        session: SessionId,
        content: ContentId,
    ) -> Result<ReadView<EventSnapshot>>;
    fn retain_output(&self, scope: OutputScope, stream: &ContentStreamView) -> Result<()>;
    fn release_output(&self, scope: OutputScope, content: ContentId);
    async fn append_text(
        &self,
        scope: OutputScope,
        content: ContentId,
        epoch: ContentEpoch,
        text: &str,
    ) -> Result<TextCursor>;
    fn try_append_text(
        &self,
        scope: OutputScope,
        content: ContentId,
        epoch: ContentEpoch,
        text: &str,
    ) -> Result<TextCursor>;
    async fn text_range(
        &self,
        session: SessionId,
        content: ContentId,
        after: Option<TextCursor>,
        limit: usize,
    ) -> Result<ReadView<TextRange>>;
    async fn watch_content(&self, session: SessionId, content: ContentId) -> Result<ContentWatch>;
    async fn activity(
        &self,
        session: SessionId,
        activity: ActivityId,
    ) -> Result<ReadView<BackgroundActivityView>>;
    async fn active_activities(
        &self,
        session: SessionId,
        after: Option<ActiveActivityCursor>,
        limit: usize,
    ) -> Result<ReadView<ActiveActivityPage>>;
    async fn tool(&self, session: SessionId, tool: ToolId) -> Result<ReadView<ToolView>>;
    async fn interaction(
        &self,
        session: SessionId,
        interaction: InteractionId,
    ) -> Result<ReadView<InteractionView>>;
    async fn round(
        &self,
        session: SessionId,
        round: RoundId,
    ) -> Result<ReadView<ProviderRoundView>>;
    fn watch(&self, session: SessionId, after: Option<WatchCursor>) -> Result<KernelWatch>;
    async fn shutdown(&self) -> Result<()>;
    async fn commit(&self, request: RequestId, command: Command)
    -> Result<CommitReceipt<Mutation>>;
    async fn metadata(&self, session: SessionId) -> Result<ReadView<SessionMetadata>>;
    async fn control_version(&self, session: SessionId) -> Result<ReadView<i64>>;
    async fn execution(
        &self,
        session: SessionId,
        execution: ExecutionId,
    ) -> Result<ReadView<ExecutionView>>;
    async fn history_window(
        &self,
        session: SessionId,
        before: Option<HistoryCursor>,
        message_ids: &[i64],
        limit: usize,
    ) -> Result<ReadView<HistoryPage>>;
    async fn message_history(
        &self,
        session: SessionId,
        before: Option<HistoryCursor>,
        message_limit: usize,
        entry_limit: usize,
    ) -> Result<ReadView<MessageHistoryPage>>;
    async fn history(
        &self,
        session: SessionId,
        before: Option<HistoryCursor>,
        limit: usize,
    ) -> Result<ReadView<HistoryPage>>;
}

#[derive(Clone)]
pub struct KernelPorts {
    pub sessions: SessionCommands,
    pub executor: ExecutorCommands,
    pub reader: KernelReader,
}

impl KernelPorts {
    /// Infrastructure lifecycle boundary: drain accepted commands and close
    /// connections. Business handles expose no storage flush/shutdown API.
    pub async fn shutdown(self) -> Result<()> {
        self.reader.0.backend.shutdown().await
    }
}

pub fn session_kernel(backend: Arc<dyn KernelBackend>, epoch: KernelEpoch) -> KernelPorts {
    let inner = Arc::new(crate::kernel::KernelInner { backend, epoch });
    KernelPorts {
        sessions: SessionCommands(inner.clone()),
        executor: ExecutorCommands(inner.clone()),
        reader: KernelReader(inner),
    }
}
