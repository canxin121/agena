//! Session configuration, immutable branch membership and durable deliveries.
use super::*;
use agena_domain::{
    AgentStatus, PermissionConfig, SessionConversation, SessionRelationKind, SessionStateKind,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct SessionConfiguration {
    /// Missing on older sessions means manual: existing names are preserved.
    pub title_state: SessionTitleState,
    /// A user stop fences queued claims until an explicit resume, including
    /// after reconnecting or reopening the database.
    pub message_queue_paused: bool,
    pub parent_session_id: Option<SessionId>,
    pub relation_kind: SessionRelationKind,
    /// Present only on a conversation beside an existing run (side / btw).
    pub conversation: Option<SessionConversation>,
    /// Session-scoped prompt layer: inline fragments for this session only.
    pub prompts: agena_prompt_layers::PromptLayer,
    pub permission: PermissionConfig,
    pub permission_ceiling: PermissionConfig,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_authority: Option<agena_domain::ChannelAuthority>,
    /// Bot ownership. `None` is an ordinary project conversation; `Some`
    /// names the instance Bot this conversation belongs to. Ownership is
    /// projected into `agena_sessions.owner_*` columns for indexed listing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<agena_domain::SessionOwner>,
    pub capability_denied_tool_names: BTreeSet<String>,
    pub effective_workspace_root: Option<PathBuf>,
    pub agent: Option<SessionAgentConfiguration>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SessionTitleSource {
    Default,
    Automatic,
    #[default]
    Manual,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct SessionTitleState {
    pub source: SessionTitleSource,
    pub checked_user_messages: u64,
    pub checked_at_ms: i64,
    pub calibrated: bool,
}

impl SessionTitleState {
    pub fn automatic() -> Self {
        Self {
            source: SessionTitleSource::Default,
            ..Self::default()
        }
    }
}

/// Compare-and-set guard for a background title result. Checked with the
/// title write in the kernel transaction, including results that keep the name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutomaticTitleUpdate {
    pub expected_metadata_version: MetadataVersion,
    pub expected_membership_generation: MembershipGeneration,
    pub user_message_count: u64,
    pub last_user_message: EntryId,
    pub last_user_revision: EntryRevision,
    pub title: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionAgentConfiguration {
    pub name: String,
    pub status: AgentStatus,
    pub started_at_ms: Option<i64>,
    pub finished_at_ms: Option<i64>,
    pub failure: Option<agena_failure::Failure>,
    pub limits: AgentExecutionLimits,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct AgentExecutionLimits {
    pub timeout_ms: Option<u64>,
    pub max_tokens: Option<u64>,
    pub max_cost_microusd: Option<u64>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct SessionConfigurationChange {
    pub title_state: Option<SessionTitleState>,
    pub message_queue_paused: Option<bool>,
    pub parent_session_id: Option<SessionId>,
    pub relation_kind: Option<SessionRelationKind>,
    pub conversation: Option<Option<SessionConversation>>,
    pub prompts: Option<agena_prompt_layers::PromptLayer>,
    pub permission: Option<PermissionConfig>,
    pub permission_ceiling: Option<PermissionConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_authority: Option<agena_domain::ChannelAuthority>,
    pub owner: Option<Option<agena_domain::SessionOwner>>,
    pub capability_denied_tool_names: Option<BTreeSet<String>>,
    pub effective_workspace_root: Option<Option<PathBuf>>,
    pub agent: Option<Option<SessionAgentConfiguration>>,
}
/// Retained isolation facts for a live target and its bounded native ancestry.
/// Soft deletion of an ordinary parent does not strand a surviving fork, and
/// deleting a private parent never erases its inherited scope restrictions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagementAncestor {
    pub session: SessionId,
    pub workspace_id: Option<i64>,
    pub effective_workspace_root: Option<PathBuf>,
    pub owner: Option<agena_domain::SessionOwner>,
    pub excluded_scope: bool,
}

impl ManagementAncestor {
    /// True when this ancestor is a Bot's operator conversation: a local root
    /// with no project scope, which may manage ordinary project sessions.
    pub fn is_operator_bot(&self) -> bool {
        self.workspace_id.is_none()
            && self
                .owner
                .as_ref()
                .is_some_and(|owner| owner.surface.is_local())
            && !self.excluded_scope
    }
}
/// Native Bot fork fence. Captured once and persisted with the operation intent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForkGuard {
    pub metadata_version: i64,
    pub entry_count: u64,
    pub title: Option<String>,
}
/// Exact confirmed target and direct human turn. All checks run in the same
/// writer transaction as soft deletion; no queue is cancelled by this path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeleteGuard {
    pub metadata_version: i64,
    pub control_version: i64,
    pub entry_count: u64,
    pub source_session: SessionId,
    pub source_message: EntryId,
    pub source_revision: EntryRevision,
}
/// Versions observed before an exact registry session is detached from its project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DetachGlobalGuard {
    pub metadata_version: i64,
    pub control_version: i64,
}
impl DetachGlobalGuard {
    /// Trusted registry ID whose ordinary metadata is unavailable after deletion.
    /// The writer accepts this guard only while the root is still a tombstone.
    pub fn tombstone() -> Self {
        Self {
            metadata_version: 0,
            control_version: 0,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DetachGlobalResult {
    pub root: SessionId,
    pub affected_sessions: u64,
    pub changed_sessions: u64,
    pub changed_reminders: u64,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RewindResult {
    pub session: SessionMetadata,
    pub document: ComposerDocument,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PromptWindowStatus {
    pub generation: u64,
    pub checkpoint_entry_id: Option<EntryId>,
    pub through_entry_id: Option<EntryId>,
    pub policy_key: Option<String>,
    pub consecutive_compaction_failures: u32,
    pub auto_compaction_disabled: bool,
    pub native_compaction_unavailable: bool,
    pub last_before_tokens: Option<u64>,
    pub last_after_tokens: Option<u64>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum CompactionPolicyOutcome {
    Succeeded {
        native: bool,
        before_tokens: Option<u64>,
        after_tokens: Option<u64>,
    },
    Failed,
    NativeUnavailable,
    Unchanged,
}
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SummaryQuery {
    pub workspace_id: Option<i64>,
    /// Include global sessions when no particular workspace is selected.
    #[serde(default)]
    pub include_global: bool,
    /// Restrict to conversations owned by this Bot (UUID text).
    #[serde(default)]
    pub owner_bot_id: Option<String>,
    /// Restrict to one entrance shape (`local`/`direct`/`group`/`broadcast`).
    #[serde(default)]
    pub owner_surface: Option<String>,
    pub parent_id: Option<SessionId>,
    pub roots_only: bool,
    pub exclude_subagents: bool,
    pub temporary_only: bool,
    pub agents_only: bool,
    pub search: Option<String>,
    pub favorite: Option<bool>,
    pub pinned: Option<bool>,
    pub states: Vec<SessionStateKind>,
    pub before: Option<SessionSummaryCursor>,
    pub offset: u64,
    pub limit: usize,
}
/// Canonical activity of visible, non-temporary, non-subagent sessions.
///
/// All fields come from one read snapshot. A metadata edit cannot make an old
/// execution look recent. Only each execution's current attempt participates.
/// If terminal attempts share the newest millisecond, any failure in that
/// timestamp group keeps attention visible; UUID/session order is not time.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionActivitySummary {
    pub running_sessions: u64,
    pub awaiting_interaction_sessions: u64,
    pub latest_terminal_at_ms: Option<i64>,
    pub latest_terminal_failed: bool,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct WorkspaceStatistics {
    pub total: u64,
    pub roots: u64,
    pub pinned: u64,
    pub running: u64,
    pub attention: u64,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeliveryView {
    pub id: agena_domain::CommandId,
    pub session: SessionId,
    pub target_execution: Option<ExecutionId>,
    pub business_key: String,
    pub payload: serde_json::Value,
    pub entry: Option<EntryId>,
    pub consumed: bool,
}

/// Bounded scheduler facts captured before the serialized stop decision.
/// A claimed one-shot occurrence stops being a wake source once its exact
/// delivery has been observed by a successful provider round.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StopConditions {
    pub scheduler_waiting: bool,
    pub pending_once_delivery_keys: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StopDecision {
    /// Business input must reach a real provider round before stopping.
    Continue,
    /// Release this execution and wait for an actual background delivery.
    AwaitingBackground,
    /// Stop hooks may run; targeted guidance is fenced until a new round.
    Ready,
}

/// Read-only local Bot mapping. An established mapping with no live target
/// represents an explicitly deleted conversation and must not adopt old history.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalBotSession {
    pub session: Option<SessionId>,
    pub established: bool,
}

impl SessionCommands {
    /// Return the pinned local conversation, adopting legacy history once.
    /// A deleted pinned conversation is replaced with a fresh empty root.
    pub async fn ensure_local_bot_session(
        &self,
        request: RequestId,
        bot: uuid::Uuid,
        title: String,
    ) -> Result<CommitReceipt<SessionMetadata>> {
        session_receipt(
            self.0
                .command(request, Command::EnsureLocalBotSession { bot, title })
                .await?,
        )
    }

    pub async fn create_global(
        &self,
        request: RequestId,
        title: String,
        configuration: SessionConfiguration,
    ) -> Result<CommitReceipt<SessionMetadata>> {
        session_receipt(
            self.0
                .command(
                    request,
                    Command::CreateGlobal {
                        title,
                        configuration,
                    },
                )
                .await?,
        )
    }

    pub async fn detach_global(
        &self,
        request: RequestId,
        session: SessionId,
        guard: DetachGlobalGuard,
    ) -> Result<CommitReceipt<DetachGlobalResult>> {
        let receipt = self
            .0
            .command(request, Command::DetachGlobal { session, guard })
            .await?;
        match receipt.value {
            Mutation::DetachedGlobal(value) => Ok(CommitReceipt {
                position: receipt.position,
                value,
            }),
            _ => Err(KernelError::Storage(
                "incompatible global detach result".into(),
            )),
        }
    }
    pub async fn present_interaction(
        &self,
        request: RequestId,
        session: SessionId,
        interaction: InteractionId,
    ) -> Result<CommitReceipt<InteractionView>> {
        let r = self
            .0
            .command(
                request,
                Command::PresentInteraction {
                    session,
                    interaction,
                },
            )
            .await?;
        match r.value {
            Mutation::InteractionView(value) => Ok(CommitReceipt {
                position: r.position,
                value: *value,
            }),
            _ => Err(unexpected()),
        }
    }
    pub async fn create_configured(
        &self,
        request: RequestId,
        workspace_id: i64,
        title: String,
        configuration: SessionConfiguration,
    ) -> Result<CommitReceipt<SessionMetadata>> {
        session_receipt(
            self.0
                .command(
                    request,
                    Command::CreateConfigured {
                        workspace_id,
                        title,
                        configuration,
                    },
                )
                .await?,
        )
    }
    pub async fn configure(
        &self,
        request: RequestId,
        session: SessionId,
        change: SessionConfigurationChange,
    ) -> Result<CommitReceipt<SessionConfiguration>> {
        let r = self
            .0
            .command(request, Command::Configure { session, change })
            .await?;
        match r.value {
            Mutation::Configuration(value) => Ok(CommitReceipt {
                position: r.position,
                value: *value,
            }),
            _ => Err(unexpected()),
        }
    }
    /// The terminal status and parent mailbox entry commit together.
    pub async fn finish_agent(
        &self,
        request: RequestId,
        session: SessionId,
        agent: SessionAgentConfiguration,
        business_key: String,
        payload: serde_json::Value,
    ) -> Result<CommitReceipt<SessionConfiguration>> {
        let receipt = self
            .0
            .command(
                request,
                Command::FinishAgent {
                    session,
                    agent,
                    business_key,
                    payload,
                },
            )
            .await?;
        match receipt.value {
            Mutation::Configuration(value) => Ok(CommitReceipt {
                position: receipt.position,
                value: *value,
            }),
            _ => Err(unexpected()),
        }
    }
    pub async fn fork(
        &self,
        request: RequestId,
        session: SessionId,
        through: EntryId,
    ) -> Result<CommitReceipt<SessionMetadata>> {
        session_receipt(
            self.0
                .command(
                    request,
                    Command::Fork {
                        session,
                        through,
                        rewind: false,
                        conversation_mode: None,
                        management: None,
                    },
                )
                .await?,
        )
    }
    /// Ordinary fork with a stable observed source boundary and atomic title.
    pub async fn fork_checked(
        &self,
        request: RequestId,
        session: SessionId,
        through: EntryId,
        guard: ForkGuard,
    ) -> Result<CommitReceipt<SessionMetadata>> {
        session_receipt(
            self.0
                .command(
                    request,
                    Command::Fork {
                        session,
                        through,
                        rewind: false,
                        conversation_mode: None,
                        management: Some(guard),
                    },
                )
                .await?,
        )
    }
    /// Soft-delete only an idle, unchanged, human-confirmed target.
    pub async fn delete_checked(
        &self,
        request: RequestId,
        session: SessionId,
        guard: DeleteGuard,
    ) -> Result<CommitReceipt<SessionMetadata>> {
        session_receipt(
            self.0
                .command(
                    request,
                    Command::DeleteSession {
                        session,
                        guard: Some(guard),
                    },
                )
                .await?,
        )
    }
    /// Create a conversation branch with its visibility fixed before its
    /// first committed change; a BTW branch is never a visible list item.
    pub async fn fork_conversation(
        &self,
        request: RequestId,
        session: SessionId,
        through: EntryId,
        mode: agena_domain::ConversationMode,
    ) -> Result<CommitReceipt<SessionMetadata>> {
        session_receipt(
            self.0
                .command(
                    request,
                    Command::Fork {
                        session,
                        through,
                        rewind: false,
                        conversation_mode: Some(mode),
                        management: None,
                    },
                )
                .await?,
        )
    }
    /// Create a branch before a completed user message and restore its input.
    /// The original session and all of its facts remain available.
    pub async fn rewind(
        &self,
        request: RequestId,
        session: SessionId,
        message: EntryId,
    ) -> Result<CommitReceipt<RewindResult>> {
        let r = self
            .0
            .command(
                request,
                Command::Fork {
                    session,
                    through: message,
                    rewind: true,
                    conversation_mode: None,
                    management: None,
                },
            )
            .await?;
        match r.value {
            Mutation::Rewind(value) => Ok(CommitReceipt {
                position: r.position,
                value: *value,
            }),
            _ => Err(unexpected()),
        }
    }
    /// Hard-truncate this session's history before a completed user message.
    /// No branch is created; the picked input is returned for the composer.
    pub async fn truncate(
        &self,
        request: RequestId,
        session: SessionId,
        message: EntryId,
    ) -> Result<CommitReceipt<RewindResult>> {
        let r = self
            .0
            .command(request, Command::Truncate { session, message })
            .await?;
        match r.value {
            Mutation::Rewind(value) => Ok(CommitReceipt {
                position: r.position,
                value: *value,
            }),
            _ => Err(unexpected()),
        }
    }
    /// Start this conversation over without deleting anything.
    ///
    /// `ClearMode::History` retires every segment so neither the transcript nor
    /// the model sees the old turns; `ClearMode::Context` only advances the
    /// prompt window, leaving the transcript readable. The stored entries,
    /// content and retired segments remain available for recovery.
    pub async fn clear(
        &self,
        request: RequestId,
        session: SessionId,
        mode: ClearMode,
    ) -> Result<CommitReceipt<SessionMetadata>> {
        session_receipt(
            self.0
                .command(request, Command::Clear { session, mode })
                .await?,
        )
    }
    pub async fn withdraw(
        &self,
        request: RequestId,
        session: SessionId,
        message: EntryId,
    ) -> Result<CommitReceipt<SessionMetadata>> {
        session_receipt(
            self.0
                .command(request, Command::Withdraw { session, message })
                .await?,
        )
    }
    pub async fn delete(
        &self,
        request: RequestId,
        session: SessionId,
    ) -> Result<CommitReceipt<SessionMetadata>> {
        session_receipt(
            self.0
                .command(
                    request,
                    Command::DeleteSession {
                        session,
                        guard: None,
                    },
                )
                .await?,
        )
    }
    /// Continue an owned user request with a new generation, or use a new
    /// local continuation for inherited input and completed control operations.
    pub async fn continue_execution(
        &self,
        request: RequestId,
        session: SessionId,
        message: EntryId,
    ) -> Result<CommitReceipt<ExecutionView>> {
        execution_receipt(
            self.0
                .command(request, Command::Continue { session, message })
                .await?,
        )
    }

    /// Queue a session-owned control operation over its current visible context.
    /// Inherited input remains read-only and no user message is synthesized.
    pub async fn start_control_execution(
        &self,
        request: RequestId,
        session: SessionId,
        purpose: ControlExecutionPurpose,
    ) -> Result<CommitReceipt<ExecutionView>> {
        execution_receipt(
            self.0
                .command(request, Command::StartControlExecution { session, purpose })
                .await?,
        )
    }
}
fn session_receipt(r: CommitReceipt<Mutation>) -> Result<CommitReceipt<SessionMetadata>> {
    match r.value {
        Mutation::Session(value) => Ok(CommitReceipt {
            position: r.position,
            value: *value,
        }),
        _ => Err(unexpected()),
    }
}
impl KernelReader {
    /// Target must be live; ancestors retain their policy after soft deletion.
    pub async fn management_ancestry(
        &self,
        session: SessionId,
    ) -> Result<ReadView<Vec<ManagementAncestor>>> {
        self.0.backend.management_ancestry(session).await
    }
    pub async fn prompt_window_status(
        &self,
        session: SessionId,
    ) -> Result<ReadView<PromptWindowStatus>> {
        self.0.backend.prompt_window_status(session).await
    }

    /// Infrastructure activity projection. Unfiltered reads remain bounded and
    /// tombstoned origins never contribute live work.
    pub async fn background_activities(
        &self,
        session: Option<SessionId>,
        after: Option<agena_domain::ActivityId>,
        active_only: bool,
        limit: usize,
    ) -> Result<ReadView<Vec<BackgroundActivityView>>> {
        self.0
            .backend
            .background_activities(session, after, active_only, limit)
            .await
    }
    /// Read forward in membership order with bounded identity and body pages.
    pub async fn history_after(
        &self,
        session: SessionId,
        after_entry: Option<EntryId>,
        limit: usize,
    ) -> Result<ReadView<HistoryPage>> {
        self.0
            .backend
            .history_after(session, after_entry, limit)
            .await
    }
    pub async fn find_agent(
        &self,
        parent: SessionId,
        agent_name: &str,
    ) -> Result<ReadView<Option<SessionId>>> {
        self.0.backend.find_agent(parent, agent_name).await
    }

    pub async fn latest_round(
        &self,
        session: SessionId,
    ) -> Result<ReadView<Option<ProviderRoundView>>> {
        self.0.backend.latest_round(session).await
    }
    pub async fn latest_usage_round(
        &self,
        session: SessionId,
    ) -> Result<ReadView<Option<ProviderRoundView>>> {
        self.0.backend.latest_usage_round(session).await
    }
    pub async fn configuration(
        &self,
        session: SessionId,
    ) -> Result<ReadView<SessionConfiguration>> {
        self.0.backend.configuration(session).await
    }
    /// Read the current local root without creating a conversation.
    pub async fn local_bot_session(&self, bot: uuid::Uuid) -> Result<ReadView<LocalBotSession>> {
        self.0.backend.local_bot_session(bot).await
    }
    pub async fn query_summaries(
        &self,
        query: SummaryQuery,
    ) -> Result<ReadView<SessionSummaryPage>> {
        self.0.backend.query_summaries(query).await
    }
    /// Read cross-session activity and the latest durable terminal outcome
    /// without loading transcripts or relying on lifecycle callback ordering.
    pub async fn activity_summary(&self) -> Result<ReadView<SessionActivitySummary>> {
        self.0.backend.activity_summary().await
    }
    pub async fn workspace_statistics(
        &self,
        workspace_ids: &[i64],
    ) -> Result<ReadView<BTreeMap<i64, WorkspaceStatistics>>> {
        self.0.backend.workspace_statistics(workspace_ids).await
    }
    pub async fn rounds(
        &self,
        session: SessionId,
        after: Option<RoundId>,
        limit: usize,
    ) -> Result<ReadView<Vec<ProviderRoundView>>> {
        self.0.backend.rounds(session, after, limit).await
    }
    /// Read terminal, origin-owned tool result metadata in ascending tool ID
    /// order. This historical read excludes model/raw-output copies and is not
    /// filtered by the session's current transcript membership.
    pub async fn owned_tool_metadata(
        &self,
        session: SessionId,
        after: Option<ToolId>,
        limit: usize,
    ) -> Result<ReadView<Vec<OwnedToolMetadata>>> {
        self.0
            .backend
            .owned_tool_metadata(session, after, limit)
            .await
    }

    pub fn watch_all(&self, after: Option<WatchCursor>) -> Result<KernelWatch> {
        self.0.backend.watch_all(after)
    }
    /// Read the current delivery state rather than replaying an old command
    /// receipt. Scope ownership is checked exactly as for pending deliveries.
    pub async fn delivery(
        &self,
        session: SessionId,
        id: agena_domain::CommandId,
    ) -> Result<ReadView<DeliveryView>> {
        self.0.backend.delivery(session, id).await
    }
    pub async fn pending_deliveries(
        &self,
        session: SessionId,
        after: Option<agena_domain::CommandId>,
        limit: usize,
    ) -> Result<ReadView<Vec<DeliveryView>>> {
        self.0
            .backend
            .pending_deliveries(session, after, limit)
            .await
    }
}
impl ExecutorCommands {
    /// Atomic proactive handoff: publish and queue a control continuation in
    /// one commit, with the business identity surviving a lost acknowledgement.
    /// System-origin event delivery has no fabricated direct-user anchor.
    pub async fn deliver_control_notice(
        &self,
        request: RequestId,
        session: SessionId,
        business_key: String,
        payload: serde_json::Value,
        text: String,
    ) -> Result<CommitReceipt<ExecutionView>> {
        if !matches!(
            payload.get("kind").and_then(serde_json::Value::as_str),
            Some(
                "assistant_notice"
                    | "integration_notice"
                    | "integration_instruction"
                    | "bot_instruction"
            )
        ) {
            return Err(KernelError::Invalid(
                "control notice requires a typed system payload".into(),
            ));
        }
        execution_receipt(
            self.0
                .command(
                    request,
                    Command::DeliverAndResume {
                        session,
                        business_key,
                        payload,
                        message: None,
                        text,
                    },
                )
                .await?,
        )
    }

    /// Guidance belongs to this execution. Admission and the terminal fence
    /// are checked in the same transaction as durable publication.
    pub async fn deliver_to_execution(
        &self,
        request: RequestId,
        session: SessionId,
        execution: ExecutionId,
        business_key: String,
        payload: serde_json::Value,
    ) -> Result<CommitReceipt<DeliveryView>> {
        let receipt = self
            .0
            .command(
                request,
                Command::DeliverToExecution {
                    session,
                    execution,
                    business_key,
                    payload,
                },
            )
            .await?;
        match receipt.value {
            Mutation::Delivery(value) => Ok(CommitReceipt {
                position: receipt.position,
                value,
            }),
            _ => Err(unexpected()),
        }
    }

    /// Atomically consume an idle-session delivery and queue its continuation.
    /// Restart can never observe the delivery consumed without a durable run.
    pub async fn resume_delivery(
        &self,
        request: RequestId,
        session: SessionId,
        delivery: agena_domain::CommandId,
        message: EntryId,
        text: String,
    ) -> Result<CommitReceipt<ExecutionView>> {
        execution_receipt(
            self.0
                .command(
                    request,
                    Command::ResumeDelivery {
                        session,
                        delivery,
                        message: Some(message),
                        text,
                    },
                )
                .await?,
        )
    }

    /// Publish a reliable business event once. A repeated business key with a
    /// different payload conflicts rather than making a second delivery.
    pub async fn deliver(
        &self,
        request: RequestId,
        session: SessionId,
        business_key: String,
        payload: serde_json::Value,
    ) -> Result<CommitReceipt<DeliveryView>> {
        let r = self
            .0
            .command(
                request,
                Command::Deliver {
                    session,
                    business_key,
                    payload,
                },
            )
            .await?;
        match r.value {
            Mutation::Delivery(value) => Ok(CommitReceipt {
                position: r.position,
                value,
            }),
            _ => Err(unexpected()),
        }
    }
    pub async fn consume_delivery(
        &self,
        request: RequestId,
        session: SessionId,
        delivery: agena_domain::CommandId,
        message: EntryId,
        text: String,
    ) -> Result<CommitReceipt<EntryId>> {
        let r = self
            .0
            .command(
                request,
                Command::ConsumeDelivery {
                    session,
                    delivery,
                    message,
                    text,
                },
            )
            .await?;
        match r.value {
            Mutation::Checkpoint(value) => Ok(CommitReceipt {
                position: r.position,
                value,
            }),
            _ => Err(unexpected()),
        }
    }
}
impl AttemptHandle {
    /// Persist only compaction policy statistics. Prompt membership and the
    /// installed checkpoint are changed by the separate compaction command.
    pub async fn record_compaction(
        &self,
        request: RequestId,
        policy_key: String,
        outcome: CompactionPolicyOutcome,
    ) -> Result<CommitReceipt<PromptWindowStatus>> {
        let receipt = self
            .inner
            .command(
                request,
                Command::RecordCompaction {
                    scope: self.scope,
                    policy_key,
                    outcome,
                },
            )
            .await?;
        match receipt.value {
            Mutation::PromptWindowStatus(value) => Ok(CommitReceipt {
                position: receipt.position,
                value,
            }),
            _ => Err(unexpected()),
        }
    }

    pub async fn notice(&self, request: RequestId, text: String) -> Result<CommitReceipt<EntryId>> {
        let r = self
            .inner
            .command(
                request,
                Command::Notice {
                    scope: self.scope,
                    text,
                },
            )
            .await?;
        match r.value {
            Mutation::Checkpoint(value) => Ok(CommitReceipt {
                position: r.position,
                value,
            }),
            _ => Err(unexpected()),
        }
    }
}
