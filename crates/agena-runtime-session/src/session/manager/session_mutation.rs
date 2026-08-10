use std::collections::HashMap;
use std::future::Future;
use std::sync::{
    Arc, Mutex as StdMutex, Weak,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

use tokio::sync::Semaphore;

use crate::AppError;

const DEFAULT_MAX_QUEUED_MUTATIONS_PER_SESSION: usize = 64;
const DEFAULT_MUTATION_WAIT_TIMEOUT: Duration = Duration::from_secs(30);

tokio::task_local! {
    static HELD_SESSION_MUTATION: i64;
}

/// Coordinates transactional mutations of a session without holding a mutex
/// guard across `.await` points.
///
/// A lane is a single-permit semaphore and is referenced weakly by the global
/// registry, so idle session ids disappear automatically. Waiting is bounded
/// by both count and time. `HELD_SESSION_MUTATION` rejects nested acquisition
/// before it can form either a self-deadlock or a cross-session lock cycle.
#[derive(Clone)]
pub(super) struct SessionMutationCoordinator {
    lanes: Arc<StdMutex<HashMap<i64, Weak<SessionMutationLane>>>>,
    max_queued_per_session: usize,
    wait_timeout: Duration,
}

struct SessionMutationLane {
    permit: Arc<Semaphore>,
    queued: AtomicUsize,
}

struct QueueReservation<'a> {
    queued: &'a AtomicUsize,
}

impl Drop for QueueReservation<'_> {
    fn drop(&mut self) {
        self.queued.fetch_sub(1, Ordering::AcqRel);
    }
}

impl SessionMutationCoordinator {
    pub(super) fn new() -> Self {
        Self::with_limits(
            DEFAULT_MAX_QUEUED_MUTATIONS_PER_SESSION,
            DEFAULT_MUTATION_WAIT_TIMEOUT,
        )
    }

    fn with_limits(max_queued_per_session: usize, wait_timeout: Duration) -> Self {
        assert!(max_queued_per_session > 0);
        Self {
            lanes: Arc::new(StdMutex::new(HashMap::new())),
            max_queued_per_session,
            wait_timeout,
        }
    }

    /// Run one mutation in the session's exclusive lane.
    ///
    /// This is deliberately closure/future based rather than returning a
    /// permit. The task-local invariant therefore covers the complete critical
    /// section and catches accidental nested coordinator calls immediately.
    pub(super) async fn run<T, F>(&self, session_id: i64, mutation: F) -> Result<T, AppError>
    where
        F: Future<Output = Result<T, AppError>>,
    {
        if let Ok(held_session_id) = HELD_SESSION_MUTATION.try_with(|held| *held) {
            return Err(AppError::NestedSessionMutation {
                held_session_id,
                requested_session_id: session_id,
            });
        }

        let lane = self.lane(session_id);
        let reservation = lane
            .queued
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |queued| {
                (queued < self.max_queued_per_session).then_some(queued + 1)
            })
            .map(|_| QueueReservation {
                queued: &lane.queued,
            })
            .map_err(|_| AppError::SessionMutationBusy(session_id))?;

        let permit =
            tokio::time::timeout(self.wait_timeout, Arc::clone(&lane.permit).acquire_owned())
                .await
                .map_err(|_| AppError::SessionMutationBusy(session_id))?
                .map_err(|_| {
                    AppError::Internal(format!(
                        "session {session_id} mutation coordinator closed unexpectedly"
                    ))
                })?;
        drop(reservation);

        HELD_SESSION_MUTATION
            .scope(session_id, async move {
                let result = mutation.await;
                drop(permit);
                result
            })
            .await
    }

    fn lane(&self, session_id: i64) -> Arc<SessionMutationLane> {
        let mut lanes = self
            .lanes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(lane) = lanes.get(&session_id).and_then(Weak::upgrade) {
            return lane;
        }
        let lane = Arc::new(SessionMutationLane {
            permit: Arc::new(Semaphore::new(1)),
            queued: AtomicUsize::new(0),
        });
        lanes.insert(session_id, Arc::downgrade(&lane));
        lane
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use std::time::Duration;

    use tokio::sync::Semaphore;

    use super::SessionMutationCoordinator;
    use crate::AppError;

    #[tokio::test]
    async fn same_session_is_serial_but_different_sessions_can_run_together() {
        let coordinator = SessionMutationCoordinator::with_limits(8, Duration::from_secs(1));
        let active_on_first = Arc::new(AtomicUsize::new(0));
        let peak_on_first = Arc::new(AtomicUsize::new(0));
        let all_active = Arc::new(AtomicUsize::new(0));
        let peak_all = Arc::new(AtomicUsize::new(0));
        let gate = Arc::new(Semaphore::new(0));
        let mut tasks = Vec::new();

        for session_id in [1, 1, 2] {
            let coordinator = coordinator.clone();
            let active_on_first = Arc::clone(&active_on_first);
            let peak_on_first = Arc::clone(&peak_on_first);
            let all_active = Arc::clone(&all_active);
            let peak_all = Arc::clone(&peak_all);
            let gate = Arc::clone(&gate);
            tasks.push(tokio::spawn(async move {
                coordinator
                    .run(session_id, async {
                        if session_id == 1 {
                            let active = active_on_first.fetch_add(1, Ordering::SeqCst) + 1;
                            peak_on_first.fetch_max(active, Ordering::SeqCst);
                        }
                        let active = all_active.fetch_add(1, Ordering::SeqCst) + 1;
                        peak_all.fetch_max(active, Ordering::SeqCst);
                        let permit = gate.acquire().await.expect("test gate remains open");
                        permit.forget();
                        all_active.fetch_sub(1, Ordering::SeqCst);
                        if session_id == 1 {
                            active_on_first.fetch_sub(1, Ordering::SeqCst);
                        }
                        Ok(())
                    })
                    .await
            }));
        }

        tokio::time::timeout(Duration::from_secs(1), async {
            while peak_all.load(Ordering::SeqCst) < 2 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("different sessions start concurrently");
        assert_eq!(peak_on_first.load(Ordering::SeqCst), 1);
        gate.add_permits(3);
        for task in tasks {
            task.await
                .expect("mutation task joins")
                .expect("mutation runs");
        }
        assert_eq!(peak_on_first.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn nested_mutation_is_rejected_instead_of_deadlocking() {
        let coordinator = SessionMutationCoordinator::with_limits(2, Duration::from_secs(1));
        let nested = coordinator.clone();
        let error = coordinator
            .run(7, async move { nested.run(8, async { Ok(()) }).await })
            .await
            .expect_err("nested session mutation must fail");

        assert!(matches!(
            error,
            AppError::NestedSessionMutation {
                held_session_id: 7,
                requested_session_id: 8,
            }
        ));
    }

    #[tokio::test]
    async fn waiters_are_bounded_and_time_out() {
        let coordinator = SessionMutationCoordinator::with_limits(1, Duration::from_millis(20));
        let gate = Arc::new(Semaphore::new(0));
        let holder_coordinator = coordinator.clone();
        let holder_gate = Arc::clone(&gate);
        let holder = tokio::spawn(async move {
            holder_coordinator
                .run(3, async move {
                    let permit = holder_gate.acquire().await.expect("test gate remains open");
                    permit.forget();
                    Ok(())
                })
                .await
        });
        tokio::task::yield_now().await;

        let error = coordinator
            .run(3, async { Ok(()) })
            .await
            .expect_err("waiter reaches its deadline");
        assert!(matches!(error, AppError::SessionMutationBusy(3)));

        gate.add_permits(1);
        holder
            .await
            .expect("holder joins")
            .expect("holder succeeds");
    }
}
