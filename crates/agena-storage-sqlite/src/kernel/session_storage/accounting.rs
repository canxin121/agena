use sea_orm::sqlx::{self, Row, SqliteConnection};

use super::*;

pub(super) const FILE_PAGE_SIZE: usize = 30;
pub(super) const MAX_SELECTED_FILES: usize = 100;
const RETAIN_EVENTS: i64 = 128;

pub(super) struct MeasuredSession {
    pub summary: SessionStorageSummary,
    pub shared_history: bool,
    pub retained_bytes: u64,
    pub version: (i64, i64, i64),
}

pub(super) fn unsigned(value: i64) -> u64 {
    u64::try_from(value).unwrap_or_default()
}

const MEASURE: &str = r#"
WITH target AS MATERIALIZED (
    SELECT id FROM agena_sessions WHERE id=?1
    UNION ALL SELECT id FROM agena_sessions WHERE ?1 IS NULL
), payload_target AS MATERIALIZED (
    SELECT id FROM target WHERE ?2
), payloads(id, bucket, bytes) AS (
    SELECT s.id, 0, length(CAST(s.configuration_json AS BLOB))
        + COALESCE(length(CAST(s.selection_json AS BLOB)),0)
        FROM payload_target t CROSS JOIN agena_sessions s ON s.id=t.id
    UNION ALL SELECT e.origin_session_id, 0, e.body_bytes
        FROM agena_entries e JOIN payload_target s ON s.id=e.origin_session_id
    UNION ALL SELECT r.session_id, 0, r.provider_state_bytes
        + COALESCE(length(CAST(r.input_metadata_json AS BLOB)),0)
        FROM agena_rounds r JOIN payload_target s ON s.id=r.session_id
    UNION ALL SELECT r.session_id, 0, i.head_bytes
        FROM agena_round_input_sources i JOIN agena_rounds r ON r.id=i.round_id JOIN payload_target s ON s.id=r.session_id
    UNION ALL SELECT r.session_id, 1, t.input_bytes+t.result_bytes
        FROM agena_tool_calls t JOIN agena_rounds r ON r.id=t.round_id JOIN payload_target s ON s.id=r.session_id
    UNION ALL SELECT s.id, CASE WHEN e.kind='tool' OR c.kind!='text' THEN 1 ELSE 0 END, b.body_bytes
        FROM payload_target s CROSS JOIN agena_entries e ON e.origin_session_id=s.id
        CROSS JOIN agena_content_streams c ON c.entry_id=e.id
        CROSS JOIN agena_content_blocks b ON b.source_key=c.source_key
    UNION ALL SELECT b.session_id, 2, length(c.body)
        FROM agena_blob_chunks c JOIN agena_blobs b ON b.id=c.blob_id JOIN payload_target s ON s.id=b.session_id
    UNION ALL SELECT e.session_id, 0,
        COALESCE(length(CAST(e.interactions_json AS BLOB)),0)
        + COALESCE(length(CAST(e.activities_json AS BLOB)),0)
        + COALESCE(length(CAST(e.blobs_json AS BLOB)),0)
        + COALESCE(length(CAST(e.interaction_presented_at AS BLOB)),0)
        FROM agena_branch_entries e JOIN payload_target s ON s.id=e.session_id
    UNION ALL SELECT e.session_id, 0, e.head_bytes
        FROM agena_branch_sources e JOIN payload_target s ON s.id=e.session_id WHERE e.head_owner_session_id IS NULL
    UNION ALL SELECT e.session_id, 1, e.request_bytes+e.decision_bytes+e.remember_bytes
        FROM agena_interactions e JOIN payload_target s ON s.id=e.session_id
    UNION ALL SELECT e.session_id, 1, length(CAST(e.outcome_json AS BLOB))
        FROM agena_background_activities e JOIN payload_target s ON s.id=e.session_id
    UNION ALL SELECT e.session_id, 0, length(CAST(e.payload_json AS BLOB))
        FROM agena_deliveries e JOIN payload_target s ON s.id=e.session_id
    UNION ALL SELECT e.owner_session_id, 0, length(CAST(e.run_json AS BLOB))
        FROM agena_scheduler_history e JOIN payload_target s ON s.id=e.owner_session_id
    UNION ALL SELECT e.source_session_id, 0, length(CAST(e.reminder_json AS BLOB))
        FROM agena_assistant_reminders e JOIN payload_target s ON s.id=e.source_session_id
        WHERE e.state IN ('completed','cancelled')
    UNION ALL SELECT e.session_id, 0, length(CAST(e.record_json AS BLOB))
        FROM agena_channel_sessions e JOIN payload_target s ON s.id=e.session_id
), totals AS (
    SELECT id, SUM(CASE WHEN bucket=0 THEN bytes END) AS conversation,
        SUM(CASE WHEN bucket=1 THEN bytes END) AS recordings,
        SUM(CASE WHEN bucket=2 THEN bytes END) AS media
    FROM payloads GROUP BY id
)
SELECT s.id,s.title,s.workspace_id,s.owner_bot_id,s.owner_surface,s.deleted_at_ms IS NOT NULL AS deleted,
    s.updated_at_ms,s.metadata_version,s.membership_generation,s.control_version,w.path,b.name AS bot_name,
    CASE WHEN ?2 THEN length(CAST(s.configuration_json AS BLOB))
        + COALESCE(length(CAST(s.selection_json AS BLOB)),0) ELSE 0 END AS retained_bytes,
    COALESCE(t.conversation,0) AS conversation,COALESCE(t.recordings,0) AS recordings,COALESCE(t.media,0) AS media,
    EXISTS(SELECT 1 FROM agena_attempts a WHERE a.session_id=s.id AND a.phase IN ('queued','running'))
        OR EXISTS(SELECT 1 FROM agena_background_activities a WHERE a.session_id=s.id AND a.phase='running')
        OR EXISTS(SELECT 1 FROM agena_content_streams c WHERE c.session_id=s.id AND c.capture_state='open')
        OR EXISTS(SELECT 1 FROM agena_sessions child
                   CROSS JOIN agena_attempts a ON a.session_id=child.id
                   WHERE child.root_session_id=COALESCE(s.root_session_id,s.id)
                     AND child.relation_kind='subagent' AND child.deleted_at_ms IS NULL
                     AND a.phase IN ('queued','running')) AS busy,
    EXISTS(SELECT 1 FROM agena_history_snapshots h WHERE h.session_id=s.id)
        OR EXISTS(SELECT 1 FROM agena_blobs m JOIN agena_blob_pins p ON p.blob_id=m.id
                   WHERE m.session_id=s.id AND p.token IN (SELECT value FROM json_each(?3)))
        AS snapshot_open,
    CASE WHEN ?1 IS NULL THEN 0 ELSE (EXISTS (
        SELECT 1 FROM agena_entries e CROSS JOIN agena_branch_entries f ON f.entry_id=e.id
        WHERE e.origin_session_id=s.id AND f.session_id!=s.id
    ) OR EXISTS (
        SELECT 1 FROM agena_entries e CROSS JOIN agena_branch_entries f ON f.response_entry_id=e.id
        WHERE e.origin_session_id=s.id AND f.session_id!=s.id
    ) OR EXISTS (
        SELECT 1 FROM agena_entries e CROSS JOIN agena_segment_entries m ON m.entry_id=e.id
        CROSS JOIN agena_session_segments p ON p.segment_id=m.segment_id
        WHERE e.origin_session_id=s.id AND p.session_id!=s.id
    ) OR EXISTS (
        SELECT 1 FROM agena_entries e CROSS JOIN agena_segment_entries m ON m.entry_id=e.id
        CROSS JOIN agena_retired_session_segments p ON p.segment_id=m.segment_id
        WHERE e.origin_session_id=s.id AND p.session_id!=s.id
    ) OR EXISTS (
        SELECT 1 FROM agena_entries e CROSS JOIN agena_session_runs p ON p.entry_id=e.id
        WHERE e.origin_session_id=s.id AND p.session_id!=s.id
    ) OR EXISTS (
        SELECT 1 FROM agena_entries e CROSS JOIN agena_prompt_input_entries i ON i.entry_id=e.id
        CROSS JOIN agena_round_input_blocks l ON l.block_id=i.block_id
        CROSS JOIN agena_rounds r ON r.id=l.round_id
        WHERE e.origin_session_id=s.id AND r.session_id!=s.id
    ) OR EXISTS (
        SELECT 1 FROM agena_entries e CROSS JOIN agena_prompt_input_entries i ON i.response_entry_id=e.id
        CROSS JOIN agena_round_input_blocks l ON l.block_id=i.block_id
        CROSS JOIN agena_rounds r ON r.id=l.round_id
        WHERE e.origin_session_id=s.id AND r.session_id!=s.id
    ) OR EXISTS (
        SELECT 1 FROM agena_rounds r CROSS JOIN agena_prompt_replay_entries i ON i.source_round_id=r.id
        CROSS JOIN agena_round_replay_blocks l ON l.block_id=i.block_id
        CROSS JOIN agena_rounds other ON other.id=l.round_id
        WHERE r.session_id=s.id AND other.session_id!=s.id
    ) OR EXISTS (
        SELECT 1 FROM agena_entries e CROSS JOIN agena_content_streams c ON c.entry_id=e.id
        CROSS JOIN agena_branch_sources f ON f.source_key=c.source_key
        WHERE e.origin_session_id=s.id AND f.session_id!=s.id
    ) OR EXISTS (
        SELECT 1 FROM agena_branch_sources f WHERE f.head_owner_session_id=s.id
    ) OR EXISTS (
        SELECT 1 FROM agena_blobs m CROSS JOIN agena_blob_references f ON f.blob_id=m.id
        CROSS JOIN agena_entries e ON e.id=f.entry_id
        WHERE m.session_id=s.id AND e.origin_session_id!=s.id
    ) OR EXISTS (
        SELECT 1 FROM agena_entries e JOIN agena_prompt_windows p
          ON p.checkpoint_entry_id=e.id OR p.through_entry_id=e.id
        WHERE e.origin_session_id=s.id AND p.session_id!=s.id
    ) OR EXISTS (
        SELECT 1 FROM agena_entries e JOIN agena_compaction_rounds c ON c.base_checkpoint_entry_id=e.id
        JOIN agena_rounds r ON r.id=c.round_id
        WHERE e.origin_session_id=s.id AND r.session_id!=s.id
    ) OR EXISTS (
        SELECT 1 FROM agena_entries e CROSS JOIN agena_content_streams c ON c.entry_id=e.id
        CROSS JOIN agena_round_input_sources i ON i.source_key=c.source_key
        CROSS JOIN agena_rounds r ON r.id=i.round_id
        WHERE e.origin_session_id=s.id AND r.session_id!=s.id
    ) OR EXISTS (
        SELECT 1 FROM agena_entries e CROSS JOIN agena_content_streams c ON c.entry_id=e.id
        CROSS JOIN agena_entries other ON other.content_id=c.id
        WHERE e.origin_session_id=s.id AND other.origin_session_id!=s.id
    ) OR EXISTS (
        SELECT 1 FROM agena_rounds r JOIN agena_branch_rounds f ON f.round_id=r.id
        WHERE r.session_id=s.id AND f.session_id!=s.id
    ) OR EXISTS (
        SELECT 1 FROM agena_background_activities a JOIN agena_branch_activities f ON f.activity_id=a.id
        WHERE a.session_id=s.id AND f.session_id!=s.id
    ) OR EXISTS (
        SELECT 1 FROM agena_entries e JOIN agena_deliveries d ON d.result_entry_id=e.id
        WHERE e.origin_session_id=s.id AND d.session_id!=s.id
    ) OR EXISTS (
        SELECT 1 FROM agena_rounds r JOIN agena_deliveries d ON d.observed_round_id=r.id
        WHERE r.session_id=s.id AND d.session_id!=s.id
    ) OR EXISTS (
        SELECT 1 FROM agena_executions x CROSS JOIN agena_deliveries d ON d.target_execution_id=x.id
        WHERE x.session_id=s.id AND d.session_id!=s.id
    ) OR EXISTS (
        SELECT 1 FROM agena_executions x CROSS JOIN agena_deliveries d ON d.resume_execution_id=x.id
        WHERE x.session_id=s.id AND d.session_id!=s.id
    ) OR EXISTS (
        SELECT 1 FROM agena_entries e JOIN agena_executions x ON x.message_id=e.id
        WHERE e.origin_session_id=s.id AND x.session_id!=s.id
    ) OR EXISTS (
        SELECT 1 FROM agena_entries e JOIN agena_entries other ON other.message_id=e.id
        WHERE e.origin_session_id=s.id AND other.origin_session_id!=s.id
    ) OR EXISTS (
        SELECT 1 FROM agena_entries e JOIN agena_attempts a ON a.response_entry_id=e.id
        WHERE e.origin_session_id=s.id AND a.session_id!=s.id
    )) END AS shared_history
