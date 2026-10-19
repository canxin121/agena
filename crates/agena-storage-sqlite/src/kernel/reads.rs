use sea_orm::sqlx::{self, Row, SqliteConnection, SqlitePool, sqlite::SqliteRow};

use super::*;

// Covering metadata index avoids reading cold JSON overflow pages merely to
// select a bounded byte window. body_bytes is a generated, stored derivative.
pub(super) const HISTORY_PREFLIGHT_SQL: &str = "SELECT m.entry_id,m.ordinal,e.body_bytes FROM agena_segment_entries m JOIN agena_entries e INDEXED BY idx_agena_entry_bytes ON e.id=m.entry_id WHERE m.segment_id=? AND m.ordinal>=? AND m.ordinal<? ORDER BY m.ordinal DESC LIMIT ?";
pub(super) const PENDING_INTERACTIONS_PREFLIGHT_SQL: &str = "SELECT id,request_bytes FROM agena_interactions INDEXED BY idx_agena_interaction_pending_session WHERE session_id=? AND decision_json IS NULL AND id>? ORDER BY id LIMIT ?";
pub(super) static SUMMARY_SQL: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    format!("SELECT {}", super::lifecycle::summary_fields())
        + "s.id,s.workspace_id,s.title,s.favorite,s.pinned,s.metadata_version,s.membership_generation,s.created_at_ms,s.updated_at_ms,t.entry_count,t.user_message_count,t.queued_executions,t.running_executions,t.last_message_at_ms FROM agena_sessions s JOIN agena_session_statistics t ON t.session_id=s.id WHERE s.id=? AND s.deleted_at_ms IS NULL"
});

pub(super) fn summaries_sql(before: bool) -> String {
    // CROSS JOIN fixes the indexed workspace window as the driving table.
    // Each statistics lookup is a single row; LIMIT never follows a scan of
    // every session's entries/attempts or a full-workspace sort.
    let boundary = if before {
        " AND (s.updated_at_ms,s.id)<(?,?)"
    } else {
        ""
    };
    let extra = super::lifecycle::summary_fields();
    format!(
        "SELECT {extra}s.id,s.workspace_id,s.title,s.favorite,s.pinned,s.metadata_version,s.membership_generation,s.created_at_ms,s.updated_at_ms,t.entry_count,t.user_message_count,t.queued_executions,t.running_executions,t.last_message_at_ms FROM agena_sessions s INDEXED BY idx_agena_sessions_workspace CROSS JOIN agena_session_statistics t ON t.session_id=s.id WHERE s.workspace_id=? AND s.deleted_at_ms IS NULL{boundary} ORDER BY s.updated_at_ms DESC,s.id DESC LIMIT ?"
    )
}

pub(super) async fn summaries(
    pool: &SqlitePool,
    workspace_id: i64,
    before: Option<SessionSummaryCursor>,
    limit: usize,
    diagnostics: &KernelDiagnostics,
) -> Result<SessionSummaryPage> {
    if workspace_id <= 0 {
        return Err(invalid("invalid workspace"));
    }
    if before.is_some_and(|cursor| cursor.workspace_id != workspace_id) {
        return Err(KernelError::Ownership);
    }
    let limit = limit.clamp(1, 200);
    let sql = summaries_sql(before.is_some());
    let mut query = sqlx::query(&sql).bind(workspace_id);
    if let Some(before) = before {
        query = query.bind(before.updated_at_ms).bind(before.id.get());
    }
    metrics::add(&diagnostics.0.summary_queries, 1);
    let mut rows = query
        .bind((limit + 1) as i64)
        .fetch_all(pool)
        .await
        .map_err(sql_error)?;
    metrics::add(&diagnostics.0.summary_rows, rows.len());
    let has_more = rows.len() > limit;
    rows.truncate(limit);
    let sessions = rows.iter().map(summary_row).collect::<Result<Vec<_>>>()?;
    let next = has_more
        .then(|| {
            sessions.last().map(|last| SessionSummaryCursor {
                workspace_id,
                updated_at_ms: last.updated_at_ms,
                id: last.id,
            })
        })
        .flatten();
    Ok(SessionSummaryPage {
        total: None,
        workspace_id,
        sessions,
        next,
        has_more,
    })
}

pub(super) async fn summary(
    connection: &mut SqliteConnection,
    session: SessionId,
) -> Result<SessionSummaryView> {
    let row = sqlx::query(&SUMMARY_SQL)
        .bind(session.get())
        .fetch_optional(connection)
        .await
        .map_err(sql_error)?
        .ok_or_else(|| KernelError::NotFound("session".into()))?;
    summary_row(&row)
}

