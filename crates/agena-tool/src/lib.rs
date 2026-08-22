//! # agena-tool
//!
//! Provider-independent tool contracts.
//!
//! Concrete executors, built-in tools, plugin hosts, and permission policy
//! implementations belong in adapter/runtime crates rather than this crate.
//!
//! ## What lives here
//!
//! - **Tool descriptors** — normalization helpers ([`normalize_tool_title`],
//!   [`normalize_tool_summary`], [`compose_tool_title`]) and
//!   [`invocation_call_summary`].
//! - **Execution contracts** — [`PreparedToolInvocation`],
//!   [`ToolPermissionCheck`], [`ToolExecutionSummary`], [`ToolRuntimeEvent`],
//!   and the runtime event sink.
//! - **Shell** — [`shell`] provides [`ShellRequest`] / [`ShellOutput`] and
//!   [`ShellError`]; [`shell_analysis`] analyzes command shapes.
//! - **Search** — [`code_search`] and [`tool_search`] locate code and tools.
//! - **Value types** — [`ReadMode`], [`SnapshotBackend`],
//!   [`ToolAvailability`], patch operations, and cron summaries.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

pub mod tool_activity;
use agena_domain::{
    CommandBeginEvent, CommandEndEvent, CommandOutputDeltaEvent, PermissionAction,
    PermissionDecision, RawOutput, ToolInvocation, ToolPermissionContract, ToolResultState,
};
pub use agena_plugin_contracts::{
    TOOL_SUMMARY_MAX_DISPLAY_WIDTH, TOOL_TITLE_MAX_DISPLAY_WIDTH, normalize_tool_summary,
    normalize_tool_title,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
pub use tool_activity::{
    RenderContext, RenderError, ToolActivityEvent, ToolActivityResult, ToolHumanRenderer,
};

/// Compose one compact tool headline from an action and a detail fragment
/// (for example `Run process · cargo test` or
/// `Read README.md · 12 lines`). The first value is retained when the detail
/// is empty, and the result is always bounded by the title contract.
pub fn compose_tool_title(tool_name: impl AsRef<str>, summary: impl AsRef<str>) -> String {
    let tool_name = tool_name.as_ref().trim();
    let summary = summary.as_ref().trim();
    if summary.is_empty() || summary == tool_name {
        return normalize_tool_title(tool_name);
    }
    if tool_name.is_empty() {
        return normalize_tool_title(summary);
    }
    normalize_tool_title(format!("{tool_name} · {summary}"))
}

/// Render the title that is available as soon as a tool call is created.
///
/// This deliberately uses only the invocation name and input.  The title is
/// human-facing, so a namespace such as `agena.fs` is translated into an
/// action and the most useful safe input values become its subject:
/// `Read README.md`, `Search files · TODO`, or `Run process · cargo test`.
pub fn initial_tool_title(invocation: &ToolInvocation) -> String {
    let input = serde_json::Value::from(invocation.input.clone());
    render_invocation_title(invocation.name.as_str(), &input)
}

/// Return whether a renderer supplied only one spelling of the invocation
/// identity (for example `shell.run`, `agena.shell.run`, or
/// `agena_shell_run`) instead of a human action title.  Execution adapters
/// commonly use the raw name as their fallback; treating it as a custom title
/// would make the completed headline regress to `agena.shell.run · passed`.
pub fn is_tool_identity_title(title: &str, invocation: &ToolInvocation) -> bool {
    let title = title.trim();
    !title.is_empty()
        && normalized_tool_identity(title) == normalized_tool_identity(invocation.name.as_str())
}

/// Render the final title after the tool has produced its raw result.
///
/// The full raw result remains available to the presentation blocks and the
/// detail sections.  The headline only keeps the smallest useful result fact
/// (for example `passed`, `36 matches`, or `1 file changed`) so it remains a
/// scan label rather than becoming a second output dump.
pub fn completed_tool_title(invocation: &ToolInvocation, output: &RawOutput) -> String {
    completed_tool_title_with_action_for_invocation(
        invocation,
        initial_tool_title(invocation),
        output,
    )
}

/// Render a terminal title when a tool has no raw output but still has a
/// meaningful lifecycle result, such as a denied, cancelled, or empty
/// successful operation.
pub fn tool_title_for_state(invocation: &ToolInvocation, state: ToolResultState) -> String {
    let result = match state {
        ToolResultState::Completed => "completed",
        ToolResultState::PolicyDenied => "permission denied",
        ToolResultState::UserDeclined => "declined",
        ToolResultState::CapabilityUnavailable => "unavailable",
        ToolResultState::ToolUnavailable => "unavailable",
        ToolResultState::Failed => "failed",
        ToolResultState::Cancelled => "cancelled",
        ToolResultState::Pending | ToolResultState::Running => {
            return initial_tool_title(invocation);
        }
    };
    compose_tool_title(initial_tool_title(invocation), result)
}

/// Render a terminal title when both lifecycle state and raw output are
/// available. Raw result facts remain primary, while a failure/denial/
/// cancellation state is appended when the payload did not describe it
/// explicitly. This prevents a failed call with a partial payload from
/// looking like a successful result.
pub fn completed_tool_title_for_state(
    invocation: &ToolInvocation,
    state: ToolResultState,
    output: &RawOutput,
) -> String {
    completed_tool_title_with_action_for_state(
        invocation,
        initial_tool_title(invocation),
        state,
        output,
    )
}

/// Complete an already-rendered action title with the compact result fact.
/// Execution views use this variant because a tool may have a more specific
/// action label than the generic invocation renderer can infer.
pub fn completed_tool_title_with_action(
    action_title: impl AsRef<str>,
    output: &RawOutput,
) -> String {
    complete_title(action_title.as_ref(), result_title_fragment(output))
}

/// Complete the invocation's call-time title with a result fact while
/// retaining the tool identity for tool-specific result semantics. The
/// `_action_title` parameter remains for source compatibility with execution
/// adapters that already have a plugin-provided action label, but it cannot
/// replace the invocation title: the completed headline must retain the
/// action/input that was visible when the call started. Keeping the invocation
/// here lets a `200` mean `HTTP 200`, a `kind=updated` mean `updated`, and a
/// provider `pending_calls` array mean `N pending calls` instead of falling
/// back to an unhelpful generic scalar.
pub fn completed_tool_title_with_action_for_invocation(
    invocation: &ToolInvocation,
    _action_title: impl AsRef<str>,
    output: &RawOutput,
) -> String {
    // The call-time action/input title is the stable identity of the
    // operation. A plugin may provide a nicer completion label, but allowing
    // that label to replace the invocation title would make the headline lose
    // the input that the user saw when the call started (for example,
    // `Search web · Agena` becoming `ChatGPT web search · response received`).
    // Keep the optional action argument for callers that need to supply an
    // already-rendered title, while treating the invocation-aware path as the
    // canonical lifecycle projection.
    let action_title = initial_tool_title(invocation);
    complete_title(
        action_title.as_str(),
        result_title_fragment_for_invocation(invocation, output),
    )
}

/// State-aware form of the invocation-aware action-title completion helper.
/// Plugin renderers and read-time API projections use it when the durable part
/// carries both a raw result and a lifecycle state.
pub fn completed_tool_title_with_action_for_state(
    invocation: &ToolInvocation,
    action_title: impl AsRef<str>,
    state: ToolResultState,
    output: &RawOutput,
) -> String {
    // A partial/streaming raw output is not a terminal result. Keep the
    // call-time title until the lifecycle itself reaches a terminal state;
    // otherwise a checkpoint could briefly claim success from an incomplete
    // payload.
    if matches!(state, ToolResultState::Pending | ToolResultState::Running) {
        return initial_tool_title(invocation);
    }
    let title = completed_tool_title_with_action_for_invocation(invocation, action_title, output);
    let suffix = match state {
        ToolResultState::PolicyDenied => Some("permission denied"),
        ToolResultState::UserDeclined => Some("declined"),
        ToolResultState::CapabilityUnavailable | ToolResultState::ToolUnavailable => {
            Some("unavailable")
        }
        ToolResultState::Failed => Some("failed"),
        ToolResultState::Cancelled => Some("cancelled"),
        ToolResultState::Completed
            if result_title_fragment_for_tool(invocation.name.as_str(), output).is_empty() =>
        {
            Some("completed")
        }
        ToolResultState::Pending | ToolResultState::Running | ToolResultState::Completed => None,
    };
    let Some(suffix) = suffix else {
        return title;
    };
    let lower = title.to_ascii_lowercase();
    if lower.contains(suffix) {
        title
    } else {
        compose_tool_title(title, suffix)
    }
}

fn complete_title(action_title: &str, result: String) -> String {
    let action_title = normalize_tool_title(action_title);
    if action_title.is_empty() {
        return normalize_tool_title(result);
    }
    let result = missing_result_facts(action_title.as_str(), result.as_str());
    if result.is_empty() {
        return action_title;
    }
    compose_tool_title(action_title, result)
}

fn missing_result_facts(title: &str, result: &str) -> String {
    let title = title.trim().to_ascii_lowercase();
    let result = result.trim();
    if result.is_empty() {
        return String::new();
    }
    let title_parts = title.split('·').map(str::trim).collect::<Vec<_>>();
    let missing = result
        .split('·')
        .map(str::trim)
        .filter(|result_part| {
            let result_part_lower = result_part.to_ascii_lowercase();
            !title_parts.iter().any(|title_part| {
                *title_part == result_part_lower.as_str()
                    || (title_part.ends_with(result_part_lower.as_str())
                        && result_part_lower.chars().any(|character| {
                            matches!(character, '/' | '.' | ':' | '#' | '@')
                                || character.is_ascii_digit()
                        }))
                    || result_part_lower
                        .split_whitespace()
                        .next()
                        .is_some_and(|prefix| prefix.contains('/') && *title_part == prefix)
            })
        })
        .collect::<Vec<_>>();
    missing.join(" · ")
}

/// Extract the one result fact worth placing in a collapsed tool headline.
///
/// This function intentionally recognises facts rather than serialising the
/// payload.  It keeps secrets, large text, schemas, and nested machine data
/// out of the title while still covering the common tool result shapes.
pub fn result_title_fragment(output: &RawOutput) -> String {
    let mut candidates = Vec::new();
    if let Some(object) = output
        .payload
        .as_ref()
        .and_then(serde_json::Value::as_object)
        && let Some(fragment) = result_title_fragment_from_object(object)
    {
        candidates.push(fragment);
    }

    // Some plugin/execution adapters expose the same compact facts as string
    // metadata rather than putting them in the structured payload. Metadata
    // is still part of RawOutput, so it must participate in the final title.
    if !output.metadata.is_empty() {
        let metadata = output
            .metadata
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect::<serde_json::Map<_, _>>();
        if let Some(fragment) = result_title_fragment_from_object(&metadata) {
            candidates.push(fragment);
        }
    }

    if let Some(object) = serde_json::from_str::<serde_json::Value>(output.text.trim())
        .ok()
        .and_then(|value| value.as_object().cloned())
        && let Some(fragment) = result_title_fragment_from_object(&object)
    {
        candidates.push(fragment);
    }

    if let Some(fragment) = choose_result_fragment(candidates) {
        return finalize_result_fragment(fragment, output);
    }

    if !output.text.trim().is_empty()
        && let Some(value) = compact_result_text(output.text.as_str())
    {
        return finalize_result_fragment(value, output);
    }
    if output.truncated {
        return "output truncated".to_owned();
    }
    if !output.managed_outputs.is_empty() {
        return format_count(output.managed_outputs.len(), "outputs saved");
    }
    if !output.attachments.is_empty() {
        return format_count(output.attachments.len(), "attachments");
    }
    String::new()
}

/// Extract the compact result fact for a named tool without requiring a full
/// invocation. Human renderers use this to keep their one-line summary and
/// collapsed title aligned for tool-specific facts such as HTTP status,
/// changed-file kind, and provider continuation calls.
pub fn result_title_fragment_for_tool(tool_name: &str, output: &RawOutput) -> String {
    let key = normalized_tool_identity(tool_name);

    // Provider-native image calls persist binary results as attachments while
    // keeping the provider envelope intentionally text-light. Surface the
    // artifact count before inspecting the generic response envelope so a
    // completed generation reads as `1 image`, not merely `response received`.
    if is_provider_tool_identity(key.as_str())
        && key.contains("image")
        && !output.attachments.is_empty()
    {
        return finalize_result_fragment(format_count(output.attachments.len(), "images"), output);
    }

    let mut candidates = Vec::new();
    if let Some(object) = output
        .payload
        .as_ref()
        .and_then(serde_json::Value::as_object)
        && let Some(fragment) = tool_result_fragment(key.as_str(), object)
    {
        candidates.push(fragment);
    }

    if !output.metadata.is_empty() {
        let metadata = output
            .metadata
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect::<serde_json::Map<_, _>>();
        if let Some(fragment) = tool_result_fragment(key.as_str(), &metadata) {
            candidates.push(fragment);
        }
    }
    if let Some(object) = serde_json::from_str::<serde_json::Value>(output.text.trim())
        .ok()
        .and_then(|value| value.as_object().cloned())
        && let Some(fragment) = tool_result_fragment(key.as_str(), &object)
    {
        candidates.push(fragment);
    }
    if let Some(fragment) = tool_result_fragment_from_text(key.as_str(), output.text.as_str()) {
        candidates.push(fragment);
    }
    if is_provider_tool_identity(key.as_str()) && !output.text.trim().is_empty() {
        candidates.push("response received".to_owned());
    }
    if let Some(fragment) = choose_result_fragment(candidates) {
        return finalize_result_fragment(fragment, output);
    }
    finalize_result_fragment(result_title_fragment(output), output)
}

/// Pick the most useful result fact when an adapter split it across the
/// structured payload, metadata, and text channels. A bare lifecycle marker
/// such as `completed` must not beat a concrete fact such as `passed`, `HTTP
/// 200`, or `2 matches`; an explicit failure remains authoritative over any
/// partial success marker.
fn choose_result_fragment(candidates: Vec<String>) -> Option<String> {
    let mut best: Option<(u8, String)> = None;
    for candidate in candidates {
        let candidate = candidate.trim();
        if candidate.is_empty() {
            continue;
        }
        let candidate = candidate.to_owned();
        let strength = result_fragment_strength(candidate.as_str());
        if best
            .as_ref()
            .is_none_or(|(best_strength, _)| strength > *best_strength)
        {
            best = Some((strength, candidate));
        }
    }
    best.map(|(_, fragment)| fragment)
}

fn result_fragment_strength(fragment: &str) -> u8 {
    let lower = fragment.trim().to_ascii_lowercase();
    if lower == "failed"
        || lower.starts_with("failed ·")
        || lower == "cancelled"
        || lower == "timed out"
        || lower.contains("permission denied")
        || lower.contains("declined")
    {
        return 6;
    }
    if matches!(
        lower.as_str(),
        "completed" | "running" | "queued" | "response received" | "image response received"
    ) {
        return 1;
    }
    if lower.contains("passed")
        || lower.contains("updated")
        || lower.contains("created")
        || lower.contains("saved")
        || lower.contains("removed")
        || lower.contains("connected")
        || lower.contains("http ")
        || lower.chars().any(|character| character.is_ascii_digit())
    {
        return 4;
    }
    3
}

fn result_output_is_truncated(output: &RawOutput) -> bool {
    if output.truncated {
        return true;
    }
    let reports_truncated = |value: &serde_json::Value| {
        value
            .as_object()
            .and_then(|object| object.get("truncated"))
            .and_then(serde_json::Value::as_bool)
            == Some(true)
    };
    output.payload.as_ref().is_some_and(reports_truncated)
        || output
            .metadata
            .get("truncated")
            .is_some_and(|value| value.as_bool() == Some(true))
        || serde_json::from_str::<serde_json::Value>(output.text.trim())
            .ok()
            .is_some_and(|value| reports_truncated(&value))
}

fn finalize_result_fragment(mut fragment: String, output: &RawOutput) -> String {
    if fragment.trim().is_empty() {
        if !output.attachments.is_empty() {
            return format_count(output.attachments.len(), "attachments");
        }
        if !output.managed_outputs.is_empty() {
            return format_count(output.managed_outputs.len(), "outputs saved");
        }
        return fragment;
    }

    // A generic `completed` marker carries no result detail. Prefer a concrete
    // artifact count when that is all the output exposes.
    if result_fragment_strength(fragment.as_str()) == 1 {
        if !output.attachments.is_empty() {
            fragment = format_count(output.attachments.len(), "attachments");
        } else if !output.managed_outputs.is_empty() {
            fragment = format_count(output.managed_outputs.len(), "outputs saved");
        }
    }

    if result_output_is_truncated(output) && !fragment.to_ascii_lowercase().contains("truncated") {
        fragment.push_str(" · truncated");
    }
    fragment
}

fn result_title_fragment_for_invocation(invocation: &ToolInvocation, output: &RawOutput) -> String {
    result_title_fragment_for_tool(invocation.name.as_str(), output)
}

fn tool_result_fragment_from_text(key: &str, text: &str) -> Option<String> {
    let text = text.trim();
    let lower = text.to_ascii_lowercase();
    let specific = match key {
        "memory.get" if lower.starts_with("loaded ") => Some("loaded".to_owned()),
        "memory.write" if lower.starts_with("saved ") => Some("saved".to_owned()),
        "memory.delete" if lower.starts_with("deleted ") => Some("removed".to_owned()),
        "plan.clear" if lower.contains("cleared") => Some("cleared".to_owned()),
        "mcp.servers.status" if lower.starts_with("no mcp servers") => Some("0 servers".to_owned()),
        _ => None,
    };
    specific.or_else(|| discovery_text_result_fragment(key, text))
}

fn discovery_text_result_fragment(key: &str, text: &str) -> Option<String> {
    let is_discovery = matches!(
        key,
        "tools.list"
            | "tools.search"
            | "tools.tags"
            | "tools.plugins.list"
            | "tools.plugins.search"
            | "tools.plugins.tags"
            | "tools.plugins_list"
            | "tools.plugins_search"
            | "tools.plugins_tags"
            | "tools_list"
            | "tools_search"
            | "tools_tags"
    );
    if !is_discovery {
        return None;
    }
    let line = text.lines().map(str::trim).find(|line| !line.is_empty())?;
    let returned_marker = line.to_ascii_lowercase().find("returned ")?;
    let numbers = line[returned_marker + "returned ".len()..]
        .split_whitespace()
        .take(3)
        .filter_map(|value| {
            value
                .trim_end_matches(|character: char| !character.is_ascii_digit())
                .parse::<usize>()
                .ok()
        })
        .collect::<Vec<_>>();
    if numbers.len() < 2 {
        return None;
    }
    let label = if key.contains("plugins") {
        if key.ends_with("tags") || key.ends_with("_tags") {
            "tags"
        } else {
            "plugins"
        }
    } else if key.ends_with("tags") || key.ends_with("_tags") {
        "tags"
    } else {
        "tools"
    };
    let mut fragment = format!("{}/{} {label}", numbers[0], numbers[1]);
    if line.to_ascii_lowercase().contains("more available")
        || text.to_ascii_lowercase().contains("more available")
    {
        fragment.push_str(" · more available");
    }
    Some(fragment)
}

fn tool_result_fragment(
    key: &str,
    object: &serde_json::Map<String, serde_json::Value>,
) -> Option<String> {
    // Shell payloads carry both a lifecycle status and the more useful exit
    // code/process facts. Prefer the latter so `shell.run` ends as `passed`
    // or `failed · exit 1`, while list/logs/stop calls still report what they
    // actually returned.
    if matches!(
        key,
        "shell.run" | "shell.list" | "shell.logs" | "shell.stop" | "shell"
    ) && let Some(fragment) = shell_result_fragment(key, object)
    {
        return Some(fragment);
    }

    // A failure-like terminal state is authoritative. A successful
    // `status: completed` is intentionally deferred until after
    // tool-specific facts have been inspected: provider/web/MCP envelopes
    // commonly carry both `status: completed` and useful `results`,
    // `sources`, or `pending_calls`, and the latter makes a much better
    // headline. A bare `{status: completed}` still falls back to `completed`
    // at the end of this function.
    let terminal_status = object
        .get("status")
        .or_else(|| object.get("state"))
        .and_then(serde_json::Value::as_str)
        .and_then(normalize_result_status);
    if let Some(status) = terminal_status.as_deref()
        && matches!(status, "failed" | "cancelled" | "timed out")
    {
        return Some(status.to_owned());
    }
    match key {
        "code.search_ast" => {
            let matches = object.get("matches").and_then(array_len_or_u64);
            let scanned_files = object.get("scanned_files").and_then(value_as_u64);
            if matches.is_none() && scanned_files.is_none() {
                return None;
            }
            let matches = matches.unwrap_or_default();
            return Some(match scanned_files {
                Some(scanned_files) => format!(
                    "{} · {}",
                    format_count(matches as usize, "matches"),
                    format_count(scanned_files as usize, "files scanned")
                ),
                None => format_count(matches as usize, "matches"),
            });
        }
        "code.syntax_tree" => {
            let language = object
                .get("language")
                .and_then(serde_json::Value::as_str)
                .map(normalize_tool_title);
            let root = object
                .get("root_kind")
                .and_then(serde_json::Value::as_str)
                .map(normalize_tool_title);
            let parse_error =
                object.get("has_error").and_then(serde_json::Value::as_bool) == Some(true);
            let mut parts = language.into_iter().chain(root).collect::<Vec<_>>();
            if parse_error {
                parts.push("parse errors".to_owned());
            }
            if !parts.is_empty() {
                return Some(parts.join(" · "));
            }
        }
        "report.findings" => {
            if let Some(fragment) = findings_result_fragment(object) {
                return Some(fragment);
            }
        }
        "session.environment" => {
            let branch = object
                .get("git_branch")
                .and_then(serde_json::Value::as_str)
                .filter(|value| !value.trim().is_empty());
            let sha = object
                .get("git_short_sha")
                .and_then(serde_json::Value::as_str)
                .filter(|value| !value.trim().is_empty());
            if let Some(branch) = branch {
                let mut fragment = match sha {
                    Some(sha) => format!("{branch} @ {sha}"),
                    None => branch.to_owned(),
                };
                if object.get("git_dirty").and_then(serde_json::Value::as_bool) == Some(true) {
                    fragment.push_str(" · dirty");
                }
                return Some(fragment);
            }
        }
        "session.model" => {
            if let Some(fragment) = model_result_fragment(object) {
                return Some(fragment);
            }
        }
        "session.tokens" => {
            if let Some(fragment) = token_result_fragment(object) {
                return Some(fragment);
            }
        }
        "session.get" | "session.rename" => {
            if let Some(fragment) = session_result_fragment(object) {
                return Some(fragment);
            }
        }
        "interaction.ask" => {
            if let Some(fragment) = interaction_result_fragment(key, object) {
                return Some(fragment);
            }
        }
        "interaction.notify" => {
            if let Some(fragment) = interaction_result_fragment(key, object) {
                return Some(fragment);
            }
        }
        "lsp.servers" | "lsp.definition" | "lsp.references" | "lsp.hover" | "lsp.diagnostics" => {
            if let Some(fragment) = lsp_result_fragment(key, object) {
                return Some(fragment);
            }
        }
        "cron.create" | "cron.list" | "cron.delete" | "cron.update" | "cron.pause"
        | "cron.resume" | "cron.history" => {
            if let Some(fragment) = cron_result_fragment(key, object) {
                return Some(fragment);
            }
        }
        "monitor.start" | "monitor.stop" | "monitor" => {
            if let Some(fragment) = monitor_result_fragment(key, object) {
                return Some(fragment);
            }
        }
        "web.search" | "web.crawl" => {
            if let Some(fragment) = web_result_fragment(key, object) {
                return Some(fragment);
            }
        }
        value if is_provider_tool_identity(value) => {
            if let Some(fragment) = provider_result_fragment(value, object) {
                return Some(fragment);
            }
        }
        "mcp.resources.list"
        | "mcp.resources.templates.list"
        | "mcp.resources.read"
        | "mcp.prompts.list"
        | "mcp.prompts.get"
        | "mcp.tools.call"
        | "mcp.tools.search"
        | "mcp.servers.status"
        | "mcp.servers.reconnect" => {
            if let Some(fragment) = mcp_result_fragment(key, object) {
                return Some(fragment);
            }
        }
        "memory.search" | "memory.get" | "memory.list" | "memory.write" | "memory.delete" => {
            if let Some(fragment) = memory_result_fragment(key, object) {
                return Some(fragment);
            }
        }
        "plan.get" | "plan.set" | "plan.edit" | "plan.phase" | "plan.review" | "plan.clear" => {
            if let Some(fragment) = plan_result_fragment(object) {
                return Some(fragment);
            }
        }
        "tools.list"
        | "tools.search"
        | "tools.tags"
        | "tools.plugins.list"
        | "tools.plugins.search"
        | "tools.plugins.tags"
        | "tools.plugins_list"
        | "tools.plugins_search"
        | "tools.plugins_tags"
        | "tools_list"
        | "tools_search"
        | "tools_tags" => {
            if let Some(fragment) = discovery_result_fragment(key, object) {
                return Some(fragment);
            }
        }
        "tasks.run" | "tasks.get" | "tasks.list" | "tasks.cancel" | "tasks.followup"
        | "tasks.message" | "tasks.output" => {
            if let Some(fragment) = task_result_fragment(key, object) {
                return Some(fragment);
            }
        }
        "skills.create"
        | "skills.update"
        | "skills.delete"
        | "skills.get"
        | "skills.list"
        | "skills.read_resource"
        | "skills.refresh" => {
            if let Some(fragment) = skill_result_fragment(key, object) {
                return Some(fragment);
            }
        }
        "settings.get" | "settings.list" | "settings.inspect" => {
            if let Some(fragment) = settings_result_fragment(key, object) {
                return Some(fragment);
            }
        }
        "notebook.edit_cell" => {
            let action = object
                .get("action")
                .and_then(serde_json::Value::as_str)
                .map(|value| normalize_tool_title(value.replace('_', " ")));
            let cell_index = object
                .get("cell_index")
                .and_then(value_as_u64)
                .map(|value| format!("cell {value}"));
            let cell_count = object
                .get("cell_count")
                .and_then(value_as_u64)
                .map(|value| format_count(value as usize, "cells"));
            let parts = action
                .into_iter()
                .chain(cell_index)
                .chain(cell_count)
                .collect::<Vec<_>>();
            if !parts.is_empty() {
                return Some(parts.join(" · "));
            }
        }
        "snapshot.enter" => {
            let branch = object
                .get("branch")
                .and_then(serde_json::Value::as_str)
                .filter(|value| !value.trim().is_empty());
            let path = object
                .get("path")
                .and_then(serde_json::Value::as_str)
                .filter(|value| !value.trim().is_empty());
            if let Some(branch) = branch {
                return Some(format!("entered · {branch}"));
            }
            if path.is_some() {
                return Some("entered".to_owned());
            }
        }
        "snapshot.exit" => {
            if let Some(action) = object
                .get("action")
                .and_then(serde_json::Value::as_str)
                .filter(|value| !value.trim().is_empty())
            {
                return Some(normalize_tool_title(action.replace('_', " ")));
            }
            if object
                .get("path")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|value| !value.trim().is_empty())
            {
                return Some("exited".to_owned());
            }
        }
        "browser_open"
        | "web.browser_open"
        | "browser_snapshot"
        | "web.browser_snapshot"
        | "browser_click"
        | "web.browser_click"
        | "browser_type"
        | "web.browser_type"
        | "browser_wait"
        | "web.browser_wait" => {
            if let Some(fragment) = browser_result_fragment(key, object) {
                return Some(fragment);
            }
        }
        "browser_close" | "web.browser_close" => {
            if object.get("closed").and_then(serde_json::Value::as_bool) == Some(true) {
                return Some("closed".to_owned());
            }
        }
        "browser_shutdown" | "web.browser_shutdown" => {
            if let Some(closed) = object.get("closed").and_then(serde_json::Value::as_bool) {
                return Some(if closed { "closed" } else { "not running" }.to_owned());
            }
        }
        "fs.write" => {
            let kind = object.get("kind").and_then(serde_json::Value::as_str);
            let bytes = object.get("bytes").and_then(value_as_u64);
            return match (kind, bytes) {
                (Some(kind), Some(bytes)) => Some(format!(
                    "{} · {}",
                    normalize_result_status(kind).unwrap_or_else(|| normalize_tool_title(kind)),
                    format_bytes(bytes)
                )),
                (Some(kind), None) => {
                    normalize_result_status(kind).or_else(|| Some(normalize_tool_title(kind)))
                }
                _ => None,
            };
        }
        "fs.replace" => {
            if let Some(count) = object.get("replacements").and_then(value_as_u64) {
                return Some(format_count(count as usize, "replacements"));
            }
        }
        "fs.glob" => {
            let count = object.get("count").and_then(value_as_u64).or_else(|| {
                object
                    .get("paths")
                    .and_then(serde_json::Value::as_array)
                    .map(|paths| paths.len() as u64)
            });
            if let Some(count) = count {
                let mut fragment = format_count(count as usize, "matches");
                if object.get("truncated").and_then(serde_json::Value::as_bool) == Some(true) {
                    fragment.push_str(" · truncated");
                }
                return Some(fragment);
            }
        }
        "fs.grep" => {
            let count = object.get("matches").and_then(value_as_u64).or_else(|| {
                object
                    .get("results")
                    .and_then(serde_json::Value::as_array)
                    .map(|results| results.len() as u64)
            });
            if let Some(count) = count {
                let mut fragment = format_count(count as usize, "matches");
                if object.get("truncated").and_then(serde_json::Value::as_bool) == Some(true) {
                    fragment.push_str(" · truncated");
                }
                return Some(fragment);
            }
        }
        "fs.read" => {
            let count = object
                .get("loaded_paths")
                .and_then(serde_json::Value::as_array)
                .map(|paths| paths.len() as u64);
            let attachment = object
                .get("attachment")
                .and_then(serde_json::Value::as_object);
            if let Some(count) = count {
                let mut fragment = format_count(count as usize, "files loaded");
                if object.get("truncated").and_then(serde_json::Value::as_bool) == Some(true) {
                    fragment.push_str(" · truncated");
                }
                return Some(fragment);
            }
            if let Some(size) = attachment
                .and_then(|attachment| attachment.get("size_bytes"))
                .and_then(value_as_u64)
            {
                return Some(format!("loaded · {}", format_bytes(size)));
            }
        }
        "fs.read_many" => {
            if let Some(files) = object.get("files").and_then(serde_json::Value::as_array) {
                let count = format_count(files.len(), "files read");
                return Some(
                    if object.get("truncated").and_then(serde_json::Value::as_bool) == Some(true) {
                        format!("{count} · truncated")
                    } else {
                        count
                    },
                );
            }
        }
        "fs.stat" => {
            let kind = object.get("kind").and_then(serde_json::Value::as_str);
            let size = object.get("size").and_then(value_as_u64);
            if let Some(kind) = kind {
                return Some(match size {
                    Some(size) => {
                        format!("{} · {}", normalize_tool_title(kind), format_bytes(size))
                    }
                    None => normalize_tool_title(kind),
                });
            }
        }
        "fs.view_image"
        | "openai.image_generation"
        | "openai.image_edit"
        | "chatgpt.image_generation"
        | "chatgpt.image_edit"
        | "gemini.image_generation"
        | "gemini.image_edit" => {
            if let Some(bytes) = object.get("size_bytes").and_then(value_as_u64) {
                return Some(format!("image · {}", format_bytes(bytes)));
            }
            if let Some(mime) = object.get("mime").and_then(serde_json::Value::as_str) {
                return Some(normalize_tool_title(mime));
            }
        }
        "web.fetch" | "web_fetch" | "claude.web_fetch" => {
            if let Some(status) = object.get("status").and_then(value_as_u64) {
                return Some(format!("HTTP {status}"));
            }
        }
        "settings.validate" => {
            if let Some(fragment) = settings_result_fragment(key, object) {
                return Some(fragment);
            }
        }
        "settings.set" | "settings.patch" | "settings.delete" => {
            if let Some(fragment) = settings_result_fragment(key, object) {
                return Some(fragment);
            }
        }
        "snapshot.status" | "repo.status" => {
            if let Some(changes) = object.get("changes").and_then(serde_json::Value::as_array) {
                let mut parts = vec![format_count(changes.len(), "files changed")];
                if object.get("dirty").and_then(serde_json::Value::as_bool) == Some(true) {
                    parts.push("dirty".to_owned());
                }
                return Some(parts.join(" · "));
            }
            if let Some(snapshots) = object
                .get("snapshots")
                .and_then(serde_json::Value::as_array)
            {
                return Some(format_count(snapshots.len(), "snapshots"));
            }
        }
        "browser_list" | "web.browser_list" => {
            if let Some(sessions) = object.get("sessions").and_then(serde_json::Value::as_array) {
                return Some(format_count(sessions.len(), "pages"));
            }
        }
        "browser_screenshot"
        | "web.browser_screenshot"
        | "browser_download"
        | "web.browser_download" => {
            if let Some(bytes) = object.get("size_bytes").and_then(value_as_u64) {
                return Some(format!("saved · {}", format_bytes(bytes)));
            }
        }
        _ => {}
    }

    if let Some(calls) = first_array(object, &["pending_calls", "pending_actions", "tool_calls"])
        && !calls.is_empty()
    {
        return Some(format_count(calls.len(), "pending calls"));
    }
    if object
        .get("continuation_required")
        .and_then(serde_json::Value::as_bool)
        == Some(true)
    {
        return Some("continuation required".to_owned());
    }
    if let Some(sources) = object.get("sources").and_then(serde_json::Value::as_array) {
        return Some(format_count(sources.len(), "sources"));
    }
    terminal_status
}

