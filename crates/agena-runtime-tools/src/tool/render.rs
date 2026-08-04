//! Unified human-facing rendering for tool results.
//!
//! Every tool result is one [`ToolResult`] variant. The human view of any
//! tool result is produced by exactly one function — `ToolResult::render_markdown`
//! — which dispatches to each variant's impl. Terminals and web UIs both render
//! this Markdown with their own Markdown pipeline; no consumer ever matches on
//! tool names to build a presentation.
//!
//! Large text bodies are never embedded in the compact result. They live in a
//! managed file (`.agena/tool-results/…`) referenced by `DetailSource::Managed`,
//! and the renderer reads them lazily through [`RenderContext::read_managed`].
//! This keeps the durable record small and defers I/O to the moment a human
//! actually expands an Activity.

use std::fmt::Write as _;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Where the human-readable body of a tool result lives.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum DetailSource {
    /// Small body stored inline in the compact result.
    Inline(String),
    /// Large body stored in a managed file under `.agena/tool-results/`.
    Managed { path: PathBuf },
}

impl DetailSource {
    /// Small inline bodies are kept in the compact record; larger ones spill
    /// to a managed file so the durable record stays small.
    pub fn new(body: String, managed_path: Option<PathBuf>) -> Self {
        match managed_path {
            Some(path) if !body.is_empty() => Self::Managed { path },
            _ if body.is_empty() => Self::Inline(body),
            _ => Self::Inline(body),
        }
    }

    /// Resolve the body for rendering, reading a managed file lazily.
    pub fn text<'a>(&'a self, ctx: &'a RenderContext<'_>) -> Option<String> {
        match self {
            Self::Inline(text) => Some(text.clone()),
            Self::Managed { path } => (ctx.read_managed)(path),
        }
    }
}

/// Context passed to every tool-result renderer.
pub struct RenderContext<'a> {
    pub workspace_root: &'a std::path::Path,
    /// For a streaming tool that is still running: the current tail of its
    /// in-memory output buffer, so the live detail can show output as it
    /// arrives without waiting for the terminal frame.
    pub live_tail: Option<&'a str>,
    /// The shell command line, when the tool is a shell/process execution and
    /// the caller knows it. Lets the human view show `$ command` instead of a
    /// bare output card.
    pub command: Option<&'a str>,
    /// Lazily read a managed file body. Callers own caching; the renderer
    /// reads at most once per expanded Activity.
    pub read_managed: &'a dyn Fn(&PathBuf) -> Option<String>,
}

/// A tool result renders its human-facing view as a single Markdown document.
pub trait ToolResultRender {
    fn render_markdown(&self, ctx: &RenderContext<'_>) -> String;
}

/// Markdown body accumulator. Section headers, code cards, and lists are all
/// emitted through this one writer so every tool's detail has a consistent
/// shape and spacing.
#[derive(Debug, Default)]
pub struct MarkdownWriter {
    body: String,
}

impl MarkdownWriter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn finish(self) -> String {
        self.body
    }

    /// A bold heading line, e.g. ``### `src/main.rs` (file)``.
    pub fn heading(&mut self, text: impl AsRef<str>) {
        if !self.body.is_empty() {
            self.body.push('\n');
        }
        let _ = writeln!(self.body, "### {}", text.as_ref());
    }

    /// A paragraph / plain line.
    pub fn line(&mut self, text: impl AsRef<str>) {
        if !self.body.is_empty() && !self.body.ends_with("\n\n") {
            self.body.push('\n');
        }
        let _ = writeln!(self.body, "{}", text.as_ref());
    }

    /// A fenced code card. `lang` is the fence language (`sh`, `diff`, …).
    pub fn code_block(&mut self, lang: &str, text: &str) {
        if !self.body.is_empty() {
            self.body.push('\n');
        }
        let trimmed = text.trim_end();
        let _ = writeln!(self.body, "```{lang}");
        let _ = writeln!(self.body, "{trimmed}");
        self.body.push_str("```\n");
    }

    /// A Markdown list item.
    pub fn list_item(&mut self, item: impl AsRef<str>) {
        let _ = writeln!(self.body, "- {}", item.as_ref());
    }

