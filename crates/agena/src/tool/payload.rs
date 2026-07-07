use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use strum::Display;

use crate::message::{
    ApplyPatchToolInput, AskUserToolInput, AttachmentKind, CronCreateToolInput,
    CronDeleteToolInput, CronListToolInput, EnterSnapshotToolInput, ExitSnapshotToolInput,
    FileChangeRecord, GlobToolInput, GrepToolInput, LspDefinitionToolInput,
    LspDiagnosticsToolInput, LspHoverToolInput, LspReferencesToolInput, ProcessEvent, ProcessShell,
    ProcessStatus, ProcessSummary, ProcessToolInput, ReadToolInput, ScheduleWakeupToolInput,
    StructuredObject, ToolInvocation, ToolOutput, ToolSearchToolInput, WebFetchToolInput,
    WebSearchToolInput,
};

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Display)]
#[serde(tag = "tool", rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum ToolPayloadInput {
    Process(ProcessToolInput),
    Read(ReadToolInput),
    ApplyPatch(ApplyPatchToolInput),
    Glob(GlobToolInput),
    Grep(GrepToolInput),
    Task(crate::message::TaskToolInput),
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
    ScheduleWakeup(ScheduleWakeupToolInput),
    LspDefinition(LspDefinitionToolInput),
    LspReferences(LspReferencesToolInput),
    LspHover(LspHoverToolInput),
    LspDiagnostics(LspDiagnosticsToolInput),
}

