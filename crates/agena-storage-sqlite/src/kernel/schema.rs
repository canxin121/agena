//! The new kernel's durable declarations. Never installed beside the old Part
//! schema: each database has one exact schema and one fact authority.

use crate::schema::validation::{declaration, schema_objects};
use sea_orm::{ConnectionTrait, DatabaseConnection, DbErr, Statement, TransactionTrait};

pub(super) const TABLES: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS agena_background_activities (id TEXT PRIMARY KEY, session_id INTEGER NOT NULL, execution_id TEXT NOT NULL, generation INTEGER NOT NULL, round_id INTEGER NOT NULL, tool_id INTEGER NOT NULL, entry_id INTEGER NOT NULL, owner_epoch TEXT NOT NULL, kind TEXT NOT NULL CHECK(kind IN ('process','terminal','agent','monitor')), label TEXT NOT NULL CHECK(length(CAST(label AS BLOB)) BETWEEN 1 AND 1024), phase TEXT NOT NULL CHECK(phase IN ('running','succeeded','failed','stopped','interrupted')), outcome_json TEXT CHECK(outcome_json IS NULL OR (json_valid(outcome_json) AND length(CAST(outcome_json AS BLOB))<=65536)), created_at_ms INTEGER NOT NULL, finished_at_ms INTEGER, UNIQUE(id,entry_id,session_id,execution_id,generation), FOREIGN KEY(tool_id,execution_id,generation,round_id) REFERENCES agena_tool_calls(id,execution_id,generation,round_id), FOREIGN KEY(entry_id,session_id,execution_id,generation) REFERENCES agena_entries(id,origin_session_id,execution_id,generation), CHECK((phase='running' AND outcome_json IS NULL AND finished_at_ms IS NULL) OR (phase!='running' AND outcome_json IS NOT NULL AND json_extract(outcome_json,'$.outcome') IS NOT NULL AND json_extract(outcome_json,'$.outcome')=phase AND finished_at_ms IS NOT NULL AND finished_at_ms>=created_at_ms)))",
    "CREATE TABLE IF NOT EXISTS agena_sessions (id INTEGER PRIMARY KEY AUTOINCREMENT, workspace_id INTEGER NOT NULL REFERENCES agena_workspaces(id), title TEXT NOT NULL, parent_session_id INTEGER REFERENCES agena_sessions(id), root_session_id INTEGER REFERENCES agena_sessions(id), depth INTEGER NOT NULL DEFAULT 0 CHECK(typeof(depth)='integer' AND depth>=0), relation_kind TEXT NOT NULL DEFAULT 'root' CHECK(relation_kind IN ('root','child','fork','rewind','subagent')), configuration_json TEXT NOT NULL DEFAULT '{}' CHECK(json_valid(configuration_json) AND length(CAST(configuration_json AS BLOB))<=65536), agent_name TEXT GENERATED ALWAYS AS (json_extract(configuration_json,'$.agent.name')) STORED, agent_status TEXT GENERATED ALWAYS AS (json_extract(configuration_json,'$.agent.status')) STORED, temporary INTEGER GENERATED ALWAYS AS (COALESCE(json_extract(configuration_json,'$.conversation.mode'),'')='btw') STORED, favorite INTEGER NOT NULL DEFAULT 0 CHECK(favorite IN (0,1)), pinned INTEGER NOT NULL DEFAULT 0 CHECK(pinned IN (0,1)), selection_json TEXT CHECK(selection_json IS NULL OR json_valid(selection_json)), metadata_version INTEGER NOT NULL DEFAULT 1 CHECK(typeof(metadata_version)='integer' AND metadata_version>0), membership_generation INTEGER NOT NULL DEFAULT 1 CHECK(typeof(membership_generation)='integer' AND membership_generation>0), control_version INTEGER NOT NULL DEFAULT 1 CHECK(typeof(control_version)='integer' AND control_version>0), created_at_ms INTEGER NOT NULL, updated_at_ms INTEGER NOT NULL, deleted_at_ms INTEGER, CHECK(updated_at_ms>=created_at_ms))",
    "CREATE TABLE IF NOT EXISTS agena_session_statistics (session_id INTEGER PRIMARY KEY REFERENCES agena_sessions(id), entry_count INTEGER NOT NULL DEFAULT 0 CHECK(typeof(entry_count)='integer' AND entry_count>=0), user_message_count INTEGER NOT NULL DEFAULT 0 CHECK(typeof(user_message_count)='integer' AND user_message_count>=0), queued_executions INTEGER NOT NULL DEFAULT 0 CHECK(typeof(queued_executions)='integer' AND queued_executions>=0), running_executions INTEGER NOT NULL DEFAULT 0 CHECK(typeof(running_executions)='integer' AND running_executions>=0), last_message_at_ms INTEGER, child_session_count INTEGER NOT NULL DEFAULT 0 CHECK(typeof(child_session_count)='integer' AND child_session_count>=0), processing_state TEXT NOT NULL DEFAULT 'ready' CHECK(processing_state IN ('ready','running','awaiting_interaction','failed')))",
    "CREATE TABLE IF NOT EXISTS agena_executions (id TEXT PRIMARY KEY, session_id INTEGER NOT NULL REFERENCES agena_sessions(id), message_id INTEGER NOT NULL REFERENCES agena_entries(id) DEFERRABLE INITIALLY DEFERRED, current_generation INTEGER NOT NULL CHECK(typeof(current_generation)='integer' AND current_generation>0), created_at_ms INTEGER NOT NULL, UNIQUE(id,session_id), FOREIGN KEY(id,current_generation) REFERENCES agena_attempts(execution_id,generation) DEFERRABLE INITIALLY DEFERRED)",
    "CREATE TABLE IF NOT EXISTS agena_attempts (execution_id TEXT NOT NULL, generation INTEGER NOT NULL CHECK(typeof(generation)='integer' AND generation>0), session_id INTEGER NOT NULL, owner_epoch TEXT, response_entry_id INTEGER REFERENCES agena_entries(id), stop_requested INTEGER NOT NULL DEFAULT 0 CHECK(stop_requested IN (0,1)), phase TEXT NOT NULL CHECK(phase IN ('queued','running','succeeded','failed','cancelled','interrupted')), started_at_ms INTEGER, finished_at_ms INTEGER, PRIMARY KEY(execution_id,generation), UNIQUE(execution_id,generation,session_id), FOREIGN KEY(execution_id,session_id) REFERENCES agena_executions(id,session_id), CHECK((phase='queued' AND owner_epoch IS NULL AND started_at_ms IS NULL AND finished_at_ms IS NULL) OR (phase='running' AND owner_epoch IS NOT NULL AND started_at_ms IS NOT NULL AND finished_at_ms IS NULL) OR (phase IN ('succeeded','failed','cancelled','interrupted') AND finished_at_ms IS NOT NULL)), CHECK(finished_at_ms IS NULL OR started_at_ms IS NULL OR finished_at_ms>=started_at_ms)) WITHOUT ROWID",
    "CREATE TABLE IF NOT EXISTS agena_rounds (id INTEGER PRIMARY KEY AUTOINCREMENT, execution_id TEXT NOT NULL, generation INTEGER NOT NULL, session_id INTEGER NOT NULL, ordinal INTEGER NOT NULL CHECK(ordinal>=0), phase TEXT NOT NULL CHECK(phase IN ('running','succeeded','failed','interrupted')), provider_state_json TEXT CHECK(provider_state_json IS NULL OR json_valid(provider_state_json)), provider_state_bytes INTEGER GENERATED ALWAYS AS (COALESCE(length(CAST(provider_state_json AS BLOB)),0)) STORED, input_metadata_json TEXT NOT NULL CHECK(json_valid(input_metadata_json)), created_at_ms INTEGER NOT NULL, finished_at_ms INTEGER, UNIQUE(execution_id,generation,ordinal), UNIQUE(id,execution_id,generation), FOREIGN KEY(execution_id,generation,session_id) REFERENCES agena_attempts(execution_id,generation,session_id), CHECK((phase='running' AND finished_at_ms IS NULL) OR (phase!='running' AND finished_at_ms IS NOT NULL)))",
    "CREATE TABLE IF NOT EXISTS agena_tool_calls (id INTEGER PRIMARY KEY AUTOINCREMENT, round_id INTEGER NOT NULL, execution_id TEXT NOT NULL, generation INTEGER NOT NULL, call_id TEXT NOT NULL CHECK(length(call_id)>0), name TEXT NOT NULL CHECK(length(name)>0), input_json TEXT NOT NULL CHECK(json_valid(input_json)), input_bytes INTEGER GENERATED ALWAYS AS (length(CAST(input_json AS BLOB))) STORED, execution_kind TEXT NOT NULL DEFAULT 'runtime' CHECK(execution_kind IN ('runtime','provider_hosted')), phase TEXT NOT NULL CHECK(phase IN ('preparing','running','succeeded','failed','cancelled','interrupted')), result_json TEXT CHECK(result_json IS NULL OR json_valid(result_json)), result_bytes INTEGER GENERATED ALWAYS AS (COALESCE(length(CAST(result_json AS BLOB)),0)) STORED, created_at_ms INTEGER NOT NULL, finished_at_ms INTEGER, UNIQUE(round_id,call_id), UNIQUE(id,execution_id,generation,round_id), FOREIGN KEY(round_id,execution_id,generation) REFERENCES agena_rounds(id,execution_id,generation), CHECK((phase IN ('preparing','running') AND result_json IS NULL AND finished_at_ms IS NULL) OR (phase NOT IN ('preparing','running') AND result_json IS NOT NULL AND finished_at_ms IS NOT NULL)))",
    "CREATE TABLE IF NOT EXISTS agena_interactions (id INTEGER PRIMARY KEY AUTOINCREMENT, tool_id INTEGER NOT NULL REFERENCES agena_tool_calls(id), session_id INTEGER NOT NULL, execution_id TEXT NOT NULL, generation INTEGER NOT NULL CHECK(generation>0), request_kind TEXT NOT NULL CHECK(request_kind IN ('permission','input')), request_json TEXT NOT NULL CHECK(json_valid(request_json)), request_bytes INTEGER GENERATED ALWAYS AS (length(CAST(request_json AS BLOB))) STORED, decision_json TEXT CHECK(decision_json IS NULL OR json_valid(decision_json)), decision_bytes INTEGER GENERATED ALWAYS AS (COALESCE(length(CAST(decision_json AS BLOB)),0)) STORED, remember_json TEXT CHECK(remember_json IS NULL OR json_valid(remember_json)), remember_bytes INTEGER GENERATED ALWAYS AS (COALESCE(length(CAST(remember_json AS BLOB)),0)) STORED, created_at_ms INTEGER NOT NULL, decided_at_ms INTEGER, UNIQUE(id,tool_id), FOREIGN KEY(tool_id,execution_id,generation) REFERENCES agena_tool_calls(id,execution_id,generation), FOREIGN KEY(execution_id,generation,session_id) REFERENCES agena_attempts(execution_id,generation,session_id), CHECK((decision_json IS NULL AND decided_at_ms IS NULL) OR (decision_json IS NOT NULL AND decided_at_ms IS NOT NULL)))",
    "CREATE TABLE IF NOT EXISTS agena_entries (id INTEGER PRIMARY KEY AUTOINCREMENT, origin_session_id INTEGER NOT NULL REFERENCES agena_sessions(id), execution_id TEXT NOT NULL, generation INTEGER NOT NULL, message_id INTEGER REFERENCES agena_entries(id), revision INTEGER NOT NULL DEFAULT 1 CHECK(typeof(revision)='integer' AND revision>0), kind TEXT NOT NULL CHECK(kind IN ('user','text','reasoning','tool','interaction','notice','compaction')), body_json TEXT CHECK(body_json IS NULL OR json_valid(body_json)), body_bytes INTEGER GENERATED ALWAYS AS (COALESCE(length(CAST(body_json AS BLOB)),0)) STORED, round_id INTEGER, tool_id INTEGER REFERENCES agena_tool_calls(id), interaction_id INTEGER REFERENCES agena_interactions(id), content_id TEXT REFERENCES agena_content_streams(id) DEFERRABLE INITIALLY DEFERRED, created_at_ms INTEGER NOT NULL, UNIQUE(id,origin_session_id,execution_id,generation), FOREIGN KEY(execution_id,generation,origin_session_id) REFERENCES agena_attempts(execution_id,generation,session_id) DEFERRABLE INITIALLY DEFERRED, FOREIGN KEY(round_id,execution_id,generation) REFERENCES agena_rounds(id,execution_id,generation), FOREIGN KEY(tool_id,execution_id,generation,round_id) REFERENCES agena_tool_calls(id,execution_id,generation,round_id), FOREIGN KEY(interaction_id,tool_id) REFERENCES agena_interactions(id,tool_id), CHECK((kind='user' AND message_id IS NULL AND body_json IS NOT NULL AND round_id IS NULL AND tool_id IS NULL AND interaction_id IS NULL AND content_id IS NULL) OR (kind IN ('text','reasoning') AND message_id IS NOT NULL AND round_id IS NOT NULL AND content_id IS NOT NULL AND body_json IS NULL AND tool_id IS NULL AND interaction_id IS NULL) OR (kind='tool' AND message_id IS NOT NULL AND round_id IS NOT NULL AND tool_id IS NOT NULL AND body_json IS NULL AND interaction_id IS NULL AND content_id IS NULL) OR (kind='interaction' AND message_id IS NOT NULL AND round_id IS NOT NULL AND tool_id IS NOT NULL AND interaction_id IS NOT NULL AND body_json IS NULL AND content_id IS NULL) OR (kind IN ('notice','compaction') AND message_id IS NOT NULL AND body_json IS NOT NULL AND interaction_id IS NULL)))",
    "CREATE TABLE IF NOT EXISTS agena_round_inputs (round_id INTEGER NOT NULL REFERENCES agena_rounds(id), ordinal INTEGER NOT NULL CHECK(ordinal>=0), entry_id INTEGER NOT NULL REFERENCES agena_entries(id), entry_revision INTEGER NOT NULL CHECK(typeof(entry_revision)='integer' AND entry_revision>0), response_entry_id INTEGER REFERENCES agena_entries(id), execution_phase TEXT NOT NULL CHECK(execution_phase IN ('queued','running','succeeded','failed','cancelled','interrupted')), tool_phase TEXT CHECK(tool_phase IS NULL OR tool_phase IN ('preparing','running','succeeded','failed','cancelled','interrupted')), interaction_decided INTEGER NOT NULL CHECK(interaction_decided IN (0,1)), PRIMARY KEY(round_id,ordinal), UNIQUE(round_id,entry_id)) WITHOUT ROWID",
    "CREATE TABLE IF NOT EXISTS agena_round_input_sources (round_id INTEGER NOT NULL, entry_id INTEGER NOT NULL, content_id TEXT NOT NULL REFERENCES agena_content_streams(id), head_json TEXT NOT NULL CHECK(json_valid(head_json)), head_bytes INTEGER GENERATED ALWAYS AS (length(CAST(head_json AS BLOB))) STORED, PRIMARY KEY(round_id,content_id), FOREIGN KEY(round_id,entry_id) REFERENCES agena_round_inputs(round_id,entry_id)) WITHOUT ROWID",
    "CREATE TABLE IF NOT EXISTS agena_history_segments (id INTEGER PRIMARY KEY AUTOINCREMENT)",
    "CREATE TABLE IF NOT EXISTS agena_prompt_windows (session_id INTEGER PRIMARY KEY REFERENCES agena_sessions(id), rewrite_epoch INTEGER NOT NULL DEFAULT 1 CHECK(typeof(rewrite_epoch)='integer' AND rewrite_epoch>0), policy_key TEXT CHECK(policy_key IS NULL OR length(CAST(policy_key AS BLOB)) BETWEEN 1 AND 1024), consecutive_compaction_failures INTEGER NOT NULL DEFAULT 0 CHECK(typeof(consecutive_compaction_failures)='integer' AND consecutive_compaction_failures BETWEEN 0 AND 3), auto_compaction_disabled INTEGER NOT NULL DEFAULT 0 CHECK(auto_compaction_disabled IN (0,1)), native_compaction_unavailable INTEGER NOT NULL DEFAULT 0 CHECK(native_compaction_unavailable IN (0,1)), last_before_tokens INTEGER CHECK(last_before_tokens IS NULL OR (typeof(last_before_tokens)='integer' AND last_before_tokens>=0)), last_after_tokens INTEGER CHECK(last_after_tokens IS NULL OR (typeof(last_after_tokens)='integer' AND last_after_tokens>=0)), checkpoint_entry_id INTEGER REFERENCES agena_entries(id), through_entry_id INTEGER REFERENCES agena_entries(id), through_sequence INTEGER CHECK(through_sequence IS NULL OR through_sequence>=0), through_ordinal INTEGER CHECK(through_ordinal IS NULL OR through_ordinal BETWEEN 0 AND 127), CHECK((checkpoint_entry_id IS NULL AND through_entry_id IS NULL AND through_sequence IS NULL AND through_ordinal IS NULL) OR (checkpoint_entry_id IS NOT NULL AND through_entry_id IS NOT NULL AND through_sequence IS NOT NULL AND through_ordinal IS NOT NULL)))",
    "CREATE TABLE IF NOT EXISTS agena_compaction_rounds (round_id INTEGER PRIMARY KEY REFERENCES agena_rounds(id), base_checkpoint_entry_id INTEGER REFERENCES agena_entries(id), rewrite_epoch INTEGER NOT NULL CHECK(typeof(rewrite_epoch)='integer' AND rewrite_epoch>0))",
    "CREATE TABLE IF NOT EXISTS agena_round_input_replays (round_id INTEGER NOT NULL REFERENCES agena_rounds(id), source_round_id INTEGER NOT NULL REFERENCES agena_rounds(id), phase TEXT NOT NULL CHECK(phase IN ('running','succeeded','failed','interrupted')), has_provider_state INTEGER NOT NULL CHECK(has_provider_state IN (0,1)), PRIMARY KEY(round_id,source_round_id)) WITHOUT ROWID",
    "CREATE TABLE IF NOT EXISTS agena_segment_entries (segment_id INTEGER NOT NULL REFERENCES agena_history_segments(id) ON DELETE CASCADE, ordinal INTEGER NOT NULL CHECK(ordinal BETWEEN 0 AND 127), entry_id INTEGER NOT NULL REFERENCES agena_entries(id), PRIMARY KEY(segment_id,ordinal), UNIQUE(segment_id,entry_id)) WITHOUT ROWID",
    "CREATE TABLE IF NOT EXISTS agena_session_segments (session_id INTEGER NOT NULL REFERENCES agena_sessions(id), sequence INTEGER NOT NULL CHECK(typeof(sequence)='integer' AND sequence>=0), segment_id INTEGER NOT NULL REFERENCES agena_history_segments(id), first_ordinal INTEGER NOT NULL CHECK(first_ordinal BETWEEN 0 AND 127), end_ordinal INTEGER NOT NULL CHECK(end_ordinal BETWEEN 1 AND 128 AND end_ordinal>first_ordinal), since_generation INTEGER NOT NULL DEFAULT 1 CHECK(typeof(since_generation)='integer' AND since_generation>0), PRIMARY KEY(session_id,sequence)) WITHOUT ROWID",
    "CREATE TABLE IF NOT EXISTS agena_history_snapshots (token TEXT PRIMARY KEY, session_id INTEGER NOT NULL REFERENCES agena_sessions(id), generation INTEGER NOT NULL CHECK(typeof(generation)='integer' AND generation>0), owner_epoch TEXT NOT NULL, last_sequence INTEGER CHECK(last_sequence IS NULL OR last_sequence>=0))",
    "CREATE TABLE IF NOT EXISTS agena_retired_session_segments (id INTEGER PRIMARY KEY AUTOINCREMENT, session_id INTEGER NOT NULL REFERENCES agena_sessions(id), sequence INTEGER NOT NULL CHECK(sequence>=0), segment_id INTEGER NOT NULL REFERENCES agena_history_segments(id), first_ordinal INTEGER NOT NULL CHECK(first_ordinal BETWEEN 0 AND 127), end_ordinal INTEGER NOT NULL CHECK(end_ordinal BETWEEN 1 AND 128 AND end_ordinal>first_ordinal), since_generation INTEGER NOT NULL CHECK(typeof(since_generation)='integer' AND since_generation>0), until_generation INTEGER NOT NULL CHECK(typeof(until_generation)='integer' AND until_generation>since_generation), UNIQUE(session_id,sequence,since_generation))",
    "CREATE TABLE IF NOT EXISTS agena_history_snapshot_gc_state (id INTEGER PRIMARY KEY CHECK(id=1), after_id INTEGER NOT NULL CHECK(after_id>=0))",
    "CREATE TABLE IF NOT EXISTS agena_content_streams (id TEXT PRIMARY KEY, entry_id INTEGER NOT NULL, session_id INTEGER NOT NULL, execution_id TEXT NOT NULL, generation INTEGER NOT NULL, activity_id TEXT, kind TEXT NOT NULL CHECK(kind IN ('text','log','structured','terminal','document')), epoch TEXT NOT NULL, committed_head INTEGER NOT NULL DEFAULT 0 CHECK(typeof(committed_head)='integer' AND committed_head>=0), ordinal INTEGER NOT NULL DEFAULT 0 CHECK(typeof(ordinal)='integer' AND ordinal>=0), lost_bytes INTEGER NOT NULL DEFAULT 0 CHECK(typeof(lost_bytes)='integer' AND lost_bytes>=0), lost_packets INTEGER NOT NULL DEFAULT 0 CHECK(typeof(lost_packets)='integer' AND lost_packets>=0), capture_state TEXT NOT NULL CHECK(capture_state IN ('open','complete','interrupted')), retained_start INTEGER NOT NULL DEFAULT 0 CHECK(retained_start>=0 AND retained_start<=committed_head), capture_error TEXT, capture_error_bytes INTEGER GENERATED ALWAYS AS (COALESCE(length(CAST(capture_error AS BLOB)),0)) STORED, FOREIGN KEY(entry_id,session_id,execution_id,generation) REFERENCES agena_entries(id,origin_session_id,execution_id,generation) DEFERRABLE INITIALLY DEFERRED, FOREIGN KEY(activity_id,entry_id,session_id,execution_id,generation) REFERENCES agena_background_activities(id,entry_id,session_id,execution_id,generation))",
    "CREATE TABLE IF NOT EXISTS agena_content_capture_losses (content_id TEXT NOT NULL REFERENCES agena_content_streams(id), first_packet INTEGER NOT NULL CHECK(typeof(first_packet)='integer' AND first_packet>0), last_packet INTEGER NOT NULL CHECK(typeof(last_packet)='integer' AND last_packet>=first_packet), after_position INTEGER NOT NULL CHECK(typeof(after_position)='integer' AND after_position>=0), byte_count INTEGER NOT NULL CHECK(typeof(byte_count)='integer' AND byte_count>=0), diagnostic TEXT NOT NULL CHECK(length(CAST(diagnostic AS BLOB)) BETWEEN 1 AND 256), PRIMARY KEY(content_id,first_packet)) WITHOUT ROWID",
    "CREATE TABLE IF NOT EXISTS agena_content_blocks (content_id TEXT NOT NULL REFERENCES agena_content_streams(id), logical_start INTEGER NOT NULL CHECK(logical_start>=0), logical_end INTEGER NOT NULL CHECK(logical_end>logical_start), codec TEXT NOT NULL CHECK(codec IN ('utf8','events_v1','checkpoint_v1')), digest BLOB NOT NULL CHECK(length(digest)=32), body BLOB NOT NULL CHECK(length(body)>0 AND length(body)<=33554432), PRIMARY KEY(content_id,logical_start)) WITHOUT ROWID",
    "CREATE TABLE IF NOT EXISTS agena_content_checkpoints (content_id TEXT NOT NULL, logical_sequence INTEGER NOT NULL CHECK(logical_sequence>0), block_start INTEGER NOT NULL CHECK(block_start>=0), PRIMARY KEY(content_id,logical_sequence), FOREIGN KEY(content_id,block_start) REFERENCES agena_content_blocks(content_id,logical_start)) WITHOUT ROWID",
    "CREATE TABLE IF NOT EXISTS agena_content_gaps (content_id TEXT NOT NULL REFERENCES agena_content_streams(id), logical_start INTEGER NOT NULL CHECK(logical_start>=0), logical_end INTEGER NOT NULL CHECK(logical_end>logical_start), reason TEXT NOT NULL, PRIMARY KEY(content_id,logical_start)) WITHOUT ROWID",
    "CREATE TABLE IF NOT EXISTS agena_blobs (id TEXT PRIMARY KEY, session_id INTEGER NOT NULL REFERENCES agena_sessions(id), state TEXT NOT NULL CHECK(state IN ('preparing','ready','retired')), expected_bytes INTEGER NOT NULL CHECK(expected_bytes>=0), received_bytes INTEGER NOT NULL DEFAULT 0 CHECK(received_bytes>=0 AND received_bytes<=expected_bytes), abandoned INTEGER NOT NULL DEFAULT 0 CHECK(abandoned IN (0,1)), digest BLOB CHECK(digest IS NULL OR length(digest)=32), created_at_ms INTEGER NOT NULL, CHECK(state!='ready' OR (digest IS NOT NULL AND received_bytes=expected_bytes)))",
    "CREATE TABLE IF NOT EXISTS agena_blob_chunks (blob_id TEXT NOT NULL REFERENCES agena_blobs(id), logical_start INTEGER NOT NULL CHECK(logical_start>=0 AND logical_start%131072=0), digest BLOB NOT NULL CHECK(length(digest)=32), body BLOB NOT NULL CHECK(length(body)>0 AND length(body)<=131072), PRIMARY KEY(blob_id,logical_start)) WITHOUT ROWID",
    "CREATE TABLE IF NOT EXISTS agena_blob_references (entry_id INTEGER NOT NULL REFERENCES agena_entries(id), blob_id TEXT NOT NULL REFERENCES agena_blobs(id), field TEXT NOT NULL, PRIMARY KEY(entry_id,blob_id,field)) WITHOUT ROWID",
    "CREATE TABLE IF NOT EXISTS agena_blob_pins (blob_id TEXT NOT NULL REFERENCES agena_blobs(id), token TEXT NOT NULL, owner_epoch TEXT NOT NULL, expires_at_ms INTEGER NOT NULL, PRIMARY KEY(blob_id,token)) WITHOUT ROWID",
    "CREATE TABLE IF NOT EXISTS agena_blob_gc_state (id INTEGER PRIMARY KEY CHECK(id=1), after_id TEXT NOT NULL)",
    "CREATE TABLE IF NOT EXISTS agena_commits (id INTEGER PRIMARY KEY CHECK(id=1), sequence INTEGER NOT NULL CHECK(typeof(sequence)='integer' AND sequence>=0))",
    "CREATE TABLE IF NOT EXISTS agena_command_results (request_id TEXT PRIMARY KEY, request_digest BLOB NOT NULL CHECK(length(request_digest)=32), receipt_json TEXT NOT NULL CHECK(json_valid(receipt_json)), commit_sequence INTEGER NOT NULL CHECK(commit_sequence>0), command_ordinal INTEGER NOT NULL CHECK(command_ordinal>=0), created_at_ms INTEGER NOT NULL)",
    "CREATE TABLE IF NOT EXISTS agena_deliveries (id TEXT PRIMARY KEY, session_id INTEGER NOT NULL REFERENCES agena_sessions(id), business_key TEXT NOT NULL UNIQUE, target_execution_id TEXT REFERENCES agena_executions(id), payload_json TEXT NOT NULL CHECK(json_valid(payload_json)), state TEXT NOT NULL CHECK(state IN ('pending','claimed','consumed','failed')), owner_epoch TEXT, lease_until_ms INTEGER, result_entry_id INTEGER REFERENCES agena_entries(id), observed_round_id INTEGER REFERENCES agena_rounds(id), resume_execution_id TEXT REFERENCES agena_executions(id), resume_generation INTEGER CHECK(resume_generation IS NULL OR (typeof(resume_generation)='integer' AND resume_generation>0)), CHECK((state='claimed' AND owner_epoch IS NOT NULL AND lease_until_ms IS NOT NULL) OR (state!='claimed' AND owner_epoch IS NULL AND lease_until_ms IS NULL)))",
];

