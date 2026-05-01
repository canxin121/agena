use std::cmp::min;
use std::collections::HashMap;

use super::shell::{ExecutionPolicy, ShellRequest};

use crate::message::{BashToolInput, BuiltinToolOutput};
use crate::plugin::{CommandAfterInput, CommandBeforeInput, CommandBeforeOutcome};

use super::{
    BuiltinExecution, BuiltinExecutionContext, ToolError, ToolExecutionView, ToolExecutor,
};

const DEFAULT_TIMEOUT_MS: u64 = 120_000;
const MAX_OUTPUT_BYTES: usize = 50 * 1024;
const MAX_OUTPUT_LINES: usize = 2_000;

pub(super) fn execute(
    executor: &ToolExecutor,
    input: &BashToolInput,
    context: BuiltinExecutionContext,
) -> Result<BuiltinExecution, ToolError> {
    if input.command.trim().is_empty() {
        return Err(ToolError::InvalidInput(
            "bash command must not be empty".to_string(),
        ));
    }

    let analysis = analyze_command(input.command.as_str());
    if matches!(executor.sandbox_policy(), ExecutionPolicy::ReadOnly)
        && let CommandClassification::Mutating { reason } = &analysis.classification
    {
        return Err(ToolError::PermissionDenied(format!(
            "bash command appears to modify files under a read-only sandbox: {reason}"
        )));
    }

    let cwd = input
        .workdir
        .as_deref()
        .map(|workdir| executor.resolve_target_path(workdir))
        .unwrap_or_else(|| executor.workspace_root().to_path_buf());
    executor.ensure_read_permission(&cwd)?;

    let mut env = inherited_environment();
    env.extend(executor.shell_env_overrides(&cwd, context.session_id, context.call_id)?);

    // Plugin chain: command.execute.before. Plugins can transform the
    // command line, override env, or abort the call entirely.
    let command_after_hook = {
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
                env = updated.env.into_iter().collect();
                if updated.args.len() >= 2 && updated.args[0] == "-c" && updated.command == "sh" {
                    Some((updated.args[1].clone(), updated.cwd))
                } else {
                    Some((input.command.clone(), updated.cwd))
                }
            }
            Ok(CommandBeforeOutcome::Abort(reason)) => {
                return Err(ToolError::PermissionDenied(format!(
                    "command aborted by plugin: {reason}"
                )));
            }
            Err(err) => {
                tracing::warn!(
                    target: "agena_plugin_host::command_before",
                    "command.execute.before hook failed (continuing): {err}"
                );
                None
            }
        }
    };
    let (final_command, final_cwd) =
        command_after_hook.unwrap_or_else(|| (input.command.clone(), cwd));

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

    let output = BuiltinToolOutput::Bash {
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
        "exit_interpretation".to_string(),
        exit_interpretation.label().to_string(),
    );
    if let Some(primary_command) = analysis.primary_command.as_deref() {
        view.metadata
            .insert("primary_command".to_string(), primary_command.to_string());
    }
    view.metadata.insert(
        "sandbox_mode".to_string(),
        format!("{:?}", executor.sandbox_policy()),
    );
    if execution.timed_out {
        view.metadata.insert(
            "timeout_ms".to_string(),
            request.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS).to_string(),
        );
    }

    Ok(BuiltinExecution::new(output, view))
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
    let byte_truncated = joined.as_bytes().len() > MAX_OUTPUT_BYTES;
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