FROM target ids CROSS JOIN agena_sessions s ON s.id=ids.id LEFT JOIN totals t ON t.id=s.id
LEFT JOIN agena_workspaces w ON w.id=s.workspace_id LEFT JOIN agena_bots b ON b.id=s.owner_bot_id
"#;

pub(super) async fn measure(
    connection: &mut SqliteConnection,
    session: Option<SessionId>,
    include_payloads: bool,
    active_blob_tokens: &[String],
) -> Result<Vec<MeasuredSession>> {
    let rows = sqlx::query(MEASURE)
        .bind(session.map(SessionId::get))
        .bind(include_payloads)
        .bind(encode(active_blob_tokens)?)
        .fetch_all(connection)
        .await
        .map_err(sql_error)?;
    rows.into_iter()
        .map(|row| {
            let id = SessionId::try_from(row.try_get::<i64, _>("id").map_err(storage)?)
                .map_err(storage)?;
            let busy = row.try_get("busy").map_err(storage)?;
            let snapshot_open = row.try_get("snapshot_open").map_err(storage)?;
            Ok(MeasuredSession {
                summary: SessionStorageSummary {
                    session_id: id,
                    title: row.try_get("title").map_err(storage)?,
                    workspace_id: row.try_get("workspace_id").map_err(storage)?,
                    workspace_name: row
                        .try_get::<Option<String>, _>("path")
                        .map_err(storage)?
                        .map(|path| {
                            Path::new(&path)
                                .file_name()
                                .and_then(|name| name.to_str())
                                .unwrap_or(&path)
                                .to_owned()
                        }),
                    bot_id: row.try_get("owner_bot_id").map_err(storage)?,
                    bot_name: row.try_get("bot_name").map_err(storage)?,
                    surface: row.try_get("owner_surface").map_err(storage)?,
                    deleted: row.try_get("deleted").map_err(storage)?,
                    busy,
                    busy_reason: if busy {
                        Some(StorageProtection::Running)
                    } else if snapshot_open {
                        Some(StorageProtection::Reading)
                    } else {
                        None
                    },
                    modified_at_ms: row.try_get("updated_at_ms").map_err(storage)?,
                    bytes: SessionStorageBytes {
                        conversation: unsigned(row.try_get("conversation").map_err(storage)?),
                        recordings: unsigned(row.try_get("recordings").map_err(storage)?),
                        media: unsigned(row.try_get("media").map_err(storage)?),
                        ..Default::default()
                    },
                },
                shared_history: row.try_get("shared_history").map_err(storage)?,
                retained_bytes: unsigned(row.try_get("retained_bytes").map_err(storage)?),
                version: (
                    row.try_get("metadata_version").map_err(storage)?,
                    row.try_get("membership_generation").map_err(storage)?,
                    row.try_get("control_version").map_err(storage)?,
                ),
            })
        })
        .collect()
}

