//! Indexed output packets in the activity's private state namespace. Keeping
//! them separate from transcript captures preserves the existing text and log
//! resources and lets older databases open with the same schema.

use agena_domain::{
    ActivityId, ActivityOutputChunk, MAX_ACTIVITY_SPILL_PATHS, valid_activity_spill_path,
};
use serde::{Deserialize, Serialize};

use super::*;
use port::ActivityScope;

#[derive(Serialize, Deserialize)]
struct OutputState {
    head: ActivityOutputHead,
    retention: ActivityOutputRetention,
    packets: usize,
    terminal: Option<ActivityOutputTerminal>,
    #[serde(default)]
    trailing_gap: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    spill_paths: Vec<String>,
}

pub(super) fn command_admission(
    command: &Command,
    chunks: &[ActivityOutputChunk],
) -> Result<([u8; 32], usize)> {
    let bytes: usize = chunks.iter().map(|chunk| chunk.text.capacity()).sum();
    if chunks.len() > MAX_ACTIVITY_OUTPUT_CHUNKS || bytes > MAX_ACTIVITY_OUTPUT_BYTES {
        return Err(invalid("activity output batch exceeds its bounds"));
    }
    // JSON escaping must not reduce the advertised UTF-8 byte capacity. The
    // writer queue owns the raw strings, and the receipt needs only a digest.
    struct DigestWriter(Sha256);
    impl std::io::Write for DigestWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.update(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut hash = DigestWriter(Sha256::new());
    serde_json::to_writer(&mut hash, command).map_err(invalid)?;
    Ok((
        hash.0.finalize().into(),
        bytes + std::mem::size_of_val(chunks) + 512,
    ))
}

fn packet_prefix(session: SessionId, activity: ActivityId) -> String {
    format!("activity-output/{}/{activity}/packet/", session.get())
}

fn state_key(session: SessionId, activity: ActivityId) -> String {
    format!("activity-output/{}/{activity}/stream", session.get())
}

fn packet_key(prefix: &str, at: u64) -> String {
    format!("{prefix}{at:020}")
}

async fn load(connection: &mut SqliteConnection, key: &str) -> Result<Option<OutputState>> {
    let row: Option<String> =
        sqlx::query_scalar("SELECT value_json FROM agena_server_state WHERE key=?")
            .bind(key)
            .fetch_optional(connection)
            .await
            .map_err(sql_error)?;
    row.as_deref().map(decode).transpose()
}

async fn store<T: Serialize>(
    connection: &mut SqliteConnection,
    key: &str,
    value: &T,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO agena_server_state(key,value_json) VALUES (?,?)
                 ON CONFLICT(key) DO UPDATE SET value_json=excluded.value_json",
    )
    .bind(key)
    .bind(encode(value)?)
    .execute(connection)
    .await
    .map_err(sql_error)?;
    Ok(())
}

pub(super) async fn initialize(
    connection: &mut SqliteConnection,
    session: SessionId,
    activity: ActivityId,
    retention: ActivityOutputRetention,
) -> Result<()> {
    for limit in [retention.live_bytes, retention.settled_bytes] {
        if !(1..=MAX_ACTIVITY_OUTPUT_BYTES).contains(&limit) {
            return Err(invalid("activity output retention is out of range"));
        }
    }
    store(
        connection,
        &state_key(session, activity),
        &OutputState {
            head: ActivityOutputHead::default(),
            retention,
            packets: 0,
            terminal: None,
            trailing_gap: false,
            spill_paths: Vec::new(),
        },
    )
    .await
}

pub(super) async fn append(
    connection: &mut SqliteConnection,
    scope: ActivityScope,
    total: u64,
    chunks: &[ActivityOutputChunk],
) -> Result<Mutation> {
    super::activity::check(connection, scope, true).await?;
    let key = state_key(scope.session, scope.id);
    let mut state = load(connection, &key)
        .await?
        .ok_or_else(|| invalid("activity has no packet output"))?;
    if chunks.len() > MAX_ACTIVITY_OUTPUT_CHUNKS
        || chunks.iter().map(|chunk| chunk.text.len()).sum::<usize>() > MAX_ACTIVITY_OUTPUT_BYTES
        || total < state.head.total
    {
        return Err(invalid("activity output batch exceeds its bounds"));
    }

    let prefix = packet_prefix(scope.session, scope.id);
    let mut previous = state.head.total;
    for chunk in chunks {
        let end = chunk
            .at
            .checked_add(chunk.text.len() as u64)
            .filter(|end| *end <= total)
            .ok_or_else(|| invalid("invalid activity output offset"))?;
        if chunk.text.is_empty() || chunk.at < previous {
            return Err(invalid("activity output packets overlap or are empty"));
        }
        let mut chunk = chunk.clone();
        chunk.gap_before |= state.trailing_gap || chunk.at > previous;
        state.trailing_gap = false;
        store(connection, &packet_key(&prefix, chunk.at), &chunk).await?;
        if state.packets == 0 {
            state.head.earliest = chunk.at;
        }
        state.packets += 1;
        previous = end;
    }
    state.trailing_gap |= total > previous;
    state.head.total = total;
    let cap = state.retention.live_bytes as u64;
    trim(connection, &prefix, &mut state, cap).await?;
    store(connection, &key, &state).await?;
    Ok(Mutation::ActivityOutput(state.head))
}

