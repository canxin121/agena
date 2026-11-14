//! Typed business use cases shared by HTTP and the operation dispatcher.

use agena_api::business::{
    BusinessCommand, BusinessQuery, MemoryWriteRequest, NotificationFilterParams,
};
use agena_api::pagination::{PageInfo, PaginatedResponse, normalize_limit};
use agena_api::resource::{
    BackgroundActivityResource, NotificationActionTargetResource, NotificationResource,
};
use agena_assistant::{
    AssistantMemory, AssistantMemoryUpdate, AssistantProfile, AssistantSnapshot, AssistantUpdate,
    ChannelPolicyUpdate, CreateIntegration, IntegrationBinding, IntegrationControl,
    IntegrationDetail, IntegrationsSnapshot, PlatformInventory, Reminder, ReminderControl,
};
use agena_notification::NotificationService;
use agena_notification::service::NotificationError;
use serde::Serialize;
use serde_json::Value;
use uuid::Uuid;

use crate::dto::MemoryResource;
use crate::{Application, ApplicationError};

/// Directory and records displayed by the memory management surface.
#[derive(Debug, Serialize)]
pub struct MemoryOverview {
    pub workspace_root: std::path::PathBuf,
    pub directory: std::path::PathBuf,
    pub items: Vec<MemoryResource>,
}

/// The ensured memory index, shared by every transport.
#[derive(Debug, Serialize)]
pub struct MemoryIndex {
    pub path: std::path::PathBuf,
}

impl Application {
    pub(crate) async fn dispatch_business_query(
        &self,
        query: BusinessQuery,
    ) -> Result<Value, ApplicationError> {
        match query {
            BusinessQuery::ListBots(request) => {
                data(self.list_bots(request.include_archived).await?)
            }
            BusinessQuery::GetBot { bot_id } => data(self.get_bot(bot_id).await?),
            BusinessQuery::GetAssistant { session_id } => {
                data(self.assistant_snapshot(session_id).await?)
            }
            BusinessQuery::ListIntegrations { session_id } => {
                data(self.integrations(session_id).await?)
            }
            BusinessQuery::GetIntegration {
                session_id,
                binding_id,
            } => data(self.integration_detail(session_id, binding_id).await?),
            BusinessQuery::ListMemories => data(self.list_memories().await?),
            BusinessQuery::GetMemory { name } => data(self.get_memory(name).await?),
            BusinessQuery::MemoryOverview => data(self.memory_overview().await?),
            BusinessQuery::ListNotifications(request) => {
                data(self.list_notifications(request).await?)
            }
        }
    }

    pub(crate) async fn dispatch_business_command(
        &self,
        command: BusinessCommand,
    ) -> Result<Value, ApplicationError> {
        match command {
            BusinessCommand::CreateBot(request) => data(self.create_bot(request).await?),
            BusinessCommand::UpdateBot { bot_id, request } => {
                data(self.update_bot(bot_id, request).await?)
            }
            BusinessCommand::ArchiveBot { bot_id, request } => {
                data(self.archive_bot(bot_id, request).await?)
            }
            BusinessCommand::EnsureDefaultBotSession => data(self.ensure_bot_session().await?),
            BusinessCommand::EnsureBotSession { bot_id } => {
                data(self.ensure_named_bot_session(bot_id).await?)
            }
            BusinessCommand::CreateBotConversation { bot_id, request } => {
                data(self.create_bot_conversation(bot_id, request.title).await?)
            }
            BusinessCommand::UpdateAssistant {
                session_id,
                request,
            } => data(self.update_assistant(session_id, request).await?),
            BusinessCommand::ControlAssistantReminder {
                session_id,
                reminder_id,
                request,
            } => data(
                self.control_assistant_reminder(session_id, reminder_id, request)
                    .await?,
            ),
            BusinessCommand::ChangeAssistantMemory {
                session_id,
                request,
            } => data(self.change_assistant_memory(session_id, request).await?),
            BusinessCommand::CreateIntegration {
                session_id,
                request,
            } => data(self.create_integration(session_id, request).await?),
            BusinessCommand::ControlIntegration {
                session_id,
                binding_id,
                request,
            } => data(
                self.control_integration(session_id, binding_id, request)
                    .await?,
            ),
            BusinessCommand::UpdateChannelAccess {
                session_id,
                binding_id,
                request,
            } => data(
                self.update_channel_access(session_id, binding_id, request)
                    .await?,
            ),
            BusinessCommand::SaveMemory { name, request } => {
                data(self.save_memory(name, request).await?)
            }
            BusinessCommand::DeleteMemory { name, request } => {
                data(self.delete_memory(name, request.expected_sha256).await?)
            }
            BusinessCommand::DeleteMemories(request) => {
                data(self.delete_memories(request.records).await?)
            }
            BusinessCommand::EnsureMemoryIndex => data(self.ensure_memory_index().await?),
            BusinessCommand::DismissNotification { notification_id } => {
                self.dismiss_notification(notification_id).await?;
                Ok(Value::Null)
            }
            BusinessCommand::ResolveNotificationAction {
                notification_id,
                action_id,
            } => data(
                self.resolve_notification_action(notification_id, action_id)
                    .await?,
            ),
            BusinessCommand::PauseActivity { activity_id } => {
                data(self.pause_activity(&activity_id).await?)
            }
            BusinessCommand::ResumeActivity { activity_id } => {
                data(self.resume_activity(&activity_id).await?)
            }
            BusinessCommand::DeleteActivity { activity_id } => {
                data(self.delete_activity(&activity_id).await?)
            }
        }
    }

