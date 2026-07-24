//! Stable commands for application-originated event publication.

use async_trait::async_trait;
use thiserror::Error;

#[derive(Debug, Clone)]
pub enum RuntimeEventPublishRequest {
    PermissionRuleCreated(agena_domain::PermissionRuleEvent),
    PermissionRuleUpdated(agena_domain::PermissionRuleEvent),
    PermissionRuleRevoked(agena_domain::PermissionRuleEvent),
    PluginEvent {
        plugin_id: agena_plugin_host::PluginKey,
        kind_label: String,
        payload: serde_json::Value,
    },
}

#[derive(Debug, Clone, Error, PartialEq, Eq)]
#[error("event publication failed: {message}")]
pub struct RuntimeEventPublishError {
    message: String,
}

impl RuntimeEventPublishError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

#[async_trait]
pub trait RuntimeEventPublishService: Send + Sync {
    async fn publish_event(
        &self,
        request: RuntimeEventPublishRequest,
    ) -> Result<(), RuntimeEventPublishError>;
}