    /// A bolded note / status line.
    pub fn note(&mut self, text: impl AsRef<str>) {
        if !self.body.is_empty() {
            self.body.push('\n');
        }
        let _ = writeln!(self.body, "_{}_", text.as_ref());
    }

    /// A key/value line, e.g. ``- **shell**: bash``, for compact parameters.
    pub fn kv(&mut self, key: impl AsRef<str>, value: impl AsRef<str>) {
        let _ = writeln!(self.body, "- **{}**: {}", key.as_ref(), value.as_ref());
    }

    /// A two-column Markdown table from rows of `(label, value)`.
    pub fn table(&mut self, rows: &[(String, String)]) {
        if rows.is_empty() {
            return;
        }
        if !self.body.is_empty() {
            self.body.push('\n');
        }
        self.body.push_str("| Field | Value |\n");
        self.body.push_str("| --- | --- |\n");
        for (key, value) in rows {
            let value = value.replace('|', "\\|");
            let _ = writeln!(self.body, "| {key} | {value} |");
        }
        self.body.push('\n');
    }
}

/// Render any tool result to its human-facing Markdown.
///
/// This is the **single** dispatch every consumer calls. It decodes the
/// compact payload (already serialized with the tool discriminant) back into
/// the typed enum and renders the matched variant. No consumer matches on tool
/// names; all presentation lives here.
pub fn render_tool_payload_markdown(
    payload: &serde_json::Value,
    ctx: &RenderContext<'_>,
) -> String {
    render_tool_payload_markdown_with_name("", payload, ctx)
}

/// Render a tool result, reconstructing the `ToolPayloadOutput` discriminant
/// from the tool name when the compact payload was stored without its `tool`
/// tag (the durable `ToolOutput` strips it). This lets the durable compact
/// record stay small while still rendering the correct variant.
pub fn render_tool_payload_markdown_with_name(
    tool_name: &str,
    payload: &serde_json::Value,
    ctx: &RenderContext<'_>,
) -> String {
    let mut decoded = payload.clone();
    if (!decoded.is_object() || !decoded.as_object().unwrap().contains_key("tool"))
        && let Some(payload_name) =
            crate::tool::payload::ToolPayloadOutput::payload_name_for(tool_name)
        && let Some(object) = decoded.as_object_mut()
    {
        object.insert("tool".to_string(), serde_json::Value::String(payload_name));
    }
    match serde_json::from_value::<crate::tool::payload::ToolPayloadOutput>(decoded) {
        Ok(typed) => typed.render_markdown(ctx),
        // Opaque/unknown payloads render as a plain text card.
        Err(_) => {
            let mut w = MarkdownWriter::new();
            w.code_block(
                "json",
                serde_json::to_string_pretty(payload)
                    .unwrap_or_else(|_| "unparseable tool result".to_string())
                    .as_str(),
            );
            w.finish()
        }
    }
}

