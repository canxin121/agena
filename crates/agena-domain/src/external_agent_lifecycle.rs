use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::ExternalAgent;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum ExternalAgentWorkspaceAction {
    List {
        agent: ExternalAgent,
        cursor: Option<String>,
    },
    Import {
        agent: ExternalAgent,
        native_id: String,
    },
    Open {
        agent: ExternalAgent,
        native_id: String,
    },
    PreviewHistory {
        agent: ExternalAgent,
        native_id: String,
        /// Exclusive one-based turn boundary returned by the previous page.
        before_turn: Option<u64>,
    },
    IndexList {
        agent: ExternalAgent,
        cursor: Option<String>,
        #[serde(default)]
        archived: ExternalAgentArchiveFilter,
    },
    BatchGet {
        agent: ExternalAgent,
        native_ids: Vec<String>,
    },
    SetArchived {
        agent: ExternalAgent,
        native_id: String,
        archived: bool,
    },
    SetTitle {
        agent: ExternalAgent,
        native_id: String,
        title: String,
    },
    Delete {
        agent: ExternalAgent,
        native_id: String,
    },
    Watch {
        agent: ExternalAgent,
        wait_seconds: Option<u16>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ExternalAgentWorkspaceResult {
    Page {
        sessions: Vec<ExternalAgentSessionInfo>,
        next_cursor: Option<String>,
    },
    Imported {
        session_id: i64,
    },
    Opened {
        session_id: i64,
    },
    HistoryPreview {
        preview: ExternalAgentHistoryPreview,
    },
    IndexPage {
        sessions: Vec<ExternalAgentIndexedSession>,
        next_cursor: Option<String>,
        capabilities: ExternalAgentIndexCapabilities,
    },
    Batch {
        sessions: Vec<ExternalAgentIndexedSession>,
    },
    Archive {
        native_id: String,
        archived: bool,
    },
    Title {
        native_id: String,
        title: String,
    },
    Deleted {
        native_id: String,
    },
    Watch {
        supported: bool,
        changed: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalAgentHistoryPreview {
    pub turns: Vec<ExternalAgentHistoryPreviewTurn>,
    pub next_before_turn: Option<u64>,
    pub omitted_content: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExternalAgentHistoryPreviewTurn {
    pub user: String,
    pub reply: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExternalAgentArchiveFilter {
    #[default]
    Unarchived,
    Archived,
    All,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct ExternalAgentIndexCapabilities {
    pub index: bool,
    pub archive: bool,
    pub batch_get: bool,
    pub watch: bool,
    #[serde(default)]
    pub title: bool,
    #[serde(default)]
    pub delete: bool,
    #[serde(default)]
    pub fork: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalAgentSessionInfo {
    pub native_id: String,
    pub cwd: PathBuf,
    pub title: Option<String>,
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalAgentIndexedSession {
    #[serde(flatten)]
    pub session: ExternalAgentSessionInfo,
    pub archived: Option<bool>,
    pub state: Option<String>,
    pub model: Option<String>,
    pub last_prompt_at: Option<String>,
}

impl From<ExternalAgentSessionInfo> for ExternalAgentIndexedSession {
    fn from(session: ExternalAgentSessionInfo) -> Self {
        Self {
            session,
            archived: None,
            state: None,
            model: None,
            last_prompt_at: None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExternalAgentSessionMetadata {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
}

impl ExternalAgentSessionMetadata {
    pub fn is_empty(&self) -> bool {
        self.title.is_none() && self.updated_at.is_none()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum ExternalAgentSessionAction {
    Status,
    SetTitle {
        title: String,
    },
    SetDirectories {
        directories: Vec<PathBuf>,
    },
    Close,
    Delete,
    Fork {
        title: Option<String>,
        before_message_id: Option<i64>,
    },
    Rewind {
        before_message_id: i64,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ExternalAgentSessionResult {
    Status {
        session: Option<ExternalAgentSessionInfo>,
        native_status: Option<String>,
        additional_directories: Vec<PathBuf>,
        capabilities: ExternalAgentSessionCapabilities,
        #[serde(default)]
        fork_points: Vec<ExternalAgentForkPoint>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        rewind_points: Vec<ExternalAgentForkPoint>,
    },
    Title {
        title: String,
    },
    Directories {
        directories: Vec<PathBuf>,
    },
    Closed,
    Deleted,
    Forked {
        session_id: i64,
    },
    Rewound {
        session_id: i64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalAgentForkPoint {
    pub message_id: i64,
    pub label: String,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ExternalAgentSessionCapabilities {
    pub status: bool,
    pub title: bool,
    pub close: bool,
    pub delete: bool,
    pub fork: bool,
    pub fork_before_message: bool,
    pub rewind_before_message: bool,
    pub additional_directories: bool,
    pub system_prompt: bool,
    #[serde(default = "can_continue_by_default")]
    pub continue_conversation: bool,
}

fn can_continue_by_default() -> bool {
    true
}
