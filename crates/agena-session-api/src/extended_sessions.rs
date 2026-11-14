//! Session capabilities shared by HTTP, native and callback transports.
use agena_domain::{CommandId, ComposerDocument, ExecutionId};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    content::ReadContentParams,
    live::ToolDetailSection,
    resource::{PermissionConfigResource, RunOptions},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "method", content = "params", rename_all = "snake_case")]
pub enum SessionQuery {
    Overview {
        workspace_id: Option<i64>,
        recent_limit: u64,
    },
    ExecutionAttempt {
        session_id: i64,
        execution_id: Uuid,
        attempt_generation: i64,
    },
    PromptPreview {
        session_id: i64,
    },
    Plan {
        session_id: i64,
    },
    ToolDetail {
        session_id: i64,
        part_id: i64,
        section: ToolDetailSection,
        locale: Option<String>,
    },
    QueuedMessage {
        session_id: i64,
        execution_id: ExecutionId,
    },
    InlineUi {
        session_id: i64,
        reference: String,
    },
    InlineUiSnapshot {
        session_id: i64,
        reference: String,
    },
    FileChanges {
        session_id: i64,
        #[serde(flatten)]
        query: SessionFileChangesParams,
    },
    WorkspaceStats {
        ids: Vec<i64>,
    },
    ReadStream(ReadStreamParams),
    ReadTransfer(ReadTransferParams),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "method", content = "params", rename_all = "snake_case")]
