//! Transport-independent, bounded projections of durable and runtime changes.
use crate::{Application, ApplicationError};
use agena_api::{
    Scope,
    live::{RuntimeSignalResource, SessionChangeResource, SessionPartsResource},
};
use agena_runtime::{RuntimeLiveSignal, RuntimeLiveSignalItem};
use agena_session_api::bridge::SessionApiBackendSlot;
use agena_storage::kernel::{KernelError, KernelWatch, WatchError};
use tokio::sync::mpsc;

#[derive(Debug, Clone)]
pub enum LiveItem {
    SessionChanged(SessionChangeResource),
    RuntimeSignal(RuntimeSignalResource),
    Lagged(u64),
}

/// A bounded subscription. Dropping it stops the source projection immediately.
pub struct LiveSubscription {
    rx: mpsc::Receiver<LiveItem>,
    projection_task: tokio::task::JoinHandle<()>,
}
impl LiveSubscription {
    pub async fn recv(&mut self) -> Option<LiveItem> {
        self.rx.recv().await
    }
}
impl Drop for LiveSubscription {
    fn drop(&mut self) {
        self.projection_task.abort();
    }
}

impl Application {
    pub fn watch_changes(&self, scope: Scope) -> Result<LiveSubscription, ApplicationError> {
        subscribe_with_queue_capacity(self, scope, 256)
    }

    pub async fn session_parts(
        &self,
        session_id: i64,
    ) -> Result<SessionPartsResource, ApplicationError> {
        self.read_runs(agena_api::queries::ReadRunsParams {
            session_id,
            limit: Some(8),
            part_limit: Some(32),
            ..Default::default()
        })
        .await
    }
}

// A subscription must not retain Application: Application also owns plugin
// polling subscriptions. Acquire the shared backend only for a bounded read.
async fn read_changed_parts(
    slot: &SessionApiBackendSlot,
    request: agena_api::queries::ReadPartsParams,
) -> Result<SessionPartsResource, ApplicationError> {
    let backend = slot
        .backend()
        .ok_or_else(|| ApplicationError::internal("The application has stopped."))?;
    let value = backend
        .query(agena_session_api::Query::ReadParts(request))
        .await
        .map_err(ApplicationError::from_failure)?;
    match serde_json::from_value(value).map_err(|error| ApplicationError::internal_error(&error))? {
        agena_api::queries::QueryResult::Parts(parts) => Ok(parts),
        _ => Err(ApplicationError::internal(
            "The parts query returned an unexpected result.",
        )),
    }
}

fn includes_session_change(
    scope: &Scope,
    session_id: i64,
    workspace_id: Option<i64>,
    include_in_lists: bool,
) -> bool {
    match scope {
        Scope::Session {
            session_id: requested,
        } => *requested == session_id,
        Scope::Workspace {
            workspace_id: requested,
        } => include_in_lists && workspace_id == Some(*requested),
        Scope::Global => include_in_lists && workspace_id.is_some(),
    }
}

fn belongs_to_workspace(workspace_id: i64, session_workspace_id: Option<i64>) -> bool {
    session_workspace_id == Some(workspace_id)
}

fn is_global_invalidation(signal: &RuntimeSignalResource) -> bool {
    // These runtime-wide signals carry no conversation content. A scoped
    // viewer still needs new tools and attention from other conversations.
    signal.session_id.is_none()
        && (signal.kind == "tool_registry_changed"
            || (signal.kind == "plugin"
                && signal
                    .payload
                    .get("kind")
                    .and_then(serde_json::Value::as_str)
                    == Some("attention.changed")))
}

async fn recover_watch(
    watch: &mut KernelWatch,
    replacement: impl FnOnce() -> Result<KernelWatch, KernelError>,
    tx: &mpsc::Sender<LiveItem>,
) -> bool {
    // Recovery reads may begin as soon as the reset reaches the transport.
    // Install their live tail first, including while delivery is backpressured.
    match replacement() {
        Ok(replacement) => *watch = replacement,
        Err(error) => {
            tracing::warn!(%error, "kernel change feed ended during recovery");
            return false;
        }
    }
    tx.send(LiveItem::Lagged(1)).await.is_ok()
}

