use agena_domain::{ExternalAgent, SessionRelationKind};
use agena_storage::backend::content_blocks::DEFAULT_TEXT_BLOCK_BYTES;

use super::*;

pub(super) fn state_budget(configuration_bytes: usize) -> usize {
    // Allow room for the state key and the accompanying title metadata.
    (64 * 1024_usize).saturating_sub(configuration_bytes + 128)
}

pub(super) fn bound_configuration_state(configuration: &mut SessionConfiguration) -> Result<()> {
    if !configuration.external_agent_state.is_empty() {
        let state = std::mem::take(&mut configuration.external_agent_state);
        configuration.external_agent_state =
            state.bounded_snapshot(state_budget(encode(configuration)?.len()));
    }
    Ok(())
}

pub(super) fn validate_link(link: &ExternalAgentLink) -> Result<()> {
    if !link.cwd.is_absolute()
        || link.native_id.trim().is_empty()
        || link.native_id.len() > 4096
        || link.native_id.chars().any(char::is_control)
    {
        return Err(invalid("invalid native session identity"));
    }
    Ok(())
}

/// All native bindings are written in the serialized kernel transaction so
/// repeated imports or racing commands cannot create duplicate owners.
pub(super) async fn existing_link(
    connection: &mut SqliteConnection,
    workspace: i64,
    agent: ExternalAgent,
    link: &ExternalAgentLink,
) -> Result<Option<SessionId>> {
    let id: Option<i64> = sqlx::query_scalar(
        "SELECT id FROM agena_sessions
         WHERE workspace_id=? AND deleted_at_ms IS NULL
           AND json_extract(configuration_json,'$.external_agent')=?
           AND COALESCE(json_extract(configuration_json,'$.external_agent_link.native_id'),
                        json_extract(configuration_json,'$.external_agent_resume.native_id'))=?
           AND COALESCE(json_extract(configuration_json,'$.external_agent_link.cwd'),
                        json_extract(configuration_json,'$.external_agent_resume.cwd'))=?
         LIMIT 1",
    )
    .bind(workspace)
    .bind(agent.id())
    .bind(&link.native_id)
    .bind(link.cwd.to_string_lossy().as_ref())
    .fetch_optional(connection)
    .await
    .map_err(sql_error)?;
    id.map(|id| SessionId::try_from(id).map_err(storage))
        .transpose()
}

pub(super) async fn require_idle(
    connection: &mut SqliteConnection,
    session: SessionId,
) -> Result<()> {
    let busy: bool = sqlx::query_scalar(
        "SELECT queued_executions+running_executions>0 FROM agena_session_statistics WHERE session_id=?",
    )
    .bind(session.get())
    .fetch_one(connection)
    .await
    .map_err(sql_error)?;
    if busy {
        return Err(KernelError::Conflict(
            "the external agent session is busy".into(),
        ));
    }
    Ok(())
}

