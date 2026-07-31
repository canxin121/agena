//! Stable commands for application-originated event publication.

use async_trait::async_trait;

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeEventPublishError {
    pub failure: agena_failure::Failure,
}

impl RuntimeEventPublishError {
    pub fn internal(diagnostic: impl std::fmt::Display) -> Self {
        Self {
            failure: crate::service_failure::unexpected_service_failure(
                "event.publish_failed",
                "The event could not be saved.",
                diagnostic,
            ),
        }
    }
}

impl std::fmt::Display for RuntimeEventPublishError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        crate::service_failure::display_service_failure(&self.failure, formatter)
    }
}

impl std::error::Error for RuntimeEventPublishError {}

#[async_trait]
pub trait RuntimeEventPublishService: Send + Sync {
    async fn publish_event(
        &self,
        request: RuntimeEventPublishRequest,
    ) -> Result<(), RuntimeEventPublishError>;
}
