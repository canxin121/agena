//! Compact projections for session lists and workspace activity.
use std::collections::BTreeMap;

use super::*;

fn admit(backend: &Backend) -> Result<OwnedSemaphorePermit> {
    backend.cache.ensure_open()?;
    backend
        .control_reads
        .clone()
        .try_acquire_owned()
        .map_err(|_| KernelError::Busy)
}

fn view<T: serde::Serialize>(
    backend: &Backend,
    workspace: Option<i64>,
    value: T,
) -> Result<ReadView<T>> {
    let owner = workspace.map_or(ReadOwner::Shared, ReadOwner::Workspace);
    let lease = backend
        .views
        .reserve_owned(owner, modeled_read_bytes(&value)?)?;
    Ok(ReadView::from_lease(value, lease))
}

fn filter<'a>(
    builder: &mut sqlx::QueryBuilder<'a, sqlx::Sqlite>,
    query: &'a SummaryQuery,
    cursor: bool,
) {
    builder.push("s.deleted_at_ms IS NULL");
    if query.exclude_bot_owned {
        builder.push(" AND s.owner_bot_id IS NULL");
    }
    if let Some(bot) = &query.owner_bot_id {
        builder.push(" AND s.owner_bot_id=").push_bind(bot.clone());
    }
    if let Some(surface) = &query.owner_surface {
        builder
            .push(" AND s.owner_surface=")
            .push_bind(surface.clone());
    }
    if let Some(workspace) = query.workspace_id {
        builder.push(" AND s.workspace_id=").push_bind(workspace);
    } else if !query.include_global {
        builder.push(" AND s.workspace_id IS NOT NULL");
    }
    if let Some(parent) = query.parent_id {
        builder
            .push(" AND s.parent_session_id=")
            .push_bind(parent.get());
    }
    if query.agents_only {
        builder.push(" AND s.agent_name IS NOT NULL");
    }
    if query.roots_only {
        builder.push(" AND s.parent_session_id IS NULL");
    }
    if query.exclude_subagents {
        builder.push(" AND s.relation_kind!='subagent'");
    }
    builder.push(if query.temporary_only {
        " AND s.temporary=1"
    } else {
        " AND s.temporary=0"
    });
    if let Some(search) = &query.search {
        builder
            .push(" AND s.title LIKE ")
            .push_bind(format!(
                "%{}%",
                search
                    .replace('\\', "\\\\")
                    .replace('%', "\\%")
                    .replace('_', "\\_")
            ))
            .push(" ESCAPE '\\'");
    }
    if let Some(favorite) = query.favorite {
        builder.push(if favorite {
            " AND s.favorite=1"
        } else {
            " AND s.favorite=0"
        });
    }
    if let Some(pinned) = query.pinned {
        builder.push(if pinned {
            " AND s.pinned=1"
        } else {
            " AND s.pinned=0"
        });
    }
    if !query.states.is_empty() {
        builder.push(" AND t.processing_state IN (");
        let mut separated = builder.separated(",");
        for state in &query.states {
            separated.push_bind(state.as_str());
        }
        separated.push_unseparated(")");
    }
    if cursor && let Some(before) = query.before {
        builder
            .push(" AND (s.updated_at_ms,s.id)<(")
            .push_bind(before.updated_at_ms)
            .push(",")
            .push_bind(before.id.get())
            .push(")");
    }
}
async fn summary_page(
    connection: &mut SqliteConnection,
    query: &SummaryQuery,
) -> Result<SessionSummaryPage> {
    let limit = if query.limit == 0 {
        200
    } else {
        query.limit.min(200)
    };
    let mut builder = sqlx::QueryBuilder::new(format!(
        "SELECT {} FROM agena_sessions s",
        reads::SUMMARY_FIELDS
    ));
    // Broad queries walk the sessions in cursor order. Selective state filters
    // can start from the statistics index and sort just the matching sessions.
    let join = if query.states.is_empty()
        || query
            .states
            .contains(&agena_domain::SessionStateKind::Ready)
    {
        " CROSS JOIN"
    } else {
        " JOIN"
    };
    builder
        .push(join)
        .push(" agena_session_statistics t ON t.session_id=s.id WHERE ");
    filter(&mut builder, query, true);
    builder
        .push(" ORDER BY s.updated_at_ms DESC,s.id DESC LIMIT ")
        .push_bind((limit + 1) as i64);
    if query.offset != 0 {
        builder
            .push(" OFFSET ")
            .push_bind(i64::try_from(query.offset).map_err(invalid)?);
    }
    let rows = builder
        .build()
        .fetch_all(connection)
        .await
        .map_err(sql_error)?;
    let has_more = rows.len() > limit;
    let sessions = rows
        .iter()
        .take(limit)
        .map(reads::summary_row)
        .collect::<Result<Vec<_>>>()?;
    let next = if has_more {
        sessions.last().map(|last| SessionSummaryCursor {
            workspace_id: query.workspace_id,
            updated_at_ms: last.updated_at_ms,
            id: last.id,
        })
    } else {
        None
    };
    Ok(SessionSummaryPage {
        workspace_id: query.workspace_id,
        sessions,
        next,
        has_more,
        total: None,
    })
}

