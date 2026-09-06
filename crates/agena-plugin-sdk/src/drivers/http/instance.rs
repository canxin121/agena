//! HTTP requests are fenced by their initialized instance. Lifecycle changes
//! serialize, retire admitted work, and compare the revision observed before init.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use axum::http::HeaderMap;
use serde_json::Value;
use tokio::sync::OwnedRwLockReadGuard;
use tokio_util::sync::CancellationToken;

use super::*;
use crate::host_api::HostCallbackContext;

#[cfg(test)]
mod tests;

/// Caller-selected instance owner: 1–128 ASCII letters, digits, `-`, `_`, or `.`.
/// This fences lifecycle requests; endpoint authentication is still required.
pub const INSTANCE_HEADER: &str = "x-agena-plugin-instance";
/// Revision observed through `meta/http.state`, or `-` before any initialization.
pub const EXPECTED_REVISION_HEADER: &str = "x-agena-expected-revision";
/// HTTP lifecycle extension version, independent of the global RPC version.
pub const HTTP_INSTANCE_VERSION: u32 = 1;
const CONTROL_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
/// State observed before attempting a compare-and-replace initialization.
pub struct HttpInstanceState {
    pub version: u32,
    pub revision: Option<String>,
}

pub(super) struct Instances<P: Plugin> {
    control: tokio::sync::Mutex<()>,
    current: Mutex<Option<Arc<Instance<P>>>>,
}

pub(super) struct Instance<P: Plugin> {
    revision: String,
    owner: Option<String>,
    stop: CancellationToken,
    operations: Arc<RwLock<()>>,
    ready: AtomicBool,
    needs_cleanup: AtomicBool,
    dispatcher: Mutex<Option<Arc<PluginDispatcher<P>>>>,
    pub(super) host_proxy: Arc<host_proxy::HostClientProxy>,
    pub(super) callback_client: RwLock<Option<Arc<HttpCallbackHostClient>>>,
}

pub(super) struct Admission<P: Plugin> {
    pub(super) instance: Arc<Instance<P>>,
    pub(super) dispatcher: Arc<PluginDispatcher<P>>,
    pub(super) stop: CancellationToken,
    _operation: Arc<OwnedRwLockReadGuard<()>>,
}

impl<P: Plugin> Clone for Admission<P> {
    fn clone(&self) -> Self {
        Self {
            instance: self.instance.clone(),
            dispatcher: self.dispatcher.clone(),
            stop: self.stop.clone(),
            _operation: self._operation.clone(),
        }
    }
}

impl<P: Plugin> Instance<P> {
    fn dispatcher(&self) -> crate::Result<Arc<PluginDispatcher<P>>> {
        self.dispatcher
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
            .ok_or_else(stopped)
    }
}

impl<P: Plugin> Default for Instances<P> {
    fn default() -> Self {
        Self {
            control: tokio::sync::Mutex::new(()),
            current: Mutex::new(None),
        }
    }
}

impl<P: Plugin> Admission<P> {
    pub(super) async fn run<T>(
        &self,
        future: impl std::future::Future<Output = crate::Result<T>>,
    ) -> crate::Result<T> {
        tokio::select! {
            biased;
            _ = self.stop.cancelled() => Err(stopped()),
            result = future => result,
        }
    }
}

fn stopped() -> PluginError {
    PluginError::from_kind(
        PluginErrorKind::HostUnavailable,
        "HTTP plugin instance is stopped or belongs to another owner",
    )
}

fn header(headers: &HeaderMap, name: &str) -> crate::Result<Option<String>> {
    let Some(value) = headers.get(name) else {
        return Ok(None);
    };
    let value = value
        .to_str()
        .map_err(|error| PluginError::invalid_params_error(&error))?;
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
        || headers.get_all(name).iter().count() != 1
    {
        return Err(PluginError::invalid_params("invalid HTTP instance header"));
    }
    Ok(Some(value.into()))
}

