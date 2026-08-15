//! Plugin-driven TUI effects: display contributions, notifications, theme
//! palettes, statuses, slash commands and their effect values.

use agena_api::{
    commands::{
        Command, CommandResult, ReplacePermissionRuleParams, RevokePermissionRuleParams,
        UpsertPermissionRuleParams,
    },
    resource::PermissionRuleResource,
};
use anyhow::{Context, Result, anyhow, bail};
use serde_json::json;

/// Effect of a plugin command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginCommandEffect {
    None,
    Message(String),
    SubmitPrompt(String),
    OpenPluginWorkbench {
        plugin_id: String,
        tab: Option<String>,
    },
    OpenUrl(String),
}

fn merge_plugin_command_input(
    base: Option<serde_json::Value>,
    overlay: Option<serde_json::Value>,
) -> serde_json::Value {
    match (base, overlay) {
        (Some(serde_json::Value::Object(mut base)), Some(serde_json::Value::Object(overlay))) => {
            base.extend(overlay);
            serde_json::Value::Object(base)
        }
        (_, Some(value)) => value,
        (Some(value), None) => value,
        (None, None) => json!({}),
    }
}

fn parse_plugin_command_literal(
    raw: &str,
    schema: Option<&serde_json::Value>,
) -> serde_json::Value {
    let parsed =
        serde_json::from_str(raw).unwrap_or_else(|_| serde_json::Value::String(raw.into()));
    let Some(expected) = schema
        .and_then(serde_json::Value::as_object)
        .and_then(|schema| schema.get("type"))
    else {
        return parsed;
    };
    let matches_type = |kind: &str| match kind {
        "string" => parsed.is_string(),
        "integer" => parsed.as_i64().is_some() || parsed.as_u64().is_some(),
        "number" => parsed.is_number(),
        "boolean" => parsed.is_boolean(),
        "object" => parsed.is_object(),
        "array" => parsed.is_array(),
        "null" => parsed.is_null(),
        _ => true,
    };
    let accepted = match expected {
        serde_json::Value::String(kind) => matches_type(kind),
        serde_json::Value::Array(kinds) => kinds
            .iter()
            .filter_map(serde_json::Value::as_str)
            .any(matches_type),
        _ => true,
    };
    if accepted {
        parsed
    } else {
        serde_json::Value::String(raw.into())
    }
}

fn plugin_command_input(
    command: &agena_plugin_host::PluginCommandDefinition,
    raw: &str,
) -> Result<serde_json::Value> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(json!({}));
    }
    let Some(schema) = command.input_schema.as_ref() else {
        return Ok(json!({ "args": raw }));
    };
    let schema_object = schema.as_object();
    let schema_type = schema_object
        .and_then(|schema| schema.get("type"))
        .and_then(serde_json::Value::as_str);
    let properties = schema_object
        .and_then(|schema| schema.get("properties"))
        .and_then(serde_json::Value::as_object);

    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(raw) {
        if schema_type != Some("object") || parsed.is_object() {
            return Ok(parsed);
        }
        if let Some(properties) = properties
            && properties.len() == 1
        {
            let (name, _) = properties.iter().next().expect("one command property");
            return Ok(json!({ (name): parsed }));
        }
    }

    if schema_type == Some("object")
        && let Some(properties) = properties
    {
        if properties.len() == 1 {
            let (name, property_schema) = properties.iter().next().expect("one command property");
            return Ok(json!({
                (name): parse_plugin_command_literal(raw, Some(property_schema)),
            }));
        }

        let mut aliases = std::collections::HashMap::<&str, &str>::new();
        for (name, property_schema) in properties {
            if let Some(values) = property_schema
                .get("x-agena-aliases")
                .and_then(serde_json::Value::as_array)
            {
                for alias in values.iter().filter_map(serde_json::Value::as_str) {
                    aliases.insert(alias, name.as_str());
                }
            }
        }
        let mut output = serde_json::Map::new();
        for token in raw.split_whitespace() {
            let Some((raw_name, value)) = token.split_once('=') else {
                output.clear();
                break;
            };
            let name = if properties.contains_key(raw_name) {
                raw_name
            } else if let Some(name) = aliases.get(raw_name) {
                name
            } else {
                output.clear();
                break;
            };
            output.insert(
                name.to_string(),
                parse_plugin_command_literal(value, properties.get(name)),
            );
        }
        if !output.is_empty() {
            return Ok(serde_json::Value::Object(output));
        }
    }

    let literal = parse_plugin_command_literal(raw, Some(schema));
    if !literal.is_string() || schema_type == Some("string") {
        return Ok(literal);
    }
    Ok(json!({ "args": raw }))
}

