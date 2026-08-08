//! Fork/rewind branch materialization.
//!
//! A fork (or rewind) is stored as a view definition: a session row plus a
//! lineage row recording the parent cutoff. Opening the branch for the first
//! time materializes the view — deriving shared message memberships from the
//! parent's event log and physically copying only the in-flight tail.

use std::sync::Arc;

use sea_orm::{ConnectionTrait, EntityTrait};

use super::{
    AppError, EventKind, SessionStore, insert_session_message_memberships,
    rewrite_event_session_ids, visit_event_message_ids,
};
use crate::db::entities::session_lineage;
use agena_domain::RunAbortReason;

/// Split `items` into the shared prefix (completed executions) and the
/// in-flight tail that must be physically copied into a branch.
///
/// A run whose own RunCompleted/RunAborted is present in `items` is
/// completed: its terminal message rows are immutable and shared by
/// reference. The tail starts at the first event of an execution that is
/// still open at the cutoff.
pub(crate) fn split_fork_history(items: &[EventKind]) -> (&[EventKind], Vec<EventKind>) {
    let mut completed_executions = std::collections::HashSet::new();
    {
        let mut run_start_by_run = std::collections::HashMap::new();
        for item in items {
            match item {
                EventKind::RunStarted(payload) => {
                    run_start_by_run.insert(payload.run_id, payload.execution_id);
                }
                EventKind::RunCompleted(payload) => {
                    if let Some(execution_id) = run_start_by_run.get(&payload.run_id) {
                        completed_executions.insert(*execution_id);
                    }
                }
                EventKind::RunAborted(payload) => {
                    if let Some(execution_id) = run_start_by_run.get(&payload.run_id) {
                        completed_executions.insert(*execution_id);
                    }
                }
                _ => {}
            }
        }
    }
    let tail_start = items
        .iter()
        .position(|item| {
            let execution_id = match item {
                EventKind::ExecutionStarted(payload) => Some(payload.execution_id),
                EventKind::RunStarted(payload) => Some(payload.execution_id),
                EventKind::ExecutionFinished(payload) => Some(payload.execution_id),
                EventKind::UserMessageAppended(payload) => Some(payload.execution_id),
                EventKind::AssistantMessageFinished(payload) => Some(payload.execution_id),
                EventKind::CompactionCompleted(payload) => Some(payload.execution_id),
                EventKind::MessagePartCheckpointed(payload) => payload.execution_id,
                _ => None,
            };
            execution_id.is_some_and(|id| !completed_executions.contains(&id))
        })
        .unwrap_or(items.len());
    (&items[..tail_start], items[tail_start..].to_vec())
}

impl SessionStore {
    /// Materialize a fork/rewind branch's view on first open.
    ///
    /// Idempotent: the lineage marker (`view_materialized_seq_global`) is set
    /// once the shared memberships and (rare) in-flight tail are in place.
    /// Crash windows self-heal on the next open: a tail already appended but
    /// not yet marked is detected by the branch having its own events, so the
    /// append is skipped while memberships and the marker are re-derived.
    pub(crate) async fn materialize_fork_view(&self, session_id: i64) -> Result<(), AppError> {
        let Some((parent_id, cutoff_seq)) = self.history.fork_source_cutoff(session_id).await?
        else {
            return Ok(());
        };
        if let Some(lineage) = session_lineage::Entity::find_by_id(session_id)
            .one(&self.db)
            .await?
            && lineage.view_materialized_seq_global.is_some()
        {
            return Ok(());
        }

        // Serialize first opens inside this process so two concurrent loads
        // cannot both append the tail.
        let guard = {
            let mut locks = self.materialize_locks.lock().map_err(|_| {
                AppError::Internal("materialize coordinator lock was poisoned".to_string())
            })?;
            Arc::clone(
                locks
                    .entry(session_id)
                    .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(()))),
            )
        };
        let _guard = guard.lock().await;
        if let Some(lineage) = session_lineage::Entity::find_by_id(session_id)
            .one(&self.db)
            .await?
            && lineage.view_materialized_seq_global.is_some()
        {
            return Ok(());
        }

        let parent_events = self
            .history
            .list_session_events_before(parent_id, cutoff_seq, None)
            .await
            .map_err(|error| AppError::Internal(error.to_string()))?;
        let parent_kinds: Vec<EventKind> = parent_events
            .iter()
            .map(|event| event.kind.clone())
            .collect();

        // Tail: an in-flight execution at the cutoff is physically copied so
        // the branch keeps a snapshot immune to later parent streaming. A
        // completed prefix is never copied; membership edges make it visible.
        let own_events = self
            .history
            .list_session_events(session_id)
            .await
            .map_err(|error| AppError::Internal(error.to_string()))?;
        if own_events.is_empty() {
            let (_, mut tail_items) = split_fork_history(parent_kinds.as_slice());
            if !tail_items.is_empty() {
                self.remap_copied_history_ids(&mut tail_items).await?;
                for item in &mut tail_items {
                    rewrite_event_session_ids(item, session_id);
                }
                self.history
                    .append_items_silent(session_id, tail_items)
                    .await
                    .map_err(|error| AppError::Internal(error.to_string()))?;
            }
        }

        // Shared prefix: only the completed prefix is shared by reference.
        // In-flight messages were physically copied into the branch's own
        // log above, so referencing them again here would double them in the
        // projection. Same visitor as `rebuild_projection_from_history`, so
        // the kind coverage cannot drift between the two derivation paths.
        let (share_items, tail_items) = split_fork_history(parent_kinds.as_slice());
        // Message events of the in-flight tail can also appear in the share
        // part through part-checkpoint events whose execution id is not yet
        // assigned (the user message is persisted before its execution
        // starts). Those messages are physically copied into the branch
        // above, so referencing the parent's original row again here would
        // double them in the projection. Exclude every message id the tail
        // references from the shared membership derivation.
        let mut tail_message_ids = std::collections::HashSet::new();
        for item in &tail_items {
            visit_event_message_ids(item, |id| {
                if id > 0 {
                    tail_message_ids.insert(id);
                }
            });
        }
        let mut shared_ids = Vec::new();
        for item in share_items {
            visit_event_message_ids(item, |id| {
                if id > 0 && !tail_message_ids.contains(&id) && !shared_ids.contains(&id) {
                    shared_ids.push(id);
                }
            });
        }
        if !shared_ids.is_empty() {
            insert_session_message_memberships(&self.db, session_id, &shared_ids).await?;
        }

        // Close runs that were in flight at the cutoff (idempotent: a run
        // already closed by a previous attempt is not re-aborted).
        self.history
            .reconcile_unmatched_runs(session_id, RunAbortReason::ForkCutoff)
            .await
            .map_err(|error| AppError::Internal(error.to_string()))?;

        // Marker: the view is now complete. A crash between the tail/membership
        // writes and this update re-runs the idempotent steps on the next open.
        let stmt = sea_orm::Statement::from_sql_and_values(
            self.db.get_database_backend(),
            "UPDATE agena_session_lineage SET view_materialized_seq_global = ? WHERE session_id = ?",
            [cutoff_seq.into(), session_id.into()],
        );
        self.db
            .execute(stmt)
            .await
            .map_err(|error| AppError::Internal(error.to_string()))?;
        Ok(())
    }
}
