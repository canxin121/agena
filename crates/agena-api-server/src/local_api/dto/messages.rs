use super::*;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum PartLoadMode {
    None,
    #[default]
    Summary,
    Full,
}

#[derive(Debug, Clone, Serialize)]
pub struct MessageResource {
    pub id: i64,
    pub session_id: i64,
    pub role: MessageRole,
    pub state: MessageStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub metadata: MessageMetadata,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<MessageUsage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finish: Option<String>,
    pub part_count: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parts: Option<Vec<MessagePart>>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct MessageListQuery {
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub limit: Option<u64>,
    #[serde(default)]
    pub parts: PartLoadMode,
}

#[cfg(test)]
#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct MessagePartWriteRequest {
    pub content: PartContent,
    #[serde(default)]
    pub status: Option<ExecutionStatus>,
    #[serde(default)]
    pub operation_id: Option<String>,
    #[serde(default)]
    pub created_at: Option<DateTime<Utc>>,
}

#[cfg(test)]
#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct MessageWriteRequest {
    pub role: Role,
    #[serde(default)]
    pub state: Option<MessageStatus>,
    #[serde(default)]
    pub metadata: Option<MessageMetadata>,
    #[serde(default)]
    pub usage: Option<MessageUsage>,
    #[serde(default)]
    pub finish: Option<String>,
    #[serde(default)]
    pub created_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub parts: Vec<MessagePartWriteRequest>,
}
