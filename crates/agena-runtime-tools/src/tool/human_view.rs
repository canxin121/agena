//! Built-in human renderer for tool results.
//!
//! Tool output is durable machine-readable data, but the transcript should
//! present the useful facts a person needs to make a decision. This renderer
//! is the shared fallback for bundled tools: structured results become
//! Markdown, tables, command cards, diffs, or search results instead of a
//! generic JSON dump. JSON remains the last-resort representation for an
//! opaque result that has no readable text projection.

use std::collections::BTreeSet;
use std::fmt::Display;

use agena_domain::{RawOutput, ToolOutput, ViewBlock, WebSearchResult};
use agena_tool::{
    CronJobSummary, CronRunSummary, RenderContext, RenderError, ToolHumanRenderer,
    normalize_tool_summary,
};
use serde::Serialize;
use serde_json::{Value, json};

use crate::tool::payload::ToolPayloadOutput;

/// A renderer for one built-in tool result. `tool_name` follows the same
/// resolution rules as [`ToolPayloadOutput::from_tool_output`]; `command` and
/// `cwd` let shell executions render a `$ command` card instead of a bare
/// output card.
#[derive(Debug, Clone)]
pub struct BuiltinHumanRenderer {
    pub tool_name: String,
    /// The shell command line from the invocation input, when known.
    pub command: Option<String>,
    /// The working directory from the invocation input, when known.
    pub cwd: Option<String>,
}

impl BuiltinHumanRenderer {
    const GENERIC_MAX_DEPTH: usize = 2;
    const GENERIC_MAX_ROWS: usize = 100;
    const GENERIC_MAX_COLUMNS: usize = 10;
    const GENERIC_MAX_VALUE_CHARS: usize = 1_200;

    pub fn new(tool_name: impl Into<String>) -> Self {
        Self {
            tool_name: tool_name.into(),
            command: None,
            cwd: None,
        }
    }

    pub fn with_command(mut self, command: impl Into<String>) -> Self {
        self.command = Some(command.into());
        self
    }

    pub fn with_cwd(mut self, cwd: impl Into<String>) -> Self {
        self.cwd = Some(cwd.into());
        self
    }

    /// Derive the compact collapsed-row summary from the human-readable
    /// channel and the structured payload. The model projection is often a
    /// JSON serialization of the payload, which is useful to the model but is
    /// not a useful transcript headline.
    pub fn human_summary(raw: &RawOutput) -> String {
        let text = raw.text.trim();
        let text_json = Self::json_document(text);
        let payload = raw.payload.as_ref().or(text_json.as_ref());

        if !text.is_empty()
            && text_json.is_none()
            && Self::is_summary_text(text)
            && !payload.is_some_and(Self::has_descriptive_summary)
        {
            return normalize_tool_summary(text);
        }
        if let Some(summary) = payload.and_then(Self::payload_summary) {
            return normalize_tool_summary(summary);
        }
        if !text.is_empty() && text_json.is_none() {
            let first_line = text
                .lines()
                .map(str::trim)
                .find(|line| !line.is_empty())
                .unwrap_or(text);
            return normalize_tool_summary(first_line);
        }
        if raw.truncated {
            return "Output truncated".to_owned();
        }
        if !raw.managed_outputs.is_empty() {
            return format!("{} managed output", raw.managed_outputs.len());
        }
        "No output".to_owned()
    }

    fn has_descriptive_summary(payload: &Value) -> bool {
        let Some(object) = payload.as_object() else {
            return false;
        };
        ["summary", "message", "title", "description"]
            .iter()
            .any(|key| {
                object
                    .get(*key)
                    .and_then(Value::as_str)
                    .is_some_and(|value| !value.trim().is_empty())
            })
    }

    fn payload_summary(payload: &Value) -> Option<String> {
        let object = payload.as_object()?;
        for key in ["summary", "message", "title", "description"] {
            if let Some(Value::String(value)) = object.get(key)
                && !value.trim().is_empty()
            {
                return Some(value.clone());
            }
        }

        for key in [
            "results",
            "findings",
            "servers",
            "resources",
            "resource_templates",
            "prompts",
            "jobs",
            "entries",
            "steps",
            "paths",
            "matches",
            "items",
            "tasks",
        ] {
            if let Some(Value::Array(values)) = object.get(key) {
                return Some(format!(
                    "{} {}",
                    values.len(),
                    Self::count_label(key, values.len())
                ));
            }
        }

        for key in ["count", "total", "tool_count", "finding_count"] {
            if let Some(value) = object.get(key).and_then(Value::as_u64) {
                return Some(format!(
                    "{value} {}",
                    Self::count_label(key, value as usize)
                ));
            }
        }

        object
            .get("status")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .map(ToOwned::to_owned)
    }

