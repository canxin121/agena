use std::collections::BTreeMap;

use agena_tool::{CronJobSummary, CronRunSummary};
use serde::{Deserialize, Serialize};
use strum::Display;

use crate::part::{
    ApplyPatchToolInput, AskUserToolInput, AttachmentKind, CronCreateToolInput,
    CronDeleteToolInput, CronHistoryToolInput, CronJobControlToolInput, CronListToolInput,
    CronUpdateToolInput, GlobToolInput, GrepToolInput, LspDefinitionToolInput,
    LspDiagnosticsToolInput, LspHoverToolInput, LspReferencesToolInput, MonitorToolInput,
    ReadToolInput, ShellToolInput, ToolSearchToolInput, WebFetchToolInput, WebSearchToolInput,
};
use agena_domain::{
    FileChangeRecord, StructuredObject, ToolInvocation, ToolOutput, WebSearchResult,
};
use agena_domain::{ProcessEvent, ProcessShell, ProcessStatus, ProcessSummary};
use agena_plugin_host::registry::RegisteredTool;

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Display)]
#[serde(tag = "tool", rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
/// Input payload of a tool execution.
pub enum ToolPayloadInput {
    Shell(ShellToolInput),
    Monitor(MonitorToolInput),
    Read(ReadToolInput),
    ApplyPatch(ApplyPatchToolInput),
    Glob(GlobToolInput),
    Grep(GrepToolInput),
    ToolSearch(ToolSearchToolInput),
    #[serde(rename = "ask_user")]
    AskUser(AskUserToolInput),
    WebFetch(WebFetchToolInput),
    WebSearch(WebSearchToolInput),
    CronCreate(CronCreateToolInput),
    CronList(CronListToolInput),
    CronDelete(CronDeleteToolInput),
    CronUpdate(CronUpdateToolInput),
    CronPause(CronJobControlToolInput),
    CronResume(CronJobControlToolInput),
    CronHistory(CronHistoryToolInput),
    LspDefinition(LspDefinitionToolInput),
    LspReferences(LspReferencesToolInput),
    LspHover(LspHoverToolInput),
    LspDiagnostics(LspDiagnosticsToolInput),
}

