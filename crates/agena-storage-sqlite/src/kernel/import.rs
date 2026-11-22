//! New-format archive import. The untrusted archive is parsed into a private,
//! disk-backed SQLite staging database before the sole writer publishes it.
//! No import path can obtain an attempt capability or launch external work.

use std::{
    collections::{HashMap, HashSet},
    path::Path,
};

use futures_util::TryStreamExt;
use serde_json::{Value, json};
use sqlx::sqlite::SqliteConnectOptions;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};

use super::*;

const MAX_LINE: usize = 32 * 1024 * 1024;
const MAX_BLOCK: i64 = 32 * 1024 * 1024;
const PIECE: usize = content::BLOCK_PIECE_BYTES;
const TABLES: &[&str] = &[
    "agena_executions",
    "agena_execution_purposes",
    "agena_attempts",
    "agena_rounds",
    "agena_tool_calls",
    "agena_interactions",
    "agena_entries",
    "agena_background_activities",
    "agena_content_streams",
    "agena_content_blocks",
    "agena_content_checkpoints",
    "agena_content_gaps",
    "agena_content_capture_losses",
    "agena_blobs",
    "agena_blob_chunks",
    "agena_blob_references",
    "agena_round_inputs",
    "agena_round_input_sources",
    "agena_round_input_replays",
    "agena_compaction_rounds",
    "agena_history_segments",
    "agena_segment_entries",
    "agena_session_segments",
    "agena_branch_entries",
    "agena_branch_sources",
    "agena_branch_rounds",
    "agena_branch_activities",
];

// Staging keeps the public UUIDs and permits out-of-order JSONL records.
const CONTENT_STAGING_TABLES: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS agena_content_capture_losses (content_id TEXT NOT NULL REFERENCES agena_content_streams(id), first_packet INTEGER NOT NULL CHECK(typeof(first_packet)='integer' AND first_packet>0), last_packet INTEGER NOT NULL CHECK(typeof(last_packet)='integer' AND last_packet>=first_packet), after_position INTEGER NOT NULL CHECK(typeof(after_position)='integer' AND after_position>=0), byte_count INTEGER NOT NULL CHECK(typeof(byte_count)='integer' AND byte_count>=0), diagnostic TEXT NOT NULL CHECK(length(CAST(diagnostic AS BLOB)) BETWEEN 1 AND 256), PRIMARY KEY(content_id,first_packet)) WITHOUT ROWID",
    "CREATE TABLE IF NOT EXISTS agena_content_blocks (content_id TEXT NOT NULL REFERENCES agena_content_streams(id), logical_start INTEGER NOT NULL CHECK(logical_start>=0), logical_end INTEGER NOT NULL CHECK(logical_end>logical_start), codec TEXT NOT NULL CHECK(codec IN ('utf8','events_v1','checkpoint_v1')), digest BLOB NOT NULL CHECK(length(digest)=32), body_bytes INTEGER NOT NULL CHECK(body_bytes BETWEEN 1 AND 33554432), body BLOB, PRIMARY KEY(content_id,logical_start), CHECK((body_bytes<=65536 AND body IS NOT NULL AND length(body)=body_bytes) OR (body_bytes>65536 AND body IS NULL))) WITHOUT ROWID",
    "CREATE TABLE IF NOT EXISTS agena_content_checkpoints (content_id TEXT NOT NULL, logical_sequence INTEGER NOT NULL CHECK(logical_sequence>0), block_start INTEGER NOT NULL CHECK(block_start>=0), PRIMARY KEY(content_id,logical_sequence), FOREIGN KEY(content_id,block_start) REFERENCES agena_content_blocks(content_id,logical_start)) WITHOUT ROWID",
    "CREATE TABLE IF NOT EXISTS agena_content_gaps (content_id TEXT NOT NULL REFERENCES agena_content_streams(id), logical_start INTEGER NOT NULL CHECK(logical_start>=0), logical_end INTEGER NOT NULL CHECK(logical_end>logical_start), reason TEXT NOT NULL, PRIMARY KEY(content_id,logical_start)) WITHOUT ROWID",
];

type ArchiveColumns = HashMap<String, Vec<String>>;

pub(super) struct Staged {
    root: tempfile::TempDir,
    pub request: RequestId,
    pub workspace: i64,
    digest: [u8; 32],
    last_user_message_at_ms: Option<i64>,
    columns: ArchiveColumns,
}

pub(super) struct Job {
    pub staged: Staged,
    pub response: oneshot::Sender<Result<CommitReceipt<SessionMetadata>>>,
    pub _permit: OwnedSemaphorePermit,
}

fn integer(value: &Value, key: &str) -> Result<i64> {
    value
        .get(key)
        .and_then(Value::as_i64)
        .ok_or_else(|| invalid(format!("archive requires integer {key}")))
}
fn text<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| invalid(format!("archive requires string {key}")))
}
fn object(value: &Value) -> Result<serde_json::Map<String, Value>> {
    value
        .as_object()
        .cloned()
        .ok_or_else(|| invalid("archive fact must be an object"))
}

fn archive_column(table: &str, column: &str) -> bool {
    !matches!(
        (table, column),
        ("agena_content_streams", "source_key")
            | ("agena_rounds", "input_source_count" | "input_source_bytes")
    )
}

async fn read_archive_columns(connection: &mut SqliteConnection) -> Result<ArchiveColumns> {
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT t.name,c.name FROM sqlite_schema t
         CROSS JOIN pragma_table_xinfo(t.name) c
         WHERE t.type='table' AND t.name GLOB 'agena_*' AND c.hidden=0
         ORDER BY t.name,c.cid",
    )
    .fetch_all(connection)
    .await
    .map_err(sql_error)?;
    let mut columns = ArchiveColumns::new();
    for (table, column) in rows {
        if archive_column(&table, &column) {
            columns.entry(table).or_default().push(column);
        }
    }
    Ok(columns)
}

fn table_columns<'a>(columns: &'a ArchiveColumns, table: &str) -> Result<&'a [String]> {
    columns
        .get(table)
        .map(Vec::as_slice)
        .ok_or_else(|| storage(format!("archive table {table} has no writable columns")))
}

/// Internal table names and conflict keys are constants, never archive input.
async fn put(
    connection: &mut SqliteConnection,
    columns: &ArchiveColumns,
    table: &str,
    value: &Value,
    keys: &[&str],
) -> Result<()> {
    let names = table_columns(columns, table)?
        .iter()
        .map(String::as_str)
        .filter(|name| value.get(*name).is_some())
        .collect::<Vec<_>>();
    if names.is_empty() {
        return Err(invalid("archive fact contains no recognized columns"));
    }
    let mut query = sqlx::QueryBuilder::<sqlx::Sqlite>::new(format!(
        "INSERT INTO {table} ({}) VALUES (",
        names.join(",")
    ));
    let mut separated = query.separated(",");
    for name in &names {
        match &value[*name] {
            Value::Null => {
                separated.push_bind(Option::<String>::None);
            }
            Value::String(value) => {
                separated.push_bind(value.clone());
            }
            Value::Number(value) => {
                separated.push_bind(
                    value
                        .as_i64()
                        .ok_or_else(|| invalid("archive integer exceeds SQLite range"))?,
                );
            }
            Value::Bool(value) => {
                separated.push_bind(i64::from(*value));
            }
            _ => {
                return Err(invalid(format!(
                    "archive scalar column {name} has a compound value"
                )));
            }
        }
    }
    separated.push_unseparated(")");
    if !keys.is_empty() {
        let updates = names
            .iter()
            .copied()
            .filter(|name| !keys.contains(name))
            .map(|name| format!("{name}=excluded.{name}"))
            .collect::<Vec<_>>();
        query.push(format!(" ON CONFLICT({}) ", keys.join(",")));
        if updates.is_empty() {
            query.push("DO NOTHING");
        } else {
            query.push(format!("DO UPDATE SET {}", updates.join(",")));
        }
    }
    query.build().execute(connection).await.map_err(sql_error)?;
    Ok(())
}

async fn raw(
    connection: &mut SqliteConnection,
    columns: &ArchiveColumns,
    table: &str,
    value: &Value,
    keys: &[&str],
) -> Result<()> {
    let mut value = object(value)?;
    if value.contains_key("session_id") {
        value.insert("session_id".into(), json!(1));
    }
    put(connection, columns, table, &Value::Object(value), keys).await
}

