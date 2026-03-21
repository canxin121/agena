use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter,
    QueryOrder,
};

use crate::db::entities::{message, part};
use crate::error::AppError;
use crate::message::{PartContent, SessionMessage, SessionMessagePart};
use crate::role::Role;

pub struct MessageRepository {
    db: DatabaseConnection,
}

impl MessageRepository {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }

    pub fn db(&self) -> &DatabaseConnection {
        &self.db
    }

    pub async fn create_message(&self, role: Role) -> Result<SessionMessage, AppError> {
        let now_ms = chrono::Utc::now().timestamp_millis();
        let model = message::ActiveModel {
            role: Set(role_to_db(role).to_string()),
            created_at_ms: Set(now_ms),
            metadata_json: Set("{}".to_string()),
            usage_json: Set(None),
            finish: Set(None),
            ..Default::default()
        }
        .insert(&self.db)
        .await?;

        model_to_session_message(model)
    }

    pub async fn create_user(&self, text: impl Into<String>) -> Result<SessionMessage, AppError> {
        let mut message = self.create_message(Role::User).await?;
        let part = self.create_part(message.id, PartContent::text(text)).await?;
        message.push_part(part);
        Ok(message)
    }

    pub async fn create_assistant(&self) -> Result<SessionMessage, AppError> {
        self.create_message(Role::Assistant).await
    }

    pub async fn create_system(
        &self,
        text: impl Into<String>,
    ) -> Result<SessionMessage, AppError> {
        let mut message = self.create_message(Role::System).await?;
        let part = self.create_part(message.id, PartContent::text(text)).await?;
        message.push_part(part);
        Ok(message)
    }

    pub async fn create_tool(&self) -> Result<SessionMessage, AppError> {
        self.create_message(Role::Tool).await
    }

    pub async fn create_part(
        &self,
        message_id: i64,
        part_content: PartContent,
    ) -> Result<SessionMessagePart, AppError> {
        let now_ms = chrono::Utc::now().timestamp_millis();
        let part_type_json = serde_json::to_string(&part_content)?;
        let model = part::ActiveModel {
            message_id: Set(message_id),
            part_type_json: Set(part_type_json),
            created_at_ms: Set(now_ms),
            ..Default::default()
        }
        .insert(&self.db)
        .await?;

        part_model_to_message_part(model)
    }

    pub async fn list_parts_by_message(
        &self,
        message_id: i64,
    ) -> Result<Vec<SessionMessagePart>, AppError> {
        let models = part::Entity::find()
            .filter(part::Column::MessageId.eq(message_id))
            .order_by_asc(part::Column::Id)
            .all(&self.db)
            .await?;

        models
            .into_iter()
            .map(part_model_to_message_part)
            .collect()
    }

    pub async fn get_message_with_parts(
        &self,
        message_id: i64,
    ) -> Result<Option<SessionMessage>, AppError> {
        let model = message::Entity::find_by_id(message_id).one(&self.db).await?;
        let Some(model) = model else {
            return Ok(None);
        };
        let mut message = model_to_session_message(model)?;
        message.parts = self.list_parts_by_message(message.id).await?;
        Ok(Some(message))
    }
}

fn role_to_db(role: Role) -> &'static str {
    match role {
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::System => "system",
        Role::Tool => "tool",
    }
}

fn role_from_db(role: &str) -> Result<Role, AppError> {
    match role {
        "user" => Ok(Role::User),
        "assistant" => Ok(Role::Assistant),
        "system" => Ok(Role::System),
        "tool" => Ok(Role::Tool),
        other => Err(AppError::InvalidRole(other.to_string())),
    }
}

fn model_to_session_message(model: message::Model) -> Result<SessionMessage, AppError> {
    let metadata_value: serde_json::Value = serde_json::from_str(&model.metadata_json)?;
    let metadata = match metadata_value {
        serde_json::Value::Object(map) => map.into_iter().collect(),
        _ => {
            return Err(AppError::Internal(
                "message.metadata is not a JSON object".to_string(),
            ));
        }
    };

    let usage = match model.usage_json {
        Some(value) => Some(serde_json::from_str(&value)?),
        None => None,
    };

    let created_at = chrono::DateTime::from_timestamp_millis(model.created_at_ms)
        .ok_or_else(|| AppError::Internal("invalid message.created_at_ms".to_string()))?;

    Ok(SessionMessage {
        id: model.id,
        role: role_from_db(&model.role)?,
        parts: Vec::new(),
        created_at,
        metadata,
        usage,
        finish: model.finish,
    })
}

fn part_model_to_message_part(model: part::Model) -> Result<SessionMessagePart, AppError> {
    let content: PartContent = serde_json::from_str(&model.part_type_json)?;
    let created_at = chrono::DateTime::from_timestamp_millis(model.created_at_ms)
        .ok_or_else(|| AppError::Internal("invalid part.created_at_ms".to_string()))?;

    Ok(SessionMessagePart {
        id: model.id,
        message_id: model.message_id,
        created_at,
        content,
    })
}