impl ToolPayloadInput {
    /// Stable tool name as serialized in the wire format.
    pub fn tool_name(&self) -> &'static str {
        match self {
            Self::Shell(_) => "shell",
            Self::Monitor(_) => "monitor",
            Self::Read(_) => "read",
            Self::ApplyPatch(_) => "apply_patch",
            Self::Glob(_) => "glob",
            Self::Grep(_) => "grep",
            Self::ToolSearch(_) => "tool_search",
            Self::AskUser(_) => "ask_user",
            Self::WebFetch(_) => "web_fetch",
            Self::WebSearch(_) => "web_search",
            Self::CronCreate(_) => "cron_create",
            Self::CronList(_) => "cron_list",
            Self::CronDelete(_) => "cron_delete",
            Self::CronUpdate(_) => "cron_update",
            Self::CronPause(_) => "cron_pause",
            Self::CronResume(_) => "cron_resume",
            Self::CronHistory(_) => "cron_history",
            Self::LspDefinition(_) => "lsp_definition",
            Self::LspReferences(_) => "lsp_references",
            Self::LspHover(_) => "lsp_hover",
            Self::LspDiagnostics(_) => "lsp_diagnostics",
        }
    }

    /// Convert into a generic [`ToolInvocation`] carrying this tool's name
    /// and a `StructuredObject` payload.
    pub fn into_invocation(self) -> ToolInvocation {
        let tool_name = self.tool_name();
        let value = match serde_json::to_value(&self) {
            Ok(value) => value,
            Err(error) => {
                tracing::error!(
                    tool_name,
                    diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                        "serialize typed tool input payload",
                        &error,
                    ),
                    "typed tool input payload could not be serialized"
                );
                serde_json::Value::Object(Default::default())
            }
        };
        let mut object = match value {
            serde_json::Value::Object(map) => map,
            _ => serde_json::Map::new(),
        };
        object.remove("tool");
        let name = invocation_name_for_payload_tool(tool_name, &mut object)
            .unwrap_or_else(|| tool_name.to_string());
        let payload = match StructuredObject::try_from(serde_json::Value::Object(object)) {
            Ok(payload) => payload,
            Err(error) => {
                tracing::error!(
                    tool_name,
                    error = %error,
                    "serialized tool input object could not be converted to StructuredObject"
                );
                StructuredObject::default()
            }
        };
        ToolInvocation::new(name, payload)
    }

    /// Reconstruct a `ToolPayloadInput` from a [`ToolInvocation`], or `None`
    /// if the invocation does not use one of the typed payload tool names.
    /// Only the registered compact, canonical, and plugin-wire spellings are
    /// accepted; an unknown invocation name is not coerced into a payload
    /// variant.
    pub fn from_invocation(invocation: &ToolInvocation) -> Option<Self> {
        let value: serde_json::Value = invocation.input.clone().into();
        let mut object = match value {
            serde_json::Value::Object(map) => map,
            serde_json::Value::Null => serde_json::Map::new(),
            _ => return None,
        };
        let payload_name = payload_name_for_invocation(invocation.name.as_str(), &mut object)?;
        object.insert("tool".to_string(), serde_json::Value::String(payload_name));
        match serde_json::from_value(serde_json::Value::Object(object)) {
            Ok(payload) => Some(payload),
            Err(error) => {
                tracing::warn!(
                    tool_name = %invocation.name,
                    diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                        "decode a typed tool invocation payload",
                        &error,
                    ),
                    "typed tool invocation payload projection failed"
                );
                None
            }
        }
    }

    /// Decode the payload for a bundled tool whose registered handler is a
    /// definition-only adapter around [`crate::tool::orchestrator`].
    ///
    /// Dispatch is deliberately keyed by the resolved registry identity, not
    /// by spelling conventions in `ToolInvocation::name`. The provider uses
    /// compact names such as `shell.exec`, while the registry owns the stable
    /// identity `agena.shell.exec`; guessing from the terminal name (`exec`)
    /// previously sent valid built-ins back into their non-executable plugin
    /// adapters.
    pub(crate) fn from_executor_backed_invocation(
        registered: &RegisteredTool,
        invocation: &ToolInvocation,
    ) -> Option<Result<Self, serde_json::Error>> {
        let (payload_name, action) = match (
            registered.namespace(),
            registered.plugin_name(),
            registered.tool_name(),
        ) {
            ("agena", "fs", "read") => ("read", None),
            ("agena", "fs", "glob") => ("glob", None),
            ("agena", "fs", "grep") => ("grep", None),
            ("agena", "fs", "apply_patch") => ("apply_patch", None),
            (
                "agena",
                "shell",
                action @ ("exec" | "spawn" | "open" | "list" | "logs" | "read" | "stop" | "write"
                | "resize" | "signal"),
            ) => ("shell", Some(action)),
            ("agena", "monitor", action @ ("start" | "stop")) => ("monitor", Some(action)),
            ("agena", "cron", "create") => ("cron_create", None),
            ("agena", "cron", "list") => ("cron_list", None),
            ("agena", "cron", "delete") => ("cron_delete", None),
            ("agena", "cron", "update") => ("cron_update", None),
            ("agena", "cron", "pause") => ("cron_pause", None),
            ("agena", "cron", "resume") => ("cron_resume", None),
            ("agena", "cron", "history") => ("cron_history", None),
            ("agena", "lsp", "definition") => ("lsp_definition", None),
            ("agena", "lsp", "references") => ("lsp_references", None),
            ("agena", "lsp", "hover") => ("lsp_hover", None),
            ("agena", "lsp", "diagnostics") => ("lsp_diagnostics", None),
            _ => return None,
        };

        let value: serde_json::Value = invocation.input.clone().into();
        let mut object = match value {
            serde_json::Value::Object(object) => object,
            serde_json::Value::Null => serde_json::Map::new(),
            _ => return None,
        };
        if payload_name == "shell" {
            // Native adapters must honor the same normalization/constraints
            // as their live public contracts, before hooks or process launch.
            let value = serde_json::Value::Object(object.clone());
            let parsed = match action? {
                "exec" => crate::part::ShellLaunchInput::parse_input(value).map(|input| {
                    ShellToolInput::Exec {
                        shell: input.shell,
                        command: Box::new(input.command),
                    }
                }),
                "spawn" => crate::part::ShellLaunchInput::parse_input(value).map(|input| {
                    ShellToolInput::Spawn {
                        shell: input.shell,
                        command: Box::new(input.command),
                    }
                }),
                "watch" => crate::part::ShellWatchInput::parse_input(value).map(|input| {
                    ShellToolInput::Watch {
                        input: Box::new(input),
                    }
                }),
                "open" => crate::part::ShellOpenInput::parse_input(value).map(|input| {
                    ShellToolInput::Open {
                        input: Box::new(input),
                    }
                }),
                "read" => crate::part::ShellReadInput::parse_input(value)
                    .map(|input| ShellToolInput::Read { input }),
                "write" => crate::part::ShellWriteInput::parse_input(value)
                    .map(|input| ShellToolInput::Write { input }),
                action => {
                    object.insert(
                        "action".to_owned(),
                        serde_json::Value::String(action.to_owned()),
                    );
                    ShellToolInput::parse_input(serde_json::Value::Object(object))
                }
            };
            return Some(
                parsed
                    .map(Self::Shell)
                    .map_err(|error| serde::de::Error::custom(error.to_string())),
            );
        }
        if payload_name == "monitor" {
            object.insert(
                "action".to_owned(),
                serde_json::Value::String(action?.to_owned()),
            );
            return Some(
                MonitorToolInput::parse_input(serde_json::Value::Object(object))
                    .map(Self::Monitor)
                    .map_err(|error| serde::de::Error::custom(error.to_string())),
            );
        }
        object.insert(
            "tool".to_string(),
            serde_json::Value::String(payload_name.to_string()),
        );
        if let Some(action) = action {
            object.insert(
                "action".to_string(),
                serde_json::Value::String(action.to_string()),
            );
        }
        Some(serde_json::from_value(serde_json::Value::Object(object)))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// Output of reading an attachment.
pub struct ReadAttachmentOutput {
    pub path: String,
    pub kind: AttachmentKind,
    pub mime: String,
    pub size_bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_count: Option<u32>,
}

/// Structured grep records keep paths (including colons) separate from text.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GrepRecord {
    Line {
        path: String,
        line: u64,
        text: String,
        matched: bool,
        #[serde(default, skip_serializing_if = "is_false")]
        text_truncated: bool,
    },
    File {
        path: String,
    },
    Count {
        path: String,
        /// Number of matching lines, not the number of regex occurrences.
        count: u64,
        /// False means this is only a lower bound from a partial file scan.
        complete: bool,
    },
}