fn shell_result_fragment(
    key: &str,
    object: &serde_json::Map<String, serde_json::Value>,
) -> Option<String> {
    match key {
        "shell.list" => object
            .get("processes")
            .and_then(serde_json::Value::as_array)
            .map(|processes| format_count(processes.len(), "processes")),
        "shell.logs" => object
            .get("events")
            .and_then(serde_json::Value::as_array)
            .map(|events| {
                let mut fragment = format_count(events.len(), "events");
                if object.get("has_more").and_then(serde_json::Value::as_bool) == Some(true) {
                    fragment.push_str(" · more available");
                }
                fragment
            }),
        "shell.stop" => {
            if let Some(stopped) = object
                .get("stopped")
                .or_else(|| object.get("removed"))
                .and_then(serde_json::Value::as_bool)
            {
                return Some(if stopped {
                    "stopped".to_owned()
                } else {
                    "not found".to_owned()
                });
            }
            object
                .get("status")
                .or_else(|| object.get("state"))
                .and_then(serde_json::Value::as_str)
                .and_then(normalize_result_status)
                .or_else(|| Some("stopped".to_owned()))
        }
        _ => {
            if let Some(exit_code) = object.get("exit_code").and_then(value_as_i64) {
                return Some(if exit_code == 0 {
                    "passed".to_owned()
                } else {
                    format!("failed · exit {exit_code}")
                });
            }
            object
                .get("status")
                .or_else(|| object.get("state"))
                .and_then(serde_json::Value::as_str)
                .and_then(normalize_result_status)
        }
    }
}

fn is_provider_tool_identity(key: &str) -> bool {
    key.starts_with("chatgpt.")
        || key.starts_with("claude.")
        || key.starts_with("gemini.")
        || key.starts_with("openai.")
}

fn provider_result_fragment(
    key: &str,
    object: &serde_json::Map<String, serde_json::Value>,
) -> Option<String> {
    let pending = first_array(object, &["pending_calls", "pending_actions", "tool_calls"])
        .filter(|calls| !calls.is_empty())
        .map(|calls| format_count(calls.len(), "pending calls"));
    if let Some(pending) = pending {
        return Some(pending);
    }
    if object
        .get("continuation_required")
        .and_then(serde_json::Value::as_bool)
        == Some(true)
    {
        return Some("continuation required".to_owned());
    }

    // Provider envelopes intentionally share one outer shape, but the useful
    // completion fact is tool-specific. Keep that fact compact here instead
    // of making every provider look like the same generic "response
    // received": file search should report files, maps should report places,
    // code execution should report its outcome, and a computer call should
    // report the action/page it reached.
    if let Some(fragment) = provider_operation_result_fragment(key, object) {
        return Some(fragment);
    }

    let sources = object
        .get("sources")
        .and_then(serde_json::Value::as_array)
        .filter(|sources| !sources.is_empty())
        .map(|sources| format_count(sources.len(), "sources"));
    if let Some(sources) = sources {
        return Some(sources);
    }

    // Provider payloads deliberately keep the full response in the raw text
    // channel and store only a bounded receipt/assistant-content envelope in
    // the structured payload. A stable fact is more useful in a collapsed row
    // than repeating the first sentence of an arbitrarily long answer.
    let response_received = object
        .get("response_id")
        .filter(|value| !value.is_null())
        .is_some()
        || object
            .get("assistant_content")
            .filter(|value| !value.is_null())
            .is_some()
        || object
            .get("response_receipt")
            .filter(|value| !value.is_null())
            .is_some();
    if response_received {
        return Some(if key.contains("image") {
            "image response received".to_owned()
        } else {
            "response received".to_owned()
        });
    }
    None
}

