//! Launch authorization is attempt-scoped; the minted activity lease is not.
//! Recovery settles recorded intents without repeating external side effects.

use agena_domain::{ActivityId, ContentKind};
use port::{ActivityScope, OutputScope, Scope};
use sqlx::sqlite::SqliteRow;

use super::*;

mod command_activity;
pub(super) use command_activity::begin_command;

pub(super) const MAX_ACTIVITIES_PER_TOOL: i64 = 16;
const MAX_ACTIVE_ACTIVITIES: i64 = 128;
pub(super) const ACTIVE_SQL: &str = "SELECT * FROM agena_background_activities INDEXED BY idx_agena_background_active WHERE session_id=? AND phase='running' AND id>? ORDER BY id LIMIT ?";

pub(super) async fn begin(
    connection: &mut SqliteConnection,
    scope: Scope,
    tool: ToolId,
    start: &BackgroundActivityStart,
    output_retention: Option<ActivityOutputRetention>,
) -> Result<Mutation> {
    commands::check_scope(connection, scope, true).await?;
    let round = commands::check_tool(connection, scope, tool).await?;
    let runtime: bool =
        sqlx::query_scalar("SELECT execution_kind='runtime' FROM agena_tool_calls WHERE id=?")
            .bind(tool.get())
            .fetch_one(&mut *connection)
            .await
            .map_err(sql_error)?;
    if !runtime {
        return Err(KernelError::Conflict(
            "provider-hosted operations cannot launch local activities".into(),
        ));
    }
    // Pending, denied and cancelled interactions cannot mint a side-effect
    // capability. Completed user-input answers are compatible with a launch.
    let blocked: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agena_interactions WHERE tool_id=? AND (decision_json IS NULL OR (json_extract(decision_json,'$.kind')='cancel' OR (json_extract(decision_json,'$.kind')='permission' AND json_extract(decision_json,'$.value.allowed')=0))))")
        .bind(tool.get()).fetch_one(&mut *connection).await.map_err(sql_error)?;
    if blocked {
        return Err(KernelError::Conflict(
            "activity launch has unresolved or rejected authorization".into(),
        ));
    }
    let entry: i64 =
        sqlx::query_scalar("SELECT id FROM agena_entries WHERE tool_id=? AND kind='tool'")
            .bind(tool.get())
            .fetch_one(&mut *connection)
            .await
            .map_err(sql_error)?;
    let origin = ActivityOrigin { scope, round, tool, entry };
    Ok(Mutation::Activity(insert(connection, origin, start, output_retention).await?))
}

struct ActivityOrigin {
    scope: Scope,
    round: RoundId,
    tool: ToolId,
    entry: i64,
}

async fn insert(
    connection: &mut SqliteConnection,
    origin: ActivityOrigin,
    start: &BackgroundActivityStart,
    output_retention: Option<ActivityOutputRetention>,
) -> Result<ActivityId> {
    if start.label.is_empty() || start.label.len() > 1024 {
        return Err(invalid("activity label must contain 1..1024 bytes"));
    }
    let ActivityOrigin { scope, round, tool, entry } = origin;
    let active: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM agena_background_activities WHERE phase='running'",
    )
    .fetch_one(&mut *connection)
    .await
    .map_err(sql_error)?;
    if active >= MAX_ACTIVE_ACTIVITIES {
        return Err(KernelError::Busy);
    }
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM agena_background_activities WHERE tool_id=?")
            .bind(tool.get())
            .fetch_one(&mut *connection)
            .await
            .map_err(sql_error)?;
    if count >= MAX_ACTIVITIES_PER_TOOL {
        return Err(invalid("operation activity identity limit exceeded"));
    }
    let id = ActivityId::new();
    sqlx::query("INSERT INTO agena_background_activities (id,session_id,execution_id,generation,round_id,tool_id,entry_id,owner_epoch,kind,label,phase,created_at_ms) VALUES (?,?,?,?,?,?,?,?,?,?,'running',?)")
        .bind(id.to_string()).bind(scope.session.get()).bind(scope.execution.to_string()).bind(scope.generation.get()).bind(round.get()).bind(tool.get()).bind(entry).bind(scope.epoch.to_string()).bind(commands::enum_text(&start.kind)?).bind(&start.label).bind(now_ms())
        .execute(&mut *connection).await.map_err(sql_error)?;
    if let Some(retention) = output_retention {
        super::activity_output::initialize(connection, scope.session, id, retention).await?;
    }
    bump(connection, entry).await?;
    Ok(id)
}

