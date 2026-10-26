use agena_domain::{StructuredObject, ToolApiCall, ToolApiFunction};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};

use crate::{CatalogTool, ToolApiCatalog};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolApiResponse {
    pub title: String,
    pub summary: String,
    pub output_text: String,
}

/// The sole output that can cross from the protocol into execution. It has
/// no plugin identity, execution handle, permission verdict, or hook state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionRequest {
    pub tool: String,
    pub input: StructuredObject,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolApiAction {
    Respond(ToolApiResponse),
    Execute(ExecutionRequest),
}

#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
#[error("{0}")]
pub struct ToolApiError(pub String);

impl ToolApiError {
    pub fn failure(&self) -> agena_failure::Failure {
        use agena_failure::*;
        Failure::new(
            FailureCode::new("tool_api.invalid_request"),
            FailureCategory::InvalidInput,
            FailureResponsibility::Caller,
            RetryDirective::CorrectInput,
            RecoveryDirective::None,
            FailureImpact::RequestRejected,
            UserPresentation::validated("tool_api.invalid_request", self.0.clone()),
        )
        .with_model_feedback(ModelFeedback::invalid_input().with_text(self.0.clone()))
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(untagged)]
pub(crate) enum StringBatch {
    One(#[schemars(length(min = 1))] String),
    Many(#[schemars(length(min = 1))] Vec<String>),
}

impl StringBatch {
    fn values(&self) -> Result<&[String], ToolApiError> {
        let values = match self {
            Self::One(value) => std::slice::from_ref(value),
            Self::Many(values) => values,
        };
        if values.is_empty() || values.iter().any(|value| value.trim().is_empty()) {
            return Err(ToolApiError(
                "selectors must contain at least one non-empty name".into(),
            ));
        }
        Ok(values)
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ListInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    offset: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    limit: Option<u32>,
    /// One plugin id or a non-empty array of plugin ids, with OR semantics.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    plugin: Option<StringBatch>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct HelpInput {
    /// One exact execution-tool name or a non-empty array of exact names.
    tool: StringBatch,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct CallInput {
    /// Exact execution-tool name from the live catalog; never a protocol function.
    #[schemars(length(min = 1))]
    tool: String,
    /// One complete JSON object matching the execution tool's input contract.
    input: serde_json::Map<String, Value>,
}

fn parse<T: DeserializeOwned>(call: &ToolApiCall) -> Result<T, ToolApiError> {
    serde_json::from_value(Value::from(call.arguments.clone()))
        .map_err(|error| ToolApiError(format!("invalid {} arguments: {error}", call.function)))
}

impl ToolApiCatalog {
    pub fn dispatch(&self, call: &ToolApiCall) -> Result<ToolApiAction, ToolApiError> {
        match call.function {
            ToolApiFunction::Call => {
                let input: CallInput = parse(call)?;
                let tool = input.tool.trim();
                if tool.is_empty()
                    || ToolApiFunction::from_function_name(tool).is_some()
                    || agena_domain::is_control_function_name(tool)
                    || tool.starts_with("agena.tools.")
                {
                    return Err(ToolApiError("tools_call.tool must name an execution tool; protocol and control functions cannot be invoked through tools_call".into()));
                }
                Ok(ToolApiAction::Execute(ExecutionRequest {
                    tool: tool.to_owned(),
                    input: StructuredObject::try_from(Value::Object(input.input))
                        .map_err(ToolApiError)?,
                }))
            }
            ToolApiFunction::List | ToolApiFunction::PluginsList => {
                let input: ListInput = parse(call)?;
                let filters = input.plugin.as_ref().map(StringBatch::values).transpose()?;
                let offset = input.offset.unwrap_or(0) as usize;
                let limit = input.limit.unwrap_or(20).max(1) as usize;
                let matches = |id: &str| {
                    filters.is_none_or(|filters| {
                        filters.iter().any(|filter| {
                            id == filter.trim() || id.rsplit('.').next() == Some(filter.trim())
                        })
                    })
                };
                let (kind, rows) = if call.function == ToolApiFunction::List {
                    (
                        "tools",
                        self.tools
                            .iter()
                            .filter(|tool| matches(&tool.plugin_id))
                            .map(|tool| {
                                format!(
                                    "- {} [{}] ({}): {}",
                                    tool.name,
                                    tool.tags.join(", "),
                                    tool.plugin_id,
                                    compact(&tool.summary)
                                )
                            })
                            .collect::<Vec<_>>(),
                    )
                } else {
                    (
                        "plugins",
                        self.plugins
                            .iter()
                            .filter(|plugin| matches(&plugin.id))
                            .map(|plugin| {
                                format!(
                                    "- {} [{}] (v{}): {} · {} tool(s)",
                                    plugin.id,
                                    plugin.tags.join(", "),
                                    plugin.version,
                                    compact(&plugin.summary),
                                    plugin.tool_count
                                )
                            })
                            .collect::<Vec<_>>(),
                    )
                };
                let total = rows.len();
                let page = rows
                    .into_iter()
                    .skip(offset)
                    .take(limit)
                    .collect::<Vec<_>>();
                let returned = page.len();
                let mut text = format!(
                    "Available {kind}: returned {returned} of {total} starting at offset {offset}.\n{}",
                    page.join("\n")
                );
                let next = offset.saturating_add(returned);
                if returned > 0 && next < total {
                    text.push_str(&format!(
                        "\nMore available: yes. Continue with `{}` using `offset: {next}`.",
                        call.function
                    ));
                } else {
                    text.push_str("\nMore available: no.");
                }
                if call.function == ToolApiFunction::PluginsList {
                    text.push_str(
                        "\nList a plugin's tools with `tools_list` and its `plugin` selector.",
                    );
                }
                Ok(ToolApiAction::Respond(ToolApiResponse {
                    title: format!("List {kind} · {returned}/{total}"),
                    summary: format!("Returned {returned} of {total} {kind}."),
                    output_text: text,
                }))
            }
            ToolApiFunction::Help => {
                let input: HelpInput = parse(call)?;
                let names = input.tool.values()?;
                let mut outputs = Vec::new();
                let mut failures = 0usize;
                for name in names {
                    let found = self
                        .tools
                        .iter()
                        .filter(|tool| tool.name == *name || tool.canonical_name == *name)
                        .collect::<Vec<_>>();
                    match found.as_slice() {
                        [tool] => outputs.push(self.render_help(tool)),
                        _ => {
                            let message = format!(
                                "Tool `{name}` is unavailable or ambiguous. Enumerate plugins_list, then tools_list for the owning plugin, and use an exact returned identifier."
                            );
                            if names.len() == 1 {
                                return Err(ToolApiError(message));
                            }
                            failures += 1;
                            outputs.push(message);
                        }
                    }
                }
                Ok(ToolApiAction::Respond(ToolApiResponse {
                    title: format!("Inspect {} tools", names.len()),
                    summary: format!(
                        "{} tool contracts returned · {failures} unavailable",
                        names.len() - failures
                    ),
                    output_text: outputs.join("\n\n---\n\n"),
                }))
            }
        }
    }

    fn render_help(&self, tool: &CatalogTool) -> String {
        let mut text = format!(
            "Tool: {}\n{}\nTags: {}",
            tool.name,
            tool.summary,
            tool.tags.join(", ")
        );
        if tool.plugin_id == "agena.shell" {
            let shell_tools = [
                ("exec", "wait for command completion"),
                ("spawn", "background command and completion notification"),
                ("watch", "service readiness and background process watch"),
                ("open", "interactive shell/terminal/PTY"),
                ("logs", "explicit-cursor compatibility output"),
                ("read", "job/terminal output without input"),
                ("write", "terminal input"),
                ("stop", "process-tree cleanup"),
                ("signal", "terminal interruption/control"),
                ("list", "owned jobs/terminals"),
                ("resize", "terminal dimensions"),
            ]
            .into_iter()
            .filter_map(|(action, purpose)| {
                let canonical = format!("agena.shell.{action}");
                self.tools
                    .iter()
                    .find(|tool| tool.canonical_name == canonical)
                    .map(|tool| format!("`{}` ({purpose})", tool.name))
            })
            .collect::<Vec<_>>();
            if !shell_tools.is_empty() {
                text.push_str(&format!("\nAvailable Shell tools: {}. Read detailed contracts with tools_help (batch supported).", shell_tools.join("; ")));
            }
        }
        if let Some(usage) = &tool.input_usage {
            text.push_str(&format!("\nUsage:\n{usage}"));
        }
        if !tool.input_examples.is_empty() {
            text.push_str("\nExamples:");
            for example in &tool.input_examples {
                text.push_str(&format!("\n- {example}"));
            }
        }
        text.push_str(&format!(
            "\nHelp:\n{}\nInput schema:\n{}\nOutput schema:\n{}\nTo run this execution tool, call tools_call with tool={} and input set to one complete JSON object matching the input schema. Supply all required fields with the user's exact task values; never run an empty or default-input probe. This contract is reusable.",
            tool.help, tool.input_schema, tool.output_schema, json!(tool.name),
        ));
        text
    }
}

fn compact(text: &str) -> String {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.chars().count() > 160 {
        format!("{}…", text.chars().take(159).collect::<String>())
    } else {
        text
    }
}