fn provider_operation_result_fragment(
    key: &str,
    object: &serde_json::Map<String, serde_json::Value>,
) -> Option<String> {
    let count = |keys: &[&str], label: &str| {
        provider_count_fact(&serde_json::Value::Object(object.clone()), keys, label)
    };

    match key {
        "chatgpt.file_search" | "claude.file_search" | "gemini.file_search" => count(
            &[
                "file_results",
                "file_search_results",
                "files",
                "documents",
                "matches",
                "results",
            ],
            "files",
        )
        .or_else(|| {
            count(
                &[
                    "file_count",
                    "retrieved_count",
                    "match_count",
                    "result_count",
                ],
                "files",
            )
        })
        .or_else(|| count(&["sources"], "files")),
        "chatgpt.tool_search"
        | "claude.tool_search_bm25"
        | "claude.tool_search_regex"
        | "claude.tool_search_tool_bm25"
        | "claude.tool_search_tool_regex" => {
            count(&["tool_references", "tools", "matches", "results"], "tools")
                .or_else(|| count(&["tool_count", "result_count", "total"], "tools"))
        }
        "gemini.google_maps" => count(
            &[
                "places",
                "map_results",
                "locations",
                "results",
                "groundingChunks",
            ],
            "places",
        )
        .or_else(|| count(&["place_count", "result_count", "total"], "places"))
        .or_else(|| count(&["sources"], "places")),
        "gemini.retrieval" => count(
            &[
                "retrieved",
                "matches",
                "chunks",
                "documents",
                "results",
                "groundingChunks",
            ],
            "matches",
        )
        .or_else(|| {
            count(
                &[
                    "retrieved_count",
                    "match_count",
                    "chunk_count",
                    "result_count",
                ],
                "matches",
            )
        })
        .or_else(|| count(&["sources"], "matches")),
        "gemini.url_context" | "claude.web_fetch" => {
            if let Some(status) = provider_find_value(
                &serde_json::Value::Object(object.clone()),
                &["status", "http_status", "status_code"],
            )
            .and_then(value_as_u64)
            {
                return Some(format!("HTTP {status}"));
            }
            count(
                &["fetched_urls", "loaded_urls", "pages", "documents", "urls"],
                "pages",
            )
            .or_else(|| count(&["fetched_count", "loaded_count", "page_count"], "pages"))
            .or_else(|| count(&["sources"], "pages"))
        }
        "chatgpt.code_interpreter" | "claude.code_execution" | "gemini.code_execution" => {
            provider_execution_result_fragment(object)
        }
        "chatgpt.computer"
        | "chatgpt.computer_use_preview"
        | "claude.computer"
        | "gemini.computer_use" => provider_computer_result_fragment(object),
        "gemini.mcp_server" | "claude.mcp_toolset" | "chatgpt.mcp" => {
            provider_connection_result_fragment(object)
        }
        "claude.memory" => provider_memory_result_fragment(object),
        "claude.text_editor" | "claude.str_replace_based_edit_tool" => {
            let root = serde_json::Value::Object(object.clone());
            if provider_find_value(&root, &["changed"]).and_then(serde_json::Value::as_bool)
                == Some(true)
            {
                if let Some(replacements) =
                    provider_find_value(&root, &["replacements"]).and_then(value_as_u64)
                {
                    return Some(format!(
                        "updated · {}",
                        format_count(replacements as usize, "replacements")
                    ));
                }
                return Some("updated".to_owned());
            }
            None
        }
        "chatgpt.apply_patch" => {
            let root = serde_json::Value::Object(object.clone());
            provider_count_fact(&root, &["changes", "files"], "files changed").or_else(|| {
                provider_find_value(&root, &["changed", "updated"])
                    .and_then(serde_json::Value::as_bool)
                    .filter(|changed| *changed)
                    .map(|_| "updated".to_owned())
            })
        }
        "claude.advisor" => {
            if provider_find_value(
                &serde_json::Value::Object(object.clone()),
                &["error", "errors"],
            )
            .is_some_and(|value| !value.is_null())
            {
                Some("error".to_owned())
            } else {
                Some("response received".to_owned())
            }
        }
        "chatgpt.local_shell" | "chatgpt.shell" | "claude.bash" => {
            provider_execution_result_fragment(object)
        }
        "chatgpt.image_generation"
        | "chatgpt.image_edit"
        | "gemini.image_generation"
        | "gemini.image_edit" => count(&["images", "outputs", "image_count"], "images"),
        _ => None,
    }
}

fn provider_execution_result_fragment(
    object: &serde_json::Map<String, serde_json::Value>,
) -> Option<String> {
    let root = serde_json::Value::Object(object.clone());
    if let Some(exit_code) =
        provider_find_value(&root, &["exit_code", "exitCode"]).and_then(value_as_i64)
    {
        return Some(if exit_code == 0 {
            "passed".to_owned()
        } else {
            format!("failed · exit {exit_code}")
        });
    }
    if provider_find_value(&root, &["error", "errors"]).is_some_and(|value| !value.is_null()) {
        return Some("failed".to_owned());
    }
    if let Some(outcome) = provider_find_value(&root, &["outcome", "result_status"])
        .and_then(serde_json::Value::as_str)
        .and_then(normalize_result_status)
    {
        return Some(if outcome == "completed" {
            "passed".to_owned()
        } else {
            outcome
        });
    }
    provider_find_value(&root, &["outputs", "results", "output_count"]).and_then(|value| {
        value
            .as_array()
            .map(|values| format_count(values.len(), "outputs"))
            .or_else(|| value_as_u64(value).map(|count| format_count(count as usize, "outputs")))
    })
}

fn provider_computer_result_fragment(
    object: &serde_json::Map<String, serde_json::Value>,
) -> Option<String> {
    let root = serde_json::Value::Object(object.clone());
    let action = provider_find_value(&root, &["action", "action_type", "actionType"])
        .and_then(|value| match value {
            serde_json::Value::Object(object) => object
                .get("type")
                .or_else(|| object.get("action"))
                .and_then(serde_json::Value::as_str),
            serde_json::Value::String(value) => Some(value.as_str()),
            _ => None,
        })
        .map(|value| normalize_tool_title(value.replace('_', " ")));
    let page = provider_find_value(&root, &["page_title", "page", "title"])
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(normalize_tool_title);
    let mut parts = action.into_iter().chain(page).collect::<Vec<_>>();
    if parts.is_empty()
        && let Some(actions) =
            provider_find_value(&root, &["actions"]).and_then(serde_json::Value::as_array)
    {
        parts.push(format_count(actions.len(), "actions"));
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
}

fn provider_connection_result_fragment(
    object: &serde_json::Map<String, serde_json::Value>,
) -> Option<String> {
    let root = serde_json::Value::Object(object.clone());
    if let Some(connected) =
        provider_find_value(&root, &["connected"]).and_then(serde_json::Value::as_bool)
    {
        return Some(if connected {
            "connected".to_owned()
        } else {
            "disconnected".to_owned()
        });
    }
    provider_find_value(&root, &["connection_status", "status"])
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(|value| match value.to_ascii_lowercase().as_str() {
            "connected" | "ready" | "ok" | "success" => "connected".to_owned(),
            "disconnected" | "failed" | "error" => "disconnected".to_owned(),
            _ => normalize_tool_title(value.replace('_', " ")),
        })
}

fn provider_memory_result_fragment(
    object: &serde_json::Map<String, serde_json::Value>,
) -> Option<String> {
    let root = serde_json::Value::Object(object.clone());
    for (keys, label) in [
        (&["removed", "deleted"][..], "removed"),
        (&["saved", "written", "created"][..], "saved"),
        (&["loaded", "retrieved"][..], "loaded"),
    ] {
        if provider_find_value(&root, keys).and_then(serde_json::Value::as_bool) == Some(true) {
            return Some(label.to_owned());
        }
    }
    provider_find_value(&root, &["operation", "action", "command"])
        .and_then(serde_json::Value::as_str)
        .and_then(|value| {
            let value = value.to_ascii_lowercase();
            if value.contains("delete") || value.contains("remove") {
                Some("removed".to_owned())
            } else if value.contains("write") || value.contains("save") || value.contains("create")
            {
                Some("saved".to_owned())
            } else if value.contains("read") || value.contains("view") || value.contains("load") {
                Some("loaded".to_owned())
            } else {
                None
            }
        })
}

fn provider_count_fact(root: &serde_json::Value, keys: &[&str], label: &str) -> Option<String> {
    let value = provider_find_value(root, keys)?;
    let count = value
        .as_array()
        .map(Vec::len)
        .or_else(|| value_as_u64(value).map(|value| value as usize))?;
    Some(format_count(count, label))
}

fn provider_find_value<'a>(
    value: &'a serde_json::Value,
    keys: &[&str],
) -> Option<&'a serde_json::Value> {
    fn visit<'a>(
        value: &'a serde_json::Value,
        keys: &[&str],
        depth: usize,
    ) -> Option<&'a serde_json::Value> {
        if depth > 6 {
            return None;
        }
        match value {
            serde_json::Value::Object(object) => {
                for key in keys {
                    if let Some(value) = object.get(*key) {
                        return Some(value);
                    }
                }
                for (key, child) in object {
                    if matches!(key.as_str(), "provider_raw" | "raw" | "trace") {
                        continue;
                    }
                    if let Some(value) = visit(child, keys, depth + 1) {
                        return Some(value);
                    }
                }
            }
            serde_json::Value::Array(values) => {
                for child in values {
                    if let Some(value) = visit(child, keys, depth + 1) {
                        return Some(value);
                    }
                }
            }
            _ => {}
        }
        None
    }
    visit(value, keys, 0)
}

fn array_len_or_u64(value: &serde_json::Value) -> Option<u64> {
    value
        .as_array()
        .map(|values| values.len() as u64)
        .or_else(|| value_as_u64(value))
}

fn findings_result_fragment(object: &serde_json::Map<String, serde_json::Value>) -> Option<String> {
    let total = object
        .get("findings")
        .and_then(|value| value.as_array().map(|values| values.len() as u64))
        .or_else(|| object.get("finding_count").and_then(value_as_u64))
        .or_else(|| {
            object
                .get("counts")
                .and_then(serde_json::Value::as_object)
                .map(|counts| counts.values().filter_map(value_as_u64).sum::<u64>())
        });
    let Some(total) = total else {
        return None;
    };
    let mut parts = vec![format_count(total as usize, "findings")];
    if let Some(counts) = object.get("counts").and_then(serde_json::Value::as_object) {
        for severity in ["critical", "high", "medium", "low", "info"] {
            if let Some(count) = counts.get(severity).and_then(value_as_u64)
                && count > 0
            {
                parts.push(format_count(count as usize, severity));
            }
        }
    }
    Some(parts.join(" · "))
}

