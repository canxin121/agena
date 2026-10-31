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
//!   [`ToolPermissionCheck`], and [`ToolExecutionSummary`].
//! - **Shell** — [`shell`] provides [`ShellRequest`] / [`ShellOutput`] and
//!   [`ShellError`]; [`shell_analysis`] analyzes command shapes.
//! - **Search** — [`code_search`] and [`tool_search`] locate code and tools.
//! - **Value types** — [`ReadMode`],
//!   [`ToolAvailability`], patch operations, and cron summaries.

use std::collections::BTreeMap;
use std::time::Duration;

pub mod tool_activity;
pub mod tool_title_locale;
use agena_domain::{
    PermissionAction, PermissionDecision, RawOutput, ToolInvocation, ToolResultState,
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
pub use tool_title_locale::localize_tool_title;

/// Compose one compact tool headline from an action and a detail fragment
/// (for example `Execute command · cargo test` or
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
/// `Read README.md`, `Search files · TODO`, or `Execute command · cargo test`.
pub fn initial_tool_title(invocation: &ToolInvocation) -> String {
    let input = serde_json::Value::from(invocation.input.clone());
    render_invocation_title(invocation.name.as_str(), &input)
}

/// Return whether a renderer supplied only one spelling of the invocation
/// identity (for example `shell.exec`, `agena.shell.exec`, or
/// `agena_shell_exec`) instead of a human action title.  Execution adapters
/// commonly use the raw name as their fallback; treating it as a custom title
/// would make the completed headline regress to `agena.shell.exec · passed`.
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
    complete_title(
        initial_tool_title(invocation).as_str(),
        result_title_fragment_for_tool(invocation.name.as_str(), output),
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
    // A partial/streaming raw output is not a terminal result. Keep the
    // call-time title until the lifecycle itself reaches a terminal state;
    // otherwise a checkpoint could briefly claim success from an incomplete
    // payload.
    if matches!(state, ToolResultState::Pending | ToolResultState::Running) {
        return initial_tool_title(invocation);
    }
    let title = completed_tool_title(invocation, output);
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

    if let Some(object) = serde_json::from_str::<serde_json::Value>(output.text_content().trim())
        .ok()
        .and_then(|value| value.as_object().cloned())
        && let Some(fragment) = result_title_fragment_from_object(&object)
    {
        candidates.push(fragment);
    }

    if let Some(fragment) = choose_result_fragment(candidates) {
        return finalize_result_fragment(fragment, output);
    }

    if !output.text_content().trim().is_empty()
        && let Some(value) = compact_result_text(output.text_content())
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
    if (key == "image_generation"
        || (is_provider_tool_identity(key.as_str()) && key.contains("image")))
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

    if let Some(object) = serde_json::from_str::<serde_json::Value>(output.text_content().trim())
        .ok()
        .and_then(|value| value.as_object().cloned())
        && let Some(fragment) = tool_result_fragment(key.as_str(), &object)
    {
        candidates.push(fragment);
    }
    if let Some(fragment) = tool_result_fragment_from_text(key.as_str(), output.text_content()) {
        candidates.push(fragment);
    }
    if is_provider_tool_identity(key.as_str()) && !output.text_content().trim().is_empty() {
        candidates.push("response received".to_owned());
    }
    if let Some(fragment) = choose_result_fragment(candidates) {
        return finalize_result_fragment(fragment, output);
    }
    finalize_result_fragment(result_title_fragment(output), output)
}

/// Pick the most useful result fact from the structured payload and text.
/// A bare lifecycle marker
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
        || serde_json::from_str::<serde_json::Value>(output.text_content().trim())
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

/// Stable result facts for file discovery and text search. Headline and detail
/// renderers share counts and completeness; row layout and output limits stay
/// with each renderer. This reads only documented result fields and never
/// copies file content into a summary.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileSearchResultFacts {
    pub count: Option<u64>,
    pub truncated: bool,
    pub scan_complete: Option<bool>,
    matching_files: bool,
}

impl FileSearchResultFacts {
    pub fn glob(object: &serde_json::Map<String, serde_json::Value>) -> Self {
        Self {
            count: object.get("count").and_then(value_as_u64).or_else(|| {
                object
                    .get("paths")
                    .and_then(serde_json::Value::as_array)
                    .map(|paths| paths.len() as u64)
            }),
            truncated: object.get("truncated").and_then(serde_json::Value::as_bool) == Some(true),
            ..Self::default()
        }
    }