pub(super) async fn check(
    connection: &mut SqliteConnection,
    scope: ActivityScope,
    running: bool,
) -> Result<SqliteRow> {
    let row = sqlx::query("SELECT a.* FROM agena_background_activities a JOIN agena_sessions s ON s.id=a.session_id WHERE a.id=? AND a.session_id=? AND a.owner_epoch=? AND s.deleted_at_ms IS NULL")
        .bind(scope.id.to_string()).bind(scope.session.get()).bind(scope.epoch.to_string()).fetch_optional(connection).await.map_err(sql_error)?.ok_or(KernelError::Ownership)?;
    if running && row.try_get::<&str, _>("phase").map_err(storage)? != "running" {
        return Err(KernelError::Conflict("activity has ended".into()));
    }
    Ok(row)
}

pub(super) async fn check_output(
    connection: &mut SqliteConnection,
    scope: OutputScope,
) -> Result<()> {
    match scope {
        OutputScope::Attempt(scope) => {
            commands::check_scope(connection, scope, true).await?;
        }
        OutputScope::Activity(scope) => {
            check(connection, scope, true).await?;
        }
    }
    Ok(())
}

pub(super) async fn owns_output(
    connection: &mut SqliteConnection,
    scope: OutputScope,
    content: ContentId,
    epoch: ContentEpoch,
) -> Result<bool> {
    let sql = match scope {
        OutputScope::Attempt(_) => {
            "SELECT EXISTS(SELECT 1 FROM agena_content_streams WHERE id=? AND epoch=? AND activity_id IS NULL AND session_id=? AND execution_id=? AND generation=?)"
        }
        OutputScope::Activity(_) => {
            "SELECT EXISTS(SELECT 1 FROM agena_content_streams WHERE id=? AND epoch=? AND session_id=? AND activity_id=?)"
        }
    };
    let query = sqlx::query_scalar(sql)
        .bind(content.to_string())
        .bind(epoch.to_string())
        .bind(scope.session().get());
    let query = match scope {
        OutputScope::Attempt(scope) => query
            .bind(scope.execution.to_string())
            .bind(scope.generation.get()),
        OutputScope::Activity(scope) => query.bind(scope.id.to_string()),
    };
    query.fetch_one(connection).await.map_err(sql_error)
}

pub(super) async fn end(
    connection: &mut SqliteConnection,
    scope: ActivityScope,
    outcome: &BackgroundActivityOutcome,
    output_terminal: Option<&ActivityOutputTerminal>,
) -> Result<Mutation> {
    let row = check(connection, scope, false).await?;
    let encoded = encode(outcome)?;
    if encoded.len() > 64 * 1024
        || outcome
            .capture_end()
            .diagnostic()
            .is_some_and(|text| text.len() > 4096)
    {
        return Err(invalid(
            "activity result or capture diagnostic exceeds its byte limit",
        ));
    }
    if let Some(previous) = row
        .try_get::<Option<&str>, _>("outcome_json")
        .map_err(storage)?
    {
        if decode::<BackgroundActivityOutcome>(previous)? != *outcome {
            return Err(KernelError::Conflict(
                "activity already ended differently".into(),
            ));
        }
        return Ok(Mutation::Activity(scope.id));
    }
    sqlx::query("UPDATE agena_background_activities SET phase=json_extract(?,'$.outcome'),outcome_json=?,finished_at_ms=MAX(?,created_at_ms) WHERE id=?")
        .bind(&encoded).bind(&encoded).bind(now_ms()).bind(scope.id.to_string()).execute(&mut *connection).await.map_err(sql_error)?;
    if output_terminal.is_some() {
        let position = load_output_position(connection, &output_key(scope.session, scope.id)).await?;
        super::activity_output::settle(connection, scope.session, scope.id,
            position.packet_offset.unwrap_or_default(), output_terminal).await?;
    }
    bump(connection, row.try_get("entry_id").map_err(storage)?).await?;
    Ok(Mutation::Activity(scope.id))
}

