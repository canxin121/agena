use std::{collections::VecDeque, sync::Mutex};

use agena_domain::KernelEpoch;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use super::{CommitPosition, ReadBudget, ReadLease, ReadView, SessionId, WatchCursor};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChangeKind {
    Metadata,
    Membership,
    Execution,
    Round,
    Tool,
    Interaction,
    Content,
    Activity,
}

/// Bounded listing facts captured in the same transaction as a command.
/// These describe resource scopes without loading transcript or configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionListState {
    pub session: SessionId,
    pub workspace_id: i64,
    pub parent_session_id: Option<SessionId>,
    pub favorite: bool,
    pub pinned: bool,
    pub include_in_lists: bool,
    pub is_subagent: bool,
    pub processing_state: agena_domain::SessionStateKind,
    pub queued_executions: u64,
    pub running_executions: u64,
    pub metadata_version: i64,
    pub updated_at_ms: i64,
    pub user_message_count: u64,
    pub last_message_at_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionListChange {
    pub before: Option<SessionListState>,
    pub after: Option<SessionListState>,
    /// At most two parent rows whose child count changed. Their own list
    /// membership and bucket counts remain unchanged.
    pub parents: Vec<SessionListState>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KernelChange {
    pub cursor: WatchCursor,
    pub session: SessionId,
    pub kind: ChangeKind,
    pub workspace_id: i64,
    /// False for temporary side conversations, including their deletion event.
    pub include_in_lists: bool,
    pub list_change: Option<SessionListChange>,
    /// Real visible entry identities, capped at 128. A client can refresh an
    /// older loaded entry without scanning a transcript tail.
    pub affected_entries: Vec<super::EntryId>,
    /// Membership removal or a change exceeding the identity budget requires
    /// a fresh bounded snapshot before applying subsequent events.
    pub requires_snapshot: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WatchError {
    #[error("change feed requires a new snapshot for server epoch {epoch}")]
    Reset { epoch: KernelEpoch },
    #[error("change feed has closed")]
    Closed,
}

pub struct KernelWatch {
    pub(crate) head: Option<WatchCursor>,
    pub(crate) needs_snapshot: bool,
    epoch: KernelEpoch,
    session: Option<SessionId>,
    replay: VecDeque<ReadView<KernelChange>>,
    receiver: broadcast::Receiver<Notice>,
    lagged: bool,
    _lease: Option<ReadLease>,
}

impl KernelWatch {
    pub fn with_lease(mut self, lease: ReadLease) -> Self {
        self._lease = Some(lease);
        self
    }

    pub fn head(&self) -> Option<&WatchCursor> {
        self.head.as_ref()
    }
    pub fn needs_snapshot(&self) -> bool {
        self.needs_snapshot
    }

    /// Drain the bounded committed feed without waiting for a future write.
    pub fn try_recv(&mut self) -> std::result::Result<Option<ReadView<KernelChange>>, WatchError> {
        if self.lagged {
            return Err(WatchError::Reset { epoch: self.epoch });
        }
        if let Some(change) = self.replay.pop_front() {
            return Ok(Some(change));
        }
        loop {
            match self.receiver.try_recv() {
                Ok(Notice::Change(change))
                    if self.session.is_none_or(|session| change.session == session) =>
                {
                    return Ok(Some(change));
                }
                Ok(Notice::Change(_)) => {}
                Ok(Notice::Reset) | Err(broadcast::error::TryRecvError::Lagged(_)) => {
                    self.lagged = true;
                    return Err(WatchError::Reset { epoch: self.epoch });
                }
                Err(broadcast::error::TryRecvError::Empty) => return Ok(None),
                Err(broadcast::error::TryRecvError::Closed) => return Err(WatchError::Closed),
            }
        }
    }

    pub async fn recv(&mut self) -> std::result::Result<ReadView<KernelChange>, WatchError> {
        if self.lagged {
            return Err(WatchError::Reset { epoch: self.epoch });
        }
        if let Some(change) = self.replay.pop_front() {
            return Ok(change);
        }
        loop {
            match self.receiver.recv().await {
                Ok(Notice::Change(change))
                    if self.session.is_none_or(|session| change.session == session) =>
                {
                    return Ok(change);
                }
                Ok(Notice::Change(_)) => {}
                Ok(Notice::Reset) => {
                    self.lagged = true;
                    return Err(WatchError::Reset { epoch: self.epoch });
                }
                Err(broadcast::error::RecvError::Closed) => return Err(WatchError::Closed),
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    self.lagged = true;
                    return Err(WatchError::Reset { epoch: self.epoch });
                }
            }
        }
    }
}

#[derive(Clone)]
enum Notice {
    Change(ReadView<KernelChange>),
    Reset,
}

struct FeedState {
    head: Option<CommitPosition>,
    floor: Option<CommitPosition>,
    replay: VecDeque<ReadView<KernelChange>>,
    sender: Option<broadcast::Sender<Notice>>,
}

/// Infrastructure-only publisher. It invokes no user callback and never waits
/// for a receiver. The one bounded ring is shared by all subscribers.
pub struct ChangeFeed {
    epoch: KernelEpoch,
    capacity: usize,
    budget: ReadBudget,
    state: Mutex<FeedState>,
}

impl ChangeFeed {
    pub fn new(epoch: KernelEpoch, capacity: usize) -> Self {
        Self::with_budget(epoch, capacity, ReadBudget::default())
    }

    pub fn with_budget(epoch: KernelEpoch, capacity: usize, budget: ReadBudget) -> Self {
        let capacity = capacity.max(1);
        let (sender, _) = broadcast::channel(capacity);
        Self {
            epoch,
            capacity,
            budget,
            state: Mutex::new(FeedState {
                head: None,
                floor: None,
                replay: VecDeque::new(),
                sender: Some(sender),
            }),
        }
    }

    pub fn publish(&self, position: CommitPosition, session: SessionId, kind: ChangeKind) {
        self.publish_entries(position, session, kind, Vec::new(), false);
    }

    pub fn publish_entries(
        &self,
        position: CommitPosition,
        session: SessionId,
        kind: ChangeKind,
        affected_entries: Vec<super::EntryId>,
        requires_snapshot: bool,
    ) {
        self.publish_session(
            position,
            session,
            0,
            true,
            kind,
            affected_entries,
            requires_snapshot,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub fn publish_session(
        &self,
        position: CommitPosition,
        session: SessionId,
        workspace_id: i64,
        include_in_lists: bool,
        kind: ChangeKind,
        affected_entries: Vec<super::EntryId>,
        requires_snapshot: bool,
    ) {
        self.publish_scoped(
            position,
            session,
            workspace_id,
            include_in_lists,
            kind,
            affected_entries,
            requires_snapshot,
            None,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub fn publish_scoped(
        &self,
        position: CommitPosition,
        session: SessionId,
        workspace_id: i64,
        include_in_lists: bool,
        kind: ChangeKind,
        affected_entries: Vec<super::EntryId>,
        requires_snapshot: bool,
        list_change: Option<SessionListChange>,
    ) {
        let change = KernelChange {
            cursor: WatchCursor {
                session,
                epoch: self.epoch,
                position,
            },
            session,
            kind,
            workspace_id,
            include_in_lists,
            list_change,
            affected_entries,
            requires_snapshot,
        };
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if state.sender.is_none() {
            return;
        }
        state.head = Some(position);
        let change = match self.budget.wrap(session, change) {
            Ok(change) => change,
            Err(_) => {
                // Post-commit resource pressure is a reset, never a failed
                // business receipt. No observer can retain unaccounted changes.
                state.replay.clear();
                state.floor = Some(position);
                if let Some(sender) = &state.sender {
                    let _ = sender.send(Notice::Reset);
                }
                return;
            }
        };
        state.replay.push_back(change.clone());
        while state.replay.len() > self.capacity {
            state.floor = state
                .replay
                .pop_front()
                .map(|change| change.cursor.position);
        }
        if let Some(sender) = &state.sender {
            let _ = sender.send(Notice::Change(change));
        }
    }

    pub fn watch(&self, session: SessionId, after: Option<WatchCursor>) -> KernelWatch {
        self.watch_inner(Some(session), after)
    }
    pub fn watch_all(&self, after: Option<WatchCursor>) -> KernelWatch {
        self.watch_inner(None, after)
    }
    pub fn invalidate(&self) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.replay.clear();
        state.floor = state.head;
        if let Some(sender) = &state.sender {
            let _ = sender.send(Notice::Reset);
        }
    }
    fn watch_inner(&self, session: Option<SessionId>, after: Option<WatchCursor>) -> KernelWatch {
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let receiver = match &state.sender {
            Some(sender) => sender.subscribe(),
            None => {
                let (sender, receiver) = broadcast::channel(1);
                drop(sender);
                receiver
            }
        };
        let resumable = after.as_ref().is_some_and(|cursor| {
            session.is_none_or(|session| cursor.session == session)
                && cursor.epoch == self.epoch
                && state.head.is_some_and(|head| cursor.position <= head)
                && state.floor.is_none_or(|floor| cursor.position >= floor)
        });
        let replay = if resumable {
            let position = after.as_ref().expect("resumable cursor").position;
            state
                .replay
                .iter()
                .filter(|change| {
                    session.is_none_or(|session| change.session == session)
                        && change.cursor.position > position
                })
                .cloned()
                .collect()
        } else {
            VecDeque::new()
        };
        KernelWatch {
            head: state.head.map(|position| WatchCursor {
                session: session
                    .or_else(|| state.replay.back().map(|change| change.session))
                    .unwrap_or_else(|| {
                        SessionId::try_from(1).expect("positive global cursor placeholder")
                    }),
                epoch: self.epoch,
                position,
            }),
            needs_snapshot: !resumable,
            epoch: self.epoch,
            session,
            replay,
            receiver,
            lagged: false,
            _lease: None,
        }
    }

    pub fn close(&self) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.sender = None;
        state.replay.clear();
    }
}