async fn summary_count(connection: &mut SqliteConnection, query: &SummaryQuery) -> Result<u64> {
    let mut count = sqlx::QueryBuilder::new("SELECT count(*) FROM agena_sessions s");
    // Every session has a statistics row created in the same write transaction.
    // Only state filters need that table to compute an exact count.
    if !query.states.is_empty() {
        count.push(" JOIN agena_session_statistics t ON t.session_id=s.id");
    }
    count.push(" WHERE ");
    filter(&mut count, query, false);
    let total: i64 = count
        .build_query_scalar()
        .fetch_one(connection)
        .await
        .map_err(sql_error)?;
    u64::try_from(total).map_err(storage)
}

pub(super) async fn query_summaries(
    backend: &Backend,
    query: SummaryQuery,
) -> Result<ReadView<SessionSummaryPage>> {
    let workspace = query.workspace_id;
    if workspace.is_some_and(|id| id <= 0) {
        return Err(invalid("invalid workspace"));
    }
    if query
        .before
        .is_some_and(|cursor| cursor.workspace_id != workspace)
    {
        return Err(KernelError::Ownership);
    }
    let _read = admit(backend)?;
    let mut connection = backend
        .reader
        .get_sqlite_connection_pool()
        .acquire()
        .await
        .map_err(storage)?;
    let value = match query.selection {
        SummarySelection::Page => summary_page(&mut connection, &query).await?,
        SummarySelection::Total => SessionSummaryPage {
            workspace_id: workspace,
            sessions: Vec::new(),
            next: None,
            has_more: false,
            total: Some(summary_count(&mut connection, &query).await?),
        },
        SummarySelection::PageWithTotal => {
            let mut tx = connection.begin().await.map_err(storage)?;
            let mut page = summary_page(&mut tx, &query).await?;
            page.total = Some(summary_count(&mut tx, &query).await?);
            tx.rollback().await.map_err(storage)?;
            page
        }
    };
    drop(connection);
    view(backend, workspace, value)
}

// UNION projects a favorite once even when it also belongs to a state bucket.
// The favorite branch uses its partial index even when SQLite has no stats
// and estimates that the broader workspace/owner index is more selective.
static OVERVIEW_SQL: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    let visibility = schema::overview_visibility("s");
    let scope = format!("s.workspace_id=?1 AND {visibility}");
    format!(
        "WITH included AS (
            SELECT s.id FROM agena_sessions s INDEXED BY idx_agena_sessions_favorite
            WHERE {scope} AND s.favorite=1
            UNION
            SELECT t.session_id FROM agena_session_statistics t
            WHERE t.overview_workspace_id=?1 AND t.processing_state!='ready'
            UNION
            SELECT id FROM (
                SELECT s.id FROM agena_sessions s
                CROSS JOIN agena_session_statistics t ON t.session_id=s.id
                WHERE {scope} AND t.processing_state='ready'
                ORDER BY s.updated_at_ms DESC,s.id DESC LIMIT ?2
            )
        )
        SELECT {} FROM included
        CROSS JOIN agena_sessions s ON s.id=included.id
        CROSS JOIN agena_session_statistics t ON t.session_id=s.id
        ORDER BY s.updated_at_ms DESC,s.id DESC",
        reads::SUMMARY_FIELDS
    )
});

