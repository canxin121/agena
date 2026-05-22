use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use strum::Display;

use crate::message::{
    ApplyPatchToolInput, AskUserToolInput, AttachmentKind, BashToolInput, CronCreateToolInput,
    CronDeleteToolInput, CronListToolInput, EnterPlanModeToolInput, EnterWorktreeToolInput,
    ExitPlanModeToolInput, ExitWorktreeToolInput, FileChangeEntry, GlobToolInput, GrepToolInput,
    LspDefinitionToolInput, LspDiagnosticsToolInput, LspHoverToolInput, LspReferencesToolInput,
    MonitorEvent, MonitorStatus, MonitorToolInput, NotebookEditToolInput, PowerShellToolInput,
    ReadToolInput, ScheduleWakeupToolInput, StructuredObject, TodoItem, TodoWriteToolInput,
    ToolInvocation, ToolOutput, ToolSearchToolInput, WebFetchToolInput, WebSearchToolInput,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Display)]
#[serde(tag = "tool", rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum ToolPayloadInput {
    Bash(BashToolInput),
    Read(ReadToolInput),
    ApplyPatch(ApplyPatchToolInput),
    Glob(GlobToolInput),
    Grep(GrepToolInput),
    Task(crate::message::TaskToolInput),
    ToolSearch(ToolSearchToolInput),
    TodoWrite(TodoWriteToolInput),
    #[serde(rename = "ask_user")]
    AskUser(AskUserToolInput),
    Monitor(MonitorToolInput),
    WebFetch(WebFetchToolInput),
    WebSearch(WebSearchToolInput),
    EnterPlanMode(EnterPlanModeToolInput),
    ExitPlanMode(ExitPlanModeToolInput),
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
    NotebookEdit(NotebookEditToolInput),
    PowerShell(PowerShellToolInput),
}