pub(super) async fn set_spill_paths(
    connection: &mut SqliteConnection,
    scope: ActivityScope,
    paths: &[String],
) -> Result<Mutation> {
    if paths.len() > MAX_ACTIVITY_SPILL_PATHS
        || paths.iter().any(|path| !valid_activity_spill_path(path))
    {
        return Err(invalid("activity output spill paths exceed their bounds"));
    }
    super::activity::check(connection, scope, true).await?;
    let key = state_key(scope.session, scope.id);
    let mut state = load(connection, &key)
        .await?
        .ok_or_else(|| invalid("activity has no packet output"))?;
    if state.spill_paths != paths {
        state.spill_paths = paths.to_vec();
        store(connection, &key, &state).await?;
    }
    Ok(Mutation::ActivityOutput(state.head))
}

async fn trim(
    connection: &mut SqliteConnection,
    prefix: &str,
    state: &mut OutputState,
    cap: u64,
) -> Result<()> {
    if state.packets == 0 {
        state.head.earliest = state.head.total;
        return Ok(());
    }
    let floor = state.head.total.saturating_sub(cap);
    if state.head.earliest >= floor && state.packets <= MAX_ACTIVITY_OUTPUT_CHUNKS {
        return Ok(());
    }

    let ceiling = format!("{prefix}~");
    let newest: String = sqlx::query_scalar(
        "SELECT key FROM agena_server_state
          WHERE key>=? AND key<? ORDER BY key DESC LIMIT 1",
    )
    .bind(prefix)
    .bind(&ceiling)
    .fetch_one(&mut *connection)
    .await
    .map_err(sql_error)?;
    let mut boundary = packet_key(prefix, floor).min(newest.clone());
    if state.packets > MAX_ACTIVITY_OUTPUT_CHUNKS {
        let by_count: String = sqlx::query_scalar(
            "SELECT key FROM agena_server_state
              WHERE key>=? AND key<? ORDER BY key DESC LIMIT 1 OFFSET ?",
        )
        .bind(prefix)
        .bind(&ceiling)
        .bind((MAX_ACTIVITY_OUTPUT_CHUNKS - 1) as i64)
        .fetch_one(&mut *connection)
        .await
        .map_err(sql_error)?;
        boundary = boundary.max(by_count);
    }
    let deleted = sqlx::query("DELETE FROM agena_server_state WHERE key>=? AND key<?")
        .bind(prefix)
        .bind(&boundary)
        .execute(&mut *connection)
        .await
        .map_err(sql_error)?;
    state.packets -= deleted.rows_affected() as usize;

    if newest < packet_key(prefix, floor) {
        let row: String =
            sqlx::query_scalar("SELECT value_json FROM agena_server_state WHERE key=?")
                .bind(&newest)
                .fetch_one(&mut *connection)
                .await
                .map_err(sql_error)?;
        let mut chunk: ActivityOutputChunk = decode(&row)?;
        let skip = (floor - chunk.at).min(chunk.text.len() as u64) as usize;
        let skip = chunk.text.ceil_char_boundary(skip);
        sqlx::query("DELETE FROM agena_server_state WHERE key=?")
            .bind(&newest)
            .execute(&mut *connection)
            .await
            .map_err(sql_error)?;
        chunk.at += skip as u64;
        chunk.text.drain(..skip);
        chunk.gap_before = true;
        if chunk.text.is_empty() {
            state.packets -= 1;
        } else {
            store(connection, &packet_key(prefix, chunk.at), &chunk).await?;
        }
    }

    let first: Option<String> = sqlx::query_scalar(
        "SELECT key FROM agena_server_state
          WHERE key>=? AND key<? ORDER BY key LIMIT 1",
    )
    .bind(prefix)
    .bind(&ceiling)
    .fetch_optional(&mut *connection)
    .await
    .map_err(sql_error)?;
    state.head.earliest = first
        .as_deref()
        .map(|key| key[prefix.len()..].parse::<u64>().map_err(storage))
        .transpose()?
        .unwrap_or(state.head.total);
    Ok(())
}

