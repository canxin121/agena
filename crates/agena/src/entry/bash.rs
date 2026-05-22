use std::cmp::min;
use std::collections::HashMap;

use super::shell::ShellRequest;

use crate::message::BashToolInput;
use crate::plugin::{CommandAfterInput, CommandBeforeInput, CommandBeforeOutcome};

use super::{
    PreparedShellCommand, ToolError, ToolExecutionView, ToolExecutor, ToolPayloadExecution,
    ToolPayloadOutput, ToolRuntimeContext,
};

const DEFAULT_TIMEOUT_MS: u64 = 120_000;
const MAX_OUTPUT_BYTES: usize = 50 * 1024;
const MAX_OUTPUT_LINES: usize = 2_000;

pub(super) fn prepare_command(
    executor: &ToolExecutor,
    input: &BashToolInput,
    session_id: i64,
    call_id: i64,
) -> Result<Option<PreparedShellCommand>, ToolError> {
    let cwd = input
        .workdir
        .as_deref()
        .map(|workdir| executor.resolve_target_path(workdir))
        .unwrap_or_else(|| executor.workspace_root().to_path_buf());
    executor.ensure_read_permission(&cwd)?;

    let mut env = inherited_environment();
    env.extend(executor.shell_env_overrides(&cwd, Some(session_id), Some(call_id))?);

    let env_btree = env
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect::<std::collections::BTreeMap<String, String>>();
    let hook_input = CommandBeforeInput {
        command: "sh".to_string(),
        args: vec!["-c".to_string(), input.command.clone()],
        cwd: cwd.clone(),
        env: env_btree,
    };
    match executor
        .plugin_manager()
        .dispatch_command_before_blocking(hook_input)
    {
        Ok(CommandBeforeOutcome::Continue(updated)) => {
            let command =
                if updated.args.len() >= 2 && updated.args[0] == "-c" && updated.command == "sh" {
                    updated.args[1].clone()
                } else {
                    input.command.clone()
                };
            Ok(Some(PreparedShellCommand {
                command,
                cwd: updated.cwd,
            }))
        }
        Ok(CommandBeforeOutcome::Abort(reason)) => Err(ToolError::PermissionDenied(format!(
            "command aborted by plugin: {reason}"
        ))),
        Err(err) => {
            tracing::warn!(
                target: "agena_plugin_host::command_before",
                "command.execute.before hook failed (continuing): {err}"
            );
            Ok(None)
        }
    }
}

