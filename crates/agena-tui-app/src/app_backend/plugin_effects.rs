//! Plugin-driven TUI effects: display contributions, notifications, theme
//! palettes, statuses, slash commands and their effect values.

use agena_api::commands::UpsertPermissionRuleParams;
use agena_api::resource::PermissionRuleResource;
use agena_application::Application;
use anyhow::{Context, Result, anyhow};
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
/// every frame by the status line and terminal title.
pub(crate) fn plugin_display_contributions(
    application: &Application,
) -> Vec<agena_plugin_host::HostDisplayContribution> {
    application.plugin_runtime().display_contributions()
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
    application: &Application,
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

/// Plugin notifications emitted through the unified `host.notify` entry
/// (Phase 6). Bounded recent queue; the TUI dedupes/consumes each intent.
pub(crate) fn plugin_host_notifications(
    application: &Application,
) -> Vec<agena_plugin_host::HostNotification> {
    application.plugin_runtime().host_notifications()
}

/// Human-readable workspace name derived from the workspace root's file name.
pub(crate) fn workspace_name(application: &Application) -> String {
    let workspace_root = application.workspace_root();
    workspace_root
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_owned)
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| workspace_root.display().to_string())
}

/// Theme palettes contributed by plugins. Synchronous: applied at startup and
/// whenever the runtime reloads.
pub(crate) fn plugin_theme_palettes(
    application: &Application,
) -> Vec<agena_plugin_host::HostThemePalette> {
    application.plugin_runtime().theme_palettes()
}

pub(crate) fn plugin_statuses(
    application: &Application,
) -> Vec<agena_plugin_host::status::PluginStatus> {
    application.plugin_runtime().plugin_statuses()
}

pub(crate) fn plugin_inspect(
    application: &Application,
    plugin_id: &str,
) -> Option<agena_plugin_host::PluginInspect> {
    application.plugin_runtime().plugin_inspect(plugin_id)
}

pub(crate) fn plugin_logs(
    application: &Application,
    plugin_id: &str,
    after_seq: Option<u64>,
    limit: usize,
) -> Vec<agena_plugin_host::PluginLogRecord> {
    application
        .plugin_runtime()
        .plugin_logs(plugin_id, after_seq, limit)
}

pub(crate) fn plugin_slash_commands(
    application: &Application,
) -> Vec<agena_plugin_host::PluginCommandCatalogItem> {
    application
        .plugin_runtime()
        .studio_commands()
        .into_iter()
        .filter(|entry| {
            entry
                .command
                .slash
                .as_deref()
                .is_some_and(|slash| !slash.trim().trim_start_matches('/').is_empty())
        })
        .collect()
}

/// Invoke a plugin command from a `/` slash command, resolving its effect
/// (message, prompt, workbench, URL, or nested tool/command invocations) with
/// a bounded recursion depth.
pub(crate) async fn invoke_plugin_slash_command(
    application: &Application,
    entry: &agena_plugin_host::PluginCommandCatalogItem,
    session_id: Option<i64>,
    raw: &str,
) -> Result<PluginCommandEffect> {
    const MAX_COMMAND_DEPTH: usize = 8;

    let plugin_id = entry.plugin_id.to_string();
    let slash = entry.command.slash.clone();
    let mut action = entry.command.action.clone();
    let mut input = plugin_command_input(&entry.command, raw)?;
    let mut depth = 0usize;

    loop {
        if depth > MAX_COMMAND_DEPTH {
            return Err(anyhow!("plugin command recursion limit exceeded"));
        }

        match action {
            agena_plugin_host::PluginUiAction::None => return Ok(PluginCommandEffect::None),
            agena_plugin_host::PluginUiAction::SubmitPrompt { prompt } => {
                return Ok(PluginCommandEffect::SubmitPrompt(prompt));
            }
            agena_plugin_host::PluginUiAction::OpenPluginWorkbench { tab } => {
                return Ok(PluginCommandEffect::OpenPluginWorkbench { plugin_id, tab });
            }
            agena_plugin_host::PluginUiAction::OpenUrl { url } => {
                return Ok(PluginCommandEffect::OpenUrl(url));
            }
            agena_plugin_host::PluginUiAction::InvokeTool {
                tool,
                input: base_input,
                submit_output_as_prompt,
            } => {
                let output = invoke_plugin_workbench_tool(
                    application,
                    plugin_id.as_str(),
                    tool.as_str(),
                    merge_plugin_command_input(base_input, Some(input)),
                    session_id,
                )
                .await?;
                if output.trim().is_empty() {
                    return Ok(PluginCommandEffect::None);
                }
                return if submit_output_as_prompt {
                    Ok(PluginCommandEffect::SubmitPrompt(output))
                } else {
                    Ok(PluginCommandEffect::Message(output))
                };
            }
            agena_plugin_host::PluginUiAction::InvokeCommand {
                command,
                input: base_input,
            } => {
                let session_id = session_id.ok_or_else(|| {
                    anyhow!("plugin command invocation requires an active session")
                })?;
                let output = application
                    .session_execution_services()
                    .map_err(|error| anyhow!(error.to_string()))?
                    .plugin_commands
                    .invoke_session_plugin_command(agena_runtime::SessionPluginCommandRequest {
                        session_id,
                        plugin_id: plugin_id.clone(),
                        command_id: command.clone(),
                        input: merge_plugin_command_input(base_input, Some(input)),
                        slash: slash.clone(),
                        raw: raw.to_string(),
                        workspace_root: Some(
                            application.workspace_root().to_string_lossy().into_owned(),
                        ),
                    })
                    .await
                    .map_err(|error| anyhow!(error.to_string()))?;

                match output {
                    agena_plugin_host::PluginCommandOutput::None => {
                        return Ok(PluginCommandEffect::None);
                    }
                    agena_plugin_host::PluginCommandOutput::Message { text } => {
                        return Ok(PluginCommandEffect::Message(text));
                    }
                    agena_plugin_host::PluginCommandOutput::SubmitPrompt { prompt } => {
                        return Ok(PluginCommandEffect::SubmitPrompt(prompt));
                    }
                    agena_plugin_host::PluginCommandOutput::OpenPluginWorkbench { tab } => {
                        return Ok(PluginCommandEffect::OpenPluginWorkbench { plugin_id, tab });
                    }
                    agena_plugin_host::PluginCommandOutput::OpenUrl { url } => {
                        return Ok(PluginCommandEffect::OpenUrl(url));
                    }
                    agena_plugin_host::PluginCommandOutput::InvokeTool {
                        tool,
                        input: next_input,
                        submit_output_as_prompt,
                    } => {
                        action = agena_plugin_host::PluginUiAction::InvokeTool {
                            tool,
                            input: next_input,
                            submit_output_as_prompt,
                        };
                        input = serde_json::json!({});
                    }
                    agena_plugin_host::PluginCommandOutput::InvokeCommand {
                        command,
                        input: next_input,
                    } => {
                        action = application
                            .plugin_runtime()
                            .resolve_studio_action(plugin_id.as_str(), command.as_str())
                            .unwrap_or(agena_plugin_host::PluginUiAction::InvokeCommand {
                                command,
                                input: next_input.clone(),
                            });
                        input = next_input.unwrap_or_else(|| serde_json::json!({}));
                    }
                }
                depth += 1;
            }
        }
    }
}

