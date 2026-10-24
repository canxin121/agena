//! Business operations on immutable membership and bounded configuration.
use super::*;
use agena_domain::SessionRelationKind;
use std::collections::BTreeMap;

pub(super) async fn configuration(
    connection: &mut SqliteConnection,
    session: SessionId,
) -> Result<SessionConfiguration> {
    let row=sqlx::query("SELECT configuration_json,parent_session_id,relation_kind FROM agena_sessions WHERE id=? AND deleted_at_ms IS NULL").bind(session.get()).fetch_optional(connection).await.map_err(sql_error)?.ok_or_else(||KernelError::NotFound("session".into()))?;
    let mut config: SessionConfiguration =
        decode(row.try_get("configuration_json").map_err(storage)?)?;
    config.parent_session_id = row
        .try_get::<Option<i64>, _>("parent_session_id")
        .map_err(storage)?
        .map(SessionId::try_from)
        .transpose()
        .map_err(storage)?;
    config.relation_kind =
        SessionRelationKind::parse(row.try_get("relation_kind").map_err(storage)?)
            .ok_or_else(|| storage("invalid session relation"))?;
    Ok(config)
}
async fn configure(
    connection: &mut SqliteConnection,
    session: SessionId,
    change: &SessionConfigurationChange,
) -> Result<Mutation> {
    let original = configuration(connection, session).await?;
    let mut next = original.clone();
    if let Some(paused) = change.message_queue_paused {
        next.message_queue_paused = paused;
    }
    if let Some(parent) = change.parent_session_id
        && original.parent_session_id != Some(parent)
    {
        let entries: i64 = sqlx::query_scalar(
            "SELECT entry_count FROM agena_session_statistics WHERE session_id=?",
        )
        .bind(session.get())
        .fetch_one(&mut *connection)
        .await
        .map_err(sql_error)?;
        if entries != 0 || original.parent_session_id.is_some() || parent == session {
            return Err(KernelError::Conflict(
                "parent is immutable after creation".into(),
            ));
        }
        let own = reads::metadata(connection, session).await?;
        let parent_meta = reads::metadata(connection, parent).await?;
        if own.workspace_id != parent_meta.workspace_id {
            return Err(KernelError::Ownership);
        }
        let row = sqlx::query(
            "SELECT COALESCE(root_session_id,id) AS root,depth FROM agena_sessions WHERE id=?",
        )
        .bind(parent.get())
        .fetch_one(&mut *connection)
        .await
        .map_err(sql_error)?;
        sqlx::query(
            "UPDATE agena_sessions SET parent_session_id=?,root_session_id=?,depth=? WHERE id=?",
        )
        .bind(parent.get())
        .bind(row.try_get::<i64, _>("root").map_err(storage)?)
        .bind(row.try_get::<i64, _>("depth").map_err(storage)? + 1)
        .bind(session.get())
        .execute(&mut *connection)
        .await
        .map_err(sql_error)?;
        next.parent_session_id = Some(parent);
    }
    if let Some(relation) = change.relation_kind
        && original.relation_kind != relation
    {
        let entries: i64 = sqlx::query_scalar(
            "SELECT entry_count FROM agena_session_statistics WHERE session_id=?",
        )
        .bind(session.get())
        .fetch_one(&mut *connection)
        .await
        .map_err(sql_error)?;
        if entries != 0
            || (relation == SessionRelationKind::Root && next.parent_session_id.is_some())
            || (relation != SessionRelationKind::Root && next.parent_session_id.is_none())
        {
            return Err(KernelError::Conflict(
                "invalid or immutable relation".into(),
            ));
        }
        next.relation_kind = relation;
    }
    if let Some(value) = &change.conversation {
        next.conversation = value.clone();
    }
    if let Some(value) = &change.permission {
        next.permission = value.clone();
    }
    if let Some(value) = &change.permission_ceiling {
        if !original.permission_ceiling.is_empty() && &original.permission_ceiling != value {
            return Err(KernelError::Conflict(
                "delegated permission ceiling is immutable".into(),
            ));
        }
        next.permission_ceiling = value.clone();
    }
    if let Some(value) = &change.capability_denied_tool_names {
        if !original.capability_denied_tool_names.is_subset(value) {
            return Err(KernelError::Conflict(
                "capability denials cannot be removed".into(),
            ));
        }
        next.capability_denied_tool_names = value.clone();
    }
    if let Some(value) = &change.effective_workspace_root {
        next.effective_workspace_root = value.clone();
    }
    if let Some(value) = &change.agent {
        if original.agent.is_some() && value.is_none() {
            return Err(KernelError::Conflict(
                "agent identity cannot be removed".into(),
            ));
        }
        if let Some(agent) = value
            && (next.relation_kind != SessionRelationKind::Subagent
                || next.parent_session_id.is_none()
                || agent.name.is_empty()
                || agent.name.len() > 64
                || !agent
                    .name
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
                || matches!(agent.name.as_str(), "parent" | "self")
                || agent.name.bytes().all(|b| b.is_ascii_digit()))
        {
            return Err(invalid("invalid agent identity"));
        }
        if let (Some(old), Some(new)) = (&original.agent, value)
            && (old.name != new.name
                || (old.status.is_terminal()
                    && old != new
                    && !(new.status == agena_domain::AgentStatus::Running
                        && new.started_at_ms > old.started_at_ms
                        && new.finished_at_ms.is_none())))
        {
            return Err(KernelError::Conflict(
                "agent identity and terminal outcome are immutable".into(),
            ));
        }
        next.agent = value.clone();
    }
    let body = encode(&next)?;
    if body.len() > 65536 {
        return Err(invalid("session configuration exceeds 64 KiB"));
    }
    if next != original {
        sqlx::query("UPDATE agena_sessions SET configuration_json=?,relation_kind=?,metadata_version=metadata_version+1,updated_at_ms=MAX(?,updated_at_ms+1) WHERE id=?").bind(body).bind(next.relation_kind.as_str()).bind(now_ms()).bind(session.get()).execute(&mut *connection).await.map_err(sql_error)?;
    }
    if original.message_queue_paused != next.message_queue_paused {
        // Re-run the existing compact-state projection when resuming; no
        // execution transition is needed to wake a retained queue.
        sqlx::query("UPDATE agena_session_statistics SET queued_executions=queued_executions WHERE session_id=?")
            .bind(session.get()).execute(&mut *connection).await.map_err(sql_error)?;
    }
    Ok(Mutation::Configuration(Box::new(next)))
}
async fn boundary(
    connection: &mut SqliteConnection,
    session: SessionId,
    entry: EntryId,
) -> Result<(i64, i64)> {
    let row=sqlx::query("SELECT s.sequence,m.ordinal FROM agena_segment_entries m JOIN agena_session_segments s ON s.segment_id=m.segment_id WHERE s.session_id=? AND m.entry_id=? AND m.ordinal>=s.first_ordinal AND m.ordinal<s.end_ordinal").bind(session.get()).bind(entry.get()).fetch_optional(connection).await.map_err(sql_error)?.ok_or(KernelError::Ownership)?;
    Ok((
        row.try_get("sequence").map_err(storage)?,
        row.try_get("ordinal").map_err(storage)?,
    ))
}
async fn recount(connection: &mut SqliteConnection, session: SessionId) -> Result<()> {
    sqlx::query("UPDATE agena_session_statistics SET entry_count=(SELECT COALESCE(sum(end_ordinal-first_ordinal),0) FROM agena_session_segments WHERE session_id=?),user_message_count=(SELECT count(*) FROM agena_session_segments s JOIN agena_segment_entries m ON m.segment_id=s.segment_id AND m.ordinal>=s.first_ordinal AND m.ordinal<s.end_ordinal JOIN agena_entries e ON e.id=m.entry_id WHERE s.session_id=? AND e.kind='user'),last_message_at_ms=(SELECT MAX(e.created_at_ms) FROM agena_session_segments s JOIN agena_segment_entries m ON m.segment_id=s.segment_id AND m.ordinal>=s.first_ordinal AND m.ordinal<s.end_ordinal JOIN agena_entries e ON e.id=m.entry_id WHERE s.session_id=?) WHERE session_id=?").bind(session.get()).bind(session.get()).bind(session.get()).bind(session.get()).execute(connection).await.map_err(sql_error)?;
    Ok(())
}
async fn fork(
    connection: &mut SqliteConnection,
    session: SessionId,
    through: EntryId,
    rewind: bool,
    conversation_mode: Option<agena_domain::ConversationMode>,
) -> Result<Mutation> {
    if rewind && conversation_mode.is_some() {
        return Err(invalid("rewind cannot create a conversation branch"));
    }
    let metadata = reads::metadata(connection, session).await?;
    let row =
        sqlx::query("SELECT kind,body_json,execution_id,generation FROM agena_entries WHERE id=?")
            .bind(through.get())
            .fetch_one(&mut *connection)
            .await
            .map_err(sql_error)?;
    let kind: &str = row.try_get("kind").map_err(storage)?;
    let restored = if rewind {
        Some(decode::<agena_domain::ComposerDocument>(
            row.try_get("body_json").map_err(storage)?,
        )?)
    } else {
        None
    };
    if rewind {
        if kind != "user" {
            return Err(invalid("rewind requires a user message"));
        }
        let phase:Option<String>=sqlx::query_scalar("SELECT a.phase FROM agena_executions e JOIN agena_attempts a ON a.execution_id=e.id AND a.generation=e.current_generation WHERE e.id=?").bind(row.try_get::<&str,_>("execution_id").map_err(storage)?).fetch_optional(&mut *connection).await.map_err(sql_error)?;
        if phase
            .as_deref()
            .is_none_or(|phase| matches!(phase, "queued" | "running"))
        {
            return Err(KernelError::Conflict(
                "rewind requires a completed user message".into(),
            ));
        }
    }
    boundary(connection, session, through).await?;
    // A message boundary is the first actual assistant entry, while its
    // immutable facts may span many segments. Freeze the complete visible
    // response at this transaction's cut, including tool and interaction facts.
    let cut = if !rewind && kind != "user" {
        sqlx::query_scalar::<_,i64>("SELECT e.id FROM agena_entries e JOIN agena_segment_entries m ON m.entry_id=e.id JOIN agena_session_segments s ON s.segment_id=m.segment_id WHERE s.session_id=? AND m.ordinal>=s.first_ordinal AND m.ordinal<s.end_ordinal AND e.execution_id=? AND e.generation=? AND e.kind!='user' ORDER BY s.sequence DESC,m.ordinal DESC LIMIT 1")
            .bind(session.get()).bind(row.try_get::<&str,_>("execution_id").map_err(storage)?).bind(row.try_get::<i64,_>("generation").map_err(storage)?)
            .fetch_optional(&mut *connection).await.map_err(sql_error)?.map(EntryId::try_from).transpose().map_err(storage)?.unwrap_or(through)
    } else {
        through
    };
    let (sequence, ordinal) = boundary(connection, session, cut).await?;
    let end = ordinal + i64::from(!rewind);
    let old_config = configuration(connection, session).await?;
    let mut config = old_config;
    config.parent_session_id = Some(session);
    config.relation_kind = if rewind {
        SessionRelationKind::Rewind
    } else {
        SessionRelationKind::Fork
    };
    config.agent = None;
    config.conversation = conversation_mode.map(|mode| agena_domain::SessionConversation {
        mode,
        parent_session_id: session.get(),
    });
    let row = sqlx::query(
        "SELECT COALESCE(root_session_id,id) AS root,depth FROM agena_sessions WHERE id=?",
    )
    .bind(session.get())
    .fetch_one(&mut *connection)
    .await
    .map_err(sql_error)?;
    let now = now_ms();
    let id:i64=sqlx::query_scalar("INSERT INTO agena_sessions (workspace_id,title,selection_json,parent_session_id,root_session_id,depth,relation_kind,configuration_json,created_at_ms,updated_at_ms) VALUES (?,?,?,?,?,?,?,?,?,?) RETURNING id").bind(metadata.workspace_id).bind(&metadata.title).bind(metadata.selection.as_ref().map(encode).transpose()?).bind(session.get()).bind(row.try_get::<i64,_>("root").map_err(storage)?).bind(row.try_get::<i64,_>("depth").map_err(storage)?+1).bind(config.relation_kind.as_str()).bind(encode(&config)?).bind(now).bind(now).fetch_one(&mut *connection).await.map_err(sql_error)?;
    let target = SessionId::try_from(id).map_err(storage)?;
    sqlx::query("INSERT INTO agena_session_statistics (session_id) VALUES (?)")
        .bind(id)
        .execute(&mut *connection)
        .await
        .map_err(sql_error)?;
    sqlx::query("INSERT INTO agena_session_segments (session_id,sequence,segment_id,first_ordinal,end_ordinal,since_generation) SELECT ?,sequence,segment_id,first_ordinal,CASE WHEN sequence=? THEN MIN(end_ordinal,?) ELSE end_ordinal END,1 FROM agena_session_segments WHERE session_id=? AND (sequence<? OR (sequence=? AND first_ordinal<?))").bind(id).bind(sequence).bind(end).bind(session.get()).bind(sequence).bind(sequence).bind(end).execute(&mut *connection).await.map_err(sql_error)?;
    recount(connection, target).await?;
    sqlx::query("INSERT INTO agena_prompt_windows (session_id) VALUES (?)")
        .bind(id)
        .execute(&mut *connection)
        .await
        .map_err(sql_error)?;
    // A branch may reuse the checkpoint only if both its summary and coverage
    // boundary remain in the immutable membership inherited at this cut.
    sqlx::query("UPDATE agena_prompt_windows SET (checkpoint_entry_id,through_entry_id,through_sequence,through_ordinal)=(SELECT checkpoint_entry_id,through_entry_id,through_sequence,through_ordinal FROM agena_prompt_windows WHERE session_id=?) WHERE session_id=? AND EXISTS (SELECT 1 FROM agena_prompt_windows w WHERE w.session_id=? AND w.checkpoint_entry_id IS NOT NULL AND EXISTS(SELECT 1 FROM agena_session_segments s JOIN agena_segment_entries m ON m.segment_id=s.segment_id WHERE s.session_id=? AND m.entry_id=w.checkpoint_entry_id AND m.ordinal>=s.first_ordinal AND m.ordinal<s.end_ordinal) AND EXISTS(SELECT 1 FROM agena_session_segments s JOIN agena_segment_entries m ON m.segment_id=s.segment_id WHERE s.session_id=? AND m.entry_id=w.through_entry_id AND m.ordinal>=s.first_ordinal AND m.ordinal<s.end_ordinal))").bind(session.get()).bind(id).bind(session.get()).bind(id).bind(id).execute(&mut *connection).await.map_err(sql_error)?;
    let metadata = reads::metadata(connection, target).await?;
    Ok(if rewind {
        Mutation::Rewind(Box::new(RewindResult {
            session: metadata,
            document: restored.expect("rewind document was validated"),
        }))
    } else {
        Mutation::Session(Box::new(metadata))
    })
}
async fn withdraw(
    connection: &mut SqliteConnection,
    session: SessionId,
    message: EntryId,
) -> Result<Mutation> {
    let metadata = reads::metadata(connection, session).await?;
    let (sequence, ordinal) = boundary(connection, session, message).await?;
    let row =
        sqlx::query("SELECT origin_session_id,kind,execution_id FROM agena_entries WHERE id=?")
            .bind(message.get())
            .fetch_one(&mut *connection)
            .await
            .map_err(sql_error)?;
    if row.try_get::<&str, _>("kind").map_err(storage)? != "user"
        || row
            .try_get::<i64, _>("origin_session_id")
            .map_err(storage)?
            != session.get()
    {
        return Err(KernelError::Ownership);
    }
    let later:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agena_session_segments s JOIN agena_segment_entries m ON m.segment_id=s.segment_id AND m.ordinal>=s.first_ordinal AND m.ordinal<s.end_ordinal JOIN agena_entries e ON e.id=m.entry_id WHERE s.session_id=? AND (s.sequence>? OR (s.sequence=? AND m.ordinal>?)) AND e.kind='user')").bind(session.get()).bind(sequence).bind(sequence).bind(ordinal).fetch_one(&mut *connection).await.map_err(sql_error)?;
    let active:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agena_attempts WHERE session_id=? AND phase IN ('queued','running'))").bind(session.get()).fetch_one(&mut *connection).await.map_err(sql_error)?;
    if later || active {
        return Err(KernelError::Conflict(
            "withdraw requires the final completed user message".into(),
        ));
    }
    let next = metadata.membership_generation.next().map_err(storage)?;
    sqlx::query("INSERT INTO agena_retired_session_segments (session_id,sequence,segment_id,first_ordinal,end_ordinal,since_generation,until_generation) SELECT session_id,sequence,segment_id,first_ordinal,end_ordinal,since_generation,? FROM agena_session_segments edge WHERE session_id=? AND sequence>=? AND EXISTS(SELECT 1 FROM agena_history_snapshots root WHERE root.session_id=edge.session_id AND root.generation>=edge.since_generation AND root.generation<?)").bind(next.get()).bind(session.get()).bind(sequence).bind(next.get()).execute(&mut *connection).await.map_err(sql_error)?;
    let cut=sqlx::query("SELECT segment_id,first_ordinal FROM agena_session_segments WHERE session_id=? AND sequence=?").bind(session.get()).bind(sequence).fetch_one(&mut *connection).await.map_err(sql_error)?;
    sqlx::query("DELETE FROM agena_session_segments WHERE session_id=? AND sequence>=?")
        .bind(session.get())
        .bind(sequence)
        .execute(&mut *connection)
        .await
        .map_err(sql_error)?;
    let first: i64 = cut.try_get("first_ordinal").map_err(storage)?;
    if ordinal > first {
        sqlx::query("INSERT INTO agena_session_segments (session_id,sequence,segment_id,first_ordinal,end_ordinal,since_generation) VALUES (?,?,?,?,?,?)").bind(session.get()).bind(sequence).bind(cut.try_get::<i64,_>("segment_id").map_err(storage)?).bind(first).bind(ordinal).bind(next.get()).execute(&mut *connection).await.map_err(sql_error)?;
    }
    sqlx::query("UPDATE agena_sessions SET membership_generation=?,updated_at_ms=MAX(?,updated_at_ms+1) WHERE id=?").bind(next.get()).bind(now_ms()).bind(session.get()).execute(&mut *connection).await.map_err(sql_error)?;
    sqlx::query("UPDATE agena_prompt_windows SET checkpoint_entry_id=NULL,through_entry_id=NULL,through_sequence=NULL,through_ordinal=NULL,rewrite_epoch=rewrite_epoch+1 WHERE session_id=?").bind(session.get()).execute(&mut *connection).await.map_err(sql_error)?;
    recount(connection, session).await?;
    Ok(Mutation::Session(Box::new(
        reads::metadata(connection, session).await?,
    )))
}
async fn delete(connection: &mut SqliteConnection, session: SessionId) -> Result<Mutation> {
    let metadata = reads::metadata(connection, session).await?;
    // All owned executions are settled in bounded batches within this one
    // command transaction; shared branch membership remains immutable.
    loop {
        let ids:Vec<String>=sqlx::query_scalar("SELECT execution_id FROM agena_attempts WHERE session_id=? AND phase IN ('queued','running') ORDER BY execution_id LIMIT 128").bind(session.get()).fetch_all(&mut *connection).await.map_err(sql_error)?;
        if ids.is_empty() {
            break;
        }
        for id in ids {
            let (view, _) =
                reads::execution(connection, session, id.parse().map_err(storage)?).await?;
            commands::finish(connection, view, ExecutionPhase::Cancelled).await?;
        }
    }
    sqlx::query("UPDATE agena_background_activities SET phase='interrupted',outcome_json=?,finished_at_ms=MAX(?,created_at_ms) WHERE session_id=? AND phase='running'").bind(encode(&BackgroundActivityOutcome::Interrupted{diagnostic:"origin session deleted".into()})?).bind(now_ms()).bind(session.get()).execute(&mut *connection).await.map_err(sql_error)?;
    sqlx::query("UPDATE agena_sessions SET deleted_at_ms=?,membership_generation=membership_generation+1,metadata_version=metadata_version+1 WHERE id=?").bind(now_ms()).bind(session.get()).execute(&mut *connection).await.map_err(sql_error)?;
    Ok(Mutation::Session(Box::new(SessionMetadata {
        metadata_version: metadata.metadata_version.next().map_err(storage)?,
        membership_generation: metadata.membership_generation.next().map_err(storage)?,
        ..metadata
    })))
}
async fn delivery(
    connection: &mut SqliteConnection,
    session: SessionId,
    id: agena_domain::CommandId,
) -> Result<DeliveryView> {
    let row=sqlx::query("SELECT business_key,target_execution_id,payload_json,result_entry_id,state FROM agena_deliveries WHERE id=? AND session_id=?").bind(id.to_string()).bind(session.get()).fetch_optional(connection).await.map_err(sql_error)?.ok_or(KernelError::Ownership)?;
    Ok(DeliveryView {
        id,
        session,
        target_execution: row
            .try_get::<Option<String>, _>("target_execution_id")
            .map_err(storage)?
            .map(|id| id.parse().map_err(storage))
            .transpose()?,
        business_key: row.try_get("business_key").map_err(storage)?,
        payload: decode(row.try_get("payload_json").map_err(storage)?)?,
        entry: row
            .try_get::<Option<i64>, _>("result_entry_id")
            .map_err(storage)?
            .map(EntryId::try_from)
            .transpose()
            .map_err(storage)?,
        consumed: row.try_get::<&str, _>("state").map_err(storage)? == "consumed",
    })
}