pub(super) const INDEXES: &[&str] = &[
    "CREATE INDEX IF NOT EXISTS idx_agena_background_active ON agena_background_activities(session_id,id) WHERE phase='running'",
    "CREATE INDEX IF NOT EXISTS idx_agena_background_tool ON agena_background_activities(tool_id,id)",
    "CREATE INDEX IF NOT EXISTS idx_agena_background_entry ON agena_background_activities(entry_id,id)",
    "CREATE INDEX IF NOT EXISTS idx_agena_content_activity ON agena_content_streams(activity_id,id)",
    "CREATE INDEX IF NOT EXISTS idx_agena_sessions_workspace ON agena_sessions(workspace_id,updated_at_ms,id) WHERE deleted_at_ms IS NULL",
    "CREATE INDEX IF NOT EXISTS idx_agena_execution_owner ON agena_executions(session_id,id)",
    "CREATE INDEX IF NOT EXISTS idx_agena_execution_latest ON agena_executions(session_id,message_id DESC)",
    "CREATE INDEX IF NOT EXISTS idx_agena_execution_message ON agena_executions(message_id)",
    "CREATE INDEX IF NOT EXISTS idx_agena_attempt_active ON agena_attempts(session_id,execution_id,generation) WHERE phase IN ('queued','running')",
    "CREATE UNIQUE INDEX IF NOT EXISTS idx_agena_tool_scope ON agena_tool_calls(id,execution_id,generation)",
    "CREATE INDEX IF NOT EXISTS idx_agena_tool_active ON agena_tool_calls(execution_id,generation,id) WHERE phase IN ('preparing','running')",
    "CREATE INDEX IF NOT EXISTS idx_agena_tool_drafts ON agena_tool_calls(round_id,id) WHERE phase='preparing'",
    "CREATE INDEX IF NOT EXISTS idx_agena_round_session ON agena_rounds(session_id,id)",
    "CREATE INDEX IF NOT EXISTS idx_agena_delivery_session ON agena_deliveries(session_id,id) WHERE state='pending'",
    "CREATE INDEX IF NOT EXISTS idx_agena_delivery_execution ON agena_deliveries(target_execution_id) WHERE state='pending'",
    "CREATE INDEX IF NOT EXISTS idx_agena_delivery_target_execution ON agena_deliveries(target_execution_id)",
    "CREATE INDEX IF NOT EXISTS idx_agena_session_family ON agena_sessions(root_session_id,id) WHERE deleted_at_ms IS NULL",
    "CREATE INDEX IF NOT EXISTS idx_agena_round_active ON agena_rounds(execution_id,generation,id) WHERE phase='running'",
    "CREATE INDEX IF NOT EXISTS idx_agena_attempt_running ON agena_attempts(execution_id,generation) WHERE phase='running'",
    "CREATE INDEX IF NOT EXISTS idx_agena_content_active ON agena_content_streams(execution_id,generation,id) WHERE capture_state='open'",
    "CREATE INDEX IF NOT EXISTS idx_agena_interaction_tool ON agena_interactions(tool_id,id)",
    "CREATE INDEX IF NOT EXISTS idx_agena_interaction_pending ON agena_interactions(tool_id,id) WHERE decision_json IS NULL",
    "CREATE INDEX IF NOT EXISTS idx_agena_interaction_pending_session ON agena_interactions(session_id,id,request_bytes) WHERE decision_json IS NULL",
    "CREATE INDEX IF NOT EXISTS idx_agena_interaction_attempt ON agena_interactions(execution_id,generation,session_id)",
    "CREATE INDEX IF NOT EXISTS idx_agena_interaction_tool ON agena_interactions(tool_id,id)",
    "CREATE INDEX IF NOT EXISTS idx_agena_session_agent_active ON agena_sessions(agent_status,id) WHERE deleted_at_ms IS NULL",
    "CREATE UNIQUE INDEX IF NOT EXISTS idx_agena_session_agent_name ON agena_sessions(parent_session_id,agent_name) WHERE deleted_at_ms IS NULL AND agent_name IS NOT NULL",
    "CREATE INDEX IF NOT EXISTS idx_agena_session_parent ON agena_sessions(parent_session_id,updated_at_ms DESC,id DESC) WHERE deleted_at_ms IS NULL",
    "CREATE INDEX IF NOT EXISTS idx_agena_session_global ON agena_sessions(updated_at_ms DESC,id DESC) WHERE deleted_at_ms IS NULL",
    "CREATE INDEX IF NOT EXISTS idx_agena_entry_owner ON agena_entries(origin_session_id,id)",
    "CREATE INDEX IF NOT EXISTS idx_agena_owned_tool_entries ON agena_entries(origin_session_id,id,body_bytes,tool_id) WHERE kind='tool'",
    "CREATE INDEX IF NOT EXISTS idx_agena_entry_bytes ON agena_entries(id,body_bytes)",
    "CREATE INDEX IF NOT EXISTS idx_agena_entry_attempt ON agena_entries(execution_id,generation,origin_session_id)",
    "CREATE INDEX IF NOT EXISTS idx_agena_entry_message ON agena_entries(message_id) WHERE message_id IS NOT NULL",
    "CREATE INDEX IF NOT EXISTS idx_agena_entry_round ON agena_entries(round_id) WHERE round_id IS NOT NULL",
    "CREATE INDEX IF NOT EXISTS idx_agena_entry_content ON agena_entries(content_id) WHERE content_id IS NOT NULL",
    "CREATE INDEX IF NOT EXISTS idx_agena_entry_tool ON agena_entries(tool_id,id) WHERE tool_id IS NOT NULL",
    "CREATE INDEX IF NOT EXISTS idx_agena_entry_interaction ON agena_entries(interaction_id,id) WHERE interaction_id IS NOT NULL",
    "CREATE INDEX IF NOT EXISTS idx_agena_segment_entry_reverse ON agena_segment_entries(entry_id,segment_id)",
    "CREATE INDEX IF NOT EXISTS idx_agena_entry_prompt ON agena_entries(id,body_bytes,tool_id,interaction_id)",
    "CREATE INDEX IF NOT EXISTS idx_agena_interaction_bytes ON agena_interactions(id,request_bytes,decision_bytes,remember_bytes)",
    "CREATE INDEX IF NOT EXISTS idx_agena_tool_bytes ON agena_tool_calls(id,input_bytes,result_bytes)",
    "CREATE INDEX IF NOT EXISTS idx_agena_round_bytes ON agena_rounds(id,provider_state_bytes)",
    "CREATE INDEX IF NOT EXISTS idx_agena_round_export ON agena_rounds(execution_id,id)",
    "CREATE INDEX IF NOT EXISTS idx_agena_prompt_checkpoint ON agena_prompt_windows(checkpoint_entry_id) WHERE checkpoint_entry_id IS NOT NULL",
    "CREATE INDEX IF NOT EXISTS idx_agena_prompt_through ON agena_prompt_windows(through_entry_id) WHERE through_entry_id IS NOT NULL",
    "CREATE INDEX IF NOT EXISTS idx_agena_compaction_base ON agena_compaction_rounds(base_checkpoint_entry_id) WHERE base_checkpoint_entry_id IS NOT NULL",
    "CREATE INDEX IF NOT EXISTS idx_agena_round_source_entry ON agena_round_input_sources(round_id,entry_id)",
    "CREATE INDEX IF NOT EXISTS idx_agena_round_source_content ON agena_round_input_sources(content_id)",
    "CREATE INDEX IF NOT EXISTS idx_agena_round_source_bytes ON agena_round_input_sources(round_id,content_id,head_bytes)",
    "CREATE INDEX IF NOT EXISTS idx_agena_round_replay_source ON agena_round_input_replays(source_round_id)",
    "CREATE INDEX IF NOT EXISTS idx_agena_round_input_response ON agena_round_inputs(response_entry_id) WHERE response_entry_id IS NOT NULL",
    "CREATE INDEX IF NOT EXISTS idx_agena_attempt_response ON agena_attempts(response_entry_id) WHERE response_entry_id IS NOT NULL",
    "CREATE INDEX IF NOT EXISTS idx_agena_round_input_entry ON agena_round_inputs(entry_id)",
    "CREATE INDEX IF NOT EXISTS idx_agena_session_segment_reverse ON agena_session_segments(segment_id,session_id)",
    "CREATE INDEX IF NOT EXISTS idx_agena_history_snapshot_owner ON agena_history_snapshots(session_id,generation)",
    "CREATE INDEX IF NOT EXISTS idx_agena_retired_segment_order ON agena_retired_session_segments(session_id,sequence,since_generation,until_generation)",
    "CREATE INDEX IF NOT EXISTS idx_agena_retired_segment_reverse ON agena_retired_session_segments(segment_id)",
    "CREATE INDEX IF NOT EXISTS idx_agena_content_owner ON agena_content_streams(session_id,execution_id,generation)",
    "CREATE INDEX IF NOT EXISTS idx_agena_content_prompt ON agena_content_streams(entry_id,id,capture_error_bytes)",
    "CREATE INDEX IF NOT EXISTS idx_agena_content_entry ON agena_content_streams(entry_id,id)",
    "CREATE INDEX IF NOT EXISTS idx_agena_checkpoint_block ON agena_content_checkpoints(content_id,block_start)",
    "CREATE INDEX IF NOT EXISTS idx_agena_blob_reference_reverse ON agena_blob_references(blob_id,entry_id)",
    "CREATE INDEX IF NOT EXISTS idx_agena_blob_state ON agena_blobs(state,created_at_ms,id)",
    "CREATE INDEX IF NOT EXISTS idx_agena_blob_pin_expiry ON agena_blob_pins(expires_at_ms,blob_id)",
    "CREATE INDEX IF NOT EXISTS idx_agena_delivery_active ON agena_deliveries(state,lease_until_ms,id)",
    "CREATE INDEX IF NOT EXISTS idx_agena_delivery_result ON agena_deliveries(result_entry_id) WHERE result_entry_id IS NOT NULL",
    "CREATE INDEX IF NOT EXISTS idx_agena_delivery_unobserved ON agena_deliveries(session_id,target_execution_id,id) WHERE state='consumed' AND observed_round_id IS NULL AND (target_execution_id IS NOT NULL OR COALESCE(json_extract(payload_json,'$.wake'),1)!=0 OR json_extract(payload_json,'$.kind')='agent_message')",
    "CREATE INDEX IF NOT EXISTS idx_agena_delivery_observed_round ON agena_deliveries(observed_round_id) WHERE observed_round_id IS NOT NULL",
];