async fn entry(
    connection: &mut SqliteConnection,
    columns: &ArchiveColumns,
    data: &Value,
    member: bool,
    ordinal: i64,
) -> Result<Option<i64>> {
    let facts = data
        .get("facts")
        .ok_or_else(|| invalid("archive entry has no facts"))?;
    let e: EntryView = serde_json::from_value(facts["entry"].clone()).map_err(invalid)?;
    raw(
        connection,
        columns,
        "agena_executions",
        &facts["execution"],
        &["id"],
    )
    .await?;
    if !facts["execution_purpose"].is_null() {
        let purpose: ControlExecutionPurpose =
            serde_json::from_value(facts["execution_purpose"].clone()).map_err(invalid)?;
        raw(
            connection,
            columns,
            "agena_execution_purposes",
            &json!({"execution_id": e.execution_id.to_string(), "purpose": purpose}),
            &["execution_id"],
        )
        .await?;
    }
    raw(
        connection,
        columns,
        "agena_attempts",
        &facts["attempt"],
        &["execution_id", "generation"],
    )
    .await?;
    if !facts["tool"].is_null() {
        raw(
            connection,
            columns,
            "agena_tool_calls",
            &facts["tool"],
            &["id"],
        )
        .await?;
    }
    if !facts["interaction"].is_null() {
        raw(
            connection,
            columns,
            "agena_interactions",
            &facts["interaction"],
            &["id"],
        )
        .await?;
    }
    if let Some(activities) = facts["activities"].as_array() {
        if activities.len() > 16 {
            return Err(invalid("archive activity identity budget exceeded"));
        }
        for activity in activities {
            raw(
                connection,
                columns,
                "agena_background_activities",
                activity,
                &["id"],
            )
            .await?;
        }
    }
    let created_at_ms = integer(facts, "created_at_ms")?;
    let mut row = json!({"id":e.id.get(),"origin_session_id":1,"execution_id":e.execution_id.to_string(),"generation":e.generation.get(),"message_id":e.message_id.get(),"revision":e.revision.get(),"round_id":facts["round_id"],"created_at_ms":created_at_ms,"body_json":null,"tool_id":null,"interaction_id":null,"content_id":null});
    let kind = match &e.payload {
        EntryPayload::User {
            document,
            author,
            origin,
        } => {
            row["message_id"] = Value::Null;
            row["body_json"] = json!(encode_user_body(
                document,
                author.as_ref(),
                origin.as_ref()
            )?);
            "user"
        }
        EntryPayload::Text { content_id } => {
            row["content_id"] = json!(content_id);
            "text"
        }
        EntryPayload::Reasoning { content_id } => {
            row["content_id"] = json!(content_id);
            "reasoning"
        }
        EntryPayload::Tool { tool_id } => {
            row["tool_id"] = json!(tool_id.get());
            "tool"
        }
        EntryPayload::Interaction { interaction_id } => {
            row["interaction_id"] = json!(interaction_id.get());
            row["tool_id"] = facts["interaction"]["tool_id"].clone();
            "interaction"
        }
        EntryPayload::Notice { text } => {
            row["body_json"] = json!(encode(text)?);
            "notice"
        }
        EntryPayload::Compaction { summary, .. } => {
            row["content_id"] = json!(summary);
            row["body_json"] = json!(encode(&e.payload)?);
            "compaction"
        }
        EntryPayload::ContextCleared { .. } => {
            row["body_json"] = json!(encode(&e.payload)?);
            "compaction"
        }
    };
    row["kind"] = json!(kind);
    put(connection, columns, "agena_entries", &row, &[]).await?;
    if !facts["branch_facts"].is_null() {
        let frozen = &facts["branch_facts"];
        serde_json::from_value::<super::branches::EntryFacts>(frozen.clone()).map_err(invalid)?;
        let mut frozen_columns = object(frozen)?;
        frozen_columns.insert("session_id".into(), json!(1));
        frozen_columns.insert("entry_id".into(), json!(e.id.get()));
        for (field, limit) in [("interactions", 128), ("activities", 16), ("blobs", 16)] {
            let values = frozen[field]
                .as_array()
                .filter(|values| values.len() <= limit)
                .ok_or_else(|| invalid("archive frozen entry identity budget exceeded"))?;
            let encoded = (!values.is_empty()).then(|| encode(values)).transpose()?;
            frozen_columns.insert(format!("{field}_json"), json!(encoded));
        }
        put(
            connection,
            columns,
            "agena_branch_entries",
            &Value::Object(frozen_columns),
            &["session_id", "entry_id"],
        )
        .await?;
        let declared = frozen["sources"]
            .as_array()
            .filter(|sources| sources.len() <= 128)
            .ok_or_else(|| invalid("archive frozen source identity budget exceeded"))?;
        let sources = facts["branch_sources"]
            .as_array()
            .map_or(&[][..], Vec::as_slice);
        let mut visible = declared
            .iter()
            .map(|id| {
                id.as_str()
                    .ok_or_else(|| invalid("invalid archive frozen source identity"))
            })
            .collect::<Result<HashSet<_>>>()?;
        if visible.len() != declared.len() || sources.len() != declared.len() {
            return Err(invalid(
                "archive frozen source list disagrees with its heads",
            ));
        }
        for source in sources {
            if !visible.remove(text(source, "content_id")?) {
                return Err(invalid(
                    "archive frozen source list disagrees with its heads",
                ));
            }
            if integer(source, "entry_id")? != e.id.get() {
                return Err(invalid("archive frozen source belongs to another entry"));
            }
            let mut source = object(source)?;
            source.insert("session_id".into(), json!(1));
            put(
                connection,
                columns,
                "agena_branch_sources",
                &Value::Object(source),
                &["session_id", "content_id"],
            )
            .await?;
        }
        if !facts["branch_round"].is_null() {
            let mut round = object(&facts["branch_round"])?;
            if round.get("round_id") != Some(&facts["round_id"]) {
                return Err(invalid("archive frozen round belongs to another entry"));
            }
            round.insert("session_id".into(), json!(1));
            put(
                connection,
                columns,
                "agena_branch_rounds",
                &Value::Object(round),
                &["session_id", "round_id"],
            )
            .await?;
        }
        if let Some(activities) = facts["branch_activities"].as_array() {
            if activities.len() > super::activity::MAX_ACTIVITIES_PER_TOOL as usize {
                return Err(invalid("archive frozen activity identity budget exceeded"));
            }
            for activity in activities {
                let mut activity = object(activity)?;
                activity.insert("session_id".into(), json!(1));
                put(
                    connection,
                    columns,
                    "agena_branch_activities",
                    &Value::Object(activity),
                    &["session_id", "activity_id"],
                )
                .await?;
            }
        }
    }
    sqlx::query("INSERT INTO import_entries VALUES (?,?,?)")
        .bind(e.id.get())
        .bind(i64::from(member))
        .bind(ordinal)
        .execute(&mut *connection)
        .await
        .map_err(sql_error)?;
    if let Some(sources) = facts["sources"].as_array() {
        if sources.len() > 128 {
            return Err(invalid("archive source identity budget exceeded"));
        }
        for source in sources {
            sqlx::query("INSERT INTO import_source_owners VALUES (?,?)")
                .bind(
                    source
                        .as_str()
                        .ok_or_else(|| invalid("invalid source identity"))?,
                )
                .bind(e.id.get())
                .execute(&mut *connection)
                .await
                .map_err(sql_error)?;
        }
    }
    if let Some(blobs) = facts["blobs"].as_array() {
        if blobs.len() > 16 {
            return Err(invalid("archive blob reference budget exceeded"));
        }
        for (index, blob) in blobs.iter().enumerate() {
            let descriptor: BlobDescriptor =
                serde_json::from_value(blob.clone()).map_err(invalid)?;
            put(
                connection,
                columns,
                "agena_blob_references",
                &json!({"entry_id":e.id.get(),"blob_id":descriptor.id,"field":index.to_string()}),
                &[],
            )
            .await?;
        }
    }
    Ok((member && kind == "user").then_some(created_at_ms))
}

struct ActiveBlock {
    content: String,
    start: i64,
    end: i64,
    codec: String,
    digest: Vec<u8>,
    length: i64,
    received: i64,
    hash: Sha256,
}
struct ActiveBlob {
    id: String,
    length: i64,
    received: i64,
    digest: [u8; 32],
    hash: Sha256,
    chunk: Vec<u8>,
}

async fn close_block(
    connection: &mut SqliteConnection,
    block: &mut Option<ActiveBlock>,
) -> Result<()> {
    if let Some(block) = block.take() {
        if block.received != block.length || block.hash.finalize().as_slice() != block.digest {
            return Err(invalid("archive content block length/checksum mismatch"));
        }
        let mut body = Vec::with_capacity(block.length as usize);
        {
            let mut pieces = sqlx::query_scalar::<_, Vec<u8>>(
                "SELECT body FROM import_pieces WHERE content_id=? AND block_start=? ORDER BY byte_offset",
            )
            .bind(&block.content)
            .bind(block.start)
            .fetch(&mut *connection);
            while let Some(piece) = pieces.try_next().await.map_err(sql_error)? {
                body.extend_from_slice(&piece);
            }
        }
        validate_block(
            connection,
            &block.content,
            block.start,
            block.end,
            &block.codec,
            &body,
        )
        .await?;
        let inline = (body.len() <= PIECE).then_some(body.as_slice());
        sqlx::query("INSERT INTO agena_content_blocks (content_id,logical_start,logical_end,codec,digest,body_bytes,body) VALUES (?,?,?,?,?,?,?)")
            .bind(&block.content)
            .bind(block.start)
            .bind(block.end)
            .bind(&block.codec)
            .bind(&block.digest)
            .bind(block.length)
            .bind(inline)
            .execute(&mut *connection)
            .await
            .map_err(sql_error)?;
        if inline.is_some() {
            sqlx::query("DELETE FROM import_pieces WHERE content_id=? AND block_start=?")
                .bind(&block.content)
                .bind(block.start)
                .execute(&mut *connection)
                .await
                .map_err(sql_error)?;
        }
    }
    Ok(())
}
async fn close_blob(
    connection: &mut SqliteConnection,
    blob: &mut Option<ActiveBlob>,
) -> Result<()> {
    if let Some(mut blob) = blob.take() {
        if blob.received != blob.length || <[u8; 32]>::from(blob.hash.finalize()) != blob.digest {
            return Err(invalid("archive blob length/checksum mismatch"));
        }
        if !blob.chunk.is_empty() {
            let start = blob.received - blob.chunk.len() as i64;
            sqlx::query("INSERT INTO agena_blob_chunks VALUES (?,?,?,?)")
                .bind(&blob.id)
                .bind(start)
                .bind(Sha256::digest(&blob.chunk).to_vec())
                .bind(std::mem::take(&mut blob.chunk))
                .execute(&mut *connection)
                .await
                .map_err(sql_error)?;
        }
    }
    Ok(())
}

