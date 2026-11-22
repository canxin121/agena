//! A branch shares immutable payloads, but owns the control facts and content
//! heads observed at its creation cut. Nested branches reuse that same cut.
use super::*;
use content::heads;
use serde::Deserialize;
use std::{collections::HashMap, sync::LazyLock};

const ENTRY_FIELDS: &str = "revision,execution_phase,execution_started,execution_generation,
    response_entry_id,content_state,compaction_phase,compaction_committed,tool_phase,
    tool_finished_at_ms,interaction_decided,interaction_presented_at,
    interactions_json,activities_json,blobs_json";
const ENTRY_FACTS_JSON: &str = "json_object(
    'revision',revision,'execution_phase',execution_phase,'execution_started',execution_started,
    'execution_generation',execution_generation,'response_entry_id',response_entry_id,
    'content_state',content_state,'compaction_phase',compaction_phase,'compaction_committed',compaction_committed,
    'tool_phase',tool_phase,'tool_finished_at_ms',tool_finished_at_ms,
    'interaction_decided',interaction_decided,'interaction_presented_at',interaction_presented_at,
    'interactions',json(COALESCE(interactions_json,'[]')),
    'activities',json(COALESCE(activities_json,'[]')),'blobs',json(COALESCE(blobs_json,'[]'))
)";
static ENTRY_FACT_SQL: LazyLock<String> = LazyLock::new(|| {
    format!("SELECT {ENTRY_FACTS_JSON} FROM agena_branch_entries WHERE session_id=? AND entry_id=?")
});
static ENTRY_FACTS_SQL: LazyLock<String> = LazyLock::new(|| {
    format!(
        "SELECT entry_id,{ENTRY_FACTS_JSON} AS facts_json FROM agena_branch_entries
         WHERE session_id=? AND entry_id IN (SELECT value FROM json_each(?))"
    )
});
const BRANCH_MEMBERS: &str = "FROM agena_session_segments s
    CROSS JOIN agena_segment_entries m ON m.segment_id=s.segment_id
        AND m.ordinal>=s.first_ordinal AND m.ordinal<s.end_ordinal";
