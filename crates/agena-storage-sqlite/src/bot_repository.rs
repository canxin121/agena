//! Durable instance Bot identities. Persona is presentation, never authority.
//!
//! A Bot owns no single conversation. Its conversations are ordinary
//! `agena_sessions` rows whose configuration carries `owner.bot`; this table
//! holds only the identity, revision and archived state that every such
//! conversation is checked against.
use crate::receipt_payload::{pack, unpack};
use agena_storage::{BotRecord, BotRepository, BotRepositoryError, BotWithLocalSession};
use async_trait::async_trait;
use sea_orm::{
    ConnectionTrait, DatabaseBackend, DatabaseConnection, Statement, TransactionTrait, Value,
};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use uuid::Uuid;

pub struct SqliteBotRepository {
    database: Arc<DatabaseConnection>,
    reader: Arc<DatabaseConnection>,
}

impl SqliteBotRepository {
    pub fn new(database: Arc<DatabaseConnection>) -> Self {
        Self::with_reader(database.clone(), database)
    }

    pub fn with_reader(database: Arc<DatabaseConnection>, reader: Arc<DatabaseConnection>) -> Self {
        Self { database, reader }
    }
}

fn backend(error: impl std::fmt::Display) -> BotRepositoryError {
    BotRepositoryError::Backend(error.to_string())
}

fn statement(sql: &str, values: Vec<Value>) -> Statement {
    Statement::from_sql_and_values(DatabaseBackend::Sqlite, sql, values)
}

const PROFILE_FIELDS: &str = "b.id,b.name,b.name_folded,b.instructions,b.revision,b.archived,b.created_at_ms,b.updated_at_ms,a.avatar,p.ids_json,p.publish_to_channels,json_extract(r.value_json,'$') AS agent_profile";
const PROFILE_TABLES: &str = "FROM agena_bots b LEFT JOIN agena_bot_avatars a ON a.bot_id=b.id
    LEFT JOIN agena_bot_prompts p ON p.bot_id=b.id
    LEFT JOIN agena_server_state r ON r.key='bot-agent-profile/'||b.id";

pub(crate) const LIVE_LOCAL_SESSION: &str = "agena_sessions s ON s.id=m.session_id
    AND s.deleted_at_ms IS NULL AND s.owner_bot_id=m.bot_id AND s.owner_surface='local'
    AND s.parent_session_id IS NULL AND s.workspace_id IS NULL";

fn profiles_query(suffix: &str) -> String {
    format!(
        "SELECT {PROFILE_FIELDS},s.id AS local_session_id {PROFILE_TABLES}
         LEFT JOIN agena_bot_local_sessions m ON m.bot_id=b.id
         LEFT JOIN {LIVE_LOCAL_SESSION} {suffix}"
    )
}

fn profile_row(row: sea_orm::QueryResult) -> Result<BotWithLocalSession, BotRepositoryError> {
    Ok(BotWithLocalSession {
        local_session_id: row.try_get("", "local_session_id").map_err(backend)?,
        record: record(row)?,
    })
}

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
        agent_profile: row.try_get("", "agent_profile").map_err(backend)?,
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