fn model_result_fragment(object: &serde_json::Map<String, serde_json::Value>) -> Option<String> {
    let values = ["model_provider_id", "model_adapter_id", "model_id"]
        .iter()
        .filter_map(|key| {
            object
                .get(*key)
                .and_then(serde_json::Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .map(ToOwned::to_owned)
        })
        .collect::<Vec<_>>();
    if !values.is_empty() {
        return Some(values.join("/"));
    }
    object
        .get("model")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(ToOwned::to_owned)
}

fn token_result_fragment(object: &serde_json::Map<String, serde_json::Value>) -> Option<String> {
    let current = object.get("current_tokens").and_then(value_as_u64);
    let remaining = object.get("remaining_tokens").and_then(value_as_u64);
    match (current, remaining) {
        (Some(current), Some(remaining)) => Some(format!("{current} used · {remaining} remaining")),
        (Some(current), None) => Some(format!("{current} used")),
        (None, Some(remaining)) => Some(format!("{remaining} remaining")),
        (None, None) => None,
    }
}

fn session_result_fragment(object: &serde_json::Map<String, serde_json::Value>) -> Option<String> {
    let session = object
        .get("session")
        .and_then(serde_json::Value::as_object)
        .unwrap_or(object);
    let id = session
        .get("id")
        .or_else(|| session.get("session_id"))
        .and_then(title_value_text);
    let title = session
        .get("title")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(normalize_tool_title);
    match (id, title) {
        (Some(id), Some(title)) => Some(format!("#{id} · {title}")),
        (Some(id), None) => Some(format!("#{id}")),
        (None, Some(title)) => Some(title),
        (None, None) => None,
    }
}

fn task_result_fragment(
    key: &str,
    object: &serde_json::Map<String, serde_json::Value>,
) -> Option<String> {
    if object.get("timed_out").and_then(serde_json::Value::as_bool) == Some(true) {
        return Some("timed out".to_owned());
    }

    if let Some(tasks) = object.get("tasks").and_then(serde_json::Value::as_array) {
        if tasks.len() == 1
            && let Some(task) = tasks.first().and_then(serde_json::Value::as_object)
            && let Some(status) = task_status(task)
        {
            return Some(task_action_result_fragment(key, Some(status.as_str())));
        }
        let mut counts = BTreeMap::<String, usize>::new();
        for task in tasks.iter().filter_map(serde_json::Value::as_object) {
            if let Some(status) = task_status(task) {
                *counts.entry(status).or_default() += 1;
            }
        }
        let mut parts = vec![format_count(tasks.len(), "tasks")];
        for (status, count) in counts {
            parts.push(format_count(count, status.as_str()));
        }
        return Some(parts.join(" · "));
    }

    let task = object.get("task").and_then(serde_json::Value::as_object);
    let status = task.and_then(task_status).or_else(|| {
        object
            .get("status")
            .and_then(serde_json::Value::as_str)
            .and_then(normalize_result_status)
    });
    let chunks = object
        .get("chunks")
        .and_then(serde_json::Value::as_array)
        .map(|chunks| format_count(chunks.len(), "chunks"));
    let more = object.get("has_more").and_then(serde_json::Value::as_bool) == Some(true);
    let mut parts = Vec::new();
    if let Some(chunks) = chunks {
        parts.push(chunks);
    }
    if let Some(status) = status {
        parts.push(task_action_result_fragment(key, Some(status.as_str())));
    } else if matches!(key, "tasks.cancel" | "tasks.message" | "tasks.followup") {
        parts.push(task_action_result_fragment(key, None));
    }
    if more {
        parts.push("more available".to_owned());
    }
    if !parts.is_empty() {
        return Some(parts.join(" · "));
    }
    None
}

fn task_action_result_fragment(key: &str, status: Option<&str>) -> String {
    let status = status.map(str::trim).filter(|value| !value.is_empty());
    match key {
        "tasks.cancel" => match status {
            Some("cancelling") | Some("running") | Some("created") => {
                "cancellation requested".to_owned()
            }
            Some("cancelled") => "cancelled".to_owned(),
            Some(status)
                if matches!(status, "completed" | "failed" | "timed out" | "interrupted") =>
            {
                format!("already {status}")
            }
            Some(status) => format!("cancellation requested · {status}"),
            None => "cancellation requested".to_owned(),
        },
        "tasks.message" => status
            .map(|status| format!("message sent · {status}"))
            .unwrap_or_else(|| "message sent".to_owned()),
        "tasks.followup" => status
            .map(|status| format!("follow-up started · {status}"))
            .unwrap_or_else(|| "follow-up started".to_owned()),
        "tasks.run" => match status {
            Some("running") | Some("created") => "started · running".to_owned(),
            Some(status) => status.to_owned(),
            None => "started".to_owned(),
        },
        _ => status.unwrap_or("completed").to_owned(),
    }
}

fn task_status(object: &serde_json::Map<String, serde_json::Value>) -> Option<String> {
    object
        .get("status")
        .or_else(|| object.get("state"))
        .and_then(serde_json::Value::as_str)
        .and_then(normalize_result_status)
}

fn skill_result_fragment(
    key: &str,
    object: &serde_json::Map<String, serde_json::Value>,
) -> Option<String> {
    match key {
        "skills.create" | "skills.update" | "skills.delete" => {
            let operation = object
                .get("operation")
                .and_then(serde_json::Value::as_str)
                .map(|value| match value {
                    "deleted" | "removed" => "removed".to_owned(),
                    value => normalize_tool_title(value),
                });
            let name = object
                .get("name")
                .and_then(serde_json::Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .map(normalize_tool_title);
            let parts = operation.into_iter().chain(name).collect::<Vec<_>>();
            if !parts.is_empty() {
                return Some(parts.join(" · "));
            }
        }
        "skills.list" => {
            let returned = object.get("returned").and_then(value_as_u64).or_else(|| {
                object
                    .get("tools")
                    .and_then(|value| value.as_array().map(|v| v.len() as u64))
            });
            if let Some(returned) = returned {
                let total = object.get("total").and_then(value_as_u64);
                return Some(match total {
                    Some(total) => format!("{returned} of {total} tools"),
                    None => format_count(returned as usize, "tools"),
                });
            }
        }
        "skills.get" => {
            let name = object.get("name").and_then(serde_json::Value::as_str);
            let kind = object.get("kind").and_then(serde_json::Value::as_str);
            let parts = name
                .into_iter()
                .chain(kind)
                .map(normalize_tool_title)
                .collect::<Vec<_>>();
            if !parts.is_empty() {
                return Some(parts.join(" · "));
            }
        }
        "skills.read_resource" => {
            let path = object.get("path").and_then(serde_json::Value::as_str);
            let bytes = object.get("bytes").and_then(value_as_u64);
            let parts = path
                .into_iter()
                .map(normalize_tool_title)
                .chain(bytes.map(|value| format_bytes(value)))
                .collect::<Vec<_>>();
            if !parts.is_empty() {
                return Some(parts.join(" · "));
            }
        }
        "skills.refresh" => {
            let changed = object
                .get("changed")
                .and_then(serde_json::Value::as_bool)
                .map(|changed| if changed { "changed" } else { "unchanged" });
            let generation = object
                .get("generation")
                .and_then(value_as_u64)
                .map(|value| format!("catalog {value}"));
            let parts = changed
                .into_iter()
                .map(ToOwned::to_owned)
                .chain(generation)
                .collect::<Vec<_>>();
            if !parts.is_empty() {
                return Some(parts.join(" · "));
            }
        }
        _ => {}
    }
    None
}

fn settings_result_fragment(
    key: &str,
    object: &serde_json::Map<String, serde_json::Value>,
) -> Option<String> {
    let path = object
        .get("path")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(normalize_tool_title);
    match key {
        "settings.get" => {
            let source = object
                .get("source")
                .or_else(|| object.get("layer"))
                .and_then(serde_json::Value::as_str)
                .map(normalize_tool_title);
            // The path is already part of the call-time title. Only append
            // the resolved source here; repeating the path makes a completed
            // row look like `Read setting path · path · workspace`.
            let parts = source.or_else(|| path.map(|_| "read".to_owned()));
            if let Some(parts) = parts {
                return Some(parts);
            }
        }
        "settings.list" => {
            let count = object
                .get("items")
                .and_then(|value| value.as_array().map(|values| values.len() as u64))
                .or_else(|| object.get("count").and_then(value_as_u64));
            if let Some(count) = count {
                return Some(format_count(count as usize, "settings"));
            }
        }
        "settings.inspect" => {
            if path.is_some() {
                // The inspected path is part of the initial title. The
                // completion fact only needs to say that the inspection
                // succeeded.
                return Some("inspected".to_owned());
            }
        }
        "settings.set" | "settings.delete" | "settings.patch" => {
            let changed = object.get("changed").and_then(serde_json::Value::as_bool);
            let deleted = object.get("deleted").and_then(serde_json::Value::as_bool);
            let operation = if key == "settings.delete" && deleted == Some(true) {
                Some("removed".to_owned())
            } else {
                changed.map(|value| if value { "updated" } else { "unchanged" }.to_owned())
            };
            let dry_run = object.get("dry_run").and_then(serde_json::Value::as_bool) == Some(true);
            let reload = object
                .get("reload_required")
                .and_then(serde_json::Value::as_bool)
                == Some(true);
            let updated_paths = object
                .get("updated_paths")
                .and_then(|value| value.as_array().map(|values| values.len() as u64))
                .map(|value| format_count(value as usize, "settings updated"));
            let mut parts = Vec::new();
            if dry_run {
                parts.push("preview".to_owned());
            }
            if let Some(operation) = operation {
                parts.push(operation);
            }
            if let Some(updated_paths) = updated_paths {
                parts.push(updated_paths);
            }
            if reload {
                parts.push("reload required".to_owned());
            }
            if !parts.is_empty() {
                return Some(parts.join(" · "));
            }
        }
        "settings.validate" => {
            if let Some(valid) = object.get("valid").and_then(serde_json::Value::as_bool) {
                let result = if valid { "valid" } else { "invalid" };
                if let Some(warnings) = object.get("warnings").and_then(serde_json::Value::as_array)
                    && !warnings.is_empty()
                {
                    return Some(format!(
                        "{result} · {}",
                        format_count(warnings.len(), "warnings")
                    ));
                }
                return Some(result.to_owned());
            }
        }
        _ => {}
    }
    None
}

fn mcp_result_fragment(
    key: &str,
    object: &serde_json::Map<String, serde_json::Value>,
) -> Option<String> {
    let mut parts = Vec::new();
    match key {
        "mcp.resources.list" => {
            if let Some(resources) = object
                .get("resources")
                .and_then(serde_json::Value::as_array)
            {
                parts.push(format_count(resources.len(), "resources"));
            }
        }
        "mcp.resources.templates.list" => {
            if let Some(templates) = object
                .get("resource_templates")
                .and_then(serde_json::Value::as_array)
            {
                parts.push(format_count(templates.len(), "resource templates"));
            }
        }
        "mcp.resources.read" => {
            if let Some(contents) = object.get("contents").and_then(serde_json::Value::as_array) {
                parts.push(format_count(contents.len(), "content blocks"));
            }
        }
        "mcp.prompts.list" => {
            if let Some(prompts) = object.get("prompts").and_then(serde_json::Value::as_array) {
                parts.push(format_count(prompts.len(), "prompts"));
            }
        }
        "mcp.prompts.get" => {
            if let Some(messages) = object.get("messages").and_then(serde_json::Value::as_array) {
                parts.push(format_count(messages.len(), "messages"));
            }
        }
        "mcp.tools.call" => {
            if let Some(content) = object.get("content").and_then(serde_json::Value::as_array) {
                parts.push(format_count(content.len(), "content blocks"));
            }
            if object
                .get("structured_content")
                .is_some_and(|value| !value.is_null())
            {
                parts.push("structured result".to_owned());
            }
        }
        "mcp.tools.search" => {
            if let Some(results) = object.get("results").and_then(serde_json::Value::as_array) {
                parts.push(format_count(results.len(), "tools"));
            }
        }
        "mcp.servers.status" => {
            if let Some(servers) = object.get("servers").and_then(serde_json::Value::as_array) {
                let connected = servers
                    .iter()
                    .filter(|server| {
                        server.get("connected").and_then(serde_json::Value::as_bool) == Some(true)
                    })
                    .count();
                let tools = servers
                    .iter()
                    .filter_map(|server| server.get("tool_count").and_then(value_as_u64))
                    .sum::<u64>();
                parts.push(format!("{connected}/{} connected", servers.len()));
                parts.push(format_count(tools as usize, "tools"));
            }
        }
        "mcp.servers.reconnect" => {
            if let Some(connected) = object.get("connected").and_then(serde_json::Value::as_bool) {
                parts.push(if connected {
                    "connected".to_owned()
                } else {
                    "disconnected".to_owned()
                });
            }
            if let Some(tool_count) = object.get("tool_count").and_then(value_as_u64) {
                parts.push(format_count(tool_count as usize, "tools"));
            }
        }
        _ => {}
    }
    append_cursor_fact(&mut parts, object);
    (!parts.is_empty()).then(|| parts.join(" · "))
}

fn memory_result_fragment(
    key: &str,
    object: &serde_json::Map<String, serde_json::Value>,
) -> Option<String> {
    match key {
        "memory.search" => object
            .get("results")
            .or_else(|| object.get("matches"))
            .and_then(serde_json::Value::as_array)
            .map(|results| format_count(results.len(), "matches")),
        "memory.list" => object
            .get("memories")
            .and_then(serde_json::Value::as_array)
            .map(|memories| format_count(memories.len(), "records")),
        "memory.get" => object
            .get("name")
            .and_then(serde_json::Value::as_str)
            .filter(|name| !name.trim().is_empty())
            .map(|_| {
                if object.get("found").and_then(serde_json::Value::as_bool) == Some(false) {
                    "not found".to_owned()
                } else {
                    "loaded".to_owned()
                }
            }),
        "memory.write" => {
            let saved = object
                .get("saved")
                .or_else(|| object.get("written"))
                .or_else(|| object.get("created"))
                .and_then(serde_json::Value::as_bool);
            saved.map(|saved| {
                if saved {
                    "saved".to_owned()
                } else {
                    "not saved".to_owned()
                }
            })
        }
        "memory.delete" => object
            .get("removed")
            .or_else(|| object.get("deleted"))
            .and_then(serde_json::Value::as_bool)
            .map(|removed| {
                if removed {
                    "removed".to_owned()
                } else {
                    "not found".to_owned()
                }
            }),
        _ => None,
    }
}

fn interaction_result_fragment(
    key: &str,
    object: &serde_json::Map<String, serde_json::Value>,
) -> Option<String> {
    match key {
        "interaction.ask" => {
            let timed_out =
                object.get("timed_out").and_then(serde_json::Value::as_bool) == Some(true);
            let answered = object
                .get("answers")
                .and_then(serde_json::Value::as_object)
                .map(|answers| {
                    answers
                        .values()
                        .filter(|value| match value {
                            serde_json::Value::Array(values) => !values.is_empty(),
                            serde_json::Value::String(value) => !value.trim().is_empty(),
                            serde_json::Value::Null => false,
                            _ => true,
                        })
                        .count()
                });
            let questions = object
                .get("questions")
                .and_then(serde_json::Value::as_array)
                .map(Vec::len);
            let mut parts = Vec::new();
            if let Some(answered) = answered {
                parts.push(format_count(answered, "answered"));
            } else if let Some(questions) = questions {
                parts.push(format_count(questions, "questions"));
            }
            if timed_out {
                parts.push("timed out".to_owned());
            }
            (!parts.is_empty()).then(|| parts.join(" · "))
        }
        "interaction.notify" => {
            let level = object
                .get("level")
                .and_then(serde_json::Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .map(|value| normalize_tool_title(value.replace('_', " ")));
            let title = object
                .get("title")
                .and_then(serde_json::Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .map(normalize_tool_title);
            let parts = level.into_iter().chain(title).collect::<Vec<_>>();
            (!parts.is_empty()).then(|| parts.join(" · "))
        }
        _ => None,
    }
}

fn lsp_result_fragment(
    key: &str,
    object: &serde_json::Map<String, serde_json::Value>,
) -> Option<String> {
    match key {
        "lsp.servers" => object
            .get("servers")
            .and_then(serde_json::Value::as_array)
            .map(|servers| format_count(servers.len(), "servers")),
        "lsp.definition" => object
            .get("locations")
            .and_then(serde_json::Value::as_array)
            .map(|locations| format_count(locations.len(), "definitions")),
        "lsp.references" => object
            .get("locations")
            .and_then(serde_json::Value::as_array)
            .map(|locations| format_count(locations.len(), "references")),
        "lsp.hover" => object
            .get("contents")
            .filter(|value| match value {
                serde_json::Value::String(value) => !value.trim().is_empty(),
                serde_json::Value::Null => false,
                _ => true,
            })
            .map(|_| "hover returned".to_owned())
            .or_else(|| Some("no hover information".to_owned())),
        "lsp.diagnostics" => object
            .get("entries")
            .and_then(serde_json::Value::as_array)
            .map(|entries| format_count(entries.len(), "diagnostics")),
        _ => None,
    }
}

fn cron_result_fragment(
    key: &str,
    object: &serde_json::Map<String, serde_json::Value>,
) -> Option<String> {
    match key {
        "cron.create" => {
            let mut parts = vec!["created".to_owned()];
            if let Some(next) = object
                .get("next_fire_at")
                .and_then(serde_json::Value::as_str)
                .filter(|value| !value.trim().is_empty())
            {
                parts.push(format!("next run {}", compact_result_timestamp(next)));
            }
            Some(parts.join(" · "))
        }
        "cron.list" => object
            .get("jobs")
            .and_then(serde_json::Value::as_array)
            .map(|jobs| format_count(jobs.len(), "schedules")),
        "cron.delete" => object
            .get("removed")
            .and_then(serde_json::Value::as_bool)
            .map(|removed| if removed { "removed" } else { "not found" }.to_owned()),
        "cron.update" | "cron.pause" | "cron.resume" => {
            let job = object
                .get("job")
                .and_then(serde_json::Value::as_object)
                .unwrap_or(object);
            let mut parts = Vec::new();
            if key == "cron.pause" {
                parts.push("paused".to_owned());
            } else if key == "cron.resume" {
                parts.push(
                    if job.get("completed").and_then(serde_json::Value::as_bool) == Some(true) {
                        "already completed".to_owned()
                    } else {
                        "resumed".to_owned()
                    },
                );
            } else {
                parts.push("updated".to_owned());
            }
            if let Some(next) = job
                .get("next_fire_at")
                .and_then(serde_json::Value::as_str)
                .filter(|value| !value.trim().is_empty())
            {
                parts.push(format!("next run {}", compact_result_timestamp(next)));
            }
            Some(parts.join(" · "))
        }
        "cron.history" => object
            .get("entries")
            .and_then(serde_json::Value::as_array)
            .map(|entries| format_count(entries.len(), "runs")),
        _ => None,
    }
}

fn compact_result_timestamp(value: &str) -> String {
    let value = value.trim();
    if value.is_ascii()
        && value.len() >= 16
        && value.as_bytes().get(10) == Some(&b'T')
        && value.as_bytes().get(13) == Some(&b':')
    {
        let mut compact = format!("{} {}", &value[..10], &value[11..16]);
        if value.ends_with('Z') {
            compact.push('Z');
        } else if value.as_bytes().get(19) == Some(&b'+') || value.as_bytes().get(19) == Some(&b'-')
        {
            compact.push_str(&value[19..value.len().min(25)]);
        }
        compact
    } else {
        normalize_tool_title(value)
    }
}

fn monitor_result_fragment(
    key: &str,
    object: &serde_json::Map<String, serde_json::Value>,
) -> Option<String> {
    let action = object
        .get("action")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_else(|| key.rsplit('.').next().unwrap_or("monitor"));
    let status = object
        .get("status")
        .and_then(serde_json::Value::as_str)
        .and_then(normalize_result_status);
    match action.to_ascii_lowercase().as_str() {
        "start" => match status {
            Some(status) if status == "running" => Some(status),
            Some(status) if matches!(status.as_str(), "failed" | "cancelled" | "timed out") => {
                Some(status)
            }
            Some(status) => Some(format!("started · {status}")),
            None => Some("started".to_owned()),
        },
        "stop" => match status {
            Some(status) if matches!(status.as_str(), "failed" | "cancelled" | "timed out") => {
                Some(status)
            }
            _ => Some("stopped".to_owned()),
        },
        _ => status,
    }
}

fn web_result_fragment(
    key: &str,
    object: &serde_json::Map<String, serde_json::Value>,
) -> Option<String> {
    match key {
        "web.search" => {
            let count = object
                .get("results")
                .and_then(serde_json::Value::as_array)
                .map(Vec::len)?;
            let engine = object
                .get("engine")
                .and_then(serde_json::Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .map(normalize_tool_title);
            let mut parts = vec![format_count(count, "results")];
            if let Some(engine) = engine {
                parts.push(engine);
            }
            Some(parts.join(" · "))
        }
        "web.crawl" => {
            let mut parts = Vec::new();
            for (field, label) in [
                ("stored_count", "indexed"),
                ("cached_count", "cached"),
                ("failure_count", "failures"),
            ] {
                if let Some(value) = object.get(field).and_then(value_as_u64) {
                    parts.push(format!("{value} {label}"));
                }
            }
            if parts.is_empty() {
                object
                    .get("documents")
                    .and_then(serde_json::Value::as_array)
                    .map(|documents| format_count(documents.len(), "documents"))
            } else {
                Some(parts.join(" · "))
            }
        }
        _ => None,
    }
}

fn plan_result_fragment(object: &serde_json::Map<String, serde_json::Value>) -> Option<String> {
    if object.get("cleared").and_then(serde_json::Value::as_bool) == Some(true) {
        return Some("cleared".to_owned());
    }
    let plan = object
        .get("plan")
        .and_then(serde_json::Value::as_object)
        .unwrap_or(object);
    let mut parts = Vec::new();
    if let Some(phase) = plan
        .get("phase")
        .and_then(serde_json::Value::as_str)
        .filter(|phase| !phase.trim().is_empty())
    {
        parts.push(normalize_tool_title(phase.replace('_', " ")));
    }
    if let Some(steps) = plan.get("steps").and_then(serde_json::Value::as_array) {
        let completed = steps
            .iter()
            .filter(|step| {
                matches!(
                    step.get("status").and_then(serde_json::Value::as_str),
                    Some("completed" | "skipped")
                )
            })
            .count();
        parts.push(format!("{completed}/{} steps", steps.len()));
        let blocked = steps
            .iter()
            .filter(|step| {
                step.get("status").and_then(serde_json::Value::as_str) == Some("blocked")
            })
            .count();
        if blocked > 0 {
            parts.push(format_count(blocked, "blocked"));
        }
    }
    if let Some(decision) = object
        .get("decision")
        .and_then(serde_json::Value::as_str)
        .filter(|decision| !decision.trim().is_empty())
    {
        parts.push(normalize_tool_title(decision.replace('_', " ")));
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
}

fn discovery_result_fragment(
    key: &str,
    object: &serde_json::Map<String, serde_json::Value>,
) -> Option<String> {
    let (count_key, label) = if key.contains("plugins") {
        if key.ends_with("tags") {
            ("tags", "tags")
        } else {
            ("plugins", "plugins")
        }
    } else if key.ends_with("tags") {
        ("tags", "tags")
    } else {
        ("tools", "tools")
    };
    if let Some(values) = object.get(count_key).and_then(serde_json::Value::as_array) {
        return Some(format_count(values.len(), label));
    }
    for key in ["returned", "returned_count", "count", "total_returned"] {
        if let Some(count) = object.get(key).and_then(value_as_u64) {
            return Some(format_count(count as usize, label));
        }
    }
    None
}

fn append_cursor_fact(
    parts: &mut Vec<String>,
    object: &serde_json::Map<String, serde_json::Value>,
) {
    if object
        .get("next_cursor")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|cursor| !cursor.trim().is_empty())
    {
        parts.push("more available".to_owned());
    }
}

fn browser_result_fragment(
    key: &str,
    object: &serde_json::Map<String, serde_json::Value>,
) -> Option<String> {
    let result = object.get("result").and_then(serde_json::Value::as_object);
    let snapshot = object
        .get("snapshot")
        .and_then(serde_json::Value::as_object)
        .or_else(|| {
            result.and_then(|result| {
                result
                    .get("snapshot")
                    .and_then(serde_json::Value::as_object)
            })
        });
    let action = object
        .get("action")
        .or_else(|| result.and_then(|result| result.get("action")));
    if action
        .and_then(serde_json::Value::as_object)
        .and_then(|action| action.get("ok"))
        .and_then(serde_json::Value::as_bool)
        == Some(false)
    {
        return Some("failed".to_owned());
    }
    if let Some(snapshot) = snapshot {
        let title = snapshot
            .get("title")
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .map(normalize_tool_title);
        let elements = snapshot
            .get("elements")
            .and_then(|value| value.as_array().map(|values| values.len() as u64));
        let mut parts = Vec::new();
        match key {
            "browser_click" | "web.browser_click" => parts.push("clicked".to_owned()),
            "browser_type" | "web.browser_type" => parts.push("typed".to_owned()),
            "browser_wait" | "web.browser_wait" => parts.push("ready".to_owned()),
            _ => {}
        }
        parts.extend(title);
        if !matches!(
            key,
            "browser_click" | "web.browser_click" | "browser_type" | "web.browser_type"
        ) {
            parts
                .extend(elements.map(|value| format_count(value as usize, "interactive elements")));
        }
        if !parts.is_empty() {
            return Some(parts.join(" · "));
        }
    }
    if object.get("elapsed_ms").and_then(value_as_u64).is_some() {
        return object
            .get("elapsed_ms")
            .and_then(value_as_u64)
            .map(|value| format!("waited {value} ms"));
    }
    if object.get("ok").and_then(serde_json::Value::as_bool) == Some(true)
        || object
            .get("result")
            .and_then(serde_json::Value::as_object)
            .and_then(|result| result.get("ok"))
            .and_then(serde_json::Value::as_bool)
            == Some(true)
    {
        return Some("completed".to_owned());
    }
    None
}

fn first_array<'a>(
    object: &'a serde_json::Map<String, serde_json::Value>,
    keys: &[&str],
) -> Option<&'a Vec<serde_json::Value>> {
    keys.iter()
        .find_map(|key| object.get(*key).and_then(serde_json::Value::as_array))
}

fn result_title_fragment_from_object(
    object: &serde_json::Map<String, serde_json::Value>,
) -> Option<String> {
    if let Some(exit_code) = object.get("exit_code").and_then(value_as_i64) {
        return Some(if exit_code == 0 {
            "passed".to_owned()
        } else {
            format!("failed · exit {exit_code}")
        });
    }

    // Grep returns both a numeric `matches` fact and a parallel `results`
    // array; the numeric fact is the more useful collapsed-row label.
    if let Some(count) = object.get("matches").and_then(value_as_u64) {
        return Some(format_count(count as usize, "matches"));
    }

    // Prefer explicitly named arrays before generic `count`/`total` fields.
    // Glob, for example, returns both `count` and `paths`; the latter tells us
    // that the count is a set of file matches rather than generic items.
    for (key, label) in [
        ("changes", "files changed"),
        ("loaded_paths", "files loaded"),
        ("files", "files"),
        ("paths", "matches"),
        ("matches", "matches"),
        ("findings", "findings"),
        ("tools", "tools"),
        ("servers", "servers"),
        ("snapshots", "snapshots"),
        ("resources", "resources"),
        ("resource_templates", "resource templates"),
        ("prompts", "prompts"),
        ("memories", "memories"),
        ("results", "results"),
        ("items", "results"),
        ("tasks", "tasks"),
        ("entries", "entries"),
        ("jobs", "schedules"),
        ("locations", "locations"),
        ("processes", "processes"),
        ("events", "events"),
        ("sources", "sources"),
        ("warnings", "warnings"),
    ] {
        if let Some(values) = object.get(key).and_then(serde_json::Value::as_array) {
            return Some(format_count(values.len(), label));
        }
    }

    for (key, label) in [
        ("count", "items"),
        ("total", "items"),
        ("tool_count", "tools"),
        ("finding_count", "findings"),
        ("result_count", "results"),
        ("changed_files", "files changed"),
        ("loaded_files", "files loaded"),
        ("file_count", "files"),
        ("source_count", "sources"),
        ("snapshot_count", "snapshots"),
        ("warning_count", "warnings"),
        ("event_count", "events"),
        ("memory_count", "memories"),
        ("total_tools", "tools"),
        ("returned_tools", "tools"),
        ("scanned_files", "files scanned"),
    ] {
        if let Some(count) = object.get(key).and_then(value_as_u64) {
            return Some(format_count(count as usize, label));
        }
    }

    if let Some(status) = object
        .get("status")
        .or_else(|| object.get("state"))
        .and_then(serde_json::Value::as_str)
        .and_then(normalize_result_status)
    {
        return Some(status);
    }

    for key in [
        "summary",
        "message",
        "result",
        "title",
        "description",
        "text",
    ] {
        if let Some(value) = object.get(key).and_then(serde_json::Value::as_str)
            && let Some(value) = compact_result_text(value)
        {
            return Some(value);
        }
    }

    for key in ["created", "updated", "deleted", "removed", "saved"] {
        if object.get(key).and_then(serde_json::Value::as_bool) == Some(true) {
            return Some(match key {
                "created" => "created".to_owned(),
                "updated" => "updated".to_owned(),
                "deleted" | "removed" => "removed".to_owned(),
                "saved" => "saved".to_owned(),
                _ => unreachable!(),
            });
        }
    }

    // Provider and MCP envelopes commonly put the useful compact fact below
    // result, structured_content, or data. Inspect only these bounded
    // semantic containers; arbitrary recursive payload walking would make a
    // title depend on incidental nested JSON.
    for key in [
        "result",
        "structured_content",
        "data",
        "response",
        "plan",
        "task",
        "snapshot",
        "browser",
    ] {
        if let Some(nested) = object.get(key).and_then(serde_json::Value::as_object)
            && let Some(fragment) = result_title_fragment_from_object(nested)
        {
            return Some(fragment);
        }
    }
    None
}

fn value_as_i64(value: &serde_json::Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_u64().and_then(|value| i64::try_from(value).ok()))
        .or_else(|| value.as_str().and_then(|value| value.trim().parse().ok()))
}

fn value_as_u64(value: &serde_json::Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_i64().and_then(|value| u64::try_from(value).ok()))
        .or_else(|| value.as_str().and_then(|value| value.trim().parse().ok()))
}

fn render_invocation_title(tool_name: &str, input: &serde_json::Value) -> String {
    let action = tool_action_label(tool_name);
    let subject = invocation_title_subject(tool_name, input);
    if subject.is_empty() {
        return normalize_tool_title(action);
    }
    if action_is_noun_phrase_with_subject(action.as_str()) {
        normalize_tool_title(format!("{action} {subject}"))
    } else {
        compose_tool_title(action, subject)
    }
}

fn action_is_noun_phrase_with_subject(action: &str) -> bool {
    matches!(
        action,
        "Read"
            | "Write"
            | "Create"
            | "Update"
            | "Delete"
            | "Open page"
            | "Fetch page"
            | "View image"
            | "Inspect file"
            | "Inspect symbol"
            | "Read memory"
            | "Save memory"
            | "Delete memory"
            | "Update settings"
            | "Reset settings"
            | "Edit text"
            | "Edit image"
            | "Rename session"
    )
}

fn tool_action_label(tool_name: &str) -> String {
    let key = normalized_tool_identity(tool_name);
    match key.as_str() {
        "fs.read" | "read" => "Read".to_owned(),
        "fs.read_many" => "Read files".to_owned(),
        "fs.write" | "write" => "Write".to_owned(),
        "fs.apply_patch" | "apply_patch" => "Apply patch".to_owned(),
        "fs.glob" | "glob" => "Find files".to_owned(),
        "fs.grep" | "grep" => "Search files".to_owned(),
        "fs.replace" => "Replace text".to_owned(),
        "fs.stat" => "Inspect file".to_owned(),
        "fs.view_image" => "View image".to_owned(),
        "code.search_ast" => "Search AST".to_owned(),
        "code.syntax_tree" => "Inspect syntax tree".to_owned(),
        "shell.run" | "shell" => "Run process".to_owned(),
        "shell.list" => "List processes".to_owned(),
        "shell.logs" => "Show process logs".to_owned(),
        "shell.stop" => "Stop process".to_owned(),
        "monitor.start" => "Start monitor".to_owned(),
        "monitor.stop" => "Stop monitor".to_owned(),
        "tools.search" | "tools_search" | "tool_search" => "Search tools".to_owned(),
        "tools.list" | "tools_list" => "List tools".to_owned(),
        "tools.help" | "tools_help" => "Read tool help".to_owned(),
        "tools.tags" | "tools_tags" => "List tool tags".to_owned(),
        "tools.call" | "tools_call" => "Call tool".to_owned(),
        "tools.plugins_list" | "tools.plugins.list" | "tools_plugins_list" | "plugins.list" => {
            "List plugins".to_owned()
        }
        "tools.plugins_search"
        | "tools.plugins.search"
        | "tools_plugins_search"
        | "plugins.search" => "Search plugins".to_owned(),
        "tools.plugins_tags" | "tools.plugins.tags" | "tools_plugins_tags" | "plugins.tags" => {
            "List plugin tags".to_owned()
        }
        "interaction.ask" | "ask_user" => "Ask user".to_owned(),
        "interaction.notify" => "Send notification".to_owned(),
        "web.fetch" | "web_fetch" => "Fetch page".to_owned(),
        "web.search" | "web_search" | "chatgpt.web_search" | "claude.web_search"
        | "gemini.web_search" => "Search web".to_owned(),
        "chatgpt.web_search_preview" | "claude.web_search_preview" => "Search web".to_owned(),
        "claude.web_fetch" => "Fetch page".to_owned(),
        "gemini.google_search" => "Search web".to_owned(),
        "gemini.google_maps" => "Search maps".to_owned(),
        "gemini.url_context" => "Read web context".to_owned(),
        "gemini.mcp_server" => "Connect MCP server".to_owned(),
        "gemini.retrieval" => "Retrieve context".to_owned(),
        "gemini.function" => "Declare function tool".to_owned(),
        "chatgpt.code_interpreter" | "claude.code_execution" | "gemini.code_execution" => {
            "Run code".to_owned()
        }
        "chatgpt.local_shell" | "claude.bash" | "bash" | "local_shell" => "Run process".to_owned(),
        "chatgpt.tool_search"
        | "claude.tool_search_bm25"
        | "claude.tool_search_regex"
        | "claude.tool_search_tool_bm25"
        | "claude.tool_search_tool_regex" => "Search tools".to_owned(),
        "chatgpt.programmatic_tool_calling" => "Call tools programmatically".to_owned(),
        "chatgpt.mcp" => "Connect MCP server".to_owned(),
        "chatgpt.shell" => "Run hosted shell".to_owned(),
        "chatgpt.apply_patch" => "Apply patch".to_owned(),
        "chatgpt.function" => "Declare function tool".to_owned(),
        "chatgpt.custom" => "Declare custom tool".to_owned(),
        "chatgpt.namespace" => "Declare tool namespace".to_owned(),
        "claude.str_replace_based_edit_tool" => "Edit text".to_owned(),
        "claude.text_editor" => "Edit text".to_owned(),
        "claude.memory" => "Use memory".to_owned(),
        "claude.advisor" => "Consult advisor".to_owned(),
        "claude.mcp_toolset" => "Configure MCP toolset".to_owned(),
        "chatgpt.file_search" | "claude.file_search" | "gemini.file_search" => {
            "Search files".to_owned()
        }
        "openai.image_generation" | "chatgpt.image_generation" | "gemini.image_generation" => {
            "Generate image".to_owned()
        }
        "openai.image_edit" | "chatgpt.image_edit" | "gemini.image_edit" => "Edit image".to_owned(),
        "openai.web_search" => "Search web".to_owned(),
        "chatgpt.computer"
        | "chatgpt.computer_use_preview"
        | "claude.computer"
        | "gemini.computer_use" => "Use computer".to_owned(),
        "snapshot.enter" | "enter_snapshot" => "Enter snapshot".to_owned(),
        "snapshot.exit" | "exit_snapshot" => "Exit snapshot".to_owned(),
        "lsp.definition" | "lsp_definition" => "Find definition".to_owned(),
        "lsp.references" | "lsp_references" => "Find references".to_owned(),
        "lsp.hover" | "lsp_hover" => "Inspect symbol".to_owned(),
        "lsp.diagnostics" | "lsp_diagnostics" => "Check diagnostics".to_owned(),
        "lsp.servers" => "List language servers".to_owned(),
        "mcp.tools.call" => "Call MCP tool".to_owned(),
        "mcp.tools.search" => "Search MCP tools".to_owned(),
        "mcp.tools" => "List MCP tools".to_owned(),
        "mcp.resources.read" => "Read MCP resource".to_owned(),
        "mcp.resources.list" => "List MCP resources".to_owned(),
        "mcp.resources.templates.list" => "List MCP resource templates".to_owned(),
        "mcp.resources" => "List MCP resources".to_owned(),
        "mcp.prompts.list" => "List MCP prompts".to_owned(),
        "mcp.prompts.get" => "Get MCP prompt".to_owned(),
        "mcp.prompts" => "List MCP prompts".to_owned(),
        "mcp.servers.status" => "Check MCP servers".to_owned(),
        "mcp.servers.reconnect" => "Reconnect MCP server".to_owned(),
        "mcp.servers" => "List MCP servers".to_owned(),
        "memory.search" => "Search memory".to_owned(),
        "memory.get" => "Read memory".to_owned(),
        "memory.list" => "List memories".to_owned(),
        "memory.write" => "Save memory".to_owned(),
        "memory.delete" => "Delete memory".to_owned(),
        "notebook.edit_cell" => "Edit notebook cell".to_owned(),
        "settings.inspect" => "Inspect settings".to_owned(),
        "settings.set" => "Update settings".to_owned(),
        "settings.reset" => "Reset settings".to_owned(),
        "cron.create" | "cron_create" => "Create schedule".to_owned(),
        "cron.list" | "cron_list" => "List schedules".to_owned(),
        "cron.update" | "cron_update" => "Update schedule".to_owned(),
        "cron.delete" | "cron_delete" => "Delete schedule".to_owned(),
        "cron.pause" | "cron_pause" => "Pause schedule".to_owned(),
        "cron.resume" | "cron_resume" => "Resume schedule".to_owned(),
        "cron.history" | "cron_history" => "Show schedule history".to_owned(),
        "tasks.run" | "task" => "Run task".to_owned(),
        "tasks.cancel" => "Cancel task".to_owned(),
        "tasks.followup" => "Continue task".to_owned(),
        "tasks.get" => "Inspect task".to_owned(),
        "tasks.list" => "List tasks".to_owned(),
        "tasks.message" => "Message task".to_owned(),
        "tasks.output" => "Read task output".to_owned(),
        "plan.update" => "Update plan".to_owned(),
        "plan.get" => "Inspect plan".to_owned(),
        "plan.clear" => "Clear plan".to_owned(),
        "plan.edit" => "Edit plan".to_owned(),
        "plan.phase" => "Update plan phase".to_owned(),
        "plan.review" => "Review plan".to_owned(),
        "plan.set" => "Set plan".to_owned(),
        "report.findings" => "Review findings".to_owned(),
        "session.environment" => "Inspect environment".to_owned(),
        "session.get" => "Inspect session".to_owned(),
        "session.model" => "Inspect model".to_owned(),
        "session.rename" => "Rename session".to_owned(),
        "session.tokens" => "Inspect token usage".to_owned(),
        "settings.get" => "Read setting".to_owned(),
        "settings.list" => "List settings".to_owned(),
        "settings.delete" => "Delete setting".to_owned(),
        "settings.patch" => "Patch settings".to_owned(),
        "settings.validate" => "Validate settings".to_owned(),
        "skills.create" => "Create skill".to_owned(),
        "skills.delete" => "Delete skill".to_owned(),
        "skills.get" => "Inspect skill".to_owned(),
        "skills.list" => "List skills".to_owned(),
        "skills.read_resource" => "Read skill resource".to_owned(),
        "skills.refresh" => "Refresh skills".to_owned(),
        "skills.update" => "Update skill".to_owned(),
        "snapshot.status" => "Inspect snapshot".to_owned(),
        "repo.status" => "Inspect repository".to_owned(),
        "web.browser_open" | "browser_open" => "Open page".to_owned(),
        "web.browser_click" | "browser_click" => "Click element".to_owned(),
        "web.browser_type" | "browser_type" => "Enter text".to_owned(),
        "web.browser_screenshot" | "browser_screenshot" => "Capture page".to_owned(),
        "web.browser_snapshot" | "browser_snapshot" => "Inspect page".to_owned(),
        "web.browser_wait" | "browser_wait" => "Wait for page".to_owned(),
        "web.browser_close" | "browser_close" => "Close page".to_owned(),
        "web.browser_download" | "browser_download" => "Download file".to_owned(),
        "web.browser_list" | "browser_list" => "List pages".to_owned(),
        "web.browser_shutdown" | "browser_shutdown" => "Close browser".to_owned(),
        "web.crawl" => "Crawl site".to_owned(),
        _ => humanize_tool_leaf(&key),
    }
}

fn normalized_tool_identity(tool_name: &str) -> String {
    let mut key = tool_name.trim().replace("__", ".").replace('/', ".");
    if let Some(stripped) = key.strip_prefix("agena.") {
        key = stripped.to_owned();
    } else if let Some(stripped) = key.strip_prefix("agena_") {
        key = stripped.replace('_', ".");
    }
    key.to_ascii_lowercase()
}

fn humanize_tool_leaf(key: &str) -> String {
    let leaf = key.rsplit('.').next().unwrap_or(key);
    let words = leaf
        .split(['_', '-'])
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>();
    if words.is_empty() {
        return "Run tool".to_owned();
    }
    let mut title = words.join(" ");
    if let Some(first) = title.get_mut(0..1) {
        first.make_ascii_uppercase();
    }
    title
}

fn invocation_title_subject(tool_name: &str, input: &serde_json::Value) -> String {
    let key = normalized_tool_identity(tool_name);
    if key.ends_with("fs.apply_patch") {
        return patch_title_subject(input);
    }
    if key.ends_with("mcp.tools.call") {
        return mcp_call_title_subject(input);
    }
    if key.starts_with("lsp.") {
        return lsp_title_subject(input);
    }
    if is_provider_tool_identity(key.as_str()) {
        return provider_invocation_title_subject(key.as_str(), input);
    }
    let preferred: &[&str] = if key.ends_with("code.search_ast") {
        &["pattern", "path", "language"]
    } else if key.ends_with("code.syntax_tree") {
        &["path", "language"]
    } else if key.ends_with("report.findings") {
        &["summary"]
    } else if key.ends_with("shell.run") {
        &["command", "description"]
    } else if key.ends_with("shell.logs") || key.ends_with("shell.stop") {
        &["process_id", "id"]
    } else if key.ends_with("monitor.start") {
        &["command", "description", "url"]
    } else if key.ends_with("monitor.stop") {
        &["monitor_id", "id"]
    } else if key.ends_with("fs.read") || key.ends_with("fs.write") {
        &["file_path", "path", "name"]
    } else if key.ends_with("fs.read_many") {
        &["paths", "path"]
    } else if key.ends_with("fs.replace")
        || key.ends_with("fs.stat")
        || key.ends_with("fs.view_image")
        || key.ends_with("notebook.edit_cell")
    {
        &["file_path", "path", "notebook_path", "name"]
    } else if key.ends_with("fs.grep") || key.ends_with("fs.glob") {
        &["pattern", "path", "include"]
    } else if key.ends_with("web.search") || key.ends_with("web_search") {
        &["query", "q", "prompt"]
    } else if key.ends_with("web.fetch")
        || key.ends_with("web.crawl")
        || key.ends_with("browser.open")
        || key.ends_with("browser_open")
    {
        &["url", "start_url", "uri", "url_pattern"]
    } else if key.ends_with("browser.click") || key.ends_with("browser_click") {
        &["selector", "ref", "session_id"]
    } else if key.ends_with("browser.type") || key.ends_with("browser_type") {
        // Do not put arbitrary typed text (which may contain secrets) in a
        // collapsed title. The target element is enough to explain the
        // action; the full input remains available in the expanded input.
        &["selector", "ref", "element", "session_id"]
    } else if key.ends_with("browser.wait") || key.ends_with("browser_wait") {
        &["selector", "condition", "session_id"]
    } else if key.ends_with("browser.screenshot") || key.ends_with("browser_screenshot") {
        &["session_id", "path"]
    } else if key.ends_with("browser.download") || key.ends_with("browser_download") {
        &["url", "session_id"]
    } else if key.ends_with("browser.close") || key.ends_with("browser_close") {
        &["session_id"]
    } else if key == "interaction.ask" || key == "ask_user" {
        &["questions", "title"]
    } else if key == "interaction.notify" {
        // The notification body belongs in the expanded presentation. It can
        // be a long Markdown document or contain data that should not be
        // repeated in a collapsed title; the notification title and severity
        // already identify the action sufficiently.
        &["title", "level"]
    } else if key.ends_with("snapshot.enter") {
        &["path", "name", "branch"]
    } else if key.ends_with("snapshot.exit") {
        &["path", "action"]
    } else if key.starts_with("browser_") || key.starts_with("web.browser_") {
        &[
            "session_id",
            "url",
            "element",
            "ref",
            "text",
            "condition",
            "path",
        ]
    } else if key.starts_with("mcp.") {
        &["server", "uri", "name", "query", "cursor"]
    } else if key.ends_with("settings.set") || key.ends_with("settings.reset") {
        &["path", "key", "setting", "name"]
    } else if key.starts_with("settings.") {
        &["path", "key", "name"]
    } else if key.ends_with("tasks.run") || key.ends_with("plan.update") {
        // A task/plan prompt is an instruction payload, not a compact
        // operation target. It may contain secrets, a whole user request, or
        // an arbitrarily large document; keep it in the expanded input only.
        &["description", "title", "task_id", "phase"]
    } else if key.ends_with("tasks.output") {
        &["task_id", "id", "cursor"]
    } else if key.ends_with("tasks.cancel")
        || key.ends_with("tasks.message")
        || key.ends_with("tasks.followup")
        || key.ends_with("tasks.get")
    {
        // The task id identifies the operation. Message/follow-up text is
        // deliberately left out of the collapsed title: it can be long or
        // contain sensitive instructions, while the expanded input keeps it
        // available when needed.
        &["task_id", "id"]
    } else if key.ends_with("tasks.list") {
        &["status"]
    } else if key.starts_with("tasks.") {
        &[
            "task_id",
            "id",
            "description",
            "cursor",
            "message",
            "prompt",
        ]
    } else if key.starts_with("plan.") {
        &["title", "step", "phase", "decision"]
    } else if key.starts_with("cron.") {
        &["job_id", "id", "name", "expression", "at", "prompt"]
    } else if key == "memory.search" {
        &["query"]
    } else if key.starts_with("memory.") {
        &["name", "path"]
    } else if key.starts_with("session.") {
        &["title", "name", "session_id"]
    } else if key.starts_with("skills.") {
        &["name", "path", "resource", "document"]
    } else if key.starts_with("chatgpt.")
        || key.starts_with("claude.")
        || key.starts_with("gemini.")
        || key.starts_with("openai.")
    {
        &[
            "prompt",
            "query",
            "url",
            "command",
            "name",
            "tool",
            "model",
            "images",
            "server_label",
            "server_url",
            "connector_id",
            "mcp_server_name",
            "vector_store_ids",
            "file_search_store_names",
            "retrieval_types",
            "search_types",
            "operation",
            "action",
            "environment",
        ]
    } else {
        &[
            "command",
            "file_path",
            "path",
            "pattern",
            "query",
            "url",
            "prompt",
            "description",
            "title",
            "tool",
            "name",
            "key",
            "expression",
            "notebook_path",
            "process_id",
            "task_id",
            "job_id",
            "monitor_id",
            "element",
            "text",
            "body",
            "cell",
        ]
    };

    let Some(object) = input.as_object() else {
        return String::new();
    };
    let values = title_values_for_object(object, preferred);
    if !values.is_empty() {
        return values.join(" · ");
    }
    for container in ["tool_options", "options", "request_options", "input"] {
        if let Some(nested) = object.get(container).and_then(serde_json::Value::as_object) {
            let values = title_values_for_object(nested, preferred);
            if !values.is_empty() {
                return values.join(" · ");
            }
        }
    }
    String::new()
}

fn provider_invocation_title_subject(key: &str, input: &serde_json::Value) -> String {
    let Some(object) = input.as_object() else {
        return String::new();
    };

    // Provider envelopes can contain many routing fields at once (model,
    // environment, connector, schema, and the actual operation target). A
    // title should name the operation's target, not the first two arbitrary
    // strings in that envelope.
    let preferred: &[&str] = if key.contains("web_search") || key.ends_with("google_search") {
        &["query", "prompt", "q"]
    } else if key.ends_with("google_maps") || key.ends_with("retrieval") {
        &["query", "prompt"]
    } else if key.ends_with("file_search")
        || key.contains("tool_search")
        || key.ends_with("tool_search")
    {
        &["query", "prompt"]
    } else if key.ends_with("web_fetch") || key.ends_with("url_context") {
        &["url", "uri"]
    } else if key.ends_with("code_interpreter") || key.ends_with("code_execution") {
        &["command", "code", "prompt", "description"]
    } else if key.ends_with("local_shell") || key.ends_with(".shell") || key.ends_with(".bash") {
        &["command", "description"]
    } else if key.ends_with("computer")
        || key.ends_with("computer_use_preview")
        || key.ends_with("computer_use")
    {
        &["target", "url", "selector", "ref", "action", "prompt"]
    } else if key.ends_with("mcp") || key.ends_with("mcp_server") || key.ends_with("mcp_toolset") {
        &["server_label", "mcp_server_name", "server_url", "prompt"]
    } else if key.ends_with("memory") {
        &["operation", "name", "path"]
    } else if key.ends_with("text_editor") || key.ends_with("str_replace_based_edit_tool") {
        &["path", "file_path", "operation"]
    } else if key.ends_with("apply_patch") {
        let patch = patch_title_subject(input);
        if !patch.is_empty() {
            return patch;
        }
        return title_values_for_object(object, &["path", "file_path", "target"])
            .into_iter()
            .next()
            .unwrap_or_default();
    } else if key.ends_with("image_generation") || key.ends_with("image_edit") {
        &["prompt", "description", "path"]
    } else if key.ends_with("function")
        || key.ends_with("custom")
        || key.ends_with("namespace")
        || key.contains("programmatic_tool_calling")
    {
        &["prompt", "name", "tool", "function", "operation"]
    } else if key.ends_with("advisor") {
        &["prompt", "question", "topic"]
    } else {
        &[
            "prompt",
            "query",
            "url",
            "command",
            "name",
            "tool",
            "operation",
        ]
    };

    let values = title_values_for_object(object, preferred);
    if !values.is_empty() {
        return values.into_iter().take(2).collect::<Vec<_>>().join(" · ");
    }
    for container in ["tool_options", "options", "request_options", "input"] {
        if let Some(nested) = object.get(container).and_then(serde_json::Value::as_object) {
            let values = title_values_for_object(nested, preferred);
            if !values.is_empty() {
                return values.into_iter().take(2).collect::<Vec<_>>().join(" · ");
            }
        }
    }
    String::new()
}

fn mcp_call_title_subject(input: &serde_json::Value) -> String {
    let Some(object) = input.as_object() else {
        return String::new();
    };
    let server = object
        .get("server")
        .and_then(title_value_text)
        .unwrap_or_default();
    let tool = object
        .get("name")
        .or_else(|| object.get("tool"))
        .and_then(title_value_text)
        .unwrap_or_default();
    match (tool.is_empty(), server.is_empty()) {
        (false, false) => format!("{tool} @ {server}"),
        (false, true) => tool,
        (true, false) => server,
        (true, true) => String::new(),
    }
}

fn lsp_title_subject(input: &serde_json::Value) -> String {
    let Some(object) = input.as_object() else {
        return String::new();
    };
    let position = object
        .get("position")
        .and_then(serde_json::Value::as_object);
    let path = position
        .and_then(|position| position.get("file_path"))
        .or_else(|| object.get("file_path"))
        .and_then(title_value_text);
    let line = position
        .and_then(|position| position.get("line"))
        .or_else(|| object.get("line"))
        .and_then(value_as_u64);
    let character = position
        .and_then(|position| position.get("character"))
        .or_else(|| object.get("character"))
        .and_then(value_as_u64);
    let Some(path) = path else {
        return String::new();
    };
    match (line, character) {
        (Some(line), Some(character)) => format!("{path}:{}:{}", line + 1, character + 1),
        (Some(line), None) => format!("{path}:{}", line + 1),
        _ => path,
    }
}

fn patch_title_subject(input: &serde_json::Value) -> String {
    let Some(patch) = input
        .as_object()
        .and_then(|object| object.get("patch"))
        .and_then(serde_json::Value::as_str)
    else {
        return String::new();
    };
    patch
        .lines()
        .filter_map(|line| {
            ["*** Update File: ", "*** Add File: ", "*** Delete File: "]
                .iter()
                .find_map(|prefix| line.trim().strip_prefix(prefix))
        })
        .take(2)
        .map(|path| normalize_tool_title(path.trim()))
        .collect::<Vec<_>>()
        .join(" · ")
}

fn title_values_for_object(
    object: &serde_json::Map<String, serde_json::Value>,
    preferred: &[&str],
) -> Vec<String> {
    preferred
        .iter()
        .filter_map(|key| {
            object
                .get(*key)
                .and_then(|value| title_value_text_for_key(key, value))
        })
        .filter(|value| !value.is_empty())
        .take(2)
        .collect::<Vec<_>>()
}

fn title_value_text_for_key(key: &str, value: &serde_json::Value) -> Option<String> {
    let key = key.to_ascii_lowercase();
    let normalized_key = key.replace('-', "_");
    let sensitive = [
        "password",
        "passcode",
        "secret",
        "authorization",
        "credential",
        "cookie",
    ]
    .iter()
    .any(|marker| normalized_key == *marker || normalized_key.contains(&format!("_{marker}")))
        || normalized_key == "token"
        || normalized_key.ends_with("_token")
        || normalized_key.contains("api_key")
        || normalized_key.contains("apikey")
        || normalized_key.contains("private_key")
        || normalized_key.contains("client_secret");
    if sensitive {
        return None;
    }
    if let Some(value) = title_value_text(value) {
        return Some(value);
    }
    let values = value.as_array()?;
    if values.is_empty() {
        return None;
    }
    if key == "paths" || key == "images" {
        let first = values.first().and_then(title_value_text)?;
        return if values.len() == 1 {
            Some(first)
        } else {
            Some(format!("{first} +{}", values.len() - 1))
        };
    }
    Some(format_count(
        values.len(),
        if key == "questions" {
            "questions"
        } else {
            "items"
        },
    ))
}

fn title_value_text(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(value) => {
            let value = value.trim();
            (!value.is_empty()).then(|| normalize_tool_title(value))
        }
        serde_json::Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}

fn format_count(count: usize, label: &str) -> String {
    let label = match (count, label) {
        (1, "files changed") => "file changed",
        (1, "files loaded") => "file loaded",
        (1, "files read") => "file read",
        (1, "files scanned") => "file scanned",
        (1, "settings updated") => "setting updated",
        (1, "settings") => "setting",
        (1, "replacements") => "replacement",
        (1, "matches") => "match",
        (1, "results") => "result",
        (1, "items") => "item",
        (1, "tools") => "tool",
        (1, "findings") => "finding",
        (1, "cells") => "cell",
        (1, "chunks") => "chunk",
        (1, "interactive elements") => "interactive element",
        (1, "tokens") => "token",
        (1, "resource templates") => "resource template",
        (1, "servers") => "server",
        (1, "definitions") => "definition",
        (1, "references") => "reference",
        (1, "diagnostics") => "diagnostic",
        (1, "resources") => "resource",
        (1, "prompts") => "prompt",
        (1, "messages") => "message",
        (1, "content blocks") => "content block",
        (1, "tasks") => "task",
        (1, "files") => "file",
        (1, "entries") => "entry",
        (1, "schedules") => "schedule",
        (1, "records") => "record",
        (1, "runs") => "run",
        (1, "documents") => "document",
        (1, "locations") => "location",
        (1, "processes") => "process",
        (1, "snapshots") => "snapshot",
        (1, "pages") => "page",
        (1, "places") => "place",
        (1, "actions") => "action",
        (1, "outputs") => "output",
        (1, "questions") => "question",
        (1, "sources") => "source",
        (1, "images") => "image",
        (1, "pending calls") => "pending call",
        (1, "warnings") => "warning",
        (1, "events") => "event",
        (1, "memories") => "memory",
        (1, "outputs saved") => "output saved",
        (1, "attachments") => "attachment",
        _ => label,
    };
    format!("{count} {label}")
}

fn format_bytes(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = KIB * 1024;
    const GIB: u64 = MIB * 1024;
    if bytes >= GIB {
        format!("{:.1} GiB", bytes as f64 / GIB as f64)
    } else if bytes >= MIB {
        format!("{:.1} MiB", bytes as f64 / MIB as f64)
    } else if bytes >= KIB {
        format!("{:.1} KiB", bytes as f64 / KIB as f64)
    } else {
        format!("{bytes} B")
    }
}

fn normalize_result_status(status: &str) -> Option<String> {
    let status = status.trim().to_ascii_lowercase();
    if status.is_empty() {
        return None;
    }
    Some(match status.as_str() {
        "ok" | "success" | "succeeded" | "complete" | "completed" | "done" => {
            "completed".to_owned()
        }
        "error" | "failed" | "failure" => "failed".to_owned(),
        "cancelled" | "canceled" => "cancelled".to_owned(),
        "pending" | "queued" => "queued".to_owned(),
        "running" | "in_progress" => "running".to_owned(),
        "timed_out" | "timeout" => "timed out".to_owned(),
        other => normalize_tool_title(other.replace('_', " ")),
    })
}

fn compact_result_text(value: &str) -> Option<String> {
    let value = value
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default();
    if value.is_empty() {
        None
    } else {
        Some(normalize_tool_summary(value))
    }
}

/// Pick the single most informative string argument of a tool invocation to
/// use as a call-start title summary ("README.md", "cargo test", "filesystem").
/// Returns an empty string when the input carries no obvious subject so the
/// caller can fall back to the bare tool name.
pub fn invocation_call_summary(input: &serde_json::Value) -> String {
    const PREFERRED_KEYS: &[&str] = &[
        "tool",
        "command",
        "description",
        "file_path",
        "path",
        "pattern",
        "query",
        "url",
        "title",
        "expression",
        "notebook_path",
        "process_id",
        "task_id",
        "function",
        "model",
        "id",
        "name",
    ];
    for key in PREFERRED_KEYS {
        if let Some(value) = input.get(*key).and_then(serde_json::Value::as_str) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return trimmed.to_owned();
            }
        }
    }
    // Provider-native envelopes nest the real target under `input`.
    if let Some(inner) = input.get("input").and_then(serde_json::Value::as_object)
        && let Some(value) = inner.get("tool").and_then(serde_json::Value::as_str)
    {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return trimmed.to_owned();
        }
    }
    String::new()
}

