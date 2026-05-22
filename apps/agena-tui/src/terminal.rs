use std::{
    io::{self, Stdout},
    panic,
    sync::Once,
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
use ratatui::{
    Terminal,
    backend::{Backend as RatatuiBackend, CrosstermBackend},
};

type AppTerminal = Terminal<CrosstermBackend<Stdout>>;

pub struct TerminalGuard {
    terminal: AppTerminal,
    restored: bool,
}

impl TerminalGuard {
    pub fn enter() -> Result<Self> {
        install_panic_hook();

        set_stdio_terminal()?;

        let stdout = io::stdout();
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;
        terminal
            .clear()
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;

        Ok(Self {
            terminal,
            restored: false,
        })
    }

    pub fn terminal_mut(&mut self) -> &mut AppTerminal {
        &mut self.terminal
    }

    pub fn restore(&mut self) -> Result<()> {
        if self.restored {
            return Ok(());
        }

        self.restored = true;
        self.terminal
            .flush()
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        restore_stdio_terminal()
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}

fn install_panic_hook() {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        let previous = panic::take_hook();
        panic::set_hook(Box::new(move |panic_info| {
            let _ = restore_stdio_terminal();
            previous(panic_info);
        }));
    });
}

pub fn suspend_stdio_terminal() -> Result<()> {
    restore_stdio_terminal()
}

pub fn resume_terminal<B: RatatuiBackend>(terminal: &mut Terminal<B>) -> Result<()> {
    set_stdio_terminal()?;
    terminal
        .clear()
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    Ok(())
}

fn set_stdio_terminal() -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    // Keep mouse capture off so dragging to select and copy behaves like a
    // normal terminal. Transcript scrolling remains keyboard-driven.
    execute!(
        stdout,
        EnterAlternateScreen,
        EnableBracketedPaste,
        EnableFocusChange,
        Hide
    )?;
    let _ = execute!(
        stdout,
        PushKeyboardEnhancementFlags(
            KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                | KeyboardEnhancementFlags::REPORT_EVENT_TYPES
                | KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS
        )
    );
    Ok(())
}

fn restore_stdio_terminal() -> Result<()> {
    disable_raw_mode()?;
    let mut stdout = io::stdout();
    let _ = execute!(stdout, PopKeyboardEnhancementFlags);
    execute!(
        stdout,
        Show,
        DisableFocusChange,
        DisableBracketedPaste,
        LeaveAlternateScreen
    )?;
    Ok(())
}
