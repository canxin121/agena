//! Session configuration, immutable branch membership and durable deliveries.
use super::*;
use agena_domain::{
    PermissionConfig, SessionConversation, SessionRelationKind, SessionStateKind, SubtaskStatus,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct SessionConfiguration {
    pub parent_session_id: Option<SessionId>,
    pub relation_kind: SessionRelationKind,
    pub conversation: Option<SessionConversation>,
    pub permission: PermissionConfig,
    pub permission_ceiling: PermissionConfig,
    pub capability_denied_tool_names: BTreeSet<String>,
    pub effective_workspace_root: Option<PathBuf>,
    pub task: Option<SessionTaskConfiguration>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionTaskConfiguration {
    pub id: String,
    pub status: SubtaskStatus,
    pub started_at_ms: Option<i64>,
    pub finished_at_ms: Option<i64>,
    pub failure: Option<agena_failure::Failure>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct SessionConfigurationChange {
    pub parent_session_id: Option<SessionId>,
    pub relation_kind: Option<SessionRelationKind>,
    pub conversation: Option<Option<SessionConversation>>,
    pub permission: Option<PermissionConfig>,
    pub permission_ceiling: Option<PermissionConfig>,
    pub capability_denied_tool_names: Option<BTreeSet<String>>,
    pub effective_workspace_root: Option<Option<PathBuf>>,
    pub task: Option<Option<SessionTaskConfiguration>>,
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
    pub parent_id: Option<SessionId>,
    pub roots_only: bool,
    pub exclude_subagents: bool,
    pub temporary_only: bool,
    pub tasks_only: bool,
    pub search: Option<String>,
    pub favorite: Option<bool>,
    pub pinned: Option<bool>,
    pub states: Vec<SessionStateKind>,
    pub before: Option<SessionSummaryCursor>,
    pub offset: u64,
    pub limit: usize,
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

impl SessionCommands {
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
                .command(request, Command::DeleteSession { session })
                .await?,
        )
    }
    /// Continue a completed, origin-owned user request with a new generation.
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
    pub async fn find_task(
        &self,
        parent: SessionId,
        task_id: &str,
    ) -> Result<ReadView<Option<SessionId>>> {
        self.0.backend.find_task(parent, task_id).await
    }

    pub async fn latest_round(
        &self,
        session: SessionId,
    ) -> Result<ReadView<Option<ProviderRoundView>>> {
        self.0.backend.latest_round(session).await
    }
    pub async fn configuration(
        &self,
        session: SessionId,
    ) -> Result<ReadView<SessionConfiguration>> {
        self.0.backend.configuration(session).await
    }
    pub async fn query_summaries(
        &self,
        query: SummaryQuery,
    ) -> Result<ReadView<SessionSummaryPage>> {
        self.0.backend.query_summaries(query).await
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
    pub fn watch_all(&self, after: Option<WatchCursor>) -> Result<KernelWatch> {
        self.0.backend.watch_all(after)
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
                        message,
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
