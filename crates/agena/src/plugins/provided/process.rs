//! `agena.process` plugin: run shell commands and manage background processes.

use crate::message::{ProcessShell, ProcessToolInput, ShellCommandInput};
use crate::plugin::PluginError;
use crate::plugin::sdk::{
    NetworkRequest, PathRequest, Plugin, PluginManifest, Result as SdkResult, ToolInvokeOutput,
    ToolTag, async_trait,
};
use crate::plugins::provided::router;
use agena_macros::{StaticToolSurface, ToolInputShape};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub(crate) const PROCESS_PLUGIN_ID: &str = "agena.process";

pub(crate) struct ProcessPlugin;

pub(crate) fn new_plugin() -> ProcessPlugin {
    ProcessPlugin
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInputShape)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProcessRunToolArgs {
    #[serde(default)]
    shell: ProcessShell,
    #[serde(flatten)]
    #[tool(flatten_shape)]
    command: ShellCommandInput,
    #[serde(default)]
    background: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInputShape)]
#[tool_input(trim("process_id"), non_empty("process_id"))]
#[serde(deny_unknown_fields)]
pub(crate) struct ProcessLogsToolArgs {
    process_id: String,
    #[serde(default)]
    since_seq: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    limit: Option<u32>,
    #[serde(default)]
    wait_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInputShape)]
#[tool_input(trim("process_id"), non_empty("process_id"))]
#[serde(deny_unknown_fields)]
pub(crate) struct ProcessStopToolArgs {
    process_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, StaticToolSurface)]
#[tool_surface(
    tool = "process",
    description = "Run one shell process or manage background process logs for the current session. Use action `run`, `list`, `logs`, or `stop`.",
    summary = "Run commands and manage background processes.",
    help = "Use action `run` to execute a command. Set `background = true` to keep it attached to the session and receive a `process_id`. Use `list` to inspect active background processes, `logs` to tail buffered output, and `stop` to terminate a background process.",
    display = detailed,
    tags(ToolTag::Mutating, ToolTag::Shell),
    concurrency_safe = false
)]
#[serde(tag = "action", rename_all = "snake_case")]
pub(crate) enum ProcessToolSurfaceInput {
    #[tool(exec = "run")]
    Run {
        #[serde(flatten)]
        #[tool(flatten_shape)]
        args: ProcessRunToolArgs,
    },
    #[tool(exec = "list")]
    List,
    #[tool(exec = "logs")]
    Logs {
        #[serde(flatten)]
        #[tool(flatten_shape)]
        args: ProcessLogsToolArgs,
    },
    #[tool(exec = "stop")]
    Stop {
        #[serde(flatten)]
        #[tool(flatten_shape)]
        args: ProcessStopToolArgs,
    },
}

#[async_trait]
impl Plugin for ProcessPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::builder("agena", "process", env!("CARGO_PKG_VERSION"))
            .description("Command execution and background process tools.")
            .brief_detailed()
            .tools(ProcessToolSurfaceInput::tool_definitions())
            .build()
    }

    async fn tool_invoke(
        &self,
        input: crate::plugin::sdk::ToolInvokeInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let parsed = ProcessToolSurfaceInput::parse_tool(input.tool_name.as_str(), input.input)?;
        let process_input = into_process_tool_input(parsed);
        router::invoke_tool(
            "process",
            json_input(process_input)?,
            input.session_id,
            input.call_id,
        )
    }

    async fn permission_paths(
        &self,
        tool: &str,
        input: &serde_json::Value,
    ) -> SdkResult<Vec<PathRequest>> {
        let parsed = ProcessToolSurfaceInput::parse_tool(tool, input.clone())?;
        router::permission_paths_for("process", &json_input(into_process_tool_input(parsed))?)
    }

    async fn permission_networks(
        &self,
        tool: &str,
        input: &serde_json::Value,
    ) -> SdkResult<Vec<NetworkRequest>> {
        let parsed = ProcessToolSurfaceInput::parse_tool(tool, input.clone())?;
        router::permission_networks_for("process", &json_input(into_process_tool_input(parsed))?)
    }
}

fn into_process_tool_input(input: ProcessToolSurfaceInput) -> ProcessToolInput {
    match input {
        ProcessToolSurfaceInput::Run { args } => ProcessToolInput::Run {
            shell: args.shell,
            command: args.command,
            background: args.background,
        },
        ProcessToolSurfaceInput::List => ProcessToolInput::List {},
        ProcessToolSurfaceInput::Logs { args } => ProcessToolInput::Logs {
            process_id: args.process_id,
            since_seq: args.since_seq,
            limit: args.limit,
            wait_ms: args.wait_ms,
        },
        ProcessToolSurfaceInput::Stop { args } => ProcessToolInput::Stop {
            process_id: args.process_id,
        },
    }
}

fn json_input<T: Serialize>(input: T) -> SdkResult<serde_json::Value> {
    serde_json::to_value(input).map_err(|err| PluginError::invalid_params(err.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tool_surface_routes_run_and_logs_variants() {
        let (run_tool, run_input) = ProcessToolSurfaceInput::resolve_tool(
            "process",
            json!({
                "action": "run",
                "command": "  echo ok  ",
                "description": "  smoke  ",
                "filesystem_effects": [],
                "network_effects": []
            }),
        )
        .expect("process run should resolve");
        assert_eq!(run_tool, "run");
        assert_eq!(
            run_input,
            json!({
                "shell": "bash",
                "command": "echo ok",
                "description": "smoke",
                "filesystem_effects": [],
                "network_effects": [],
                "background": false
            })
        );

        let (logs_tool, logs_input) = ProcessToolSurfaceInput::resolve_tool(
            "process",
            json!({
                "action": "logs",
                "process_id": " proc_1 ",
                "since_seq": 5,
                "wait_ms": 20
            }),
        )
        .expect("process logs should resolve");
        assert_eq!(logs_tool, "logs");
        assert_eq!(
            logs_input,
            json!({
                "process_id": "proc_1",
                "since_seq": 5,
                "wait_ms": 20
            })
        );
    }

    #[test]
    fn process_schema_usage_includes_background_flow() {
        let usage = crate::tool::definition::schema_usage_text(
            &ProcessToolSurfaceInput::tool_definition()
                .contract
                .input_schema,
        )
        .expect("usage text");
        assert!(usage.contains("Actions:"));
        assert!(usage.contains("- run"));
        assert!(usage.contains("- logs"));
        assert!(usage.contains("- stop"));
        assert!(usage.contains("process_id"));
        assert!(usage.contains("background"));
    }
}
