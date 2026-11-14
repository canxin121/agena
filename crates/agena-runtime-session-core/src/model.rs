//! Bounded, read-only session projections. Transcript content is read through
//! kernel history/content capabilities and is never retained on this value.
use agena_domain::{
    AgentStatus, ExecutionSelection, PermissionConfig, SessionConversation, SessionLifecycleState,
    SessionRelationKind, WorkflowState,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: i64,
    pub parent_id: Option<i64>,
    pub root_id: i64,
    pub depth: i64,
    pub workspace_id: Option<i64>,
    pub title: String,
    pub favorite: bool,
    pub pinned: bool,
    pub version: i64,
    pub relation_kind: SessionRelationKind,
    pub lifecycle_state: SessionLifecycleState,
    pub agent_name: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub message_count: u64,
    pub workflow: WorkflowState,
    pub execution: SessionExecutionContext,
    pub agent_status: Option<AgentStatus>,
    pub agent_started_at_ms: Option<i64>,
    pub agent_finished_at_ms: Option<i64>,
    pub agent_failure: Option<agena_failure::Failure>,
}

impl Session {
    pub fn blocked(&self) -> bool {
        self.workflow == WorkflowState::AwaitingInteraction
    }
    pub fn workflow_state(&self) -> WorkflowState {
        self.workflow
    }
    pub fn is_subagent(&self) -> bool {
        self.relation_kind.is_subagent()
    }
    pub fn effective_workspace_root(&self) -> Option<&Path> {
        self.execution.effective_workspace_root.as_deref()
    }
}

/// A request-scoped execution context; it carries policy and routing only.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionExecutionContext {
    #[serde(default)]
    pub global: bool,
    /// Conversation identity is independent of its execution workspace.
    #[serde(default)]
    pub owner: Option<agena_domain::SessionOwner>,
    pub scope_session_id: Option<i64>,
    #[serde(default)]
    pub relation_kind: SessionRelationKind,
    pub conversation: Option<SessionConversation>,
    #[serde(default)]
    pub prompts: agena_prompt_layers::PromptLayer,
    pub selection: ExecutionSelection,
    pub effective_permission: PermissionConfig,
    pub permission_ceiling: PermissionConfig,
    #[serde(default)]
    pub channel_authority: Option<agena_domain::ChannelAuthority>,
    #[serde(skip)]
    pub channel_verifier:
        Option<std::sync::Arc<dyn agena_runtime_contracts::ChannelAccessVerifier>>,
    pub capability_denied_tool_names: BTreeSet<String>,
    pub effective_workspace_root: Option<PathBuf>,
}
impl agena_runtime_contracts::ToolSessionContext for SessionExecutionContext {
    fn is_global(&self) -> bool {
        self.global
    }
    fn session_kind(&self) -> agena_domain::SessionKind {
        if self.global || self.owner.is_some() {
            agena_domain::SessionKind::Bot
        } else {
            agena_domain::SessionKind::Project
        }
    }
    fn relation_kind(&self) -> SessionRelationKind {
        self.relation_kind
    }
    fn conversation(&self) -> Option<&SessionConversation> {
        self.conversation.as_ref()
    }
    fn session_id(&self) -> Option<i64> {
        self.scope_session_id
    }
    fn effective_workspace_root(&self) -> Option<&Path> {
        self.effective_workspace_root.as_deref()
    }
    fn effective_permission(&self) -> &PermissionConfig {
        &self.effective_permission
    }
    fn permission_ceiling(&self) -> &PermissionConfig {
        &self.permission_ceiling
    }
    fn capability_denied_tool_names(&self) -> &BTreeSet<String> {
        &self.capability_denied_tool_names
    }
    fn channel_authority(&self) -> Option<&agena_domain::ChannelAuthority> {
        self.channel_authority.as_ref()
    }
    fn channel_verifier(
        &self,
    ) -> Option<std::sync::Arc<dyn agena_runtime_contracts::ChannelAccessVerifier>> {
        self.channel_verifier.clone()
    }
    fn selected_model(&self) -> Option<&str> {
        self.selection.model.as_deref()
    }
}
