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
    let previous =
        if let Some(existing) = existing_link(connection, workspace, agent.clone(), link).await? {
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
        if let Some(round) = import_turn(connection, &created, now, turn, &agent, &link).await?
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
    agent: &ExternalAgent,
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

pub(super) struct ScannedRoundContinuity {
    pub round_inputs: Vec<EntryId>,
    pub continuity: ExternalAgentRoundContinuity,
}

const CONTINUITY_SELECT_COLS: &str = "e.id,e.kind,e.generation,e.round_id,
    COALESCE(f.revision,e.revision) AS revision,
    COALESCE(e.message_id,e.id) AS message_id,
    COALESCE((SELECT p.purpose FROM agena_execution_purposes p WHERE p.execution_id=e.execution_id),'message') AS execution_purpose,
    COALESCE(f.execution_phase,CASE WHEN e.kind='user' THEN latest.phase END,orig_a.phase) AS execution_phase,
    CASE WHEN f.entry_id IS NOT NULL THEN f.execution_started
         ELSE EXISTS(SELECT 1 FROM agena_attempts prior WHERE prior.execution_id=e.execution_id AND prior.started_at_ms IS NOT NULL)
    END AS ever_started,
    COALESCE(br.phase,r.phase) AS round_phase,
    cr.round_id AS compaction_round,
    e.content_id,e.tool_id,e.interaction_id,
    CASE WHEN e.kind IN ('user','compaction') THEN e.body_json ELSE NULL END AS body_json,
    CASE WHEN cs.kind='structured' AND (f.entry_id IS NULL OR bs.source_key IS NOT NULL) THEN 1 ELSE 0 END AS is_structured";

const CONTINUITY_JOINS: &str = "
    LEFT JOIN agena_rounds r ON r.id=e.round_id
    LEFT JOIN agena_compaction_rounds cr ON cr.round_id=e.round_id
    LEFT JOIN agena_attempts orig_a ON orig_a.execution_id=e.execution_id AND orig_a.generation=e.generation
    LEFT JOIN agena_executions x ON e.kind='user' AND x.id=e.execution_id
    LEFT JOIN agena_attempts latest ON latest.execution_id=x.id AND latest.generation=x.current_generation
    LEFT JOIN agena_branch_entries f ON f.session_id=?1 AND f.entry_id=e.id
    LEFT JOIN agena_branch_rounds br ON br.session_id=f.session_id AND br.round_id=e.round_id
    LEFT JOIN agena_content_streams cs ON cs.id=e.content_id
    LEFT JOIN agena_branch_sources bs ON bs.session_id=?1 AND bs.source_key=cs.source_key";

/// Streams lightweight history metadata for an external ACP agent round in O(1)
/// memory without touching `agena_tool_calls`, `agena_interactions`, or
/// `agena_content_blocks`. External ACP peers own their native context window,
/// so capturing only the current turn's user message (plus any unobserved
/// delivery entries) avoids the 8 MiB native LLM provider input limit on large
/// imported or long-running ACP conversations while preserving exact resume,
/// pending-retry, branch, and anchor invariants.
pub(super) async fn scan_round_continuity(
    connection: &mut SqliteConnection,
    session: SessionId,
    message: EntryId,
    previous: Option<&ExternalAgentResume>,
    pending: Option<&ExternalAgentPendingTurn>,
) -> Result<ScannedRoundContinuity> {
    let window_state = sqlx::query("SELECT checkpoint_entry_id,through_sequence,through_ordinal FROM agena_prompt_windows WHERE session_id=?")
        .bind(session.get())
        .fetch_optional(&mut *connection)
        .await
        .map_err(sql_error)?
        .ok_or_else(|| KernelError::NotFound("prompt window".into()))?;
    let checkpoint: Option<i64> = window_state
        .try_get("checkpoint_entry_id")
        .map_err(storage)?;
    let through_sequence: Option<i64> = window_state.try_get("through_sequence").map_err(storage)?;
    let through_ordinal: Option<i64> = window_state.try_get("through_ordinal").map_err(storage)?;

    let unobserved_deliveries: std::collections::HashSet<i64> = sqlx::query_scalar(
        "SELECT DISTINCT result_entry_id FROM agena_deliveries
         WHERE session_id=?1 AND state='consumed' AND observed_round_id IS NULL
           AND result_entry_id IS NOT NULL",
    )
    .bind(session.get())
    .fetch_all(&mut *connection)
    .await
    .map_err(sql_error)?
    .into_iter()
    .collect();

    struct ScanState<'a> {
        message: EntryId,
        previous: Option<&'a ExternalAgentResume>,
        pending: Option<&'a ExternalAgentPendingTurn>,
        unobserved_deliveries: std::collections::HashSet<i64>,
        round_inputs: Vec<EntryId>,
        saw_message: bool,
        after_msg_only_own_notices: bool,
        failed_users_before_msg: std::collections::HashSet<EntryId>,
        anchor_tail: Option<EntryId>,
        has_prior_turns: bool,
        last_non_notice_id: Option<EntryId>,
        hash_unstructured: Sha256,
        hash_structured: Sha256,
        hash_pending_current: Sha256,
        hash_pending_prior: Option<Sha256>,
        prev_seen: bool,
        prev_phase_ok: bool,
        prev_sig_ok: bool,
        prev_after_ok: bool,
        pend_seen: bool,
        pend_sig_ok: bool,
        pend_after_ok: bool,
    }

    impl<'a> ScanState<'a> {
        fn update_hash(
            hash: &mut Sha256,
            id: EntryId,
            generation: Option<AttemptGeneration>,
            round_id: Option<RoundId>,
            revision: Option<EntryRevision>,
            payload: &EntryPayload,
        ) -> Result<()> {
            let bytes =
                serde_json::to_vec(&(id, generation, round_id, revision, payload)).map_err(storage)?;
            hash.update(bytes.len().to_le_bytes());
            hash.update(bytes);
            Ok(())
        }

        fn process_row(&mut self, row: &sea_orm::sqlx::sqlite::SqliteRow, is_checkpoint: bool) -> Result<()> {
            let id_raw: i64 = row.try_get("id").map_err(storage)?;
            let kind: &str = row.try_get("kind").map_err(storage)?;
            let msg_id_raw: i64 = row.try_get("message_id").map_err(storage)?;
            let execution_phase_str: &str = row.try_get("execution_phase").map_err(storage)?;

            if !is_checkpoint {
                let execution_purpose: &str = row.try_get("execution_purpose").map_err(storage)?;
                if kind == "notice" && id_raw == msg_id_raw && execution_purpose == "compaction" {
                    return Ok(());
                }
                if msg_id_raw > self.message.get() {
                    return Ok(());
                }
                let ever_started: bool = row.try_get("ever_started").map_err(storage)?;
                if kind == "user" && execution_phase_str == "cancelled" && !ever_started {
                    return Ok(());
                }
                let round_phase: Option<&str> = row.try_get("round_phase").map_err(storage)?;
                let compaction_round: Option<i64> =
                    row.try_get("compaction_round").map_err(storage)?;
                if kind == "compaction"
                    || (kind != "notice"
                        && (compaction_round.is_some()
                            || matches!(round_phase, Some("failed" | "interrupted"))))
                {
                    return Ok(());
                }
            }

            let id = EntryId::try_from(id_raw).map_err(storage)?;
            let msg_id = EntryId::try_from(msg_id_raw).map_err(storage)?;
            let execution_phase: ExecutionPhase = decode(&encode(&execution_phase_str)?)?;

            if id == self.message || self.unobserved_deliveries.contains(&id_raw) {
                self.round_inputs.push(id);
            }

            // Track anchor tail and post-message notices identically to TurnInput::new.
            if id == self.message {
                self.saw_message = true;
            } else if self.saw_message {
                if !(kind == "notice" && msg_id == self.message) {
                    self.after_msg_only_own_notices = false;
                }
            } else if kind == "user"
                && matches!(
                    execution_phase,
                    ExecutionPhase::Failed | ExecutionPhase::Cancelled
                )
            {
                self.failed_users_before_msg.insert(id);
            } else if !(kind == "notice" && self.failed_users_before_msg.contains(&msg_id)) {
                self.anchor_tail = Some(id);
            }

            if kind == "notice" {
                return Ok(());
            }

            self.last_non_notice_id = Some(id);
            if kind == "user" && id != self.message {
                self.has_prior_turns = true;
            }

            let generation = AttemptGeneration::try_from(
                row.try_get::<i64, _>("generation").map_err(storage)?,
            )
            .map_err(storage)?;
            let round_id = row
                .try_get::<Option<i64>, _>("round_id")
                .map_err(storage)?
                .map(RoundId::try_from)
                .transpose()
                .map_err(storage)?;
            let revision = EntryRevision::try_from(
                row.try_get::<i64, _>("revision").map_err(storage)?,
            )
            .map_err(storage)?;
            let is_structured: bool = row.try_get::<i64, _>("is_structured").map_err(storage)? != 0;

            let payload = match kind {
                "user" => {
                    let body: &str = row
                        .try_get::<Option<&str>, _>("body_json")
                        .map_err(storage)?
                        .ok_or_else(|| storage("user entry has no document"))?;
                    let body = decode_user_body(body)?;
                    EntryPayload::User {
                        document: body.document,
                        author: body.author,
                        origin: body.origin,
                    }
                }
                "text" | "reasoning" => {
                    let content_id: agena_domain::ContentId = row
                        .try_get::<String, _>("content_id")
                        .map_err(storage)?
                        .parse()
                        .map_err(storage)?;
                    if kind == "text" {
                        EntryPayload::Text { content_id }
                    } else {
                        EntryPayload::Reasoning { content_id }
                    }
                }
                "tool" => EntryPayload::Tool {
                    tool_id: ToolId::try_from(row.try_get::<i64, _>("tool_id").map_err(storage)?)
                        .map_err(storage)?,
                },
                "interaction" => EntryPayload::Interaction {
                    interaction_id: InteractionId::try_from(
                        row.try_get::<i64, _>("interaction_id").map_err(storage)?,
                    )
                    .map_err(storage)?,
                },
                "compaction" => {
                    let body: &str = row
                        .try_get::<Option<&str>, _>("body_json")
                        .map_err(storage)?
                        .ok_or_else(|| storage("compaction entry has no body"))?;
                    decode(body)?
                }
                _ => return Err(storage("unknown entry kind")),
            };

            let rev_unstructured = match payload {
                EntryPayload::User { .. } => None,
                _ => Some(revision),
            };
            let rev_structured = match payload {
                EntryPayload::User { .. } => None,
                EntryPayload::Text { .. } | EntryPayload::Reasoning { .. } if is_structured => None,
                _ => Some(revision),
            };
            Self::update_hash(
                &mut self.hash_unstructured,
                id,
                Some(generation),
                round_id,
                rev_unstructured,
                &payload,
            )?;
            Self::update_hash(
                &mut self.hash_structured,
                id,
                Some(generation),
                round_id,
                rev_structured,
                &payload,
            )?;
            Self::update_hash(
                &mut self.hash_pending_current,
                id,
                (id != self.message).then_some(generation),
                round_id,
                rev_structured,
                &payload,
            )?;
            if let (Some(prior), Some(hash)) = (self.pending, self.hash_pending_prior.as_mut()) {
                Self::update_hash(
                    hash,
                    id,
                    (id != prior.message).then_some(generation),
                    round_id,
                    rev_structured,
                    &payload,
                )?;
            }

            if let Some(prior) = self.previous
                && prior.message != self.message
            {
                if id == prior.message {
                    self.prev_seen = true;
                    self.prev_phase_ok = execution_phase == ExecutionPhase::Succeeded;
                    let sig: [u8; 32] = if prior.incarnation.is_some() {
                        self.hash_structured.clone().finalize().into()
                    } else {
                        self.hash_unstructured.clone().finalize().into()
                    };
                    self.prev_sig_ok = sig == prior.input;
                } else if self.prev_seen && id != self.message {
                    let follows = round_id == Some(prior.round)
                        || (kind == "user"
                            && matches!(
                                execution_phase,
                                ExecutionPhase::Failed | ExecutionPhase::Cancelled
                            ));
                    if !follows {
                        self.prev_after_ok = false;
                    }
                }
            }

            if let Some(prior) = self.pending
                && prior.message != self.message
            {
                if id == prior.message {
                    self.pend_seen = true;
                    if let Some(hash) = self.hash_pending_prior.as_ref() {
                        let sig: [u8; 32] = hash.clone().finalize().into();
                        self.pend_sig_ok = sig == prior.input;
                    }
                } else if self.pend_seen && id != self.message {
                    let follows = round_id == Some(prior.round)
                        || (kind == "user"
                            && matches!(
                                execution_phase,
                                ExecutionPhase::Failed | ExecutionPhase::Cancelled
                            ));
                    if !follows {
                        self.pend_after_ok = false;
                    }
                }
            }

            Ok(())
        }
    }

    let mut state = ScanState {
        message,
        previous,
        pending,
        unobserved_deliveries,
        round_inputs: Vec::new(),
        saw_message: false,
        after_msg_only_own_notices: true,
        failed_users_before_msg: std::collections::HashSet::new(),
        anchor_tail: None,
        has_prior_turns: false,
        last_non_notice_id: None,
        hash_unstructured: Sha256::new(),
        hash_structured: Sha256::new(),
        hash_pending_current: Sha256::new(),
        hash_pending_prior: pending
            .filter(|prior| prior.message != message)
            .map(|_| Sha256::new()),
        prev_seen: false,
        prev_phase_ok: false,
        prev_sig_ok: false,
        prev_after_ok: true,
        pend_seen: false,
        pend_sig_ok: false,
        pend_after_ok: true,
    };

    if let Some(checkpoint_id) = checkpoint {
        let sql = format!("SELECT {CONTINUITY_SELECT_COLS} FROM agena_entries e {CONTINUITY_JOINS} WHERE e.id=?2");
        if let Some(row) = sqlx::query(&sql)
            .bind(session.get())
            .bind(checkpoint_id)
            .fetch_optional(&mut *connection)
            .await
            .map_err(sql_error)?
        {
            state.process_row(&row, true)?;
        }
    }

    let segment_sql = format!(
        "SELECT {CONTINUITY_SELECT_COLS}
         FROM agena_segment_entries m
         JOIN agena_entries e ON e.id=m.entry_id
         {CONTINUITY_JOINS}
         WHERE m.segment_id=?2 AND m.ordinal>=?3 AND m.ordinal<?4
         ORDER BY m.ordinal"
    );
    let mut after = through_sequence.unwrap_or(0);
    loop {
        let segments = sqlx::query(
            "SELECT sequence,segment_id,first_ordinal,end_ordinal FROM agena_session_segments WHERE session_id=? AND sequence>=? ORDER BY sequence LIMIT 64",
        )
        .bind(session.get())
        .bind(after)
        .fetch_all(&mut *connection)
        .await
        .map_err(sql_error)?;
        if segments.is_empty() {
            break;
        }
        for segment in segments {
            let sequence: i64 = segment.try_get("sequence").map_err(storage)?;
            let mut first: i64 = segment.try_get("first_ordinal").map_err(storage)?;
            if Some(sequence) == through_sequence {
                first = first.max(through_ordinal.unwrap_or(-1) + 1);
            }
            let end: i64 = segment.try_get("end_ordinal").map_err(storage)?;
            if first < end {
                let rows = sqlx::query(&segment_sql)
                    .bind(session.get())
                    .bind(segment.try_get::<i64, _>("segment_id").map_err(storage)?)
                    .bind(first)
                    .bind(end)
                    .fetch_all(&mut *connection)
                    .await
                    .map_err(sql_error)?;
                for row in &rows {
                    state.process_row(row, false)?;
                }
            }
            after = sequence
                .checked_add(1)
                .ok_or_else(|| storage("prompt sequence overflow"))?;
        }
    }

    let resume_signature_unstructured: [u8; 32] = state.hash_unstructured.finalize().into();
    let resume_signature_structured: [u8; 32] = state.hash_structured.finalize().into();
    let pending_signature: [u8; 32] = state.hash_pending_current.finalize().into();
    let is_last = state.last_non_notice_id == Some(message);
    let can_resume_previous = is_last
        && state.previous.is_some_and(|prior| {
            prior.message != message
                && state.prev_seen
                && state.prev_phase_ok
                && state.prev_sig_ok
                && state.prev_after_ok
        });
    let can_resume_pending = is_last
        && state.pending.is_some_and(|prior| {
            if prior.message == message {
                pending_signature == prior.input
            } else {
                state.pend_seen && state.pend_sig_ok && state.pend_after_ok
            }
        });
    let resume_anchor = (state.saw_message && state.after_msg_only_own_notices).then_some(
        ExternalAgentHistoryAnchor {
            tail: state.anchor_tail,
        },
    );

    Ok(ScannedRoundContinuity {
        round_inputs: state.round_inputs,
        continuity: ExternalAgentRoundContinuity {
            has_prior_turns: state.has_prior_turns,
            resume_anchor,
            can_resume_previous,
            can_resume_pending,
            resume_signature_unstructured,
            resume_signature_structured,
            pending_signature,
        },
    })
}
