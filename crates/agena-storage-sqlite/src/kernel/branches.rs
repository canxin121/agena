//! A branch shares immutable payloads, but owns the control facts and content
//! heads observed at its creation cut. Nested branches reuse that same cut.
use super::*;
use serde::Deserialize;
use std::collections::HashMap;

pub(super) async fn backfill(pool: &sqlx::SqlitePool) -> Result<()> {
    let mut transaction = pool.begin().await.map_err(storage)?;
    let sessions = sqlx::query("SELECT id,parent_session_id FROM agena_sessions s WHERE s.relation_kind IN ('fork','rewind') AND s.parent_session_id IS NOT NULL AND NOT EXISTS(SELECT 1 FROM agena_branch_entries captured WHERE captured.session_id=s.id) AND EXISTS(SELECT 1 FROM agena_session_segments segments JOIN agena_segment_entries m ON m.segment_id=segments.segment_id AND m.ordinal>=segments.first_ordinal AND m.ordinal<segments.end_ordinal WHERE segments.session_id=s.id AND NOT EXISTS(SELECT 1 FROM agena_branch_entries b WHERE b.session_id=s.id AND b.entry_id=m.entry_id) AND EXISTS(SELECT 1 FROM agena_entries e WHERE e.id=m.entry_id AND e.origin_session_id!=s.id)) ORDER BY s.depth,s.id")
        .fetch_all(&mut *transaction).await.map_err(sql_error)?;
    for row in sessions {
        let session =
            SessionId::try_from(row.try_get::<i64, _>("id").map_err(storage)?).map_err(storage)?;
        let parent = SessionId::try_from(
            row.try_get::<i64, _>("parent_session_id")
                .map_err(storage)?,
        )
        .map_err(storage)?;
        // Older databases did not record the historical content cut. Freeze
        // their currently visible inherited facts once, before recovery can
        // mutate open attempts/streams. Previously lost cuts are unrecoverable.
        capture(&mut transaction, parent, session).await?;
    }
    transaction.commit().await.map_err(storage)
}

#[derive(Debug, Deserialize)]
pub(super) struct EntryFacts {
    revision: EntryRevision,
    execution_phase: ExecutionPhase,
    execution_started: i64,
    execution_generation: AttemptGeneration,
    response_entry_id: Option<EntryId>,
    content_state: Option<CaptureState>,
    compaction_phase: Option<ProviderRoundPhase>,
    compaction_committed: i64,
    tool_phase: Option<ToolPhase>,
    tool_finished_at_ms: Option<i64>,
    interaction_decided: i64,
    interaction_presented_at: Option<chrono::DateTime<chrono::Utc>>,
    sources: Vec<String>,
    interactions: Vec<i64>,
    activities: Vec<String>,
    blobs: Vec<String>,
}

impl EntryFacts {
    pub(super) fn apply_execution(&self, execution: &mut ExecutionView) {
        execution.generation = self.execution_generation;
        execution.phase = self.execution_phase;
    }

    fn apply_entry(&self, entry: &mut EntryView) {
        entry.revision = self.revision;
        entry.execution_phase = self.execution_phase;
        entry.execution_started = self.execution_started != 0;
        entry.execution_generation = self.execution_generation;
        entry.response_entry_id = self.response_entry_id;
        entry.content_state = self.content_state;
        entry.compaction_phase = self.compaction_phase;
        entry.compaction_committed = self.compaction_committed != 0;
    }

    pub(super) fn apply_tool(&self, tool: &mut ToolView) {
        tool.revision = self.revision;
        if let Some(phase) = self.tool_phase {
            tool.phase = phase;
            tool.finished_at_ms = self.tool_finished_at_ms;
            if phase == ToolPhase::Preparing {
                tool.call.input = serde_json::Value::Null;
            }
            if matches!(phase, ToolPhase::Preparing | ToolPhase::Running) {
                tool.result = None;
            }
        }
        tool.outputs
            .retain(|output| self.sources.contains(&output.id.to_string()));
        tool.interactions
            .retain(|id| self.interactions.contains(&id.get()));
        tool.activities
            .retain(|id| self.activities.contains(&id.to_string()));
        self.apply_blobs(&mut tool.blobs);
    }