impl GrepRecord {
    pub(super) fn display(&self) -> String {
        match self {
            Self::Line {
                path,
                line,
                text,
                matched,
                text_truncated,
            } => format!(
                "{path}{separator}{line}{separator} {text}{suffix}",
                separator = if *matched { ':' } else { '-' },
                suffix = if *text_truncated {
                    "… [line shortened]"
                } else {
                    ""
                },
            ),
            Self::File { path } => path.clone(),
            Self::Count {
                path,
                count,
                complete,
            } => format!(
                "{path}: {count}{}",
                if *complete { "" } else { "+ (partial)" }
            ),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "tool", rename_all = "snake_case")]
/// Output payload of a tool execution.
pub enum ToolPayloadOutput {
    Read {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        read_info: Option<serde_json::Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        preview: Option<String>,
        #[serde(default, skip_serializing_if = "is_false")]
        truncated: bool,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        loaded_paths: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        attachment: Option<ReadAttachmentOutput>,
    },
    ApplyPatch {
        operation_id: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        changes: Vec<FileChangeRecord>,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        diff: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        progress: Vec<String>,
    },
    Glob {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        count: Option<u32>,
        /// Matched paths, in sorted order. Kept in the payload so discovery
        /// results remain useful to the model as well as the transcript UI.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        paths: Vec<String>,
        #[serde(default, skip_serializing_if = "is_false")]
        truncated: bool,
    },
    Grep {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        matches: Option<u32>,
        /// Matching `path:line: text` records.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        results: Vec<String>,
        #[serde(default)]
        mode: crate::part::GrepMode,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        records: Vec<GrepRecord>,
        /// True only if the selected scope was fully searched. In files mode,
        /// a file needs only one match to establish membership.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        scan_complete: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        note: Option<String>,
        #[serde(default, skip_serializing_if = "is_false")]
        truncated: bool,
    },
    ToolSearch {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        results: Vec<String>,
    },
    #[serde(rename = "ask_user")]
    AskUser {
        #[serde(
            default,
            skip_serializing_if = "agena_domain::user_input_answers_is_empty"
        )]
        answers: BTreeMap<String, Vec<String>>,
        #[serde(default, skip_serializing_if = "is_false")]
        timed_out: bool,
    },
    Shell {
        action: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        output_resource: Option<agena_domain::ContentRef>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        terminal: Option<agena_domain::TerminalScreen>,
        #[serde(default)]
        dropped_bytes: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        shell: Option<ProcessShell>,
        #[serde(default)]
        background: bool,
        /// Ready for the current watch policy; this is independent of process exit.
        #[serde(default)]
        ready: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        process_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        status: Option<ProcessStatus>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        output: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        events: Vec<ProcessEvent>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        processes: Vec<ProcessSummary>,
        #[serde(default)]
        last_seq: u64,
        /// Bytes already consumed in the next event; pair with last_seq when replaying.
        #[serde(default)]
        next_event_offset: u32,
        #[serde(default)]
        has_more: bool,
        #[serde(default)]
        dropped_lines: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        exit_code: Option<i32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        completion_reason: Option<String>,
    },
    /// `monitor.start` / `monitor.stop` output. The monitor keeps running after
    /// `start` returns; its tool part stays `InProgress` until the monitor
    /// settles, with every event projected as a `system_notification` part.
    Monitor {
        action: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        monitor_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        status: Option<ProcessStatus>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        output: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        processes: Vec<ProcessSummary>,
        #[serde(default)]
        last_seq: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        exit_code: Option<i32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        completion_reason: Option<String>,
    },
    WebFetch {
        url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        markdown: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        summary: Option<String>,
        #[serde(default)]
        truncated: bool,
        #[serde(default)]
        cached: bool,
        status: u16,
    },
    WebSearch {
        query: String,
        backend: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        results: Vec<WebSearchResult>,
    },
    CronCreate {
        id: String,
        next_fire_at: Option<String>,
    },
    CronList {
        jobs: Vec<CronJobSummary>,
    },
    CronDelete {
        id: String,
        removed: bool,
    },
    CronUpdate {
        job: CronJobSummary,
    },
    CronPause {
        job: CronJobSummary,
    },
    CronResume {
        job: CronJobSummary,
    },
    CronHistory {
        entries: Vec<CronRunSummary>,
    },
    LspDefinition {
        locations: Vec<String>,
    },
    LspReferences {
        locations: Vec<String>,
    },
    LspHover {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        contents: Option<String>,
    },
    LspDiagnostics {
        entries: Vec<String>,
        #[serde(default)]
        state: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        document_version: Option<i32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reported_version: Option<i32>,
    },
}

