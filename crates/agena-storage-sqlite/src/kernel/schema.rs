//! Complete session-kernel declarations for the current instance baseline.
//! Each database has one exact schema and one fact authority.

use crate::schema::validation::{declaration, schema_objects};
use sea_orm::{ConnectionTrait, DatabaseConnection, DbErr, Statement, TransactionTrait};

pub(super) const ARCHIVE_TABLES: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS agena_background_activities (id TEXT PRIMARY KEY, session_id INTEGER NOT NULL, execution_id TEXT NOT NULL, generation INTEGER NOT NULL, round_id INTEGER NOT NULL, tool_id INTEGER NOT NULL, entry_id INTEGER NOT NULL, owner_epoch TEXT NOT NULL, kind TEXT NOT NULL CHECK(kind IN ('process','terminal','agent','monitor')), label TEXT NOT NULL CHECK(length(CAST(label AS BLOB)) BETWEEN 1 AND 1024), phase TEXT NOT NULL CHECK(phase IN ('running','succeeded','failed','stopped','interrupted')), outcome_json TEXT CHECK(outcome_json IS NULL OR (json_valid(outcome_json) AND length(CAST(outcome_json AS BLOB))<=65536)), created_at_ms INTEGER NOT NULL, finished_at_ms INTEGER, UNIQUE(id,entry_id,session_id,execution_id,generation), FOREIGN KEY(tool_id,execution_id,generation,round_id) REFERENCES agena_tool_calls(id,execution_id,generation,round_id), FOREIGN KEY(entry_id,session_id,execution_id,generation) REFERENCES agena_entries(id,origin_session_id,execution_id,generation), CHECK((phase='running' AND outcome_json IS NULL AND finished_at_ms IS NULL) OR (phase!='running' AND outcome_json IS NOT NULL AND json_extract(outcome_json,'$.outcome') IS NOT NULL AND json_extract(outcome_json,'$.outcome')=phase AND finished_at_ms IS NOT NULL AND finished_at_ms>=created_at_ms)))",
    "CREATE TABLE IF NOT EXISTS agena_sessions (id INTEGER PRIMARY KEY AUTOINCREMENT, workspace_id INTEGER REFERENCES agena_workspaces(id) CHECK(workspace_id IS NULL OR workspace_id>0), title TEXT NOT NULL, parent_session_id INTEGER REFERENCES agena_sessions(id), root_session_id INTEGER REFERENCES agena_sessions(id), depth INTEGER NOT NULL DEFAULT 0 CHECK(typeof(depth)='integer' AND depth>=0), relation_kind TEXT NOT NULL DEFAULT 'root' CHECK(relation_kind IN ('root','child','fork','rewind','subagent')), configuration_json TEXT NOT NULL DEFAULT '{}' CHECK(json_valid(configuration_json) AND length(CAST(configuration_json AS BLOB))<=65536), agent_name TEXT GENERATED ALWAYS AS (json_extract(configuration_json,'$.agent.name')) STORED, agent_status TEXT GENERATED ALWAYS AS (json_extract(configuration_json,'$.agent.status')) STORED, temporary INTEGER GENERATED ALWAYS AS (COALESCE(json_extract(configuration_json,'$.conversation.mode'),'')='btw') STORED, owner_bot_id TEXT GENERATED ALWAYS AS (json_extract(configuration_json,'$.owner.bot')) STORED, owner_surface TEXT GENERATED ALWAYS AS (json_extract(configuration_json,'$.owner.surface')) STORED, owner_scope_key TEXT GENERATED ALWAYS AS (json_extract(configuration_json,'$.owner.scope_key')) STORED, favorite INTEGER NOT NULL DEFAULT 0 CHECK(favorite IN (0,1)), pinned INTEGER NOT NULL DEFAULT 0 CHECK(pinned IN (0,1)), selection_json TEXT CHECK(selection_json IS NULL OR json_valid(selection_json)), metadata_version INTEGER NOT NULL DEFAULT 1 CHECK(typeof(metadata_version)='integer' AND metadata_version>0), membership_generation INTEGER NOT NULL DEFAULT 1 CHECK(typeof(membership_generation)='integer' AND membership_generation>0), control_version INTEGER NOT NULL DEFAULT 1 CHECK(typeof(control_version)='integer' AND control_version>0), created_at_ms INTEGER NOT NULL, updated_at_ms INTEGER NOT NULL, deleted_at_ms INTEGER, CHECK(updated_at_ms>=created_at_ms), CHECK(owner_bot_id IS NULL OR (length(owner_bot_id)=36 AND owner_surface IN ('local','direct','group','broadcast'))), CHECK(owner_surface IS NULL OR owner_bot_id IS NOT NULL), CHECK(owner_scope_key IS NULL OR owner_surface IN ('direct','group','broadcast')))",
    "CREATE TABLE IF NOT EXISTS agena_executions (id TEXT PRIMARY KEY, session_id INTEGER NOT NULL REFERENCES agena_sessions(id), message_id INTEGER NOT NULL REFERENCES agena_entries(id) DEFERRABLE INITIALLY DEFERRED, current_generation INTEGER NOT NULL CHECK(typeof(current_generation)='integer' AND current_generation>0), created_at_ms INTEGER NOT NULL, UNIQUE(id,session_id), FOREIGN KEY(id,current_generation) REFERENCES agena_attempts(execution_id,generation) DEFERRABLE INITIALLY DEFERRED)",
    "CREATE TABLE IF NOT EXISTS agena_attempts (execution_id TEXT NOT NULL, generation INTEGER NOT NULL CHECK(typeof(generation)='integer' AND generation>0), session_id INTEGER NOT NULL, owner_epoch TEXT, response_entry_id INTEGER REFERENCES agena_entries(id) DEFERRABLE INITIALLY DEFERRED, stop_requested INTEGER NOT NULL DEFAULT 0 CHECK(stop_requested IN (0,1)), phase TEXT NOT NULL CHECK(phase IN ('queued','running','succeeded','failed','cancelled','interrupted')), started_at_ms INTEGER, finished_at_ms INTEGER, PRIMARY KEY(execution_id,generation), UNIQUE(execution_id,generation,session_id), FOREIGN KEY(execution_id,session_id) REFERENCES agena_executions(id,session_id), CHECK((phase='queued' AND owner_epoch IS NULL AND started_at_ms IS NULL AND finished_at_ms IS NULL) OR (phase='running' AND owner_epoch IS NOT NULL AND started_at_ms IS NOT NULL AND finished_at_ms IS NULL) OR (phase IN ('succeeded','failed','cancelled','interrupted') AND finished_at_ms IS NOT NULL)), CHECK(finished_at_ms IS NULL OR started_at_ms IS NULL OR finished_at_ms>=started_at_ms)) WITHOUT ROWID",
    "CREATE TABLE IF NOT EXISTS agena_rounds (id INTEGER PRIMARY KEY AUTOINCREMENT, execution_id TEXT NOT NULL, generation INTEGER NOT NULL, session_id INTEGER NOT NULL, ordinal INTEGER NOT NULL CHECK(ordinal>=0), phase TEXT NOT NULL CHECK(phase IN ('running','succeeded','failed','interrupted')), input_source_count INTEGER NOT NULL DEFAULT 0 CHECK(typeof(input_source_count)='integer' AND input_source_count>=0), input_source_bytes INTEGER NOT NULL DEFAULT 0 CHECK(typeof(input_source_bytes)='integer' AND input_source_bytes>=0), provider_state_json TEXT CHECK(provider_state_json IS NULL OR json_valid(provider_state_json)), provider_state_bytes INTEGER GENERATED ALWAYS AS (COALESCE(length(CAST(provider_state_json AS BLOB)),0)) STORED, input_metadata_json TEXT NOT NULL CHECK(json_valid(input_metadata_json)), created_at_ms INTEGER NOT NULL, finished_at_ms INTEGER, UNIQUE(execution_id,generation,ordinal), UNIQUE(id,execution_id,generation), FOREIGN KEY(execution_id,generation,session_id) REFERENCES agena_attempts(execution_id,generation,session_id), CHECK((phase='running' AND finished_at_ms IS NULL) OR (phase!='running' AND finished_at_ms IS NOT NULL)))",
    "CREATE TABLE IF NOT EXISTS agena_tool_calls (id INTEGER PRIMARY KEY AUTOINCREMENT, round_id INTEGER NOT NULL, execution_id TEXT NOT NULL, generation INTEGER NOT NULL, call_id TEXT NOT NULL CHECK(length(call_id)>0), name TEXT NOT NULL CHECK(length(name)>0), input_json TEXT NOT NULL CHECK(json_valid(input_json)), input_bytes INTEGER GENERATED ALWAYS AS (length(CAST(input_json AS BLOB))) STORED, execution_kind TEXT NOT NULL DEFAULT 'runtime' CHECK(execution_kind IN ('runtime','provider_hosted')), phase TEXT NOT NULL CHECK(phase IN ('preparing','running','succeeded','failed','cancelled','interrupted')), result_json TEXT CHECK(result_json IS NULL OR json_valid(result_json)), result_bytes INTEGER GENERATED ALWAYS AS (COALESCE(length(CAST(result_json AS BLOB)),0)) STORED, created_at_ms INTEGER NOT NULL, finished_at_ms INTEGER, UNIQUE(round_id,call_id), UNIQUE(id,execution_id,generation,round_id), FOREIGN KEY(round_id,execution_id,generation) REFERENCES agena_rounds(id,execution_id,generation), CHECK((phase IN ('preparing','running') AND result_json IS NULL AND finished_at_ms IS NULL) OR (phase NOT IN ('preparing','running') AND result_json IS NOT NULL AND finished_at_ms IS NOT NULL)))",
    "CREATE TABLE IF NOT EXISTS agena_interactions (id INTEGER PRIMARY KEY AUTOINCREMENT, tool_id INTEGER NOT NULL REFERENCES agena_tool_calls(id), session_id INTEGER NOT NULL, execution_id TEXT NOT NULL, generation INTEGER NOT NULL CHECK(generation>0), request_kind TEXT NOT NULL CHECK(request_kind IN ('permission','input')), request_json TEXT NOT NULL CHECK(json_valid(request_json)), request_bytes INTEGER GENERATED ALWAYS AS (length(CAST(request_json AS BLOB))) STORED, decision_json TEXT CHECK(decision_json IS NULL OR json_valid(decision_json)), decision_bytes INTEGER GENERATED ALWAYS AS (COALESCE(length(CAST(decision_json AS BLOB)),0)) STORED, remember_json TEXT CHECK(remember_json IS NULL OR json_valid(remember_json)), remember_bytes INTEGER GENERATED ALWAYS AS (COALESCE(length(CAST(remember_json AS BLOB)),0)) STORED, created_at_ms INTEGER NOT NULL, decided_at_ms INTEGER, UNIQUE(id,tool_id), FOREIGN KEY(tool_id,execution_id,generation) REFERENCES agena_tool_calls(id,execution_id,generation), FOREIGN KEY(execution_id,generation,session_id) REFERENCES agena_attempts(execution_id,generation,session_id), CHECK((decision_json IS NULL AND decided_at_ms IS NULL) OR (decision_json IS NOT NULL AND decided_at_ms IS NOT NULL)))",
    "CREATE TABLE IF NOT EXISTS agena_entries (id INTEGER PRIMARY KEY AUTOINCREMENT, origin_session_id INTEGER NOT NULL REFERENCES agena_sessions(id), execution_id TEXT NOT NULL, generation INTEGER NOT NULL, message_id INTEGER REFERENCES agena_entries(id), revision INTEGER NOT NULL DEFAULT 1 CHECK(typeof(revision)='integer' AND revision>0), kind TEXT NOT NULL CHECK(kind IN ('user','text','reasoning','tool','interaction','notice','compaction')), body_json TEXT CHECK(body_json IS NULL OR json_valid(body_json)), body_bytes INTEGER GENERATED ALWAYS AS (COALESCE(length(CAST(body_json AS BLOB)),0)) STORED, round_id INTEGER, tool_id INTEGER REFERENCES agena_tool_calls(id), interaction_id INTEGER REFERENCES agena_interactions(id), content_id TEXT REFERENCES agena_content_streams(id) DEFERRABLE INITIALLY DEFERRED, created_at_ms INTEGER NOT NULL, UNIQUE(id,origin_session_id,execution_id,generation), FOREIGN KEY(execution_id,generation,origin_session_id) REFERENCES agena_attempts(execution_id,generation,session_id) DEFERRABLE INITIALLY DEFERRED, FOREIGN KEY(round_id,execution_id,generation) REFERENCES agena_rounds(id,execution_id,generation), FOREIGN KEY(tool_id,execution_id,generation,round_id) REFERENCES agena_tool_calls(id,execution_id,generation,round_id), FOREIGN KEY(interaction_id,tool_id) REFERENCES agena_interactions(id,tool_id), CHECK((kind='user' AND message_id IS NULL AND body_json IS NOT NULL AND round_id IS NULL AND tool_id IS NULL AND interaction_id IS NULL AND content_id IS NULL) OR (kind IN ('text','reasoning') AND message_id IS NOT NULL AND round_id IS NOT NULL AND content_id IS NOT NULL AND body_json IS NULL AND tool_id IS NULL AND interaction_id IS NULL) OR (kind='tool' AND message_id IS NOT NULL AND round_id IS NOT NULL AND tool_id IS NOT NULL AND body_json IS NULL AND interaction_id IS NULL AND content_id IS NULL) OR (kind='interaction' AND message_id IS NOT NULL AND round_id IS NOT NULL AND tool_id IS NOT NULL AND interaction_id IS NOT NULL AND body_json IS NULL AND content_id IS NULL) OR (kind IN ('notice','compaction') AND message_id IS NOT NULL AND body_json IS NOT NULL AND interaction_id IS NULL)))",
    "CREATE TABLE IF NOT EXISTS agena_history_segments (id INTEGER PRIMARY KEY AUTOINCREMENT)",
    "CREATE TABLE IF NOT EXISTS agena_prompt_windows (session_id INTEGER PRIMARY KEY REFERENCES agena_sessions(id), rewrite_epoch INTEGER NOT NULL DEFAULT 1 CHECK(typeof(rewrite_epoch)='integer' AND rewrite_epoch>0), policy_key TEXT CHECK(policy_key IS NULL OR length(CAST(policy_key AS BLOB)) BETWEEN 1 AND 1024), consecutive_compaction_failures INTEGER NOT NULL DEFAULT 0 CHECK(typeof(consecutive_compaction_failures)='integer' AND consecutive_compaction_failures BETWEEN 0 AND 3), auto_compaction_disabled INTEGER NOT NULL DEFAULT 0 CHECK(auto_compaction_disabled IN (0,1)), native_compaction_unavailable INTEGER NOT NULL DEFAULT 0 CHECK(native_compaction_unavailable IN (0,1)), last_before_tokens INTEGER CHECK(last_before_tokens IS NULL OR (typeof(last_before_tokens)='integer' AND last_before_tokens>=0)), last_after_tokens INTEGER CHECK(last_after_tokens IS NULL OR (typeof(last_after_tokens)='integer' AND last_after_tokens>=0)), checkpoint_entry_id INTEGER REFERENCES agena_entries(id), through_entry_id INTEGER REFERENCES agena_entries(id), through_sequence INTEGER CHECK(through_sequence IS NULL OR through_sequence>=0), through_ordinal INTEGER CHECK(through_ordinal IS NULL OR through_ordinal BETWEEN 0 AND 127), CHECK((checkpoint_entry_id IS NULL AND through_entry_id IS NULL AND through_sequence IS NULL AND through_ordinal IS NULL) OR (checkpoint_entry_id IS NOT NULL AND through_entry_id IS NOT NULL AND through_sequence IS NOT NULL AND through_ordinal IS NOT NULL)))",
    "CREATE TABLE IF NOT EXISTS agena_compaction_rounds (round_id INTEGER PRIMARY KEY REFERENCES agena_rounds(id), base_checkpoint_entry_id INTEGER REFERENCES agena_entries(id), rewrite_epoch INTEGER NOT NULL CHECK(typeof(rewrite_epoch)='integer' AND rewrite_epoch>0))",
    "CREATE TABLE IF NOT EXISTS agena_segment_entries (segment_id INTEGER NOT NULL REFERENCES agena_history_segments(id) ON DELETE CASCADE, ordinal INTEGER NOT NULL CHECK(ordinal BETWEEN 0 AND 127), entry_id INTEGER NOT NULL REFERENCES agena_entries(id), PRIMARY KEY(segment_id,ordinal)) WITHOUT ROWID",
    // The two aggregates describe the visible slice of the edge.
    "CREATE TABLE IF NOT EXISTS agena_session_segments (
        session_id INTEGER NOT NULL REFERENCES agena_sessions(id),
        sequence INTEGER NOT NULL CHECK(typeof(sequence)='integer' AND sequence>=0),
        segment_id INTEGER NOT NULL REFERENCES agena_history_segments(id),
        first_ordinal INTEGER NOT NULL CHECK(first_ordinal BETWEEN 0 AND 127),
        end_ordinal INTEGER NOT NULL CHECK(end_ordinal BETWEEN 1 AND 128 AND end_ordinal>first_ordinal),
        since_generation INTEGER NOT NULL DEFAULT 1 CHECK(typeof(since_generation)='integer' AND since_generation>0),
        user_message_count INTEGER NOT NULL CHECK(typeof(user_message_count)='integer' AND user_message_count BETWEEN 0 AND end_ordinal-first_ordinal),
        max_entry_created_at_ms INTEGER NOT NULL CHECK(typeof(max_entry_created_at_ms)='integer'),
        PRIMARY KEY(session_id,sequence)
    ) WITHOUT ROWID",
    "CREATE TABLE IF NOT EXISTS agena_content_streams (source_key INTEGER PRIMARY KEY, id TEXT NOT NULL UNIQUE, entry_id INTEGER NOT NULL, session_id INTEGER NOT NULL, execution_id TEXT NOT NULL, generation INTEGER NOT NULL, activity_id TEXT, kind TEXT NOT NULL CHECK(kind IN ('text','log','structured','terminal','document')), epoch TEXT NOT NULL, committed_head INTEGER NOT NULL DEFAULT 0 CHECK(typeof(committed_head)='integer' AND committed_head>=0), ordinal INTEGER NOT NULL DEFAULT 0 CHECK(typeof(ordinal)='integer' AND ordinal>=0), lost_bytes INTEGER NOT NULL DEFAULT 0 CHECK(typeof(lost_bytes)='integer' AND lost_bytes>=0), lost_packets INTEGER NOT NULL DEFAULT 0 CHECK(typeof(lost_packets)='integer' AND lost_packets>=0), capture_state TEXT NOT NULL CHECK(capture_state IN ('open','complete','interrupted')), retained_start INTEGER NOT NULL DEFAULT 0 CHECK(retained_start>=0 AND retained_start<=committed_head), capture_error TEXT, capture_error_bytes INTEGER GENERATED ALWAYS AS (COALESCE(length(CAST(capture_error AS BLOB)),0)) STORED, FOREIGN KEY(entry_id,session_id,execution_id,generation) REFERENCES agena_entries(id,origin_session_id,execution_id,generation) DEFERRABLE INITIALLY DEFERRED, FOREIGN KEY(activity_id,entry_id,session_id,execution_id,generation) REFERENCES agena_background_activities(id,entry_id,session_id,execution_id,generation))",
    "CREATE TABLE IF NOT EXISTS agena_blobs (id TEXT PRIMARY KEY, session_id INTEGER NOT NULL REFERENCES agena_sessions(id), state TEXT NOT NULL CHECK(state IN ('preparing','ready','retired')), expected_bytes INTEGER NOT NULL CHECK(expected_bytes>=0), received_bytes INTEGER NOT NULL DEFAULT 0 CHECK(received_bytes>=0 AND received_bytes<=expected_bytes), abandoned INTEGER NOT NULL DEFAULT 0 CHECK(abandoned IN (0,1)), unprotected INTEGER NOT NULL DEFAULT 1 CHECK(unprotected IN (0,1)), digest BLOB CHECK(digest IS NULL OR length(digest)=32), created_at_ms INTEGER NOT NULL, CHECK(state!='ready' OR (digest IS NOT NULL AND received_bytes=expected_bytes)))",
    "CREATE TABLE IF NOT EXISTS agena_blob_chunks (blob_id TEXT NOT NULL REFERENCES agena_blobs(id), logical_start INTEGER NOT NULL CHECK(logical_start>=0 AND logical_start%131072=0), digest BLOB NOT NULL CHECK(length(digest)=32), body BLOB NOT NULL CHECK(length(body)>0 AND length(body)<=131072), PRIMARY KEY(blob_id,logical_start)) WITHOUT ROWID",
    "CREATE TABLE IF NOT EXISTS agena_blob_references (entry_id INTEGER NOT NULL REFERENCES agena_entries(id), blob_id TEXT NOT NULL REFERENCES agena_blobs(id), field TEXT NOT NULL, PRIMARY KEY(entry_id,blob_id,field)) WITHOUT ROWID",
    "CREATE TABLE IF NOT EXISTS agena_execution_purposes (execution_id TEXT PRIMARY KEY REFERENCES agena_executions(id), purpose TEXT NOT NULL CHECK(purpose IN ('continuation','compaction'))) WITHOUT ROWID",
    "CREATE TABLE IF NOT EXISTS agena_branch_entries (
        session_id INTEGER NOT NULL REFERENCES agena_sessions(id),
        entry_id INTEGER NOT NULL REFERENCES agena_entries(id),
        revision INTEGER NOT NULL CHECK(revision>0),
        execution_phase TEXT NOT NULL CHECK(execution_phase IN ('queued','running','succeeded','failed','cancelled','interrupted')),
        execution_started INTEGER NOT NULL CHECK(execution_started IN (0,1)),
        execution_generation INTEGER NOT NULL CHECK(execution_generation>0),
        response_entry_id INTEGER REFERENCES agena_entries(id),
        content_state TEXT,
        compaction_phase TEXT,
        compaction_committed INTEGER NOT NULL CHECK(compaction_committed IN (0,1)),
        tool_phase TEXT,
        tool_finished_at_ms INTEGER,
        interaction_decided INTEGER NOT NULL CHECK(interaction_decided IN (0,1)),
        interaction_presented_at TEXT,
        interactions_json TEXT CHECK(interactions_json IS NULL OR json_type(interactions_json)='array'),
        activities_json TEXT CHECK(activities_json IS NULL OR json_type(activities_json)='array'),
        blobs_json TEXT CHECK(blobs_json IS NULL OR json_type(blobs_json)='array'),
        PRIMARY KEY(session_id,entry_id)
     ) WITHOUT ROWID",
    "CREATE TABLE IF NOT EXISTS agena_branch_rounds (session_id INTEGER NOT NULL REFERENCES agena_sessions(id), round_id INTEGER NOT NULL REFERENCES agena_rounds(id), phase TEXT NOT NULL CHECK(phase IN ('running','succeeded','failed','interrupted')), has_provider_state INTEGER NOT NULL CHECK(has_provider_state IN (0,1)), finished_at_ms INTEGER, PRIMARY KEY(session_id,round_id)) WITHOUT ROWID",
    "CREATE TABLE IF NOT EXISTS agena_branch_activities (session_id INTEGER NOT NULL REFERENCES agena_sessions(id), activity_id TEXT NOT NULL REFERENCES agena_background_activities(id), has_outcome INTEGER NOT NULL CHECK(has_outcome IN (0,1)), finished_at_ms INTEGER, PRIMARY KEY(session_id,activity_id)) WITHOUT ROWID",
];