#[cfg(test)]
mod tool_title_tests {
    use super::{
        RawOutput, TOOL_SUMMARY_MAX_DISPLAY_WIDTH, TOOL_TITLE_MAX_DISPLAY_WIDTH, ToolInvocation,
        ToolResultState, completed_tool_title, completed_tool_title_with_action_for_invocation,
        compose_tool_title, initial_tool_title, invocation_call_summary, is_tool_identity_title,
        normalize_tool_summary, normalize_tool_title, result_title_fragment,
    };
    use agena_domain::StructuredObject;
    use serde_json::json;
    use unicode_width::UnicodeWidthStr;

    #[test]
    fn composed_titles_join_the_tool_name_and_call_summary() {
        assert_eq!(
            compose_tool_title("fs.read", "Read README.md"),
            "fs.read · Read README.md"
        );
        assert_eq!(
            compose_tool_title("tools.search", "Search tools · filesystem"),
            "tools.search · Search tools · filesystem"
        );
    }

    #[test]
    fn composed_titles_fall_back_to_the_bare_tool_name() {
        assert_eq!(compose_tool_title("shell.run", ""), "shell.run");
        assert_eq!(compose_tool_title("shell.run", "   "), "shell.run");
        assert_eq!(compose_tool_title("fs.read", "fs.read"), "fs.read");
        assert_eq!(compose_tool_title("", ""), "");
    }