pub(super) async fn validate_cursor(
    connection: &mut SqliteConnection,
    session: SessionId,
    activity: ActivityId,
    offset: u64,
) -> Result<()> {
    if load(connection, &state_key(session, activity))
        .await?
        .is_none_or(|state| offset > state.head.total)
    {
        return Err(invalid("activity packet read exceeded its durable output"));
    }
    Ok(())
}

pub(super) async fn settle(
    connection: &mut SqliteConnection,
    session: SessionId,
    activity: ActivityId,
    consumed: u64,
    terminal: Option<&ActivityOutputTerminal>,
) -> Result<()> {
    let key = state_key(session, activity);
    let Some(mut state) = load(connection, &key).await? else {
        return Ok(());
    };
    if let Some(terminal) = terminal {
        if terminal
            .detail
            .as_ref()
            .is_some_and(|detail| detail.len() > 4096)
            || terminal
                .result
                .as_ref()
                .is_some_and(|result| result.len() > 64 * 1024)
        {
            return Err(invalid("activity output settlement exceeds its byte limit"));
        }
        state.terminal = Some(terminal.clone());
    }
    let cap = (state.retention.settled_bytes as u64).max(state.head.total.saturating_sub(consumed));
    trim(
        connection,
        &packet_prefix(session, activity),
        &mut state,
        cap,
    )
    .await?;
    store(connection, &key, &state).await
}

pub(super) async fn authorize(
    connection: &mut SqliteConnection,
    session: SessionId,
    activity: ActivityId,
) -> Result<()> {
    let visible: bool = sqlx::query_scalar("SELECT EXISTS(
        SELECT 1 FROM agena_background_activities a
        JOIN agena_sessions s ON s.id=a.session_id AND s.deleted_at_ms IS NULL
        WHERE a.id=? AND a.session_id=? AND EXISTS (
            SELECT 1 FROM agena_segment_entries e JOIN agena_session_segments ss ON ss.segment_id=e.segment_id
            WHERE e.entry_id=a.entry_id AND ss.session_id=a.session_id
              AND e.ordinal>=ss.first_ordinal AND e.ordinal<ss.end_ordinal
        ))")
        .bind(activity.to_string()).bind(session.get()).fetch_one(connection).await.map_err(sql_error)?;
    if !visible {
        return Err(KernelError::Ownership);
    }
    Ok(())
}

pub(super) async fn read(
    backend: &Backend,
    session: SessionId,
    activity: ActivityId,
    from: Option<u64>,
) -> Result<ReadView<Option<ActivityOutputPage>>> {
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
    authorize(&mut tx, session, activity).await?;
    let value = if let Some(state) = load(&mut tx, &state_key(session, activity)).await? {
        let chunks = if let Some(from) = from.filter(|offset| *offset < state.head.total) {
            let prefix = packet_prefix(session, activity);
            let ceiling = format!("{prefix}~");
            let rows: Vec<String> = sqlx::query_scalar(
                "SELECT value_json FROM agena_server_state
                WHERE key>=COALESCE((SELECT key FROM agena_server_state WHERE key>=? AND key<=?
                    ORDER BY key DESC LIMIT 1), ?) AND key<? ORDER BY key",
            )
            .bind(&prefix)
            .bind(packet_key(&prefix, from))
            .bind(&prefix)
            .bind(&ceiling)
            .fetch_all(&mut *tx)
            .await
            .map_err(sql_error)?;
            let chunks: Vec<ActivityOutputChunk> =
                rows.iter().map(|row| decode(row)).collect::<Result<_>>()?;
            chunks
                .into_iter()
                .filter(|chunk| chunk.at + chunk.text.len() as u64 > from)
                .collect()
        } else {
            Vec::new()
        };
        Some(ActivityOutputPage {
            head: state.head,
            chunks,
            terminal: state.terminal,
            spill_paths: state.spill_paths,
        })
    } else {
        None
    };
    tx.commit().await.map_err(storage)?;
    let bytes = modeled_read_bytes(&value)?;
    backend.wrap_read(session, value, bytes)
}
