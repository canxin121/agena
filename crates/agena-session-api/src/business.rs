//! Business operations shared by HTTP, native clients and trusted local plugins.
//!
//! Requests reuse domain inputs. A session ID selects a target or assistant /
//! integration scope; callers do not need a current conversation or tool call.

use agena_assistant::{
    AssistantMemoryUpdate, AssistantUpdate, BotArchive, BotCreate, BotUpdate, ChannelPolicyUpdate,
    CreateIntegration, IntegrationControl, ReminderControl,
};
use agena_notification::model::{
    NotificationScope, NotificationSeverity, NotificationSource, NotificationSurface,
};
use agena_notification::service::NotificationFilter;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

mod notification;
pub use notification::*;

/// Business reads that complement the session and workspace queries.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "method", content = "params", rename_all = "snake_case")]
pub enum BusinessQuery {
    ListBots(BotListQuery),
    GetBot { bot_id: Uuid },
    GetAssistant { session_id: i64 },
    ListIntegrations { session_id: i64 },
    GetIntegration { session_id: i64, binding_id: Uuid },
    ListMemories,
    GetMemory { name: String },
    MemoryOverview,
    ListNotifications(NotificationFilterParams),
}

impl BusinessQuery {
    /// Target conversation, where the operation selects a conversation scope.
    pub fn session_id(&self) -> Option<i64> {
        match self {
            Self::GetAssistant { session_id }
            | Self::ListIntegrations { session_id }
            | Self::GetIntegration { session_id, .. } => Some(*session_id),
            _ => None,
        }
    }
}

/// Business writes executed by the same application use cases as HTTP.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "method", content = "params", rename_all = "snake_case")]
pub enum BusinessCommand {
    CreateBot(BotCreate),
    UpdateBot {
        bot_id: Uuid,
        #[serde(flatten)]
        request: BotUpdate,
    },
    ArchiveBot {
        bot_id: Uuid,
        #[serde(flatten)]
        request: BotArchive,
    },
    EnsureDefaultBotSession,
    EnsureBotSession {
        bot_id: Uuid,
    },
    /// Open or ensure the one local conversation; title applies only on creation.
    CreateBotConversation {
        bot_id: Uuid,
        #[serde(flatten)]
        request: BotConversationCreate,
    },
    UpdateAssistant {
        session_id: i64,
        #[serde(flatten)]
        request: AssistantUpdate,
    },
    ControlAssistantReminder {
        session_id: i64,
        reminder_id: Uuid,
        #[serde(flatten)]
        request: ReminderControl,
    },
    ChangeAssistantMemory {
        session_id: i64,
        #[serde(flatten)]
        request: AssistantMemoryUpdate,
    },
    CreateIntegration {
        session_id: i64,
        #[serde(flatten)]
        request: CreateIntegration,
    },
    ControlIntegration {
        session_id: i64,
        binding_id: Uuid,
        #[serde(flatten)]
        request: IntegrationControl,
    },
    UpdateChannelAccess {
        session_id: i64,
        binding_id: Uuid,
        #[serde(flatten)]
        request: ChannelPolicyUpdate,
    },
    SaveMemory {
        name: String,
        #[serde(flatten)]
        request: MemoryWriteRequest,
    },
    DeleteMemory {
        name: String,
        #[serde(flatten)]
        request: MemoryDeleteQuery,
    },
    DeleteMemories(MemoryBatchDeleteRequest),
    EnsureMemoryIndex,
    DismissNotification {
        notification_id: String,
    },
    ResolveNotificationAction {
        notification_id: String,
        action_id: String,
    },
    PauseActivity {
        activity_id: String,
    },
    ResumeActivity {
        activity_id: String,
    },
    DeleteActivity {
        activity_id: String,
    },
}

impl BusinessCommand {
    /// Target conversation, where the operation selects a conversation scope.
    pub fn session_id(&self) -> Option<i64> {
        match self {
            Self::UpdateAssistant { session_id, .. }
            | Self::ControlAssistantReminder { session_id, .. }
            | Self::ChangeAssistantMemory { session_id, .. }
            | Self::CreateIntegration { session_id, .. }
            | Self::ControlIntegration { session_id, .. }
            | Self::UpdateChannelAccess { session_id, .. } => Some(*session_id),
            _ => None,
        }
    }
}

