//! Shell command execution contracts (`ShellRequest`, `ShellOutput`).

/// Default timeout for a shell-tool process invocation.
pub const DEFAULT_SHELL_TIMEOUT_MS: u64 = 120_000;

const MAX_OUTPUT_BYTES: usize = 16 * 1024;
const MAX_OUTPUT_LINES: usize = 200;

/// Limit a shell result for model and presentation consumption.
pub fn truncate_shell_output(output: &str) -> (String, bool) {
    truncate_shell_output_budget(output, MAX_OUTPUT_BYTES)
}

/// Apply a requested preview budget before formatting, accounting for JSON
/// escaping and the omission notice. Disk capture is independent of preview.
pub fn truncate_shell_output_budget(output: &str, budget: usize) -> (String, bool) {
    let budget = budget.clamp(384, MAX_OUTPUT_BYTES);
    let line_count = output.lines().count();
    if json_text_bytes(output) <= budget && line_count <= MAX_OUTPUT_LINES {
        return (output.to_owned(), false);
    }
    let marker = format!(
        "\n\n[output truncated: preview keeps beginning and end of {line_count} lines / {} bytes. The output resource retains content independently of this preview.]\n\n",
        output.len(),
    );
    // Bound bytes before collecting lines: a multi-megabyte single line or
    // millions of empty lines must not allocate a full line index/copy.
    let half = budget.saturating_sub(json_text_bytes(&marker)) / 2;
    let rows = (MAX_OUTPUT_LINES - 6) / 2;
    let mut head_end = json_prefix_end(output, half);
    while !output.is_char_boundary(head_end) {
        head_end -= 1;
    }
    let mut tail_start = output.len();
    let mut tail_used = 0;
    for ch in output.chars().rev() {
        let cost = json_text_bytes(ch.encode_utf8(&mut [0; 4]));
        if tail_used + cost > half {
            break;
        }
        tail_used += cost;
        tail_start -= ch.len_utf8();
    }
    while !output.is_char_boundary(tail_start) {
        tail_start += 1;
    }
    let head = output[..head_end]
        .lines()
        .take(rows)
        .collect::<Vec<_>>()
        .join("\n");
    let mut tail = output[tail_start..]
        .lines()
        .rev()
        .take(rows)
        .collect::<Vec<_>>();
    tail.reverse();
    (format!("{head}{marker}{}", tail.join("\n")), true)
}

fn json_text_bytes(text: &str) -> usize {
    text.chars()
        .map(|ch| match ch {
            '"' | '\\' | '\n' | '\r' | '\t' | '\u{8}' | '\u{c}' => 2,
            '\u{0}'..='\u{1f}' => 6,
            _ => ch.len_utf8(),
        })
        .sum()
}
fn json_prefix_end(text: &str, budget: usize) -> usize {
    let mut used = 0;
    let mut end = 0;
    for ch in text.chars() {
        let cost = json_text_bytes(ch.encode_utf8(&mut [0; 4]));
        if used + cost > budget {
            break;
        }
        used += cost;
        end += ch.len_utf8();
    }
    end
}

/// Build the platform-default shell command for one script expression.
///
/// Historical helper: it always runs `/bin/sh -lc` on POSIX systems. New code
/// resolves the user's own shell through `agena_process::shell` and builds argv
/// with [`ShellLaunchSpec`]; this remains for callers that need the fixed
/// platform default.
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

/// Shell dialect whose command-line shape Agena builds.
///
/// The runtime resolves the concrete shell program (the user's login shell on
/// Unix, the configured command interpreter on Windows); this enum only records
/// which flag shape that program expects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ShellDialect {
    Bash,
    Zsh,
    #[default]
    Sh,
    Powershell,
    Cmd,
}

impl ShellDialect {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Bash => "bash",
            Self::Zsh => "zsh",
            Self::Sh => "sh",
            Self::Powershell => "powershell",
            Self::Cmd => "cmd",
        }
    }

    /// True for dialects that read POSIX startup files and accept `-lc`/`-c`.
    pub const fn is_posix(self) -> bool {
        matches!(self, Self::Bash | Self::Zsh | Self::Sh)
    }

    /// Build the dialect-specific argv for one command string.
    ///
    /// `login` only distinguishes `-lc` from `-c`; PowerShell and Command Prompt
    /// keep their existing non-interactive flags, matching Agena's historical
    /// behavior for those dialects.
    pub fn argv(self, program: &str, command: &str, login: bool) -> Vec<String> {
        match self {
            Self::Bash | Self::Zsh | Self::Sh => vec![
                program.to_string(),
                if login { "-lc" } else { "-c" }.to_string(),
                command.to_string(),
            ],
            Self::Powershell => vec![
                program.to_string(),
                "-NoLogo".to_string(),
                "-NoProfile".to_string(),
                "-NonInteractive".to_string(),
                "-Command".to_string(),
                command.to_string(),
            ],
            Self::Cmd => vec![
                program.to_string(),
                "/d".to_string(),
                "/s".to_string(),
                "/c".to_string(),
                command.to_string(),
            ],
        }
    }
}