impl ToolResultRender for crate::tool::payload::ToolPayloadOutput {
    fn render_markdown(&self, ctx: &RenderContext<'_>) -> String {
        use crate::tool::payload::ToolPayloadOutput as P;
        let mut w = MarkdownWriter::new();
        match self {
            P::Read {
                preview,
                truncated,
                loaded_paths,
                ..
            } => {
                let heading = if loaded_paths.is_empty() {
                    "read".to_string()
                } else {
                    format!("`{}`", loaded_paths.join(", "))
                };
                w.heading(heading);
                if let Some(preview) = preview.as_ref().filter(|text| !text.trim().is_empty()) {
                    // Numbered file previews (``N: content``) are readable as a
                    // plain code card; bare entries (directory listing) render
                    // as a Markdown list.
                    let looks_like_dir = !preview
                        .lines()
                        .any(|line| line.split_once(':').is_some_and(|(n, _)| n.trim().parse::<u32>().is_ok()));
                    if looks_like_dir {
                        for entry in preview.lines() {
                            w.list_item(format!("`{entry}`"));
                        }
                    } else {
                        w.code_block("", preview.as_str());
                    }
                }
                if *truncated {
                    w.note("truncated — use read with a different offset/limit");
                }
            }
            P::ApplyPatch {
                changes,
                diff,
                operation_id,
                ..
            } => {
                w.heading(format!("`apply_patch` · {operation_id}"));
                for change in changes {
                    w.list_item(file_change_line(change));
                }
                if !diff.trim().is_empty() {
                    w.code_block("diff", diff);
                }
            }
            P::Glob {
                paths, truncated, ..
            } => {
                w.heading(format!("`glob` · {} match(es)", paths.len()));
                for path in paths {
                    w.list_item(format!("`{path}`"));
                }
                if *truncated {
                    w.note("truncated — more matches available");
                }
            }
            P::Grep {
                results, truncated, ..
            } => {
                w.heading(format!("`grep` · {} match(es)", results.len()));
                for result in results {
                    w.list_item(format!("`{result}`"));
                }
                if *truncated {
                    w.note("truncated");
                }
            }
            P::Task {
                task_id,
                status,
                final_text,
                input_tokens,
                output_tokens,
                total_cost_microusd,
                ..
            } => {
                w.heading(format!("`task` {task_id} · {status}"));
                if let Some(final_text) = final_text.as_ref().filter(|text| !text.trim().is_empty()) {
                    w.code_block("", final_text.as_str());
                }
                w.line(format!(
                    "tokens: {input_tokens} in · {output_tokens} out · cost ${:.4}",
                    *total_cost_microusd as f64 / 1_000_000.0
                ));
            }
            P::ToolSearch { results } => {
                w.heading(format!("`tool_search` · {} tool(s)", results.len()));
                for name in results {
                    w.list_item(format!("`{name}`"));
                }
            }
            P::AskUser { answers, timed_out } => {
                w.heading(if *timed_out { "ask_user · timed out" } else { "ask_user" });
                for (question, values) in answers {
                    w.list_item(format!("**{question}**: {}", values.join(", ")));
                }
            }
            P::Shell {
                action,
                exit_code,
                output,
                background,
                status,
                process_id,
                shell,
                dropped_lines,
                ..
            } => {
                if let Some(command) = ctx.command.filter(|command| !command.trim().is_empty()) {
                    w.heading(format!("`$ {command}`"));
                } else {
                    w.heading("`shell`");
                }
                // Parameters render as a compact table; the output is a clean
                // code block below it (never raw JSON).
                let mut params: Vec<(String, String)> = Vec::new();
                if *action == "run" {
                    if let Some(shell) = shell {
                        params.push(("shell".to_owned(), shell.to_string()));
                    }
                    if let Some(status) = status {
                        params.push(("status".to_owned(), status.to_string()));
                    }
                    if let Some(exit_code) = exit_code {
                        params.push(("exit".to_owned(), exit_code.to_string()));
                    }
                    if *background {
                        params.push(("mode".to_owned(), "background".to_owned()));
                    }
                    if let Some(dropped_lines) = dropped_lines.filter(|count| *count > 0) {
                        params.push(("dropped".to_owned(), dropped_lines.to_string()));
                    }
                } else if let Some(process_id) = process_id {
                    params.push(("process".to_owned(), process_id.clone()));
                    if let Some(status) = status {
                        params.push(("status".to_owned(), status.to_string()));
                    }
                    params.push(("hint".to_owned(), "use `shell.logs` to read output".to_owned()));
                }
                w.table(&params);

                if *action == "run" {
                    let live_tail = ctx.live_tail;
                    let body = match (output.as_deref(), live_tail) {
                        (Some(text), _) => Some(text),
                        (None, Some(tail)) => Some(tail),
                        _ => None,
                    };
                    if let Some(body) = body.filter(|text| !text.trim().is_empty()) {
                        w.code_block("", body);
                    }
                }
            }
            P::WebFetch {
                url,
                markdown,
                summary,
                status,
                ..
            } => {
                w.heading(format!("`web_fetch` · {url}"));
                if let Some(summary) = summary.as_ref().filter(|text| !text.trim().is_empty()) {
                    w.line(summary);
                }
                if let Some(markdown) = markdown.as_ref().filter(|text| !text.trim().is_empty()) {
                    w.code_block("markdown", markdown.as_str());
                }
                w.line(format!("status {status}"));
            }
            P::WebSearch {
                query,
                results,
                backend,
            } => {
                w.heading(format!("`web_search` · {query}"));
                w.note(format!("backend {backend}"));
                for result in results {
                    w.list_item(format!("[{}]({})", result.title, result.url));
                    if let Some(snippet) = result.snippet.as_ref() {
                        w.line(snippet);
                    }
                }
            }
            P::EnterSnapshot {
                path,
                branch,
                backend,
                note,
            } => {
                w.heading(format!("`enter_snapshot` · {branch}"));
                w.line(format!("path: `{path}` · backend: {}", backend.as_deref().unwrap_or("—")));
                if let Some(note) = note.as_ref().filter(|text| !text.trim().is_empty()) {
                    w.line(note);
                }
            }
            P::ExitSnapshot { action, path } => {
                w.heading(format!("`exit_snapshot` · {action}"));
                w.line(format!("`{path}`"));
            }
            P::CronCreate { id, next_fire_at } => {
                w.heading(format!("`cron_create` · {id}"));
                if let Some(next) = next_fire_at {
                    w.line(format!("next run {next}"));
                }
            }
            P::CronList { jobs } => {
                w.heading(format!("`cron_list` · {} job(s)", jobs.len()));
                for job in jobs {
                    let state = if job.paused {
                        "⏸ paused"
                    } else if job.completed {
                        "✓ completed"
                    } else {
                        "▶ active"
                    };
                    let expr = job
                        .expression
                        .as_deref()
                        .map(|expr| format!(" `{expr}`"))
                        .unwrap_or_default();
                    let next = job
                        .next_fire_at
                        .as_deref()
                        .map(|next| format!(" · next {next}"))
                        .unwrap_or_default();
                    w.list_item(format!("**{state}**{expr}{next} · {}", job.prompt));
                }
            }
            P::CronDelete { id, removed } => {
                w.heading(format!("`cron_delete` · {id} · {removed}"));
            }
            P::CronUpdate { job } | P::CronPause { job } | P::CronResume { job } => {
                w.heading(format!("`cron` · {}", job.id));
                w.line(job.prompt.clone());
            }
            P::CronHistory { entries } => {
                w.heading(format!("`cron_history` · {} run(s)", entries.len()));
                for entry in entries {
                    w.list_item(cron_run_summary_line(entry));
                }
            }
            P::ScheduleWakeup { id, next_fire_at } => {
                w.heading(format!("`schedule_wakeup` · {id}"));
                w.line(format!("next fire {next_fire_at}"));
            }
            P::LspDefinition { locations } => {
                w.heading(format!("`lsp_definition` · {} location(s)", locations.len()));
                for location in locations {
                    w.list_item(format!("`{location}`"));
                }
            }
            P::LspReferences { locations } => {
                w.heading(format!("`lsp_references` · {} reference(s)", locations.len()));
                for location in locations {
                    w.list_item(format!("`{location}`"));
                }
            }
            P::LspHover { contents } => {
                w.heading("`lsp_hover`");
                if let Some(contents) = contents.as_ref().filter(|text| !text.trim().is_empty()) {
                    w.code_block("", contents.as_str());
                }
            }
            P::LspDiagnostics { entries } => {
                w.heading(format!("`lsp_diagnostics` · {} diagnostic(s)", entries.len()));
                for entry in entries {
                    let marker = if entry.contains("[error]") {
                        "❌"
                    } else if entry.contains("[warning]") {
                        "⚠️"
                    } else {
                        "•"
                    };
                    w.list_item(format!("{marker} `{entry}`"));
                }
            }
        }
        w.finish()
    }
}

