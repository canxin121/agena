use chrono::{DateTime, Utc};
use futures_util::TryStreamExt;

use super::*;

const ROUND_USAGE_PREDICATE: &str = "json_type(provider_state_json,'$.usage')='object'";

fn tool_usage_path() -> String {
    // Dot-containing metadata keys are quoted in SQLite JSON paths.
    format!(
        r#"'$.metadata."{}"'"#,
        agena_provider::PROVIDER_TOOL_USAGE_METADATA_KEY
    )
}

fn tool_usage_predicate(path: &str) -> String {
    format!("json_type(result_json,{path})='text'")
}

pub(super) fn indexes() -> [(&'static str, String); 3] {
    let tools = tool_usage_predicate(&tool_usage_path());
    [
        (
            "idx_agena_round_usage_time",
            format!(
                "CREATE INDEX IF NOT EXISTS idx_agena_round_usage_time ON agena_rounds(created_at_ms) WHERE {ROUND_USAGE_PREDICATE}"
            ),
        ),
        (
            "idx_agena_tool_usage_time",
            format!(
                "CREATE INDEX IF NOT EXISTS idx_agena_tool_usage_time ON agena_tool_calls(created_at_ms) WHERE {tools}"
            ),
        ),
        (
            "idx_agena_tool_usage_round",
            format!(
                "CREATE INDEX IF NOT EXISTS idx_agena_tool_usage_round ON agena_tool_calls(round_id) WHERE {tools}"
            ),
        ),
    ]
}

fn millisecond_boundary(value: DateTime<Utc>) -> i64 {
    // Stored times are integer milliseconds. Round either boundary up so
    // the interval remains [from, to) even for finer-grained RFC3339 input.
    value.timestamp_millis() + i64::from(!value.timestamp_subsec_nanos().is_multiple_of(1_000_000))
}

fn filter_observations(
    rows: &mut sqlx::QueryBuilder<'_, sqlx::Sqlite>,
    scope: UsageScope<'_>,
    timestamp: &str,
) -> Result<()> {
    rows.push(" AND s.deleted_at_ms IS NULL");
    match scope {
        UsageScope::Session(session) => {
            rows.push(" AND s.id=").push_bind(session.get());
        }
        UsageScope::Instance(query) => {
            rows.push(" AND s.temporary=0");
            if let Some(from) = query.from {
                rows.push(" AND ")
                    .push(timestamp)
                    .push(" >= ")
                    .push_bind(millisecond_boundary(from));
            }
            if let Some(to) = query.to {
                rows.push(" AND ")
                    .push(timestamp)
                    .push(" < ")
                    .push_bind(millisecond_boundary(to));
            }
            if !query.include_subagents {
                rows.push(" AND s.relation_kind!='subagent'");
            }
            match query.session_ids.as_slice() {
                [] => {}
                // The unbounded-start report can start from its single owner.
                // A bounded window also benefits from the time-driven plan.
                [session] if query.from.is_none() => {
                    rows.push(" AND s.id=").push_bind(*session);
                }
                sessions => {
                    rows.push(" AND s.id IN (SELECT value FROM json_each(")
                        .push_bind(encode(sessions)?)
                        .push("))");
                }
            }
        }
    }
    Ok(())
}

pub(super) async fn visit_observations(
    backend: &Backend,
    scope: UsageScope<'_>,
    visit: &mut (dyn FnMut(StoredUsageObservation) + Send),
) -> Result<()> {
    backend.cache.ensure_open()?;
    let _read = backend
        .content_reads
        .clone()
        .try_acquire_owned()
        .map_err(|_| KernelError::Busy)?;
    // Apply the owner and time filters inside each UNION ALL branch so SQLite
    // can use the owner and time indexes directly for either observation kind.
    // Joining the original owner keeps fork references out of billing.
    let path = tool_usage_path();
    let tool_predicate = tool_usage_predicate(&path);
    let tool_tables = if matches!(scope, UsageScope::Session(_)) {
        "agena_rounds r CROSS JOIN agena_tool_calls t INDEXED BY idx_agena_tool_usage_round ON t.round_id=r.id"
    } else {
        "agena_tool_calls t JOIN agena_rounds r ON r.id=t.round_id"
    };
    let mut rows = sqlx::QueryBuilder::new(format!(
        "SELECT s.id, s.title, s.relation_kind='subagent' AS is_subagent,
                r.created_at_ms, 1 AS round,
                json_object('version', r.provider_state_json -> '$.version',
                            'model', r.provider_state_json -> '$.model',
                            'usage', r.provider_state_json -> '$.usage') AS payload
         FROM agena_rounds r JOIN agena_sessions s ON s.id=r.session_id
         WHERE {ROUND_USAGE_PREDICATE}",
    ));
    filter_observations(&mut rows, scope, "r.created_at_ms")?;
    rows.push(format!(
        " UNION ALL
          SELECT s.id, s.title, s.relation_kind='subagent',
                 t.created_at_ms, 0, json_extract(t.result_json,{path})
          FROM {tool_tables} JOIN agena_sessions s ON s.id=r.session_id
          WHERE {tool_predicate}",
    ));
    filter_observations(&mut rows, scope, "t.created_at_ms")?;
    let mut tx = backend
        .reader
        .get_sqlite_connection_pool()
        .begin()
        .await
        .map_err(storage)?;
    if let UsageScope::Session(session) = scope {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM agena_sessions WHERE id=? AND deleted_at_ms IS NULL)",
        )
        .bind(session.get())
        .fetch_one(&mut *tx)
        .await
        .map_err(sql_error)?;
        if !exists {
            return Err(KernelError::NotFound("session".into()));
        }
    }
    let mut stream = rows.build().fetch(&mut *tx);
    while let Some(row) = stream.try_next().await.map_err(sql_error)? {
        let payload = row.try_get("payload").map_err(storage)?;
        let source = if row.try_get("round").map_err(storage)? {
            StoredUsageSource::Round(payload)
        } else {
            StoredUsageSource::Tool(payload)
        };
        visit(StoredUsageObservation {
            session_id: SessionId::try_from(row.try_get::<i64, _>("id").map_err(storage)?)
                .map_err(storage)?,
            session_title: row.try_get("title").map_err(storage)?,
            is_subagent: row.try_get("is_subagent").map_err(storage)?,
            created_at_ms: row.try_get("created_at_ms").map_err(storage)?,
            source,
        });
    }
    Ok(())
}