    pub fn grep(object: &serde_json::Map<String, serde_json::Value>) -> Self {
        let mode = match object.get("mode") {
            None => "content",
            Some(value) => value.as_str().unwrap_or(""),
        };
        let scan_complete = object
            .get("scan_complete")
            .and_then(serde_json::Value::as_bool);
        let records = object.get("records").and_then(serde_json::Value::as_array);
        let legacy_results = object.get("results").and_then(serde_json::Value::as_array);
        let count = object.get("matches").and_then(value_as_u64).or_else(|| {
            if let Some(records) = records.filter(|records| !records.is_empty()) {
                // Context lines are not matches. Files/count modes report
                // matching files; a Count record's count is a per-file line
                // count (possibly partial), not the number of result files.
                // Unknown future record kinds must not become a false zero.
                return records.iter().try_fold(0_u64, |count, record| {
                    let kind = record.get("kind")?.as_str()?;
                    let matched = match (mode, kind) {
                        ("content", "line") => record.get("matched")?.as_bool()?,
                        ("files", "file") | ("count", "count") => true,
                        _ => return None,
                    };
                    Some(count + u64::from(matched))
                });
            }
            // Empty arrays are omitted by the current typed payload. An
            // explicit scan marker still establishes an empty searched
            // portion, while an opaque/missing result remains unknown.
            legacy_results
                .map(|results| results.len() as u64)
                .or_else(|| {
                    (matches!(mode, "content" | "files" | "count")
                        && object
                            .get("records")
                            .is_none_or(serde_json::Value::is_array)
                        && object
                            .get("results")
                            .is_none_or(serde_json::Value::is_array)
                        && (records.is_some() || scan_complete.is_some()))
                    .then_some(0)
                })
        });
        Self {
            count,
            truncated: object.get("truncated").and_then(serde_json::Value::as_bool) == Some(true),
            scan_complete,
            matching_files: matches!(mode, "files" | "count"),
        }
    }

    pub fn count_text(&self) -> Option<String> {
        let count = self.count?;
        let label = match (self.matching_files, count) {
            (true, 1) => "matching file",
            (true, _) => "matching files",
            (false, 1) => "match",
            (false, _) => "matches",
        };
        Some(format!("{count} {label}"))
    }

    pub fn result_fragment(&self) -> Option<String> {
        let mut fragment = self.count_text()?;
        if self.truncated {
            fragment.push_str(" · truncated");
        } else if self.scan_complete == Some(false) {
            fragment.push_str(" · incomplete");
        }
        Some(fragment)
    }
}