impl ToolPayloadOutput {
    /// Resolve the `tool` discriminant tag for a given invocation/registry
    /// tool name, if one maps to a typed output variant. `None` means the
    /// payload is opaque and renders as a fallback JSON card.
    pub fn payload_name_for(tool_name: &str) -> Option<String> {
        payload_name_for_output_tool(tool_name)
    }

    /// Convert into the plugin-neutral [`ToolOutput`] payload used by
    /// persisted message parts. The tool name lives on [`ToolInvocation`],
    /// so the payload stores only the output fields.
    pub fn into_tool_output(self) -> ToolOutput {
        let value = match serde_json::to_value(&self) {
            Ok(value) => value,
            Err(error) => {
                tracing::error!(
                    diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                        "serialize typed tool output payload",
                        &error,
                    ),
                    "typed tool output payload could not be serialized"
                );
                serde_json::Value::Object(Default::default())
            }
        };
        let mut object = match value {
            serde_json::Value::Object(map) => map,
            _ => serde_json::Map::new(),
        };
        object.remove("tool");
        let payload = match StructuredObject::try_from(serde_json::Value::Object(object)) {
            Ok(payload) => payload,
            Err(error) => {
                tracing::error!(
                    error = %error,
                    "serialized tool output object could not be converted to StructuredObject"
                );
                StructuredObject::default()
            }
        };
        ToolOutput {
            payload,
            managed_outputs: Vec::new(),
            truncated: false,
        }
    }

    /// Reverse of [`into_tool_output`]: try to decode a generic output
    /// payload for the named tool back into the execution-layer enum.
    pub fn from_tool_output(tool_name: &str, output: &ToolOutput) -> Option<Self> {
        let value: serde_json::Value = output.payload.clone().into();
        let mut object = match value {
            serde_json::Value::Object(map) => map,
            serde_json::Value::Null => serde_json::Map::new(),
            _ => return None,
        };
        let payload_name = payload_name_for_output_tool(tool_name)?;
        if payload_name == "web_search" {
            // The web plugin reports its selected engine and descriptions;
            // the transcript's typed output uses backend and snippet. Adapt
            // that boundary without changing the original plugin payload.
            if !object.contains_key("backend")
                && let Some(engine) = object.get("engine").cloned()
            {
                object.insert("backend".into(), engine);
            }
            if let Some(serde_json::Value::Array(results)) = object.get_mut("results") {
                for result in results {
                    if let Some(result) = result.as_object_mut()
                        && !result.contains_key("snippet")
                        && let Some(description) = result.get("description").cloned()
                    {
                        result.insert("snippet".into(), description);
                    }
                }
            }
        }
        object.insert("tool".to_string(), serde_json::Value::String(payload_name));
        match serde_json::from_value(serde_json::Value::Object(object)) {
            Ok(payload) => Some(payload),
            Err(error) => {
                tracing::warn!(
                    tool_name,
                    diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                        "decode a typed tool output payload",
                        &error,
                    ),
                    "typed tool output payload projection failed"
                );
                None
            }
        }
    }
}