pub(super) const ARCHIVE_ROUND_INPUTS: &str = "CREATE TABLE IF NOT EXISTS agena_round_inputs (round_id INTEGER NOT NULL REFERENCES agena_rounds(id), ordinal INTEGER NOT NULL CHECK(ordinal>=0), entry_id INTEGER NOT NULL REFERENCES agena_entries(id), entry_revision INTEGER NOT NULL CHECK(typeof(entry_revision)='integer' AND entry_revision>0), response_entry_id INTEGER REFERENCES agena_entries(id), execution_phase TEXT NOT NULL CHECK(execution_phase IN ('queued','running','succeeded','failed','cancelled','interrupted')), tool_phase TEXT CHECK(tool_phase IS NULL OR tool_phase IN ('preparing','running','succeeded','failed','cancelled','interrupted')), interaction_decided INTEGER NOT NULL CHECK(interaction_decided IN (0,1)), PRIMARY KEY(round_id,ordinal), UNIQUE(round_id,entry_id)) WITHOUT ROWID";

pub(super) const ARCHIVE_ROUND_REPLAYS: &str =
    "CREATE TABLE IF NOT EXISTS agena_round_input_replays (
        round_id INTEGER NOT NULL REFERENCES agena_rounds(id),
        source_round_id INTEGER NOT NULL REFERENCES agena_rounds(id),
        phase TEXT NOT NULL CHECK(phase IN ('running','succeeded','failed','interrupted')),
        has_provider_state INTEGER NOT NULL CHECK(has_provider_state IN (0,1)),
        PRIMARY KEY(round_id,source_round_id)
     ) WITHOUT ROWID";

