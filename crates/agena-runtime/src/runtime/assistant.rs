//! Assistant lifecycle, reminder scheduling and integration services.
use std::{sync::Arc, time::Duration};

use agena_assistant::{AssistantService, Error, Result};
use async_trait::async_trait;
use tokio::sync::{Mutex, Notify};
use uuid::Uuid;

mod channel_access;
mod companion;
mod integrations;

use super::builder::WeakAgenaRuntime;
use crate::session::SessionManager;

pub(super) struct AssistantRuntime {
    runtime: WeakAgenaRuntime,
    companion: Arc<dyn agena_assistant::CompanionRepository>,
    companion_gate: Mutex<()>,
    integrations: Arc<dyn agena_assistant::IntegrationRepository>,
    integration_gate: Mutex<()>,
    wake: Notify,
}

impl AssistantRuntime {
    pub(super) fn new(
        runtime: WeakAgenaRuntime,
        companion: Arc<dyn agena_assistant::CompanionRepository>,
        integrations: Arc<dyn agena_assistant::IntegrationRepository>,
    ) -> Arc<Self> {
        Arc::new(Self {
            runtime,
            companion,
            companion_gate: Mutex::new(()),
            integrations,
            integration_gate: Mutex::new(()),
            wake: Notify::new(),
        })
    }

    pub(super) fn start(self: &Arc<Self>, control: Arc<crate::TaskControl>) {
        self.install_channel_verifier();
        if let Ok(manager) = self.manager() {
            manager.install_memory_preferences(self.companion.clone());
        }
        self.start_integrations(control.clone());
        let service = self.clone();
        control.spawn(async move {
            loop {
                if let Err(error) = service.deliver_companion_reminders().await {
                    tracing::warn!(%error, "assistant reminder processing deferred");
                }
                tokio::select! {
                    _ = service.wake.notified() => {},
                    _ = tokio::time::sleep(Duration::from_secs(1)) => {},
                }
            }
        });
    }

    fn manager(&self) -> Result<Arc<SessionManager>> {
        let runtime = self
            .runtime
            .upgrade()
            .ok_or(Error::Unavailable("runtime is unavailable"))?;
        if runtime.is_shutdown() {
            return Err(Error::Unavailable("runtime is stopping"));
        }
        runtime
            .session_manager()
            .ok_or(Error::Unavailable("session runtime is unavailable"))
    }
}

fn runtime_error(error: impl std::fmt::Display) -> Error {
    tracing::warn!(diagnostic=%error, "assistant runtime operation failed");
    Error::Unavailable("runtime operation could not be completed; inspect server diagnostics")
}

#[async_trait]
impl AssistantService for AssistantRuntime {
    async fn update_channel_policy(
        &self,
        source: i64,
        id: Uuid,
        r: agena_assistant::ChannelPolicyUpdate,
    ) -> Result<agena_assistant::IntegrationBinding> {
        self.update_channel_access(source, id, r).await
    }

    async fn integrations(&self, source: i64) -> Result<agena_assistant::IntegrationsSnapshot> {
        self.integration_snapshot(source).await
    }
    async fn create_integration(
        &self,
        source: i64,
        request: agena_assistant::CreateIntegration,
    ) -> Result<agena_assistant::IntegrationBinding> {
        self.add_integration(source, request).await
    }
    async fn integration_detail(
        &self,
        source: i64,
        id: Uuid,
    ) -> Result<agena_assistant::IntegrationDetail> {
        self.read_integration(source, id).await
    }
    async fn control_integration(
        &self,
        source: i64,
        id: Uuid,
        request: agena_assistant::IntegrationControl,
    ) -> Result<agena_assistant::IntegrationBinding> {
        self.change_integration(source, id, request).await
    }

    async fn assistant(&self, source: i64) -> Result<agena_assistant::AssistantSnapshot> {
        self.assistant_snapshot(source).await
    }
    async fn update_assistant(
        &self,
        source: i64,
        update: agena_assistant::AssistantUpdate,
    ) -> Result<agena_assistant::AssistantProfile> {
        self.change_assistant_profile(source, update).await
    }
    async fn control_reminder(
        &self,
        source: i64,
        id: Uuid,
        control: agena_assistant::ReminderControl,
    ) -> Result<agena_assistant::Reminder> {
        self.change_reminder(source, id, control).await
    }
    async fn change_assistant_memory(
        &self,
        source: i64,
        update: agena_assistant::AssistantMemoryUpdate,
    ) -> Result<Option<agena_assistant::AssistantMemory>> {
        self.edit_assistant_memory(source, update).await
    }
}
