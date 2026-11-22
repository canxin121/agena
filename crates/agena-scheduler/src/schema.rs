//! Scheduler declarations for the service instance database.
//!
//! The database owner composes these with the session and server schema,
//! initializes the complete baseline once, and admits existing databases.
//! Scheduler repositories receive that already initialized connection.

/// All scheduler declarations, composed into the service instance schema by
/// its database owner. Runtime consumers only receive an initialized pool.
pub fn declarations() -> impl Iterator<Item = &'static str> {
    TABLES.iter().chain(INDEXES).copied()
}

/// `agena_scheduler_jobs` mirrors the hot scheduling fields of `ScheduledJob`
/// (`retry_at_ms`, `paused`, `completed`) as columns alongside
/// `next_fire_at_ms` so the scheduler's due scan can filter in SQL instead of
/// decoding every job JSON every tick. `delivery_key` / `claimed_at_ms` are the
/// cross-process claim lease: `delivery_key` identifies one worker attempt,
/// and `claimed_at_ms` is refreshed by its heartbeat. The stable business key
/// and original claim time remain in `job_json.pending_delivery`. Other hot
/// columns are derived copies of the JSON, written together with it.
const TABLES: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS agena_scheduler_jobs (id TEXT PRIMARY KEY, job_json TEXT NOT NULL CHECK(json_valid(job_json)), owner_workspace TEXT GENERATED ALWAYS AS (json_extract(job_json,'$.owner_workspace')) STORED, owner_session_id INTEGER GENERATED ALWAYS AS (json_extract(job_json,'$.owner_session_id')) STORED, next_fire_at_ms INTEGER NULL, retry_at_ms INTEGER NULL, delivery_key TEXT NULL, claimed_at_ms INTEGER NULL, paused INTEGER NOT NULL DEFAULT 0, completed INTEGER NOT NULL DEFAULT 0)",
    "CREATE TABLE IF NOT EXISTS agena_scheduler_history (id INTEGER PRIMARY KEY AUTOINCREMENT, job_id TEXT NOT NULL, owner_workspace TEXT NULL, owner_session_id INTEGER NULL, run_json TEXT NOT NULL CHECK(json_valid(run_json)), finished_at_ms INTEGER NOT NULL)",
];

const INDEXES: &[&str] = &[
    "CREATE INDEX IF NOT EXISTS idx_agena_scheduler_next_fire ON agena_scheduler_jobs(next_fire_at_ms IS NULL, next_fire_at_ms, id)",
    "CREATE INDEX IF NOT EXISTS idx_agena_scheduler_history_finished ON agena_scheduler_history(finished_at_ms DESC, id DESC)",
    "CREATE INDEX IF NOT EXISTS idx_agena_scheduler_history_job_finished ON agena_scheduler_history(job_id, finished_at_ms DESC, id DESC)",
    "CREATE INDEX IF NOT EXISTS idx_agena_scheduler_session ON agena_scheduler_jobs(owner_session_id, next_fire_at_ms IS NULL, next_fire_at_ms, id) WHERE owner_session_id IS NOT NULL",
    "CREATE INDEX IF NOT EXISTS idx_agena_scheduler_owner ON agena_scheduler_jobs(owner_workspace, owner_session_id, next_fire_at_ms IS NULL, next_fire_at_ms, id) WHERE owner_workspace IS NOT NULL",
    "CREATE INDEX IF NOT EXISTS idx_agena_scheduler_session_pending ON agena_scheduler_jobs(owner_session_id, id) WHERE paused = 0 AND completed = 0 AND (next_fire_at_ms IS NOT NULL OR json_extract(job_json, '$.pending_delivery') IS NOT NULL)",
    "CREATE INDEX IF NOT EXISTS idx_agena_scheduler_pending_deadline ON agena_scheduler_jobs(COALESCE(retry_at_ms, next_fire_at_ms), id) WHERE paused = 0 AND completed = 0 AND delivery_key IS NULL",
    "CREATE INDEX IF NOT EXISTS idx_agena_scheduler_claim_expiry ON agena_scheduler_jobs(claimed_at_ms, id) WHERE paused = 0 AND completed = 0 AND delivery_key IS NOT NULL",
];
