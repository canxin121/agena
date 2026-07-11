use std::{
    env,
    io::{self, IsTerminal, Stdout, Write},
    panic,
    sync::Once,
};

use agena_tui_components::TerminalRgb;
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
    background: Option<TerminalRgb>,
    restored: bool,
}

impl TerminalGuard {
    pub fn enter() -> Result<Self> {
        install_panic_hook();

        set_stdio_terminal()?;
        let background = detect_terminal_background();

        let stdout = io::stdout();
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;
        terminal
            .clear()
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;

        Ok(Self {
            terminal,
            background,
            restored: false,
        })
    }

    pub fn terminal_mut(&mut self) -> &mut AppTerminal {
        &mut self.terminal
    }

    pub fn background(&self) -> Option<TerminalRgb> {
        self.background
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

fn detect_terminal_background() -> Option<TerminalRgb> {
    background_from_environment().or_else(query_terminal_background)
}

fn background_from_environment() -> Option<TerminalRgb> {
    if let Ok(value) = env::var("COLORFGBG")
        && let Some(color) = parse_colorfgbg(&value)
    {
        return Some(color);
    }

    for key in ["TERM_BACKGROUND", "VSCODE_THEME_KIND"] {
        let Ok(value) = env::var(key) else {
            continue;
        };
        match value.trim().to_ascii_lowercase().as_str() {
            "dark" | "highcontrastdark" => return Some(TerminalRgb::new(24, 24, 27)),
            "light" | "highcontrastlight" => return Some(TerminalRgb::new(250, 250, 250)),
            _ => {}
        }
    }
    None
}

fn parse_colorfgbg(value: &str) -> Option<TerminalRgb> {
    let index = value
        .split([';', ':'])
        .next_back()?
        .trim()
        .parse::<u8>()
        .ok()?;
    let (red, green, blue) = match index {
        0 => (0, 0, 0),
        1 => (170, 0, 0),
        2 => (0, 170, 0),
        3 => (170, 85, 0),
        4 => (0, 0, 170),
        5 => (170, 0, 170),
        6 => (0, 170, 170),
        7 => (170, 170, 170),
        8 => (85, 85, 85),
        9 => (255, 85, 85),
        10 => (85, 255, 85),
        11 => (255, 255, 85),
        12 => (85, 85, 255),
        13 => (255, 85, 255),
        14 => (85, 255, 255),
        15 => (255, 255, 255),
        16..=231 => {
            let offset = index - 16;
            let component = |value: u8| if value == 0 { 0 } else { 55 + value * 40 };
            (
                component(offset / 36),
                component((offset % 36) / 6),
                component(offset % 6),
            )
        }
        232..=255 => {
            let gray = 8 + (index - 232) * 10;
            (gray, gray, gray)
        }
    };
    Some(TerminalRgb::new(red, green, blue))
}

#[cfg(unix)]
fn query_terminal_background() -> Option<TerminalRgb> {
    use std::os::fd::AsRawFd;

    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return None;
    }

    let mut stdout = io::stdout();
    stdout.write_all(b"\x1b]11;?\x1b\\").ok()?;
    stdout.flush().ok()?;

    let fd = io::stdin().as_raw_fd();
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(120);
    let mut response = Vec::with_capacity(64);
    while std::time::Instant::now() < deadline && response.len() < 256 {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        let timeout = i32::try_from(remaining.as_millis().max(1)).unwrap_or(120);
        let mut pollfd = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: `pollfd` points to one initialized entry and remains valid
        // for the duration of this blocking call.
        if unsafe { libc::poll(&mut pollfd, 1, timeout) } <= 0 {
            break;
        }
        let mut chunk = [0_u8; 64];
        // SAFETY: `chunk` is writable for its full length and `fd` is the
        // process stdin descriptor, verified above to be a terminal.
        let count = unsafe { libc::read(fd, chunk.as_mut_ptr().cast(), chunk.len()) };
        if count <= 0 {
            break;
        }
        response.extend_from_slice(&chunk[..count as usize]);
        if response.contains(&0x07) || response.windows(2).any(|bytes| bytes == b"\x1b\\") {
            break;
        }
    }
    parse_osc11_response(&response)
}

#[cfg(not(unix))]
fn query_terminal_background() -> Option<TerminalRgb> {
    None
}

fn parse_osc11_response(response: &[u8]) -> Option<TerminalRgb> {
    let response = String::from_utf8_lossy(response);
    let payload = response.split("]11;").nth(1)?;
    let payload = payload
        .strip_prefix("rgb:")
        .or_else(|| payload.strip_prefix("rgba:"))?;
    let mut components = payload
        .split(['/', '\x07', '\x1b'])
        .take(3)
        .map(parse_osc_component);
    Some(TerminalRgb::new(
        components.next()??,
        components.next()??,
        components.next()??,
    ))
}

fn parse_osc_component(value: &str) -> Option<u8> {
    let value = value.trim();
    if value.is_empty() || value.len() > 4 {
        return None;
    }
    let raw = u32::from_str_radix(value, 16).ok()?;
    let maximum = (1_u32 << (value.len() * 4)) - 1;
    Some(((raw * 255 + maximum / 2) / maximum) as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_colorfgbg_ansi_and_extended_backgrounds() {
        assert_eq!(parse_colorfgbg("15;0"), Some(TerminalRgb::new(0, 0, 0)));
        assert_eq!(
            parse_colorfgbg("0;15"),
            Some(TerminalRgb::new(255, 255, 255))
        );
        assert_eq!(
            parse_colorfgbg("0;231"),
            Some(TerminalRgb::new(255, 255, 255))
        );
    }

    #[test]
    fn parses_common_osc11_response_widths_and_terminators() {
        assert_eq!(
            parse_osc11_response(b"\x1b]11;rgb:1e1e/2020/2424\x1b\\"),
            Some(TerminalRgb::new(30, 32, 36))
        );
        assert_eq!(
            parse_osc11_response(b"\x1b]11;rgb:f/a/0\x07"),
            Some(TerminalRgb::new(255, 170, 0))
        );
    }
}