pub(super) async fn fork_points(
    connection: &mut SqliteConnection,
    session: SessionId,
    link: &ExternalAgentLink,
    message: Option<EntryId>,
) -> Result<Vec<(ExternalAgentForkCursor, String)>> {
    let Some(incarnation) = link.incarnation else {
        return Ok(Vec::new());
    };
    let rows = sqlx::query(
        "SELECT r.provider_state_json -> '$.replay' AS cursor,
                (SELECT substr(trim(node.value ->> '$.text'),1,160)
                 FROM json_each(u.body_json,'$.document') node
                 WHERE node.value ->> '$.type' = 'text' AND trim(node.value ->> '$.text') != ''
                 LIMIT 1) AS label
         FROM agena_executions x
         JOIN agena_entries u ON u.id=x.message_id AND u.origin_session_id=x.session_id AND u.kind='user'
         JOIN agena_attempts a ON a.execution_id=x.id AND a.generation=x.current_generation
         JOIN agena_rounds r ON r.execution_id=x.id AND r.generation=a.generation
         WHERE x.session_id=?1 AND x.message_id BETWEEN COALESCE(?4,1) AND COALESCE(?4,9223372036854775807)
           AND a.phase='succeeded' AND r.phase='succeeded'
           AND r.provider_state_json ->> '$.replay.native_id' = ?2
           AND r.provider_state_json ->> '$.replay.incarnation' = ?3
           AND r.provider_state_json ->> '$.replay.message_id' = x.message_id
           AND r.provider_state_json ->> '$.replay.cursor_id' IS NOT NULL
           AND (COALESCE(r.provider_state_json ->> '$.replay.fork',1)=1
                OR r.provider_state_json ->> '$.replay.rewind'=1)
           AND EXISTS(SELECT 1 FROM agena_segment_entries m
                      JOIN agena_session_segments s ON s.segment_id=m.segment_id
                      WHERE s.session_id=x.session_id AND m.entry_id=x.message_id
                        AND m.ordinal>=s.first_ordinal AND m.ordinal<s.end_ordinal)
         ORDER BY x.message_id DESC LIMIT ?5",
    )
    .bind(session.get())
    .bind(&link.native_id)
    .bind(incarnation.get())
    .bind(message.map(EntryId::get))
    .bind(if message.is_some() { 1_i64 } else { 50 })
    .fetch_all(connection)
    .await
    .map_err(sql_error)?;
    rows.into_iter()
        .map(|row| {
            let cursor =
                decode::<ExternalAgentForkCursor>(row.try_get("cursor").map_err(storage)?)?;
            let label = row
                .try_get::<Option<String>, _>("label")
                .map_err(storage)?
                .map(|label| label.split_whitespace().collect::<Vec<_>>().join(" "))
                .filter(|label| !label.is_empty())
                .unwrap_or_else(|| format!("Message {}", cursor.message_id.get()));
            Ok((cursor, label))
        })
        .collect()
}

pub(super) async fn update_cursor(
    connection: &mut SqliteConnection,
    session: SessionId,
    link: &ExternalAgentLink,
    change: &ExternalAgentCursorChange,
) -> Result<()> {
    let Some(incarnation) = link.incarnation else {
        return Ok(());
    };
    let known: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM agena_rounds
         WHERE session_id=?1 AND phase='succeeded'
           AND provider_state_json ->> '$.replay.native_id'=?2
           AND provider_state_json ->> '$.replay.incarnation'=?3
           AND provider_state_json ->> '$.replay.cursor_id'=?4)",
    )
    .bind(session.get())
    .bind(&link.native_id)
    .bind(incarnation.get())
    .bind(&change.cursor_id)
    .fetch_one(&mut *connection)
    .await
    .map_err(sql_error)?;
    let new_user = (!known && (change.fork || change.rewind))
        .then_some(change.before_user.as_deref())
        .flatten();
    if !known && new_user.is_none() {
        return Ok(());
    }
    // A previously announced cursor keeps its original user anchor. A new
    // cursor can only attach to an exact native message ID we already hold.
    sqlx::query(
        "UPDATE agena_rounds SET provider_state_json=json_set(provider_state_json,
          '$.replay.cursor_id',?4,'$.replay.fork',json(?5),'$.replay.rewind',json(?6))
         WHERE session_id=?1 AND phase='succeeded'
           AND provider_state_json ->> '$.replay.native_id'=?2
           AND provider_state_json ->> '$.replay.incarnation'=?3
           AND (provider_state_json ->> '$.replay.cursor_id'=?4
                OR provider_state_json ->> '$.replay.native_message_id'=?7)",
    )
    .bind(session.get())
    .bind(&link.native_id)
    .bind(incarnation.get())
    .bind(&change.cursor_id)
    .bind(if change.fork { "true" } else { "false" })
    .bind(if change.rewind { "true" } else { "false" })
    .bind(new_user)
    .execute(connection)
    .await
    .map_err(sql_error)?;
    Ok(())
}

