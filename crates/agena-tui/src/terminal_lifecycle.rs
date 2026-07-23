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
}

/// Process-wide mode ownership is shared with the process panic hook so a
/// lifecycle drop never pops the keyboard enhancement stack after emergency
/// restoration already did so.
pub static TERMINAL_MODES_ACTIVE: AtomicBool = AtomicBool::new(false);
pub static TERMINAL_KEYBOARD_STACK_ACTIVE: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuspendReason {
    ExternalEditor,
    ExternalPager,
    FileUpload,
    FileDownload,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct FakeDriver {
        calls: usize,
        fail_at: Vec<usize>,
        actions: Vec<&'static str>,
    }

    impl FakeDriver {
        fn perform(&mut self, action: &'static str) -> io::Result<()> {
            self.actions.push(action);
            let call = self.calls;
            self.calls += 1;
            if self.fail_at.contains(&call) {
                Err(io::Error::other(format!("injected failure at {action}")))
            } else {
                Ok(())
            }
        }
    }

    impl LifecycleDriver for FakeDriver {
        fn enable_raw(&mut self) -> io::Result<()> {
            self.perform("enable-raw")
        }

        fn disable_raw(&mut self) -> io::Result<()> {
            self.perform("disable-raw")
        }

        fn control(&mut self, control: Control) -> io::Result<()> {
            let action = match control {
                Control::EnterAlternateScreen => "enter-alternate",
                Control::LeaveAlternateScreen => "leave-alternate",
                Control::EnableBracketedPaste => "enable-paste",
                Control::DisableBracketedPaste => "disable-paste",
                Control::EnableFocusChange => "enable-focus",
                Control::DisableFocusChange => "disable-focus",
                Control::EnableMouseCapture => "enable-mouse",
                Control::DisableMouseCapture => "disable-mouse",
                Control::HideCursor => "hide-cursor",
                Control::ShowCursor => "show-cursor",
                Control::PushKeyboard(_) => "push-keyboard",
                Control::PopKeyboard => "pop-keyboard",
            };
            self.perform(action)
        }

        fn flush(&mut self) -> io::Result<()> {
            self.perform("flush")
        }
    }

    fn fully_enabled_capabilities() -> LifecycleCapabilities {
        LifecycleCapabilities {
            alternate_screen: true,
            bracketed_paste: true,
            focus_reporting: true,
            mouse_capture: true,
            keyboard_disambiguation: true,
            keyboard_alternate_keys: true,
            keyboard_event_types: false,
        }
    }

    #[test]
    fn keyboard_flags_never_include_release_events_by_default() {
        let flags = keyboard_enhancement_flags(&fully_enabled_capabilities());
        assert!(!flags.contains(KeyboardEnhancementFlags::REPORT_EVENT_TYPES));
    }

    #[cfg(unix)]
    #[test]
    fn mouse_capture_reports_dragging_without_hover_motion_floods() {
        use crossterm::Command;

        let mut enable = String::new();
        SetButtonMouseCapture::<true>
            .write_ansi(&mut enable)
            .expect("mouse command should format");
        assert_eq!(enable, "\x1b[?1006h\x1b[?1002h");

        let mut disable = String::new();
        SetButtonMouseCapture::<false>
            .write_ansi(&mut disable)
            .expect("mouse command should format");
        assert_eq!(disable, "\x1b[?1002l\x1b[?1006l");
    }

    #[test]
    fn runtime_input_modes_are_activated_only_after_startup_probes() {
        let capabilities = fully_enabled_capabilities();
        let mut lifecycle = TerminalLifecycle::default();
        let mut driver = FakeDriver::default();

        lifecycle
            .begin_startup_probe_with(&capabilities, &mut driver)
            .expect("probe phase");
        assert_eq!(lifecycle.phase, Phase::Probing);
        assert_eq!(driver.actions, ["enable-raw", "enter-alternate", "flush"]);
        assert!(!lifecycle.mouse_capture);

        lifecycle
            .activate_after_startup_probe_with(&capabilities, &mut driver)
            .expect("runtime phase");
        assert_eq!(lifecycle.phase, Phase::Active);
        assert_eq!(
            &driver.actions[3..],
            [
                "enable-raw",
                "enable-paste",
                "enable-focus",
                "enable-mouse",
                "hide-cursor",
                "push-keyboard",
                "flush",
            ]
        );
    }

    #[test]
    fn mouse_capture_can_be_toggled_without_repeating_terminal_commands() {
        let capabilities = fully_enabled_capabilities();
        let mut lifecycle = TerminalLifecycle::default();
        let mut driver = FakeDriver::default();
        lifecycle
            .enter_with(&capabilities, &mut driver)
            .expect("active terminal lifecycle");

        let startup_actions = driver.actions.len();
        lifecycle
            .set_mouse_capture_with(false, &capabilities, &mut driver)
            .expect("disable mouse capture");
        assert_eq!(
            &driver.actions[startup_actions..],
            ["disable-mouse", "flush"]
        );
        assert!(!lifecycle.mouse_capture);

        let disabled_actions = driver.actions.len();
        lifecycle
            .set_mouse_capture_with(false, &capabilities, &mut driver)
            .expect("keep mouse capture disabled");
        assert_eq!(driver.actions.len(), disabled_actions);

        lifecycle
            .set_mouse_capture_with(true, &capabilities, &mut driver)
            .expect("enable mouse capture");
        assert_eq!(
            &driver.actions[disabled_actions..],
            ["enable-mouse", "flush"]
        );
        assert!(lifecycle.mouse_capture);
    }

    #[test]
    fn disabled_mouse_capture_stays_disabled_after_resume() {
        let capabilities = fully_enabled_capabilities();
        let mut lifecycle = TerminalLifecycle::default();
        let mut driver = FakeDriver::default();
        lifecycle
            .enter_with(&capabilities, &mut driver)
            .expect("active terminal lifecycle");
        lifecycle
            .set_mouse_capture_with(false, &capabilities, &mut driver)
            .expect("disable mouse capture");

        lifecycle
            .leave_with(&mut driver)
            .expect("suspend terminal lifecycle");
        lifecycle.phase = Phase::Suspended;
        driver.actions.clear();

        lifecycle
            .enter_with(&capabilities, &mut driver)
            .expect("resume terminal lifecycle");
        assert!(!lifecycle.mouse_capture);
        assert!(!driver.actions.contains(&"enable-mouse"));
    }

    #[test]
    fn every_startup_failure_rolls_back_all_completed_terminal_modes() {
        let capabilities = fully_enabled_capabilities();
        // raw, alternate screen, probe flush, raw reassertion, then paste,
        // focus, mouse, cursor, keyboard, and the runtime-mode flush.
        for fail_at in 0..=9 {
            let mut lifecycle = TerminalLifecycle::default();
            let mut driver = FakeDriver {
                fail_at: vec![fail_at],
                ..FakeDriver::default()
            };
            assert!(lifecycle.enter_with(&capabilities, &mut driver).is_err());
            assert_eq!(lifecycle.phase, Phase::Detached);
            assert!(!lifecycle.raw);
            assert!(!lifecycle.alternate_screen);
            assert!(!lifecycle.bracketed_paste);
            assert!(!lifecycle.focus_reporting);
            assert!(!lifecycle.mouse_capture);
            assert!(!lifecycle.cursor_hidden);
            assert!(!lifecycle.keyboard_enhancement);
            if fail_at > 0 {
                assert!(driver.actions.contains(&"disable-raw"));
            }
        }
    }

    #[test]
    fn rollback_continues_after_a_cleanup_action_also_fails() {
        let capabilities = fully_enabled_capabilities();
        let mut lifecycle = TerminalLifecycle::default();
        let mut driver = FakeDriver {
            // Enabling paste fails after raw/alternate succeeded; leaving the
            // alternate screen then fails too. Raw mode and the final flush
            // must still be attempted.
            fail_at: vec![4, 6],
            ..FakeDriver::default()
        };

        let error = lifecycle
            .enter_with(&capabilities, &mut driver)
            .expect_err("startup and its rollback should fail");
        let error = format!("{error:#}");
        assert!(error.contains("injected failure at enable-paste"));
        assert!(
            error.contains("terminal startup rollback also failed"),
            "both failures should remain visible: {error}"
        );
        assert!(driver.actions.contains(&"disable-raw"));
        assert_eq!(driver.actions.last(), Some(&"flush"));
        assert_eq!(lifecycle.phase, Phase::Detached);
        assert!(!lifecycle.raw);
        assert!(!lifecycle.alternate_screen);
    }

    #[test]
    fn acknowledging_the_panic_hook_restore_clears_non_idempotent_state() {
        let mut lifecycle = TerminalLifecycle {
            phase: Phase::Active,
            raw: true,
            alternate_screen: true,
            bracketed_paste: true,
            focus_reporting: true,
            mouse_capture_requested: true,
            mouse_capture: true,
            cursor_hidden: true,
            keyboard_enhancement: true,
        };
        lifecycle.acknowledge_emergency_restore();
        assert_eq!(lifecycle.phase, Phase::Detached);
        assert!(!lifecycle.raw);
        assert!(!lifecycle.alternate_screen);
        assert!(!lifecycle.bracketed_paste);
        assert!(!lifecycle.focus_reporting);
        assert!(!lifecycle.mouse_capture);
        assert!(!lifecycle.cursor_hidden);
        assert!(!lifecycle.keyboard_enhancement);
    }
}