impl ToolPayloadInput {
    /// Stable tool name as serialized in the wire format.
    pub fn tool_name(&self) -> &'static str {
        match self {
            Self::Bash(_) => "bash",
            Self::Read(_) => "read",
            Self::ApplyPatch(_) => "apply_patch",
            Self::Glob(_) => "glob",
            Self::Grep(_) => "grep",
            Self::Task(_) => "task",
            Self::ToolSearch(_) => "tool_search",
            Self::TodoWrite(_) => "todo_write",
            Self::AskUser(_) => "ask_user",
            Self::Monitor(_) => "monitor",
            Self::WebFetch(_) => "web_fetch",
            Self::WebSearch(_) => "web_search",
            Self::EnterPlanMode(_) => "enter_plan_mode",
            Self::ExitPlanMode(_) => "exit_plan_mode",
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
            Self::NotebookEdit(_) => "notebook_edit",
            Self::PowerShell(_) => "powershell",
        }
    }

    /// Convert into a generic [`ToolInvocation`] carrying this tool's name
    /// and a `StructuredObject` payload.
    pub fn into_invocation(self) -> ToolInvocation {
        let legacy_name = self.tool_name();
        let value = serde_json::to_value(&self).unwrap_or(serde_json::Value::Null);
        let mut object = match value {
            serde_json::Value::Object(map) => map,
            _ => serde_json::Map::new(),
        };
        object.remove("tool");
        let name = match grouped_invocation_for_tool(legacy_name, &mut object) {
            Some((entry, action)) => {
                object.insert(
                    "action".to_string(),
                    serde_json::Value::String(action.to_string()),
                );
                entry.to_string()
            }
            None => legacy_name.to_string(),
        };
        let payload =
            StructuredObject::try_from(serde_json::Value::Object(object)).unwrap_or_default();
        ToolInvocation::new(name, payload)
    }

    /// Reconstruct a `ToolPayloadInput` from a [`ToolInvocation`], or `None`
    /// if the invocation does not use one of the typed payload tool names.
    /// Namespaced forms like `plugin/tool` are matched by their terminal
    /// segment so user-installed replacements inherit the same semantics.
    pub fn from_invocation(invocation: &ToolInvocation) -> Option<Self> {
        let value: serde_json::Value = invocation.input.clone().into();
        let mut object = match value {
            serde_json::Value::Object(map) => map,
            serde_json::Value::Null => serde_json::Map::new(),
            _ => return None,
        };
        let name = canonical_tool_payload_name(invocation.name.as_str());
        let payload_name = grouped_tool_payload_name(name, &mut object).unwrap_or(name);
        object.insert(
            "tool".to_string(),
            serde_json::Value::String(payload_name.into()),
        );
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
    Bash {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        output: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
    },
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
        changes: Vec<FileChangeEntry>,
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
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        loaded_tools: Vec<String>,
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
    Monitor {
        action: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        monitor_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        status: Option<MonitorStatus>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        events: Vec<MonitorEvent>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        monitors: Vec<crate::message::MonitorSummary>,
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
    EnterPlanMode {
        plan_path: String,
        slug: String,
    },
    ExitPlanMode {
        approved: bool,
        plan_path: String,
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
    NotebookEdit {
        path: String,
        edit_mode: String,
        cell_index: u32,
        cell_count: u32,
    },
    PowerShell {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        output: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
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
            Self::Bash { .. } => "bash",
            Self::Read { .. } => "read",
            Self::ApplyPatch { .. } => "apply_patch",
            Self::Glob { .. } => "glob",
            Self::Grep { .. } => "grep",
            Self::Task { .. } => "task",
            Self::ToolSearch { .. } => "tool_search",
            Self::TodoWrite { .. } => "todo_write",
            Self::AskUser { .. } => "ask_user",
            Self::Monitor { .. } => "monitor",
            Self::WebFetch { .. } => "web_fetch",
            Self::WebSearch { .. } => "web_search",
            Self::EnterPlanMode { .. } => "enter_plan_mode",
            Self::ExitPlanMode { .. } => "exit_plan_mode",
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
            Self::NotebookEdit { .. } => "notebook_edit",
            Self::PowerShell { .. } => "powershell",
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
        object.insert(
            "tool".to_string(),
            serde_json::Value::String(canonical_tool_payload_name(tool_name).into()),
        );
        serde_json::from_value(serde_json::Value::Object(object)).ok()
    }
}

fn canonical_tool_payload_name(name: &str) -> &str {
    name.rsplit('/').next().unwrap_or(name)
}

fn grouped_invocation_for_tool(
    tool: &str,
    input: &mut serde_json::Map<String, serde_json::Value>,
) -> Option<(&'static str, &'static str)> {
    Some(match tool {
        "read" => ("fs", "read"),
        "glob" => ("fs", "glob"),
        "grep" => ("fs", "grep"),
        "apply_patch" => ("fs", "apply_patch"),
        "notebook_edit" => ("fs", "notebook_edit"),
        "bash" => {
            input.insert(
                "shell".to_string(),
                serde_json::Value::String("bash".to_string()),
            );
            ("shell", "exec")
        }
        "powershell" => {
            input.insert(
                "shell".to_string(),
                serde_json::Value::String("powershell".to_string()),
            );
            ("shell", "exec")
        }
        "monitor" => {
            let command = match input.get("action").and_then(serde_json::Value::as_str) {
                Some("start") => "monitor_start",
                Some("list") => "monitor_list",
                Some("read") => "monitor_read",
                Some("stop") => "monitor_stop",
                _ => "monitor",
            };
            if command != "monitor" {
                input.remove("action");
            }
            ("shell", command)
        }
        "web_fetch" => ("web", "fetch"),
        "web_search" => ("web", "search"),
        "task" => ("task", "run"),
        "tool_search" => ("tools", "search"),
        "todo_write" => ("todo", "write"),
        "ask_user" => ("user", "request_input"),
        "enter_plan_mode" => ("plan", "enter"),
        "exit_plan_mode" => ("plan", "exit"),
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
            ("worktree", "enter")
        }
        "exit_worktree" => ("worktree", "exit"),
        "cron_create" => ("schedule", "create"),
        "cron_list" => ("schedule", "list"),
        "cron_delete" => ("schedule", "delete"),
        "schedule_wakeup" => ("schedule", "wakeup"),
        "lsp_definition" => ("lsp", "definition"),
        "lsp_references" => ("lsp", "references"),
        "lsp_hover" => ("lsp", "hover"),
        "lsp_diagnostics" => ("lsp", "diagnostics"),
        _ => return None,
    })
}

fn grouped_tool_payload_name(
    entry: &str,
    input: &mut serde_json::Map<String, serde_json::Value>,
) -> Option<&'static str> {
    let action = input.get("action")?.as_str()?.to_string();
    let tool = match (entry, action.as_str()) {
        ("fs", "read") => "read",
        ("fs", "glob") => "glob",
        ("fs", "grep") => "grep",
        ("fs", "apply_patch") => "apply_patch",
        ("fs", "notebook_edit") => "notebook_edit",
        ("shell", "exec") => match input.get("shell").and_then(serde_json::Value::as_str) {
            Some("bash") => "bash",
            Some("powershell") => "powershell",
            _ => return None,
        },
        ("shell", "monitor_start") => "monitor",
        ("shell", "monitor_list") => "monitor",
        ("shell", "monitor_read") => "monitor",
        ("shell", "monitor_stop") => "monitor",
        ("web", "fetch") => "web_fetch",
        ("web", "search") => "web_search",
        ("task", "run") => "task",
        ("tools", "search") => "tool_search",
        ("todo", "write") => "todo_write",
        ("user", "request_input") => "ask_user",
        ("plan", "enter") => "enter_plan_mode",
        ("plan", "exit") => "exit_plan_mode",
        ("worktree", "enter") => "enter_worktree",
        ("worktree", "exit") => "exit_worktree",
        ("schedule", "create") => "cron_create",
        ("schedule", "list") => "cron_list",
        ("schedule", "delete") => "cron_delete",
        ("schedule", "wakeup") => "schedule_wakeup",
        ("lsp", "definition") => "lsp_definition",
        ("lsp", "references") => "lsp_references",
        ("lsp", "hover") => "lsp_hover",
        ("lsp", "diagnostics") => "lsp_diagnostics",
        _ => return None,
    };
    input.remove("action");
    match (entry, action.as_str()) {
        ("shell", "exec") => {
            input.remove("shell");
        }
        ("shell", "monitor_start") => {
            input.insert(
                "action".to_string(),
                serde_json::Value::String("start".to_string()),
            );
        }
        ("shell", "monitor_list") => {
            input.insert(
                "action".to_string(),
                serde_json::Value::String("list".to_string()),
            );
        }
        ("shell", "monitor_read") => {
            input.insert(
                "action".to_string(),
                serde_json::Value::String("read".to_string()),
            );
        }
        ("shell", "monitor_stop") => {
            input.insert(
                "action".to_string(),
                serde_json::Value::String("stop".to_string()),
            );
        }
        ("worktree", "enter") => match input
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
        ("worktree", "exit") => {
            if let Some(exit_action) = input.remove("exit_action") {
                input.insert("action".to_string(), exit_action);
            }
        }
        _ => {}
    }
    Some(tool)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::message::{FilesystemAccess, FilesystemEffect};

    fn sample_effects() -> Vec<FilesystemEffect> {
        vec![FilesystemEffect {
            path: ".".to_string(),
            access: FilesystemAccess::Read,
        }]
    }

    #[test]
    fn bash_payload_uses_shell_exec_action_shape() {
        let invocation = ToolPayloadInput::Bash(BashToolInput {
            command: "pwd".to_string(),
            description: "print cwd".to_string(),
            timeout_ms: Some(1000),
            workdir: Some("repo".to_string()),
            filesystem_effects: sample_effects(),
        })
        .into_invocation();

        assert_eq!(invocation.name, "shell");
        let payload = serde_json::Value::from(invocation.input);
        assert_eq!(payload["action"], "exec");
        assert_eq!(payload["shell"], "bash");
        assert_eq!(payload["command"], "pwd");
    }

    #[test]
    fn powershell_payload_uses_shell_exec_action_shape() {
        let invocation = ToolPayloadInput::PowerShell(PowerShellToolInput {
            command: "Get-Location".to_string(),
            description: "print cwd".to_string(),
            timeout_ms: None,
            workdir: None,
            filesystem_effects: sample_effects(),
        })
        .into_invocation();

        assert_eq!(invocation.name, "shell");
        let payload = serde_json::Value::from(invocation.input);
        assert_eq!(payload["action"], "exec");
        assert_eq!(payload["shell"], "powershell");
        assert_eq!(payload["command"], "Get-Location");
    }

    #[test]
    fn shell_exec_invocation_round_trips_to_bash_payload() {
        let invocation = ToolInvocation::new(
            "shell",
            StructuredObject::try_from(json!({
                "action": "exec",
                "shell": "bash",
                "command": "pwd",
                "description": "print cwd",
                "filesystem_effects": [{"path": ".", "access": "read"}]
            }))
            .expect("structured object should build"),
        );

        let payload = ToolPayloadInput::from_invocation(&invocation)
            .expect("shell exec invocation should decode");
        match payload {
            ToolPayloadInput::Bash(args) => {
                assert_eq!(args.command, "pwd");
                assert_eq!(args.description, "print cwd");
            }
            other => panic!("expected bash payload, got {other:?}"),
        }
    }
}
