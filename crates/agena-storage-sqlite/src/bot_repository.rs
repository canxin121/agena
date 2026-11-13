//! Durable instance Bot identities. Persona is presentation, never authority.
//!
//! A Bot owns no single conversation. Its conversations are ordinary
//! `agena_sessions` rows whose configuration carries `owner.bot`; this table
//! holds only the identity, revision and archived state that every such
//! conversation is checked against.
use agena_storage::{BotRecord, BotRepository, BotRepositoryError};
use async_trait::async_trait;
use sea_orm::{ConnectionTrait, DatabaseBackend, DatabaseConnection, Statement, Value};
use std::sync::Arc;
use uuid::Uuid;

pub struct SqliteBotRepository {
    database: Arc<DatabaseConnection>,
}

impl SqliteBotRepository {
    pub fn new(database: Arc<DatabaseConnection>) -> Self {
        Self { database }
    }
}

fn backend(error: impl std::fmt::Display) -> BotRepositoryError {
    BotRepositoryError::Backend(error.to_string())
}

fn statement(sql: &str, values: Vec<Value>) -> Statement {
    Statement::from_sql_and_values(DatabaseBackend::Sqlite, sql, values)
}

const SELECT: &str = "SELECT id,name,name_folded,instructions,revision,archived,created_at_ms,updated_at_ms FROM agena_bots";

fn record(row: sea_orm::QueryResult) -> Result<BotRecord, BotRepositoryError> {
    Ok(BotRecord {
        id: row
            .try_get::<String>("", "id")
            .map_err(backend)?
            .parse()
            .map_err(|_| BotRepositoryError::Backend("stored Bot id is not a UUID".into()))?,
        name: row.try_get("", "name").map_err(backend)?,
        name_folded: row.try_get("", "name_folded").map_err(backend)?,
        instructions: row.try_get("", "instructions").map_err(backend)?,
        revision: row.try_get("", "revision").map_err(backend)?,
        archived: row.try_get::<i64>("", "archived").map_err(backend)? != 0,
        created_at_ms: row.try_get("", "created_at_ms").map_err(backend)?,
        updated_at_ms: row.try_get("", "updated_at_ms").map_err(backend)?,
    })
}

async fn read(
    db: &impl ConnectionTrait,
    id: Uuid,
) -> Result<Option<BotRecord>, BotRepositoryError> {
    db.query_one(statement(
        &format!("{SELECT} WHERE id=?"),
        vec![id.to_string().into()],
    ))
    .await
    .map_err(backend)?
    .map(record)
    .transpose()
}

fn classify_write(error: sea_orm::DbErr) -> BotRepositoryError {
    let text = error.to_string();
    if text.contains("uq_agena_bot_active_name") || text.contains("UNIQUE constraint failed") {
        BotRepositoryError::NameTaken
    } else {
        BotRepositoryError::Backend(text)
    }
}

#[async_trait]
impl BotRepository for SqliteBotRepository {
    async fn create(&self, value: &BotRecord) -> Result<BotRecord, BotRepositoryError> {
        self.database
            .execute(statement(
                "INSERT INTO agena_bots(id,name,name_folded,instructions,revision,archived,created_at_ms,updated_at_ms) VALUES(?,?,?,?,?,?,?,?)",
                vec![
                    value.id.to_string().into(),
                    value.name.clone().into(),
                    value.name_folded.clone().into(),
                    value.instructions.clone().into(),
                    value.revision.into(),
                    i64::from(value.archived).into(),
                    value.created_at_ms.into(),
                    value.updated_at_ms.into(),
                ],
            ))
            .await
            .map_err(classify_write)?;
        read(self.database.as_ref(), value.id)
            .await?
            .ok_or_else(|| BotRepositoryError::Backend("created Bot row is missing".into()))
    }

    async fn update(
        &self,
        value: &BotRecord,
        expected_revision: i64,
    ) -> Result<BotRecord, BotRepositoryError> {
        let changed = self
            .database
            .execute(statement(
                "UPDATE agena_bots SET name=?,name_folded=?,instructions=?,revision=?,updated_at_ms=? WHERE id=? AND revision=? AND archived=0",
                vec![
                    value.name.clone().into(),
                    value.name_folded.clone().into(),
                    value.instructions.clone().into(),
                    (expected_revision + 1).into(),
                    value.updated_at_ms.into(),
                    value.id.to_string().into(),
                    expected_revision.into(),
                ],
            ))
            .await
            .map_err(classify_write)?;
        if changed.rows_affected() != 1 {
            return Err(BotRepositoryError::Conflict);
        }
        read(self.database.as_ref(), value.id)
            .await?
            .ok_or(BotRepositoryError::NotFound)
    }

    async fn archive(
        &self,
        id: Uuid,
        expected_revision: i64,
        now_ms: i64,
    ) -> Result<BotRecord, BotRepositoryError> {
        let changed = self
            .database
            .execute(statement(
                "UPDATE agena_bots SET archived=1,revision=revision+1,updated_at_ms=? WHERE id=? AND revision=? AND archived=0",
                vec![now_ms.into(), id.to_string().into(), expected_revision.into()],
            ))
            .await
            .map_err(classify_write)?;
        if changed.rows_affected() != 1 {
            return Err(BotRepositoryError::Conflict);
        }
        read(self.database.as_ref(), id)
            .await?
            .ok_or(BotRepositoryError::NotFound)
    }

    async fn get(&self, id: Uuid) -> Result<BotRecord, BotRepositoryError> {
        read(self.database.as_ref(), id)
            .await?
            .ok_or(BotRepositoryError::NotFound)
    }

    async fn list(&self, include_archived: bool) -> Result<Vec<BotRecord>, BotRepositoryError> {
        let sql = if include_archived {
            format!("{SELECT} ORDER BY created_at_ms,id")
        } else {
            format!("{SELECT} WHERE archived=0 ORDER BY created_at_ms,id")
        };
        self.database
            .query_all(statement(&sql, vec![]))
            .await
            .map_err(backend)?
            .into_iter()
            .map(record)
            .collect()
    }

    async fn count(&self) -> Result<u64, BotRepositoryError> {
        let row = self
            .database
            .query_one(statement("SELECT COUNT(*) AS n FROM agena_bots", vec![]))
            .await
            .map_err(backend)?
            .ok_or_else(|| BotRepositoryError::Backend("count returned no row".into()))?;
        let count = row.try_get::<i64>("", "n").map_err(backend)?;
        u64::try_from(count).map_err(|_| BotRepositoryError::Backend("negative Bot count".into()))
    }

    async fn receipt(&self, key: &str) -> Result<Option<(String, String)>, BotRepositoryError> {
        let Some(row) = self
            .database
            .query_one(statement(
                "SELECT input,result FROM agena_bot_receipts WHERE request_key=?",
                vec![key.into()],
            ))
            .await
            .map_err(backend)?
        else {
            return Ok(None);
        };
        Ok(Some((
            row.try_get("", "input").map_err(backend)?,
            row.try_get("", "result").map_err(backend)?,
        )))
    }

    async fn save_receipt(
        &self,
        key: &str,
        input: &str,
        result: &str,
    ) -> Result<(), BotRepositoryError> {
        self.database
            .execute(statement(
                "INSERT INTO agena_bot_receipts(request_key,input,result) VALUES(?,?,?) ON CONFLICT(request_key) DO NOTHING",
                vec![key.into(), input.into(), result.into()],
            ))
            .await
            .map_err(backend)?;
        Ok(())
    }
}
