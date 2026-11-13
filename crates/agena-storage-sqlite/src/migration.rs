//! One-time offline upgrade into the current schema baseline.
//!
//! The application refuses incompatible databases instead of rewriting them in
//! place. This module is the sanctioned way to cross a baseline boundary: it
//! builds a complete new database beside the old one, copies every table that
//! still exists, applies the documented shape changes, and only then reports
//! success. The old file is never modified.
//!
//! Run it with the `agena migrate` command while the server is stopped.

use sea_orm::{ConnectionTrait, DatabaseBackend, DbErr, Statement};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// The bridge that reads `bot_registry_v1` out of the instance JSON store.
const LEGACY_BOT_REGISTRY: &str = "bot_registry_v1";

async fn exec(db: &impl ConnectionTrait, sql: &str) -> Result<(), DbErr> {
    db.execute(Statement::from_string(
        DatabaseBackend::Sqlite,
        sql.to_owned(),
    ))
    .await
    .map(|_| ())
}

async fn scalar_i64(db: &impl ConnectionTrait, sql: &str) -> Result<Option<i64>, DbErr> {
    let Some(row) = db
        .query_one(Statement::from_string(
            DatabaseBackend::Sqlite,
            sql.to_owned(),
        ))
        .await?
    else {
        return Ok(None);
    };
    Ok(row.try_get_by_index::<i64>(0).ok())
}

async fn scalar_text(db: &impl ConnectionTrait, sql: &str) -> Result<Option<String>, DbErr> {
    let Some(row) = db
        .query_one(Statement::from_string(
            DatabaseBackend::Sqlite,
            sql.to_owned(),
        ))
        .await?
    else {
        return Ok(None);
    };
    Ok(row.try_get_by_index::<String>(0).ok())
}

/// One column of one table, as reported by `PRAGMA table_info`.
///
/// `table_info` omits `GENERATED` columns entirely, which is exactly what the
/// copy needs: a derived column cannot be written, so it must never appear in
/// the insert list.
struct Column {
    name: String,
}

async fn columns(
    db: &impl ConnectionTrait,
    schema: &str,
    table: &str,
) -> Result<Vec<Column>, DbErr> {
    let rows = db
        .query_all(Statement::from_string(
            DatabaseBackend::Sqlite,
            format!("PRAGMA {schema}.table_info(\"{table}\")"),
        ))
        .await?;
    Ok(rows
        .into_iter()
        .filter_map(|row| row.try_get::<String>("", "name").ok())
        .map(|name| Column { name })
        .collect())
}

async fn tables(db: &impl ConnectionTrait, schema: &str) -> Result<BTreeSet<String>, DbErr> {
    let rows = db
        .query_all(Statement::from_string(
            DatabaseBackend::Sqlite,
            format!(
                "SELECT name FROM {schema}.sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'"
            ),
        ))
        .await?;
    Ok(rows
        .into_iter()
        .filter_map(|row| row.try_get::<String>("", "name").ok())
        .collect())
}

/// Copy every row of every table both schemas still share.
///
/// Only columns present in both are copied, generated columns are skipped, and
/// a new-schema `NOT NULL` column without a default on a table the old schema
/// lacks fails loudly rather than silently dropping data.
async fn copy_shared_tables(
    new: &impl ConnectionTrait,
    old: &impl ConnectionTrait,
) -> Result<Vec<String>, DbErr> {
    let new_tables = tables(new, "main").await?;
    let old_tables = tables(old, "legacy").await?;
    let shared = new_tables
        .intersection(&old_tables)
        .cloned()
        .collect::<Vec<_>>();
    for table in &shared {
        let new_columns = columns(new, "main", table).await?;
        let old_columns = columns(old, "legacy", table)
            .await?
            .into_iter()
            .map(|column| column.name)
            .collect::<BTreeSet<_>>();
        let copyable = new_columns
            .iter()
            .map(|column| column.name.clone())
            .filter(|name| old_columns.contains(name))
            .collect::<Vec<_>>();
        if copyable.is_empty() {
            continue;
        }
        let list = copyable
            .iter()
            .map(|name| format!("\"{name}\""))
            .collect::<Vec<_>>()
            .join(",");
        exec(
            new,
            &format!("INSERT INTO main.\"{table}\" ({list}) SELECT {list} FROM legacy.\"{table}\""),
        )
        .await
        .map_err(|error| DbErr::Custom(format!("copying {table} failed: {error}")))?;
    }
    Ok(shared)
}

