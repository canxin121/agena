//! User/automatic permission review is not network or plugin execution time.
//! Nested and concurrent callbacks for one live tool share a clock; overlapping
//! waits pause it once. Entries disappear with their last operation/guard.

use std::{
    collections::HashMap,
    future::Future,
    sync::{Arc, LazyLock, Mutex, Weak},
    time::Duration,
};
use tokio::{sync::Notify, time::Instant};

type Key = (i64, i64);
tokio::task_local! { static CONTEXT: Option<Key>; }

/// Scope execution budgets to one host call; this conveys no I/O authority.
pub async fn scope<F: Future>(key: Option<Key>, future: F) -> F::Output {
    CONTEXT.scope(key, future).await
}
static CLOCKS: LazyLock<Mutex<HashMap<Key, Weak<Clock>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Default)]
struct State {
    waits: usize,
    since: Option<Instant>,
    paused: Duration,
}
#[derive(Default)]
struct Clock {
    state: Mutex<State>,
    changed: Notify,
}

impl Clock {
    fn current() -> Arc<Self> {
        let key = CONTEXT
            .try_with(|key| *key)
            .ok()
            .flatten()
            .filter(|(session, call)| *session >= 0 && *call >= 0);
        let Some(key) = key else {
            return Arc::default();
        };
        let mut clocks = CLOCKS.lock().unwrap_or_else(|e| e.into_inner());
        clocks.retain(|_, clock| clock.strong_count() > 0);
        if let Some(clock) = clocks.get(&key).and_then(Weak::upgrade) {
            return clock;
        }
        let clock = Arc::new(Self::default());
        clocks.insert(key, Arc::downgrade(&clock));
        clock
    }
    fn snapshot(&self) -> (Duration, bool) {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        (
            state.paused + state.since.map(|since| since.elapsed()).unwrap_or_default(),
            state.waits > 0,
        )
    }
}

struct Wait(Arc<Clock>);
impl Wait {
    fn begin(clock: Arc<Clock>) -> Self {
        {
            let mut state = clock.state.lock().unwrap_or_else(|e| e.into_inner());
            if state.waits == 0 {
                state.since = Some(Instant::now());
            }
            state.waits += 1;
        }
        clock.changed.notify_waiters();
        Self(clock)
    }
}
impl Drop for Wait {
    fn drop(&mut self) {
        let mut state = self.0.state.lock().unwrap_or_else(|e| e.into_inner());
        state.waits -= 1;
        if state.waits == 0
            && let Some(since) = state.since.take()
        {
            state.paused += since.elapsed();
        }
        drop(state);
        self.0.changed.notify_waiters();
    }
}

/// Mark only a validated permission callback's wait. Dropping/cancelling the
/// future resumes the clock as well; this never creates an approval or grant.
pub async fn waiting<F: Future>(future: F) -> F::Output {
    let _wait = Wait::begin(Clock::current());
    future.await
}

#[derive(Debug)]
pub struct Elapsed;
impl std::fmt::Display for Elapsed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("operation exceeded its active time budget")
    }
}
impl std::error::Error for Elapsed {}

/// A deadline shared by sequential operations, excluding permission review.
pub struct Deadline {
    clock: Arc<Clock>,
    start: Instant,
    baseline: Duration,
    budget: Duration,
}
impl Deadline {
    pub fn new(budget: Duration) -> Self {
        let clock = Clock::current();
        let baseline = clock.snapshot().0;
        Self {
            clock,
            start: Instant::now(),
            baseline,
            budget,
        }
    }
    pub fn remaining(&self) -> Duration {
        self.budget.saturating_sub(
            self.start
                .elapsed()
                .saturating_sub(self.clock.snapshot().0.saturating_sub(self.baseline)),
        )
    }
    pub async fn run<F: Future>(&self, future: F) -> Result<F::Output, Elapsed> {
        timeout(self.remaining(), future).await
    }
}

/// A bounded operation whose budget excludes time spent awaiting permission.
/// Cancellation remains the caller's responsibility and is never paused.
pub async fn timeout<F: Future>(budget: Duration, future: F) -> Result<F::Output, Elapsed> {
    let clock = Clock::current();
    let start = Instant::now();
    let baseline = clock.snapshot().0;
    tokio::pin!(future);
    loop {
        let changed = clock.changed.notified();
        tokio::pin!(changed);
        changed.as_mut().enable();
        let (paused, waiting) = clock.snapshot();
        let elapsed = start
            .elapsed()
            .saturating_sub(paused.saturating_sub(baseline));
        let remaining = budget.saturating_sub(elapsed);
        tokio::select! {
            biased;
            output = &mut future => return Ok(output),
            _ = &mut changed => {},
            _ = tokio::time::sleep(remaining), if !waiting => return Err(Elapsed),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn context(call: i64) -> Option<Key> {
        Some((78134, call))
    }

    #[tokio::test]
    async fn review_wait_outlives_execution_budget() {
        scope(context(1), async {
            let result = timeout(Duration::from_millis(20), async {
                waiting(tokio::time::sleep(Duration::from_millis(80))).await;
                42
            })
            .await;
            assert_eq!(result.unwrap(), 42);
        })
        .await;
    }

    #[tokio::test]
    async fn active_work_still_times_out_after_review() {
        scope(context(2), async {
            assert!(
                timeout(Duration::from_millis(20), async {
                    waiting(tokio::time::sleep(Duration::from_millis(50))).await;
                    tokio::time::sleep(Duration::from_millis(100)).await;
                })
                .await
                .is_err()
            );
        })
        .await;
    }

    #[tokio::test]
    async fn cancelling_a_permission_wait_resumes_the_clock() {
        scope(context(3), async {
            let clock = Clock::current();
            {
                let pending = waiting(std::future::pending::<()>());
                tokio::pin!(pending);
                tokio::select! {
                    _ = &mut pending => unreachable!(),
                    _ = tokio::time::sleep(Duration::from_millis(10)) => {},
                }
                assert!(clock.snapshot().1);
            }
            assert!(!clock.snapshot().1);
            assert!(
                timeout(Duration::from_millis(5), std::future::pending::<()>())
                    .await
                    .is_err()
            );
        })
        .await;
    }

    #[tokio::test]
    async fn overlapping_waits_count_once_and_other_calls_keep_their_budget() {
        let same = context(4);
        scope(same.clone(), async {
            timeout(Duration::from_millis(25), async {
                let first = scope(
                    same.clone(),
                    waiting(tokio::time::sleep(Duration::from_millis(60))),
                );
                let second = scope(
                    same.clone(),
                    waiting(tokio::time::sleep(Duration::from_millis(80))),
                );
                let unrelated = scope(
                    context(5),
                    timeout(Duration::from_millis(10), std::future::pending::<()>()),
                );
                let (_, _, unrelated) = tokio::join!(first, second, unrelated);
                assert!(unrelated.is_err());
            })
            .await
            .unwrap();
        })
        .await;
    }
}