async fn write_agent_profile(
    db: &impl ConnectionTrait,
    value: &BotRecord,
) -> Result<(), BotRepositoryError> {
    let key = format!("bot-agent-profile/{}", value.id);
    let query = match &value.agent_profile {
        Some(profile) => statement(
            "INSERT INTO agena_server_state(key,value_json) VALUES(?,?) ON CONFLICT(key) DO UPDATE SET value_json=excluded.value_json",
            vec![
                key.into(),
                serde_json::to_string(profile).map_err(backend)?.into(),
            ],
        ),
        None => statement(
            "DELETE FROM agena_server_state WHERE key=?",
            vec![key.into()],
        ),
    };
    db.execute(query).await.map_err(backend)?;
    Ok(())
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
        &format!("SELECT {PROFILE_FIELDS} {PROFILE_TABLES} WHERE b.id=?"),
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
    async fn agent_profile(&self, id: Uuid) -> Result<Option<String>, BotRepositoryError> {
        self.reader
            .query_one(statement(
                "SELECT json_extract(value_json,'$') AS agent_profile FROM agena_server_state WHERE key=?",
                vec![format!("bot-agent-profile/{id}").into()],
            ))
            .await
            .map_err(backend)?
            .map(|row| row.try_get("", "agent_profile").map_err(backend))
            .transpose()
    }

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
        write_agent_profile(&transaction, value).await?;
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
        let current = read(&transaction, value.id)
            .await?
            .ok_or(BotRepositoryError::NotFound)?;
        if current.agent_profile != value.agent_profile {
            let busy = transaction
                .query_one(statement(
                    "SELECT EXISTS(SELECT 1 FROM agena_sessions s WHERE s.owner_bot_id=? AND s.deleted_at_ms IS NULL AND s.relation_kind<>'subagent' AND json_extract(s.configuration_json,'$.profile') IS NULL AND (EXISTS(SELECT 1 FROM agena_attempts a WHERE a.session_id=s.id AND a.phase IN ('queued','running')) OR EXISTS(SELECT 1 FROM agena_interactions i WHERE i.session_id=s.id AND i.decision_json IS NULL))) AS busy",
                    vec![value.id.to_string().into()],
                ))
                .await
                .map_err(backend)?
                .ok_or_else(|| backend("missing Bot activity result"))?
                .try_get::<bool>("", "busy")
                .map_err(backend)?;
            if busy {
                return Err(BotRepositoryError::AgentProfileBusy);
            }
            write_agent_profile(&transaction, value).await?;
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
        read(self.reader.as_ref(), id)
            .await?
            .ok_or(BotRepositoryError::NotFound)
    }

    async fn list(
        &self,
        include_archived: bool,
    ) -> Result<Vec<BotWithLocalSession>, BotRepositoryError> {
        let suffix = if include_archived {
            "ORDER BY b.created_at_ms,b.id"
        } else {
            "WHERE b.archived=0 ORDER BY b.created_at_ms,b.id"
        };
        self.reader
            .query_all(statement(&profiles_query(suffix), vec![]))
            .await
            .map_err(backend)?
            .into_iter()
            .map(profile_row)
            .collect()
    }

    async fn first_active(&self) -> Result<Option<BotWithLocalSession>, BotRepositoryError> {
        self.reader
            .query_one(statement(
                &profiles_query("WHERE b.id=(SELECT id FROM agena_bots WHERE archived=0 ORDER BY created_at_ms,id LIMIT 1)"),
                vec![],
            ))
            .await
            .map_err(backend)?
            .map(profile_row)
            .transpose()
    }

    async fn count(&self) -> Result<u64, BotRepositoryError> {
        let row = self
            .reader
            .query_one(statement("SELECT COUNT(*) AS n FROM agena_bots", vec![]))
            .await
            .map_err(backend)?
            .ok_or_else(|| BotRepositoryError::Backend("count returned no row".into()))?;
        let count = row.try_get::<i64>("", "n").map_err(backend)?;
        u64::try_from(count).map_err(|_| BotRepositoryError::Backend("negative Bot count".into()))
    }

    async fn receipt(&self, key: &str, input: &str) -> Result<Option<String>, BotRepositoryError> {
        let Some(row) = self
            .reader
            .query_one(statement(
                "SELECT input_digest,result FROM agena_bot_receipts WHERE request_key=?",
                vec![key.into()],
            ))
            .await
            .map_err(backend)?
        else {
            return Ok(None);
        };
        let stored: Vec<u8> = row.try_get("", "input_digest").map_err(backend)?;
        if stored.as_slice() != Sha256::digest(input).as_slice() {
            return Err(BotRepositoryError::ReplayedInput);
        }
        let payload: Vec<u8> = row.try_get("", "result").map_err(backend)?;
        let result = unpack(&payload).map_err(backend)?;
        Ok(Some(
            String::from_utf8(result.into_owned()).map_err(backend)?,
        ))
    }

    async fn save_receipt(
        &self,
        key: &str,
        input: &str,
        result: &str,
    ) -> Result<(), BotRepositoryError> {
        serde_json::from_str::<serde::de::IgnoredAny>(result).map_err(backend)?;
        let payload = pack(result.as_bytes()).map_err(backend)?;
        self.database
            .execute(statement(
                "INSERT INTO agena_bot_receipts(request_key,input_digest,result) VALUES(?,?,?) ON CONFLICT(request_key) DO NOTHING",
                vec![key.into(), Sha256::digest(input).to_vec().into(), payload.into()],
            ))
            .await
            .map_err(backend)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
