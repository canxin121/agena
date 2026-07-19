use std::{
    io,
    io::IsTerminal,
    panic,
    sync::{
        Once,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use agena::config::TuiGraphicsModeConfig;
use agena_tui_components::TerminalRgb;
use anyhow::{Context, Result, bail};
use crossterm::event::{
    self, BackgroundColorQuery, Event, TerminalResponse, TerminalResponseCapture,
};
use ratatui::{Frame, Terminal, backend::CrosstermBackend};

use crate::math_render::MathGraphicsConfig;

mod broker;
mod capabilities;
mod graphics;
mod identity;
mod input;
mod lifecycle;
mod overrides;
mod profiles;
mod protocol;
mod transport;
mod version;

pub use capabilities::{
    CapabilityEvidence, CapabilityPath, ProviderReadiness, TerminalColorDetection,
    TerminalColorSource, TerminalContext,
};
pub use identity::TerminalFamily;
pub use lifecycle::SuspendReason;

type AppTerminal = Terminal<CrosstermBackend<io::Stdout>>;
const PROTOCOL_SETTLE_TIMEOUT: Duration = Duration::from_secs(1);

#[derive(Debug)]
struct ProtocolTransaction {
    _capture: TerminalResponseCapture,
    state: ProtocolTransactionState,
}

#[derive(Debug)]
enum ProtocolTransactionState {
    /// Distinct CPR barrier sent after every synchronous startup probe. Its
    /// arrival proves that earlier query replies can no longer be in flight.
    StartupBarrier,
    /// A live background query followed by DSR. The candidate is committed
    /// only when the ordered DSR barrier arrives.
    ColorRefresh {
        query: BackgroundColorQuery,
        source: TerminalColorSource,
        candidate: Option<TerminalRgb>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProtocolProgress {
    Pending,
    Complete(Option<TerminalColorDetection>),
}

impl ProtocolTransactionState {
    fn observe(&mut self, response: TerminalResponse) -> ProtocolProgress {
        match self {
            Self::StartupBarrier => {
                if matches!(response, TerminalResponse::CursorPosition { .. }) {
                    ProtocolProgress::Complete(None)
                } else {
                    ProtocolProgress::Pending
                }
            }
            Self::ColorRefresh {
                query,
                source,
                candidate,
            } => match response {
                TerminalResponse::BackgroundColor {
                    query: response_query,
                    red,
                    green,
                    blue,
                } if response_query == *query => {
                    *candidate = Some(TerminalRgb::new(red, green, blue));
                    ProtocolProgress::Pending
                }
                // A success or failure DSR is still the ordered completion
                // marker. Commit a color only when one was received before it.
                TerminalResponse::DeviceStatus(_) => {
                    ProtocolProgress::Complete(candidate.map(|background| TerminalColorDetection {
                        background: Some(background),
                        source: *source,
                    }))
                }
                _ => ProtocolProgress::Pending,
            },
        }
    }
}

/// Owns the terminal for the entire TUI lifetime.
///
/// No application code may create a second terminal event reader, mutate tty
/// modes, or write terminal control protocols directly. Startup probes finish
/// before `input_reader` is created; a typed barrier then drains any delayed
/// replies. Live queries and keyboard input share this one event reader.
pub struct TerminalRuntime {
    terminal: AppTerminal,
    input_reader: input::TerminalInput,
    input: input::InputNormalizer,
    broker: broker::TerminalProtocolBroker,
    lifecycle: lifecycle::TerminalLifecycle,
    context: TerminalContext,
    background: Option<TerminalRgb>,
    math_graphics: MathGraphicsConfig,
    color_refresh_through_tmux: bool,
    protocol_transaction: Option<ProtocolTransaction>,
    generation: u64,
    restored: bool,
    ownership: TerminalOwnershipGuard,
}

impl TerminalRuntime {
    pub fn enter(graphics_mode: TuiGraphicsModeConfig) -> Result<Self> {
        ensure_interactive_terminal_io()?;
        let mut context = TerminalContext::detect();
        ensure_supported_terminal(&context)?;
        // Provider and multiplexer helpers are read-only, bounded subprocess
        // probes. Finish them before entering raw/alternate-screen mode so a
        // slow local helper never leaves the user staring at a blank screen.
        let graphics_policy = graphics::GraphicsTransportPolicy::detect(&context, graphics_mode);
        install_panic_hook();

        // Keep this guard local until construction succeeds. Every `?` below
        // drops it, clearing the process-wide mode flags after the lifecycle
        // has rolled back any terminal state it already activated.
        let ownership = TerminalOwnershipGuard::acquire()?;
        let mut lifecycle = lifecycle::TerminalLifecycle::default();
        // Startup probes need raw input and the alternate screen, but runtime
        // protocols are intentionally not enabled yet. The final mouse,
        // focus, paste, and keyboard modes are emitted only after every
        // response-bearing probe has finished.
        lifecycle.begin_startup_probe(&context.capabilities)?;
        TERMINAL_MODES_ACTIVE.store(true, Ordering::Release);
        TERMINAL_KEYBOARD_STACK_ACTIVE.store(false, Ordering::Release);

        // Color detection is independent from graphics support. Query it even
        // in Unicode graphics mode, before runtime input starts, and use the
        // environment only when both bounded terminal attempts fail.
        let queried_color =
            protocol::query_terminal_background(&context, graphics_policy.through_tmux);
        let color_query_succeeded = queried_color.is_some();
        let color = queried_color.unwrap_or_else(protocol::detect_terminal_background);
        context.record_color_detection(color);
        let background = color.background;

        // ratatui-image owns the separate startup graphics capability query.
        // It finishes before input starts; the typed startup barrier below
        // accounts for any reply delayed past its bounded liveness deadline.
        let math_graphics = MathGraphicsConfig::query(
            background,
            graphics_policy.probe_native,
            graphics_policy.through_tmux,
            graphics_policy.protocol_hint,
        );
        if !math_graphics.is_native() {
            let reason = if graphics_policy.probe_native {
                "endpoint negotiation did not select a native image protocol"
            } else {
                graphics_policy.reason
            };
            context.record_runtime_diagnostic("terminal.graphics.unicode-fallback", reason);
            tracing::warn!(reason, "using Unicode graphics fallback");
        }
        if math_graphics.is_native() {
            context.capabilities.inline_images = capabilities::CapabilityEvidence::supported(
                capabilities::CapabilitySource::TerminalQuery,
            );
        }
        if color_query_succeeded {
            context.capabilities.default_color_query = capabilities::CapabilityEvidence::supported(
                capabilities::CapabilitySource::TerminalQuery,
            );
        }

        lifecycle.activate_after_startup_probe(&context.capabilities)?;
        TERMINAL_KEYBOARD_STACK_ACTIVE
            .store(lifecycle.keyboard_enhancement_active(), Ordering::Release);

        let backend = CrosstermBackend::new(io::stdout());
        let mut terminal = Terminal::new(backend)?;
        terminal
            .clear()
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        let input_reader =
            input::TerminalInput::new().context("failed to register terminal input readiness")?;
        let input = input::InputNormalizer::default();
        let mut broker = broker::TerminalProtocolBroker;

        // Synchronous compatibility probes above have bounded liveness
        // deadlines, but a deadline is not a protocol boundary: the terminal
        // may emit a reply later. Begin byte-level response capture and append
        // one distinct CPR barrier after every startup query. Until its typed
        // response arrives, no response-shaped bytes can enter keyboard input.
        let protocol_transaction = if cfg!(unix) {
            let capture = TerminalResponseCapture::begin()
                .context("failed to begin the startup terminal-response transaction")?;
            let startup_barrier = protocol::startup_barrier(graphics_policy.through_tmux);
            broker
                .write_frame(terminal.backend_mut(), &startup_barrier)
                .context("failed to write the startup terminal-response barrier")?;
            Some(ProtocolTransaction {
                _capture: capture,
                state: ProtocolTransactionState::StartupBarrier,
            })
        } else {
            // Crossterm receives native console records rather than a VT byte
            // stream on Windows, so the Unix OSC/key ambiguity does not exist.
            None
        };

        Ok(Self {
            terminal,
            input_reader,
            input,
            broker,
            lifecycle,
            context,
            background,
            math_graphics,
            color_refresh_through_tmux: graphics_policy.through_tmux,
            protocol_transaction,
            generation: 1,
            restored: false,
            ownership,
        })
    }

    pub fn background(&self) -> Option<TerminalRgb> {
        self.background
    }

    pub fn context(&self) -> &TerminalContext {
        &self.context
    }

    pub(crate) fn math_graphics(&self) -> MathGraphicsConfig {
        self.math_graphics.clone()
    }

    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }

    /// Refresh a startup-verified color query after the terminal regains
    /// focus. Unsupported queries are never retried here, and the last
    /// known-good appearance remains active until a complete response arrives.
    pub(crate) fn refresh_color_on_focus(&mut self) {
        self.request_terminal_color_refresh();
    }

    fn request_terminal_color_refresh(&mut self) {
        if cfg!(not(unix)) {
            return;
        }
        if !self.context.color.source.supports_live_refresh() {
            return;
        }
        // Terminal protocols do not carry transaction IDs. Never overlap a
        // query with the startup drain or another color query; completion is
        // established by the barrier, not by elapsed time.
        if self.protocol_transaction.is_some() {
            return;
        }
        let Some((query, transaction)) = protocol::color_refresh_transaction(
            self.context.color.source,
            self.color_refresh_through_tmux,
        ) else {
            return;
        };
        let capture = match TerminalResponseCapture::begin() {
            Ok(capture) => capture,
            Err(error) => {
                tracing::warn!(%error, "failed to begin terminal background refresh");
                return;
            }
        };
        if let Err(error) = self
            .broker
            .write_transaction(self.terminal.backend_mut(), &transaction.frames())
        {
            tracing::warn!(%error, "failed to write terminal background refresh");
            return;
        }
        self.protocol_transaction = Some(ProtocolTransaction {
            _capture: capture,
            state: ProtocolTransactionState::ColorRefresh {
                query,
                source: self.context.color.source,
                candidate: None,
            },
        });
    }

    fn apply_terminal_color(&mut self, detection: TerminalColorDetection) {
        if detection == self.context.color {
            return;
        }

        self.context.record_color_detection(detection);
        self.background = detection.background;
        if let Some(background) = detection.background {
            self.math_graphics.apply_terminal_appearance(background);
        }
        self.generation = self.generation.saturating_add(1);
        if let Err(error) = self.terminal.clear() {
            tracing::warn!(
                error = %error,
                "failed to clear stale terminal graphics after an appearance change"
            );
        }
        tracing::debug!(
            background = ?detection.background,
            source = ?detection.source,
            generation = self.context.color_generation,
            "terminal appearance changed"
        );
    }

    /// Consume a typed terminal response before the input normalizer sees it.
    /// The response capture guard is released only by the transaction's exact
    /// completion marker; there is no grace interval and no payload matching
    /// after bytes have already become key events.
    fn handle_terminal_response(&mut self, response: TerminalResponse) {
        let progress = match self.protocol_transaction.as_mut() {
            Some(transaction) => transaction.state.observe(response),
            None => {
                tracing::trace!(?response, "discarded unsolicited terminal response");
                ProtocolProgress::Pending
            }
        };

        if let ProtocolProgress::Complete(detection) = progress {
            // Dropping the transaction releases Crossterm's byte-level capture
            // only after the ordered protocol barrier has been observed.
            self.protocol_transaction.take();
            if let Some(detection) = detection {
                self.apply_terminal_color(detection);
            }
        }
    }

    /// Serialize an application protocol command with frame output. Callers
    /// supply a complete OSC/CSI/DCS frame; partial protocol writes are never
    /// exposed outside the runtime.
    pub fn write_protocol(&mut self, frame: &[u8]) -> Result<()> {
        self.broker.write_frame(self.terminal.backend_mut(), frame)
    }

    pub fn draw(&mut self, render: impl FnOnce(&mut Frame<'_>)) -> Result<()> {
        self.terminal
            .draw(render)
            .map(|_| ())
            .map_err(|error| anyhow::anyhow!(error.to_string()))
    }

    pub async fn next_event(&mut self) -> Option<Result<Event, std::io::Error>> {
        loop {
            if let Some(event) = self.input.pop_ready() {
                return Some(Ok(event));
            }

            let next = if let Some(deadline) = self.input.deadline() {
                tokio::select! {
                    event = self.input_reader.next() => Some(event),
                    () = tokio::time::sleep_until(deadline.into()) => {
                        self.input.flush_timed_out();
                        continue;
                    }
                }
            } else {
                Some(self.input_reader.next().await)
            };
            match next {
                Some(Ok(Event::TerminalResponse(response))) => {
                    self.handle_terminal_response(response);
                }
                Some(Ok(event)) => self.input.accept(event),
                Some(Err(error)) => return Some(Err(error)),
                None => {
                    self.input.flush_all();
                    return self.input.pop_ready().map(Ok);
                }
            }
        }
    }

    pub fn set_text_input_active(&mut self, active: bool) {
        self.input.set_text_input_active(active);
    }

    /// Enable application mouse handling only for surfaces that implement it.
    /// When disabled, the terminal receives mouse gestures normally, allowing
    /// native text selection and scrollback behavior.
    pub fn set_mouse_capture_active(&mut self, active: bool) -> Result<()> {
        self.lifecycle
            .set_mouse_capture(active, &self.context.capabilities)
    }

    /// Run a blocking terminal-integrated operation while the TUI is fully
    /// suspended. The TUI is resumed even if the operation panics; the panic is
    /// then resumed after tty ownership has been recovered.
    pub fn with_suspended<T>(
        &mut self,
        reason: SuspendReason,
        operation: impl FnOnce() -> T,
    ) -> Result<T> {
        // Never hand stdin to a child while a terminal reply can still arrive.
        // Failure to reach the barrier aborts the suspension; it never resumes
        // input under a different owner after an arbitrary timeout.
        self.settle_protocol_transaction(PROTOCOL_SETTLE_TIMEOUT)
            .context("terminal response transaction did not settle before suspension")?;
        self.terminal
            .flush()
            .map_err(|error| anyhow::anyhow!(error.to_string()))
            .context("failed to flush the TUI before suspending the terminal")?;

        // No background task owns stdin. Drain only events already reported as
        // ready so they remain TUI input instead of leaking into the child.
        let preserved_input = self.preserve_pending_input()?;
        let suspend_result = self.lifecycle.suspend(reason);
        TERMINAL_MODES_ACTIVE.store(false, Ordering::Release);
        TERMINAL_KEYBOARD_STACK_ACTIVE.store(false, Ordering::Release);
        if let Err(error) = suspend_result {
            let suspend_error = error.context(format!(
                "failed to completely suspend the terminal for {reason:?}"
            ));
            return match self.resume_after_suspension(reason, preserved_input) {
                Ok(()) => Err(suspend_error),
                Err(recovery_error) => Err(suspend_error.context(format!(
                    "terminal recovery after the failed suspension also failed: {recovery_error:#}"
                ))),
            };
        }

        let result = panic::catch_unwind(panic::AssertUnwindSafe(operation));
        let recovery_result = self.resume_after_suspension(reason, preserved_input);

        match result {
            Ok(value) => {
                recovery_result?;
                Ok(value)
            }
            Err(payload) => {
                if let Err(error) = recovery_result {
                    tracing::error!(
                        error = %error,
                        "terminal recovery also failed while propagating a suspended-operation panic"
                    );
                }
                panic::resume_unwind(payload)
            }
        }
    }

    fn resume_after_suspension(
        &mut self,
        reason: SuspendReason,
        preserved_input: std::collections::VecDeque<Event>,
    ) -> Result<()> {
        self.lifecycle
            .resume(&self.context.capabilities)
            .with_context(|| format!("failed to resume the terminal after {reason:?}"))?;
        TERMINAL_MODES_ACTIVE.store(true, Ordering::Release);
        TERMINAL_KEYBOARD_STACK_ACTIVE.store(
            self.lifecycle.keyboard_enhancement_active(),
            Ordering::Release,
        );
        self.generation = self.generation.saturating_add(1);
        self.input.reset();
        self.input.restore_ready(preserved_input);
        // Editors and transfer helpers may stay open while the OS or terminal
        // switches appearance. Refresh before the first resumed frame.
        self.request_terminal_color_refresh();
        self.terminal
            .clear()
            .map_err(|error| anyhow::anyhow!(error.to_string()))
            .with_context(|| format!("failed to clear the terminal after {reason:?}"))?;
        Ok(())
    }

    fn preserve_pending_input(&mut self) -> Result<std::collections::VecDeque<Event>> {
        const MAX_PENDING_EVENTS: usize = 4_096;

        self.input.flush_all();
        for _ in 0..MAX_PENDING_EVENTS {
            if !event::poll(Duration::ZERO)
                .context("failed to inspect pending terminal input before suspension")?
            {
                self.input.flush_all();
                return Ok(self.input.take_ready());
            }
            match event::read()
                .context("failed to preserve pending terminal input before suspension")?
            {
                Event::TerminalResponse(response) => self.handle_terminal_response(response),
                event => self.input.accept(event),
            }
        }
        self.input.flush_all();
        bail!(
            "terminal input remained continuously ready while suspending; external program was not started"
        )
    }

    pub fn restore(&mut self) -> Result<()> {
        if self.restored {
            return Ok(());
        }
        let protocol_result = if TERMINAL_MODES_ACTIVE.load(Ordering::Acquire) {
            self.settle_protocol_transaction(PROTOCOL_SETTLE_TIMEOUT)
        } else {
            Ok(())
        };
        let _ = self.terminal.flush();
        let lifecycle_result = if TERMINAL_MODES_ACTIVE.load(Ordering::Acquire) {
            self.lifecycle.shutdown()
        } else {
            // The panic hook already performed the visible emergency restore,
            // so acknowledge it without popping the keyboard stack twice.
            self.lifecycle.acknowledge_emergency_restore();
            Ok(())
        };
        if lifecycle_result.is_ok() {
            self.restored = true;
            TERMINAL_MODES_ACTIVE.store(false, Ordering::Release);
            TERMINAL_KEYBOARD_STACK_ACTIVE.store(false, Ordering::Release);
            self.ownership.release();
        }
        match (protocol_result, lifecycle_result) {
            (Err(protocol_error), Err(lifecycle_error)) => Err(protocol_error.context(format!(
                "terminal lifecycle restoration also failed: {lifecycle_error:#}"
            ))),
            (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
            (Ok(()), Ok(())) => Ok(()),
        }
    }

    fn settle_protocol_transaction(&mut self, timeout: Duration) -> Result<()> {
        let deadline = Instant::now() + timeout;
        while self.protocol_transaction.is_some() {
            let now = Instant::now();
            if now >= deadline
                || !event::poll(deadline.saturating_duration_since(now))
                    .context("failed to wait for the terminal protocol barrier")?
            {
                bail!("terminal did not acknowledge the protocol barrier within {timeout:?}");
            }
            match event::read().context("failed to read the terminal protocol barrier")? {
                Event::TerminalResponse(response) => self.handle_terminal_response(response),
                event => self.input.accept(event),
            }
        }
        Ok(())
    }
}

fn ensure_interactive_terminal_io() -> Result<()> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        bail!(
            "the Agena TUI requires both stdin and stdout to be attached to the same interactive terminal; use the non-interactive CLI or run Agena from a terminal"
        );
    }
    if !stdin_and_stdout_share_terminal()? {
        bail!(
            "the Agena TUI requires stdin and stdout to refer to the same terminal device; split terminal redirection is unsupported"
        );
    }
    Ok(())
}

fn ensure_supported_terminal(context: &TerminalContext) -> Result<()> {
    if context.identity.family == TerminalFamily::Dumb {
        bail!("the Agena TUI cannot run with TERM=dumb; select a VT-compatible terminal type");
    }
    Ok(())
}

#[cfg(unix)]
fn stdin_and_stdout_share_terminal() -> Result<bool> {
    use std::mem::MaybeUninit;
    use std::os::fd::AsRawFd;

    fn stat(fd: i32) -> Result<libc::stat> {
        let mut stat = MaybeUninit::<libc::stat>::uninit();
        // SAFETY: `fstat` initializes the complete `libc::stat` structure on
        // success, and the borrowed standard descriptor remains valid here.
        if unsafe { libc::fstat(fd, stat.as_mut_ptr()) } != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        // SAFETY: successful `fstat` initialized the value above.
        Ok(unsafe { stat.assume_init() })
    }

    let stdin = stat(io::stdin().as_raw_fd())?;
    let stdout = stat(io::stdout().as_raw_fd())?;
    Ok(stdin.st_rdev == stdout.st_rdev && stdin.st_ino == stdout.st_ino)
}

#[cfg(not(unix))]
fn stdin_and_stdout_share_terminal() -> Result<bool> {
    // A Windows process can be attached to only one console at a time. The
    // `IsTerminal` checks above therefore establish the same invariant.
    Ok(true)
}

impl Drop for TerminalRuntime {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}

fn install_panic_hook() {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        let previous = panic::take_hook();
        panic::set_hook(Box::new(move |panic_info| {
            if TERMINAL_MODES_ACTIVE.swap(false, Ordering::AcqRel) {
                let pop_keyboard = TERMINAL_KEYBOARD_STACK_ACTIVE.swap(false, Ordering::AcqRel);
                let _ = lifecycle::emergency_restore(pop_keyboard);
            }
            previous(panic_info);
        }));
    });
}

static TERMINAL_OWNED: AtomicBool = AtomicBool::new(false);
static TERMINAL_MODES_ACTIVE: AtomicBool = AtomicBool::new(false);
static TERMINAL_KEYBOARD_STACK_ACTIVE: AtomicBool = AtomicBool::new(false);

struct TerminalOwnershipGuard {
    active: bool,
}

impl TerminalOwnershipGuard {
    fn acquire() -> Result<Self> {
        TERMINAL_OWNED
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| anyhow::anyhow!("a TerminalRuntime already owns this process terminal"))?;
        Ok(Self { active: true })
    }

    fn release(&mut self) {
        if self.active {
            TERMINAL_MODES_ACTIVE.store(false, Ordering::Release);
            TERMINAL_KEYBOARD_STACK_ACTIVE.store(false, Ordering::Release);
            TERMINAL_OWNED.store(false, Ordering::Release);
            self.active = false;
        }
    }
}

