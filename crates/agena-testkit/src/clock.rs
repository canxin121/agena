//! A frozen, case-owned clock for real scheduler deadlines and explicit virtual
//! sleeps/timeouts. Advancing hours never requires waiting hours of wall time.
//!
//! This does not pause Tokio globally, alter HTTP/cleanup safety budgets, or
//! replace arbitrary plugin system clocks. A scheduler acknowledgement is an
//! admission fence, not proof that a delivery has finished; observe real job
//! history or a model/tool gate for completion.

mod state;

use std::{future::Future, path::PathBuf, sync::Arc, time::Duration};

use anyhow::{Context, Result, anyhow, ensure};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use state::{ClockState, duration_millis, exact_millis};

const POLL: Duration = Duration::from_millis(5);
const OBSERVATION_TIMEOUT: Duration = Duration::from_secs(8);

/// A manually advanced clock with millisecond precision. Cloned handles and
/// separate `case.clock()` handles serialize updates through the owning case.
/// The clock remains frozen between explicit `advance`, `advance_to`, or `set`
/// calls, including across product restarts.
#[derive(Clone, Debug)]
pub struct TestClock {
    path: PathBuf,
    gate: Arc<Mutex<()>>,
}

/// Compatibility name for the former scheduler-only clock driver.
pub type ClockDriver = TestClock;

/// Acknowledged scheduler admission boundary. Already admitted deliveries may
/// continue while admissions are paused; their count is reported explicitly.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SchedulerObservation {
    pub revision: u64,
    pub now_ms: i64,
    pub scheduler_paused: bool,
    pub active_deliveries: usize,
    pub process_id: u32,
}

impl TestClock {
    pub(crate) fn new(path: PathBuf, gate: Arc<Mutex<()>>) -> Self {
        Self { path, gate }
    }

    pub async fn now(&self) -> Result<DateTime<Utc>> {
        let _guard = self.gate.lock().await;
        self.read().await?.now()
    }

    /// Virtual monotonic elapsed time. Explicit wall-clock rewinds via `set`
    /// or `freeze` do not rewind this counter or already scheduled durations.
    pub async fn elapsed(&self) -> Result<Duration> {
        let _guard = self.gate.lock().await;
        Ok(Duration::from_millis(self.read().await?.elapsed_ms))
    }

    async fn read(&self) -> Result<ClockState> {
        let raw = tokio::fs::read_to_string(&self.path)
            .await
            .with_context(|| format!("reading test clock {}", self.path.display()))?;
        ClockState::parse(&raw)
    }

    /// Set the frozen wall time without changing monotonic elapsed time. Use
    /// `advance`/`advance_to` when duration-based sleepers should also advance.
    pub async fn freeze(&self, now: DateTime<Utc>) -> Result<()> {
        let now_ms = exact_millis(now)?;
        let _guard = self.gate.lock().await;
        let mut state = self.read().await?;
        state.now_ms = now_ms;
        self.write(&mut state).await
    }

    pub async fn set(&self, now: DateTime<Utc>) -> Result<()> {
        self.freeze(now).await
    }

    /// Advance wall and monotonic time together. Sub-millisecond values,
    /// negative durations and overflows fail without publishing any change.
    pub async fn advance(&self, duration: chrono::Duration) -> Result<DateTime<Utc>> {
        ensure!(
            duration >= chrono::Duration::zero(),
            "test clock must not move backwards via advance; use set for explicit wall-clock rewinds"
        );
        ensure!(
            duration == chrono::Duration::milliseconds(duration.num_milliseconds()),
            "test clock has millisecond precision; sub-millisecond durations are unsupported"
        );
        let _guard = self.gate.lock().await;
        let state = self.read().await?;
        self.advance_state(state, duration).await
    }

    /// Advance to an exact wall deadline and move duration-based timers by
    /// the same amount. Unlike `set`, this rejects a backwards movement.
    pub async fn advance_to(&self, now: DateTime<Utc>) -> Result<DateTime<Utc>> {
        exact_millis(now)?;
        let _guard = self.gate.lock().await;
        let state = self.read().await?;
        let duration = now - state.now()?;
        ensure!(
            duration >= chrono::Duration::zero(),
            "advance_to cannot move test time backwards; use set for an explicit wall-clock rewind"
        );
        self.advance_state(state, duration).await
    }

    async fn advance_state(
        &self,
        mut state: ClockState,
        duration: chrono::Duration,
    ) -> Result<DateTime<Utc>> {
        let next = state
            .now()?
            .checked_add_signed(duration)
            .context("test clock wall time overflow")?;
        state.now_ms = exact_millis(next)?;
        state.elapsed_ms = state
            .elapsed_ms
            .checked_add(duration.num_milliseconds().try_into()?)
            .context("test clock elapsed time overflow")?;
        self.write(&mut state).await?;
        Ok(next)
    }