pub(super) async fn prepare(
    path: &Path,
    request: RequestId,
    workspace: i64,
    validator: &SessionImportValidator,
) -> Result<Staged> {
    if workspace <= 0 {
        return Err(invalid("workspace identity must be positive"));
    }
    let temporary = agena_storage_layout::StorageLayout::current().temp_dir();
    agena_storage_layout::secure_directory(&temporary).map_err(storage)?;
    let root = tempfile::tempdir_in(temporary).map_err(storage)?;
    let options = SqliteConnectOptions::new()
        .filename(root.path().join("archive.sqlite"))
        .create_if_missing(true)
        .foreign_keys(false)
        // A source header can be as large as one archive record.
        .row_buffer_size(1)
        .pragma("journal_mode", "DELETE")
        .pragma("cache_size", "-4096");
    let mut connection = SqliteConnection::connect_with(&options)
        .await
        .map_err(storage)?;
    // This private database is attached only after the complete archive validates.
    let mut db = connection.begin().await.map_err(storage)?;
    sqlx::query("CREATE TABLE agena_workspaces(id INTEGER PRIMARY KEY)")
        .execute(&mut *db)
        .await
        .map_err(storage)?;
    sqlx::query("INSERT INTO agena_workspaces VALUES (1)")
        .execute(&mut *db)
        .await
        .map_err(storage)?;
    for table in schema::ARCHIVE_TABLES.iter().chain(CONTENT_STAGING_TABLES) {
        sqlx::query(table)
            .execute(&mut *db)
            .await
            .map_err(storage)?;
    }
    for index in schema::ARCHIVE_INDEXES {
        sqlx::query(index)
            .execute(&mut *db)
            .await
            .map_err(storage)?;
    }
    for ddl in [
        schema::ARCHIVE_ROUND_INPUTS,
        schema::ARCHIVE_ROUND_REPLAYS,
        "CREATE TABLE agena_branch_sources(
            session_id INTEGER NOT NULL REFERENCES agena_sessions(id),
            entry_id INTEGER NOT NULL REFERENCES agena_entries(id),
            content_id TEXT NOT NULL REFERENCES agena_content_streams(id),
            head_json TEXT NOT NULL CHECK(json_valid(head_json)),
            PRIMARY KEY(session_id,content_id)
         ) WITHOUT ROWID",
        "CREATE TABLE agena_round_input_sources(
            round_id INTEGER NOT NULL,
            entry_id INTEGER NOT NULL,
            content_id TEXT NOT NULL REFERENCES agena_content_streams(id),
            head_json TEXT NOT NULL CHECK(json_valid(head_json)),
            PRIMARY KEY(round_id,content_id),
            FOREIGN KEY(round_id,entry_id) REFERENCES agena_round_inputs(round_id,entry_id)
         ) WITHOUT ROWID",
        "CREATE TABLE import_json_remaps(field TEXT NOT NULL,row_id INTEGER NOT NULL,old TEXT NOT NULL,reply_json TEXT,PRIMARY KEY(field,row_id)) WITHOUT ROWID",
        "CREATE TABLE import_entries(id INTEGER PRIMARY KEY,member INTEGER NOT NULL,ordinal INTEGER NOT NULL)",
        "CREATE TABLE import_source_owners(id TEXT PRIMARY KEY,entry_id INTEGER NOT NULL)",
        "CREATE TABLE import_source_bounds(id TEXT PRIMARY KEY,physical_start INTEGER NOT NULL)",
        "CREATE TABLE import_maps(kind TEXT NOT NULL,old TEXT NOT NULL,new TEXT NOT NULL,epoch TEXT,PRIMARY KEY(kind,old)) WITHOUT ROWID",
        "CREATE TABLE import_pieces(content_id TEXT NOT NULL,block_start INTEGER NOT NULL,byte_offset INTEGER NOT NULL,body BLOB NOT NULL,PRIMARY KEY(content_id,block_start,byte_offset),FOREIGN KEY(content_id,block_start) REFERENCES agena_content_blocks(content_id,logical_start)) WITHOUT ROWID",
        "CREATE TABLE import_events(content_id TEXT NOT NULL,sequence INTEGER NOT NULL,block_start INTEGER NOT NULL,checkpoint INTEGER NOT NULL,PRIMARY KEY(content_id,sequence)) WITHOUT ROWID",
        "CREATE TABLE import_validation_state(content_id TEXT PRIMARY KEY,terminal_json TEXT,document_json TEXT)",
        "CREATE INDEX import_entry_order ON import_entries(member,ordinal)",
    ] {
        sqlx::query(ddl).execute(&mut *db).await.map_err(storage)?;
    }
    let columns = read_archive_columns(&mut db).await?;
    let file = tokio::fs::File::open(path).await.map_err(storage)?;
    let mut input = BufReader::with_capacity(PIECE, file);
    let mut hash = Sha256::new();
    let mut records = 0u64;
    let mut bytes = 0u64;
    let mut entries = 0i64;
    let mut last_user_message_at_ms = None;
    let mut header = false;
    let mut footer = false;
    let mut expected_entries = 0u64;
    let mut block = None;
    let mut blob = None;
    loop {
        let mut line = Vec::new();
        let count = (&mut input)
            .take((MAX_LINE + 2) as u64)
            .read_until(b'\n', &mut line)
            .await
            .map_err(storage)?;
        if count == 0 {
            break;
        }
        if footer {
            return Err(invalid(
                "archive contains trailing records after its footer",
            ));
        }
        if count > MAX_LINE + 1 || line.last() != Some(&b'\n') {
            return Err(invalid("archive line is over budget or unterminated"));
        }
        let record: Value = serde_json::from_slice(&line).map_err(invalid)?;
        let kind = text(&record, "kind")?;
        let data = &record["data"];
        if kind != "content_piece" {
            close_block(&mut db, &mut block).await?;
        }
        if kind != "blob_piece" {
            close_blob(&mut db, &mut blob).await?;
        }
        if kind == "footer" {
            let report: ExportReport = serde_json::from_value(data.clone()).map_err(invalid)?;
            if report.records != records
                || report.bytes != bytes
                || report.entries != entries as u64
                || report.entries != expected_entries
                || report.sha256 != <[u8; 32]>::from(hash.clone().finalize())
            {
                return Err(invalid("archive footer count/checksum mismatch"));
            }
            footer = true;
            continue;
        }
        if !header && kind != "header" {
            return Err(invalid("archive must begin with its header"));
        }
        hash.update(&line);
        records = records
            .checked_add(1)
            .ok_or_else(|| invalid("archive record overflow"))?;
        bytes = bytes
            .checked_add(count as u64)
            .ok_or_else(|| invalid("archive byte overflow"))?;
        match kind {
            "header" => {
                if header || data["format"] != "agena.session-kernel" || data["version"] != 1 {
                    return Err(invalid("unsupported or repeated archive header"));
                }
                let metadata: SessionMetadata =
                    serde_json::from_value(data["snapshot"]["metadata"].clone())
                        .map_err(invalid)?;
                expected_entries = data["snapshot"]["entry_count"]
                    .as_u64()
                    .ok_or_else(|| invalid("archive header has no entry count"))?;
                let mut configuration: SessionConfiguration =
                    serde_json::from_value(data["configuration"].clone()).map_err(invalid)?;
                configuration.parent_session_id = None;
                configuration.relation_kind = agena_domain::SessionRelationKind::Root;
                lifecycle::normalize_configuration_permissions(&mut configuration)?;
                validator(&configuration).map_err(invalid)?;
                put(&mut db,&columns,"agena_sessions",&json!({"id":1,"workspace_id":1,"root_session_id":1,"title":metadata.title,"favorite":metadata.favorite,"pinned":metadata.pinned,"selection_json":metadata.selection.map(|value|encode(&value)).transpose()?,"configuration_json":encode(&configuration)?,"created_at_ms":now_ms(),"updated_at_ms":now_ms()}),&[]).await?;
                let mut window = object(&data["prompt_window"])?;
                window.insert("session_id".into(), json!(1));
                put(
                    &mut db,
                    &columns,
                    "agena_prompt_windows",
                    &Value::Object(window),
                    &[],
                )
                .await?;
                header = true;
            }
            "entry" | "support_entry" => {
                let member = kind == "entry";
                let ordinal = if member { entries } else { -1 };
                let timestamp = entry(&mut db, &columns, data, member, ordinal).await?;
                last_user_message_at_ms = last_user_message_at_ms.max(timestamp);
                entries += i64::from(member);
            }
            "attempt" => {
                raw(
                    &mut db,
                    &columns,
                    "agena_attempts",
                    data,
                    &["execution_id", "generation"],
                )
                .await?
            }
            "round" => {
                raw(&mut db, &columns, "agena_rounds", data, &["id"]).await?;
                if data["purpose"] == "compaction" {
                    put(&mut db,&columns,"agena_compaction_rounds",&json!({"round_id":data["id"],"base_checkpoint_entry_id":data["base_checkpoint_entry_id"],"rewrite_epoch":data["input_rewrite_epoch"]}),&["round_id"]).await?;
                }
            }
            "round_input" => {
                raw(
                    &mut db,
                    &columns,
                    "agena_round_inputs",
                    data,
                    &["round_id", "ordinal"],
                )
                .await?
            }
            "round_input_source" => {
                raw(
                    &mut db,
                    &columns,
                    "agena_round_input_sources",
                    data,
                    &["round_id", "content_id"],
                )
                .await?
            }
            "round_input_replay" => {
                raw(
                    &mut db,
                    &columns,
                    "agena_round_input_replays",
                    data,
                    &["round_id", "source_round_id"],
                )
                .await?
            }
            "content" => {
                let head: ContentStreamView =
                    serde_json::from_value(data.clone()).map_err(invalid)?;
                let owner: i64 =
                    sqlx::query_scalar("SELECT entry_id FROM import_source_owners WHERE id=?")
                        .bind(head.id.to_string())
                        .fetch_one(&mut *db)
                        .await
                        .map_err(sql_error)?;
                if owner != head.entry.get() {
                    return Err(KernelError::Ownership);
                }
                let e = sqlx::query("SELECT execution_id,generation FROM agena_entries WHERE id=?")
                    .bind(owner)
                    .fetch_one(&mut *db)
                    .await
                    .map_err(sql_error)?;
                put(&mut db,&columns,"agena_content_streams",&json!({"id":head.id,"entry_id":owner,"session_id":1,"execution_id":e.try_get::<String,_>("execution_id").map_err(storage)?,"generation":e.try_get::<i64,_>("generation").map_err(storage)?,"activity_id":head.activity_id,"kind":commands::enum_text(&head.kind)?,"epoch":head.epoch,"committed_head":head.committed,"ordinal":head.ordinal,"lost_bytes":head.capture_loss.committed_bytes,"lost_packets":head.capture_loss.committed_packets,"capture_state":commands::enum_text(&head.state)?,"retained_start":head.retained_start,"capture_error":head.capture_error}),&[]).await?;
                sqlx::query("INSERT INTO import_source_bounds VALUES (?,?)")
                    .bind(head.id.to_string())
                    .bind(integer(data, "physical_start")?)
                    .execute(&mut *db)
                    .await
                    .map_err(sql_error)?;
            }
            "content_gap" => raw(&mut db, &columns, "agena_content_gaps", data, &[]).await?,
            "content_checkpoint" => {
                raw(&mut db, &columns, "agena_content_checkpoints", data, &[]).await?
            }
            "capture_loss" => {
                let loss: CaptureLoss =
                    serde_json::from_value(data["loss"].clone()).map_err(invalid)?;
                put(&mut db,&columns,"agena_content_capture_losses",&json!({"content_id":data["content_id"],"first_packet":loss.first_packet,"last_packet":loss.last_packet,"after_position":loss.after_position,"byte_count":loss.bytes,"diagnostic":loss.diagnostic}),&[]).await?;
            }
            "content_block" => {
                let length = integer(data, "byte_len")?;
                if length <= 0 || length > MAX_BLOCK {
                    return Err(invalid("archive content block exceeds its bounded format"));
                }
                let digest: Vec<u8> =
                    serde_json::from_value(data["sha256"].clone()).map_err(invalid)?;
                if digest.len() != 32 {
                    return Err(invalid("archive block digest length is invalid"));
                }
                let content = text(data, "content_id")?.to_owned();
                let start = integer(data, "logical_start")?;
                let end = integer(data, "logical_end")?;
                let codec = text(data, "codec")?.to_owned();
                block = Some(ActiveBlock {
                    content,
                    start,
                    end,
                    codec,
                    digest,
                    length,
                    received: 0,
                    hash: Sha256::new(),
                });
            }
            "content_piece" => {
                let current = block
                    .as_mut()
                    .ok_or_else(|| invalid("content piece has no preceding block"))?;
                let encoded = text(data, "bytes_hex")?;
                if encoded.len() > PIECE * 2 {
                    return Err(invalid("archive content piece exceeds 64 KiB"));
                }
                let piece = hex::decode(encoded).map_err(invalid)?;
                if text(data, "content_id")? != current.content
                    || integer(data, "block_start")? != current.start
                    || integer(data, "byte_offset")? != current.received
                    || piece.is_empty()
                    || current.received + piece.len() as i64 > current.length
                    || piece.len() != PIECE
                        && current.received + piece.len() as i64 != current.length
                {
                    return Err(invalid("archive content pieces are not contiguous"));
                }
                sqlx::query("INSERT INTO import_pieces VALUES (?,?,?,?)")
                    .bind(&current.content)
                    .bind(current.start)
                    .bind(current.received)
                    .bind(&piece)
                    .execute(&mut *db)
                    .await
                    .map_err(sql_error)?;
                current.hash.update(&piece);
                current.received += piece.len() as i64;
            }
            "blob" => {
                let descriptor: BlobDescriptor =
                    serde_json::from_value(data["descriptor"].clone()).map_err(invalid)?;
                let length = i64::try_from(descriptor.byte_len).map_err(invalid)?;
                if let Some(old) =
                    sqlx::query("SELECT expected_bytes,digest FROM agena_blobs WHERE id=?")
                        .bind(descriptor.id.to_string())
                        .fetch_optional(&mut *db)
                        .await
                        .map_err(sql_error)?
                    && (old.try_get::<i64, _>("expected_bytes").map_err(storage)? != length
                        || old.try_get::<Vec<u8>, _>("digest").map_err(storage)?
                            != descriptor.sha256)
                {
                    return Err(invalid("conflicting repeated archive blob descriptor"));
                }
                // A blob may be referenced by several entries and re-exported.
                sqlx::query("DELETE FROM agena_blob_chunks WHERE blob_id=?")
                    .bind(descriptor.id.to_string())
                    .execute(&mut *db)
                    .await
                    .map_err(sql_error)?;
                sqlx::query("INSERT INTO agena_blobs(id,session_id,state,expected_bytes,received_bytes,digest,created_at_ms) VALUES (?,1,'ready',?,?,?,?) ON CONFLICT(id) DO UPDATE SET expected_bytes=excluded.expected_bytes,received_bytes=excluded.received_bytes,digest=excluded.digest").bind(descriptor.id.to_string()).bind(length).bind(length).bind(descriptor.sha256.to_vec()).bind(now_ms()).execute(&mut *db).await.map_err(sql_error)?;
                blob = Some(ActiveBlob {
                    id: descriptor.id.to_string(),
                    length,
                    received: 0,
                    digest: descriptor.sha256,
                    hash: Sha256::new(),
                    chunk: Vec::with_capacity(128 * 1024),
                });
            }
            "blob_piece" => {
                let current = blob
                    .as_mut()
                    .ok_or_else(|| invalid("blob piece has no descriptor"))?;
                let encoded = text(data, "bytes_hex")?;
                if encoded.len() > PIECE * 2 {
                    return Err(invalid("archive blob piece exceeds 64 KiB"));
                }
                let piece = hex::decode(encoded).map_err(invalid)?;
                if text(data, "blob_id")? != current.id
                    || integer(data, "byte_offset")? != current.received
                    || current.received + piece.len() as i64 > current.length
                    || piece.is_empty() && current.length != 0
                    || piece.len() != PIECE
                        && current.received + piece.len() as i64 != current.length
                {
                    return Err(invalid("archive blob pieces are not contiguous"));
                }
                current.hash.update(&piece);
                current.received += piece.len() as i64;
                current.chunk.extend_from_slice(&piece);
                if current.chunk.len() == 128 * 1024 {
                    sqlx::query("INSERT INTO agena_blob_chunks VALUES (?,?,?,?)")
                        .bind(&current.id)
                        .bind(current.received - current.chunk.len() as i64)
                        .bind(Sha256::digest(&current.chunk).to_vec())
                        .bind(std::mem::take(&mut current.chunk))
                        .execute(&mut *db)
                        .await
                        .map_err(sql_error)?;
                    current.chunk = Vec::with_capacity(128 * 1024);
                }
            }
            _ => return Err(invalid(format!("unknown kernel archive record {kind}"))),
        }
    }
    if !header || !footer {
        return Err(invalid("archive has no complete success footer"));
    }
    validate(&mut db).await?;
    make_maps(&mut db).await?;
    stage_json(&mut db).await?;
    let mut digest = Sha256::new();
    digest.update(b"agena/kernel/import/v1");
    digest.update(workspace.to_le_bytes());
    digest.update(hash.finalize());
    db.commit().await.map_err(storage)?;
    connection.close().await.map_err(storage)?;
    Ok(Staged {
        root,
        request,
        workspace,
        digest: digest.finalize().into(),
        last_user_message_at_ms,
        columns,
    })
}