/// One-to-one payload variant mappings for every typed tool family:
/// `(serialized variant tag, compact name, canonical name, plugin-format name)`.
///
/// The provider catalog uses compact names such as `fs.read` and `cron.create`,
/// workflows use canonical names such as `agena.fs.read`, and the plugin wire
/// format uses names such as `agena_fs_read` / `agena_web__fetch`. Every
/// spelling must resolve to the same payload variant tag; a missed spelling
/// makes the renderer fall back to dumping the whole payload as a raw JSON
/// card. Retired tools have output variants only.
const PAYLOAD_OUTPUT_TOOLS: &[(&str, &str, &str, &str)] = &[
    ("read", "fs.read", "agena.fs.read", "agena_fs_read"),
    (
        "read",
        "fs.read_media",
        "agena.fs.read_media",
        "agena_fs_read_media",
    ),
    ("glob", "fs.glob", "agena.fs.glob", "agena_fs_glob"),
    ("grep", "fs.grep", "agena.fs.grep", "agena_fs_grep"),
    (
        "apply_patch",
        "fs.apply_patch",
        "agena.fs.apply_patch",
        "agena_fs_apply_patch",
    ),
    (
        "ask_user",
        "interaction.ask",
        "agena.interaction.ask",
        "agena_interaction_ask",
    ),
    (
        "web_fetch",
        "web.fetch",
        "agena.web.fetch",
        "agena_web__fetch",
    ),
    (
        "web_search",
        "web.search",
        "agena.web.search",
        "agena_web__search",
    ),
    (
        "cron_create",
        "cron.create",
        "agena.cron.create",
        "agena_cron_create",
    ),
    (
        "cron_list",
        "cron.list",
        "agena.cron.list",
        "agena_cron_list",
    ),
    (
        "cron_delete",
        "cron.delete",
        "agena.cron.delete",
        "agena_cron_delete",
    ),
    (
        "cron_update",
        "cron.update",
        "agena.cron.update",
        "agena_cron_update",
    ),
    (
        "cron_pause",
        "cron.pause",
        "agena.cron.pause",
        "agena_cron_pause",
    ),
    (
        "cron_resume",
        "cron.resume",
        "agena.cron.resume",
        "agena_cron_resume",
    ),
    (
        "cron_history",
        "cron.history",
        "agena.cron.history",
        "agena_cron_history",
    ),
    (
        "lsp_definition",
        "lsp.definition",
        "agena.lsp.definition",
        "agena_lsp_definition",
    ),
    (
        "lsp_references",
        "lsp.references",
        "agena.lsp.references",
        "agena_lsp_references",
    ),
    (
        "lsp_hover",
        "lsp.hover",
        "agena.lsp.hover",
        "agena_lsp_hover",
    ),
    (
        "lsp_diagnostics",
        "lsp.diagnostics",
        "agena.lsp.diagnostics",
        "agena_lsp_diagnostics",
    ),
];