pub(super) fn summary_row(row: &SqliteRow) -> Result<SessionSummaryView> {
    let count = |name| -> Result<u64> {
        u64::try_from(row.try_get::<i64, _>(name).map_err(storage)?).map_err(storage)
    };
    Ok(SessionSummaryView {
        parent_session_id: row
            .try_get::<Option<i64>, _>("parent_session_id")
            .map_err(storage)?
            .map(SessionId::try_from)
            .transpose()
            .map_err(storage)?,
        root_session_id: SessionId::try_from(
            row.try_get::<i64, _>("root_session_id").map_err(storage)?,
        )
        .map_err(storage)?,
        depth: row.try_get::<i64, _>("depth").map_err(storage)? as u32,
        relation_kind: agena_domain::SessionRelationKind::parse(
            row.try_get("relation_kind").map_err(storage)?,
        )
        .ok_or_else(|| storage("invalid session relation"))?,
        task_id: row.try_get("task_id").map_err(storage)?,
        subtask_status: row
            .try_get::<Option<&str>, _>("subtask_status")
            .map_err(storage)?
            .map(|status| {
                agena_domain::SubtaskStatus::parse(status)
                    .ok_or_else(|| storage("invalid task status"))
            })
            .transpose()?,
        child_session_count: count("child_session_count")?,
        state: agena_domain::SessionStateKind::parse(
            row.try_get("processing_state").map_err(storage)?,
        )
        .ok_or_else(|| storage("invalid session state"))?,
        id: SessionId::try_from(row.try_get::<i64, _>("id").map_err(storage)?).map_err(storage)?,
        workspace_id: row.try_get("workspace_id").map_err(storage)?,
        title: row.try_get("title").map_err(storage)?,
        favorite: row.try_get("favorite").map_err(storage)?,
        pinned: row.try_get("pinned").map_err(storage)?,
        metadata_version: MetadataVersion::try_from(
            row.try_get::<i64, _>("metadata_version").map_err(storage)?,
        )
        .map_err(storage)?,
        membership_generation: MembershipGeneration::try_from(
            row.try_get::<i64, _>("membership_generation")
                .map_err(storage)?,
        )
        .map_err(storage)?,
        entry_count: count("entry_count")?,
        user_message_count: count("user_message_count")?,
        queued_executions: count("queued_executions")?,
        running_executions: count("running_executions")?,
        last_message_at_ms: row.try_get("last_message_at_ms").map_err(storage)?,
        created_at_ms: row.try_get("created_at_ms").map_err(storage)?,
        updated_at_ms: row.try_get("updated_at_ms").map_err(storage)?,
    })
}

pub(super) async fn metadata(
    connection: &mut SqliteConnection,
    session: SessionId,
) -> Result<SessionMetadata> {
    let row = sqlx::query("SELECT id,workspace_id,title,favorite,pinned,selection_json,metadata_version,membership_generation FROM agena_sessions WHERE id=? AND deleted_at_ms IS NULL")
        .bind(session.get()).fetch_optional(connection).await.map_err(sql_error)?
        .ok_or_else(|| KernelError::NotFound("session".into()))?;
    Ok(SessionMetadata {
        id: session,
        workspace_id: row.try_get("workspace_id").map_err(storage)?,
        title: row.try_get("title").map_err(storage)?,
        favorite: row.try_get("favorite").map_err(storage)?,
        pinned: row.try_get("pinned").map_err(storage)?,
        selection: row
            .try_get::<Option<String>, _>("selection_json")
            .map_err(storage)?
            .as_deref()
            .map(decode)
            .transpose()?,
        metadata_version: MetadataVersion::try_from(
            row.try_get::<i64, _>("metadata_version").map_err(storage)?,
        )
        .map_err(storage)?,
        membership_generation: MembershipGeneration::try_from(
            row.try_get::<i64, _>("membership_generation")
                .map_err(storage)?,
        )
        .map_err(storage)?,
    })
}

pub(super) async fn execution_visible(
    connection: &mut SqliteConnection,
    session: SessionId,
    id: ExecutionId,
) -> Result<ExecutionView> {
    let origin:Option<i64>=sqlx::query_scalar("SELECT x.session_id FROM agena_executions x WHERE x.id=? AND EXISTS(SELECT 1 FROM agena_segment_entries m JOIN agena_session_segments s ON s.segment_id=m.segment_id JOIN agena_sessions owner ON owner.id=s.session_id WHERE m.entry_id=x.message_id AND s.session_id=? AND owner.deleted_at_ms IS NULL AND m.ordinal>=s.first_ordinal AND m.ordinal<s.end_ordinal)").bind(id.to_string()).bind(session.get()).fetch_optional(&mut *connection).await.map_err(sql_error)?;
    let origin = SessionId::try_from(origin.ok_or(KernelError::Ownership)?).map_err(storage)?;
    // A tombstoned origin remains readable through a surviving branch.
    let row=sqlx::query("SELECT x.message_id,x.current_generation,a.phase FROM agena_executions x JOIN agena_attempts a ON a.execution_id=x.id AND a.generation=x.current_generation WHERE x.id=?").bind(id.to_string()).fetch_one(connection).await.map_err(sql_error)?;
    Ok(ExecutionView {
        id,
        session_id: origin,
        message_id: EntryId::try_from(row.try_get::<i64, _>("message_id").map_err(storage)?)
            .map_err(storage)?,
        generation: AttemptGeneration::try_from(
            row.try_get::<i64, _>("current_generation")
                .map_err(storage)?,
        )
        .map_err(storage)?,
        phase: decode(&encode(row.try_get::<&str, _>("phase").map_err(storage)?)?)?,
    })
}

