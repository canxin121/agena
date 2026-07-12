use std::{io, panic, sync::Once};

use agena_tui_components::TerminalRgb;
use anyhow::Result;
use crossterm::event::{Event, EventStream};
use futures_util::StreamExt;
use ratatui::{Frame, Terminal, backend::CrosstermBackend};

use crate::math_render::{MathGraphicsConfig, foreground_for_background};

mod broker;
mod capabilities;
mod identity;
mod input;
mod lifecycle;
mod overrides;
mod profiles;
mod protocol;
mod transport;
mod version;

pub use capabilities::{CapabilityEvidence, TerminalContext};
pub use identity::TerminalFamily;
pub use lifecycle::SuspendReason;

type AppTerminal = Terminal<CrosstermBackend<io::Stdout>>;

/// Owns the terminal for the entire TUI lifetime.
///
/// No application code may create a second terminal event stream, mutate tty
/// modes, or write terminal control protocols directly. Protocol negotiation
/// happens before `events` is created, and suspended operations temporarily
/// revoke the event stream before returning the tty to an external program.
pub struct TerminalRuntime {
    terminal: AppTerminal,
    events: Option<EventStream>,
    input: input::InputNormalizer,
    broker: broker::TerminalProtocolBroker,
    lifecycle: lifecycle::TerminalLifecycle,
    context: TerminalContext,
    background: Option<TerminalRgb>,
    math_graphics: MathGraphicsConfig,
    generation: u64,
    restored: bool,
}

impl TerminalRuntime {
    pub fn enter() -> Result<Self> {
        install_panic_hook();

        let mut context = TerminalContext::detect();
        let mut lifecycle = lifecycle::TerminalLifecycle::default();
        lifecycle.enter(&context.capabilities)?;

        // Background hints are environment-only. Agena does not read terminal
        // query responses outside the application event stream.
        let background = protocol::detect_terminal_background(&context);
        // ratatui-image owns the one synchronous capability query. It must run
        // after alternate-screen entry and before EventStream starts, otherwise
        // graphics replies can be mistaken for keyboard input.
        let math_graphics = MathGraphicsConfig::query(foreground_for_background(background));
        if math_graphics.is_native() {
            context.capabilities.inline_images = capabilities::CapabilityEvidence::supported(
                capabilities::CapabilitySource::TerminalQuery,
            );
        }

        let backend = CrosstermBackend::new(io::stdout());
        let mut terminal = Terminal::new(backend)?;
        terminal
            .clear()
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;

        Ok(Self {
            terminal,
            events: Some(EventStream::new()),
            input: input::InputNormalizer::default(),
            broker: broker::TerminalProtocolBroker::default(),
            lifecycle,
            context,
            background,
            math_graphics,
            generation: 1,
            restored: false,
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

            let events = match self.events.as_mut() {
                Some(events) => events,
                None => return std::future::pending().await,
            };
            let next = if let Some(deadline) = self.input.deadline() {
                tokio::select! {
                    event = events.next() => event,
                    () = tokio::time::sleep_until(deadline.into()) => {
                        self.input.flush_timed_out();
                        continue;
                    }
                }
            } else {
                events.next().await
            };
            match next {
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

    /// Run a blocking terminal-integrated operation while the TUI is fully
    /// suspended. The TUI is resumed even if the operation panics; the panic is
    /// then resumed after tty ownership has been recovered.
    pub fn with_suspended<T>(
        &mut self,
        reason: SuspendReason,
        operation: impl FnOnce() -> T,
    ) -> Result<T> {
        self.terminal
            .flush()
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;

        self.events.take();
        self.input.flush_all();
        let preserved_input = self.input.take_ready();
        if let Err(error) = self.lifecycle.suspend(reason) {
            self.input.restore_ready(preserved_input);
            self.events = Some(EventStream::new());
            return Err(error);
        }

        let result = panic::catch_unwind(panic::AssertUnwindSafe(operation));
        let resume_result = self.lifecycle.resume(&self.context.capabilities);
        if resume_result.is_ok() {
            self.generation = self.generation.saturating_add(1);
            self.broker.next_generation();
            self.events = Some(EventStream::new());
            self.input.reset();
            self.input.restore_ready(preserved_input);
            self.terminal
                .clear()
                .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        }

        match result {
            Ok(value) => {
                resume_result?;
                Ok(value)
            }
            Err(payload) => {
                let _ = resume_result;
                panic::resume_unwind(payload)
            }
        }
    }

    pub fn restore(&mut self) -> Result<()> {
        if self.restored {
            return Ok(());
        }
        self.restored = true;
        self.events.take();
        let _ = self.terminal.flush();
        self.lifecycle.shutdown()
    }
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
            let _ = lifecycle::emergency_restore();
            previous(panic_info);
        }));
    });
}