const TABLES: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS agena_content_capture_losses (source_key INTEGER NOT NULL REFERENCES agena_content_streams(source_key), first_packet INTEGER NOT NULL CHECK(typeof(first_packet)='integer' AND first_packet>0), last_packet INTEGER NOT NULL CHECK(typeof(last_packet)='integer' AND last_packet>=first_packet), after_position INTEGER NOT NULL CHECK(typeof(after_position)='integer' AND after_position>=0), byte_count INTEGER NOT NULL CHECK(typeof(byte_count)='integer' AND byte_count>=0), diagnostic TEXT NOT NULL CHECK(length(CAST(diagnostic AS BLOB)) BETWEEN 1 AND 256), PRIMARY KEY(source_key,first_packet)) WITHOUT ROWID",
    "CREATE TABLE IF NOT EXISTS agena_content_blocks (
        source_key INTEGER NOT NULL REFERENCES agena_content_streams(source_key),
        logical_start INTEGER NOT NULL CHECK(logical_start>=0),
        logical_end INTEGER NOT NULL CHECK(logical_end>logical_start),
        codec TEXT NOT NULL CHECK(codec IN ('utf8','events_v1','checkpoint_v1')),
        digest BLOB NOT NULL CHECK(length(digest)=32),
        body_bytes INTEGER NOT NULL CHECK(body_bytes BETWEEN 1 AND 33554432),
        body BLOB,
        PRIMARY KEY(source_key,logical_start),
        CHECK((body_bytes<=65536 AND body IS NOT NULL AND length(body)=body_bytes) OR (body_bytes>65536 AND body IS NULL))
    ) WITHOUT ROWID",
    "CREATE TABLE IF NOT EXISTS agena_content_block_pieces (
        source_key INTEGER NOT NULL,
        block_start INTEGER NOT NULL,
        byte_offset INTEGER NOT NULL CHECK(byte_offset>=0 AND byte_offset%65536=0),
        body BLOB NOT NULL CHECK(length(body) BETWEEN 1 AND 65536),
        PRIMARY KEY(source_key,block_start,byte_offset),
        FOREIGN KEY(source_key,block_start) REFERENCES agena_content_blocks(source_key,logical_start) ON DELETE CASCADE
    ) WITHOUT ROWID",
    "CREATE TABLE IF NOT EXISTS agena_content_checkpoints (source_key INTEGER NOT NULL, logical_sequence INTEGER NOT NULL CHECK(logical_sequence>0), block_start INTEGER NOT NULL CHECK(block_start>=0), PRIMARY KEY(source_key,logical_sequence), FOREIGN KEY(source_key,block_start) REFERENCES agena_content_blocks(source_key,logical_start)) WITHOUT ROWID",
    "CREATE TABLE IF NOT EXISTS agena_content_gaps (source_key INTEGER NOT NULL REFERENCES agena_content_streams(source_key), logical_start INTEGER NOT NULL CHECK(logical_start>=0), logical_end INTEGER NOT NULL CHECK(logical_end>logical_start), reason TEXT NOT NULL, PRIMARY KEY(source_key,logical_start)) WITHOUT ROWID",
    "CREATE TABLE IF NOT EXISTS agena_branch_sources (
        session_id INTEGER NOT NULL REFERENCES agena_sessions(id),
        source_key INTEGER NOT NULL REFERENCES agena_content_streams(source_key),
        head_epoch TEXT,
        committed_head INTEGER CHECK(committed_head>=0),
        retained_start INTEGER CHECK(retained_start BETWEEN 0 AND committed_head),
        capture_state TEXT CHECK(capture_state IN ('open','complete','interrupted')),
        lost_bytes INTEGER CHECK(lost_bytes>=0),
        lost_packets INTEGER CHECK(lost_packets>=0),
        committed_lost_bytes INTEGER CHECK(committed_lost_bytes>=0),
        committed_lost_packets INTEGER CHECK(committed_lost_packets>=0),
        capture_error TEXT,
        head_bytes INTEGER CHECK(head_bytes>0),
        head_owner_session_id INTEGER CHECK(head_owner_session_id<session_id),
        PRIMARY KEY(session_id,source_key),
        FOREIGN KEY(head_owner_session_id,source_key) REFERENCES agena_branch_sources(session_id,source_key),
        CHECK(CASE WHEN head_owner_session_id IS NULL THEN
                 head_epoch IS NOT NULL AND committed_head IS NOT NULL AND retained_start IS NOT NULL
                 AND capture_state IS NOT NULL AND lost_bytes IS NOT NULL AND lost_packets IS NOT NULL
                 AND committed_lost_bytes IS NOT NULL AND committed_lost_packets IS NOT NULL AND head_bytes IS NOT NULL
             ELSE head_epoch IS NULL AND committed_head IS NULL AND retained_start IS NULL
                 AND capture_state IS NULL AND lost_bytes IS NULL AND lost_packets IS NULL
                 AND committed_lost_bytes IS NULL AND committed_lost_packets IS NULL
                 AND capture_error IS NULL AND head_bytes IS NULL END)
     ) WITHOUT ROWID, STRICT",
    "CREATE TABLE IF NOT EXISTS agena_prompt_input_blocks (
        id INTEGER PRIMARY KEY,
        digest BLOB NOT NULL UNIQUE CHECK(typeof(digest)='blob' AND length(digest)=32)
     )",
    "CREATE TABLE IF NOT EXISTS agena_prompt_input_entries (
        block_id INTEGER NOT NULL REFERENCES agena_prompt_input_blocks(id),
        ordinal INTEGER NOT NULL CHECK(ordinal BETWEEN 0 AND 63),
        entry_id INTEGER NOT NULL REFERENCES agena_entries(id),
        entry_revision INTEGER NOT NULL CHECK(typeof(entry_revision)='integer' AND entry_revision>0),
        response_entry_id INTEGER REFERENCES agena_entries(id),
        execution_phase TEXT NOT NULL CHECK(execution_phase IN ('queued','running','succeeded','failed','cancelled','interrupted')),
        tool_phase TEXT CHECK(tool_phase IS NULL OR tool_phase IN ('preparing','running','succeeded','failed','cancelled','interrupted')),
        interaction_decided INTEGER NOT NULL CHECK(interaction_decided IN (0,1)),
        PRIMARY KEY(block_id,ordinal)
     ) WITHOUT ROWID",
    "CREATE TABLE IF NOT EXISTS agena_round_input_blocks (
        round_id INTEGER NOT NULL REFERENCES agena_rounds(id),
        start_ordinal INTEGER NOT NULL CHECK(start_ordinal>=0),
        block_id INTEGER NOT NULL REFERENCES agena_prompt_input_blocks(id),
        PRIMARY KEY(round_id,start_ordinal)
     ) WITHOUT ROWID",
    "CREATE TABLE IF NOT EXISTS agena_prompt_replay_blocks (
        id INTEGER PRIMARY KEY,
        digest BLOB NOT NULL UNIQUE CHECK(length(digest)=32),
        member_count INTEGER NOT NULL CHECK(member_count BETWEEN 1 AND 64),
        replay_bytes INTEGER NOT NULL CHECK(replay_bytes>=256*member_count)
     ) STRICT",
    "CREATE TABLE IF NOT EXISTS agena_prompt_replay_entries (
        block_id INTEGER NOT NULL REFERENCES agena_prompt_replay_blocks(id),
        source_round_id INTEGER NOT NULL REFERENCES agena_rounds(id),
        phase TEXT NOT NULL CHECK(phase IN ('running','succeeded','failed','interrupted')),
        has_provider_state INTEGER NOT NULL CHECK(has_provider_state IN (0,1)),
        PRIMARY KEY(block_id,source_round_id)
     ) WITHOUT ROWID, STRICT",
    "CREATE TABLE IF NOT EXISTS agena_round_replay_blocks (
        round_id INTEGER NOT NULL REFERENCES agena_rounds(id),
        bucket INTEGER NOT NULL CHECK(bucket>=0),
        block_id INTEGER NOT NULL REFERENCES agena_prompt_replay_blocks(id),
        PRIMARY KEY(round_id,bucket)
     ) WITHOUT ROWID, STRICT",
    "CREATE TABLE IF NOT EXISTS agena_round_input_sources (
        round_id INTEGER NOT NULL,
        block_id INTEGER NOT NULL,
        entry_id INTEGER NOT NULL,
        source_key INTEGER NOT NULL REFERENCES agena_content_streams(source_key),
        head_bytes INTEGER NOT NULL CHECK(head_bytes>0),
        head_epoch TEXT NOT NULL,
        committed_head INTEGER NOT NULL CHECK(committed_head>=0),
        retained_start INTEGER NOT NULL CHECK(retained_start BETWEEN 0 AND committed_head),
        capture_state TEXT NOT NULL CHECK(capture_state IN ('open','complete','interrupted')),
        lost_bytes INTEGER NOT NULL CHECK(lost_bytes>=0),
        lost_packets INTEGER NOT NULL CHECK(lost_packets>=0),
        committed_lost_bytes INTEGER NOT NULL CHECK(committed_lost_bytes>=0),
        committed_lost_packets INTEGER NOT NULL CHECK(committed_lost_packets>=0),
        capture_error TEXT,
        PRIMARY KEY(round_id,source_key),
        FOREIGN KEY(round_id,block_id) REFERENCES agena_round_input_blocks(round_id,block_id),
        FOREIGN KEY(block_id,entry_id) REFERENCES agena_prompt_input_entries(block_id,entry_id)
     ) WITHOUT ROWID, STRICT",
    "CREATE TABLE IF NOT EXISTS agena_bots (id TEXT PRIMARY KEY CHECK(length(id)=36), name TEXT NOT NULL CHECK(length(CAST(name AS BLOB)) BETWEEN 1 AND 128), name_folded TEXT NOT NULL, instructions TEXT NOT NULL DEFAULT '' CHECK(length(CAST(instructions AS BLOB))<=262144), revision INTEGER NOT NULL CHECK(revision>0), archived INTEGER NOT NULL DEFAULT 0 CHECK(archived IN (0,1)), created_at_ms INTEGER NOT NULL, updated_at_ms INTEGER NOT NULL, CHECK(updated_at_ms>=created_at_ms)) WITHOUT ROWID",
    "CREATE TABLE IF NOT EXISTS agena_workspace_statistics (
        workspace_id INTEGER PRIMARY KEY REFERENCES agena_workspaces(id) ON DELETE CASCADE,
        total INTEGER NOT NULL DEFAULT 0 CHECK(total>=0),
        roots INTEGER NOT NULL DEFAULT 0 CHECK(roots BETWEEN 0 AND total),
        pinned INTEGER NOT NULL DEFAULT 0 CHECK(pinned BETWEEN 0 AND total),
        running INTEGER NOT NULL DEFAULT 0 CHECK(running BETWEEN 0 AND total),
        attention INTEGER NOT NULL DEFAULT 0 CHECK(attention BETWEEN 0 AND total)
     ) STRICT",
    "CREATE TABLE IF NOT EXISTS agena_session_statistics (session_id INTEGER PRIMARY KEY REFERENCES agena_sessions(id), entry_count INTEGER NOT NULL DEFAULT 0 CHECK(typeof(entry_count)='integer' AND entry_count>=0), user_message_count INTEGER NOT NULL DEFAULT 0 CHECK(typeof(user_message_count)='integer' AND user_message_count>=0), started_user_message_count INTEGER NOT NULL DEFAULT 0 CHECK(typeof(started_user_message_count)='integer' AND started_user_message_count>=0), queued_executions INTEGER NOT NULL DEFAULT 0 CHECK(typeof(queued_executions)='integer' AND queued_executions>=0), running_executions INTEGER NOT NULL DEFAULT 0 CHECK(typeof(running_executions)='integer' AND running_executions>=0), last_message_at_ms INTEGER, child_session_count INTEGER NOT NULL DEFAULT 0 CHECK(typeof(child_session_count)='integer' AND child_session_count>=0), processing_state TEXT NOT NULL DEFAULT 'ready' CHECK(processing_state IN ('ready','running','awaiting_interaction','failed')), overview_workspace_id INTEGER CHECK(overview_workspace_id IS NULL OR (typeof(overview_workspace_id)='integer' AND overview_workspace_id>0)))",
    "CREATE TABLE IF NOT EXISTS agena_session_runs (session_id INTEGER NOT NULL REFERENCES agena_sessions(id), owner_entry_id INTEGER NOT NULL, entry_id INTEGER NOT NULL REFERENCES agena_entries(id), is_user INTEGER NOT NULL CHECK(is_user IN (0,1)), PRIMARY KEY(session_id,owner_entry_id,entry_id)) WITHOUT ROWID",
    "CREATE TABLE IF NOT EXISTS agena_session_response_segments (
        session_id INTEGER NOT NULL REFERENCES agena_sessions(id),
        response_entry_id INTEGER NOT NULL,
        sequence INTEGER NOT NULL CHECK(sequence>=0),
        part_total INTEGER NOT NULL CHECK(part_total>0),
        PRIMARY KEY(session_id,response_entry_id,sequence)
     ) WITHOUT ROWID, STRICT",
    "CREATE TABLE IF NOT EXISTS agena_history_snapshots (token TEXT PRIMARY KEY, session_id INTEGER NOT NULL REFERENCES agena_sessions(id), generation INTEGER NOT NULL CHECK(typeof(generation)='integer' AND generation>0), owner_epoch TEXT NOT NULL, last_sequence INTEGER CHECK(last_sequence IS NULL OR last_sequence>=0))",
    "CREATE TABLE IF NOT EXISTS agena_retired_session_segments (id INTEGER PRIMARY KEY AUTOINCREMENT, session_id INTEGER NOT NULL REFERENCES agena_sessions(id), sequence INTEGER NOT NULL CHECK(sequence>=0), segment_id INTEGER NOT NULL REFERENCES agena_history_segments(id), first_ordinal INTEGER NOT NULL CHECK(first_ordinal BETWEEN 0 AND 127), end_ordinal INTEGER NOT NULL CHECK(end_ordinal BETWEEN 1 AND 128 AND end_ordinal>first_ordinal), since_generation INTEGER NOT NULL CHECK(typeof(since_generation)='integer' AND since_generation>0), until_generation INTEGER NOT NULL CHECK(typeof(until_generation)='integer' AND until_generation>since_generation), UNIQUE(session_id,sequence,since_generation))",
    "CREATE TABLE IF NOT EXISTS agena_blob_pins (blob_id TEXT NOT NULL REFERENCES agena_blobs(id), token TEXT NOT NULL, owner_epoch TEXT NOT NULL, PRIMARY KEY(blob_id,token)) WITHOUT ROWID",
    "CREATE TABLE IF NOT EXISTS agena_commits (id INTEGER PRIMARY KEY CHECK(id=1), sequence INTEGER NOT NULL CHECK(typeof(sequence)='integer' AND sequence>=0))",
    "CREATE TABLE IF NOT EXISTS agena_command_results (request_id BLOB NOT NULL PRIMARY KEY CHECK(typeof(request_id)='blob' AND length(request_id)=16), request_digest BLOB NOT NULL CHECK(typeof(request_digest)='blob' AND length(request_digest)=32), commit_sequence INTEGER NOT NULL CHECK(commit_sequence>0), command_ordinal INTEGER NOT NULL CHECK(command_ordinal BETWEEN 0 AND 4294967295), value_json BLOB NOT NULL CHECK(typeof(value_json)='blob' AND length(value_json)>1))",
    "CREATE TABLE IF NOT EXISTS agena_deliveries (id TEXT PRIMARY KEY, session_id INTEGER NOT NULL REFERENCES agena_sessions(id), business_key TEXT NOT NULL UNIQUE, target_execution_id TEXT REFERENCES agena_executions(id), payload_json TEXT NOT NULL CHECK(json_valid(payload_json)), state TEXT NOT NULL CHECK(state IN ('pending','consumed','failed')), result_entry_id INTEGER REFERENCES agena_entries(id), observed_round_id INTEGER REFERENCES agena_rounds(id), resume_execution_id TEXT REFERENCES agena_executions(id), resume_generation INTEGER CHECK(resume_generation IS NULL OR (typeof(resume_generation)='integer' AND resume_generation>0)))",
    "CREATE TABLE IF NOT EXISTS agena_bot_receipts (request_key TEXT PRIMARY KEY, input_digest BLOB NOT NULL CHECK(typeof(input_digest)='blob' AND length(input_digest)=32), result BLOB NOT NULL CHECK(typeof(result)='blob' AND length(result)>1)) WITHOUT ROWID",
];

