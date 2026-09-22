//! Persistent PTY sessions, owned by the same runtime as ordinary shell jobs.
//!
//! One bounded driver per live terminal owns its OS handles. Tool calls share
//! an interaction lock, while stop/shutdown bypass that lock. Neither output
//! backpressure nor a blocked input writer may prevent process cleanup.

mod call;
mod driver;
#[cfg(all(test, unix))]
mod lifecycle_tests;
mod osc_guard;
mod protocol;
mod state;
#[cfg(test)]
mod tests;

use crate::{MonitorError, MonitorListener};
use agena_domain::{ProcessEvent, ProcessStatus, ProcessSummary, TerminalScreen};
use agena_runtime_contracts::part::ShellSignal;
use state::{Control, Request, State};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex, MutexGuard, mpsc},
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

const MAX_ACTIVE: usize = 16;
const MAX_HISTORY: usize = 64;
pub(crate) const MAX_INPUT_BYTES: usize = 64 * 1024;
const MAX_BUFFER_BYTES: usize = 1024 * 1024;
const MAX_BUFFER_EVENTS: usize = 2048;
const MAX_OUTPUT_BYTES: usize = 16 * 1024;
const MAX_WAIT_MS: u64 = 30_000;

/// Trusted owner; never deserialized from model tool arguments. Stateless MCP
/// calls share the runtime's workspace owner, not an Agena conversation owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalOwner {
    pub workspace: PathBuf,
    pub session_id: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct TerminalStartParams {
    pub process_id: Option<String>,
    pub owner: TerminalOwner,
    pub command: Vec<String>,
    pub display_command: String,
    pub description: String,
    pub workdir: PathBuf,
    pub env: HashMap<String, String>,
    pub rows: u16,
    pub cols: u16,
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct TerminalRead {
    pub summary: ProcessSummary,
    pub events: Vec<ProcessEvent>,
    pub output: String,
    pub last_seq: u64,
    pub has_more: bool,
    pub dropped_bytes: u64,
    pub screen: TerminalScreen,
}

pub struct TerminalRegistry {
    sessions: Mutex<HashMap<String, Arc<State>>>,
    listener: Option<Arc<dyn MonitorListener>>,
    runtime: Option<tokio::runtime::Handle>,
    closed: std::sync::atomic::AtomicBool,
}

impl Default for TerminalRegistry {
    fn default() -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
            listener: None,
            runtime: tokio::runtime::Handle::try_current().ok(),
            closed: std::sync::atomic::AtomicBool::new(false),
        }
    }
}

impl std::fmt::Debug for TerminalRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TerminalRegistry")
            .field("sessions", &lock(&self.sessions).len())
            .finish()
    }
}

impl TerminalRegistry {
    pub(crate) fn from_handle(handle: tokio::runtime::Handle) -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
            listener: None,
            runtime: Some(handle),
            closed: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// Close admission and request termination even when an executor retains
    /// this registry after runtime shutdown. It does not block the async caller.
    pub fn shutdown(&self) {
        let sessions = lock(&self.sessions);
        self.closed
            .store(true, std::sync::atomic::Ordering::Release);
        for state in sessions.values() {
            state.request_stop(true);
        }
    }

    /// End the PTYs of one actual Agena session, not of an individual turn.
    pub fn stop_session(&self, session_id: i64) {
        for state in lock(&self.sessions).values() {
            if state.owner.session_id == Some(session_id) {
                state.request_stop(true);
            }
        }
    }

    pub(crate) fn set_listener(&mut self, listener: Arc<dyn MonitorListener>) {
        self.listener = Some(listener);
    }

    pub fn start(
        &self,
        params: TerminalStartParams,
        wait_ms: u64,
    ) -> Result<TerminalRead, MonitorError> {
        self.start_cancellable(params, wait_ms, CancellationToken::new())
    }