async fn bump(connection: &mut SqliteConnection, entry: i64) -> Result<()> {
    sqlx::query("UPDATE agena_entries SET revision=revision+1 WHERE id=?")
        .bind(entry)
        .execute(connection)
        .await
        .map_err(sql_error)?;
    Ok(())
}

pub(super) async fn open_output(
    connection: &mut SqliteConnection,
    scope: ActivityScope,
    kind: ContentKind,
) -> Result<Mutation> {
    if !matches!(
        kind,
        ContentKind::Text
            | ContentKind::Log
            | ContentKind::Structured
            | ContentKind::Terminal
            | ContentKind::Document
    ) {
        return Err(invalid("unsupported activity output kind"));
    }
    let row = check(connection, scope, true).await?;
    let active: i64 =
        sqlx::query_scalar("SELECT count(*) FROM agena_content_streams WHERE capture_state='open'")
            .fetch_one(&mut *connection)
            .await
            .map_err(sql_error)?;
    if active >= content::MAX_LIVE_SOURCES {
        return Err(KernelError::Busy);
    }
    let entry: i64 = row.try_get("entry_id").map_err(storage)?;
    content::bind_tool_entry(connection, entry).await?;
    let id = ContentId::new();
    sqlx::query("INSERT INTO agena_content_streams (id,entry_id,session_id,execution_id,generation,activity_id,kind,epoch,capture_state) VALUES (?,?,?,?,?,?,?,?,'open')")
        .bind(id.to_string()).bind(entry).bind(scope.session.get()).bind(row.try_get::<&str,_>("execution_id").map_err(storage)?).bind(row.try_get::<i64,_>("generation").map_err(storage)?).bind(scope.id.to_string()).bind(commands::enum_text(&kind)?).bind(ContentEpoch::new().to_string()).execute(&mut *connection).await.map_err(sql_error)?;
    content::events::initialize(connection, id, kind).await
}

pub(super) fn view(row: &SqliteRow) -> Result<BackgroundActivityView> {
    Ok(BackgroundActivityView {
        id: row
            .try_get::<String, _>("id")
            .map_err(storage)?
            .parse()
            .map_err(storage)?,
        origin_session_id: SessionId::try_from(
            row.try_get::<i64, _>("session_id").map_err(storage)?,
        )
        .map_err(storage)?,
        launch_tool: ToolId::try_from(row.try_get::<i64, _>("tool_id").map_err(storage)?)
            .map_err(storage)?,
        launch_entry: EntryId::try_from(row.try_get::<i64, _>("entry_id").map_err(storage)?)
            .map_err(storage)?,
        start: BackgroundActivityStart {
            kind: decode(&encode(&row.try_get::<&str, _>("kind").map_err(storage)?)?)?,
            label: row.try_get("label").map_err(storage)?,
        },
        outcome: row
            .try_get::<Option<&str>, _>("outcome_json")
            .map_err(storage)?
            .map(decode)
            .transpose()?,
        created_at_ms: row.try_get("created_at_ms").map_err(storage)?,
        finished_at_ms: row.try_get("finished_at_ms").map_err(storage)?,
    })
}