const VIEWS: &[&str] = &[
    "CREATE VIEW IF NOT EXISTS agena_branch_source_heads AS
        SELECT b.session_id,b.source_key,h.head_epoch,h.committed_head,h.retained_start,h.capture_state,
               h.lost_bytes,h.lost_packets,h.committed_lost_bytes,h.committed_lost_packets,h.capture_error,h.head_bytes
        FROM agena_branch_sources b
        CROSS JOIN agena_branch_sources h ON h.source_key=b.source_key
            AND h.session_id=COALESCE(b.head_owner_session_id,b.session_id)",
    "CREATE VIEW IF NOT EXISTS agena_round_inputs AS
        SELECT r.round_id,r.start_ordinal+e.ordinal AS ordinal,r.start_ordinal,e.ordinal AS block_ordinal,
               r.block_id,e.entry_id,e.entry_revision,e.response_entry_id,e.execution_phase,e.tool_phase,e.interaction_decided
        FROM agena_round_input_blocks r CROSS JOIN agena_prompt_input_entries e ON e.block_id=r.block_id",
    "CREATE VIEW IF NOT EXISTS agena_round_input_replays AS
        SELECT r.round_id,r.bucket,e.source_round_id,e.phase,e.has_provider_state
        FROM agena_round_replay_blocks r CROSS JOIN agena_prompt_replay_entries e ON e.block_id=r.block_id",
];