pub(super) async fn import(
    connection: &mut SqliteConnection,
    workspace: i64,
    agent: ExternalAgent,
    title: &str,
    link: &ExternalAgentLink,
    turns: &[ExternalAgentHistoryTurn],
    state: &ExternalAgentState,
    replace_view: Option<SessionId>,
) -> Result<Mutation> {
    validate_link(link)?;
    if title.trim().is_empty() || title.len() > 4096 || title.chars().any(char::is_control) {
        return Err(invalid("invalid native session title"));
    }
    let previous = if let Some(existing) = existing_link(connection, workspace, agent, link).await?
    {
        if replace_view != Some(existing) {
            return Ok(Mutation::Session(Box::new(
                reads::metadata(connection, existing).await?,
            )));
        }
        require_idle(connection, existing).await?;
        let metadata = reads::metadata(connection, existing).await?;
        let configuration = super::lifecycle::configuration(connection, existing).await?;
        super::lifecycle::delete(connection, existing).await?;
        Some((metadata, configuration))
    } else {
        None
    };
    let (previous_metadata, mut configuration) = match previous {
        Some((metadata, config)) => (Some(metadata), config),
        None => (None, SessionConfiguration::default()),
    };
    let title = previous_metadata
        .as_ref()
        .filter(|_| configuration.title_state.source == SessionTitleSource::Manual)
        .map_or(title, |metadata| metadata.title.as_str());
    let now = now_ms();
    let Mutation::Session(mut created) =
        commands::create_session(connection, Some(workspace), title, now).await?
    else {
        return Err(storage("invalid native session creation"));
    };
    let session = created.id;
    if let Some(metadata) = previous_metadata {
        created.selection = metadata.selection;
        created.favorite = metadata.favorite;
        created.pinned = metadata.pinned;
        sqlx::query(
            "UPDATE agena_permission_rules SET session_id=? WHERE session_id=? AND scope='session'",
        )
        .bind(session.get())
        .bind(metadata.id.get())
        .execute(&mut *connection)
        .await
        .map_err(sql_error)?;
    }
    let mut link = link.clone();
    let record_cursors = turns.iter().any(|turn| turn.cursor.is_some());
    for turn in turns {
        if let Some(round) = import_turn(connection, &created, now, turn, agent, &link).await?
            && record_cursors
            && link.incarnation.is_none()
        {
            link.incarnation = Some(round);
        }
    }
    sqlx::query(
        "UPDATE agena_session_statistics
         SET user_message_count=?, started_user_message_count=?, last_message_at_ms=?
         WHERE session_id=?",
    )
    .bind(turns.len() as i64)
    .bind(turns.len() as i64)
    .bind((!turns.is_empty()).then_some(now))
    .bind(session.get())
    .execute(&mut *connection)
    .await
    .map_err(sql_error)?;
    let tail: Option<i64> =
        sqlx::query_scalar("SELECT MAX(id) FROM agena_entries WHERE origin_session_id=?")
            .bind(session.get())
            .fetch_one(&mut *connection)
            .await
            .map_err(sql_error)?;
    link.resume_at = Some(ExternalAgentHistoryAnchor {
        tail: tail.map(EntryId::try_from).transpose().map_err(storage)?,
    });
    configuration.external_agent = Some(agent);
    configuration.external_agent_link = Some(link);
    configuration.external_agent_state = state.clone();
    configuration.external_agent_resume = None;
    configuration.external_agent_pending_turn = None;
    configuration.external_agent_inputs = None;
    configuration.parent_session_id = None;
    configuration.relation_kind = SessionRelationKind::Root;
    configuration.conversation = None;
    configuration.agent = None;
    bound_configuration_state(&mut configuration)?;
    sqlx::query("UPDATE agena_sessions SET configuration_json=?,selection_json=?,favorite=?,pinned=? WHERE id=?")
        .bind(encode(&configuration)?)
        .bind(created.selection.as_ref().map(encode).transpose()?)
        .bind(created.favorite)
        .bind(created.pinned)
        .bind(session.get())
        .execute(&mut *connection)
        .await
        .map_err(sql_error)?;
    Ok(Mutation::Session(Box::new(
        reads::metadata(connection, session).await?,
    )))
}