fn payload_name_for_table(tool_name: &str) -> Option<String> {
    PAYLOAD_OUTPUT_TOOLS
        .iter()
        .find(|(_, compact, canonical, plugin)| {
            *compact == tool_name || *canonical == tool_name || *plugin == tool_name
        })
        .map(|(tag, _, _, _)| tag.to_string())
}

const DIRECT_TOOL_MAPPINGS: &[(&str, &str, &str)] = &[
    ("read", "agena.fs", "read"),
    ("glob", "agena.fs", "glob"),
    ("grep", "agena.fs", "grep"),
    ("apply_patch", "agena.fs", "apply_patch"),
    ("task", "agena.agent", "run"),
    ("ask_user", "agena.interaction", "ask"),
    ("lsp_definition", "agena.lsp", "definition"),
    ("lsp_references", "agena.lsp", "references"),
    ("lsp_hover", "agena.lsp", "hover"),
    ("lsp_diagnostics", "agena.lsp", "diagnostics"),
];

fn canonical_registry_tool_name(plugin: &str, tool: &str) -> String {
    let plugin_key = plugin.parse().expect("direct tool mapping plugin key");
    agena_plugin_host::ToolKey::new(plugin_key, tool.to_string())
        .expect("direct tool mapping tool key")
        .to_string()
}

fn direct_mapping_for_tool(tool: &str) -> Option<(&'static str, &'static str)> {
    DIRECT_TOOL_MAPPINGS
        .iter()
        .find(|(name, _, _)| *name == tool)
        .map(|(_, plugin, tool)| (*plugin, *tool))
}

fn invocation_name_for_payload_tool(
    tool: &str,
    input: &mut serde_json::Map<String, serde_json::Value>,
) -> Option<String> {
    Some(match tool {
        "web_fetch" => canonical_registry_tool_name("agena.web", "fetch"),
        "web_search" => canonical_registry_tool_name("agena.web", "search"),
        "shell" => {
            let action = input
                .remove("action")
                .and_then(|value| value.as_str().map(str::to_string))
                .unwrap_or_else(|| "list".to_string());
            canonical_registry_tool_name("agena.shell", action.as_str())
        }
        "monitor" => {
            let action = input
                .remove("action")
                .and_then(|value| value.as_str().map(str::to_string))
                .unwrap_or_else(|| "start".to_string());
            canonical_registry_tool_name("agena.monitor", action.as_str())
        }
        "cron_create" => canonical_registry_tool_name("agena.cron", "create"),
        "cron_list" => canonical_registry_tool_name("agena.cron", "list"),
        "cron_delete" => canonical_registry_tool_name("agena.cron", "delete"),
        "cron_update" => canonical_registry_tool_name("agena.cron", "update"),
        "cron_pause" => canonical_registry_tool_name("agena.cron", "pause"),
        "cron_resume" => canonical_registry_tool_name("agena.cron", "resume"),
        "cron_history" => canonical_registry_tool_name("agena.cron", "history"),
        _ => {
            let (plugin, tool) = direct_mapping_for_tool(tool)?;
            canonical_registry_tool_name(plugin, tool)
        }
    })
}

fn payload_name_for_invocation(
    invocation_name: &str,
    input: &mut serde_json::Map<String, serde_json::Value>,
) -> Option<String> {
    if let Some(action) = shell_action_for_name(invocation_name) {
        input.insert(
            "action".to_owned(),
            serde_json::Value::String(action.to_owned()),
        );
        return Some("shell".to_owned());
    }
    if invocation_name == "shell" {
        return Some("shell".to_owned());
    }
    match invocation_name {
        "agena_web__fetch" | "web.fetch" | "web_fetch" => return Some("web_fetch".to_string()),
        "agena_web__search" | "web.search" | "web_search" => {
            return Some("web_search".to_string());
        }
        "agena_fs_read" | "agena.fs.read" => {
            return Some("read".to_string());
        }
        "agena_fs_glob" | "agena.fs.glob" => {
            return Some("glob".to_string());
        }
        "agena_fs_grep" | "agena.fs.grep" => {
            return Some("grep".to_string());
        }
        "agena_fs_apply_patch" | "agena.fs.apply_patch" => {
            return Some("apply_patch".to_string());
        }
        "monitor.start" | "agena_monitor_start" | "agena.monitor.start" => {
            input.insert(
                "action".to_string(),
                serde_json::Value::String("start".to_string()),
            );
            return Some("monitor".to_string());
        }
        "monitor.stop" | "agena_monitor_stop" | "agena.monitor.stop" => {
            input.insert(
                "action".to_string(),
                serde_json::Value::String("stop".to_string()),
            );
            return Some("monitor".to_string());
        }
        _ => {}
    }
    // Every other typed payload reuses the explicit current-name mapping.
    payload_name_for_output_tool(invocation_name)
}