// Staging validates references and groups rows by ownership without runtime indexes.
pub(super) const ARCHIVE_INDEXES: &[&str] = &[
    "CREATE INDEX IF NOT EXISTS idx_agena_background_tool ON agena_background_activities(tool_id,id)",
    "CREATE UNIQUE INDEX IF NOT EXISTS idx_agena_tool_scope ON agena_tool_calls(id,execution_id,generation)",
    "CREATE INDEX IF NOT EXISTS idx_agena_interaction_tool ON agena_interactions(tool_id,id)",
    "CREATE INDEX IF NOT EXISTS idx_agena_entry_round ON agena_entries(round_id) WHERE round_id IS NOT NULL",
    "CREATE INDEX IF NOT EXISTS idx_agena_entry_tool ON agena_entries(tool_id,id) WHERE tool_id IS NOT NULL",
    "CREATE INDEX IF NOT EXISTS idx_agena_entry_interaction ON agena_entries(interaction_id,id) WHERE interaction_id IS NOT NULL",
    "CREATE INDEX IF NOT EXISTS idx_agena_content_prompt ON agena_content_streams(entry_id,id,capture_error_bytes)",
    "CREATE UNIQUE INDEX IF NOT EXISTS idx_agena_segment_entry_reverse ON agena_segment_entries(entry_id,segment_id)",
];

const INDEXES: &[&str] = &[
    "CREATE INDEX IF NOT EXISTS idx_agena_background_active ON agena_background_activities(session_id,id) WHERE phase='running'",
    "CREATE INDEX IF NOT EXISTS idx_agena_background_entry ON agena_background_activities(entry_id,id)",
    "CREATE INDEX IF NOT EXISTS idx_agena_content_activity ON agena_content_streams(activity_id,id) WHERE activity_id IS NOT NULL",
    "CREATE INDEX IF NOT EXISTS idx_agena_sessions_workspace ON agena_sessions(workspace_id,updated_at_ms,id) WHERE deleted_at_ms IS NULL",
    "CREATE INDEX IF NOT EXISTS idx_agena_sessions_workspace_owner ON agena_sessions(workspace_id,owner_bot_id) WHERE workspace_id IS NOT NULL",
    "CREATE INDEX IF NOT EXISTS idx_agena_sessions_favorite ON agena_sessions(workspace_id,updated_at_ms,id) WHERE deleted_at_ms IS NULL AND temporary=0 AND favorite=1",
    "CREATE INDEX IF NOT EXISTS idx_agena_sessions_pinned ON agena_sessions(workspace_id,updated_at_ms,id) WHERE deleted_at_ms IS NULL AND temporary=0 AND pinned=1",
    "CREATE INDEX IF NOT EXISTS idx_agena_session_processing ON agena_session_statistics(processing_state)",
    "CREATE INDEX IF NOT EXISTS idx_agena_workspace_attention ON agena_session_statistics(overview_workspace_id) WHERE overview_workspace_id IS NOT NULL AND processing_state!='ready'",
    "CREATE UNIQUE INDEX IF NOT EXISTS uq_agena_bot_active_name ON agena_bots(name_folded) WHERE archived=0",
    "CREATE INDEX IF NOT EXISTS idx_agena_session_owner ON agena_sessions(owner_bot_id,updated_at_ms DESC,id DESC) WHERE owner_bot_id IS NOT NULL AND deleted_at_ms IS NULL",
    "CREATE INDEX IF NOT EXISTS idx_agena_session_owner_surface ON agena_sessions(owner_bot_id,owner_surface,updated_at_ms DESC,id DESC) WHERE owner_bot_id IS NOT NULL AND deleted_at_ms IS NULL",
    "CREATE UNIQUE INDEX IF NOT EXISTS uq_agena_session_owner_scope ON agena_sessions(owner_bot_id,owner_scope_key) WHERE owner_scope_key IS NOT NULL AND deleted_at_ms IS NULL",
    "CREATE INDEX IF NOT EXISTS idx_agena_execution_owner ON agena_executions(session_id,id)",
    "CREATE INDEX IF NOT EXISTS idx_agena_execution_latest ON agena_executions(session_id,message_id DESC)",
    "CREATE INDEX IF NOT EXISTS idx_agena_execution_message ON agena_executions(message_id)",
    "CREATE INDEX IF NOT EXISTS idx_agena_attempt_active ON agena_attempts(session_id,execution_id,generation) WHERE phase IN ('queued','running')",
    "CREATE INDEX IF NOT EXISTS idx_agena_attempt_terminal ON agena_attempts(finished_at_ms DESC,(phase='failed') DESC,session_id) WHERE finished_at_ms IS NOT NULL",
    "CREATE INDEX IF NOT EXISTS idx_agena_tool_active ON agena_tool_calls(execution_id,generation,id) WHERE phase IN ('preparing','running')",
    "CREATE INDEX IF NOT EXISTS idx_agena_tool_drafts ON agena_tool_calls(round_id,id) WHERE phase='preparing'",
    // The implicit rowid supplies the round ID and its ordering.
    "CREATE INDEX IF NOT EXISTS idx_agena_round_session ON agena_rounds(session_id)",
    "CREATE INDEX IF NOT EXISTS idx_agena_delivery_session ON agena_deliveries(session_id,id) WHERE state='pending'",
    "CREATE INDEX IF NOT EXISTS idx_agena_delivery_execution ON agena_deliveries(target_execution_id) WHERE state='pending' AND target_execution_id IS NOT NULL",
    "CREATE INDEX IF NOT EXISTS idx_agena_delivery_target_execution ON agena_deliveries(target_execution_id) WHERE target_execution_id IS NOT NULL",
    "CREATE INDEX IF NOT EXISTS idx_agena_round_active ON agena_rounds(execution_id,generation,id) WHERE phase='running'",
    "CREATE INDEX IF NOT EXISTS idx_agena_attempt_running ON agena_attempts(execution_id,generation) WHERE phase='running'",
    "CREATE INDEX IF NOT EXISTS idx_agena_content_active ON agena_content_streams(execution_id,generation,id) WHERE capture_state='open'",
    "CREATE INDEX IF NOT EXISTS idx_agena_interaction_pending ON agena_interactions(tool_id,id) WHERE decision_json IS NULL",
    "CREATE INDEX IF NOT EXISTS idx_agena_interaction_pending_session ON agena_interactions(session_id,id,request_bytes) WHERE decision_json IS NULL",
    "CREATE INDEX IF NOT EXISTS idx_agena_interaction_attempt ON agena_interactions(execution_id,generation,session_id)",
    "CREATE INDEX IF NOT EXISTS idx_agena_session_agent_active ON agena_sessions(agent_status,id) WHERE deleted_at_ms IS NULL AND agent_status IS NOT NULL",
    "CREATE UNIQUE INDEX IF NOT EXISTS idx_agena_session_agent_name ON agena_sessions(parent_session_id,agent_name) WHERE deleted_at_ms IS NULL AND agent_name IS NOT NULL",
    "CREATE INDEX IF NOT EXISTS idx_agena_session_parent ON agena_sessions(parent_session_id,updated_at_ms DESC,id DESC) WHERE deleted_at_ms IS NULL",
    "CREATE INDEX IF NOT EXISTS idx_agena_session_global ON agena_sessions(updated_at_ms DESC,id DESC) WHERE deleted_at_ms IS NULL",
    "CREATE INDEX IF NOT EXISTS idx_agena_entry_owner ON agena_entries(origin_session_id,id)",
    "CREATE INDEX IF NOT EXISTS idx_agena_owned_tool_entries ON agena_entries(origin_session_id,id,body_bytes,tool_id) WHERE kind='tool'",
    "CREATE INDEX IF NOT EXISTS idx_agena_entry_attempt ON agena_entries(execution_id,generation,origin_session_id)",
    "CREATE INDEX IF NOT EXISTS idx_agena_entry_message ON agena_entries(message_id) WHERE message_id IS NOT NULL",
    "CREATE INDEX IF NOT EXISTS idx_agena_entry_compaction_round ON agena_entries(round_id) WHERE kind='compaction' AND round_id IS NOT NULL",
    "CREATE INDEX IF NOT EXISTS idx_agena_entry_content ON agena_entries(content_id) WHERE content_id IS NOT NULL",
    "CREATE INDEX IF NOT EXISTS idx_agena_entry_preflight ON agena_entries(id,body_bytes,tool_id,interaction_id)",
    "CREATE INDEX IF NOT EXISTS idx_agena_interaction_bytes ON agena_interactions(id,request_bytes,decision_bytes,remember_bytes)",
    "CREATE INDEX IF NOT EXISTS idx_agena_tool_bytes ON agena_tool_calls(id,input_bytes,result_bytes)",
    "CREATE INDEX IF NOT EXISTS idx_agena_round_export ON agena_rounds(execution_id,id)",
    "CREATE INDEX IF NOT EXISTS idx_agena_prompt_checkpoint ON agena_prompt_windows(checkpoint_entry_id) WHERE checkpoint_entry_id IS NOT NULL",
    "CREATE INDEX IF NOT EXISTS idx_agena_prompt_through ON agena_prompt_windows(through_entry_id) WHERE through_entry_id IS NOT NULL",
    "CREATE INDEX IF NOT EXISTS idx_agena_compaction_base ON agena_compaction_rounds(base_checkpoint_entry_id) WHERE base_checkpoint_entry_id IS NOT NULL",
    "CREATE INDEX IF NOT EXISTS idx_agena_round_source_block ON agena_round_input_sources(block_id,round_id,entry_id)",
    "CREATE INDEX IF NOT EXISTS idx_agena_round_source_key ON agena_round_input_sources(source_key)",
    "CREATE INDEX IF NOT EXISTS idx_agena_prompt_replay_source ON agena_prompt_replay_entries(source_round_id)",
    "CREATE INDEX IF NOT EXISTS idx_agena_round_replay_block ON agena_round_replay_blocks(block_id)",
    "CREATE INDEX IF NOT EXISTS idx_agena_prompt_input_response ON agena_prompt_input_entries(response_entry_id) WHERE response_entry_id IS NOT NULL",
    "CREATE INDEX IF NOT EXISTS idx_agena_attempt_response ON agena_attempts(response_entry_id) WHERE response_entry_id IS NOT NULL",
    // Reverse lookups also enforce the composite parent keys of frozen sources.
    "CREATE UNIQUE INDEX IF NOT EXISTS idx_agena_prompt_input_entry ON agena_prompt_input_entries(entry_id,block_id)",
    "CREATE UNIQUE INDEX IF NOT EXISTS idx_agena_round_input_block ON agena_round_input_blocks(block_id,round_id)",
    "CREATE INDEX IF NOT EXISTS idx_agena_session_segment_reverse ON agena_session_segments(segment_id,session_id)",
    "CREATE INDEX IF NOT EXISTS idx_agena_response_segment_cut ON agena_session_response_segments(session_id,sequence,response_entry_id)",
    "CREATE INDEX IF NOT EXISTS idx_agena_history_snapshot_owner ON agena_history_snapshots(session_id,generation)",
    "CREATE INDEX IF NOT EXISTS idx_agena_retired_segment_reverse ON agena_retired_session_segments(segment_id)",
    "CREATE INDEX IF NOT EXISTS idx_agena_content_session_active ON agena_content_streams(session_id) WHERE capture_state='open'",
    "CREATE INDEX IF NOT EXISTS idx_agena_checkpoint_block ON agena_content_checkpoints(source_key,block_start)",
    "CREATE INDEX IF NOT EXISTS idx_agena_blob_reference_reverse ON agena_blob_references(blob_id,entry_id)",
    "CREATE INDEX IF NOT EXISTS idx_agena_blob_retired ON agena_blobs(created_at_ms,id) WHERE state='retired'",
    "CREATE INDEX IF NOT EXISTS idx_agena_blob_unprotected ON agena_blobs(CASE WHEN abandoned=1 THEN 0 ELSE created_at_ms END,id) WHERE state!='retired' AND unprotected=1",
    "CREATE INDEX IF NOT EXISTS idx_agena_delivery_resume ON agena_deliveries(resume_execution_id,resume_generation,session_id) WHERE state='consumed' AND resume_execution_id IS NOT NULL",
    "CREATE INDEX IF NOT EXISTS idx_agena_delivery_result ON agena_deliveries(result_entry_id) WHERE result_entry_id IS NOT NULL",
    "CREATE INDEX IF NOT EXISTS idx_agena_delivery_unobserved ON agena_deliveries(session_id,target_execution_id,id) WHERE state='consumed' AND observed_round_id IS NULL AND (target_execution_id IS NOT NULL OR COALESCE(json_extract(payload_json,'$.wake'),1)!=0 OR json_extract(payload_json,'$.kind')='agent_message')",
    "CREATE INDEX IF NOT EXISTS idx_agena_delivery_observed_round ON agena_deliveries(observed_round_id) WHERE observed_round_id IS NOT NULL",
    "CREATE INDEX IF NOT EXISTS idx_agena_branch_entry_reverse ON agena_branch_entries(entry_id,session_id)",
    "CREATE INDEX IF NOT EXISTS idx_agena_branch_source_reverse ON agena_branch_sources(source_key)",
    "CREATE INDEX IF NOT EXISTS idx_agena_branch_source_shared ON agena_branch_sources(head_owner_session_id,source_key) WHERE head_owner_session_id IS NOT NULL",
    "CREATE INDEX IF NOT EXISTS idx_agena_branch_round_reverse ON agena_branch_rounds(round_id,session_id)",
    "CREATE INDEX IF NOT EXISTS idx_agena_branch_activity_reverse ON agena_branch_activities(activity_id,session_id)",
];

