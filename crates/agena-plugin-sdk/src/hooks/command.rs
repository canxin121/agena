use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize, de::DeserializeOwned};

// ── plugin command invocation ─────────────────────────────────────────────

/// Input for an explicit plugin command or plugin UI action.
///
/// Commands are user-control and UI-routing operations and do not receive a
/// synthetic command-level permission check. Implementations must route any
/// protected filesystem, network, shell, credential, or other side effect
/// through a registered tool or a permission-enforcing Host API.

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginCommandInvokeInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_root: Option<String>,
    pub command_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slash: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub raw: String,
    #[serde(default)]
    pub input: serde_json::Value,
}

#[derive(Debug, Clone, Copy)]
pub struct PluginCommandContext<'a> {
    pub session_id: Option<i64>,
    pub call_id: Option<i64>,
    pub workspace_root: Option<&'a str>,
    pub command_id: &'a str,
    pub slash: Option<&'a str>,
    pub raw: &'a str,
}

impl PluginCommandInvokeInput {
    pub fn context(&self) -> PluginCommandContext<'_> {
        PluginCommandContext {
            session_id: self.session_id,
            call_id: self.call_id,
            workspace_root: self.workspace_root.as_deref(),
            command_id: self.command_id.as_str(),
            slash: self.slash.as_deref(),
            raw: self.raw.as_str(),
        }
    }

    pub fn parse_input<T>(&self) -> crate::Result<T>
    where
        T: DeserializeOwned,
    {
        serde_json::from_value(self.input.clone())
            .map_err(|err| crate::PluginError::invalid_params(err.to_string()))
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PluginCommandOutput {
    #[default]
    None,
    Message {
        text: String,
    },
    SubmitPrompt {
        prompt: String,
    },
    InvokeTool {
        /// A tool owned by the same plugin. The Host executes it through the
        /// normal tool permission contract; command invocation is not a
        /// permission bypass for the target tool or its resource effects.
        tool: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        input: Option<serde_json::Value>,
        #[serde(default)]
        submit_output_as_prompt: bool,
    },
    InvokeCommand {
        command: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        input: Option<serde_json::Value>,
    },
    OpenPluginWorkbench {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tab: Option<String>,
    },
    OpenUrl {
        url: String,
    },
}

impl PluginCommandOutput {
    pub fn message(text: impl Into<String>) -> Self {
        Self::Message { text: text.into() }
    }

    pub fn submit_prompt(prompt: impl Into<String>) -> Self {
        Self::SubmitPrompt {
            prompt: prompt.into(),
        }
    }

    pub fn invoke_tool(tool: impl Into<String>, input: Option<serde_json::Value>) -> Self {
        Self::InvokeTool {
            tool: tool.into(),
            input,
            submit_output_as_prompt: false,
        }
    }

    pub fn invoke_tool_with_prompt(
        tool: impl Into<String>,
        input: Option<serde_json::Value>,
        submit_output_as_prompt: bool,
    ) -> Self {
        Self::InvokeTool {
            tool: tool.into(),
            input,
            submit_output_as_prompt,
        }
    }

    pub fn invoke_command(command: impl Into<String>, input: Option<serde_json::Value>) -> Self {
        Self::InvokeCommand {
            command: command.into(),
            input,
        }
    }

    pub fn open_plugin_workbench(tab: Option<impl Into<String>>) -> Self {
        Self::OpenPluginWorkbench {
            tab: tab.map(Into::into),
        }
    }

    pub fn open_url(url: impl Into<String>) -> Self {
        Self::OpenUrl { url: url.into() }
    }
}

pub trait IntoPluginCommandOutput {
    fn into_plugin_command_output(self) -> crate::Result<PluginCommandOutput>;
}

pub fn into_plugin_command_output<T>(value: T) -> crate::Result<PluginCommandOutput>
where
    T: IntoPluginCommandOutput,
{
    value.into_plugin_command_output()
}

impl IntoPluginCommandOutput for PluginCommandOutput {
    fn into_plugin_command_output(self) -> crate::Result<PluginCommandOutput> {
        Ok(self)
    }
}

impl IntoPluginCommandOutput for () {
    fn into_plugin_command_output(self) -> crate::Result<PluginCommandOutput> {
        Ok(PluginCommandOutput::None)
    }
}

impl IntoPluginCommandOutput for String {
    fn into_plugin_command_output(self) -> crate::Result<PluginCommandOutput> {
        Ok(PluginCommandOutput::Message { text: self })
    }
}

impl IntoPluginCommandOutput for &str {
    fn into_plugin_command_output(self) -> crate::Result<PluginCommandOutput> {
        Ok(PluginCommandOutput::Message {
            text: self.to_string(),
        })
    }
}

impl<T> IntoPluginCommandOutput for Option<T>
where
    T: IntoPluginCommandOutput,
{
    fn into_plugin_command_output(self) -> crate::Result<PluginCommandOutput> {
        match self {
            Some(value) => value.into_plugin_command_output(),
            None => Ok(PluginCommandOutput::None),
        }
    }
}

impl<T, E> IntoPluginCommandOutput for std::result::Result<T, E>
where
    T: IntoPluginCommandOutput,
    E: Into<crate::PluginError>,
{
    fn into_plugin_command_output(self) -> crate::Result<PluginCommandOutput> {
        self.map_err(Into::into)?.into_plugin_command_output()
    }
}

// ── command.execute.before ─────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    use super::PluginCommandOutput;

    #[test]
    fn plugin_commands_can_only_deep_link_to_the_host_workbench() {
        let output = PluginCommandOutput::open_plugin_workbench(Some("tools"));
        let encoded = serde_json::to_value(output).expect("serialize workbench output");
        assert_eq!(encoded["kind"], "open_plugin_workbench");
        assert_eq!(encoded["tab"], "tools");

        let old_route = serde_json::json!({
            "kind": "open_route",
            "route": "agena://plugin-defined-page"
        });
        assert!(serde_json::from_value::<PluginCommandOutput>(old_route).is_err());
    }
}
