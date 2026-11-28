//! Background launch intent and outcome have their own authority. The launch
//! tool owns transcript membership; it does not own the activity's lifetime.

use agena_domain::{ActivityId, ContentKind};

use super::*;
use crate::backend::kernel::{ActivityScope, OutputScope, Scope};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackgroundActivityKind {
    Process,
    Terminal,
    Agent,
    Monitor,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackgroundActivityStart {
    pub kind: BackgroundActivityKind,
    pub label: String,
}

/// A host-issued invocation identity, shared by every activity started by
/// this command. Command arguments stay in the invocation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginCommandActivityOrigin {
    pub invocation: ExecutionId,
    pub plugin_id: String,
    pub command_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackgroundActivityNotice {
    pub text: String,
    pub origin_message_id: EntryId,
    pub wake: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wake_budget: Option<BackgroundActivityWakeBudget>,
    /// An inline notice presents exactly this prefix and terminal result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_position: Option<ActivityOutputPosition>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackgroundActivityWakeBudget {
    pub source: String,
    pub limit: usize,
}

/// The prefix already presented to the activity's owning conversation. The
/// transcript and retained output remain available independently of this cursor.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivityOutputPosition {
    pub cursor: Option<TextCursor>,
    pub lost_bytes: u64,
    pub outcome_read: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub packet_offset: Option<u64>,
}

/// A runtime tool advances a cursor only in its successful result transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivityOutputRead {
    pub activity: ActivityId,
    pub position: ActivityOutputPosition,
}

/// The runtime records an observed outcome, including an uncertain/interrupted
/// external effect. Saving this result again never launches the effect again.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum BackgroundActivityOutcome {
    Succeeded { value: serde_json::Value },
    Failed { diagnostic: String },
    Stopped,
    Interrupted { diagnostic: String },
}