async fn publish_delivery(
    connection: &mut SqliteConnection,
    session: SessionId,
    target: Option<ExecutionId>,
    business_key: &str,
    payload: &serde_json::Value,
) -> Result<DeliveryView> {
    reads::metadata(connection, session).await?;
    if business_key.is_empty() || business_key.len() > 1024 || encode(payload)?.len() > 65536 {
        return Err(invalid("invalid delivery key or payload size"));
    }
    let existing: Option<String> =
        sqlx::query_scalar("SELECT id FROM agena_deliveries WHERE business_key=?")
            .bind(business_key)
            .fetch_optional(&mut *connection)
            .await
            .map_err(sql_error)?;
    if let Some(id) = existing {
        let previous = delivery(connection, session, id.parse().map_err(storage)?).await?;
        if previous.payload != *payload || previous.target_execution != target {
            return Err(KernelError::Conflict(
                "delivery business key payload or execution changed".into(),
            ));
        }
        return Ok(previous);
    }
    if let Some(execution) = target {
        let (view, _) = reads::execution(connection, session, execution).await?;
        let stopping: bool = sqlx::query_scalar(
            "SELECT stop_requested FROM agena_attempts WHERE execution_id=? AND generation=?",
        )
        .bind(execution.to_string())
        .bind(view.generation.get())
        .fetch_one(&mut *connection)
        .await
        .map_err(sql_error)?;
        if view.phase.is_terminal() || stopping {
            return Err(KernelError::Conflict(
                "the delegated execution has ended; use agent.followup to resume it".into(),
            ));
        }
    }
    if payload.get("kind").and_then(serde_json::Value::as_str) == Some("agent_message") {
        let (count, bytes):(i64,i64)=sqlx::query_as("SELECT count(*),COALESCE(sum(length(CAST(payload_json AS BLOB))),0) FROM agena_deliveries WHERE session_id=? AND state='pending' AND json_extract(payload_json,'$.kind')='agent_message'")
            .bind(session.get()).fetch_one(&mut *connection).await.map_err(sql_error)?;
        if count >= 32 || bytes.saturating_add(encode(payload)?.len() as i64) > 128 * 1024 {
            return Err(invalid(
                "agent mailbox is full; wait for the recipient to consume accepted messages before sending more",
            ));
        }
    }
    let id = agena_domain::CommandId::new();
    sqlx::query("INSERT INTO agena_deliveries (id,session_id,target_execution_id,business_key,payload_json,state) VALUES (?,?,?,?,?,'pending')")
        .bind(id.to_string()).bind(session.get()).bind(target.map(|id| id.to_string())).bind(business_key).bind(encode(payload)?)
        .execute(&mut *connection).await.map_err(sql_error)?;
    delivery(connection, session, id).await
}

