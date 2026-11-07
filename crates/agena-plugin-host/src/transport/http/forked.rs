//! Snapshot bindings share one remote instance but retain independent callback
//! credentials, effect scopes, stream registries, and shutdown boundaries.

use std::collections::{HashMap, HashSet};

use super::*;
use crate::effect_scope::PluginEffectScope;
use crate::host::HostHandle;
use crate::sdk::drivers::http::{HttpCallbackFork, HttpCallbackRetire, HttpCallbackRetired};

pub(super) type FinishReceiver = tokio::sync::watch::Receiver<Option<Result<(), TransportError>>>;

struct PendingFork {
    transport: Arc<HttpTransport>,
    accepted: bool,
}

impl Drop for PendingFork {
    fn drop(&mut self) {
        if !self.accepted {
            // Close registration before the cancelled build can let shutdown
            // drain its predecessor. The owned worker still joins this cleanup.
            self.transport.begin_retire();
        }
    }
}

pub(super) struct PhysicalOwners {
    pub(super) active: HashSet<String>,
    closing: HashSet<String>,
    forked: bool,
    outcome: Option<Result<(), TransportError>>,
    precommit: HashMap<String, FinishReceiver>,
    cleanup_failure: Option<TransportError>,
}

impl PhysicalOwners {
    pub(super) fn initial(binding: &str) -> Self {
        Self {
            active: [binding.to_owned()].into_iter().collect(),
            closing: HashSet::new(),
            forked: false,
            outcome: None,
            precommit: HashMap::new(),
            cleanup_failure: None,
        }
    }
}

fn combine(
    first: Result<(), TransportError>,
    second: Result<(), TransportError>,
) -> Result<(), TransportError> {
    match (first, second) {
        (Ok(()), result) | (result, Ok(())) => result,
        (Err(first), Err(second)) => Err(TransportError::Io(format!(
            "{first}; HTTP instance cleanup also failed: {second}"
        ))),
    }
}

impl HttpTransport {
    fn complete_candidate_cleanup(
        &self,
        sender: &tokio::sync::watch::Sender<Option<Result<(), TransportError>>>,
        result: Result<(), TransportError>,
    ) {
        let mut owners = self
            .owners
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Err(error) = &result {
            owners.cleanup_failure.get_or_insert_with(|| error.clone());
        }
        sender.send_replace(Some(result));
        owners.precommit.remove(&self.binding_id);
    }

    async fn dispose_candidate(
        host: Option<&Arc<HostHandle>>,
        scope: &Arc<PluginEffectScope>,
    ) -> Result<(), TransportError> {
        if let Some(host) = host {
            host.dispose_plugin_resources_for_scope(scope.plugin_id(), scope)
                .await;
        }
        let report = scope.dispose().await;
        if report.errors.is_empty() {
            Ok(())
        } else {
            Err(TransportError::Io(format!(
                "HTTP candidate effect cleanup failed: {}",
                report.errors.join("; ")
            )))
        }
    }

    pub(super) fn abandon_candidate(&self) {
        let binding = self.binding().clone();
        let Some(binding) = binding else {
            self.begin_retire();
            return;
        };
        let (sender, receiver) = tokio::sync::watch::channel(None);
        let registered = {
            let mut owners = self
                .owners
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if owners.precommit.contains_key(&self.binding_id) {
                false
            } else {
                owners.precommit.insert(self.binding_id.clone(), receiver);
                true
            }
        };
        self.begin_retire();
        if !registered {
            return;
        }
        let owned = self.operation_handle();
        let host = binding.host.upgrade();
        tokio::spawn(async move {
            let transport = owned.retire().await;
            let effects = Self::dispose_candidate(host.as_ref(), &binding.scope).await;
            owned.complete_candidate_cleanup(&sender, combine(transport, effects));
        });
    }

    /// An internal handle does not itself own another logical registration.
    fn operation_handle(&self) -> Self {
        Self {
            client: self.client.clone(),
            url: self.url.clone(),
            auth_header: self.auth_header.clone(),
            stream_callbacks: self.stream_callbacks.clone(),
            binding: self.binding.clone(),
            binding_id: self.binding_id.clone(),
            handoff: self.handoff.clone(),
            owners: self.owners.clone(),
            finish: self.finish.clone(),
            retire_on_drop: false,
            next_id: self.next_id.clone(),
            streams: self.streams.clone(),
            shutdown: self.shutdown.clone(),
            local_shutdown: self.local_shutdown.clone(),
            instance_id: self.instance_id.clone(),
            expected_revision: self.expected_revision.clone(),
        }
    }