    pub(crate) fn start_cancellable(
        &self,
        params: TerminalStartParams,
        wait_ms: u64,
        cancel: CancellationToken,
    ) -> Result<TerminalRead, MonitorError> {
        call::check(&cancel)?;
        validate_size(params.rows, params.cols)?;
        validate_wait(wait_ms)?;
        if params.command.is_empty() || !params.workdir.is_dir() {
            return Err(invalid(
                "terminal requires a command and an existing working directory",
            ));
        }
        if params
            .timeout_ms
            .is_some_and(|ms| ms == 0 || ms > 86_400_000)
        {
            return Err(invalid(
                "terminal timeout_ms must be 1–86400000 ms or omitted",
            ));
        }
        let id = params
            .process_id
            .clone()
            .unwrap_or_else(|| format!("pty_{}", uuid::Uuid::new_v4().simple()));
        if id.is_empty() {
            return Err(invalid("terminal process id must not be empty"));
        }
        let (tx, rx) = mpsc::sync_channel(8);
        let state;
        {
            let mut sessions = lock(&self.sessions);
            if self.closed.load(std::sync::atomic::Ordering::Acquire) {
                return Err(invalid("terminal registry is shut down"));
            }
            if let Some(existing) = sessions.get(&id) {
                if existing.owner != params.owner
                    || existing.command != params.display_command
                    || existing.workdir != params.workdir
                {
                    return Err(invalid(
                        "terminal launch identity belongs to another operation",
                    ));
                }
                let existing = Arc::clone(existing);
                drop(sessions);
                // Replaying a launch returns the same output/identity and never spawns again.
                return existing.read_cancellable(Some(0), wait_ms, None, &cancel);
            }
            if sessions.values().filter(|s| s.is_running()).count() >= MAX_ACTIVE {
                return Err(invalid(
                    "too many live terminals (maximum 16); stop an unused terminal first",
                ));
            }
            while sessions.len() >= MAX_HISTORY {
                let oldest = sessions
                    .iter()
                    .filter(|(_, s)| !s.is_running())
                    .min_by_key(|(_, s)| s.started_at_ms)
                    .map(|(id, _)| id.clone());
                if let Some(oldest) = oldest {
                    sessions.remove(&oldest);
                } else {
                    break;
                }
            }
            state = Arc::new(State::new(id.clone(), &params, tx, self.listener.clone()));
            sessions.insert(id, Arc::clone(&state));
        }
        state.notify_started();
        let worker_state = Arc::clone(&state);
        let launch_cancel = cancel.clone();
        let runtime = self.runtime.clone();
        if let Err(error) = std::thread::Builder::new()
            .name("agena-pty".into())
            .spawn(move || {
                // Runtime activity/completion observers spawn Tokio tasks.
                // Native PTY threads must enter their owning runtime first.
                let _entered = runtime.as_ref().map(tokio::runtime::Handle::enter);
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    driver::run(&worker_state, params, rx, launch_cancel);
                }));
                if result.is_err() {
                    worker_state.finish(ProcessStatus::Failed, None, "terminal_driver_panicked");
                }
            })
        {
            state.append(format!("Terminal driver could not start: {error}\r\n").as_bytes());
            state.finish(ProcessStatus::Failed, None, "driver_start_failed");
        }
        state.read_cancellable(None, wait_ms, None, &cancel)
    }

    pub fn contains(&self, id: &str) -> bool {
        lock(&self.sessions).contains_key(id)
    }

    pub fn is_owned(&self, id: &str, owner: &TerminalOwner) -> bool {
        lock(&self.sessions)
            .get(id)
            .is_some_and(|s| &s.owner == owner)
    }

    pub fn authorize(&self, id: &str, owner: &TerminalOwner) -> Result<(), MonitorError> {
        self.lookup(id, Some(owner)).map(|_| ())
    }

    pub fn list(&self) -> Vec<ProcessSummary> {
        lock(&self.sessions).values().map(|s| s.summary()).collect()
    }

    pub fn read(
        &self,
        id: &str,
        owner: &TerminalOwner,
        since_seq: Option<u64>,
        wait_ms: u64,
        limit: Option<u32>,
    ) -> Result<TerminalRead, MonitorError> {
        self.lookup(id, Some(owner))?
            .read(since_seq, wait_ms, limit)
    }

    pub(crate) fn read_cancellable(
        &self,
        id: &str,
        owner: &TerminalOwner,
        since_seq: Option<u64>,
        wait_ms: u64,
        limit: Option<u32>,
        cancel: &CancellationToken,
    ) -> Result<TerminalRead, MonitorError> {
        self.lookup(id, Some(owner))?
            .read_cancellable(since_seq, wait_ms, limit, cancel)
    }

    pub(crate) fn read_unscoped(
        &self,
        id: &str,
        since_seq: u64,
        wait_ms: u64,
        limit: Option<u32>,
    ) -> Result<TerminalRead, MonitorError> {
        self.lookup(id, None)?.read(Some(since_seq), wait_ms, limit)
    }

    pub fn write(
        &self,
        id: &str,
        owner: &TerminalOwner,
        chars: &str,
        since_seq: Option<u64>,
        wait_ms: u64,
    ) -> Result<TerminalRead, MonitorError> {
        self.write_cancellable(
            id,
            owner,
            chars,
            since_seq,
            wait_ms,
            &CancellationToken::new(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn write_cancellable(
        &self,
        id: &str,
        owner: &TerminalOwner,
        chars: &str,
        since_seq: Option<u64>,
        wait_ms: u64,
        cancel: &CancellationToken,
    ) -> Result<TerminalRead, MonitorError> {
        call::check(cancel)?;
        validate_wait(wait_ms)?;
        if chars.len() > MAX_INPUT_BYTES {
            return Err(invalid(
                "terminal input exceeds 65536 bytes; split it into smaller writes",
            ));
        }
        let state = self.lookup(id, Some(owner))?;
        let _interaction = call::interaction(&state.interaction, cancel)?;
        // Validate the output cursor before delivering any input. A malformed
        // read request must never report failure after executing a command.
        state.validate_cursor(since_seq)?;
        if !chars.is_empty() {
            state.control_cancellable(Control::Write(chars.as_bytes().to_vec()), cancel)?;
        }
        state.read_locked_cancellable(since_seq, wait_ms, None, cancel)
    }

    pub fn resize(
        &self,
        id: &str,
        owner: &TerminalOwner,
        rows: u16,
        cols: u16,
    ) -> Result<TerminalRead, MonitorError> {
        self.resize_cancellable(id, owner, rows, cols, &CancellationToken::new())
    }

    pub(crate) fn resize_cancellable(
        &self,
        id: &str,
        owner: &TerminalOwner,
        rows: u16,
        cols: u16,
        cancel: &CancellationToken,
    ) -> Result<TerminalRead, MonitorError> {
        call::check(cancel)?;
        validate_size(rows, cols)?;
        let state = self.lookup(id, Some(owner))?;
        let _interaction = call::interaction(&state.interaction, cancel)?;
        state.control_cancellable(Control::Resize(rows, cols), cancel)?;
        state.read_locked_cancellable(None, 100, None, cancel)
    }

    pub fn signal(
        &self,
        id: &str,
        owner: &TerminalOwner,
        signal: ShellSignal,
    ) -> Result<TerminalRead, MonitorError> {
        self.signal_cancellable(id, owner, signal, &CancellationToken::new())
    }

    pub(crate) fn signal_cancellable(
        &self,
        id: &str,
        owner: &TerminalOwner,
        signal: ShellSignal,
        cancel: &CancellationToken,
    ) -> Result<TerminalRead, MonitorError> {
        call::check(cancel)?;
        let state = self.lookup(id, Some(owner))?;
        match signal {
            ShellSignal::Interrupt => {
                let since = state.summary().last_seq;
                state.control_cancellable(Control::Interrupt, cancel)?;
                // An out-of-band signal cannot wait behind a long output read.
                // Explicit cursors leave the automatic read cursor untouched.
                state.read_locked_cancellable(Some(since), 250, None, cancel)
            }
            ShellSignal::Terminate | ShellSignal::Kill => {
                state.stop(matches!(signal, ShellSignal::Kill))?;
                state.read(None, 0, None)
            }
        }
    }

    pub(crate) fn stop_unscoped(&self, id: &str) -> Result<ProcessSummary, MonitorError> {
        let state = self.lookup(id, None)?;
        state.stop(false)?;
        Ok(state.summary())
    }

    fn lookup(&self, id: &str, owner: Option<&TerminalOwner>) -> Result<Arc<State>, MonitorError> {
        lock(&self.sessions)
            .get(id)
            .filter(|s| owner.is_none_or(|owner| &s.owner == owner))
            .cloned()
            .ok_or_else(|| MonitorError::NotFound(id.to_owned()))
    }
}

impl Drop for TerminalRegistry {
    fn drop(&mut self) {
        self.shutdown();
    }
}

pub(crate) fn validate_size(rows: u16, cols: u16) -> Result<(), MonitorError> {
    if rows == 0
        || cols == 0
        || rows > 200
        || cols > 400
        || u32::from(rows) * u32::from(cols) > 40_000
    {
        return Err(invalid(
            "terminal dimensions must be 1–200 rows, 1–400 columns, and at most 40000 cells",
        ));
    }
    Ok(())
}
fn validate_wait(wait_ms: u64) -> Result<(), MonitorError> {
    if wait_ms > MAX_WAIT_MS {
        Err(invalid("terminal wait must be between 0 and 30000 ms"))
    } else {
        Ok(())
    }
}
fn invalid(message: impl Into<String>) -> MonitorError {
    MonitorError::Invalid(message.into())
}
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
