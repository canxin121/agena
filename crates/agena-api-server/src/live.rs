//! HTTP stream delivery and resource revision adaptation.
use crate::{error::ServerError, state::AppState};
use agena_api::{
    Scope,
    live::{SessionPartsResource, ToolDetailResource, ToolDetailSection},
    part::PartResource,
};
pub(crate) use agena_application::LiveItem;
use std::sync::Arc;
#[cfg(feature = "ws")]
use tokio::sync::mpsc;

#[cfg(feature = "ws")]
pub(crate) async fn spawn_content_delivery(
    state: &AppState,
    id: agena_api::subscribe::SubscriptionId,
    request: agena_api::content::ReadContentParams,
    tx: mpsc::Sender<agena_api::ws::ServerMessage>,
) -> Result<tokio::task::JoinHandle<()>, ServerError> {
    use agena_api::ws::ServerMessage;
    let watch = state
        .application()
        .watch_content(request.session_id, request.resource_id)
        .await?;
    watch.read(request.after, 1).await?; // Reject an invalid cursor before acknowledging.
    let mut delivery = watch.delivery(request.after, request.max_bytes);
    tx.send(ServerMessage::Subscribed { id: id.clone() })
        .await
        .map_err(|_| ServerError::bad_request("The subscription connection closed."))?;
    Ok(tokio::spawn(async move {
        loop {
            let page = tokio::select! {
                _ = tx.closed() => return,
                page = delivery.next() => page,
            };
            let message = match page {
                Ok(Some(page)) => ServerMessage::Content {
                    subscription: id.clone(),
                    page,
                },
                Ok(None) => break,
                Err(error) => {
                    let _ = tx
                        .send(ServerMessage::Error {
                            id: Some(id.clone()),
                            error: ServerError::from(error).into_api(),
                        })
                        .await;
                    break;
                }
            };
            if tx.send(message).await.is_err() {
                return;
            }
        }
        let _ = tx
            .send(ServerMessage::Notification(
                agena_api::notifications::Notification::SubscriptionClosed {
                    subscription: id,
                    reason: "content subscription ended".into(),
                },
            ))
            .await;
    }))
}

pub(crate) struct LiveSubscription {
    subscription: agena_application::LiveSubscription,
    revisions: Arc<crate::revisions::ResourceRevisions>,
    state: AppState,
}
impl LiveSubscription {
    pub(crate) fn revisions_for(
        &self,
        item: &LiveItem,
    ) -> std::collections::BTreeMap<String, String> {
        match item {
            LiveItem::SessionChanged(change) => self.revisions.live_tokens(Some(change), None),
            LiveItem::RuntimeSignal(signal) => self.revisions.live_tokens(None, Some(signal)),
            LiveItem::Lagged(_) => Default::default(),
        }
    }
    pub(crate) async fn recv(&mut self) -> Option<LiveItem> {
        let item = self.subscription.recv().await?;
        // Revision tokens cover the global durable feed, including commits
        // omitted by this subscription's session or workspace filter.
        if self.revisions.refresh_durable(&self.state).await.is_err() {
            return Some(LiveItem::Lagged(1));
        }
        Some(item)
    }
}

pub(crate) fn subscribe(state: &AppState, scope: Scope) -> Result<LiveSubscription, ServerError> {
    Ok(LiveSubscription {
        subscription: state.application().watch_changes(scope)?,
        revisions: state.revisions()?,
        state: state.clone(),
    })
}

pub(crate) async fn session_parts(
    state: &AppState,
    session_id: i64,
) -> Result<SessionPartsResource, ServerError> {
    state
        .application()
        .session_parts(session_id)
        .await
        .map_err(ServerError::from)
}

pub(crate) async fn project_tool_detail(
    state: &AppState,
    part: &PartResource,
    section: ToolDetailSection,
    locale: Option<&str>,
) -> Option<ToolDetailResource> {
    state
        .application()
        .project_tool_detail(part, section, locale)
        .await
}
