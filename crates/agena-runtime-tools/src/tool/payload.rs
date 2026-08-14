use std::collections::BTreeMap;

use agena_tool::{CronJobSummary, CronRunSummary};
use serde::{Deserialize, Serialize};
use strum::Display;

use crate::part::{
    ApplyPatchToolInput, AskUserToolInput, AttachmentKind, CronCreateToolInput,
    CronDeleteToolInput, CronHistoryToolInput, CronJobControlToolInput, CronListToolInput,
    CronUpdateToolInput, EnterSnapshotToolInput, ExitSnapshotToolInput, GlobToolInput,
    GrepToolInput, LspDefinitionToolInput, LspDiagnosticsToolInput, LspHoverToolInput,
    LspReferencesToolInput, MonitorToolInput, ReadToolInput, ShellToolInput, ToolSearchToolInput,
    WebFetchToolInput, WebSearchToolInput,
};
use agena_domain::{
    FileChangeRecord, StructuredObject, ToolInvocation, ToolOutput, WebSearchResult,
};
use agena_domain::{ProcessEvent, ProcessShell, ProcessStatus, ProcessSummary};
use agena_plugin_host::registry::RegisteredTool;

fn is_false(value: &bool) -> bool {
    !*value
}

fn is_zero_u64(value: &u64) -> bool {
    *value == 0
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Display)]
#[serde(tag = "tool", rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
/// Input payload of a tool execution.
pub enum ToolPayloadInput {
    #[serde(alias = "process")]
    Shell(ShellToolInput),
    Monitor(MonitorToolInput),
    Read(ReadToolInput),
    ApplyPatch(ApplyPatchToolInput),
    Glob(GlobToolInput),
    Grep(GrepToolInput),
    Task(crate::part::TaskToolInput),
    ToolSearch(ToolSearchToolInput),
    #[serde(rename = "ask_user")]
    AskUser(AskUserToolInput),
    WebFetch(WebFetchToolInput),
    WebSearch(WebSearchToolInput),
    EnterSnapshot(EnterSnapshotToolInput),
    ExitSnapshot(ExitSnapshotToolInput),
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
            Self::Task(_) => "task",
            Self::ToolSearch(_) => "tool_search",
            Self::AskUser(_) => "ask_user",
            Self::WebFetch(_) => "web_fetch",
            Self::WebSearch(_) => "web_search",
            Self::EnterSnapshot(_) => "enter_snapshot",
            Self::ExitSnapshot(_) => "exit_snapshot",
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
        let value = serde_json::to_value(&self).unwrap_or(serde_json::Value::Null);
        let mut object = match value {
            serde_json::Value::Object(map) => map,
            _ => serde_json::Map::new(),
        };
        object.remove("tool");
        let name = invocation_name_for_payload_tool(tool_name, &mut object)
            .unwrap_or_else(|| tool_name.to_string());
        let payload =
            StructuredObject::try_from(serde_json::Value::Object(object)).unwrap_or_default();
        ToolInvocation::new(name, payload)
    }

    /// Reconstruct a `ToolPayloadInput` from a [`ToolInvocation`], or `None`
    /// if the invocation does not use one of the typed payload tool names.
    /// Namespaced forms like `plugin__tool` and dotted forms like `fs.read`
    /// are matched by their terminal segment so user-installed replacements
    /// inherit the same semantics.
    pub fn from_invocation(invocation: &ToolInvocation) -> Option<Self> {
        let value: serde_json::Value = invocation.input.clone().into();
        let mut object = match value {
            serde_json::Value::Object(map) => map,
            serde_json::Value::Null => serde_json::Map::new(),
            _ => return None,
        };
        let payload_name = payload_name_for_invocation(invocation.name.as_str(), &mut object)
            .unwrap_or_else(|| canonical_tool_payload_name(invocation.name.as_str()).to_string());
        object.insert("tool".to_string(), serde_json::Value::String(payload_name));
        serde_json::from_value(serde_json::Value::Object(object)).ok()
    }

    /// Decode the payload for a bundled tool whose registered handler is a
    /// definition-only adapter around [`crate::tool::orchestrator`].
    ///
    /// Dispatch is deliberately keyed by the resolved registry identity, not
    /// by spelling conventions in `ToolInvocation::name`. The provider uses
    /// compact names such as `shell.run`, while the registry owns the stable
    /// identity `agena.shell.run`; guessing from the terminal name (`run`)
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
            ("agena", "shell", action @ ("run" | "list" | "logs" | "stop")) => {
                ("shell", Some(action))
            }
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "tool", rename_all = "snake_case")]
/// Output payload of a tool execution.
pub enum ToolPayloadOutput {
    Read {
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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        before_hash: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        after_hash: Option<String>,
        inverse_patch: String,
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
        #[serde(default, skip_serializing_if = "is_false")]
        truncated: bool,
    },
    Task {
        task_id: String,
        session_id: i64,
        parent_session_id: i64,
        access: String,
        status: String,
        #[serde(default, skip_serializing_if = "is_false")]
        resumed: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        final_text: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model_feedback: Option<agena_failure::ModelFeedback>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model_provider_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model_adapter_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model_id: Option<String>,
        #[serde(default, skip_serializing_if = "is_zero_u64")]
        input_tokens: u64,
        #[serde(default, skip_serializing_if = "is_zero_u64")]
        output_tokens: u64,
        #[serde(default, skip_serializing_if = "is_zero_u64")]
        reasoning_tokens: u64,
        #[serde(default, skip_serializing_if = "is_zero_u64")]
        cache_write_tokens: u64,
        #[serde(default, skip_serializing_if = "is_zero_u64")]
        cache_read_tokens: u64,
        #[serde(default, skip_serializing_if = "is_zero_u64")]
        total_cost_microusd: u64,
    },
    ToolSearch {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        results: Vec<String>,
    },
    #[serde(rename = "ask_user")]
    AskUser {
        #[serde(
            default,
            deserialize_with = "agena_domain::deserialize_user_input_answers",
            skip_serializing_if = "agena_domain::user_input_answers_is_empty"
        )]
        answers: BTreeMap<String, Vec<String>>,
        #[serde(default, skip_serializing_if = "is_false")]
        timed_out: bool,
    },
    #[serde(alias = "process")]
    Shell {
        action: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        shell: Option<ProcessShell>,
        #[serde(default)]
        background: bool,
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
        #[serde(default)]
        has_more: bool,
        #[serde(default)]
        dropped_lines: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        exit_code: Option<i32>,
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
    EnterSnapshot {
        path: String,
        branch: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        backend: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        note: Option<String>,
    },
    ExitSnapshot {
        action: String,
        path: String,
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
    },
}