/// Validate terminal frames a cell at a time. A 16 MiB frame never becomes a
/// second screen tree; only geometry, row presence and one cell run are live.
#[derive(Default)]
struct Cells {
    rows: [u64; 4],
    max_row: u16,
    max_end: usize,
    max_text: usize,
    count: usize,
}
impl<'de> serde::Deserialize<'de> for Cells {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = Cells;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("ordered terminal cells")
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> std::result::Result<Cells, A::Error> {
                use unicode_width::UnicodeWidthStr as _;
                let mut cells = Cells::default();
                let mut previous = None;
                while let Some(run) = seq.next_element::<agena_domain::TerminalCellRun>()? {
                    let width = run.text.as_str().width();
                    let end = usize::from(run.col) + width;
                    if run.row >= 200
                        || run.col >= 400
                        || run.text.is_empty()
                        || run.text.chars().any(char::is_control)
                        || width == 0
                        || end > 400
                        || previous.is_some_and(|(row, last)| {
                            run.row < row || run.row == row && usize::from(run.col) < last
                        })
                    {
                        return Err(serde::de::Error::custom("invalid terminal cell run"));
                    }
                    cells.count += 1;
                    if cells.count > 40_000 {
                        return Err(serde::de::Error::custom("terminal cell budget exceeded"));
                    }
                    cells.rows[usize::from(run.row) / 64] |= 1u64 << (run.row % 64);
                    cells.max_row = cells.max_row.max(run.row);
                    cells.max_end = cells.max_end.max(end);
                    cells.max_text = cells.max_text.max(run.text.len());
                    previous = Some((run.row, end));
                }
                Ok(cells)
            }
        }
        deserializer.deserialize_seq(Visitor)
    }
}
#[derive(serde::Deserialize)]
struct Screen {
    rows: u16,
    cols: u16,
    cursor_row: u16,
    cursor_col: u16,
    cursor_visible: bool,
    alternate_screen: bool,
    bracketed_paste: bool,
    application_cursor: bool,
    cells: Cells,
}
impl Screen {
    fn validate(&self, changed: Option<&[u16]>) -> Result<()> {
        let _modes = (
            self.cursor_visible,
            self.bracketed_paste,
            self.application_cursor,
        );
        if self.rows == 0
            || self.cols == 0
            || self.rows > 200
            || self.cols > 400
            || usize::from(self.rows) * usize::from(self.cols) > 40_000
            || self.cursor_row >= self.rows
            || self.cursor_col >= self.cols
            || self.cells.count > 0
                && (self.cells.max_row >= self.rows
                    || self.cells.max_end > usize::from(self.cols)
                    || self.cells.max_text > usize::from(self.cols) * 64)
        {
            return Err(invalid("invalid archive terminal geometry"));
        }
        if let Some(rows) = changed {
            if rows.len() > 200
                || rows.iter().any(|r| *r >= self.rows)
                || rows.windows(2).any(|p| p[0] >= p[1])
            {
                return Err(invalid("invalid archive terminal patch rows"));
            }
            for row in 0..self.rows {
                if self.cells.rows[usize::from(row) / 64] & (1u64 << (row % 64)) != 0
                    && rows.binary_search(&row).is_err()
                {
                    return Err(invalid("terminal patch cell lies outside changed rows"));
                }
            }
        }
        Ok(())
    }
}

async fn validate_block(
    connection: &mut SqliteConnection,
    id: &str,
    start: i64,
    end: i64,
    codec: &str,
    body: &[u8],
) -> Result<()> {
    let source = sqlx::query(
        "SELECT kind,retained_start,committed_head FROM agena_content_streams WHERE id=?",
    )
    .bind(id)
    .fetch_one(&mut *connection)
    .await
    .map_err(sql_error)?;
    let kind: String = source.try_get("kind").map_err(storage)?;
    let floor: i64 = source.try_get("retained_start").map_err(storage)?;
    if end <= start
        || end
            > source
                .try_get::<i64, _>("committed_head")
                .map_err(storage)?
    {
        return Err(invalid("archive block exceeds committed source"));
    }
    if codec == "utf8" {
        if kind != "text" || end - start != body.len() as i64 {
            return Err(invalid("archive text block range is invalid"));
        }
        std::str::from_utf8(body).map_err(invalid)?;
        return Ok(());
    }
    if codec != "events_v1"
        || kind == "text"
        || !body.starts_with(b"AGE1")
        || end - start > 4096
        || body.len() > agena_domain::MAX_TERMINAL_FRAME_BYTES + 17
    {
        return Err(invalid("invalid archive event codec or range"));
    }
    let prior = sqlx::query("SELECT * FROM import_validation_state WHERE content_id=?")
        .bind(id)
        .fetch_optional(&mut *connection)
        .await
        .map_err(sql_error)?;
    let mut geometry = prior
        .as_ref()
        .and_then(|r| {
            r.try_get::<Option<String>, _>("terminal_json")
                .ok()
                .flatten()
        })
        .map(|s| decode::<(u16, u16, bool)>(&s))
        .transpose()?;
    let mut document = prior
        .as_ref()
        .and_then(|r| {
            r.try_get::<Option<String>, _>("document_json")
                .ok()
                .flatten()
        })
        .map(|s| decode::<agena_domain::ContentDocument>(&s))
        .transpose()?;
    let mut offset = 4usize;
    let mut sequence = start;
    while offset < body.len() {
        if sequence >= end || body.len() - offset < 13 {
            return Err(invalid("archive event count/header mismatch"));
        }
        let tag = body[offset];
        let length =
            u32::from_le_bytes(body[offset + 9..offset + 13].try_into().expect("header")) as usize;
        offset += 13;
        if length > agena_domain::MAX_TERMINAL_FRAME_BYTES || length > body.len() - offset {
            return Err(invalid("archive event payload length is invalid"));
        }
        let payload = &body[offset..offset + length];
        offset += length;
        sequence += 1;
        let checkpoint = match tag {
            1 | 2 if kind == "log" => {
                std::str::from_utf8(payload).map_err(invalid)?;
                false
            }
            3 if kind == "structured" => {
                let next: agena_domain::ContentDocument =
                    serde_json::from_slice(payload).map_err(invalid)?;
                next.validate().map_err(invalid)?;
                document = Some(next);
                true
            }
            4 if kind == "structured" => {
                let (base, mutation): (u64, agena_domain::DocumentMutation) =
                    serde_json::from_slice(payload).map_err(invalid)?;
                if base == 0 || base != sequence as u64 - 1 {
                    return Err(invalid("archive document patch base mismatch"));
                }
                mutation.validate().map_err(invalid)?;
                if let Some(prior) = &document {
                    document = Some(prior.updated(&mutation).map_err(invalid)?);
                } else if sequence > floor {
                    return Err(invalid("archive document patch has no retained checkpoint"));
                }
                false
            }
            5 if kind == "terminal" => {
                let screen: Screen = serde_json::from_slice(payload).map_err(invalid)?;
                screen.validate(None)?;
                geometry = Some((screen.rows, screen.cols, screen.alternate_screen));
                true
            }
            6 if kind == "terminal" => {
                let (base, screen, changed): (u64, Screen, Vec<u16>) =
                    serde_json::from_slice(payload).map_err(invalid)?;
                screen.validate(Some(&changed))?;
                let next = (screen.rows, screen.cols, screen.alternate_screen);
                if base == 0
                    || base != sequence as u64 - 1
                    || geometry.is_some_and(|old| old != next)
                    || geometry.is_none() && sequence > floor
                {
                    return Err(invalid("archive terminal patch base/geometry mismatch"));
                }
                false
            }
            7 if kind == "document" => {
                let (input, checkpoint): (
                    agena_domain::ContentInput,
                    Option<agena_domain::ContentDocument>,
                ) = serde_json::from_slice(payload).map_err(invalid)?;
                input.validate().map_err(invalid)?;
                if !matches!(
                    input,
                    agena_domain::ContentInput::Text { .. }
                        | agena_domain::ContentInput::Log { .. }
                        | agena_domain::ContentInput::Structured { .. }
                ) {
                    return Err(invalid("archive mixed event kind is invalid"));
                }
                let indexed = checkpoint.is_some();
                if let Some(next) = checkpoint {
                    next.validate().map_err(invalid)?;
                    document = Some(next);
                } else if let agena_domain::ContentInput::Structured { event } = &input {
                    if document.is_none() && start == 0 {
                        document = Some(agena_domain::ContentDocument::default());
                    }
                    if let Some(prior) = &document {
                        document = Some(prior.updated(event).map_err(invalid)?);
                    } else if sequence > floor {
                        return Err(invalid("archive mixed mutation has no retained checkpoint"));
                    }
                }
                indexed
            }
            _ => return Err(invalid("archive event tag does not match source kind")),
        };
        sqlx::query("INSERT INTO import_events VALUES (?,?,?,?)")
            .bind(id)
            .bind(sequence)
            .bind(start)
            .bind(i64::from(checkpoint))
            .execute(&mut *connection)
            .await
            .map_err(sql_error)?;
    }
    if sequence != end {
        return Err(invalid("archive event block trailer/count mismatch"));
    }
    sqlx::query("INSERT INTO import_validation_state VALUES (?,?,?) ON CONFLICT(content_id) DO UPDATE SET terminal_json=excluded.terminal_json,document_json=excluded.document_json").bind(id).bind(geometry.as_ref().map(encode).transpose()?).bind(document.as_ref().map(encode).transpose()?).execute(&mut *connection).await.map_err(sql_error)?;
    Ok(())
}

async fn reject_query(
    connection: &mut SqliteConnection,
    sql: &str,
    diagnostic: &str,
) -> Result<()> {
    let invalid: bool = sqlx::query_scalar(sql)
        .fetch_one(connection)
        .await
        .map_err(sql_error)?;
    if invalid {
        return Err(invalid_archive(diagnostic));
    }
    Ok(())
}
fn invalid_archive(message: &str) -> KernelError {
    invalid(message)
}

const SOURCE_HEAD_COLUMNS: &str = "c.id,c.entry_id,c.kind,c.activity_id,c.ordinal,
    c.epoch,c.committed_head,c.lost_bytes,c.lost_packets";
const BRANCH_SOURCE_HEADS: &str = "b.head_json,b.entry_id=c.entry_id AS linked
    FROM agena_branch_sources b
    CROSS JOIN agena_content_streams c ON c.id=b.content_id
    WHERE b.session_id=1 ORDER BY b.content_id";
const ROUND_SOURCE_HEADS: &str = "s.head_json,
    (e.id=c.entry_id OR (e.kind='compaction' AND e.content_id IS c.id)) AS linked,
    json_extract(b.head_json,'$.epoch') AS frozen_epoch,
    json_extract(b.head_json,'$.committed') AS frozen_head
    FROM agena_round_input_sources s
    CROSS JOIN agena_content_streams c ON c.id=s.content_id
    CROSS JOIN agena_entries e ON e.id=s.entry_id
    LEFT JOIN agena_branch_sources b ON b.session_id=1 AND b.content_id=c.id
    ORDER BY s.round_id,s.content_id";