fn subscribe_with_queue_capacity(
    application: &Application,
    scope: Scope,
    capacity: usize,
) -> Result<LiveSubscription, ApplicationError> {
    let kernel = application.session_kernel()?.clone();
    let mut watch = match scope {
        Scope::Session { session_id } => kernel.reader.watch(
            session_id
                .try_into()
                .map_err(|error| ApplicationError::bad_request_error(&error))?,
            None,
        ),
        _ => kernel.reader.watch_all(None),
    }
    .map_err(|error| ApplicationError::internal_error(&error))?;
    let signals = application.live_signal_service()?;
    let mut signals = signals.subscribe();
    let (tx, rx) = mpsc::channel(capacity);
    let backend = application.session_api_backend.clone();
    let projection_task = tokio::spawn(async move {
        loop {
            tokio::select! {
                _=tx.closed()=>break,
                notice=watch.recv()=>{
                    let change=match notice {Ok(change)=>change,Err(WatchError::Closed)=>break,
                        Err(WatchError::Reset{..})=>{
                            let replacement=||match scope {Scope::Session{session_id}=>kernel.reader.watch(
                                session_id.try_into().map_err(|error:agena_domain::session_kernel::InvalidKernelNumber|KernelError::Invalid(error.to_string()))?,None),
                                _=>kernel.reader.watch_all(None)};
                            if !recover_watch(&mut watch,replacement,&tx).await {break;}
                            continue;
                        }};
                    let id=change.session.get();
                    if !includes_session_change(&scope,id,change.workspace_id,change.include_in_lists){continue;}
                    let metadata=match kernel.reader.summary(change.session).await {
                        Ok(metadata)=>metadata,
                        Err(agena_storage::kernel::KernelError::NotFound(_))=>{
                            if tx.send(LiveItem::SessionChanged(SessionChangeResource::SessionDeleted{session_id:id,workspace_id:change.workspace_id})).await.is_err(){break;}
                            continue;
                        },Err(_)=>{if tx.send(LiveItem::Lagged(1)).await.is_err(){break;}continue;}
                    };
                    if !includes_session_change(&scope,id,metadata.workspace_id,change.include_in_lists){continue;}
                    if tx.send(LiveItem::SessionChanged(SessionChangeResource::SessionMetaUpdated {session_id:id,
                        version:metadata.metadata_version.get(),title:metadata.title.clone(),favorite:metadata.favorite,
                        pinned:metadata.pinned,updated_at_ms:metadata.updated_at_ms})).await.is_err(){break;}
                    // Ordinary byte checkpoints have their own content watch.
                    // Content commits on this feed are terminal checkpoints:
                    // their changed entries and snapshot requirements matter.
                    if change.requires_snapshot {
                        if tx.send(LiveItem::Lagged(1)).await.is_err(){break;}
                        continue;
                    }
                    if change.affected_entries.is_empty() { continue; }
                    let page=match read_changed_parts(&backend, agena_api::queries::ReadPartsParams {
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
                    let global_invalidation=is_global_invalidation(&signal);
                    let allowed=global_invalidation||match (&scope,signal.session_id) {
                        (Scope::Global,_)=>true,(Scope::Session{session_id},Some(id))=>*session_id==id,
                        (Scope::Workspace{workspace_id},Some(id))=>match id.try_into(){Ok(id)=>kernel.reader.metadata(id).await.is_ok_and(|m|belongs_to_workspace(*workspace_id,m.workspace_id)),Err(_)=>false},_=>false};
                    if allowed&&tx.send(LiveItem::RuntimeSignal(signal)).await.is_err(){break;}
                }
            }
        }
    });
    Ok(LiveSubscription {
        rx,
        projection_task,
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

#[cfg(test)]
mod tests;