pub(super) fn count_files(summary: &mut SessionStorageSummary, files: &[File]) {
    let totals = &mut summary.bytes;
    for file in files {
        let bucket = match file.kind {
            SessionFileKind::Cache => &mut totals.cache,
            SessionFileKind::Plugin => &mut totals.plugins,
            SessionFileKind::Upload | SessionFileKind::Artifact => &mut totals.files,
        };
        *bucket = bucket.saturating_add(file.bytes());
    }
}

pub(super) async fn referenced(
    connection: &mut SqliteConnection,
    name: &str,
    except: Option<SessionId>,
) -> Result<bool> {
    // Session-owned upload and generated file names contain stable call or
    // upload identities. Also check queued and retired entries, including
    // references stored as escaped JSON strings or inside a local URL.
    let encoded = encode(name)?;
    let escaped = &encoded[1..encoded.len() - 1];
    sqlx::query_scalar(
        r#"
        SELECT EXISTS(SELECT 1 FROM agena_entries e
            WHERE (?2 IS NULL OR e.origin_session_id!=?2) AND e.body_json IS NOT NULL
              AND (instr(e.body_json,?1)>0 OR instr(e.body_json,?3)>0))
        OR EXISTS(SELECT 1 FROM agena_tool_calls t JOIN agena_rounds r ON r.id=t.round_id
            WHERE (?2 IS NULL OR r.session_id!=?2)
              AND (instr(t.input_json,?1)>0 OR instr(t.input_json,?3)>0
                   OR instr(t.result_json,?1)>0 OR instr(t.result_json,?3)>0))
        OR EXISTS(SELECT 1 FROM agena_deliveries d WHERE (?2 IS NULL OR d.session_id!=?2)
            AND (instr(d.payload_json,?1)>0 OR instr(d.payload_json,?3)>0))
    "#,
    )
    .bind(name)
    .bind(except.map(SessionId::get))
    .bind(escaped)
    .fetch_one(connection)
    .await
    .map_err(sql_error)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct Recording {
    pub id: ContentId,
    pub before: u64,
    pub bytes: u64,
}

pub(super) async fn recordings(
    connection: &mut SqliteConnection,
    session: SessionId,
) -> Result<Vec<Recording>> {
    // Logs are a separate stream from the tool result included in model input.
    // A retained terminal checkpoint or a mixed document may encode a base
    // snapshot, so the bulk log action only selects plain command logs.
    let rows = sqlx::query(
        r#"
        SELECT c.id, c.committed_head-?2 AS cut, SUM(b.body_bytes) AS bytes
        FROM agena_entries own CROSS JOIN agena_content_streams c ON c.entry_id=own.id
        CROSS JOIN agena_content_blocks b ON b.source_key=c.source_key AND b.logical_end<=c.committed_head-?2
        WHERE own.origin_session_id=?1 AND c.kind='log' AND c.capture_state!='open'
          AND c.committed_head-?2>c.retained_start
          AND NOT EXISTS(SELECT 1 FROM agena_branch_sources ref WHERE ref.source_key=c.source_key)
          AND NOT EXISTS(SELECT 1 FROM agena_round_input_sources ref WHERE ref.source_key=c.source_key)
          AND NOT EXISTS(SELECT 1 FROM agena_prompt_windows w JOIN agena_entries e
                         ON e.id=w.checkpoint_entry_id WHERE e.content_id=c.id)
        GROUP BY c.id ORDER BY c.id
    "#,
    )
    .bind(session.get())
    .bind(RETAIN_EVENTS)
    .fetch_all(connection)
    .await
    .map_err(sql_error)?;
    rows.into_iter()
        .map(|row| {
            Ok(Recording {
                id: row
                    .try_get::<String, _>("id")
                    .map_err(storage)?
                    .parse()
                    .map_err(storage)?,
                before: unsigned(row.try_get("cut").map_err(storage)?),
                bytes: unsigned(row.try_get("bytes").map_err(storage)?),
            })
        })
        .collect()
}

pub(super) async fn recording_shared(
    connection: &mut SqliteConnection,
    content: ContentId,
) -> Result<bool> {
    sqlx::query_scalar(
        "SELECT EXISTS(
             SELECT 1 FROM agena_content_streams c WHERE c.id=? AND (
                 EXISTS(SELECT 1 FROM agena_branch_sources b WHERE b.source_key=c.source_key)
                 OR EXISTS(SELECT 1 FROM agena_round_input_sources i WHERE i.source_key=c.source_key)
             )
         )",
    )
    .bind(content.to_string())
    .fetch_one(connection)
    .await
    .map_err(sql_error)
}