pub(super) async fn read(
    backend: &Backend,
    session: SessionId,
    id: ActivityId,
    listed_only: bool,
) -> Result<ReadView<BackgroundActivityView>> {
    backend.cache.ensure_open()?;
    let _read = backend
        .control_reads
        .clone()
        .try_acquire_owned()
        .map_err(|_| KernelError::Busy)?;
    let row = sqlx::query(
        "SELECT a.* FROM agena_background_activities a
         WHERE a.id=? AND EXISTS (
             SELECT 1 FROM agena_segment_entries m
             JOIN agena_session_segments s ON s.segment_id=m.segment_id
             JOIN agena_sessions owner ON owner.id=s.session_id
             WHERE m.entry_id=a.entry_id AND s.session_id=? AND owner.deleted_at_ms IS NULL
               AND m.ordinal>=s.first_ordinal AND m.ordinal<s.end_ordinal
         ) AND (NOT ? OR (a.session_id=? AND NOT EXISTS (
             SELECT 1 FROM agena_server_state WHERE key=?
         )))",
    )
    .bind(id.to_string())
    .bind(session.get())
    .bind(listed_only)
    .bind(session.get())
    .bind(unlisted_key(session, id))
    .fetch_optional(backend.reader.get_sqlite_connection_pool())
    .await
    .map_err(sql_error)?
    .ok_or_else(|| KernelError::NotFound("activity".into()))?;
    let mut value = view(&row)?;
    let mut connection = backend
        .reader
        .get_sqlite_connection_pool()
        .acquire()
        .await
        .map_err(storage)?;
    if branches::inherited_entry(&mut connection, session, value.launch_entry).await? {
        let frozen = sqlx::query("SELECT has_outcome,finished_at_ms FROM agena_branch_activities WHERE session_id=? AND activity_id=?")
            .bind(session.get()).bind(id.to_string()).fetch_optional(&mut *connection).await.map_err(sql_error)?
            .ok_or_else(|| KernelError::NotFound("activity at branch boundary".into()))?;
        value.finished_at_ms = frozen.try_get("finished_at_ms").map_err(storage)?;
        if frozen.try_get::<i64, _>("has_outcome").map_err(storage)? == 0 {
            value.outcome = None;
        }
    }
    let bytes = modeled_read_bytes(&value)?;
    backend.wrap_read(session, value, bytes)
}

