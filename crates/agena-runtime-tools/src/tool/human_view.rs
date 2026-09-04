//! Built-in human renderer for tool results.
//!
//! Tool output is durable machine-readable data, but the transcript should
//! present the useful facts a person needs to make a decision. This renderer
//! is the shared fallback for bundled tools: structured results become
//! Markdown, tables, command cards, diffs, or search results instead of a
//! generic JSON dump. JSON remains the last-resort representation for an
//! opaque result that has no readable text projection.

use std::collections::BTreeSet;

use agena_domain::{
    ArtifactRef, AttachmentSource, RawOutput, ToolOutput, ViewBlock, WebSearchResult,
};
use agena_tool::{
    CronJobSummary, CronRunSummary, RenderContext, RenderError, ToolHumanRenderer,
    normalize_tool_summary, result_title_fragment_for_tool,
};
use chrono::{DateTime, Utc};
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
    const HUMAN_TEXT_MAX_CHARS: usize = 12_000;

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

    /// Derive the one-line result summary with the same tool-specific facts
    /// used by the collapsed title. Generic payload summaries remain the
    /// fallback so a descriptive message is preferred over a bare completed
    /// status.
    pub fn human_summary_for_tool(tool_name: &str, raw: &RawOutput) -> String {
        let specialized = result_title_fragment_for_tool(tool_name, raw);
        if !specialized.is_empty()
            && !matches!(
                specialized.as_str(),
                "completed" | "running" | "queued" | "failed"
            )
        {
            return normalize_tool_summary(specialized);
        }
        Self::human_summary(raw)
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
            "snapshots",
            "resources",
            "resource_templates",
            "prompts",
            "jobs",
            "entries",
            "steps",
            "paths",
            "matches",
            "changes",
            "loaded_paths",
            "files",
            "memories",
            "sources",
            "warnings",
            "events",
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

        for key in [
            "count",
            "total",
            "tool_count",
            "finding_count",
            "file_count",
            "source_count",
            "snapshot_count",
            "warning_count",
            "event_count",
            "memory_count",
        ] {
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
            ("snapshots", true) => "snapshot",
            ("snapshots", false) => "snapshots",
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
            ("changes", true) => "file changed",
            ("changes", false) => "files changed",
            ("loaded_paths", true) => "file loaded",
            ("loaded_paths", false) => "files loaded",
            ("files", true) => "file",
            ("files", false) => "files",
            ("matches", true) => "match",
            ("matches", false) => "matches",
            ("memories", true) => "memory",
            ("memories", false) => "memories",
            ("sources", true) => "source",
            ("sources", false) => "sources",
            ("warnings", true) => "warning",
            ("warnings", false) => "warnings",
            ("events", true) => "event",
            ("events", false) => "events",
            ("items", true) => "item",
            ("items", false) => "items",
            ("tasks", true) => "task",
            ("tasks", false) => "tasks",
            ("count", _) => "items",
            ("total", _) => "items",
            ("tool_count", _) => "tools",
            ("finding_count", _) => "findings",
            ("file_count", _) => "files",
            ("source_count", _) => "sources",
            ("snapshot_count", _) => "snapshots",
            ("warning_count", _) => "warnings",
            ("event_count", _) => "events",
            ("memory_count", _) => "memories",
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
                text: Self::bounded_human_text(raw.text.as_str()),
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
                        value: Self::redacted_setting_value(payload, None),
                    });
                }
            } else if let Some(value) = text_json {
                blocks.push(ViewBlock::Json {
                    id: Some("payload".into()),
                    value: Self::redacted_setting_value(&value, None),
                });
            } else if !raw.text.trim().is_empty() {
                // Keep an unusual but readable text result visible even if a
                // future generic projection decides it cannot classify it.
                blocks.push(ViewBlock::Markdown {
                    id: Some("text".into()),
                    text: Self::bounded_human_text(raw.text.as_str()),
                });
            }
        }

        Self::raw_metadata_blocks(&mut blocks, raw);
        Self::raw_media_blocks(&mut blocks, raw);
        Self::raw_attachment_blocks(&mut blocks, raw);
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

    /// Keep a long human document useful in a collapsed presentation by
    /// retaining both its beginning and its conclusion. The durable raw
    /// output still contains the complete text; this bound only controls the
    /// eagerly rendered view.
    fn bounded_human_text(value: &str) -> String {
        let character_count = value.chars().count();
        if character_count <= Self::HUMAN_TEXT_MAX_CHARS {
            return value.to_owned();
        }
        let head_count = Self::HUMAN_TEXT_MAX_CHARS / 2;
        let tail_count = Self::HUMAN_TEXT_MAX_CHARS - head_count;
        let head = value.chars().take(head_count).collect::<String>();
        let tail = value
            .chars()
            .rev()
            .take(tail_count)
            .collect::<String>()
            .chars()
            .rev()
            .collect::<String>();
        format!("{head}\n… [middle truncated] …\n{tail}")
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
        if matches!(
            key,
            "input_schema" | "output_schema" | "schema" | "provider_raw" | "raw" | "trace"
        ) {
            return true;
        }
        // A generic fallback must never turn a credential-bearing payload
        // into a readable secret dump. Settings have an additional
        // path-aware redaction pass below; this key-level guard covers
        // metadata and unknown plugin envelopes.
        Self::sensitive_setting_key(key)
    }

    fn sensitive_setting_key(key: &str) -> bool {
        let key = key.to_ascii_lowercase();
        [
            "password",
            "passcode",
            "secret",
            "api_key",
            "apikey",
            "authorization",
            "access_token",
            "refresh_token",
            "private_key",
            "client_secret",
            "credential",
            "cookie",
        ]
        .iter()
        .any(|marker| key.contains(marker))
            || key == "token"
            || key.ends_with("_token")
    }

    fn setting_path_is_sensitive(path: &str) -> bool {
        let path = path.to_ascii_lowercase();
        Self::sensitive_setting_key(path.as_str())
            || path.split(['.', '/', ':']).any(Self::sensitive_setting_key)
    }

    /// Redact settings without hiding useful non-secret configuration. A
    /// setting named `providers.openai.api_key` is masked even though its
    /// actual field is simply called `value`; nested records are also
    /// redacted when their own `path` identifies a secret setting.
    fn redacted_setting_value(value: &Value, path_hint: Option<&str>) -> Value {
        fn redact(value: &Value, inherited_sensitive: bool, path_hint: Option<&str>) -> Value {
            if inherited_sensitive {
                return Value::String("[redacted]".to_owned());
            }
            match value {
                Value::Object(object) => {
                    let object_path = object
                        .get("path")
                        .or_else(|| object.get("key"))
                        .or_else(|| object.get("setting"))
                        .and_then(Value::as_str)
                        .or(path_hint);
                    let object_sensitive =
                        object_path.is_some_and(BuiltinHumanRenderer::setting_path_is_sensitive);
                    let mut sanitized = serde_json::Map::new();
                    for (key, child) in object {
                        if BuiltinHumanRenderer::sensitive_setting_key(key)
                            || (object_sensitive
                                && matches!(key.as_str(), "value" | "current" | "previous"))
                        {
                            sanitized.insert(key.clone(), Value::String("[redacted]".to_owned()));
                        } else {
                            sanitized.insert(key.clone(), redact(child, false, object_path));
                        }
                    }
                    Value::Object(sanitized)
                }
                Value::Array(values) => Value::Array(
                    values
                        .iter()
                        .map(|value| redact(value, false, path_hint))
                        .collect(),
                ),
                value => value.clone(),
            }
        }

        redact(
            value,
            path_hint.is_some_and(Self::setting_path_is_sensitive),
            path_hint,
        )
    }

    fn redacted_setting_records(values: &[Value]) -> Vec<Value> {
        values
            .iter()
            .map(|value| {
                let Some(object) = value.as_object() else {
                    return value.clone();
                };
                let path = object
                    .get("path")
                    .or_else(|| object.get("key"))
                    .and_then(Value::as_str);
                Self::redacted_setting_value(value, path)
            })
            .collect()
    }

    fn generic_object_table(id: &str, values: &[Value]) -> Option<ViewBlock> {
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
            id: Some(id.to_owned()),
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
                if let Some(table) = Self::generic_object_table(id, values) {
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

    fn attachment_artifact(attachment: &agena_domain::AttachmentItem) -> ArtifactRef {
        let uri = match &attachment.source {
            AttachmentSource::Url { url } | AttachmentSource::DataUrl { url } => url.clone(),
            AttachmentSource::Base64 { data } => {
                format!("data:{};base64,{data}", attachment.mime)
            }
            AttachmentSource::FileId { file_id } => format!("file-id:{file_id}"),
            AttachmentSource::LocalPath { path } => format!("file://{path}"),
        };
        ArtifactRef {
            uri,
            mime: attachment.mime.clone(),
            name: attachment
                .filename
                .clone()
                .or_else(|| attachment.title.clone()),
            size_bytes: attachment.size_bytes,
            sha256: attachment.sha256.clone(),
        }
    }

    fn raw_media_blocks(blocks: &mut Vec<ViewBlock>, raw: &RawOutput) {
        for (index, attachment) in raw.attachments.iter().enumerate() {
            let id = if raw.attachments.len() == 1 {
                "media".to_owned()
            } else {
                format!("media-{index}")
            };
            blocks.push(ViewBlock::Media {
                id: Some(id),
                artifact: Self::attachment_artifact(attachment),
            });
        }
    }

    fn raw_attachment_blocks(blocks: &mut Vec<ViewBlock>, raw: &RawOutput) {
        if raw.attachments.is_empty()
            || blocks.iter().any(|block| {
                matches!(
                    block.block_id(),
                    Some("attachment") | Some("attachments") | Some("image-meta")
                )
            })
        {
            return;
        }
        if raw.attachments.len() == 1 {
            let attachment = &raw.attachments[0];
            let fields = [
                ("Kind", attachment.kind.to_string()),
                ("Label", attachment.summary_label()),
                ("MIME", attachment.mime.clone()),
                (
                    "Source",
                    attachment
                        .source
                        .summary_hint()
                        .map(Self::bounded_generic_text)
                        .unwrap_or_default(),
                ),
                (
                    "Size",
                    attachment
                        .size_bytes
                        .map(|size| format!("{size} bytes"))
                        .unwrap_or_default(),
                ),
                (
                    "Dimensions",
                    match (attachment.width, attachment.height) {
                        (Some(width), Some(height)) => format!("{width} × {height}"),
                        _ => String::new(),
                    },
                ),
                (
                    "Duration",
                    attachment
                        .duration_ms
                        .map(|duration| format!("{duration} ms"))
                        .unwrap_or_default(),
                ),
                (
                    "Pages",
                    attachment
                        .page_count
                        .map(|pages| pages.to_string())
                        .unwrap_or_default(),
                ),
                ("SHA-256", attachment.sha256.clone().unwrap_or_default()),
            ];
            blocks.push(Self::details_block("attachments", "Attachment", &fields));
            return;
        }

        let rows = raw
            .attachments
            .iter()
            .map(|attachment| {
                vec![
                    Value::String(attachment.kind.to_string()),
                    Value::String(attachment.summary_label()),
                    Value::String(attachment.mime.clone()),
                    Value::String(
                        attachment
                            .source
                            .summary_hint()
                            .map(Self::bounded_generic_text)
                            .unwrap_or_default(),
                    ),
                    attachment
                        .size_bytes
                        .map(|size| Value::String(format!("{size} bytes")))
                        .unwrap_or(Value::Null),
                ]
            })
            .collect::<Vec<_>>();
        blocks.push(ViewBlock::Table {
            id: Some("attachments".into()),
            columns: vec![
                "Kind".into(),
                "Label".into(),
                "MIME".into(),
                "Source".into(),
                "Size".into(),
            ],
            rows,
        });
    }

    fn markdown_block(id: impl Into<String>, text: impl Into<String>) -> ViewBlock {
        ViewBlock::Markdown {
            id: Some(id.into()),
            text: text.into(),
        }
    }

    /// Render source/text previews with a fence that cannot be closed by the
    /// content itself. Tool output is untrusted user/workspace data; a fixed
    /// triple-backtick fence would turn a file containing ``` into malformed
    /// Markdown and could make the rest of the presentation look like code.
    fn markdown_code_block(
        id: impl Into<String>,
        heading: &str,
        text: &str,
        language: Option<&str>,
    ) -> ViewBlock {
        let max_backticks = text
            .split(|character| character != '`')
            .map(str::len)
            .max()
            .unwrap_or_default();
        let fence = "`".repeat(max_backticks.max(2) + 1);
        let language = language.unwrap_or_default();
        Self::markdown_block(
            id,
            format!(
                "### {heading}\n{fence}{language}\n{}\n{fence}",
                Self::bounded_human_text(text)
            ),
        )
    }

    fn source_language(path: &str) -> Option<&'static str> {
        let extension = path.rsplit_once('.')?.1.to_ascii_lowercase();
        Some(match extension.as_str() {
            "rs" => "rust",
            "js" | "jsx" | "mjs" | "cjs" => "javascript",
            "ts" | "tsx" => "typescript",
            "py" => "python",
            "rb" => "ruby",
            "go" => "go",
            "java" => "java",
            "c" => "c",
            "h" | "hpp" | "cc" | "cpp" => "cpp",
            "cs" => "csharp",
            "php" => "php",
            "sh" | "bash" | "zsh" => "bash",
            "ps1" => "powershell",
            "json" => "json",
            "yaml" | "yml" => "yaml",
            "toml" => "toml",
            "html" | "htm" => "html",
            "css" => "css",
            "sql" => "sql",
            "md" | "markdown" => "markdown",
            _ => return None,
        })
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

    fn details_block_if_nonempty(
        id: impl Into<String>,
        title: &str,
        fields: &[(&str, String)],
    ) -> Option<ViewBlock> {
        fields
            .iter()
            .any(|(_, value)| !value.trim().is_empty())
            .then(|| Self::details_block(id, title, fields))
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
                    if let Some(Value::String(message)) = user.get(key)
                        && !message.trim().is_empty()
                    {
                        return message.clone();
                    }
                }
            }
            for key in ["message", "detail", "fallback"] {
                if let Some(Value::String(message)) = object.get(key)
                    && !message.trim().is_empty()
                {
                    return message.clone();
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

    fn event_log_blocks(events: &[agena_domain::ProcessEvent]) -> Vec<ViewBlock> {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        for event in events {
            match event.stream {
                agena_domain::ProcessStream::Stdout => stdout.push(event.line.clone()),
                agena_domain::ProcessStream::Stderr => stderr.push(event.line.clone()),
            }
        }
        let mut blocks = Vec::new();
        if !stdout.is_empty() {
            blocks.push(ViewBlock::Log {
                id: Some("stdout".into()),
                stream: agena_domain::CommandOutputStream::Stdout,
                text: Self::bounded_human_text(stdout.join("\n").as_str()),
            });
        }
        if !stderr.is_empty() {
            blocks.push(ViewBlock::Log {
                id: Some("stderr".into()),
                stream: agena_domain::CommandOutputStream::Stderr,
                text: Self::bounded_human_text(stderr.join("\n").as_str()),
            });
        }
        blocks
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
                    .map(Self::bounded_generic_text)
                    .unwrap_or_default();
                let schedule = match job.timezone.as_deref() {
                    Some(timezone) if !timezone.trim().is_empty() && !schedule.is_empty() => {
                        format!("{schedule} ({})", Self::bounded_generic_text(timezone))
                    }
                    _ => schedule,
                };
                let status = if job.paused {
                    "paused"
                } else if job.completed {
                    "completed"
                } else {
                    job.last_run_status.as_deref().unwrap_or("ready")
                };
                let failure = job
                    .last_run_failure
                    .as_ref()
                    .map(Self::serialized)
                    .map(|value| Self::readable_problem(&value))
                    .map(|value| Self::bounded_generic_text(&value))
                    .unwrap_or_default();
                vec![
                    json!(job.id),
                    json!(schedule),
                    json!(status),
                    json!(
                        job.next_fire_at
                            .as_deref()
                            .map(Self::compact_timestamp)
                            .unwrap_or_default()
                    ),
                    json!(Self::bounded_generic_text(&job.prompt)),
                    json!(failure),
                ]
            })
            .collect()
    }

    fn cron_columns() -> Vec<&'static str> {
        vec!["ID", "Schedule", "Status", "Next", "Task", "Issue"]
    }

    /// ISO timestamps are useful in the raw result but too wide for a table
    /// cell and a collapsed title. Keep the date, minute, and timezone while
    /// dropping seconds/fractional precision.
    fn compact_timestamp(value: &str) -> String {
        let value = value.trim();
        if value.is_ascii()
            && value.len() >= 16
            && value.as_bytes().get(10) == Some(&b'T')
            && value.as_bytes().get(13) == Some(&b':')
        {
            let mut compact = format!("{} {}", &value[..10], &value[11..16]);
            if value.ends_with('Z') {
                compact.push('Z');
            } else if value.as_bytes().get(19) == Some(&b'+')
                || value.as_bytes().get(19) == Some(&b'-')
            {
                let end = value
                    .char_indices()
                    .skip(19)
                    .take_while(|(_, character)| *character != ' ')
                    .map(|(index, character)| index + character.len_utf8())
                    .last()
                    .unwrap_or(value.len())
                    .min(value.len());
                compact.push_str(&value[19..end.min(25)]);
            }
            return compact;
        }
        Self::bounded_generic_text(value)
    }

    fn compact_epoch_millis(value: &Value) -> String {
        let Some(millis) = value
            .as_i64()
            .or_else(|| value.as_u64().and_then(|value| i64::try_from(value).ok()))
        else {
            return Self::generic_field_value(value);
        };
        DateTime::<Utc>::from_timestamp_millis(millis)
            .map(|timestamp| timestamp.format("%Y-%m-%d %H:%M UTC").to_string())
            .unwrap_or_else(|| millis.to_string())
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
                    .map(|value| Self::bounded_generic_text(&value))
                    .unwrap_or_default();
                vec![
                    json!(entry.job_id),
                    json!(Self::compact_timestamp(&entry.triggered_at)),
                    json!(entry.status),
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
        blocks.push(Self::markdown_block(
            "output-text",
            Self::bounded_human_text(raw.text.as_str()),
        ));
    }

    fn normalized_tool_name(tool_name: &str) -> String {
        let mut key = tool_name.trim().replace("__", ".").replace('/', ".");
        if let Some(stripped) = key.strip_prefix("agena.") {
            key = stripped.to_owned();
        } else if let Some(stripped) = key.strip_prefix("agena_") {
            key = stripped.replace('_', ".");
        }
        key.to_ascii_lowercase()
    }

    fn object_text(object: &serde_json::Map<String, Value>, key: &str) -> String {
        object
            .get(key)
            .filter(|value| !value.is_null())
            .map(Self::generic_field_value)
            .unwrap_or_default()
    }

    fn object_string(object: &serde_json::Map<String, Value>, key: &str) -> Option<String> {
        object
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
    }

    fn object_array<'a>(
        object: &'a serde_json::Map<String, Value>,
        key: &str,
    ) -> Option<&'a Vec<Value>> {
        object.get(key).and_then(Value::as_array)
    }

    fn scalar_table(
        id: impl Into<String>,
        title: &str,
        values: &[Value],
        columns: &[(&str, &str)],
    ) -> Option<ViewBlock> {
        if values.is_empty() {
            return None;
        }

        // Plugin envelopes are open-ended. When a backend returns a compact
        // scalar list instead of records, keep every item visible as a
        // bounded Markdown list rather than silently producing an empty
        // table. The same path also handles mixed object/scalar arrays.
        if values.iter().any(|value| !value.is_object()) {
            let lines = values
                .iter()
                .take(Self::GENERIC_MAX_ROWS)
                .map(Self::generic_field_value)
                .map(|value| format!("- {value}"))
                .collect::<Vec<_>>();
            let suffix = (values.len() > Self::GENERIC_MAX_ROWS).then(|| {
                format!(
                    "\n\n_Showing the first {} of {} items._",
                    Self::GENERIC_MAX_ROWS,
                    values.len()
                )
            });
            return Some(Self::markdown_block(
                id,
                format!(
                    "### {title}\n{}{}",
                    lines.join("\n"),
                    suffix.unwrap_or_default()
                ),
            ));
        }

        let rows = values
            .iter()
            .filter_map(Value::as_object)
            .map(|object| {
                columns
                    .iter()
                    .map(|(key, _)| {
                        object
                            .get(*key)
                            .map(Self::bounded_generic_value)
                            .unwrap_or(Value::Null)
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        Self::table_block(id, columns.iter().map(|(_, label)| *label).collect(), rows).or_else(
            || {
                Some(Self::markdown_block(
                    format!("{}-empty", title.to_ascii_lowercase().replace(' ', "-")),
                    format!(
                        "### {title}\n{}",
                        values
                            .iter()
                            .take(Self::GENERIC_MAX_ROWS)
                            .map(Self::generic_field_value)
                            .map(|value| format!("- {value}"))
                            .collect::<Vec<_>>()
                            .join("\n")
                    ),
                ))
            },
        )
    }

    fn grep_rows(results: &[String]) -> Vec<Vec<Value>> {
        results
            .iter()
            .map(|result| {
                // The grep executor emits `path:line: text`. Accept the
                // optional `path:line:column: text` form as well, while
                // retaining the complete record when a future backend uses a
                // different diagnostic format.
                let parts = result.splitn(4, ':').collect::<Vec<_>>();
                if parts.len() >= 3 && parts[1].trim().parse::<u64>().is_ok() {
                    let (column, text) =
                        if parts.len() == 4 && parts[2].trim().parse::<u64>().is_ok() {
                            (Value::String(parts[2].trim().to_owned()), parts[3].trim())
                        } else {
                            (Value::Null, parts[2].trim())
                        };
                    vec![
                        Value::String(parts[0].trim().to_owned()),
                        Value::String(parts[1].trim().to_owned()),
                        column,
                        Value::String(Self::bounded_generic_text(text)),
                    ]
                } else {
                    vec![
                        Value::String(Self::bounded_generic_text(result)),
                        Value::Null,
                        Value::Null,
                        Value::Null,
                    ]
                }
            })
            .collect()
    }

    fn diagnostic_rows(values: &[Value]) -> Vec<Vec<Value>> {
        values.iter().map(Self::diagnostic_row).collect()
    }

    fn diagnostic_row(value: &Value) -> Vec<Value> {
        let (location, severity, message) = match value {
            Value::String(text) => Self::parse_diagnostic_text(text),
            Value::Object(object) => Self::diagnostic_object_parts(object),
            other => (
                String::new(),
                String::new(),
                Self::generic_field_value(other),
            ),
        };
        vec![
            Value::String(if location.is_empty() {
                "—".to_owned()
            } else {
                Self::bounded_generic_text(location.as_str())
            }),
            Value::String(if severity.is_empty() {
                "—".to_owned()
            } else {
                Self::bounded_generic_text(severity.as_str())
            }),
            Value::String(if message.is_empty() {
                "—".to_owned()
            } else {
                Self::bounded_generic_text(message.as_str())
            }),
        ]
    }

    fn diagnostic_object_parts(
        object: &serde_json::Map<String, Value>,
    ) -> (String, String, String) {
        let mut location = object
            .get("location")
            .or_else(|| object.get("path"))
            .or_else(|| object.get("file"))
            .or_else(|| object.get("file_path"))
            .or_else(|| object.get("uri"))
            .map(Self::diagnostic_location_value)
            .unwrap_or_default();

        let mut line = object
            .get("line")
            .or_else(|| object.get("line_number"))
            .and_then(Self::diagnostic_number);
        let mut column = object
            .get("column")
            .or_else(|| object.get("character"))
            .and_then(Self::diagnostic_number);

        if let Some(range) = object.get("range").and_then(Value::as_object)
            && let Some(start) = range.get("start").and_then(Value::as_object)
        {
            line = line.or_else(|| {
                start
                    .get("line")
                    .and_then(Self::diagnostic_number)
                    .map(|value| value.saturating_add(1))
            });
            column = column.or_else(|| {
                start
                    .get("character")
                    .or_else(|| start.get("column"))
                    .and_then(Self::diagnostic_number)
                    .map(|value| value.saturating_add(1))
            });
        }

        if location.is_empty() {
            location = match (line, column) {
                (Some(line), Some(column)) => format!("line {line}:{column}"),
                (Some(line), None) => format!("line {line}"),
                _ => String::new(),
            };
        }

        if !location.is_empty() {
            if let Some(line) = line
                && !location
                    .rsplit_once(':')
                    .and_then(|(_, value)| value.parse::<u64>().ok())
                    .is_some_and(|value| value == line)
            {
                location.push(':');
                location.push_str(line.to_string().as_str());
            }
            if let Some(column) = column
                && !location.ends_with(format!(":{column}").as_str())
            {
                location.push(':');
                location.push_str(column.to_string().as_str());
            }
        }

        let severity = object
            .get("severity")
            .or_else(|| object.get("level"))
            .or_else(|| object.get("kind"))
            .map(Self::diagnostic_severity_value)
            .unwrap_or_default();
        let message = object
            .get("message")
            .or_else(|| object.get("text"))
            .or_else(|| object.get("description"))
            .map(Self::diagnostic_text_value)
            .unwrap_or_default();
        (location, severity, message)
    }

    fn diagnostic_location_value(value: &Value) -> String {
        match value {
            Value::String(value) => value.trim().to_owned(),
            Value::Object(object) => {
                let path = object
                    .get("path")
                    .or_else(|| object.get("file"))
                    .or_else(|| object.get("file_path"))
                    .or_else(|| object.get("uri"))
                    .map(Self::diagnostic_text_value)
                    .unwrap_or_default();
                let line = object.get("line").and_then(Self::diagnostic_number);
                let column = object
                    .get("column")
                    .or_else(|| object.get("character"))
                    .and_then(Self::diagnostic_number);
                if path.is_empty() {
                    Self::generic_field_value(value)
                } else {
                    match (line, column) {
                        (Some(line), Some(column)) => format!("{path}:{line}:{column}"),
                        (Some(line), None) => format!("{path}:{line}"),
                        _ => path,
                    }
                }
            }
            _ => Self::diagnostic_text_value(value),
        }
    }

    fn diagnostic_text_value(value: &Value) -> String {
        match value {
            Value::String(value) => value.trim().to_owned(),
            _ => Self::generic_field_value(value),
        }
    }

    fn diagnostic_number(value: &Value) -> Option<u64> {
        value
            .as_u64()
            .or_else(|| value.as_i64().and_then(|value| u64::try_from(value).ok()))
            .or_else(|| value.as_str().and_then(|value| value.trim().parse().ok()))
    }

    fn diagnostic_severity_value(value: &Value) -> String {
        let severity = Self::diagnostic_text_value(value).to_ascii_lowercase();
        match severity.as_str() {
            "1" => "error".to_owned(),
            "2" => "warning".to_owned(),
            "3" => "info".to_owned(),
            "4" => "hint".to_owned(),
            _ => severity,
        }
    }

    fn parse_diagnostic_text(text: &str) -> (String, String, String) {
        let text = text.trim();
        if text.is_empty() {
            return (String::new(), String::new(), String::new());
        }

        let mut location_end = None;
        for (index, character) in text.char_indices() {
            if character != ':' {
                continue;
            }
            let after = &text[index + character.len_utf8()..];
            let line_digits = after
                .char_indices()
                .take_while(|(_, character)| character.is_ascii_digit())
                .map(|(index, character)| index + character.len_utf8())
                .last()
                .unwrap_or_default();
            if line_digits == 0 {
                continue;
            }
            let mut end = index + character.len_utf8() + line_digits;
            let rest = &text[end..];
            if let Some(column_start) = rest.strip_prefix(':') {
                let column_digits = column_start
                    .char_indices()
                    .take_while(|(_, character)| character.is_ascii_digit())
                    .map(|(index, character)| index + character.len_utf8())
                    .last()
                    .unwrap_or_default();
                if column_digits > 0 {
                    end += 1 + column_digits;
                }
            }
            let remainder = &text[end..];
            if remainder.is_empty()
                || remainder
                    .chars()
                    .next()
                    .is_some_and(|character| character.is_whitespace() || character == ':')
            {
                location_end = Some(end);
                break;
            }
        }

        let (location, remainder) = location_end
            .map(|end| (text[..end].trim().to_owned(), text[end..].trim()))
            .unwrap_or_else(|| (String::new(), text));
        let remainder = remainder.trim_start_matches([':', ' ']).trim();
        let mut words = remainder.splitn(2, |character: char| {
            character.is_whitespace() || character == ':'
        });
        let first = words.next().unwrap_or_default().trim();
        let severity_token = first.trim_matches(['[', ']']).trim();
        let severity = Self::diagnostic_severity_value(&Value::String(severity_token.to_owned()));
        let known_severity = matches!(
            severity.as_str(),
            "error" | "warning" | "warn" | "info" | "information" | "hint" | "note"
        );
        if known_severity {
            let message = words
                .next()
                .unwrap_or_default()
                .trim_start_matches(':')
                .trim()
                .to_owned();
            return (location, severity, message);
        }
        (location, String::new(), remainder.to_owned())
    }

    fn mcp_content_text(value: &Value) -> String {
        match value {
            Value::String(text) => Self::bounded_generic_text(text),
            Value::Object(object) => ["text", "markdown", "content", "value"]
                .iter()
                .find_map(|key| object.get(*key).and_then(Value::as_str))
                .map(Self::bounded_generic_text)
                .unwrap_or_else(|| Self::bounded_generic_text(&Self::display_value(value))),
            value => Self::bounded_generic_text(&Self::display_value(value)),
        }
    }

    fn mcp_content_rows(values: &[Value]) -> Vec<Vec<Value>> {
        values
            .iter()
            .filter_map(Value::as_object)
            .map(|object| {
                vec![
                    Value::String(Self::object_text(object, "type")),
                    Value::String(
                        object
                            .get("text")
                            .or_else(|| object.get("content"))
                            .map(Self::mcp_content_text)
                            .unwrap_or_default(),
                    ),
                    Value::String(Self::object_text(object, "mime_type")),
                    Value::String(Self::object_text(object, "uri")),
                ]
            })
            .collect()
    }

    fn mcp_prompt_message_rows(values: &[Value]) -> Vec<Vec<Value>> {
        values
            .iter()
            .filter_map(Value::as_object)
            .map(|object| {
                vec![
                    Value::String(Self::object_text(object, "role")),
                    Value::String(
                        object
                            .get("content")
                            .map(Self::mcp_content_text)
                            .unwrap_or_default(),
                    ),
                ]
            })
            .collect()
    }

    fn provider_operation_label(tool_name: &str) -> &'static str {
        match tool_name {
            "chatgpt.web_search"
            | "chatgpt.web_search_preview"
            | "claude.web_search"
            | "gemini.google_search" => "Web search",
            "claude.web_fetch" | "gemini.url_context" => "Web retrieval",
            "chatgpt.file_search" | "claude.file_search" | "gemini.file_search" => "File search",
            "gemini.google_maps" => "Map search",
            "gemini.retrieval" => "Context retrieval",
            "chatgpt.code_interpreter" | "claude.code_execution" | "gemini.code_execution" => {
                "Code execution"
            }
            "chatgpt.computer"
            | "chatgpt.computer_use_preview"
            | "claude.computer"
            | "gemini.computer_use" => "Computer action",
            "chatgpt.local_shell" | "chatgpt.shell" | "claude.bash" => "Shell execution",
            "chatgpt.mcp" | "gemini.mcp_server" | "claude.mcp_toolset" => "MCP connection",
            "claude.memory" => "Memory operation",
            "claude.text_editor" | "claude.str_replace_based_edit_tool" => "Text edit",
            "claude.advisor" => "Advisor response",
            "chatgpt.apply_patch" => "Patch operation",
            "chatgpt.function" | "gemini.function" => "Function call",
            "chatgpt.custom" => "Custom tool call",
            "chatgpt.namespace" => "Namespace call",
            "chatgpt.programmatic_tool_calling" => "Programmatic call",
            "chatgpt.tool_search"
            | "claude.tool_search_bm25"
            | "claude.tool_search_regex"
            | "claude.tool_search_tool_bm25"
            | "claude.tool_search_tool_regex" => "Tool search",
            value if value.contains("image") => "Image output",
            value
                if value.contains("function")
                    || value.contains("custom")
                    || value.contains("namespace")
                    || value.contains("programmatic") =>
            {
                "Tool call"
            }
            _ => "Provider operation",
        }
    }

    fn empty_state_block(tool_name: &str) -> ViewBlock {
        let key = Self::normalized_tool_name(tool_name);
        if key.starts_with("chatgpt.")
            || key.starts_with("claude.")
            || key.starts_with("gemini.")
            || key.starts_with("openai.")
        {
            let operation = key
                .rsplit('.')
                .next()
                .map(Self::humanize_key)
                .unwrap_or_else(|| "Operation".to_owned());
            let operation_id = operation.to_ascii_lowercase().replace(' ', "-");
            return Self::markdown_block(
                format!("provider-{operation_id}-empty"),
                format!(
                    "### {}\nNo result returned.",
                    Self::provider_operation_label(key.as_str())
                ),
            );
        }
        if key.starts_with("tools.") || key.starts_with("plugins.") {
            let subject = if key.contains("plugins") {
                "plugins"
            } else if key.ends_with("tags") || key.ends_with("_tags") {
                "tool tags"
            } else {
                "tools"
            };
            return Self::markdown_block(
                "discovery-empty",
                format!(
                    "### {}\nNo matching {subject} found.",
                    Self::humanize_key(subject)
                ),
            );
        }

        let operation = key
            .rsplit('.')
            .next()
            .map(Self::humanize_key)
            .unwrap_or_else(|| "Tool result".to_owned());
        let id = if key.is_empty() {
            "tool-empty".to_owned()
        } else {
            format!("{}-empty", key.replace('.', "-"))
        };
        Self::markdown_block(id, format!("### {operation}\nNo result returned."))
    }

    fn provider_call_value(call: &serde_json::Map<String, Value>, keys: &[&str]) -> String {
        keys.iter()
            .find_map(|key| call.get(*key))
            .and_then(|value| match value {
                Value::String(value) => (!value.trim().is_empty()).then(|| value.trim().to_owned()),
                Value::Number(value) => Some(value.to_string()),
                Value::Bool(value) => Some(value.to_string()),
                Value::Object(object) => object
                    .get("name")
                    .or_else(|| object.get("type"))
                    .or_else(|| object.get("command"))
                    .or_else(|| object.get("query"))
                    .map(Self::generic_field_value),
                _ => None,
            })
            .unwrap_or_default()
    }

    fn provider_call_action(call: &serde_json::Map<String, Value>) -> String {
        if let Some(action) = call.get("action") {
            match action {
                Value::Object(action) => {
                    for key in ["type", "name", "action", "command", "query"] {
                        if let Some(value) = action.get(key).and_then(Value::as_str)
                            && !value.trim().is_empty()
                        {
                            return value.trim().to_owned();
                        }
                    }
                }
                Value::String(action) if !action.trim().is_empty() => {
                    return action.trim().to_owned();
                }
                _ => {}
            }
        }
        Self::provider_call_value(call, &["name", "tool", "type", "query", "command"])
    }

    fn provider_call_target(call: &serde_json::Map<String, Value>) -> String {
        for container_key in ["action", "operation", "input"] {
            let Some(value) = call.get(container_key) else {
                continue;
            };
            if let Some(object) = value.as_object() {
                for key in [
                    "path",
                    "command",
                    "commands",
                    "name",
                    "server_label",
                    "server",
                    "query",
                    "target",
                ] {
                    if let Some(value) = object.get(key) {
                        let rendered = Self::generic_field_value(value);
                        if !rendered.trim().is_empty() {
                            return rendered;
                        }
                    }
                }
            } else {
                let rendered = Self::generic_field_value(value);
                if !rendered.trim().is_empty() {
                    return rendered;
                }
            }
        }
        Self::provider_call_value(
            call,
            &[
                "path",
                "command",
                "name",
                "tool",
                "server_label",
                "server",
                "query",
            ],
        )
    }

    fn provider_call_operation_label(tool_name: &str) -> &'static str {
        match tool_name {
            "chatgpt.apply_patch" => "Patch operation",
            "chatgpt.function" | "gemini.function" => "Function call",
            "chatgpt.custom" => "Custom tool call",
            "chatgpt.namespace" => "Namespace call",
            "chatgpt.programmatic_tool_calling" => "Programmatic call",
            "chatgpt.local_shell" | "chatgpt.shell" | "claude.bash" => "Shell call",
            _ => "Tool call",
        }
    }

    fn provider_call_rows(calls: &[Value]) -> Vec<Vec<Value>> {
        calls
            .iter()
            .filter_map(Value::as_object)
            .map(|call| {
                vec![
                    Value::String(Self::provider_call_value(call, &["type"])),
                    Value::String(Self::provider_call_action(call)),
                    Value::String(Self::provider_call_value(
                        call,
                        &["id", "call_id", "callId"],
                    )),
                    Value::String(Self::provider_call_value(call, &["status", "state"])),
                    Value::String(Self::provider_call_value(
                        call,
                        &["server_label", "server", "server_name", "mcp_server_name"],
                    )),
                ]
            })
            .collect()
    }

    fn provider_call_operation_blocks(key: &str, calls: &[Value]) -> Vec<ViewBlock> {
        calls
            .iter()
            .enumerate()
            .filter_map(|(index, call)| {
                let call = call.as_object()?;
                let fields = [
                    ("Type", Self::provider_call_value(call, &["type"])),
                    ("Action", Self::provider_call_action(call)),
                    (
                        "ID",
                        Self::provider_call_value(call, &["id", "call_id", "callId"]),
                    ),
                    (
                        "Status",
                        Self::provider_call_value(call, &["status", "state"]),
                    ),
                    ("Target", Self::provider_call_target(call)),
                ];
                Self::details_block_if_nonempty(
                    format!("provider-call-operation-{index}"),
                    Self::provider_call_operation_label(key),
                    &fields,
                )
            })
            .collect()
    }

    fn provider_count_value(object: &serde_json::Map<String, Value>, keys: &[&str]) -> String {
        keys.iter()
            .find_map(|key| object.get(*key))
            .and_then(|value| {
                value
                    .as_array()
                    .map(|values| values.len().to_string())
                    .or_else(|| value.as_u64().map(|value| value.to_string()))
            })
            .unwrap_or_default()
    }

    fn provider_content_blocks(value: &Value) -> Vec<ViewBlock> {
        let mut text_parts = Vec::new();
        let mut structured = Vec::new();
        let mut collect = |value: &Value| match value {
            Value::String(text) if !text.trim().is_empty() => {
                text_parts.push(Self::bounded_human_text(text));
            }
            Value::Object(object) => {
                let text = ["text", "content", "markdown"]
                    .iter()
                    .find_map(|key| object.get(*key).and_then(Value::as_str))
                    .filter(|text| !text.trim().is_empty());
                if let Some(text) = text {
                    text_parts.push(Self::bounded_human_text(text));
                } else {
                    structured.push(value.clone());
                }
            }
            _ => structured.push(value.clone()),
        };

        match value {
            Value::Array(values) => values.iter().for_each(&mut collect),
            value => collect(value),
        }

        let mut blocks = Vec::new();
        if !text_parts.is_empty() {
            blocks.push(Self::markdown_block(
                "provider-content",
                format!("### Assistant response\n{}", text_parts.join("\n\n")),
            ));
        }
        if !structured.is_empty() {
            blocks.extend(Self::generic_value_blocks(
                "provider-content-data",
                "Other content",
                &Value::Array(structured),
                0,
            ));
        }
        if blocks.is_empty() {
            blocks.extend(Self::generic_value_blocks(
                "provider-content",
                "Assistant content",
                value,
                0,
            ));
        }
        blocks
    }

    fn provider_has_pending_calls(object: &serde_json::Map<String, Value>) -> bool {
        object
            .get("pending_calls")
            .and_then(Value::as_array)
            .is_some_and(|calls| !calls.is_empty())
            || object.get("continuation_required").and_then(Value::as_bool) == Some(true)
    }

    fn specific_provider_operation_blocks(
        key: &str,
        object: &serde_json::Map<String, Value>,
    ) -> Vec<ViewBlock> {
        let mut blocks = Vec::new();
        match key {
            "chatgpt.file_search" | "claude.file_search" | "gemini.file_search" => {
                let fields = [
                    ("Query", Self::object_text(object, "query")),
                    ("Store", Self::object_text(object, "vector_store_id")),
                    ("Stores", Self::object_text(object, "vector_store_ids")),
                    (
                        "Hits",
                        Self::provider_count_value(
                            object,
                            &[
                                "file_results",
                                "file_search_results",
                                "files",
                                "documents",
                                "matches",
                                "results",
                                "file_count",
                                "retrieved_count",
                            ],
                        ),
                    ),
                ];
                if let Some(block) =
                    Self::details_block_if_nonempty("provider-file-search", "File search", &fields)
                {
                    blocks.push(block);
                }
                for (child_key, title) in [
                    ("file_results", "File results"),
                    ("file_search_results", "File results"),
                    ("files", "Files"),
                    ("documents", "Documents"),
                    ("matches", "Matches"),
                    ("results", "Results"),
                ] {
                    if let Some(values) = Self::object_array(object, child_key) {
                        if let Some(table) = Self::scalar_table(
                            format!("provider-file-{child_key}"),
                            title,
                            values,
                            &[
                                ("name", "Name"),
                                ("file_name", "File"),
                                ("title", "Title"),
                                ("path", "Path"),
                                ("score", "Score"),
                                ("snippet", "Snippet"),
                            ],
                        ) {
                            blocks.push(table);
                        } else if values.is_empty() {
                            blocks.push(Self::markdown_block(
                                format!("provider-file-{child_key}"),
                                format!("### {title}\nNo file hits returned."),
                            ));
                        }
                        break;
                    }
                }
                if !blocks.iter().any(|block| {
                    matches!(
                        block.block_id(),
                        Some(
                            "provider-file-file_results"
                                | "provider-file-file_search_results"
                                | "provider-file-files"
                                | "provider-file-documents"
                                | "provider-file-matches"
                                | "provider-file-results"
                        )
                    )
                }) && !Self::provider_has_pending_calls(object)
                {
                    blocks.push(Self::markdown_block(
                        "provider-file-results",
                        "### File results\nNo file hits returned.",
                    ));
                }
            }
            "chatgpt.tool_search"
            | "claude.tool_search_bm25"
            | "claude.tool_search_regex"
            | "claude.tool_search_tool_bm25"
            | "claude.tool_search_tool_regex" => {
                let fields = [
                    ("Query", Self::object_text(object, "query")),
                    ("Total", Self::object_text(object, "total")),
                    (
                        "Matches",
                        Self::provider_count_value(
                            object,
                            &[
                                "tool_references",
                                "tools",
                                "matches",
                                "results",
                                "tool_count",
                                "result_count",
                            ],
                        ),
                    ),
                ];
                if let Some(block) =
                    Self::details_block_if_nonempty("provider-tool-search", "Tool search", &fields)
                {
                    blocks.push(block);
                }
                for child_key in ["tool_references", "tools", "matches", "results"] {
                    if let Some(values) = Self::object_array(object, child_key) {
                        if let Some(table) = Self::scalar_table(
                            "provider-tool-results",
                            "Matching tools",
                            values,
                            &[
                                ("name", "Name"),
                                ("title", "Title"),
                                ("server", "Server"),
                                ("description", "Description"),
                            ],
                        ) {
                            blocks.push(table);
                        } else if values.is_empty() {
                            blocks.push(Self::markdown_block(
                                "provider-tool-results",
                                "### Matching tools\nNo matching tools found.",
                            ));
                        }
                        break;
                    }
                }
                if blocks
                    .iter()
                    .all(|block| block.block_id() != Some("provider-tool-results"))
                    && !Self::provider_has_pending_calls(object)
                {
                    blocks.push(Self::markdown_block(
                        "provider-tool-results",
                        "### Matching tools\nNo matching tools found.",
                    ));
                }
            }
            "gemini.google_maps" => {
                let fields = [
                    ("Query", Self::object_text(object, "query")),
                    (
                        "Places",
                        Self::provider_count_value(
                            object,
                            &[
                                "places",
                                "map_results",
                                "locations",
                                "results",
                                "place_count",
                                "result_count",
                            ],
                        ),
                    ),
                ];
                if let Some(block) =
                    Self::details_block_if_nonempty("provider-maps", "Map search", &fields)
                {
                    blocks.push(block);
                }
                for child_key in ["places", "map_results", "locations", "results"] {
                    if let Some(values) = Self::object_array(object, child_key) {
                        if let Some(table) = Self::scalar_table(
                            "provider-places",
                            "Places",
                            values,
                            &[
                                ("name", "Name"),
                                ("address", "Address"),
                                ("rating", "Rating"),
                                ("url", "URL"),
                                ("distance", "Distance"),
                            ],
                        ) {
                            blocks.push(table);
                        } else if values.is_empty() {
                            blocks.push(Self::markdown_block(
                                "provider-places",
                                "### Places\nNo places found.",
                            ));
                        }
                        break;
                    }
                }
                if blocks
                    .iter()
                    .all(|block| block.block_id() != Some("provider-places"))
                    && !Self::provider_has_pending_calls(object)
                {
                    blocks.push(Self::markdown_block(
                        "provider-places",
                        "### Places\nNo places found.",
                    ));
                }
            }
            "gemini.retrieval" => {
                let fields = [
                    ("Query", Self::object_text(object, "query")),
                    (
                        "Retrieved",
                        Self::provider_count_value(
                            object,
                            &[
                                "retrieved",
                                "matches",
                                "chunks",
                                "documents",
                                "results",
                                "retrieved_count",
                                "match_count",
                                "chunk_count",
                            ],
                        ),
                    ),
                    ("Source", Self::object_text(object, "retrieval_type")),
                ];
                if let Some(block) = Self::details_block_if_nonempty(
                    "provider-retrieval",
                    "Context retrieval",
                    &fields,
                ) {
                    blocks.push(block);
                }
                if Self::provider_count_value(
                    object,
                    &[
                        "retrieved",
                        "matches",
                        "chunks",
                        "documents",
                        "results",
                        "retrieved_count",
                        "match_count",
                        "chunk_count",
                    ],
                )
                .is_empty()
                    && !Self::provider_has_pending_calls(object)
                {
                    blocks.push(Self::markdown_block(
                        "provider-retrieval-empty",
                        "### Retrieved context\nNo context matches returned.",
                    ));
                }
            }
            "gemini.url_context" | "claude.web_fetch" => {
                let fields = [
                    ("URL", Self::object_text(object, "url")),
                    (
                        "Fetched",
                        Self::provider_count_value(
                            object,
                            &[
                                "fetched_urls",
                                "loaded_urls",
                                "pages",
                                "documents",
                                "urls",
                                "fetched_count",
                                "loaded_count",
                                "page_count",
                            ],
                        ),
                    ),
                    ("HTTP status", Self::object_text(object, "status")),
                ];
                if let Some(block) = Self::details_block_if_nonempty(
                    "provider-web-fetch",
                    "Fetched context",
                    &fields,
                ) {
                    blocks.push(block);
                }
                if Self::provider_count_value(
                    object,
                    &[
                        "fetched_urls",
                        "loaded_urls",
                        "pages",
                        "documents",
                        "urls",
                        "fetched_count",
                        "loaded_count",
                        "page_count",
                    ],
                )
                .is_empty()
                    && !Self::provider_has_pending_calls(object)
                {
                    blocks.push(Self::markdown_block(
                        "provider-web-fetch-empty",
                        "### Fetched context\nNo URLs were fetched.",
                    ));
                }
            }
            "chatgpt.code_interpreter" | "claude.code_execution" | "gemini.code_execution" => {
                let fields = [
                    ("Status", Self::object_text(object, "status")),
                    ("Exit code", Self::object_text(object, "exit_code")),
                    ("Outcome", Self::object_text(object, "outcome")),
                    (
                        "Outputs",
                        Self::provider_count_value(object, &["outputs", "results", "output_count"]),
                    ),
                ];
                if let Some(block) =
                    Self::details_block_if_nonempty("provider-code", "Code execution", &fields)
                {
                    blocks.push(block);
                }
            }
            "chatgpt.computer"
            | "chatgpt.computer_use_preview"
            | "claude.computer"
            | "gemini.computer_use" => {
                let fields = [
                    ("Action", Self::object_text(object, "action")),
                    ("Page", Self::object_text(object, "page_title")),
                    ("URL", Self::object_text(object, "url")),
                    ("Actions", Self::provider_count_value(object, &["actions"])),
                ];
                if let Some(block) =
                    Self::details_block_if_nonempty("provider-computer", "Computer result", &fields)
                {
                    blocks.push(block);
                }
                if let Some(actions) = Self::object_array(object, "actions")
                    && let Some(table) = Self::scalar_table(
                        "provider-computer-actions",
                        "Computer actions",
                        actions,
                        &[
                            ("type", "Action"),
                            ("status", "Status"),
                            ("x", "X"),
                            ("y", "Y"),
                            ("text", "Text"),
                        ],
                    )
                {
                    blocks.push(table);
                }
                if !Self::provider_has_pending_calls(object)
                    && Self::object_array(object, "actions").is_none()
                    && Self::object_text(object, "action").is_empty()
                    && Self::object_text(object, "page_title").is_empty()
                {
                    blocks.push(Self::markdown_block(
                        "provider-computer-empty",
                        "### Computer result\nNo computer action returned.",
                    ));
                }
            }
            "chatgpt.local_shell" | "chatgpt.shell" | "claude.bash" => {
                let pending_command = Self::object_array(object, "pending_calls")
                    .and_then(|calls| calls.first())
                    .and_then(Value::as_object)
                    .map(Self::provider_call_target)
                    .unwrap_or_default();
                let command = {
                    let value = Self::object_text(object, "command");
                    if value.is_empty() {
                        pending_command
                    } else {
                        value
                    }
                };
                let fields = [
                    ("Command", command),
                    ("Status", Self::object_text(object, "status")),
                    ("Exit code", Self::object_text(object, "exit_code")),
                    ("Output", Self::object_text(object, "output")),
                    (
                        "Pending calls",
                        Self::provider_count_value(object, &["pending_calls"]),
                    ),
                ];
                if let Some(block) =
                    Self::details_block_if_nonempty("provider-shell", "Shell result", &fields)
                {
                    blocks.push(block);
                }
            }
            "chatgpt.mcp" | "gemini.mcp_server" | "claude.mcp_toolset" => {
                let fields = [
                    ("Server", Self::object_text(object, "server_label")),
                    ("URL", Self::object_text(object, "server_url")),
                    ("Status", Self::object_text(object, "status")),
                    ("Connected", Self::object_text(object, "connected")),
                    ("Tools", Self::object_text(object, "tool_count")),
                ];
                if let Some(block) =
                    Self::details_block_if_nonempty("provider-mcp", "MCP connection", &fields)
                {
                    blocks.push(block);
                }
            }
            "claude.memory" => {
                let fields = [
                    ("Operation", Self::object_text(object, "operation")),
                    ("Status", Self::object_text(object, "status")),
                    ("Loaded", Self::object_text(object, "loaded")),
                    ("Saved", Self::object_text(object, "saved")),
                    ("Removed", Self::object_text(object, "removed")),
                ];
                if let Some(block) =
                    Self::details_block_if_nonempty("provider-memory", "Memory result", &fields)
                {
                    blocks.push(block);
                }
            }
            "claude.text_editor" => {
                let fields = [
                    ("Operation", Self::object_text(object, "operation")),
                    ("Path", Self::object_text(object, "path")),
                    ("Changed", Self::object_text(object, "changed")),
                    ("Replacements", Self::object_text(object, "replacements")),
                ];
                if let Some(block) =
                    Self::details_block_if_nonempty("provider-editor", "Text edit", &fields)
                {
                    blocks.push(block);
                }
            }
            "claude.str_replace_based_edit_tool" => {
                let fields = [
                    ("Operation", Self::object_text(object, "operation")),
                    ("Path", Self::object_text(object, "path")),
                    ("Changed", Self::object_text(object, "changed")),
                    ("Replacements", Self::object_text(object, "replacements")),
                ];
                if let Some(block) =
                    Self::details_block_if_nonempty("provider-editor", "Text edit", &fields)
                {
                    blocks.push(block);
                }
            }
            "chatgpt.apply_patch" => {
                let fields = [
                    (
                        "Changed files",
                        Self::provider_count_value(object, &["changes", "files"]),
                    ),
                    ("Status", Self::object_text(object, "status")),
                ];
                if let Some(block) =
                    Self::details_block_if_nonempty("provider-patch", "Patch result", &fields)
                {
                    blocks.push(block);
                }
                for child_key in ["changes", "files"] {
                    if let Some(values) = Self::object_array(object, child_key) {
                        if let Some(table) = Self::scalar_table(
                            "provider-patch-changes",
                            "Changed files",
                            values,
                            &[("path", "Path"), ("kind", "Change"), ("status", "Status")],
                        ) {
                            blocks.push(table);
                        }
                        break;
                    }
                }
            }
            "claude.advisor" => {
                if let Some(error) = object.get("error") {
                    blocks.extend(Self::generic_value_blocks(
                        "provider-advisor-error",
                        "Advisor error",
                        error,
                        0,
                    ));
                } else {
                    blocks.push(Self::markdown_block(
                        "provider-advisor",
                        "### Advisor\nResponse received.",
                    ));
                }
            }
            "openai.image_generation"
            | "openai.image_edit"
            | "chatgpt.image_generation"
            | "chatgpt.image_edit"
            | "gemini.image_generation"
            | "gemini.image_edit" => {
                let fields = [
                    ("Path", Self::object_text(object, "path")),
                    ("MIME", Self::object_text(object, "mime")),
                    (
                        "Images",
                        Self::provider_count_value(object, &["images", "outputs", "image_count"]),
                    ),
                    ("Size", Self::object_text(object, "size_bytes")),
                    ("SHA-256", Self::object_text(object, "sha256")),
                    ("Prompt", Self::object_text(object, "revised_prompt")),
                ];
                if let Some(block) =
                    Self::details_block_if_nonempty("provider-image", "Image result", &fields)
                {
                    blocks.push(block);
                }
            }
            _ => {}
        }
        blocks
    }

    fn specific_provider_blocks(
        tool_name: &str,
        object: &serde_json::Map<String, Value>,
    ) -> Vec<ViewBlock> {
        let mut blocks = Vec::new();
        let operation_blocks = Self::specific_provider_operation_blocks(tool_name, object);
        let operation_has_status = operation_blocks.iter().any(|block| {
            block
                .text_value()
                .is_some_and(|text| text.contains("**Status**:"))
        });
        let status_is_failure = object
            .get("status")
            .or_else(|| object.get("state"))
            .and_then(Value::as_str)
            .is_some_and(|status| {
                matches!(
                    status.to_ascii_lowercase().as_str(),
                    "failed" | "error" | "cancelled" | "timed_out" | "timed out"
                )
            });
        let mut fields = Vec::new();
        for (key, label) in [
            ("operation", "Operation"),
            ("provider", "Provider"),
            ("tool", "Provider tool"),
            ("status", "Status"),
            ("model", "Model"),
            ("request_id", "Request"),
            ("response_id", "Response"),
            ("continuation_required", "Continuation required"),
        ] {
            if key == "status" && operation_has_status && !status_is_failure {
                continue;
            }
            let value = Self::object_text(object, key);
            if !value.is_empty() {
                fields.push((label, value));
            }
        }
        fields.insert(
            0,
            ("What", Self::provider_operation_label(tool_name).to_owned()),
        );
        if !fields.is_empty() {
            blocks.push(Self::details_block(
                "provider-meta",
                "Provider response",
                &fields,
            ));
        }
        blocks.extend(operation_blocks);
        if let Some(calls) = Self::object_array(object, "pending_calls") {
            if !calls.is_empty() && !Self::provider_call_rows(calls).is_empty() {
                // Keep the typed columns stable even when a provider puts
                // `action` or the server identity in a nested object. The
                // generic nested projection below still preserves the full
                // bounded call details.
                let rows = Self::provider_call_rows(calls);
                blocks.push(ViewBlock::Table {
                    id: Some("provider-calls".into()),
                    columns: vec![
                        "Type".into(),
                        "Action".into(),
                        "ID".into(),
                        "Status".into(),
                        "Server".into(),
                    ],
                    rows,
                });
                blocks.extend(Self::provider_call_operation_blocks(tool_name, calls));
            } else if object.get("continuation_required").and_then(Value::as_bool) == Some(true) {
                blocks.push(Self::markdown_block(
                    "provider-calls",
                    "### Pending calls\nNo pending calls could be decoded.",
                ));
            }
        }
        if let Some(sources) = Self::object_array(object, "sources") {
            let source_title = match tool_name {
                "gemini.google_maps" => "Places",
                "gemini.retrieval" => "Retrieved sources",
                "gemini.url_context" | "claude.web_fetch" => "Fetched sources",
                "chatgpt.file_search" | "claude.file_search" | "gemini.file_search" => {
                    "File citations"
                }
                _ => "Sources",
            };
            if !sources.is_empty()
                && let Some(table) = Self::scalar_table(
                    "provider-sources",
                    source_title,
                    sources,
                    &[
                        ("title", "Title"),
                        ("url", "URL"),
                        ("domain", "Domain"),
                        ("snippet", "Snippet"),
                    ],
                )
            {
                blocks.push(table);
            } else if tool_name.contains("web_search")
                || tool_name.contains("web_fetch")
                || tool_name.ends_with(".google_search")
            {
                blocks.push(Self::markdown_block(
                    "provider-sources",
                    "### Sources\nNo sources returned.",
                ));
            }
        }
        if let Some(usage) = object.get("usage") {
            blocks.extend(Self::generic_value_blocks(
                "provider-usage",
                "Usage",
                usage,
                0,
            ));
        }
        if let Some(receipt) = object.get("response_receipt") {
            blocks.extend(Self::generic_value_blocks(
                "provider-receipt",
                "Response receipt",
                receipt,
                0,
            ));
        }
        if let Some(content) = object.get("assistant_content") {
            blocks.extend(Self::provider_content_blocks(content));
        }
        if let Some(error) = object.get("error") {
            blocks.extend(Self::generic_value_blocks(
                "provider-error",
                "Provider error",
                error,
                0,
            ));
        }
        blocks
    }

    fn string_list_block(
        id: impl Into<String>,
        title: &str,
        values: Option<&Vec<Value>>,
        empty: &str,
    ) -> ViewBlock {
        let values = values
            .map(|values| {
                values
                    .iter()
                    .map(Self::generic_field_value)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if values.is_empty() {
            return Self::markdown_block(id, format!("### {title}\n{empty}"));
        }
        let mut lines = vec![format!("### {title}")];
        lines.extend(values.into_iter().map(|value| format!("- `{value}`")));
        Self::markdown_block(id, lines.join("\n"))
    }

    fn specific_lsp_blocks(key: &str, object: &serde_json::Map<String, Value>) -> Vec<ViewBlock> {
        let mut blocks = Vec::new();
        match key {
            "lsp.servers" => {
                if let Some(servers) = Self::object_array(object, "servers")
                    && let Some(table) = Self::scalar_table(
                        "lsp-servers",
                        "Configured language servers",
                        servers,
                        &[
                            ("name", "Server"),
                            ("command", "Command"),
                            ("args", "Arguments"),
                            ("file_extensions", "File extensions"),
                        ],
                    )
                {
                    blocks.push(table);
                } else {
                    blocks.push(Self::markdown_block(
                        "lsp-servers",
                        "### Configured language servers\nNo language servers configured.",
                    ));
                }
            }
            "lsp.definition" | "lsp.references" => {
                let title = if key == "lsp.definition" {
                    "Definitions"
                } else {
                    "References"
                };
                let empty = if key == "lsp.definition" {
                    "No definitions found."
                } else {
                    "No references found."
                };
                blocks.push(Self::string_list_block(
                    if key == "lsp.definition" {
                        "lsp-definitions"
                    } else {
                        "lsp-references"
                    },
                    title,
                    object.get("locations").and_then(Value::as_array),
                    empty,
                ));
            }
            "lsp.hover" => {
                if let Some(contents) = object.get("contents") {
                    if let Some(text) = contents.as_str().filter(|text| !text.trim().is_empty()) {
                        blocks.push(Self::markdown_block(
                            "lsp-hover",
                            format!(
                                "### Hover information\n{}",
                                Self::bounded_generic_text(text)
                            ),
                        ));
                    } else if !contents.is_null() {
                        blocks.extend(Self::generic_value_blocks(
                            "lsp-hover",
                            "Hover information",
                            contents,
                            0,
                        ));
                    }
                }
                if blocks.is_empty() {
                    blocks.push(Self::markdown_block(
                        "lsp-hover",
                        "### Hover information\nNo hover information.",
                    ));
                }
            }
            "lsp.diagnostics" => {
                if let Some(entries) = Self::object_array(object, "entries") {
                    if let Some(table) = Self::table_block(
                        "lsp-diagnostics",
                        vec!["Location", "Severity", "Message"],
                        Self::diagnostic_rows(entries),
                    ) {
                        blocks.push(table);
                    } else {
                        blocks.push(Self::markdown_block(
                            "lsp-diagnostics",
                            "### Diagnostics\nNo diagnostics.",
                        ));
                    }
                } else {
                    blocks.push(Self::markdown_block(
                        "lsp-diagnostics",
                        "### Diagnostics\nNo diagnostics.",
                    ));
                }
            }
            _ => {}
        }
        blocks
    }

    fn specific_interaction_blocks(
        key: &str,
        object: &serde_json::Map<String, Value>,
    ) -> Vec<ViewBlock> {
        let mut blocks = Vec::new();
        match key {
            "interaction.ask" | "ask_user" => {
                let answers = object.get("answers").and_then(Value::as_object);
                if let Some(questions) = Self::object_array(object, "questions") {
                    let rows = questions
                        .iter()
                        .enumerate()
                        .filter_map(|(index, question)| {
                            let question = question.as_object()?;
                            let prompt = question
                                .get("question")
                                .or_else(|| question.get("text"))
                                .map(Self::generic_field_value)
                                .unwrap_or_else(|| format!("Question {}", index + 1));
                            let answer = answers
                                .and_then(|answers| answers.get(index.to_string().as_str()))
                                .map(Self::generic_field_value)
                                .unwrap_or_else(|| "—".to_owned());
                            Some(vec![Value::String(prompt), Value::String(answer)])
                        })
                        .collect::<Vec<_>>();
                    if let Some(table) =
                        Self::table_block("interaction-answers", vec!["Question", "Answer"], rows)
                    {
                        blocks.push(table);
                    }
                } else if let Some(answers) = answers {
                    let rows = answers
                        .iter()
                        .map(|(question, answer)| {
                            vec![
                                Value::String(question.clone()),
                                Value::String(Self::generic_field_value(answer)),
                            ]
                        })
                        .collect::<Vec<_>>();
                    if let Some(table) =
                        Self::table_block("interaction-answers", vec!["Question", "Answer"], rows)
                    {
                        blocks.push(table);
                    }
                }
                if let Some(block) = Self::details_block_if_nonempty(
                    "interaction-status",
                    "User input",
                    &[
                        ("Timed out", Self::object_text(object, "timed_out")),
                        ("Auto resolved", Self::object_text(object, "auto_resolved")),
                    ],
                ) {
                    blocks.push(block);
                }
                if blocks.is_empty() {
                    blocks.push(Self::markdown_block(
                        "interaction-answers",
                        "### Answers\nNo answers were recorded.",
                    ));
                }
            }
            "interaction.notify" => {
                if let Some(block) = Self::details_block_if_nonempty(
                    "notification-meta",
                    "Notification",
                    &[
                        ("Title", Self::object_text(object, "title")),
                        ("Level", Self::object_text(object, "level")),
                    ],
                ) {
                    blocks.push(block);
                }
                if let Some(body) = Self::object_string(object, "body_markdown") {
                    blocks.push(Self::markdown_block("notification-body", body));
                } else if blocks.is_empty() {
                    blocks.push(Self::markdown_block(
                        "notification-body",
                        "### Notification\nNo notification body.",
                    ));
                }
            }
            _ => {}
        }
        blocks
    }

    fn specific_web_blocks(key: &str, object: &serde_json::Map<String, Value>) -> Vec<ViewBlock> {
        let mut blocks = Vec::new();
        match key {
            "web.search" => {
                if let Some(block) = Self::details_block_if_nonempty(
                    "web-search-meta",
                    "Web search",
                    &[
                        ("Query", Self::object_text(object, "query")),
                        ("Engine", Self::object_text(object, "engine")),
                        ("Tried", Self::object_text(object, "attempted_engines")),
                    ],
                ) {
                    blocks.push(block);
                }
                if let Some(results) = Self::object_array(object, "results") {
                    if results.is_empty() {
                        blocks.push(Self::markdown_block(
                            "web-search-results",
                            "### Search results\nNo web search results.",
                        ));
                    } else if let Some(table) = Self::scalar_table(
                        "web-search-results",
                        "Search results",
                        results,
                        &[
                            ("title", "Title"),
                            ("url", "URL"),
                            ("description", "Description"),
                            ("source", "Source"),
                            ("engine", "Engine"),
                        ],
                    ) {
                        blocks.push(table);
                    }
                }
            }
            "web.fetch" => {
                if let Some(block) = Self::details_block_if_nonempty(
                    "web-fetch-meta",
                    "Fetched page",
                    &[
                        ("Title", Self::object_text(object, "title")),
                        ("URL", Self::object_text(object, "url")),
                        ("Canonical URL", Self::object_text(object, "canonical_url")),
                        ("HTTP status", Self::object_text(object, "status")),
                        ("Content type", Self::object_text(object, "content_type")),
                        ("Rendered", Self::object_text(object, "rendered")),
                        ("Truncated", Self::object_text(object, "truncated")),
                    ],
                ) {
                    blocks.push(block);
                }
                if let Some(markdown) = Self::object_string(object, "markdown") {
                    blocks.push(Self::markdown_block(
                        "web-fetch-content",
                        Self::bounded_generic_text(&markdown),
                    ));
                }
                if let Some(links) = object.get("links").and_then(Value::as_array) {
                    blocks.push(Self::string_list_block(
                        "web-fetch-links",
                        "Links",
                        Some(links),
                        "No links found.",
                    ));
                }
            }
            "web.crawl" => {
                if let Some(block) = Self::details_block_if_nonempty(
                    "web-crawl-summary",
                    "Crawl run",
                    &[
                        ("Start URL", Self::object_text(object, "start_url")),
                        ("Engine", Self::object_text(object, "engine")),
                        ("Rendered", Self::object_text(object, "rendered")),
                        ("Indexed", Self::object_text(object, "stored_count")),
                        ("Cached", Self::object_text(object, "cached_count")),
                        (
                            "Exact duplicates",
                            Self::object_text(object, "duplicate_count"),
                        ),
                        (
                            "Near duplicates",
                            Self::object_text(object, "near_duplicate_count"),
                        ),
                        ("Failures", Self::object_text(object, "failure_count")),
                        (
                            "Total cached documents",
                            Self::object_text(object, "total_documents"),
                        ),
                    ],
                ) {
                    blocks.push(block);
                }
                if let Some(documents) = Self::object_array(object, "documents") {
                    if let Some(table) = Self::scalar_table(
                        "web-crawl-documents",
                        "Indexed pages",
                        documents,
                        &[
                            ("title", "Title"),
                            ("url", "URL"),
                            ("depth", "Depth"),
                            ("chunk_count", "Chunks"),
                            ("fetched_at", "Fetched"),
                        ],
                    ) {
                        blocks.push(table);
                    } else if documents.is_empty() {
                        blocks.push(Self::markdown_block(
                            "web-crawl-documents",
                            "### Indexed pages\nNo pages indexed.",
                        ));
                    }
                }
                if let Some(failures) = Self::object_array(object, "failures")
                    && !failures.is_empty()
                {
                    blocks.push(Self::markdown_block(
                        "web-crawl-failures",
                        format!(
                            "### Crawl failures\n{}",
                            failures
                                .iter()
                                .map(Self::readable_problem)
                                .map(|value| format!("- {value}"))
                                .collect::<Vec<_>>()
                                .join("\n")
                        ),
                    ));
                }
            }
            _ => {}
        }
        blocks
    }

    fn parse_discovery_tool_line(line: &str) -> Option<Vec<Value>> {
        let mut rest = line.trim().strip_prefix("- ")?.trim();
        let name_end = rest.find(char::is_whitespace)?;
        let name = rest[..name_end].to_owned();
        rest = rest[name_end..].trim_start();
        let tags_end = rest.strip_prefix('[')?.find(']')?;
        let tags = rest[1..=tags_end].to_owned();
        rest = rest[tags_end + 2..].trim_start();
        let mut plugin = String::new();
        if rest.starts_with('(')
            && let Some(end) = rest.find(')')
        {
            plugin = rest[1..end].to_owned();
            rest = rest[end + 1..].trim_start();
        }
        let summary = rest
            .strip_prefix(':')
            .map(str::trim)
            .unwrap_or_default()
            .to_owned();
        Some(vec![
            Value::String(name),
            Value::String(tags),
            Value::String(plugin),
            Value::String(summary),
        ])
    }

    fn parse_discovery_plugin_line(line: &str) -> Option<Vec<Value>> {
        let mut rest = line.trim().strip_prefix("- ")?.trim();
        let name_end = rest.find(char::is_whitespace)?;
        let name = rest[..name_end].to_owned();
        rest = rest[name_end..].trim_start();
        let tags_end = rest.strip_prefix('[')?.find(']')?;
        let tags = rest[1..=tags_end].to_owned();
        rest = rest[tags_end + 2..].trim_start();
        let mut version = String::new();
        if rest.starts_with('(')
            && let Some(end) = rest.find(')')
        {
            version = rest[1..end]
                .strip_prefix('v')
                .unwrap_or(&rest[1..end])
                .to_owned();
            rest = rest[end + 1..].trim_start();
        }
        let (summary, tools) = if let Some(summary) = rest.strip_prefix(':') {
            let summary = summary.trim();
            if let Some((summary, tools)) = summary.split_once(" · tools:") {
                (summary.trim().to_owned(), tools.trim().to_owned())
            } else {
                (summary.to_owned(), String::new())
            }
        } else {
            (String::new(), String::new())
        };
        Some(vec![
            Value::String(name),
            Value::String(version),
            Value::String(tags),
            Value::String(tools),
            Value::String(summary),
        ])
    }

    fn specific_discovery_text_blocks(tool_name: &str, text: &str) -> Vec<ViewBlock> {
        let text = text.trim();
        if text.is_empty() {
            return Vec::new();
        }
        let key = Self::normalized_tool_name(tool_name);
        let is_tools_api = key.starts_with("tools.")
            || key.starts_with("tools_")
            || key.starts_with("plugins.")
            || key.starts_with("plugins_");
        if !is_tools_api {
            return Vec::new();
        }
        if key == "tools.help" || key == "tools_help" {
            return vec![Self::markdown_block("discovery-help", text.to_owned())];
        }
        let is_plugins = key.contains("plugins");
        let is_tags = key.ends_with("tags") || key.ends_with("_tags");
        let mut rows = Vec::new();
        let mut unparsed = Vec::new();
        for line in text.lines() {
            let line = line.trim();
            if !line.starts_with("- ") {
                continue;
            }
            let row = if is_tags {
                line.strip_prefix("- ")
                    .and_then(|value| value.split_once(':'))
                    .map(|(tag, count)| {
                        vec![
                            Value::String(tag.trim().to_owned()),
                            Value::String(count.trim().to_owned()),
                        ]
                    })
            } else if is_plugins {
                Self::parse_discovery_plugin_line(line)
            } else {
                Self::parse_discovery_tool_line(line)
            };
            if let Some(row) = row {
                rows.push(row);
            } else {
                unparsed.push(line.to_owned());
            }
        }

        let mut blocks = Vec::new();
        if let Some(first_line) = text.lines().map(str::trim).find(|line| !line.is_empty()) {
            blocks.push(Self::markdown_block(
                "discovery-page",
                format!("### Discovery page\n{first_line}"),
            ));
        }
        let columns = if is_tags {
            vec!["Tag", "Count"]
        } else if is_plugins {
            vec!["Plugin", "Version", "Tags", "Tools", "Summary"]
        } else {
            vec!["Tool", "Tags", "Plugin", "Summary"]
        };
        let has_rows = !rows.is_empty();
        if let Some(table) = Self::table_block(
            if is_tags {
                "discovery-tags"
            } else if is_plugins {
                "discovery-plugins"
            } else {
                "discovery-tools"
            },
            columns,
            rows,
        ) {
            blocks.push(table);
        } else if !has_rows {
            let empty_label = if is_tags {
                "tags"
            } else if is_plugins {
                "plugins"
            } else {
                "tools"
            };
            blocks.push(Self::markdown_block(
                "discovery-empty",
                format!(
                    "### {}\nNo matching {empty_label} found.",
                    Self::humanize_key(empty_label)
                ),
            ));
        }
        if !unparsed.is_empty() {
            blocks.push(Self::markdown_block(
                "discovery-source",
                format!("### Additional output\n{}", unparsed.join("\n")),
            ));
        }
        if blocks.len() == 1
            && blocks
                .first()
                .and_then(ViewBlock::text_value)
                .is_some_and(|value| {
                    value
                        == format!(
                            "### Discovery page\n{}",
                            text.lines().next().unwrap_or_default()
                        )
                })
        {
            blocks.clear();
            blocks.push(Self::markdown_block("discovery", text.to_owned()));
        }
        blocks
    }

    fn specific_mcp_blocks(key: &str, object: &serde_json::Map<String, Value>) -> Vec<ViewBlock> {
        let mut blocks = Vec::new();
        let identity = [
            ("Server", Self::object_text(object, "server")),
            ("Tool", Self::object_text(object, "tool")),
            ("URI", Self::object_text(object, "uri")),
            ("Prompt", Self::object_text(object, "prompt")),
            ("Next cursor", Self::object_text(object, "next_cursor")),
        ];
        if let Some(block) = Self::details_block_if_nonempty("mcp-meta", "MCP result", &identity) {
            blocks.push(block);
        }
        if let Some(meta) = object.get("mcp_meta") {
            blocks.extend(Self::generic_value_blocks(
                "mcp-wire-meta",
                "MCP metadata",
                meta,
                0,
            ));
        }

        match key {
            "mcp.resources.list" => {
                if let Some(resources) = Self::object_array(object, "resources")
                    && let Some(table) = Self::scalar_table(
                        "mcp-resources",
                        "Resources",
                        resources,
                        &[
                            ("name", "Name"),
                            ("uri", "URI"),
                            ("mime_type", "MIME"),
                            ("description", "Description"),
                        ],
                    )
                {
                    blocks.push(table);
                } else if object
                    .get("resources")
                    .and_then(Value::as_array)
                    .is_some_and(Vec::is_empty)
                {
                    blocks.push(Self::markdown_block(
                        "mcp-resources",
                        "### Resources\nNo MCP resources found.",
                    ));
                }
            }
            "mcp.resources.templates.list" => {
                if let Some(templates) = Self::object_array(object, "resource_templates")
                    && let Some(table) = Self::scalar_table(
                        "mcp-resource-templates",
                        "Resource templates",
                        templates,
                        &[
                            ("name", "Name"),
                            ("uri_template", "URI template"),
                            ("mime_type", "MIME"),
                            ("description", "Description"),
                        ],
                    )
                {
                    blocks.push(table);
                } else if object
                    .get("resource_templates")
                    .and_then(Value::as_array)
                    .is_some_and(Vec::is_empty)
                {
                    blocks.push(Self::markdown_block(
                        "mcp-resource-templates",
                        "### Resource templates\nNo MCP resource templates found.",
                    ));
                }
            }
            "mcp.resources.read" => {
                if let Some(contents) = Self::object_array(object, "contents")
                    && let Some(table) = Self::table_block(
                        "mcp-resource-contents",
                        vec!["URI", "MIME", "Text"],
                        contents
                            .iter()
                            .filter_map(Value::as_object)
                            .map(|content| {
                                vec![
                                    Value::String(Self::object_text(content, "uri")),
                                    Value::String(Self::object_text(content, "mime_type")),
                                    Value::String(
                                        content
                                            .get("text")
                                            .or_else(|| content.get("blob"))
                                            .map(Self::mcp_content_text)
                                            .unwrap_or_default(),
                                    ),
                                ]
                            })
                            .collect(),
                    )
                {
                    blocks.push(table);
                } else if let Some(contents) = object.get("contents").and_then(Value::as_array)
                    && !contents.is_empty()
                {
                    blocks.extend(Self::generic_value_blocks(
                        "mcp-resource-contents",
                        "Resource contents",
                        &Value::Array(contents.clone()),
                        0,
                    ));
                } else if object
                    .get("contents")
                    .and_then(Value::as_array)
                    .is_some_and(Vec::is_empty)
                {
                    blocks.push(Self::markdown_block(
                        "mcp-resource-contents",
                        "### Resource contents\nNo content returned.",
                    ));
                }
            }
            "mcp.prompts.list" => {
                if let Some(prompts) = Self::object_array(object, "prompts")
                    && let Some(table) = Self::scalar_table(
                        "mcp-prompts",
                        "Prompt templates",
                        prompts,
                        &[
                            ("name", "Name"),
                            ("description", "Description"),
                            ("arguments", "Arguments"),
                        ],
                    )
                {
                    blocks.push(table);
                } else if object
                    .get("prompts")
                    .and_then(Value::as_array)
                    .is_some_and(Vec::is_empty)
                {
                    blocks.push(Self::markdown_block(
                        "mcp-prompts",
                        "### Prompt templates\nNo MCP prompts found.",
                    ));
                }
            }
            "mcp.prompts.get" => {
                if let Some(messages) = Self::object_array(object, "messages")
                    && let Some(table) = Self::table_block(
                        "mcp-prompt-messages",
                        vec!["Role", "Content"],
                        Self::mcp_prompt_message_rows(messages),
                    )
                {
                    blocks.push(table);
                } else if let Some(messages) = object.get("messages").and_then(Value::as_array)
                    && !messages.is_empty()
                {
                    blocks.extend(Self::generic_value_blocks(
                        "mcp-prompt-messages",
                        "Prompt messages",
                        &Value::Array(messages.clone()),
                        0,
                    ));
                } else if object
                    .get("messages")
                    .and_then(Value::as_array)
                    .is_some_and(Vec::is_empty)
                {
                    blocks.push(Self::markdown_block(
                        "mcp-prompt-messages",
                        "### Prompt messages\nNo prompt messages returned.",
                    ));
                }
            }
            "mcp.tools.call" => {
                if let Some(content) = Self::object_array(object, "content")
                    && let Some(table) = Self::table_block(
                        "mcp-content",
                        vec!["Type", "Text", "MIME", "URI"],
                        Self::mcp_content_rows(content),
                    )
                {
                    blocks.push(table);
                } else if let Some(content) = object.get("content").and_then(Value::as_array)
                    && !content.is_empty()
                {
                    blocks.extend(Self::generic_value_blocks(
                        "mcp-content",
                        "Content blocks",
                        &Value::Array(content.clone()),
                        0,
                    ));
                } else if object
                    .get("content")
                    .and_then(Value::as_array)
                    .is_some_and(Vec::is_empty)
                    && object.get("structured_content").is_none_or(Value::is_null)
                {
                    blocks.push(Self::markdown_block(
                        "mcp-content",
                        "### Content blocks\nNo content returned.",
                    ));
                }
                if let Some(structured) = object.get("structured_content") {
                    blocks.extend(Self::generic_value_blocks(
                        "mcp-structured-content",
                        "Structured content",
                        structured,
                        0,
                    ));
                }
            }
            "mcp.tools.search" => {
                if let Some(block) = Self::details_block_if_nonempty(
                    "mcp-search-meta",
                    "MCP tool search",
                    &[
                        ("Query", Self::object_text(object, "query")),
                        ("Total", Self::object_text(object, "total")),
                        (
                            "Index fingerprint",
                            Self::object_text(object, "index_fingerprint"),
                        ),
                    ],
                ) {
                    blocks.push(block);
                }
                if let Some(results) = Self::object_array(object, "results")
                    && let Some(table) = Self::scalar_table(
                        "mcp-tools",
                        "Matching MCP tools",
                        results,
                        &[
                            ("server", "Server"),
                            ("name", "Name"),
                            ("title", "Title"),
                            ("description", "Description"),
                            ("risk_hint", "Risk"),
                        ],
                    )
                {
                    blocks.push(table);
                } else if let Some(results) = object.get("results").and_then(Value::as_array)
                    && !results.is_empty()
                {
                    blocks.extend(Self::generic_value_blocks(
                        "mcp-tools",
                        "Matching MCP tools",
                        &Value::Array(results.clone()),
                        0,
                    ));
                } else if object
                    .get("results")
                    .and_then(Value::as_array)
                    .is_some_and(Vec::is_empty)
                {
                    blocks.push(Self::markdown_block(
                        "mcp-tools",
                        "### Matching MCP tools\nNo matching MCP tools found.",
                    ));
                }
            }
            "mcp.servers.status" => {
                if let Some(block) = Self::details_block_if_nonempty(
                    "mcp-status-meta",
                    "MCP server check",
                    &[("Checked at", Self::object_text(object, "checked_at"))],
                ) {
                    blocks.push(block);
                }
                if let Some(servers) = Self::object_array(object, "servers")
                    && let Some(table) = Self::scalar_table(
                        "mcp-servers",
                        "MCP servers",
                        servers,
                        &[
                            ("name", "Server"),
                            ("status", "Status"),
                            ("connected", "Connected"),
                            ("tool_count", "Tools"),
                            ("resource_count", "Resources"),
                            ("prompt_count", "Prompts"),
                            ("auth_mode", "Auth"),
                            ("network_target", "Network"),
                        ],
                    )
                {
                    blocks.push(table);
                    blocks.extend(Self::generic_array_nested_blocks(
                        "mcp-server-details",
                        servers,
                        0,
                    ));
                } else if object
                    .get("servers")
                    .and_then(Value::as_array)
                    .is_some_and(Vec::is_empty)
                {
                    blocks.push(Self::markdown_block(
                        "mcp-servers",
                        "### MCP servers\nNo MCP servers configured.",
                    ));
                }
            }
            "mcp.servers.reconnect" => {
                let fields = [
                    ("Connected", Self::object_text(object, "connected")),
                    ("Reconnected", Self::object_text(object, "reconnected")),
                    ("Status", Self::object_text(object, "status")),
                    ("Tools", Self::object_text(object, "tool_count")),
                    ("Attempt", Self::object_text(object, "attempt")),
                    ("Message", Self::object_text(object, "message")),
                ];
                if let Some(block) =
                    Self::details_block_if_nonempty("mcp-reconnect", "Reconnect", &fields)
                {
                    blocks.push(block);
                } else {
                    blocks.push(Self::markdown_block(
                        "mcp-reconnect",
                        "### Reconnect\nNo reconnect details returned.",
                    ));
                }
            }
            _ => {}
        }
        blocks
    }

    fn specific_memory_blocks(
        key: &str,
        object: &serde_json::Map<String, Value>,
    ) -> Vec<ViewBlock> {
        let mut blocks = Vec::new();
        match key {
            "memory.search" => {
                let fields = [
                    ("Query", Self::object_text(object, "query")),
                    ("Limit", Self::object_text(object, "limit")),
                ];
                if let Some(block) =
                    Self::details_block_if_nonempty("memory-search-meta", "Memory search", &fields)
                {
                    blocks.push(block);
                }
                if let Some(results) = Self::object_array(object, "results")
                    && let Some(table) = Self::scalar_table(
                        "memory-results",
                        "Matching memories",
                        results,
                        &[
                            ("id", "ID"),
                            ("name", "Name"),
                            ("memory_type", "Type"),
                            ("score", "Score"),
                            ("description", "Description"),
                            ("snippet", "Snippet"),
                            ("path", "Path"),
                        ],
                    )
                {
                    blocks.push(table);
                } else if let Some(results) = object.get("results").and_then(Value::as_array)
                    && !results.is_empty()
                {
                    blocks.extend(Self::generic_value_blocks(
                        "memory-results",
                        "Matching memories",
                        &Value::Array(results.clone()),
                        0,
                    ));
                } else if object
                    .get("results")
                    .and_then(Value::as_array)
                    .is_some_and(Vec::is_empty)
                {
                    blocks.push(Self::markdown_block(
                        "memory-results",
                        "### Matching memories\nNo matching memories found.",
                    ));
                }
            }
            "memory.list" => {
                let fields = [("Limit", Self::object_text(object, "limit"))];
                if let Some(block) =
                    Self::details_block_if_nonempty("memory-list-meta", "Memory catalog", &fields)
                {
                    blocks.push(block);
                }
                if let Some(memories) = Self::object_array(object, "memories")
                    && let Some(table) = Self::scalar_table(
                        "memories",
                        "Memory records",
                        memories,
                        &[
                            ("id", "ID"),
                            ("name", "Name"),
                            ("memory_type", "Type"),
                            ("description", "Description"),
                            ("file_name", "File"),
                            ("size", "Size"),
                            ("path", "Path"),
                            ("content_hash", "Hash"),
                        ],
                    )
                {
                    blocks.push(table);
                } else if let Some(memories) = object.get("memories").and_then(Value::as_array)
                    && !memories.is_empty()
                {
                    blocks.extend(Self::generic_value_blocks(
                        "memories",
                        "Memory records",
                        &Value::Array(memories.clone()),
                        0,
                    ));
                } else if object
                    .get("memories")
                    .and_then(Value::as_array)
                    .is_some_and(Vec::is_empty)
                {
                    blocks.push(Self::markdown_block(
                        "memories",
                        "### Memory records\nNo memory records found.",
                    ));
                }
            }
            "memory.get" => {
                let fields = [
                    ("Name", Self::object_text(object, "name")),
                    ("Type", Self::object_text(object, "memory_type")),
                    ("Description", Self::object_text(object, "description")),
                    ("File", Self::object_text(object, "file_name")),
                    ("Path", Self::object_text(object, "path")),
                    ("Hash", Self::object_text(object, "content_hash")),
                ];
                if let Some(block) =
                    Self::details_block_if_nonempty("memory-meta", "Memory", &fields)
                {
                    blocks.push(block);
                }
                if let Some(body) = Self::object_string(object, "body") {
                    blocks.push(Self::markdown_block(
                        "memory-body",
                        format!("### Memory content\n{}", Self::bounded_human_text(&body)),
                    ));
                }
            }
            "memory.write" => {
                let fields = [
                    ("Name", Self::object_text(object, "name")),
                    ("Type", Self::object_text(object, "memory_type")),
                    ("Description", Self::object_text(object, "description")),
                    ("File", Self::object_text(object, "file_name")),
                    ("Path", Self::object_text(object, "path")),
                    ("Hash", Self::object_text(object, "content_hash")),
                    ("Saved", Self::object_text(object, "saved")),
                    ("Bytes", Self::object_text(object, "bytes")),
                ];
                if let Some(block) =
                    Self::details_block_if_nonempty("memory-write", "Memory saved", &fields)
                {
                    blocks.push(block);
                } else {
                    blocks.push(Self::markdown_block(
                        "memory-write",
                        "### Memory saved\nNo save details returned.",
                    ));
                }
                if let Some(body) = Self::object_string(object, "body") {
                    blocks.push(Self::markdown_block(
                        "memory-body",
                        format!("### Memory content\n{}", Self::bounded_human_text(&body)),
                    ));
                }
            }
            "memory.delete" => {
                let fields = [
                    ("Name", Self::object_text(object, "name")),
                    (
                        "Removed",
                        if object.get("removed").is_some() {
                            Self::object_text(object, "removed")
                        } else {
                            Self::object_text(object, "deleted")
                        },
                    ),
                ];
                if let Some(block) =
                    Self::details_block_if_nonempty("memory-delete", "Memory deletion", &fields)
                {
                    blocks.push(block);
                } else {
                    blocks.push(Self::markdown_block(
                        "memory-delete",
                        "### Memory deletion\nNo deletion details returned.",
                    ));
                }
            }
            _ => {}
        }
        blocks
    }

    fn specific_plan_blocks(key: &str, object: &serde_json::Map<String, Value>) -> Vec<ViewBlock> {
        let mut blocks = Vec::new();
        if object.get("cleared").and_then(Value::as_bool).is_some() {
            let fields = [("Cleared", Self::object_text(object, "cleared"))];
            blocks.push(Self::details_block("plan-clear", "Plan", &fields));
        }
        let plan = object.get("plan").and_then(Value::as_object);
        if let Some(plan) = plan {
            let fields = [
                ("Title", Self::object_text(plan, "title")),
                ("Objective", Self::object_text(plan, "objective")),
                ("Phase", Self::object_text(plan, "phase")),
                ("Autorun", Self::object_text(plan, "autorun")),
                ("View", Self::object_text(object, "view")),
                ("Decision", Self::object_text(object, "decision")),
            ];
            if let Some(block) = Self::details_block_if_nonempty("plan-meta", "Plan", &fields) {
                blocks.push(block);
            }
            if let Some(steps) = plan.get("steps").and_then(Value::as_array) {
                let rows = steps
                    .iter()
                    .enumerate()
                    .filter_map(|(index, step)| {
                        let step = step.as_object()?;
                        Some(vec![
                            json!(index + 1),
                            step.get("title").cloned().unwrap_or(Value::Null),
                            step.get("status").cloned().unwrap_or(Value::Null),
                            step.get("executor").cloned().unwrap_or(Value::Null),
                            json!(
                                step.get("checkpoints")
                                    .and_then(Value::as_array)
                                    .map_or(0, Vec::len)
                            ),
                            step.get("note").cloned().unwrap_or(Value::Null),
                        ])
                    })
                    .collect::<Vec<_>>();
                if let Some(table) = Self::table_block(
                    "plan-steps",
                    vec!["#", "Step", "Status", "Executor", "Checks", "Note"],
                    rows,
                ) {
                    blocks.push(table);
                } else if steps.is_empty() {
                    blocks.push(Self::markdown_block(
                        "plan-steps",
                        "### Plan steps\nNo plan steps defined.",
                    ));
                }

                let mut checkpoint_rows = Vec::new();
                for (step_index, step) in steps.iter().enumerate() {
                    let Some(step) = step.as_object() else {
                        continue;
                    };
                    let Some(checkpoints) = step.get("checkpoints").and_then(Value::as_array)
                    else {
                        continue;
                    };
                    for (check_index, checkpoint) in checkpoints.iter().enumerate() {
                        let Some(checkpoint) = checkpoint.as_object() else {
                            continue;
                        };
                        checkpoint_rows.push(vec![
                            json!(step_index + 1),
                            step.get("title").cloned().unwrap_or(Value::Null),
                            json!(check_index + 1),
                            checkpoint.get("text").cloned().unwrap_or(Value::Null),
                            checkpoint.get("status").cloned().unwrap_or(Value::Null),
                        ]);
                    }
                }
                if let Some(table) = Self::table_block(
                    "plan-checkpoints",
                    vec!["Step", "Step title", "Check", "Checkpoint", "Status"],
                    checkpoint_rows,
                ) {
                    blocks.push(table);
                }
            }
            if let Some(document) = Self::object_string(plan, "document_markdown") {
                blocks.push(Self::markdown_block(
                    "plan-document",
                    Self::bounded_human_text(&document),
                ));
            }
        }
        if let Some(current_step) = object.get("current_step").and_then(Value::as_object) {
            let fields = [
                ("Index", Self::object_text(object, "current_step_index")),
                ("Title", Self::object_text(current_step, "title")),
                ("Goal", Self::object_text(object, "current_step_goal")),
                ("Status", Self::object_text(current_step, "status")),
            ];
            if let Some(block) =
                Self::details_block_if_nonempty("plan-current", "Current step", &fields)
            {
                blocks.push(block);
            }
        }
        if key == "plan.review"
            && let Some(decision) = Self::object_string(object, "decision")
        {
            blocks.push(Self::markdown_block(
                "plan-decision",
                format!("### Review decision\n{decision}"),
            ));
        }
        if blocks.is_empty() {
            blocks.push(Self::markdown_block(
                "plan-empty",
                "### Plan\nNo plan details returned.",
            ));
        }
        blocks
    }

    fn specific_filesystem_blocks(
        key: &str,
        object: &serde_json::Map<String, Value>,
        raw: &RawOutput,
    ) -> Vec<ViewBlock> {
        let mut blocks = Vec::new();
        match key {
            "fs.write" => {
                let fields = [
                    ("Path", Self::object_text(object, "path")),
                    ("Action", Self::object_text(object, "kind")),
                    ("Bytes", Self::object_text(object, "bytes")),
                    ("SHA-256", Self::object_text(object, "sha256")),
                ];
                if let Some(block) =
                    Self::details_block_if_nonempty("file-write", "File written", &fields)
                {
                    blocks.push(block);
                }
            }
            "fs.replace" => {
                let fields = [
                    ("Path", Self::object_text(object, "path")),
                    ("Replacements", Self::object_text(object, "replacements")),
                    ("Before SHA-256", Self::object_text(object, "before_sha256")),
                    ("After SHA-256", Self::object_text(object, "after_sha256")),
                ];
                if let Some(block) =
                    Self::details_block_if_nonempty("file-replace", "Text replaced", &fields)
                {
                    blocks.push(block);
                }
            }
            "fs.read_many" => {
                if let Some(files) = Self::object_array(object, "files")
                    && let Some(table) = Self::scalar_table(
                        "read-many-files",
                        "Files read",
                        files,
                        &[
                            ("path", "Path"),
                            ("bytes", "Bytes"),
                            ("returned_bytes", "Returned"),
                            ("truncated", "Truncated"),
                            ("sha256", "SHA-256"),
                            ("error", "Error"),
                        ],
                    )
                {
                    blocks.push(table);
                }
                let fields = [
                    (
                        "Maximum bytes",
                        Self::object_text(object, "max_total_bytes"),
                    ),
                    (
                        "Remaining bytes",
                        Self::object_text(object, "remaining_bytes"),
                    ),
                    ("Truncated", Self::object_text(object, "truncated")),
                ];
                if let Some(block) =
                    Self::details_block_if_nonempty("read-many-meta", "Read budget", &fields)
                {
                    blocks.push(block);
                }
                if !raw.text.trim().is_empty() {
                    blocks.push(Self::markdown_block(
                        "file-previews",
                        format!(
                            "### File previews\n{}",
                            Self::bounded_human_text(raw.text.as_str())
                        ),
                    ));
                }
            }
            "fs.stat" => {
                let fields = [
                    ("Path", Self::object_text(object, "path")),
                    ("Type", Self::object_text(object, "kind")),
                    ("Size", Self::object_text(object, "size")),
                    (
                        "Modified",
                        object
                            .get("modified_at_ms")
                            .map(Self::compact_epoch_millis)
                            .unwrap_or_default(),
                    ),
                    ("Read-only", Self::object_text(object, "readonly")),
                    ("SHA-256", Self::object_text(object, "sha256")),
                    ("Hash skipped", Self::object_text(object, "hash_skipped")),
                    (
                        "Symlink target",
                        Self::object_text(object, "symlink_target"),
                    ),
                ];
                if let Some(block) =
                    Self::details_block_if_nonempty("file-stat", "File metadata", &fields)
                {
                    blocks.push(block);
                }
            }
            "fs.view_image" => {
                let fields = [
                    ("Path", Self::object_text(object, "path")),
                    ("Detail", Self::object_text(object, "detail")),
                    ("MIME", Self::object_text(object, "mime")),
                    ("Size", Self::object_text(object, "size_bytes")),
                    ("SHA-256", Self::object_text(object, "sha256")),
                ];
                if let Some(block) = Self::details_block_if_nonempty("image-meta", "Image", &fields)
                {
                    blocks.push(block);
                }
            }
            _ => {}
        }
        blocks
    }

    fn specific_code_blocks(key: &str, object: &serde_json::Map<String, Value>) -> Vec<ViewBlock> {
        let mut blocks = Vec::new();
        match key {
            "code.search_ast" => {
                let fields = [
                    ("Language", Self::object_text(object, "language")),
                    ("Pattern", Self::object_text(object, "pattern")),
                    ("Scanned files", Self::object_text(object, "scanned_files")),
                ];
                if let Some(block) =
                    Self::details_block_if_nonempty("ast-search-meta", "AST search", &fields)
                {
                    blocks.push(block);
                }
                if let Some(matches) = Self::object_array(object, "matches")
                    && let Some(table) = Self::scalar_table(
                        "ast-matches",
                        "Structural matches",
                        matches,
                        &[
                            ("path", "Path"),
                            ("line", "Line"),
                            ("column", "Column"),
                            ("text", "Match"),
                        ],
                    )
                {
                    blocks.push(table);
                }
            }
            "code.syntax_tree" => {
                let fields = [
                    ("Path", Self::object_text(object, "path")),
                    ("Language", Self::object_text(object, "language")),
                    ("Root", Self::object_text(object, "root_kind")),
                    ("Parse errors", Self::object_text(object, "has_error")),
                ];
                if let Some(block) =
                    Self::details_block_if_nonempty("syntax-tree-meta", "Syntax tree", &fields)
                {
                    blocks.push(block);
                }
                if let Some(tree) = object.get("tree") {
                    blocks.extend(Self::generic_value_blocks("syntax-tree", "Tree", tree, 0));
                }
            }
            _ => {}
        }
        blocks
    }

    fn specific_report_blocks(object: &serde_json::Map<String, Value>) -> Vec<ViewBlock> {
        let mut blocks = Vec::new();
        if let Some(summary) = Self::object_string(object, "summary") {
            blocks.push(Self::markdown_block(
                "findings-summary",
                format!("### Summary\n{summary}"),
            ));
        }
        if let Some(findings) = Self::object_array(object, "findings")
            && let Some(table) = Self::scalar_table(
                "findings",
                "Findings",
                findings,
                &[
                    ("severity", "Severity"),
                    ("file", "File"),
                    ("line", "Line"),
                    ("end_line", "End line"),
                    ("title", "Finding"),
                    ("confidence", "Confidence"),
                    ("body", "Details"),
                ],
            )
        {
            blocks.push(table);
        }
        if let Some(counts) = object.get("counts") {
            blocks.extend(Self::generic_value_blocks(
                "finding-counts",
                "Severity counts",
                counts,
                0,
            ));
        }
        blocks
    }

    fn specific_session_blocks(
        key: &str,
        object: &serde_json::Map<String, Value>,
    ) -> Vec<ViewBlock> {
        let mut blocks = Vec::new();
        match key {
            "session.environment" => {
                let fields = [
                    ("Workspace", Self::object_text(object, "workspace_root")),
                    ("Git branch", Self::object_text(object, "git_branch")),
                    ("Commit", Self::object_text(object, "git_short_sha")),
                    ("Dirty", Self::object_text(object, "git_dirty")),
                    ("Shell", Self::object_text(object, "shell")),
                    ("OS", Self::object_text(object, "os")),
                    ("Architecture", Self::object_text(object, "arch")),
                ];
                if let Some(block) =
                    Self::details_block_if_nonempty("environment", "Environment", &fields)
                {
                    blocks.push(block);
                }
            }
            "session.model" => {
                let fields = [
                    ("Provider", Self::object_text(object, "model_provider_id")),
                    ("Adapter", Self::object_text(object, "model_adapter_id")),
                    ("Model", Self::object_text(object, "model_id")),
                    ("Thinking", Self::object_text(object, "thinking_mode")),
                    ("Speed", Self::object_text(object, "speed_mode")),
                    ("Verbosity", Self::object_text(object, "verbosity")),
                    (
                        "Context window",
                        Self::object_text(object, "model_context_window_tokens"),
                    ),
                    (
                        "Max input",
                        Self::object_text(object, "model_max_input_tokens"),
                    ),
                    (
                        "Max output",
                        Self::object_text(object, "model_max_output_tokens"),
                    ),
                ];
                if let Some(block) = Self::details_block_if_nonempty("model", "Model", &fields) {
                    blocks.push(block);
                }
            }
            "session.tokens" => {
                let fields = [
                    ("Current", Self::object_text(object, "current_tokens")),
                    (
                        "Measured prompt",
                        Self::object_text(object, "measured_prompt_tokens"),
                    ),
                    ("Projected", Self::object_text(object, "projected_tokens")),
                    ("Limit", Self::object_text(object, "limit_tokens")),
                    ("Remaining", Self::object_text(object, "remaining_tokens")),
                    ("Reserved", Self::object_text(object, "reserved_tokens")),
                    ("Usage ratio", Self::object_text(object, "usage_ratio")),
                ];
                if let Some(block) =
                    Self::details_block_if_nonempty("tokens", "Token usage", &fields)
                {
                    blocks.push(block);
                }
            }
            "session.get" | "session.rename" => {
                let session = object
                    .get("session")
                    .and_then(Value::as_object)
                    .unwrap_or(object);
                let fields = [
                    ("ID", Self::object_text(session, "id")),
                    ("Title", Self::object_text(session, "title")),
                    ("Parent", Self::object_text(session, "parent_id")),
                    ("Root", Self::object_text(session, "root_id")),
                    ("Subagent", Self::object_text(session, "is_subagent")),
                ];
                if let Some(block) = Self::details_block_if_nonempty("session", "Session", &fields)
                {
                    blocks.push(block);
                }
            }
            _ => {}
        }
        blocks
    }

    fn specific_task_blocks(key: &str, object: &serde_json::Map<String, Value>) -> Vec<ViewBlock> {
        let mut blocks = Vec::new();
        if let Some(tasks) = Self::object_array(object, "tasks") {
            if !tasks.is_empty()
                && let Some(table) = Self::scalar_table(
                    "tasks",
                    "Tasks",
                    tasks,
                    &[
                        ("task_id", "Task"),
                        ("status", "Status"),
                        ("description", "Description"),
                        ("access", "Access"),
                        ("model_id", "Model"),
                        ("session_id", "Session"),
                    ],
                )
            {
                blocks.push(table);
            } else if tasks.is_empty() {
                blocks.push(Self::markdown_block("tasks", "### Tasks\nNo tasks found."));
            }
        }
        if let Some(task) = object.get("task").and_then(Value::as_object) {
            let title = match key {
                "tasks.run" => "Task started",
                "tasks.cancel" => "Task cancellation",
                "tasks.followup" => "Task follow-up",
                "tasks.message" => "Task message",
                "tasks.output" => "Task output",
                "tasks.get" => "Task details",
                _ => "Task",
            };
            let fields = [
                ("Task", Self::object_text(task, "task_id")),
                ("Description", Self::object_text(task, "description")),
                ("Status", Self::object_text(task, "status")),
                ("Access", Self::object_text(task, "access")),
                ("Session", Self::object_text(task, "session_id")),
                ("Model", Self::object_text(task, "model_id")),
                ("Error", Self::object_text(task, "error")),
            ];
            blocks.push(Self::details_block("task-meta", title, &fields));
        }
        if let Some(chunks) = Self::object_array(object, "chunks") {
            if !chunks.is_empty()
                && let Some(table) = Self::scalar_table(
                    "task-chunks",
                    "Task output",
                    chunks,
                    &[("role", "Role"), ("text", "Text")],
                )
            {
                blocks.push(table);
            } else if let Some(chunks) = object.get("chunks").and_then(Value::as_array)
                && !chunks.is_empty()
            {
                blocks.extend(Self::generic_value_blocks(
                    "task-chunks",
                    "Task output",
                    &Value::Array(chunks.clone()),
                    0,
                ));
            } else if chunks.is_empty() {
                blocks.push(Self::markdown_block(
                    "task-chunks",
                    "### Task output\nNo output chunks returned.",
                ));
            }
        }
        let output_fields = [
            ("Next Cursor", Self::object_text(object, "next_cursor")),
            ("More available", Self::object_text(object, "has_more")),
            ("Timed out", Self::object_text(object, "timed_out")),
        ];
        if let Some(block) = Self::details_block_if_nonempty(
            "task-output-meta",
            "Task output status",
            &output_fields,
        ) {
            blocks.push(block);
        }
        if let Some(final_text) = object
            .get("final_text")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
        {
            blocks.push(Self::markdown_block(
                "task-result",
                Self::bounded_human_text(final_text),
            ));
        }
        blocks
    }

    fn specific_skill_blocks(key: &str, object: &serde_json::Map<String, Value>) -> Vec<ViewBlock> {
        let mut blocks = Vec::new();
        match key {
            "skills.list" => {
                if let Some(tools) = Self::object_array(object, "tools") {
                    if !tools.is_empty()
                        && let Some(table) = Self::scalar_table(
                            "skills",
                            "Discovered skills",
                            tools,
                            &[
                                ("name", "Name"),
                                ("kind", "Kind"),
                                ("summary", "Summary"),
                                ("source", "Source"),
                                ("editable", "Editable"),
                                ("content_hash", "Hash"),
                            ],
                        )
                    {
                        blocks.push(table);
                    } else if !tools.is_empty() {
                        blocks.extend(Self::generic_value_blocks(
                            "skills",
                            "Discovered skills",
                            &Value::Array(tools.clone()),
                            0,
                        ));
                    } else if tools.is_empty() {
                        blocks.push(Self::markdown_block(
                            "skills",
                            "### Discovered skills\nNo skills found.",
                        ));
                    }
                }
                let fields = [
                    ("Returned", Self::object_text(object, "returned")),
                    ("Total", Self::object_text(object, "total")),
                    ("Offset", Self::object_text(object, "offset")),
                    ("Filter", Self::object_text(object, "kind")),
                ];
                if let Some(block) =
                    Self::details_block_if_nonempty("skills-page", "Catalog page", &fields)
                {
                    blocks.push(block);
                }
                if let Some(diagnostics) = Self::object_array(object, "diagnostics") {
                    if !diagnostics.is_empty()
                        && let Some(table) = Self::scalar_table(
                            "skills-diagnostics",
                            "Discovery diagnostics",
                            diagnostics,
                            &[("problem", "Problem")],
                        )
                    {
                        blocks.push(table);
                    } else if !diagnostics.is_empty() {
                        blocks.extend(Self::generic_value_blocks(
                            "skills-diagnostics",
                            "Discovery diagnostics",
                            &Value::Array(diagnostics.clone()),
                            0,
                        ));
                    } else if diagnostics.is_empty() {
                        blocks.push(Self::markdown_block(
                            "skills-diagnostics",
                            "### Discovery diagnostics\nNo discovery diagnostics.",
                        ));
                    }
                }
            }
            "skills.get" => {
                let fields = [
                    ("Name", Self::object_text(object, "name")),
                    ("Kind", Self::object_text(object, "kind")),
                    ("Source", Self::object_text(object, "source")),
                    ("Source path", Self::object_text(object, "source_path")),
                    ("Editable", Self::object_text(object, "editable")),
                    ("Hash", Self::object_text(object, "content_hash")),
                ];
                if let Some(block) = Self::details_block_if_nonempty("skill-meta", "Skill", &fields)
                {
                    blocks.push(block);
                }
                if let Some(body) = Self::object_string(object, "body") {
                    blocks.push(Self::markdown_block(
                        "skill-body",
                        format!("### Body\n{}", Self::bounded_human_text(&body)),
                    ));
                }
            }
            "skills.create" | "skills.update" | "skills.delete" => {
                let fields = [
                    ("Operation", Self::object_text(object, "operation")),
                    ("Name", Self::object_text(object, "name")),
                    ("Path", Self::object_text(object, "path")),
                    (
                        "Catalog generation",
                        Self::object_text(object, "catalog_generation"),
                    ),
                    (
                        "Catalog changed",
                        Self::object_text(object, "catalog_changed"),
                    ),
                    ("Editable", Self::object_text(object, "editable")),
                ];
                if let Some(block) =
                    Self::details_block_if_nonempty("skill-write", "Skill change", &fields)
                {
                    blocks.push(block);
                }
            }
            "skills.read_resource" => {
                let fields = [
                    ("Skill", Self::object_text(object, "name")),
                    ("Path", Self::object_text(object, "path")),
                    ("Source", Self::object_text(object, "source")),
                    ("Source path", Self::object_text(object, "source_path")),
                    ("Bytes", Self::object_text(object, "bytes")),
                    ("Hash", Self::object_text(object, "content_hash")),
                ];
                if let Some(block) = Self::details_block_if_nonempty(
                    "skill-resource-meta",
                    "Skill resource",
                    &fields,
                ) {
                    blocks.push(block);
                }
                if let Some(content) = object
                    .get("content")
                    .or_else(|| object.get("body"))
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                {
                    blocks.push(Self::markdown_block(
                        "skill-resource-content",
                        format!("### Content\n{}", Self::bounded_human_text(content)),
                    ));
                }
            }
            "skills.refresh" => {
                let fields = [
                    ("Changed", Self::object_text(object, "changed")),
                    ("Generation", Self::object_text(object, "generation")),
                    ("Skills", Self::object_text(object, "skills")),
                    ("Commands", Self::object_text(object, "commands")),
                ];
                if let Some(block) =
                    Self::details_block_if_nonempty("skills-refresh", "Skill catalog", &fields)
                {
                    blocks.push(block);
                }
                for child_key in ["tools", "skills", "commands"] {
                    if let Some(values) = Self::object_array(object, child_key) {
                        if let Some(table) = Self::scalar_table(
                            format!("skills-refresh-{child_key}"),
                            &Self::humanize_key(child_key),
                            values,
                            &[
                                ("name", "Name"),
                                ("kind", "Kind"),
                                ("summary", "Summary"),
                                ("source", "Source"),
                            ],
                        ) {
                            blocks.push(table);
                        } else if values.iter().all(Value::is_string) {
                            blocks.push(Self::markdown_block(
                                format!("skills-refresh-{child_key}"),
                                format!(
                                    "### {}\n{}",
                                    Self::humanize_key(child_key),
                                    values
                                        .iter()
                                        .map(Self::generic_field_value)
                                        .map(|value| format!("- {value}"))
                                        .collect::<Vec<_>>()
                                        .join("\n")
                                ),
                            ));
                        }
                    }
                }
                if let Some(watcher) = object.get("watcher") {
                    blocks.extend(Self::generic_value_blocks(
                        "skills-watcher",
                        "Watcher",
                        watcher,
                        0,
                    ));
                }
            }
            _ => {}
        }
        blocks
    }

    fn specific_settings_blocks(
        key: &str,
        object: &serde_json::Map<String, Value>,
    ) -> Vec<ViewBlock> {
        let mut blocks = Vec::new();
        match key {
            "settings.get" => {
                let fields = [
                    ("Path", Self::object_text(object, "path")),
                    ("Source", Self::object_text(object, "source")),
                    ("Layer", Self::object_text(object, "layer")),
                    ("Config path", Self::object_text(object, "config_path")),
                    ("Found", Self::object_text(object, "config_found")),
                ];
                if let Some(block) =
                    Self::details_block_if_nonempty("settings-read-meta", "Setting", &fields)
                {
                    blocks.push(block);
                }
                if let Some(value) = object.get("value") {
                    let path = Self::object_string(object, "path");
                    let value = Self::redacted_setting_value(value, path.as_deref());
                    blocks.extend(Self::generic_value_blocks(
                        "settings-value",
                        "Value",
                        &value,
                        0,
                    ));
                }
            }
            "settings.list" => {
                if let Some(items) = Self::object_array(object, "items") {
                    let items = Self::redacted_setting_records(items);
                    if !items.is_empty()
                        && let Some(table) = Self::scalar_table(
                            "settings-items",
                            "Settings",
                            &items,
                            &[
                                ("path", "Path"),
                                ("kind", "Kind"),
                                ("value", "Value"),
                                ("source", "Source"),
                            ],
                        )
                    {
                        blocks.push(table);
                    } else if items.is_empty() {
                        blocks.push(Self::markdown_block(
                            "settings-items",
                            "### Settings\nNo settings found.",
                        ));
                    }
                }
                let fields = [
                    ("Source", Self::object_text(object, "source")),
                    ("Path", Self::object_text(object, "path")),
                    ("Config path", Self::object_text(object, "config_path")),
                    ("Found", Self::object_text(object, "config_found")),
                ];
                if let Some(block) = Self::details_block_if_nonempty(
                    "settings-list-meta",
                    "Settings source",
                    &fields,
                ) {
                    blocks.push(block);
                }
            }
            "settings.inspect" => {
                let fields = [("Path", Self::object_text(object, "path"))];
                if let Some(block) = Self::details_block_if_nonempty(
                    "settings-inspect-meta",
                    "Settings inspection",
                    &fields,
                ) {
                    blocks.push(block);
                }
                for (child_key, title) in [
                    ("global", "Global"),
                    ("workspace", "Workspace"),
                    ("effective", "Effective"),
                    ("applied_layers", "Applied layers"),
                    ("layers", "Layers"),
                ] {
                    if let Some(value) = object.get(child_key) {
                        let child_id = format!("settings-{child_key}");
                        let path = Self::object_string(object, "path");
                        let value = Self::redacted_setting_value(value, path.as_deref());
                        blocks.extend(Self::generic_value_blocks(
                            child_id.as_str(),
                            title,
                            &value,
                            0,
                        ));
                    }
                }
            }
            "settings.set" | "settings.delete" | "settings.patch" => {
                let fields = [
                    ("Operation", Self::object_text(object, "operation")),
                    ("Path", Self::object_text(object, "path")),
                    ("Layer", Self::object_text(object, "layer")),
                    ("Config path", Self::object_text(object, "config_path")),
                    ("Dry run", Self::object_text(object, "dry_run")),
                    ("Changed", Self::object_text(object, "changed")),
                    ("Created", Self::object_text(object, "created")),
                    ("Deleted", Self::object_text(object, "deleted")),
                    ("Validated", Self::object_text(object, "validated")),
                    (
                        "Reload required",
                        Self::object_text(object, "reload_required"),
                    ),
                    (
                        "Reload requested",
                        Self::object_text(object, "reload_requested"),
                    ),
                ];
                if let Some(block) = Self::details_block_if_nonempty(
                    "settings-edit-meta",
                    "Settings change",
                    &fields,
                ) {
                    blocks.push(block);
                }
                for (child_key, title) in [
                    ("previous", "Previous"),
                    ("current", "Current"),
                    ("reload", "Reload"),
                ] {
                    if let Some(value) = object.get(child_key) {
                        let child_id = format!("settings-{child_key}");
                        let path = Self::object_string(object, "path");
                        let value = Self::redacted_setting_value(value, path.as_deref());
                        blocks.extend(Self::generic_value_blocks(
                            child_id.as_str(),
                            title,
                            &value,
                            0,
                        ));
                    }
                }
                for child_key in ["updated_paths", "changed_paths", "deleted_paths"] {
                    if let Some(value) = object.get(child_key) {
                        let child_id = format!("settings-{child_key}");
                        let child_title = Self::humanize_key(child_key);
                        blocks.extend(Self::generic_value_blocks(
                            child_id.as_str(),
                            child_title.as_str(),
                            value,
                            0,
                        ));
                    }
                }
            }
            "settings.validate" => {
                let fields = [
                    ("Valid", Self::object_text(object, "valid")),
                    ("Layer", Self::object_text(object, "layer")),
                    ("Config path", Self::object_text(object, "config_path")),
                    ("Found", Self::object_text(object, "config_found")),
                ];
                if let Some(block) = Self::details_block_if_nonempty(
                    "settings-validation",
                    "Settings validation",
                    &fields,
                ) {
                    blocks.push(block);
                }
                if let Some(warnings) = Self::object_array(object, "warnings") {
                    if !warnings.is_empty()
                        && let Some(table) = Self::scalar_table(
                            "settings-warnings",
                            "Validation warnings",
                            warnings,
                            &[("path", "Path"), ("message", "Warning")],
                        )
                    {
                        blocks.push(table);
                    } else if warnings.is_empty() {
                        blocks.push(Self::markdown_block(
                            "settings-warnings",
                            "### Validation warnings\nNo validation warnings.",
                        ));
                    }
                }
                if let Some(files) = object.get("files") {
                    blocks.extend(Self::generic_value_blocks(
                        "settings-files",
                        "Files",
                        files,
                        0,
                    ));
                }
            }
            _ => {}
        }
        blocks
    }

    fn browser_snapshot_object(
        object: &serde_json::Map<String, Value>,
    ) -> Option<&serde_json::Map<String, Value>> {
        object
            .get("snapshot")
            .and_then(Value::as_object)
            .or_else(|| {
                object
                    .get("result")
                    .and_then(Value::as_object)
                    .and_then(|result| result.get("snapshot"))
                    .and_then(Value::as_object)
            })
    }

    fn browser_action_summary(key: &str, object: &serde_json::Map<String, Value>) -> String {
        let result = object.get("result").and_then(Value::as_object);
        let action = object
            .get("action")
            .or_else(|| result.and_then(|result| result.get("action")));
        let label = match key {
            "browser_open" | "web.browser_open" => "opened",
            "browser_snapshot" | "web.browser_snapshot" => "inspected",
            "browser_click" | "web.browser_click" => "clicked",
            "browser_type" | "web.browser_type" => "typed",
            "browser_wait" | "web.browser_wait" => "ready",
            _ => "completed",
        };
        let Some(action) = action else {
            return label.to_owned();
        };
        if let Some(action) = action.as_object() {
            if action.get("ok").and_then(Value::as_bool) == Some(false) {
                return "failed".to_owned();
            }
            if let Some(method) = action.get("method").and_then(Value::as_str) {
                return format!("{label} via {method}");
            }
        }
        label.to_owned()
    }

    fn specific_browser_blocks(
        key: &str,
        object: &serde_json::Map<String, Value>,
    ) -> Vec<ViewBlock> {
        let mut blocks = Vec::new();
        match key {
            "browser_list" | "web.browser_list" => {
                if let Some(sessions) = Self::object_array(object, "sessions") {
                    if !sessions.is_empty()
                        && let Some(table) = Self::scalar_table(
                            "browser-sessions",
                            "Browser pages",
                            sessions,
                            &[
                                ("session_id", "Session"),
                                ("title", "Title"),
                                ("url", "URL"),
                                ("attached", "Attached"),
                            ],
                        )
                    {
                        blocks.push(table);
                    } else if sessions.is_empty() {
                        blocks.push(Self::markdown_block(
                            "browser-sessions",
                            "### Browser pages\nNo browser pages found.",
                        ));
                    }
                }
                let fields = [(
                    "Browser running",
                    Self::object_text(object, "browser_running"),
                )];
                if let Some(block) =
                    Self::details_block_if_nonempty("browser-status", "Browser", &fields)
                {
                    blocks.push(block);
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
                let snapshot = Self::browser_snapshot_object(object);
                let fields = [
                    ("Session", Self::object_text(object, "session_id")),
                    ("Action", Self::browser_action_summary(key, object)),
                    (
                        "Title",
                        snapshot
                            .map(|value| Self::object_text(value, "title"))
                            .unwrap_or_default(),
                    ),
                    (
                        "URL",
                        snapshot
                            .map(|value| Self::object_text(value, "url"))
                            .unwrap_or_default(),
                    ),
                    ("Condition", Self::object_text(object, "condition")),
                    ("Elapsed", Self::object_text(object, "elapsed_ms")),
                    (
                        "Document Requests Intercepted",
                        Self::object_text(object, "document_requests_intercepted"),
                    ),
                ];
                if let Some(block) =
                    Self::details_block_if_nonempty("browser-page", "Page result", &fields)
                {
                    blocks.push(block);
                }
                if let Some(result) = object.get("result").and_then(Value::as_object)
                    && let Some(action) = result.get("action")
                {
                    blocks.extend(Self::generic_value_blocks(
                        "browser-action",
                        "Action",
                        action,
                        0,
                    ));
                }
                if let Some(snapshot) = snapshot {
                    if let Some(elements) = Self::object_array(snapshot, "elements")
                        && let Some(table) = Self::scalar_table(
                            "browser-elements",
                            "Interactive elements",
                            elements,
                            &[
                                ("ref", "Ref"),
                                ("role", "Role"),
                                ("name", "Name"),
                                ("selector", "Selector"),
                            ],
                        )
                    {
                        blocks.push(table);
                    }
                    if let Some(text) = Self::object_string(snapshot, "text") {
                        blocks.push(Self::markdown_block(
                            "browser-text",
                            format!("### Page text\n{}", Self::bounded_generic_text(&text)),
                        ));
                    }
                }
            }
            "browser_close" | "web.browser_close" => {
                let fields = [
                    ("Session", Self::object_text(object, "session_id")),
                    ("Closed", Self::object_text(object, "closed")),
                ];
                if let Some(block) =
                    Self::details_block_if_nonempty("browser-close", "Page closed", &fields)
                {
                    blocks.push(block);
                }
            }
            "browser_shutdown" | "web.browser_shutdown" => {
                let fields = [("Closed", Self::object_text(object, "closed"))];
                if let Some(block) =
                    Self::details_block_if_nonempty("browser-shutdown", "Browser shutdown", &fields)
                {
                    blocks.push(block);
                }
            }
            "browser_screenshot" | "web.browser_screenshot" => {
                let fields = [
                    ("Session", Self::object_text(object, "session_id")),
                    ("Path", Self::object_text(object, "path")),
                    ("Size", Self::object_text(object, "size_bytes")),
                ];
                if let Some(block) =
                    Self::details_block_if_nonempty("browser-screenshot", "Screenshot", &fields)
                {
                    blocks.push(block);
                }
            }
            "browser_download" | "web.browser_download" => {
                let fields = [
                    ("Session", Self::object_text(object, "session_id")),
                    ("URL", Self::object_text(object, "url")),
                    ("Path", Self::object_text(object, "path")),
                    ("Size", Self::object_text(object, "size_bytes")),
                ];
                if let Some(block) =
                    Self::details_block_if_nonempty("browser-download", "Download", &fields)
                {
                    blocks.push(block);
                }
                if let Some(redirects) = object.get("preflight_redirects") {
                    blocks.extend(Self::generic_value_blocks(
                        "browser-redirects",
                        "Preflight redirects",
                        redirects,
                        0,
                    ));
                }
            }
            _ => {}
        }
        blocks
    }

    fn specific_repository_blocks(
        key: &str,
        object: &serde_json::Map<String, Value>,
    ) -> Vec<ViewBlock> {
        let mut blocks = Vec::new();
        match key {
            "repo.status" | "snapshot.status" => {
                let fields = [
                    ("Root", Self::object_text(object, "root")),
                    ("Branch", Self::object_text(object, "branch")),
                    ("Head", Self::object_text(object, "head")),
                    ("Dirty", Self::object_text(object, "dirty")),
                ];
                if let Some(block) =
                    Self::details_block_if_nonempty("repository-meta", "Repository", &fields)
                {
                    blocks.push(block);
                }
                if let Some(changes) = Self::object_array(object, "changes") {
                    if !changes.is_empty()
                        && let Some(table) = Self::scalar_table(
                            "repository-changes",
                            "Changes",
                            changes,
                            &[
                                ("path", "Path"),
                                ("kind", "Kind"),
                                ("additions", "Additions"),
                                ("deletions", "Deletions"),
                            ],
                        )
                    {
                        blocks.push(table);
                    } else if changes.is_empty() {
                        blocks.push(Self::markdown_block(
                            "repository-changes",
                            "### Changes\nNo uncommitted changes.",
                        ));
                    }
                }
                if let Some(snapshots) = Self::object_array(object, "snapshots") {
                    if !snapshots.is_empty()
                        && let Some(table) = Self::scalar_table(
                            "active-snapshots",
                            "Active snapshots",
                            snapshots,
                            &[
                                ("session_id", "Session"),
                                ("path", "Path"),
                                ("branch", "Branch"),
                                ("created_here", "Created here"),
                            ],
                        )
                    {
                        blocks.push(table);
                    } else if snapshots.is_empty() {
                        blocks.push(Self::markdown_block(
                            "active-snapshots",
                            "### Active snapshots\nNo active snapshots.",
                        ));
                    }
                }
            }
            "snapshot.enter" | "snapshot.exit" => {
                let fields = [
                    ("Action", Self::object_text(object, "action")),
                    ("Path", Self::object_text(object, "path")),
                    ("Branch", Self::object_text(object, "branch")),
                    ("Backend", Self::object_text(object, "backend")),
                    ("Note", Self::object_text(object, "note")),
                ];
                if let Some(block) =
                    Self::details_block_if_nonempty("snapshot-operation", "Snapshot", &fields)
                {
                    blocks.push(block);
                }
            }
            _ => {}
        }
        blocks
    }

    fn specific_notebook_blocks(object: &serde_json::Map<String, Value>) -> Vec<ViewBlock> {
        let fields = [
            ("Path", Self::object_text(object, "path")),
            ("Action", Self::object_text(object, "action")),
            ("Cell", Self::object_text(object, "cell_index")),
            ("Cells", Self::object_text(object, "cell_count")),
            ("Before SHA-256", Self::object_text(object, "before_sha256")),
            ("After SHA-256", Self::object_text(object, "after_sha256")),
        ];
        Self::details_block_if_nonempty("notebook-edit", "Notebook cell", &fields)
            .into_iter()
            .collect()
    }

    /// Build the object used by tool-specific presentation renderers from all
    /// structured result channels. Adapters normally put the main result in
    /// `payload`, but receipts, exit codes, truncation markers, and provider
    /// facts can arrive in `metadata` or as a JSON text fallback. Keep the
    /// payload's values when keys overlap, except that an explicit failure
    /// status from a supplemental channel must not be hidden by a stale
    /// `status: completed` payload.
    fn specific_result_object(raw: &RawOutput) -> Option<serde_json::Map<String, Value>> {
        let mut object = serde_json::Map::new();
        let mut merge = |source: &serde_json::Map<String, Value>| {
            for (key, value) in source {
                let should_replace = matches!(key.as_str(), "status" | "state")
                    && Self::is_failure_status(value)
                    && object
                        .get(key)
                        .is_none_or(|current| !Self::is_failure_status(current));
                if should_replace {
                    object.insert(key.clone(), value.clone());
                } else {
                    object.entry(key.clone()).or_insert_with(|| value.clone());
                }
            }
        };

        if let Some(payload) = raw.payload.as_ref().and_then(Value::as_object) {
            merge(payload);
        }
        if let Some(text_object) =
            Self::json_document(raw.text.as_str()).and_then(|value| value.as_object().cloned())
        {
            merge(&text_object);
        }
        if !raw.metadata.is_empty() {
            let metadata = raw
                .metadata
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect::<serde_json::Map<_, _>>();
            merge(&metadata);
        }
        (!object.is_empty()).then_some(object)
    }

    fn is_failure_status(value: &Value) -> bool {
        value.as_str().is_some_and(|status| {
            matches!(
                status.trim().to_ascii_lowercase().as_str(),
                "failed"
                    | "failure"
                    | "error"
                    | "cancelled"
                    | "canceled"
                    | "timed_out"
                    | "timed out"
            )
        })
    }

    fn specific_tool_blocks(tool_name: &str, raw: &RawOutput) -> Vec<ViewBlock> {
        let key = Self::normalized_tool_name(tool_name);
        let Some(object) = Self::specific_result_object(raw) else {
            if key == "memory.delete" {
                return vec![Self::markdown_block(
                    "memory-delete",
                    if raw.text.trim().is_empty() {
                        "### Memory deletion\nNo deletion details returned.".to_owned()
                    } else {
                        format!("### Memory deletion\n{}", raw.text.trim())
                    },
                )];
            }
            if raw.text.trim().is_empty() {
                return vec![Self::empty_state_block(&key)];
            }
            return Vec::new();
        };
        let mut blocks = if key.starts_with("chatgpt.")
            || key.starts_with("claude.")
            || key.starts_with("gemini.")
            || key.starts_with("openai.")
            || object.contains_key("response_receipt")
            || object.contains_key("continuation_required")
            || object.contains_key("pending_calls")
            || (object.contains_key("provider") && object.contains_key("tool"))
        {
            Self::specific_provider_blocks(&key, &object)
        } else {
            Vec::new()
        };
        match key.as_str() {
            value if value.starts_with("fs.") => {
                blocks.extend(Self::specific_filesystem_blocks(value, &object, raw));
            }
            "code.search_ast" | "code.syntax_tree" => {
                blocks.extend(Self::specific_code_blocks(key.as_str(), &object));
            }
            "report.findings" => blocks.extend(Self::specific_report_blocks(&object)),
            value if value.starts_with("session.") => {
                blocks.extend(Self::specific_session_blocks(value, &object));
            }
            value if value.starts_with("interaction.") || value == "ask_user" => {
                blocks.extend(Self::specific_interaction_blocks(value, &object));
            }
            value if value.starts_with("lsp.") => {
                blocks.extend(Self::specific_lsp_blocks(value, &object));
            }
            value if value.starts_with("mcp.") => {
                blocks.extend(Self::specific_mcp_blocks(value, &object));
            }
            value if value.starts_with("memory.") => {
                blocks.extend(Self::specific_memory_blocks(value, &object));
            }
            value if value.starts_with("plan.") => {
                blocks.extend(Self::specific_plan_blocks(value, &object));
            }
            value if value.starts_with("tasks.") => {
                blocks.extend(Self::specific_task_blocks(value, &object));
            }
            value if value.starts_with("skills.") => {
                blocks.extend(Self::specific_skill_blocks(value, &object));
            }
            value if value.starts_with("settings.") => {
                blocks.extend(Self::specific_settings_blocks(value, &object));
            }
            value if value.starts_with("web.") && !value.starts_with("web.browser_") => {
                blocks.extend(Self::specific_web_blocks(value, &object));
            }
            value if value.starts_with("browser_") || value.starts_with("web.browser_") => {
                blocks.extend(Self::specific_browser_blocks(value, &object));
            }
            "repo.status" | "snapshot.status" | "snapshot.enter" | "snapshot.exit" => {
                blocks.extend(Self::specific_repository_blocks(key.as_str(), &object));
            }
            "notebook.edit_cell" => blocks.extend(Self::specific_notebook_blocks(&object)),
            _ => {}
        }
        if blocks.is_empty() && raw.text.trim().is_empty() && object.is_empty() {
            blocks.push(Self::empty_state_block(&key));
        }
        blocks
    }

    fn structured_blocks(
        tool_name: &str,
        raw: &RawOutput,
        output: &ToolOutput,
        command: Option<&str>,
        cwd: Option<&str>,
    ) -> Vec<ViewBlock> {
        let parsed = ToolPayloadOutput::from_tool_output(tool_name, output);
        let mut blocks = if parsed.is_none() {
            Self::specific_tool_blocks(tool_name, raw)
        } else {
            Vec::new()
        };
        if parsed.is_none() && blocks.is_empty() {
            blocks = Self::specific_discovery_text_blocks(tool_name, raw.text.as_str());
            if blocks.is_empty() {
                return Vec::new();
            }
        }
        let discovery_text_projected = parsed.is_none()
            && blocks.iter().any(|block| {
                block
                    .block_id()
                    .is_some_and(|id| id.starts_with("discovery"))
            });

        if let Some(parsed) = parsed {
            match parsed {
                ToolPayloadOutput::ApplyPatch {
                    changes,
                    diff,
                    before_hash,
                    after_hash,
                    progress,
                    ..
                } => {
                    let has_changes = !changes.is_empty();
                    let has_diff = !diff.trim().is_empty();
                    if !changes.is_empty() {
                        blocks.push(ViewBlock::FileChanges {
                            id: Some("changes".into()),
                            changes,
                        });
                    }
                    if has_diff {
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
                    if !has_changes && !has_diff && fields.is_empty() {
                        blocks.push(Self::markdown_block(
                            "patch-empty",
                            "### Patch\nNo file changes returned.",
                        ));
                    }
                }
                ToolPayloadOutput::Read {
                    preview,
                    loaded_paths,
                    truncated,
                    attachment,
                } => {
                    let has_preview = preview
                        .as_deref()
                        .is_some_and(|preview| !preview.trim().is_empty());
                    let has_attachment = attachment.is_some();
                    if let Some(block) =
                        Self::list_block("loaded-paths", "Loaded paths", &loaded_paths)
                    {
                        blocks.push(block);
                    }
                    if let Some(preview) = preview {
                        let language = loaded_paths
                            .first()
                            .and_then(|path| Self::source_language(path));
                        blocks.push(Self::markdown_code_block(
                            "preview",
                            "Preview",
                            preview.as_str(),
                            language,
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
                    if loaded_paths.is_empty() && !has_preview && !has_attachment {
                        blocks.push(Self::markdown_block(
                            "read-empty",
                            "### Read result\nNo file content returned.",
                        ));
                    }
                }
                ToolPayloadOutput::Glob {
                    paths,
                    count,
                    truncated,
                } => {
                    let visible_paths = paths.iter().take(Self::GENERIC_MAX_ROWS);
                    let mut lines = vec![format!(
                        "### {}",
                        count
                            .map(|count| format!("{count} matches"))
                            .unwrap_or_else(|| "Matches".into())
                    )];
                    lines.extend(visible_paths.map(|path| format!("- `{path}`")));
                    if paths.len() > Self::GENERIC_MAX_ROWS {
                        lines.push(format!(
                            "_Showing the first {} of {} paths._",
                            Self::GENERIC_MAX_ROWS,
                            paths.len()
                        ));
                    }
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
                    if let Some(table) = Self::table_block(
                        "matches",
                        vec!["Path", "Line", "Column", "Match"],
                        Self::grep_rows(results.as_slice()),
                    ) {
                        blocks.push(table);
                    } else {
                        blocks.push(Self::markdown_block(
                            "matches",
                            format!(
                                "### {}\nNo lines matched.",
                                matches
                                    .map(|matches| format!("{matches} matches"))
                                    .unwrap_or_else(|| "Matches".into())
                            ),
                        ));
                    }
                    let mut status_lines = Vec::new();
                    if matches.is_some_and(|matches| matches as usize > results.len()) {
                        status_lines.push(format!(
                            "_Showing {} of {} matches._",
                            results.len(),
                            matches.unwrap_or_default()
                        ));
                    }
                    if truncated {
                        status_lines.push("_Results truncated._".to_owned());
                    }
                    if !status_lines.is_empty() {
                        blocks.push(Self::markdown_block(
                            "matches-status",
                            status_lines.join("\n"),
                        ));
                    }
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
                    let task_title = match Self::normalized_tool_name(tool_name).as_str() {
                        "tasks.cancel" => "Task cancellation",
                        "tasks.followup" => "Task follow-up",
                        "tasks.message" => "Task message",
                        "tasks.output" => "Task output",
                        "tasks.get" => "Task details",
                        "tasks.run" | "task" => "Task started",
                        _ => "Task",
                    };
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
                    if input_tokens > 0 || output_tokens > 0 || reasoning_tokens > 0 {
                        fields.push((
                            "Tokens",
                            format!(
                                "in {input_tokens} · out {output_tokens} · reasoning {reasoning_tokens}"
                            ),
                        ));
                    }
                    if cache_write_tokens > 0 || cache_read_tokens > 0 {
                        fields.push((
                            "Cache",
                            format!("write {cache_write_tokens} · read {cache_read_tokens}"),
                        ));
                    }
                    if total_cost_microusd > 0 {
                        fields.push(("Cost", format!("{total_cost_microusd} micro-USD")));
                    }
                    blocks.push(Self::details_block("task", task_title, &fields));
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
                    let rows = answers
                        .iter()
                        .map(|(question, values)| {
                            vec![
                                Value::String(question.clone()),
                                Value::String(if values.is_empty() {
                                    "—".to_owned()
                                } else {
                                    Self::bounded_generic_text(values.join(", ").as_str())
                                }),
                            ]
                        })
                        .collect::<Vec<_>>();
                    if let Some(table) =
                        Self::table_block("answers", vec!["Question", "Answer"], rows)
                    {
                        blocks.push(table);
                    } else {
                        blocks.push(Self::markdown_block(
                            "answers",
                            "### Answers\nNo answers were recorded.",
                        ));
                    }
                    if timed_out {
                        blocks.push(Self::markdown_block(
                            "answers-status",
                            "_The request timed out._",
                        ));
                    }
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
                    if action == "run" {
                        let event_stdout = events
                            .iter()
                            .filter(|event| {
                                matches!(event.stream, agena_domain::ProcessStream::Stdout)
                            })
                            .map(|event| event.line.as_str())
                            .collect::<Vec<_>>()
                            .join("\n");
                        let stdout = output
                            .map(|output| Self::bounded_human_text(&output))
                            .unwrap_or_else(|| {
                                if !raw.text.is_empty() {
                                    Self::bounded_human_text(raw.text.as_str())
                                } else {
                                    Self::bounded_human_text(event_stdout.as_str())
                                }
                            });
                        let command = command
                            .filter(|command| !command.trim().is_empty())
                            .map(str::to_owned)
                            .unwrap_or_else(|| "shell run".to_owned());
                        let stderr = events
                            .iter()
                            .filter(|event| {
                                matches!(event.stream, agena_domain::ProcessStream::Stderr)
                            })
                            .map(|event| event.line.as_str())
                            .collect::<Vec<_>>()
                            .join("\n");
                        blocks.push(ViewBlock::Command {
                            id: Some("command".into()),
                            command,
                            cwd: cwd.map(str::to_owned),
                            exit_code,
                            stdout,
                            stderr: Self::bounded_human_text(stderr.as_str()),
                        });
                    } else {
                        blocks.extend(Self::event_log_blocks(&events));
                        if blocks
                            .iter()
                            .all(|block| block.block_id() != Some("stdout"))
                            && let Some(output) = output
                            && !output.trim().is_empty()
                        {
                            blocks.push(ViewBlock::Log {
                                id: Some("stdout".into()),
                                stream: agena_domain::CommandOutputStream::Stdout,
                                text: Self::bounded_human_text(output.as_str()),
                            });
                        }
                    }
                    let mut fields = vec![("Action", action.clone())];
                    if let Some(shell) = shell {
                        fields.push(("Shell", shell.to_string()));
                    }
                    if background {
                        fields.push(("Background", "yes".to_owned()));
                    }
                    if let Some(status) = status {
                        fields.push(("Status", status.to_string()));
                    }
                    if let Some(process_id) = process_id {
                        fields.push(("Process", process_id));
                    }
                    if last_seq > 0 {
                        fields.push(("Last event", last_seq.to_string()));
                    }
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
                    } else if matches!(action.as_str(), "list" | "logs" | "stop") {
                        blocks.push(Self::markdown_block(
                            "processes",
                            match action.as_str() {
                                "list" => "### Processes\nNo active processes.",
                                "logs" => "### Processes\nNo process events returned.",
                                "stop" => "### Processes\nNo process record returned.",
                                _ => unreachable!(),
                            },
                        ));
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
                    if let Some(output) = output.filter(|output| !output.trim().is_empty()) {
                        blocks.push(ViewBlock::Log {
                            id: Some("monitor-output".into()),
                            stream: agena_domain::CommandOutputStream::Stdout,
                            text: Self::bounded_human_text(output.as_str()),
                        });
                    } else if !raw.text.trim().is_empty() {
                        blocks.push(ViewBlock::Log {
                            id: Some("monitor-output".into()),
                            stream: agena_domain::CommandOutputStream::Stdout,
                            text: Self::bounded_human_text(raw.text.as_str()),
                        });
                    }
                    let mut fields = vec![("Action", action)];
                    if let Some(monitor_id) = monitor_id {
                        fields.push(("Monitor", monitor_id));
                    }
                    if let Some(status) = status {
                        fields.push(("Status", status.to_string()));
                    }
                    if last_seq > 0 {
                        fields.push(("Last event", last_seq.to_string()));
                    }
                    if let Some(completion_reason) = completion_reason {
                        fields.push(("Completion", completion_reason));
                    }
                    if let Some(exit_code) = exit_code {
                        fields.push(("Exit", exit_code.to_string()));
                    }
                    if let Some(block) =
                        Self::details_block_if_nonempty("monitor-meta", "Monitor", &fields)
                    {
                        blocks.push(block);
                    }
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
                        lines.push(Self::bounded_human_text(&summary));
                    }
                    if let Some(markdown) = markdown {
                        lines.push(String::new());
                        // The fetched document is already Markdown. Keep it in
                        // the Markdown block so headings, links, and lists stay
                        // readable in both TUI and Web.
                        lines.push(Self::bounded_human_text(&markdown));
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
                    if items.is_empty() {
                        blocks.push(Self::markdown_block(
                            "search",
                            "### Search results\nNo web search results.",
                        ));
                    } else {
                        blocks.push(ViewBlock::SearchResults {
                            id: Some("search".into()),
                            total: Some(items.len() as u64),
                            items,
                        });
                    }
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
                            "Status",
                            "Attempt",
                            "Delivery",
                            "Session",
                            "Issue",
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
                        .cloned()
                        .map(Value::String)
                        .collect::<Vec<_>>();
                    if let Some(table) = Self::table_block(
                        "diagnostics",
                        vec!["Location", "Severity", "Message"],
                        Self::diagnostic_rows(values.as_slice()),
                    ) {
                        blocks.push(table);
                    } else {
                        blocks.push(Self::markdown_block(
                            "diagnostics",
                            "### Diagnostics\nNo diagnostics.",
                        ));
                    }
                }
            }
        }

        if !discovery_text_projected {
            Self::append_distinct_raw_text(&mut blocks, raw);
        }
        Self::raw_metadata_blocks(&mut blocks, raw);
        Self::raw_media_blocks(&mut blocks, raw);
        Self::raw_attachment_blocks(&mut blocks, raw);
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
    fn shell_logs_render_streams_and_shell_list_avoids_a_fake_command() {
        let logs = BuiltinHumanRenderer::new("shell.logs");
        let raw = RawOutput {
            payload: Some(json!({
                "action": "logs",
                "process_id": "p-1",
                "events": [
                    {"seq": 1, "stream": "stdout", "ts_ms": 1, "line": "started"},
                    {"seq": 2, "stream": "stderr", "ts_ms": 2, "line": "warning"}
                ],
                "last_seq": 2
            })),
            ..RawOutput::default()
        };
        let blocks = logs.render_human(&ctx(), &raw).expect("render");
        assert!(blocks.iter().any(|block| matches!(
            block,
            ViewBlock::Log {
                id: Some(id),
                stream: agena_domain::CommandOutputStream::Stdout,
                text
            } if id == "stdout" && text.contains("started")
        )));
        assert!(blocks.iter().any(|block| matches!(
            block,
            ViewBlock::Log {
                id: Some(id),
                stream: agena_domain::CommandOutputStream::Stderr,
                text
            } if id == "stderr" && text.contains("warning")
        )));
        assert!(
            !blocks
                .iter()
                .any(|block| matches!(block, ViewBlock::Command { .. }))
        );

        let list = BuiltinHumanRenderer::new("shell.list");
        let raw = RawOutput {
            payload: Some(json!({
                "action": "list",
                "processes": [{
                    "process_id": "p-1",
                    "command": "cargo test",
                    "description": "tests",
                    "status": "running",
                    "background": true,
                    "monitored": false,
                    "started_at_ms": 1,
                    "buffered_lines": 2,
                    "last_seq": 2,
                    "dropped_lines": 0
                }]
            })),
            ..RawOutput::default()
        };
        let blocks = list.render_human(&ctx(), &raw).expect("render");
        assert!(blocks.iter().any(|block| matches!(block, ViewBlock::Table {
            id: Some(id), ..
        } if id == "processes")));
        assert!(
            !blocks
                .iter()
                .any(|block| matches!(block, ViewBlock::Command { .. }))
        );
    }

    #[test]
    fn file_read_preview_uses_a_safe_language_aware_fence() {
        let renderer = BuiltinHumanRenderer::new("fs.read");
        let raw = RawOutput {
            payload: Some(json!({
                "preview": "fn main() {\n```\n}",
                "loaded_paths": ["src/main.rs"]
            })),
            ..RawOutput::default()
        };
        let blocks = renderer.render_human(&ctx(), &raw).expect("render");
        let preview = blocks.iter().find_map(|block| match block {
            ViewBlock::Markdown { id: Some(id), text } if id == "preview" => Some(text.as_str()),
            _ => None,
        });
        let Some(preview) = preview else {
            panic!("expected preview block, got {blocks:?}");
        };
        assert!(preview.starts_with("### Preview\n````rust\n"));
        assert!(preview.ends_with("\n````"));
    }

    #[test]
    fn read_many_keeps_file_sections_in_a_named_preview_block() {
        let renderer = BuiltinHumanRenderer::new("fs.read_many");
        let raw = RawOutput {
            text: "===== a.rs =====\nfn a() {}\n\n===== b.rs =====\nfn b() {}".into(),
            payload: Some(json!({
                "files": [
                    {"path": "a.rs", "bytes": 9, "returned_bytes": 9, "truncated": false},
                    {"path": "b.rs", "bytes": 9, "returned_bytes": 9, "truncated": false}
                ],
                "max_total_bytes": 100,
                "remaining_bytes": 82,
                "truncated": false
            })),
            ..RawOutput::default()
        };
        let blocks = renderer.render_human(&ctx(), &raw).expect("render");
        assert!(blocks.iter().any(|block| matches!(
            block,
            ViewBlock::Markdown { id: Some(id), text }
                if id == "file-previews" && text.contains("===== a.rs =====")
        )));
        assert!(!blocks.iter().any(|block| matches!(
            block,
            ViewBlock::Markdown { id: Some(id), .. } if id == "output-text"
        )));
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
    fn filesystem_and_interaction_results_use_dense_typed_blocks() {
        let grep = BuiltinHumanRenderer::new("fs.grep")
            .render_human(
                &ctx(),
                &RawOutput {
                    payload: Some(json!({
                        "matches": 2,
                        "results": ["src/lib.rs:7: fn render()", "src/main.rs:12: let value"]
                    })),
                    ..RawOutput::default()
                },
            )
            .expect("render grep");
        assert!(grep.iter().any(|block| matches!(
            block,
            ViewBlock::Table {
                id: Some(id),
                columns,
                rows
            } if id == "matches"
                && columns == &["Path", "Line", "Column", "Match"]
                && rows[0][0] == json!("src/lib.rs")
                && rows[0][1] == json!("7")
                && rows[0][3] == json!("fn render()")
        )));

        let stat = BuiltinHumanRenderer::new("fs.stat")
            .render_human(
                &ctx(),
                &RawOutput {
                    payload: Some(json!({
                        "path": "src/lib.rs",
                        "kind": "file",
                        "size": 128,
                        "modified_at_ms": 1_000
                    })),
                    ..RawOutput::default()
                },
            )
            .expect("render stat");
        let stat_text = serde_json::to_string(&stat).expect("serialize stat blocks");
        assert!(stat_text.contains("1970-01-01 00:00 UTC"));
        assert!(!stat_text.contains("modified_at_ms"));

        let answers = BuiltinHumanRenderer::new("interaction.ask")
            .render_human(
                &ctx(),
                &RawOutput {
                    payload: Some(json!({"answers": {"0": ["yes"]}})),
                    ..RawOutput::default()
                },
            )
            .expect("render answers");
        assert!(answers.iter().any(|block| matches!(
            block,
            ViewBlock::Table {
                id: Some(id),
                columns,
                rows
            } if id == "answers"
                && columns == &["Question", "Answer"]
                && rows[0][0] == json!("0")
                && rows[0][1] == json!("yes")
        )));
    }

    #[test]
    fn settings_are_redacted_and_attachments_have_media_projection() {
        let settings = BuiltinHumanRenderer::new("settings.get")
            .render_human(
                &ctx(),
                &RawOutput {
                    payload: Some(json!({
                        "path": "providers.openai.api_key",
                        "source": "workspace",
                        "value": "sk-live-do-not-show"
                    })),
                    ..RawOutput::default()
                },
            )
            .expect("render secret setting");
        let settings_text = serde_json::to_string(&settings).expect("serialize settings blocks");
        assert!(!settings_text.contains("sk-live-do-not-show"));
        assert!(settings_text.contains("[redacted]"));

        let attachment = agena_domain::AttachmentItem {
            kind: agena_domain::AttachmentKind::Image,
            mime: "image/png".into(),
            source: AttachmentSource::LocalPath {
                path: "/tmp/chart.png".into(),
            },
            filename: Some("chart.png".into()),
            title: None,
            size_bytes: Some(1024),
            sha256: Some("abc".into()),
            width: Some(100),
            height: Some(50),
            duration_ms: None,
            page_count: None,
        };
        let blocks = BuiltinHumanRenderer::new("fs.view_image")
            .render_human(
                &ctx(),
                &RawOutput {
                    attachments: vec![attachment],
                    ..RawOutput::default()
                },
            )
            .expect("render attachment");
        assert!(blocks.iter().any(|block| matches!(
            block,
            ViewBlock::Media {
                id: Some(id),
                artifact
            } if id == "media" && artifact.mime == "image/png" && artifact.uri == "file:///tmp/chart.png"
        )));
    }

    #[test]
    fn specific_presentations_merge_metadata_and_keep_scalar_records_visible() {
        let mut metadata = BTreeMap::new();
        metadata.insert(
            "contents".into(),
            json!([{"uri": "mcp://demo/readme", "mime_type": "text/plain", "text": "hello"}]),
        );
        let blocks = BuiltinHumanRenderer::new("mcp.resources.read")
            .render_human(
                &ctx(),
                &RawOutput {
                    payload: Some(json!({"server": "demo", "uri": "mcp://demo/readme"})),
                    metadata,
                    ..RawOutput::default()
                },
            )
            .expect("render metadata-backed MCP result");
        assert!(blocks.iter().any(|block| matches!(
            block,
            ViewBlock::Table { id: Some(id), columns, rows }
                if id == "mcp-resource-contents"
                    && columns == &["URI", "MIME", "Text"]
                    && rows[0][2] == json!("hello")
        )));

        let mut metadata = BTreeMap::new();
        metadata.insert("messages".into(), json!(["Review this document"]));
        let blocks = BuiltinHumanRenderer::new("mcp.prompts.get")
            .render_human(
                &ctx(),
                &RawOutput {
                    payload: Some(json!({"prompt": "review"})),
                    metadata,
                    ..RawOutput::default()
                },
            )
            .expect("render scalar MCP messages");
        let text = serde_json::to_string(&blocks).expect("serialize MCP blocks");
        assert!(text.contains("Review this document"));
        assert!(
            !blocks
                .iter()
                .any(|block| matches!(block, ViewBlock::Json { .. }))
        );
    }

    #[test]
    fn monitor_results_use_state_and_logs_instead_of_a_fake_command() {
        let blocks = BuiltinHumanRenderer::new("monitor.start")
            .render_human(
                &ctx(),
                &RawOutput {
                    payload: Some(json!({
                        "action": "start",
                        "monitor_id": "mon-1",
                        "status": "running",
                        "output": "watching"
                    })),
                    ..RawOutput::default()
                },
            )
            .expect("render monitor");
        assert!(blocks.iter().any(|block| matches!(
            block,
            ViewBlock::Log {
                id: Some(id),
                text,
                ..
            } if id == "monitor-output" && text == "watching"
        )));
        assert!(
            blocks
                .iter()
                .any(|block| block.block_id() == Some("monitor-meta"))
        );
        assert!(
            !blocks
                .iter()
                .any(|block| matches!(block, ViewBlock::Command { .. }))
        );
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
    fn specific_plugin_tools_use_compact_details_and_tables() {
        let cases = [
            (
                "skills.list",
                json!({
                    "tools": [{
                        "name": "review",
                        "kind": "skill",
                        "summary": "Review changes",
                        "source": "workspace",
                        "editable": true
                    }],
                    "returned": 1,
                    "total": 1,
                    "offset": 0
                }),
                vec!["skills"],
            ),
            (
                "skills.get",
                json!({
                    "name": "review",
                    "kind": "skill",
                    "source": "workspace",
                    "body": "Review the change carefully.",
                    "content_hash": "hash-1"
                }),
                vec!["skill-meta", "skill-body"],
            ),
            (
                "settings.set",
                json!({
                    "path": "providers.openai.model",
                    "layer": "workspace",
                    "changed": true,
                    "validated": true,
                    "current": "gpt-5"
                }),
                vec!["settings-edit-meta", "settings-current"],
            ),
            (
                "settings.inspect",
                json!({
                    "path": "providers.openai",
                    "global": {"defined": true},
                    "workspace": {"defined": false},
                    "layers": [{"name": "global", "active": true}]
                }),
                vec!["settings-inspect-meta", "settings-global"],
            ),
            (
                "code.search_ast",
                json!({
                    "language": "rust",
                    "pattern": "fn $NAME()",
                    "matches": [{"path": "src/lib.rs", "line": 7, "text": "fn render()"}]
                }),
                vec!["ast-search-meta", "ast-matches"],
            ),
            (
                "report.findings",
                json!({
                    "summary": "Review complete",
                    "findings": [{"severity": "high", "file": "src/lib.rs", "line": 7}],
                    "counts": {"high": 1}
                }),
                vec!["findings-summary", "findings"],
            ),
            (
                "browser_snapshot",
                json!({
                    "session_id": "session-1",
                    "snapshot": {
                        "title": "Agena docs",
                        "url": "https://example.test/docs",
                        "text": "Welcome",
                        "elements": [{"ref": "e1", "role": "link", "name": "API"}]
                    }
                }),
                vec!["browser-page", "browser-elements", "browser-text"],
            ),
            (
                "repo.status",
                json!({
                    "root": "/workspace",
                    "branch": "main",
                    "head": "abc123",
                    "dirty": true,
                    "changes": [{"path": "src/lib.rs", "kind": "modified"}]
                }),
                vec!["repository-meta", "repository-changes"],
            ),
            (
                "notebook.edit_cell",
                json!({
                    "path": "demo.ipynb",
                    "action": "replace",
                    "cell_index": 0,
                    "cell_count": 2,
                    "before_sha256": "before",
                    "after_sha256": "after"
                }),
                vec!["notebook-edit"],
            ),
            (
                "chatgpt.web_search",
                json!({
                    "provider": "openai",
                    "tool": "web_search",
                    "model": "gpt-5",
                    "request_id": "req-1",
                    "sources": [{"title": "Guide", "url": "https://example.test"}],
                    "usage": {"input_tokens": 10}
                }),
                vec!["provider-meta", "provider-sources", "provider-usage"],
            ),
        ];

        for (tool_name, payload, expected_ids) in cases {
            let blocks = BuiltinHumanRenderer::new(tool_name)
                .render_human(
                    &ctx(),
                    &RawOutput {
                        payload: Some(payload),
                        ..RawOutput::default()
                    },
                )
                .expect("render");
            let serialized = serde_json::to_string(&blocks).expect("serialize blocks");
            for expected_id in expected_ids {
                assert!(
                    serialized.contains(expected_id),
                    "{tool_name} omitted specialized block {expected_id}: {serialized}"
                );
            }
            assert!(
                !blocks
                    .iter()
                    .any(|block| matches!(block, ViewBlock::Json { .. })),
                "{tool_name} should not fall back to a JSON card: {blocks:?}"
            );
        }
    }

    #[test]
    fn provider_tools_render_operation_specific_results_without_json() {
        let cases = [
            (
                "chatgpt.file_search",
                json!({
                    "provider": "chatgpt",
                    "tool": "file_search",
                    "results": [{"file_name": "README.md", "score": 0.9}, {"file_name": "guide.md", "score": 0.8}],
                    "response_id": "resp-1"
                }),
                vec!["provider-file-search", "provider-file-results", "README.md"],
            ),
            (
                "chatgpt.tool_search",
                json!({
                    "provider": "chatgpt",
                    "tool": "tool_search",
                    "results": [{"name": "web.search", "description": "Search web"}],
                    "response_id": "resp-2"
                }),
                vec![
                    "provider-tool-search",
                    "provider-tool-results",
                    "web.search",
                ],
            ),
            (
                "gemini.google_maps",
                json!({
                    "provider": "gemini",
                    "tool": "google_maps",
                    "places": [{"name": "Cafe", "address": "1 Main St"}],
                    "response_id": "int-1"
                }),
                vec!["provider-maps", "provider-places", "Cafe"],
            ),
            (
                "gemini.retrieval",
                json!({
                    "provider": "gemini",
                    "tool": "retrieval",
                    "retrieved_count": 3,
                    "response_id": "int-2"
                }),
                vec!["provider-retrieval", "Retrieved", "3"],
            ),
            (
                "claude.web_fetch",
                json!({
                    "provider": "claude",
                    "tool": "web_fetch",
                    "url": "https://example.test",
                    "status": 200,
                    "response_id": "msg-1"
                }),
                vec!["provider-web-fetch", "HTTP status", "200"],
            ),
            (
                "claude.code_execution",
                json!({
                    "provider": "claude",
                    "tool": "code_execution",
                    "status": "completed",
                    "exit_code": 0,
                    "response_id": "msg-2"
                }),
                vec!["provider-code", "Exit code", "0"],
            ),
            (
                "chatgpt.computer",
                json!({
                    "provider": "chatgpt",
                    "tool": "computer",
                    "action": {"type": "click", "x": 20, "y": 30},
                    "page_title": "Agena docs",
                    "response_id": "resp-3"
                }),
                vec!["provider-computer", "click", "Agena docs"],
            ),
            (
                "gemini.mcp_server",
                json!({
                    "provider": "gemini",
                    "tool": "mcp_server",
                    "connected": true,
                    "status": "ready",
                    "response_id": "int-3"
                }),
                vec!["provider-mcp", "Connected", "true"],
            ),
            (
                "claude.memory",
                json!({
                    "provider": "claude",
                    "tool": "memory",
                    "operation": "save",
                    "saved": true,
                    "response_id": "msg-3"
                }),
                vec!["provider-memory", "Saved", "true"],
            ),
            (
                "claude.advisor",
                json!({
                    "provider": "claude",
                    "tool": "advisor",
                    "error": {"message": "Advisor unavailable"},
                    "response_id": "msg-4"
                }),
                vec!["provider-advisor-error", "Advisor unavailable"],
            ),
        ];

        for (tool, payload, expected) in cases {
            let blocks = BuiltinHumanRenderer::new(tool)
                .render_human(
                    &ctx(),
                    &RawOutput {
                        payload: Some(payload),
                        ..RawOutput::default()
                    },
                )
                .expect("render provider result");
            let serialized = serde_json::to_string(&blocks).expect("serialize provider blocks");
            for fragment in expected {
                assert!(
                    serialized.contains(fragment),
                    "{tool} omitted {fragment}: {serialized}"
                );
            }
            assert!(
                !blocks
                    .iter()
                    .any(|block| matches!(block, ViewBlock::Json { .. })),
                "{tool} should not fall back to JSON: {blocks:?}"
            );
        }
    }

    #[test]
    fn provider_pending_calls_show_action_and_server_columns() {
        let renderer = BuiltinHumanRenderer::new("chatgpt.mcp");
        let raw = RawOutput {
            payload: Some(json!({
                "provider": "chatgpt",
                "tool": "mcp",
                "pending_calls": [{
                    "type": "mcp_call",
                    "id": "call-1",
                    "action": {"type": "search", "query": "docs"},
                    "server_label": "docs"
                }],
                "continuation_required": true
            })),
            ..RawOutput::default()
        };
        let blocks = renderer
            .render_human(&ctx(), &raw)
            .expect("render pending call");
        assert!(blocks.iter().any(|block| {
            matches!(
                block,
                ViewBlock::Table { columns, rows, .. }
                    if columns == &["Type", "Action", "ID", "Status", "Server"]
                        && rows.iter().any(|row| row.iter().any(|value| value == "search"))
                        && rows.iter().any(|row| row.iter().any(|value| value == "docs"))
            )
        }));
    }

    #[test]
    fn lsp_diagnostics_use_location_severity_and_message_columns() {
        let renderer = BuiltinHumanRenderer::new("lsp.diagnostics");
        let raw = RawOutput {
            payload: Some(json!({
                "entries": [
                    "src/lib.rs:1 warning",
                    "src/main.rs:2:4: error: unused value",
                    "src/check.rs:8:2 [hint] consider simplifying",
                    {"path": "src/util.rs", "line": 7, "column": 3, "severity": "info", "message": "style note"}
                ]
            })),
            ..RawOutput::default()
        };
        let blocks = renderer
            .render_human(&ctx(), &raw)
            .expect("render diagnostics");
        assert!(blocks.iter().any(|block| {
            matches!(
                block,
                ViewBlock::Table {
                    id: Some(id),
                    columns,
                    rows
                } if id == "lsp-diagnostics"
                    && columns == &["Location", "Severity", "Message"]
                    && rows[0] == vec![json!("src/lib.rs:1"), json!("warning"), json!("—")]
                    && rows[1] == vec![json!("src/main.rs:2:4"), json!("error"), json!("unused value")]
                    && rows[2] == vec![json!("src/check.rs:8:2"), json!("hint"), json!("consider simplifying")]
                    && rows[3] == vec![json!("src/util.rs:7:3"), json!("info"), json!("style note")]
            )
        }));

        let typed = BuiltinHumanRenderer::new("lsp.diagnostics")
            .render_human(
                &ctx(),
                &RawOutput {
                    payload: Some(json!({"entries": ["src/lib.rs:1 warning"]})),
                    ..RawOutput::default()
                },
            )
            .expect("render typed diagnostics");
        assert!(typed.iter().any(|block| {
            matches!(
                block,
                ViewBlock::Table { id: Some(id), columns, .. }
                    if id == "diagnostics" && columns == &["Location", "Severity", "Message"]
            )
        }));
    }

    #[test]
    fn discovery_lsp_interaction_and_web_results_use_named_human_blocks() {
        let cases = [
            (
                "tools.list",
                RawOutput::text(
                    "Available tools: returned 2 of 3 starting at offset 0.\n- fs.read [filesystem] (agena.fs): Read files\n- web.search [discovery] (agena.web): Search web",
                ),
                vec!["discovery-tools", "Tool", "web.search"],
            ),
            (
                "tools.tags",
                RawOutput::text(
                    "Available tool tags: returned 2 of 2 starting at offset 0.\n- filesystem: 14\n- discovery: 37",
                ),
                vec!["discovery-tags", "filesystem", "37"],
            ),
            (
                "plugins.list",
                RawOutput::text(
                    "Available plugins: returned 1 of 1 starting at offset 0.\n- agena.web [network] (v0.1.0): Web tools · tools: browser_open, web.fetch",
                ),
                vec!["discovery-plugins", "0.1.0", "browser_open"],
            ),
            (
                "lsp.servers",
                RawOutput {
                    payload: Some(
                        json!({"servers": [{"name": "rust-analyzer", "command": "rust-analyzer", "args": [], "file_extensions": ["rs"]}]}),
                    ),
                    ..RawOutput::default()
                },
                vec!["lsp-servers", "rust-analyzer"],
            ),
            (
                "interaction.notify",
                RawOutput {
                    payload: Some(
                        json!({"title": "Build", "level": "success", "body_markdown": "**Done**"}),
                    ),
                    ..RawOutput::default()
                },
                vec!["notification-meta", "notification-body", "**Done**"],
            ),
            (
                "web.crawl",
                RawOutput {
                    payload: Some(json!({
                        "start_url": "https://example.test",
                        "engine": "spider",
                        "stored_count": 2,
                        "cached_count": 1,
                        "failure_count": 1,
                        "documents": [{"title": "Home", "url": "https://example.test", "depth": 0, "chunk_count": 3, "fetched_at": "2026-08-22T10:00:00Z"}],
                        "failures": [{"message": "Page unavailable"}]
                    })),
                    ..RawOutput::default()
                },
                vec![
                    "web-crawl-summary",
                    "web-crawl-documents",
                    "web-crawl-failures",
                ],
            ),
        ];
        for (tool, raw, expected) in cases {
            let blocks = BuiltinHumanRenderer::new(tool)
                .render_human(&ctx(), &raw)
                .expect("render");
            let serialized = serde_json::to_string(&blocks).expect("serialize blocks");
            for fragment in expected {
                assert!(
                    serialized.contains(fragment),
                    "{tool} omitted {fragment}: {serialized}"
                );
            }
            assert!(
                !blocks
                    .iter()
                    .any(|block| matches!(block, ViewBlock::Json { .. })),
                "{tool} should not render an opaque JSON block: {blocks:?}"
            );
        }
    }

    #[test]
    fn mcp_memory_plan_and_attachments_keep_high_value_facts() {
        let cases = [
            (
                "mcp.tools.call",
                RawOutput {
                    payload: Some(json!({
                        "server": "demo",
                        "tool": "search",
                        "content": [{"type": "text", "text": "3 matches"}],
                        "structured_content": {"matches": 3},
                        "mcp_meta": {"request_id": "mcp-1"}
                    })),
                    ..RawOutput::default()
                },
                vec!["mcp-content", "mcp-structured-content", "mcp-wire-meta"],
            ),
            (
                "memory.delete",
                RawOutput::text("Deleted old-notes from durable memory."),
                vec!["memory-delete", "Deleted old-notes"],
            ),
            (
                "plan.get",
                RawOutput {
                    payload: Some(json!({
                        "plan": {
                            "title": "Release",
                            "objective": "Ship safely",
                            "phase": "active",
                            "steps": [{"title": "Test", "status": "completed", "checkpoints": [{"text": "CI", "status": "passed"}]}]
                        },
                        "current_step": {"title": "Test", "status": "completed"},
                        "current_step_index": 0
                    })),
                    ..RawOutput::default()
                },
                vec![
                    "plan-meta",
                    "plan-steps",
                    "plan-checkpoints",
                    "plan-current",
                ],
            ),
        ];
        for (tool, raw, expected) in cases {
            let blocks = BuiltinHumanRenderer::new(tool)
                .render_human(&ctx(), &raw)
                .expect("render");
            let serialized = serde_json::to_string(&blocks).expect("serialize blocks");
            for fragment in expected {
                assert!(
                    serialized.contains(fragment),
                    "{tool} omitted {fragment}: {serialized}"
                );
            }
            assert!(
                !blocks
                    .iter()
                    .any(|block| matches!(block, ViewBlock::Json { .. }))
            );
        }

        let attachment = agena_domain::AttachmentItem {
            kind: agena_domain::AttachmentKind::Image,
            mime: "image/png".into(),
            source: agena_domain::AttachmentSource::LocalPath {
                path: "/tmp/chart.png".into(),
            },
            filename: Some("chart.png".into()),
            title: Some("Chart".into()),
            size_bytes: Some(1024),
            sha256: Some("abc".into()),
            width: Some(100),
            height: Some(50),
            duration_ms: None,
            page_count: None,
        };
        let blocks = BuiltinHumanRenderer::new("chatgpt.image_generation")
            .render_human(
                &ctx(),
                &RawOutput {
                    attachments: vec![attachment],
                    ..RawOutput::default()
                },
            )
            .expect("render attachment");
        let serialized = serde_json::to_string(&blocks).expect("serialize attachment blocks");
        assert!(serialized.contains("attachments"));
        assert!(serialized.contains("chart.png"));
        assert!(serialized.contains("image/png"));
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
            }] if id == "plugin-empty-empty"
                && text == "### Empty\nNo result returned."
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
    fn tool_specific_human_summaries_match_collapsed_title_facts() {
        let cases = [
            (
                "fs.write",
                json!({"kind": "updated", "bytes": 4}),
                "updated · 4 B",
            ),
            ("web.fetch", json!({"status": 200}), "HTTP 200"),
            (
                "settings.validate",
                json!({"valid": true, "warnings": [{"path": "model"}]}),
                "valid · 1 warning",
            ),
        ];

        for (tool_name, payload, expected) in cases {
            let raw = RawOutput {
                payload: Some(payload),
                ..RawOutput::default()
            };
            assert_eq!(
                BuiltinHumanRenderer::human_summary_for_tool(tool_name, &raw),
                expected,
                "{tool_name} summary should use the same compact fact as its title"
            );
        }
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
                .all(|b| !matches!(b, ViewBlock::Command { .. }))
        );
        assert!(blocks.iter().any(|b| b.block_id() == Some("monitor-meta")));
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