pub(super) async fn bots(connection: &mut SqliteConnection) -> Result<Vec<StorageBot>> {
    let rows = sqlx::query(r#"
        SELECT b.id,b.name,length(CAST(b.instructions AS BLOB))
          + COALESCE(length(CAST(a.avatar AS BLOB)),0) AS profile_bytes,
          COALESCE((SELECT SUM(length(CAST(g.binding_json AS BLOB))
             + COALESCE((SELECT SUM(length(CAST(p.event_json AS BLOB))+COALESCE(length(CAST(i.note AS BLOB)),0))
                 FROM agena_integration_inbox i CROSS JOIN agena_integration_event_payloads p ON p.inbox_id=i.id
                 WHERE i.binding_id=g.id),0)
             + COALESCE((SELECT SUM(COALESCE(length(CAST(t.text AS BLOB)),0)
                    + COALESCE(length(CAST(o.route_json AS BLOB)),0)
                    + COALESCE(length(CAST(o.remote_id AS BLOB)),0)
                    + COALESCE(length(CAST(o.error AS BLOB)),0))
                 FROM agena_integration_outbox o LEFT JOIN agena_integration_delivery_text t ON t.delivery_key=o.delivery_key
                 WHERE o.binding_id=g.id),0)
             + COALESCE((SELECT SUM(length(CAST(a.input_json AS BLOB))+length(a.result_json)
                  +length(CAST(a.audit_json AS BLOB))) FROM agena_channel_access_audit a WHERE a.binding_id=g.id),0)
          ) FROM agena_integration_bindings g WHERE g.owner_bot_id=b.id),0) AS channel_bytes
        FROM agena_bots b LEFT JOIN agena_bot_avatars a ON a.bot_id=b.id
        ORDER BY b.name_folded,b.id
    "#).fetch_all(connection).await.map_err(sql_error)?;
    rows.into_iter()
        .map(|row| {
            Ok(StorageBot {
                id: row.try_get("id").map_err(storage)?,
                name: row.try_get("name").map_err(storage)?,
                profile_bytes: unsigned(row.try_get("profile_bytes").map_err(storage)?),
                channel_bytes: unsigned(row.try_get("channel_bytes").map_err(storage)?),
                channel_files: 0,
                session_count: 0,
                session_bytes: 0,
            })
        })
        .collect()
}