/// Rewrite each conversation's `configuration_json` for the new ownership
/// model and the narrowed conversation-kind enum.
///
/// * `owner` is filled from the legacy Bot registry below.
/// * `channel_authority.kind` loses the retired `public_channel` /
///   `private_channel` values, which collapse onto `group`.
async fn rewrite_sessions(
    new: &impl ConnectionTrait,
    bots: &BTreeMap<i64, String>,
) -> Result<(), DbErr> {
    let rows = new
        .query_all(Statement::from_string(
            DatabaseBackend::Sqlite,
            "SELECT id, configuration_json FROM main.agena_sessions".to_owned(),
        ))
        .await?;
    for row in rows {
        let id = row.try_get::<i64>("", "id").unwrap_or_default();
        let raw = row
            .try_get::<String>("", "configuration_json")
            .unwrap_or_default();
        let mut value: serde_json::Value = match serde_json::from_str(&raw) {
            Ok(value) => value,
            // A malformed configuration is preserved verbatim; the copy already
            // kept the bytes and the application will report it on open.
            Err(_) => continue,
        };
        let Some(object) = value.as_object_mut() else {
            continue;
        };
        if let Some(authority) = object
            .get_mut("channel_authority")
            .and_then(serde_json::Value::as_object_mut)
            && let Some(kind) = authority.get_mut("kind")
        {
            let mapped = match kind.as_str() {
                Some("direct") => Some("direct"),
                Some("group") => Some("group"),
                Some("public_channel") | Some("private_channel") => Some("group"),
                Some("broadcast") => Some("broadcast"),
                _ => None,
            };
            if let Some(mapped) = mapped {
                *kind = serde_json::Value::String(mapped.to_owned());
            }
        }
        if let Some(bot) = bots.get(&id) {
            object.insert(
                "owner".into(),
                serde_json::json!({"bot": bot, "surface": "local"}),
            );
        }
        let encoded = serde_json::to_string(&value).map_err(|error| {
            DbErr::Custom(format!("session {id} could not be re-encoded: {error}"))
        })?;
        new.execute(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            "UPDATE main.agena_sessions SET configuration_json=? WHERE id=?",
            [encoded.into(), id.into()],
        ))
        .await?;
    }
    Ok(())
}

/// Read the legacy JSON Bot registry: `session_id -> bot uuid`.
async fn legacy_bots(old: &impl ConnectionTrait) -> Result<BTreeMap<i64, String>, DbErr> {
    let Some(raw) = scalar_text(
        old,
        &format!(
            "SELECT value_json FROM legacy.agena_server_state WHERE key='{LEGACY_BOT_REGISTRY}'"
        ),
    )
    .await?
    else {
        return Ok(BTreeMap::new());
    };
    let value: serde_json::Value = serde_json::from_str(&raw)
        .map_err(|error| DbErr::Custom(format!("legacy Bot registry is malformed: {error}")))?;
    let mut bots = BTreeMap::new();
    for bot in value
        .get("bots")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
    {
        let (Some(id), Some(session)) = (
            bot.get("id").and_then(serde_json::Value::as_str),
            bot.get("session_id").and_then(serde_json::Value::as_i64),
        ) else {
            continue;
        };
        if session > 0 && uuid::Uuid::parse_str(id).is_ok() {
            bots.insert(session, id.to_owned());
        }
    }
    Ok(bots)
}

/// Move every legacy Bot identity into `agena_bots`, and drop the JSON key.
async fn migrated_bot_registry(
    new: &impl ConnectionTrait,
    old: &impl ConnectionTrait,
    bots: &BTreeMap<i64, String>,
) -> Result<(), DbErr> {
    let Some(raw) = scalar_text(
        old,
        &format!(
            "SELECT value_json FROM legacy.agena_server_state WHERE key='{LEGACY_BOT_REGISTRY}'"
        ),
    )
    .await?
    else {
        return Ok(());
    };
    let value: serde_json::Value = serde_json::from_str(&raw)
        .map_err(|error| DbErr::Custom(format!("legacy Bot registry is malformed: {error}")))?;
    let caseless = |name: &str| name.trim().to_lowercase();
    for bot in value
        .get("bots")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(id) = bot.get("id").and_then(serde_json::Value::as_str) else {
            continue;
        };
        if uuid::Uuid::parse_str(id).is_err() {
            continue;
        }
        let name = bot
            .get("name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("Bot");
        let instructions = bot
            .pointer("/persona/instructions")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        let revision = bot
            .get("revision")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(1);
        let archived = bot
            .get("archived")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        let created = bot
            .get("created_at_ms")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0);
        let updated = bot
            .get("updated_at_ms")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(created);
        new.execute(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            "INSERT INTO main.agena_bots(id,name,name_folded,instructions,revision,archived,created_at_ms,updated_at_ms) VALUES(?,?,?,?,?,?,?,?) ON CONFLICT(id) DO NOTHING",
            [
                id.into(),
                name.into(),
                caseless(name).into(),
                instructions.into(),
                revision.into(),
                i64::from(archived).into(),
                created.into(),
                updated.into(),
            ],
        ))
        .await?;
    }
    // The registry key must not survive: the application never reads it again,
    // and leaving it behind would make a later diagnosis ambiguous.
    exec(
        new,
        &format!("DELETE FROM main.agena_server_state WHERE key='{LEGACY_BOT_REGISTRY}'"),
    )
    .await?;
    let _ = bots;
    Ok(())
}