async fn notice(
    connection: &mut SqliteConnection,
    session: SessionId,
    message: EntryId,
    text: &str,
) -> Result<EntryId> {
    if text.len() > 256 * 1024 {
        return Err(invalid("notice exceeds 256 KiB"));
    }
    boundary(connection, session, message).await?;
    let row=sqlx::query("SELECT e.origin_session_id,e.execution_id,x.current_generation AS generation FROM agena_entries e JOIN agena_executions x ON x.id=e.execution_id WHERE e.id=? AND e.kind='user'").bind(message.get()).fetch_optional(&mut *connection).await.map_err(sql_error)?.ok_or_else(||invalid("notice requires user message"))?;
    if row
        .try_get::<i64, _>("origin_session_id")
        .map_err(storage)?
        != session.get()
    {
        return Err(KernelError::Ownership);
    }
    let id:i64=sqlx::query_scalar("INSERT INTO agena_entries (origin_session_id,execution_id,generation,message_id,kind,body_json,created_at_ms) VALUES (?,?,?,?,'notice',?,?) RETURNING id").bind(session.get()).bind(row.try_get::<&str,_>("execution_id").map_err(storage)?).bind(row.try_get::<i64,_>("generation").map_err(storage)?).bind(message.get()).bind(encode(text)?).bind(now_ms()).fetch_one(&mut *connection).await.map_err(sql_error)?;
    commands::append_history(connection, session, &[id]).await?;
    EntryId::try_from(id).map_err(storage)
}
pub(super) async fn apply(
    connection: &mut SqliteConnection,
    command: &Command,
) -> Result<Option<Mutation>> {
    let value = match command {
        Command::RecordCompaction {
            scope,
            policy_key,
            outcome,
        } => record_compaction(connection, *scope, policy_key, outcome).await?,
        Command::ResumeDelivery {
            session,
            delivery: id,
            message,
            text,
        } => {
            reads::metadata(connection, *session).await?;
            let previous = delivery(connection, *session, *id).await?;
            if previous.target_execution.is_some() {
                return Err(KernelError::Conflict(
                    "execution guidance cannot start another execution".into(),
                ));
            }
            if previous.consumed {
                let row=sqlx::query("SELECT d.resume_execution_id,d.resume_generation,a.phase,x.message_id FROM agena_deliveries d JOIN agena_executions x ON x.id=d.resume_execution_id JOIN agena_attempts a ON a.execution_id=x.id AND a.generation=d.resume_generation WHERE d.id=? AND d.session_id=?")
                    .bind(id.to_string()).bind(session.get()).fetch_optional(&mut *connection).await.map_err(sql_error)?.ok_or_else(||KernelError::Conflict("delivery was consumed by an existing execution".into()))?;
                if row.try_get::<i64, _>("message_id").map_err(storage)? != message.get() {
                    return Err(KernelError::Conflict(
                        "delivery continuation message changed".into(),
                    ));
                }
                Mutation::Execution(ExecutionView {
                    id: row
                        .try_get::<&str, _>("resume_execution_id")
                        .map_err(storage)?
                        .parse()
                        .map_err(storage)?,
                    session_id: *session,
                    message_id: *message,
                    generation: AttemptGeneration::try_from(
                        row.try_get::<i64, _>("resume_generation")
                            .map_err(storage)?,
                    )
                    .map_err(storage)?,
                    phase: decode(&encode(row.try_get::<&str, _>("phase").map_err(storage)?)?)?,
                })
            } else {
                let continued = Box::pin(commands::apply(
                    connection,
                    &Command::Continue {
                        session: *session,
                        message: *message,
                    },
                ))
                .await?;
                let Mutation::Execution(execution) = continued else {
                    return Err(storage("invalid delivery continuation result"));
                };
                let entry = notice(connection, *session, *message, text).await?;
                sqlx::query("UPDATE agena_deliveries SET state='consumed',result_entry_id=?,resume_execution_id=?,resume_generation=?,owner_epoch=NULL,lease_until_ms=NULL WHERE id=? AND session_id=? AND state='pending'")
                    .bind(entry.get()).bind(execution.id.to_string()).bind(execution.generation.get()).bind(id.to_string()).bind(session.get()).execute(&mut *connection).await.map_err(sql_error)?;
                Mutation::Execution(execution)
            }
        }
        Command::PresentInteraction {
            session,
            interaction,
        } => {
            reads::metadata(connection, *session).await?;
            let row=sqlx::query("SELECT i.tool_id,i.request_json,i.decision_json,i.remember_json,i.created_at_ms AS interaction_created_at_ms,i.decided_at_ms AS interaction_decided_at_ms,e.revision FROM agena_interactions i JOIN agena_entries e ON e.interaction_id=i.id WHERE i.id=? AND i.session_id=? AND e.origin_session_id=?").bind(interaction.get()).bind(session.get()).bind(session.get()).fetch_optional(&mut *connection).await.map_err(sql_error)?.ok_or(KernelError::Ownership)?;
            let mut view = reads::interaction_row(*interaction, &row)?;
            if view.decision.is_none()
                && let InteractionRequest::Input { request } = &mut view.request
                && request.presented_at.is_none()
            {
                request.presented_at = Some(chrono::Utc::now());
                sqlx::query("UPDATE agena_interactions SET request_json=? WHERE id=?")
                    .bind(encode(&view.request)?)
                    .bind(interaction.get())
                    .execute(&mut *connection)
                    .await
                    .map_err(sql_error)?;
                sqlx::query("UPDATE agena_entries SET revision=revision+1 WHERE interaction_id=?")
                    .bind(interaction.get())
                    .execute(&mut *connection)
                    .await
                    .map_err(sql_error)?;
                view.revision = view.revision.next().map_err(storage)?;
            }
            Mutation::InteractionView(Box::new(view))
        }
        Command::CreateConfigured {
            workspace_id,
            title,
            configuration: config,
        } => {
            let created = Box::pin(commands::apply(
                connection,
                &Command::CreateSession {
                    workspace_id: *workspace_id,
                    title: title.clone(),
                },
            ))
            .await?;
            let Mutation::Session(metadata) = created else {
                return Err(storage("invalid create result"));
            };
            configure(
                connection,
                metadata.id,
                &SessionConfigurationChange {
                    message_queue_paused: Some(config.message_queue_paused),
                    parent_session_id: config.parent_session_id,
                    relation_kind: Some(config.relation_kind),
                    conversation: Some(config.conversation.clone()),
                    permission: Some(config.permission.clone()),
                    permission_ceiling: Some(config.permission_ceiling.clone()),
                    capability_denied_tool_names: Some(config.capability_denied_tool_names.clone()),
                    effective_workspace_root: Some(config.effective_workspace_root.clone()),
                    agent: Some(config.agent.clone()),
                },
            )
            .await?;
            Mutation::Session(Box::new(reads::metadata(connection, metadata.id).await?))
        }
        Command::Configure { session, change } => configure(connection, *session, change).await?,
        Command::FinishAgent {
            session,
            agent,
            business_key,
            payload,
        } => {
            let config = configuration(connection, *session).await?;
            if config
                .agent
                .as_ref()
                .is_none_or(|previous| previous.name != agent.name)
                || config.relation_kind != SessionRelationKind::Subagent
                || !agent.status.is_terminal()
            {
                return Err(KernelError::Ownership);
            }
            let parent = config.parent_session_id.ok_or(KernelError::Ownership)?;
            let outcome = configure(
                connection,
                *session,
                &SessionConfigurationChange {
                    agent: Some(Some(agent.clone())),
                    ..Default::default()
                },
            )
            .await?;
            publish_delivery(connection, parent, None, business_key, payload).await?;
            outcome
        }

        Command::Fork {
            session,
            through,
            rewind,
            conversation_mode,
        } => fork(connection, *session, *through, *rewind, *conversation_mode).await?,
        Command::Withdraw { session, message } => withdraw(connection, *session, *message).await?,
        Command::DeleteSession { session } => delete(connection, *session).await?,
        Command::Continue { session, message } => {
            boundary(connection, *session, *message).await?;
            let id: Option<String> = sqlx::query_scalar(
                "SELECT id FROM agena_executions WHERE session_id=? AND message_id=?",
            )
            .bind(session.get())
            .bind(message.get())
            .fetch_optional(&mut *connection)
            .await
            .map_err(sql_error)?;
            let execution = id.ok_or(KernelError::Ownership)?.parse().map_err(storage)?;
            Box::pin(commands::apply(
                connection,
                &Command::Retry {
                    session: *session,
                    execution,
                },
            ))
            .await?
        }
        Command::Notice { scope, text } => {
            let view = commands::check_scope(connection, *scope, true).await?;
            Mutation::Checkpoint(notice(connection, scope.session, view.message_id, text).await?)
        }
        Command::Deliver {
            session,
            business_key,
            payload,
        } => Mutation::Delivery(
            publish_delivery(connection, *session, None, business_key, payload).await?,
        ),
        Command::DeliverToExecution {
            session,
            execution,
            business_key,
            payload,
        } => Mutation::Delivery(
            publish_delivery(
                connection,
                *session,
                Some(*execution),
                business_key,
                payload,
            )
            .await?,
        ),
        Command::ConsumeDelivery {
            session,
            delivery: id,
            message,
            text,
        } => {
            reads::metadata(connection, *session).await?;
            let previous = delivery(connection, *session, *id).await?;
            if let Some(execution) = previous.target_execution {
                let (target, _) = reads::execution(connection, *session, execution).await?;
                if target.message_id != *message {
                    return Err(KernelError::Ownership);
                }
                if previous.entry.is_none() && target.phase != ExecutionPhase::Running {
                    return Err(KernelError::Conflict(
                        "execution guidance is no longer active".into(),
                    ));
                }
            }
            if let Some(entry) = previous.entry {
                Mutation::Checkpoint(entry)
            } else {
                let pending: bool = sqlx::query_scalar(
                    "SELECT state='pending' FROM agena_deliveries WHERE id=? AND session_id=?",
                )
                .bind(id.to_string())
                .bind(session.get())
                .fetch_one(&mut *connection)
                .await
                .map_err(sql_error)?;
                if !pending {
                    return Err(KernelError::Conflict(
                        "delivery is no longer pending".into(),
                    ));
                }
                let entry = notice(connection, *session, *message, text).await?;
                sqlx::query("UPDATE agena_deliveries SET state='consumed',result_entry_id=?,owner_epoch=NULL,lease_until_ms=NULL WHERE id=? AND session_id=? AND state='pending'").bind(entry.get()).bind(id.to_string()).bind(session.get()).execute(&mut *connection).await.map_err(sql_error)?;
                Mutation::Checkpoint(entry)
            }
        }
        _ => return Ok(None),
    };
    Ok(Some(value))
}

