#[cfg(not(target_os = "android"))]
use std::io::Write;

#[cfg(not(target_os = "android"))]
use base64::{Engine as _, engine::general_purpose::STANDARD};

#[cfg(not(target_os = "android"))]
use crate::terminal::TerminalContext;

#[derive(Debug, Clone)]
pub struct ClipboardTextError(pub String);

impl std::fmt::Display for ClipboardTextError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0.as_str())
    }
}

impl std::error::Error for ClipboardTextError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardCopyMethod {
    Native,
    Osc52,
    Tmux,
}

impl ClipboardCopyMethod {
    pub const fn is_unconfirmed_terminal_request(self) -> bool {
        matches!(self, Self::Osc52 | Self::Tmux)
    }
}

#[cfg(not(target_os = "android"))]
pub fn set_clipboard_text(
    text: &str,
    context: &TerminalContext,
    mut write_terminal: impl FnMut(&[u8]) -> Result<(), ClipboardTextError>,
) -> Result<ClipboardCopyMethod, ClipboardTextError> {
    ClipboardService::new(context).copy_text(text, &mut write_terminal)
}

#[cfg(not(target_os = "android"))]
struct ClipboardService<'a> {
    context: &'a TerminalContext,
}

#[cfg(not(target_os = "android"))]
impl<'a> ClipboardService<'a> {
    fn new(context: &'a TerminalContext) -> Self {
        Self { context }
    }

    fn copy_text(
        &self,
        text: &str,
        write_terminal: &mut impl FnMut(&[u8]) -> Result<(), ClipboardTextError>,
    ) -> Result<ClipboardCopyMethod, ClipboardTextError> {
        const MAX_OSC52_TEXT_BYTES: usize = 100_000;

        let mut failures = Vec::new();
        if self
            .context
            .capabilities
            .clipboard_write_native
            .is_supported()
        {
            match set_clipboard_text_native(text) {
                Ok(()) => return Ok(ClipboardCopyMethod::Native),
                Err(error) => failures.push(format!("native clipboard failed: {error}")),
            }
        }

        if self.context.in_tmux() {
            match set_clipboard_text_via_tmux(text) {
                Ok(()) => return Ok(ClipboardCopyMethod::Tmux),
                Err(error) => failures.push(format!("tmux clipboard failed: {error}")),
            }
        }

        if self
            .context
            .capabilities
            .clipboard_write_osc52
            .is_supported_or_unknown()
        {
            if text.len() > MAX_OSC52_TEXT_BYTES {
                failures.push(format!(
                    "terminal clipboard payload exceeds the {MAX_OSC52_TEXT_BYTES}-byte safety limit"
                ));
            } else {
                let sequence = osc52_copy_sequence(text);
                write_terminal(sequence.as_slice())?;
                return Ok(ClipboardCopyMethod::Osc52);
            }
        }

        if failures.is_empty() {
            failures.push("no compatible clipboard provider is available".to_string());
        }
        Err(ClipboardTextError(failures.join("; ")))
    }
}

#[cfg(not(target_os = "android"))]
fn set_clipboard_text_native(text: &str) -> Result<(), ClipboardTextError> {
    let mut clipboard =
        arboard::Clipboard::new().map_err(|error| ClipboardTextError(error.to_string()))?;
    clipboard
        .set_text(text.to_string())
        .map_err(|error| ClipboardTextError(error.to_string()))
}

#[cfg(not(target_os = "android"))]
fn set_clipboard_text_via_tmux(text: &str) -> Result<(), ClipboardTextError> {
    use std::process::{Command, Stdio};

    let mut child = Command::new("tmux")
        .args(["load-buffer", "-w", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| ClipboardTextError(error.to_string()))?;
    child
        .stdin
        .take()
        .ok_or_else(|| ClipboardTextError("tmux clipboard stdin is unavailable".to_string()))?
        .write_all(text.as_bytes())
        .map_err(|error| ClipboardTextError(error.to_string()))?;
    let output = child
        .wait_with_output()
        .map_err(|error| ClipboardTextError(error.to_string()))?;
    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(ClipboardTextError(if stderr.is_empty() {
            "tmux rejected the clipboard request".to_string()
        } else {
            stderr
        }))
    }
}

#[cfg(not(target_os = "android"))]
fn osc52_copy_sequence(text: &str) -> Vec<u8> {
    format!("\x1b]52;c;{}\x07", STANDARD.encode(text.as_bytes())).into_bytes()
}

#[cfg(target_os = "android")]
pub fn set_clipboard_text(
    _: &str,
    _: &crate::terminal::TerminalContext,
    _: impl FnMut(&[u8]) -> Result<(), ClipboardTextError>,
) -> Result<ClipboardCopyMethod, ClipboardTextError> {
    Err(ClipboardTextError(
        "clipboard text copy is unsupported on Android".to_string(),
    ))
}

#[cfg(all(test, not(target_os = "android")))]
mod tests {
    use super::osc52_copy_sequence;

    #[test]
    fn osc52_copy_sequence_targets_the_local_terminal_clipboard() {
        assert_eq!(
            osc52_copy_sequence("copied from Agena"),
            b"\x1b]52;c;Y29waWVkIGZyb20gQWdlbmE=\x07"
        );
    }
}