pub(super) const TRIGGERS: &[&str] = &[
    "CREATE TRIGGER IF NOT EXISTS agena_segment_immutable_insert BEFORE INSERT ON agena_segment_entries WHEN EXISTS (SELECT 1 FROM agena_session_segments WHERE segment_id=NEW.segment_id) OR EXISTS (SELECT 1 FROM agena_retired_session_segments WHERE segment_id=NEW.segment_id) BEGIN SELECT RAISE(ABORT,'published history segment is immutable'); END",
    "CREATE TRIGGER IF NOT EXISTS agena_segment_immutable_update BEFORE UPDATE ON agena_segment_entries WHEN EXISTS (SELECT 1 FROM agena_session_segments WHERE segment_id=OLD.segment_id OR segment_id=NEW.segment_id) OR EXISTS (SELECT 1 FROM agena_retired_session_segments WHERE segment_id=OLD.segment_id OR segment_id=NEW.segment_id) BEGIN SELECT RAISE(ABORT,'published history segment is immutable'); END",
    "CREATE TRIGGER IF NOT EXISTS agena_segment_immutable_delete BEFORE DELETE ON agena_segment_entries WHEN EXISTS (SELECT 1 FROM agena_session_segments WHERE segment_id=OLD.segment_id) OR EXISTS (SELECT 1 FROM agena_retired_session_segments WHERE segment_id=OLD.segment_id) BEGIN SELECT RAISE(ABORT,'published history segment is immutable'); END",
    // Disjoint intervals have ordered ends as well as starts. The last block
    // starting before the new end is the only one we need to check.
    "CREATE TRIGGER IF NOT EXISTS agena_content_block_no_overlap BEFORE INSERT ON agena_content_blocks WHEN NEW.logical_start < (SELECT logical_end FROM agena_content_blocks WHERE source_key=NEW.source_key AND logical_start<NEW.logical_end ORDER BY logical_start DESC LIMIT 1) BEGIN SELECT RAISE(ABORT,'content block ranges overlap'); END",
    "CREATE TRIGGER IF NOT EXISTS agena_capture_loss_immutable BEFORE UPDATE ON agena_content_capture_losses BEGIN SELECT RAISE(ABORT,'capture loss window is immutable'); END",
    "CREATE TRIGGER IF NOT EXISTS agena_content_block_immutable BEFORE UPDATE ON agena_content_blocks BEGIN SELECT RAISE(ABORT,'content block is immutable'); END",
    "CREATE TRIGGER IF NOT EXISTS agena_content_block_piece_bounds BEFORE INSERT ON agena_content_block_pieces
        WHEN NOT EXISTS(SELECT 1 FROM agena_content_blocks WHERE source_key=NEW.source_key
            AND logical_start=NEW.block_start AND body IS NULL AND length(NEW.body)=MIN(65536,body_bytes-NEW.byte_offset))
        BEGIN SELECT RAISE(ABORT,'content block piece is outside its bounds'); END",
    "CREATE TRIGGER IF NOT EXISTS agena_content_block_piece_immutable BEFORE UPDATE ON agena_content_block_pieces BEGIN SELECT RAISE(ABORT,'content block piece is immutable'); END",
    "CREATE TRIGGER IF NOT EXISTS agena_blob_chunk_immutable BEFORE UPDATE ON agena_blob_chunks BEGIN SELECT RAISE(ABORT,'blob chunk is immutable'); END",
    "CREATE TRIGGER IF NOT EXISTS agena_blob_ready_reference BEFORE INSERT ON agena_blob_references WHEN NOT EXISTS(SELECT 1 FROM agena_blobs WHERE id=NEW.blob_id AND state='ready') BEGIN SELECT RAISE(ABORT,'blob is not ready'); END",
    "CREATE TRIGGER IF NOT EXISTS agena_blob_reference_protect AFTER INSERT ON agena_blob_references
        BEGIN UPDATE agena_blobs SET unprotected=0 WHERE id=NEW.blob_id AND unprotected=1; END",
    "CREATE TRIGGER IF NOT EXISTS agena_blob_pin_protect AFTER INSERT ON agena_blob_pins
        BEGIN UPDATE agena_blobs SET unprotected=0 WHERE id=NEW.blob_id AND unprotected=1; END",
    "CREATE TRIGGER IF NOT EXISTS agena_blob_reference_unprotect AFTER DELETE ON agena_blob_references
        WHEN NOT EXISTS(SELECT 1 FROM agena_blob_references WHERE blob_id=OLD.blob_id)
         AND NOT EXISTS(SELECT 1 FROM agena_blob_pins WHERE blob_id=OLD.blob_id)
        BEGIN UPDATE agena_blobs SET unprotected=1 WHERE id=OLD.blob_id AND unprotected=0; END",
    "CREATE TRIGGER IF NOT EXISTS agena_blob_pin_unprotect AFTER DELETE ON agena_blob_pins
        WHEN NOT EXISTS(SELECT 1 FROM agena_blob_pins WHERE blob_id=OLD.blob_id)
         AND NOT EXISTS(SELECT 1 FROM agena_blob_references WHERE blob_id=OLD.blob_id)
        BEGIN UPDATE agena_blobs SET unprotected=1 WHERE id=OLD.blob_id AND unprotected=0; END",
    "CREATE TRIGGER IF NOT EXISTS agena_branch_source_share_direct BEFORE INSERT ON agena_branch_sources
        WHEN NEW.head_owner_session_id IS NOT NULL AND NOT EXISTS(
            SELECT 1 FROM agena_branch_sources h WHERE h.session_id=NEW.head_owner_session_id
                AND h.source_key=NEW.source_key AND h.head_owner_session_id IS NULL
        ) BEGIN SELECT RAISE(ABORT,'shared branch source must reference a frozen head'); END",
];

fn workspace_statistics_visibility(row: &str) -> String {
    format!(
        "{row}.deleted_at_ms IS NULL AND {row}.temporary=0
         AND {row}.owner_bot_id IS NULL"
    )
}

pub(super) fn overview_visibility(row: &str) -> String {
    let visible = workspace_statistics_visibility(row);
    format!("{visible} AND {row}.relation_kind!='subagent'")
}

const RUNNING_STATES: &str = "('running','awaiting_interaction')";
const ATTENTION_STATES: &str = "('awaiting_interaction','failed')";

fn workspace_count_assignments(sign: char, session: &str, stats: &str) -> String {
    format!(
        "SET total=total{sign}1,
             roots=roots{sign}({session}.parent_session_id IS NULL),
             pinned=agena_workspace_statistics.pinned{sign}{session}.pinned,
             running=running{sign}({stats}.processing_state IN {RUNNING_STATES}),
             attention=attention{sign}({stats}.processing_state IN {ATTENTION_STATES})"
    )
}

fn workspace_statistics_delta(sign: char, stats: &str) -> String {
    let assignments = workspace_count_assignments(sign, "s", stats);
    let visible = workspace_statistics_visibility("s");
    format!(
        "UPDATE agena_workspace_statistics {assignments}
         FROM agena_sessions s
         WHERE agena_workspace_statistics.workspace_id=s.workspace_id
           AND s.id={stats}.session_id AND {visible};"
    )
}

