use std::io;

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

use super::capabilities::TerminalCapabilities;

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
    Active,
    Suspended,
}

#[derive(Debug, Default)]
pub(super) struct TerminalLifecycle {
    phase: Phase,
    raw: bool,
    alternate_screen: bool,
    bracketed_paste: bool,
    focus_reporting: bool,
    cursor_hidden: bool,
    keyboard_enhancement: bool,
}

#[derive(Debug, Clone, Copy)]
enum Control {
    EnterAlternateScreen,
    LeaveAlternateScreen,
    EnableBracketedPaste,
    DisableBracketedPaste,
    EnableFocusChange,
    DisableFocusChange,
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
    pub(super) const fn keyboard_enhancement_active(&self) -> bool {
        self.keyboard_enhancement
    }

    pub(super) fn acknowledge_emergency_restore(&mut self) {
        self.raw = false;
        self.alternate_screen = false;
        self.bracketed_paste = false;
        self.focus_reporting = false;
        self.cursor_hidden = false;
        self.keyboard_enhancement = false;
        self.phase = Phase::Detached;
    }

    pub(super) fn enter(&mut self, capabilities: &TerminalCapabilities) -> Result<()> {
        self.enter_with(capabilities, &mut SystemLifecycleDriver::new())
    }

    fn enter_with(
        &mut self,
        capabilities: &TerminalCapabilities,
        driver: &mut impl LifecycleDriver,
    ) -> Result<()> {
        if self.phase == Phase::Active {
            return Ok(());
        }

        if let Err(error) = self.enter_inner(capabilities, driver) {
            return match self.leave_with(driver) {
                Ok(()) => Err(error),
                Err(cleanup_error) => Err(error.context(format!(
                    "terminal startup rollback also failed: {cleanup_error:#}"
                ))),
            };
        }
        self.phase = Phase::Active;
        Ok(())
    }

    fn enter_inner(
        &mut self,
        capabilities: &TerminalCapabilities,
        driver: &mut impl LifecycleDriver,
    ) -> Result<()> {
        driver.enable_raw()?;
        self.raw = true;

        if capabilities.alternate_screen.is_operational() {
            driver.control(Control::EnterAlternateScreen)?;
            self.alternate_screen = true;
        }
        if capabilities.bracketed_paste.is_operational() {
            driver.control(Control::EnableBracketedPaste)?;
            self.bracketed_paste = true;
        }
        if capabilities.focus_reporting.is_operational() {
            driver.control(Control::EnableFocusChange)?;
            self.focus_reporting = true;
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

    pub(super) fn suspend(&mut self, _reason: SuspendReason) -> Result<()> {
        if self.phase != Phase::Active {
            return Ok(());
        }
        let result = self.leave();
        self.phase = Phase::Suspended;
        result
    }

    pub(super) fn resume(&mut self, capabilities: &TerminalCapabilities) -> Result<()> {
        self.enter(capabilities)
    }

    pub(super) fn shutdown(&mut self) -> Result<()> {
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
            || self.cursor_hidden
            || self.keyboard_enhancement
        {
            if !super::TERMINAL_MODES_ACTIVE.load(std::sync::atomic::Ordering::Acquire) {
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

fn keyboard_enhancement_flags(capabilities: &TerminalCapabilities) -> KeyboardEnhancementFlags {
    let mut flags = KeyboardEnhancementFlags::empty();
    if capabilities.keyboard_disambiguation.is_operational() {
        flags |= KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES;
    }
    if capabilities.keyboard_alternate_keys.is_operational() {
        flags |= KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS;
    }
    if capabilities.keyboard_event_types.is_operational() {
        flags |= KeyboardEnhancementFlags::REPORT_EVENT_TYPES;
    }
    flags
}

pub(super) fn emergency_restore(pop_keyboard: bool) -> Result<()> {
    if pop_keyboard {
        let mut stdout = io::stdout();
        let _ = execute!(stdout, PopKeyboardEnhancementFlags);
    }
    emergency_restore_without_keyboard_pop()
}

fn emergency_restore_without_keyboard_pop() -> Result<()> {
    let mut stdout = io::stdout();
    let _ = execute!(
        stdout,
        Show,
        DisableFocusChange,
        DisableBracketedPaste,
        LeaveAlternateScreen
    );
    disable_raw_mode()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::capabilities::{CapabilitySource, Support, TerminalContext};
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

    fn fully_enabled_capabilities() -> TerminalCapabilities {
        let mut capabilities = TerminalContext::detect().capabilities;
        let enabled =
            super::super::capabilities::CapabilityEvidence::forced(CapabilitySource::UserOverride);
        capabilities.alternate_screen = enabled;
        capabilities.bracketed_paste = enabled;
        capabilities.focus_reporting = enabled;
        capabilities.keyboard_disambiguation = enabled;
        capabilities.keyboard_alternate_keys = enabled;
        capabilities
    }

    #[test]
    fn keyboard_flags_never_include_release_events_by_default() {
        let context = TerminalContext::detect();
        let flags = keyboard_enhancement_flags(&context.capabilities);
        assert!(!flags.contains(KeyboardEnhancementFlags::REPORT_EVENT_TYPES));
        assert_eq!(
            context.capabilities.keyboard_event_types.support,
            Support::Unsupported
        );
        assert_eq!(
            context.capabilities.keyboard_event_types.source,
            CapabilitySource::ConservativeDefault
        );
    }

    #[test]
    fn every_startup_failure_rolls_back_all_completed_terminal_modes() {
        let capabilities = fully_enabled_capabilities();
        // raw, alternate screen, paste, focus, cursor, keyboard, final flush
        for fail_at in 0..=6 {
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
            fail_at: vec![2, 4],
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
            cursor_hidden: true,
            keyboard_enhancement: true,
        };
        lifecycle.acknowledge_emergency_restore();
        assert_eq!(lifecycle.phase, Phase::Detached);
        assert!(!lifecycle.raw);
        assert!(!lifecycle.alternate_screen);
        assert!(!lifecycle.bracketed_paste);
        assert!(!lifecycle.focus_reporting);
        assert!(!lifecycle.cursor_hidden);
        assert!(!lifecycle.keyboard_enhancement);
    }
}