pub enum SessionCommand {
    UpdateMetadata {
        session_id: i64,
        #[serde(flatten)]
        update: SessionMetadataUpdate,
        expected_version: Option<i64>,
    },
    UpdatePermission {
        session_id: i64,
        permission: PermissionConfigResource,
        expected_version: Option<i64>,
    },
    CancelExecutionAttempt {
        session_id: i64,
        execution_id: Uuid,
        attempt_generation: i64,
    },
    EditQueuedMessage {
        request_id: CommandId,
        session_id: i64,
        execution_id: ExecutionId,
        expected_revision: i64,
        document: ComposerDocument,
    },
    DeleteQueuedMessage {
        request_id: CommandId,
        session_id: i64,
        execution_id: ExecutionId,
    },
    TakeQueuedMessage {
        request_id: CommandId,
        session_id: i64,
        execution_id: ExecutionId,
    },
    PauseQueue {
        session_id: i64,
        paused: bool,
    },
    ClearQueue {
        request_id: CommandId,
        session_id: i64,
    },
    StartBtw {
        session_id: i64,
        question: String,
        #[serde(default)]
        options: RunOptions,
    },
    OpenStream(StreamSource),
    CloseStream {
        stream_id: Uuid,
    },
    /// Export and media downloads return a bounded, seekable transfer handle.
    Export {
        session_id: i64,
    },
    OpenMedia {
        session_id: i64,
        path: String,
    },
    BeginUpload(UploadPurpose),
    WriteTransfer {
        transfer_id: Uuid,
        offset: u64,
        data_base64: String,
    },
    FinishUpload {
        transfer_id: Uuid,
    },
    CloseTransfer {
        transfer_id: Uuid,
    },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionMetadataUpdate {
    pub title: Option<String>,
    pub favorite: Option<bool>,
    pub pinned: Option<bool>,
    pub auto_title: Option<bool>,
    pub prompts: Option<agena_prompt_layers::PromptLayer>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionFileChangesParams {
    #[serde(default)]
    pub offset: usize,
    pub limit: Option<usize>,
    #[serde(default)]
    pub summary: bool,
    pub path: Option<String>,
    pub max_bytes: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ChangeScope {
    Global,
    Workspace { workspace_id: i64 },
    Session { session_id: i64 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", content = "params", rename_all = "snake_case")]
pub enum StreamSource {
    Changes {
        scope: ChangeScope,
        locale: Option<String>,
    },
    Content(ReadContentParams),
    Notifications {
        filter: crate::business::NotificationFilterParams,
        since_ms: Option<i64>,
    },
    InlineUi {
        session_id: i64,
        reference: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamHandle {
    pub stream_id: Uuid,
    /// Sequence numbers belong to this live handle. Durable content keeps its own cursor.
    pub after_seq: u64,
    pub idle_timeout_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadStreamParams {
    pub stream_id: Uuid,
    #[serde(default)]
    pub after_seq: u64,
    pub limit: Option<usize>,
    pub wait_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamEvent {
    pub sequence: u64,
    pub event: String,
    pub data: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamPage {
    pub events: Vec<StreamEvent>,
    pub next_seq: u64,
    /// Some requested live events have left the bounded retention window.
    pub lagged: bool,
    pub closed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum UploadPurpose {
    Attachment {
        session_id: i64,
        filename: String,
        mime: Option<String>,
    },
    Import {
        request_id: CommandId,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferHandle {
    pub transfer_id: Uuid,
    pub filename: Option<String>,
    pub size: u64,
    pub chunk_bytes: usize,
    pub idle_timeout_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadTransferParams {
    pub transfer_id: Uuid,
    #[serde(default)]
    pub offset: u64,
    pub max_bytes: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferChunk {
    pub offset: u64,
    pub next_offset: u64,
    pub data_base64: String,
    pub eof: bool,
}

impl SessionQuery {
    pub fn session_id(&self) -> Option<i64> {
        match self {
            Self::ExecutionAttempt { session_id, .. }
            | Self::PromptPreview { session_id }
            | Self::Plan { session_id }
            | Self::ToolDetail { session_id, .. }
            | Self::QueuedMessage { session_id, .. }
            | Self::InlineUi { session_id, .. }
            | Self::InlineUiSnapshot { session_id, .. }
            | Self::FileChanges { session_id, .. } => Some(*session_id),
            _ => None,
        }
    }
}
impl SessionCommand {
    pub fn session_id(&self) -> Option<i64> {
        match self {
            Self::UpdateMetadata { session_id, .. }
            | Self::UpdatePermission { session_id, .. }
            | Self::CancelExecutionAttempt { session_id, .. }
            | Self::EditQueuedMessage { session_id, .. }
            | Self::DeleteQueuedMessage { session_id, .. }
            | Self::TakeQueuedMessage { session_id, .. }
            | Self::PauseQueue { session_id, .. }
            | Self::ClearQueue { session_id, .. }
            | Self::StartBtw { session_id, .. }
            | Self::Export { session_id }
            | Self::OpenMedia { session_id, .. }
            | Self::BeginUpload(UploadPurpose::Attachment { session_id, .. }) => Some(*session_id),
            Self::OpenStream(source) => source.session_id(),
            _ => None,
        }
    }
}
impl StreamSource {
    pub fn session_id(&self) -> Option<i64> {
        match self {
            Self::Changes {
                scope: ChangeScope::Session { session_id },
                ..
            }
            | Self::InlineUi { session_id, .. } => Some(*session_id),
            Self::Content(request) => Some(request.session_id),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn metadata_and_file_history_parameters_survive_the_shared_wire_envelope() {
        let command = json!({"method":"extended_session","params":{"method":"update_metadata","params":{
            "session_id":7,"title":"renamed","favorite":true,"pinned":false,"expected_version":12
        }}});
        let decoded: crate::Command = serde_json::from_value(command).unwrap();
        assert_eq!(decoded.session_id(), Some(7));
        let encoded = serde_json::to_value(decoded).unwrap();
        assert_eq!(encoded["params"]["params"]["expected_version"], 12);
        assert_eq!(encoded["params"]["params"]["favorite"], true);
        let query = SessionQuery::FileChanges {
            session_id: 7,
            query: SessionFileChangesParams {
                offset: 2,
                limit: Some(3),
                path: Some("src/main.rs".into()),
                max_bytes: Some(42),
                summary: false,
            },
        };
        let envelope = serde_json::to_value(crate::Query::from(query)).unwrap();
        let decoded: crate::Query = serde_json::from_value(envelope.clone()).unwrap();
        assert_eq!(serde_json::to_value(decoded).unwrap(), envelope);
    }
}
