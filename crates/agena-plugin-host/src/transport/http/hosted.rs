//! HTTP host handoff updates a retained SDK callback client after cancelling
//! predecessor work. The compare-and-replace control RPC never repeats init.

use std::sync::{Arc, MutexGuard, Weak};

use super::*;
use crate::effect_scope::PluginEffectScope;
use crate::host::HostHandle;
use crate::sdk::PluginErrorKind;
use crate::sdk::drivers::http::{HttpCallbackBinding, HttpCallbackRebind};

#[derive(Clone)]
pub(super) struct HttpBinding {
    host: Weak<HostHandle>,
    scope: Arc<PluginEffectScope>,
    callback: HttpCallbackBinding,
    pub(super) initialized: bool,
}

struct HandoffGuard<'a> {
    transport: &'a HttpTransport,
    previous_host: Arc<HostHandle>,
    previous_scope: Arc<PluginEffectScope>,
    host: Arc<HostHandle>,
    scope: Arc<PluginEffectScope>,
    committed: bool,
}

impl Drop for HandoffGuard<'_> {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        self.transport
            .close_local("HTTP plugin host handoff did not complete");
        self.previous_scope.stop_admission();
        self.scope.stop_admission();
        for host in [&self.previous_host, &self.host] {
            host.status_registry().record_spawn_failure(
                self.scope.plugin_id(),
                "HTTP plugin host handoff did not complete",
            );
        }
        let previous = Arc::clone(&self.previous_host);
        let previous_scope = Arc::clone(&self.previous_scope);
        let host = Arc::clone(&self.host);
        let scope = Arc::clone(&self.scope);
        tokio::spawn(async move {
            previous
                .dispose_plugin_resources_for_scope(previous_scope.plugin_id(), &previous_scope)
                .await;
            host.dispose_plugin_resources_for_scope(scope.plugin_id(), &scope)
                .await;
        });
    }
}

async fn destination(
    host: &Arc<HostHandle>,
    scope: &Arc<PluginEffectScope>,
) -> Result<HttpCallbackBinding, TransportError> {
    if !host.is_current_effect_scope(scope.plugin_id(), scope) || !scope.is_accepting() {
        return Err(TransportError::disconnected(
            "HTTP callback destination scope is stopped or stale",
        ));
    }
    let plugin = scope.plugin_id().to_string();
    let url = host.callback_url(&plugin);
    let token = host.callback_token(&plugin).await;
    if !host.is_current_effect_scope(scope.plugin_id(), scope) || !scope.is_accepting() {
        return Err(TransportError::disconnected(
            "HTTP callback destination changed while issuing its credential",
        ));
    }
    if url.is_some() && token.is_none() {
        return Err(TransportError::disconnected(
            "HTTP callback destination could not issue a credential",
        ));
    }
    Ok(HttpCallbackBinding { url, token })
}