/// Display contributions published by loaded plugins. Synchronous: consumed
/// every frame by the status line and terminal title. In remote client mode
/// these come from the cached plugin UI catalog fetched from the center.
pub(crate) fn plugin_display_contributions(
    application: &super::TuiBackend,
) -> Vec<agena_plugin_host::HostDisplayContribution> {
    application
        .plugin_catalog()
        .map(|catalog| catalog.tui.display)
        .unwrap_or_default()
}

/// Re-publish the plan progress display contribution for `session_id`.
///
/// The composer's bottom-right plan chip is backed by an in-memory display
/// contribution that starts empty after a process restart or a runtime
/// reload. Invoking `agena.plan.get` re-syncs the contribution from durable
/// storage (the planning plugin re-publishes on every plan read) without
/// mutating the plan. Returns `true` when the session has an active plan, so
/// the caller can back off for sessions without one.
pub(crate) async fn refresh_plan_display(
    application: &super::TuiBackend,
    session_id: i64,
) -> Result<bool> {
    let response = application
        .invoke_plugin_ui_tool(
            "agena.plan",
            "get",
            serde_json::json!({ "view": "summary" }),
            Some(session_id),
        )
        .await?;
    Ok(response
        .payload
        .as_ref()
        .and_then(|payload| payload.get("plan"))
        .is_some_and(|plan| !plan.is_null()))
}

/// Plugin notifications emitted through the unified `host.notify` entry.
/// Notifications are push events; no processing-center HTTP endpoint exposes
/// the host's in-memory notification queue, so remote client mode degrades to
/// an empty queue.
pub(crate) fn plugin_host_notifications(
    application: &super::TuiBackend,
) -> Vec<agena_plugin_host::HostNotification> {
    let _ = application;
    Vec::new()
}

/// Human-readable workspace name derived from the workspace root's file name.
pub(crate) fn workspace_name(application: &super::TuiBackend) -> String {
    let workspace_root = application.workspace_root();
    workspace_root
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_owned)
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| workspace_root.display().to_string())
}

/// Theme palettes contributed by plugins. Synchronous: applied at startup and
/// whenever the runtime reloads. In remote client mode these come from the
/// cached plugin UI catalog fetched from the center.
pub(crate) fn plugin_theme_palettes(
    application: &super::TuiBackend,
) -> Vec<agena_plugin_host::HostThemePalette> {
    application
        .plugin_catalog()
        .map(|catalog| catalog.tui.themes)
        .unwrap_or_default()
}

pub(crate) fn plugin_statuses(
    application: &super::TuiBackend,
) -> Vec<agena_plugin_host::status::PluginStatus> {
    application.plugin_statuses()
}

pub(crate) fn plugin_inspect(
    application: &super::TuiBackend,
    plugin_id: &str,
) -> Option<agena_plugin_host::PluginInspect> {
    let _ = (application, plugin_id);
    // `PluginInspect` is a Serialize-only host DTO whose members are not all
    // deserializable; no public center response reproduces it. Degrade to
    // None in remote client mode.
    None
}

pub(crate) fn plugin_logs(
    application: &super::TuiBackend,
    plugin_id: &str,
    after_seq: Option<u64>,
    limit: usize,
) -> Vec<agena_plugin_host::PluginLogRecord> {
    let _ = (application, plugin_id, after_seq, limit);
    // Log reading is synchronous in the TUI event loop and the workbench has
    // no async load path; degrade to an empty log view in remote client mode.
    Vec::new()
}