async fn import_turn(
    connection: &mut SqliteConnection,
    metadata: &SessionMetadata,
    now: i64,
    turn: &ExternalAgentHistoryTurn,
    agent: ExternalAgent,
    link: &ExternalAgentLink,
) -> Result<Option<RoundId>> {
    if turn.user.is_empty() {
        return Err(invalid("imported user message is empty"));
    }
    let session = metadata.id;
    let execution = ExecutionId::new().to_string();
    // Native history is historical evidence, never verified operator input
    // with authority to start new tool work.
    let body = encode_user_body(&turn.user, None, None)?;
    if body.len() > 256 * 1024 {
        return Err(invalid("imported composer document exceeds 256 KiB"));
    }
    let user: i64 = sqlx::query_scalar(
        "INSERT INTO agena_entries (origin_session_id,execution_id,generation,kind,body_json,created_at_ms)
         VALUES (?,?,1,'user',?,?) RETURNING id",
    )
    .bind(session.get())
    .bind(&execution)
    .bind(body)
    .bind(now)
    .fetch_one(&mut *connection)
    .await
    .map_err(sql_error)?;
    sqlx::query(
        "INSERT INTO agena_executions (id,session_id,message_id,current_generation,created_at_ms)
         VALUES (?,?,?,1,?)",
    )
    .bind(&execution)
    .bind(session.get())
    .bind(user)
    .bind(now)
    .execute(&mut *connection)
    .await
    .map_err(sql_error)?;
    sqlx::query(
        "INSERT INTO agena_attempts (execution_id,generation,session_id,phase,started_at_ms,finished_at_ms)
         VALUES (?,1,?,'succeeded',?,?)",
    )
    .bind(&execution)
    .bind(session.get())
    .bind(now)
    .bind(now)
    .execute(&mut *connection)
    .await
    .map_err(sql_error)?;
    publish_history_blobs(connection, session, user, now, &turn.blobs).await?;
    commands::append_history(connection, session, user).await?;
    history_runs::start_message(
        connection,
        session,
        EntryId::try_from(user).map_err(storage)?,
    )
    .await?;
    if turn.prelude.is_empty() && turn.response.is_empty() && turn.cursor.is_none() {
        return Ok(None);
    }
    let round: i64 = sqlx::query_scalar(
        "INSERT INTO agena_rounds (execution_id,generation,session_id,ordinal,phase,
                                   input_metadata_json,created_at_ms,finished_at_ms)
         VALUES (?,1,?,0,'succeeded',?,?,?) RETURNING id",
    )
    .bind(&execution)
    .bind(session.get())
    .bind(encode(metadata)?)
    .bind(now)
    .bind(now)
    .fetch_one(&mut *connection)
    .await
    .map_err(sql_error)?;
    let round_id = RoundId::try_from(round).map_err(storage)?;
    if let Some(cursor) = &turn.cursor {
        let cursor = ExternalAgentForkCursor {
            native_id: link.native_id.clone(),
            incarnation: Some(link.incarnation.unwrap_or(round_id)),
            message_id: EntryId::try_from(user).map_err(storage)?,
            native_message_id: cursor.message_id.clone(),
            cursor_id: cursor.cursor_id.clone(),
            fork: cursor.fork,
            rewind: cursor.rewind,
        };
        let provider_state =
            serde_json::json!({"version": 1, "model": agent.run_identity(), "replay": cursor});
        sqlx::query("UPDATE agena_rounds SET provider_state_json=? WHERE id=?")
            .bind(encode(&provider_state)?)
            .bind(round)
            .execute(&mut *connection)
            .await
            .map_err(sql_error)?;
    }
    let round = ImportedRound {
        session,
        execution: &execution,
        user,
        round,
        now,
    };
    for part in turn.prelude.iter().chain(&turn.response) {
        let entry = match part {
            ExternalAgentHistoryPart::Text { text, reasoning } if !text.is_empty() => {
                import_text(connection, &round, text, *reasoning).await?
            }
            ExternalAgentHistoryPart::Text { .. } => continue,
            ExternalAgentHistoryPart::Notice { text } => sqlx::query_scalar(
                "INSERT INTO agena_entries (origin_session_id,execution_id,generation,message_id,
                                                kind,body_json,created_at_ms)
                     VALUES (?,?,1,?,'notice',?,?) RETURNING id",
            )
            .bind(session.get())
            .bind(&execution)
            .bind(user)
            .bind(encode(text)?)
            .bind(now)
            .fetch_one(&mut *connection)
            .await
            .map_err(sql_error)?,
            ExternalAgentHistoryPart::Tool {
                start,
                result,
                blobs,
            } => {
                let entry = import_tool(connection, &round, start, result).await?;
                publish_history_blobs(connection, session, entry, now, blobs).await?;
                entry
            }
        };
        commands::append_history(connection, session, entry).await?;
    }
    Ok(Some(round_id))
}