/// Invoke a plugin Tool API endpoint from a user-driven TUI surface, returning
/// the human-readable output text.
pub(crate) async fn invoke_plugin_workbench_tool(
    application: &Application,
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
    application: &Application,
    params: UpsertPermissionRuleParams,
) -> Result<PermissionRuleResource> {
    let UpsertPermissionRuleParams {
        action_key,
        subject_kind,
        tool_name,
        qualifier,
        path_access_kind,
        workspace_root,
        target_path,
        network_target,
        network_host,
        network_port,
        scope,
        session_id,
        mode,
    } = params;
    application
        .service()
        .create_permission_rule(agena_application::dto::PermissionRuleWriteRequest {
            action_key,
            subject_kind,
            tool_name,
            qualifier,
            path_access_kind,
            workspace_root,
            target_path,
            network_target,
            network_host,
            network_port,
            scope,
            session_id,
            mode,
        })
        .await
        .map_err(anyhow::Error::new)
        .context("failed to create permission rule")
}

pub(crate) async fn replace_permission_rule(
    application: &Application,
    rule_id: i64,
    params: UpsertPermissionRuleParams,
) -> Result<PermissionRuleResource> {
    let UpsertPermissionRuleParams {
        action_key,
        subject_kind,
        tool_name,
        qualifier,
        path_access_kind,
        workspace_root,
        target_path,
        network_target,
        network_host,
        network_port,
        scope,
        session_id,
        mode,
    } = params;
    application
        .service()
        .replace_permission_rule(
            rule_id,
            agena_application::dto::PermissionRuleWriteRequest {
                action_key,
                subject_kind,
                tool_name,
                qualifier,
                path_access_kind,
                workspace_root,
                target_path,
                network_target,
                network_host,
                network_port,
                scope,
                session_id,
                mode,
            },
        )
        .await
        .map_err(anyhow::Error::new)
        .context("failed to replace permission rule")
}

pub(crate) async fn revoke_permission_rule(
    application: &Application,
    rule_id: i64,
) -> Result<PermissionRuleResource> {
    application
        .service()
        .revoke_permission_rule(rule_id, None)
        .await
        .map_err(anyhow::Error::new)
        .context("failed to revoke permission rule")
}

pub(crate) async fn create_commit(
    application: &Application,
    message: String,
) -> Result<(String, String)> {
    let commit = application
        .git_commit(agena_application::dto::GitCommitRequest { message })
        .await
        .map_err(|error| anyhow!(error.to_string()))?;
    Ok((commit.commit, commit.summary))
}

pub(crate) async fn create_pr(
    application: &Application,
    title: String,
    body: Option<String>,
    base: Option<String>,
    head: Option<String>,
) -> Result<String> {
    let pull_request = application
        .git_create_pull_request(agena_application::dto::GitPullRequestCreateRequest {
            title,
            body,
            base,
            head,
        })
        .await
        .map_err(|error| anyhow!(error.to_string()))?;
    Ok(pull_request.url)
}