    pub async fn assistant_snapshot(
        &self,
        source: i64,
    ) -> Result<AssistantSnapshot, ApplicationError> {
        self.assistant_service()?
            .assistant(source)
            .await
            .map_err(Into::into)
    }

    pub async fn update_assistant(
        &self,
        source: i64,
        request: AssistantUpdate,
    ) -> Result<AssistantProfile, ApplicationError> {
        self.assistant_service()?
            .update_assistant(source, request)
            .await
            .map_err(Into::into)
    }

    pub async fn control_assistant_reminder(
        &self,
        source: i64,
        id: Uuid,
        request: ReminderControl,
    ) -> Result<Reminder, ApplicationError> {
        self.assistant_service()?
            .control_reminder(source, id, request)
            .await
            .map_err(Into::into)
    }

    pub async fn change_assistant_memory(
        &self,
        source: i64,
        request: AssistantMemoryUpdate,
    ) -> Result<Option<AssistantMemory>, ApplicationError> {
        self.assistant_service()?
            .change_assistant_memory(source, request)
            .await
            .map_err(Into::into)
    }

    pub async fn integrations(
        &self,
        source: i64,
    ) -> Result<IntegrationsSnapshot, ApplicationError> {
        self.assistant_service()?
            .integrations(source)
            .await
            .map_err(Into::into)
    }

    pub async fn create_integration(
        &self,
        source: i64,
        request: CreateIntegration,
    ) -> Result<IntegrationBinding, ApplicationError> {
        self.assistant_service()?
            .create_integration(source, request)
            .await
            .map_err(Into::into)
    }

    pub async fn integration_detail(
        &self,
        source: i64,
        id: Uuid,
    ) -> Result<IntegrationDetail, ApplicationError> {
        self.assistant_service()?
            .integration_detail(source, id)
            .await
            .map_err(Into::into)
    }

    pub async fn control_integration(
        &self,
        source: i64,
        id: Uuid,
        request: IntegrationControl,
    ) -> Result<IntegrationBinding, ApplicationError> {
        self.assistant_service()?
            .control_integration(source, id, request)
            .await
            .map_err(Into::into)
    }

    pub async fn update_channel_access(
        &self,
        source: i64,
        id: Uuid,
        request: ChannelPolicyUpdate,
    ) -> Result<IntegrationBinding, ApplicationError> {
        self.assistant_service()?
            .update_channel_policy(source, id, request)
            .await
            .map_err(Into::into)
    }

    pub async fn platform_inventory(
        &self,
        source: i64,
    ) -> Result<PlatformInventory, ApplicationError> {
        self.assistant_service()?
            .platform_inventory(source)
            .await
            .map_err(Into::into)
    }

    pub async fn list_memories(&self) -> Result<Vec<MemoryResource>, ApplicationError> {
        self.run_blocking(|app| app.service().list_memories())
            .await?
    }