pub(super) const TRIGGERS: &[&str] = &[
    "CREATE TRIGGER IF NOT EXISTS agena_segment_immutable_insert BEFORE INSERT ON agena_segment_entries WHEN EXISTS (SELECT 1 FROM agena_session_segments WHERE segment_id=NEW.segment_id) OR EXISTS (SELECT 1 FROM agena_retired_session_segments WHERE segment_id=NEW.segment_id) BEGIN SELECT RAISE(ABORT,'published history segment is immutable'); END",
    "CREATE TRIGGER IF NOT EXISTS agena_segment_immutable_update BEFORE UPDATE ON agena_segment_entries WHEN EXISTS (SELECT 1 FROM agena_session_segments WHERE segment_id=OLD.segment_id OR segment_id=NEW.segment_id) OR EXISTS (SELECT 1 FROM agena_retired_session_segments WHERE segment_id=OLD.segment_id OR segment_id=NEW.segment_id) BEGIN SELECT RAISE(ABORT,'published history segment is immutable'); END",
    "CREATE TRIGGER IF NOT EXISTS agena_segment_immutable_delete BEFORE DELETE ON agena_segment_entries WHEN EXISTS (SELECT 1 FROM agena_session_segments WHERE segment_id=OLD.segment_id) OR EXISTS (SELECT 1 FROM agena_retired_session_segments WHERE segment_id=OLD.segment_id) BEGIN SELECT RAISE(ABORT,'published history segment is immutable'); END",
    "CREATE TRIGGER IF NOT EXISTS agena_content_block_no_overlap BEFORE INSERT ON agena_content_blocks WHEN EXISTS (SELECT 1 FROM agena_content_blocks WHERE content_id=NEW.content_id AND logical_start<NEW.logical_end AND logical_end>NEW.logical_start) BEGIN SELECT RAISE(ABORT,'content block ranges overlap'); END",
    "CREATE TRIGGER IF NOT EXISTS agena_capture_loss_immutable BEFORE UPDATE ON agena_content_capture_losses BEGIN SELECT RAISE(ABORT,'capture loss window is immutable'); END",
    "CREATE TRIGGER IF NOT EXISTS agena_content_block_immutable BEFORE UPDATE ON agena_content_blocks BEGIN SELECT RAISE(ABORT,'content block is immutable'); END",
    "CREATE TRIGGER IF NOT EXISTS agena_blob_chunk_immutable BEFORE UPDATE ON agena_blob_chunks BEGIN SELECT RAISE(ABORT,'blob chunk is immutable'); END",
    "CREATE TRIGGER IF NOT EXISTS agena_blob_ready_reference BEFORE INSERT ON agena_blob_references WHEN NOT EXISTS(SELECT 1 FROM agena_blobs WHERE id=NEW.blob_id AND state='ready') BEGIN SELECT RAISE(ABORT,'blob is not ready'); END",
];