struct ImportedRound<'a> {
    session: SessionId,
    execution: &'a str,
    user: i64,
    round: i64,
    now: i64,
}

async fn import_text(
    connection: &mut SqliteConnection,
    round: &ImportedRound<'_>,
    text: &str,
    reasoning: bool,
) -> Result<i64> {
    let content = ContentId::new().to_string();
    let epoch = ContentEpoch::new().to_string();
    let entry: i64 = sqlx::query_scalar(
        "INSERT INTO agena_entries (origin_session_id,execution_id,generation,message_id,
                                    kind,round_id,content_id,created_at_ms)
         VALUES (?,?,1,?,?,?,?,?) RETURNING id",
    )
    .bind(round.session.get())
    .bind(round.execution)
    .bind(round.user)
    .bind(if reasoning { "reasoning" } else { "text" })
    .bind(round.round)
    .bind(&content)
    .bind(round.now)
    .fetch_one(&mut *connection)
    .await
    .map_err(sql_error)?;
    let source: i64 = sqlx::query_scalar(
        "INSERT INTO agena_content_streams (id,entry_id,session_id,execution_id,generation,
                                            kind,epoch,committed_head,capture_state)
         VALUES (?,?,?,?,1,'text',?,?,'complete') RETURNING source_key",
    )
    .bind(&content)
    .bind(entry)
    .bind(round.session.get())
    .bind(round.execution)
    .bind(epoch)
    .bind(text.len() as i64)
    .fetch_one(&mut *connection)
    .await
    .map_err(sql_error)?;
    write_text(connection, source, text).await?;
    Ok(entry)
}

async fn import_tool(
    connection: &mut SqliteConnection,
    round: &ImportedRound<'_>,
    start: &ToolStart,
    result: &ToolResult,
) -> Result<i64> {
    if start.call_id.is_empty()
        || start.call_id.len() > 4096
        || start.name.trim().is_empty()
        || start.name.len() > 4096
        || matches!(result.phase, ToolPhase::Preparing | ToolPhase::Running)
    {
        return Err(invalid("invalid imported native tool call"));
    }
    let tool: i64 = sqlx::query_scalar(
        "INSERT INTO agena_tool_calls (round_id,execution_id,generation,call_id,name,input_json,
                                       execution_kind,phase,result_json,created_at_ms,finished_at_ms)
         VALUES (?,?,1,?,?,?,'provider_hosted',?,?,?,?) RETURNING id",
    )
    .bind(round.round)
    .bind(round.execution)
    .bind(&start.call_id)
    .bind(&start.name)
    .bind(encode(&start.input)?)
    .bind(commands::enum_text(&result.phase)?)
    .bind(encode(&result.value)?)
    .bind(round.now)
    .bind(round.now)
    .fetch_one(&mut *connection)
    .await
    .map_err(sql_error)?;
    sqlx::query_scalar(
        "INSERT INTO agena_entries (origin_session_id,execution_id,generation,message_id,
                                    kind,round_id,tool_id,created_at_ms)
         VALUES (?,?,1,?,'tool',?,?,?) RETURNING id",
    )
    .bind(round.session.get())
    .bind(round.execution)
    .bind(round.user)
    .bind(round.round)
    .bind(tool)
    .bind(round.now)
    .fetch_one(connection)
    .await
    .map_err(sql_error)
}

