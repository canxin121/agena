//! SQLite invariant-trigger declarations for the shared Agena schema.

use sea_orm::{ConnectionTrait, DbErr, Statement};

pub async fn install_invariant_triggers<C>(db: &C) -> Result<(), DbErr>
where
    C: ConnectionTrait,
{
    let backend = db.get_database_backend();
    for sql in [
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
        "CREATE TRIGGER IF NOT EXISTS agena_sessions_lifecycle_insert_valid \
         BEFORE INSERT ON agena_sessions \
         WHEN NEW.lifecycle_state NOT IN ('creating', 'ready') \
           OR NEW.creation_error IS NOT NULL \
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
        "CREATE TRIGGER IF NOT EXISTS agena_sessions_creation_error_shape \
         BEFORE UPDATE OF lifecycle_state, creation_error ON agena_sessions \
         WHEN (NEW.lifecycle_state = 'failed' AND (NEW.creation_error IS NULL OR length(trim(NEW.creation_error)) = 0)) \
           OR (NEW.lifecycle_state != 'failed' AND NEW.creation_error IS NOT NULL) \
         BEGIN SELECT RAISE(ABORT, 'creation error must describe only failed sessions'); END",
        "CREATE TRIGGER IF NOT EXISTS agena_session_lineage_shape_insert_valid \
         BEFORE INSERT ON agena_session_lineage \
         WHEN NEW.relation_kind NOT IN ('child', 'fork', 'rewind', 'subagent') \
           OR NOT EXISTS (SELECT 1 FROM agena_sessions s WHERE s.id = NEW.session_id AND s.parent_id IS NOT NULL) \
           OR (NEW.relation_kind = 'subagent' AND (NEW.task_id IS NULL OR NEW.subtask_status IS NULL)) \
           OR (NEW.relation_kind = 'subagent' AND (length(trim(NEW.task_id)) = 0 OR NEW.subtask_status NOT IN ('created', 'running', 'completed', 'failed', 'cancelled', 'timed_out', 'interrupted'))) \
           OR (NEW.relation_kind = 'subagent' AND ( \
             (NEW.subtask_status = 'created' AND (NEW.subtask_started_at_ms IS NOT NULL OR NEW.subtask_finished_at_ms IS NOT NULL OR NEW.subtask_error IS NOT NULL)) \
             OR (NEW.subtask_status = 'running' AND (NEW.subtask_started_at_ms IS NULL OR NEW.subtask_finished_at_ms IS NOT NULL OR NEW.subtask_error IS NOT NULL)) \
             OR (NEW.subtask_status = 'completed' AND (NEW.subtask_started_at_ms IS NULL OR NEW.subtask_finished_at_ms IS NULL OR NEW.subtask_error IS NOT NULL)) \
             OR (NEW.subtask_status IN ('failed', 'cancelled', 'timed_out', 'interrupted') AND (NEW.subtask_started_at_ms IS NULL OR NEW.subtask_finished_at_ms IS NULL OR NEW.subtask_error IS NULL OR length(trim(NEW.subtask_error)) = 0)) \
             OR (NEW.subtask_started_at_ms IS NOT NULL AND NEW.subtask_started_at_ms < 0) \
             OR (NEW.subtask_finished_at_ms IS NOT NULL AND NEW.subtask_finished_at_ms < NEW.subtask_started_at_ms) \
           )) \
           OR (NEW.relation_kind != 'subagent' AND (NEW.task_id IS NOT NULL OR NEW.subtask_status IS NOT NULL OR NEW.subtask_started_at_ms IS NOT NULL OR NEW.subtask_finished_at_ms IS NOT NULL OR NEW.subtask_error IS NOT NULL)) \
           OR (NEW.relation_kind IN ('fork', 'rewind') AND NEW.source_cutoff_seq_global IS NULL) \
           OR NEW.source_cutoff_seq_global < 0 \
           OR NEW.source_message_id <= 0 \
           OR (NEW.relation_kind NOT IN ('fork', 'rewind') AND (NEW.source_cutoff_seq_global IS NOT NULL OR NEW.source_message_id IS NOT NULL)) \
           OR (NEW.relation_kind = 'rewind' AND NEW.source_message_id IS NULL) \
         BEGIN SELECT RAISE(ABORT, 'invalid session lineage'); END",
        "CREATE TRIGGER IF NOT EXISTS agena_session_lineage_provenance_immutable \
         BEFORE UPDATE OF relation_kind, source_cutoff_seq_global, source_message_id, task_id ON agena_session_lineage \
         WHEN OLD.relation_kind IS NOT NEW.relation_kind \
           OR OLD.source_cutoff_seq_global IS NOT NEW.source_cutoff_seq_global \
           OR OLD.source_message_id IS NOT NEW.source_message_id \
           OR OLD.task_id IS NOT NEW.task_id \
         BEGIN SELECT RAISE(ABORT, 'session lineage provenance is immutable'); END",
        "CREATE TRIGGER IF NOT EXISTS agena_session_lineage_subtask_status_valid \
         BEFORE UPDATE OF subtask_status, subtask_started_at_ms, subtask_finished_at_ms, subtask_error ON agena_session_lineage \
         WHEN NEW.relation_kind != 'subagent' \
           OR NEW.subtask_status NOT IN ('created', 'running', 'completed', 'failed', 'cancelled', 'timed_out', 'interrupted') \
           OR (NEW.subtask_status = 'created' AND (NEW.subtask_started_at_ms IS NOT NULL OR NEW.subtask_finished_at_ms IS NOT NULL OR NEW.subtask_error IS NOT NULL)) \
           OR (NEW.subtask_status = 'running' AND (NEW.subtask_started_at_ms IS NULL OR NEW.subtask_finished_at_ms IS NOT NULL OR NEW.subtask_error IS NOT NULL)) \
           OR (NEW.subtask_status = 'completed' AND (NEW.subtask_started_at_ms IS NULL OR NEW.subtask_finished_at_ms IS NULL OR NEW.subtask_error IS NOT NULL)) \
           OR (NEW.subtask_status IN ('failed', 'cancelled', 'timed_out', 'interrupted') AND (NEW.subtask_started_at_ms IS NULL OR NEW.subtask_finished_at_ms IS NULL OR NEW.subtask_error IS NULL OR length(trim(NEW.subtask_error)) = 0)) \
           OR (NEW.subtask_started_at_ms IS NOT NULL AND NEW.subtask_started_at_ms < 0) \
           OR (NEW.subtask_finished_at_ms IS NOT NULL AND NEW.subtask_finished_at_ms < NEW.subtask_started_at_ms) \
         BEGIN SELECT RAISE(ABORT, 'invalid delegated-task lifecycle'); END",
        "CREATE TRIGGER IF NOT EXISTS agena_session_lineage_task_unique \
         BEFORE INSERT ON agena_session_lineage \
         WHEN NEW.task_id IS NOT NULL AND EXISTS ( \
             SELECT 1 FROM agena_session_lineage existing \
             JOIN agena_sessions existing_session ON existing_session.id = existing.session_id \
             JOIN agena_sessions new_session ON new_session.id = NEW.session_id \
             WHERE existing.task_id = NEW.task_id \
               AND existing_session.parent_id = new_session.parent_id \
         ) \
         BEGIN SELECT RAISE(ABORT, 'delegated task identity already exists for parent'); END",
        "CREATE TRIGGER IF NOT EXISTS agena_transcript_messages_identity_immutable \
         BEFORE UPDATE OF message_id, session_id, turn_id, role, created_at_ms ON agena_transcript_messages \
         WHEN OLD.message_id != NEW.message_id \
           OR OLD.session_id != NEW.session_id \
           OR OLD.turn_id IS NOT NEW.turn_id \
           OR OLD.role != NEW.role \
           OR OLD.created_at_ms != NEW.created_at_ms \
         BEGIN SELECT RAISE(ABORT, 'message identity and ownership are immutable'); END",
        "CREATE TRIGGER IF NOT EXISTS agena_transcript_parts_identity_immutable \
         BEFORE UPDATE OF part_id, message_id, part_index, kind, activity_id, segment_id, operation_id, created_at_ms ON agena_transcript_parts \
         WHEN OLD.part_id != NEW.part_id \
           OR OLD.message_id != NEW.message_id \
           OR OLD.part_index != NEW.part_index \
           OR OLD.kind != NEW.kind \
           OR OLD.activity_id IS NOT NEW.activity_id \
           OR OLD.segment_id IS NOT NEW.segment_id \
           OR OLD.operation_id IS NOT NEW.operation_id \
           OR OLD.created_at_ms != NEW.created_at_ms \
         BEGIN SELECT RAISE(ABORT, 'part identity and ownership are immutable'); END",
        "CREATE TRIGGER IF NOT EXISTS agena_activities_owner_insert_valid \
         BEFORE INSERT ON agena_activities \
         WHEN NEW.position < 0 OR NEW.revision_seq < 0 \
           OR NOT ( \
             (NEW.owner_kind = 'turn_input' AND EXISTS (SELECT 1 FROM agena_turns WHERE turn_id = NEW.owner_id)) \
             OR (NEW.owner_kind = 'response' AND EXISTS (SELECT 1 FROM agena_responses WHERE response_id = NEW.owner_id)) \
             OR (NEW.owner_kind = 'activity' AND EXISTS (SELECT 1 FROM agena_activities WHERE activity_id = NEW.owner_id)) \
             OR (NEW.owner_kind = 'session' AND EXISTS (SELECT 1 FROM agena_sessions WHERE CAST(id AS TEXT) = NEW.owner_id)) \
           ) \
           OR EXISTS (SELECT 1 FROM agena_text_segments text WHERE text.owner_kind = NEW.owner_kind AND text.owner_id = NEW.owner_id AND text.position = NEW.position) \
         BEGIN SELECT RAISE(ABORT, 'invalid activity owner or content position'); END",
        "CREATE TRIGGER IF NOT EXISTS agena_activities_identity_immutable \
         BEFORE UPDATE OF activity_id, owner_kind, owner_id, actor, position, started_at_ms ON agena_activities \
         WHEN OLD.activity_id != NEW.activity_id OR OLD.owner_kind != NEW.owner_kind \
           OR OLD.owner_id != NEW.owner_id OR OLD.actor != NEW.actor \
           OR OLD.position != NEW.position OR OLD.started_at_ms != NEW.started_at_ms \
         BEGIN SELECT RAISE(ABORT, 'activity identity and ownership are immutable'); END",
        "CREATE TRIGGER IF NOT EXISTS agena_text_segments_owner_insert_valid \
         BEFORE INSERT ON agena_text_segments \
         WHEN NEW.position < 0 OR NEW.revision_seq < 0 \
           OR NOT ( \
             (NEW.owner_kind = 'turn_input' AND EXISTS (SELECT 1 FROM agena_turns WHERE turn_id = NEW.owner_id)) \
             OR (NEW.owner_kind = 'response' AND EXISTS (SELECT 1 FROM agena_responses WHERE response_id = NEW.owner_id)) \
           ) \
           OR EXISTS (SELECT 1 FROM agena_activities activity WHERE activity.owner_kind = NEW.owner_kind AND activity.owner_id = NEW.owner_id AND activity.position = NEW.position) \
         BEGIN SELECT RAISE(ABORT, 'invalid text owner or content position'); END",
        "CREATE TRIGGER IF NOT EXISTS agena_text_segments_identity_immutable \
         BEFORE UPDATE OF segment_id, owner_kind, owner_id, position, created_at_ms ON agena_text_segments \
         WHEN OLD.segment_id != NEW.segment_id OR OLD.owner_kind != NEW.owner_kind \
           OR OLD.owner_id != NEW.owner_id OR OLD.position != NEW.position \
           OR OLD.created_at_ms != NEW.created_at_ms \
         BEGIN SELECT RAISE(ABORT, 'text segment identity and ownership are immutable'); END",
        "CREATE TRIGGER IF NOT EXISTS agena_turn_content_delete \
         AFTER DELETE ON agena_turns \
         BEGIN \
           DELETE FROM agena_text_segments WHERE owner_kind = 'turn_input' AND owner_id = OLD.turn_id; \
           DELETE FROM agena_activities WHERE owner_kind = 'turn_input' AND owner_id = OLD.turn_id; \
         END",
        "CREATE TRIGGER IF NOT EXISTS agena_response_content_delete \
         AFTER DELETE ON agena_responses \
         BEGIN \
           DELETE FROM agena_text_segments WHERE owner_kind = 'response' AND owner_id = OLD.response_id; \
           DELETE FROM agena_activities WHERE owner_kind = 'response' AND owner_id = OLD.response_id; \
         END",
        "CREATE TRIGGER IF NOT EXISTS agena_activity_children_delete \
         AFTER DELETE ON agena_activities \
         BEGIN \
           DELETE FROM agena_activities WHERE owner_kind = 'activity' AND owner_id = OLD.activity_id; \
         END",
        "CREATE TRIGGER IF NOT EXISTS agena_session_activities_delete \
         AFTER DELETE ON agena_sessions \
         BEGIN \
           DELETE FROM agena_activities WHERE owner_kind = 'session' AND owner_id = CAST(OLD.id AS TEXT); \
         END",
        "CREATE TRIGGER IF NOT EXISTS agena_events_append_only \
         BEFORE UPDATE ON agena_events \
         BEGIN SELECT RAISE(ABORT, 'event log rows are append-only'); END",
        "CREATE TRIGGER IF NOT EXISTS agena_events_scope_insert_valid \
         BEFORE INSERT ON agena_events \
         WHEN (NEW.session_id IS NULL) != (NEW.seq_session IS NULL) \
           OR (NEW.session_id IS NOT NULL AND NEW.workspace_id IS NOT NULL AND NOT EXISTS ( \
             SELECT 1 FROM agena_sessions s \
             WHERE s.id = NEW.session_id AND s.workspace_id = NEW.workspace_id \
           )) \
         BEGIN SELECT RAISE(ABORT, 'invalid session event scope or workspace ownership'); END",
        "CREATE TRIGGER IF NOT EXISTS agena_transcript_messages_shape_insert_valid \
         BEFORE INSERT ON agena_transcript_messages \
         WHEN NEW.part_count < 0 \
         BEGIN SELECT RAISE(ABORT, 'message part count cannot be negative'); END",
        "CREATE TRIGGER IF NOT EXISTS agena_transcript_messages_shape_update_valid \
         BEFORE UPDATE OF part_count ON agena_transcript_messages \
         WHEN NEW.part_count < 0 \
         BEGIN SELECT RAISE(ABORT, 'message part count cannot be negative'); END",
        "CREATE TRIGGER IF NOT EXISTS agena_transcript_parts_shape_insert_valid \
         BEFORE INSERT ON agena_transcript_parts \
         WHEN NEW.part_index < 0 \
         BEGIN SELECT RAISE(ABORT, 'part index cannot be negative'); END",
    ] {
        db.execute(Statement::from_string(backend, sql.to_owned()))
            .await?;
    }
    Ok(())
}