pub(super) async fn execution(
    connection: &mut SqliteConnection,
    session: SessionId,
    execution: ExecutionId,
) -> Result<(ExecutionView, Option<KernelEpoch>)> {
    let row = sqlx::query("SELECT e.message_id,e.current_generation,a.phase,a.owner_epoch FROM agena_executions e JOIN agena_attempts a ON a.execution_id=e.id AND a.generation=e.current_generation JOIN agena_sessions s ON s.id=e.session_id WHERE e.id=? AND e.session_id=? AND s.deleted_at_ms IS NULL")
        .bind(execution.to_string()).bind(session.get()).fetch_optional(connection).await.map_err(sql_error)?
        .ok_or(KernelError::Ownership)?;
    let phase: String = row.try_get("phase").map_err(storage)?;
    let phase: ExecutionPhase = decode(&serde_json::to_string(&phase).map_err(storage)?)?;
    let epoch = row
        .try_get::<Option<String>, _>("owner_epoch")
        .map_err(storage)?
        .map(|epoch| epoch.parse().map_err(storage))
        .transpose()?;
    Ok((
        ExecutionView {
            id: execution,
            session_id: session,
            message_id: EntryId::try_from(row.try_get::<i64, _>("message_id").map_err(storage)?)
                .map_err(storage)?,
            generation: AttemptGeneration::try_from(
                row.try_get::<i64, _>("current_generation")
                    .map_err(storage)?,
            )
            .map_err(storage)?,
            phase,
        },
        epoch,
    ))
}

pub(super) fn entry_row(row: &SqliteRow) -> Result<EntryView> {
    let id = EntryId::try_from(row.try_get::<i64, _>("id").map_err(storage)?).map_err(storage)?;
    let body: Option<&str> = row.try_get("body_json").map_err(storage)?;
    let kind: String = row.try_get("kind").map_err(storage)?;
    let payload = match kind.as_str() {
        "user" => EntryPayload::User {
            document: decode(body.ok_or_else(|| storage("user entry has no document"))?)?,
        },
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
        "notice" => EntryPayload::Notice {
            text: decode(body.ok_or_else(|| storage("notice entry has no body"))?)?,
        },
        "compaction" => decode(body.ok_or_else(|| storage("compaction entry has no body"))?)?,
        _ => return Err(storage("unknown entry kind")),
    };
    Ok(EntryView {
        id,
        origin_session_id: SessionId::try_from(
            row.try_get::<i64, _>("origin_session_id")
                .map_err(storage)?,
        )
        .map_err(storage)?,
        message_id: row
            .try_get::<Option<i64>, _>("message_id")
            .map_err(storage)?
            .map(EntryId::try_from)
            .transpose()
            .map_err(storage)?
            .unwrap_or(id),
        execution_id: row
            .try_get::<String, _>("execution_id")
            .map_err(storage)?
            .parse()
            .map_err(storage)?,
        generation: AttemptGeneration::try_from(
            row.try_get::<i64, _>("generation").map_err(storage)?,
        )
        .map_err(storage)?,
        revision: EntryRevision::try_from(row.try_get::<i64, _>("revision").map_err(storage)?)
            .map_err(storage)?,
        created_at_ms: row.try_get("created_at_ms").map_err(storage)?,
        execution_phase: decode(&encode(
            row.try_get::<&str, _>("execution_phase").map_err(storage)?,
        )?)?,
        response_entry_id: row
            .try_get::<Option<i64>, _>("response_entry_id")
            .unwrap_or(None)
            .map(EntryId::try_from)
            .transpose()
            .map_err(storage)?,
        content_state: row
            .try_get::<Option<&str>, _>("content_state")
            .ok()
            .flatten()
            .map(|state| decode(&encode(state)?))
            .transpose()?,
        payload,
    })
}

