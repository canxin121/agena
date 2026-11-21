use agena_domain::UsageStatsQuery;
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

pub(super) fn indexes() -> [(&'static str, String); 2] {
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
    ]
}

fn millisecond_boundary(value: DateTime<Utc>) -> i64 {
    // Stored times are integer milliseconds. Round either boundary up so
    // the interval remains [from, to) even for finer-grained RFC3339 input.
    value.timestamp_millis() + i64::from(!value.timestamp_subsec_nanos().is_multiple_of(1_000_000))
}

pub(super) async fn visit_observations(
    backend: &Backend,
    query: &UsageStatsQuery,
    visit: &mut (dyn FnMut(StoredUsageObservation) + Send),
) -> Result<()> {
    backend.cache.ensure_open()?;
    let _read = backend
        .content_reads
        .clone()
        .try_acquire_owned()
        .map_err(|_| KernelError::Busy)?;
    // UNION ALL keeps the two origins in one SQLite read snapshot without
    // sorting or materializing the result. Both origins join their owner,
    // so a fork's references to inherited rounds/results never bill twice.
    let path = tool_usage_path();
    let tool_predicate = tool_usage_predicate(&path);
    let mut rows = sqlx::QueryBuilder::new(format!(
        "SELECT s.id, s.title, s.relation_kind='subagent' AS is_subagent,
                u.created_at_ms, u.round, u.payload
         FROM (
           SELECT session_id, created_at_ms, 1 AS round,
                  json_object('version', provider_state_json -> '$.version',
                              'model', provider_state_json -> '$.model',
                              'usage', provider_state_json -> '$.usage') AS payload
           FROM agena_rounds WHERE {ROUND_USAGE_PREDICATE}
           UNION ALL
           SELECT r.session_id, t.created_at_ms, 0,
                  json_extract(t.result_json,{path})
           FROM agena_tool_calls t JOIN agena_rounds r ON r.id=t.round_id
           WHERE {tool_predicate}
         ) u JOIN agena_sessions s ON s.id=u.session_id
         WHERE s.deleted_at_ms IS NULL AND s.temporary=0",
    ));
    if let Some(from) = query.from {
        rows.push(" AND u.created_at_ms >= ")
            .push_bind(millisecond_boundary(from));
    }
    if let Some(to) = query.to {
        rows.push(" AND u.created_at_ms < ")
            .push_bind(millisecond_boundary(to));
    }
    if !query.include_subagents {
        rows.push(" AND s.relation_kind!='subagent'");
    }
    if !query.session_ids.is_empty() {
        rows.push(" AND s.id IN (SELECT value FROM json_each(")
            .push_bind(encode(&query.session_ids)?)
            .push("))");
    }
    let mut stream = rows
        .build()
        .fetch(backend.reader.get_sqlite_connection_pool());
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
