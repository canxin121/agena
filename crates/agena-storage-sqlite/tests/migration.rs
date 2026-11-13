//! Offline baseline upgrade: shape changes, ownership backfill, safety.
use agena_storage_sqlite::migration::{migrate_database, migrated_path};
use sea_orm::{ConnectionTrait, DatabaseBackend, DatabaseConnection, Statement};

async fn open(path: &std::path::Path) -> DatabaseConnection {
    let url = format!("sqlite://{}?mode=rwc", path.display());
    let options: sea_orm::sqlx::sqlite::SqliteConnectOptions =
        std::str::FromStr::from_str(&url).unwrap();
    let pool = sea_orm::sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .unwrap();
    sea_orm::SqlxSqliteConnector::from_sqlx_sqlite_pool(pool)
}

async fn exec(db: &DatabaseConnection, sql: &str) {
    db.execute(Statement::from_string(
        DatabaseBackend::Sqlite,
        sql.to_owned(),
    ))
    .await
    .unwrap();
}

/// A legacy instance: identity table, conversations with a channel authority
/// that still uses a retired kind, a NOT NULL binding workspace, and the JSON
/// Bot registry.
async fn legacy_database(path: &std::path::Path) {
    let db = open(path).await;
    exec(
        &db,
        "CREATE TABLE agena_instances (id INTEGER PRIMARY KEY CHECK(id=1), schema_baseline TEXT NOT NULL, instance_id TEXT NOT NULL, created_at_ms INTEGER NOT NULL)",
    )
    .await;
    exec(
        &db,
        "CREATE TABLE agena_server_state (key TEXT PRIMARY KEY, value_json TEXT NOT NULL, updated_at_ms INTEGER NOT NULL)",
    )
    .await;
    exec(
        &db,
        "CREATE TABLE agena_workspaces (id INTEGER PRIMARY KEY AUTOINCREMENT, path TEXT NOT NULL, created_at_ms INTEGER NOT NULL, updated_at_ms INTEGER NOT NULL)",
    )
    .await;
    exec(
        &db,
        "INSERT INTO agena_workspaces(id,path,created_at_ms,updated_at_ms) VALUES(7,'/tmp/seven',1,1),(8,'/tmp/eight',1,1)",
    )
    .await;
    exec(
        &db,
        "CREATE TABLE agena_sessions (id INTEGER PRIMARY KEY AUTOINCREMENT, workspace_id INTEGER, title TEXT NOT NULL, parent_session_id INTEGER, root_session_id INTEGER, depth INTEGER NOT NULL DEFAULT 0, relation_kind TEXT NOT NULL DEFAULT 'root', configuration_json TEXT NOT NULL DEFAULT '{}', favorite INTEGER NOT NULL DEFAULT 0, pinned INTEGER NOT NULL DEFAULT 0, selection_json TEXT, metadata_version INTEGER NOT NULL DEFAULT 1, membership_generation INTEGER NOT NULL DEFAULT 1, control_version INTEGER NOT NULL DEFAULT 1, created_at_ms INTEGER NOT NULL, updated_at_ms INTEGER NOT NULL, deleted_at_ms INTEGER)",
    )
    .await;
    exec(
        &db,
        "CREATE TABLE agena_integration_bindings (id TEXT PRIMARY KEY, workspace_id INTEGER NOT NULL, source_session_id INTEGER NOT NULL, request_key TEXT NOT NULL, revision INTEGER NOT NULL, state TEXT NOT NULL, next_poll_ms INTEGER NOT NULL, binding_json TEXT NOT NULL, UNIQUE(source_session_id,request_key))",
    )
    .await;
    // One Bot conversation, one project conversation, one broadcast room.
    exec(
        &db,
        r#"INSERT INTO agena_sessions(id,workspace_id,title,configuration_json,created_at_ms,updated_at_ms) VALUES
            (1,NULL,'Bot','{}',1,1),
            (2,7,'Project','{"prompts":{"mode":"replace","order":[],"inline":[]}}',1,1),
            (3,8,'Room','{"channel_authority":{"binding_id":"b","epoch":1,"actor":"a","role":"member","identity":{"tenant":"t","account":"a"},"resource":"r","thread":null,"session_mode":"per_conversation","grant":{},"expires_at_ms":99,"memory_workspace_root":"/tmp/x","private_paths":[],"revoked":false,"kind":"private_channel"}}',1,1)"#,
    )
    .await;
    exec(
        &db,
        r#"INSERT INTO agena_integration_bindings VALUES('b',8,3,'k',1,'enabled',0,'{}')"#,
    )
    .await;
    exec(
        &db,
        r#"INSERT INTO agena_server_state VALUES('bot_registry_v1','{"bots":[{"id":"8f2c0000-0000-4000-8000-000000000001","name":"Nova","persona":{"instructions":"be brief"},"session_id":1,"revision":3,"archived":false,"created_at_ms":5,"updated_at_ms":6}],"retired_sessions":[9],"receipts":{},"pending":null}',0)"#,
    )
    .await;
    exec(
        &db,
        "CREATE TABLE agena_channel_sessions (session_id INTEGER PRIMARY KEY, binding_id TEXT NOT NULL, scope_key TEXT NOT NULL, epoch INTEGER NOT NULL, actor TEXT NOT NULL, record_json TEXT NOT NULL, UNIQUE(binding_id,scope_key))",
    )
    .await;
    exec(
        &db,
        r#"INSERT INTO agena_channel_sessions VALUES(3,'b','scope',1,'a','{}')"#,
    )
    .await;
    db.close().await.unwrap();
}

