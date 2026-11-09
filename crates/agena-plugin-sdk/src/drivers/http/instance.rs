//! HTTP requests are fenced by their initialized instance. Lifecycle changes
//! serialize per owner, retire admitted work, and compare that owner’s revision.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};

use axum::http::HeaderMap;
use serde_json::Value;
use tokio::sync::OwnedRwLockReadGuard;
use tokio_util::sync::CancellationToken;

use super::*;
use crate::host_api::HostCallbackContext;

/// Caller-selected instance owner: 1–128 ASCII letters, digits, `-`, `_`, or `.`.
/// This fences lifecycle requests; endpoint authentication is still required.
pub const INSTANCE_HEADER: &str = "x-agena-plugin-instance";
/// Revision observed through `meta/http.state`, or `-` before any initialization.
pub const EXPECTED_REVISION_HEADER: &str = "x-agena-expected-revision";
/// HTTP lifecycle extension version, independent of the global RPC version.
pub const HTTP_INSTANCE_VERSION: u32 = 1;
const CONTROL_TIMEOUT: Duration = Duration::from_secs(30);
// A delayed predecessor may acknowledge retirement after a successor's init
// already shut it down. Keep bounded evidence of actual successful cleanup.
// Owner identities and their revisions are retained within this bound; once it
// is full, new identities fail closed instead of forgetting a prior revision.
const COMPLETED_SHUTDOWN_CAPACITY: usize = 256;

#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
/// State observed before attempting a compare-and-replace initialization.
pub struct HttpInstanceState {
    pub version: u32,
    pub revision: Option<String>,
}

pub(super) struct Instances<P: Plugin> {
    control: Mutex<HashMap<Option<String>, Weak<tokio::sync::Mutex<()>>>>,
    known_owners: Mutex<HashSet<String>>,
    current: Mutex<HashMap<Option<String>, Arc<Instance<P>>>>,
    completed_shutdowns: Mutex<VecDeque<CompletedShutdown>>,
}

struct CompletedShutdown {
    owner: String,
    bindings: Vec<String>,
    revision: String,
    no_binding: bool,
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
    pub(super) bindings: Arc<bindings::CallbackBindings>,
}

pub(super) struct Admission<P: Plugin> {
    pub(super) instance: Arc<Instance<P>>,
    pub(super) dispatcher: Arc<PluginDispatcher<P>>,
    pub(super) stop: CancellationToken,
    _operation: Arc<OwnedRwLockReadGuard<()>>,
    _binding: bindings::BindingLease,
}