    fn count_label(key: &str, count: usize) -> &'static str {
        match (key, count == 1) {
            ("results", true) => "result",
            ("results", false) => "results",
            ("findings", true) => "finding",
            ("findings", false) => "findings",
            ("servers", true) => "server",
            ("servers", false) => "servers",
            ("resources", true) => "resource",
            ("resources", false) => "resources",
            ("resource_templates", true) => "template",
            ("resource_templates", false) => "templates",
            ("prompts", true) => "prompt",
            ("prompts", false) => "prompts",
            ("jobs", true) => "job",
            ("jobs", false) => "jobs",
            ("entries", true) => "entry",
            ("entries", false) => "entries",
            ("steps", true) => "step",
            ("steps", false) => "steps",
            ("paths", true) => "path",
            ("paths", false) => "paths",
            ("matches", true) => "match",
            ("matches", false) => "matches",
            ("items", true) => "item",
            ("items", false) => "items",
            ("tasks", true) => "task",
            ("tasks", false) => "tasks",
            ("count", _) => "items",
            ("total", _) => "items",
            ("tool_count", _) => "tools",
            ("finding_count", _) => "findings",
            _ => "items",
        }
    }

    /// Render opaque output with the human-readable channel first.
    ///
    /// Bundled plugins deliberately keep their durable payloads open-ended:
    /// the settings, MCP, browser, memory, workflow, and provider adapters
    /// all return different JSON shapes. Treating every unknown shape as a
    /// JSON card made a short human summary hide the useful fields (for
    /// example, `settings.inspect` returned only "Inspected settings"). The
    /// generic projection below turns shallow objects and arrays into the same
    /// Markdown/details/table vocabulary used by the typed renderers. JSON is
    /// still the last resort for a genuinely opaque value.
    fn fallback(raw: &RawOutput) -> Vec<ViewBlock> {
        let mut blocks = Vec::new();
        let text_json = Self::json_document(raw.text.as_str());
        if !raw.text.trim().is_empty() && text_json.is_none() {
            blocks.push(ViewBlock::Markdown {
                id: Some("text".into()),
                text: raw.text.clone(),
            });
        }

        let payload = raw.payload.as_ref().or(text_json.as_ref());
        let generic = payload
            .filter(|payload| !payload.is_null())
            .map(|payload| Self::generic_payload_blocks(payload, "result"))
            .unwrap_or_default();
        // A plugin's text channel is often a deliberately concise list or
        // document preview. Keep it as the first human-facing block, but do
        // not let that preview hide facts that exist only in the structured
        // payload (server identity, cursors, hashes, counts, nested records,
        // and so on). The generic projection is bounded and gives those
        // supplemental facts a readable details/table/list representation.
        if !generic.is_empty() {
            blocks.extend(generic);
        }

        if blocks.is_empty() {
            if let Some(payload) = raw.payload.as_ref() {
                if !payload.is_null() {
                    blocks.push(ViewBlock::Json {
                        id: Some("payload".into()),
                        value: payload.clone(),
                    });
                }
            } else if let Some(value) = text_json {
                blocks.push(ViewBlock::Json {
                    id: Some("payload".into()),
                    value,
                });
            } else if !raw.text.trim().is_empty() {
                // Keep an unusual but readable text result visible even if a
                // future generic projection decides it cannot classify it.
                blocks.push(ViewBlock::Markdown {
                    id: Some("text".into()),
                    text: raw.text.clone(),
                });
            }
        }

        Self::raw_metadata_blocks(&mut blocks, raw);
        if blocks.is_empty() {
            blocks.push(Self::markdown_block("empty", "No output."));
        }
        Self::raw_flags(&mut blocks, raw);
        blocks
    }

    fn json_document(text: &str) -> Option<Value> {
        let text = text.trim();
        if text.is_empty() {
            return None;
        }
        let value = serde_json::from_str(text).ok()?;
        matches!(value, Value::Object(_) | Value::Array(_)).then_some(value)
    }

    fn is_summary_text(text: &str) -> bool {
        let text = text.trim();
        if text.is_empty() || Self::json_document(text).is_some() || text.len() > 320 {
            return false;
        }
        let lines = text.lines().filter(|line| !line.trim().is_empty()).count();
        lines <= 2
            && !text.contains("```")
            && !text.lines().any(|line| {
                let line = line.trim_start();
                line.starts_with("- ") || line.starts_with("* ") || line.starts_with("| ")
            })
    }

    fn humanize_key(key: &str) -> String {
        let mut label = String::with_capacity(key.len());
        let mut uppercase = true;
        for character in key.chars() {
            if matches!(character, '_' | '-' | '.') {
                label.push(' ');
                uppercase = true;
            } else if uppercase {
                label.extend(character.to_uppercase());
                uppercase = false;
            } else {
                label.push(character);
            }
        }
        label
    }

    fn generic_scalar(value: &Value) -> bool {
        matches!(
            value,
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_)
        )
    }

    fn generic_field_value(value: &Value) -> String {
        match value {
            Value::Null => "—".to_owned(),
            Value::String(text) if text.trim().is_empty() => "—".to_owned(),
            Value::String(text) => Self::bounded_generic_text(text),
            Value::Bool(value) => value.to_string(),
            Value::Number(value) => value.to_string(),
            _ => Self::bounded_generic_text(&Self::display_value(value)),
        }
    }

    fn bounded_generic_text(value: &str) -> String {
        let mut characters = value.chars();
        let bounded = characters
            .by_ref()
            .take(Self::GENERIC_MAX_VALUE_CHARS)
            .collect::<String>();
        if characters.next().is_some() {
            format!("{bounded}… [truncated]")
        } else {
            bounded
        }
    }

    fn bounded_generic_value(value: &Value) -> Value {
        match value {
            Value::String(text) => Value::String(Self::bounded_generic_text(text)),
            _ => value.clone(),
        }
    }

    fn generic_skip_key(key: &str) -> bool {
        // These values are either redundant with the readable text or are
        // machine contracts that make a table unreadable. Their presence is
        // still available in the model payload and in the JSON fallback when
        // no other projection can be made.
        matches!(
            key,
            "input_schema" | "output_schema" | "schema" | "provider_raw" | "raw" | "trace"
        )
    }

    fn generic_object_table(key: &str, values: &[Value]) -> Option<ViewBlock> {
        let objects = values
            .iter()
            .filter_map(Value::as_object)
            .collect::<Vec<_>>();
        if objects.len() != values.len() || objects.is_empty() {
            return None;
        }

        let mut columns = Vec::new();
        let mut seen = BTreeSet::new();
        for object in &objects {
            for (column, value) in *object {
                if Self::generic_skip_key(column)
                    || !Self::generic_scalar(value)
                    || !seen.insert(column.clone())
                {
                    continue;
                }
                columns.push(column.clone());
                if columns.len() == Self::GENERIC_MAX_COLUMNS {
                    break;
                }
            }
            if columns.len() == Self::GENERIC_MAX_COLUMNS {
                break;
            }
        }
        if columns.is_empty() {
            return None;
        }

        let rows = objects
            .iter()
            .take(Self::GENERIC_MAX_ROWS)
            .map(|object| {
                columns
                    .iter()
                    .map(|column| object.get(column).cloned().unwrap_or(Value::Null))
                    .map(|value| Self::bounded_generic_value(&value))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        Some(ViewBlock::Table {
            id: Some(format!("result-{key}")),
            columns: columns
                .iter()
                .map(|column| Self::humanize_key(column))
                .collect(),
            rows,
        })
    }

    fn generic_value_blocks(id: &str, title: &str, value: &Value, depth: usize) -> Vec<ViewBlock> {
        match value {
            Value::Object(object) => Self::generic_object_blocks(id, title, object, depth),
            Value::Array(values) => {
                if values.is_empty() {
                    return vec![Self::markdown_block(
                        format!("{id}-empty"),
                        format!("### {title}\nNo items."),
                    )];
                }
                if let Some(table) = Self::generic_object_table(title, values) {
                    let mut blocks = vec![table];
                    if values.len() > Self::GENERIC_MAX_ROWS {
                        blocks.push(Self::markdown_block(
                            format!("{id}-table-status"),
                            format!(
                                "_Showing the first {} of {} rows._",
                                Self::GENERIC_MAX_ROWS,
                                values.len()
                            ),
                        ));
                    }
                    blocks.extend(Self::generic_array_nested_blocks(id, values, depth));
                    return blocks;
                }
                let items = values
                    .iter()
                    .take(Self::GENERIC_MAX_ROWS)
                    .map(Self::generic_field_value)
                    .collect::<Vec<_>>();
                vec![Self::markdown_block(
                    format!("{id}-list"),
                    format!(
                        "### {title}\n{}",
                        items
                            .iter()
                            .map(|item| format!("- {item}"))
                            .collect::<Vec<_>>()
                            .join("\n")
                    ),
                )]
                .into_iter()
                .chain((values.len() > Self::GENERIC_MAX_ROWS).then(|| {
                    Self::markdown_block(
                        format!("{id}-list-status"),
                        format!(
                            "_Showing the first {} of {} items._",
                            Self::GENERIC_MAX_ROWS,
                            values.len()
                        ),
                    )
                }))
                .collect()
            }
            value => vec![Self::details_block(
                id,
                title,
                &[(title, Self::generic_field_value(value))],
            )],
        }
    }

    /// Preserve nested facts that cannot fit into the scalar columns of an
    /// object table. For example, workflow steps have `checkpoints`, MCP
    /// prompt messages have nested content, and browser records can carry
    /// nested result objects. Flattening one level keeps the table compact
    /// while still exposing those facts as their own bounded table/list.
    fn generic_array_nested_blocks(id: &str, values: &[Value], depth: usize) -> Vec<ViewBlock> {
        if depth > Self::GENERIC_MAX_DEPTH {
            return Vec::new();
        }

        let mut nested = std::collections::BTreeMap::<String, (Vec<Value>, bool)>::new();
        for object in values.iter().filter_map(Value::as_object) {
            for (key, value) in object {
                if Self::generic_skip_key(key) || Self::generic_scalar(value) {
                    continue;
                }
                let (values, truncated) = nested.entry(key.clone()).or_default();
                if values.len() >= Self::GENERIC_MAX_ROWS {
                    *truncated = true;
                    continue;
                }
                match value {
                    Value::Array(items) => {
                        let remaining = Self::GENERIC_MAX_ROWS.saturating_sub(values.len());
                        if items.len() > remaining {
                            *truncated = true;
                        }
                        values.extend(items.iter().take(remaining).cloned());
                    }
                    value => values.push(value.clone()),
                }
            }
        }

        nested
            .into_iter()
            .flat_map(|(key, (values, truncated))| {
                let child_id = format!("{id}-{}", key.replace(['.', '_'], "-"));
                let child_title = Self::humanize_key(key.as_str());
                let mut blocks = Self::generic_value_blocks(
                    child_id.as_str(),
                    child_title.as_str(),
                    &Value::Array(values),
                    depth + 1,
                );
                if truncated {
                    blocks.push(Self::markdown_block(
                        format!("{child_id}-status"),
                        format!(
                            "_Nested {child_title} values were capped at {}._",
                            Self::GENERIC_MAX_ROWS
                        ),
                    ));
                }
                blocks
            })
            .collect()
    }

    fn generic_object_blocks(
        id: &str,
        title: &str,
        object: &serde_json::Map<String, Value>,
        depth: usize,
    ) -> Vec<ViewBlock> {
        let mut blocks = Vec::new();
        let fields = object
            .iter()
            .filter(|(key, value)| !Self::generic_skip_key(key) && Self::generic_scalar(value))
            .filter(|(_, value)| !matches!(value, Value::Null))
            .map(|(key, value)| (Self::humanize_key(key), Self::generic_field_value(value)))
            .collect::<Vec<_>>();
        if !fields.is_empty() {
            let fields = fields
                .iter()
                .map(|(key, value)| (key.as_str(), value.clone()))
                .collect::<Vec<_>>();
            blocks.push(Self::details_block(id, title, &fields));
        }

        for (key, value) in object {
            if Self::generic_skip_key(key) || Self::generic_scalar(value) {
                continue;
            }
            let child_id = format!("{id}-{}", key.replace(['.', '_'], "-"));
            let child_title = Self::humanize_key(key);
            // Arrays at the depth boundary can still become useful bounded
            // tables/lists (browser snapshots commonly put `elements` here).
            // Only stop descending into another object at the boundary; the
            // array renderer itself will suppress deeper nested projections.
            if depth >= Self::GENERIC_MAX_DEPTH && matches!(value, Value::Object(_)) {
                blocks.push(Self::markdown_block(
                    child_id,
                    format!("### {child_title}\n{}", Self::generic_field_value(value)),
                ));
            } else {
                blocks.extend(Self::generic_value_blocks(
                    child_id.as_str(),
                    child_title.as_str(),
                    value,
                    depth + 1,
                ));
            }
        }
        blocks
    }

    fn generic_payload_blocks(payload: &Value, id: &str) -> Vec<ViewBlock> {
        Self::generic_value_blocks(id, "Details", payload, 0)
    }

    fn raw_metadata_blocks(blocks: &mut Vec<ViewBlock>, raw: &RawOutput) {
        if raw.metadata.is_empty() {
            return;
        }
        let metadata = Value::Object(raw.metadata.clone().into_iter().collect());
        blocks.extend(Self::generic_value_blocks(
            "metadata", "Metadata", &metadata, 0,
        ));
    }

    fn markdown_block(id: impl Into<String>, text: impl Into<String>) -> ViewBlock {
        ViewBlock::Markdown {
            id: Some(id.into()),
            text: text.into(),
        }
    }

    fn details_block(id: impl Into<String>, title: &str, fields: &[(&str, String)]) -> ViewBlock {
        let mut lines = vec![format!("### {title}")];
        for (label, value) in fields {
            if !value.trim().is_empty() {
                lines.push(format!("- **{label}**: {value}"));
            }
        }
        Self::markdown_block(id, lines.join("\n"))
    }

    fn list_block(id: impl Into<String>, title: &str, values: &[String]) -> Option<ViewBlock> {
        if values.is_empty() {
            return None;
        }
        let mut lines = vec![format!("### {title}")];
        lines.extend(values.iter().map(|value| format!("- {value}")));
        Some(Self::markdown_block(id, lines.join("\n")))
    }

    fn table_block(
        id: impl Into<String>,
        columns: Vec<&str>,
        rows: Vec<Vec<Value>>,
    ) -> Option<ViewBlock> {
        if rows.is_empty() {
            return None;
        }
        Some(ViewBlock::Table {
            id: Some(id.into()),
            columns: columns.into_iter().map(str::to_owned).collect(),
            rows,
        })
    }

    fn serialized<T: Serialize>(value: &T) -> Value {
        serde_json::to_value(value).unwrap_or(Value::Null)
    }

    fn display_option<T: Display>(value: Option<T>) -> String {
        value.map(|value| value.to_string()).unwrap_or_default()
    }

    fn display_value(value: &Value) -> String {
        match value {
            Value::Null => String::new(),
            Value::String(value) => value.clone(),
            _ => serde_json::to_string(value).unwrap_or_default(),
        }
    }

    fn readable_problem(value: &Value) -> String {
        if let Value::Object(object) = value {
            if let Some(Value::Object(user)) = object.get("user") {
                for key in ["fallback", "message"] {
                    if let Some(Value::String(message)) = user.get(key) {
                        if !message.trim().is_empty() {
                            return message.clone();
                        }
                    }
                }
            }
            for key in ["message", "detail", "fallback"] {
                if let Some(Value::String(message)) = object.get(key) {
                    if !message.trim().is_empty() {
                        return message.clone();
                    }
                }
            }
        }
        Self::display_value(value)
    }

    fn model_feedback_kind_label(kind: &agena_failure::ModelFeedbackKind) -> &'static str {
        match kind {
            agena_failure::ModelFeedbackKind::InternalToolFailure => "internal tool failure",
            agena_failure::ModelFeedbackKind::InvalidInput => "invalid input",
            agena_failure::ModelFeedbackKind::InvalidPattern => "invalid pattern",
            agena_failure::ModelFeedbackKind::ToolUnavailable => "tool unavailable",
            agena_failure::ModelFeedbackKind::StaleToolCall => "stale tool call",
            agena_failure::ModelFeedbackKind::PermissionRequired => "permission required",
            agena_failure::ModelFeedbackKind::UserInputRequired => "user input required",
            agena_failure::ModelFeedbackKind::PluginFailure => "plugin failure",
            agena_failure::ModelFeedbackKind::PermissionDenied => "permission denied",
            agena_failure::ModelFeedbackKind::UserDeclined => "user declined",
        }
    }

    fn model_feedback_block(feedback: &agena_failure::ModelFeedback) -> ViewBlock {
        let mut lines = vec![
            "### Model feedback".to_owned(),
            format!(
                "- **Category**: `{}`",
                Self::model_feedback_kind_label(&feedback.kind)
            ),
            format!("- **Message**: {}", feedback.message()),
        ];
        if feedback.field_count() > 0 {
            lines.push(format!("- **Field issues**: {}", feedback.field_count()));
        }
        Self::markdown_block("task-feedback", lines.join("\n"))
    }

    fn event_streams(events: &[agena_domain::ProcessEvent]) -> (String, String) {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        for event in events {
            if event.stream.to_string() == "stdout" {
                stdout.push(event.line.clone());
            } else {
                stderr.push(event.line.clone());
            }
        }
        (stdout.join("\n"), stderr.join("\n"))
    }

    fn process_rows(processes: &[agena_domain::ProcessSummary]) -> Vec<Vec<Value>> {
        processes
            .iter()
            .map(|process| {
                vec![
                    json!(process.process_id),
                    json!(process.status.to_string()),
                    json!(process.command),
                    json!(
                        process
                            .exit_code
                            .map(|code| code.to_string())
                            .unwrap_or_default()
                    ),
                    json!(process.buffered_lines),
                    json!(process.dropped_lines),
                ]
            })
            .collect()
    }

    fn cron_job_rows(jobs: &[CronJobSummary]) -> Vec<Vec<Value>> {
        jobs.iter()
            .map(|job| {
                let schedule = job
                    .expression
                    .as_deref()
                    .or(job.at.as_deref())
                    .unwrap_or_default();
                let status = if job.paused {
                    "paused"
                } else if job.completed {
                    "completed"
                } else {
                    job.last_run_status.as_deref().unwrap_or("ready")
                };
                let retry = match (&job.retry_at, job.retry_max_attempts) {
                    (Some(at), attempts) if !at.trim().is_empty() => {
                        format!("{attempts} at {at}")
                    }
                    (_, attempts) => attempts.to_string(),
                };
                let failure = job
                    .last_run_failure
                    .as_ref()
                    .map(Self::serialized)
                    .map(|value| Self::readable_problem(&value))
                    .unwrap_or_default();
                vec![
                    json!(job.id),
                    json!(job.kind),
                    json!(schedule),
                    json!(job.timezone.clone().unwrap_or_default()),
                    json!(status),
                    json!(job.next_fire_at.clone().unwrap_or_default()),
                    json!(job.last_fired_at.clone().unwrap_or_default()),
                    json!(job.run_count),
                    json!(retry),
                    json!(job.misfire_policy),
                    json!(job.prompt),
                    json!(failure),
                ]
            })
            .collect()
    }

    fn cron_columns() -> Vec<&'static str> {
        vec![
            "ID", "Kind", "Schedule", "Timezone", "Status", "Next", "Last", "Runs", "Retry",
            "Misfire", "Prompt", "Failure",
        ]
    }

    fn cron_run_rows(entries: &[CronRunSummary]) -> Vec<Vec<Value>> {
        entries
            .iter()
            .map(|entry| {
                let failure = entry
                    .failure
                    .as_ref()
                    .map(Self::serialized)
                    .map(|value| Self::readable_problem(&value))
                    .unwrap_or_default();
                vec![
                    json!(entry.job_id),
                    json!(entry.triggered_at),
                    json!(entry.finished_at),
                    json!(entry.status),
                    json!(entry.scheduled_for.clone().unwrap_or_default()),
                    json!(
                        entry
                            .attempt
                            .map(|attempt| attempt.to_string())
                            .unwrap_or_default()
                    ),
                    json!(entry.delivery_key.clone().unwrap_or_default()),
                    json!(
                        entry
                            .session_id
                            .map(|id| id.to_string())
                            .unwrap_or_default()
                    ),
                    json!(failure),
                ]
            })
            .collect()
    }

    fn raw_flags(blocks: &mut Vec<ViewBlock>, raw: &RawOutput) {
        if !raw.truncated && raw.managed_outputs.is_empty() {
            return;
        }
        let mut fields = Vec::new();
        if raw.truncated {
            fields.push(("Output", "truncated".to_owned()));
        }
        if !raw.managed_outputs.is_empty() {
            fields.push((
                "Managed output",
                raw.managed_outputs
                    .iter()
                    .map(|item| item.path.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
            ));
        }
        blocks.push(Self::details_block("output-meta", "Output status", &fields));
    }

    /// Typed payload renderers should not accidentally discard a plugin's
    /// human text channel. Most built-ins put the same preview in both
    /// places, so suppress an exact/containing duplicate; a distinct rich
    /// result summary or transcript is still shown as its own Markdown block.
    fn append_distinct_raw_text(blocks: &mut Vec<ViewBlock>, raw: &RawOutput) {
        let text = raw.text.trim();
        if text.is_empty() || Self::json_document(text).is_some() {
            return;
        }
        if blocks.iter().any(|block| match block {
            ViewBlock::Text { text: value, .. }
            | ViewBlock::Markdown { text: value, .. }
            | ViewBlock::Log { text: value, .. }
            | ViewBlock::Diff { diff: value, .. } => value.contains(text),
            ViewBlock::Command { stdout, stderr, .. } => {
                stdout.contains(text) || stderr.contains(text)
            }
            _ => false,
        }) {
            return;
        }
        blocks.push(Self::markdown_block("output-text", raw.text.clone()));
    }

    fn structured_blocks(
        tool_name: &str,
        raw: &RawOutput,
        output: &ToolOutput,
        command: Option<&str>,
        cwd: Option<&str>,
    ) -> Vec<ViewBlock> {
        let Some(parsed) = ToolPayloadOutput::from_tool_output(tool_name, output) else {
            return Vec::new();
        };
        let mut blocks = Vec::new();

        match parsed {
            ToolPayloadOutput::ApplyPatch {
                changes,
                diff,
                before_hash,
                after_hash,
                progress,
                ..
            } => {
                if !changes.is_empty() {
                    blocks.push(ViewBlock::FileChanges {
                        id: Some("changes".into()),
                        changes,
                    });
                }
                if !diff.trim().is_empty() {
                    blocks.push(ViewBlock::Diff {
                        id: Some("diff".into()),
                        diff,
                        language: Some("diff".into()),
                    });
                }
                let mut fields = Vec::new();
                if let Some(hash) = before_hash {
                    fields.push(("Before", hash));
                }
                if let Some(hash) = after_hash {
                    fields.push(("After", hash));
                }
                if !progress.is_empty() {
                    fields.push(("Progress", progress.join(" · ")));
                }
                if !fields.is_empty() {
                    blocks.push(Self::details_block("patch-meta", "Patch", &fields));
                }
            }
            ToolPayloadOutput::Read {
                preview,
                loaded_paths,
                truncated,
                attachment,
            } => {
                if let Some(block) = Self::list_block("loaded-paths", "Loaded paths", &loaded_paths)
                {
                    blocks.push(block);
                }
                if let Some(preview) = preview {
                    blocks.push(Self::markdown_block(
                        "preview",
                        format!("### Preview\n```text\n{preview}\n```"),
                    ));
                }
                if let Some(attachment) = attachment {
                    let mut fields = vec![
                        ("Path", attachment.path),
                        ("Type", attachment.kind.to_string()),
                        ("MIME", attachment.mime),
                        ("Size", format!("{} bytes", attachment.size_bytes)),
                    ];
                    if let Some(filename) = attachment.filename {
                        fields.push(("Filename", filename));
                    }
                    if let Some(width) = attachment.width {
                        fields.push(("Width", width.to_string()));
                    }
                    if let Some(height) = attachment.height {
                        fields.push(("Height", height.to_string()));
                    }
                    if let Some(duration) = attachment.duration_ms {
                        fields.push(("Duration", format!("{duration} ms")));
                    }
                    if let Some(page_count) = attachment.page_count {
                        fields.push(("Pages", page_count.to_string()));
                    }
                    blocks.push(Self::details_block("attachment", "Attachment", &fields));
                }
                if truncated {
                    blocks.push(Self::markdown_block("read-status", "_Preview truncated._"));
                }
            }
            ToolPayloadOutput::Glob {
                paths,
                count,
                truncated,
            } => {
                let mut lines = vec![format!(
                    "### {}",
                    count
                        .map(|count| format!("{count} matches"))
                        .unwrap_or_else(|| "Matches".into())
                )];
                lines.extend(paths.iter().map(|path| format!("- `{path}`")));
                if truncated {
                    lines.push("_Results truncated._".into());
                }
                blocks.push(Self::markdown_block("matches", lines.join("\n")));
            }
            ToolPayloadOutput::Grep {
                results,
                matches,
                truncated,
            } => {
                let mut lines = vec![format!(
                    "### {}",
                    matches
                        .map(|matches| format!("{matches} matches"))
                        .unwrap_or_else(|| "Matches".into())
                )];
                lines.extend(results.iter().map(|result| format!("- `{result}`")));
                if truncated {
                    lines.push("_Results truncated._".into());
                }
                blocks.push(Self::markdown_block("matches", lines.join("\n")));
            }
            ToolPayloadOutput::Task {
                task_id,
                session_id,
                parent_session_id,
                access,
                status,
                resumed,
                final_text,
                model_feedback,
                model_provider_id,
                model_adapter_id,
                model_id,
                input_tokens,
                output_tokens,
                reasoning_tokens,
                cache_write_tokens,
                cache_read_tokens,
                total_cost_microusd,
            } => {
                let mut fields = vec![
                    ("Status", status),
                    ("Task", task_id),
                    ("Session", session_id.to_string()),
                    ("Parent session", parent_session_id.to_string()),
                    ("Access", access),
                    ("Resumed", resumed.to_string()),
                ];
                if let Some(provider) = model_provider_id {
                    fields.push(("Provider", provider));
                }
                if let Some(adapter) = model_adapter_id {
                    fields.push(("Adapter", adapter));
                }
                if let Some(model) = model_id {
                    fields.push(("Model", model));
                }
                fields.push((
                    "Tokens",
                    format!(
                        "in {input_tokens} · out {output_tokens} · reasoning {reasoning_tokens}"
                    ),
                ));
                fields.push((
                    "Cache",
                    format!("write {cache_write_tokens} · read {cache_read_tokens}"),
                ));
                fields.push(("Cost", format!("{total_cost_microusd} micro-USD")));
                blocks.push(Self::details_block("task", "Task", &fields));
                if let Some(final_text) = final_text {
                    blocks.push(Self::markdown_block("task-result", final_text));
                }
                if let Some(feedback) = model_feedback {
                    blocks.push(Self::model_feedback_block(&feedback));
                }
            }
            ToolPayloadOutput::ToolSearch { results } => {
                let values = results
                    .iter()
                    .map(|result| format!("`{result}`"))
                    .collect::<Vec<_>>();
                if let Some(block) = Self::list_block("tools", "Available tools", &values) {
                    blocks.push(block);
                } else {
                    blocks.push(Self::markdown_block("tools", "No matching tools."));
                }
            }
            ToolPayloadOutput::AskUser { answers, timed_out } => {
                let mut lines = vec!["### Answers".to_owned()];
                if answers.is_empty() {
                    lines.push("No answers were recorded.".into());
                } else {
                    for (question, values) in answers {
                        let values = if values.is_empty() {
                            "_No answer_".to_owned()
                        } else {
                            values.join(", ")
                        };
                        lines.push(format!("- **{question}**: {values}"));
                    }
                }
                if timed_out {
                    lines.push("_The request timed out._".into());
                }
                blocks.push(Self::markdown_block("answers", lines.join("\n")));
            }
            ToolPayloadOutput::Shell {
                action,
                shell,
                background,
                process_id,
                status,
                output,
                description,
                events,
                processes,
                last_seq,
                has_more,
                dropped_lines,
                exit_code,
            } => {
                let (event_stdout, event_stderr) = Self::event_streams(&events);
                let stdout = output.unwrap_or_else(|| {
                    if !raw.text.is_empty() {
                        raw.text.clone()
                    } else {
                        event_stdout
                    }
                });
                let command = command
                    .filter(|command| !command.trim().is_empty())
                    .map(str::to_owned)
                    .unwrap_or_else(|| format!("shell {action}"));
                blocks.push(ViewBlock::Command {
                    id: Some("command".into()),
                    command,
                    cwd: cwd.map(str::to_owned),
                    exit_code,
                    stdout,
                    stderr: event_stderr,
                });
                let mut fields = vec![
                    ("Action", action),
                    (
                        "Shell",
                        shell.map(|shell| shell.to_string()).unwrap_or_default(),
                    ),
                    ("Background", background.to_string()),
                    ("Status", Self::display_option(status)),
                    ("Process", process_id.unwrap_or_default()),
                    ("Last event", last_seq.to_string()),
                ];
                if let Some(description) = description {
                    fields.push(("Description", description));
                }
                if has_more {
                    fields.push(("Events", "more available".into()));
                }
                if dropped_lines > 0 {
                    fields.push(("Dropped lines", dropped_lines.to_string()));
                }
                if fields.iter().any(|(_, value)| !value.trim().is_empty()) {
                    blocks.push(Self::details_block("process-meta", "Process", &fields));
                }
                if let Some(table) = Self::table_block(
                    "processes",
                    vec!["ID", "Status", "Command", "Exit", "Buffered", "Dropped"],
                    Self::process_rows(&processes),
                ) {
                    blocks.push(table);
                }
            }
            ToolPayloadOutput::Monitor {
                action,
                monitor_id,
                status,
                output,
                processes,
                last_seq,
                exit_code,
                completion_reason,
            } => {
                blocks.push(ViewBlock::Command {
                    id: Some("command".into()),
                    command: format!("monitor {action}"),
                    cwd: None,
                    exit_code,
                    stdout: output.unwrap_or_else(|| raw.text.clone()),
                    stderr: String::new(),
                });
                let fields = vec![
                    ("Action", action),
                    ("Monitor", monitor_id.unwrap_or_default()),
                    ("Status", Self::display_option(status)),
                    ("Last event", last_seq.to_string()),
                    ("Completion", completion_reason.unwrap_or_default()),
                ];
                blocks.push(Self::details_block("monitor-meta", "Monitor", &fields));
                if let Some(table) = Self::table_block(
                    "processes",
                    vec!["ID", "Status", "Command", "Exit", "Buffered", "Dropped"],
                    Self::process_rows(&processes),
                ) {
                    blocks.push(table);
                }
            }
            ToolPayloadOutput::WebFetch {
                url,
                markdown,
                summary,
                truncated,
                cached,
                status,
            } => {
                let mut lines = vec![format!("### [{url}]({url})")];
                lines.push(format!("- **HTTP status**: `{status}`"));
                lines.push(format!("- **Cached**: `{cached}`"));
                if truncated {
                    lines.push("- **Content**: _truncated_".into());
                }
                if let Some(summary) = summary {
                    lines.push(String::new());
                    lines.push(summary);
                }
                if let Some(markdown) = markdown {
                    lines.push(String::new());
                    // The fetched document is already Markdown. Keep it in
                    // the Markdown block so headings, links, and lists stay
                    // readable in both TUI and Web.
                    lines.push(markdown);
                }
                blocks.push(Self::markdown_block("web-fetch", lines.join("\n")));
            }
            ToolPayloadOutput::WebSearch {
                query,
                backend,
                results,
            } => {
                blocks.push(Self::details_block(
                    "search-meta",
                    "Search",
                    &[("Query", query), ("Backend", backend)],
                ));
                let items: Vec<WebSearchResult> = results;
                blocks.push(ViewBlock::SearchResults {
                    id: Some("search".into()),
                    total: Some(items.len() as u64),
                    items,
                });
            }
            ToolPayloadOutput::EnterSnapshot {
                path,
                branch,
                backend,
                note,
            } => {
                blocks.push(Self::details_block(
                    "snapshot",
                    "Snapshot entered",
                    &[
                        ("Path", path),
                        ("Branch", branch),
                        ("Backend", backend.unwrap_or_default()),
                        ("Note", note.unwrap_or_default()),
                    ],
                ));
            }
            ToolPayloadOutput::ExitSnapshot { action, path } => {
                blocks.push(Self::details_block(
                    "snapshot",
                    "Snapshot exited",
                    &[("Action", action), ("Path", path)],
                ));
            }
            ToolPayloadOutput::CronCreate { id, next_fire_at } => {
                blocks.push(Self::details_block(
                    "cron",
                    "Cron job created",
                    &[("ID", id), ("Next run", next_fire_at.unwrap_or_default())],
                ));
            }
            ToolPayloadOutput::CronList { jobs } => {
                if let Some(table) = Self::table_block(
                    "cron-jobs",
                    Self::cron_columns(),
                    Self::cron_job_rows(&jobs),
                ) {
                    blocks.push(table);
                } else {
                    blocks.push(Self::markdown_block("cron-jobs", "No scheduled jobs."));
                }
            }
            ToolPayloadOutput::CronDelete { id, removed } => {
                blocks.push(Self::details_block(
                    "cron",
                    "Cron job deleted",
                    &[("ID", id), ("Removed", removed.to_string())],
                ));
            }
            ToolPayloadOutput::CronUpdate { job } => {
                blocks.push(Self::details_block(
                    "cron",
                    "Cron job updated",
                    &[
                        ("ID", job.id.clone()),
                        (
                            "Status",
                            if job.paused {
                                "paused".into()
                            } else {
                                "active".into()
                            },
                        ),
                    ],
                ));
                if let Some(table) = Self::table_block(
                    "cron-job",
                    Self::cron_columns(),
                    Self::cron_job_rows(&[job]),
                ) {
                    blocks.push(table);
                }
            }
            ToolPayloadOutput::CronPause { job } => {
                blocks.push(Self::details_block(
                    "cron",
                    "Cron job paused",
                    &[("ID", job.id.clone())],
                ));
                if let Some(table) = Self::table_block(
                    "cron-job",
                    Self::cron_columns(),
                    Self::cron_job_rows(&[job]),
                ) {
                    blocks.push(table);
                }
            }
            ToolPayloadOutput::CronResume { job } => {
                blocks.push(Self::details_block(
                    "cron",
                    "Cron job resumed",
                    &[("ID", job.id.clone())],
                ));
                if let Some(table) = Self::table_block(
                    "cron-job",
                    Self::cron_columns(),
                    Self::cron_job_rows(&[job]),
                ) {
                    blocks.push(table);
                }
            }
            ToolPayloadOutput::CronHistory { entries } => {
                if let Some(table) = Self::table_block(
                    "cron-history",
                    vec![
                        "Job",
                        "Triggered",
                        "Finished",
                        "Status",
                        "Scheduled",
                        "Attempt",
                        "Delivery",
                        "Session",
                        "Failure",
                    ],
                    Self::cron_run_rows(&entries),
                ) {
                    blocks.push(table);
                } else {
                    blocks.push(Self::markdown_block("cron-history", "No run history."));
                }
            }
            ToolPayloadOutput::LspDefinition { locations } => {
                let values = locations
                    .iter()
                    .map(|location| format!("`{location}`"))
                    .collect::<Vec<_>>();
                if let Some(block) = Self::list_block("definitions", "Definitions", &values) {
                    blocks.push(block);
                } else {
                    blocks.push(Self::markdown_block("definitions", "No definitions found."));
                }
            }
            ToolPayloadOutput::LspReferences { locations } => {
                let values = locations
                    .iter()
                    .map(|location| format!("`{location}`"))
                    .collect::<Vec<_>>();
                if let Some(block) = Self::list_block("references", "References", &values) {
                    blocks.push(block);
                } else {
                    blocks.push(Self::markdown_block("references", "No references found."));
                }
            }
            ToolPayloadOutput::LspHover { contents } => {
                blocks.push(Self::markdown_block(
                    "hover",
                    contents.unwrap_or_else(|| "No hover information.".into()),
                ));
            }
            ToolPayloadOutput::LspDiagnostics { entries } => {
                let values = entries
                    .iter()
                    .map(|entry| format!("- {entry}"))
                    .collect::<Vec<_>>();
                blocks.push(Self::markdown_block(
                    "diagnostics",
                    if values.is_empty() {
                        "### Diagnostics\nNo diagnostics.".into()
                    } else {
                        format!("### Diagnostics\n{}", values.join("\n"))
                    },
                ));
            }
        }

        Self::append_distinct_raw_text(&mut blocks, raw);
        Self::raw_metadata_blocks(&mut blocks, raw);
        Self::raw_flags(&mut blocks, raw);
        blocks
    }
}

