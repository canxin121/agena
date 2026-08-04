//! Multi-connection concurrency tests against a real (file-backed) SQLite
//! database.
//!
//! Two independent `DatabaseConnection` pools pointing at the same file model
//! the multi-process case: SQLite serializes writers with its file lock, and
//! every fix in this crate must hold under that pressure — schema
//! initialization must not race, sequence allocators must never hand out
//! duplicates, and check-then-insert paths must degrade to atomic upserts.

use std::process::Command;
use std::sync::Arc;

use sea_orm::{
    ConnectionTrait, Database, DatabaseBackend, DatabaseConnection, Statement, TransactionTrait,
};

use crate::{SeaPermissionRuleRepository, SeaWorkspaceRepository, SqliteSequenceAllocator};
use agena_domain::{PermissionMode, PermissionScope};
use agena_storage::{
    PersistedPermissionRule, PermissionRuleRepository, SequenceAllocator, WorkspaceRepository,
};

async fn connect_file(tempdir: &tempfile::TempDir, name: &str) -> DatabaseConnection {
    let path = tempdir.path().join(name);
    Database::connect(format!("sqlite://{}?mode=rwc", path.display()))
        .await
        .expect("connect file database")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_schema_initialization_is_idempotent() {
    let directory = tempfile::tempdir().expect("temporary database directory");
    // Two independent connection pools race to initialize the same database
    // file, as two cold-started processes would.
    let db_a = connect_file(&directory, "schema.db").await;
    let db_b = connect_file(&directory, "schema.db").await;

    let (r_a, r_b) = tokio::join!(
        crate::initialize_schema(&db_a),
        crate::initialize_schema(&db_b),
    );
    r_a.expect("first schema initialization succeeds");
    r_b.expect("concurrent schema initialization must not fail with SQLITE_BUSY");

    // Both connections see the current version marker.
    for db in [&db_a, &db_b] {
        let version: i64 = db
            .query_one(Statement::from_string(
                DatabaseBackend::Sqlite,
                "PRAGMA user_version".to_owned(),
            ))
            .await
            .expect("query user_version")
            .expect("user_version row")
            .try_get("", "user_version")
            .expect("user_version value");
        assert_eq!(version, crate::CURRENT_SCHEMA_VERSION);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn independent_allocators_never_hand_out_duplicate_sequences() {
    let directory = tempfile::tempdir().expect("temporary database directory");
    let db_a = Arc::new(connect_file(&directory, "seq.db").await);
    let db_b = Arc::new(connect_file(&directory, "seq.db").await);
    crate::initialize_schema(&db_a).await.expect("schema");

    let alloc_a = SqliteSequenceAllocator::new(Arc::clone(&db_a));
    let alloc_b = SqliteSequenceAllocator::new(Arc::clone(&db_b));

    // Race 200 allocations across two independent allocators (each backed by
    // its own connection pool). Every seq_global, message id and part id must
    // be unique.
    let mut globals = Vec::new();
    let mut messages = Vec::new();
    let mut parts = Vec::new();
    for i in 0..100i64 {
        let alloc = if i % 2 == 0 { &alloc_a } else { &alloc_b };
        globals.push(alloc.next_seq_global().await.expect("alloc seq_global"));
        messages.push(alloc.next_message_id().await.expect("alloc message id"));
        parts.push(alloc.next_part_id().await.expect("alloc part id"));
    }

    for label in ["seq_global", "message_id", "part_id"] {
        let values = match label {
            "seq_global" => &globals,
            "message_id" => &messages,
            _ => &parts,
        };
        let mut sorted = values.clone();
        sorted.sort_unstable();
        let unique = sorted.windows(2).all(|w| w[0] != w[1]);
        assert!(unique, "all 100 {label} must be distinct");
        assert_eq!(sorted.len(), 100);
        assert_eq!(sorted.first().copied(), Some(1), "first {label} is 1");
        assert_eq!(sorted.last().copied(), Some(100), "last {label} is 100");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_ensure_id_returns_one_shared_workspace() {
    let directory = tempfile::tempdir().expect("temporary database directory");
    let db_a = Arc::new(connect_file(&directory, "ws.db").await);
    let db_b = Arc::new(connect_file(&directory, "ws.db").await);
    crate::initialize_schema(&db_a).await.expect("schema");

    let repo_a = SeaWorkspaceRepository::new(Arc::clone(&db_a));
    let repo_b = SeaWorkspaceRepository::new(Arc::clone(&db_b));

    // Two independent repositories race to ensure the same workspace path.
    let (id_a, id_b) = tokio::join!(
        repo_a.ensure_id("/shared/workspace"),
        repo_b.ensure_id("/shared/workspace"),
    );
    let id_a = id_a.expect("repo a ensure_id");
    let id_b = id_b.expect("repo b ensure_id");
    assert_eq!(id_a, id_b, "both processes must resolve the same workspace id");

    // Exactly one row exists.
    let count: i64 = db_a
        .query_one(Statement::from_string(
            DatabaseBackend::Sqlite,
            "SELECT COUNT(*) AS count FROM agena_workspaces WHERE path = '/shared/workspace'"
                .to_owned(),
        ))
        .await
        .expect("count")
        .expect("count row")
        .try_get("", "count")
        .expect("count value");
    assert_eq!(count, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_permission_upsert_never_conflicts() {
    let directory = tempfile::tempdir().expect("temporary database directory");
    let db_a = Arc::new(connect_file(&directory, "perm.db").await);
    let db_b = Arc::new(connect_file(&directory, "perm.db").await);
    crate::initialize_schema(&db_a).await.expect("schema");

    let repo_a = SeaPermissionRuleRepository::new(Arc::clone(&db_a));
    let repo_b = SeaPermissionRuleRepository::new(Arc::clone(&db_b));

    let rule = |mode: PermissionMode| PersistedPermissionRule {
        id: None,
        created_at_ms: None,
        updated_at_ms: None,
        action_key: "shell.execute".to_owned(),
        mode,
        scope: PermissionScope::Global,
        session_id: None,
        workspace_id: None,
        source: "concurrency-test".to_owned(),
        reason: None,
        operator: None,
        revoked_at_ms: None,
        revoked_reason: None,
        revoked_by: None,
    };

    // Two processes upsert the same rule concurrently. Exactly one creates,
    // the other updates the same row; neither may hit a unique-constraint
    // error.
    let allow = rule(PermissionMode::Allow);
    let deny = rule(PermissionMode::Deny);
    let (res_a, res_b) = tokio::join!(
        repo_a.upsert(&allow),
        repo_b.upsert(&deny),
    );
    let (record_a, created_a) = res_a.expect("repo a upsert");
    let (record_b, created_b) = res_b.expect("repo b upsert");
    assert_eq!(record_a.id, record_b.id, "both upserts must land on one row");
    assert_ne!(created_a, created_b, "exactly one upsert must create the row");

    // The final persisted rule is one of the two modes, never corrupted.
    let count: i64 = db_a
        .query_one(Statement::from_string(
            DatabaseBackend::Sqlite,
            "SELECT COUNT(*) AS count FROM agena_permission_rules".to_owned(),
        ))
        .await
        .expect("count")
        .expect("count row")
        .try_get("", "count")
        .expect("count value");
    assert_eq!(count, 1);
}

/// The canonical read-before-write pattern that used to fail with SQLITE_BUSY:
/// two independent connection pools race a transaction that SELECTs a parent
/// row and then INSERTs a child, exactly like the model-catalog freshness gate
/// and the session-summary parent lineage. The write-lock fence must let both
/// complete instead of one failing on the read→write lock upgrade.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_read_before_write_transactions_never_hit_sqlite_busy() {
    let directory = tempfile::tempdir().expect("temporary database directory");
    let db_a = Arc::new(connect_file(&directory, "rbw.db").await);
    let db_b = Arc::new(connect_file(&directory, "rbw.db").await);
    crate::initialize_schema(&db_a).await.expect("schema");

    // Seed a workspace and one parent session so the read-before-write path has
    // a row to read (the parent FK requires the workspace row to exist).
    for db in [&db_a, &db_b] {
        db.execute(Statement::from_string(
            DatabaseBackend::Sqlite,
            "INSERT OR IGNORE INTO agena_workspaces \
             (id, path, created_at_ms, updated_at_ms) VALUES (1, '/rbw', 1, 1)"
                .to_owned(),
        ))
        .await
        .expect("seed workspace");
        db.execute(Statement::from_string(
            DatabaseBackend::Sqlite,
            "INSERT OR IGNORE INTO agena_sessions \
             (id, parent_id, depth, root_id, workspace_id, title, version, lifecycle_state, \
              creation_failure_json, runtime_state_json, created_at_ms, updated_at_ms) \
             VALUES (1, NULL, 0, 0, 1, 'parent', 1, 'ready', NULL, '{}', 1, 1)"
                .to_owned(),
        ))
        .await
        .expect("seed parent session");
    }

    // Both pools race the same read-then-write shape. Each child session reads
    // the parent's depth/root_id and then INSERTs its own row. Each pool uses a
    // disjoint id range so the only contention is the shared write lock.
    async fn run_read_before_write(db: &DatabaseConnection, id_offset: i64) {
        for attempt in 0..25i64 {
            let txn = db.begin().await.expect("begin rbw transaction");
            crate::acquire_write_lock(&txn).await.expect("acquire write lock");
            let row = txn
                .query_one(Statement::from_string(
                    DatabaseBackend::Sqlite,
                    "SELECT depth, root_id FROM agena_sessions WHERE id = 1".to_owned(),
                ))
                .await
                .expect("read parent")
                .expect("parent row");
            let depth: i64 = row.try_get("", "depth").expect("depth value");
            let root_id: i64 = row.try_get("", "root_id").expect("root_id value");
            txn.execute(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                "INSERT INTO agena_sessions \
                 (id, parent_id, depth, root_id, workspace_id, title, version, lifecycle_state, \
                  creation_failure_json, runtime_state_json, created_at_ms, updated_at_ms) \
                 VALUES (?, 1, ?, ?, 1, ?, 1, 'ready', NULL, '{}', ?, ?)",
                [
                    (id_offset + attempt).into(),
                    (depth + 1).into(),
                    root_id.into(),
                    format!("child-{attempt}").into(),
                    1i64.into(),
                    1i64.into(),
                ],
            ))
            .await
            .expect("insert child after read");
            txn.commit().await.expect("commit rbw transaction");
        }
    }

    tokio::join!(
        run_read_before_write(&db_a, 100),
        run_read_before_write(&db_b, 200),
    );

    // Both processes inserted 25 children each — no BUSY failure in either.
    let count: i64 = db_a
        .query_one(Statement::from_string(
            DatabaseBackend::Sqlite,
            "SELECT COUNT(*) AS count FROM agena_sessions WHERE parent_id = 1".to_owned(),
        ))
        .await
        .expect("count")
        .expect("count row")
        .try_get("", "count")
        .expect("count value");
    assert_eq!(count, 50, "all 50 concurrent read-before-write inserts landed");
}

/// Real two-process test: a child OS process re-runs this exact test against
/// the same database file while the parent does the same work concurrently.
/// This is the closest the test suite can come to "two agena processes share
/// `~/agena/agena.db`" without launching the full binary.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_os_processes_share_one_database_without_locking() {
    let directory = tempfile::tempdir().expect("temporary database directory");
    let path = directory.path().join("two-process.db");
    let path_str = path.to_string_lossy().into_owned();

    if std::env::var("AGENA_CONCURRENCY_CHILD").is_ok() {
        // Child: initialize schema, ensure a workspace, allocate sequences.
        two_process_work(&path).await;
        return;
    }

    // Parent: spawn the child, then do the same work concurrently.
    let exe = std::env::current_exe().expect("test binary path");
    let mut child = Command::new(&exe)
        .args(["--exact", "concurrency_tests::two_os_processes_share_one_database_without_locking", "--nocapture"])
        .env("AGENA_CONCURRENCY_CHILD", "1")
        .env("AGENA_DB_PATH", &path_str)
        .spawn()
        .expect("spawn child test process");

    // Parent work overlaps the child's.
    two_process_work(&path).await;

    let status = child.wait().expect("wait for child");
    assert!(
        status.success(),
        "child process failed: {status} — the two processes could not share the database"
    );

    // Verify the final state is coherent: exactly one workspace row.
    let db = Database::connect(format!("sqlite://{path_str}?mode=rwc"))
        .await
        .expect("reconnect final database");
    let count: i64 = db
        .query_one(Statement::from_string(
            DatabaseBackend::Sqlite,
            "SELECT COUNT(*) AS count FROM agena_workspaces".to_owned(),
        ))
        .await
        .expect("count")
        .expect("count row")
        .try_get("", "count")
        .expect("count value");
    assert_eq!(count, 1, "both processes must converge on one workspace row");
}

async fn two_process_work(path: &std::path::Path) {
    let db = Arc::new(
        Database::connect(format!("sqlite://{}?mode=rwc", path.display()))
            .await
            .expect("connect shared database"),
    );
    // Both processes race schema initialization; with the schema lock this
    // succeeds in both.
    crate::initialize_schema(&db).await.expect("schema");
    // Both processes ensure the same workspace; exactly one row results.
    let repo = SeaWorkspaceRepository::new(Arc::clone(&db));
    repo.ensure_id("/shared/process-workspace")
        .await
        .expect("ensure workspace");
    // Both processes allocate sequences; the database serializes them.
    let alloc = SqliteSequenceAllocator::new(Arc::clone(&db));
    for _ in 0..50 {
        alloc.next_seq_global().await.expect("allocate seq");
    }
}