/// Apply the paused presentation at the write boundary, never by loading
/// configuration documents on the compact session-list read path.
pub(super) async fn refresh_paused_summary(
    connection: &mut SqliteConnection,
    session: SessionId,
) -> Result<()> {
    sqlx::query("UPDATE agena_session_statistics SET processing_state='ready' WHERE session_id=? AND running_executions=0 AND queued_executions>0 AND EXISTS(SELECT 1 FROM agena_sessions s WHERE s.id=? AND COALESCE(json_extract(s.configuration_json,'$.message_queue_paused'),0)=1)")
        .bind(session.get()).bind(session.get()).execute(connection).await.map_err(sql_error)?;
    Ok(())
}

pub(super) const STATE_SQL: &str = "t.processing_state";
pub(super) fn summary_fields() -> String {
    "s.parent_session_id,COALESCE(s.root_session_id,s.id) AS root_session_id,s.depth,s.relation_kind,s.agent_name,s.agent_status AS agent_status,t.child_session_count,t.processing_state,".to_owned()
}
fn filter<'a>(
    builder: &mut sqlx::QueryBuilder<'a, sqlx::Sqlite>,
    query: &'a SummaryQuery,
    cursor: bool,
) {
    builder.push("s.deleted_at_ms IS NULL");
    if let Some(workspace) = query.workspace_id {
        builder.push(" AND s.workspace_id=").push_bind(workspace);
    }
    if let Some(parent) = query.parent_id {
        builder
            .push(" AND s.parent_session_id=")
            .push_bind(parent.get());
    }
    if query.agents_only {
        builder.push(" AND s.agent_name IS NOT NULL");
    }
    if query.roots_only {
        builder.push(" AND s.parent_session_id IS NULL");
    }
    if query.exclude_subagents {
        builder.push(" AND s.relation_kind!='subagent'");
    }
    builder.push(if query.temporary_only {
        " AND s.temporary=1"
    } else {
        " AND s.temporary=0"
    });
    if let Some(search) = &query.search {
        builder
            .push(" AND s.title LIKE ")
            .push_bind(format!(
                "%{}%",
                search
                    .replace('\\', "\\\\")
                    .replace('%', "\\%")
                    .replace('_', "\\_")
            ))
            .push(" ESCAPE '\\'");
    }
    if let Some(favorite) = query.favorite {
        builder.push(" AND s.favorite=").push_bind(favorite);
    }
    if let Some(pinned) = query.pinned {
        builder.push(" AND s.pinned=").push_bind(pinned);
    }
    if !query.states.is_empty() {
        builder.push(" AND (").push(STATE_SQL).push(") IN (");
        let mut separated = builder.separated(",");
        for state in &query.states {
            separated.push_bind(state.as_str());
        }
        separated.push_unseparated(")");
    }
    if cursor && let Some(before) = query.before {
        builder
            .push(" AND (s.updated_at_ms,s.id)<(")
            .push_bind(before.updated_at_ms)
            .push(",")
            .push_bind(before.id.get())
            .push(")");
    }
}
pub(super) async fn query_summaries(
    backend: &Backend,
    query: SummaryQuery,
) -> Result<ReadView<SessionSummaryPage>> {
    backend.cache.ensure_open()?;
    if query.workspace_id.is_some_and(|id| id <= 0) {
        return Err(invalid("invalid workspace"));
    }
    let workspace = query.workspace_id.unwrap_or(0);
    if query
        .before
        .is_some_and(|cursor| cursor.workspace_id != workspace)
    {
        return Err(KernelError::Ownership);
    }
    let _read = backend
        .control_reads
        .clone()
        .try_acquire_owned()
        .map_err(|_| KernelError::Busy)?;
    let mut tx = backend
        .reader
        .get_sqlite_connection_pool()
        .begin()
        .await
        .map_err(storage)?;
    let limit = if query.limit == 0 {
        200
    } else {
        query.limit.clamp(1, 200)
    };
    let fields = reads::SUMMARY_SQL
        .rsplit_once(" FROM agena_sessions s ")
        .map(|(fields, _)| fields)
        .ok_or_else(|| storage("invalid summary fields"))?;
    let mut builder = sqlx::QueryBuilder::new(format!(
        "{fields} FROM agena_sessions s JOIN agena_session_statistics t ON t.session_id=s.id WHERE "
    ));
    filter(&mut builder, &query, true);
    builder
        .push(" ORDER BY s.updated_at_ms DESC,s.id DESC LIMIT ")
        .push_bind((limit + 1) as i64)
        .push(" OFFSET ")
        .push_bind(i64::try_from(query.offset).map_err(invalid)?);
    let mut rows = builder
        .build()
        .fetch_all(&mut *tx)
        .await
        .map_err(sql_error)?;
    let has_more = rows.len() > limit;
    rows.truncate(limit);
    let sessions = rows
        .iter()
        .map(reads::summary_row)
        .collect::<Result<Vec<_>>>()?;
    let next = if has_more {
        sessions.last().map(|value| SessionSummaryCursor {
            workspace_id: workspace,
            updated_at_ms: value.updated_at_ms,
            id: value.id,
        })
    } else {
        None
    };
    let mut count = sqlx::QueryBuilder::new(
        "SELECT count(*) FROM agena_sessions s JOIN agena_session_statistics t ON t.session_id=s.id WHERE ",
    );
    filter(&mut count, &query, false);
    let total: i64 = count
        .build_query_scalar()
        .fetch_one(&mut *tx)
        .await
        .map_err(sql_error)?;
    tx.rollback().await.map_err(storage)?;
    let value = SessionSummaryPage {
        workspace_id: workspace,
        sessions,
        next,
        has_more,
        total: Some(total as u64),
    };
    let bytes = modeled_read_bytes(&value)?;
    Ok(ReadView::from_lease(
        value,
        backend
            .views
            .reserve_owned(ReadOwner::Workspace(workspace), bytes)?,
    ))
}
pub(super) async fn workspace_statistics(
    backend: &Backend,
    ids: &[i64],
) -> Result<ReadView<BTreeMap<i64, WorkspaceStatistics>>> {
    backend.cache.ensure_open()?;
    if ids.len() > 256 || ids.iter().any(|id| *id <= 0) {
        return Err(invalid(
            "workspace statistics admits at most 256 valid workspace ids",
        ));
    }
    let _read = backend
        .control_reads
        .clone()
        .try_acquire_owned()
        .map_err(|_| KernelError::Busy)?;
    let sql = format!(
        "SELECT s.workspace_id,count(*) AS total,sum(s.parent_session_id IS NULL) AS roots,sum(s.pinned) AS pinned,sum(({STATE_SQL}) IN ('running','awaiting_interaction')) AS running,sum(({STATE_SQL}) IN ('awaiting_interaction','failed')) AS attention FROM agena_sessions s JOIN agena_session_statistics t ON t.session_id=s.id WHERE s.deleted_at_ms IS NULL AND s.workspace_id IN (SELECT value FROM json_each(?)) AND s.temporary=0 GROUP BY s.workspace_id"
    );
    let rows = sqlx::query(&sql)
        .bind(encode(ids)?)
        .fetch_all(backend.reader.get_sqlite_connection_pool())
        .await
        .map_err(sql_error)?;
    let mut value = ids
        .iter()
        .map(|id| (*id, WorkspaceStatistics::default()))
        .collect::<BTreeMap<_, _>>();
    for row in rows {
        let count = |name| {
            row.try_get::<i64, _>(name)
                .map(|v| v as u64)
                .map_err(storage)
        };
        value.insert(
            row.try_get("workspace_id").map_err(storage)?,
            WorkspaceStatistics {
                total: count("total")?,
                roots: count("roots")?,
                pinned: count("pinned")?,
                running: count("running")?,
                attention: count("attention")?,
            },
        );
    }
    let bytes = modeled_read_bytes(&value)?;
    Ok(ReadView::from_lease(
        value,
        backend
            .views
            .reserve_owned(ReadOwner::Workspace(0), bytes)?,
    ))
}
pub(super) async fn rounds(
    backend: &Backend,
    session: SessionId,
    after: Option<RoundId>,
    limit: usize,
) -> Result<ReadView<Vec<ProviderRoundView>>> {
    backend.cache.ensure_open()?;
    let _read = backend
        .content_reads
        .clone()
        .try_acquire_owned()
        .map_err(|_| KernelError::Busy)?;
    let mut tx = backend
        .reader
        .get_sqlite_connection_pool()
        .begin()
        .await
        .map_err(storage)?;
    reads::metadata(&mut tx, session).await?;
    let rows=sqlx::query("SELECT id,provider_state_bytes FROM agena_rounds WHERE session_id=? AND id>? ORDER BY id LIMIT ?").bind(session.get()).bind(after.map_or(0,|id|id.get())).bind(limit.clamp(1,128) as i64).fetch_all(&mut *tx).await.map_err(sql_error)?;
    let mut values = Vec::new();
    let mut bytes = 0usize;
    for row in rows {
        let id =
            RoundId::try_from(row.try_get::<i64, _>("id").map_err(storage)?).map_err(storage)?;
        let cost = row
            .try_get::<i64, _>("provider_state_bytes")
            .map_err(storage)? as usize
            + 10000 * 16
            + 512;
        if !values.is_empty() && bytes + cost > 4 * 1024 * 1024 {
            break;
        }
        let row=sqlx::query("SELECT execution_id,phase,provider_state_json,created_at_ms,finished_at_ms FROM agena_rounds WHERE id=?").bind(id.get()).fetch_one(&mut *tx).await.map_err(sql_error)?;
        let state = row
            .try_get::<Option<&str>, _>("provider_state_json")
            .map_err(storage)?;
        let input: Vec<i64> = sqlx::query_scalar(
            "SELECT entry_id FROM agena_round_inputs WHERE round_id=? ORDER BY ordinal LIMIT 10001",
        )
        .bind(id.get())
        .fetch_all(&mut *tx)
        .await
        .map_err(sql_error)?;
        if input.len() > 10000 {
            return Err(storage("round input exceeds identity budget"));
        }
        values.push(ProviderRoundView {
            execution_id: row
                .try_get::<&str, _>("execution_id")
                .map_err(storage)?
                .parse()
                .map_err(storage)?,
            created_at_ms: row.try_get("created_at_ms").map_err(storage)?,
            finished_at_ms: row.try_get("finished_at_ms").map_err(storage)?,
            id,
            phase: decode(&encode(&row.try_get::<&str, _>("phase").map_err(storage)?)?)?,
            inputs: input
                .into_iter()
                .map(EntryId::try_from)
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(storage)?,
            provider_state: state.map(decode).transpose()?,
        });
        bytes += cost;
    }
    tx.rollback().await.map_err(storage)?;
    let bytes = modeled_read_bytes(&values)?;
    backend.wrap_read(session, values, bytes)
}
pub(super) async fn pending_deliveries(
    backend: &Backend,
    session: SessionId,
    after: Option<agena_domain::CommandId>,
    limit: usize,
) -> Result<ReadView<Vec<DeliveryView>>> {
    backend.cache.ensure_open()?;
    let _read = backend
        .content_reads
        .clone()
        .try_acquire_owned()
        .map_err(|_| KernelError::Busy)?;
    let mut tx = backend
        .reader
        .get_sqlite_connection_pool()
        .begin()
        .await
        .map_err(storage)?;
    reads::metadata(&mut tx, session).await?;
    let ids:Vec<String>=sqlx::query_scalar("SELECT id FROM agena_deliveries WHERE session_id=? AND state='pending' AND id>? ORDER BY id LIMIT ?").bind(session.get()).bind(after.map(|id|id.to_string()).unwrap_or_default()).bind(limit.clamp(1,64) as i64).fetch_all(&mut *tx).await.map_err(sql_error)?;
    let mut values = Vec::with_capacity(ids.len());
    for id in ids {
        values.push(delivery(&mut tx, session, id.parse().map_err(storage)?).await?);
    }
    tx.rollback().await.map_err(storage)?;
    let bytes = modeled_read_bytes(&values)?;
    backend.wrap_read(session, values, bytes)
}