pub(super) fn execute(
    executor: &ToolExecutor,
    input: &BashToolInput,
    context: ToolRuntimeContext,
) -> Result<ToolPayloadExecution, ToolError> {
    if input.command.trim().is_empty() {
        return Err(ToolError::InvalidInput(
            "bash command must not be empty".to_string(),
        ));
    }

    let analysis = analyze_command(input.command.as_str());
    if input.filesystem_effects.is_empty()
        && let Some(reason) = filesystem_command_reason(input.command.as_str())
    {
        return Err(ToolError::InvalidInput(format!(
            "bash filesystem_effects must declare every accessed path because the command appears to touch the filesystem: {reason}"
        )));
    }

    let cwd = input
        .workdir
        .as_deref()
        .map(|workdir| executor.resolve_target_path(workdir))
        .unwrap_or_else(|| executor.workspace_root().to_path_buf());
    executor.ensure_read_permission(&cwd)?;
    executor.ensure_filesystem_effects_permission(&input.filesystem_effects, &cwd)?;
    executor.ensure_network_effects_permission(&input.network_effects)?;

    let mut env = inherited_environment();
    env.extend(executor.shell_env_overrides(&cwd, context.session_id, context.call_id)?);

    let prepared = match context.prepared_shell_command {
        Some(prepared) => Some(prepared),
        None => match (context.session_id, context.call_id) {
            (Some(session_id), Some(call_id)) => {
                executor.prepare_shell_command(input, session_id, call_id)?
            }
            _ => None,
        },
    };
    let (final_command, final_cwd) = prepared
        .map(|prepared| (prepared.command, prepared.cwd))
        .unwrap_or_else(|| (input.command.clone(), cwd));
    let final_analysis = analyze_command(final_command.as_str());
    let command_rewritten = final_command != input.command;

    let request = ShellRequest {
        command: shell_command_for_platform(&final_command),
        cwd: final_cwd,
        env,
        timeout_ms: Some(input.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS)),
    };

    let execution = executor.execute_shell_command(&request)?;

    // Plugin chain: command.execute.after. Plugins can observe or rewrite
    // stdout/stderr; we use the (potentially patched) combined output.
    let patched_after = {
        let hook_input = CommandAfterInput {
            command: "sh".to_string(),
            args: vec!["-c".to_string(), final_command.clone()],
            cwd: request.cwd.clone(),
            exit_code: Some(execution.exit_code),
            stdout: execution.stdout.clone(),
            stderr: execution.stderr.clone(),
            timed_out: execution.timed_out,
        };
        match executor
            .plugin_manager()
            .dispatch_command_after_blocking(hook_input)
        {
            Ok(after) => Some(after),
            Err(err) => {
                tracing::warn!(
                    target: "agena_plugin_host::command_after",
                    "command.execute.after hook failed (continuing): {err}"
                );
                None
            }
        }
    };
    let aggregated_for_display = patched_after
        .map(|a| {
            if a.stdout.is_empty() {
                a.stderr
            } else if a.stderr.is_empty() {
                a.stdout
            } else {
                format!("{}\n{}", a.stdout, a.stderr)
            }
        })
        .unwrap_or(execution.aggregated_output.clone());

    let (trimmed_output, truncated) = truncate_output(&aggregated_for_display);
    let exit_interpretation =
        interpret_exit_code(&analysis, execution.exit_code, execution.timed_out);

    let status_text = if execution.timed_out {
        format!(
            "Command timed out after {} ms (exit_code={}).",
            request.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS),
            execution.exit_code
        )
    } else if matches!(exit_interpretation, ExitInterpretation::NoMatches) {
        format!(
            "Command completed with no matches (exit_code={}) in {} ms.",
            execution.exit_code,
            execution.duration.as_millis()
        )
    } else if matches!(exit_interpretation, ExitInterpretation::DifferencesFound) {
        format!(
            "Command completed and found differences (exit_code={}) in {} ms.",
            execution.exit_code,
            execution.duration.as_millis()
        )
    } else {
        format!(
            "Command exited with code {} in {} ms.",
            execution.exit_code,
            execution.duration.as_millis()
        )
    };
    let display_output = if trimmed_output.trim().is_empty() {
        status_text.clone()
    } else {
        trimmed_output.clone()
    };

    let output = ToolPayloadOutput::Bash {
        output: Some(display_output.clone()),
        description: Some(status_text.clone()),
    };

    let title = if input.description.trim().is_empty() {
        format!("Bash {}", input.command)
    } else {
        format!("Bash {}", input.description)
    };

    let mut view = ToolExecutionView::simple(title, display_output);
    view.metadata
        .insert("exit_code".to_string(), execution.exit_code.to_string());
    view.metadata.insert(
        "duration_ms".to_string(),
        execution.duration.as_millis().to_string(),
    );
    view.metadata
        .insert("timed_out".to_string(), execution.timed_out.to_string());
    view.metadata
        .insert("truncated".to_string(), truncated.to_string());
    view.metadata.insert(
        "command_classification".to_string(),
        analysis.classification.label().to_string(),
    );
    view.metadata.insert(
        "final_command_classification".to_string(),
        final_analysis.classification.label().to_string(),
    );
    view.metadata.insert(
        "command_rewritten".to_string(),
        command_rewritten.to_string(),
    );
    view.metadata.insert(
        "exit_interpretation".to_string(),
        exit_interpretation.label().to_string(),
    );
    if let Some(primary_command) = analysis.primary_command.as_deref() {
        view.metadata
            .insert("primary_command".to_string(), primary_command.to_string());
    }
    if let Some(primary_command) = final_analysis.primary_command.as_deref() {
        view.metadata.insert(
            "final_primary_command".to_string(),
            primary_command.to_string(),
        );
    }
    if command_rewritten {
        view.metadata
            .insert("final_command".to_string(), final_command.clone());
    }
    if execution.timed_out {
        view.metadata.insert(
            "timeout_ms".to_string(),
            request.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS).to_string(),
        );
    }

    Ok(ToolPayloadExecution::new(output, view))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CommandAnalysis {
    primary_command: Option<String>,
    subcommand: Option<String>,
    args: Vec<String>,
    classification: CommandClassification,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CommandClassification {
    ReadOnly,
    Mutating { reason: String },
    Unknown,
}

impl CommandClassification {
    const fn label(&self) -> &'static str {
        match self {
            Self::ReadOnly => "read_only",
            Self::Mutating { .. } => "mutating",
            Self::Unknown => "unknown",
        }
    }
}

/// Classify a shell command line as definitely read-only. Returns `false`
/// for mutating commands and for anything we cannot prove is read-only —
/// callers gating untrusted execution should treat `false` as "ask first".
pub fn is_read_only_command(command: &str) -> bool {
    matches!(
        analyze_command(command).classification,
        CommandClassification::ReadOnly
    )
}

pub(super) fn mutating_command_reason(command: &str) -> Option<String> {
    match analyze_command(command).classification {
        CommandClassification::Mutating { reason } => Some(reason),
        CommandClassification::ReadOnly | CommandClassification::Unknown => None,
    }
}

pub(crate) fn filesystem_command_reason(command: &str) -> Option<String> {
    if let Some(reason) = mutating_command_reason(command) {
        return Some(reason);
    }

    if contains_input_redirection(command) {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExitInterpretation {
    Success,
    NoMatches,
    DifferencesFound,
    Error,
}

impl ExitInterpretation {
    const fn label(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::NoMatches => "no_matches",
            Self::DifferencesFound => "differences_found",
            Self::Error => "error",
        }
    }
}

fn inherited_environment() -> HashMap<String, String> {
    std::env::vars().collect::<HashMap<_, _>>()
}

fn shell_command_for_platform(command: &str) -> Vec<String> {
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

fn truncate_output(output: &str) -> (String, bool) {
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

fn analyze_command(command: &str) -> CommandAnalysis {
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

fn classify_command(command: &str, tokens: &[String]) -> CommandClassification {
    if contains_write_redirection(command) {
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

fn interpret_exit_code(
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

fn contains_write_redirection(command: &str) -> bool {
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
            '>' if !single_quote && !double_quote => {
                if chars.peek() == Some(&'&') {
                    continue;
                }
                return true;
            }
            _ => {}
        }
    }

    false
}

fn contains_input_redirection(command: &str) -> bool {
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
            '<' if !single_quote && !double_quote => {
                if chars.peek() == Some(&'<') {
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