/// Registered shell identities in their current protocol spellings.
fn shell_action_for_name(name: &str) -> Option<&'static str> {
    [
        "exec", "spawn", "watch", "open", "list", "logs", "read", "write", "stop", "resize",
        "signal",
    ]
    .into_iter()
    .find(|action| {
        name == format!("shell.{action}")
            || name == format!("agena.shell.{action}")
            || name == format!("agena_shell_{action}")
    })
}

fn payload_name_for_output_tool(tool_name: &str) -> Option<String> {
    if tool_name == "web_search" {
        return Some("web_search".to_owned());
    }
    // Shell/process tools share one `shell` payload variant; the subcommand
    // (`run`/`list`/`logs`/`stop`) lives in `action`, not in the variant tag.
    // Monitor tools share one `monitor` payload variant; the subcommand
    // (`start`/`stop`) lives in `action`, not in the variant tag.
    if matches!(
        tool_name,
        "monitor"
            | "monitor.start"
            | "monitor.stop"
            | "agena.monitor.start"
            | "agena.monitor.stop"
            | "agena_monitor_start"
            | "agena_monitor_stop"
    ) {
        return Some("monitor".to_string());
    }
    if tool_name == "shell" || shell_action_for_name(tool_name).is_some() {
        return Some("shell".to_string());
    }
    payload_name_for_table(tool_name)
}

#[cfg(test)]
mod output_tests {
    use super::*;

    #[test]
    fn web_plugin_results_keep_their_engine_and_descriptions_in_typed_projection() {
        for engine in ["auto", "duckduckgo", "brave", "searxng"] {
            let output = ToolOutput {
                payload: StructuredObject::try_from(serde_json::json!({
                    "query": "fixture query",
                    "engine": engine,
                    "attempted_engines": ["duckduckgo", "bing"],
                    "partial": true,
                    "engine_errors": ["controlled unavailable engine"],
                    "results": [{
                        "title": "Fixture result", "url": "https://example.com/",
                        "description": "Readable search snippet", "source": "example.com",
                        "engine": "duckduckgo", "engines": ["duckduckgo", "bing"]
                    }]
                }))
                .unwrap(),
                managed_outputs: Vec::new(),
                truncated: false,
            };
            let original = output.clone();
            for name in [
                "web.search",
                "agena.web.search",
                "agena_web__search",
                "web_search",
            ] {
                let Some(ToolPayloadOutput::WebSearch {
                    query,
                    backend,
                    results,
                }) = ToolPayloadOutput::from_tool_output(name, &output)
                else {
                    panic!("plugin output did not project for {name}");
                };
                assert_eq!(query, "fixture query");
                assert_eq!(backend, engine);
                assert_eq!(results.len(), 1);
                assert_eq!(
                    results[0].snippet.as_deref(),
                    Some("Readable search snippet")
                );
            }
            assert_eq!(output.payload, original.payload);
        }
    }

    #[test]
    fn canonical_search_output_keeps_its_backend_and_snippet() {
        let output = ToolOutput {
            payload: StructuredObject::try_from(serde_json::json!({
                "query": "fixture query", "backend": "canonical", "engine": "other",
                "results": [{"title":"Fixture", "url":"https://example.com/",
                    "snippet":"Canonical snippet", "description":"Other description"}]
            }))
            .unwrap(),
            managed_outputs: Vec::new(),
            truncated: false,
        };
        let Some(ToolPayloadOutput::WebSearch {
            backend, results, ..
        }) = ToolPayloadOutput::from_tool_output("web.search", &output)
        else {
            panic!("canonical output did not project")
        };
        assert_eq!(backend, "canonical");
        assert_eq!(results[0].snippet.as_deref(), Some("Canonical snippet"));
    }
}