pub(super) async fn find_agent(
    backend: &Backend,
    parent: SessionId,
    agent_name: &str,
) -> Result<ReadView<Option<SessionId>>> {
    backend.cache.ensure_open()?;
    if agent_name.is_empty() || agent_name.len() > 4096 {
        return Err(invalid("invalid agent identity"));
    }
    let _read = backend
        .control_reads
        .clone()
        .try_acquire_owned()
        .map_err(|_| KernelError::Busy)?;
    let mut tx = backend
        .reader
        .get_sqlite_connection_pool()
        .begin()
        .await
        .map_err(storage)?;
    reads::metadata(&mut tx, parent).await?;
    let id:Option<i64>=sqlx::query_scalar("SELECT id FROM agena_sessions WHERE parent_session_id=? AND agent_name=? AND deleted_at_ms IS NULL ORDER BY id LIMIT 1")
        .bind(parent.get()).bind(agent_name).fetch_optional(&mut *tx).await.map_err(sql_error)?;
    let value = id.map(SessionId::try_from).transpose().map_err(storage)?;
    backend.wrap_read(parent, value, std::mem::size_of::<Option<SessionId>>())
}

pub(super) async fn history_after(
    backend: &Backend,
    session: SessionId,
    after: Option<EntryId>,
    limit: usize,
) -> Result<ReadView<HistoryPage>> {
    backend.cache.ensure_open()?;
    let _read = backend
        .content_reads
        .clone()
        .try_acquire_owned()
        .map_err(|_| KernelError::Busy)?;
    let mut tx = backend
        .reader
        .get_sqlite_connection_pool()
        .begin()
        .await
        .map_err(storage)?;
    let metadata = reads::metadata(&mut tx, session).await?;
    let (sequence, ordinal) = if let Some(after) = after {
        boundary(&mut tx, session, after).await?
    } else {
        (-1, -1)
    };
    let limit = limit.clamp(1, 200);
    let segments=sqlx::query("SELECT sequence,segment_id,first_ordinal,end_ordinal FROM agena_session_segments WHERE session_id=? AND sequence>=? ORDER BY sequence LIMIT ?")
        .bind(session.get()).bind(sequence).bind((limit+2) as i64).fetch_all(&mut *tx).await.map_err(sql_error)?;
    metrics::add(&backend.live.diagnostics().0.history_queries, 2);
    let mut selected = Vec::<i64>::with_capacity(limit);
    let mut bytes = 0usize;
    let mut next = None;
    let mut has_more = false;
    'segments: for segment in segments {
        let seq: i64 = segment.try_get("sequence").map_err(storage)?;
        let first = segment
            .try_get::<i64, _>("first_ordinal")
            .map_err(storage)?
            .max(if seq == sequence { ordinal + 1 } else { 0 });
        let rows=sqlx::query("SELECT m.entry_id,m.ordinal,e.body_bytes FROM agena_segment_entries m JOIN agena_entries e ON e.id=m.entry_id WHERE m.segment_id=? AND m.ordinal>=? AND m.ordinal<? ORDER BY m.ordinal LIMIT ?")
            .bind(segment.try_get::<i64,_>("segment_id").map_err(storage)?).bind(first).bind(segment.try_get::<i64,_>("end_ordinal").map_err(storage)?).bind((limit+1-selected.len()) as i64).fetch_all(&mut *tx).await.map_err(sql_error)?;
        metrics::add(&backend.live.diagnostics().0.history_queries, 1);
        metrics::add(
            &backend.live.diagnostics().0.history_candidate_rows,
            rows.len(),
        );
        for row in rows {
            let charge = usize::try_from(row.try_get::<i64, _>("body_bytes").map_err(storage)?)
                .map_err(storage)?
                .saturating_add(512);
            if selected.len() == limit || bytes.saturating_add(charge) > 1024 * 1024 {
                if selected.is_empty() {
                    return Err(KernelError::Busy);
                }
                has_more = true;
                break 'segments;
            }
            selected.push(row.try_get("entry_id").map_err(storage)?);
            bytes += charge;
            next = Some(HistoryCursor {
                session,
                membership: metadata.membership_generation,
                segment_sequence: seq,
                ordinal: u32::try_from(row.try_get::<i64, _>("ordinal").map_err(storage)?)
                    .map_err(storage)?,
                anchor: Some(
                    EntryId::try_from(row.try_get::<i64, _>("entry_id").map_err(storage)?)
                        .map_err(storage)?,
                ),
            });
        }
    }
    let mut entries = Vec::with_capacity(selected.len());
    if !selected.is_empty() {
        let rows=sqlx::query("SELECT e.*,EXISTS(SELECT 1 FROM agena_attempts started WHERE started.execution_id=e.execution_id AND started.started_at_ms IS NOT NULL) AS execution_started,CASE WHEN e.kind='user' THEN (SELECT x.current_generation FROM agena_executions x WHERE x.id=e.execution_id) ELSE e.generation END AS execution_generation,(SELECT a.response_entry_id FROM agena_attempts a WHERE a.execution_id=e.execution_id AND a.generation=e.generation) AS response_entry_id,(SELECT a.phase FROM agena_attempts a WHERE a.execution_id=e.execution_id AND a.generation=CASE WHEN e.kind='user' THEN (SELECT x.current_generation FROM agena_executions x WHERE x.id=e.execution_id) ELSE e.generation END) AS execution_phase FROM agena_entries e WHERE e.id IN (SELECT value FROM json_each(?))")
            .bind(encode(&selected)?).fetch_all(&mut *tx).await.map_err(sql_error)?;
        metrics::add(&backend.live.diagnostics().0.history_queries, 1);
        metrics::add(&backend.live.diagnostics().0.history_entry_rows, rows.len());
        let mut by_id = std::collections::HashMap::with_capacity(rows.len());
        for row in rows {
            metrics::add(
                &backend.live.diagnostics().0.history_entry_body_bytes,
                row.try_get::<Option<&str>, _>("body_json")
                    .map_err(storage)?
                    .map_or(0, str::len),
            );
            by_id.insert(
                row.try_get::<i64, _>("id").map_err(storage)?,
                reads::entry_row(&row)?,
            );
        }
        for id in selected {
            entries.push(
                by_id
                    .remove(&id)
                    .ok_or_else(|| storage("history entry disappeared"))?,
            );
        }
    }
    let value = HistoryPage {
        metadata,
        entries,
        next: has_more.then_some(next).flatten(),
        has_more,
    };
    let bytes = modeled_read_bytes(&value)?;
    backend.wrap_read(session, value, bytes)
}