fn cron_run_summary_line(entry: &agena_tool::CronRunSummary) -> String {
    let attempt = entry
        .attempt
        .map(|attempt| format!(" · attempt {attempt}"))
        .unwrap_or_default();
    let scheduled = entry
        .scheduled_for
        .as_deref()
        .map(|scheduled| format!(" · scheduled {scheduled}"))
        .unwrap_or_default();
    let failure = entry
        .failure
        .as_ref()
        .map(|failure| format!(" · {}", failure.user.fallback))
        .unwrap_or_default();
    format!(
        "**{}** · {} → {}{attempt}{scheduled}{failure}",
        entry.status, entry.triggered_at, entry.finished_at
    )
}

fn file_change_line(change: &agena_domain::FileChangeRecord) -> String {
    use agena_domain::FileChangeKind;
    let action = match change.kind {
        FileChangeKind::Added => "added",
        FileChangeKind::Updated => "updated",
        FileChangeKind::Deleted => "deleted",
        FileChangeKind::Moved => "moved",
    };
    match change.kind {
        FileChangeKind::Moved => format!(
            "**{action}** `{}` → `{}`",
            change.from_path.as_deref().unwrap_or(&change.path),
            change.path
        ),
        _ => format!("**{action}** `{}`", change.path),
    }
}