pub(crate) fn plugin_slash_commands(
    application: &super::TuiBackend,
) -> Vec<agena_plugin_host::PluginCommandCatalogItem> {
    application
        .plugin_catalog()
        .map(|catalog| {
            catalog
                .studio
                .commands
                .into_iter()
                .filter(|entry| {
                    entry
                        .command
                        .slash
                        .as_deref()
                        .is_some_and(|slash| !slash.trim().trim_start_matches('/').is_empty())
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Invoke a plugin command from a `/` slash command, resolving its effect
/// (message, prompt, workbench, URL, or a tool invocation) over HTTP.
///
/// Nested in-process command dispatch has no public center endpoint and is
/// refused with a clear error.
pub(crate) async fn invoke_plugin_slash_command(
    application: &super::TuiBackend,
    entry: &agena_plugin_host::PluginCommandCatalogItem,
    session_id: Option<i64>,
    raw: &str,
) -> Result<PluginCommandEffect> {
    let backend = application;

    let plugin_id = entry.plugin_id.to_string();
    let action = entry.command.action.clone();
    let input = plugin_command_input(&entry.command, raw)?;

    match action {
        agena_plugin_host::PluginUiAction::None => Ok(PluginCommandEffect::None),
        agena_plugin_host::PluginUiAction::SubmitPrompt { prompt } => {
            Ok(PluginCommandEffect::SubmitPrompt(prompt))
        }
        agena_plugin_host::PluginUiAction::OpenPluginWorkbench { tab } => {
            Ok(PluginCommandEffect::OpenPluginWorkbench { plugin_id, tab })
        }
        agena_plugin_host::PluginUiAction::OpenUrl { url } => {
            Ok(PluginCommandEffect::OpenUrl(url))
        }
        agena_plugin_host::PluginUiAction::InvokeTool {
            tool,
            input: base_input,
            submit_output_as_prompt,
        } => {
            let output = invoke_plugin_workbench_tool(
                backend,
                plugin_id.as_str(),
                tool.as_str(),
                merge_plugin_command_input(base_input, Some(input)),
                session_id,
            )
            .await?;
            if output.trim().is_empty() {
                return Ok(PluginCommandEffect::None);
            }
            Ok(if submit_output_as_prompt {
                PluginCommandEffect::SubmitPrompt(output)
            } else {
                PluginCommandEffect::Message(output)
            })
        }
        agena_plugin_host::PluginUiAction::InvokeCommand { .. } => Err(anyhow!(
            "nested plugin command dispatch is unavailable in remote TUI mode until it has a public center API"
        )),
    }
}

/// Invoke a plugin Tool API endpoint from a user-driven TUI surface, returning
/// the human-readable output text.
pub(crate) async fn invoke_plugin_workbench_tool(
    application: &super::TuiBackend,
    plugin_id: &str,
    tool_name: &str,
    input: serde_json::Value,
    session_id: Option<i64>,
) -> Result<String> {
    let response = application
        .invoke_plugin_ui_tool(plugin_id, tool_name, input, session_id)
        .await?;
    Ok(response.output_text)
}

pub(crate) async fn create_permission_rule(
    application: &super::TuiBackend,
    params: UpsertPermissionRuleParams,
) -> Result<PermissionRuleResource> {
    let result = application
        .client()
        .command(Command::UpsertPermissionRule(params))
        .await?;
    let CommandResult::PermissionRule(rule) = result else {
        bail!("processing center returned the wrong permission-rule result");
    };
    Ok(rule)
}

pub(crate) async fn replace_permission_rule(
    application: &super::TuiBackend,
    rule_id: i64,
    params: UpsertPermissionRuleParams,
) -> Result<PermissionRuleResource> {
    let result = application
        .client()
        .command(Command::ReplacePermissionRule(ReplacePermissionRuleParams {
            rule_id,
            rule: params,
        }))
        .await?;
    let CommandResult::PermissionRule(rule) = result else {
        bail!("processing center returned the wrong permission-rule result");
    };
    Ok(rule)
}

pub(crate) async fn revoke_permission_rule(
    application: &super::TuiBackend,
    rule_id: i64,
) -> Result<PermissionRuleResource> {
    let result = application
        .client()
        .command(Command::RevokePermissionRule(RevokePermissionRuleParams {
            rule_id,
            reason: None,
        }))
        .await?;
    let CommandResult::PermissionRule(rule) = result else {
        bail!("processing center returned the wrong permission-rule result");
    };
    Ok(rule)
}

pub(crate) async fn create_commit(
    application: &super::TuiBackend,
    message: String,
) -> Result<(String, String)> {
    let commit: agena_application::dto::GitCommitResource = serde_json::from_value(
        application.client().create_git_commit(message).await?,
    )
    .context("the center returned an undecodable git commit result")?;
    Ok((commit.commit, commit.summary))
}

pub(crate) async fn create_pr(
    application: &super::TuiBackend,
    title: String,
    body: Option<String>,
    base: Option<String>,
    head: Option<String>,
) -> Result<String> {
    let pull_request: agena_application::dto::GitPullRequestResource = serde_json::from_value(
        application
            .client()
            .create_git_pull_request(title, body, base, head)
            .await?,
    )
    .context("the center returned an undecodable git pull-request result")?;
    Ok(pull_request.url)
}