impl<P: Plugin> Instances<P> {
    pub(super) fn current(&self) -> Option<Arc<Instance<P>>> {
        self.current
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    fn owned(&self, headers: &HeaderMap) -> crate::Result<Arc<Instance<P>>> {
        let owner = header(headers, INSTANCE_HEADER)?;
        let current = self.current().ok_or_else(stopped)?;
        if current.owner != owner {
            return Err(stopped());
        }
        Ok(current)
    }

    pub(super) async fn admit(&self, headers: &HeaderMap) -> crate::Result<Admission<P>> {
        let current = self.owned(headers)?;
        if !current.ready.load(Ordering::Acquire) {
            return Err(stopped());
        }
        let guard = tokio::select! {
            biased;
            _ = current.stop.cancelled() => return Err(stopped()),
            guard = current.operations.clone().read_owned() => guard,
        };
        if current.stop.is_cancelled() || !current.ready.load(Ordering::Acquire) {
            return Err(stopped());
        }
        Ok(Admission {
            stop: current.stop.clone(),
            dispatcher: current.dispatcher()?,
            instance: current,
            _operation: Arc::new(guard),
        })
    }
}

async fn cleanup<P: Plugin>(current: &Instance<P>) -> crate::Result<()> {
    current.stop.cancel();
    let _quiet = current.operations.write().await;
    if current.needs_cleanup.load(Ordering::Acquire) {
        let dispatcher = current.dispatcher()?;
        crate::host_api::run_in_isolated_host_callback_context(
            HostCallbackContext::default(),
            dispatcher.dispatch(method::META_SHUTDOWN, serde_json::json!({})),
        )
        .await?;
        current.needs_cleanup.store(false, Ordering::Release);
    }
    // Keep the owner/revision tombstone for fencing, but release the object's
    // fields as soon as shutdown completes, even if the router keeps serving.
    let retired = current
        .dispatcher
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .take();
    drop(retired);
    Ok(())
}

// A lifecycle future can be dropped by its caller or its deadline. Keep the
// instance stopped and give cleanup one bounded retry, serialized with any
// successor. A later initializer must still finish cleanup before taking over.
struct CleanupGuard<P: Plugin> {
    state: Arc<HttpDriverState<P>>,
    instance: Arc<Instance<P>>,
    committed: bool,
}

impl<P: Plugin> Drop for CleanupGuard<P> {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        self.instance.stop.cancel();
        let state = self.state.clone();
        let instance = self.instance.clone();
        tokio::spawn(async move {
            let result = tokio::time::timeout(CONTROL_TIMEOUT, async {
                let _control = state.instances.control.lock().await;
                if !state
                    .instances
                    .current()
                    .is_some_and(|current| Arc::ptr_eq(&current, &instance))
                {
                    return Ok(());
                }
                cleanup(&instance).await
            })
            .await;
            match result {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    tracing::error!(diagnostic = %error.diagnostic_message(), "HTTP instance cleanup failed after its lifecycle operation stopped")
                }
                Err(error) => {
                    tracing::error!(diagnostic = %error, "HTTP instance cleanup timed out after its lifecycle operation stopped")
                }
            }
        });
    }
}

pub(super) async fn control<P: Plugin>(
    state: Arc<HttpDriverState<P>>,
    headers: &HeaderMap,
    method_name: &str,
    params: Value,
    context: Option<HostCallbackContext>,
) -> crate::Result<Value> {
    tokio::time::timeout(
        CONTROL_TIMEOUT,
        control_inner(state, headers, method_name, params, context),
    )
    .await
    .map_err(|error| {
        PluginError::from_kind(
            PluginErrorKind::Timeout,
            agena_failure::diagnostic::format_error_chain_with_context(
                "HTTP instance lifecycle operation exceeded 30 seconds",
                &error,
            ),
        )
        .with_hook(method_name)
    })?
}

