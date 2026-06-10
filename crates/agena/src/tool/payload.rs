use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use strum::Display;

use crate::message::{
    ApplyPatchToolInput, AskUserToolInput, AttachmentKind, CronCreateToolInput,
    CronDeleteToolInput, CronListToolInput, EnterWorktreeToolInput, ExitWorktreeToolInput,
    FileChangeRecord, GlobToolInput, GrepToolInput, LspDefinitionToolInput,
    LspDiagnosticsToolInput, LspHoverToolInput, LspReferencesToolInput, ProcessEvent, ProcessShell,
    ProcessStatus, ProcessSummary, ProcessToolInput, ReadToolInput, ScheduleWakeupToolInput,
    StructuredObject, TodoItem, TodoWriteToolInput, ToolInvocation, ToolOutput,
    ToolSearchToolInput, WebFetchToolInput, WebSearchToolInput,
};

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
    TodoWrite(TodoWriteToolInput),
    #[serde(rename = "ask_user")]
    AskUser(AskUserToolInput),
    WebFetch(WebFetchToolInput),
    WebSearch(WebSearchToolInput),
    EnterWorktree(EnterWorktreeToolInput),
    ExitWorktree(ExitWorktreeToolInput),
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
            Self::TodoWrite(_) => "todo_write",
            Self::AskUser(_) => "ask_user",
            Self::WebFetch(_) => "web_fetch",
            Self::WebSearch(_) => "web_search",
            Self::EnterWorktree(_) => "enter_worktree",
            Self::ExitWorktree(_) => "exit_worktree",
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
    /// Namespaced forms like `plugin__tool` are matched by their terminal
    /// segment so user-installed replacements inherit the same semantics.
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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        truncated: Option<bool>,
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
    TodoWrite {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        items: Vec<TodoItem>,
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
    EnterWorktree {
        path: String,
        branch: String,
    },
    ExitWorktree {
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
            Self::TodoWrite { .. } => "todo_write",
            Self::AskUser { .. } => "ask_user",
            Self::WebFetch { .. } => "web_fetch",
            Self::WebSearch { .. } => "web_search",
            Self::EnterWorktree { .. } => "enter_worktree",
            Self::ExitWorktree { .. } => "exit_worktree",
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
        ToolOutput { payload }
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
        .map(|(_, tool_name)| tool_name)
        .unwrap_or(name)
}

const DIRECT_GROUPED_TOOL_MAPPINGS: &[(&str, &str, &str, &str)] = &[
    ("read", "agena.fs", "fs", "read"),
    ("glob", "agena.fs", "fs", "glob"),
    ("grep", "agena.fs", "fs", "grep"),
    ("apply_patch", "agena.fs", "fs", "apply_patch"),
    ("task", "agena.tasks", "task", "run"),
    ("tool_search", "agena.catalog", "tools", "search"),
    ("todo_write", "agena.planning", "todo", "write"),
    ("ask_user", "agena.runtime", "user", "request_input"),
    ("exit_worktree", "agena.repo", "worktree", "exit"),
    ("lsp_definition", "agena.lsp", "lsp", "definition"),
    ("lsp_references", "agena.lsp", "lsp", "references"),
    ("lsp_hover", "agena.lsp", "lsp", "hover"),
    ("lsp_diagnostics", "agena.lsp", "lsp", "diagnostics"),
];

fn exposed_tool_name(plugin: &str, tool: &str) -> String {
    crate::plugin::registry::exposed_tool_name(plugin, tool)
}

fn grouped_mapping_for_tool(tool: &str) -> Option<(&'static str, &'static str, &'static str)> {
    DIRECT_GROUPED_TOOL_MAPPINGS
        .iter()
        .find(|(name, _, _, _)| *name == tool)
        .map(|(_, plugin, entry, action)| (*plugin, *entry, *action))
}

fn tool_name_for_grouped_mapping(entry: &str, action: &str) -> Option<&'static str> {
    DIRECT_GROUPED_TOOL_MAPPINGS
        .iter()
        .find(|(_, _, mapped_entry, mapped_action)| {
            *mapped_entry == entry && *mapped_action == action
        })
        .map(|(tool, _, _, _)| *tool)
}

