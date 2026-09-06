//! HTTP callback credentials route to their issuing host generation, including
//! candidates not yet published as the runtime snapshot. Routes are weak and
//! shared only by one reload lineage. Admission rechecks token and exact scope
//! together; a lease and cancellation cover every subsequent await.

use std::sync::Weak;

use super::host_handle::recover_read;
use super::*;
use crate::effect_scope::PluginEffectLease;
use crate::sdk::rpc::{ErrorObject, JsonRpcVersion, Request, Response, ResponsePayload, codes};

#[cfg(test)]
mod tests;

/// Failure to admit an authenticated HTTP plugin callback.
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum PluginCallbackRpcError {
    #[error("invalid or missing plugin callback bearer token")]
    InvalidCallbackToken,
    #[error("plugin callback request is missing callback context")]
    MissingCallbackContext,
}

#[derive(Clone)]
struct CallbackRoute {
    host: Weak<HostHandle>,
    scope: Weak<PluginEffectScope>,
    plugin: PluginKey,
}

#[derive(Default)]
pub(super) struct CallbackRoutes {
    routes: Mutex<HashMap<String, CallbackRoute>>,
}

/// A runtime lineage's callback dispatcher. It can be served before the first
/// host is built; registered credentials retain neither hosts nor scopes.
#[derive(Clone, Default)]
pub struct PluginCallbackDispatcher {
    pub(super) routes: Arc<CallbackRoutes>,
}

impl CallbackRoutes {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, CallbackRoute>> {
        self.routes.lock().unwrap_or_else(|error| {
            tracing::error!(diagnostic = %error, "recovering a poisoned HTTP callback route registry");
            error.into_inner()
        })
    }

    pub(super) fn remove(&self, token: &str) {
        self.lock().remove(token);
    }

    fn matches(&self, token: &str, host: &HostHandle, scope: &PluginEffectScope) -> bool {
        self.lock().get(token).is_some_and(|route| {
            std::ptr::eq(route.host.as_ptr(), host) && std::ptr::eq(route.scope.as_ptr(), scope)
        })
    }

    async fn admit(
        &self,
        plugin: &str,
        token: Option<&str>,
    ) -> Result<AdmittedCallback, PluginCallbackRpcError> {
        let denied = PluginCallbackRpcError::InvalidCallbackToken;
        let token = token.ok_or_else(|| denied.clone())?;
        let route = self
            .lock()
            .get(token)
            .cloned()
            .ok_or_else(|| denied.clone())?;
        if route.plugin.to_string() != plugin {
            return Err(denied);
        }
        let host = route.host.upgrade().ok_or_else(|| denied.clone())?;
        let scope = route.scope.upgrade().ok_or_else(|| denied.clone())?;
        let lease = {
            let tokens = host.tokens.lock().await;
            let scopes = recover_read(&host.effect_scopes, "plugin effect scope registry");
            if tokens
                .get(&route.plugin)
                .is_none_or(|expected| expected != token)
                || scopes
                    .get(&route.plugin)
                    .is_none_or(|current| !Arc::ptr_eq(current, &scope))
            {
                return Err(denied);
            }
            scope.lease().map_err(|_| denied.clone())?
        };
        Ok(AdmittedCallback {
            host,
            scope,
            _lease: lease,
        })
    }
}

struct AdmittedCallback {
    host: Arc<HostHandle>,
    scope: Arc<PluginEffectScope>,
    _lease: PluginEffectLease,
}

impl HostHandle {
    pub(crate) fn lease_http_request(
        &self,
        scope: &Arc<PluginEffectScope>,
        context: Option<HostCallbackContext>,
    ) -> Result<PluginEffectLease, PluginError> {
        self.validated_callback_context(Some(scope.plugin_id().to_string()), context)?;
        let scopes = recover_read(&self.effect_scopes, "plugin effect scope registry");
        if scopes
            .get(scope.plugin_id())
            .is_none_or(|current| !Arc::ptr_eq(current, scope))
        {
            return Err(PluginError::from_kind(
                PluginErrorKind::HostUnavailable,
                "HTTP request belongs to a stale plugin scope",
            ));
        }
        scope.lease().map_err(|_| {
            PluginError::from_kind(
                PluginErrorKind::HostUnavailable,
                "HTTP plugin scope stopped accepting requests",
            )
        })
    }