pub(super) async fn active(
    backend: &Backend,
    session: SessionId,
    after: Option<ActiveActivityCursor>,
    limit: usize,
) -> Result<ReadView<ActiveActivityPage>> {
    backend.cache.ensure_open()?;
    if after.is_some_and(|cursor| cursor.session != session) {
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
    reads::metadata(&mut tx, session).await?;
    let limit = limit.clamp(1, 100);
    let rows = sqlx::query(ACTIVE_SQL)
        .bind(session.get())
        .bind(after.map_or_else(String::new, |cursor| cursor.after.to_string()))
        .bind((limit + 1) as i64)
        .fetch_all(&mut *tx)
        .await
        .map_err(sql_error)?;
    let has_more = rows.len() > limit;
    let activities = rows
        .iter()
        .take(limit)
        .map(view)
        .collect::<Result<Vec<_>>>()?;
    let next = has_more.then(|| ActiveActivityCursor {
        session,
        after: activities.last().expect("nonempty activity page").id,
    });
    let value = ActiveActivityPage {
        activities,
        next,
        has_more,
    };
    let bytes = modeled_read_bytes(&value)?;
    backend.wrap_read(session, value, bytes)
}

pub(super) async fn session_activities_by_label(
    backend: &Backend,
    session: SessionId,
    prefix: &str,
    limit: usize,
) -> Result<ReadView<Vec<BackgroundActivityView>>> {
    backend.cache.ensure_open()?;
    let _read = backend
        .control_reads
        .clone()
        .try_acquire_owned()
        .map_err(|_| KernelError::Busy)?;
    let rows = sqlx::query(
        "SELECT a.* FROM agena_background_activities a
         JOIN agena_sessions owner ON owner.id=a.session_id
         WHERE a.session_id=? AND owner.deleted_at_ms IS NULL
           AND substr(a.label, 1, length(?))=?
           AND NOT EXISTS (
               SELECT 1 FROM agena_server_state
               WHERE key='activity-output/' || a.session_id || '/' || a.id || '/unlisted'
           )
           AND EXISTS (
               SELECT 1 FROM agena_segment_entries m
               JOIN agena_session_segments s ON s.segment_id=m.segment_id
               WHERE m.entry_id=a.entry_id AND s.session_id=a.session_id
                 AND m.ordinal>=s.first_ordinal AND m.ordinal<s.end_ordinal
           )
         ORDER BY (a.phase='running') DESC, a.entry_id DESC, a.created_at_ms DESC, a.id
         LIMIT ?",
    )
    .bind(session.get())
    .bind(prefix)
    .bind(prefix)
    .bind(limit.clamp(1, 32) as i64)
    .fetch_all(backend.reader.get_sqlite_connection_pool())
    .await
    .map_err(sql_error)?;
    let value = rows.iter().map(view).collect::<Result<Vec<_>>>()?;
    let bytes = modeled_read_bytes(&value)?;
    backend.wrap_read(session, value, bytes)
}

fn output_key(session: SessionId, activity: ActivityId) -> String {
    format!("activity-output/{}/{activity}", session.get())
}

fn unlisted_key(session: SessionId, activity: ActivityId) -> String {
    format!("{}/unlisted", output_key(session, activity))
}

pub(super) async fn unlist_settled(
    connection: &mut SqliteConnection,
    session: SessionId,
    activity: ActivityId,
) -> Result<Mutation> {
    let result = sqlx::query(
        "INSERT INTO agena_server_state(key, value_json)
         SELECT ?, 'true' FROM agena_background_activities a
         JOIN agena_sessions owner ON owner.id=a.session_id
         WHERE a.id=? AND a.session_id=? AND a.phase!='running'
           AND owner.deleted_at_ms IS NULL
           AND EXISTS (
               SELECT 1 FROM agena_segment_entries m
               JOIN agena_session_segments s ON s.segment_id=m.segment_id
               WHERE m.entry_id=a.entry_id AND s.session_id=a.session_id
                 AND m.ordinal>=s.first_ordinal AND m.ordinal<s.end_ordinal
           )
         ON CONFLICT(key) DO NOTHING",
    )
    .bind(unlisted_key(session, activity))
    .bind(activity.to_string())
    .bind(session.get())
    .execute(connection)
    .await
    .map_err(sql_error)?;
    Ok(Mutation::ActivityUnlisted(result.rows_affected() > 0))
}

pub(super) fn notice_key(activity: ActivityId) -> String {
    format!("activity:{activity}")
}

/// The cursor is internal session state. Keep the exact database baseline and
/// the immutable activity outcome intact while advancing it.
pub(super) async fn output_position(
    backend: &Backend,
    session: SessionId,
    activity: ActivityId,
) -> Result<ActivityOutputPosition> {
    backend.cache.ensure_open()?;
    let _read = backend
        .control_reads
        .clone()
        .try_acquire_owned()
        .map_err(|_| KernelError::Busy)?;
    let value: Option<String> = sqlx::query_scalar(
        "SELECT position.value_json
         FROM agena_background_activities a
         JOIN agena_sessions s ON s.id=a.session_id AND s.deleted_at_ms IS NULL
         LEFT JOIN agena_server_state position ON position.key=?
         WHERE a.id=? AND a.session_id=?",
    )
    .bind(output_key(session, activity))
    .bind(activity.to_string())
    .bind(session.get())
    .fetch_optional(backend.reader.get_sqlite_connection_pool())
    .await
    .map_err(sql_error)?
    .ok_or(KernelError::Ownership)?;
    Ok(value
        .as_deref()
        .map(decode)
        .transpose()?
        .unwrap_or_default())
}

pub(super) async fn acknowledge_output(
    connection: &mut SqliteConnection,
    session: SessionId,
    read: ActivityOutputRead,
) -> Result<()> {
    let key = output_key(session, read.activity);
    let activity = read.activity.to_string();
    let phase: String = sqlx::query_scalar(
        "SELECT phase FROM agena_background_activities WHERE id=? AND session_id=?",
    )
    .bind(&activity)
    .bind(session.get())
    .fetch_optional(&mut *connection)
    .await
    .map_err(sql_error)?
    .ok_or(KernelError::Ownership)?;
    if read.position.outcome_read && phase == "running" {
        return Err(invalid("cannot acknowledge an unsettled activity's result"));
    }
    if let Some(offset) = read.position.packet_offset {
        if read.position.cursor.is_some() || read.position.lost_bytes != 0 {
            return Err(invalid("activity packet and text positions are distinct"));
        }
        super::activity_output::validate_cursor(connection, session, read.activity, offset).await?;
    } else if let Some(cursor) = read.position.cursor {
        let valid: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM agena_content_streams
             WHERE activity_id=? AND session_id=? AND id=? AND epoch=?
               AND kind='text' AND committed_head>=? AND lost_bytes>=?)",
        )
        .bind(&activity)
        .bind(session.get())
        .bind(cursor.content.to_string())
        .bind(cursor.epoch.to_string())
        .bind(i64::try_from(cursor.offset).map_err(invalid)?)
        .bind(i64::try_from(read.position.lost_bytes).map_err(invalid)?)
        .fetch_one(&mut *connection)
        .await
        .map_err(sql_error)?;
        if !valid {
            return Err(invalid("activity read exceeded its durable output"));
        }
    } else if read.position.lost_bytes > 0 {
        return Err(invalid("activity output loss requires a content cursor"));
    }
    let mut position = load_output_position(connection, &key).await?;
    if let Some(offset) = read.position.packet_offset {
        position.packet_offset = Some(position.packet_offset.unwrap_or_default().max(offset));
    }
    if let Some(cursor) = read.position.cursor {
        if position.cursor.is_some_and(|current| {
            current.content == cursor.content && current.epoch == cursor.epoch
        }) {
            let previous = position.cursor.as_mut().expect("checked cursor");
            previous.offset = previous.offset.max(cursor.offset);
            position.lost_bytes = position.lost_bytes.max(read.position.lost_bytes);
        } else {
            position.cursor = Some(cursor);
            position.lost_bytes = read.position.lost_bytes;
        }
    }
    position.outcome_read |= read.position.outcome_read;
    sqlx::query(
        "INSERT INTO agena_server_state(key,value_json) VALUES (?,?)
         ON CONFLICT(key) DO UPDATE SET value_json=excluded.value_json",
    )
    .bind(key)
    .bind(encode(&position)?)
    .execute(&mut *connection)
    .await
    .map_err(sql_error)?;
    if read.position.outcome_read && let Some(offset) = position.packet_offset {
        super::activity_output::settle(connection, session, read.activity, offset, None).await?;
    }
    Ok(())
}