    pub(super) fn apply_blobs(&self, blobs: &mut Vec<BlobDescriptor>) {
        blobs.retain(|blob| self.blobs.contains(&blob.id.to_string()));
    }

    pub(super) fn apply_interaction(&self, interaction: &mut InteractionView) {
        interaction.revision = self.revision;
        if let InteractionRequest::Input { request } = &mut interaction.request {
            request.presented_at = self.interaction_presented_at;
        }
        if self.interaction_decided == 0 {
            interaction.decision = None;
            interaction.remembered = None;
            interaction.decided_at_ms = None;
        }
    }
}

pub(super) async fn capture(
    connection: &mut SqliteConnection,
    source: SessionId,
    target: SessionId,
) -> Result<()> {
    // Copy small mutable facts only. User documents, tool payloads, replay and
    // content blocks retain their existing immutable identities and storage.
    sqlx::query("INSERT OR IGNORE INTO agena_branch_entries (session_id,entry_id,facts_json) SELECT ?,e.id,COALESCE(f.facts_json,json_object('revision',e.revision,'execution_phase',a.phase,'execution_started',EXISTS(SELECT 1 FROM agena_attempts started WHERE started.execution_id=e.execution_id AND started.started_at_ms IS NOT NULL),'execution_generation',a.generation,'response_entry_id',original.response_entry_id,'content_state',c.capture_state,'compaction_phase',CASE WHEN cr.round_id IS NOT NULL THEN r.phase END,'compaction_committed',EXISTS(SELECT 1 FROM agena_entries checkpoint WHERE checkpoint.round_id=e.round_id AND checkpoint.kind='compaction'),'tool_phase',CASE WHEN e.kind='tool' THEN t.phase END,'tool_finished_at_ms',t.finished_at_ms,'interaction_decided',i.decision_json IS NOT NULL,'interaction_presented_at',json_extract(i.request_json,'$.Input.request.presented_at'),'sources',(SELECT json_group_array(id) FROM agena_content_streams WHERE entry_id=e.id),'interactions',(SELECT json_group_array(id) FROM agena_interactions WHERE tool_id=e.tool_id),'activities',(SELECT json_group_array(id) FROM agena_background_activities WHERE entry_id=e.id),'blobs',(SELECT json_group_array(blob_id) FROM agena_blob_references WHERE entry_id=e.id))) FROM agena_session_segments s JOIN agena_segment_entries m ON m.segment_id=s.segment_id AND m.ordinal>=s.first_ordinal AND m.ordinal<s.end_ordinal JOIN agena_entries e ON e.id=m.entry_id JOIN agena_attempts original ON original.execution_id=e.execution_id AND original.generation=e.generation JOIN agena_attempts a ON a.execution_id=e.execution_id AND a.generation=CASE WHEN e.kind='user' THEN (SELECT current_generation FROM agena_executions WHERE id=e.execution_id) ELSE e.generation END LEFT JOIN agena_content_streams c ON c.id=e.content_id LEFT JOIN agena_tool_calls t ON t.id=e.tool_id LEFT JOIN agena_interactions i ON i.id=e.interaction_id LEFT JOIN agena_rounds r ON r.id=e.round_id LEFT JOIN agena_compaction_rounds cr ON cr.round_id=e.round_id LEFT JOIN agena_branch_entries f ON f.session_id=? AND f.entry_id=e.id WHERE s.session_id=? AND e.origin_session_id!=s.session_id")
        .bind(target.get()).bind(source.get()).bind(target.get()).execute(&mut *connection).await.map_err(sql_error)?;
    sqlx::query("INSERT OR IGNORE INTO agena_branch_sources (session_id,content_id,entry_id,head_json) SELECT ?,c.id,c.entry_id,CASE WHEN parent.entry_id IS NOT NULL THEN inherited.head_json ELSE json_object('id',c.id,'entry',c.entry_id,'epoch',c.epoch,'kind',c.kind,'activity_id',c.activity_id,'ordinal',c.ordinal,'capture_loss',json_object('bytes',c.lost_bytes,'packets',c.lost_packets,'committed_bytes',c.lost_bytes,'committed_packets',c.lost_packets),'accepted',c.committed_head,'committed',c.committed_head,'retained_start',c.retained_start,'state',c.capture_state,'capture_error',c.capture_error) END FROM agena_branch_entries own JOIN agena_content_streams c ON c.entry_id=own.entry_id LEFT JOIN agena_branch_entries parent ON parent.session_id=? AND parent.entry_id=c.entry_id LEFT JOIN agena_branch_sources inherited ON inherited.session_id=? AND inherited.content_id=c.id WHERE own.session_id=? AND (parent.entry_id IS NULL OR inherited.content_id IS NOT NULL)")
        .bind(target.get()).bind(source.get()).bind(source.get()).bind(target.get()).execute(&mut *connection).await.map_err(sql_error)?;
    sqlx::query("INSERT OR IGNORE INTO agena_branch_rounds (session_id,round_id,phase,has_provider_state,finished_at_ms) SELECT DISTINCT ?,r.id,CASE WHEN f.round_id IS NOT NULL THEN f.phase ELSE r.phase END,CASE WHEN f.round_id IS NOT NULL THEN f.has_provider_state ELSE r.provider_state_json IS NOT NULL END,CASE WHEN f.round_id IS NOT NULL THEN f.finished_at_ms ELSE r.finished_at_ms END FROM agena_branch_entries own JOIN agena_entries e ON e.id=own.entry_id JOIN agena_rounds r ON r.id=e.round_id LEFT JOIN agena_branch_entries parent ON parent.session_id=? AND parent.entry_id=e.id LEFT JOIN agena_branch_rounds f ON f.session_id=? AND f.round_id=r.id WHERE own.session_id=? AND (parent.entry_id IS NULL OR f.round_id IS NOT NULL)")
        .bind(target.get()).bind(source.get()).bind(source.get()).bind(target.get()).execute(&mut *connection).await.map_err(sql_error)?;
    sqlx::query("INSERT OR IGNORE INTO agena_branch_activities (session_id,activity_id,has_outcome,finished_at_ms) SELECT ?,a.id,CASE WHEN f.activity_id IS NOT NULL THEN f.has_outcome ELSE a.outcome_json IS NOT NULL END,CASE WHEN f.activity_id IS NOT NULL THEN f.finished_at_ms ELSE a.finished_at_ms END FROM agena_branch_entries own JOIN agena_background_activities a ON a.entry_id=own.entry_id LEFT JOIN agena_branch_entries parent ON parent.session_id=? AND parent.entry_id=a.entry_id LEFT JOIN agena_branch_activities f ON f.session_id=? AND f.activity_id=a.id WHERE own.session_id=? AND (parent.entry_id IS NULL OR f.activity_id IS NOT NULL)")
        .bind(target.get()).bind(source.get()).bind(source.get()).bind(target.get()).execute(&mut *connection).await.map_err(sql_error)?;
    Ok(())
}