    pub async fn get_memory(&self, name: String) -> Result<MemoryResource, ApplicationError> {
        self.run_blocking(move |app| app.service().get_memory(&name))
            .await?
    }

    pub async fn save_memory(
        &self,
        name: String,
        request: MemoryWriteRequest,
    ) -> Result<MemoryResource, ApplicationError> {
        self.run_blocking(move |app| app.service().save_memory(&name, request))
            .await?
    }

    pub async fn delete_memory(
        &self,
        name: String,
        expected_sha256: String,
    ) -> Result<MemoryResource, ApplicationError> {
        self.run_blocking(move |app| app.service().delete_memory(&name, &expected_sha256))
            .await?
    }

    pub async fn delete_memories(
        &self,
        records: Vec<agena_domain::MemoryRevision>,
    ) -> Result<Vec<String>, ApplicationError> {
        self.run_blocking(move |app| app.service().delete_memories(&records))
            .await?
    }

    pub async fn memory_overview(&self) -> Result<MemoryOverview, ApplicationError> {
        let workspace_root = self.runtime_diagnostics().await.workspace_root;
        let (directory, items) = self
            .run_blocking(|app| {
                Ok::<_, ApplicationError>((
                    app.service().memory_directory(),
                    app.service().list_memories()?,
                ))
            })
            .await??;
        Ok(MemoryOverview {
            workspace_root,
            directory,
            items,
        })
    }

    pub async fn ensure_memory_index(&self) -> Result<MemoryIndex, ApplicationError> {
        let path = self
            .run_blocking(|app| app.service().memory_index_path())
            .await??;
        Ok(MemoryIndex { path })
    }

    /// Replay notifications and then follow live updates from the shared store.
    /// Dropping the receiver stops the subscription producer.
    pub async fn watch_notifications(
        &self,
        request: NotificationFilterParams,
        since_ms: Option<i64>,
    ) -> Result<
        tokio::sync::mpsc::Receiver<agena_api::business::NotificationStreamEvent>,
        ApplicationError,
    > {
        watch_notification_store(self.notifications().clone(), request, since_ms).await
    }

    pub async fn list_notifications(
        &self,
        request: NotificationFilterParams,
    ) -> Result<PaginatedResponse<NotificationResource>, ApplicationError> {
        let limit = normalize_limit(request.limit) as usize;
        let mut filter = request.into_filter().map_err(|diagnostic| {
            ApplicationError::bad_request_with_diagnostic(
                "The notification filter is invalid.",
                diagnostic,
            )
        })?;
        filter.limit = Some(limit);
        let items = self
            .notifications()
            .list(filter)
            .await
            .map_err(ApplicationError::from)?;
        let returned = items.len() as u64;
        let next_cursor = items
            .last()
            .map(|notification| notification.created_at_ms.to_string());
        Ok(PaginatedResponse {
            total: None,
            items: items.iter().map(NotificationResource::from).collect(),
            page: PageInfo {
                next_cursor,
                has_more: returned >= limit as u64,
                returned,
                ..Default::default()
            },
        })
    }

    pub async fn dismiss_notification(&self, id: String) -> Result<(), ApplicationError> {
        self.notifications()
            .dismiss(id, None)
            .await
            .map_err(Into::into)
    }

    pub async fn resolve_notification_action(
        &self,
        id: String,
        action_id: String,
    ) -> Result<NotificationActionTargetResource, ApplicationError> {
        let target = self
            .notifications()
            .resolve_target(id, action_id)
            .await
            .map_err(ApplicationError::from)?;
        Ok(NotificationActionTargetResource::from(&target))
    }

    pub async fn pause_activity(
        &self,
        id: &str,
    ) -> Result<BackgroundActivityResource, ApplicationError> {
        let activity = self
            .runtime_activities()?
            .pause_activity(id)
            .await
            .map_err(|error| ApplicationError::bad_request_error(&error))?;
        Ok(BackgroundActivityResource::from(&activity))
    }

