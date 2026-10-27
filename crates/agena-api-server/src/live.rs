//! Bounded kernel change delivery shared by WS, SSE and IPC.
use crate::{error::ServerError, state::AppState};
use agena_api::{
    Scope,
    live::{
        RuntimeSignalResource, SessionChangeResource, SessionPartsResource, ToolDetailResource,
        ToolDetailSection,
    },
    part::PartResource,
};
use agena_runtime::{RuntimeLiveSignal, RuntimeLiveSignalItem};
use agena_runtime_contracts::part_content::ToolCallContent;
use agena_storage::kernel::{ChangeKind, WatchError};
use std::sync::Arc;
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

#[derive(Debug, Clone)]
pub(crate) enum LiveItem {
    SessionChanged(SessionChangeResource),
    RuntimeSignal(RuntimeSignalResource),
    Lagged(u64),
}
pub(crate) struct LiveSubscription {
    rx: mpsc::Receiver<LiveItem>,
    projection_task: tokio::task::JoinHandle<()>,
    revisions: Arc<crate::revisions::ResourceRevisions>,
}
impl LiveSubscription {
    pub(crate) fn revisions_for(
        &self,
        item: &LiveItem,
    ) -> std::collections::BTreeMap<String, String> {
        match item {
            LiveItem::SessionChanged(change) => self.revisions.live_tokens(Some(change), None),
            LiveItem::RuntimeSignal(signal) => self.revisions.live_tokens(None, Some(signal)),
            _ => Default::default(),
        }
    }
    pub(crate) async fn recv(&mut self) -> Option<LiveItem> {
        self.rx.recv().await
    }
}
impl Drop for LiveSubscription {
    fn drop(&mut self) {
        self.projection_task.abort();
    }
}
pub(crate) fn subscribe(state: &AppState, scope: Scope) -> Result<LiveSubscription, ServerError> {
    subscribe_with_queue_capacity(state, scope, 256)
}