fn invocation_name_for_payload_tool(
    tool: &str,
    input: &mut serde_json::Map<String, serde_json::Value>,
) -> Option<String> {
    Some(match tool {
        "web_fetch" => exposed_tool_name("agena.web", "fetch"),
        "web_search" => exposed_tool_name("agena.web", "search"),
        "process" => exposed_tool_name("agena.process", "process"),
        "cron_create" => exposed_tool_name("agena.cron", "schedule.create"),
        "cron_list" => exposed_tool_name("agena.cron", "schedule.list"),
        "cron_delete" => exposed_tool_name("agena.cron", "schedule.delete"),
        "schedule_wakeup" => exposed_tool_name("agena.cron", "schedule.wakeup"),
        "enter_worktree" => {
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
            input.insert(
                "action".to_string(),
                serde_json::Value::String("enter".to_string()),
            );
            exposed_tool_name("agena.repo", "worktree")
        }
        _ => {
            let (plugin, entry, action) = grouped_mapping_for_tool(tool)?;
            input.insert(
                "action".to_string(),
                serde_json::Value::String(action.to_string()),
            );
            exposed_tool_name(plugin, entry)
        }
    })
}

fn payload_name_for_invocation(
    invocation_name: &str,
    input: &mut serde_json::Map<String, serde_json::Value>,
) -> Option<String> {
    match invocation_name {
        "agena_web__fetch" | "web_fetch" => return Some("web_fetch".to_string()),
        "agena_web__search" | "web_search" => return Some("web_search".to_string()),
        "agena_process__process_run" => {
            input.insert(
                "action".to_string(),
                serde_json::Value::String("run".to_string()),
            );
            return Some("process".to_string());
        }
        "agena_process__process_list" => {
            input.insert(
                "action".to_string(),
                serde_json::Value::String("list".to_string()),
            );
            return Some("process".to_string());
        }
        "agena_process__process_logs" => {
            input.insert(
                "action".to_string(),
                serde_json::Value::String("logs".to_string()),
            );
            return Some("process".to_string());
        }
        "agena_process__process_stop" => {
            input.insert(
                "action".to_string(),
                serde_json::Value::String("stop".to_string()),
            );
            return Some("process".to_string());
        }
        "agena_cron__schedule_create" => return Some("cron_create".to_string()),
        "agena_cron__schedule_list" => return Some("cron_list".to_string()),
        "agena_cron__schedule_delete" => return Some("cron_delete".to_string()),
        "agena_cron__schedule_wakeup" => return Some("schedule_wakeup".to_string()),
        _ => {}
    }
    let entry = canonical_tool_payload_name(invocation_name);
    grouped_tool_payload_name(entry, input).map(str::to_string)
}

fn grouped_tool_payload_name(
    entry: &str,
    input: &mut serde_json::Map<String, serde_json::Value>,
) -> Option<&'static str> {
    let action = input.get("action")?.as_str()?.to_string();
    let tool = match (entry, action.as_str()) {
        ("process", "run" | "list" | "logs" | "stop") => "process",
        ("worktree", "enter") => "enter_worktree",
        _ => tool_name_for_grouped_mapping(entry, action.as_str())?,
    };
    input.remove("action");
    match tool {
        "process" => {
            input.insert("action".to_string(), serde_json::Value::String(action));
        }
        "enter_worktree" => match input
            .get("target")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
        {
            "existing" => {
                input.remove("target");
                input.remove("name");
            }
            "new" => {
                input.remove("target");
                input.remove("path");
            }
            _ => {}
        },
        "exit_worktree" => {
            if let Some(exit_action) = input.remove("exit_action") {
                input.insert("action".to_string(), exit_action);
            }
        }
        _ => {}
    }
    Some(tool)
}

fn payload_name_for_output_tool(tool_name: &str) -> Option<String> {
    match tool_name {
        "agena_web__fetch" | "web_fetch" | "fetch" => Some("web_fetch".to_string()),
        "agena_web__search" | "web_search" | "search" => Some("web_search".to_string()),
        "agena_process__process"
        | "agena_process__process_run"
        | "agena_process__process_list"
        | "agena_process__process_logs"
        | "agena_process__process_stop"
        | "process" => Some("process".to_string()),
        "agena_cron__schedule_create" | "schedule.create" => Some("cron_create".to_string()),
        "agena_cron__schedule_list" | "schedule.list" => Some("cron_list".to_string()),
        "agena_cron__schedule_delete" | "schedule.delete" => Some("cron_delete".to_string()),
        "agena_cron__schedule_wakeup" | "schedule.wakeup" => Some("schedule_wakeup".to_string()),
        "agena_fs__fs" | "fs" => None,
        "agena_tasks__task" | "task" => Some("task".to_string()),
        "agena_catalog__tools" | "tools" => Some("tool_search".to_string()),
        "agena_planning__todo" | "todo" => Some("todo_write".to_string()),
        "agena_runtime__user" | "user" => Some("ask_user".to_string()),
        "agena_planning__plan" | "plan" => None,
        "agena_repo__worktree" | "worktree" => None,
        "agena_cron__schedule" | "schedule" => None,
        "agena_lsp__lsp" | "lsp" => None,
        _ => None,
    }
}