pub(super) async fn background_activities(
    backend: &Backend,
    session: Option<SessionId>,
    after: Option<agena_domain::ActivityId>,
    active_only: bool,
    limit: usize,
) -> Result<ReadView<Vec<BackgroundActivityView>>> {
    backend.cache.ensure_open()?;
    let _read = backend
        .control_reads
        .clone()
        .try_acquire_owned()
        .map_err(|_| KernelError::Busy)?;
    let mut tx = backend
        .reader
        .get_sqlite_connection_pool()
        .begin()
        .await
        .map_err(storage)?;
    if let Some(session) = session {
        reads::metadata(&mut tx, session).await?;
    }
    let mut query = sqlx::QueryBuilder::new(
        "SELECT a.* FROM agena_background_activities a JOIN agena_sessions s ON s.id=a.session_id WHERE s.deleted_at_ms IS NULL AND a.id>",
    );
    query.push_bind(after.map(|id| id.to_string()).unwrap_or_default());
    if let Some(session) = session {
        query.push(" AND a.session_id=").push_bind(session.get());
    }
    if active_only {
        query.push(" AND a.phase='running'");
    }
    query
        .push(" ORDER BY a.id LIMIT ")
        .push_bind(limit.clamp(1, 256) as i64);
    let rows = query.build().fetch_all(&mut *tx).await.map_err(sql_error)?;
    let value = rows
        .iter()
        .map(activity::view)
        .collect::<Result<Vec<_>>>()?;
    let bytes = modeled_read_bytes(&value)?;
    if let Some(session) = session {
        backend.wrap_read(session, value, bytes)
    } else {
        Ok(ReadView::from_lease(
            value,
            backend
                .views
                .reserve_owned(ReadOwner::Workspace(0), bytes)?,
        ))
    }
}