async fn validate_source_boundaries(connection: &mut SqliteConnection) -> Result<()> {
    // Foreign keys in staging were checked before these joins. Validate the
    // frozen branch headers first so they can supply the earlier epoch below.
    for (sources, check_epoch, diagnostic) in [
        (
            BRANCH_SOURCE_HEADS,
            false,
            "archive frozen branch source boundary is inconsistent",
        ),
        (
            ROUND_SOURCE_HEADS,
            true,
            "archive captured input source boundary is inconsistent",
        ),
    ] {
        let sql = format!("SELECT {SOURCE_HEAD_COLUMNS},{sources}");
        let mut rows = sqlx::query(&sql).fetch(&mut *connection);
        while let Some(row) = rows.try_next().await.map_err(sql_error)? {
            let head: ContentStreamView =
                decode(row.try_get::<&str, _>("head_json").map_err(storage)?)?;
            let activity = head.activity_id.map(|id| id.to_string());
            if !row.try_get::<bool, _>("linked").map_err(storage)?
                || head.id.to_string() != row.try_get::<&str, _>("id").map_err(storage)?
                || head.entry.get() != row.try_get::<i64, _>("entry_id").map_err(storage)?
                || commands::enum_text(&head.kind)?
                    != row.try_get::<&str, _>("kind").map_err(storage)?
                || activity.as_deref()
                    != row
                        .try_get::<Option<&str>, _>("activity_id")
                        .map_err(storage)?
                || i64::from(head.ordinal) != row.try_get::<i64, _>("ordinal").map_err(storage)?
                || head.committed > row.try_get::<i64, _>("committed_head").map_err(storage)? as u64
                || head.accepted != head.committed
                || head.retained_start > head.committed
                || head.capture_loss.bytes > i64::MAX as u64
                || head.capture_loss.packets > i64::MAX as u64
                || head.capture_loss.committed_bytes
                    > row.try_get::<i64, _>("lost_bytes").map_err(storage)? as u64
                || head.capture_loss.committed_packets
                    > row.try_get::<i64, _>("lost_packets").map_err(storage)? as u64
            {
                return Err(invalid(diagnostic));
            }
            if check_epoch
                && head.epoch.to_string() != row.try_get::<&str, _>("epoch").map_err(storage)?
            {
                let frozen_epoch = row
                    .try_get::<Option<&str>, _>("frozen_epoch")
                    .map_err(storage)?;
                let frozen_epoch = frozen_epoch
                    .map(str::parse::<ContentEpoch>)
                    .transpose()
                    .map_err(storage)?;
                let frozen_head = row
                    .try_get::<Option<i64>, _>("frozen_head")
                    .map_err(storage)?;
                if frozen_epoch != Some(head.epoch)
                    || frozen_head.is_none_or(|end| head.committed > end as u64)
                {
                    return Err(invalid(diagnostic));
                }
            }
        }
    }
    Ok(())
}

async fn validate(connection: &mut SqliteConnection) -> Result<()> {
    if !sqlx::query("PRAGMA foreign_key_check")
        .fetch_optional(&mut *connection)
        .await
        .map_err(sql_error)?
        .is_none()
    {
        return Err(invalid(
            "archive contains missing or inconsistent fact references",
        ));
    }
    for (sql, reason) in [
        (
            "SELECT EXISTS(SELECT 1 FROM import_source_owners o LEFT JOIN agena_content_streams s ON s.id=o.id AND s.entry_id=o.entry_id WHERE s.id IS NULL)",
            "archive source manifest is incomplete",
        ),
        (
            "SELECT EXISTS(SELECT 1 FROM agena_content_streams s LEFT JOIN import_source_owners o ON o.id=s.id AND o.entry_id=s.entry_id WHERE o.id IS NULL)",
            "archive source has no authorized owner manifest",
        ),
        (
            "SELECT EXISTS(SELECT 1 FROM agena_content_streams s JOIN import_source_bounds b ON b.id=s.id WHERE b.physical_start<0 OR b.physical_start>s.retained_start OR (s.retained_start=s.committed_head AND b.physical_start!=s.committed_head))",
            "archive physical content boundary is invalid",
        ),
        (
            "SELECT EXISTS(SELECT 1 FROM agena_content_streams s JOIN import_source_bounds b ON b.id=s.id WHERE (SELECT COALESCE(SUM(logical_end-logical_start),0) FROM agena_content_blocks c WHERE c.content_id=s.id)!=s.committed_head-b.physical_start OR (s.committed_head>b.physical_start AND ((SELECT MIN(logical_start) FROM agena_content_blocks c WHERE c.content_id=s.id)!=b.physical_start OR (SELECT MAX(logical_end) FROM agena_content_blocks c WHERE c.content_id=s.id)!=s.committed_head)))",
            "archive committed content blocks are incomplete",
        ),
        (
            "SELECT EXISTS(SELECT 1 FROM (SELECT content_id,logical_start,LAG(logical_end) OVER(PARTITION BY content_id ORDER BY logical_start) AS previous FROM agena_content_blocks) WHERE previous IS NOT NULL AND previous!=logical_start)",
            "archive content blocks overlap or have gaps",
        ),
        (
            "SELECT EXISTS(SELECT 1 FROM agena_content_checkpoints c LEFT JOIN import_events e ON e.content_id=c.content_id AND e.sequence=c.logical_sequence AND e.block_start=c.block_start AND e.checkpoint=1 WHERE e.sequence IS NULL)",
            "archive checkpoint index does not reference a checkpoint",
        ),
        (
            "SELECT EXISTS(SELECT 1 FROM import_events e LEFT JOIN agena_content_checkpoints c ON e.content_id=c.content_id AND e.sequence=c.logical_sequence AND e.block_start=c.block_start WHERE e.checkpoint=1 AND c.logical_sequence IS NULL)",
            "archive event checkpoint index is incomplete",
        ),
        (
            "SELECT EXISTS(SELECT 1 FROM agena_content_streams s WHERE s.lost_bytes!=(SELECT COALESCE(SUM(byte_count),0) FROM agena_content_capture_losses l WHERE l.content_id=s.id) OR s.lost_packets!=(SELECT COALESCE(MAX(last_packet),0) FROM agena_content_capture_losses l WHERE l.content_id=s.id) OR (s.capture_state='complete' AND s.lost_packets>0))",
            "archive capture loss totals are inconsistent",
        ),
        (
            "SELECT EXISTS(SELECT 1 FROM (SELECT l.*,LAG(last_packet,1,0) OVER(PARTITION BY content_id ORDER BY first_packet) AS previous FROM agena_content_capture_losses l) l JOIN agena_content_streams s ON s.id=l.content_id WHERE first_packet!=previous+1 OR after_position>s.committed_head)",
            "archive capture loss records are not contiguous",
        ),
        (
            "SELECT EXISTS(SELECT 1 FROM agena_content_streams s WHERE s.retained_start>0 AND NOT EXISTS(SELECT 1 FROM agena_content_gaps g WHERE g.content_id=s.id AND g.logical_start=0 AND g.logical_end=s.retained_start))",
            "archive retained prefix has no explicit gap",
        ),
        (
            "SELECT EXISTS(SELECT 1 FROM agena_content_gaps g JOIN agena_content_streams s ON s.id=g.content_id WHERE g.logical_start!=0 OR g.logical_end>s.retained_start)",
            "archive content gap is outside its retained prefix",
        ),
        (
            "SELECT EXISTS(SELECT 1 FROM agena_blob_references GROUP BY entry_id HAVING COUNT(*)>16)",
            "archive per-entry blob identity budget exceeded",
        ),
        (
            "SELECT EXISTS(SELECT 1 FROM agena_content_streams GROUP BY entry_id HAVING COUNT(*)>128)",
            "archive per-entry source identity budget exceeded",
        ),
        (
            "SELECT EXISTS(SELECT 1 FROM agena_interactions GROUP BY tool_id HAVING COUNT(*)>128)",
            "archive per-tool interaction identity budget exceeded",
        ),
        (
            "SELECT EXISTS(SELECT 1 FROM agena_interactions WHERE json_extract(decision_json,'$.kind')='permission' AND COALESCE(json_extract(decision_json,'$.value.reply.request_id')=CAST(id AS TEXT),0)=0)",
            "archive permission reply references another interaction",
        ),
        (
            "SELECT EXISTS(SELECT 1 FROM agena_background_activities GROUP BY tool_id HAVING COUNT(*)>16)",
            "archive per-tool activity identity budget exceeded",
        ),
        (
            "SELECT EXISTS(SELECT 1 FROM agena_blobs b WHERE b.expected_bytes!=(SELECT COALESCE(SUM(length(body)),0) FROM agena_blob_chunks c WHERE c.blob_id=b.id))",
            "archive blob chunks are incomplete",
        ),
        (
            "SELECT EXISTS(SELECT 1 FROM (SELECT blob_id,logical_start,LAG(logical_start+length(body),1,0) OVER(PARTITION BY blob_id ORDER BY logical_start) AS previous FROM agena_blob_chunks) WHERE logical_start!=previous)",
            "archive blob chunks are not contiguous",
        ),
        (
            "SELECT EXISTS(SELECT 1 FROM agena_entries e JOIN agena_executions x ON x.id=e.execution_id WHERE (e.kind='user' AND x.message_id!=e.id) OR (e.kind!='user' AND e.message_id!=x.message_id))",
            "archive entry belongs to an inconsistent execution message",
        ),
        (
            "SELECT EXISTS(SELECT 1 FROM agena_executions x JOIN agena_entries e ON e.id=x.message_id LEFT JOIN agena_execution_purposes p ON p.execution_id=x.id WHERE (p.execution_id IS NULL AND e.kind!='user') OR (p.execution_id IS NOT NULL AND (e.kind!='notice' OR e.message_id!=e.id OR e.origin_session_id!=x.session_id)))",
            "archive execution purpose has an inconsistent input anchor",
        ),
        (
            "SELECT EXISTS(
                SELECT 1 FROM agena_attempts a JOIN agena_entries e ON e.id=a.response_entry_id
                WHERE e.execution_id!=a.execution_id OR e.generation!=a.generation
                   OR e.kind NOT IN ('text','reasoning','tool')
             )",
            "archive response anchor belongs to another attempt or entry kind",
        ),
        (
            "SELECT EXISTS(SELECT 1 FROM agena_branch_sources b JOIN agena_content_streams c ON c.id=b.content_id LEFT JOIN agena_branch_entries e ON e.session_id=b.session_id AND e.entry_id=b.entry_id WHERE c.entry_id!=b.entry_id OR e.entry_id IS NULL)",
            "archive frozen source belongs to another entry",
        ),
        (
            "SELECT EXISTS(
                SELECT 1 FROM agena_branch_entries b JOIN agena_entries e ON e.id=b.entry_id
                JOIN agena_attempts a ON a.execution_id=e.execution_id AND a.generation=e.generation
                WHERE b.revision>e.revision
                   OR NOT EXISTS(SELECT 1 FROM agena_attempts frozen WHERE frozen.execution_id=e.execution_id
                                  AND frozen.generation=b.execution_generation)
                   OR (e.kind!='user' AND b.response_entry_id IS NOT NULL
                       AND b.response_entry_id IS NOT a.response_entry_id)
             )",
            "archive frozen entry references inconsistent revision or execution facts",
        ),
        (
            "SELECT EXISTS(SELECT 1 FROM agena_branch_entries b JOIN agena_entries e ON e.id=b.entry_id JOIN json_each(b.interactions_json) interaction WHERE NOT EXISTS(SELECT 1 FROM agena_interactions i WHERE i.id=interaction.value AND i.tool_id=e.tool_id)) OR EXISTS(SELECT 1 FROM agena_branch_entries b JOIN json_each(b.activities_json) activity WHERE NOT EXISTS(SELECT 1 FROM agena_background_activities a WHERE a.id=activity.value AND a.entry_id=b.entry_id)) OR EXISTS(SELECT 1 FROM agena_branch_entries b JOIN json_each(b.blobs_json) blob WHERE NOT EXISTS(SELECT 1 FROM agena_blob_references r WHERE r.entry_id=b.entry_id AND r.blob_id=blob.value))",
            "archive frozen entry references another entry's resources",
        ),
        (
            "SELECT EXISTS(
                 SELECT 1 FROM agena_branch_rounds b JOIN agena_rounds r ON r.id=b.round_id
                 WHERE (b.has_provider_state=1 AND r.provider_state_json IS NULL)
                    OR NOT EXISTS(
                        SELECT 1 FROM agena_entries source INDEXED BY idx_agena_entry_round
                        CROSS JOIN agena_branch_entries e ON e.entry_id=source.id AND e.session_id=b.session_id
                        WHERE source.round_id=b.round_id
                    )
             )",
            "archive frozen round has no matching branch entry or provider state",
        ),
        (
            "SELECT EXISTS(
                 SELECT 1 FROM agena_round_input_replays q
                 CROSS JOIN agena_rounds r ON r.id=q.source_round_id
                 WHERE q.has_provider_state=1 AND r.provider_state_bytes=0
             )",
            "archive replay has no matching provider state",
        ),
        (
            "SELECT EXISTS(SELECT 1 FROM agena_round_input_replays GROUP BY round_id HAVING count(*)>10000)",
            "archive replay identity budget exceeded",
        ),
        (
            "SELECT EXISTS(SELECT 1 FROM agena_branch_activities b JOIN agena_background_activities a ON a.id=b.activity_id WHERE (b.has_outcome=1 AND a.outcome_json IS NULL) OR NOT EXISTS(SELECT 1 FROM agena_branch_entries e WHERE e.session_id=b.session_id AND e.entry_id=a.entry_id))",
            "archive frozen activity has no matching branch entry or outcome",
        ),
    ] {
        reject_query(connection, sql, reason).await?;
    }
    validate_source_boundaries(connection).await?;
    // Canonical immutable membership contains only visible entries. Closure
    // support facts remain durable dependencies without becoming history rows.
    sqlx::query(
        "INSERT INTO agena_history_segments(id)
         SELECT ordinal/128+1 FROM import_entries WHERE member=1 AND ordinal%128=0 ORDER BY ordinal",
    )
    .execute(&mut *connection)
    .await
    .map_err(sql_error)?;
    sqlx::query("INSERT INTO agena_segment_entries SELECT ordinal/128+1,ordinal%128,id FROM import_entries WHERE member=1 ORDER BY ordinal").execute(&mut *connection).await.map_err(sql_error)?;
    sqlx::query(
        "INSERT INTO agena_session_segments (
             session_id,sequence,segment_id,first_ordinal,end_ordinal,since_generation,
             user_message_count,max_entry_created_at_ms
         )
         SELECT 1,m.segment_id-1,m.segment_id,0,MAX(m.ordinal)+1,1,
                COUNT(*) FILTER (WHERE e.kind='user'),MAX(e.created_at_ms)
         FROM agena_segment_entries m CROSS JOIN agena_entries e ON e.id=m.entry_id
         GROUP BY m.segment_id",
    )
    .execute(&mut *connection)
    .await
    .map_err(sql_error)?;
    let window = sqlx::query("SELECT * FROM agena_prompt_windows WHERE session_id=1")
        .fetch_one(&mut *connection)
        .await
        .map_err(sql_error)?;
    if let Some(through) = window
        .try_get::<Option<i64>, _>("through_entry_id")
        .map_err(storage)?
    {
        let ordinal: i64 =
            sqlx::query_scalar("SELECT ordinal FROM import_entries WHERE id=? AND member=1")
                .bind(through)
                .fetch_optional(&mut *connection)
                .await
                .map_err(sql_error)?
                .ok_or_else(|| invalid("archive prompt boundary is not a visible entry"))?;
        sqlx::query("UPDATE agena_prompt_windows SET through_sequence=?,through_ordinal=? WHERE session_id=1").bind(ordinal/128).bind(ordinal%128).execute(&mut *connection).await.map_err(sql_error)?;
    }
    let finished = now_ms();
    sqlx::query("UPDATE agena_attempts SET phase='interrupted',owner_epoch=NULL,finished_at_ms=MAX(?,COALESCE(started_at_ms,?)) WHERE phase IN ('queued','running')").bind(finished).bind(finished).execute(&mut *connection).await.map_err(sql_error)?;
    sqlx::query("UPDATE agena_rounds SET phase='interrupted',finished_at_ms=MAX(?,created_at_ms) WHERE phase='running'").bind(finished).execute(&mut *connection).await.map_err(sql_error)?;
    sqlx::query("UPDATE agena_entries SET revision=revision+1 WHERE tool_id IN (SELECT id FROM agena_tool_calls WHERE phase IN ('preparing','running')) OR interaction_id IN (SELECT id FROM agena_interactions WHERE decision_json IS NULL)").execute(&mut *connection).await.map_err(sql_error)?;
    sqlx::query("UPDATE agena_tool_calls SET phase='interrupted',result_json='null',finished_at_ms=MAX(?,created_at_ms) WHERE phase IN ('preparing','running')").bind(finished).execute(&mut *connection).await.map_err(sql_error)?;
    sqlx::query("UPDATE agena_interactions SET decision_json='{\"kind\":\"cancel\"}',remember_json=NULL,decided_at_ms=MAX(?,created_at_ms) WHERE decision_json IS NULL").bind(finished).execute(&mut *connection).await.map_err(sql_error)?;
    sqlx::query("UPDATE agena_background_activities SET phase='interrupted',outcome_json='{\"outcome\":\"interrupted\",\"diagnostic\":\"archive imported; external activity was not resumed\"}',finished_at_ms=MAX(?,created_at_ms) WHERE phase='running'").bind(finished).execute(&mut *connection).await.map_err(sql_error)?;
    sqlx::query("UPDATE agena_content_streams SET capture_state='interrupted',capture_error=COALESCE(capture_error,'archive imported; capture was not resumed') WHERE capture_state='open'").execute(&mut *connection).await.map_err(sql_error)?;
    Ok(())
}