impl BackgroundActivityOutcome {
    pub fn capture_end(&self) -> CaptureEnd {
        match self {
            Self::Interrupted { diagnostic } => CaptureEnd::Interrupted {
                diagnostic: diagnostic.clone(),
            },
            _ => CaptureEnd::Complete,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BackgroundActivityView {
    pub id: ActivityId,
    pub origin_session_id: SessionId,
    pub launch_tool: ToolId,
    pub launch_entry: EntryId,
    pub start: BackgroundActivityStart,
    /// None denotes a committed launch intent with a live lease. Startup
    /// recovery records Interrupted; it never reissues an external launch.
    pub outcome: Option<BackgroundActivityOutcome>,
    pub created_at_ms: i64,
    pub finished_at_ms: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActiveActivityCursor {
    pub session: SessionId,
    pub after: ActivityId,
}

#[derive(Debug, Clone, Serialize)]
pub struct ActiveActivityPage {
    pub activities: Vec<BackgroundActivityView>,
    pub next: Option<ActiveActivityCursor>,
    pub has_more: bool,
}

/// Minted only by a running, authorized runtime tool. This handle cannot finish
/// or retry an attempt, execute inherited work, or open unrelated content.
#[derive(Clone)]
pub struct ActivityHandle {
    pub(super) inner: Arc<KernelInner>,
    pub(super) scope: ActivityScope,
}

impl ToolHandle {
    /// Commit this launch intent before starting the external process/task.
    /// Keep the request ID if persistence fails; a retry returns the same ID.
    pub async fn begin_activity(
        &self,
        request: RequestId,
        activity: BackgroundActivityStart,
    ) -> Result<CommitReceipt<ActivityHandle>> {
        self.begin_activity_with_output(request, activity, None).await
    }

    pub async fn begin_activity_with_output(
        &self,
        request: RequestId,
        activity: BackgroundActivityStart,
        output_retention: Option<ActivityOutputRetention>,
    ) -> Result<CommitReceipt<ActivityHandle>> {
        let inner = &self.round.attempt.inner;
        let receipt = inner
            .command(
                request,
                Command::BeginActivity {
                    scope: self.round.attempt.scope,
                    tool: self.id,
                    activity,
                    output_retention,
                },
            )
            .await?;
        match receipt.value {
            Mutation::Activity(id) => Ok(CommitReceipt {
                position: receipt.position,
                value: ActivityHandle {
                    inner: inner.clone(),
                    scope: ActivityScope {
                        id,
                        session: self.round.attempt.scope.session,
                        epoch: inner.epoch,
                    },
                },
            }),
            _ => Err(unexpected()),
        }
    }
}

impl ActivityHandle {
    pub fn id(&self) -> ActivityId {
        self.scope.id
    }
    pub fn session_id(&self) -> SessionId {
        self.scope.session
    }
    pub async fn end(
        &self,
        request: RequestId,
        outcome: BackgroundActivityOutcome,
    ) -> Result<CommitReceipt<ActivityId>> {
        self.end_with_notice(request, outcome, None).await
    }

    /// Settle the activity and enqueue its owner's notice in one commit.
    pub async fn end_with_notice(
        &self,
        request: RequestId,
        outcome: BackgroundActivityOutcome,
        notice: Option<BackgroundActivityNotice>,
    ) -> Result<CommitReceipt<ActivityId>> {
        self.end_with_output(request, outcome, notice, None).await
    }

    pub async fn end_with_output(
        &self,
        request: RequestId,
        outcome: BackgroundActivityOutcome,
        notice: Option<BackgroundActivityNotice>,
        output_terminal: Option<ActivityOutputTerminal>,
    ) -> Result<CommitReceipt<ActivityId>> {
        let receipt = self
            .inner
            .command(
                request,
                Command::EndActivity {
                    scope: self.scope,
                    outcome,
                    notice,
                    output_terminal,
                },
            )
            .await?;
        match receipt.value {
            Mutation::Activity(id) => Ok(CommitReceipt {
                position: receipt.position,
                value: id,
            }),
            _ => Err(unexpected()),
        }
    }

    pub(super) async fn open_output(
        &self,
        request: RequestId,
        kind: ContentKind,
    ) -> Result<CommitReceipt<Arc<super::output::Producer>>> {
        let receipt = self
            .inner
            .command(
                request,
                Command::OpenActivityOutput {
                    scope: self.scope,
                    kind,
                },
            )
            .await?;
        match receipt.value {
            Mutation::Content(stream) => Ok(CommitReceipt {
                position: receipt.position,
                value: super::output::Producer::new(
                    self.inner.clone(),
                    OutputScope::Activity(self.scope),
                    stream,
                )?,
            }),
            _ => Err(unexpected()),
        }
    }
}

impl KernelReader {
    pub async fn activity_output_position(
        &self,
        session: SessionId,
        activity: ActivityId,
    ) -> Result<ActivityOutputPosition> {
        self.0
            .backend
            .activity_output_position(session, activity)
            .await
    }

    /// Recent listed activities whose launch is still in the originating transcript.
    pub async fn session_activities_by_label(
        &self,
        session: SessionId,
        prefix: &str,
        limit: usize,
    ) -> Result<ReadView<Vec<BackgroundActivityView>>> {
        self.0
            .backend
            .session_activities_by_label(session, prefix, limit)
            .await
    }

    pub async fn activity(
        &self,
        session: SessionId,
        activity: ActivityId,
    ) -> Result<ReadView<BackgroundActivityView>> {
        self.0.backend.activity(session, activity, false).await
    }

    pub async fn listed_activity(
        &self,
        session: SessionId,
        activity: ActivityId,
    ) -> Result<ReadView<BackgroundActivityView>> {
        self.0.backend.activity(session, activity, true).await
    }
    /// Origin-owned activities only; inherited transcript references grant
    /// reads through activity(), never work scheduling or write capability.
    pub async fn active_activities(
        &self,
        session: SessionId,
        after: Option<ActiveActivityCursor>,
        limit: usize,
    ) -> Result<ReadView<ActiveActivityPage>> {
        self.0
            .backend
            .active_activities(session, after, limit)
            .await
    }
}

impl SessionCommands {
    /// Commit a command's operation and an activity under the same request ID.
    pub async fn begin_plugin_command_activity(
        &self,
        request: RequestId,
        session: SessionId,
        origin: PluginCommandActivityOrigin,
        activity: BackgroundActivityStart,
        retention: ActivityOutputRetention,
    ) -> Result<CommitReceipt<(EntryId, ActivityHandle)>> {
        let receipt = self.0.command(request, Command::BeginPluginCommandActivity {
            scope: Scope {
                session,
                execution: origin.invocation,
                generation: AttemptGeneration::INITIAL,
                epoch: self.0.epoch,
            },
            origin,
            activity,
            retention,
        }).await?;
        match receipt.value {
            Mutation::PluginCommandActivity { anchor, id } => Ok(CommitReceipt {
                position: receipt.position,
                value: (anchor, ActivityHandle {
                    inner: self.0.clone(),
                    scope: ActivityScope { id, session, epoch: self.0.epoch },
                }),
            }),
            _ => Err(unexpected()),
        }
    }

    /// Remove a settled, origin-owned activity from the session registry.
    pub async fn unlist_settled_activity(
        &self,
        request: RequestId,
        session: SessionId,
        activity: ActivityId,
    ) -> Result<CommitReceipt<bool>> {
        let receipt = self.0.command(request, Command::UnlistSettledActivity { session, activity })
            .await?;
        match receipt.value {
            Mutation::ActivityUnlisted(changed) => Ok(CommitReceipt {
                position: receipt.position,
                value: changed,
            }),
            _ => Err(unexpected()),
        }
    }
}
