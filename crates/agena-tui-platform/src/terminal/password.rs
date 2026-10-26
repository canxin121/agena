use std::io::{self, Write};

use anyhow::{Context, Result, bail};
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    terminal::{disable_raw_mode, enable_raw_mode},
};

struct PasswordInputGuard;

impl Drop for PasswordInputGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = writeln!(io::stdout(), "\r");
    }
}

/// Read the shared server password before the TUI takes over the terminal.
/// Input is never echoed or persisted; all exits restore terminal input modes.
pub fn read_server_password() -> Result<String> {
    super::ensure_interactive_terminal_io()?;
    let _ownership = super::TerminalOwnershipGuard::acquire()?;
    enable_raw_mode().context("failed to hide server password input")?;
    let _input = PasswordInputGuard;
    print!("Server password: ");
    io::stdout().flush()?;

    let mut password = String::new();
    loop {
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind == KeyEventKind::Release {
            continue;
        }
        match key.code {
            KeyCode::Enter => {
                if password.trim().is_empty() {
                    bail!(
                        "server password must not be empty; set AGENA_SERVER_PASSWORD or enter a password"
                    );
                }
                return Ok(password);
            }
            KeyCode::Esc => bail!("server password entry cancelled"),
            KeyCode::Char('c' | 'd') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                bail!("server password entry cancelled");
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => password.clear(),
            KeyCode::Backspace => {
                password.pop();
            }
            KeyCode::Char(ch)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                    && !ch.is_control() =>
            {
                // Match the server's bound before submitting an oversized secret.
                if password.len() + ch.len_utf8() > 4096 {
                    bail!("server password is too long");
                }
                password.push(ch);
            }
            _ => {}
        }
    }
}