    /// Wait on virtual monotonic time. Dropping this future cancels the wait.
    /// The duration starts when this future is polled; for races where the
    /// advance may happen first, use an absolute `sleep_until_elapsed` deadline.
    pub async fn sleep(&self, duration: Duration) -> Result<()> {
        let millis = duration_millis(duration)?;
        let deadline = self
            .elapsed()
            .await?
            .as_millis()
            .checked_add(u128::from(millis))
            .context("test clock deadline overflow")?;
        let deadline = u64::try_from(deadline).context("test clock deadline overflow")?;
        self.sleep_until_elapsed(Duration::from_millis(deadline))
            .await
    }

    /// Wait until the absolute virtual monotonic deadline. Registration order
    /// cannot miss a jump: an already reached deadline completes immediately.
    pub async fn sleep_until_elapsed(&self, deadline: Duration) -> Result<()> {
        let deadline_ms = duration_millis(deadline)?;
        loop {
            if self.elapsed().await?.as_millis() >= u128::from(deadline_ms) {
                return Ok(());
            }
            tokio::time::sleep(POLL).await;
        }
    }

    /// Wait for a wall deadline; explicit wall rewinds postpone this wait.
    pub async fn sleep_until(&self, deadline: DateTime<Utc>) -> Result<()> {
        exact_millis(deadline)?;
        loop {
            if self.now().await? >= deadline {
                return Ok(());
            }
            tokio::time::sleep(POLL).await;
        }
    }

    /// Run a future with a virtual monotonic deadline. If the operation and
    /// timeout are ready together, the completed operation wins deterministically.
    /// Keep the test case's real timeout enabled as a separate hang safeguard.
    pub async fn timeout<F: Future>(&self, duration: Duration, future: F) -> Result<F::Output> {
        let millis = duration_millis(duration)?;
        let elapsed_ms = u64::try_from(self.elapsed().await?.as_millis())?;
        let deadline_ms = elapsed_ms
            .checked_add(millis)
            .context("test clock deadline overflow")?;
        tokio::select! {
            biased;
            result = future => Ok(result),
            result = self.sleep_until_elapsed(Duration::from_millis(deadline_ms)) => {
                result?;
                Err(anyhow!("test clock virtual timeout after {duration:?}"))
            }
        }
    }

    /// Require a fresh acknowledgement, even after a product restart. This
    /// bumps the revision so an old observation cannot satisfy the barrier.
    /// Due deliveries may still be running when the scheduler acknowledges it.
    pub async fn wait_observed(&self) -> Result<SchedulerObservation> {
        let _guard = self.gate.lock().await;
        let mut state = self.read().await?;
        self.write(&mut state).await?;
        self.wait_revision(&state).await
    }

    /// Stop new admissions and wait for the scheduler to acknowledge the
    /// boundary. Previously admitted deliveries may finish normally. For a
    /// fully idle boundary, verify the returned `active_deliveries` is zero.
    pub async fn pause_scheduler(&self) -> Result<SchedulerObservation> {
        self.set_scheduler_paused(true).await
    }

    /// Release the admission gate and wait for an observed scheduler pass.
    pub async fn resume_scheduler(&self) -> Result<SchedulerObservation> {
        self.set_scheduler_paused(false).await
    }

    async fn set_scheduler_paused(&self, paused: bool) -> Result<SchedulerObservation> {
        let _guard = self.gate.lock().await;
        let mut state = self.read().await?;
        state.scheduler_paused = paused;
        self.write(&mut state).await?;
        self.wait_revision(&state).await
    }

    async fn wait_revision(&self, expected: &ClockState) -> Result<SchedulerObservation> {
        let path = self.path.with_extension("observed.json");
        tokio::time::timeout(OBSERVATION_TIMEOUT, async {
            loop {
                match tokio::fs::read(&path).await {
                    Ok(raw) => {
                        let observation: SchedulerObservation = serde_json::from_slice(&raw).context("decode scheduler clock acknowledgement")?;
                        if observation.revision >= expected.revision {
                            ensure!(observation.scheduler_paused == expected.scheduler_paused, "scheduler clock acknowledgement has an unexpected admission state");
                            return Ok(observation);
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error).context("read scheduler clock acknowledgement"),
                }
                tokio::time::sleep(POLL).await;
            }
        })
        .await
        .with_context(|| format!("scheduler did not acknowledge test clock revision {}; confirm that an opted-in debug Agena child is running", expected.revision))?
    }

    async fn write(&self, state: &mut ClockState) -> Result<()> {
        state.revise()?;
        let tmp = self
            .path
            .with_extension(format!("{}.swap", uuid::Uuid::new_v4()));
        tokio::fs::write(&tmp, serde_json::to_vec(state)?)
            .await
            .with_context(|| format!("prepare test clock update {}", tmp.display()))?;
        if let Err(error) = tokio::fs::rename(&tmp, &self.path).await {
            let _ = tokio::fs::remove_file(&tmp).await;
            return Err(error).context("atomically publish test clock");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