pub(super) async fn inherited_entry(
    connection: &mut SqliteConnection,
    session: SessionId,
    entry: EntryId,
) -> Result<bool> {
    sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM agena_branch_entries WHERE session_id=? AND entry_id=?)",
    )
    .bind(session.get())
    .bind(entry.get())
    .fetch_one(connection)
    .await
    .map_err(sql_error)
}

pub(super) async fn source(
    connection: &mut SqliteConnection,
    session: SessionId,
    content: ContentId,
) -> Result<Option<ContentStreamView>> {
    let head: Option<String> = sqlx::query_scalar(
        "SELECT head_json FROM agena_branch_sources WHERE session_id=? AND content_id=?",
    )
    .bind(session.get())
    .bind(content.to_string())
    .fetch_optional(connection)
    .await
    .map_err(sql_error)?;
    head.as_deref().map(decode).transpose()
}

pub(super) async fn entry_facts(
    connection: &mut SqliteConnection,
    session: SessionId,
    entry: EntryId,
) -> Result<Option<EntryFacts>> {
    let facts: Option<String> = sqlx::query_scalar(
        "SELECT facts_json FROM agena_branch_entries WHERE session_id=? AND entry_id=?",
    )
    .bind(session.get())
    .bind(entry.get())
    .fetch_optional(connection)
    .await
    .map_err(sql_error)?;
    facts.as_deref().map(decode).transpose()
}