fn tool_result_fragment(
    key: &str,
    object: &serde_json::Map<String, serde_json::Value>,
) -> Option<String> {
    // Shell payloads carry both a lifecycle status and the more useful exit
    // code/process facts. Prefer the latter so `shell.exec` ends as `passed`
    // or `failed · exit 1`, while list/logs/stop calls still report what they
    // actually returned.
    if (key == "shell"
        || (key.starts_with("shell.")
            && agena_domain::ToolPermissionConfig::canonical_shell_tool_name(key).is_some()))
        && let Some(fragment) = shell_result_fragment(key, object)
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
        "code.rewrite_ast" => {
            if let Some(count) = object.get("replacements").and_then(value_as_u64) {
                let status =
                    if object.get("applied").and_then(serde_json::Value::as_bool) == Some(true) {
                        "applied"
                    } else {
                        "preview"
                    };
                return Some(format!("{} replacements · {status}", count));
            }
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
        "web.search" | "web.crawl" | "web.query" | "web.fetch_many" => {
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
        "agent.spawn" | "agent.message" | "agent.wait" | "agent.followup" | "agent.cancel" => {
            return agent_result_fragment(key, object);
        }
        "commands.install"
        | "commands.remove"
        | "commands.get"
        | "commands.list"
        | "commands.read_resource"
        | "commands.refresh" => {
            if let Some(fragment) = command_result_fragment(key, object) {
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
            if let Some(fragment) = FileSearchResultFacts::glob(object).result_fragment() {
                return Some(fragment);
            }
        }
        "fs.grep" => {
            if let Some(fragment) = FileSearchResultFacts::grep(object).result_fragment() {
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
        "fs.document" => {
            if let Some(lines) = object.get("matching_lines").and_then(value_as_u64) {
                let boundary =
                    if object.get("truncated").and_then(serde_json::Value::as_bool) == Some(true) {
                        " · truncated"
                    } else {
                        ""
                    };
                return Some(format!("{lines} matching lines{boundary}"));
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
        "chatgpt.image_generate"
        | "chatgpt.responses_image_generation"
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
        "web.fetch" | "web.read" | "web_fetch" | "claude.web_fetch" => {
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
            if let Some(status) = object
                .get("status")
                .or_else(|| object.get("state"))
                .and_then(serde_json::Value::as_str)
                .and_then(normalize_result_status)
                && matches!(
                    status.as_str(),
                    "failed" | "cancelled" | "timed out" | "stopped"
                )
            {
                return Some(status);
            }
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
    let key = provider_tools::canonical_cloud_identity(key);
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
        "chatgpt.file_search" | "gemini.file_search" => count(
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
        "chatgpt.shell" => provider_execution_result_fragment(object),
        "chatgpt.image_generate"
        | "chatgpt.responses_image_generation"
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
    let total = total?;
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

fn agent_result_fragment(
    key: &str,
    object: &serde_json::Map<String, serde_json::Value>,
) -> Option<String> {
    let state = object
        .get("status")
        .or_else(|| object.get("state"))
        .or_else(|| object.get("reason"))
        .and_then(serde_json::Value::as_str)
        .and_then(normalize_result_status);
    state.or_else(|| {
        Some(
            match key {
                "agent.spawn" => "started",
                "agent.message" => "message queued",
                "agent.wait" => "awakened",
                "agent.followup" => "follow-up accepted",
                "agent.cancel" => "cancelled",
                _ => return None,
            }
            .to_owned(),
        )
    })
}

fn command_result_fragment(
    key: &str,
    object: &serde_json::Map<String, serde_json::Value>,
) -> Option<String> {
    match key {
        "commands.install" | "commands.remove" => {
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
        "commands.list" => {
            let returned = object.get("returned").and_then(value_as_u64);
            if let Some(returned) = returned {
                let total = object.get("total").and_then(value_as_u64);
                return Some(match total {
                    Some(total) => format!("{returned} of {total} packages"),
                    None => format_count(returned as usize, "packages"),
                });
            }
        }
        "commands.get" => {
            let name = object.get("name").and_then(serde_json::Value::as_str);
            let source = object.get("source").and_then(serde_json::Value::as_str);
            let parts = name
                .into_iter()
                .chain(source)
                .map(normalize_tool_title)
                .collect::<Vec<_>>();
            if !parts.is_empty() {
                return Some(parts.join(" · "));
            }
        }
        "commands.read_resource" => {
            let path = object.get("path").and_then(serde_json::Value::as_str);
            let bytes = object.get("bytes").and_then(value_as_u64);
            let parts = path
                .into_iter()
                .map(normalize_tool_title)
                .chain(bytes.map(format_bytes))
                .collect::<Vec<_>>();
            if !parts.is_empty() {
                return Some(parts.join(" · "));
            }
        }
        "commands.refresh" => {
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
        "web.search" | "web.query" => {
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
        "web.fetch_many" => object
            .get("usable_count")
            .and_then(value_as_u64)
            .map(|count| format!("{count} readable pages")),
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
    if let Some(tool) = provider_tools::cloud_tool(&key) {
        return tool.title.to_owned();
    }
    match key.as_str() {
        "fs.read" | "read" => "Read".to_owned(),
        "fs.read_many" => "Read files".to_owned(),
        "fs.write" | "write" => "Write".to_owned(),
        "fs.apply_patch" | "apply_patch" => "Apply patch".to_owned(),
        "fs.glob" | "glob" => "Find files".to_owned(),
        "fs.grep" | "grep" => "Search files".to_owned(),
        "fs.replace" => "Replace text".to_owned(),
        "fs.stat" => "Inspect file".to_owned(),
        "code.search_ast" => "Search AST".to_owned(),
        "code.syntax_tree" => "Inspect syntax tree".to_owned(),
        "code.rewrite_ast" => "Rewrite AST".to_owned(),
        "fs.document" => "Read document".to_owned(),
        "shell" => "Run command".to_owned(),
        "shell.exec" => "Execute command".to_owned(),
        "shell.spawn" => "Spawn background command".to_owned(),
        "shell.watch" => "Watch command output".to_owned(),
        "shell.open" => "Open interactive terminal".to_owned(),
        "shell.read" => "Read terminal output".to_owned(),
        "shell.list" => "List processes".to_owned(),
        "shell.logs" => "Show process logs".to_owned(),
        "shell.stop" => "Stop process".to_owned(),
        "shell.write" => "Interact with terminal".to_owned(),
        "shell.resize" => "Resize terminal".to_owned(),
        "shell.signal" => "Signal terminal".to_owned(),
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
        "web.fetch_many" => "Read pages".to_owned(),
        "web.read" => "Continue reading page".to_owned(),
        "web.query" => "Search crawled pages".to_owned(),
        "web.search" | "web_search" | "chatgpt.web_search" | "claude.web_search"
        | "gemini.web_search" => "Search web".to_owned(),
        "claude.web_fetch" => "Fetch page".to_owned(),
        "gemini.google_search" => "Search web".to_owned(),
        "gemini.google_maps" => "Search maps".to_owned(),
        "gemini.url_context" => "Read web context".to_owned(),
        "chatgpt.code_interpreter" | "claude.code_execution" | "gemini.code_execution" => {
            "Run code".to_owned()
        }
        "bash" | "local_shell" => "Run process".to_owned(),
        "chatgpt.shell" => "Run hosted shell".to_owned(),
        "claude.advisor" => "Consult advisor".to_owned(),
        "chatgpt.file_search" | "gemini.file_search" => "Search files".to_owned(),
        "chatgpt.responses_image_generation" => "Generate image via OpenAI Responses".to_owned(),
        "image_generation" | "chatgpt.image_generate" | "gemini.image_generation" => {
            "Generate image".to_owned()
        }
        "chatgpt.image_edit" | "gemini.image_edit" => "Edit image".to_owned(),
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
        "agent.spawn" => "Start agent".to_owned(),
        "agent.cancel" => "Cancel agent".to_owned(),
        "agent.followup" => "Continue agent".to_owned(),
        "agent.message" => "Message agent".to_owned(),
        "agent.wait" => "Wait for agents".to_owned(),
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
        "commands.install" => "Install command".to_owned(),
        "commands.remove" => "Remove command".to_owned(),
        "commands.get" => "Inspect command".to_owned(),
        "commands.list" => "List commands".to_owned(),
        "commands.read_resource" => "Read command resource".to_owned(),
        "commands.refresh" => "Refresh commands".to_owned(),
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

/// Normalize a tool identity for titles and human presentation.
///
/// This accepts historical display spellings and folds case. It must not be
/// used for registry lookup, execution dispatch, or permission decisions.
pub fn normalized_tool_identity(tool_name: &str) -> String {
    // Exact cloud identities retain underscores in the action portion of a
    // wire name; replacing every underscore would erase the cloud marker.
    if let Some(tool) = provider_tools::cloud_tool(tool_name) {
        return tool.name.to_owned();
    }

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

/// Select a compact, display-safe invocation subject using the same rules as
/// [`initial_tool_title`]. The source input is borrowed and remains unchanged;
/// clients keep the complete original input for details and copying.
pub fn invocation_title_subject(tool_name: &str, input: &serde_json::Value) -> String {
    let key = normalized_tool_identity(tool_name);
    if key == "image_generation" {
        // A hosted image prompt is a document-sized instruction, not a
        // compact subject. Its result description belongs beside the image;
        // the original prompt remains available in the input details.
        return String::new();
    }
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
    } else if key.ends_with("code.rewrite_ast") {
        &["path", "pattern", "replacement", "apply"]
    } else if key.ends_with("code.syntax_tree") {
        &["path", "language"]
    } else if key.ends_with("report.findings") {
        &["summary"]
    } else if ["shell.exec", "shell.spawn", "shell.watch", "shell.open"]
        .iter()
        .any(|name| key.ends_with(name))
    {
        &["command", "description"]
    } else if [
        "shell.logs",
        "shell.read",
        "shell.write",
        "shell.stop",
        "shell.signal",
        "shell.resize",
    ]
    .iter()
    .any(|name| key.ends_with(name))
    {
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
        || key.ends_with("notebook.edit_cell")
    {
        &["file_path", "path", "notebook_path", "name"]
    } else if key.ends_with("fs.grep") || key.ends_with("fs.glob") {
        &["pattern", "path", "include"]
    } else if key.ends_with("web.search")
        || key.ends_with("web.query")
        || key.ends_with("web_search")
    {
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
    } else if key.ends_with("agent.spawn") || key.ends_with("plan.update") {
        // An agent/plan prompt is an instruction payload, not a compact
        // operation target. It may contain secrets, a whole user request, or
        // an arbitrarily large document; keep it in the expanded input only.
        &["name", "title", "phase"]
    } else if key.starts_with("agent.") {
        &["target", "name", "timeout_ms"]
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
    } else if key.starts_with("commands.") {
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
            "agent_id",
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
    let key = provider_tools::canonical_cloud_identity(key);
    let Some(object) = input.as_object() else {
        return String::new();
    };

    // Provider envelopes can contain many routing fields at once (model,
    // environment, connector, schema, and the actual operation target). A
    // title should name the operation's target, not the first two arbitrary
    // strings in that envelope.
    let preferred: &[&str] = if key.contains("web_search") || key.ends_with("google_search") {
        &["query", "prompt", "q"]
    } else if key.ends_with("google_maps")
        || key.ends_with("retrieval")
        || key.ends_with("file_search")
        || key.contains("tool_search")
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
    } else if key.ends_with("image_generate")
        || key.ends_with("image_generation")
        || key.ends_with("image_edit")
    {
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
        "agent_id",
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

pub mod cli_catalog;
pub mod code_search;
pub mod file_io;
pub mod path_filter;
pub mod provider_tools;
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

/// One file-level change produced by the apply-patch tool.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppliedFileChange {
    pub path: String,
    pub kind: PatchOpKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_path: Option<String>,
}

/// Result metadata emitted after an apply-patch operation.
/// Completed changes are recovered through Git, not a generated inverse patch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApplyPatchExecution {
    pub operation_id: String,
    pub files: Vec<AppliedFileChange>,
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
    NoAgent,
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
        if lowered.contains("no-agent") || lowered.contains("chat") {
            return Self::NoAgent;
        }
        Self::Full
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
    /// Resolved shell launch for `command`. `None` keeps the historical
    /// platform-default wrap for callers that never resolved a user shell.
    pub launch: Option<crate::shell::ShellLaunchSpec>,
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
mod hosted_image_title_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn hosted_image_titles_keep_prompts_out_of_the_headline() {
        for prompt in [
            "A rose".to_owned(),
            "一朵玫瑰，柔和自然光，背景中的细节保持清晰。".repeat(20),
        ] {
            let invocation = ToolInvocation::new(
                "image_generation",
                agena_domain::StructuredObject::try_from(json!({"prompt": prompt})).unwrap(),
            );
            let mut output: RawOutput = serde_json::from_value(json!({
                "payload": {"text": prompt},
                "attachments": [{
                    "kind": "image", "mime": "image/png", "filename": "rose.png",
                    "source": {"source": "local_path", "path": "/generated/rose.png"},
                }],
            }))
            .unwrap();
            assert_eq!(initial_tool_title(&invocation), "Generate image");
            assert_eq!(
                completed_tool_title(&invocation, &output),
                "Generate image · 1 image"
            );
            assert_eq!(
                completed_tool_title_for_state(&invocation, ToolResultState::Failed, &output),
                "Generate image · 1 image · failed",
            );
            output.attachments.push(output.attachments[0].clone());
            assert_eq!(
                completed_tool_title(&invocation, &output),
                "Generate image · 2 images"
            );
            assert_eq!(output.text_content(), prompt);
        }
    }
}

#[cfg(test)]
mod presentation_identity_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn presentation_aliases_keep_cloud_action_underscores() {
        for name in [
            "chatgpt.web_search",
            "agena.chatgpt.web_search",
            "agena_chatgpt_web_search",
        ] {
            assert_eq!(normalized_tool_identity(name), "chatgpt.web_search");
        }
        for name in [
            "shell.exec",
            "agena.shell.exec",
            "agena_shell_exec",
            " SHELL/EXEC ",
        ] {
            assert_eq!(normalized_tool_identity(name), "shell.exec");
        }
        assert_eq!(
            normalized_tool_identity("custom.Operation_name"),
            "custom.operation_name"
        );
    }

    #[test]
    fn shared_subjects_keep_tool_specific_fields_and_borrow_original_input() {
        let input = json!({
            "paths": ["中文.rs", "second.rs"],
            "description": "not the file target",
            "api_key": "private credential"
        });
        let original = input.clone();
        assert_eq!(
            invocation_title_subject("agena.fs.read_many", &input),
            "中文.rs +1"
        );
        assert_eq!(input, original);
        assert_eq!(
            invocation_title_subject("shell.read", &json!({"process_id": 42})),
            "42"
        );
        assert!(
            invocation_title_subject("image_generation", &json!({"prompt": "private prompt"}))
                .is_empty()
        );
        assert!(
            invocation_title_subject("plan.update", &json!({"prompt": "private prompt"}))
                .is_empty()
        );
    }
}

#[cfg(test)]
mod file_search_result_tests {
    use super::*;
    use serde_json::json;

    fn summary(tool_name: &str, payload: serde_json::Value) -> String {
        result_title_fragment_for_tool(
            tool_name,
            &RawOutput {
                payload: Some(payload),
                ..Default::default()
            },
        )
    }

    #[test]
    fn grep_counts_matched_records_without_context_or_per_file_line_totals() {
        assert_eq!(
            summary(
                "fs.grep",
                json!({
                    "mode": "content", "records": [
                        {"kind": "line", "matched": false},
                        {"kind": "line", "matched": true},
                        {"kind": "line", "matched": false}
                    ]
                })
            ),
            "1 match"
        );
        assert_eq!(
            summary(
                "agena.fs.grep",
                json!({
                    "mode": "files", "records": [{"kind": "file"}, {"kind": "file"}]
                })
            ),
            "2 matching files"
        );
        assert_eq!(
            summary(
                "agena_fs_grep",
                json!({
                    "mode": "count", "records": [
                        {"kind": "count", "count": 90, "complete": true},
                        {"kind": "count", "count": 3, "complete": false}
                    ], "truncated": true, "scan_complete": false
                })
            ),
            "2 matching files · truncated"
        );
    }

    #[test]
    fn search_preserves_reported_counts_legacy_shapes_and_unknown_results() {
        for (tool_name, payload, expected) in [
            (
                "fs.glob",
                json!({"count": " 12 ", "paths": ["a"], "truncated": true}),
                "12 matches · truncated",
            ),
            ("fs.glob", json!({"paths": ["a"]}), "1 match"),
            (
                "fs.grep",
                json!({"matches": 12, "results": ["a:1: match"]}),
                "12 matches",
            ),
            (
                "fs.grep",
                json!({"matches": 4_294_967_297_u64}),
                "4294967297 matches",
            ),
            (
                "fs.grep",
                json!({"matches": 12, "records": [{"kind": "future"}]}),
                "12 matches",
            ),
            (
                "fs.grep",
                json!({"results": ["a:1: match", "b:2: match"]}),
                "2 matches",
            ),
            (
                "fs.grep",
                json!({"mode": "files", "scan_complete": true}),
                "0 matching files",
            ),
            (
                "fs.grep",
                json!({"mode": "files", "scan_complete": false}),
                "0 matching files · incomplete",
            ),
        ] {
            assert_eq!(summary(tool_name, payload), expected);
        }
        for payload in [
            json!({}),
            json!({"mode":"files"}),
            json!({"records":[{"kind":"future"}]}),
            json!({"mode":"future", "scan_complete":true}),
            json!({"mode":null, "scan_complete":true}),
            json!({"mode":"files", "records":{"kind":"future"}, "scan_complete":true}),
            json!({"results":"malformed", "scan_complete":true}),
        ] {
            assert_eq!(
                FileSearchResultFacts::grep(payload.as_object().unwrap()).count,
                None
            );
        }
    }

    #[test]
    fn search_failure_stays_authoritative_and_summary_never_reads_file_text() {
        let mut raw = RawOutput {
            payload: Some(json!({
                "mode": "content", "matches": 12, "status": "failed",
                "records": [{"kind":"line", "matched":true, "text":"private source content"}],
                "provider_raw": {"access_token":"private credential"}
            })),
            ..Default::default()
        };
        let original = raw.clone();
        assert_eq!(result_title_fragment_for_tool("fs.grep", &raw), "failed");
        assert_eq!(raw, original);
        raw.payload.as_mut().unwrap()["status"] = json!("completed");
        let original = raw.clone();
        assert_eq!(
            result_title_fragment_for_tool("fs.grep", &raw),
            "12 matches"
        );
        assert_eq!(raw, original);
    }
}