    #[test]
    fn invocation_call_summary_prefers_the_most_informative_argument() {
        assert_eq!(
            invocation_call_summary(&serde_json::json!({"path": "README.md"})),
            "README.md"
        );
        assert_eq!(
            invocation_call_summary(&serde_json::json!({"command": "cargo test", "timeout_ms": 5})),
            "cargo test"
        );
        assert_eq!(
            invocation_call_summary(&serde_json::json!({"query": "filesystem"})),
            "filesystem"
        );
        assert_eq!(
            invocation_call_summary(
                &serde_json::json!({"tool": "fs.write", "input": {"path": "notes.txt"}})
            ),
            "fs.write"
        );
        assert_eq!(invocation_call_summary(&serde_json::json!({})), "");
    }

    #[test]
    fn normal_titles_are_preserved_and_whitespace_is_collapsed() {
        assert_eq!(
            normalize_tool_title("  Read   crates/agena-domain/src/activity.rs  "),
            "Read crates/agena-domain/src/activity.rs"
        );
    }

    #[test]
    fn genuinely_long_titles_are_width_bounded_with_an_ellipsis() {
        let title = format!("Inspect {}", "很长的标题".repeat(20));
        let bounded = normalize_tool_title(title);

        assert!(bounded.ends_with('…'));
        assert!(UnicodeWidthStr::width(bounded.as_str()) <= TOOL_TITLE_MAX_DISPLAY_WIDTH);
    }

    #[test]
    fn summaries_are_single_line_and_defensively_bounded() {
        let summary = format!("  42 matches\n{}  ", "in many files ".repeat(20));
        let bounded = normalize_tool_summary(summary);

        assert!(bounded.starts_with("42 matches in many files"));
        assert!(bounded.ends_with('…'));
        assert!(UnicodeWidthStr::width(bounded.as_str()) <= TOOL_SUMMARY_MAX_DISPLAY_WIDTH);
    }

    #[test]
    fn initial_titles_use_the_invocation_action_and_subject() {
        let read = ToolInvocation::new(
            "agena.fs.read",
            StructuredObject::try_from(json!({"file_path": "README.md"}))
                .expect("structured input"),
        );
        assert_eq!(initial_tool_title(&read), "Read README.md");

        let shell = ToolInvocation::new(
            "agena.shell.run",
            StructuredObject::try_from(json!({"command": "cargo test"})).expect("structured input"),
        );
        assert_eq!(initial_tool_title(&shell), "Run process · cargo test");

        let search = ToolInvocation::new(
            "tools_search",
            StructuredObject::try_from(json!({"query": "filesystem"})).expect("structured input"),
        );
        assert_eq!(initial_tool_title(&search), "Search tools · filesystem");
    }

    #[test]
    fn initial_titles_keep_instruction_and_secret_values_out_of_collapsed_text() {
        let task = ToolInvocation::new(
            "agena.tasks.run",
            StructuredObject::try_from(json!({
                "prompt": "Do not expose this full task prompt or bearer-secret",
                "task_id": "task-1"
            }))
            .expect("structured input"),
        );
        let task_title = initial_tool_title(&task);
        assert_eq!(task_title, "Run task · task-1");
        assert!(!task_title.contains("bearer-secret"));

        let browser = ToolInvocation::new(
            "agena.web.browser_type",
            StructuredObject::try_from(json!({
                "selector": "#password",
                "text": "super-secret-password"
            }))
            .expect("structured input"),
        );
        let browser_title = initial_tool_title(&browser);
        assert_eq!(browser_title, "Enter text · #password");
        assert!(!browser_title.contains("super-secret-password"));

        let notification = ToolInvocation::new(
            "agena.interaction.notify",
            StructuredObject::try_from(json!({
                "title": "Build",
                "body": "private notification body with a token"
            }))
            .expect("structured input"),
        );
        let notification_title = initial_tool_title(&notification);
        assert_eq!(notification_title, "Send notification · Build");
        assert!(!notification_title.contains("private notification body"));

        let setting = ToolInvocation::new(
            "agena.settings.get",
            StructuredObject::try_from(json!({
                "path": "providers.openai.api_key",
                "value": "sk-live-do-not-show"
            }))
            .expect("structured input"),
        );
        let setting_title = initial_tool_title(&setting);
        assert_eq!(setting_title, "Read setting · providers.openai.api_key");
        assert!(!setting_title.contains("sk-live-do-not-show"));
    }