impl ToolPayloadOutput {
    /// Resolve the `tool` discriminant tag for a given invocation/registry
    /// tool name, if one maps to a typed output variant. `None` means the
    /// payload is opaque and renders as a fallback JSON card.
    pub fn payload_name_for(tool_name: &str) -> Option<String> {
        payload_name_for_output_tool(tool_name).or_else(|| {
            let canonical = canonical_tool_payload_name(tool_name);
            payload_name_for_output_tool(canonical)
        })
    }

    /// Convert into the plugin-neutral [`ToolOutput`] payload used by
    /// persisted message parts. The tool name lives on [`ToolInvocation`],
    /// so the payload stores only the output fields.
    pub fn into_tool_output(self) -> ToolOutput {
        let value = serde_json::to_value(&self).unwrap_or(serde_json::Value::Null);
        let mut object = match value {
            serde_json::Value::Object(map) => map,
            _ => serde_json::Map::new(),
        };
        object.remove("tool");
        let payload =
            StructuredObject::try_from(serde_json::Value::Object(object)).unwrap_or_default();
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
        let payload_name = payload_name_for_output_tool(tool_name)
            .unwrap_or_else(|| canonical_tool_payload_name(tool_name).to_string());
        object.insert("tool".to_string(), serde_json::Value::String(payload_name));
        serde_json::from_value(serde_json::Value::Object(object)).ok()
    }
}