// Listings read only compact facts. These projections change in the same
// transaction as their source facts, including recovery and archive import.
fn summary_triggers() -> Vec<String> {
    const STATE: &str = "CASE WHEN EXISTS(SELECT 1 FROM agena_interactions i JOIN agena_tool_calls c ON c.id=i.tool_id JOIN agena_executions x ON x.id=c.execution_id JOIN agena_attempts a ON a.execution_id=x.id AND a.generation=x.current_generation WHERE i.session_id=s.id AND i.decision_json IS NULL AND c.generation=x.current_generation AND a.phase='running' AND c.phase='running') THEN 'awaiting_interaction' WHEN t.queued_executions+t.running_executions>0 THEN 'running' WHEN (SELECT a.phase FROM agena_executions x JOIN agena_attempts a ON a.execution_id=x.id AND a.generation=x.current_generation WHERE x.session_id=s.id ORDER BY x.message_id DESC LIMIT 1) IN ('failed','interrupted') THEN 'failed' ELSE 'ready' END";
    let mut triggers = Vec::new();
    for (name, event, session) in [
        (
            "statistics_insert",
            "AFTER INSERT ON agena_session_statistics",
            "NEW.session_id",
        ),
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

pub(crate) async fn initialize(database: &DatabaseConnection) -> Result<(), DbErr> {
    // Unchanged infrastructure has one declaration shared with the retiring
    // Part adapter. No old chat tables or business engine are installed here.
    let infrastructure = crate::schema::TABLES
        .iter()
        .chain(crate::schema::INDEXES)
        .filter(|sql| {
            [
                "agena_workspaces",
                "agena_permission_rules",
                "agena_model_catalog_entries",
                "agena_model_catalog_state",
            ]
            .iter()
            .any(|name| {
                sql.contains(&format!(" {name} (")) || sql.contains(&format!(" ON {name}("))
            })
        });
    let summary_triggers = summary_triggers();
    let declarations = TABLES
        .iter()
        .copied()
        .chain(INDEXES.iter().copied())
        .chain(TRIGGERS.iter().copied())
        .chain(infrastructure.copied())
        .chain(summary_triggers.iter().map(String::as_str));
    let actual = schema_objects(database).await?;
    if !actual.is_empty() {
        let expected = declarations
            .map(|sql| {
                let d = declaration(sql)?;
                Ok(((d.kind.to_owned(), d.name.to_owned()), d.stored_sql))
            })
            .collect::<Result<std::collections::BTreeMap<_, _>, DbErr>>()?;
        if expected != actual {
            return Err(DbErr::Custom("database does not match the session-kernel schema; open a fresh database path; the existing database has not been modified".into()));
        }
        return Ok(());
    }
    let transaction = database.begin().await?;
    for sql in declarations.chain([
        "INSERT INTO agena_commits VALUES (1,0)",
        "INSERT INTO agena_blob_gc_state VALUES (1,'')",
        "INSERT INTO agena_history_snapshot_gc_state VALUES (1,0)",
    ]) {
        transaction
            .execute(Statement::from_string(database.get_database_backend(), sql))
            .await?;
    }
    transaction.commit().await
}