impl<P: Plugin> Clone for Admission<P> {
    fn clone(&self) -> Self {
        Self {
            instance: self.instance.clone(),
            dispatcher: self.dispatcher.clone(),
            stop: self.stop.clone(),
            _operation: self._operation.clone(),
            _binding: self._binding.clone(),
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
            control: Mutex::new(HashMap::new()),
            known_owners: Mutex::new(HashSet::new()),
            current: Mutex::new(HashMap::new()),
            completed_shutdowns: Mutex::new(VecDeque::new()),
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

/// HTTP request IDs are only unique within the caller's instance. Include the
/// same owner header for ordinary and control requests so a stale owner cannot
/// cancel a successor's work, including initialization not yet admitted.
pub(super) fn request_id(headers: &HeaderMap, id: &RequestId) -> crate::Result<RequestId> {
    let owner = header(headers, INSTANCE_HEADER)?;
    let binding = bindings::header(headers)?;
    serde_json::to_string(&(owner, binding, id))
        .map(RequestId::Str)
        .map_err(PluginError::from)
}

impl<P: Plugin> Instances<P> {
    fn owner_control(&self, owner: Option<&str>) -> Arc<tokio::sync::Mutex<()>> {
        let mut controls = self
            .control
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        controls.retain(|_, lock| lock.strong_count() > 0);
        let key = owner.map(str::to_owned);
        if let Some(lock) = controls.get(&key).and_then(Weak::upgrade) {
            return lock;
        }
        let lock = Arc::new(tokio::sync::Mutex::new(()));
        controls.insert(key, Arc::downgrade(&lock));
        lock
    }

    fn shutdown_completed(&self, owner: Option<&str>, binding: Option<&str>) -> bool {
        let Some(owner) = owner else {
            // Legacy requests have no identity with which to prove retirement.
            return false;
        };
        self.completed_shutdowns
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .iter()
            .any(|completed| {
                completed.owner == owner
                    && match binding {
                        Some(id) => completed.bindings.iter().any(|old| old == id),
                        None => completed.no_binding,
                    }
            })
    }

    fn binding_shutdown_completed(&self, owner: Option<&str>, binding: &str) -> bool {
        let Some(owner) = owner else {
            return false;
        };
        self.completed_shutdowns
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .iter()
            .any(|completed| {
                completed.owner == owner && completed.bindings.iter().any(|id| id == binding)
            })
    }

    fn record_completed_shutdown(
        &self,
        owner: Option<&str>,
        revision: &str,
        mut bindings: Vec<String>,
        mut no_binding: bool,
    ) {
        let Some(owner) = owner else {
            return;
        };
        let mut completed = self
            .completed_shutdowns
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(previous) = completed.iter().find(|previous| previous.owner == owner) {
            no_binding |= previous.no_binding;
            bindings.extend(previous.bindings.iter().cloned());
            bindings.sort_unstable();
            bindings.dedup();
        }
        completed.retain(|previous| previous.owner != owner);
        completed.push_back(CompletedShutdown {
            owner: owner.to_owned(),
            bindings,
            revision: revision.to_owned(),
            no_binding,
        });
    }

    fn current(&self, owner: Option<&str>) -> Option<Arc<Instance<P>>> {
        self.current
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(&owner.map(str::to_owned))
            .cloned()
    }

    fn revision(&self, owner: Option<&str>) -> Option<String> {
        self.current(owner)
            .map(|instance| instance.revision.clone())
            .or_else(|| {
                let owner = owner?;
                self.completed_shutdowns
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .iter()
                    .find(|entry| entry.owner == owner)
                    .map(|entry| entry.revision.clone())
            })
    }

    fn owned(&self, headers: &HeaderMap) -> crate::Result<Arc<Instance<P>>> {
        let owner = header(headers, INSTANCE_HEADER)?;
        self.current(owner.as_deref()).ok_or_else(stopped)
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
        let binding = current
            .bindings
            .admit(bindings::header(headers)?.as_deref())
            .await?;
        Ok(Admission {
            stop: binding.stop.clone(),
            dispatcher: current.dispatcher()?,
            instance: current,
            _operation: Arc::new(guard),
            _binding: binding,
        })
    }
}

async fn cleanup<P: Plugin>(instances: &Instances<P>, current: &Instance<P>) -> crate::Result<()> {
    current.stop.cancel();
    let _quiet = current.operations.write().await;
    if current.needs_cleanup.load(Ordering::Acquire) {
        let dispatcher = current.dispatcher()?;
        bindings::shutdown_scope(crate::host_api::run_in_isolated_host_callback_context(
            HostCallbackContext::default(),
            dispatcher.dispatch(method::META_SHUTDOWN, serde_json::json!({})),
        ))
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
    // Record success only after the shutdown hook and object retirement finish.
    // CleanupGuard's retry uses this same path, so cancellation cannot invent a
    // receipt and a later successful retry can truthfully complete retirement.
    let no_binding = current.bindings.authorize_shutdown(None).is_ok();
    let bindings = current.bindings.clear_after_shutdown()?;
    instances.record_completed_shutdown(
        current.owner.as_deref(),
        &current.revision,
        bindings,
        no_binding,
    );
    let mut live = instances
        .current
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if live
        .get(&current.owner)
        .is_some_and(|entry| std::ptr::eq(entry.as_ref(), current))
    {
        live.remove(&current.owner);
    }
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
                let owner_control = state.instances.owner_control(instance.owner.as_deref());
                let _control = owner_control.lock().await;
                if !state
                    .instances
                    .current(instance.owner.as_deref())
                    .is_some_and(|current| Arc::ptr_eq(&current, &instance))
                {
                    return Ok(());
                }
                cleanup(&state.instances, &instance).await
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
        let owner = header(headers, INSTANCE_HEADER)?;
        return Ok(serde_json::to_value(HttpInstanceState {
            version: HTTP_INSTANCE_VERSION,
            revision: state.instances.revision(owner.as_deref()),
        })?);
    }
    // Independent from dispatch permits: control must be able to cancel all
    // admitted requests even when each ordinary slot is occupied by a stream.
    let owner = header(headers, INSTANCE_HEADER)?;
    let owner_control = state.instances.owner_control(owner.as_deref());
    let _control = owner_control.lock().await;
    if method_name == method::META_HOST_RETIRE {
        let retire: HttpCallbackRetire = serde_json::from_value(params)?;
        bindings::identifier(&retire.binding_id)?;
        if bindings::header(headers)?.as_deref() != Some(retire.binding_id.as_str()) {
            return Err(PluginError::invalid_params(
                "HTTP retirement requires its matching binding header",
            ));
        }
        let owner = header(headers, INSTANCE_HEADER)?;
        let current = state.instances.current(owner.as_deref());
        let still_owned = current.as_ref().is_some_and(|current| {
            current.owner == owner && current.needs_cleanup.load(Ordering::Acquire)
        });
        if (!still_owned
            || current.as_ref().is_some_and(|instance| {
                instance
                    .bindings
                    .authorize_shutdown(Some(&retire.binding_id))
                    .is_err()
            }))
            && state
                .instances
                .binding_shutdown_completed(owner.as_deref(), &retire.binding_id)
        {
            return Ok(serde_json::to_value(HttpCallbackRetired {
                last: false,
                instance_stopped: true,
                fenced_uncommitted: false,
            })?);
        }
        let current = state.instances.owned(headers)?;
        if current.stop.is_cancelled() || !current.ready.load(Ordering::Acquire) {
            return Err(stopped());
        }
        let retired = if retire.abort_uncommitted {
            current.bindings.abort_fork(&retire.binding_id).await?
        } else {
            current.bindings.retire(&retire.binding_id).await?
        };
        return Ok(serde_json::to_value(retired)?);
    }
    if method_name == method::META_SHUTDOWN {
        let owner = header(headers, INSTANCE_HEADER)?;
        // Always prefer the live owner, including when a caller reuses an old
        // owner ID. A receipt must never bypass a new instance's shutdown hook.
        let current = match state.instances.current(owner.as_deref()) {
            Some(current) if current.owner == owner => current,
            _ if state
                .instances
                .shutdown_completed(owner.as_deref(), bindings::header(headers)?.as_deref()) =>
            {
                return Ok(serde_json::json!({}));
            }
            _ => return Err(stopped()),
        };
        current
            .bindings
            .authorize_shutdown(bindings::header(headers)?.as_deref())?;
        let mut guard = CleanupGuard {
            state: state.clone(),
            instance: current.clone(),
            committed: false,
        };
        cleanup(&state.instances, &current).await?;
        guard.committed = true;
        return Ok(serde_json::json!({}));
    }
    if method_name == method::META_HOST_REBIND {
        let current = state.instances.owned(headers)?;
        if current.stop.is_cancelled() || !current.ready.load(Ordering::Acquire) {
            return Err(stopped());
        }
        current
            .bindings
            .authorize_shutdown(bindings::header(headers)?.as_deref())?;
        rebind_callback_client(&state, &current, params).await?;
        return Ok(serde_json::json!({}));
    }
    if method_name == method::META_HOST_FORK {
        let current = state.instances.owned(headers)?;
        if current.stop.is_cancelled() || !current.ready.load(Ordering::Acquire) {
            return Err(stopped());
        }
        let fork: HttpCallbackFork = serde_json::from_value(params)?;
        if bindings::header(headers)?.as_deref() != Some(fork.expected_id.as_str()) {
            return Err(PluginError::invalid_params(
                "HTTP fork requires its expected binding header",
            ));
        }
        fork_callback_client(&current, fork).await?;
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
    let current = state.instances.current(owner.as_deref());
    let revision = state.instances.revision(owner.as_deref());
    if let Some(id) = owner.as_ref() {
        let expected = expected.ok_or_else(|| {
            PluginError::invalid_params("HTTP initialization requires an expected revision")
        })?;
        if revision.as_deref().unwrap_or("-") != expected {
            return Err(PluginError::from_kind(
                PluginErrorKind::PolicyDenied,
                "HTTP plugin instance changed before initialization",
            ));
        }
        let completed = state
            .instances
            .completed_shutdowns
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        {
            let mut known = state
                .instances
                .known_owners
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if !known.contains(id) && known.len() >= COMPLETED_SHUTDOWN_CAPACITY {
                return Err(PluginError::from_kind(
                    PluginErrorKind::PolicyDenied,
                    "HTTP owner history capacity exhausted",
                ));
            }
            known.insert(id.clone());
        }
        if revision.is_some() && bindings::header(headers)?.is_none() {
            return Err(PluginError::invalid_params(
                "HTTP reinitialization requires a fresh callback binding",
            ));
        }
        if let Some(entry) = completed
            .iter()
            .find(|entry| Some(entry.owner.as_str()) == owner.as_deref())
        {
            let id = bindings::header(headers)?;
            if id.as_ref().is_some_and(|id| entry.bindings.contains(id))
                || entry.bindings.len() >= COMPLETED_SHUTDOWN_CAPACITY
            {
                return Err(PluginError::from_kind(
                    PluginErrorKind::PolicyDenied,
                    "HTTP callback binding was already retired or history is full",
                ));
            }
        }
    } else if expected.is_some() {
        return Err(stopped());
    }
    // Do not allow a predecessor's binding to name its successor.
    if let Some(previous) = &current
        && owner.is_some()
    {
        previous
            .bindings
            .reject_reused_binding(bindings::header(headers)?.as_deref())?;
    }
    let stop = CancellationToken::new();
    let bindings = Arc::new(bindings::CallbackBindings::new(
        bindings::header(headers)?,
        HttpCallbackBinding {
            url: ctx.host_callback_url.clone(),
            token: ctx.host_callback_token.clone(),
        },
        stop.clone(),
    )?);
    let callback = ctx
        .host_callback_url
        .as_ref()
        .map(|_| HttpCallbackHostClient::from_bindings(bindings.clone()).map(Arc::new))
        .transpose()?;
    // Construction and immutable-manifest validation precede predecessor retirement.
    let plugin = state.prepare_plugin()?;
    if let Some(current) = current {
        let mut retiring = CleanupGuard {
            state: state.clone(),
            instance: current.clone(),
            committed: false,
        };
        cleanup(&state.instances, &current).await?;
        retiring.committed = true;
    }
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
        bindings,
    });
    // The predecessor’s retirement evidence remains fenced across generations.
    state
        .instances
        .current
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .insert(instance.owner.clone(), instance.clone());
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    struct ShutdownProbe {
        calls: AtomicUsize,
        fail: AtomicBool,
        hold: AtomicBool,
        entered: tokio::sync::Semaphore,
        release: CancellationToken,
    }

    struct ProbePlugin(Arc<ShutdownProbe>);

    #[async_trait::async_trait]
    impl Plugin for ProbePlugin {
        fn manifest(&self) -> crate::PluginManifest {
            crate::PluginManifest::new("test", "http-retirement", "test")
        }

        async fn shutdown(&self) -> crate::Result<()> {
            self.0.calls.fetch_add(1, Ordering::SeqCst);
            self.0.entered.add_permits(1);
            if self.0.hold.load(Ordering::SeqCst) {
                self.0.release.cancelled().await;
            }
            if self.0.fail.load(Ordering::SeqCst) {
                Err(PluginError::internal("deliberate shutdown failure"))
            } else {
                Ok(())
            }
        }
    }

    type State = Arc<HttpDriverState<ProbePlugin>>;

    fn fixture() -> (State, Arc<ShutdownProbe>) {
        let probe = Arc::new(ShutdownProbe {
            calls: AtomicUsize::new(0),
            fail: AtomicBool::new(false),
            hold: AtomicBool::new(false),
            entered: tokio::sync::Semaphore::new(0),
            release: CancellationToken::new(),
        });
        let factory_probe = probe.clone();
        let state = Arc::new(HttpDriverState::new(
            move || ProbePlugin(factory_probe.clone()),
            Arc::new(crate::host_api::NoopHostClient),
        ));
        (state, probe)
    }

    fn headers(owner: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(INSTANCE_HEADER, owner.parse().unwrap());
        headers
    }

    async fn initialize(state: &State, owner: &str) -> crate::Result<Value> {
        initialize_binding(state, owner, None).await
    }

    async fn initialize_binding(
        state: &State,
        owner: &str,
        binding: Option<&str>,
    ) -> crate::Result<Value> {
        let observed = control(
            state.clone(),
            &headers(owner),
            method::META_HTTP_STATE,
            serde_json::json!({}),
            None,
        )
        .await?;
        let mut headers = headers(owner);
        if let Some(binding) = binding {
            headers.insert(bindings::BINDING_HEADER, binding.parse().unwrap());
        }
        headers.insert(
            EXPECTED_REVISION_HEADER,
            observed["revision"]
                .as_str()
                .unwrap_or("-")
                .parse()
                .unwrap(),
        );
        control(
            state.clone(),
            &headers,
            method::META_INIT,
            serde_json::json!({
                "agena_version": "test",
                "workspace_root": "/",
                "plugin_id": "test.http-retirement",
                "settings": {},
                "protocol_version": crate::rpc::PROTOCOL_VERSION,
            }),
            None,
        )
        .await
    }

    async fn shutdown(state: &State, owner: &str) -> crate::Result<Value> {
        control(
            state.clone(),
            &headers(owner),
            method::META_SHUTDOWN,
            serde_json::json!({}),
            None,
        )
        .await
    }

    #[tokio::test]
    async fn replaced_owner_shutdown_is_idempotent_without_stopping_successor() {
        let (state, probe) = fixture();
        initialize(&state, "first").await.unwrap();
        initialize(&state, "second").await.unwrap();
        assert_eq!(probe.calls.load(Ordering::SeqCst), 0);
        shutdown(&state, "first").await.unwrap();
        shutdown(&state, "first").await.unwrap();
        assert_eq!(probe.calls.load(Ordering::SeqCst), 1);
        assert!(state.instances.admit(&headers("first")).await.is_err());
        assert!(state.instances.admit(&headers("second")).await.is_ok());
        shutdown(&state, "second").await.unwrap();
        shutdown(&state, "second").await.unwrap();
        assert_eq!(probe.calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn unknown_or_malformed_owner_cannot_acknowledge_shutdown() {
        let (state, probe) = fixture();
        initialize(&state, "first").await.unwrap();
        initialize(&state, "second").await.unwrap();
        let mut duplicate = headers("first");
        duplicate.append(INSTANCE_HEADER, "first".parse().unwrap());
        for (request_headers, expected) in [
            (headers("unknown"), PluginErrorKind::HostUnavailable),
            (HeaderMap::new(), PluginErrorKind::HostUnavailable),
            (duplicate, PluginErrorKind::InvalidParams),
            (headers("first!"), PluginErrorKind::InvalidParams),
        ] {
            let error = control(
                state.clone(),
                &request_headers,
                method::META_SHUTDOWN,
                serde_json::json!({}),
                None,
            )
            .await
            .unwrap_err();
            assert_eq!(error.kind, expected);
        }
        assert_eq!(probe.calls.load(Ordering::SeqCst), 0);
        shutdown(&state, "first").await.unwrap();
        shutdown(&state, "second").await.unwrap();
    }

    #[tokio::test]
    async fn failed_cleanup_does_not_create_a_receipt_or_replace_current() {
        let (state, probe) = fixture();
        initialize(&state, "first").await.unwrap();
        initialize(&state, "second").await.unwrap();
        probe.fail.store(true, Ordering::SeqCst);
        let error = initialize_binding(&state, "first", Some("next"))
            .await
            .unwrap_err();
        assert_eq!(error.diagnostic_message(), "deliberate shutdown failure");
        assert!(!state.instances.shutdown_completed(Some("first"), None));
        assert!(state.instances.current(Some("first")).is_some());
        assert!(state.instances.admit(&headers("first")).await.is_err());
        assert!(state.instances.admit(&headers("second")).await.is_ok());
    }

    #[tokio::test]
    async fn reused_owner_must_run_its_new_shutdown_hook() {
        let (state, probe) = fixture();
        initialize(&state, "first").await.unwrap();
        shutdown(&state, "first").await.unwrap();
        initialize_binding(&state, "first", Some("next"))
            .await
            .unwrap();
        probe.fail.store(true, Ordering::SeqCst);
        let mut next = headers("first");
        next.insert(bindings::BINDING_HEADER, "next".parse().unwrap());
        let error = control(
            state.clone(),
            &next,
            method::META_SHUTDOWN,
            serde_json::json!({}),
            None,
        )
        .await
        .unwrap_err();
        assert_eq!(error.diagnostic_message(), "deliberate shutdown failure");
        assert!(probe.calls.load(Ordering::SeqCst) >= 2);
    }

    #[tokio::test]
    async fn cancelled_cleanup_only_records_success_after_its_retry_finishes() {
        let (state, probe) = fixture();
        initialize(&state, "first").await.unwrap();
        probe.hold.store(true, Ordering::SeqCst);
        let shutdown_state = state.clone();
        let caller = tokio::spawn(async move { shutdown(&shutdown_state, "first").await });
        let first = tokio::time::timeout(Duration::from_secs(2), probe.entered.acquire())
            .await
            .unwrap()
            .unwrap();
        first.forget();
        caller.abort();
        assert!(caller.await.unwrap_err().is_cancelled());
        let retry = tokio::time::timeout(Duration::from_secs(2), probe.entered.acquire())
            .await
            .unwrap()
            .unwrap();
        retry.forget();
        assert!(!state.instances.shutdown_completed(Some("first"), None));
        probe.release.cancel();
        // A same-owner retry waits for cleanup before accepting its receipt.
        shutdown(&state, "first").await.unwrap();
        assert!(state.instances.shutdown_completed(Some("first"), None));
        initialize(&state, "second").await.unwrap();
        assert_eq!(probe.calls.load(Ordering::SeqCst), 2);
        assert!(state.instances.admit(&headers("second")).await.is_ok());
        shutdown(&state, "second").await.unwrap();
    }

    #[tokio::test]
    async fn blocked_cleanup_does_not_block_another_owner_lifecycle() {
        let (state, probe) = fixture();
        initialize(&state, "global").await.unwrap();
        initialize(&state, "project").await.unwrap();
        probe.hold.store(true, Ordering::SeqCst);
        let global_state = state.clone();
        let global = tokio::spawn(async move { shutdown(&global_state, "global").await });
        tokio::time::timeout(Duration::from_secs(2), probe.entered.acquire())
            .await
            .unwrap()
            .unwrap()
            .forget();
        assert!(state.instances.admit(&headers("project")).await.is_ok());
        // An independently initialized owner must not wait on the stalled hook.
        tokio::time::timeout(Duration::from_secs(2), initialize(&state, "third"))
            .await
            .unwrap()
            .unwrap();
        probe.release.cancel();
        global.await.unwrap().unwrap();
        shutdown(&state, "project").await.unwrap();
        shutdown(&state, "third").await.unwrap();
    }

    #[tokio::test]
    async fn full_owner_history_fails_closed_without_stopping_current() {
        let (state, probe) = fixture();
        for index in 0..COMPLETED_SHUTDOWN_CAPACITY {
            let owner = format!("owner-{index}");
            initialize(&state, &owner).await.unwrap();
            shutdown(&state, &owner).await.unwrap();
        }
        assert_eq!(
            state.instances.completed_shutdowns.lock().unwrap().len(),
            COMPLETED_SHUTDOWN_CAPACITY
        );
        assert_eq!(
            initialize(&state, "unknown").await.unwrap_err().kind,
            PluginErrorKind::PolicyDenied
        );
        shutdown(&state, "owner-0").await.unwrap();
        assert_eq!(
            probe.calls.load(Ordering::SeqCst),
            COMPLETED_SHUTDOWN_CAPACITY
        );
    }
}
