//! Serialize each marker's writes and retain the user's latest intent while
//! HTTP acknowledgements and catalog reads are in flight.
use std::collections::BTreeMap;

use crate::{App, AppMessage, Route, SessionResource, UiResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum SessionMark {
    Pin,
    Favorite,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MarkWrite {
    pub session_id: i64,
    pub mark: SessionMark,
    pub value: bool,
    request_id: u64,
}

#[derive(Debug)]
struct PendingMark {
    confirmed: bool,
    desired: bool,
    write: MarkWrite,
}

#[derive(Debug, Default)]
pub(crate) struct SessionMarkWrites {
    pending: BTreeMap<(i64, SessionMark), PendingMark>,
    committed: BTreeMap<(i64, SessionMark), (i64, bool)>,
    next_id: u64,
}

impl SessionMarkWrites {
    pub fn is_idle(&self) -> bool {
        self.pending.is_empty()
    }

    fn visible_value(&self, session_id: i64, mark: SessionMark, version: i64, value: bool) -> bool {
        if let Some(pending) = self.pending.get(&(session_id, mark)) {
            return pending.desired;
        }
        self.committed
            .get(&(session_id, mark))
            .filter(|(committed_version, _)| version < *committed_version)
            .map_or(value, |(_, value)| *value)
    }

    /// Overlay only marker fields. Keeping the snapshot's actual version and
    /// execution fields lets transcript freshness checks keep doing their job.
    pub(crate) fn project(&self, session: &mut SessionResource) {
        session.pinned = self.visible_value(
            session.id,
            SessionMark::Pin,
            session.version,
            session.pinned,
        );
        session.favorite = self.visible_value(
            session.id,
            SessionMark::Favorite,
            session.version,
            session.favorite,
        );
    }

    fn toggle(
        &mut self,
        session_id: i64,
        mark: SessionMark,
        current: bool,
    ) -> (bool, Option<MarkWrite>) {
        if let Some(pending) = self.pending.get_mut(&(session_id, mark)) {
            pending.desired = !pending.desired;
            return (pending.desired, None);
        }
        let write = self.new_write(session_id, mark, !current);
        self.pending.insert(
            (session_id, mark),
            PendingMark {
                confirmed: current,
                desired: !current,
                write,
            },
        );
        (!current, Some(write))
    }

    fn new_write(&mut self, session_id: i64, mark: SessionMark, value: bool) -> MarkWrite {
        self.next_id = self.next_id.saturating_add(1);
        MarkWrite {
            session_id,
            mark,
            value,
            request_id: self.next_id,
        }
    }

    /// Returns the effective visible value and, if needed, one compensating
    /// write. Duplicate/late replies never apply to a later mutation.
    fn complete(
        &mut self,
        write: MarkWrite,
        result: Option<bool>,
    ) -> Option<(bool, Option<MarkWrite>)> {
        let key = (write.session_id, write.mark);
        let pending = self.pending.get(&key)?;
        if pending.write != write {
            return None;
        }
        let mut pending = self.pending.remove(&key)?;
        if let Some(confirmed) = result {
            pending.confirmed = confirmed;
        } else if pending.desired == write.value {
            pending.desired = pending.confirmed;
        }
        let desired = pending.desired;
        let next = if desired != pending.confirmed {
            pending.write = self.new_write(write.session_id, write.mark, desired);
            let next = pending.write;
            self.pending.insert(key, pending);
            Some(next)
        } else {
            None
        };
        Some((desired, next))
    }
}

impl App {
    pub(crate) fn toggle_session_pin(&mut self, session_id: i64, current: bool) -> bool {
        self.toggle_session_mark(session_id, SessionMark::Pin, current)
    }

    pub(crate) fn toggle_session_favorite(&mut self, session_id: i64, current: bool) -> bool {
        self.toggle_session_mark(session_id, SessionMark::Favorite, current)
    }

    fn toggle_session_mark(&mut self, session_id: i64, mark: SessionMark, current: bool) -> bool {
        let (desired, write) = self.session_mark_writes.toggle(session_id, mark, current);
        self.apply_session_mark(session_id, mark, desired);
        if let Some(write) = write {
            self.spawn_session_mark_write(write);
        }
        desired
    }

    fn spawn_session_mark_write(&self, write: MarkWrite) {
        let application = self.application.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = tokio::time::timeout(std::time::Duration::from_secs(30), async {
                match write.mark {
                    SessionMark::Pin => {
                        application
                            .set_session_pinned(write.session_id, write.value)
                            .await
                    }
                    SessionMark::Favorite => {
                        application
                            .set_session_favorite(write.session_id, write.value)
                            .await
                    }
                }
            })
            .await
            .map_err(|_| crate::UiFailure::internal("Session marker update timed out"))
            .and_then(|result| result.map_err(crate::UiFailure::from_backend));
            let _ = tx
                .send(AppMessage::SessionMarkUpdated { write, result })
                .await;
        });
    }

    fn apply_session_mark(&mut self, session_id: i64, mark: SessionMark, value: bool) {
        let (pinned, favorite) = match mark {
            SessionMark::Pin => (Some(value), None),
            SessionMark::Favorite => (None, Some(value)),
        };
        self.update_session_search_marks(session_id, pinned, favorite);
        if let Some(favorite) = favorite {
            self.sessions.set_item_favorite(session_id, favorite);
        }
        // Only the acknowledged field belongs to this operation. Replacing
        // the whole resource could revert a concurrent pin/favorite or run.
        if self.transcript.session_id == Some(session_id)
            && let Some(execution) = self.transcript.execution.as_mut()
        {
            match mark {
                SessionMark::Pin => execution.session.pinned = value,
                SessionMark::Favorite => execution.session.favorite = value,
            }
        }
        if let Route::Hub(state) = &mut self.current_route {
            state
                .presentation
                .set_pending_marks(session_id, pinned, favorite);
        }
    }

    pub(crate) fn handle_session_mark_updated(
        &mut self,
        write: MarkWrite,
        result: UiResult<SessionResource>,
    ) {
        let confirmed = result.as_ref().ok().map(|session| match write.mark {
            SessionMark::Pin => session.pinned,
            SessionMark::Favorite => session.favorite,
        });
        let Some((desired, next)) = self.session_mark_writes.complete(write, confirmed) else {
            return;
        };
        if let (Ok(session), Some(confirmed)) = (&result, confirmed) {
            self.session_mark_writes
                .committed
                .insert((write.session_id, write.mark), (session.version, confirmed));
        }
        self.apply_session_mark(write.session_id, write.mark, desired);
        if let Err(error) = result {
            self.flash_error(error);
        }
        if let Some(next) = next {
            self.spawn_session_mark_write(next);
        }
        if self.session_mark_writes.is_idle() {
            self.refresh_hub_after_mutation();
            self.request_sessions(false);
        }
    }
}

#[cfg(test)]
mod tests;