    #[test]
    fn high_frequency_tools_have_specific_compact_actions() {
        let cases = [
            (
                "agena.fs.write",
                json!({"path": "src/lib.rs", "content": "fn main() {}"}),
                "Write src/lib.rs",
            ),
            (
                "agena.fs.read_many",
                json!({"paths": ["a.rs", "b.rs"]}),
                "Read files · a.rs +1",
            ),
            (
                "agena.mcp.resources.templates.list",
                json!({"server": "demo"}),
                "List MCP resource templates · demo",
            ),
            (
                "agena.mcp.tools.call",
                json!({"server": "demo", "name": "search"}),
                "Call MCP tool · search @ demo",
            ),
            (
                "agena.mcp.servers.reconnect",
                json!({"server": "demo"}),
                "Reconnect MCP server · demo",
            ),
            (
                "agena.openai.image_generation",
                json!({"prompt": "a watercolor city"}),
                "Generate image · a watercolor city",
            ),
            (
                "agena.chatgpt.programmatic_tool_calling",
                json!({"prompt": "find the test tool"}),
                "Call tools programmatically · find the test tool",
            ),
            (
                "agena.interaction.ask",
                json!({"questions": [{"question": "Continue?"}]}),
                "Ask user · 1 question",
            ),
            (
                "agena.tasks.cancel",
                json!({"task_id": "task-1"}),
                "Cancel task · task-1",
            ),
            (
                "agena.settings.get",
                json!({"path": "providers.openai.model"}),
                "Read setting · providers.openai.model",
            ),
            (
                "agena.web.browser_open",
                json!({"url": "https://example.test/docs", "session_id": "session-1"}),
                "Open page https://example.test/docs",
            ),
            ("agena.repo.status", json!({}), "Inspect repository"),
        ];
        for (name, input, expected) in cases {
            let invocation = ToolInvocation::new(
                name,
                StructuredObject::try_from(input).expect("structured input"),
            );
            assert_eq!(initial_tool_title(&invocation), expected, "{name}");
        }
    }

    #[test]
    fn completed_titles_add_one_compact_result_fact() {
        let read = ToolInvocation::new(
            "agena.fs.read",
            StructuredObject::try_from(json!({"file_path": "README.md"}))
                .expect("structured input"),
        );
        let output = RawOutput {
            payload: Some(json!({"loaded_paths": ["README.md"]})),
            ..RawOutput::default()
        };
        assert_eq!(
            completed_tool_title(&read, &output),
            "Read README.md · 1 file loaded"
        );

        let shell = ToolInvocation::new(
            "agena.shell.run",
            StructuredObject::try_from(json!({"command": "cargo test"})).expect("structured input"),
        );
        let output = RawOutput {
            payload: Some(json!({"exit_code": 0})),
            ..RawOutput::default()
        };
        assert_eq!(
            completed_tool_title(&shell, &output),
            "Run process · cargo test · passed"
        );

        let glob = ToolInvocation::new(
            "agena.fs.glob",
            StructuredObject::try_from(json!({"pattern": "**/*.rs"})).expect("structured input"),
        );
        let output = RawOutput {
            payload: Some(json!({
                "count": 2,
                "paths": ["src/lib.rs", "src/main.rs"]
            })),
            ..RawOutput::default()
        };
        assert_eq!(
            completed_tool_title(&glob, &output),
            "Find files · **/*.rs · 2 matches"
        );

        let grep = ToolInvocation::new(
            "agena.fs.grep",
            StructuredObject::try_from(json!({"pattern": "TODO", "path": "src"}))
                .expect("structured input"),
        );
        let output = RawOutput {
            payload: Some(json!({
                "matches": 2,
                "results": ["src/lib.rs:1: TODO", "src/main.rs:2: TODO"],
                "truncated": true
            })),
            ..RawOutput::default()
        };
        assert_eq!(
            completed_tool_title(&grep, &output),
            "Search files · TODO · src · 2 matches · truncated"
        );

        let notify = ToolInvocation::new(
            "agena.interaction.notify",
            StructuredObject::try_from(json!({
                "title": "Build",
                "level": "success",
                "body": "A very long notification body that belongs in the expanded card"
            }))
            .expect("structured input"),
        );
        assert_eq!(
            initial_tool_title(&notify),
            "Send notification · Build · success"
        );

        assert_eq!(
            super::tool_title_for_state(&shell, ToolResultState::Failed),
            "Run process · cargo test · failed"
        );
    }

    #[test]
    fn result_titles_prefer_counts_and_metadata_facts() {
        let grep_output = RawOutput {
            payload: Some(json!({
                "matches": 2,
                "results": ["a.rs:1: TODO", "b.rs:2: TODO"]
            })),
            ..RawOutput::default()
        };
        assert_eq!(result_title_fragment(&grep_output), "2 matches");

        let mut metadata = std::collections::BTreeMap::new();
        metadata.insert("exit_code".to_owned(), json!("1"));
        let metadata_output = RawOutput {
            metadata,
            ..RawOutput::default()
        };
        assert_eq!(result_title_fragment(&metadata_output), "failed · exit 1");
    }

    #[test]
    fn completed_titles_use_tool_specific_result_facts() {
        let write = ToolInvocation::new(
            "agena.fs.write",
            StructuredObject::try_from(json!({"path": "notes.txt"})).expect("structured input"),
        );
        assert_eq!(
            completed_tool_title(
                &write,
                &RawOutput {
                    payload: Some(json!({"kind": "updated", "bytes": 4})),
                    ..RawOutput::default()
                }
            ),
            "Write notes.txt · updated · 4 B"
        );

        let fetch = ToolInvocation::new(
            "agena.web.fetch",
            StructuredObject::try_from(json!({"url": "https://example.test"}))
                .expect("structured input"),
        );
        assert_eq!(
            completed_tool_title(
                &fetch,
                &RawOutput {
                    payload: Some(json!({"status": 200, "url": "https://example.test"})),
                    ..RawOutput::default()
                }
            ),
            "Fetch page https://example.test · HTTP 200"
        );

        let provider = ToolInvocation::new(
            "agena.chatgpt.web_search",
            StructuredObject::try_from(json!({"prompt": "Agena"})).expect("structured input"),
        );
        assert_eq!(
            completed_tool_title(
                &provider,
                &RawOutput {
                    payload: Some(json!({
                        "pending_calls": [{"id": "call-1"}, {"id": "call-2"}]
                    })),
                    ..RawOutput::default()
                }
            ),
            "Search web · Agena · 2 pending calls"
        );

        let settings = ToolInvocation::new(
            "agena.settings.validate",
            StructuredObject::try_from(json!({})).expect("structured input"),
        );
        assert_eq!(
            completed_tool_title(
                &settings,
                &RawOutput {
                    payload: Some(json!({"valid": true, "warnings": [{"path": "model"}]})),
                    ..RawOutput::default()
                }
            ),
            "Validate settings · valid · 1 warning"
        );
    }

    #[test]
    fn invocation_aware_completion_keeps_the_call_time_title() {
        let invocation = ToolInvocation::new(
            "agena.chatgpt.web_search",
            StructuredObject::try_from(json!({"prompt": "Agena"})).expect("input"),
        );
        let title = completed_tool_title_with_action_for_invocation(
            &invocation,
            "ChatGPT web search",
            &RawOutput {
                payload: Some(json!({"response_id": "resp-1"})),
                ..RawOutput::default()
            },
        );
        assert_eq!(title, "Search web · Agena · response received");
    }

    #[test]
    fn completed_titles_keep_shell_and_provider_results_compact() {
        let shell = ToolInvocation::new(
            "agena.shell.run",
            StructuredObject::try_from(json!({"command": "cargo test"})).expect("input"),
        );
        assert_eq!(
            completed_tool_title(
                &shell,
                &RawOutput {
                    payload: Some(json!({
                        "action": "run",
                        "status": "completed",
                        "exit_code": 0,
                        "output": "all tests passed"
                    })),
                    ..RawOutput::default()
                }
            ),
            "Run process · cargo test · passed"
        );

        let provider = ToolInvocation::new(
            "agena.chatgpt.web_search",
            StructuredObject::try_from(json!({"prompt": "Agena"})).expect("input"),
        );
        assert_eq!(
            completed_tool_title(
                &provider,
                &RawOutput {
                    payload: Some(json!({
                        "provider": "chatgpt",
                        "tool": "web_search",
                        "response_id": "resp-1",
                        "assistant_content": [{"type": "output_text", "text": "A long answer"}],
                        "sources": []
                    })),
                    text: "A long answer that should not become the collapsed title.".into(),
                    ..RawOutput::default()
                }
            ),
            "Search web · Agena · response received"
        );

        let provider_with_status = RawOutput {
            payload: Some(json!({
                "status": "completed",
                "sources": [{"title": "Guide"}, {"title": "Reference"}],
                "response_id": "resp-2"
            })),
            ..RawOutput::default()
        };
        assert_eq!(
            completed_tool_title(&provider, &provider_with_status),
            "Search web · Agena · 2 sources"
        );

        let failed_provider = RawOutput {
            payload: Some(json!({
                "status": "failed",
                "sources": [{"title": "ignored after failure"}]
            })),
            ..RawOutput::default()
        };
        assert_eq!(
            completed_tool_title(&provider, &failed_provider),
            "Search web · Agena · failed"
        );

        let image = ToolInvocation::new(
            "agena.gemini.image_generation",
            StructuredObject::try_from(json!({"prompt": "a cat"})).expect("input"),
        );
        let image_title = completed_tool_title(
            &image,
            &RawOutput {
                attachments: vec![agena_domain::AttachmentItem {
                    kind: agena_domain::AttachmentKind::Image,
                    mime: "image/png".into(),
                    source: agena_domain::AttachmentSource::LocalPath {
                        path: "/tmp/cat.png".into(),
                    },
                    filename: None,
                    title: None,
                    size_bytes: Some(10),
                    sha256: None,
                    width: None,
                    height: None,
                    duration_ms: None,
                    page_count: None,
                }],
                ..RawOutput::default()
            },
        );
        assert_eq!(
            image_title, "Generate image · a cat · 1 image",
            "{image_title}"
        );
    }

    #[test]
    fn provider_titles_surface_each_operation_result_fact() {
        let cases = [
            (
                "agena.chatgpt.file_search",
                json!({"prompt": "rendering", "tool_options": {"vector_store_ids": ["vs-1"]}}),
                json!({"results": [{"file_name": "a.md"}, {"file_name": "b.md"}]}),
                "2 files",
            ),
            (
                "agena.chatgpt.tool_search",
                json!({"prompt": "find a search tool"}),
                json!({"results": [{"name": "web.search"}, {"name": "fs.grep"}]}),
                "2 tools",
            ),
            (
                "agena.chatgpt.computer",
                json!({"prompt": "open the docs"}),
                json!({"action": {"type": "click"}, "page_title": "Agena docs"}),
                "click · Agena docs",
            ),
            (
                "agena.claude.code_execution",
                json!({"prompt": "run the tests"}),
                json!({"status": "completed", "exit_code": 0}),
                "passed",
            ),
            (
                "agena.gemini.google_maps",
                json!({"prompt": "cafes near me"}),
                json!({"places": [{"name": "One"}, {"name": "Two"}]}),
                "2 places",
            ),
            (
                "agena.gemini.retrieval",
                json!({"prompt": "find the policy"}),
                json!({"retrieved_count": 4}),
                "4 matches",
            ),
            (
                "agena.gemini.url_context",
                json!({"prompt": "read https://example.test"}),
                json!({"fetched_urls": ["https://example.test"]}),
                "1 page",
            ),
            (
                "agena.claude.web_fetch",
                json!({"prompt": "https://example.test"}),
                json!({"status": 200}),
                "HTTP 200",
            ),
            (
                "agena.claude.memory",
                json!({"prompt": "save this"}),
                json!({"saved": true}),
                "saved",
            ),
            (
                "agena.claude.advisor",
                json!({"prompt": "review this"}),
                json!({"error": {"message": "unavailable"}}),
                "error",
            ),
            (
                "agena.claude.bash",
                json!({"prompt": "pwd"}),
                json!({"exit_code": 1}),
                "failed · exit 1",
            ),
            (
                "agena.gemini.mcp_server",
                json!({"prompt": "connect", "tool_options": {"url": "https://mcp.test"}}),
                json!({"connected": true, "status": "ready"}),
                "connected",
            ),
        ];
        for (name, input, payload, expected) in cases {
            let invocation = ToolInvocation::new(
                name,
                StructuredObject::try_from(input).expect("structured input"),
            );
            let title = completed_tool_title(
                &invocation,
                &RawOutput {
                    payload: Some(payload),
                    ..RawOutput::default()
                },
            );
            assert!(
                title.contains(expected),
                "{name} omitted {expected:?}: {title}"
            );
        }
    }

    #[test]
    fn provider_input_titles_use_nested_tool_targets() {
        let invocation = ToolInvocation::new(
            "agena.chatgpt.mcp",
            StructuredObject::try_from(json!({
                "tool_options": {"server_label": "docs"}
            }))
            .expect("input"),
        );
        assert_eq!(initial_tool_title(&invocation), "Connect MCP server · docs");
    }

    #[test]
    fn completed_titles_cover_lsp_interaction_cron_monitor_web_and_discovery() {
        let cases = [
            (
                "agena.lsp.definition",
                json!({"position": {"file_path": "src/lib.rs", "line": 4, "character": 2}}),
                json!({"locations": ["src/other.rs:8:1", "src/lib.rs:20:3"]}),
                vec!["2 definitions"],
            ),
            (
                "agena.interaction.ask",
                json!({"questions": [{"question": "Continue?"}]}),
                json!({"answers": {"0": ["yes"]}}),
                vec!["1 answered"],
            ),
            (
                "agena.cron.delete",
                json!({"id": "job-1"}),
                json!({"removed": false}),
                vec!["not found"],
            ),
            (
                "agena.monitor.start",
                json!({"command": "cargo watch"}),
                json!({"action": "start", "status": "running"}),
                vec!["running"],
            ),
            (
                "agena.web.crawl",
                json!({"start_url": "https://example.test"}),
                json!({"stored_count": 3, "cached_count": 1, "failure_count": 0}),
                vec!["3 indexed", "1 cached", "0 failures"],
            ),
        ];
        for (name, input, payload, facts) in cases {
            let invocation = ToolInvocation::new(
                name,
                StructuredObject::try_from(input).expect("structured input"),
            );
            let completed = completed_tool_title(
                &invocation,
                &RawOutput {
                    payload: Some(payload),
                    ..RawOutput::default()
                },
            );
            for fact in facts {
                assert!(
                    completed.contains(fact),
                    "{name} omitted {fact}: {completed}"
                );
            }
        }
    }

    #[test]
    fn completed_titles_use_compact_text_facts_for_discovery_and_memory() {
        let tools = ToolInvocation::new(
            "agena.tools.list",
            StructuredObject::try_from(json!({})).expect("structured input"),
        );
        let title = completed_tool_title(
            &tools,
            &RawOutput::text(
                "Available tools: returned 2 of 3 starting at offset 0.\n- fs.read [filesystem] (agena.fs): Read files\nMore available.",
            ),
        );
        assert_eq!(title, "List tools · 2/3 tools · more available");

        let memory = ToolInvocation::new(
            "agena.memory.delete",
            StructuredObject::try_from(json!({"name": "old-notes"})).expect("structured input"),
        );
        assert_eq!(
            completed_tool_title(
                &memory,
                &RawOutput::text("Deleted old-notes from durable memory."),
            ),
            "Delete memory old-notes · removed"
        );
    }

    #[test]
    fn lsp_initial_titles_include_the_one_based_position() {
        let invocation = ToolInvocation::new(
            "agena.lsp.hover",
            StructuredObject::try_from(json!({
                "position": {"file_path": "src/lib.rs", "line": 9, "character": 3}
            }))
            .expect("structured input"),
        );
        assert_eq!(
            initial_tool_title(&invocation),
            "Inspect symbol src/lib.rs:10:4"
        );
    }

    #[test]
    fn completed_titles_cover_code_session_task_skill_browser_and_repo_results() {
        let check =
            |name: &str, input: serde_json::Value, payload: serde_json::Value, facts: &[&str]| {
                let invocation = ToolInvocation::new(
                    name,
                    StructuredObject::try_from(input).expect("structured input"),
                );
                let initial = initial_tool_title(&invocation);
                let completed = completed_tool_title(
                    &invocation,
                    &RawOutput {
                        payload: Some(payload),
                        ..RawOutput::default()
                    },
                );
                assert!(
                    completed.starts_with(initial.as_str()),
                    "{name} must retain its call-time title: {completed}"
                );
                for fact in facts {
                    assert!(
                        completed.contains(fact),
                        "{name} omitted result fact {fact:?}: {completed}"
                    );
                }
            };

        check(
            "agena.code.search_ast",
            json!({"pattern": "fn $NAME()", "path": "src"}),
            json!({
                "language": "rust",
                "scanned_files": 4,
                "matches": [{"path": "src/lib.rs"}, {"path": "src/main.rs"}]
            }),
            &["2 matches", "4 files scanned"],
        );
        check(
            "agena.code.syntax_tree",
            json!({"path": "src/lib.rs"}),
            json!({
                "language": "rust",
                "root_kind": "source_file",
                "has_error": true
            }),
            &["rust", "source_file", "parse errors"],
        );
        check(
            "agena.report.findings",
            json!({"summary": "Review"}),
            json!({
                "findings": [{"severity": "high"}],
                "counts": {"high": 1}
            }),
            &["1 finding", "1 high"],
        );
        check(
            "agena.session.environment",
            json!({}),
            json!({"git_branch": "main", "git_short_sha": "abc123", "git_dirty": true}),
            &["main @ abc123", "dirty"],
        );
        check(
            "agena.session.model",
            json!({}),
            json!({
                "model_provider_id": "openai",
                "model_adapter_id": "responses",
                "model_id": "gpt-5"
            }),
            &["openai/responses/gpt-5"],
        );
        check(
            "agena.session.tokens",
            json!({}),
            json!({"current_tokens": 12000, "remaining_tokens": 5000}),
            &["12000 used", "5000 remaining"],
        );
        check(
            "agena.tasks.list",
            json!({}),
            json!({
                "tasks": [
                    {"task_id": "t-1", "status": "completed"},
                    {"task_id": "t-2", "status": "running"}
                ]
            }),
            &["2 tasks", "1 completed", "1 running"],
        );
        check(
            "agena.tasks.output",
            json!({"task_id": "t-1"}),
            json!({
                "task": {"task_id": "t-1", "status": "completed"},
                "chunks": [{"text": "done"}, {"text": "verified"}],
                "has_more": true
            }),
            &["2 chunks", "completed", "more available"],
        );
        check(
            "agena.skills.create",
            json!({"document": "---\nname: team_review\n---\nReview."}),
            json!({"operation": "created", "name": "team_review", "catalog_generation": 3}),
            &["created", "team_review"],
        );
        check(
            "agena.settings.inspect",
            json!({"path": "providers.openai"}),
            json!({"path": "providers.openai"}),
            &["providers.openai", "inspected"],
        );
        check(
            "agena.notebook.edit_cell",
            json!({"path": "demo.ipynb", "cell_index": 2}),
            json!({"action": "replace", "cell_index": 2, "cell_count": 4}),
            &["replace", "cell 2", "4 cells"],
        );
        check(
            "agena.browser_snapshot",
            json!({"session_id": "session-1"}),
            json!({
                "session_id": "session-1",
                "snapshot": {
                    "title": "Agena docs",
                    "elements": [{"ref": "e1"}]
                }
            }),
            &["Agena docs", "1 interactive element"],
        );
        check(
            "agena.repo.status",
            json!({}),
            json!({
                "dirty": true,
                "changes": [{"path": "src/lib.rs"}]
            }),
            &["1 file changed", "dirty"],
        );
    }