#[tokio::test]
async fn migration_copies_rows_and_backfills_ownership_and_kinds() {
    let dir = tempfile::tempdir().unwrap();
    let old = dir.path().join("agena.db");
    legacy_database(&old).await;
    let new = migrated_path(&old);
    assert_eq!(new.file_name().unwrap(), "agena.db.migrated");

    let counts = migrate_database(&old, &new).await.unwrap();
    assert_eq!(counts.get("agena_sessions"), Some(&3));
    assert_eq!(counts.get("agena_channel_sessions"), Some(&1));
    assert_eq!(counts.get("agena_bots"), Some(&1));
    // The legacy key is gone, so nothing can read it by accident.
    assert_eq!(counts.get("agena_server_state"), Some(&0));

    let db = open(&new).await;
    async fn row(db: &DatabaseConnection, sql: &str) -> sea_orm::QueryResult {
        db.query_one(Statement::from_string(
            DatabaseBackend::Sqlite,
            sql.to_owned(),
        ))
        .await
        .unwrap()
        .unwrap()
    }

    // Bot identity moved into its own table with its folded name.
    let bot = row(
        &db,
        "SELECT name,name_folded,instructions,revision FROM agena_bots",
    )
    .await;
    assert_eq!(bot.try_get::<String>("", "name").unwrap(), "Nova");
    assert_eq!(bot.try_get::<String>("", "name_folded").unwrap(), "nova");
    assert_eq!(
        bot.try_get::<String>("", "instructions").unwrap(),
        "be brief"
    );
    assert_eq!(bot.try_get::<i64>("", "revision").unwrap(), 3);

    // The Bot conversation now carries its owner, projected into generated columns.
    let owned = row(
        &db,
        "SELECT owner_bot_id,owner_surface FROM agena_sessions WHERE id=1",
    )
    .await;
    assert_eq!(
        owned.try_get::<String>("", "owner_bot_id").unwrap(),
        "8f2c0000-0000-4000-8000-000000000001"
    );
    assert_eq!(
        owned.try_get::<String>("", "owner_surface").unwrap(),
        "local"
    );

    // Project conversations stay unowned.
    let project = row(&db, "SELECT owner_bot_id FROM agena_sessions WHERE id=2").await;
    assert_eq!(
        project
            .try_get::<Option<String>>("", "owner_bot_id")
            .unwrap(),
        None
    );

    // The retired room kind collapses onto `group`.
    let room = row(
        &db,
        "SELECT configuration_json FROM agena_sessions WHERE id=3",
    )
    .await;
    let configuration: serde_json::Value =
        serde_json::from_str(&room.try_get::<String>("", "configuration_json").unwrap()).unwrap();
    assert_eq!(configuration["channel_authority"]["kind"], "group");

    // The binding keeps its workspace and gains no Bot owner.
    let binding = row(
        &db,
        "SELECT workspace_id,owner_bot_id FROM agena_integration_bindings",
    )
    .await;
    assert_eq!(binding.try_get::<i64>("", "workspace_id").unwrap(), 8);
    assert_eq!(
        binding
            .try_get::<Option<String>>("", "owner_bot_id")
            .unwrap(),
        None
    );
    db.close().await.unwrap();
}

#[tokio::test]
async fn migration_never_overwrites_and_never_touches_the_source() {
    let dir = tempfile::tempdir().unwrap();
    let old = dir.path().join("agena.db");
    legacy_database(&old).await;
    let before = std::fs::read(&old).unwrap();
    let new = dir.path().join("out.db");
    migrate_database(&old, &new).await.unwrap();
    assert_eq!(
        std::fs::read(&old).unwrap(),
        before,
        "source must be untouched"
    );

    let again = migrate_database(&old, &new).await;
    assert!(again.is_err(), "an existing destination is refused");
    assert!(
        migrate_database(&dir.path().join("missing.db"), &dir.path().join("x.db"))
            .await
            .is_err()
    );
}
