#[cfg(not(any(
    target_os = "android",
    target_os = "ios",
    target_os = "tvos",
    target_os = "watchos",
    target_os = "visionos"
)))]
use base64::{Engine as _, engine::general_purpose::STANDARD};

#[cfg(not(any(
    target_os = "android",
    target_os = "ios",
    target_os = "tvos",
    target_os = "watchos",
    target_os = "visionos"
)))]
use crate::terminal::TerminalContext;

#[derive(Debug, Clone)]
/// Error reading or writing text through the clipboard.
pub struct ClipboardTextError(pub String);

impl std::fmt::Display for ClipboardTextError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0.as_str())
    }
}

impl std::error::Error for ClipboardTextError {}

impl ClipboardTextError {
    pub fn from_error(error: &(dyn std::error::Error + 'static)) -> Self {
        Self(agena_failure::diagnostic::format_error_chain(error))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Method used to copy text to the clipboard.
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

#[cfg(not(any(
    target_os = "android",
    target_os = "ios",
    target_os = "tvos",
    target_os = "watchos",
    target_os = "visionos"
)))]
pub fn set_clipboard_text(
    text: &str,
    context: &TerminalContext,
    mut write_terminal: impl FnMut(&[u8]) -> Result<(), ClipboardTextError>,
) -> Result<ClipboardCopyMethod, ClipboardTextError> {
    ClipboardService::new(context).copy_text(text, &mut write_terminal)
}

#[cfg(not(any(
    target_os = "android",
    target_os = "ios",
    target_os = "tvos",
    target_os = "watchos",
    target_os = "visionos"
)))]
pub fn get_clipboard_text(context: &TerminalContext) -> Result<String, ClipboardTextError> {
    if !context.capabilities.clipboard_read_native.is_operational() {
        return Err(ClipboardTextError(
            "native clipboard read is unavailable in the current terminal".to_owned(),
        ));
    }
    let mut clipboard =
        arboard::Clipboard::new().map_err(|error| ClipboardTextError::from_error(&error))?;
    clipboard
        .get_text()
        .map_err(|error| ClipboardTextError::from_error(&error))
}

#[cfg(not(any(
    target_os = "android",
    target_os = "ios",
    target_os = "tvos",
    target_os = "watchos",
    target_os = "visionos"
)))]
struct ClipboardService<'a> {
    context: &'a TerminalContext,
}

#[cfg(not(any(
    target_os = "android",
    target_os = "ios",
    target_os = "tvos",
    target_os = "watchos",
    target_os = "visionos"
)))]
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
            .is_operational()
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
            .is_operational()
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

#[cfg(not(any(
    target_os = "android",
    target_os = "ios",
    target_os = "tvos",
    target_os = "watchos",
    target_os = "visionos"
)))]
fn set_clipboard_text_native(text: &str) -> Result<(), ClipboardTextError> {
    let mut clipboard =
        arboard::Clipboard::new().map_err(|error| ClipboardTextError::from_error(&error))?;
    clipboard
        .set_text(text.to_string())
        .map_err(|error| ClipboardTextError::from_error(&error))
}

#[cfg(not(any(
    target_os = "android",
    target_os = "ios",
    target_os = "tvos",
    target_os = "watchos",
    target_os = "visionos"
)))]
fn set_clipboard_text_via_tmux(text: &str) -> Result<(), ClipboardTextError> {
    use std::{process::Command, time::Duration};

    let mut command = Command::new("tmux");
    command.args(["load-buffer", "-w", "-"]);
    let status = crate::helper_runner::run_with_input(
        &mut command,
        text.as_bytes().to_vec(),
        "tmux clipboard copy",
        Duration::from_secs(10),
    )
    .map_err(|error| ClipboardTextError::from_error(&error))?;
    if status.success() {
        Ok(())
    } else {
        Err(ClipboardTextError(
            "tmux rejected the clipboard request".to_owned(),
        ))
    }
}

#[cfg(not(any(
    target_os = "android",
    target_os = "ios",
    target_os = "tvos",
    target_os = "watchos",
    target_os = "visionos"
)))]
fn osc52_copy_sequence(text: &str) -> Vec<u8> {
    format!("\x1b]52;c;{}\x07", STANDARD.encode(text.as_bytes())).into_bytes()
}

#[cfg(any(
    target_os = "android",
    target_os = "ios",
    target_os = "tvos",
    target_os = "watchos",
    target_os = "visionos"
))]
pub fn set_clipboard_text(
    _: &str,
    _: &crate::terminal::TerminalContext,
    _: impl FnMut(&[u8]) -> Result<(), ClipboardTextError>,
) -> Result<ClipboardCopyMethod, ClipboardTextError> {
    Err(ClipboardTextError(
        "clipboard text copy is unsupported on this platform".to_string(),
    ))
}

#[cfg(any(
    target_os = "android",
    target_os = "ios",
    target_os = "tvos",
    target_os = "watchos",
    target_os = "visionos"
))]
pub fn get_clipboard_text(
    _: &crate::terminal::TerminalContext,
) -> Result<String, ClipboardTextError> {
    Err(ClipboardTextError(
        "clipboard text read is unsupported on this platform".to_string(),
    ))
}

#[cfg(all(
    test,
    not(any(
        target_os = "android",
        target_os = "ios",
        target_os = "tvos",
        target_os = "watchos",
        target_os = "visionos"
    ))
))]
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
