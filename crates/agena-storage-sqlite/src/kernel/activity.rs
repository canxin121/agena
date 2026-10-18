//! Launch authorization is attempt-scoped; the minted activity lease is not.
//! Recovery settles recorded intents without repeating external side effects.

use agena_domain::{ActivityId, ContentKind};
use port::{ActivityScope, OutputScope, Scope};
use sqlx::sqlite::SqliteRow;

use super::*;

pub(super) const MAX_ACTIVITIES_PER_TOOL: i64 = 16;
const MAX_ACTIVE_ACTIVITIES: i64 = 128;
pub(super) const ACTIVE_SQL: &str = "SELECT * FROM agena_background_activities INDEXED BY idx_agena_background_active WHERE session_id=? AND phase='running' AND id>? ORDER BY id LIMIT ?";

pub(super) async fn begin(
    connection: &mut SqliteConnection,
    scope: Scope,
    tool: ToolId,
    start: &BackgroundActivityStart,
) -> Result<Mutation> {
    commands::check_scope(connection, scope, true).await?;
    let round = commands::check_tool(connection, scope, tool).await?;
    if start.label.is_empty() || start.label.len() > 1024 {
        return Err(invalid("activity label must contain 1..1024 bytes"));
    }
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
        return Err(invalid("tool activity identity limit exceeded"));
    }
    let entry: i64 =
        sqlx::query_scalar("SELECT id FROM agena_entries WHERE tool_id=? AND kind='tool'")
            .bind(tool.get())
            .fetch_one(&mut *connection)
            .await
            .map_err(sql_error)?;
    let id = ActivityId::new();
    sqlx::query("INSERT INTO agena_background_activities (id,session_id,execution_id,generation,round_id,tool_id,entry_id,owner_epoch,kind,label,phase,created_at_ms) VALUES (?,?,?,?,?,?,?,?,?,?,'running',?)")
        .bind(id.to_string()).bind(scope.session.get()).bind(scope.execution.to_string()).bind(scope.generation.get()).bind(round.get()).bind(tool.get()).bind(entry).bind(scope.epoch.to_string()).bind(commands::enum_text(&start.kind)?).bind(&start.label).bind(now_ms())
        .execute(&mut *connection).await.map_err(sql_error)?;
    bump(connection, entry).await?;
    Ok(Mutation::Activity(id))
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
) -> Result<ReadView<BackgroundActivityView>> {
    backend.cache.ensure_open()?;
    let _read = backend
        .control_reads
        .clone()
        .try_acquire_owned()
        .map_err(|_| KernelError::Busy)?;
    let row = sqlx::query("SELECT a.* FROM agena_background_activities a WHERE a.id=? AND EXISTS(SELECT 1 FROM agena_segment_entries m JOIN agena_session_segments s ON s.segment_id=m.segment_id JOIN agena_sessions owner ON owner.id=s.session_id WHERE m.entry_id=a.entry_id AND s.session_id=? AND owner.deleted_at_ms IS NULL AND m.ordinal>=s.first_ordinal AND m.ordinal<s.end_ordinal)")
        .bind(id.to_string()).bind(session.get()).fetch_optional(backend.reader.get_sqlite_connection_pool()).await.map_err(sql_error)?.ok_or_else(||KernelError::NotFound("activity".into()))?;
    let value = view(&row)?;
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