#[cfg(test)]
mod tests {
    use super::{DetailSource, MarkdownWriter};

    #[test]
    fn writer_builds_a_spaced_markdown_document() {
        let mut w = MarkdownWriter::new();
        w.heading("`main.rs` (file)");
        w.code_block("rust", "fn main() {}");
        w.list_item("one");
        w.list_item("two");
        w.note("truncated");
        let out = w.finish();
        assert!(out.starts_with("### `main.rs` (file)\n"));
        assert!(out.contains("```rust\nfn main() {}\n```\n"));
        assert!(out.contains("- one\n- two\n"));
        assert!(out.ends_with("_truncated_\n"));
    }

    #[test]
    fn detail_source_inline_round_trips() {
        let source = DetailSource::Inline("hello".to_string());
        let ctx = crate::tool::RenderContext {
            workspace_root: std::path::Path::new("/tmp"),
            live_tail: None,
            command: None,
            read_managed: &|_| None,
        };
        assert_eq!(source.text(&ctx), Some("hello".to_string()));
    }

    fn render(payload: serde_json::Value) -> String {
        render_with_command(payload, None)
    }

    fn render_with_command(payload: serde_json::Value, command: Option<&str>) -> String {
        super::render_tool_payload_markdown(&payload, &crate::tool::RenderContext {
            workspace_root: std::path::Path::new("/tmp"),
            live_tail: None,
            command,
            read_managed: &|_| None,
        })
    }

    #[test]
    fn glob_renders_a_path_list() {
        let out = render(serde_json::json!({
            "tool": "glob",
            "paths": ["src/a.rs", "src/b.rs"],
            "count": 2,
            "truncated": false
        }));
        assert!(out.contains("`src/a.rs`"));
        assert!(out.contains("`src/b.rs`"));
        assert!(out.contains("2 match(es)"));
    }

    #[test]
    fn apply_patch_renders_a_change_list_and_diff() {
        let out = render(serde_json::json!({
            "tool": "apply_patch",
            "operation_id": "op-1",
            "changes": [{"path": "README.md", "kind": "updated", "from_path": null}],
            "diff": "--- a\n+++ b\n",
            "inverse_patch": ""
        }));
        assert!(out.contains("op-1"));
        assert!(out.contains("**updated** `README.md`"));
        assert!(out.contains("```diff"));
    }

    #[test]
    fn shell_renders_a_command_card() {
        let out = render(serde_json::json!({
            "tool": "shell",
            "action": "run",
            "shell": "bash",
            "exit_code": 0,
            "output": "test result: ok",
            "status": "exited"
        }));
        // Parameters render as a table; the output is a clean code block.
        assert!(out.contains("| shell | bash |"), "{out}");
        assert!(out.contains("| status | exited |"), "{out}");
        assert!(out.contains("| exit | 0 |"), "{out}");
        assert!(out.contains("```\ntest result: ok\n```"), "{out}");
        // Raw JSON must never be dumped inline.
        assert!(!out.contains("\"output\": \"test result: ok\""), "{out}");
    }

    #[test]
    fn shell_renders_the_command_line_when_known() {
        let out = render_with_command(
            serde_json::json!({
                "tool": "shell",
                "action": "run",
                "exit_code": 0,
                "output": "ok",
                "status": "exited"
            }),
            Some("cargo test"),
        );
        assert!(out.contains("`$ cargo test`"), "{out}");
        assert!(out.contains("```\nok\n```"), "{out}");
    }

    #[test]
    fn unknown_payload_falls_back_to_json_card() {
        let out = render(serde_json::json!({"some": "opaque"}));
        assert!(out.contains("```json"));
        assert!(out.contains("opaque"));
    }
}
