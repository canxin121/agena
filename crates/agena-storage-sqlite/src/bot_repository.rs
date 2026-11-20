//! Durable instance Bot identities. Persona is presentation, never authority.
//!
//! A Bot owns no single conversation. Its conversations are ordinary
//! `agena_sessions` rows whose configuration carries `owner.bot`; this table
//! holds only the identity, revision and archived state that every such
//! conversation is checked against.
use agena_storage::{BotRecord, BotRepository, BotRepositoryError};
use async_trait::async_trait;
use sea_orm::{
    ConnectionTrait, DatabaseBackend, DatabaseConnection, Statement, TransactionTrait, Value,
};
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

const SELECT: &str = "SELECT b.id,b.name,b.name_folded,b.instructions,b.revision,b.archived,b.created_at_ms,b.updated_at_ms,a.avatar,p.ids_json,p.publish_to_channels FROM agena_bots b LEFT JOIN agena_bot_avatars a ON a.bot_id=b.id LEFT JOIN agena_bot_prompts p ON p.bot_id=b.id";

fn record(row: sea_orm::QueryResult) -> Result<BotRecord, BotRepositoryError> {
    let prompt_ids = row
        .try_get::<Option<String>>("", "ids_json")
        .map_err(backend)?;
    Ok(BotRecord {
        id: row
            .try_get::<String>("", "id")
            .map_err(backend)?
            .parse()
            .map_err(|_| BotRepositoryError::Backend("stored Bot id is not a UUID".into()))?,
        name: row.try_get("", "name").map_err(backend)?,
        name_folded: row.try_get("", "name_folded").map_err(backend)?,
        instructions: row.try_get("", "instructions").map_err(backend)?,
        prompt_ids: prompt_ids
            .as_deref()
            .map(serde_json::from_str)
            .transpose()
            .map_err(backend)?
            .unwrap_or_default(),
        publish_prompts: row
            .try_get::<Option<i64>>("", "publish_to_channels")
            .map_err(backend)?
            == Some(1),
        avatar: row.try_get("", "avatar").map_err(backend)?,
        revision: row.try_get("", "revision").map_err(backend)?,
        archived: row.try_get::<i64>("", "archived").map_err(backend)? != 0,
        created_at_ms: row.try_get("", "created_at_ms").map_err(backend)?,
        updated_at_ms: row.try_get("", "updated_at_ms").map_err(backend)?,
    })
}

async fn write_prompts(
    db: &impl ConnectionTrait,
    value: &BotRecord,
) -> Result<(), BotRepositoryError> {
    let id = value.id.to_string();
    let query = if value.prompt_ids.is_empty() {
        statement(
            "DELETE FROM agena_bot_prompts WHERE bot_id=?",
            vec![id.into()],
        )
    } else {
        statement(
            "INSERT INTO agena_bot_prompts(bot_id,ids_json,publish_to_channels) VALUES(?,?,?) ON CONFLICT(bot_id) DO UPDATE SET ids_json=excluded.ids_json,publish_to_channels=excluded.publish_to_channels",
            vec![
                id.into(),
                serde_json::to_string(&value.prompt_ids)
                    .map_err(backend)?
                    .into(),
                i64::from(value.publish_prompts).into(),
            ],
        )
    };
    db.execute(query).await.map_err(backend)?;
    Ok(())
}

async fn read(
    db: &impl ConnectionTrait,
    id: Uuid,
) -> Result<Option<BotRecord>, BotRepositoryError> {
    db.query_one(statement(
        &format!("{SELECT} WHERE b.id=?"),
        vec![id.to_string().into()],
    ))
    .await
    .map_err(backend)?
    .map(record)
    .transpose()
}

async fn write_avatar(
    db: &impl ConnectionTrait,
    value: &BotRecord,
) -> Result<(), BotRepositoryError> {
    match &value.avatar {
        Some(avatar) => {
            db.execute(statement(
                "INSERT INTO agena_bot_avatars(bot_id,avatar) VALUES(?,?) ON CONFLICT(bot_id) DO UPDATE SET avatar=excluded.avatar",
                vec![value.id.to_string().into(), avatar.clone().into()],
            ))
            .await
            .map_err(backend)?;
        }
        None => {
            db.execute(statement(
                "DELETE FROM agena_bot_avatars WHERE bot_id=?",
                vec![value.id.to_string().into()],
            ))
            .await
            .map_err(backend)?;
        }
    }
    Ok(())
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
        let transaction = self.database.begin().await.map_err(backend)?;
        transaction
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
        write_avatar(&transaction, value).await?;
        write_prompts(&transaction, value).await?;
        let created = read(&transaction, value.id)
            .await?
            .ok_or_else(|| BotRepositoryError::Backend("created Bot row is missing".into()))?;
        transaction.commit().await.map_err(backend)?;
        Ok(created)
    }

    async fn update(
        &self,
        value: &BotRecord,
        expected_revision: i64,
    ) -> Result<BotRecord, BotRepositoryError> {
        let transaction = self.database.begin().await.map_err(backend)?;
        let changed = transaction
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
        write_avatar(&transaction, value).await?;
        write_prompts(&transaction, value).await?;
        let updated = read(&transaction, value.id)
            .await?
            .ok_or(BotRepositoryError::NotFound)?;
        transaction.commit().await.map_err(backend)?;
        Ok(updated)
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
            format!("{SELECT} ORDER BY b.created_at_ms,b.id")
        } else {
            format!("{SELECT} WHERE b.archived=0 ORDER BY b.created_at_ms,b.id")
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

#[cfg(test)]
mod tests;
