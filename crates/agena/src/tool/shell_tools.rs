use std::cmp::min;
use std::collections::HashMap;
use std::path::PathBuf;

use crate::message::{FilesystemEffect, ProcessShell, ProcessStatus};

use super::{ToolError, ToolExecutionView, ToolExecutor, ToolPayloadExecution, ToolPayloadOutput};

pub(crate) const DEFAULT_TIMEOUT_MS: u64 = 120_000;

const MAX_OUTPUT_BYTES: usize = 50 * 1024;
const MAX_OUTPUT_LINES: usize = 2_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CommandAnalysis {
    pub primary_command: Option<String>,
    pub subcommand: Option<String>,
    pub args: Vec<String>,
    pub classification: CommandClassification,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CommandClassification {
    ReadOnly,
    Mutating { reason: String },
    Unknown,
}

impl CommandClassification {
    pub(crate) const fn label(&self) -> &'static str {
        match self {
            Self::ReadOnly => "read_only",
            Self::Mutating { .. } => "mutating",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExitInterpretation {
    Success,
    NoMatches,
    DifferencesFound,
    Error,
}

impl ExitInterpretation {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::NoMatches => "no_matches",
            Self::DifferencesFound => "differences_found",
            Self::Error => "error",
        }
    }
}

pub(crate) fn inherited_environment() -> HashMap<String, String> {
    std::env::vars().collect::<HashMap<_, _>>()
}

pub(crate) fn resolve_workdir(
    executor: &ToolExecutor,
    workdir: Option<&str>,
) -> Result<PathBuf, ToolError> {
    let cwd = workdir
        .map(|value| executor.resolve_target_path(value))
        .unwrap_or_else(|| executor.workspace_root().to_path_buf());
    executor.ensure_read_permission(&cwd)?;
    Ok(cwd)
}

pub(crate) fn truncate_output(output: &str) -> (String, bool) {
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

/// Build the common persisted payload and UI view for a completed foreground
/// shell command. Shell-specific callers can append their own metadata.
pub(crate) fn shell_execution_result(
    shell: ProcessShell,
    title: impl Into<String>,
    status_text: String,
    trimmed_output: String,
    truncated: bool,
    exit_code: i32,
    duration_ms: u128,
    timed_out: bool,
) -> ToolPayloadExecution {
    let display_output = if trimmed_output.trim().is_empty() {
        status_text.clone()
    } else {
        trimmed_output
    };
    let output = ToolPayloadOutput::Shell {
        action: "run".to_string(),
        shell: Some(shell),
        background: false,
        process_id: None,
        status: Some(if timed_out {
            ProcessStatus::TimedOut
        } else {
            ProcessStatus::Exited
        }),
        output: Some(display_output.clone()),
        description: Some(status_text.clone()),
        events: Vec::new(),
        processes: Vec::new(),
        last_seq: 0,
        has_more: false,
        dropped_lines: 0,
        exit_code: Some(exit_code),
    };
    let mut view = ToolExecutionView::simple(title, display_output);
    view.metadata
        .insert("exit_code".to_string(), exit_code.to_string());
    view.metadata
        .insert("duration_ms".to_string(), duration_ms.to_string());
    view.metadata
        .insert("timed_out".to_string(), timed_out.to_string());
    view.metadata
        .insert("truncated".to_string(), truncated.to_string());
    view.metadata.insert("status".to_string(), status_text);
    ToolPayloadExecution::new(output, view)
}

pub(crate) fn validate_declared_filesystem_effects(
    tool_name: &str,
    command: &str,
    effects: &[FilesystemEffect],
) -> Result<(), ToolError> {
    if effects.is_empty()
        && let Some(reason) = filesystem_command_reason(command)
    {
        return Err(ToolError::InvalidInput(format!(
            "{tool_name} filesystem_effects must declare every accessed path because the command appears to touch the filesystem: {reason}"
        )));
    }
    Ok(())
}

pub(crate) fn shell_command_for_platform(command: &str) -> Vec<String> {
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

pub(crate) fn powershell_command_for_windows(command: &str) -> Vec<String> {
    vec![
        "powershell.exe".to_string(),
        "-NoLogo".to_string(),
        "-NoProfile".to_string(),
        "-NonInteractive".to_string(),
        "-Command".to_string(),
        command.to_string(),
    ]
}

pub(crate) fn mutating_command_reason(command: &str) -> Option<String> {
    match analyze_command(command).classification {
        CommandClassification::Mutating { reason } => Some(reason),
        CommandClassification::ReadOnly | CommandClassification::Unknown => None,
    }
}

pub(crate) fn filesystem_command_reason(command: &str) -> Option<String> {
    if let Some(reason) = mutating_command_reason(command) {
        return Some(reason);
    }

    if contains_unquoted_redirection(command, '<', '<') {
        return Some("uses shell input redirection".to_string());
    }

    let tokens = shell_tokens(command);
    for segment in command_segments(tokens.as_slice()) {
        if let Some(reason) = filesystem_segment_reason(segment) {
            return Some(reason);
        }
    }
    None
}

pub(crate) fn network_command_reason(command: &str) -> Option<String> {
    let tokens = shell_tokens(command);
    for segment in command_segments(tokens.as_slice()) {
        if let Some(reason) = network_segment_reason(segment) {
            return Some(reason);
        }
    }
    None
}

pub(crate) fn analyze_command(command: &str) -> CommandAnalysis {
    let tokens = shell_tokens(command);
    let (primary_command, subcommand, args) = first_command(tokens.as_slice());
    let classification = classify_command(command, tokens.as_slice());

    CommandAnalysis {
        primary_command,
        subcommand,
        args,
        classification,
    }
}

pub(crate) fn interpret_exit_code(
    analysis: &CommandAnalysis,
    exit_code: i32,
    timed_out: bool,
) -> ExitInterpretation {
    if timed_out {
        return ExitInterpretation::Error;
    }
    if exit_code == 0 {
        return ExitInterpretation::Success;
    }

    match (
        analysis.primary_command.as_deref(),
        analysis.subcommand.as_deref(),
        exit_code,
    ) {
        (Some("grep" | "rg"), _, 1) | (Some("git"), Some("grep"), 1) => {
            ExitInterpretation::NoMatches
        }
        (Some("diff" | "cmp"), _, 1) => ExitInterpretation::DifferencesFound,
        (Some("git"), Some("diff"), 1)
            if analysis
                .args
                .iter()
                .any(|arg| arg == "--exit-code" || arg == "--quiet") =>
        {
            ExitInterpretation::DifferencesFound
        }
        _ => ExitInterpretation::Error,
    }
}

fn classify_command(command: &str, tokens: &[String]) -> CommandClassification {
    if contains_unquoted_redirection(command, '>', '&') {
        return CommandClassification::Mutating {
            reason: "uses shell output redirection".to_string(),
        };
    }

    let segments = command_segments(tokens);
    if segments.is_empty() {
        return CommandClassification::Unknown;
    }

    let mut saw_unknown = false;
    for segment in segments {
        match classify_segment(segment) {
            CommandClassification::Mutating { reason } => {
                return CommandClassification::Mutating { reason };
            }
            CommandClassification::Unknown => saw_unknown = true,
            CommandClassification::ReadOnly => {}
        }
    }

    if saw_unknown {
        CommandClassification::Unknown
    } else {
        CommandClassification::ReadOnly
    }
}

fn classify_segment(tokens: &[String]) -> CommandClassification {
    let (Some(primary), subcommand, args) = first_command(tokens) else {
        return CommandClassification::Unknown;
    };

    if is_obvious_write_command(primary.as_str(), subcommand.as_deref(), args.as_slice()) {
        return CommandClassification::Mutating {
            reason: format!("invokes mutating command '{primary}'"),
        };
    }

    if is_known_read_only_command(primary.as_str(), subcommand.as_deref(), args.as_slice()) {
        return CommandClassification::ReadOnly;
    }

    CommandClassification::Unknown
}

fn network_segment_reason(tokens: &[String]) -> Option<String> {
    let (Some(primary), subcommand, args) = first_command(tokens) else {
        return None;
    };
    let primary_norm = primary.to_ascii_lowercase();
    let subcommand_norm = subcommand.as_deref().map(str::to_ascii_lowercase);

    if matches!(
        primary_norm.as_str(),
        "curl"
            | "wget"
            | "ssh"
            | "scp"
            | "sftp"
            | "rsync"
            | "telnet"
            | "nc"
            | "ncat"
            | "socat"
            | "ping"
            | "traceroute"
            | "dig"
            | "nslookup"
            | "host"
            | "ftp"
            | "tftp"
            | "invoke-webrequest"
            | "invoke-restmethod"
            | "start-bitstransfer"
            | "test-netconnection"
    ) {
        return Some(format!("invokes network command '{primary}'"));
    }

    if primary_norm == "git"
        && matches!(
            subcommand_norm.as_deref(),
            Some("fetch" | "pull" | "push" | "ls-remote")
        )
    {
        return Some(format!(
            "invokes git subcommand '{}' that may contact a remote",
            subcommand.unwrap_or_default()
        ));
    }

    if primary_norm == "git"
        && subcommand_norm.as_deref() == Some("clone")
        && args
            .iter()
            .filter(|arg| !arg.starts_with('-'))
            .any(|arg| looks_like_remote_target(arg))
    {
        return Some("invokes git clone with a remote target".to_string());
    }

    None
}

fn filesystem_segment_reason(tokens: &[String]) -> Option<String> {
    let (Some(primary), subcommand, args) = first_command(tokens) else {
        return Some("contains a command that could not be classified".to_string());
    };
    let primary_norm = primary.to_ascii_lowercase();
    let subcommand_norm = subcommand.as_deref().map(str::to_ascii_lowercase);

    if primary_norm == "curl" {
        return curl_filesystem_reason(args.as_slice());
    }

    if matches!(
        primary_norm.as_str(),
        "invoke-webrequest" | "invoke-restmethod"
    ) {
        return powershell_web_cmdlet_filesystem_reason(primary.as_str(), args.as_slice());
    }

    if primary_norm == "start-bitstransfer" {
        return Some("invokes Start-BitsTransfer which may read or write local files".to_string());
    }

    if matches!(
        primary_norm.as_str(),
        "pwd"
            | "echo"
            | "printf"
            | "env"
            | "printenv"
            | "date"
            | "uname"
            | "whoami"
            | "id"
            | "hostname"
            | "sleep"
            | "true"
            | "false"
            | "yes"
            | "seq"
            | "ps"
            | "kill"
            | "get-location"
            | "ping"
            | "traceroute"
            | "dig"
            | "nslookup"
            | "host"
            | "telnet"
            | "nc"
            | "ncat"
            | "socat"
            | "test-netconnection"
    ) {
        return None;
    }

    if matches!(
        primary_norm.as_str(),
        "cat"
            | "ls"
            | "find"
            | "grep"
            | "rg"
            | "diff"
            | "cmp"
            | "head"
            | "tail"
            | "sed"
            | "awk"
            | "sort"
            | "uniq"
            | "wc"
            | "stat"
            | "file"
            | "basename"
            | "dirname"
            | "realpath"
            | "which"
            | "type"
            | "du"
            | "tree"
            | "wget"
    ) {
        return Some(format!("invokes filesystem command '{primary}'"));
    }

    if primary_norm == "git" {
        return Some(match subcommand_norm.as_deref() {
            Some(subcommand) => format!("invokes git subcommand '{subcommand}'"),
            None => "invokes git".to_string(),
        });
    }

    if matches!(
        primary_norm.as_str(),
        "python"
            | "python3"
            | "node"
            | "perl"
            | "ruby"
            | "php"
            | "lua"
            | "sh"
            | "bash"
            | "zsh"
            | "fish"
            | "pwsh"
            | "powershell"
            | "cmd"
            | "make"
            | "just"
            | "cargo"
            | "go"
            | "java"
            | "javac"
            | "rustc"
            | "npm"
            | "pnpm"
            | "yarn"
            | "bun"
            | "scp"
            | "sftp"
            | "rsync"
            | "ssh"
    ) {
        return Some(format!(
            "invokes '{primary}' which may read or write local files"
        ));
    }

    Some(format!(
        "invokes command '{primary}' that is not proven to avoid filesystem access"
    ))
}

fn curl_filesystem_reason(args: &[String]) -> Option<String> {
    if args.iter().any(|arg| arg.starts_with("file://")) {
        return Some("invokes curl with a local file URL".to_string());
    }

    let mut index = 0;
    while index < args.len() {
        let arg = args[index].as_str();
        match arg {
            "-o" | "--output" => {
                if args.get(index + 1).is_none_or(|value| value != "-") {
                    return Some("invokes curl output option that writes a local file".to_string());
                }
            }
            "-O" | "--remote-name" => {
                return Some(
                    "invokes curl remote-name output which writes to the current directory"
                        .to_string(),
                );
            }
            "-T" | "--upload-file" => {
                return Some("invokes curl upload option that reads a local file".to_string());
            }
            "-K" | "--config" => {
                return Some("invokes curl config option that reads a local file".to_string());
            }
            "-c" | "--cookie-jar" => {
                return Some("invokes curl cookie-jar option that writes a local file".to_string());
            }
            "--cacert" | "--cert" | "--key" => {
                return Some(format!(
                    "invokes curl option '{arg}' that reads a local file"
                ));
            }
            "-b" | "--cookie" => {
                if args
                    .get(index + 1)
                    .is_some_and(|value| curl_cookie_option_uses_file(value.as_str()))
                {
                    return Some(
                        "invokes curl cookie option that reads from a local file".to_string(),
                    );
                }
            }
            "-d" | "--data" | "--data-ascii" | "--data-binary" | "--data-raw" | "--json" => {
                if args
                    .get(index + 1)
                    .is_some_and(|value| curl_data_option_uses_file(value.as_str()))
                {
                    return Some(format!(
                        "invokes curl option '{arg}' that reads request data from a local file"
                    ));
                }
            }
            "-F" | "--form" => {
                if args
                    .get(index + 1)
                    .is_some_and(|value| curl_form_option_uses_file(value.as_str()))
                {
                    return Some("invokes curl form upload that reads a local file".to_string());
                }
            }
            _ => {
                if arg.starts_with("-o") && arg.len() > 2 && &arg[2..] != "-" {
                    return Some("invokes curl output option that writes a local file".to_string());
                }
                if arg.starts_with("-T") && arg.len() > 2 {
                    return Some("invokes curl upload option that reads a local file".to_string());
                }
                if arg.starts_with("-K") && arg.len() > 2 {
                    return Some("invokes curl config option that reads a local file".to_string());
                }
                if arg.starts_with("--upload-file=") {
                    return Some("invokes curl upload option that reads a local file".to_string());
                }
                if arg.starts_with("--output=") && arg != "--output=-" {
                    return Some("invokes curl output option that writes a local file".to_string());
                }
                if arg.starts_with("--config=") {
                    return Some("invokes curl config option that reads a local file".to_string());
                }
                if arg.starts_with("--cookie-jar=") {
                    return Some(
                        "invokes curl cookie-jar option that writes a local file".to_string(),
                    );
                }
                if arg.starts_with("--cacert=")
                    || arg.starts_with("--cert=")
                    || arg.starts_with("--key=")
                {
                    return Some(format!(
                        "invokes curl option '{arg}' that reads a local file"
                    ));
                }
                if arg.starts_with("--cookie=")
                    && curl_cookie_option_uses_file(
                        arg.split_once('=').map(|(_, value)| value).unwrap_or(""),
                    )
                {
                    return Some(
                        "invokes curl cookie option that reads from a local file".to_string(),
                    );
                }
                if arg.starts_with("--data=")
                    || arg.starts_with("--data-ascii=")
                    || arg.starts_with("--data-binary=")
                    || arg.starts_with("--data-raw=")
                    || arg.starts_with("--json=")
                {
                    let value = arg.split_once('=').map(|(_, value)| value).unwrap_or("");
                    if curl_data_option_uses_file(value) {
                        return Some(
                            "invokes curl data option that reads request data from a local file"
                                .to_string(),
                        );
                    }
                }
                if arg.starts_with("--form=") {
                    let value = arg.split_once('=').map(|(_, value)| value).unwrap_or("");
                    if curl_form_option_uses_file(value) {
                        return Some(
                            "invokes curl form upload that reads a local file".to_string(),
                        );
                    }
                }
                if arg.starts_with("-c") && arg.len() > 2 {
                    return Some(
                        "invokes curl cookie-jar option that writes a local file".to_string(),
                    );
                }
                if arg.starts_with("-b") && arg.len() > 2 && curl_cookie_option_uses_file(&arg[2..])
                {
                    return Some(
                        "invokes curl cookie option that reads from a local file".to_string(),
                    );
                }
                if arg.starts_with("-d") && arg.len() > 2 && curl_data_option_uses_file(&arg[2..]) {
                    return Some(
                        "invokes curl data option that reads request data from a local file"
                            .to_string(),
                    );
                }
                if arg.starts_with("-F") && arg.len() > 2 && curl_form_option_uses_file(&arg[2..]) {
                    return Some("invokes curl form upload that reads a local file".to_string());
                }
            }
        }
        index += 1;
    }

    None
}

fn powershell_web_cmdlet_filesystem_reason(primary: &str, args: &[String]) -> Option<String> {
    for arg in args {
        let lower = arg.to_ascii_lowercase();
        if lower == "-outfile" || lower.starts_with("-outfile:") {
            return Some(format!(
                "invokes {primary} with -OutFile which writes a local file"
            ));
        }
        if lower == "-infile" || lower.starts_with("-infile:") {
            return Some(format!(
                "invokes {primary} with -InFile which reads a local file"
            ));
        }
    }
    None
}

fn curl_cookie_option_uses_file(value: &str) -> bool {
    let trimmed = value.trim();
    !trimmed.is_empty() && !trimmed.contains('=')
}

fn curl_data_option_uses_file(value: &str) -> bool {
    value.trim_start().starts_with('@')
}

fn curl_form_option_uses_file(value: &str) -> bool {
    value.contains('@') || value.contains('<')
}

fn first_command(tokens: &[String]) -> (Option<String>, Option<String>, Vec<String>) {
    let Some(segment) = command_segments(tokens).into_iter().next() else {
        return (None, None, Vec::new());
    };

    let mut index = 0;
    while index < segment.len() && is_assignment(segment[index].as_str()) {
        index += 1;
    }
    while index < segment.len() && is_command_wrapper(segment[index].as_str()) {
        index += 1;
    }
    let Some(primary) = segment.get(index).cloned() else {
        return (None, None, Vec::new());
    };

    let args = segment.iter().skip(index + 1).cloned().collect::<Vec<_>>();
    let subcommand = if primary == "git" {
        args.iter().find(|arg| !arg.starts_with('-')).cloned()
    } else {
        None
    };

    (Some(primary), subcommand, args)
}

fn command_segments(tokens: &[String]) -> Vec<&[String]> {
    let mut segments = Vec::new();
    let mut start = 0;
    for (index, token) in tokens.iter().enumerate() {
        if is_separator(token.as_str()) {
            if start < index {
                segments.push(&tokens[start..index]);
            }
            start = index + 1;
        }
    }
    if start < tokens.len() {
        segments.push(&tokens[start..]);
    }
    segments
}

fn shell_tokens(command: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut single_quote = false;
    let mut double_quote = false;
    let mut escape = false;

    for ch in command.chars() {
        if escape {
            current.push(ch);
            escape = false;
            continue;
        }

        match ch {
            '\\' if !single_quote => {
                escape = true;
            }
            '\'' if !double_quote => {
                single_quote = !single_quote;
            }
            '"' if !single_quote => {
                double_quote = !double_quote;
            }
            c if c.is_whitespace() && !single_quote && !double_quote => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            ';' | '|' | '&' if !single_quote && !double_quote => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
                tokens.push(ch.to_string());
            }
            _ => current.push(ch),
        }
    }

    if !current.is_empty() {
        tokens.push(current);
    }

    tokens
}

fn contains_unquoted_redirection(
    command: &str,
    redirection: char,
    ignored_following: char,
) -> bool {
    let mut single_quote = false;
    let mut double_quote = false;
    let mut escape = false;
    let mut chars = command.chars().peekable();

    while let Some(ch) = chars.next() {
        if escape {
            escape = false;
            continue;
        }

        match ch {
            '\\' if !single_quote => {
                escape = true;
            }
            '\'' if !double_quote => {
                single_quote = !single_quote;
            }
            '"' if !single_quote => {
                double_quote = !double_quote;
            }
            operator if operator == redirection && !single_quote && !double_quote => {
                if chars.peek() == Some(&ignored_following) {
                    chars.next();
                    continue;
                }
                return true;
            }
            _ => {}
        }
    }

    false
}

fn is_known_read_only_command(primary: &str, subcommand: Option<&str>, args: &[String]) -> bool {
    if matches!(
        primary,
        "cat"
            | "pwd"
            | "ls"
            | "find"
            | "grep"
            | "rg"
            | "diff"
            | "cmp"
            | "head"
            | "tail"
            | "sed"
            | "awk"
            | "sort"
            | "uniq"
            | "wc"
            | "stat"
            | "file"
            | "basename"
            | "dirname"
            | "realpath"
            | "echo"
            | "printf"
            | "env"
            | "which"
            | "type"
            | "du"
            | "ps"
    ) {
        return !matches!(primary, "sed") || !args.iter().any(|arg| is_in_place_flag(arg));
    }

    if primary == "git" {
        return matches!(
            subcommand,
            Some("status" | "diff" | "grep" | "show" | "log" | "rev-parse")
        );
    }

    false
}

fn is_obvious_write_command(primary: &str, subcommand: Option<&str>, args: &[String]) -> bool {
    if matches!(
        primary,
        "touch"
            | "mkdir"
            | "rmdir"
            | "rm"
            | "mv"
            | "cp"
            | "install"
            | "chmod"
            | "chown"
            | "ln"
            | "tee"
            | "dd"
            | "truncate"
    ) {
        return true;
    }

    if matches!(primary, "sed" | "perl") && args.iter().any(|arg| is_in_place_flag(arg)) {
        return true;
    }

    if primary == "git" {
        return matches!(
            subcommand,
            Some(
                "apply"
                    | "am"
                    | "add"
                    | "checkout"
                    | "switch"
                    | "restore"
                    | "commit"
                    | "merge"
                    | "rebase"
                    | "cherry-pick"
                    | "revert"
                    | "clean"
                    | "stash"
            )
        );
    }

    false
}

fn is_in_place_flag(arg: &str) -> bool {
    arg == "-i" || (arg.starts_with("-i") && arg.len() > 2)
}

fn is_separator(token: &str) -> bool {
    matches!(token, ";" | "|" | "&")
}

fn is_assignment(token: &str) -> bool {
    let Some((name, _value)) = token.split_once('=') else {
        return false;
    };
    !name.is_empty()
        && name
            .chars()
            .all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
        && name
            .chars()
            .next()
            .is_some_and(|ch| ch == '_' || ch.is_ascii_alphabetic())
}

fn is_command_wrapper(token: &str) -> bool {
    matches!(token, "env" | "command" | "builtin" | "nohup")
}

fn looks_like_remote_target(arg: &str) -> bool {
    let trimmed = arg.trim();
    if trimmed.is_empty() {
        return false;
    }
    trimmed.contains("://")
        || trimmed.starts_with("ssh://")
        || trimmed.starts_with("git@")
        || trimmed.starts_with("http://")
        || trimmed.starts_with("https://")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unquoted_redirection_detection_preserves_shell_specific_exceptions() {
        assert!(contains_unquoted_redirection("cat < input.txt", '<', '<'));
        assert!(!contains_unquoted_redirection("cat <<EOF", '<', '<'));
        assert!(contains_unquoted_redirection(
            "echo value > output.txt",
            '>',
            '&'
        ));
        assert!(!contains_unquoted_redirection("echo value >&2", '>', '&'));
        assert!(!contains_unquoted_redirection(
            "echo 'value > output'",
            '>',
            '&'
        ));
        assert!(!contains_unquoted_redirection(
            "echo \"value < input\"",
            '<',
            '<'
        ));
        assert!(!contains_unquoted_redirection("echo \\> output", '>', '&'));
    }
}