/// The documented shape change on `agena_integration_bindings`.
///
/// The old schema stored a `NOT NULL` workspace and no Bot owner. Its rows are
/// copied first, then the mandatory workspace is relaxed and the Bot column is
/// filled from the owning conversation's owner. Because SQLite cannot alter a
/// column constraint, the table is rebuilt.
async fn rebuild_integration_bindings(new: &impl ConnectionTrait) -> Result<(), DbErr> {
    exec(
        new,
        "CREATE TABLE main.agena_integration_bindings_v2 (id TEXT PRIMARY KEY, workspace_id INTEGER NULL CHECK(workspace_id IS NULL OR workspace_id>0), owner_bot_id TEXT NULL CHECK(owner_bot_id IS NULL OR length(owner_bot_id)=36), source_session_id INTEGER NOT NULL CHECK(source_session_id>0), request_key TEXT NOT NULL, revision INTEGER NOT NULL CHECK(revision>0), state TEXT NOT NULL, next_poll_ms INTEGER NOT NULL, binding_json TEXT NOT NULL CHECK(json_valid(binding_json)), UNIQUE(source_session_id,request_key), CHECK((workspace_id IS NULL) <> (owner_bot_id IS NULL)))",
    )
    .await?;
    exec(
        new,
        "INSERT INTO main.agena_integration_bindings_v2 (id,workspace_id,owner_bot_id,source_session_id,request_key,revision,state,next_poll_ms,binding_json) SELECT id,workspace_id,json_extract((SELECT configuration_json FROM main.agena_sessions WHERE id=agena_integration_bindings.source_session_id),'$.owner.bot'),source_session_id,request_key,revision,state,next_poll_ms,binding_json FROM main.agena_integration_bindings",
    )
    .await?;
    exec(new, "DROP TABLE main.agena_integration_bindings").await?;
    exec(
        new,
        "ALTER TABLE main.agena_integration_bindings_v2 RENAME TO agena_integration_bindings",
    )
    .await?;
    Ok(())
}

/// Migrate `old` into a fresh database at `new`, returning how many rows moved.
///
/// Refuses to overwrite an existing `new` file: the operator must choose a path
/// that does not exist yet, so a failed run never destroys a good database.
pub async fn migrate_database(old: &Path, new: &Path) -> Result<BTreeMap<String, u64>, DbErr> {
    if new.exists() {
        return Err(DbErr::Custom(format!(
            "refusing to overwrite {}; choose a new path for the migrated database",
            new.display()
        )));
    }
    if !old.exists() {
        return Err(DbErr::Custom(format!("no database at {}", old.display())));
    }
    let target = new.to_path_buf();
    let opened =
        crate::connection::KernelDatabase::open(&format!("sqlite://{}?mode=rwc", target.display()))
            .await?;
    let database = &*opened.writer;

    // The copy legitimately passes through states SQLite would reject: a child
    // table before its parent, and a drop-and-rename rebuild of the binding
    // table. Enforcement is therefore off for the copy itself. The destination
    // is verified afterwards — a full `foreign_key_check` plus the same exact
    // schema comparison the server performs on open — so an inconsistent copy
    // is reported, never accepted. The source is attached read-write only so
    // `read_uncommitted` reads see its latest committed state; nothing is ever
    // written to it, and a rejected run leaves `new` empty for a retry.
    exec(database, "PRAGMA foreign_keys=OFF").await?;
    exec(
        database,
        &format!(
            "ATTACH DATABASE '{}' AS legacy",
            old.display().to_string().replace('\'', "''")
        ),
    )
    .await?;
    exec(database, "PRAGMA legacy.read_uncommitted=1").await?;
    let copy = async {
        let bots = legacy_bots(database).await?;
        copy_shared_tables(database, database).await?;
        rewrite_sessions(database, &bots).await?;
        migrated_bot_registry(database, database, &bots).await?;
        rebuild_integration_bindings(database).await?;
        Ok::<(), DbErr>(())
    }
    .await;
    copy?;
    // Detach the source before reporting, so no write can reach it through a
    // lingering attachment.
    exec(database, "DETACH DATABASE legacy").await?;
    exec(database, "PRAGMA foreign_keys=ON").await?;

    if let Some(row) = database
        .query_one(Statement::from_string(
            DatabaseBackend::Sqlite,
            "PRAGMA foreign_key_check".to_owned(),
        ))
        .await?
    {
        let table = row.try_get::<String>("", "table").unwrap_or_default();
        return Err(DbErr::Custom(format!(
            "migrated database violates a foreign key in {table}; the source was not modified"
        )));
    }
    exec(database, "ANALYZE").await?;

    let mut counts = BTreeMap::new();
    for table in tables(database, "main").await? {
        if let Some(count) =
            scalar_i64(database, &format!("SELECT count(*) FROM \"{table}\"")).await?
        {
            counts.insert(table, u64::try_from(count).unwrap_or(0));
        }
    }
    opened.close().await?;
    Ok(counts)
}

/// Default destination for a migration beside the source database.
pub fn migrated_path(old: &Path) -> PathBuf {
    let mut name = old.as_os_str().to_owned();
    name.push(".migrated");
    PathBuf::from(name)
}
