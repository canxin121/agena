//! Assistant preferences, reminders and trusted integration contracts.

mod companion;
pub use companion::*;
mod integrations;
pub use integrations::*;

use async_trait::async_trait;
use uuid::Uuid;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Invalid(&'static str),
    #[error("{0}")]
    Policy(String),
    #[error("resource not found in this scope")]
    NotFound,
    #[error("{0}")]
    Conflict(&'static str),
    #[error("{0}")]
    Unavailable(&'static str),
    #[error("assistant persistence failed: {0}")]
    Storage(String),
}

/// Operator-facing service. This is not a public webhook contract: a trusted
/// adapter must authenticate external identities before granting this access.
#[async_trait]
pub trait AssistantService: Send + Sync {
    async fn integrations(&self, source: i64) -> Result<IntegrationsSnapshot>;
    async fn update_channel_policy(
        &self,
        source: i64,
        id: Uuid,
        request: ChannelPolicyUpdate,
    ) -> Result<IntegrationBinding>;

    async fn create_integration(
        &self,
        source: i64,
        request: CreateIntegration,
    ) -> Result<IntegrationBinding>;
    async fn integration_detail(&self, source: i64, id: Uuid) -> Result<IntegrationDetail>;
    async fn control_integration(
        &self,
        source: i64,
        id: Uuid,
        request: IntegrationControl,
    ) -> Result<IntegrationBinding>;

    async fn assistant(&self, source: i64) -> Result<AssistantSnapshot>;
    async fn update_assistant(
        &self,
        source: i64,
        update: AssistantUpdate,
    ) -> Result<AssistantProfile>;
    async fn control_reminder(
        &self,
        source: i64,
        id: Uuid,
        control: ReminderControl,
    ) -> Result<Reminder>;
    async fn change_assistant_memory(
        &self,
        source: i64,
        update: AssistantMemoryUpdate,
    ) -> Result<Option<AssistantMemory>>;
}

pub(crate) fn validate_key(key: &str) -> Result<()> {
    if key.is_empty() || key.len() > 160 || key.trim() != key || key.chars().any(char::is_control) {
        return Err(Error::Invalid(
            "request key must contain 1–160 non-control bytes without surrounding whitespace",
        ));
    }
    Ok(())
}

mod channel_access;
pub use channel_access::*;

pub mod bot;
pub use bot::*;