static CAPTURE_ENTRIES_SQL: LazyLock<String> = LazyLock::new(|| {
    format!(
        "INSERT INTO agena_branch_entries (session_id,entry_id,{ENTRY_FIELDS})
         SELECT s.session_id,e.id,e.revision,a.phase,
             EXISTS(SELECT 1 FROM agena_attempts started
                    WHERE started.execution_id=e.execution_id AND started.started_at_ms IS NOT NULL),
             a.generation,original.response_entry_id,c.capture_state,
             CASE WHEN cr.round_id IS NOT NULL THEN r.phase END,
             EXISTS(SELECT 1 FROM agena_entries checkpoint
                    WHERE checkpoint.round_id=e.round_id AND checkpoint.kind='compaction'),
             CASE WHEN e.kind='tool' THEN t.phase END,t.finished_at_ms,i.decision_json IS NOT NULL,
             json_extract(i.request_json,'$.Input.request.presented_at'),
             (SELECT NULLIF(json_group_array(id),'[]') FROM agena_interactions WHERE tool_id=e.tool_id),
             (SELECT NULLIF(json_group_array(id),'[]') FROM agena_background_activities WHERE entry_id=e.id),
             (SELECT NULLIF(json_group_array(blob_id),'[]') FROM agena_blob_references WHERE entry_id=e.id)
         {BRANCH_MEMBERS}
         LEFT JOIN agena_branch_entries f ON f.session_id=?1 AND f.entry_id=m.entry_id
         CROSS JOIN agena_entries e ON e.id=m.entry_id
         CROSS JOIN agena_attempts original ON original.execution_id=e.execution_id AND original.generation=e.generation
         CROSS JOIN agena_attempts a ON a.execution_id=e.execution_id AND a.generation=CASE
             WHEN e.kind='user' THEN (SELECT current_generation FROM agena_executions WHERE id=e.execution_id)
             ELSE e.generation END
         LEFT JOIN agena_content_streams c ON c.id=e.content_id
         LEFT JOIN agena_tool_calls t ON t.id=e.tool_id
         LEFT JOIN agena_interactions i ON i.id=e.interaction_id
         LEFT JOIN agena_rounds r ON r.id=e.round_id
         LEFT JOIN agena_compaction_rounds cr ON cr.round_id=r.id
         WHERE s.session_id=?2 AND e.origin_session_id!=s.session_id AND f.entry_id IS NULL
         UNION ALL
         SELECT s.session_id,m.entry_id,{ENTRY_FIELDS}
         {BRANCH_MEMBERS}
         CROSS JOIN agena_branch_entries f ON f.session_id=?1 AND f.entry_id=m.entry_id
         WHERE s.session_id=?2 AND EXISTS(SELECT 1 FROM agena_branch_entries WHERE session_id=?1)"
    )
});
static CAPTURE_SOURCES_SQL: LazyLock<String> = LazyLock::new(|| {
    let columns = heads::COLUMNS;
    let head_bytes = &*heads::LIVE_HEAD_BYTES;
    format!(
        "INSERT INTO agena_branch_sources(session_id,source_key,{columns})
         SELECT ?2,c.source_key,c.epoch,c.committed_head,c.retained_start,c.capture_state,
                c.lost_bytes,c.lost_packets,c.lost_bytes,c.lost_packets,c.capture_error,{head_bytes}
         FROM agena_branch_entries own
         LEFT JOIN agena_branch_entries parent ON parent.session_id=?1 AND parent.entry_id=own.entry_id
         CROSS JOIN agena_content_streams c ON c.entry_id=own.entry_id
         WHERE own.session_id=?2 AND parent.entry_id IS NULL"
    )
});
const SHARE_SOURCES_SQL: &str =
    "INSERT INTO agena_branch_sources(session_id,source_key,head_owner_session_id)
     SELECT ?2,h.source_key,COALESCE(h.head_owner_session_id,h.session_id)
     FROM agena_branch_entries own
     CROSS JOIN agena_branch_entries parent ON parent.session_id=?1 AND parent.entry_id=own.entry_id
     CROSS JOIN agena_content_streams c ON c.entry_id=parent.entry_id
     CROSS JOIN agena_branch_sources h ON h.session_id=?1 AND h.source_key=c.source_key
     WHERE own.session_id=?2 AND EXISTS(SELECT 1 FROM agena_branch_sources WHERE session_id=?1)";
static SOURCE_SQL: LazyLock<String> = LazyLock::new(|| {
    let columns = heads::READ_COLUMNS;
    format!(
        "SELECT {columns} FROM agena_content_streams c
         CROSS JOIN agena_branch_source_heads h ON h.session_id=? AND h.source_key=c.source_key
         WHERE c.id=?"
    )
});

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
    sqlx::query(&CAPTURE_ENTRIES_SQL)
        .bind(source.get())
        .bind(target.get())
        .execute(&mut *connection)
        .await
        .map_err(sql_error)?;
    for sql in [&*CAPTURE_SOURCES_SQL, SHARE_SOURCES_SQL] {
        sqlx::query(sql)
            .bind(source.get())
            .bind(target.get())
            .execute(&mut *connection)
            .await
            .map_err(sql_error)?;
    }
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
    let row = sqlx::query(&SOURCE_SQL)
        .bind(session.get())
        .bind(content.to_string())
        .fetch_optional(connection)
        .await
        .map_err(sql_error)?;
    row.as_ref()
        .map(|row| heads::read(content, row))
        .transpose()
}

pub(super) async fn encoded_entry_facts(
    connection: &mut SqliteConnection,
    session: SessionId,
    entry: EntryId,
) -> Result<Option<String>> {
    sqlx::query_scalar(&ENTRY_FACT_SQL)
        .bind(session.get())
        .bind(entry.get())
        .fetch_optional(connection)
        .await
        .map_err(sql_error)
}

pub(super) async fn entry_facts(
    connection: &mut SqliteConnection,
    session: SessionId,
    entry: EntryId,
) -> Result<Option<EntryFacts>> {
    let facts = encoded_entry_facts(connection, session, entry).await?;
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
    let rows = sqlx::query(&ENTRY_FACTS_SQL)
        .bind(session.get())
        .bind(encode(ids)?)
        .fetch_all(connection)
        .await
        .map_err(sql_error)?;
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