async fn make_maps(connection: &mut SqliteConnection) -> Result<()> {
    for (kind, table) in [
        ("execution", "agena_executions"),
        ("activity", "agena_background_activities"),
        ("content", "agena_content_streams"),
        ("blob", "agena_blobs"),
    ] {
        let mut after = String::new();
        loop {
            let rows: Vec<String> = sqlx::query_scalar(&format!(
                "SELECT id FROM {table} WHERE id>? ORDER BY id LIMIT 128"
            ))
            .bind(&after)
            .fetch_all(&mut *connection)
            .await
            .map_err(sql_error)?;
            let Some(last) = rows.last() else { break };
            let mut insert =
                sqlx::QueryBuilder::new("INSERT INTO import_maps (kind,old,new,epoch) ");
            insert.push_values(&rows, |mut row, old| {
                let new = match kind {
                    "execution" => ExecutionId::new().to_string(),
                    "activity" => agena_domain::ActivityId::new().to_string(),
                    "content" => ContentId::new().to_string(),
                    _ => agena_domain::BlobId::new().to_string(),
                };
                let epoch = (kind == "content").then(|| ContentEpoch::new().to_string());
                row.push_bind(kind)
                    .push_bind(old)
                    .push_bind(new)
                    .push_bind(epoch);
            });
            insert
                .build()
                .execute(&mut *connection)
                .await
                .map_err(sql_error)?;
            after.clone_from(last);
        }
    }
    Ok(())
}

fn reference_text(value: &Value) -> Result<Option<std::borrow::Cow<'_, str>>> {
    if value.is_null() {
        return Ok(None);
    }
    value
        .as_str()
        .map(std::borrow::Cow::Borrowed)
        .or_else(|| {
            value
                .as_i64()
                .map(|n| std::borrow::Cow::Owned(n.to_string()))
        })
        .map(Some)
        .ok_or_else(|| invalid("archive identity is not a scalar"))
}

