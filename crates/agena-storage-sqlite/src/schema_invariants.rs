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
        "CREATE TRIGGER IF NOT EXISTS agena_session_lineage_shape_insert_valid \
         BEFORE INSERT ON agena_session_lineage \
         WHEN NEW.relation_kind NOT IN ('child', 'fork', 'rewind', 'subagent') \
           OR NOT EXISTS (SELECT 1 FROM agena_sessions s WHERE s.id = NEW.session_id AND s.parent_id IS NOT NULL) \
           OR (NEW.relation_kind = 'subagent' AND (NEW.task_id IS NULL OR NEW.subtask_status IS NULL)) \
           OR (NEW.relation_kind = 'subagent' AND (length(trim(NEW.task_id)) = 0 OR NEW.subtask_status NOT IN ('created', 'running', 'completed', 'failed', 'cancelled', 'timed_out', 'interrupted'))) \
           OR (NEW.relation_kind = 'subagent' AND ( \
             (NEW.subtask_status = 'created' AND (NEW.subtask_started_at_ms IS NOT NULL OR NEW.subtask_finished_at_ms IS NOT NULL OR NEW.subtask_failure_json IS NOT NULL)) \
             OR (NEW.subtask_status = 'running' AND (NEW.subtask_started_at_ms IS NULL OR NEW.subtask_finished_at_ms IS NOT NULL OR NEW.subtask_failure_json IS NOT NULL)) \
             OR (NEW.subtask_status = 'completed' AND (NEW.subtask_started_at_ms IS NULL OR NEW.subtask_finished_at_ms IS NULL OR NEW.subtask_failure_json IS NOT NULL)) \
             OR (NEW.subtask_status = 'cancelled' AND (NEW.subtask_started_at_ms IS NULL OR NEW.subtask_finished_at_ms IS NULL OR NEW.subtask_failure_json IS NOT NULL)) \
             OR (NEW.subtask_status IN ('failed', 'timed_out', 'interrupted') AND (NEW.subtask_started_at_ms IS NULL OR NEW.subtask_finished_at_ms IS NULL OR NEW.subtask_failure_json IS NULL OR length(trim(NEW.subtask_failure_json)) = 0 OR CASE WHEN json_valid(NEW.subtask_failure_json) = 1 THEN (json_type(NEW.subtask_failure_json, '$.id') != 'text' OR json_type(NEW.subtask_failure_json, '$.code') != 'text' OR json_type(NEW.subtask_failure_json, '$.user.fallback') != 'text') ELSE 1 END)) \
             OR (NEW.subtask_started_at_ms IS NOT NULL AND NEW.subtask_started_at_ms < 0) \
             OR (NEW.subtask_finished_at_ms IS NOT NULL AND NEW.subtask_finished_at_ms < NEW.subtask_started_at_ms) \
           )) \
           OR (NEW.relation_kind != 'subagent' AND (NEW.task_id IS NOT NULL OR NEW.subtask_status IS NOT NULL OR NEW.subtask_started_at_ms IS NOT NULL OR NEW.subtask_finished_at_ms IS NOT NULL OR NEW.subtask_failure_json IS NOT NULL)) \
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
         BEFORE UPDATE OF subtask_status, subtask_started_at_ms, subtask_finished_at_ms, subtask_failure_json ON agena_session_lineage \
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
        "CREATE TRIGGER IF NOT EXISTS agena_turns_shape_insert_valid \
         BEFORE INSERT ON agena_turns \
         WHEN NEW.turn_seq <= 0 OR NEW.created_at_ms < 0 \
         BEGIN SELECT RAISE(ABORT, 'invalid canonical turn'); END",
        "CREATE TRIGGER IF NOT EXISTS agena_turns_identity_immutable \
         BEFORE UPDATE OF turn_id, session_id, turn_seq, created_at_ms ON agena_turns \
         WHEN OLD.turn_id != NEW.turn_id OR OLD.session_id != NEW.session_id \
           OR OLD.turn_seq != NEW.turn_seq OR OLD.created_at_ms != NEW.created_at_ms \
         BEGIN SELECT RAISE(ABORT, 'canonical turn identity is immutable'); END",
        "CREATE TRIGGER IF NOT EXISTS agena_assistant_replies_shape_insert_valid \
         BEFORE INSERT ON agena_assistant_replies \
         WHEN NEW.status NOT IN ('pending', 'in_progress', 'completed', 'failed', 'cancelled') \
           OR NEW.revision_seq < 0 OR NEW.created_at_ms < 0 \
           OR (NEW.status IN ('completed', 'failed', 'cancelled') \
               AND (NEW.finished_at_ms IS NULL OR NEW.finished_at_ms < NEW.created_at_ms)) \
           OR (NEW.status IN ('pending', 'in_progress') AND NEW.finished_at_ms IS NOT NULL) \
         BEGIN SELECT RAISE(ABORT, 'invalid assistant reply lifecycle'); END",
        "CREATE TRIGGER IF NOT EXISTS agena_assistant_replies_shape_update_valid \
         BEFORE UPDATE OF status, revision_seq, finished_at_ms ON agena_assistant_replies \
         WHEN NEW.status NOT IN ('pending', 'in_progress', 'completed', 'failed', 'cancelled') \
           OR NEW.revision_seq < OLD.revision_seq \
           OR (NEW.status IN ('completed', 'failed', 'cancelled') \
               AND (NEW.finished_at_ms IS NULL OR NEW.finished_at_ms < NEW.created_at_ms)) \
           OR (NEW.status IN ('pending', 'in_progress') AND NEW.finished_at_ms IS NOT NULL) \
         BEGIN SELECT RAISE(ABORT, 'invalid assistant reply lifecycle'); END",
        "CREATE TRIGGER IF NOT EXISTS agena_assistant_replies_identity_immutable \
         BEFORE UPDATE OF reply_id, turn_id, created_at_ms ON agena_assistant_replies \
         WHEN OLD.reply_id != NEW.reply_id OR OLD.turn_id != NEW.turn_id \
           OR OLD.created_at_ms != NEW.created_at_ms \
         BEGIN SELECT RAISE(ABORT, 'assistant reply identity is immutable'); END",
        "CREATE TRIGGER IF NOT EXISTS agena_reply_executions_shape_insert_valid \
         BEFORE INSERT ON agena_reply_executions \
         WHEN NEW.source NOT IN ('user', 'continue', 'compaction', 'permission_reply', 'user_input_reply') \
           OR NEW.status NOT IN ('in_progress', 'completed', 'failed', 'cancelled') \
           OR NEW.revision_seq < 0 OR NEW.started_at_ms < 0 \
           OR (NEW.status IN ('completed', 'failed', 'cancelled') \
               AND (NEW.finished_at_ms IS NULL OR NEW.finished_at_ms < NEW.started_at_ms)) \
           OR (NEW.status = 'in_progress' AND NEW.finished_at_ms IS NOT NULL) \
           OR (NEW.source = 'user' AND EXISTS ( \
               SELECT 1 FROM agena_reply_executions existing WHERE existing.reply_id = NEW.reply_id \
           )) \
           OR (NEW.source != 'user' AND NOT EXISTS ( \
               SELECT 1 FROM agena_reply_executions original \
               WHERE original.reply_id = NEW.reply_id AND original.source = 'user' \
           )) \
         BEGIN SELECT RAISE(ABORT, 'invalid assistant reply execution'); END",
        "CREATE TRIGGER IF NOT EXISTS agena_reply_executions_shape_update_valid \
         BEFORE UPDATE OF status, revision_seq, finished_at_ms ON agena_reply_executions \
         WHEN NEW.status NOT IN ('in_progress', 'completed', 'failed', 'cancelled') \
           OR NEW.revision_seq < OLD.revision_seq \
           OR OLD.status != 'in_progress' \
           OR NEW.status = 'in_progress' \
           OR NEW.finished_at_ms IS NULL OR NEW.finished_at_ms < NEW.started_at_ms \
         BEGIN SELECT RAISE(ABORT, 'invalid assistant reply execution lifecycle'); END",
        "CREATE TRIGGER IF NOT EXISTS agena_reply_executions_identity_immutable \
         BEFORE UPDATE OF execution_id, reply_id, source, started_at_ms ON agena_reply_executions \
         WHEN OLD.execution_id != NEW.execution_id OR OLD.reply_id != NEW.reply_id \
           OR OLD.source != NEW.source OR OLD.started_at_ms != NEW.started_at_ms \
         BEGIN SELECT RAISE(ABORT, 'assistant reply execution identity is immutable'); END",
        "CREATE TRIGGER IF NOT EXISTS agena_model_messages_identity_immutable \
         BEFORE UPDATE OF message_id, session_id, model_turn_id, role, created_at_ms ON agena_model_messages \
         WHEN OLD.message_id != NEW.message_id \
           OR OLD.session_id != NEW.session_id \
           OR OLD.model_turn_id IS NOT NEW.model_turn_id \
           OR OLD.role != NEW.role \
           OR OLD.created_at_ms != NEW.created_at_ms \
         BEGIN SELECT RAISE(ABORT, 'message identity and ownership are immutable'); END",
        "CREATE TRIGGER IF NOT EXISTS agena_model_message_parts_identity_immutable \
         BEFORE UPDATE OF part_id, message_id, part_index, kind, activity_id, segment_id, operation_id, created_at_ms ON agena_model_message_parts \
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
             OR (NEW.owner_kind = 'assistant_reply' AND EXISTS (SELECT 1 FROM agena_assistant_replies WHERE reply_id = NEW.owner_id)) \
             OR (NEW.owner_kind = 'activity' AND EXISTS (SELECT 1 FROM agena_activities WHERE activity_id = NEW.owner_id)) \
             OR (NEW.owner_kind = 'session' AND EXISTS (SELECT 1 FROM agena_sessions WHERE CAST(id AS TEXT) = NEW.owner_id)) \
           ) \
           OR EXISTS (SELECT 1 FROM agena_text_segments text WHERE text.owner_kind = NEW.owner_kind AND text.owner_id = NEW.owner_id AND text.position = NEW.position) \
         BEGIN SELECT RAISE(ABORT, 'invalid activity owner or content position'); END",
        "CREATE TRIGGER IF NOT EXISTS agena_activities_lifecycle_insert_valid \
         BEFORE INSERT ON agena_activities \
         WHEN NEW.actor NOT IN ('user', 'assistant', 'runtime', 'tool', 'plugin') \
           OR NEW.state NOT IN ('pending', 'in_progress', 'completed', 'failed', 'cancelled') \
           OR NEW.started_at_ms < 0 \
           OR (NEW.state IN ('completed', 'failed', 'cancelled') \
               AND (NEW.finished_at_ms IS NULL OR NEW.finished_at_ms < NEW.started_at_ms)) \
           OR (NEW.state IN ('pending', 'in_progress') AND NEW.finished_at_ms IS NOT NULL) \
         BEGIN SELECT RAISE(ABORT, 'invalid activity lifecycle'); END",
        "CREATE TRIGGER IF NOT EXISTS agena_activities_lifecycle_update_valid \
         BEFORE UPDATE OF state, revision_seq, finished_at_ms ON agena_activities \
         WHEN NEW.state NOT IN ('pending', 'in_progress', 'completed', 'failed', 'cancelled') \
           OR NEW.revision_seq < OLD.revision_seq \
           OR (NEW.state IN ('completed', 'failed', 'cancelled') \
               AND (NEW.finished_at_ms IS NULL OR NEW.finished_at_ms < NEW.started_at_ms)) \
           OR (NEW.state IN ('pending', 'in_progress') AND NEW.finished_at_ms IS NOT NULL) \
         BEGIN SELECT RAISE(ABORT, 'invalid activity lifecycle'); END",
        "CREATE TRIGGER IF NOT EXISTS agena_activities_identity_immutable \
         BEFORE UPDATE OF activity_id, owner_kind, owner_id, actor, position, started_at_ms ON agena_activities \
         WHEN OLD.activity_id != NEW.activity_id OR OLD.owner_kind != NEW.owner_kind \
           OR OLD.owner_id != NEW.owner_id OR OLD.actor != NEW.actor \
           OR OLD.position != NEW.position OR OLD.started_at_ms != NEW.started_at_ms \
         BEGIN SELECT RAISE(ABORT, 'activity identity and ownership are immutable'); END",
        "CREATE TRIGGER IF NOT EXISTS agena_activities_revision_monotonic \
         BEFORE UPDATE OF revision_seq ON agena_activities \
         WHEN NEW.revision_seq < OLD.revision_seq \
         BEGIN SELECT RAISE(ABORT, 'activity revision cannot decrease'); END",
        "CREATE TRIGGER IF NOT EXISTS agena_text_segments_owner_insert_valid \
         BEFORE INSERT ON agena_text_segments \
         WHEN NEW.position < 0 OR NEW.revision_seq < 0 \
           OR NOT ( \
             (NEW.owner_kind = 'turn_input' AND EXISTS (SELECT 1 FROM agena_turns WHERE turn_id = NEW.owner_id)) \
             OR (NEW.owner_kind = 'assistant_reply' AND EXISTS (SELECT 1 FROM agena_assistant_replies WHERE reply_id = NEW.owner_id)) \
           ) \
           OR EXISTS (SELECT 1 FROM agena_activities activity WHERE activity.owner_kind = NEW.owner_kind AND activity.owner_id = NEW.owner_id AND activity.position = NEW.position) \
         BEGIN SELECT RAISE(ABORT, 'invalid text owner or content position'); END",
        "CREATE TRIGGER IF NOT EXISTS agena_text_segments_lifecycle_insert_valid \
         BEFORE INSERT ON agena_text_segments \
         WHEN NEW.created_at_ms < 0 OR NEW.updated_at_ms < NEW.created_at_ms \
         BEGIN SELECT RAISE(ABORT, 'invalid text segment lifecycle'); END",
        "CREATE TRIGGER IF NOT EXISTS agena_text_segments_lifecycle_update_valid \
         BEFORE UPDATE OF revision_seq, updated_at_ms ON agena_text_segments \
         WHEN NEW.revision_seq < OLD.revision_seq \
           OR NEW.updated_at_ms < OLD.updated_at_ms \
         BEGIN SELECT RAISE(ABORT, 'invalid text segment lifecycle'); END",
        "CREATE TRIGGER IF NOT EXISTS agena_text_segments_identity_immutable \
         BEFORE UPDATE OF segment_id, owner_kind, owner_id, position, created_at_ms ON agena_text_segments \
         WHEN OLD.segment_id != NEW.segment_id OR OLD.owner_kind != NEW.owner_kind \
           OR OLD.owner_id != NEW.owner_id OR OLD.position != NEW.position \
           OR OLD.created_at_ms != NEW.created_at_ms \
         BEGIN SELECT RAISE(ABORT, 'text segment identity and ownership are immutable'); END",
        "CREATE TRIGGER IF NOT EXISTS agena_text_segments_revision_monotonic \
         BEFORE UPDATE OF revision_seq ON agena_text_segments \
         WHEN NEW.revision_seq < OLD.revision_seq \
         BEGIN SELECT RAISE(ABORT, 'text segment revision cannot decrease'); END",
        "CREATE TRIGGER IF NOT EXISTS agena_turn_content_delete \
         AFTER DELETE ON agena_turns \
         BEGIN \
           DELETE FROM agena_text_segments WHERE owner_kind = 'turn_input' AND owner_id = OLD.turn_id; \
           DELETE FROM agena_activities WHERE owner_kind = 'turn_input' AND owner_id = OLD.turn_id; \
         END",
        "CREATE TRIGGER IF NOT EXISTS agena_assistant_reply_content_delete \
         AFTER DELETE ON agena_assistant_replies \
         BEGIN \
           DELETE FROM agena_text_segments WHERE owner_kind = 'assistant_reply' AND owner_id = OLD.reply_id; \
           DELETE FROM agena_activities WHERE owner_kind = 'assistant_reply' AND owner_id = OLD.reply_id; \
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
        "CREATE TRIGGER IF NOT EXISTS agena_model_messages_shape_insert_valid \
         BEFORE INSERT ON agena_model_messages \
         WHEN NEW.part_count < 0 \
         BEGIN SELECT RAISE(ABORT, 'message part count cannot be negative'); END",
        "CREATE TRIGGER IF NOT EXISTS agena_model_messages_shape_update_valid \
         BEFORE UPDATE OF part_count ON agena_model_messages \
         WHEN NEW.part_count < 0 \
         BEGIN SELECT RAISE(ABORT, 'message part count cannot be negative'); END",
        "CREATE TRIGGER IF NOT EXISTS agena_model_message_parts_shape_insert_valid \
         BEFORE INSERT ON agena_model_message_parts \
         WHEN NEW.part_index < 0 OR NEW.awaits_user_reply NOT IN (0, 1) \
         BEGIN SELECT RAISE(ABORT, 'invalid model message part shape'); END",
        "CREATE TRIGGER IF NOT EXISTS agena_model_message_parts_shape_update_valid \
         BEFORE UPDATE OF awaits_user_reply ON agena_model_message_parts \
         WHEN NEW.awaits_user_reply NOT IN (0, 1) \
         BEGIN SELECT RAISE(ABORT, 'invalid model message part shape'); END",
    ] {
        db.execute(Statement::from_string(backend, sql.to_owned()))
            .await?;
    }
    Ok(())
}
