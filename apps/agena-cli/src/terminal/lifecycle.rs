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

impl TerminalLifecycle {
    pub(super) fn enter(&mut self, capabilities: &TerminalCapabilities) -> Result<()> {
        if self.phase == Phase::Active {
            return Ok(());
        }

        if let Err(error) = self.enter_inner(capabilities) {
            let _ = self.leave();
            return Err(error);
        }
        self.phase = Phase::Active;
        Ok(())
    }

    fn enter_inner(&mut self, capabilities: &TerminalCapabilities) -> Result<()> {
        enable_raw_mode()?;
        self.raw = true;

        let mut stdout = io::stdout();
        if capabilities.alternate_screen.is_supported() {
            execute!(stdout, EnterAlternateScreen)?;
            self.alternate_screen = true;
        }
        if capabilities.bracketed_paste.is_supported() {
            execute!(stdout, EnableBracketedPaste)?;
            self.bracketed_paste = true;
        }
        if capabilities.focus_reporting.is_supported() {
            execute!(stdout, EnableFocusChange)?;
            self.focus_reporting = true;
        }
        execute!(stdout, Hide)?;
        self.cursor_hidden = true;

        let flags = keyboard_enhancement_flags(capabilities);
        if !flags.is_empty() && execute!(stdout, PushKeyboardEnhancementFlags(flags)).is_ok() {
            self.keyboard_enhancement = true;
        }
        Ok(())
    }

    pub(super) fn suspend(&mut self, _reason: SuspendReason) -> Result<()> {
        if self.phase != Phase::Active {
            return Ok(());
        }
        self.leave()?;
        self.phase = Phase::Suspended;
        Ok(())
    }

    pub(super) fn resume(&mut self, capabilities: &TerminalCapabilities) -> Result<()> {
        self.enter(capabilities)
    }

    pub(super) fn shutdown(&mut self) -> Result<()> {
        let result = self.leave();
        self.phase = Phase::Detached;
        result
    }

    fn leave(&mut self) -> Result<()> {
        let mut first_error: Option<anyhow::Error> = None;
        let mut record = |result: std::io::Result<()>| {
            if let Err(error) = result
                && first_error.is_none()
            {
                first_error = Some(error.into());
            }
        };
        let mut stdout = io::stdout();

        if self.keyboard_enhancement {
            record(execute!(stdout, PopKeyboardEnhancementFlags));
            self.keyboard_enhancement = false;
        }
        let _ = std::io::Write::flush(&mut stdout);
        if self.cursor_hidden {
            record(execute!(stdout, Show));
            self.cursor_hidden = false;
        }
        if self.focus_reporting {
            record(execute!(stdout, DisableFocusChange));
            self.focus_reporting = false;
        }
        if self.bracketed_paste {
            record(execute!(stdout, DisableBracketedPaste));
            self.bracketed_paste = false;
        }
        if self.alternate_screen {
            record(execute!(stdout, LeaveAlternateScreen));
            self.alternate_screen = false;
        }
        if self.raw {
            record(disable_raw_mode());
            self.raw = false;
        }

        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

fn keyboard_enhancement_flags(capabilities: &TerminalCapabilities) -> KeyboardEnhancementFlags {
    let mut flags = KeyboardEnhancementFlags::empty();
    if capabilities.keyboard_disambiguation.is_supported() {
        flags |= KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES;
    }
    if capabilities.keyboard_alternate_keys.is_supported() {
        flags |= KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS;
    }
    if capabilities.keyboard_event_types.is_supported() {
        flags |= KeyboardEnhancementFlags::REPORT_EVENT_TYPES;
    }
    flags
}

pub(super) fn emergency_restore() -> Result<()> {
    let mut stdout = io::stdout();
    let _ = execute!(stdout, PopKeyboardEnhancementFlags);
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
}
