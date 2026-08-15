use agena_application::dto::{PermissionMode as PermissionModeResource, PermissionRuleResource};
use agena_storage::MemoryType;

use super::{AppError, PermissionRuleOutput};

pub(super) fn memory_type_label(memory_type: Option<MemoryType>) -> Option<String> {
    memory_type.map(|value| value.label().to_string())
}

pub(super) fn permission_rule_output(
    rule: PermissionRuleResource,
) -> Result<PermissionRuleOutput, AppError> {
    Ok(PermissionRuleOutput {
        id: rule.id,
        action_key: rule.action_key,
        mode: match rule.mode {
            PermissionModeResource::Allow => "allow",
            PermissionModeResource::Auto => "auto",
            PermissionModeResource::Ask => "ask",
            PermissionModeResource::Deny => "deny",
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