fn visit_json_references(
    value: &mut Value,
    visit: &mut impl FnMut(&'static str, &mut Value) -> Result<()>,
) -> Result<()> {
    match value {
        Value::Array(values) => {
            for value in values {
                visit_json_references(value, visit)?;
            }
        }
        Value::Object(values) => {
            for (key, value) in values {
                match key.as_str() {
                    "resource_id" | "content_id" => visit("content", value)?,
                    "blob_id" => visit("blob", value)?,
                    "uri" => {
                        if let Some(encoded) = value
                            .as_str()
                            .and_then(|s| s.strip_prefix("agena:media:v1:"))
                        {
                            if encoded.len() > 16 * 1024 {
                                return Err(invalid(
                                    "archive media binding exceeds metadata budget",
                                ));
                            }
                            let mut binding: Value =
                                serde_json::from_slice(&hex::decode(encoded).map_err(invalid)?)
                                    .map_err(invalid)?;
                            if binding["version"] != 1 {
                                return Err(invalid("unsupported archive media binding"));
                            }
                            visit("blob", &mut binding["blob"])?;
                            *value = json!(format!(
                                "agena:media:v1:{}",
                                hex::encode(serde_json::to_vec(&binding).map_err(invalid)?)
                            ));
                        }
                    }
                    _ => visit_json_references(value, visit)?,
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn visit_field_references(
    table: &str,
    value: &mut Value,
    mut visit: impl FnMut(&'static str, &mut Value) -> Result<()>,
) -> Result<()> {
    visit_json_references(value, &mut visit)?;
    if table == "agena_entries" && value["kind"] == "compaction" {
        visit("content", &mut value["summary"])?;
    }
    Ok(())
}

async fn stage_json_remap(
    connection: &mut SqliteConnection,
    table: &str,
    column: &str,
    row_id: &str,
    value: &Value,
) -> Result<()> {
    let (field, old, reply) = match (table, column) {
        ("agena_entries", "body_json") if value["kind"] == "compaction" => {
            ("through", integer(value, "through")?.to_string(), None)
        }
        ("agena_interactions", "request_json") if value.get("Input").is_some() => {
            if !value["Input"]["request"].is_object() {
                return Err(invalid("archive input request has no request object"));
            }
            ("input_request", row_id.to_owned(), None)
        }
        ("agena_interactions", "decision_json") if value["kind"] == "permission" => {
            ("permission_reply", row_id.to_owned(), None)
        }
        ("agena_interactions", "decision_json") if value["kind"] == "input" => {
            // Input is opaque at the port. Rebind only the runtime's typed reply.
            let Some(reply) = value["value"].as_str().and_then(|encoded| {
                serde_json::from_str::<agena_domain::UserInputReply>(encoded).ok()
            }) else {
                return Ok(());
            };
            ("input_reply", row_id.to_owned(), Some(encode(&reply)?))
        }
        _ => return Ok(()),
    };
    sqlx::query("INSERT INTO import_json_remaps(field,row_id,old,reply_json) VALUES (?,?,?,?)")
        .bind(field)
        .bind(row_id)
        .bind(old)
        .bind(reply)
        .execute(connection)
        .await
        .map_err(sql_error)?;
    Ok(())
}

const JSON_PAGE_ROWS: i64 = 64;

async fn stage_json(connection: &mut SqliteConnection) -> Result<()> {
    for (table, columns) in [
        ("agena_entries", &["body_json"][..]),
        ("agena_tool_calls", &["input_json", "result_json"][..]),
        (
            "agena_interactions",
            &["request_json", "decision_json", "remember_json"][..],
        ),
        ("agena_background_activities", &["outcome_json"][..]),
    ] {
        let text_key = table == "agena_background_activities";
        let mut after = if text_key { String::new() } else { "0".into() };
        let query = format!(
            "SELECT id,{} FROM {table} WHERE id>? ORDER BY id LIMIT {JSON_PAGE_ROWS}",
            columns.join(",")
        );
        loop {
            let mut page = Vec::new();
            let mut page_bytes = 0;
            {
                let mut rows = sqlx::query(&query).bind(&after).fetch(&mut *connection);
                // Allow a single oversized row to advance the cursor.
                while page_bytes < 1024 * 1024 {
                    let Some(row) = rows.try_next().await.map_err(sql_error)? else {
                        break;
                    };
                    for column in columns {
                        page_bytes += row
                            .try_get::<Option<&str>, _>(*column)
                            .map_err(storage)?
                            .map_or(0, str::len);
                    }
                    page.push(row);
                }
            }
            if page.is_empty() {
                break;
            }
            let mut fields = Vec::new();
            let mut refs = HashSet::new();
            for row in &page {
                after = if text_key {
                    row.try_get::<String, _>("id").map_err(storage)?
                } else {
                    row.try_get::<i64, _>("id").map_err(storage)?.to_string()
                };
                for &column in columns {
                    let Some(original) = row.try_get::<Option<&str>, _>(column).map_err(storage)?
                    else {
                        continue;
                    };
                    let mut value: Value = decode(original)?;
                    visit_field_references(table, &mut value, |kind, value| {
                        if let Some(old) = reference_text(value)? {
                            refs.insert((kind, old.into_owned()));
                        }
                        Ok(())
                    })?;
                    stage_json_remap(connection, table, column, &after, &value).await?;
                    fields.push((after.clone(), column, original, value));
                }
            }
            let mut mappings = HashMap::<String, HashMap<String, String>>::new();
            if !refs.is_empty() {
                let rows: Vec<(String, String, String)> = sqlx::query_as(
                    "SELECT m.kind,m.old,m.new FROM json_each(?) refs
                     CROSS JOIN import_maps m ON m.kind=refs.value->>0 AND m.old=refs.value->>1",
                )
                .bind(encode(&refs)?)
                .fetch_all(&mut *connection)
                .await
                .map_err(sql_error)?;
                for (kind, old, new) in rows {
                    mappings.entry(kind).or_default().insert(old, new);
                }
            }
            for (id, column, original, mut value) in fields {
                visit_field_references(table, &mut value, |kind, value| {
                    if let Some(old) = reference_text(value)? {
                        let new = mappings
                            .get(kind)
                            .and_then(|m| m.get(old.as_ref()))
                            .ok_or_else(|| {
                                invalid(format!("archive is missing a referenced {kind} identity"))
                            })?;
                        *value = json!(new);
                    }
                    Ok(())
                })?;
                let rewritten = encode(&value)?;
                if rewritten != original {
                    sqlx::query(&format!("UPDATE {table} SET {column}=? WHERE id=?"))
                        .bind(rewritten)
                        .bind(id)
                        .execute(&mut *connection)
                        .await
                        .map_err(sql_error)?;
                }
            }
        }
    }
    reject_query(
        connection,
        "SELECT EXISTS(SELECT 1 FROM import_json_remaps p LEFT JOIN agena_entries e
         ON e.id=p.old WHERE p.field='through' AND e.id IS NULL)",
        "archive is missing a referenced entry identity",
    )
    .await
}

async fn publish_json(connection: &mut SqliteConnection, session: i64) -> Result<()> {
    // Only identities allocated in this writer transaction still need a JSON
    // update. The staging plan identifies their exact fields and row positions.
    for (field, table, column, kind, value) in [
        (
            "through",
            "agena_entries",
            "body_json",
            "entry",
            "json_set(t.body_json,'$.through',CAST(m.new AS INTEGER))",
        ),
        (
            "input_request",
            "agena_interactions",
            "request_json",
            "interaction",
            "json_set(t.request_json,'$.Input.request.request_id',m.new,'$.Input.request.session_id',?4)",
        ),
        (
            "permission_reply",
            "agena_interactions",
            "decision_json",
            "interaction",
            "json_set(t.decision_json,'$.value.reply.request_id',m.new)",
        ),
        (
            "input_reply",
            "agena_interactions",
            "decision_json",
            "interaction",
            "json_set(t.decision_json,'$.value',printf('%s',json_set(p.reply_json,'$.request_id',m.new)))",
        ),
    ] {
        let query = format!(
            "UPDATE archive_import.{table} AS t SET {column}=(
                SELECT {value} FROM archive_import.import_json_remaps p
                CROSS JOIN archive_import.import_maps m ON m.kind='{kind}' AND m.old=p.old
                WHERE p.field=?1 AND p.row_id=t.id)
             WHERE t.id IN (SELECT row_id FROM archive_import.import_json_remaps
                WHERE field=?1 AND row_id>?2 AND row_id<=?3)"
        );
        let mut after = 0i64;
        loop {
            let last: Option<i64> = sqlx::query_scalar(
                "SELECT MAX(row_id) FROM (SELECT row_id FROM archive_import.import_json_remaps
                 WHERE field=? AND row_id>? ORDER BY row_id LIMIT ?)",
            )
            .bind(field)
            .bind(after)
            .bind(JSON_PAGE_ROWS)
            .fetch_one(&mut *connection)
            .await
            .map_err(sql_error)?;
            let Some(last) = last else { break };
            let update = sqlx::query(&query).bind(field).bind(after).bind(last);
            let update = if field == "input_request" {
                update.bind(session)
            } else {
                update
            };
            update.execute(&mut *connection).await.map_err(sql_error)?;
            after = last;
        }
    }
    // Staging validation has already checked each array element against the
    // source table. Each identity in those tables now has a publication map.
    for (column, kind, mapped_id) in [
        ("interactions_json", "interaction", "CAST(m.new AS INTEGER)"),
        ("activities_json", "activity", "m.new"),
        ("blobs_json", "blob", "m.new"),
    ] {
        sqlx::query(&format!(
            "UPDATE archive_import.agena_branch_entries AS b
             SET {column}=(SELECT json_group_array({mapped_id} ORDER BY ref.key)
                 FROM json_each(b.{column}) ref
                 CROSS JOIN archive_import.import_maps m
                     ON m.kind=? AND m.old=CAST(ref.value AS TEXT))
             WHERE b.session_id=1 AND b.{column} IS NOT NULL"
        ))
        .bind(kind)
        .execute(&mut *connection)
        .await
        .map_err(sql_error)?;
    }
    Ok(())
}

async fn publish_round_inputs(connection: &mut SqliteConnection) -> Result<()> {
    const PAGE_SIZE: usize = 1_024;
    let mut after = (i64::MIN, -1i64);
    let mut pending = Vec::new();
    loop {
        let rows = sqlx::query(
            "SELECT s.round_id AS archive_round,s.ordinal,CAST(r.new AS INTEGER) AS round_id,
                    CAST(e.new AS INTEGER) AS entry_id,s.entry_revision,CAST(a.new AS INTEGER) AS response_entry_id,
                    s.execution_phase,s.tool_phase,s.interaction_decided
             FROM archive_import.agena_round_inputs s
             CROSS JOIN archive_import.import_maps r ON r.kind='round' AND r.old=CAST(s.round_id AS TEXT)
             CROSS JOIN archive_import.import_maps e ON e.kind='entry' AND e.old=CAST(s.entry_id AS TEXT)
             LEFT JOIN archive_import.import_maps a ON a.kind='entry' AND a.old=CAST(s.response_entry_id AS TEXT)
             WHERE (s.round_id,s.ordinal)>(?,?) ORDER BY s.round_id,s.ordinal LIMIT ?",
        )
        .bind(after.0)
        .bind(after.1)
        .bind(PAGE_SIZE as i64)
        .fetch_all(&mut *connection)
        .await
        .map_err(sql_error)?;
        if let Some(last) = rows.last() {
            after = (
                last.try_get("archive_round").map_err(storage)?,
                last.try_get("ordinal").map_err(storage)?,
            );
        }
        let more = rows.len() == PAGE_SIZE;
        for row in rows {
            pending.push(prompt_inputs::FrozenInput::from_row(row).map_err(sql_error)?);
        }
        // Carry the final block across the storage page boundary so an import
        // uses the same grouping as the live capture of these identities.
        let keep = if more {
            prompt_inputs::blocks(&pending)
                .next_back()
                .map_or(0, |block| block.len())
        } else {
            0
        };
        let ready = pending.len() - keep;
        prompt_inputs::persist(connection, &pending[..ready]).await?;
        if !more {
            return Ok(());
        }
        pending.drain(..ready);
    }
}

async fn publish_round_replays(connection: &mut SqliteConnection) -> Result<()> {
    const PAGE_SIZE: usize = 1_024;
    let mut after = (i64::MIN, i64::MIN);
    let mut pending = Vec::new();
    loop {
        let rows = sqlx::query(
            "SELECT s.round_id AS archive_round,s.source_round_id AS archive_source_round,
                    CAST(q.new AS INTEGER) AS round_id,r.id AS source_round_id,
                    s.phase,s.has_provider_state,
                    CASE WHEN s.has_provider_state=1 THEN r.provider_state_bytes ELSE 0 END AS state_bytes
             FROM archive_import.agena_round_input_replays s
             CROSS JOIN archive_import.import_maps q ON q.kind='round' AND q.old=CAST(s.round_id AS TEXT)
             CROSS JOIN archive_import.import_maps m ON m.kind='round' AND m.old=CAST(s.source_round_id AS TEXT)
             CROSS JOIN main.agena_rounds r ON r.id=CAST(m.new AS INTEGER)
             WHERE (s.round_id,s.source_round_id)>(?,?)
             ORDER BY s.round_id,s.source_round_id LIMIT ?",
        )
        .bind(after.0)
        .bind(after.1)
        .bind(PAGE_SIZE as i64)
        .fetch_all(&mut *connection)
        .await
        .map_err(sql_error)?;
        if let Some(last) = rows.last() {
            after = (
                last.try_get("archive_round").map_err(storage)?,
                last.try_get("archive_source_round").map_err(storage)?,
            );
        }
        let more = rows.len() == PAGE_SIZE;
        for row in rows {
            pending.push(prompt_replays::FrozenReplay::from_row(row).map_err(sql_error)?);
        }
        // Round maps preserve numeric order. Keep the final bucket together
        // across staged pages so it can still share its remapped replay block.
        let keep = if more {
            prompt_replays::blocks(&pending)
                .next_back()
                .map_or(0, |block| block.len())
        } else {
            0
        };
        let ready = pending.len() - keep;
        prompt_replays::persist(connection, &pending[..ready]).await?;
        if !more {
            return Ok(());
        }
        pending.drain(..ready);
    }
}

// The staged header already matches its stream. Use the published stream for
// remapped identities and keep the archived positions, state, and diagnostics.
const REMAPPED_HEAD_VALUES: &str = "c.epoch,
    json_extract(s.head_json,'$.committed'),json_extract(s.head_json,'$.retained_start'),
    json_extract(s.head_json,'$.state'),
    COALESCE(json_extract(s.head_json,'$.capture_loss.bytes'),0),
    COALESCE(json_extract(s.head_json,'$.capture_loss.packets'),0),
    COALESCE(json_extract(s.head_json,'$.capture_loss.committed_bytes'),0),
    COALESCE(json_extract(s.head_json,'$.capture_loss.committed_packets'),0),
    json_extract(s.head_json,'$.capture_error'),
    length(CAST(json_set(s.head_json,'$.id',c.id,'$.entry',c.entry_id,'$.epoch',c.epoch,
                         '$.activity_id',c.activity_id) AS BLOB))";

async fn publish_branch_sources(connection: &mut SqliteConnection, session: i64) -> Result<()> {
    let columns = content::heads::COLUMNS;
    sqlx::query(&format!(
        "INSERT INTO main.agena_branch_sources(session_id,source_key,{columns})
         SELECT ?,c.source_key,{REMAPPED_HEAD_VALUES}
         FROM archive_import.agena_branch_sources s
         CROSS JOIN archive_import.import_maps m ON m.kind='content' AND m.old=s.content_id
         CROSS JOIN main.agena_content_streams c ON c.id=m.new
         WHERE s.session_id=1"
    ))
    .bind(session)
    .execute(connection)
    .await
    .map_err(sql_error)?;
    Ok(())
}

async fn publish_round_sources(connection: &mut SqliteConnection) -> Result<()> {
    let columns = content::heads::COLUMNS;
    let inserted = sqlx::query(&format!(
        "INSERT INTO main.agena_round_input_sources(round_id,block_id,entry_id,source_key,{columns})
         SELECT CAST(r.new AS INTEGER),(
                SELECT block_id FROM main.agena_round_input_blocks
                WHERE round_id=CAST(r.new AS INTEGER) AND start_ordinal<=i.ordinal
                ORDER BY start_ordinal DESC LIMIT 1
             ),CAST(e.new AS INTEGER),c.source_key,{REMAPPED_HEAD_VALUES}
         FROM archive_import.agena_round_input_sources s
         CROSS JOIN archive_import.agena_round_inputs i ON i.round_id=s.round_id AND i.entry_id=s.entry_id
         CROSS JOIN archive_import.import_maps r ON r.kind='round' AND r.old=CAST(s.round_id AS TEXT)
         CROSS JOIN archive_import.import_maps e ON e.kind='entry' AND e.old=CAST(s.entry_id AS TEXT)
         CROSS JOIN archive_import.import_maps m ON m.kind='content' AND m.old=s.content_id
         CROSS JOIN main.agena_content_streams c ON c.id=m.new"
    ))
    .execute(&mut *connection)
    .await
    .map_err(sql_error)?;
    if inserted.rows_affected() != 0 {
        sqlx::query(&format!(
            "UPDATE main.agena_rounds AS r {}
             WHERE r.id IN (SELECT CAST(new AS INTEGER) FROM archive_import.import_maps WHERE kind='round')
                AND EXISTS(SELECT 1 FROM agena_round_input_sources WHERE round_id=r.id)",
            prompt_sources::SOURCE_BUDGET_ASSIGNMENT,
        ))
        .execute(connection)
        .await
        .map_err(sql_error)?;
    }
    Ok(())
}

fn id_kind(table: &str) -> Option<&'static str> {
    match table {
        "agena_executions" => Some("execution"),
        "agena_background_activities" => Some("activity"),
        "agena_content_streams" => Some("content"),
        "agena_blobs" => Some("blob"),
        "agena_entries" => Some("entry"),
        "agena_rounds" => Some("round"),
        "agena_tool_calls" => Some("tool"),
        "agena_interactions" => Some("interaction"),
        "agena_history_segments" => Some("segment"),
        _ => None,
    }
}
fn expression(table: &str, column: &str, session: i64, owner_epoch: &str) -> String {
    if matches!(column, "session_id" | "origin_session_id") {
        return session.to_string();
    }
    if column == "epoch" && table == "agena_content_streams" {
        return "(SELECT epoch FROM archive_import.import_maps WHERE kind='content' AND old=s.id)"
            .into();
    }
    if column == "owner_epoch" {
        return if table == "agena_background_activities" {
            format!("'{owner_epoch}'")
        } else {
            "NULL".into()
        };
    }
    let kind = match column {
        "id" => id_kind(table),
        "execution_id" => Some("execution"),
        "entry_id"
        | "message_id"
        | "response_entry_id"
        | "checkpoint_entry_id"
        | "through_entry_id"
        | "base_checkpoint_entry_id" => Some("entry"),
        "round_id" => Some("round"),
        "tool_id" => Some("tool"),
        "interaction_id" => Some("interaction"),
        "content_id" => Some("content"),
        "activity_id" => Some("activity"),
        "blob_id" => Some("blob"),
        "segment_id" => Some("segment"),
        _ => None,
    };
    if let Some(kind) = kind {
        let mapped = format!(
            "(SELECT new FROM archive_import.import_maps WHERE kind='{kind}' AND old=CAST(s.{column} AS TEXT))"
        );
        if content_key_column(table, column) {
            format!("(SELECT source_key FROM main.agena_content_streams WHERE id={mapped})")
        } else if matches!(kind, "entry" | "round" | "tool" | "interaction" | "segment") {
            format!("CAST({mapped} AS INTEGER)")
        } else {
            mapped
        }
    } else {
        format!("s.{column}")
    }
}

fn content_key_column(table: &str, column: &str) -> bool {
    column == "content_id"
        && matches!(
            table,
            "agena_content_blocks"
                | "agena_content_checkpoints"
                | "agena_content_gaps"
                | "agena_content_capture_losses"
        )
}

async fn publish_rows(
    connection: &mut SqliteConnection,
    job: &Staged,
) -> Result<CommitReceipt<SessionMetadata>> {
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agena_workspaces WHERE id=?)")
            .bind(job.workspace)
            .fetch_one(&mut *connection)
            .await
            .map_err(sql_error)?;
    if !exists {
        return Err(KernelError::NotFound("workspace".into()));
    }
    for (kind, table) in [
        ("entry", "agena_entries"),
        ("round", "agena_rounds"),
        ("tool", "agena_tool_calls"),
        ("interaction", "agena_interactions"),
        ("segment", "agena_history_segments"),
    ] {
        // An archived segment can have been collected since SQLite allocated
        // its ID. Explicit inserts must honor the same high-water mark.
        let maximum: i64 = sqlx::query_scalar(&format!(
            "SELECT MAX(COALESCE((SELECT seq FROM main.sqlite_sequence WHERE name=?),0),
                        COALESCE((SELECT MAX(id) FROM main.{table}),0))"
        ))
        .bind(table)
        .fetch_one(&mut *connection)
        .await
        .map_err(sql_error)?;
        let count: i64 =
            sqlx::query_scalar(&format!("SELECT COUNT(*) FROM archive_import.{table}"))
                .fetch_one(&mut *connection)
                .await
                .map_err(|error| storage(format!("archive import count {table}: {error}")))?;
        maximum
            .checked_add(count)
            .ok_or_else(|| invalid("import numeric identity overflow"))?;
        sqlx::query(&format!("INSERT INTO archive_import.import_maps(kind,old,new) SELECT ?,CAST(id AS TEXT),CAST(?+ROW_NUMBER() OVER(ORDER BY id) AS TEXT) FROM archive_import.{table}")).bind(kind).bind(maximum).execute(&mut *connection).await.map_err(|error| storage(format!("archive import map {table}: {error}")))?;
    }
    let id:i64=sqlx::query_scalar("INSERT INTO main.agena_sessions(workspace_id,title,favorite,pinned,selection_json,configuration_json,created_at_ms,updated_at_ms) SELECT ?,title,favorite,pinned,selection_json,configuration_json,created_at_ms,updated_at_ms FROM archive_import.agena_sessions WHERE id=1 RETURNING id").bind(job.workspace).fetch_one(&mut *connection).await.map_err(sql_error)?;
    sqlx::query("UPDATE agena_sessions SET root_session_id=id WHERE id=?")
        .bind(id)
        .execute(&mut *connection)
        .await
        .map_err(sql_error)?;
    publish_json(connection, id).await?;
    let epoch = KernelEpoch::new().to_string();
    for table in TABLES.iter().copied().chain(["agena_prompt_windows"]) {
        if table == "agena_round_inputs" {
            publish_round_inputs(connection).await?;
            continue;
        }
        if table == "agena_round_input_sources" {
            publish_round_sources(connection).await?;
            continue;
        }
        if table == "agena_round_input_replays" {
            publish_round_replays(connection).await?;
            continue;
        }
        if table == "agena_branch_sources" {
            publish_branch_sources(connection, id).await?;
            continue;
        }
        let columns = table_columns(&job.columns, table)?;
        let names = columns
            .iter()
            .map(|name| {
                if content_key_column(table, name) {
                    "source_key"
                } else {
                    name
                }
            })
            .collect::<Vec<_>>();
        let values = columns
            .iter()
            .map(|name| expression(table, name, id, &epoch))
            .collect::<Vec<_>>();
        sqlx::query(&format!(
            "INSERT INTO main.{table} ({}) SELECT {} FROM archive_import.{table} s",
            names.join(","),
            values.join(",")
        ))
        .execute(&mut *connection)
        .await
        .map_err(|error| storage(format!("archive import publish {table}: {error}")))?;
        if table == "agena_content_blocks" {
            sqlx::query(
                "INSERT INTO main.agena_content_block_pieces (source_key,block_start,byte_offset,body)
                 SELECT (SELECT c.source_key FROM archive_import.import_maps m
                         JOIN main.agena_content_streams c ON c.id=m.new
                         WHERE m.kind='content' AND m.old=p.content_id),
                        p.block_start,p.byte_offset,p.body
                 FROM archive_import.import_pieces p",
            )
            .execute(&mut *connection)
            .await
            .map_err(|error| storage(format!("archive import publish content pieces: {error}")))?;
        }
    }
    sqlx::query(
        "INSERT INTO agena_session_statistics (session_id,entry_count,user_message_count,last_message_at_ms)
         SELECT ?1,COALESCE(SUM(end_ordinal-first_ordinal),0),COALESCE(SUM(user_message_count),0),?2
         FROM agena_session_segments WHERE session_id=?1",
    )
    .bind(id)
    .bind(job.last_user_message_at_ms)
    .execute(&mut *connection)
    .await
    .map_err(|error| storage(format!("archive import statistics: {error}")))?;
    let session = SessionId::try_from(id).map_err(storage)?;
    history_runs::rebuild_imported(connection, session).await?;
    super::lifecycle::refresh_paused_summary(connection, session).await?;
    let sequence:i64=sqlx::query_scalar("UPDATE agena_commits SET sequence=sequence+1 WHERE id=1 AND sequence<9223372036854775807 RETURNING sequence").fetch_one(&mut *connection).await.map_err(sql_error)?;
    let receipt = CommitReceipt {
        position: CommitPosition {
            sequence: CommitSequence::try_from(sequence).map_err(storage)?,
            ordinal: 0,
        },
        value: reads::metadata(connection, session).await?,
    };
    let durable = CommitReceipt {
        position: receipt.position,
        value: Mutation::Session(Box::new(receipt.value.clone())),
    };
    receipts::write(connection, job.request, &job.digest, &durable).await?;
    Ok(receipt)
}

/// Only the sole writer calls this after staging verification. Request replay
/// is checked in the same transaction that allocates and publishes identities.
pub(super) async fn publish(
    database: &KernelDatabase,
    job: &Staged,
    feed: &ChangeFeed,
    cache: &Arc<read_cache::ReadCache>,
) -> Result<CommitReceipt<SessionMetadata>> {
    let mut connection = database
        .writer
        .get_sqlite_connection_pool()
        .acquire()
        .await
        .map_err(storage)?;
    // Attached schemas have a shorter lifetime than the sole writer. Do not
    // retain prepared VMs across attachment boundaries or reuse a prior
    // archive's schema slots for the next independently staged database.
    connection
        .clear_cached_statements()
        .await
        .map_err(storage)?;
    let mut archive_uri = url::Url::from_file_path(job.root.path().join("archive.sqlite"))
        .map_err(|_| invalid("temporary archive path cannot be a file URI"))?;
    // ATTACH inherits sqlite3_open_v2 flags from main. Memory-mode kernels
    // therefore need an explicit disk mode or SQLite silently attaches a new
    // empty memory database and ignores the staged file's name. rw also avoids
    // creating an empty database if the private staging file is missing.
    archive_uri.set_query(Some("mode=rw"));
    sqlx::query("ATTACH DATABASE ? AS archive_import")
        .persistent(false)
        .bind(archive_uri.as_str())
        .execute(&mut *connection)
        .await
        .map_err(sql_error)?;
    let result = async {
        let mut tx = connection
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(sql_error)?;
        // The schema defers the cyclic execution, attempt, entry and content
        // references. Other constraints stay immediate during publication.
        if let Some(receipt) = receipts::read(&mut tx, job.request, &job.digest).await? {
            let Mutation::Session(metadata) = receipt.value else {
                return Err(invalid("import replay has an incompatible receipt"));
            };
            tx.rollback().await.map_err(sql_error)?;
            return Ok(CommitReceipt {
                position: receipt.position,
                value: *metadata,
            });
        }
        let receipt = match publish_rows(&mut tx, job).await {
            Ok(receipt) => receipt,
            Err(error) => {
                abort_transaction(tx).await?;
                return Err(error);
            }
        };
        // Capture the listing facts before COMMIT, alongside the publication
        // receipt. A later metadata write must not change this event's scope.
        let imported_state = match super::list_state(&mut tx, receipt.value.id).await {
            Ok(Some(state)) => state,
            Ok(None) => {
                abort_transaction(tx).await?;
                return Err(storage("new imported session has no listing state"));
            }
            Err(error) => {
                abort_transaction(tx).await?;
                return Err(error);
            }
        };
        if imported_state.parent_session_id.is_some() {
            abort_transaction(tx).await?;
            return Err(storage("imported session must be a root"));
        }
        let include_in_lists = imported_state.include_in_lists;
        let list_change = Some(SessionListChange {
            before: None,
            after: Some(imported_state),
            parents: Vec::new(),
        });
        let fence = cache.pause();
        match tx.commit().await {
            Ok(()) => {
                fence.committed(Some(&[receipt.value.id]), &[job.workspace]);
                feed.publish_scoped(
                    receipt.position,
                    receipt.value.id,
                    Some(job.workspace),
                    include_in_lists,
                    ChangeKind::Membership,
                    Vec::new(),
                    true,
                    list_change,
                );
                Ok(receipt)
            }
            Err(sqlx::Error::Database(error)) if error.is_foreign_key_violation() => {
                Err(invalid("archive publication has unresolved references"))
            }
            Err(error) if rejected_commit(&error) => Err(storage(error)),
            Err(error) => {
                cache.close();
                Err(KernelError::CommitUnknown {
                    request_id: job.request,
                    diagnostic: error.to_string(),
                })
            }
        }
    }
    .await;
    // A durable successful receipt must remain successful if cleanup fails.
    if let Err(error) = sqlx::query("DETACH DATABASE archive_import")
        .persistent(false)
        .execute(&mut *connection)
        .await
    {
        tracing::warn!(%error,"archive staging detach failed");
        connection.close_on_drop();
    }
    if let Err(error) = connection.clear_cached_statements().await {
        tracing::warn!(%error,"archive statement cleanup failed");
        connection.close_on_drop();
    }
    result
}