impl ToolPayloadInput {
    /// Stable tool name as serialized in the wire format.
    pub fn tool_name(&self) -> &'static str {
        match self {
            Self::Process(_) => "process",
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
            Self::ScheduleWakeup(_) => "schedule_wakeup",
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
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
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
    },
    Grep {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        matches: Option<u32>,
    },
    Task {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model_provider_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model_id: Option<String>,
    },
    ToolSearch {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        results: Vec<String>,
    },
    #[serde(rename = "ask_user")]
    AskUser {
        #[serde(
            default,
            deserialize_with = "crate::message::deserialize_user_input_answers",
            skip_serializing_if = "crate::message::user_input_answers_is_empty"
        )]
        answers: BTreeMap<String, Vec<String>>,
    },
    Process {
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
        results: Vec<WebSearchHit>,
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
    ScheduleWakeup {
        id: String,
        next_fire_at: String,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CronJobSummary {
    pub id: String,
    pub kind: String,
    pub expression: Option<String>,
    pub at: Option<String>,
    pub prompt: String,
    pub next_fire_at: Option<String>,
    pub last_fired_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WebSearchHit {
    pub title: String,
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snippet: Option<String>,
}

impl ToolPayloadOutput {
    /// Stable tool name as serialized in the wire format.
    pub fn tool_name(&self) -> &'static str {
        match self {
            Self::Process { .. } => "process",
            Self::Read { .. } => "read",
            Self::ApplyPatch { .. } => "apply_patch",
            Self::Glob { .. } => "glob",
            Self::Grep { .. } => "grep",
            Self::Task { .. } => "task",
            Self::ToolSearch { .. } => "tool_search",
            Self::AskUser { .. } => "ask_user",
            Self::WebFetch { .. } => "web_fetch",
            Self::WebSearch { .. } => "web_search",
            Self::EnterSnapshot { .. } => "enter_snapshot",
            Self::ExitSnapshot { .. } => "exit_snapshot",
            Self::CronCreate { .. } => "cron_create",
            Self::CronList { .. } => "cron_list",
            Self::CronDelete { .. } => "cron_delete",
            Self::ScheduleWakeup { .. } => "schedule_wakeup",
            Self::LspDefinition { .. } => "lsp_definition",
            Self::LspReferences { .. } => "lsp_references",
            Self::LspHover { .. } => "lsp_hover",
            Self::LspDiagnostics { .. } => "lsp_diagnostics",
        }
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

const DIRECT_TOOL_MAPPINGS: &[(&str, &str, &str)] = &[
    ("read", "agena.fs", "read"),
    ("glob", "agena.fs", "glob"),
    ("grep", "agena.fs", "grep"),
    ("apply_patch", "agena.fs", "apply_patch"),
    ("task", "agena.tasks", "run"),
    ("tool_search", "agena.tools", "search"),
    ("ask_user", "agena.runtime", "request_input"),
    ("lsp_definition", "agena.lsp", "definition"),
    ("lsp_references", "agena.lsp", "references"),
    ("lsp_hover", "agena.lsp", "hover"),
    ("lsp_diagnostics", "agena.lsp", "diagnostics"),
];

fn model_tool_name(plugin: &str, tool: &str) -> String {
    let plugin_key =
        crate::plugin::PluginKey::parse(plugin).expect("direct tool mapping plugin key");
    crate::plugin::ToolKey::new(plugin_key, tool.to_string())
        .expect("direct tool mapping tool key")
        .to_model_string()
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
        "web_fetch" => model_tool_name("agena.web", "fetch"),
        "web_search" => model_tool_name("agena.web", "search"),
        "process" | "command" => {
            let action = input
                .remove("action")
                .and_then(|value| value.as_str().map(str::to_string))
                .unwrap_or_else(|| "list".to_string());
            model_tool_name("agena.process", action.as_str())
        }
        "cron_create" => model_tool_name("agena.cron", "create"),
        "cron_list" => model_tool_name("agena.cron", "list"),
        "cron_delete" => model_tool_name("agena.cron", "delete"),
        "schedule_wakeup" => model_tool_name("agena.cron", "wakeup"),
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
            model_tool_name("agena.snapshot", "enter")
        }
        "exit_snapshot" => {
            if let Some(action) = input.remove("action") {
                input.insert("exit_action".to_string(), action);
            }
            model_tool_name("agena.snapshot", "exit")
        }
        _ => {
            let (plugin, tool) = direct_mapping_for_tool(tool)?;
            model_tool_name(plugin, tool)
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
        "agena_process_run" | "agena.process.run" => {
            input.insert(
                "action".to_string(),
                serde_json::Value::String("run".to_string()),
            );
            return Some("process".to_string());
        }
        "agena_process_list" | "agena.process.list" => {
            input.insert(
                "action".to_string(),
                serde_json::Value::String("list".to_string()),
            );
            return Some("process".to_string());
        }
        "agena_process_logs" | "agena.process.logs" => {
            input.insert(
                "action".to_string(),
                serde_json::Value::String("logs".to_string()),
            );
            return Some("process".to_string());
        }
        "agena_process_stop" | "agena.process.stop" => {
            input.insert(
                "action".to_string(),
                serde_json::Value::String("stop".to_string()),
            );
            return Some("process".to_string());
        }
        "agena_cron_create" | "agena.cron.create" => {
            return Some("cron_create".to_string());
        }
        "agena_cron_list" | "agena.cron.list" => {
            return Some("cron_list".to_string());
        }
        "agena_cron_delete" | "agena.cron.delete" => {
            return Some("cron_delete".to_string());
        }
        "agena_cron_wakeup" | "agena.cron.wakeup" => {
            return Some("schedule_wakeup".to_string());
        }
        "agena_tasks_run" | "agena.tasks.run" => {
            return Some("task".to_string());
        }
        "agena_tools_search" | "agena.tools.search" => {
            return Some("tool_search".to_string());
        }
        "agena_runtime_request_input" | "agena.runtime.request_input" => {
            return Some("ask_user".to_string());
        }
        "agena_snapshot_enter" | "agena.snapshot.enter" => {
            return Some("enter_snapshot".to_string());
        }
        "agena_snapshot_exit" | "agena.snapshot.exit" => {
            return Some("exit_snapshot".to_string());
        }
        "agena_lsp_definition" | "agena.lsp.definition" => {
            return Some("lsp_definition".to_string());
        }
        "agena_lsp_references" | "agena.lsp.references" => {
            return Some("lsp_references".to_string());
        }
        "agena_lsp_hover" | "agena.lsp.hover" => {
            return Some("lsp_hover".to_string());
        }
        "agena_lsp_diagnostics" | "agena.lsp.diagnostics" => {
            return Some("lsp_diagnostics".to_string());
        }
        _ => {}
    }
    None
}

fn payload_name_for_output_tool(tool_name: &str) -> Option<String> {
    match tool_name {
        "agena_web__fetch" | "web.fetch" | "web_fetch" | "fetch" => Some("web_fetch".to_string()),
        "agena_web__search" | "web.search" | "web_search" | "search" => {
            Some("web_search".to_string())
        }
        "agena.process.run" | "agena.process.list" | "agena.process.logs"
        | "agena.process.stop" | "agena_process_run" | "agena_process_list"
        | "agena_process_logs" | "agena_process_stop" => Some("process".to_string()),
        "agena.cron.create" | "agena_cron_create" => Some("cron_create".to_string()),
        "agena.cron.list" | "agena_cron_list" => Some("cron_list".to_string()),
        "agena.cron.delete" | "agena_cron_delete" => Some("cron_delete".to_string()),
        "agena.cron.wakeup" | "agena_cron_wakeup" => Some("schedule_wakeup".to_string()),
        "agena.fs.read"
        | "agena.fs.glob"
        | "agena.fs.grep"
        | "agena.fs.apply_patch"
        | "agena_fs_read"
        | "agena_fs_glob"
        | "agena_fs_grep"
        | "agena_fs_apply_patch" => None,
        "agena.tasks.run" | "agena_tasks_run" => Some("task".to_string()),
        "agena.tools.search" | "agena_tools_search" => Some("tool_search".to_string()),
        "agena.runtime.request_input" | "agena_runtime_request_input" => {
            Some("ask_user".to_string())
        }
        "agena.plan.get" | "agena.plan.set" | "agena.plan.update" | "agena.plan.clear" => None,
        "agena.snapshot.enter"
        | "agena.snapshot.exit"
        | "agena_snapshot_enter"
        | "agena_snapshot_exit" => None,
        "agena.lsp.definition"
        | "agena.lsp.references"
        | "agena.lsp.hover"
        | "agena.lsp.diagnostics"
        | "agena_lsp_definition"
        | "agena_lsp_references"
        | "agena_lsp_hover"
        | "agena_lsp_diagnostics" => None,
        _ => None,
    }
}