impl Drop for TerminalOwnershipGuard {
    fn drop(&mut self) {
        self.release();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    #[test]
    fn process_terminal_has_exactly_one_runtime_owner() {
        static TEST_LOCK: Mutex<()> = Mutex::new(());
        let _lock = TEST_LOCK.lock().expect("terminal ownership test lock");
        TERMINAL_OWNED.store(false, Ordering::Release);
        TERMINAL_MODES_ACTIVE.store(false, Ordering::Release);
        TERMINAL_KEYBOARD_STACK_ACTIVE.store(false, Ordering::Release);
        let first = TerminalOwnershipGuard::acquire().expect("first owner");
        assert!(TerminalOwnershipGuard::acquire().is_err());
        TERMINAL_MODES_ACTIVE.store(true, Ordering::Release);
        TERMINAL_KEYBOARD_STACK_ACTIVE.store(true, Ordering::Release);
        drop(first);
        assert!(!TERMINAL_MODES_ACTIVE.load(Ordering::Acquire));
        assert!(!TERMINAL_KEYBOARD_STACK_ACTIVE.load(Ordering::Acquire));
        assert!(TerminalOwnershipGuard::acquire().is_ok());
    }

    #[test]
    fn startup_transaction_completes_only_at_its_distinct_cpr_barrier() {
        let mut transaction = ProtocolTransactionState::StartupBarrier;
        assert_eq!(
            transaction.observe(TerminalResponse::DeviceStatus(0)),
            ProtocolProgress::Pending
        );
        assert_eq!(
            transaction.observe(TerminalResponse::BackgroundColor {
                query: BackgroundColorQuery::Iterm2Osc4,
                red: 250,
                green: 250,
                blue: 250,
            }),
            ProtocolProgress::Pending
        );
        assert_eq!(
            transaction.observe(TerminalResponse::CursorPosition { column: 0, row: 0 }),
            ProtocolProgress::Complete(None)
        );
    }

    #[test]
    fn color_transaction_correlates_selector_and_commits_at_dsr_barrier() {
        let mut transaction = ProtocolTransactionState::ColorRefresh {
            query: BackgroundColorQuery::Iterm2Osc4,
            source: TerminalColorSource::Iterm2Osc4,
            candidate: None,
        };
        assert_eq!(
            transaction.observe(TerminalResponse::BackgroundColor {
                query: BackgroundColorQuery::Osc11,
                red: 1,
                green: 2,
                blue: 3,
            }),
            ProtocolProgress::Pending
        );
        assert_eq!(
            transaction.observe(TerminalResponse::BackgroundColor {
                query: BackgroundColorQuery::Iterm2Osc4,
                red: 250,
                green: 240,
                blue: 230,
            }),
            ProtocolProgress::Pending
        );
        assert_eq!(
            transaction.observe(TerminalResponse::DeviceStatus(3)),
            ProtocolProgress::Complete(Some(TerminalColorDetection {
                background: Some(TerminalRgb::new(250, 240, 230)),
                source: TerminalColorSource::Iterm2Osc4,
            }))
        );
    }

    #[test]
    fn color_transaction_without_a_matching_reply_preserves_current_color() {
        let mut transaction = ProtocolTransactionState::ColorRefresh {
            query: BackgroundColorQuery::Osc11,
            source: TerminalColorSource::Osc11,
            candidate: None,
        };
        assert_eq!(
            transaction.observe(TerminalResponse::DeviceStatus(0)),
            ProtocolProgress::Complete(None)
        );
    }
}
