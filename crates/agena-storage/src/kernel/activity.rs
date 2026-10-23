//! Background launch intent and outcome have their own authority. The launch
//! tool owns transcript membership; it does not own the activity's lifetime.

use agena_domain::{ActivityId, ContentKind};

use super::*;
use crate::backend::kernel::{ActivityScope, OutputScope};

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
    inner: Arc<KernelInner>,
    scope: ActivityScope,
}

impl ToolHandle {
    /// Commit this launch intent before starting the external process/task.
    /// Keep the request ID if persistence fails; a retry returns the same ID.
    pub async fn begin_activity(
        &self,
        request: RequestId,
        activity: BackgroundActivityStart,
    ) -> Result<CommitReceipt<ActivityHandle>> {
        let inner = &self.round.attempt.inner;
        let receipt = inner
            .command(
                request,
                Command::BeginActivity {
                    scope: self.round.attempt.scope,
                    tool: self.id,
                    activity,
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
        let receipt = self
            .inner
            .command(
                request,
                Command::EndActivity {
                    scope: self.scope,
                    outcome,
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
    pub async fn activity(
        &self,
        session: SessionId,
        activity: ActivityId,
    ) -> Result<ReadView<BackgroundActivityView>> {
        self.0.backend.activity(session, activity).await
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