pub(super) async fn history(
    pool: &SqlitePool,
    session: SessionId,
    before: Option<HistoryCursor>,
    limit: usize,
    diagnostics: &KernelDiagnostics,
) -> Result<HistoryPage> {
    let limit = limit.clamp(1, 200);
    let mut transaction = pool.begin().await.map_err(storage)?;
    metrics::add(&diagnostics.0.history_queries, 1);
    let metadata = metadata(&mut transaction, session).await?;
    if before.as_ref().is_some_and(|cursor| {
        cursor.session != session || cursor.membership != metadata.membership_generation
    }) {
        return Err(KernelError::Conflict(
            "history membership changed; restart pagination".into(),
        ));
    }
    let boundary = before
        .as_ref()
        .map_or(i64::MAX, |cursor| cursor.segment_sequence);
    // Select a bounded list of segment references first. This avoids sorting
    // or joining the complete transcript before applying a LIMIT.
    metrics::add(&diagnostics.0.history_queries, 1);
    let segments = sqlx::query("SELECT sequence,segment_id,first_ordinal,end_ordinal FROM agena_session_segments WHERE session_id=? AND sequence<=? ORDER BY sequence DESC LIMIT ?")
        .bind(session.get()).bind(boundary).bind((limit+2) as i64)
        .fetch_all(&mut *transaction).await.map_err(sql_error)?;
    let mut selected = Vec::with_capacity(limit);
    let mut next = None;
    let mut has_more = false;
    let mut bytes = 0usize;
    'segments: for segment in segments {
        let sequence: i64 = segment.try_get("sequence").map_err(storage)?;
        let mut end: i64 = segment.try_get("end_ordinal").map_err(storage)?;
        if let Some(cursor) = &before
            && cursor.segment_sequence == sequence
        {
            end = end.min(i64::from(cursor.ordinal));
        }
        // Preflight identities and lengths, not bodies. Fetching 200 large user
        // documents and enforcing the byte budget afterward materializes up to
        // 50 MiB for a nominal 1 MiB page.
        metrics::add(&diagnostics.0.history_queries, 1);
        let rows = sqlx::query(HISTORY_PREFLIGHT_SQL)
            .bind(segment.try_get::<i64, _>("segment_id").map_err(storage)?)
            .bind(
                segment
                    .try_get::<i64, _>("first_ordinal")
                    .map_err(storage)?,
            )
            .bind(end)
            .bind((limit + 1 - selected.len()) as i64)
            .fetch_all(&mut *transaction)
            .await
            .map_err(sql_error)?;
        metrics::add(&diagnostics.0.history_candidate_rows, rows.len());
        for row in rows {
            let charge = row.try_get::<i64, _>("body_bytes").map_err(storage)? as usize + 512;
            if selected.len() == limit || bytes + charge > 1024 * 1024 {
                if selected.is_empty() {
                    return Err(storage("entry exceeds the ordinary history read budget"));
                }
                has_more = true;
                break 'segments;
            }
            bytes += charge;
            next = Some(HistoryCursor {
                session,
                membership: metadata.membership_generation,
                segment_sequence: sequence,
                ordinal: row.try_get::<i64, _>("ordinal").map_err(storage)? as u32,
            });
            selected.push(row.try_get::<i64, _>("entry_id").map_err(storage)?);
        }
    }
    let mut entries = Vec::with_capacity(selected.len());
    if !selected.is_empty() {
        metrics::add(&diagnostics.0.history_queries, 1);
        let rows =
            sqlx::query("SELECT e.*,(SELECT a.response_entry_id FROM agena_attempts a WHERE a.execution_id=e.execution_id AND a.generation=e.generation) AS response_entry_id,(SELECT a.phase FROM agena_attempts a WHERE a.execution_id=e.execution_id AND a.generation=CASE WHEN e.kind='user' THEN (SELECT x.current_generation FROM agena_executions x WHERE x.id=e.execution_id) ELSE e.generation END) AS execution_phase,(SELECT c.capture_state FROM agena_content_streams c WHERE c.id=e.content_id) AS content_state FROM agena_entries e WHERE e.id IN (SELECT value FROM json_each(?))")
                .bind(encode(&selected)?)
                .fetch_all(&mut *transaction)
                .await
                .map_err(sql_error)?;
        metrics::add(&diagnostics.0.history_entry_rows, rows.len());
        let mut by_id = std::collections::HashMap::with_capacity(rows.len());
        for row in rows {
            metrics::add(
                &diagnostics.0.history_entry_body_bytes,
                row.try_get::<Option<&str>, _>("body_json")
                    .map_err(storage)?
                    .map_or(0, str::len),
            );
            by_id.insert(row.try_get::<i64, _>("id").map_err(storage)?, row);
        }
        for id in selected {
            let row = by_id
                .remove(&id)
                .ok_or_else(|| storage("selected history entry disappeared inside its snapshot"))?;
            entries.push(entry_row(&row)?);
        }
    }
    entries.reverse();
    Ok(HistoryPage {
        metadata,
        entries,
        next: if has_more { next } else { None },
        has_more,
    })
}