impl ToolHumanRenderer for BuiltinHumanRenderer {
    fn render_human(
        &self,
        _ctx: &RenderContext,
        raw: &RawOutput,
    ) -> Result<Vec<ViewBlock>, RenderError> {
        let output = match ToolOutput::from_json_payload(raw.payload.as_ref()) {
            Ok(output) => output,
            Err(_) => return Ok(Self::fallback(raw)),
        };
        let blocks = Self::structured_blocks(
            &self.tool_name,
            raw,
            &output,
            self.command.as_deref(),
            self.cwd.as_deref(),
        );
        if blocks.is_empty() {
            return Ok(Self::fallback(raw));
        }
        Ok(blocks)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agena_tool::RenderContext as ToolRenderContext;
    use serde_json::json;
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    fn ctx() -> ToolRenderContext {
        ToolRenderContext {
            workspace_root: PathBuf::from("/tmp"),
            command: None,
        }
    }

    #[test]
    fn apply_patch_renders_file_changes_and_diff() {
        let renderer = BuiltinHumanRenderer::new("fs.apply_patch");
        let raw = RawOutput {
            payload: Some(json!({
                "operation_id": "op-1",
                "inverse_patch": "",
                "changes": [{"path": "a.txt", "kind": "updated"}],
                "diff": "--- a\n+++ b\n"
            })),
            text: String::new(),
            ..RawOutput::default()
        };
        let blocks = renderer.render_human(&ctx(), &raw).expect("render");
        assert!(
            blocks
                .iter()
                .any(|b| matches!(b, ViewBlock::FileChanges { .. }))
        );
        assert!(blocks.iter().any(|b| matches!(b, ViewBlock::Diff { .. })));
    }

    #[test]
    fn shell_renders_command_card_with_output_and_status() {
        let renderer = BuiltinHumanRenderer::new("shell")
            .with_command("cargo test")
            .with_cwd("/tmp");
        let raw = RawOutput {
            payload: Some(json!({
                "action": "run",
                "exit_code": 0,
                "output": "ok\n"
            })),
            text: "ok\n".into(),
            ..RawOutput::default()
        };
        let blocks = renderer.render_human(&ctx(), &raw).expect("render");
        let command = blocks.iter().find_map(|block| match block {
            ViewBlock::Command {
                command,
                cwd,
                exit_code,
                stdout,
                ..
            } => Some((command, cwd, exit_code, stdout)),
            _ => None,
        });
        let Some((command, cwd, exit_code, stdout)) = command else {
            panic!("expected command card, got {blocks:?}");
        };
        assert_eq!(command, "cargo test");
        assert_eq!(cwd.as_deref(), Some("/tmp"));
        assert_eq!(*exit_code, Some(0));
        assert_eq!(stdout, "ok\n");
    }

    #[test]
    fn glob_renders_path_list_and_fallback_prefers_text() {
        let renderer = BuiltinHumanRenderer::new("fs.glob");
        let raw = RawOutput {
            payload: Some(json!({ "paths": ["a.rs", "b.rs"], "count": 2 })),
            text: String::new(),
            ..RawOutput::default()
        };
        let blocks = renderer.render_human(&ctx(), &raw).expect("render");
        assert!(
            blocks
                .iter()
                .any(|b| matches!(b, ViewBlock::Markdown { .. }))
        );

        let opaque = BuiltinHumanRenderer::new("unknown_tool");
        let raw = RawOutput {
            payload: Some(json!({ "x": 1 })),
            text: "line\n".into(),
            ..RawOutput::default()
        };
        let blocks = opaque.render_human(&ctx(), &raw).expect("render");
        assert!(matches!(
            blocks.first(),
            Some(ViewBlock::Markdown { id: Some(id), .. }) if id == "text"
        ));
        assert!(blocks.iter().any(|block| {
            matches!(block, ViewBlock::Markdown { id: Some(id), text } if id == "result" && text.contains("X"))
        }));
    }

    #[test]
    fn typed_projection_keeps_distinct_human_text_channel() {
        let renderer = BuiltinHumanRenderer::new("fs.glob");
        let raw = RawOutput {
            payload: Some(json!({ "paths": ["src/lib.rs"], "count": 1 })),
            text: "The search also respected the workspace ignore rules.".into(),
            ..RawOutput::default()
        };
        let blocks = renderer.render_human(&ctx(), &raw).expect("render");
        assert!(blocks.iter().any(|block| {
            matches!(block, ViewBlock::Markdown { id: Some(id), text } if id == "output-text" && text.contains("workspace ignore rules"))
        }));
    }

    #[test]
    fn generic_projection_keeps_short_plugin_summaries_and_structures_payloads() {
        let renderer = BuiltinHumanRenderer::new("settings.inspect");
        let raw = RawOutput {
            payload: Some(json!({
                "path": "providers.openai",
                "global": {"defined": true, "value": "redacted"},
                "workspace": {"defined": false},
                "effective": {"enabled": true},
                "applied_layers": ["global", "environment"],
                "layers": [
                    {"name": "global", "active": true},
                    {"name": "environment", "active": false}
                ]
            })),
            text: "Inspected global, workspace, and effective settings values.".into(),
            ..RawOutput::default()
        };
        let blocks = renderer.render_human(&ctx(), &raw).expect("render");
        assert!(blocks.iter().any(|block| {
            matches!(block, ViewBlock::Markdown { text, .. } if text.contains("Inspected global"))
        }));
        assert!(blocks.iter().any(|block| {
            matches!(block, ViewBlock::Markdown { text, .. } if text.contains("Path") && text.contains("providers.openai"))
        }));
        assert!(blocks.iter().any(|block| {
            matches!(block, ViewBlock::Table { columns, rows, .. } if columns.len() == rows.first().map_or(0, Vec::len))
        }));
        assert!(
            !blocks
                .iter()
                .any(|block| matches!(block, ViewBlock::Json { .. }))
        );
    }

    #[test]
    fn generic_projection_parses_json_text_into_readable_blocks() {
        let renderer = BuiltinHumanRenderer::new("lsp.servers");
        let raw = RawOutput {
            text: serde_json::to_string_pretty(&json!({
                "servers": [
                    {"name": "rust-analyzer", "command": "rust-analyzer", "file_extensions": ["rs"]}
                ]
            }))
            .expect("json"),
            ..RawOutput::default()
        };
        let blocks = renderer.render_human(&ctx(), &raw).expect("render");
        assert!(blocks.iter().any(|block| {
            matches!(block, ViewBlock::Table { columns, rows, .. } if columns.len() == rows[0].len())
        }));
        assert!(
            !blocks
                .iter()
                .any(|block| matches!(block, ViewBlock::Json { .. }))
        );
    }

    #[test]
    fn generic_projection_bounds_long_scalars_and_shows_raw_metadata() {
        let renderer = BuiltinHumanRenderer::new("plugin.inspect");
        let mut metadata = BTreeMap::new();
        metadata.insert("request_id".to_owned(), json!("req-1"));
        metadata.insert("provider_raw".to_owned(), json!({"secret": "omitted"}));
        let long_value = "x".repeat(BuiltinHumanRenderer::GENERIC_MAX_VALUE_CHARS + 200);
        let raw = RawOutput {
            payload: Some(json!({
                "description": long_value,
                "items": [{"name": "first", "detail": long_value}]
            })),
            metadata,
            ..RawOutput::default()
        };

        let blocks = renderer.render_human(&ctx(), &raw).expect("render");
        let text = blocks
            .iter()
            .filter_map(ViewBlock::text_value)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("Request Id"));
        assert!(text.contains("[truncated]"));
        assert!(
            text.len() < BuiltinHumanRenderer::GENERIC_MAX_VALUE_CHARS * 4,
            "generic projection should remain bounded: {} chars",
            text.len()
        );
        assert!(!text.contains("secret"));
    }