/// One resolved shell launch: program, dialect, login mode, and the replayable
/// startup preamble captured from the shell's own interactive startup.
///
/// The preamble is declaration-only shell source produced by the shell itself
/// (aliases and options). Agena never evaluates captured code; it replays those
/// declarations before the model's command so the command sees the same shell
/// state as the user's terminal.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ShellLaunchSpec {
    pub program: String,
    pub dialect: ShellDialect,
    pub login: bool,
    pub preamble: String,
    /// Where the program came from: `override`, `detected`, or `fallback`.
    pub source: String,
    /// Snapshot provenance for tool metadata: `disabled`, `used(...)`, or
    /// `fallback: ...`. Never contains captured values.
    pub snapshot: String,
}

impl ShellLaunchSpec {
    /// Tool-metadata projection: provenance only, never captured values.
    pub fn metadata(&self) -> Vec<(String, String)> {
        vec![
            ("shell".to_string(), self.dialect.name().to_string()),
            ("shell_program".to_string(), self.program.clone()),
            ("shell_source".to_string(), self.source.clone()),
            ("login_shell".to_string(), self.login.to_string()),
            ("shell_snapshot".to_string(), self.snapshot.clone()),
        ]
    }

    /// Command text with the startup preamble applied.
    pub fn script(&self, command: &str) -> String {
        if self.preamble.is_empty() {
            command.to_string()
        } else {
            format!("{}{command}", self.preamble)
        }
    }

    /// Full argv for one command string.
    pub fn argv(&self, command: &str) -> Vec<String> {
        self.dialect.argv(
            self.program.as_str(),
            self.script(command).as_str(),
            self.login,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_SHELL_TIMEOUT_MS, ShellDialect, ShellLaunchSpec, truncate_shell_output};

    #[test]
    fn preserves_short_output_and_default_timeout() {
        assert_eq!(DEFAULT_SHELL_TIMEOUT_MS, 120_000);
        assert_eq!(truncate_shell_output("ok"), ("ok".to_string(), false));
    }

    #[test]
    fn platform_default_helper_keeps_the_historical_argv() {
        let argv = super::shell_command_for_platform("echo hi");
        if cfg!(windows) {
            assert_eq!(argv, vec!["cmd.exe", "/d", "/s", "/c", "echo hi"]);
        } else {
            assert_eq!(argv, vec!["/bin/sh", "-lc", "echo hi"]);
        }
    }

    #[test]
    fn posix_dialects_switch_between_login_and_plain_startup() {
        for dialect in [ShellDialect::Bash, ShellDialect::Zsh, ShellDialect::Sh] {
            assert!(dialect.is_posix());
            assert_eq!(
                dialect.argv("/bin/zsh", "echo hi", true),
                vec!["/bin/zsh", "-lc", "echo hi"]
            );
            assert_eq!(
                dialect.argv("/bin/zsh", "echo hi", false),
                vec!["/bin/zsh", "-c", "echo hi"]
            );
        }
        assert_eq!(ShellDialect::Zsh.name(), "zsh");
        assert_eq!(ShellDialect::Sh.name(), "sh");
    }

    #[test]
    fn windows_dialects_keep_their_historical_flags() {
        assert_eq!(
            ShellDialect::Cmd.argv("cmd.exe", "dir", true),
            vec!["cmd.exe", "/d", "/s", "/c", "dir"]
        );
        assert_eq!(
            ShellDialect::Powershell.argv("powershell.exe", "Get-Date", false),
            vec![
                "powershell.exe",
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Get-Date"
            ]
        );
        assert!(!ShellDialect::Cmd.is_posix());
        assert!(!ShellDialect::Powershell.is_posix());
    }

    #[test]
    fn launch_spec_replays_the_preamble_before_the_command() {
        let spec = ShellLaunchSpec {
            program: "/bin/zsh".to_string(),
            dialect: ShellDialect::Zsh,
            login: false,
            preamble: "alias ll='ls -l'\nsetopt extendedglob\n".to_string(),
            source: "detected".to_string(),
            snapshot: "used(env=2,dropped=0)".to_string(),
        };
        assert_eq!(
            spec.script("ll /tmp"),
            "alias ll='ls -l'\nsetopt extendedglob\nll /tmp"
        );
        assert_eq!(
            spec.argv("ll /tmp"),
            vec![
                "/bin/zsh",
                "-c",
                "alias ll='ls -l'\nsetopt extendedglob\nll /tmp"
            ]
        );

        let plain = ShellLaunchSpec {
            program: "/bin/sh".to_string(),
            dialect: ShellDialect::Sh,
            login: true,
            preamble: String::new(),
            source: "fallback".to_string(),
            snapshot: "disabled".to_string(),
        };
        assert_eq!(plain.script("echo hi"), "echo hi");
        assert_eq!(plain.argv("echo hi"), vec!["/bin/sh", "-lc", "echo hi"]);
        let metadata = plain.metadata();
        assert!(metadata.contains(&("shell".to_string(), "sh".to_string())));
        assert!(metadata.contains(&("login_shell".to_string(), "true".to_string())));
        assert!(metadata.contains(&("shell_snapshot".to_string(), "disabled".to_string())));
    }
}