pub(super) async fn entry(
    pool: &SqlitePool,
    session: SessionId,
    id: EntryId,
    diagnostics: &KernelDiagnostics,
) -> Result<EntryView> {
    metrics::add(&diagnostics.0.detail_queries, 1);
    let row=sqlx::query("SELECT e.*,(SELECT x.response_entry_id FROM agena_attempts x WHERE x.execution_id=e.execution_id AND x.generation=e.generation) AS response_entry_id,(SELECT phase FROM agena_attempts phase_attempt WHERE phase_attempt.execution_id=e.execution_id AND phase_attempt.generation=CASE WHEN e.kind='user' THEN (SELECT current_generation FROM agena_executions phase_execution WHERE phase_execution.id=e.execution_id) ELSE e.generation END) AS execution_phase,(SELECT c.capture_state FROM agena_content_streams c WHERE c.id=e.content_id) AS content_state FROM agena_entries e WHERE e.id=? AND EXISTS(SELECT 1 FROM agena_segment_entries m JOIN agena_session_segments s ON s.segment_id=m.segment_id JOIN agena_sessions owner ON owner.id=s.session_id WHERE m.entry_id=e.id AND s.session_id=? AND owner.deleted_at_ms IS NULL AND m.ordinal>=s.first_ordinal AND m.ordinal<s.end_ordinal)")
        .bind(id.get()).bind(session.get()).fetch_optional(pool).await.map_err(sql_error)?.ok_or_else(||KernelError::NotFound("entry".into()))?;
    entry_row(&row)
}

pub(super) const ACTIVE_EXECUTIONS_SQL: &str = "SELECT e.id,e.message_id,e.current_generation,a.phase FROM agena_attempts a INDEXED BY idx_agena_attempt_active JOIN agena_executions e ON e.id=a.execution_id AND e.current_generation=a.generation WHERE a.session_id=? AND a.phase IN ('queued','running') AND a.execution_id>? ORDER BY a.execution_id LIMIT ?";

pub(super) async fn active_executions(
    pool: &SqlitePool,
    session: SessionId,
    after: Option<ActiveExecutionCursor>,
    limit: usize,
    diagnostics: &KernelDiagnostics,
) -> Result<ActiveExecutionPage> {
    if after.is_some_and(|cursor| cursor.session != session) {
        return Err(KernelError::Ownership);
    }
    let mut tx = pool.begin().await.map_err(storage)?;
    metrics::add(&diagnostics.0.activity_queries, 1);
    metadata(&mut tx, session).await?;
    active_executions_on(&mut tx, session, after, limit, diagnostics).await
}