    pub async fn resume_activity(
        &self,
        id: &str,
    ) -> Result<BackgroundActivityResource, ApplicationError> {
        let activity = self
            .runtime_activities()?
            .resume_activity(id)
            .await
            .map_err(|error| ApplicationError::bad_request_error(&error))?;
        Ok(BackgroundActivityResource::from(&activity))
    }

    pub async fn delete_activity(
        &self,
        id: &str,
    ) -> Result<BackgroundActivityResource, ApplicationError> {
        let activity = self
            .runtime_activities()?
            .delete_activity(id)
            .await
            .map_err(|error| ApplicationError::bad_request_error(&error))?;
        Ok(BackgroundActivityResource::from(&activity))
    }
}

async fn watch_notification_store(
    store: std::sync::Arc<agena_runtime_notifications::store::InMemoryNotificationStore>,
    request: NotificationFilterParams,
    since_ms: Option<i64>,
) -> Result<
    tokio::sync::mpsc::Receiver<agena_api::business::NotificationStreamEvent>,
    ApplicationError,
> {
    use agena_api::business::NotificationStreamEvent;
    use agena_runtime_notifications::store::SubscriptionEvent;

    let since_ms = since_ms.unwrap_or(0);
    let mut filter = request.into_filter().map_err(|diagnostic| {
        ApplicationError::bad_request_with_diagnostic(
            "The notification filter is invalid.",
            diagnostic,
        )
    })?;
    filter.limit = Some(1000);
    // Admission precedes the snapshot: a notice changed while the list loads
    // must remain queued, including updates to a replayed row.
    let mut events = store.subscribe_events();
    let replayed = store
        .list(filter.clone())
        .await
        .map_err(ApplicationError::from)?;
    let watermark = replayed
        .iter()
        .filter(|notice| notice.created_at_ms > since_ms)
        .fold(since_ms, |at, notice| at.max(notice.created_at_ms));
    let (tx, rx) = tokio::sync::mpsc::channel(256);
    tokio::spawn(async move {
        for notice in replayed
            .into_iter()
            .filter(|notice| notice.created_at_ms > since_ms)
        {
            if tx
                .send(NotificationStreamEvent::Notification(Box::new(
                    NotificationResource::from(&notice),
                )))
                .await
                .is_err()
            {
                return;
            }
        }
        if tx
            .send(NotificationStreamEvent::Resumed {
                up_to_ms: watermark,
            })
            .await
            .is_err()
        {
            return;
        }
        let live_filter = agena_notification::service::NotificationFilter {
            active_only: false,
            ..filter
        };
        loop {
            tokio::task::consume_budget().await;
            let item = tokio::select! {
                _ = tx.closed() => break,
                item = events.recv() => item,
            };
            let item = match item {
                Ok(item) => item,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                    SubscriptionEvent::Lagged(skipped)
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => SubscriptionEvent::Closed,
            };
            let (event, closed) = match item {
                SubscriptionEvent::Notification(notice) => {
                    // Dismissals must remove a previously visible row even
                    // when the initial snapshot requested active rows only.
                    if !agena_runtime_notifications::store::filter_matches(&live_filter, &notice) {
                        continue;
                    }
                    (
                        NotificationStreamEvent::Notification(Box::new(
                            NotificationResource::from(notice.as_ref()),
                        )),
                        false,
                    )
                }
                SubscriptionEvent::Lagged(skipped) => {
                    (NotificationStreamEvent::Lagged { skipped }, false)
                }
                SubscriptionEvent::Closed => (
                    NotificationStreamEvent::SubscriptionClosed {
                        reason: "notification store closed".into(),
                    },
                    true,
                ),
            };
            if tx.send(event).await.is_err() || closed {
                break;
            }
        }
    });
    Ok(rx)
}

fn data(value: impl Serialize) -> Result<Value, ApplicationError> {
    serde_json::to_value(value).map_err(|error| ApplicationError::internal_error(&error))
}

impl From<agena_assistant::Error> for ApplicationError {
    fn from(error: agena_assistant::Error) -> Self {
        match error {
            agena_assistant::Error::Invalid(message) => Self::bad_request(message),
            agena_assistant::Error::Conflict(message) => Self::conflict(message),
            agena_assistant::Error::NotFound => {
                Self::not_found("assistant resource not found in this scope")
            }
            agena_assistant::Error::Unavailable(message) => Self::service_unavailable(message),
            agena_assistant::Error::Policy(diagnostic) => Self::bad_request_with_diagnostic(
                "invalid channel access policy; check identities, owners, role capabilities and limits",
                diagnostic,
            ),
            agena_assistant::Error::Storage(diagnostic) => Self::internal(diagnostic),
        }
    }
}