/// List all registered Bots, optionally including archived identities.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BotListQuery {
    pub include_archived: bool,
}

/// Open or ensure the selected Bot's one local conversation.
/// A title only applies when the conversation is first created.
#[derive(Debug, Clone, Default, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BotConversationCreate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

/// Create a record without a revision, or update one at its observed revision.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryWriteRequest {
    #[serde(default)]
    pub description: String,
    pub kind: agena_domain::MemoryKind,
    pub body: String,
    pub expected_sha256: Option<String>,
}

/// Revisions observed before deleting a batch of memory records.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryBatchDeleteRequest {
    pub records: Vec<agena_domain::MemoryRevision>,
}

/// Revision observed before deleting one memory record.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryDeleteQuery {
    pub expected_sha256: String,
}

/// Flat notification filters shared with REST query-string decoding.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct NotificationFilterParams {
    pub scope_kind: Option<String>,
    pub scope_id: Option<i64>,
    pub scope_key: Option<String>,
    pub severity: Option<NotificationSeverity>,
    pub surface: Option<NotificationSurface>,
    pub source: Option<NotificationSource>,
    #[serde(default = "default_active_only")]
    pub active_only: bool,
    pub limit: Option<u64>,
    pub cursor: Option<i64>,
}

fn default_active_only() -> bool {
    true
}

impl Default for NotificationFilterParams {
    fn default() -> Self {
        Self {
            scope_kind: None,
            scope_id: None,
            scope_key: None,
            severity: None,
            surface: None,
            source: None,
            active_only: true,
            limit: None,
            cursor: None,
        }
    }
}

impl NotificationFilterParams {
    /// Resolve the public scope selector into the notification domain filter.
    pub fn into_filter(self) -> Result<NotificationFilter, String> {
        let scope = match (
            self.scope_kind.as_deref(),
            self.scope_id,
            self.scope_key.as_deref(),
        ) {
            (None, _, _) => None,
            (Some("session"), Some(id), _) => Some(NotificationScope::Session(id)),
            (Some("workspace"), Some(id), _) => Some(NotificationScope::Workspace(id)),
            (Some("tool_call"), _, Some(key)) => Some(NotificationScope::ToolCall(key.to_owned())),
            (Some("provider"), _, Some(key)) => Some(NotificationScope::Provider(key.to_owned())),
            (Some("plugin"), _, Some(key)) => Some(NotificationScope::Plugin(key.to_owned())),
            (Some("background_task"), _, Some(key)) => {
                Some(NotificationScope::BackgroundTask(key.to_owned()))
            }
            (Some(kind), None, None) => {
                return Err(format!(
                    "scope_kind={kind} requires scope_id (session/workspace) or scope_key (tool_call/provider/plugin/background_task)"
                ));
            }
            (Some(other), _, _) => return Err(format!("unsupported scope_kind: {other}")),
        };
        Ok(NotificationFilter {
            scope,
            severity: self.severity,
            kind: None,
            surface: self.surface,
            source: self.source,
            active_only: self.active_only,
            limit: self.limit.map(|limit| limit as usize),
            cursor: self.cursor,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn business_commands_keep_domain_inputs_and_explicit_targets() {
        let bot_id = Uuid::new_v4();
        let value = json!({
            "method": "update_bot",
            "params": {
                "bot_id": bot_id, "request_key": "rename-bot",
                "expected_revision": 7, "name": "Research",
                "persona": { "instructions": "Check the evidence." }
            }
        });
        let command: BusinessCommand = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(&command).unwrap(), value);
        match command {
            BusinessCommand::UpdateBot {
                bot_id: target,
                request,
            } => {
                assert_eq!(target, bot_id);
                assert_eq!(request.expected_revision, 7);
                assert_eq!(request.request_key, "rename-bot");
            }
            _ => panic!("expected Bot update"),
        }
    }

    #[test]
    fn notification_filters_have_http_defaults_and_scope() {
        let query: BusinessQuery = serde_json::from_value(json!({
            "method": "list_notifications",
            "params": { "scope_kind": "session", "scope_id": 42 }
        }))
        .unwrap();
        let BusinessQuery::ListNotifications(params) = query else {
            panic!("expected notification query");
        };
        assert!(params.active_only);
        assert_eq!(
            params.into_filter().unwrap().scope,
            Some(NotificationScope::Session(42))
        );
    }
}
