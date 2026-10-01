use std::collections::BTreeMap;
use std::path::PathBuf;

pub use agena_plugin_contracts::{CommandInvokeInput, CommandResult};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

// ── plugin command invocation ──────────────────────────────────────────────

#[derive(Debug, Clone, Copy)]
/// Context of a plugin command invocation.
pub struct PluginCommandContext<'a> {
    pub session_id: Option<i64>,
    pub call_id: Option<i64>,
    pub workspace_root: Option<&'a str>,
    pub command_id: &'a str,
    pub slash: Option<&'a str>,
    pub raw: &'a str,
}

impl<'a> PluginCommandContext<'a> {
    pub fn from_input(input: &'a CommandInvokeInput) -> Self {
        Self {
            session_id: input.session_id,
            call_id: input.call_id,
            workspace_root: input.workspace_root.as_deref(),
            command_id: input.command_id.as_str(),
            slash: input.slash.as_deref(),
            raw: input.raw.as_str(),
        }
    }

    pub fn parse_input<T>(input: &CommandInvokeInput) -> crate::Result<T>
    where
        T: DeserializeOwned,
    {
        serde_json::from_value(input.input.clone())
            .map_err(|err| crate::PluginError::invalid_params_error(&err))
    }
}

/// Conversion into the structured command result contract.
pub trait IntoCommandResult {
    fn into_command_result(self) -> crate::Result<CommandResult>;
}

pub fn into_command_result<T>(value: T) -> crate::Result<CommandResult>
where
    T: IntoCommandResult,
{
    value.into_command_result()
}

impl IntoCommandResult for CommandResult {
    fn into_command_result(self) -> crate::Result<CommandResult> {
        Ok(self)
    }
}

impl IntoCommandResult for () {
    fn into_command_result(self) -> crate::Result<CommandResult> {
        Ok(CommandResult::succeeded("No output"))
    }
}

impl IntoCommandResult for String {
    fn into_command_result(self) -> crate::Result<CommandResult> {
        Ok(CommandResult::succeeded(self))
    }
}

impl IntoCommandResult for &str {
    fn into_command_result(self) -> crate::Result<CommandResult> {
        Ok(CommandResult::succeeded(self))
    }
}

impl<T> IntoCommandResult for Option<T>
where
    T: IntoCommandResult,
{
    fn into_command_result(self) -> crate::Result<CommandResult> {
        match self {
            Some(value) => value.into_command_result(),
            None => Ok(CommandResult::succeeded("No output")),
        }
    }
}

impl<T, E> IntoCommandResult for std::result::Result<T, E>
where
    T: IntoCommandResult,
    E: Into<crate::PluginError>,
{
    fn into_command_result(self) -> crate::Result<CommandResult> {
        self.map_err(Into::into)?.into_command_result()
    }
}

// ── command.execute.before ─────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Input of a command-before hook.
pub struct CommandBeforeInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_root: Option<String>,
    pub command: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
}

/// What a plugin returns from `command.execute.before`.
///
/// - Return `None` (or omit the field) to pass through unchanged.
/// - Return `CommandBeforeResponse::Patch(…)` to mutate the command.
/// - Return `CommandBeforeResponse::Abort { reason }` to cancel execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum CommandBeforeResponse {
    Patch(CommandBeforePatch),
    Abort { reason: String },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
/// Patch applied to a command before execution.
pub struct CommandBeforePatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env: Option<BTreeMap<String, String>>,
}

/// Outcome of the `command.execute.before` dispatch, returned by
/// `PluginHost::dispatch_command_before_blocking`.
#[derive(Debug, Clone)]
pub enum CommandBeforeOutcome {
    Continue(CommandBeforeInput),
    Abort(String),
}

// ── command.execute.after ──────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Input of a command-after hook.
pub struct CommandAfterInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<i64>,
    pub command: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    #[serde(default)]
    pub timed_out: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
/// Patch applied to a command result after execution.
pub struct CommandAfterPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stdout: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stderr: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
}

#[cfg(test)]
mod tests {
    use agena_plugin_contracts::{CommandHostEffect, CommandResult};

    #[test]
    fn command_results_only_contain_controlled_host_effects() {
        let output =
            CommandResult::succeeded("done").with_effect(CommandHostEffect::RefreshPluginSurface {
                plugin_id: "example.plugin".to_string(),
            });
        let encoded = serde_json::to_value(output).expect("serialize command result");
        assert_eq!(encoded["status"], "succeeded");
        assert!(encoded["effects"][0]["kind"] == "refresh_plugin_surface");
        assert!(encoded.get("invoke_tool").is_none());
    }
}
