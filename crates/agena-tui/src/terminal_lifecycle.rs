//! Terminal lifecycle management (init/teardown).

use std::{
    io,
    sync::atomic::{AtomicBool, Ordering},
};

use anyhow::Result;
use crossterm::{
    cursor::{Hide, Show},
    event::{
        DisableBracketedPaste, DisableFocusChange, EnableBracketedPaste, EnableFocusChange,
        KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};

/// The terminal-mode decisions consumed by the lifecycle state machine.
///
/// Capability detection remains separate; this is deliberately a compact
/// operational projection so the lifecycle does not depend on application
/// diagnostics or backend-facing terminal context.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LifecycleCapabilities {
    pub alternate_screen: bool,
    pub bracketed_paste: bool,
    pub focus_reporting: bool,
    pub mouse_capture: bool,
    pub keyboard_disambiguation: bool,
    pub keyboard_alternate_keys: bool,
    pub keyboard_event_types: bool,
    /// Kitty protocol flag 4: report every key through CSI u. iTerm2 needs it
    /// to treat Option as Alt (Option+Backspace word deletion on macOS).
    pub keyboard_report_all_keys: bool,
}

/// Process-wide mode ownership is shared with the process panic hook so a
/// lifecycle drop never pops the keyboard enhancement stack after emergency
/// restoration already did so.
pub static TERMINAL_MODES_ACTIVE: AtomicBool = AtomicBool::new(false);
pub static TERMINAL_KEYBOARD_STACK_ACTIVE: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Reason the TUI was suspended.
pub enum SuspendReason {
    ExternalEditor,
    ExternalPager,
    FileUpload,
    ClipboardRead,
    OpenPath,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Phase {
    #[default]
    Detached,
    Probing,
    Active,
    Suspended,
}

#[derive(Debug)]
/// Lifecycle state of the terminal.
pub struct TerminalLifecycle {
    phase: Phase,
    raw: bool,
    alternate_screen: bool,
    bracketed_paste: bool,
    focus_reporting: bool,
    mouse_capture_requested: bool,
    mouse_capture: bool,
    cursor_hidden: bool,
    keyboard_enhancement: bool,
}

impl Default for TerminalLifecycle {
    fn default() -> Self {
        Self {
            phase: Phase::Detached,
            raw: false,
            alternate_screen: false,
            bracketed_paste: false,
            focus_reporting: false,
            // The TUI starts on the main chat route. The application narrows
            // this request as soon as it displays another surface.
            mouse_capture_requested: true,
            mouse_capture: false,
            cursor_hidden: false,
            keyboard_enhancement: false,
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum Control {
    EnterAlternateScreen,
    LeaveAlternateScreen,
    EnableBracketedPaste,
    DisableBracketedPaste,
    EnableFocusChange,
    DisableFocusChange,
    EnableMouseCapture,
    DisableMouseCapture,
    HideCursor,
    ShowCursor,
    PushKeyboard(KeyboardEnhancementFlags),
    PopKeyboard,
}

trait LifecycleDriver {
    fn enable_raw(&mut self) -> io::Result<()>;
    fn disable_raw(&mut self) -> io::Result<()>;
    fn control(&mut self, control: Control) -> io::Result<()>;
    fn flush(&mut self) -> io::Result<()>;
}

struct SystemLifecycleDriver {
    stdout: io::Stdout,
}

impl SystemLifecycleDriver {
    fn new() -> Self {
        Self {
            stdout: io::stdout(),
        }
    }
}

impl LifecycleDriver for SystemLifecycleDriver {
    fn enable_raw(&mut self) -> io::Result<()> {
        enable_raw_mode()
    }

    fn disable_raw(&mut self) -> io::Result<()> {
        disable_raw_mode()
    }

    fn control(&mut self, control: Control) -> io::Result<()> {
        match control {
            Control::EnterAlternateScreen => execute!(self.stdout, EnterAlternateScreen),
            Control::LeaveAlternateScreen => execute!(self.stdout, LeaveAlternateScreen),
            Control::EnableBracketedPaste => execute!(self.stdout, EnableBracketedPaste),
            Control::DisableBracketedPaste => execute!(self.stdout, DisableBracketedPaste),
            Control::EnableFocusChange => execute!(self.stdout, EnableFocusChange),
            Control::DisableFocusChange => execute!(self.stdout, DisableFocusChange),
            Control::EnableMouseCapture => enable_mouse_capture(&mut self.stdout),
            Control::DisableMouseCapture => disable_mouse_capture(&mut self.stdout),
            Control::HideCursor => execute!(self.stdout, Hide),
            Control::ShowCursor => execute!(self.stdout, Show),
            Control::PushKeyboard(flags) => {
                execute!(self.stdout, PushKeyboardEnhancementFlags(flags))
            }
            Control::PopKeyboard => execute!(self.stdout, PopKeyboardEnhancementFlags),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        std::io::Write::flush(&mut self.stdout)
    }
}

impl TerminalLifecycle {
    pub const fn keyboard_enhancement_active(&self) -> bool {
        self.keyboard_enhancement
    }

    pub fn acknowledge_emergency_restore(&mut self) {
        self.raw = false;
        self.alternate_screen = false;
        self.bracketed_paste = false;
        self.focus_reporting = false;
        self.mouse_capture = false;
        self.cursor_hidden = false;
        self.keyboard_enhancement = false;
        self.phase = Phase::Detached;
    }

    pub fn set_mouse_capture(
        &mut self,
        enabled: bool,
        capabilities: &LifecycleCapabilities,
    ) -> Result<()> {
        self.set_mouse_capture_with(enabled, capabilities, &mut SystemLifecycleDriver::new())
    }

    fn set_mouse_capture_with(
        &mut self,
        enabled: bool,
        capabilities: &LifecycleCapabilities,
        driver: &mut impl LifecycleDriver,
    ) -> Result<()> {
        self.mouse_capture_requested = enabled;
        if self.phase != Phase::Active {
            return Ok(());
        }

        let enabled = enabled && capabilities.mouse_capture;
        if enabled == self.mouse_capture {
            return Ok(());
        }

        driver.control(if enabled {
            Control::EnableMouseCapture
        } else {
            Control::DisableMouseCapture
        })?;
        self.mouse_capture = enabled;
        driver.flush()?;
        Ok(())
    }

    pub fn enter(&mut self, capabilities: &LifecycleCapabilities) -> Result<()> {
        self.enter_with(capabilities, &mut SystemLifecycleDriver::new())
    }

    /// Enter only the tty state required by synchronous startup probes.
    /// Runtime input protocols are deliberately activated after every probe,
    /// so no probe can overwrite or consume the application's final modes.
    pub fn begin_startup_probe(&mut self, capabilities: &LifecycleCapabilities) -> Result<()> {
        self.begin_startup_probe_with(capabilities, &mut SystemLifecycleDriver::new())
    }

    pub fn activate_after_startup_probe(
        &mut self,
        capabilities: &LifecycleCapabilities,
    ) -> Result<()> {
        self.activate_after_startup_probe_with(capabilities, &mut SystemLifecycleDriver::new())
    }

    fn enter_with(
        &mut self,
        capabilities: &LifecycleCapabilities,
        driver: &mut impl LifecycleDriver,
    ) -> Result<()> {
        if self.phase == Phase::Active {
            return Ok(());
        }

        self.begin_startup_probe_with(capabilities, driver)?;
        self.activate_after_startup_probe_with(capabilities, driver)
    }

    fn begin_startup_probe_with(
        &mut self,
        capabilities: &LifecycleCapabilities,
        driver: &mut impl LifecycleDriver,
    ) -> Result<()> {
        if matches!(self.phase, Phase::Probing | Phase::Active) {
            return Ok(());
        }

        if let Err(error) = self.begin_startup_probe_inner(capabilities, driver) {
            let cleanup = self.leave_with(driver);
            self.phase = Phase::Detached;
            return match cleanup {
                Ok(()) => Err(error),
                Err(cleanup_error) => Err(error.context(format!(
                    "terminal startup rollback also failed: {cleanup_error:#}"
                ))),
            };
        }
        self.phase = Phase::Probing;
        Ok(())
    }

    fn begin_startup_probe_inner(
        &mut self,
        capabilities: &LifecycleCapabilities,
        driver: &mut impl LifecycleDriver,
    ) -> Result<()> {
        driver.enable_raw()?;
        self.raw = true;

        if capabilities.alternate_screen {
            driver.control(Control::EnterAlternateScreen)?;
            self.alternate_screen = true;
        }
        driver.flush()?;
        Ok(())
    }

    fn activate_after_startup_probe_with(
        &mut self,
        capabilities: &LifecycleCapabilities,
        driver: &mut impl LifecycleDriver,
    ) -> Result<()> {
        if self.phase == Phase::Active {
            return Ok(());
        }
        if self.phase != Phase::Probing {
            anyhow::bail!("terminal runtime modes require an active startup-probe phase");
        }

        if let Err(error) = self.activate_after_startup_probe_inner(capabilities, driver) {
            let cleanup = self.leave_with(driver);
            self.phase = Phase::Detached;
            return match cleanup {
                Ok(()) => Err(error),
                Err(cleanup_error) => Err(error.context(format!(
                    "terminal startup rollback also failed: {cleanup_error:#}"
                ))),
            };
        }
        self.phase = Phase::Active;
        Ok(())
    }

    fn activate_after_startup_probe_inner(
        &mut self,
        capabilities: &LifecycleCapabilities,
        driver: &mut impl LifecycleDriver,
    ) -> Result<()> {
        // Reassert raw mode at the transaction boundary. This is idempotent
        // for a well-behaved probe and repairs the tty if a future dependency
        // accidentally restores cooked mode during capability negotiation.
        driver.enable_raw()?;
        self.raw = true;

        if capabilities.bracketed_paste {
            driver.control(Control::EnableBracketedPaste)?;
            self.bracketed_paste = true;
        }
        if capabilities.focus_reporting {
            driver.control(Control::EnableFocusChange)?;
            self.focus_reporting = true;
        }
        if self.mouse_capture_requested && capabilities.mouse_capture {
            driver.control(Control::EnableMouseCapture)?;
            self.mouse_capture = true;
        }
        driver.control(Control::HideCursor)?;
        self.cursor_hidden = true;

        let flags = keyboard_enhancement_flags(capabilities);
        if !flags.is_empty() {
            driver.control(Control::PushKeyboard(flags))?;
            self.keyboard_enhancement = true;
        }
        driver.flush()?;
        Ok(())
    }

    pub fn suspend(&mut self, _reason: SuspendReason) -> Result<()> {
        if self.phase != Phase::Active {
            return Ok(());
        }
        let result = self.leave();
        self.phase = Phase::Suspended;
        result
    }

    pub fn resume(&mut self, capabilities: &LifecycleCapabilities) -> Result<()> {
        self.enter(capabilities)
    }

    pub fn shutdown(&mut self) -> Result<()> {
        let result = self.leave();
        if result.is_ok() {
            self.phase = Phase::Detached;
        }
        result
    }

    fn leave(&mut self) -> Result<()> {
        self.leave_with(&mut SystemLifecycleDriver::new())
    }

    fn leave_with(&mut self, driver: &mut impl LifecycleDriver) -> Result<()> {
        let mut first_error: Option<anyhow::Error> = None;
        let mut record = |result: std::io::Result<()>| {
            if let Err(error) = result
                && first_error.is_none()
            {
                first_error = Some(error.into());
            }
        };
        if self.keyboard_enhancement {
            record(driver.control(Control::PopKeyboard));
            self.keyboard_enhancement = false;
        }
        record(driver.flush());
        if self.cursor_hidden {
            record(driver.control(Control::ShowCursor));
            self.cursor_hidden = false;
        }
        if self.mouse_capture {
            record(driver.control(Control::DisableMouseCapture));
            self.mouse_capture = false;
        }
        if self.focus_reporting {
            record(driver.control(Control::DisableFocusChange));
            self.focus_reporting = false;
        }
        if self.bracketed_paste {
            record(driver.control(Control::DisableBracketedPaste));
            self.bracketed_paste = false;
        }
        if self.alternate_screen {
            record(driver.control(Control::LeaveAlternateScreen));
            self.alternate_screen = false;
        }
        if self.raw {
            record(driver.disable_raw());
            self.raw = false;
        }
        record(driver.flush());

        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

impl Drop for TerminalLifecycle {
    fn drop(&mut self) {
        if self.phase != Phase::Detached
            || self.raw
            || self.alternate_screen
            || self.bracketed_paste
            || self.focus_reporting
            || self.mouse_capture
            || self.cursor_hidden
            || self.keyboard_enhancement
        {
            if !TERMINAL_MODES_ACTIVE.load(Ordering::Acquire) {
                // The process panic hook has already restored visible state.
                // Do not pop the non-idempotent keyboard stack a second time.
                self.acknowledge_emergency_restore();
                return;
            }
            // This is the last line of defence for partially constructed
            // runtimes and failed shutdowns. All emitted disable operations
            // are idempotent except the keyboard stack pop, which `leave`
            // tracks separately and clears after its first attempt.
            let _ = self.leave();
            let _ = emergency_restore_without_keyboard_pop();
        }
    }
}

fn keyboard_enhancement_flags(capabilities: &LifecycleCapabilities) -> KeyboardEnhancementFlags {
    let mut flags = KeyboardEnhancementFlags::empty();
    if capabilities.keyboard_disambiguation {
        flags |= KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES;
    }
    if capabilities.keyboard_alternate_keys {
        flags |= KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS;
    }
    if capabilities.keyboard_event_types {
        flags |= KeyboardEnhancementFlags::REPORT_EVENT_TYPES;
    }
    if capabilities.keyboard_report_all_keys {
        flags |= KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES;
    }
    flags
}

pub fn emergency_restore(pop_keyboard: bool) -> Result<()> {
    if pop_keyboard {
        let mut stdout = io::stdout();
        let _ = execute!(stdout, PopKeyboardEnhancementFlags);
    }
    emergency_restore_without_keyboard_pop()
}

fn emergency_restore_without_keyboard_pop() -> Result<()> {
    let mut stdout = io::stdout();
    let _ = execute!(stdout, Show);
    let _ = disable_mouse_capture(&mut stdout);
    let _ = execute!(
        stdout,
        DisableFocusChange,
        DisableBracketedPaste,
        LeaveAlternateScreen
    );
    disable_raw_mode()?;
    Ok(())
}

#[cfg(unix)]
#[derive(Debug, Clone, Copy)]
struct SetButtonMouseCapture<const ENABLED: bool>;

#[cfg(unix)]
impl<const ENABLED: bool> crossterm::Command for SetButtonMouseCapture<ENABLED> {
    fn write_ansi(&self, formatter: &mut impl std::fmt::Write) -> std::fmt::Result {
        if ENABLED {
            // iTerm2 treats 1000, 1002, and 1003 as mutually exclusive and
            // specifies that the last DECSET wins. Select exactly button-event
            // tracking, with SGR coordinates enabled first as required by its
            // protocol notes. Wheel, click, release, and drag are reported;
            // passive hover movement is not.
            formatter.write_str("\x1b[?1006h\x1b[?1002h")
        } else {
            // Reset exactly the modes Agena owns, in protocol-before-encoding
            // order. Resetting unrelated tracking modes can disable another
            // owner's active mode on terminals where they are exclusive.
            formatter.write_str("\x1b[?1002l\x1b[?1006l")
        }
    }
}

#[cfg(unix)]
fn enable_mouse_capture(stdout: &mut io::Stdout) -> io::Result<()> {
    execute!(stdout, SetButtonMouseCapture::<true>)
}

#[cfg(unix)]
fn disable_mouse_capture(stdout: &mut io::Stdout) -> io::Result<()> {
    execute!(stdout, SetButtonMouseCapture::<false>)
}

#[cfg(not(unix))]
fn enable_mouse_capture(stdout: &mut io::Stdout) -> io::Result<()> {
    execute!(stdout, crossterm::event::EnableMouseCapture)
}

#[cfg(not(unix))]
fn disable_mouse_capture(stdout: &mut io::Stdout) -> io::Result<()> {
    execute!(stdout, crossterm::event::DisableMouseCapture)
}
