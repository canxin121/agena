//! SQLite invariant-trigger declarations for the shared v2 Agena schema.
//!
//! These triggers enforce the parts-first invariants from the v2 design at the
//! database layer, so no caller can bypass the facade and corrupt state:
//!
//! * parts identity is immutable; lifecycle follows the v2 state machine
//!   (including retry `failed`/`cancelled` → `in_progress` with a revision bump);
//! * run markers are the root of their batch (`run_id`/`parent_part_id` NULL),
//!   carry a `run_kind`, and must record an `abort_reason` on terminal states;
//! * `run_id` and `parent_part_id` only reference real parts (runs, resp. parts);
//! * `session_parts` edges only reference real sessions and parts;
//! * sessions keep their hierarchy, lifecycle, and version invariants;
//! * subagent sessions carry the delegated-task lifecycle;
//! * leases, usage, and idempotency rows are shape-valid and reference real
//!   sessions/runs.

use sea_orm::{ConnectionTrait, DbErr, Statement};

pub async fn install_invariant_triggers<C>(db: &C) -> Result<(), DbErr>
where
    C: ConnectionTrait,
{
    let backend = db.get_database_backend();
    for sql in [
        // --- parts identity ---
        "CREATE TRIGGER IF NOT EXISTS agena_parts_identity_immutable \
         BEFORE UPDATE OF part_id, kind, role, origin_session_id, created_at_ms ON agena_parts \
         WHEN OLD.part_id != NEW.part_id OR OLD.kind != NEW.kind OR OLD.role != NEW.role \
           OR OLD.origin_session_id IS NOT NEW.origin_session_id \
           OR OLD.created_at_ms != NEW.created_at_ms \
         BEGIN SELECT RAISE(ABORT, 'part identity is immutable'); END",
        // --- parts state and visibility are drawn from the closed enumerations ---
        "CREATE TRIGGER IF NOT EXISTS agena_parts_state_valid_insert \
         BEFORE INSERT ON agena_parts \
         WHEN NEW.state NOT IN ('pending', 'in_progress', 'completed', 'failed', 'cancelled') \
           OR NEW.visibility NOT IN ('both', 'user', 'ai') \
         BEGIN SELECT RAISE(ABORT, 'invalid part state or visibility'); END",
        "CREATE TRIGGER IF NOT EXISTS agena_parts_state_valid_update \
         BEFORE UPDATE OF state, visibility ON agena_parts \
         WHEN NEW.state NOT IN ('pending', 'in_progress', 'completed', 'failed', 'cancelled') \
           OR NEW.visibility NOT IN ('both', 'user', 'ai') \
         BEGIN SELECT RAISE(ABORT, 'invalid part state or visibility'); END",
        // --- parts lifecycle shape (terminal states need a finished timestamp) ---
        // The table CHECK enforces the finished_at_ms rules on every write; this
        // trigger covers the timestamp sanity that a CHECK cannot express.
        "CREATE TRIGGER IF NOT EXISTS agena_parts_lifecycle_shape_valid \
         BEFORE INSERT ON agena_parts \
         WHEN NEW.started_at_ms < 0 OR NEW.created_at_ms < 0 \
           OR NEW.updated_at_ms < NEW.created_at_ms \
         BEGIN SELECT RAISE(ABORT, 'invalid part lifecycle shape'); END",
        "CREATE TRIGGER IF NOT EXISTS agena_parts_lifecycle_shape_update_valid \
         BEFORE UPDATE OF updated_at_ms ON agena_parts \
         WHEN NEW.updated_at_ms < OLD.updated_at_ms \
         BEGIN SELECT RAISE(ABORT, 'part updated_at cannot move backwards'); END",
        // --- parts retry: failed/cancelled → in_progress requires a revision bump ---
        "CREATE TRIGGER IF NOT EXISTS agena_parts_retry_requires_revision_bump \
         BEFORE UPDATE OF state ON agena_parts \
         WHEN NEW.state = 'in_progress' AND OLD.state IN ('failed', 'cancelled') \
           AND NEW.revision <= OLD.revision \
         BEGIN SELECT RAISE(ABORT, 'retrying a part must bump its revision'); END",
        "CREATE TRIGGER IF NOT EXISTS agena_parts_revision_monotonic \
         BEFORE UPDATE OF revision ON agena_parts \
         WHEN NEW.revision < OLD.revision \
         BEGIN SELECT RAISE(ABORT, 'part revision cannot decrease'); END",
        // --- parts content must be a JSON document ---
        "CREATE TRIGGER IF NOT EXISTS agena_parts_content_is_json \
         BEFORE INSERT ON agena_parts \
         WHEN json_valid(NEW.content) != 1 \
         BEGIN SELECT RAISE(ABORT, 'part content must be a JSON document'); END",
        // --- run markers are the root of their batch ---
        "CREATE TRIGGER IF NOT EXISTS agena_parts_run_marker_is_batch_root \
         BEFORE INSERT ON agena_parts \
         WHEN NEW.kind = 'run' AND (NEW.run_id IS NOT NULL OR NEW.parent_part_id IS NOT NULL) \
         BEGIN SELECT RAISE(ABORT, 'run marker parts must be the root of their batch'); END",
        "CREATE TRIGGER IF NOT EXISTS agena_parts_run_marker_root_immutable \
         BEFORE UPDATE OF run_id, parent_part_id ON agena_parts \
         WHEN OLD.kind = 'run' AND (NEW.run_id IS NOT NULL OR NEW.parent_part_id IS NOT NULL) \
         BEGIN SELECT RAISE(ABORT, 'run marker parts must stay the root of their batch'); END",
        "CREATE TRIGGER IF NOT EXISTS agena_parts_run_marker_requires_run_kind \
         BEFORE INSERT ON agena_parts \
         WHEN NEW.kind = 'run' AND json_type(NEW.content, '$.run_kind') IS NULL \
         BEGIN SELECT RAISE(ABORT, 'run marker part requires run_kind in content'); END",
        // --- a terminal run marker must record its abort reason (present, may
        // be JSON null for a normal completion) ---
        "CREATE TRIGGER IF NOT EXISTS agena_parts_run_marker_terminal_abort_reason \
         BEFORE INSERT ON agena_parts \
         WHEN NEW.kind = 'run' AND NEW.state IN ('completed', 'failed', 'cancelled') \
           AND json_type(NEW.content, '$.abort_reason') IS NULL \
         BEGIN SELECT RAISE(ABORT, 'terminal run marker requires abort_reason'); END",
        "CREATE TRIGGER IF NOT EXISTS agena_parts_run_marker_terminal_abort_reason_update \
         BEFORE UPDATE OF state ON agena_parts \
         WHEN NEW.kind = 'run' AND NEW.state IN ('completed', 'failed', 'cancelled') \
           AND json_type(NEW.content, '$.abort_reason') IS NULL \
         BEGIN SELECT RAISE(ABORT, 'terminal run marker requires abort_reason'); END",
        // --- part references (run_id → run marker, parent_part_id → part) ---
        "CREATE TRIGGER IF NOT EXISTS agena_parts_run_id_references_run_marker \
         BEFORE INSERT ON agena_parts \
         WHEN NEW.run_id IS NOT NULL AND NOT EXISTS ( \
             SELECT 1 FROM agena_parts run \
             WHERE run.part_id = NEW.run_id AND run.kind = 'run' \
         ) \
         BEGIN SELECT RAISE(ABORT, 'part run_id must reference a run marker part'); END",
        "CREATE TRIGGER IF NOT EXISTS agena_parts_run_id_references_run_marker_update \
         BEFORE UPDATE OF run_id ON agena_parts \
         WHEN NEW.run_id IS NOT NULL AND NOT EXISTS ( \
             SELECT 1 FROM agena_parts run \
             WHERE run.part_id = NEW.run_id AND run.kind = 'run' \
         ) \
         BEGIN SELECT RAISE(ABORT, 'part run_id must reference a run marker part'); END",
        "CREATE TRIGGER IF NOT EXISTS agena_parts_parent_references_part \
         BEFORE INSERT ON agena_parts \
         WHEN NEW.parent_part_id IS NOT NULL AND ( \
             NEW.parent_part_id = NEW.part_id \
             OR NOT EXISTS (SELECT 1 FROM agena_parts parent WHERE parent.part_id = NEW.parent_part_id) \
         ) \
         BEGIN SELECT RAISE(ABORT, 'part parent_part_id must reference an existing part'); END",
        "CREATE TRIGGER IF NOT EXISTS agena_parts_parent_references_part_update \
         BEFORE UPDATE OF parent_part_id ON agena_parts \
         WHEN NEW.parent_part_id IS NOT NULL AND ( \
             NEW.parent_part_id = NEW.part_id \
             OR NOT EXISTS (SELECT 1 FROM agena_parts parent WHERE parent.part_id = NEW.parent_part_id) \
         ) \
         BEGIN SELECT RAISE(ABORT, 'part parent_part_id must reference an existing part'); END",
        // --- session_parts edges reference real sessions and parts ---
        "CREATE TRIGGER IF NOT EXISTS agena_session_parts_references_valid \
         BEFORE INSERT ON agena_session_parts \
         WHEN NEW.added_at_ms < 0 \
           OR NOT EXISTS (SELECT 1 FROM agena_sessions WHERE id = NEW.session_id) \
           OR NOT EXISTS (SELECT 1 FROM agena_parts WHERE part_id = NEW.part_id) \
         BEGIN SELECT RAISE(ABORT, 'session_part must reference an existing session and part'); END",
        // --- sessions hierarchy ---
        "CREATE TRIGGER IF NOT EXISTS agena_sessions_hierarchy_insert_valid \
         BEFORE INSERT ON agena_sessions \
         WHEN (NEW.parent_id IS NULL AND (NEW.depth != 0 OR NEW.root_id != 0)) \
           OR (NEW.parent_id IS NOT NULL AND NOT EXISTS ( \
             SELECT 1 FROM agena_sessions parent \
             WHERE parent.id = NEW.parent_id \
               AND parent.workspace_id = NEW.workspace_id \
               AND parent.lifecycle_state = 'ready' \
               AND NEW.depth = parent.depth + 1 \
               AND NEW.root_id = parent.root_id \
           )) \
           OR NEW.relation_kind NOT IN ('root', 'child', 'fork', 'rewind', 'subagent') \
           OR (NEW.parent_id IS NULL AND NEW.relation_kind != 'root') \
           OR (NEW.parent_id IS NOT NULL AND NEW.relation_kind = 'root') \
         BEGIN SELECT RAISE(ABORT, 'invalid session hierarchy'); END",
        "CREATE TRIGGER IF NOT EXISTS agena_sessions_hierarchy_immutable \
         BEFORE UPDATE OF parent_id, root_id, depth ON agena_sessions \
         WHEN (OLD.parent_id IS NOT NEW.parent_id OR OLD.root_id != NEW.root_id OR OLD.depth != NEW.depth) \
           AND NOT (OLD.parent_id IS NULL AND NEW.parent_id IS NULL AND OLD.root_id = 0 AND NEW.root_id = NEW.id AND OLD.depth = 0 AND NEW.depth = 0) \
         BEGIN SELECT RAISE(ABORT, 'session hierarchy is immutable'); END",
        "CREATE TRIGGER IF NOT EXISTS agena_sessions_root_finalize \
         AFTER INSERT ON agena_sessions \
         WHEN NEW.parent_id IS NULL AND NEW.root_id = 0 \
         BEGIN UPDATE agena_sessions SET root_id = NEW.id WHERE id = NEW.id; END",
        // --- sessions lifecycle (creating → ready | failed, failure only when failed) ---
        "CREATE TRIGGER IF NOT EXISTS agena_sessions_lifecycle_insert_valid \
         BEFORE INSERT ON agena_sessions \
         WHEN NEW.lifecycle_state NOT IN ('creating', 'ready') \
           OR NEW.creation_failure_json IS NOT NULL \
         BEGIN SELECT RAISE(ABORT, 'invalid session lifecycle state'); END",
        "CREATE TRIGGER IF NOT EXISTS agena_sessions_lifecycle_update_valid \
         BEFORE UPDATE OF lifecycle_state ON agena_sessions \
         WHEN NEW.lifecycle_state NOT IN ('creating', 'ready', 'failed') \
         BEGIN SELECT RAISE(ABORT, 'invalid session lifecycle state'); END",
        "CREATE TRIGGER IF NOT EXISTS agena_sessions_lifecycle_transition_valid \
         BEFORE UPDATE OF lifecycle_state ON agena_sessions \
         WHEN OLD.lifecycle_state != NEW.lifecycle_state \
           AND NOT (OLD.lifecycle_state = 'creating' AND NEW.lifecycle_state IN ('ready', 'failed')) \
         BEGIN SELECT RAISE(ABORT, 'invalid session lifecycle transition'); END",
        "CREATE TRIGGER IF NOT EXISTS agena_sessions_creation_failure_shape \
         BEFORE UPDATE OF lifecycle_state, creation_failure_json ON agena_sessions \
         WHEN (NEW.lifecycle_state = 'failed' AND (NEW.creation_failure_json IS NULL OR length(trim(NEW.creation_failure_json)) = 0 OR CASE WHEN json_valid(NEW.creation_failure_json) = 1 THEN (json_type(NEW.creation_failure_json, '$.id') != 'text' OR json_type(NEW.creation_failure_json, '$.code') != 'text' OR json_type(NEW.creation_failure_json, '$.user.fallback') != 'text') ELSE 1 END)) \
           OR (NEW.lifecycle_state != 'failed' AND NEW.creation_failure_json IS NOT NULL) \
         BEGIN SELECT RAISE(ABORT, 'creation failure must describe only failed sessions'); END",
        // --- sessions version is a monotonic optimistic-lock counter ---
        "CREATE TRIGGER IF NOT EXISTS agena_sessions_version_monotonic \
         BEFORE UPDATE OF version ON agena_sessions \
         WHEN NEW.version <= OLD.version \
         BEGIN SELECT RAISE(ABORT, 'session version must increase'); END",
        // --- fork/rewind branches point at a real cutoff part; other kinds do not ---
        "CREATE TRIGGER IF NOT EXISTS agena_sessions_cutoff_references_part \
         BEFORE INSERT ON agena_sessions \
         WHEN NEW.cutoff_part_id IS NOT NULL AND NOT EXISTS ( \
             SELECT 1 FROM agena_parts WHERE part_id = NEW.cutoff_part_id \
         ) \
         BEGIN SELECT RAISE(ABORT, 'session cutoff_part_id must reference an existing part'); END",
        "CREATE TRIGGER IF NOT EXISTS agena_sessions_cutoff_required_for_branches \
         BEFORE INSERT ON agena_sessions \
         WHEN (NEW.relation_kind IN ('fork', 'rewind') AND NEW.cutoff_part_id IS NULL) \
           OR (NEW.relation_kind NOT IN ('fork', 'rewind') AND NEW.cutoff_part_id IS NOT NULL) \
         BEGIN SELECT RAISE(ABORT, 'fork/rewind sessions require a cutoff part'); END",
        // --- subagent sessions carry the delegated-task lifecycle ---
        "CREATE TRIGGER IF NOT EXISTS agena_sessions_subagent_shape \
         BEFORE INSERT ON agena_sessions \
         WHEN (NEW.relation_kind = 'subagent' AND (NEW.task_id IS NULL OR NEW.subtask_status IS NULL)) \
           OR (NEW.relation_kind = 'subagent' AND (length(trim(NEW.task_id)) = 0 OR NEW.subtask_status NOT IN ('created', 'running', 'completed', 'failed', 'cancelled', 'timed_out', 'interrupted'))) \
           OR (NEW.relation_kind != 'subagent' AND (NEW.task_id IS NOT NULL OR NEW.subtask_status IS NOT NULL OR NEW.subtask_started_at_ms IS NOT NULL OR NEW.subtask_finished_at_ms IS NOT NULL OR NEW.subtask_failure_json IS NOT NULL)) \
         BEGIN SELECT RAISE(ABORT, 'invalid subagent session shape'); END",
        "CREATE TRIGGER IF NOT EXISTS agena_sessions_subtask_lifecycle \
         BEFORE UPDATE OF subtask_status, subtask_started_at_ms, subtask_finished_at_ms, subtask_failure_json ON agena_sessions \
         WHEN NEW.relation_kind != 'subagent' \
           OR NEW.subtask_status NOT IN ('created', 'running', 'completed', 'failed', 'cancelled', 'timed_out', 'interrupted') \
           OR (NEW.subtask_status = 'created' AND (NEW.subtask_started_at_ms IS NOT NULL OR NEW.subtask_finished_at_ms IS NOT NULL OR NEW.subtask_failure_json IS NOT NULL)) \
           OR (NEW.subtask_status = 'running' AND (NEW.subtask_started_at_ms IS NULL OR NEW.subtask_finished_at_ms IS NOT NULL OR NEW.subtask_failure_json IS NOT NULL)) \
           OR (NEW.subtask_status = 'completed' AND (NEW.subtask_started_at_ms IS NULL OR NEW.subtask_finished_at_ms IS NULL OR NEW.subtask_failure_json IS NOT NULL)) \
           OR (NEW.subtask_status = 'cancelled' AND (NEW.subtask_started_at_ms IS NULL OR NEW.subtask_finished_at_ms IS NULL OR NEW.subtask_failure_json IS NOT NULL)) \
           OR (NEW.subtask_status IN ('failed', 'timed_out', 'interrupted') AND (NEW.subtask_started_at_ms IS NULL OR NEW.subtask_finished_at_ms IS NULL OR NEW.subtask_failure_json IS NULL OR length(trim(NEW.subtask_failure_json)) = 0 OR CASE WHEN json_valid(NEW.subtask_failure_json) = 1 THEN (json_type(NEW.subtask_failure_json, '$.id') != 'text' OR json_type(NEW.subtask_failure_json, '$.code') != 'text' OR json_type(NEW.subtask_failure_json, '$.user.fallback') != 'text') ELSE 1 END)) \
           OR (NEW.subtask_started_at_ms IS NOT NULL AND NEW.subtask_started_at_ms < 0) \
           OR (NEW.subtask_finished_at_ms IS NOT NULL AND NEW.subtask_finished_at_ms < NEW.subtask_started_at_ms) \
         BEGIN SELECT RAISE(ABORT, 'invalid delegated-task lifecycle'); END",
        // --- execution leases reference a real run and are shape-valid ---
        "CREATE TRIGGER IF NOT EXISTS agena_execution_leases_shape_valid \
         BEFORE INSERT ON agena_execution_leases \
         WHEN NEW.lease_started_at_ms < 0 OR NEW.heartbeat_at_ms < NEW.lease_started_at_ms \
           OR length(trim(NEW.owner_id)) = 0 \
           OR (NEW.run_id IS NOT NULL AND NOT EXISTS ( \
             SELECT 1 FROM agena_parts WHERE part_id = NEW.run_id AND kind = 'run' \
           )) \
         BEGIN SELECT RAISE(ABORT, 'invalid execution lease'); END",
        "CREATE TRIGGER IF NOT EXISTS agena_execution_leases_run_reference_update \
         BEFORE UPDATE OF run_id ON agena_execution_leases \
         WHEN NEW.run_id IS NOT NULL AND NOT EXISTS ( \
             SELECT 1 FROM agena_parts WHERE part_id = NEW.run_id AND kind = 'run' \
         ) \
         BEGIN SELECT RAISE(ABORT, 'execution lease run_id must reference a run marker'); END",
        // --- usage rows reference a real session (and workspace) plus an
        // optional run; token/cost scalars are normalized and non-negative ---
        "CREATE TRIGGER IF NOT EXISTS agena_usage_shape_valid \
         BEFORE INSERT ON agena_usage \
         WHEN NEW.input_tokens < 0 OR NEW.output_tokens < 0 OR NEW.reasoning_tokens < 0 \
           OR NEW.cache_write_tokens < 0 OR NEW.cache_read_tokens < 0 OR NEW.tool_use_tokens < 0 \
           OR NEW.other_tokens < 0 OR NEW.total_cost_micros < 0 OR NEW.created_at_ms < 0 \
           OR NOT EXISTS ( \
             SELECT 1 FROM agena_sessions \
             WHERE id = NEW.session_id AND workspace_id = NEW.workspace_id \
           ) \
           OR (NEW.run_id IS NOT NULL AND NOT EXISTS ( \
             SELECT 1 FROM agena_parts WHERE part_id = NEW.run_id AND kind = 'run' \
           )) \
         BEGIN SELECT RAISE(ABORT, 'invalid usage record'); END",
        // --- idempotency rows reference a real session and run ---
        "CREATE TRIGGER IF NOT EXISTS agena_idempotency_shape_valid \
         BEFORE INSERT ON agena_idempotency \
         WHEN length(trim(NEW.idempotency_key)) = 0 OR NEW.created_at_ms < 0 \
           OR NOT EXISTS (SELECT 1 FROM agena_sessions WHERE id = NEW.session_id) \
           OR (NEW.run_id IS NOT NULL AND NOT EXISTS ( \
             SELECT 1 FROM agena_parts WHERE part_id = NEW.run_id AND kind = 'run' \
           )) \
         BEGIN SELECT RAISE(ABORT, 'invalid idempotency record'); END",
    ] {
        db.execute(Statement::from_string(backend, sql.to_owned()))
            .await?;
    }
    Ok(())
}