pub(super) async fn apply_entries(
    connection: &mut SqliteConnection,
    session: SessionId,
    entries: &mut [EntryView],
) -> Result<()> {
    apply_entry_refs(connection, session, entries.iter_mut().collect()).await
}

async fn captured_facts(
    connection: &mut SqliteConnection,
    session: SessionId,
    ids: &[i64],
) -> Result<HashMap<i64, EntryFacts>> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows = sqlx::query("SELECT entry_id,facts_json FROM agena_branch_entries WHERE session_id=? AND entry_id IN (SELECT value FROM json_each(?))")
        .bind(session.get()).bind(encode(&ids)?).fetch_all(connection).await.map_err(sql_error)?;
    let mut facts = HashMap::with_capacity(rows.len());
    for row in rows {
        facts.insert(
            row.try_get::<i64, _>("entry_id").map_err(storage)?,
            decode::<EntryFacts>(row.try_get("facts_json").map_err(storage)?)?,
        );
    }
    Ok(facts)
}

async fn apply_entry_refs(
    connection: &mut SqliteConnection,
    session: SessionId,
    entries: Vec<&mut EntryView>,
) -> Result<()> {
    let ids: Vec<_> = entries.iter().map(|entry| entry.id.get()).collect();
    let facts = captured_facts(connection, session, &ids).await?;
    for entry in entries {
        if let Some(facts) = facts.get(&entry.id.get()) {
            facts.apply_entry(entry);
        }
    }
    Ok(())
}

pub(super) async fn freeze_round(
    connection: &mut SqliteConnection,
    session: SessionId,
    round: i64,
) -> Result<()> {
    sqlx::query("UPDATE agena_round_inputs AS input SET (entry_revision,response_entry_id,execution_phase,tool_phase,interaction_decided)=(SELECT json_extract(facts_json,'$.revision'),json_extract(facts_json,'$.response_entry_id'),json_extract(facts_json,'$.execution_phase'),json_extract(facts_json,'$.tool_phase'),json_extract(facts_json,'$.interaction_decided') FROM agena_branch_entries WHERE session_id=? AND entry_id=input.entry_id) WHERE input.round_id=? AND EXISTS(SELECT 1 FROM agena_branch_entries WHERE session_id=? AND entry_id=input.entry_id)")
        .bind(session.get()).bind(round).bind(session.get()).execute(&mut *connection).await.map_err(sql_error)?;
    sqlx::query("UPDATE agena_round_input_replays AS input SET (phase,has_provider_state)=(SELECT phase,has_provider_state FROM agena_branch_rounds WHERE session_id=? AND round_id=input.source_round_id) WHERE input.round_id=? AND EXISTS(SELECT 1 FROM agena_branch_rounds WHERE session_id=? AND round_id=input.source_round_id)")
        .bind(session.get()).bind(round).bind(session.get()).execute(connection).await.map_err(sql_error)?;
    Ok(())
}

pub(super) async fn apply_prompt_entries(
    connection: &mut SqliteConnection,
    session: SessionId,
    entries: &mut [PromptEntry],
) -> Result<()> {
    let ids: Vec<_> = entries.iter().map(|entry| entry.entry.id.get()).collect();
    let facts = captured_facts(connection, session, &ids).await?;
    for entry in entries {
        if let Some(facts) = facts.get(&entry.entry.id.get()) {
            facts.apply_entry(&mut entry.entry);
            if let Some(interaction) = &mut entry.interaction {
                facts.apply_interaction(interaction);
            }
        }
    }
    Ok(())
}

pub(super) async fn apply_snapshot_entries(
    connection: &mut SqliteConnection,
    session: SessionId,
    entries: &mut [SnapshotEntry],
) -> Result<()> {
    apply_entry_refs(
        connection,
        session,
        entries.iter_mut().map(|entry| &mut entry.entry).collect(),
    )
    .await
}