fn subscribe_with_queue_capacity(
    state: &AppState,
    scope: Scope,
    capacity: usize,
) -> Result<LiveSubscription, ServerError> {
    let kernel = state.application().session_kernel()?.clone();
    let mut watch = match scope {
        Scope::Session { session_id } => kernel.reader.watch(
            session_id
                .try_into()
                .map_err(|error| ServerError::bad_request_error(&error))?,
            None,
        ),
        _ => kernel.reader.watch_all(None),
    }
    .map_err(|error| ServerError::internal_error(&error))?;
    let revisions = state.revisions()?;
    let signals = state.live_signals()?;
    let mut signals = signals.subscribe();
    let (tx, rx) = mpsc::channel(capacity);
    let state = state.clone();
    let projection_revisions = revisions.clone();
    let projection_task = tokio::spawn(async move {
        loop {
            tokio::select! {
                _=tx.closed()=>break,
                notice=watch.recv()=>{
                    let change=match notice {Ok(change)=>change,Err(WatchError::Closed)=>break,
                        Err(WatchError::Reset{..})=>{
                            if tx.send(LiveItem::Lagged(1)).await.is_err(){break;}
                            let replacement=match scope {Scope::Session{session_id}=>kernel.reader.watch(match session_id.try_into(){Ok(id)=>id,Err(_)=>break},None),
                                _=>kernel.reader.watch_all(None)};
                            match replacement { Ok(replacement)=>watch=replacement,Err(error)=>{tracing::warn!(%error,"kernel change feed ended during recovery");break;} }
                            continue;
                        }};
                    // Session watches can omit other sessions' earlier commits.
                    // Only the global feed may advance the shared resource clock.
                    if projection_revisions.refresh_durable(&state).await.is_err() {
                        if tx.send(LiveItem::Lagged(1)).await.is_err(){break;}
                        continue;
                    }
                    let id=change.session.get();
                    if matches!(scope,Scope::Workspace{workspace_id} if workspace_id!=change.workspace_id){continue;}
                    if !change.include_in_lists && !matches!(scope,Scope::Session{..}) {continue;}
                    let metadata=match kernel.reader.summary(change.session).await {
                        Ok(metadata)=>metadata,
                        Err(agena_storage::kernel::KernelError::NotFound(_))=>{
                            if tx.send(LiveItem::SessionChanged(SessionChangeResource::SessionDeleted{session_id:id,workspace_id:change.workspace_id})).await.is_err(){break;}
                            continue;
                        },Err(_)=>{if tx.send(LiveItem::Lagged(1)).await.is_err(){break;}continue;}
                    };
                    if matches!(scope,Scope::Workspace{workspace_id} if workspace_id!=metadata.workspace_id){continue;}
                    if tx.send(LiveItem::SessionChanged(SessionChangeResource::SessionMetaUpdated {session_id:id,
                        version:metadata.metadata_version.get(),title:metadata.title.clone(),favorite:metadata.favorite,
                        pinned:metadata.pinned,updated_at_ms:metadata.updated_at_ms})).await.is_err(){break;}
                    if matches!(change.kind,ChangeKind::Content){continue;}
                    if change.requires_snapshot {
                        if tx.send(LiveItem::Lagged(1)).await.is_err(){break;}
                        continue;
                    }
                    if change.affected_entries.is_empty() { continue; }
                    let page=match state.application().read_parts(agena_api::queries::ReadPartsParams {
                        session_id:id,ids:change.affected_entries.iter().map(|entry|entry.get()).collect(),..Default::default()}).await {
                        Ok(page)=>page,Err(_)=>{if tx.send(LiveItem::Lagged(1)).await.is_err(){break;}continue;}
                    };
                    // Entry identities and revisions make repeats harmless.
                    // Tail content itself travels on its native source watch.
                    for part in page.parts {
                        if tx.send(LiveItem::SessionChanged(SessionChangeResource::PartUpdated{
                            session_id:id,part:Box::new(part)})).await.is_err(){return;}
                    }

                },
                signal=signals.recv()=>{
                    let signal=match signal {Some(RuntimeLiveSignalItem::Signal(signal))=>signal,
                        Some(RuntimeLiveSignalItem::Lagged(skipped))=>{if tx.send(LiveItem::Lagged(skipped)).await.is_err(){break;}continue;},None=>break};
                    let signal=project_signal(signal);
                    // Attention invalidates a runtime-wide plugin queue. A TUI
                    // viewing one session must also learn about another
                    // session's notification; this signal carries no transcript.
                    let global_attention=signal.session_id.is_none()&&signal.kind=="plugin"
                        &&signal.payload.get("kind").and_then(serde_json::Value::as_str)==Some("attention.changed");
                    let allowed=global_attention||match (&scope,signal.session_id) {
                        (Scope::Global,_)=>true,(Scope::Session{session_id},Some(id))=>*session_id==id,
                        (Scope::Workspace{workspace_id},Some(id))=>match id.try_into(){Ok(id)=>kernel.reader.metadata(id).await.is_ok_and(|m|m.workspace_id==*workspace_id),Err(_)=>false},_=>false};
                    if allowed&&tx.send(LiveItem::RuntimeSignal(signal)).await.is_err(){break;}
                }
            }
        }
    });
    Ok(LiveSubscription {
        rx,
        projection_task,
        revisions,
    })
}
pub(crate) async fn session_parts(
    state: &AppState,
    session_id: i64,
) -> Result<SessionPartsResource, ServerError> {
    state
        .application()
        .read_runs(agena_api::queries::ReadRunsParams {
            session_id,
            limit: Some(8),
            part_limit: Some(32),
            ..Default::default()
        })
        .await
        .map_err(ServerError::from)
}
pub(crate) async fn project_tool_detail(
    state: &AppState,
    part: &PartResource,
    section: ToolDetailSection,
    locale: Option<&str>,
) -> Option<ToolDetailResource> {
    if part.kind != ToolCallContent::kind() {
        return None;
    }
    let content = ToolCallContent::try_from(&part.content).ok()?;
    let value = match section {
        ToolDetailSection::Metadata => serde_json::to_value(content.metadata).ok()?,
        ToolDetailSection::Input => content.input,
        ToolDetailSection::Output => serde_json::to_value(content.output).ok()?,
        ToolDetailSection::Presentation => {
            let mut document = state.application().part_document(part).await;
            if let Some(document) = document.as_mut() {
                agena_application::part_locale::localize_part_document(document, locale);
            }
            serde_json::to_value(document).ok()?
        }
    };
    Some(ToolDetailResource {
        part_id: part.part_id,
        revision: part.revision,
        updated_at_ms: part.updated_at_ms,
        section,
        part_state: (section == ToolDetailSection::Output).then(|| part.state.clone()),
        value,
    })
}
fn live_signal_payload<T: serde::Serialize>(value: &T, context: &'static str) -> serde_json::Value {
    match serde_json::to_value(value) {
        Ok(payload) => payload,
        Err(error) => {
            tracing::error!(
                diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                    context,
                    &error,
                ),
                "runtime live signal payload could not be serialized"
            );
            serde_json::json!({
                "projection_error": "The runtime signal payload could not be encoded."
            })
        }
    }
}

fn project_signal(signal: RuntimeLiveSignal) -> RuntimeSignalResource {
    match signal {
        RuntimeLiveSignal::Activity(activity) => RuntimeSignalResource {
            kind: "activity".to_owned(),
            // Delegated work is displayed and controlled by its parent. Its
            // child transcript has a separate stream of part updates.
            session_id: activity
                .activity
                .parent_session_id
                .or(activity.activity.session_id),
            payload: live_signal_payload(
                &serde_json::json!({
                    "activity_id": activity.activity_id,
                    "reason": activity.reason,
                    "ts_ms": activity.ts_ms,
                    "activity": agena_api::resource::BackgroundActivityResource::from(&activity.activity),
                }),
                "serialize a runtime activity live signal payload",
            ),
        },
        RuntimeLiveSignal::Plugin {
            session_id,
            plugin_id,
            kind_label,
            payload,
        } => RuntimeSignalResource {
            kind: "plugin".to_owned(),
            session_id,
            payload: serde_json::json!({
                "plugin_id": plugin_id.to_string(),
                "kind": kind_label,
                "payload": payload,
            }),
        },
        RuntimeLiveSignal::ToolRegistryChanged(change) => RuntimeSignalResource {
            kind: "tool_registry_changed".to_owned(),
            session_id: None,
            payload: live_signal_payload(
                &*change,
                "serialize a runtime tool-registry live signal payload",
            ),
        },
    }
}