    async fn control(
        &self,
        name: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, TransportError> {
        let response = self
            .send(Request {
                jsonrpc: JsonRpcVersion,
                id: RequestId::Num(self.next_request_id()),
                method: name.to_owned(),
                params: Some(params),
                context: None,
            })
            .await?;
        match response.payload {
            ResponsePayload::Ok { result } => Ok(result),
            ResponsePayload::Err { error } => {
                if name == method::META_HOST_FORK
                    && error.code == crate::sdk::rpc::codes::METHOD_NOT_FOUND
                {
                    return Err(PluginError::not_implemented(
                        "HTTP plugin does not support snapshot-preserving host binding forks",
                    )
                    .into());
                }
                Err(
                    super::super::plugin_error_from_rpc(error, "HTTP binding control failed")
                        .into(),
                )
            }
        }
    }

    pub(super) async fn fork_binding(
        &self,
        host: Arc<HostHandle>,
        scope: Arc<PluginEffectScope>,
        previous_scope: Arc<PluginEffectScope>,
    ) -> Result<Arc<dyn PluginTransport>, TransportError> {
        let previous = self.reusable_binding(&previous_scope).ok_or_else(|| {
            TransportError::disconnected("HTTP fork belongs to a stopped or stale binding")
        })?;
        let previous_host = previous
            .host
            .upgrade()
            .ok_or_else(|| TransportError::disconnected("previous HTTP plugin host was dropped"))?;
        let callback = super::hosted::destination(&host, &scope).await?;
        let mut next = self.operation_handle();
        next.binding_id = uuid::Uuid::new_v4().simple().to_string();
        next.binding = Arc::new(std::sync::Mutex::new(Some(super::hosted::HttpBinding {
            host: Arc::downgrade(&host),
            scope: scope.clone(),
            callback: callback.clone(),
            initialized: true,
        })));
        next.stream_callbacks = Arc::new(AtomicBool::new(callback.url.is_some()));
        next.local_shutdown = CancellationToken::new();
        next.streams = Arc::new(StreamRegistry::default());
        next.finish = Arc::new(std::sync::Mutex::new(None));
        next.retire_on_drop = true;
        let next = Arc::new(next);
        let (cleanup_sender, cleanup_receiver) = tokio::sync::watch::channel(None);
        {
            let mut owners = self
                .owners
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if owners.outcome.is_some()
                || owners.closing.contains(&self.binding_id)
                || !owners.active.contains(&self.binding_id)
            {
                return Err(TransportError::disconnected("HTTP predecessor is retiring"));
            }
            owners
                .precommit
                .insert(next.binding_id.clone(), cleanup_receiver);
        }
        let cleanup_owner = next.clone();
        let accepted_cleanup = cleanup_sender.clone();
        let mut pending = PendingFork {
            transport: next.clone(),
            accepted: false,
        };
        let parent = self.operation_handle();
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let (accepted, acknowledgement) = tokio::sync::oneshot::channel();
        // Retain the candidate host until its caller actually receives this
        // fork. Every cancellation/error also awaits its imported disposers.
        tokio::spawn(async move {
            use futures_util::FutureExt;
            let control = parent.handoff.lock().await;
            let mut compensation_failure = None;
            let result = std::panic::AssertUnwindSafe(async {
                if sender.is_closed() {
                    return Err(TransportError::Cancelled);
                }
                if parent.reusable_binding(&previous_scope).is_none() {
                    return Err(TransportError::disconnected(
                        "HTTP predecessor retired before its successor fork",
                    ));
                }
                host.import_process_contributions(
                    scope.clone(),
                    previous_host.process_contributions(&previous_scope),
                )
                .await?;
                let params = serde_json::to_value(HttpCallbackFork {
                    expected_id: parent.binding_id.clone(),
                    expected: previous.callback,
                    next_id: next.binding_id.clone(),
                    next: callback,
                })?;
                // Caller cancellation does not drop this RPC. A transport
                // failure still needs a fence because its response can be lost.
                if let Err(error) = parent.control(method::META_HOST_FORK, params).await {
                    if matches!(&error, TransportError::Plugin(_)) {
                        // A completed protocol rejection is atomic. In
                        // particular, old SDKs need no unsupported retire RPC.
                        return Err(error);
                    }
                    let rollback = next
                        .control(
                            method::META_HOST_RETIRE,
                            serde_json::to_value(HttpCallbackRetire {
                                binding_id: next.binding_id.clone(),
                                abort_uncommitted: true,
                            })?,
                        )
                        .await
                        .and_then(|value| {
                            serde_json::from_value::<HttpCallbackRetired>(value).map_err(Into::into)
                        });
                    if let Err(rollback) = rollback {
                        compensation_failure = Some(TransportError::Io(format!(
                            "HTTP fork rollback was not confirmed: {rollback}"
                        )));
                    }
                    return Err(error);
                }
                {
                    let mut owners = parent
                        .owners
                        .lock()
                        .unwrap_or_else(|error| error.into_inner());
                    owners.active.insert(next.binding_id.clone());
                    owners.forked = true;
                }
                Ok(())
            })
            .catch_unwind()
            .await
            .unwrap_or_else(|_| Err(TransportError::Io("HTTP fork operation panicked".into())));
            match result {
                Ok(()) => {
                    drop(control);
                    let delivered = sender
                        .send(Ok(next.clone() as Arc<dyn PluginTransport>))
                        .is_ok();
                    if delivered
                        && matches!(
                            tokio::time::timeout(Duration::from_secs(30), acknowledgement).await,
                            Ok(Ok(()))
                        )
                    {
                        // No await separates the caller's acknowledgement from
                        // PluginHost registering its ReusedBuildGuard entry.
                        return;
                    }
                    next.begin_retire();
                    let transport = next.retire().await;
                    let effects = Self::dispose_candidate(Some(&host), &scope).await;
                    next.complete_candidate_cleanup(&cleanup_sender, combine(transport, effects));
                }
                Err(error) => {
                    next.begin_retire();
                    drop(control);
                    let cleanup = next.retire().await;
                    let effects = Self::dispose_candidate(Some(&host), &scope).await;
                    let cleanup = combine(
                        combine(cleanup, effects),
                        compensation_failure.map_or(Ok(()), Err),
                    );
                    next.complete_candidate_cleanup(&cleanup_sender, cleanup.clone());
                    let result = combine(Err(error), cleanup);
                    let _ = sender.send(result.map(|()| next.clone() as Arc<dyn PluginTransport>));
                }
            }
        });
        let result = receiver.await.map_err(|error| {
            TransportError::Io(format!("HTTP fork owner ended without a result: {error}"))
        })?;
        if result.is_ok() {
            accepted.send(()).map_err(|()| {
                TransportError::disconnected(
                    "HTTP fork was not accepted before its ownership deadline",
                )
            })?;
            cleanup_owner.complete_candidate_cleanup(&accepted_cleanup, Ok(()));
            pending.accepted = true;
        }
        result
    }

    pub(super) fn begin_retire(&self) -> FinishReceiver {
        let mut cached = self
            .finish
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(receiver) = &*cached {
            return receiver.clone();
        }
        let (sender, receiver) = tokio::sync::watch::channel(None);
        *cached = Some(receiver.clone());
        let precommit = {
            let mut owners = self
                .owners
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            owners.closing.insert(self.binding_id.clone());
            owners.precommit.contains_key(&self.binding_id)
        };
        let owned = self.operation_handle();
        match tokio::runtime::Handle::try_current() {
            Ok(runtime) => {
                runtime.spawn(async move {
                    use futures_util::FutureExt;
                    let result = std::panic::AssertUnwindSafe(owned.finish_binding(precommit))
                        .catch_unwind()
                        .await
                        .unwrap_or_else(|_| {
                            Err(TransportError::Io("HTTP binding cleanup panicked".into()))
                        });
                    sender.send_replace(Some(result));
                });
            }
            Err(error) => {
                owned.close_local("HTTP binding dropped without an executor");
                sender.send_replace(Some(Err(TransportError::Io(format!(
                    "HTTP binding cleanup requires a Tokio executor: {error}"
                )))));
            }
        }
        receiver
    }

    pub(super) async fn retire(&self) -> Result<(), TransportError> {
        let mut receiver = self.begin_retire();
        loop {
            let result = receiver.borrow_and_update().clone();
            if let Some(result) = result {
                return result;
            }
            receiver.changed().await.map_err(|error| {
                TransportError::Io(format!("HTTP binding cleanup owner ended: {error}"))
            })?;
        }
    }

    async fn await_candidate_cleanup(&self) -> Result<(), TransportError> {
        loop {
            let pending = {
                let owners = self
                    .owners
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                if owners.precommit.is_empty() {
                    return owners.cleanup_failure.clone().map_or(Ok(()), Err);
                }
                owners
                    .precommit
                    .iter()
                    .map(|(id, receiver)| (id.clone(), receiver.clone()))
                    .collect::<Vec<_>>()
            };
            for (id, mut receiver) in pending {
                loop {
                    if receiver.borrow_and_update().is_some() {
                        break;
                    }
                    if let Err(error) = receiver.changed().await {
                        let mut owners = self
                            .owners
                            .lock()
                            .unwrap_or_else(|error| error.into_inner());
                        owners.cleanup_failure.get_or_insert_with(|| {
                            TransportError::Io(format!(
                                "HTTP candidate cleanup ended without a receipt: {error}"
                            ))
                        });
                        owners.precommit.remove(&id);
                        break;
                    }
                }
            }
        }
    }

    async fn finish_binding(&self, precommit: bool) -> Result<(), TransportError> {
        let control = self.handoff.lock().await;
        let (present, last, forked, cached) = {
            let owners = self
                .owners
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            (
                owners.active.contains(&self.binding_id),
                owners.active.len() == 1,
                owners.forked,
                owners.outcome.clone(),
            )
        };
        if !present {
            self.owners
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .closing
                .remove(&self.binding_id);
            self.close_local("HTTP binding has retired");
            return cached.unwrap_or(Ok(()));
        }
        let mut result = Ok(());
        let mut shutdown_authorized = !forked;
        if forked {
            let retired = self
                .control(
                    method::META_HOST_RETIRE,
                    serde_json::to_value(HttpCallbackRetire {
                        binding_id: self.binding_id.clone(),
                        abort_uncommitted: false,
                    })?,
                )
                .await
                .and_then(|value| {
                    serde_json::from_value::<HttpCallbackRetired>(value).map_err(Into::into)
                });
            result = match retired {
                Ok(retired)
                    if !retired.fenced_uncommitted
                        && (retired.instance_stopped || retired.last == last) =>
                {
                    shutdown_authorized = retired.instance_stopped || retired.last;
                    Ok(())
                }
                Ok(_) => Err(TransportError::Rpc(
                    "HTTP peer disagrees about the final live binding".into(),
                )),
                Err(error) => Err(error),
            };
        }
        if last && shutdown_authorized {
            // Keep the final effect scope and callback credential alive while
            // the actual plugin shutdown hook runs. The host disposes them only
            // after this transport boundary has completed.
            result = combine(
                result,
                self.control(method::META_SHUTDOWN, serde_json::json!({}))
                    .await
                    .map(|_| ()),
            );
        }
        {
            let mut owners = self
                .owners
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            owners.active.remove(&self.binding_id);
            owners.closing.remove(&self.binding_id);
            if last {
                owners.outcome = Some(result.clone());
            }
        }
        self.close_local("HTTP binding retired");
        if last {
            self.shutdown.cancel();
        }
        drop(control);
        let all_remaining_closing = {
            let owners = self
                .owners
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            !owners.active.is_empty() && owners.active.is_subset(&owners.closing)
        };
        if all_remaining_closing {
            // In particular, a cancelled candidate must not outlive final
            // shutdown merely because its Drop cleanup was scheduled later.
            self.shutdown.cancelled().await;
            let final_result = self
                .owners
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .outcome
                .clone();
            if let Some(final_result) = final_result {
                result = combine(result, final_result);
            }
        }
        if !precommit && self.shutdown.is_cancelled() {
            // A candidate's worker awaits its own transport retirement before
            // disposing effects. Only a published owner joins these receipts;
            // candidates must never wait for each other's cleanup workers.
            result = combine(result, self.await_candidate_cleanup().await);
        }
        result
    }
}