async fn publish_history_blobs(
    connection: &mut SqliteConnection,
    session: SessionId,
    entry: i64,
    now: i64,
    blobs: &[ExternalAgentHistoryBlob],
) -> Result<()> {
    if blobs.len() > 16 {
        return Err(invalid("at most 16 managed blobs per entry"));
    }
    // This session did not exist at preparation time. Publish the source and
    // its ownership reference in the same transaction as its history entry.
    for (index, blob) in blobs.iter().enumerate() {
        let digest: [u8; 32] = Sha256::digest(&blob.bytes).into();
        if blob.bytes.is_empty()
            || blob.descriptor.byte_len != blob.bytes.len() as u64
            || digest != blob.descriptor.sha256
        {
            return Err(invalid("imported attachment does not match its descriptor"));
        }
        let id = blob.descriptor.id.to_string();
        sqlx::query(
            "INSERT INTO agena_blobs (id,session_id,state,expected_bytes,received_bytes,digest,created_at_ms)
             VALUES (?,?,'ready',?,?,?,?)",
        )
        .bind(&id)
        .bind(session.get())
        .bind(blob.descriptor.byte_len as i64)
        .bind(blob.descriptor.byte_len as i64)
        .bind(digest.as_slice())
        .bind(now)
        .execute(&mut *connection)
        .await
        .map_err(sql_error)?;
        for (chunk, body) in blob.bytes.chunks(blobs::CHUNK_BYTES).enumerate() {
            let sha256 = Sha256::digest(body);
            sqlx::query(
                "INSERT INTO agena_blob_chunks (blob_id,logical_start,digest,body) VALUES (?,?,?,?)",
            )
            .bind(&id)
            .bind((chunk * blobs::CHUNK_BYTES) as i64)
            .bind(sha256.as_slice())
            .bind(body)
            .execute(&mut *connection)
            .await
            .map_err(sql_error)?;
        }
        sqlx::query("INSERT INTO agena_blob_references (entry_id,blob_id,field) VALUES (?,?,?)")
            .bind(entry)
            .bind(&id)
            .bind(index.to_string())
            .execute(&mut *connection)
            .await
            .map_err(sql_error)?;
    }
    Ok(())
}

async fn write_text(connection: &mut SqliteConnection, source: i64, text: &str) -> Result<()> {
    let mut start = 0;
    while start < text.len() {
        let end = text.floor_char_boundary(start + DEFAULT_TEXT_BLOCK_BYTES);
        let body = &text.as_bytes()[start..end];
        let digest = Sha256::digest(body);
        sqlx::query(
            "INSERT INTO agena_content_blocks (source_key,logical_start,logical_end,codec,digest,body_bytes,body)
             VALUES (?,?,?,'utf8',?,?,?)",
        )
        .bind(source)
        .bind(start as i64)
        .bind(end as i64)
        .bind(digest.as_slice())
        .bind(body.len() as i64)
        .bind(body)
        .execute(&mut *connection)
        .await
        .map_err(sql_error)?;
        start = end;
    }
    Ok(())
}

pub(super) async fn detach(
    connection: &mut SqliteConnection,
    session: SessionId,
    expected: &ExternalAgentLink,
) -> Result<Mutation> {
    require_idle(connection, session).await?;
    let current = lifecycle::configuration(connection, session).await?;
    if current.native_session_link().as_ref() != Some(expected) {
        return Err(KernelError::Conflict("the native session changed".into()));
    }
    lifecycle::delete(connection, session).await
}