    #[test]
    fn generic_projection_marks_bounded_rows_and_nested_records() {
        let renderer = BuiltinHumanRenderer::new("workflow.inspect");
        let items = (0..(BuiltinHumanRenderer::GENERIC_MAX_ROWS + 5))
            .map(|index| {
                json!({
                    "name": format!("step-{index}"),
                    "status": "pending",
                    "checks": [{"text": format!("check-{index}"), "status": "pending"}]
                })
            })
            .collect::<Vec<_>>();
        let raw = RawOutput {
            payload: Some(json!({"steps": items})),
            ..RawOutput::default()
        };

        let blocks = renderer.render_human(&ctx(), &raw).expect("render");
        let text = blocks
            .iter()
            .filter_map(ViewBlock::text_value)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("Showing the first 100 of 105 rows"));
        assert!(text.contains("Nested Checks values were capped at 100"));
    }

    #[test]
    fn empty_raw_output_has_a_visible_human_block() {
        let renderer = BuiltinHumanRenderer::new("plugin.empty");
        let blocks = renderer
            .render_human(&ctx(), &RawOutput::default())
            .expect("render");
        assert!(matches!(
            blocks.as_slice(),
            [ViewBlock::Markdown {
                id: Some(id),
                text
            }] if id == "empty" && text == "No output."
        ));
    }

    #[test]
    fn human_summary_uses_readable_payload_facts_instead_of_json() {
        let raw = RawOutput {
            payload: Some(json!({
                "servers": [{"name": "rust-analyzer"}],
                "status": "connected"
            })),
            text: String::new(),
            ..RawOutput::default()
        };
        assert_eq!(BuiltinHumanRenderer::human_summary(&raw), "1 server");

        let raw = RawOutput {
            payload: Some(json!({"status": "completed"})),
            text: "Completed the operation.".into(),
            ..RawOutput::default()
        };
        assert_eq!(
            BuiltinHumanRenderer::human_summary(&raw),
            "Completed the operation."
        );
    }

    #[test]
    fn opaque_fallback_keeps_truncation_status() {
        let renderer = BuiltinHumanRenderer::new("unknown_tool");
        let raw = RawOutput {
            text: "visible output".into(),
            truncated: true,
            ..RawOutput::default()
        };
        let blocks = renderer.render_human(&ctx(), &raw).expect("render");
        assert!(matches!(blocks.first(), Some(ViewBlock::Markdown { .. })));
        assert!(blocks.iter().any(|block| {
            matches!(block, ViewBlock::Markdown { id: Some(id), text } if id == "output-meta" && text.contains("truncated"))
        }));
    }

    #[test]
    fn web_fetch_keeps_fetched_markdown_out_of_a_code_fence() {
        let renderer = BuiltinHumanRenderer::new("web.fetch");
        let raw = RawOutput {
            payload: Some(json!({
                "url": "https://example.test",
                "markdown": "## Heading\n\n- item",
                "summary": "A page",
                "status": 200,
                "cached": false
            })),
            ..RawOutput::default()
        };
        let blocks = renderer.render_human(&ctx(), &raw).expect("render");
        let text = blocks
            .iter()
            .find_map(ViewBlock::text_value)
            .expect("markdown block");
        assert!(text.contains("## Heading"));
        assert!(!text.contains("```markdown"));
    }

    #[test]
    fn cron_and_process_results_use_readable_views() {
        let cron = BuiltinHumanRenderer::new("cron.list");
        let raw = RawOutput {
            payload: Some(json!({
                "jobs": [{
                    "id": "job-1",
                    "kind": "interval",
                    "expression": "*/5 * * * *",
                    "prompt": "check status",
                    "paused": false,
                    "completed": false,
                    "misfire_policy": "skip",
                    "retry_max_attempts": 1,
                    "run_count": 2
                }]
            })),
            ..RawOutput::default()
        };
        let blocks = cron.render_human(&ctx(), &raw).expect("render");
        assert!(blocks.iter().any(|b| matches!(b, ViewBlock::Table { .. })));
        assert!(!format!("{blocks:?}").contains("CronJobSummary"));

        let monitor = BuiltinHumanRenderer::new("monitor.start");
        let raw = RawOutput {
            payload: Some(json!({
                "action": "start",
                "monitor_id": "mon-1",
                "status": "running",
                "processes": []
            })),
            ..RawOutput::default()
        };
        let blocks = monitor.render_human(&ctx(), &raw).expect("render");
        assert!(
            blocks
                .iter()
                .any(|b| matches!(b, ViewBlock::Command { .. }))
        );
        assert!(
            blocks
                .iter()
                .any(|b| matches!(b, ViewBlock::Markdown { .. }))
        );
    }

    #[test]
    fn every_typed_family_has_a_non_json_human_projection() {
        let cases = [
            (
                "fs.read",
                json!({"preview": "hello", "loaded_paths": ["a.txt"]}),
            ),
            ("fs.grep", json!({"matches": 1, "results": ["a.rs:1: hit"]})),
            (
                "tasks.run",
                json!({"task_id":"t","session_id":1,"parent_session_id":0,"access":"read","status":"completed","model_feedback":{"kind":"invalid_input"}}),
            ),
            ("tools.search", json!({"results":["fs.read"]})),
            ("interaction.ask", json!({"answers":{"0":["yes"]}})),
            ("snapshot.enter", json!({"path":"/tmp/s","branch":"main"})),
            ("snapshot.exit", json!({"action":"restore","path":"/tmp/s"})),
            ("cron.create", json!({"id":"j","next_fire_at":"tomorrow"})),
            ("cron.delete", json!({"id":"j","removed":true})),
            ("lsp.hover", json!({"contents":"**hover**"})),
            ("lsp.diagnostics", json!({"entries":["a.rs:1 warning"]})),
        ];
        for (tool, payload) in cases {
            let raw = RawOutput {
                payload: Some(payload),
                ..RawOutput::default()
            };
            let blocks = BuiltinHumanRenderer::new(tool)
                .render_human(&ctx(), &raw)
                .expect("render");
            assert!(
                blocks
                    .iter()
                    .any(|block| !matches!(block, ViewBlock::Json { .. })),
                "{tool} rendered only JSON: {blocks:?}"
            );
        }
    }
}
