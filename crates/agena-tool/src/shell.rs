//! Shell command execution contracts (`ShellRequest`, `ShellOutput`).

use std::cmp::min;

/// Default timeout for a shell-tool process invocation.
pub const DEFAULT_SHELL_TIMEOUT_MS: u64 = 120_000;

const MAX_OUTPUT_BYTES: usize = 50 * 1024;
const MAX_OUTPUT_LINES: usize = 2_000;

/// Limit a shell result for model and presentation consumption.
pub fn truncate_shell_output(output: &str) -> (String, bool) {
    let mut lines = output.lines().collect::<Vec<_>>();
    let line_truncated = lines.len() > MAX_OUTPUT_LINES;
    if line_truncated {
        lines.truncate(MAX_OUTPUT_LINES);
    }

    let joined = lines.join("\n");
    let byte_truncated = joined.len() > MAX_OUTPUT_BYTES;
    let clipped = if byte_truncated {
        let bytes = joined.as_bytes();
        String::from_utf8_lossy(&bytes[..min(bytes.len(), MAX_OUTPUT_BYTES)]).to_string()
    } else {
        joined
    };

    let truncated = line_truncated || byte_truncated;
    if truncated {
        (
            format!(
                "{}\n\n[output truncated: max {} lines / {} bytes]",
                clipped, MAX_OUTPUT_LINES, MAX_OUTPUT_BYTES
            ),
            true,
        )
    } else {
        (clipped, false)
    }
}

/// Build the platform-default shell command for one script expression.
pub fn shell_command_for_platform(command: &str) -> Vec<String> {
    if cfg!(windows) {
        vec![
            "cmd.exe".to_string(),
            "/d".to_string(),
            "/s".to_string(),
            "/c".to_string(),
            command.to_string(),
        ]
    } else {
        vec![
            "/bin/sh".to_string(),
            "-lc".to_string(),
            command.to_string(),
        ]
    }
}

/// Build the Windows PowerShell command for one script expression.
pub fn powershell_command_for_windows(command: &str) -> Vec<String> {
    vec![
        "powershell.exe".to_string(),
        "-NoLogo".to_string(),
        "-NoProfile".to_string(),
        "-NonInteractive".to_string(),
        "-Command".to_string(),
        command.to_string(),
    ]
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_SHELL_TIMEOUT_MS, truncate_shell_output};

    #[test]
    fn preserves_short_output_and_default_timeout() {
        assert_eq!(DEFAULT_SHELL_TIMEOUT_MS, 120_000);
        assert_eq!(truncate_shell_output("ok"), ("ok".to_string(), false));
    }
}