    #[test]
    fn completed_titles_use_metadata_only_facts_and_terminal_states() {
        let invocation = ToolInvocation::new(
            "agena.shell.run",
            StructuredObject::try_from(json!({"command": "cargo test"})).expect("structured input"),
        );
        let metadata = std::collections::BTreeMap::from([("exit_code".to_owned(), json!(0))]);
        let title = completed_tool_title(
            &invocation,
            &RawOutput {
                metadata,
                ..RawOutput::default()
            },
        );
        assert_eq!(title, "Run process · cargo test · passed");

        let timeout = completed_tool_title(
            &ToolInvocation::new(
                "agena.tasks.output",
                StructuredObject::try_from(json!({"task_id": "t-1"})).expect("structured input"),
            ),
            &RawOutput {
                payload: Some(json!({"timed_out": true})),
                ..RawOutput::default()
            },
        );
        assert!(timeout.ends_with("timed out"), "{timeout}");

        assert_eq!(
            super::completed_tool_title_for_state(
                &invocation,
                ToolResultState::Cancelled,
                &RawOutput::default(),
            ),
            "Run process · cargo test · cancelled"
        );
    }

    #[test]
    fn completed_titles_merge_payload_metadata_and_truncation_facts() {
        let invocation = ToolInvocation::new(
            "agena.shell.run",
            StructuredObject::try_from(json!({"command": "cargo test"})).expect("input"),
        );

        let metadata = std::collections::BTreeMap::from([
            ("exit_code".to_owned(), json!(1)),
            ("truncated".to_owned(), json!(true)),
        ]);
        let title = completed_tool_title(
            &invocation,
            &RawOutput {
                payload: Some(json!({"status": "completed"})),
                metadata,
                ..RawOutput::default()
            },
        );
        assert_eq!(
            title,
            "Run process · cargo test · failed · exit 1 · truncated"
        );

        let generic = result_title_fragment(&RawOutput {
            payload: Some(json!({"status": "completed"})),
            metadata: std::collections::BTreeMap::from([("exit_code".to_owned(), json!(1))]),
            ..RawOutput::default()
        });
        assert_eq!(generic, "failed · exit 1");
    }

    #[test]
    fn lifecycle_state_is_visible_when_raw_output_is_partial_or_empty() {
        let shell = ToolInvocation::new(
            "agena.shell.run",
            StructuredObject::try_from(json!({"command": "cargo test"})).expect("structured input"),
        );
        assert_eq!(
            super::completed_tool_title_for_state(
                &shell,
                ToolResultState::Failed,
                &RawOutput {
                    text: "process stopped before an exit code was recorded".into(),
                    ..RawOutput::default()
                },
            ),
            "Run process · cargo test · process stopped before an exit code was recorded · failed"
        );
        assert_eq!(
            super::completed_tool_title_for_state(
                &shell,
                ToolResultState::Completed,
                &RawOutput::default(),
            ),
            "Run process · cargo test · completed"
        );
        assert_eq!(
            super::completed_tool_title_for_state(
                &shell,
                ToolResultState::Running,
                &RawOutput {
                    payload: Some(json!({"exit_code": 0})),
                    ..RawOutput::default()
                },
            ),
            "Run process · cargo test"
        );
    }

    #[test]
    fn raw_tool_name_spellings_are_not_custom_action_titles() {
        let invocation = ToolInvocation::new(
            "agena.shell.run",
            StructuredObject::try_from(json!({"command": "cargo test"})).expect("structured input"),
        );
        assert!(is_tool_identity_title("shell.run", &invocation));
        assert!(is_tool_identity_title("agena_shell_run", &invocation));
        assert!(!is_tool_identity_title(
            "Run process · cargo test",
            &invocation
        ));
    }
}

pub mod code_search;
pub mod shell;
pub mod shell_analysis;
pub mod tool_search;

/// Rendering strategy for the built-in read tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum ReadMode {
    Text,
    Attachment,
    #[default]
    Auto,
}

/// Optional provider/model selection overrides for a delegated task.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct TaskModelSelection {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adapter: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verbosity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parallel_tool_calls: Option<bool>,
}

/// One file-level change produced by the apply-patch tool.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppliedFileChange {
    pub path: String,
    pub kind: PatchOpKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_path: Option<String>,
}

/// Stable result metadata emitted after an apply-patch operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApplyPatchExecution {
    pub operation_id: String,
    pub files: Vec<AppliedFileChange>,
    pub before_hash: String,
    pub after_hash: String,
    pub inverse_patch: String,
    pub diff: String,
    pub progress: Vec<String>,
}

impl ApplyPatchExecution {
    /// Decode the stable generic tool payload emitted by `fs.apply_patch`.
    ///
    /// Operator transports carry [`ToolExecutionSummary`] rather than
    /// executor-private result types. Keeping this projection here gives
    /// every client the same interpretation of the public payload.
    pub fn from_tool_payload(payload: &serde_json::Value) -> Option<Self> {
        let operation_id = payload.get("operation_id")?.as_str()?.to_owned();
        let changes: Vec<agena_domain::FileChangeRecord> = serde_json::from_value(
            payload
                .get("changes")
                .cloned()
                .unwrap_or_else(|| serde_json::json!([])),
        )
        .ok()?;
        let before_hash = payload
            .get("before_hash")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let after_hash = payload
            .get("after_hash")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let inverse_patch = payload.get("inverse_patch")?.as_str()?.to_owned();
        let diff = payload
            .get("diff")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let progress = serde_json::from_value(
            payload
                .get("progress")
                .cloned()
                .unwrap_or_else(|| serde_json::json!([])),
        )
        .ok()?;
        Some(Self {
            operation_id,
            files: changes
                .into_iter()
                .map(|change| AppliedFileChange {
                    path: change.path,
                    kind: match change.kind {
                        agena_domain::FileChangeKind::Added => PatchOpKind::Add,
                        agena_domain::FileChangeKind::Updated => PatchOpKind::Update,
                        agena_domain::FileChangeKind::Deleted => PatchOpKind::Delete,
                        agena_domain::FileChangeKind::Moved => PatchOpKind::Move,
                    },
                    from_path: change.from_path,
                })
                .collect(),
            before_hash,
            after_hash,
            inverse_patch,
            diff,
            progress,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// Kind of a patch operation.
pub enum PatchOpKind {
    Add,
    Update,
    Delete,
    Move,
}

/// Model-facing builtin-tool availability profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuiltinToolProfile {
    Full,
    ReadOnly,
    NoTask,
}

impl BuiltinToolProfile {
    pub fn infer(model_id: Option<&str>) -> Self {
        let Some(model_id) = model_id else {
            return Self::Full;
        };
        let lowered = model_id.to_ascii_lowercase();
        if lowered.contains("readonly") || lowered.contains("read_only") {
            return Self::ReadOnly;
        }
        if lowered.contains("no-task") || lowered.contains("chat") {
            return Self::NoTask;
        }
        Self::Full
    }
}

/// Snapshot backend selected by the concrete repository/snapshot adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotBackend {
    Rift,
    Git,
}

impl AsRef<str> for SnapshotBackend {
    fn as_ref(&self) -> &str {
        match self {
            Self::Rift => "rift",
            Self::Git => "git",
        }
    }
}

impl std::fmt::Display for SnapshotBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_ref())
    }
}

#[derive(Debug, Clone)]
/// Whether a snapshot backend is available and why.
pub struct SnapshotBackendSupport {
    pub backend: SnapshotBackend,
    pub available: bool,
    pub detail: String,
}

#[derive(Debug, Clone)]
/// Capabilities of the available snapshot backends.
pub struct SnapshotBackendCapabilities {
    pub preferred_backend: Option<SnapshotBackend>,
    pub git: SnapshotBackendSupport,
    pub rift: SnapshotBackendSupport,
}

impl SnapshotBackendCapabilities {
    pub fn for_backend(&self, backend: SnapshotBackend) -> &SnapshotBackendSupport {
        match backend {
            SnapshotBackend::Rift => &self.rift,
            SnapshotBackend::Git => &self.git,
        }
    }
}

/// Presentation-neutral availability result for one builtin tool.
#[derive(Debug, Clone)]
pub struct ToolAvailability {
    pub tool_name: String,
    pub enabled: bool,
    pub reason: String,
}

/// Runtime-neutral summary of one completed tool execution.
///
/// Concrete executors may attach core transcript parts or file attachments;
/// those remain outside this contract. This value carries only the stable
/// textual presentation and metadata that a runtime/application boundary can
/// forward without depending on message or UI types.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ToolExecutionSummary {
    pub title: String,
    pub summary: String,
    pub output_text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<ToolAttachmentSummary>,
}

/// Attachment metadata that can cross the executor/runtime boundary without
/// carrying the plugin SDK's concrete attachment source type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolAttachmentSummary {
    pub kind: String,
    pub mime: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_hint: Option<String>,
}

/// Stable model-facing summary for one scheduled cron job.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CronJobSummary {
    pub id: String,
    pub kind: String,
    pub expression: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timezone: Option<String>,
    pub at: Option<String>,
    pub prompt: String,
    pub next_fire_at: Option<String>,
    pub last_fired_at: Option<String>,
    #[serde(default)]
    pub paused: bool,
    #[serde(default)]
    pub completed: bool,
    #[serde(default)]
    pub misfire_policy: String,
    #[serde(default)]
    pub retry_max_attempts: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_at: Option<String>,
    #[serde(default)]
    pub run_count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run_status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run_failure: Option<agena_failure::UserProblem>,
}

/// Stable history entry emitted by `cron.history`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CronRunSummary {
    pub job_id: String,
    pub triggered_at: String,
    pub finished_at: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scheduled_for: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<agena_failure::UserProblem>,
}

/// A permission decision attached to one tool access action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolPermissionCheck {
    pub action: PermissionAction,
    pub decision: PermissionDecision,
    /// The tool's full permission contract: shell/interactive/read_only/task
    /// flags plus declared path/network specs. The decision pipeline reads
    /// these directly; never tool tags (tags are metadata for discovery/UI).
    pub contract: ToolPermissionContract,
}

impl ToolPermissionCheck {
    /// Whether the contract is path-scoped (declares concrete path specs).
    pub fn is_path_scoped(&self) -> bool {
        !self.contract.input_paths.is_empty() || !self.contract.path_access.is_empty()
    }
}

/// Invocation after tool lookup/presentation has prepared it for execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedToolInvocation {
    pub invocation: ToolInvocation,
    pub title_override: Option<String>,
    pub metadata: BTreeMap<String, String>,
}

/// Shell invocation after path/working-directory preparation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedShellCommand {
    pub command: String,
    pub cwd: std::path::PathBuf,
    /// Fully resolved environment after `shell.env` and `command.before`
    /// hooks. Carrying it with the prepared command prevents execution from
    /// re-entering synchronous plugin hooks on a blocking worker.
    pub env: std::collections::HashMap<String, String>,
}

/// Maximum-character policy applied by concrete tool-output truncators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolOutputTruncationPolicy {
    pub max_chars: usize,
}

impl Default for ToolOutputTruncationPolicy {
    fn default() -> Self {
        Self {
            max_chars: usize::MAX,
        }
    }
}

#[derive(Debug, Clone)]
/// Request to execute a shell command.
pub struct ShellRequest {
    pub command: Vec<String>,
    pub cwd: std::path::PathBuf,
    pub env: std::collections::HashMap<String, String>,
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone)]
/// Output of a shell command execution.
pub struct ShellOutput {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub aggregated_output: String,
    pub duration: Duration,
    pub timed_out: bool,
}

/// Runtime-side presentation signals emitted while a process-backed tool is
/// running. The session runtime owns delivery (and decides whether the
/// signals are ephemeral or durable); this crate only exposes a small callback
/// contract so stdout/stderr can be observed without waiting for the child
/// process to exit.
#[derive(Debug, Clone)]
pub enum ToolRuntimeEvent {
    CommandBegin(CommandBeginEvent),
    CommandOutputDelta(CommandOutputDeltaEvent),
    CommandEnd(CommandEndEvent),
}

/// Sink receiving tool runtime events.
pub type ToolRuntimeEventSink = Arc<dyn Fn(ToolRuntimeEvent) + Send + Sync>;

#[derive(Debug, thiserror::Error)]
/// Error from shell command execution.
pub enum ShellError {
    #[error("command cancelled")]
    Cancelled,
    #[error("invalid shell request: {0}")]
    InvalidRequest(String),
    #[error("failed to spawn child process: {0}")]
    Spawn(#[source] std::io::Error),
    #[error("failed to wait for child process: {0}")]
    Wait(#[source] std::io::Error),
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{
        ApplyPatchExecution, BuiltinToolProfile, PatchOpKind, PreparedShellCommand,
        SnapshotBackend, SnapshotBackendCapabilities, SnapshotBackendSupport,
        ToolAttachmentSummary, ToolAvailability, ToolExecutionSummary, ToolOutputTruncationPolicy,
    };

    #[test]
    fn truncation_policy_defaults_to_unbounded_output() {
        assert_eq!(ToolOutputTruncationPolicy::default().max_chars, usize::MAX);
    }

    #[test]
    fn prepared_shell_command_keeps_command_and_directory() {
        let command = PreparedShellCommand {
            command: "echo ok".to_string(),
            cwd: std::path::PathBuf::from("/tmp"),
            env: std::collections::HashMap::new(),
        };
        assert_eq!(command.command, "echo ok");
        assert_eq!(command.cwd, std::path::Path::new("/tmp"));
    }

    #[test]
    fn builtin_profile_inference_is_provider_independent() {
        assert_eq!(BuiltinToolProfile::infer(None), BuiltinToolProfile::Full);
        assert_eq!(
            BuiltinToolProfile::infer(Some("chat-readonly")),
            BuiltinToolProfile::ReadOnly
        );
        assert_eq!(
            BuiltinToolProfile::infer(Some("model-no-task")),
            BuiltinToolProfile::NoTask
        );
    }

    #[test]
    fn snapshot_capabilities_select_the_requested_backend() {
        let capabilities = SnapshotBackendCapabilities {
            preferred_backend: Some(SnapshotBackend::Rift),
            git: SnapshotBackendSupport {
                backend: SnapshotBackend::Git,
                available: false,
                detail: "missing git".to_owned(),
            },
            rift: SnapshotBackendSupport {
                backend: SnapshotBackend::Rift,
                available: true,
                detail: "ready".to_owned(),
            },
        };
        assert!(capabilities.for_backend(SnapshotBackend::Rift).available);
        assert!(!capabilities.for_backend(SnapshotBackend::Git).available);
    }

    #[test]
    fn availability_value_carries_only_presentation_neutral_fields() {
        let value = ToolAvailability {
            tool_name: "read".to_owned(),
            enabled: true,
            reason: "read-only profile".to_owned(),
        };
        assert!(value.enabled);
        assert_eq!(value.tool_name, "read");
    }

    #[test]
    fn execution_summary_round_trips_without_core_types() {
        let value = ToolExecutionSummary {
            title: "Read README".to_owned(),
            summary: "README.md · 1 file".to_owned(),
            output_text: "content".to_owned(),
            payload: Some(serde_json::json!({"kind": "read"})),
            metadata: BTreeMap::from([(String::from("path"), String::from("README.md"))]),
            attachments: Vec::new(),
        };
        let json = serde_json::to_value(&value).expect("serialize execution summary");
        let decoded: ToolExecutionSummary =
            serde_json::from_value(json).expect("deserialize execution summary");
        assert_eq!(decoded, value);
    }

    #[test]
    fn execution_summary_requires_the_result_summary_contract() {
        let error = serde_json::from_value::<ToolExecutionSummary>(serde_json::json!({
            "title": "legacy",
            "output_text": "output"
        }))
        .expect_err("summary is a required execution-result field");
        assert!(error.to_string().contains("summary"));
    }

    #[test]
    fn apply_patch_execution_decodes_the_generic_operator_payload() {
        let execution = ApplyPatchExecution::from_tool_payload(&serde_json::json!({
            "operation_id": "operation-1",
            "changes": [{"path": "new.txt", "kind": "added"}],
            "before_hash": "before",
            "after_hash": "after",
            "inverse_patch": "*** Begin Patch\n*** Delete File: new.txt\n*** End Patch",
            "diff": "+new",
            "progress": ["added new.txt"]
        }))
        .expect("decode apply-patch operator payload");

        assert_eq!(execution.operation_id, "operation-1");
        assert_eq!(execution.files.len(), 1);
        assert_eq!(execution.files[0].path, "new.txt");
        assert_eq!(execution.files[0].kind, PatchOpKind::Add);
        assert_eq!(execution.before_hash, "before");
        assert_eq!(execution.after_hash, "after");
    }

    #[test]
    fn attachment_summary_has_a_stable_wire_shape() {
        let value = ToolAttachmentSummary {
            kind: "file".to_owned(),
            mime: "text/plain".to_owned(),
            label: "README.md".to_owned(),
            size_bytes: Some(12),
            source_hint: Some("README.md".to_owned()),
        };
        assert_eq!(
            serde_json::to_value(value).expect("serialize attachment summary"),
            serde_json::json!({
                "kind": "file",
                "mime": "text/plain",
                "label": "README.md",
                "size_bytes": 12,
                "source_hint": "README.md"
            })
        );
    }
}