async fn control_inner<P: Plugin>(
    state: Arc<HttpDriverState<P>>,
    headers: &HeaderMap,
    method_name: &str,
    params: Value,
    context: Option<HostCallbackContext>,
) -> crate::Result<Value> {
    if method_name == method::META_HTTP_STATE {
        return Ok(serde_json::to_value(HttpInstanceState {
            version: HTTP_INSTANCE_VERSION,
            revision: state
                .instances
                .current()
                .map(|current| current.revision.clone()),
        })?);
    }
    // Independent from dispatch permits: control must be able to cancel all
    // admitted requests even when each ordinary slot is occupied by a stream.
    let _control = state.instances.control.lock().await;
    if method_name == method::META_SHUTDOWN {
        let current = state.instances.owned(headers)?;
        let mut guard = CleanupGuard {
            state: state.clone(),
            instance: current.clone(),
            committed: false,
        };
        cleanup(&current).await?;
        guard.committed = true;
        return Ok(serde_json::json!({}));
    }
    if method_name == method::META_HOST_REBIND {
        let current = state.instances.owned(headers)?;
        if current.stop.is_cancelled() || !current.ready.load(Ordering::Acquire) {
            return Err(stopped());
        }
        rebind_callback_client(&state, &current, params).await?;
        return Ok(serde_json::json!({}));
    }
    if !params.is_object() {
        return Err(PluginError::invalid_params(
            "HTTP initialization parameters must be an object",
        ));
    }
    let ctx: InitContext = serde_json::from_value(params.clone())?;
    if ctx.protocol_version != crate::rpc::PROTOCOL_VERSION {
        return Err(PluginError::invalid_params(
            "HTTP initialization protocol version is incompatible",
        ));
    }
    let owner = header(headers, INSTANCE_HEADER)?;
    let expected = header(headers, EXPECTED_REVISION_HEADER)?;
    let current = state.instances.current();
    if owner.is_some() {
        let expected = expected.ok_or_else(|| {
            PluginError::invalid_params("HTTP initialization requires an expected revision")
        })?;
        if current
            .as_ref()
            .map(|current| current.revision.as_str())
            .unwrap_or("-")
            != expected
        {
            return Err(PluginError::from_kind(
                PluginErrorKind::PolicyDenied,
                "HTTP plugin instance changed before initialization",
            ));
        }
        if current
            .as_ref()
            .is_some_and(|current| current.owner == owner)
        {
            return Err(PluginError::invalid_params(
                "HTTP plugin instance was already initialized",
            ));
        }
    } else if expected.is_some()
        || current
            .as_ref()
            .is_some_and(|current| current.owner.is_some())
    {
        return Err(stopped());
    }
    let callback = HttpCallbackHostClient::from_init_context(&ctx)?.map(Arc::new);
    // Construction and immutable-manifest validation precede predecessor retirement.
    let plugin = state.prepare_plugin()?;
    if let Some(current) = current {
        let mut retiring = CleanupGuard {
            state: state.clone(),
            instance: current.clone(),
            committed: false,
        };
        cleanup(&current).await?;
        retiring.committed = true;
    }
    let stop = CancellationToken::new();
    let target: Arc<dyn HostClient> = callback
        .clone()
        .map(|client| client as Arc<dyn HostClient>)
        .unwrap_or_else(|| state.fallback_host.clone());
    let proxy = Arc::new(host_proxy::HostClientProxy::with_cancellation(
        target,
        stop.clone(),
    ));
    let instance = Arc::new(Instance {
        revision: uuid::Uuid::new_v4().simple().to_string(),
        owner,
        stop,
        operations: Arc::new(RwLock::new(())),
        ready: AtomicBool::new(false),
        needs_cleanup: AtomicBool::new(false),
        dispatcher: Mutex::new(Some(Arc::new(PluginDispatcher::with_host(
            plugin,
            proxy.clone(),
        )))),
        host_proxy: proxy,
        callback_client: RwLock::new(callback),
    });
    *state
        .instances
        .current
        .lock()
        .unwrap_or_else(|error| error.into_inner()) = Some(instance.clone());
    let mut guard = CleanupGuard {
        state: state.clone(),
        instance: instance.clone(),
        committed: false,
    };
    let dispatcher = instance.dispatcher()?;
    instance.needs_cleanup.store(true, Ordering::Release);
    let outcome = crate::host_api::run_in_isolated_host_callback_context(
        context.unwrap_or_default(),
        dispatcher.dispatch(method::META_INIT, params),
    )
    .await?;
    let initialized: crate::InitOutcome = serde_json::from_value(outcome.clone())?;
    if initialized.protocol_version != crate::rpc::PROTOCOL_VERSION
        || initialized.manifest != state.manifest
    {
        return Err(PluginError::invalid_params(
            "HTTP plugin init returned an incompatible protocol or changed manifest",
        ));
    }
    instance.ready.store(true, Ordering::Release);
    guard.committed = true;
    Ok(outcome)
}
