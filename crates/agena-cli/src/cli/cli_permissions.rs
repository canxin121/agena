use agena_application::Application;
use agena_application::{
    dto::{CursorPaginationQuery, PermissionRuleResource, SearchPaginationQuery},
    service::PermissionRuleWriteCommand,
};
use agena_storage::MemoryType;

use super::{
    AppError, McpServerError, Path, PermissionAction, PermissionMode, PermissionModeArg,
    PermissionReplyKind, PermissionReplyKindArg, PermissionRuleOutput, PermissionScope,
    PermissionScopeArg, PermissionsWriteArgs, StructuredObject, ToolInvocation,
};

pub(super) fn structured_tool_input(
    arguments: Option<serde_json::Value>,
) -> Result<StructuredObject, McpServerError> {
    StructuredObject::try_from(arguments.unwrap_or_else(|| serde_json::json!({})))
        .map_err(McpServerError::InvalidParams)
}

pub(super) fn mcp_tool_invocation(
    name: &str,
    input: StructuredObject,
) -> Result<ToolInvocation, McpServerError> {
    Ok(ToolInvocation::new(name.to_owned(), input))
}

pub(super) fn memory_type_label(memory_type: Option<MemoryType>) -> Option<String> {
    memory_type.map(|value| value.label().to_string())
}

pub(super) fn permission_mode_from_arg(mode: PermissionModeArg) -> PermissionMode {
    match mode {
        PermissionModeArg::Allow => PermissionMode::Allow,
        PermissionModeArg::Ask => PermissionMode::Ask,
        PermissionModeArg::Deny => PermissionMode::Deny,
    }
}

pub(super) fn permission_scope_from_arg(scope: PermissionScopeArg) -> PermissionScope {
    match scope {
        PermissionScopeArg::Session => PermissionScope::Session,
        PermissionScopeArg::Workspace => PermissionScope::Workspace,
        PermissionScopeArg::Global => PermissionScope::Global,
    }
}

pub(super) fn permission_reply_kind_from_arg(kind: PermissionReplyKindArg) -> PermissionReplyKind {
    match kind {
        PermissionReplyKindArg::AllowOnce => PermissionReplyKind::AllowOnce,
        PermissionReplyKindArg::AllowAlways => PermissionReplyKind::AllowAlways,
        PermissionReplyKindArg::DenyOnce => PermissionReplyKind::DenyOnce,
        PermissionReplyKindArg::DenyAlways => PermissionReplyKind::DenyAlways,
    }
}

pub(super) fn permission_rule_output(
    rule: PermissionRuleResource,
) -> Result<PermissionRuleOutput, AppError> {
    Ok(PermissionRuleOutput {
        id: rule.id,
        action_key: rule.action_key,
        mode: match rule.mode {
            PermissionMode::Allow => "allow",
            PermissionMode::Ask => "ask",
            PermissionMode::Deny => "deny",
        }
        .to_owned(),
        scope: rule.scope,
        session_id: rule.session_id,
        workspace_id: rule.workspace_id,
        source: rule.source,
        reason: rule.reason,
        operator: rule.operator,
        revoked_at: rule.revoked_at,
        revoked_reason: rule.revoked_reason,
        revoked_by: rule.revoked_by,
        created_at: rule.created_at,
        updated_at: rule.updated_at,
    })
}

pub(super) async fn list_permission_rules(
    application: &Application,
    search: Option<String>,
) -> Result<Vec<PermissionRuleOutput>, AppError> {
    let mut cursor = None;
    let mut rules = Vec::new();
    loop {
        let page = application
            .service()
            .list_permission_rules(SearchPaginationQuery {
                pagination: CursorPaginationQuery {
                    cursor: cursor.clone(),
                    limit: Some(200),
                },
                search: search.clone(),
            })
            .await
            .map_err(|error| AppError::Internal(error.to_string()))?;
        rules.extend(
            page.items
                .into_iter()
                .map(permission_rule_output)
                .collect::<Result<Vec<_>, _>>()?,
        );
        if !page.page.has_more {
            return Ok(rules);
        }
        cursor = Some(page.page.next_cursor.ok_or_else(|| {
            AppError::Internal(
                "permission-rule page reported more results without a cursor".to_owned(),
            )
        })?);
    }
}

pub(super) fn permission_action_from_args(
    workspace_root: &Path,
    args: &PermissionsWriteArgs,
) -> Result<PermissionAction, AppError> {
    if let Some(action_key) = args
        .action_key
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return serde_json::from_str(action_key)
            .map_err(|err| AppError::Config(format!("invalid action_key json: {err}")));
    }
    if let Some(tool_name) = args
        .tool_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Ok(PermissionAction::Tool {
            tool_name: tool_name.to_string(),
            qualifier: args
                .qualifier
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned),
        });
    }
    if let Some(target) = args
        .network_target
        .as_deref()
        .or(args.network_host.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let parse_target = args
            .network_port
            .map(|port| format!("{target}:{port}"))
            .unwrap_or_else(|| target.to_string());
        let parsed: agena_domain::NetworkTarget = parse_target
            .parse()
            .map_err(|err| AppError::Config(format!("invalid network target: {err}")))?;
        return Ok(PermissionAction::NetworkAccess {
            target: target.to_string(),
            host: parsed.host().to_string(),
            port: parsed.port(),
        });
    }
    let path_access_kind = args
        .path_access_kind
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            AppError::Config(
                "permission rule requires either --action-key, or tool/path fields".to_string(),
            )
        })?;
    let target_path = args
        .target_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| AppError::Config("path_access rules require --target-path".to_string()))?;
    let workspace_root_value = args
        .workspace_root
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| workspace_root.to_string_lossy().to_string());
    Ok(PermissionAction::PathAccess {
        access_kind: path_access_kind.to_string(),
        workspace_root: workspace_root_value,
        target_path: target_path.to_string(),
    })
}

pub(super) fn permission_rule_write_command_from_args(
    workspace_root: &Path,
    args: &PermissionsWriteArgs,
) -> Result<PermissionRuleWriteCommand, AppError> {
    let scope = permission_scope_from_arg(args.scope);
    if matches!(scope, PermissionScope::Session) && args.session_id.is_none() {
        return Err(AppError::Config(
            "session scope requires --session-id".to_string(),
        ));
    }
    Ok(PermissionRuleWriteCommand {
        action: permission_action_from_args(workspace_root, args)?,
        mode: permission_mode_from_arg(args.rule_mode),
        scope,
        session_id: args.session_id,
        source: "cli".to_owned(),
        operator: Some("cli".to_owned()),
    })
}

/// Builds the one transport-neutral application handle used by CLI command
/// adapters. Runtime owns concrete adapter composition; individual commands
/// consume only the already-composed application services.
pub(super) fn application_from_runtime(
    runtime: &agena_runtime::RuntimeBootstrapResult,
) -> Result<Application, AppError> {
    Application::from_composed_runtime_services(runtime.application_services())
        .map_err(|error| AppError::Internal(error.to_string()))
}