fn workspace_statistics_triggers() -> [String; 5] {
    let scope = |row: &str| {
        let visible = workspace_statistics_visibility(row);
        format!("CASE WHEN {visible} THEN {row}.workspace_id END")
    };
    let old_scope = scope("OLD");
    let new_scope = scope("NEW");
    let remove_statistics = workspace_statistics_delta('-', "OLD");
    let remove_session = workspace_count_assignments('-', "OLD", "t");
    let visible_session = workspace_statistics_visibility("s");
    [
        "CREATE TRIGGER IF NOT EXISTS agena_workspace_statistics_create
         AFTER INSERT ON agena_workspaces BEGIN
             INSERT INTO agena_workspace_statistics(workspace_id) VALUES(NEW.id);
         END"
            .to_owned(),
        format!(
            "CREATE TRIGGER IF NOT EXISTS agena_workspace_statistics_session_update
             AFTER UPDATE OF workspace_id,deleted_at_ms,configuration_json,parent_session_id,pinned ON agena_sessions
             WHEN ({old_scope}) IS NOT ({new_scope}) OR (
                 ({new_scope}) IS NOT NULL AND
                 ((OLD.parent_session_id IS NULL)!=(NEW.parent_session_id IS NULL) OR OLD.pinned!=NEW.pinned)
             ) BEGIN
                 UPDATE agena_workspace_statistics SET
                     total=total+(workspace_id IS new_id)-(workspace_id IS old_id),
                     roots=roots+((workspace_id IS new_id) AND NEW.parent_session_id IS NULL)
                                -((workspace_id IS old_id) AND OLD.parent_session_id IS NULL),
                     pinned=pinned+(workspace_id IS new_id)*NEW.pinned-(workspace_id IS old_id)*OLD.pinned,
                     running=running+((workspace_id IS new_id)-(workspace_id IS old_id))
                                     *(t.processing_state IN {RUNNING_STATES}),
                     attention=attention+((workspace_id IS new_id)-(workspace_id IS old_id))
                                         *(t.processing_state IN {ATTENTION_STATES})
                 FROM agena_session_statistics t,
                      (SELECT {old_scope} AS old_id,{new_scope} AS new_id)
                 WHERE t.session_id=NEW.id AND workspace_id IN (old_id,new_id);
             END"
        ),
        format!(
            "CREATE TRIGGER IF NOT EXISTS agena_workspace_statistics_session_delete
             AFTER DELETE ON agena_sessions WHEN ({old_scope}) IS NOT NULL BEGIN
                 UPDATE agena_workspace_statistics {remove_session}
                 FROM agena_session_statistics t
                 WHERE t.session_id=OLD.id AND workspace_id=OLD.workspace_id;
             END"
        ),
        format!(
            "CREATE TRIGGER IF NOT EXISTS agena_workspace_statistics_statistics_delete
             AFTER DELETE ON agena_session_statistics BEGIN {remove_statistics} END"
        ),
        format!(
            "CREATE TRIGGER IF NOT EXISTS agena_workspace_statistics_state
             AFTER UPDATE OF processing_state ON agena_session_statistics
             WHEN OLD.processing_state!=NEW.processing_state BEGIN
                 UPDATE agena_workspace_statistics SET
                     running=running+(NEW.processing_state IN {RUNNING_STATES})
                                    -(OLD.processing_state IN {RUNNING_STATES}),
                     attention=attention+(NEW.processing_state IN {ATTENTION_STATES})
                                        -(OLD.processing_state IN {ATTENTION_STATES})
                 WHERE workspace_id=(SELECT s.workspace_id FROM agena_sessions s
                                     WHERE s.id=NEW.session_id AND {visible_session});
             END"
        ),
    ]
}

// Listings read only compact facts. These projections change in the same
// transaction as their source facts, including recovery and archive import.
fn summary_triggers() -> Vec<String> {
    const STATE: &str = "CASE WHEN EXISTS(SELECT 1 FROM agena_interactions i JOIN agena_tool_calls c ON c.id=i.tool_id JOIN agena_executions x ON x.id=c.execution_id JOIN agena_attempts a ON a.execution_id=x.id AND a.generation=x.current_generation WHERE i.session_id=s.id AND i.decision_json IS NULL AND c.generation=x.current_generation AND a.phase='running' AND c.phase='running') THEN 'awaiting_interaction' WHEN t.queued_executions+t.running_executions>0 THEN 'running' WHEN (SELECT a.phase FROM agena_executions x JOIN agena_attempts a ON a.execution_id=x.id AND a.generation=x.current_generation WHERE x.session_id=s.id ORDER BY x.message_id DESC LIMIT 1) IN ('failed','interrupted') THEN 'failed' ELSE 'ready' END";
    let overview_workspace = |row: &str| {
        let visibility = overview_visibility(row);
        format!("CASE WHEN {visibility} THEN {row}.workspace_id END")
    };
    let initial_workspace = overview_workspace("s");
    let old_workspace = overview_workspace("OLD");
    let new_workspace = overview_workspace("NEW");
    // Register the inserted state before deriving its replacement. A state
    // change then adjusts the same counters as it does for any later update.
    let add_statistics = workspace_statistics_delta('+', "NEW");
    let mut triggers = vec![
        format!(
            "CREATE TRIGGER IF NOT EXISTS agena_summary_statistics_insert
             AFTER INSERT ON agena_session_statistics BEGIN
                 {add_statistics}
                 UPDATE agena_session_statistics SET (processing_state,overview_workspace_id)=(
                     SELECT {STATE},{initial_workspace}
                     FROM agena_sessions s JOIN agena_session_statistics t ON t.session_id=s.id
                     WHERE s.id=NEW.session_id
                 ) WHERE session_id=NEW.session_id;
             END"
        ),
        format!(
            "CREATE TRIGGER IF NOT EXISTS agena_summary_overview_workspace
             AFTER UPDATE OF workspace_id,deleted_at_ms,configuration_json,relation_kind ON agena_sessions
             WHEN ({old_workspace}) IS NOT ({new_workspace}) BEGIN
                 UPDATE agena_session_statistics SET overview_workspace_id={new_workspace}
                 WHERE session_id=NEW.id;
             END"
        ),
    ];
    for (name, event, session) in [
        (
            "statistics_counts",
            "AFTER UPDATE OF queued_executions,running_executions ON agena_session_statistics",
            "NEW.session_id",
        ),
        (
            "attempt_insert",
            "AFTER INSERT ON agena_attempts",
            "NEW.session_id",
        ),
        (
            "attempt_phase",
            "AFTER UPDATE OF phase ON agena_attempts",
            "NEW.session_id",
        ),
        (
            "execution_generation",
            "AFTER UPDATE OF current_generation ON agena_executions",
            "NEW.session_id",
        ),
        (
            "interaction_insert",
            "AFTER INSERT ON agena_interactions",
            "NEW.session_id",
        ),
        (
            "interaction_decision",
            "AFTER UPDATE OF decision_json ON agena_interactions",
            "NEW.session_id",
        ),
        (
            "interaction_delete",
            "AFTER DELETE ON agena_interactions",
            "OLD.session_id",
        ),
        (
            "tool_phase",
            "AFTER UPDATE OF phase ON agena_tool_calls",
            "(SELECT session_id FROM agena_executions WHERE id=NEW.execution_id)",
        ),
    ] {
        triggers.push(format!("CREATE TRIGGER IF NOT EXISTS agena_summary_{name} {event} BEGIN UPDATE agena_session_statistics SET processing_state=(SELECT {STATE} FROM agena_sessions s JOIN agena_session_statistics t ON t.session_id=s.id WHERE s.id={session}) WHERE session_id={session}; END"));
    }
    triggers.extend([
        "CREATE TRIGGER IF NOT EXISTS agena_summary_child_insert AFTER INSERT ON agena_sessions WHEN NEW.parent_session_id IS NOT NULL AND NEW.deleted_at_ms IS NULL AND NEW.temporary=0 BEGIN UPDATE agena_session_statistics SET child_session_count=child_session_count+1 WHERE session_id=NEW.parent_session_id; END".to_owned(),
        "CREATE TRIGGER IF NOT EXISTS agena_summary_child_update AFTER UPDATE OF parent_session_id,deleted_at_ms,configuration_json ON agena_sessions WHEN OLD.parent_session_id IS NOT NEW.parent_session_id OR (OLD.deleted_at_ms IS NULL)!=(NEW.deleted_at_ms IS NULL) OR OLD.temporary!=NEW.temporary BEGIN UPDATE agena_session_statistics SET child_session_count=child_session_count-1 WHERE session_id=OLD.parent_session_id AND OLD.deleted_at_ms IS NULL AND OLD.temporary=0; UPDATE agena_session_statistics SET child_session_count=child_session_count+1 WHERE session_id=NEW.parent_session_id AND NEW.deleted_at_ms IS NULL AND NEW.temporary=0; END".to_owned(),
        "CREATE TRIGGER IF NOT EXISTS agena_summary_child_delete AFTER DELETE ON agena_sessions WHEN OLD.parent_session_id IS NOT NULL AND OLD.deleted_at_ms IS NULL AND OLD.temporary=0 BEGIN UPDATE agena_session_statistics SET child_session_count=child_session_count-1 WHERE session_id=OLD.parent_session_id; END".to_owned(),
        "CREATE TRIGGER IF NOT EXISTS agena_summary_child_initial AFTER INSERT ON agena_session_statistics BEGIN UPDATE agena_session_statistics SET child_session_count=(SELECT count(*) FROM agena_sessions child WHERE child.parent_session_id=NEW.session_id AND child.deleted_at_ms IS NULL AND child.temporary=0) WHERE session_id=NEW.session_id; END".to_owned(),
    ]);
    triggers
}

fn declarations() -> impl Iterator<Item = String> {
    ARCHIVE_TABLES
        .iter()
        .chain(TABLES)
        .chain(VIEWS)
        .chain(ARCHIVE_INDEXES)
        .chain(INDEXES)
        .chain(TRIGGERS)
        .chain(crate::schema::TABLES)
        .chain(crate::schema::INDEXES)
        .chain(crate::companion_repository::DECLARATIONS)
        .chain(crate::integration_repository::DECLARATIONS)
        .chain(crate::integration_repository::CHANNEL_DECLARATIONS)
        .chain(crate::integration_repository::PARTICIPANT_DECLARATIONS)
        .copied()
        .chain(agena_scheduler::schema::declarations())
        .map(str::to_owned)
        .chain(std::iter::once(crate::schema::instance_table()))
        .chain(summary_triggers())
        .chain(workspace_statistics_triggers())
        .chain(super::usage::indexes().map(|(_, sql)| sql))
}

fn object_map(declarations: impl Iterator<Item = String>) -> Result<SchemaObjects, DbErr> {
    declarations
        .map(|sql| {
            let d = declaration(&sql)?;
            Ok(((d.kind.to_owned(), d.name.to_owned()), d.stored_sql))
        })
        .collect()
}

fn incompatible_database(reason: &str) -> DbErr {
    DbErr::Custom(format!(
        "{reason}; start Agena with a new empty data directory"
    ))
}

type SchemaObjects = std::collections::BTreeMap<(String, String), String>;

async fn validate_objects(
    database: &DatabaseConnection,
    actual: SchemaObjects,
) -> Result<(), DbErr> {
    if object_map(declarations())? != actual {
        return Err(incompatible_database(
            "database schema does not match the current Agena baseline",
        ));
    }
    instance_id(database).await?;
    Ok(())
}

/// A read-only probe admits only the complete current schema.
pub(crate) async fn validate_existing(database: &DatabaseConnection) -> Result<(), DbErr> {
    validate_objects(database, schema_objects(database).await?).await
}