pub(super) async fn active_executions_on(
    connection: &mut SqliteConnection,
    session: SessionId,
    after: Option<ActiveExecutionCursor>,
    limit: usize,
    diagnostics: &KernelDiagnostics,
) -> Result<ActiveExecutionPage> {
    if after.is_some_and(|cursor| cursor.session != session) {
        return Err(KernelError::Ownership);
    }
    let limit = limit.clamp(1, 200);
    metrics::add(&diagnostics.0.activity_queries, 1);
    let rows = sqlx::query(ACTIVE_EXECUTIONS_SQL)
        .bind(session.get())
        .bind(after.map_or_else(String::new, |cursor| cursor.after.to_string()))
        .bind((limit + 1) as i64)
        .fetch_all(&mut *connection)
        .await
        .map_err(sql_error)?;
    metrics::add(&diagnostics.0.activity_rows, rows.len());
    let has_more = rows.len() > limit;
    let executions = rows
        .into_iter()
        .take(limit)
        .map(|row| {
            Ok(ExecutionView {
                id: row
                    .try_get::<String, _>("id")
                    .map_err(storage)?
                    .parse()
                    .map_err(storage)?,
                session_id: session,
                message_id: EntryId::try_from(
                    row.try_get::<i64, _>("message_id").map_err(storage)?,
                )
                .map_err(storage)?,
                generation: AttemptGeneration::try_from(
                    row.try_get::<i64, _>("current_generation")
                        .map_err(storage)?,
                )
                .map_err(storage)?,
                phase: decode(&encode(
                    &row.try_get::<String, _>("phase").map_err(storage)?,
                )?)?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let next = has_more.then(|| ActiveExecutionCursor {
        session,
        after: executions.last().expect("nonempty active page").id,
    });
    Ok(ActiveExecutionPage {
        executions,
        next,
        has_more,
    })
}

pub(super) async fn pending_interactions(
    pool: &SqlitePool,
    session: SessionId,
    after: Option<PendingInteractionCursor>,
    limit: usize,
    diagnostics: &KernelDiagnostics,
) -> Result<PendingInteractionPage> {
    if after.is_some_and(|cursor| cursor.session != session) {
        return Err(KernelError::Ownership);
    }
    let mut tx = pool.begin().await.map_err(storage)?;
    metrics::add(&diagnostics.0.activity_queries, 1);
    metadata(&mut tx, session).await?;
    pending_interactions_on(&mut tx, session, after, limit, diagnostics).await
}

pub(super) async fn pending_interactions_on(
    connection: &mut SqliteConnection,
    session: SessionId,
    after: Option<PendingInteractionCursor>,
    limit: usize,
    diagnostics: &KernelDiagnostics,
) -> Result<PendingInteractionPage> {
    if after.is_some_and(|cursor| cursor.session != session) {
        return Err(KernelError::Ownership);
    }
    let limit = limit.clamp(1, 200);
    metrics::add(&diagnostics.0.activity_queries, 1);
    let candidates = sqlx::query(PENDING_INTERACTIONS_PREFLIGHT_SQL)
        .bind(session.get())
        .bind(after.map_or(0, |cursor| cursor.after.get()))
        .bind((limit + 1) as i64)
        .fetch_all(&mut *connection)
        .await
        .map_err(sql_error)?;
    metrics::add(&diagnostics.0.interaction_candidate_rows, candidates.len());
    let mut ids = Vec::new();
    let mut bytes = 0;
    let mut has_more = false;
    for row in candidates {
        let charge = row.try_get::<i64, _>("request_bytes").map_err(storage)? as usize + 512;
        if ids.len() == limit || bytes + charge > 1024 * 1024 {
            if ids.is_empty() {
                return Err(storage(
                    "interaction exceeds the ordinary activity read budget",
                ));
            }
            has_more = true;
            break;
        }
        bytes += charge;
        ids.push(row.try_get::<i64, _>("id").map_err(storage)?);
    }
    let mut interactions = Vec::with_capacity(ids.len());
    if !ids.is_empty() {
        metrics::add(&diagnostics.0.activity_queries, 1);
        let rows=sqlx::query("SELECT i.id,i.tool_id,i.request_json,i.decision_json,i.remember_json,i.created_at_ms AS interaction_created_at_ms,i.decided_at_ms AS interaction_decided_at_ms,e.revision FROM agena_interactions i JOIN agena_entries e ON e.interaction_id=i.id WHERE i.session_id=? AND i.decision_json IS NULL AND i.id IN (SELECT value FROM json_each(?)) ORDER BY i.id")
            .bind(session.get()).bind(encode(&ids)?).fetch_all(&mut *connection).await.map_err(sql_error)?;
        if rows.len() != ids.len() {
            return Err(storage("pending interaction identity/entry mismatch"));
        }
        metrics::add(&diagnostics.0.activity_rows, rows.len());
        for row in rows {
            metrics::add(
                &diagnostics.0.interaction_body_bytes,
                row.try_get::<&str, _>("request_json")
                    .map_err(storage)?
                    .len(),
            );
            let id = InteractionId::try_from(row.try_get::<i64, _>("id").map_err(storage)?)
                .map_err(storage)?;
            interactions.push(interaction_row(id, &row)?);
        }
    }
    let next = if has_more {
        Some(PendingInteractionCursor {
            session,
            after: interactions.last().expect("nonempty pending page").id,
        })
    } else {
        None
    };
    Ok(PendingInteractionPage {
        interactions,
        next,
        has_more,
    })
}

/// Detail queries authorize through membership, so a future inherited entry
/// can be read by a branch without becoming executable work in that branch.
pub(super) async fn tool(
    pool: &SqlitePool,
    session: SessionId,
    tool: ToolId,
    diagnostics: &KernelDiagnostics,
) -> Result<ToolView> {
    let mut transaction = pool.begin().await.map_err(storage)?;
    metrics::add(&diagnostics.0.detail_queries, 1);
    let row=sqlx::query("SELECT t.round_id,t.execution_kind,t.phase,t.call_id,t.name,t.input_json,t.result_json,e.id AS entry_id,e.revision FROM agena_tool_calls t JOIN agena_entries e ON e.tool_id=t.id AND e.kind='tool' WHERE t.id=? AND EXISTS(SELECT 1 FROM agena_segment_entries m JOIN agena_session_segments s ON s.segment_id=m.segment_id JOIN agena_sessions owner ON owner.id=s.session_id WHERE s.session_id=? AND owner.deleted_at_ms IS NULL AND m.entry_id=e.id AND m.ordinal>=s.first_ordinal AND m.ordinal<s.end_ordinal)")
        .bind(tool.get()).bind(session.get()).fetch_optional(&mut *transaction).await.map_err(sql_error)?.ok_or_else(||KernelError::NotFound("tool".into()))?;
    let entry: i64 = row.try_get("entry_id").map_err(storage)?;
    metrics::add(&diagnostics.0.detail_queries, 1);
    let sources = sqlx::query(
        "SELECT id,epoch,kind,activity_id,ordinal FROM agena_content_streams WHERE entry_id=? ORDER BY ordinal,id LIMIT 129",
    )
    .bind(entry)
    .fetch_all(&mut *transaction)
    .await
    .map_err(sql_error)?;
    if sources.len() > 128 {
        return Err(storage("tool source identity budget is corrupt"));
    }
    let outputs = sources
        .into_iter()
        .map(|source| {
            Ok(ContentDescriptor {
                id: source
                    .try_get::<String, _>("id")
                    .map_err(storage)?
                    .parse()
                    .map_err(storage)?,
                entry: EntryId::try_from(entry).map_err(storage)?,
                epoch: source
                    .try_get::<String, _>("epoch")
                    .map_err(storage)?
                    .parse()
                    .map_err(storage)?,
                kind: decode(&encode(
                    &source.try_get::<String, _>("kind").map_err(storage)?,
                )?)?,
                ordinal: source.try_get::<i64, _>("ordinal").map_err(storage)? as u32,
                activity_id: source
                    .try_get::<Option<String>, _>("activity_id")
                    .map_err(storage)?
                    .map(|id| id.parse().map_err(storage))
                    .transpose()?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    metrics::add(&diagnostics.0.detail_queries, 1);
    let blobs = super::blobs::descriptors(&mut transaction, entry).await?;
    let activities: Vec<String> = sqlx::query_scalar(
        "SELECT id FROM agena_background_activities WHERE tool_id=? ORDER BY id LIMIT 17",
    )
    .bind(tool.get())
    .fetch_all(&mut *transaction)
    .await
    .map_err(sql_error)?;
    if activities.len() > super::activity::MAX_ACTIVITIES_PER_TOOL as usize {
        return Err(storage("tool activity identity budget is corrupt"));
    }
    let interactions: Vec<i64> = sqlx::query_scalar(
        "SELECT id FROM agena_interactions WHERE tool_id=? ORDER BY id LIMIT 129",
    )
    .bind(tool.get())
    .fetch_all(&mut *transaction)
    .await
    .map_err(sql_error)?;
    if interactions.len() > 128 {
        return Err(storage("tool interaction identity budget is corrupt"));
    }
    Ok(ToolView {
        entry_id: EntryId::try_from(entry).map_err(storage)?,
        execution_kind: decode(&encode(
            &row.try_get::<&str, _>("execution_kind").map_err(storage)?,
        )?)?,
        id: tool,
        round_id: RoundId::try_from(row.try_get::<i64, _>("round_id").map_err(storage)?)
            .map_err(storage)?,
        phase: decode(&encode(
            &row.try_get::<String, _>("phase").map_err(storage)?,
        )?)?,
        call: ToolStart {
            call_id: row.try_get("call_id").map_err(storage)?,
            name: row.try_get("name").map_err(storage)?,
            input: decode(&row.try_get::<String, _>("input_json").map_err(storage)?)?,
        },
        result: row
            .try_get::<Option<String>, _>("result_json")
            .map_err(storage)?
            .as_deref()
            .map(decode)
            .transpose()?,
        revision: EntryRevision::try_from(row.try_get::<i64, _>("revision").map_err(storage)?)
            .map_err(storage)?,
        outputs,
        interactions: interactions
            .into_iter()
            .map(InteractionId::try_from)
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(storage)?,
        activities: activities
            .into_iter()
            .map(|id| id.parse().map_err(storage))
            .collect::<Result<_>>()?,
        blobs,
    })
}

pub(super) async fn interaction(
    pool: &SqlitePool,
    session: SessionId,
    interaction: InteractionId,
    diagnostics: &KernelDiagnostics,
) -> Result<InteractionView> {
    metrics::add(&diagnostics.0.detail_queries, 1);
    let row=sqlx::query("SELECT i.tool_id,i.request_json,i.decision_json,i.remember_json,i.created_at_ms AS interaction_created_at_ms,i.decided_at_ms AS interaction_decided_at_ms,e.revision FROM agena_interactions i JOIN agena_entries e ON e.interaction_id=i.id WHERE i.id=? AND EXISTS(SELECT 1 FROM agena_segment_entries m JOIN agena_session_segments s ON s.segment_id=m.segment_id JOIN agena_sessions owner ON owner.id=s.session_id WHERE s.session_id=? AND owner.deleted_at_ms IS NULL AND m.entry_id=e.id AND m.ordinal>=s.first_ordinal AND m.ordinal<s.end_ordinal)")
        .bind(interaction.get()).bind(session.get()).fetch_optional(pool).await.map_err(sql_error)?.ok_or_else(||KernelError::NotFound("interaction".into()))?;
    interaction_row(interaction, &row)
}

pub(super) fn interaction_row(
    interaction: InteractionId,
    row: &SqliteRow,
) -> Result<InteractionView> {
    Ok(InteractionView {
        created_at_ms: row.try_get("interaction_created_at_ms").map_err(storage)?,
        decided_at_ms: row.try_get("interaction_decided_at_ms").map_err(storage)?,
        id: interaction,
        tool_id: ToolId::try_from(row.try_get::<i64, _>("tool_id").map_err(storage)?)
            .map_err(storage)?,
        request: decode(row.try_get::<&str, _>("request_json").map_err(storage)?)?,
        decision: row
            .try_get::<Option<&str>, _>("decision_json")
            .map_err(storage)?
            .map(decode)
            .transpose()?,
        remembered: row
            .try_get::<Option<&str>, _>("remember_json")
            .map_err(storage)?
            .map(decode)
            .transpose()?,
        revision: EntryRevision::try_from(row.try_get::<i64, _>("revision").map_err(storage)?)
            .map_err(storage)?,
    })
}

pub(super) async fn failed_round(
    pool: &SqlitePool,
    session: SessionId,
    execution: ExecutionId,
    generation: AttemptGeneration,
    diagnostics: &KernelDiagnostics,
) -> Result<Option<ProviderRoundView>> {
    metrics::add(&diagnostics.0.detail_queries, 1);
    let round_id: Option<i64> = sqlx::query_scalar(
        "SELECT r.id FROM agena_rounds r JOIN agena_attempts a ON a.execution_id=r.execution_id AND a.generation=r.generation WHERE r.execution_id=? AND r.generation=? AND a.session_id=? AND r.phase='failed' AND r.provider_state_json IS NOT NULL ORDER BY r.id DESC LIMIT 1",
    )
    .bind(execution.to_string())
    .bind(generation.get())
    .bind(session.get())
    .fetch_optional(pool)
    .await
    .map_err(sql_error)?;
    let Some(round_id) = round_id else {
        return Ok(None);
    };
    let round = RoundId::try_from(round_id).map_err(storage)?;
    super::reads::round(pool, session, round, diagnostics)
        .await
        .map(Some)
}

pub(super) async fn round(
    pool: &SqlitePool,
    session: SessionId,
    round: RoundId,
    diagnostics: &KernelDiagnostics,
) -> Result<ProviderRoundView> {
    let mut transaction = pool.begin().await.map_err(storage)?;
    metrics::add(&diagnostics.0.detail_queries, 1);
    let row=sqlx::query("SELECT r.execution_id,r.phase,r.provider_state_json,r.created_at_ms FROM agena_rounds r JOIN agena_executions e ON e.id=r.execution_id WHERE r.id=? AND EXISTS(SELECT 1 FROM agena_segment_entries m JOIN agena_session_segments s ON s.segment_id=m.segment_id JOIN agena_sessions owner ON owner.id=s.session_id WHERE s.session_id=? AND owner.deleted_at_ms IS NULL AND m.entry_id=e.message_id AND m.ordinal>=s.first_ordinal AND m.ordinal<s.end_ordinal)")
        .bind(round.get()).bind(session.get()).fetch_optional(&mut *transaction).await.map_err(sql_error)?.ok_or_else(||KernelError::NotFound("provider round".into()))?;
    metrics::add(&diagnostics.0.detail_queries, 1);
    let inputs: Vec<i64> = sqlx::query_scalar(
        "SELECT entry_id FROM agena_round_inputs WHERE round_id=? ORDER BY ordinal LIMIT 10001",
    )
    .bind(round.get())
    .fetch_all(&mut *transaction)
    .await
    .map_err(sql_error)?;
    if inputs.len() > 10000 {
        return Err(storage("provider round input identity budget is corrupt"));
    }
    Ok(ProviderRoundView {
        execution_id: row
            .try_get::<&str, _>("execution_id")
            .map_err(storage)?
            .parse()
            .map_err(storage)?,
        created_at_ms: row.try_get("created_at_ms").map_err(storage)?,
        id: round,
        phase: decode(&encode(
            &row.try_get::<String, _>("phase").map_err(storage)?,
        )?)?,
        inputs: inputs
            .into_iter()
            .map(EntryId::try_from)
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(storage)?,
        provider_state: row
            .try_get::<Option<String>, _>("provider_state_json")
            .map_err(storage)?
            .as_deref()
            .map(decode)
            .transpose()?,
    })
}