fn prompt_status_row(row: &sqlx::sqlite::SqliteRow) -> Result<PromptWindowStatus> {
    let entry = |name| {
        row.try_get::<Option<i64>, _>(name)
            .map_err(storage)?
            .map(EntryId::try_from)
            .transpose()
            .map_err(storage)
    };
    let tokens = |name| {
        row.try_get::<Option<i64>, _>(name)
            .map_err(storage)?
            .map(u64::try_from)
            .transpose()
            .map_err(storage)
    };
    Ok(PromptWindowStatus {
        generation: u64::try_from(row.try_get::<i64, _>("rewrite_epoch").map_err(storage)?)
            .map_err(storage)?,
        checkpoint_entry_id: entry("checkpoint_entry_id")?,
        through_entry_id: entry("through_entry_id")?,
        policy_key: row.try_get("policy_key").map_err(storage)?,
        consecutive_compaction_failures: u32::try_from(
            row.try_get::<i64, _>("consecutive_compaction_failures")
                .map_err(storage)?,
        )
        .map_err(storage)?,
        auto_compaction_disabled: row.try_get("auto_compaction_disabled").map_err(storage)?,
        native_compaction_unavailable: row
            .try_get("native_compaction_unavailable")
            .map_err(storage)?,
        last_before_tokens: tokens("last_before_tokens")?,
        last_after_tokens: tokens("last_after_tokens")?,
    })
}
async fn record_compaction(
    connection: &mut SqliteConnection,
    scope: port::Scope,
    key: &str,
    outcome: &CompactionPolicyOutcome,
) -> Result<Mutation> {
    commands::check_scope(connection, scope, true).await?;
    if key.is_empty() || key.len() > 1024 {
        return Err(invalid("compaction policy key must contain 1..1024 bytes"));
    }
    let row = sqlx::query("SELECT * FROM agena_prompt_windows WHERE session_id=?")
        .bind(scope.session.get())
        .fetch_one(&mut *connection)
        .await
        .map_err(sql_error)?;
    let mut status = prompt_status_row(&row)?;
    if status.policy_key.as_deref() != Some(key) {
        status.policy_key = Some(key.to_owned());
        status.consecutive_compaction_failures = 0;
        status.auto_compaction_disabled = false;
        status.native_compaction_unavailable = false;
        status.last_before_tokens = None;
        status.last_after_tokens = None;
    }
    match outcome {
        CompactionPolicyOutcome::Succeeded {
            native,
            before_tokens,
            after_tokens,
        } => {
            status.consecutive_compaction_failures = 0;
            status.auto_compaction_disabled = false;
            if *native {
                status.native_compaction_unavailable = false;
            }
            status.last_before_tokens = *before_tokens;
            status.last_after_tokens = *after_tokens;
        }
        CompactionPolicyOutcome::Failed => {
            status.consecutive_compaction_failures =
                (status.consecutive_compaction_failures + 1).min(3);
            status.auto_compaction_disabled = status.consecutive_compaction_failures >= 3;
        }
        CompactionPolicyOutcome::NativeUnavailable => status.native_compaction_unavailable = true,
        CompactionPolicyOutcome::Unchanged => {}
    }
    let before = status
        .last_before_tokens
        .map(i64::try_from)
        .transpose()
        .map_err(invalid)?;
    let after = status
        .last_after_tokens
        .map(i64::try_from)
        .transpose()
        .map_err(invalid)?;
    sqlx::query("UPDATE agena_prompt_windows SET policy_key=?,consecutive_compaction_failures=?,auto_compaction_disabled=?,native_compaction_unavailable=?,last_before_tokens=?,last_after_tokens=? WHERE session_id=?").bind(key).bind(i64::from(status.consecutive_compaction_failures)).bind(status.auto_compaction_disabled).bind(status.native_compaction_unavailable).bind(before).bind(after).bind(scope.session.get()).execute(connection).await.map_err(sql_error)?;
    Ok(Mutation::PromptWindowStatus(status))
}
pub(super) async fn prompt_window_status(
    backend: &Backend,
    session: SessionId,
) -> Result<ReadView<PromptWindowStatus>> {
    backend.cache.ensure_open()?;
    let _read = backend
        .control_reads
        .clone()
        .try_acquire_owned()
        .map_err(|_| KernelError::Busy)?;
    let mut tx = backend
        .reader
        .get_sqlite_connection_pool()
        .begin()
        .await
        .map_err(storage)?;
    reads::metadata(&mut tx, session).await?;
    let row = sqlx::query("SELECT * FROM agena_prompt_windows WHERE session_id=?")
        .bind(session.get())
        .fetch_one(&mut *tx)
        .await
        .map_err(sql_error)?;
    let value = prompt_status_row(&row)?;
    let bytes = modeled_read_bytes(&value)?;
    backend.wrap_read(session, value, bytes)
}