impl From<NotificationError> for ApplicationError {
    fn from(error: NotificationError) -> Self {
        match error {
            NotificationError::NotFound(id) => Self::not_found_with_diagnostic(
                "The notification was not found.",
                format!("notification id: {id}"),
            ),
            NotificationError::Validation(message) => {
                Self::bad_request_with_diagnostic("The notification request is invalid.", message)
            }
            NotificationError::Conflict(message) => Self::conflict_with_diagnostic(
                "The notification operation conflicts with the current state.",
                message,
            ),
            NotificationError::Unavailable(message) => Self::service_unavailable(message),
            other => Self::internal(other.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agena_api::business::NotificationStreamEvent;
    use agena_notification::model::{
        Notification, NotificationKind, NotificationScope, NotificationSeverity,
        NotificationSource, NotificationSurface,
    };
    use agena_runtime_notifications::store::InMemoryNotificationStore;
    use std::sync::Arc;

    fn notification(id: &str, session_id: i64, created_at_ms: i64) -> Notification {
        Notification {
            id: id.into(),
            kind: NotificationKind::Notice {
                code: "operation-test".into(),
            },
            severity: NotificationSeverity::Info,
            scope: NotificationScope::Session(session_id),
            surface: NotificationSurface::Log,
            source: NotificationSource::Runtime,
            summary: id.into(),
            detail: None,
            control: Default::default(),
            actions: Vec::new(),
            priority: 0,
            dedup_key: None,
            created_at_ms,
            expires_at_ms: None,
            dismissed: false,
        }
    }

    async fn next(
        receiver: &mut tokio::sync::mpsc::Receiver<NotificationStreamEvent>,
    ) -> NotificationStreamEvent {
        tokio::time::timeout(std::time::Duration::from_secs(2), receiver.recv())
            .await
            .expect("notification producer should make progress")
            .expect("notification stream should remain open")
    }

    #[tokio::test]
    async fn shared_notification_stream_replays_then_delivers_live_dismissals() {
        let store = Arc::new(InMemoryNotificationStore::new(8));
        store.ingest(notification("too-old", 42, 100));
        store.ingest(notification("replayed", 42, 200));
        store.ingest(notification("other-session", 99, 300));
        let mut stream = watch_notification_store(
            store.clone(),
            NotificationFilterParams {
                scope_kind: Some("session".into()),
                scope_id: Some(42),
                ..Default::default()
            },
            Some(150),
        )
        .await
        .unwrap();

        // This update arrives before the consumer drains the initial replay.
        store.ingest(notification("live", 42, 400));
        let NotificationStreamEvent::Notification(replayed) = next(&mut stream).await else {
            panic!("history must precede the resumed watermark");
        };
        assert_eq!(replayed.id, "replayed");
        assert!(matches!(
            next(&mut stream).await,
            NotificationStreamEvent::Resumed { up_to_ms: 200 }
        ));
        let NotificationStreamEvent::Notification(live) = next(&mut stream).await else {
            panic!("live update must follow replay");
        };
        assert_eq!(live.id, "live");

        store.dismiss("replayed".into(), None).await.unwrap();
        let NotificationStreamEvent::Notification(dismissed) = next(&mut stream).await else {
            panic!("active-only subscribers still receive dismissals");
        };
        assert_eq!(dismissed.id, "replayed");
        assert!(dismissed.dismissed);
    }

    #[tokio::test]
    async fn notification_subscription_validates_scope_before_starting() {
        let error = watch_notification_store(
            Arc::new(InMemoryNotificationStore::new(8)),
            NotificationFilterParams {
                scope_kind: Some("session".into()),
                ..Default::default()
            },
            None,
        )
        .await
        .unwrap_err();
        assert_eq!(
            error.failure.category,
            agena_failure::FailureCategory::InvalidInput
        );
    }
}
