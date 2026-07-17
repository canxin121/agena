use sea_orm::{ActiveModelTrait, EntityTrait};

use super::{
    AppError, DateTime, McpServerError, MemoryStore, MemoryType, Path, PathBuf, PermissionAction,
    PermissionMode, PermissionModeArg, PermissionReplyKind, PermissionReplyKindArg,
    PermissionRuleOutput, PermissionScope, PermissionScopeArg, PermissionsWriteArgs,
    PersistedPermissionRule, Set, StructuredObject, ToolInvocation, Utc, entities, fs,
    permission_rule_crud, workspace_crud,
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

pub(super) fn ensure_memory_index_path(store: &MemoryStore) -> Result<PathBuf, AppError> {
    store.ensure_exists()?;
    let path = store.dir().join("MEMORY.md");
    if !path.exists() {
        fs::write(&path, "")?;
    }
    Ok(path)
}

pub(super) fn memory_record_name(entry: &crate::memory::MemoryRecord) -> String {
    if entry.frontmatter.name.trim().is_empty() {
        entry.file_name.trim_end_matches(".md").to_string()
    } else {
        entry.frontmatter.name.clone()
    }
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
    row: entities::permission_rule::Model,
) -> Result<PermissionRuleOutput, AppError> {
    Ok(PermissionRuleOutput {
        id: row.id,
        action_key: row.action_key,
        mode: row.mode,
        scope: row.scope,
        session_id: row.session_id,
        workspace_id: row.workspace_id,
        source: row.source,
        reason: row.reason,
        operator: row.operator,
        revoked_at: timestamp_ms_to_datetime(row.revoked_at_ms)?,
        revoked_reason: row.revoked_reason,
        revoked_by: row.revoked_by,
        created_at: required_timestamp_ms_to_datetime(
            "permission rule created_at_ms",
            row.created_at_ms,
        )?,
        updated_at: required_timestamp_ms_to_datetime(
            "permission rule updated_at_ms",
            row.updated_at_ms,
        )?,
    })
}

pub(super) fn required_timestamp_ms_to_datetime(
    label: &str,
    value: i64,
) -> Result<DateTime<Utc>, AppError> {
    DateTime::<Utc>::from_timestamp_millis(value)
        .ok_or_else(|| AppError::Internal(format!("invalid {label}: {value}")))
}

pub(super) fn timestamp_ms_to_datetime(
    value: Option<i64>,
) -> Result<Option<DateTime<Utc>>, AppError> {
    value
        .map(|value| required_timestamp_ms_to_datetime("timestamp_ms", value))
        .transpose()
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
        let parsed: crate::permission::NetworkTarget = parse_target
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

pub(super) async fn upsert_permission_rule_from_args(
    db: &sea_orm::DatabaseConnection,
    workspace_root: &Path,
    args: &PermissionsWriteArgs,
) -> Result<PermissionRuleOutput, AppError> {
    let scope = permission_scope_from_arg(args.scope);
    let action = permission_action_from_args(workspace_root, args)?;
    let action_key = serde_json::to_string(&action).map_err(AppError::from)?;
    let workspace_id = match scope {
        PermissionScope::Workspace => Some(
            workspace_crud::ensure_workspace_id(db, workspace_root.to_string_lossy().as_ref())
                .await?,
        ),
        PermissionScope::Session | PermissionScope::Global => None,
    };
    let session_id = match scope {
        PermissionScope::Session => args.session_id,
        PermissionScope::Workspace | PermissionScope::Global => None,
    };
    if matches!(scope, PermissionScope::Session) && session_id.is_none() {
        return Err(AppError::Config(
            "session scope requires --session-id".to_string(),
        ));
    }
    let (row, _) = permission_rule_crud::upsert_rule(
        db,
        &PersistedPermissionRule {
            action_key,
            mode: permission_mode_from_arg(args.rule_mode),
            scope,
            session_id,
            workspace_id,
            source: "cli".to_string(),
            reason: None,
            operator: Some("cli".to_string()),
            revoked_at_ms: None,
            revoked_reason: None,
            revoked_by: None,
        },
    )
    .await?;
    permission_rule_output(row)
}

pub(super) async fn replace_permission_rule_from_args(
    db: &sea_orm::DatabaseConnection,
    workspace_root: &Path,
    rule_id: i64,
    args: &PermissionsWriteArgs,
) -> Result<PermissionRuleOutput, AppError> {
    let existing = entities::permission_rule::Entity::find_by_id(rule_id)
        .one(db)
        .await?
        .ok_or_else(|| AppError::Config(format!("permission rule not found: {rule_id}")))?;
    let scope = permission_scope_from_arg(args.scope);
    let action = permission_action_from_args(workspace_root, args)?;
    let action_key = serde_json::to_string(&action).map_err(AppError::from)?;
    let workspace_id = match scope {
        PermissionScope::Workspace => Some(
            workspace_crud::ensure_workspace_id(db, workspace_root.to_string_lossy().as_ref())
                .await?,
        ),
        PermissionScope::Session | PermissionScope::Global => None,
    };
    let session_id = match scope {
        PermissionScope::Session => args.session_id,
        PermissionScope::Workspace | PermissionScope::Global => None,
    };
    if matches!(scope, PermissionScope::Session) && session_id.is_none() {
        return Err(AppError::Config(
            "session scope requires --session-id".to_string(),
        ));
    }
    let mut active: entities::permission_rule::ActiveModel = existing.into();
    active.action_key = Set(action_key);
    active.mode = Set(permission_mode_from_arg(args.rule_mode).as_str().to_owned());
    active.scope = Set(scope.as_str().to_owned());
    active.session_id = Set(session_id);
    active.workspace_id = Set(workspace_id);
    active.source = Set("cli".to_string());
    active.operator = Set(Some("cli".to_string()));
    active.updated_at_ms = Set(Utc::now().timestamp_millis());
    let row = active.update(db).await?;
    permission_rule_output(row)
}