pub(super) async fn overview_summaries(
    backend: &Backend,
    workspace: i64,
    recent_limit: usize,
) -> Result<ReadView<Vec<SessionSummaryView>>> {
    if workspace <= 0 {
        return Err(invalid("invalid workspace"));
    }
    let _read = admit(backend)?;
    let rows = sqlx::query(&OVERVIEW_SQL)
        .bind(workspace)
        .bind(recent_limit.clamp(1, 200) as i64)
        .fetch_all(backend.reader.get_sqlite_connection_pool())
        .await
        .map_err(sql_error)?;
    let summaries = rows
        .iter()
        .map(reads::summary_row)
        .collect::<Result<Vec<_>>>()?;
    view(backend, Some(workspace), summaries)
}

pub(super) async fn activity_summary(
    backend: &Backend,
) -> Result<ReadView<SessionActivitySummary>> {
    let _read = admit(backend)?;
    let mut tx = backend
        .reader
        .get_sqlite_connection_pool()
        .begin()
        .await
        .map_err(storage)?;
    // Match the default session-list visibility with exclude_subagents=true.
    // Ordinary child/fork sessions remain user-visible and participate.
    let counts = sqlx::query(
        "SELECT COALESCE(sum(t.processing_state='running'),0) AS running,
                COALESCE(sum(t.processing_state='awaiting_interaction'),0) AS awaiting
         FROM agena_session_statistics t CROSS JOIN agena_sessions s ON s.id=t.session_id
         WHERE t.processing_state IN ('running','awaiting_interaction')
           AND s.deleted_at_ms IS NULL AND s.temporary=0 AND s.relation_kind!='subagent'",
    )
    .fetch_one(&mut *tx)
    .await
    .map_err(sql_error)?;
    // The time belongs to the attempt, not mutable session metadata. At equal
    // millisecond precision a failure wins the tie; no arbitrary identity is
    // interpreted as a later completion.
    let terminal = sqlx::query(
        "SELECT a.finished_at_ms,(a.phase='failed') AS failed
         FROM agena_attempts a
         CROSS JOIN agena_executions e ON e.id=a.execution_id AND e.current_generation=a.generation
         CROSS JOIN agena_sessions s ON s.id=a.session_id
         WHERE a.finished_at_ms IS NOT NULL AND s.deleted_at_ms IS NULL
           AND s.temporary=0 AND s.relation_kind!='subagent'
         ORDER BY a.finished_at_ms DESC,(a.phase='failed') DESC LIMIT 1",
    )
    .fetch_optional(&mut *tx)
    .await
    .map_err(sql_error)?;
    let value = SessionActivitySummary {
        running_sessions: u64::try_from(counts.try_get::<i64, _>("running").map_err(storage)?)
            .map_err(storage)?,
        awaiting_interaction_sessions: u64::try_from(
            counts.try_get::<i64, _>("awaiting").map_err(storage)?,
        )
        .map_err(storage)?,
        latest_terminal_at_ms: terminal
            .as_ref()
            .map(|row| row.try_get::<i64, _>("finished_at_ms").map_err(storage))
            .transpose()?,
        latest_terminal_failed: terminal
            .as_ref()
            .map(|row| row.try_get::<bool, _>("failed").map_err(storage))
            .transpose()?
            .unwrap_or(false),
    };
    tx.rollback().await.map_err(storage)?;
    view(backend, None, value)
}

pub(super) async fn workspace_statistics(
    backend: &Backend,
    ids: &[i64],
) -> Result<ReadView<BTreeMap<i64, WorkspaceStatistics>>> {
    if ids.len() > 256 || ids.iter().any(|id| *id <= 0) {
        return Err(invalid(
            "workspace statistics admits at most 256 valid workspace ids",
        ));
    }
    let _read = admit(backend)?;
    let rows = sqlx::query(
        "SELECT workspace_id,total,roots,pinned,running,attention
         FROM agena_workspace_statistics
         WHERE workspace_id IN (SELECT value FROM json_each(?))",
    )
    .bind(encode(ids)?)
    .fetch_all(backend.reader.get_sqlite_connection_pool())
    .await
    .map_err(sql_error)?;
    let mut value = ids
        .iter()
        .map(|id| (*id, WorkspaceStatistics::default()))
        .collect::<BTreeMap<_, _>>();
    for row in rows {
        let count = |name| {
            row.try_get::<i64, _>(name)
                .map(|v| v as u64)
                .map_err(storage)
        };
        value.insert(
            row.try_get("workspace_id").map_err(storage)?,
            WorkspaceStatistics {
                total: count("total")?,
                roots: count("roots")?,
                pinned: count("pinned")?,
                running: count("running")?,
                attention: count("attention")?,
            },
        );
    }
    view(backend, None, value)
}