fn canonical_tool_payload_name(name: &str) -> &str {
    name.rsplit_once("__")
        .or_else(|| name.rsplit_once('.'))
        .map(|(_, tool_name)| tool_name)
        .unwrap_or(name)
}

/// One-to-one payload variant mappings for every typed tool family:
/// `(serialized variant tag, compact name, canonical name, plugin-format name)`.
///
/// The provider catalog uses compact names such as `fs.read` and `cron.create`,
/// workflows use canonical names such as `agena.fs.read`, and the plugin wire
/// format uses names such as `agena_fs_read` / `agena_web__fetch`. Every
/// spelling must resolve to the same `ToolPayloadOutput` / `ToolPayloadInput`
/// variant tag; a missed spelling makes the renderer fall back to dumping the
/// whole payload as a raw JSON card.
const PAYLOAD_OUTPUT_TOOLS: &[(&str, &str, &str, &str)] = &[
    ("read", "fs.read", "agena.fs.read", "agena_fs_read"),
    ("glob", "fs.glob", "agena.fs.glob", "agena_fs_glob"),
    ("grep", "fs.grep", "agena.fs.grep", "agena_fs_grep"),
    (
        "apply_patch",
        "fs.apply_patch",
        "agena.fs.apply_patch",
        "agena_fs_apply_patch",
    ),
    ("task", "tasks.run", "agena.tasks.run", "agena_tasks_run"),
    (
        "tool_search",
        "tools.search",
        "agena.tools.search",
        "agena_tools_search",
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
        "enter_snapshot",
        "snapshot.enter",
        "agena.snapshot.enter",
        "agena_snapshot_enter",
    ),
    (
        "exit_snapshot",
        "snapshot.exit",
        "agena.snapshot.exit",
        "agena_snapshot_exit",
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

fn is_payload_variant_tag(name: &str) -> bool {
    matches!(
        name,
        "read"
            | "glob"
            | "monitor"
            | "grep"
            | "apply_patch"
            | "task"
            | "tool_search"
            | "ask_user"
            | "web_fetch"
            | "web_search"
            | "enter_snapshot"
            | "exit_snapshot"
            | "cron_create"
            | "cron_list"
            | "cron_delete"
            | "cron_update"
            | "cron_pause"
            | "cron_resume"
            | "cron_history"
            | "lsp_definition"
            | "lsp_references"
            | "lsp_hover"
            | "lsp_diagnostics"
    )
}

const DIRECT_TOOL_MAPPINGS: &[(&str, &str, &str)] = &[
    ("read", "agena.fs", "read"),
    ("glob", "agena.fs", "glob"),
    ("grep", "agena.fs", "grep"),
    ("apply_patch", "agena.fs", "apply_patch"),
    ("task", "agena.tasks", "run"),
    ("tool_search", "agena.tools", "search"),
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
        "shell" | "process" | "command" => {
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
        "enter_snapshot" => {
            let name = input
                .remove("name")
                .and_then(|value| value.as_str().map(str::to_string));
            let path = input
                .remove("path")
                .and_then(|value| value.as_str().map(str::to_string));
            match (name, path) {
                (_, Some(path)) => {
                    input.insert(
                        "target".to_string(),
                        serde_json::Value::String("existing".to_string()),
                    );
                    input.insert("path".to_string(), serde_json::Value::String(path));
                }
                (name, None) => {
                    input.insert(
                        "target".to_string(),
                        serde_json::Value::String("new".to_string()),
                    );
                    if let Some(name) = name {
                        input.insert("name".to_string(), serde_json::Value::String(name));
                    }
                }
            }
            canonical_registry_tool_name("agena.snapshot", "enter")
        }
        "exit_snapshot" => {
            if let Some(action) = input.remove("action") {
                input.insert("exit_action".to_string(), action);
            }
            canonical_registry_tool_name("agena.snapshot", "exit")
        }
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
    match invocation_name {
        "agena_web__fetch" | "web.fetch" | "web_fetch" => return Some("web_fetch".to_string()),
        "agena_web__search" | "web.search" | "web_search" => {
            return Some("web_search".to_string());
        }
        "agena_fs_read" | "agena.fs.read" | "read" => {
            return Some("read".to_string());
        }
        "agena_fs_glob" | "agena.fs.glob" | "glob" => {
            return Some("glob".to_string());
        }
        "agena_fs_grep" | "agena.fs.grep" | "grep" => {
            return Some("grep".to_string());
        }
        "agena_fs_apply_patch" | "agena.fs.apply_patch" | "apply_patch" => {
            return Some("apply_patch".to_string());
        }
        "shell.run" | "powershell.run" | "process.run" | "agena_shell_run" | "agena.shell.run"
        | "agena_process_run" | "agena.process.run" => {
            input.insert(
                "action".to_string(),
                serde_json::Value::String("run".to_string()),
            );
            return Some("shell".to_string());
        }
        "shell.list" | "powershell.list" | "process.list" | "agena_shell_list"
        | "agena.shell.list" | "agena_process_list" | "agena.process.list" => {
            input.insert(
                "action".to_string(),
                serde_json::Value::String("list".to_string()),
            );
            return Some("shell".to_string());
        }
        "shell.logs" | "powershell.logs" | "process.logs" | "agena_shell_logs"
        | "agena.shell.logs" | "agena_process_logs" | "agena.process.logs" => {
            input.insert(
                "action".to_string(),
                serde_json::Value::String("logs".to_string()),
            );
            return Some("shell".to_string());
        }
        "shell.stop" | "powershell.stop" | "process.stop" | "agena_shell_stop"
        | "agena.shell.stop" | "agena_process_stop" | "agena.process.stop" => {
            input.insert(
                "action".to_string(),
                serde_json::Value::String("stop".to_string()),
            );
            return Some("shell".to_string());
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
    // Every other typed payload reuses the output-tag mapping, which
    // recognizes compact (`fs.read`), canonical (`agena.fs.read`), plugin
    // (`agena_fs_read`), and bare (`read`) spellings for every family.
    payload_name_for_output_tool(invocation_name)
}

fn payload_name_for_output_tool(tool_name: &str) -> Option<String> {
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
    if matches!(
        tool_name,
        "shell"
            | "shell.run"
            | "shell.list"
            | "shell.logs"
            | "shell.stop"
            | "process"
            | "process.run"
            | "process.list"
            | "process.logs"
            | "process.stop"
            | "powershell"
            | "powershell.run"
            | "powershell.list"
            | "powershell.logs"
            | "powershell.stop"
            | "agena.shell.run"
            | "agena.shell.list"
            | "agena.shell.logs"
            | "agena.shell.stop"
            | "agena_shell_run"
            | "agena_shell_list"
            | "agena_shell_logs"
            | "agena_shell_stop"
            | "agena.process.run"
            | "agena.process.list"
            | "agena.process.logs"
            | "agena.process.stop"
            | "agena_process_run"
            | "agena_process_list"
            | "agena_process_logs"
            | "agena_process_stop"
    ) {
        return Some("shell".to_string());
    }
    payload_name_for_table(tool_name).or_else(|| {
        // The terminal segment of a tool name may already be the serialized
        // variant tag (`read`, `glob`, `cron_create`, ...). Accept bare tags
        // and names whose last segment is one of them.
        let canonical = canonical_tool_payload_name(tool_name);
        is_payload_variant_tag(canonical).then(|| canonical.to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registered_tool(plugin: &str, name: &str) -> RegisteredTool {
        let definition = serde_json::from_value::<agena_plugin_host::sdk::ToolDefinition>(
            serde_json::json!({"name": name}),
        )
        .expect("valid test tool definition");
        RegisteredTool::new(plugin.parse().expect("valid test plugin key"), definition)
            .expect("register test tool")
    }

    #[test]
    fn executor_backed_dispatch_is_an_explicit_registry_identity_set() {
        for (plugin, tool) in [
            ("agena.fs", "read"),
            ("agena.fs", "glob"),
            ("agena.fs", "grep"),
            ("agena.fs", "apply_patch"),
            ("agena.shell", "run"),
            ("agena.shell", "list"),
            ("agena.shell", "logs"),
            ("agena.shell", "stop"),
            ("agena.monitor", "start"),
            ("agena.monitor", "stop"),
            ("agena.cron", "create"),
            ("agena.cron", "list"),
            ("agena.cron", "delete"),
            ("agena.cron", "update"),
            ("agena.cron", "pause"),
            ("agena.cron", "resume"),
            ("agena.cron", "history"),
            ("agena.lsp", "definition"),
            ("agena.lsp", "references"),
            ("agena.lsp", "hover"),
            ("agena.lsp", "diagnostics"),
        ] {
            let registered = registered_tool(plugin, tool);
            let compact_name = format!("{}.{}", registered.plugin_name(), tool);
            assert!(
                ToolPayloadInput::from_executor_backed_invocation(
                    &registered,
                    &ToolInvocation::new(compact_name, StructuredObject::default()),
                )
                .is_some(),
                "{plugin}.{tool} must never fall through to its definition-only adapter"
            );
        }

        for (plugin, tool) in [
            ("agena.fs", "write"),
            ("agena.tasks", "run"),
            ("agena.snapshot", "enter"),
            ("agena.interaction", "notify"),
        ] {
            let registered = registered_tool(plugin, tool);
            assert!(
                ToolPayloadInput::from_executor_backed_invocation(
                    &registered,
                    &ToolInvocation::new(
                        format!("{}.{}", registered.plugin_name(), tool),
                        StructuredObject::default(),
                    ),
                )
                .is_none(),
                "{plugin}.{tool} owns a real plugin handler and must not be bypassed"
            );
        }
    }

    #[test]
    fn shell_payloads_emit_shell_invocations() {
        let payload: ToolPayloadInput = serde_json::from_value(serde_json::json!({
            "tool": "shell",
            "action": "list"
        }))
        .expect("shell list payload");
        assert_eq!(
            serde_json::to_value(&payload).expect("serialize shell payload")["tool"],
            "shell"
        );

        let invocation = payload.into_invocation();

        assert_eq!(invocation.name, "agena.shell.list");
    }

    #[test]
    fn legacy_process_invocations_remain_readable() {
        let invocation = ToolInvocation::new("agena.process.list", StructuredObject::default());

        let payload = ToolPayloadInput::from_invocation(&invocation).expect("legacy payload");

        assert!(matches!(
            payload,
            ToolPayloadInput::Shell(ShellToolInput::List {})
        ));
    }

    #[test]
    fn legacy_process_payload_tag_remains_readable() {
        let payload: ToolPayloadInput = serde_json::from_value(serde_json::json!({
            "tool": "process",
            "action": "list"
        }))
        .expect("legacy process payload");

        assert!(matches!(
            payload,
            ToolPayloadInput::Shell(ShellToolInput::List {})
        ));
    }

    #[test]
    fn compact_shell_run_invocations_decode_in_both_directions() {
        // The provider emits the compact name `shell.run`; the payload layer
        // must reconstruct the `shell` discriminant from it exactly like it
        // does for the canonical `agena.shell.run` form.
        // This input intentionally keeps the legacy `filesystem_effects` /
        // `network_effects` wire shape to pin the read-only compatibility
        // path that merges them into the flattened `reads`/`writes`/`network`
        // fields.
        let invocation = ToolInvocation::new(
            "shell.run",
            StructuredObject::try_from(serde_json::json!({
                "command": "cargo test",
                "background": false,
                "filesystem_effects": {"read": [], "write": []},
                "network_effects": [],
            }))
            .expect("structured shell.run input"),
        );
        let input = ToolPayloadInput::from_invocation(&invocation).expect("shell.run input");
        assert!(matches!(
            input,
            ToolPayloadInput::Shell(ShellToolInput::Run { .. })
        ));

        let details = ToolOutput {
            payload: StructuredObject::try_from(serde_json::json!({
                "action": "run",
                "shell": "bash",
                "background": false,
                "status": "exited",
                "exit_code": 0,
                "output": "test result: ok",
            }))
            .expect("shell.run output"),
            managed_outputs: Vec::new(),
            truncated: false,
        };
        let output =
            ToolPayloadOutput::from_tool_output("shell.run", &details).expect("shell.run output");
        assert!(matches!(
            output,
            ToolPayloadOutput::Shell { action, .. } if action == "run"
        ));
    }

    #[test]
    fn every_payload_tool_spelling_resolves_to_its_variant_tag() {
        // The provider catalog, workflows, plugin wire format, and legacy bare
        // tags each spell the same tool differently. Every spelling must map to
        // the same output variant tag, otherwise the renderer falls back to
        // dumping the whole payload as a raw JSON card.
        let cases: &[(&str, &str)] = &[
            // fs family
            ("fs.read", "read"),
            ("agena.fs.read", "read"),
            ("agena_fs_read", "read"),
            ("read", "read"),
            ("fs.glob", "glob"),
            ("agena.fs.glob", "glob"),
            ("agena_fs_glob", "glob"),
            ("glob", "glob"),
            ("fs.grep", "grep"),
            ("agena.fs.grep", "grep"),
            ("agena_fs_grep", "grep"),
            ("grep", "grep"),
            ("fs.apply_patch", "apply_patch"),
            ("agena.fs.apply_patch", "apply_patch"),
            ("agena_fs_apply_patch", "apply_patch"),
            ("apply_patch", "apply_patch"),
            // shell / process family
            ("shell", "shell"),
            ("shell.run", "shell"),
            ("shell.list", "shell"),
            ("shell.logs", "shell"),
            ("shell.stop", "shell"),
            ("process", "shell"),
            ("process.run", "shell"),
            ("powershell.run", "shell"),
            ("agena.shell.run", "shell"),
            ("agena_shell_run", "shell"),
            ("agena.process.list", "shell"),
            ("agena_process_stop", "shell"),
            // monitor family
            ("monitor", "monitor"),
            ("monitor.start", "monitor"),
            ("monitor.stop", "monitor"),
            ("agena.monitor.start", "monitor"),
            ("agena.monitor.stop", "monitor"),
            ("agena_monitor_start", "monitor"),
            ("agena_monitor_stop", "monitor"),
            // tasks / tools / interaction family
            ("tasks.run", "task"),
            ("agena.tasks.run", "task"),
            ("agena_tasks_run", "task"),
            ("task", "task"),
            ("tools.search", "tool_search"),
            ("agena.tools.search", "tool_search"),
            ("agena_tools_search", "tool_search"),
            ("tool_search", "tool_search"),
            ("interaction.ask", "ask_user"),
            ("agena.interaction.ask", "ask_user"),
            ("agena_interaction_ask", "ask_user"),
            ("ask_user", "ask_user"),
            // web family
            ("web.fetch", "web_fetch"),
            ("agena.web.fetch", "web_fetch"),
            ("agena_web__fetch", "web_fetch"),
            ("web_fetch", "web_fetch"),
            ("web.search", "web_search"),
            ("agena.web.search", "web_search"),
            ("agena_web__search", "web_search"),
            ("web_search", "web_search"),
            // snapshot family
            ("snapshot.enter", "enter_snapshot"),
            ("agena.snapshot.enter", "enter_snapshot"),
            ("agena_snapshot_enter", "enter_snapshot"),
            ("enter_snapshot", "enter_snapshot"),
            ("snapshot.exit", "exit_snapshot"),
            ("agena.snapshot.exit", "exit_snapshot"),
            ("agena_snapshot_exit", "exit_snapshot"),
            ("exit_snapshot", "exit_snapshot"),
            // cron family
            ("cron.create", "cron_create"),
            ("agena.cron.create", "cron_create"),
            ("agena_cron_create", "cron_create"),
            ("cron_create", "cron_create"),
            ("cron.list", "cron_list"),
            ("agena.cron.list", "cron_list"),
            ("agena_cron_list", "cron_list"),
            ("cron_list", "cron_list"),
            ("cron.delete", "cron_delete"),
            ("cron.update", "cron_update"),
            ("cron.pause", "cron_pause"),
            ("cron.resume", "cron_resume"),
            ("cron.history", "cron_history"),
            // lsp family
            ("lsp.definition", "lsp_definition"),
            ("agena.lsp.definition", "lsp_definition"),
            ("agena_lsp_definition", "lsp_definition"),
            ("lsp_definition", "lsp_definition"),
            ("lsp.references", "lsp_references"),
            ("lsp.hover", "lsp_hover"),
            ("lsp.diagnostics", "lsp_diagnostics"),
            ("agena.lsp.diagnostics", "lsp_diagnostics"),
            ("agena_lsp_diagnostics", "lsp_diagnostics"),
        ];
        for (name, expected) in cases {
            assert_eq!(
                ToolPayloadOutput::payload_name_for(name).as_deref(),
                Some(*expected),
                "tool name {name}"
            );
        }
    }

    #[test]
    fn input_invocations_resolve_for_every_compact_tool_name() {
        let cases: &[(&str, &str)] = &[
            ("fs.read", "read"),
            ("fs.glob", "glob"),
            ("fs.grep", "grep"),
            ("fs.apply_patch", "apply_patch"),
            ("web.fetch", "web_fetch"),
            ("web.search", "web_search"),
            ("tasks.run", "task"),
            ("tools.search", "tool_search"),
            ("interaction.ask", "ask_user"),
            ("snapshot.enter", "enter_snapshot"),
            ("snapshot.exit", "exit_snapshot"),
            ("cron.create", "cron_create"),
            ("cron.list", "cron_list"),
            ("lsp.definition", "lsp_definition"),
            ("lsp.diagnostics", "lsp_diagnostics"),
            ("shell.run", "shell"),
            ("shell.list", "shell"),
            ("shell.logs", "shell"),
            ("shell.stop", "shell"),
            ("monitor.start", "monitor"),
            ("monitor.stop", "monitor"),
        ];
        for (name, expected) in cases {
            let mut input = serde_json::Map::new();
            let tag = payload_name_for_invocation(name, &mut input)
                .unwrap_or_else(|| panic!("{name} must resolve"));
            assert_eq!(tag, *expected, "tool name {name}");
        }

        // Shell invocations also carry the concrete subcommand in `action`.
        let mut run_input = serde_json::Map::new();
        assert_eq!(
            payload_name_for_invocation("shell.run", &mut run_input),
            Some("shell".to_string())
        );
        assert_eq!(
            run_input.get("action").and_then(serde_json::Value::as_str),
            Some("run")
        );
    }

    #[test]
    fn output_payloads_decode_from_compact_tool_names() {
        let glob_details = ToolOutput {
            payload: StructuredObject::try_from(serde_json::json!({
                "paths": ["src/a.rs"],
                "count": 1,
            }))
            .expect("glob output"),
            managed_outputs: Vec::new(),
            truncated: false,
        };
        let output = ToolPayloadOutput::from_tool_output("fs.glob", &glob_details)
            .expect("fs.glob output must decode");
        assert!(matches!(output, ToolPayloadOutput::Glob { .. }));

        let cron_details = ToolOutput {
            payload: StructuredObject::try_from(serde_json::json!({ "jobs": [] }))
                .expect("cron list output"),
            managed_outputs: Vec::new(),
            truncated: false,
        };
        let output = ToolPayloadOutput::from_tool_output("cron.list", &cron_details)
            .expect("cron.list output must decode");
        assert!(matches!(output, ToolPayloadOutput::CronList { .. }));
    }
}