impl HttpTransport {
    pub(super) fn binding(&self) -> MutexGuard<'_, Option<HttpBinding>> {
        self.binding.lock().unwrap_or_else(|error| {
            tracing::error!(diagnostic = %error, "recovering a poisoned HTTP host binding");
            error.into_inner()
        })
    }

    pub(super) fn close_local(&self, message: &str) {
        self.shutdown.cancel();
        if let Some(binding) = self.binding().as_ref() {
            binding.scope.stop_admission();
        }
        self.streams.close(PluginError::internal(message));
    }

    pub(super) async fn bind_initial_host(
        &self,
        host: Arc<HostHandle>,
        scope: Arc<PluginEffectScope>,
    ) -> Result<(), TransportError> {
        let _handoff = self.handoff.lock().await;
        let callback = destination(&host, &scope).await?;
        let mut binding = self.binding();
        if binding.is_some()
            || self.shutdown.is_cancelled()
            || !host.is_current_effect_scope(scope.plugin_id(), &scope)
            || !scope.is_accepting()
        {
            return Err(TransportError::disconnected(
                "HTTP transport cannot install its initial host binding",
            ));
        }
        self.stream_callbacks
            .store(callback.url.is_some(), Ordering::Release);
        *binding = Some(HttpBinding {
            host: Arc::downgrade(&host),
            scope,
            callback,
            initialized: false,
        });
        Ok(())
    }

    pub(super) fn reusable_binding(&self, owner: &Arc<PluginEffectScope>) -> Option<HttpBinding> {
        let binding = self.binding().clone()?;
        (binding.initialized
            && !self.shutdown.is_cancelled()
            && owner.is_accepting()
            && Arc::ptr_eq(owner, &binding.scope)
            && binding
                .host
                .upgrade()
                .is_some_and(|host| host.is_current_effect_scope(owner.plugin_id(), owner)))
        .then_some(binding)
    }

    pub(super) async fn run_bound<T>(
        &self,
        request: &Request,
        future: impl std::future::Future<Output = Result<T, TransportError>>,
    ) -> Result<T, TransportError> {
        if matches!(
            request.method.as_str(),
            method::META_MANIFEST | method::META_PING | method::META_SHUTDOWN
        ) {
            return future.await;
        }
        let binding = self.binding().clone();
        let Some(binding) = binding else {
            return future.await;
        };
        let host = binding
            .host
            .upgrade()
            .ok_or_else(|| TransportError::disconnected("HTTP plugin host was dropped"))?;
        let _lease = host.lease_http_request(&binding.scope, request.context.clone())?;
        let stop = binding.scope.cancellation_token();
        tokio::select! {
            biased;
            _ = stop.cancelled() => Err(TransportError::disconnected("HTTP plugin owner stopped while a request was active")),
            result = future => result,
        }
    }

    async fn rebind_remote(
        &self,
        expected: HttpCallbackBinding,
        next: HttpCallbackBinding,
    ) -> Result<(), TransportError> {
        let request = Request {
            jsonrpc: JsonRpcVersion,
            id: RequestId::Num(self.next_request_id()),
            method: method::META_HOST_REBIND.into(),
            params: Some(serde_json::to_value(HttpCallbackRebind { expected, next })?),
            context: None,
        };
        let response = self.send(&request).await?;
        match response.payload {
            ResponsePayload::Ok { .. } => Ok(()),
            ResponsePayload::Err { error } => {
                if error.code == crate::sdk::rpc::codes::METHOD_NOT_FOUND {
                    return Err(PluginError::not_implemented(
                        "HTTP plugin does not support callback host handoff",
                    )
                    .into());
                }
                let error = crate::transport::plugin_error_from_rpc(
                    error,
                    "HTTP callback destination handoff failed",
                );
                Err(error.into())
            }
        }
    }

    pub(super) async fn rebind_host(
        &self,
        host: Arc<HostHandle>,
        scope: Arc<PluginEffectScope>,
        previous_scope: Arc<PluginEffectScope>,
    ) -> Result<bool, TransportError> {
        let _handoff = self.handoff.lock().await;
        let Some(previous) = self.reusable_binding(&previous_scope) else {
            return Err(TransportError::disconnected(
                "HTTP handoff belongs to a stopped or stale owner",
            ));
        };
        // Verify support and remote ownership before quiescing. An unchanged
        // plugin retains its running instance; failed handoff must not fall
        // through to an initialization that retires its live remote state.
        self.rebind_remote(previous.callback.clone(), previous.callback.clone())
            .await?;
        if self.reusable_binding(&previous_scope).is_none() {
            return Err(TransportError::disconnected(
                "HTTP handoff owner changed during preflight",
            ));
        }
        let previous_host = previous
            .host
            .upgrade()
            .ok_or_else(|| TransportError::disconnected("previous HTTP plugin host was dropped"))?;
        let callback = destination(&host, &scope).await?;
        let mut guard = HandoffGuard {
            transport: self,
            previous_host: previous_host.clone(),
            previous_scope: previous_scope.clone(),
            host: host.clone(),
            scope: scope.clone(),
            committed: false,
        };
        previous_scope.quiesce().await;
        self.streams.interrupt(PluginError::from_kind(
            PluginErrorKind::HostUnavailable,
            "HTTP stream interrupted by host handoff",
        ));
        let contributions = previous_host.process_contributions(&previous_scope);
        let disposed = previous_scope.dispose().await;
        if !disposed.errors.is_empty() {
            return Err(TransportError::Io(format!(
                "HTTP predecessor cleanup failed: {}",
                disposed.errors.join("; ")
            )));
        }
        host.import_process_contributions(scope.clone(), contributions)
            .await?;
        let stop = scope.cancellation_token();
        tokio::select! {
            biased;
            _ = stop.cancelled() => return Err(TransportError::disconnected("HTTP successor scope stopped during handoff")),
            result = self.rebind_remote(previous.callback, callback.clone()) => result?,
        };
        let mut binding = self.binding();
        if self.shutdown.is_cancelled()
            || !host.is_current_effect_scope(scope.plugin_id(), &scope)
            || !scope.is_accepting()
        {
            return Err(TransportError::disconnected(
                "HTTP successor stopped before host binding could commit",
            ));
        }
        self.stream_callbacks
            .store(callback.url.is_some(), Ordering::Release);
        *binding = Some(HttpBinding {
            host: Arc::downgrade(&host),
            scope,
            callback,
            initialized: true,
        });
        guard.committed = true;
        Ok(true)
    }
}
