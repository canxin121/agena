use super::{
    AppError, DateTime, DbErr, Mutex, PermissionMode, PermissionRuleEvent, PermissionScope,
    PersistedPermissionRule, Session, SessionCache, Utc,
};

pub(crate) fn access_cache<T>(
    cache: &Mutex<SessionCache>,
    op: impl FnOnce(&mut SessionCache) -> T,
) -> Option<T> {
    match cache.lock() {
        Ok(mut guard) => Some(op(&mut guard)),
        Err(_) => {
            tracing::warn!("session cache lock poisoned; falling back to database state");
            None
        }
    }
}

pub(crate) fn permission_mode_label(mode: PermissionMode) -> String {
    match mode {
        PermissionMode::Allow => "allow".to_string(),
        PermissionMode::Ask => "ask".to_string(),
        PermissionMode::Deny => "deny".to_string(),
    }
}

pub(crate) fn permission_scope_label(scope: PermissionScope) -> String {
    match scope {
        PermissionScope::Session => "session".to_string(),
        PermissionScope::Workspace => "workspace".to_string(),
        PermissionScope::Global => "global".to_string(),
    }
}

pub(crate) fn permission_rule_event_from_rule(
    rule_id: i64,
    rule: &PersistedPermissionRule,
    fallback_session_id: i64,
) -> PermissionRuleEvent {
    PermissionRuleEvent {
        session_id: rule.session_id.or(Some(fallback_session_id)),
        rule_id,
        action_key: rule.action_key.clone(),
        mode: permission_mode_label(rule.mode),
        scope: permission_scope_label(rule.scope),
        source: rule.source.clone(),
        reason: rule.reason.clone(),
        operator: rule.operator.clone(),
        revoked_reason: rule.revoked_reason.clone(),
        revoked_by: rule.revoked_by.clone(),
        ts_ms: Utc::now().timestamp_millis(),
    }
}

pub(crate) fn session_from_model(
    model: crate::db::crud::session::SessionRecord,
) -> Result<Session, AppError> {
    let created_at = timestamp_millis_to_utc(model.created_at_ms)?;
    let updated_at = timestamp_millis_to_utc(model.updated_at_ms)?;
    let mut session = Session::new(model.id, model.workspace_id, model.title, created_at);
    session.parent_id = model.parent_id;
    session.depth = model.depth;
    session.root_id = model.root_id;
    session.version = model.version;
    session.relation_kind = model.relation_kind;
    session.lifecycle_state = model.lifecycle_state;
    session.source_cutoff_seq_global = model.source_cutoff_seq_global;
    session.source_message_id = model.source_message_id;
    session.task_id = model.task_id;
    session.runtime = model.runtime_state.unwrap_or_default();
    session.runtime.subtask = subtask_state_from_columns(
        model.id,
        model.relation_kind.is_subagent(),
        model.subtask_status.as_deref(),
        model.subtask_started_at_ms,
        model.subtask_finished_at_ms,
        model.subtask_error,
    )?;
    session.updated_at = updated_at;
    Ok(session)
}

pub(crate) fn session_from_model_db(
    model: crate::db::crud::session::SessionRecord,
) -> Result<Session, DbErr> {
    let created_at = timestamp_millis_to_utc_db(model.created_at_ms)?;
    let updated_at = timestamp_millis_to_utc_db(model.updated_at_ms)?;
    let mut session = Session::new(model.id, model.workspace_id, model.title, created_at);
    session.parent_id = model.parent_id;
    session.depth = model.depth;
    session.root_id = model.root_id;
    session.version = model.version;
    session.relation_kind = model.relation_kind;
    session.lifecycle_state = model.lifecycle_state;
    session.source_cutoff_seq_global = model.source_cutoff_seq_global;
    session.source_message_id = model.source_message_id;
    session.task_id = model.task_id;
    session.runtime = model.runtime_state.unwrap_or_default();
    session.runtime.subtask = subtask_state_from_columns_db(
        model.id,
        model.relation_kind.is_subagent(),
        model.subtask_status.as_deref(),
        model.subtask_started_at_ms,
        model.subtask_finished_at_ms,
        model.subtask_error,
    )?;
    session.updated_at = updated_at;
    Ok(session)
}

fn subtask_state_from_columns(
    session_id: i64,
    is_subagent: bool,
    status: Option<&str>,
    started_at_ms: Option<i64>,
    finished_at_ms: Option<i64>,
    error: Option<String>,
) -> Result<crate::session::SubtaskRuntimeState, AppError> {
    subtask_state_from_columns_inner(
        session_id,
        is_subagent,
        status,
        started_at_ms,
        finished_at_ms,
        error,
    )
    .map_err(AppError::Internal)
}

fn subtask_state_from_columns_db(
    session_id: i64,
    is_subagent: bool,
    status: Option<&str>,
    started_at_ms: Option<i64>,
    finished_at_ms: Option<i64>,
    error: Option<String>,
) -> Result<crate::session::SubtaskRuntimeState, DbErr> {
    subtask_state_from_columns_inner(
        session_id,
        is_subagent,
        status,
        started_at_ms,
        finished_at_ms,
        error,
    )
    .map_err(DbErr::Custom)
}

fn subtask_state_from_columns_inner(
    session_id: i64,
    is_subagent: bool,
    status: Option<&str>,
    started_at_ms: Option<i64>,
    finished_at_ms: Option<i64>,
    error: Option<String>,
) -> Result<crate::session::SubtaskRuntimeState, String> {
    let status = match status {
        Some(value) => agena_domain::SubtaskStatus::parse(value)
            .ok_or_else(|| format!("session {session_id} has invalid subtask status `{value}`"))?,
        None if is_subagent => agena_domain::SubtaskStatus::Created,
        None => agena_domain::SubtaskStatus::Created,
    };
    Ok(crate::session::SubtaskRuntimeState {
        status,
        started_at_ms,
        finished_at_ms,
        error,
    })
}

pub(crate) fn timestamp_millis_to_utc(timestamp_ms: i64) -> Result<DateTime<Utc>, AppError> {
    DateTime::from_timestamp_millis(timestamp_ms)
        .ok_or_else(|| AppError::Internal(format!("invalid timestamp millis: {timestamp_ms}")))
}

pub(crate) fn timestamp_millis_to_utc_db(timestamp_ms: i64) -> Result<DateTime<Utc>, DbErr> {
    DateTime::from_timestamp_millis(timestamp_ms)
        .ok_or_else(|| DbErr::Custom(format!("invalid timestamp millis: {timestamp_ms}")))
}