    pub async fn callback_token(self: &Arc<Self>, plugin_id: &str) -> Option<String> {
        self.callback_base_url.as_ref()?;
        let plugin: PluginKey = plugin_id.parse().ok()?;
        let mut tokens = self.tokens.lock().await;
        let scopes = recover_read(&self.effect_scopes, "plugin effect scope registry");
        let scope = scopes.get(&plugin)?;
        let _lease = scope.lease().ok()?;
        if let Some(token) = tokens.get(&plugin)
            && self.callback_routes.matches(token, self, scope)
        {
            return Some(token.clone());
        }
        let token = format!("cb-{}", uuid::Uuid::new_v4().simple());
        if let Some(previous) = tokens.insert(plugin.clone(), token.clone()) {
            self.callback_routes.remove(&previous);
        }
        self.callback_routes.lock().insert(
            token.clone(),
            CallbackRoute {
                host: Arc::downgrade(self),
                scope: Arc::downgrade(scope),
                plugin,
            },
        );
        Some(token)
    }

    pub async fn validate_callback_token(&self, plugin_id: &str, token: Option<&str>) -> bool {
        let Some(token) = token else {
            return false;
        };
        let Ok(plugin) = plugin_id.parse::<PluginKey>() else {
            return false;
        };
        let tokens = self.tokens.lock().await;
        let scopes = recover_read(&self.effect_scopes, "plugin effect scope registry");
        tokens
            .get(&plugin)
            .is_some_and(|expected| expected == token)
            && scopes.get(&plugin).is_some_and(|scope| {
                scope.is_accepting() && self.callback_routes.matches(token, self, scope)
            })
    }

    /// Dispatch a callback through this runtime lineage's credential registry.
    /// The admitted credential, rather than the current published snapshot,
    /// determines the owning host and scope for the entire callback.
    pub async fn dispatch_callback_rpc(
        &self,
        plugin: &str,
        token: Option<&str>,
        request: Request,
    ) -> Result<Response, PluginCallbackRpcError> {
        self.callback_dispatcher()
            .dispatch(plugin, token, request)
            .await
    }

    pub fn callback_dispatcher(&self) -> PluginCallbackDispatcher {
        PluginCallbackDispatcher {
            routes: self.callback_routes.clone(),
        }
    }
}

impl PluginCallbackDispatcher {
    /// Authenticate and lease the exact issuing host and scope before dispatch.
    pub async fn dispatch(
        &self,
        plugin: &str,
        token: Option<&str>,
        request: Request,
    ) -> Result<Response, PluginCallbackRpcError> {
        let admitted = self.routes.admit(plugin, token).await?;
        let params = request.params.unwrap_or(serde_json::Value::Null);
        let context = params
            .get("context")
            .filter(|value| value.is_object())
            .and_then(|value| serde_json::from_value::<HostCallbackContext>(value.clone()).ok());
        if context.is_none() {
            return Err(PluginCallbackRpcError::MissingCallbackContext);
        }
        let stop = admitted.scope.cancellation_token();
        let result = admitted.host.run_in_callback_effect_scope(admitted.scope.clone(), async {
            tokio::select! {
                biased;
                _ = stop.cancelled() => Err(PluginError::from_kind(PluginErrorKind::HostUnavailable,
                    "plugin callback scope stopped while the request was active")),
                result = async {
                    if admitted.host.ingest_stream_event_for_plugin(plugin, &request.method, params.clone()).await? {
                        Ok(serde_json::json!({}))
                    } else {
                        admitted.host.handle_call_for_plugin(plugin, &request.method, params).await
                    }
                } => result,
            }
        }).await;
        Ok(Response {
            jsonrpc: JsonRpcVersion,
            id: request.id,
            payload: match result {
                Ok(result) => ResponsePayload::Ok { result },
                Err(error) => ResponsePayload::Err {
                    error: ErrorObject {
                        code: codes::PLUGIN_GENERIC,
                        message: error.to_string(),
                        data: error.rpc_error_data(),
                    },
                },
            },
        })
    }
}

impl Drop for HostHandle {
    fn drop(&mut self) {
        for token in self.tokens.get_mut().values() {
            self.callback_routes.remove(token);
        }
    }
}
