//! Plugin-driven TUI presentation and unified operation execution.

use agena_api::{
    commands::{
        Command, CommandResult, ReplacePermissionRuleParams, RevokePermissionRuleParams,
        UpsertPermissionRuleParams,
    },
    resource::PermissionRuleResource,
};
use anyhow::{Context, Result, anyhow, bail};

/// The TUI consumes the exact final result returned by the server-owned
/// operation resolver; it never follows plugin actions recursively.
pub type PluginOperationEffect = agena_plugin_host::PluginOperationResult;

pub(crate) fn plugin_display_contributions(
    application: &super::TuiBackend,
) -> Vec<agena_plugin_host::HostDisplayContribution> {
    application
        .plugin_catalog()
        .map(|catalog| catalog.terminal.display)
        .unwrap_or_default()
}

/// Re-publish the durable plan's passive display contribution.
pub(crate) async fn refresh_plan_display(
    application: &super::TuiBackend,
    session_id: i64,
) -> Result<bool> {
    let response = application
        .invoke_plugin_tool(
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

pub(crate) fn plugin_host_notifications(
    application: &super::TuiBackend,
) -> Vec<agena_plugin_host::HostNotification> {
    application.plugin_notifications()
}

pub(crate) fn workspace_name(application: &super::TuiBackend) -> String {
    let workspace_root = application.workspace_root();
    workspace_root
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_owned)
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| workspace_root.display().to_string())
}

pub(crate) fn plugin_theme_palettes(
    application: &super::TuiBackend,
) -> Vec<agena_plugin_host::HostThemePalette> {
    application
        .plugin_catalog()
        .map(|catalog| catalog.terminal.themes)
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
    application.plugin_inspect(plugin_id)
}

pub(crate) fn plugin_logs(
    application: &super::TuiBackend,
    plugin_id: &str,
    after_seq: Option<u64>,
    limit: usize,
) -> Vec<agena_plugin_host::PluginLogRecord> {
    application.plugin_logs(plugin_id, after_seq, limit)
}

pub(crate) fn plugin_slash_operations(
    application: &super::TuiBackend,
) -> Vec<agena_plugin_host::PluginOperationCatalogItem> {
    application
        .plugin_catalog()
        .map(|catalog| {
            catalog
                .operations
                .into_iter()
                .filter(|entry| {
                    entry.operation.discoverability.slash
                        && entry
                            .operation
                            .slash
                            .as_deref()
                            .is_some_and(|slash| !slash.trim().trim_start_matches('/').is_empty())
                })
                .collect()
        })
        .unwrap_or_default()
}

pub(crate) async fn invoke_plugin_slash_operation(
    application: &super::TuiBackend,
    entry: &agena_plugin_host::PluginOperationCatalogItem,
    session_id: Option<i64>,
    raw: &str,
) -> Result<PluginOperationEffect> {
    let plugin_id = entry.plugin_id.to_string();
    let response = application
        .client()
        .invoke_plugin_operation(
            plugin_id.as_str(),
            entry.operation.id.as_str(),
            serde_json::json!({}),
            session_id,
            entry.operation.slash.as_deref(),
            raw,
        )
        .await
        .context("failed to invoke plugin operation through the server")?;
    let result = response
        .get("result")
        .cloned()
        .ok_or_else(|| anyhow!("server response omitted plugin operation result"))?;
    let result = serde_json::from_value::<agena_plugin_host::PluginOperationResult>(result)
        .context("the server returned an undecodable plugin operation result")?;
    if let Err(error) = application.refresh_plugin_presentation_snapshot().await {
        tracing::warn!(
            diagnostic = %agena_failure::diagnostic::format_error_chain(error.as_ref()),
            "plugin operation succeeded, but refreshing the TUI plugin presentation snapshot failed"
        );
    }
    Ok(result)
}

/// Invoke an explicit plugin tool from a TUI workbench surface. Unavailable
/// tools are failures, not successful empty output.
pub(crate) async fn invoke_plugin_workbench_tool(
    application: &super::TuiBackend,
    plugin_id: &str,
    tool_name: &str,
    input: serde_json::Value,
    session_id: Option<i64>,
) -> Result<String> {
    let response = application
        .invoke_plugin_tool(plugin_id, tool_name, input, session_id)
        .await?;
    match response.status {
        agena_plugin_host::PluginToolInvokeStatus::Completed => Ok(response.output_text),
        agena_plugin_host::PluginToolInvokeStatus::CapabilityUnavailable
        | agena_plugin_host::PluginToolInvokeStatus::ToolUnavailable => {
            Err(anyhow!(if response.output_text.trim().is_empty() {
                response.title
            } else {
                response.output_text
            }))
        }
    }
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
        bail!("server returned the wrong permission-rule result");
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
        .command(Command::ReplacePermissionRule(
            ReplacePermissionRuleParams {
                rule_id,
                rule: params,
            },
        ))
        .await?;
    let CommandResult::PermissionRule(rule) = result else {
        bail!("server returned the wrong permission-rule result");
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
        bail!("server returned the wrong permission-rule result");
    };
    Ok(rule)
}

pub(crate) async fn create_commit(
    application: &super::TuiBackend,
    message: String,
) -> Result<(String, String)> {
    let commit: agena_application::dto::GitCommitResource = serde_json::from_value(
        application
            .client()
            .create_git_commit_in_workspace(Some(application.workspace_id()), message)
            .await?,
    )
    .context("the server returned an undecodable git commit result")?;
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
            .create_git_pull_request_in_workspace(
                Some(application.workspace_id()),
                title,
                body,
                base,
                head,
            )
            .await?,
    )
    .context("the server returned an undecodable git pull-request result")?;
    Ok(pull_request.url)
}