pub(super) async fn acknowledge_direct_output(
    connection: &mut SqliteConnection,
    session: SessionId,
    read: ActivityOutputRead,
    expected: ActivityOutputPosition,
) -> Result<Mutation> {
    super::activity_output::authorize(connection, session, read.activity).await?;
    // A pending completion notice may have been enqueued before the direct
    // reader acquired its gate and delivered while it loaded the packets.
    if load_output_position(connection, &output_key(session, read.activity)).await? != expected {
        return Ok(Mutation::ActivityOutputAcknowledged(false));
    }
    acknowledge_output(connection, session, read).await?;
    if read.position.outcome_read {
        retire_pending_notice(connection, session, read.activity).await?;
    }
    Ok(Mutation::ActivityOutputAcknowledged(true))
}

async fn load_output_position(connection: &mut SqliteConnection, key: &str) -> Result<ActivityOutputPosition> {
    let value: Option<String> = sqlx::query_scalar("SELECT value_json FROM agena_server_state WHERE key=?")
        .bind(key).fetch_optional(connection).await.map_err(sql_error)?;
    Ok(value.as_deref().map(decode).transpose()?.unwrap_or_default())
}

pub(super) async fn retire_pending_notice(
    connection: &mut SqliteConnection,
    session: SessionId,
    activity: ActivityId,
) -> Result<()> {
    sqlx::query(
        "UPDATE agena_deliveries SET state='failed'
         WHERE session_id=? AND business_key=? AND state='pending'",
    )
    .bind(session.get())
    .bind(notice_key(activity))
    .execute(connection)
    .await
    .map_err(sql_error)?;
    Ok(())
}

pub(super) async fn acknowledge_notice_output(
    connection: &mut SqliteConnection,
    delivery: &DeliveryView,
) -> Result<()> {
    let Some(output) = delivery.payload.get("activity_output") else {
        return Ok(());
    };
    if !delivery.business_key.starts_with("activity:") {
        return Ok(());
    }
    let read: ActivityOutputRead = serde_json::from_value(output.clone()).map_err(invalid)?;
    if delivery.business_key != notice_key(read.activity) {
        return Err(KernelError::Ownership);
    }
    acknowledge_output(connection, delivery.session, read).await
}