pub(crate) async fn initialize(database: &DatabaseConnection) -> Result<(), DbErr> {
    let actual = schema_objects(database).await?;
    if !actual.is_empty() {
        return validate_objects(database, actual).await;
    }
    // The full schema and baseline identity reach the main database together,
    // before the connection owner enables WAL. Immutable preflight reads can
    // therefore identify this baseline even after an uncheckpointed WAL crash.
    let transaction = database.begin().await?;
    for sql in declarations().chain(
        ["INSERT INTO agena_commits VALUES (1,0)"]
            .into_iter()
            .map(str::to_owned),
    ) {
        transaction
            .execute(Statement::from_string(database.get_database_backend(), sql))
            .await?;
    }
    transaction
        .execute(Statement::from_sql_and_values(
            database.get_database_backend(),
            "INSERT INTO agena_instance (id, schema_baseline, instance_id, created_at_ms) VALUES (1, ?, ?, ?)",
            [
                crate::schema::SCHEMA_BASELINE.into(),
                uuid::Uuid::new_v4().to_string().into(),
                chrono::Utc::now().timestamp_millis().into(),
            ],
        ))
        .await?;
    transaction.commit().await
}

pub(crate) async fn instance_id(database: &DatabaseConnection) -> Result<String, DbErr> {
    let row = database
        .query_one(Statement::from_string(
            database.get_database_backend(),
            "SELECT schema_baseline, instance_id FROM agena_instance WHERE id=1",
        ))
        .await?
        .ok_or_else(|| incompatible_database("instance identity is missing"))?;
    let baseline: String = row.try_get("", "schema_baseline")?;
    if baseline != crate::schema::SCHEMA_BASELINE {
        return Err(incompatible_database(
            "database identity does not match the current Agena baseline",
        ));
    }
    let id: String = row.try_get("", "instance_id")?;
    let parsed = uuid::Uuid::parse_str(&id)
        .map_err(|_| incompatible_database("instance identity is invalid"))?;
    if parsed.get_version_num() != 4 || parsed.to_string() != id {
        return Err(incompatible_database("instance identity is invalid"));
    }
    Ok(id)
}

#[cfg(test)]
mod nullable_tests {
    use super::*;
    use sea_orm::Database;

    async fn exec(db: &impl ConnectionTrait, sql: &str) {
        db.execute(Statement::from_string(
            db.get_database_backend(),
            sql.to_owned(),
        ))
        .await
        .unwrap();
    }
    async fn scalar(db: &impl ConnectionTrait, sql: &str) -> i64 {
        db.query_one(Statement::from_string(
            db.get_database_backend(),
            sql.to_owned(),
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get("", "n")
        .unwrap()
    }
    #[tokio::test]
    async fn foreign_schema_is_rejected_without_modifying_database() {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        initialize(&db).await.unwrap();
        exec(&db, "DROP TABLE agena_bot_avatars").await;
        exec(&db, "CREATE TABLE unexpected_object(id INTEGER)").await;
        let before = schema_objects(&db).await.unwrap();
        let error = initialize(&db).await.unwrap_err().to_string();
        assert!(error.contains("new empty data directory"), "{error}");
        assert_eq!(schema_objects(&db).await.unwrap(), before);
        assert!(validate_existing(&db).await.is_err());
    }
    #[tokio::test]
    async fn missing_table_is_rejected_without_repair() {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        initialize(&db).await.unwrap();
        exec(&db, "DROP TABLE agena_bot_avatars").await;
        let before = schema_objects(&db).await.unwrap();
        assert!(initialize(&db).await.is_err());
        assert_eq!(schema_objects(&db).await.unwrap(), before);
    }

    #[tokio::test]
    async fn content_blocks_accept_adjacent_ranges_and_reject_overlaps() {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        initialize(&db).await.unwrap();
        exec(&db, "PRAGMA foreign_keys=OFF").await;
        let insert = |content, start, end| {
            Statement::from_sql_and_values(
                db.get_database_backend(),
                "INSERT INTO agena_content_blocks VALUES (?,?,?,'utf8',zeroblob(32),1,x'01')",
                [sea_orm::Value::from(content), start, end],
            )
        };
        for (content, start, end) in [(1, 0, 10), (1, 20, 30), (1, 10, 20), (2, 5, 25)] {
            db.execute(insert(content, start.into(), end.into()))
                .await
                .unwrap();
        }
        for (start, end) in [(1, 2), (5, 25), (29, 35), (20, 30)] {
            let error = db
                .execute(insert(1, start.into(), end.into()))
                .await
                .unwrap_err();
            assert!(
                error.to_string().contains("content block ranges overlap"),
                "{error}"
            );
        }
    }

    #[tokio::test]
    async fn fresh_schema_requires_no_workspace() {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        initialize(&db).await.unwrap();
        exec(&db, "INSERT INTO agena_sessions(workspace_id,title,created_at_ms,updated_at_ms) VALUES(NULL,'global',1,1)").await;
        assert_eq!(
            scalar(&db, "SELECT count(*) AS n FROM agena_workspaces").await,
            0
        );
        initialize(&db).await.unwrap();
    }

    #[tokio::test]
    async fn overview_index_tracks_workspace_and_visibility_changes() {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        initialize(&db).await.unwrap();
        exec(
            &db,
            "INSERT INTO agena_workspaces(id,path,created_at_ms,updated_at_ms)
             VALUES(1,'/first',0,0),(2,'/second',0,0)",
        )
        .await;
        exec(
            &db,
            "INSERT INTO agena_sessions(id,workspace_id,title,created_at_ms,updated_at_ms)
             VALUES(1,1,'test',0,0)",
        )
        .await;
        exec(
            &db,
            "INSERT INTO agena_session_statistics(session_id) VALUES(1)",
        )
        .await;
        let indexed = "SELECT COALESCE(MAX(overview_workspace_id),0) AS n
                       FROM agena_session_statistics INDEXED BY idx_agena_workspace_attention
                       WHERE overview_workspace_id IS NOT NULL AND processing_state!='ready'";
        assert_eq!(scalar(&db, indexed).await, 0);
        for (change, expected) in [
            ("UPDATE agena_session_statistics SET queued_executions=1", 1),
            ("UPDATE agena_sessions SET workspace_id=2", 2),
            (
                r#"UPDATE agena_sessions SET configuration_json='{"conversation":{"mode":"btw"}}'"#,
                0,
            ),
            ("UPDATE agena_sessions SET configuration_json='{}'", 2),
            (
                r#"UPDATE agena_sessions SET configuration_json='{"owner":{"bot":"00000000-0000-4000-8000-000000000001","surface":"local"}}'"#,
                0,
            ),
            ("UPDATE agena_sessions SET configuration_json='{}'", 2),
            ("UPDATE agena_sessions SET relation_kind='subagent'", 0),
            ("UPDATE agena_sessions SET relation_kind='root'", 2),
            ("UPDATE agena_sessions SET workspace_id=NULL", 0),
            ("UPDATE agena_sessions SET workspace_id=1", 1),
            ("UPDATE agena_session_statistics SET queued_executions=0", 0),
            ("UPDATE agena_session_statistics SET queued_executions=1", 1),
            ("UPDATE agena_sessions SET deleted_at_ms=1", 0),
        ] {
            exec(&db, change).await;
            assert_eq!(scalar(&db, indexed).await, expected, "{change}");
        }
    }

    #[tokio::test]
    async fn workspace_counters_match_visible_session_aggregation() {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        initialize(&db).await.unwrap();
        let mismatch = "WITH expected AS (
            SELECT w.id,count(t.session_id),
                   count(CASE WHEN t.session_id IS NOT NULL AND s.parent_session_id IS NULL THEN 1 END),
                   count(CASE WHEN t.session_id IS NOT NULL AND s.pinned=1 THEN 1 END),
                   count(CASE WHEN t.processing_state IN ('running','awaiting_interaction') THEN 1 END),
                   count(CASE WHEN t.processing_state IN ('awaiting_interaction','failed') THEN 1 END)
            FROM agena_workspaces w
            LEFT JOIN agena_sessions s ON s.workspace_id=w.id
                 AND s.deleted_at_ms IS NULL AND s.temporary=0 AND s.owner_bot_id IS NULL
            LEFT JOIN agena_session_statistics t ON t.session_id=s.id
            GROUP BY w.id
        )
        SELECT count(*) AS n FROM (
            SELECT * FROM expected EXCEPT SELECT * FROM agena_workspace_statistics
        )";
        for change in [
            "INSERT INTO agena_workspaces(id,path,created_at_ms,updated_at_ms)
             VALUES(1,'/one',0,0),(2,'/two',0,0),(3,'/three',0,0)",
            "INSERT INTO agena_sessions(id,workspace_id,title,parent_session_id,relation_kind,created_at_ms,updated_at_ms)
             VALUES(1,1,'root',NULL,'root',0,0),(2,1,'child',1,'subagent',0,0)",
            // The inserted child state is recalculated to running in the insert trigger.
            "INSERT INTO agena_session_statistics(session_id,queued_executions) VALUES(1,0),(2,1)",
            "UPDATE agena_sessions SET pinned=1 WHERE id=2",
            "UPDATE agena_sessions SET parent_session_id=NULL WHERE id=2",
            "UPDATE agena_sessions SET workspace_id=2,pinned=0,parent_session_id=1 WHERE id=2",
            "UPDATE agena_session_statistics SET processing_state='awaiting_interaction' WHERE session_id=2",
            "UPDATE agena_session_statistics SET processing_state='failed' WHERE session_id=2",
            r#"UPDATE agena_sessions SET configuration_json='{"conversation":{"mode":"btw"}}' WHERE id=2"#,
            "UPDATE agena_session_statistics SET queued_executions=0 WHERE session_id=2",
            "UPDATE agena_sessions SET configuration_json='{}',pinned=1 WHERE id=2",
            r#"UPDATE agena_sessions SET configuration_json='{"owner":{"bot":"00000000-0000-4000-8000-000000000001","surface":"local"}}' WHERE id=2"#,
            "UPDATE agena_sessions SET configuration_json='{}' WHERE id=2",
            "UPDATE agena_sessions SET deleted_at_ms=1 WHERE id=2",
            "UPDATE agena_sessions SET deleted_at_ms=NULL,workspace_id=NULL WHERE id=2",
            "UPDATE agena_sessions SET workspace_id=1 WHERE id=2",
            "DELETE FROM agena_session_statistics WHERE session_id=2",
            "DELETE FROM agena_sessions WHERE id=2",
            "DELETE FROM agena_workspaces WHERE id=3",
        ] {
            exec(&db, change).await;
            assert_eq!(scalar(&db, mismatch).await, 0, "{change}");
        }
        assert_eq!(
            scalar(&db, "SELECT count(*) AS n FROM agena_workspace_statistics").await,
            2
        );
    }
}
