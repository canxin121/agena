//! Transport/authority contracts. Product behavior is covered separately by
//! TestAgena's real HTTP plugin reload scenario.

use super::*;
use std::collections::HashSet;
use std::sync::{Mutex, atomic::AtomicUsize};

use crate::effect_scope::PluginEffectScope;
use crate::host::HostHandle;
use crate::sdk::drivers::http::{BINDING_HEADER, HttpCallbackFork, HttpCallbackRetire};
use crate::sdk::host_api::{HostCallbackContext, NoopHostClient};
use crate::sdk::{InitOutcome, PluginManifest};
use crate::transport::initialization::PluginInitialization;
use axum::{Json, Router, extract::State, http::HeaderMap, routing::post};

struct PeerState {
    bindings: Mutex<HashSet<String>>,
    shutdowns: AtomicUsize,
    calls: AtomicUsize,
    reject_fork: AtomicBool,
    deny_last: AtomicBool,
    hold_fork: AtomicBool,
    entered: tokio::sync::Notify,
    release: tokio::sync::Semaphore,
    retired: tokio::sync::Notify,
    stopped: tokio::sync::Notify,
}

impl Default for PeerState {
    fn default() -> Self {
        Self {
            bindings: Mutex::new(HashSet::new()),
            shutdowns: AtomicUsize::new(0),
            calls: AtomicUsize::new(0),
            reject_fork: AtomicBool::new(false),
            deny_last: AtomicBool::new(false),
            hold_fork: AtomicBool::new(false),
            entered: tokio::sync::Notify::new(),
            release: tokio::sync::Semaphore::new(0),
            retired: tokio::sync::Notify::new(),
            stopped: tokio::sync::Notify::new(),
        }
    }
}

fn manifest() -> PluginManifest {
    PluginManifest::new("test", "binding", "1.0.0")
}

async fn rpc(
    State(state): State<Arc<PeerState>>,
    headers: HeaderMap,
    Json(request): Json<Request>,
) -> Json<Response> {
    let binding = headers
        .get(BINDING_HEADER)
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    let params = request.params.unwrap_or_default();
    let result = match request.method.as_str() {
        method::META_HTTP_STATE => serde_json::json!({
            "version": crate::sdk::drivers::http::HTTP_INSTANCE_VERSION,
            "revision": null,
        }),
        method::META_INIT => {
            state.bindings.lock().unwrap().insert(binding.clone());
            serde_json::to_value(InitOutcome::ack(manifest())).unwrap()
        }
        method::META_HOST_FORK => {
            if state.reject_fork.load(Ordering::Acquire) {
                return Json(Response {
                    jsonrpc: JsonRpcVersion,
                    id: request.id,
                    payload: ResponsePayload::Err {
                        error: crate::sdk::rpc::ErrorObject {
                            code: crate::sdk::rpc::codes::METHOD_NOT_FOUND,
                            message: "fork unsupported".into(),
                            data: None,
                        },
                    },
                });
            }
            let fork: HttpCallbackFork = serde_json::from_value(params).unwrap();
            assert_eq!(fork.expected_id, binding);
            assert!(state.bindings.lock().unwrap().contains(&binding));
            if state.hold_fork.swap(false, Ordering::AcqRel) {
                state.entered.notify_one();
                state.release.acquire().await.unwrap().forget();
            }
            state.bindings.lock().unwrap().insert(fork.next_id);
            serde_json::json!({})
        }
        method::META_HOST_RETIRE => {
            let retired: HttpCallbackRetire = serde_json::from_value(params).unwrap();
            let mut bindings = state.bindings.lock().unwrap();
            let existed = bindings.remove(&retired.binding_id);
            assert!(existed || retired.abort_uncommitted);
            let last = existed && bindings.is_empty() && !state.deny_last.load(Ordering::Acquire);
            state.retired.notify_one();
            serde_json::json!({ "last": last, "instance_stopped": false,
                "fenced_uncommitted": !existed })
        }
        method::META_SHUTDOWN => {
            state.shutdowns.fetch_add(1, Ordering::AcqRel);
            state.bindings.lock().unwrap().clear();
            state.stopped.notify_one();
            serde_json::json!({})
        }
        "test/probe" => {
            assert!(state.bindings.lock().unwrap().contains(&binding));
            let context = request.context.unwrap();
            assert_eq!(context.http_binding_id.as_deref(), Some(binding.as_str()));
            assert!(context.authority_token.is_some());
            state.calls.fetch_add(1, Ordering::AcqRel);
            serde_json::json!({"binding": binding})
        }
        other => panic!("unexpected peer method {other}"),
    };
    Json(Response {
        jsonrpc: JsonRpcVersion,
        id: request.id,
        payload: ResponsePayload::Ok { result },
    })
}

struct Peer {
    state: Arc<PeerState>,
    server: Option<tokio::task::JoinHandle<()>>,
}

impl Peer {
    async fn start() -> (
        Self,
        Arc<HttpTransport>,
        Arc<HostHandle>,
        Arc<PluginEffectScope>,
    ) {
        let state = Arc::new(PeerState::default());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let router = Router::new()
            .route("/rpc", post(rpc))
            .with_state(state.clone());
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let transport = Arc::new(HttpTransport::new(
            format!("http://{address}/rpc").parse().unwrap(),
            crate::config::HttpAuth::None,
            &|_| None,
            false,
        ));
        let (host, scope) = owner();
        transport
            .bind_host(host.clone(), scope.clone())
            .await
            .unwrap();
        transport.initialize(initialization(&host)).await.unwrap();
        (
            Self {
                state,
                server: Some(server),
            },
            transport,
            host,
            scope,
        )
    }

    async fn stop(mut self) {
        let server = self.server.take().unwrap();
        server.abort();
        let _ = server.await;
    }
}

impl Drop for Peer {
    fn drop(&mut self) {
        if let Some(server) = &self.server {
            server.abort();
        }
    }
}

fn owner() -> (Arc<HostHandle>, Arc<PluginEffectScope>) {
    let host = Arc::new(HostHandle::new(Arc::new(NoopHostClient)));
    let scope = host.begin_plugin_instance("test.binding".parse().unwrap());
    (host, scope)
}

fn initialization(host: &Arc<HostHandle>) -> PluginInitialization {
    PluginInitialization {
        host: Arc::downgrade(host),
        plugin_id: "test.binding".parse().unwrap(),
        manifest: manifest(),
        agena_version: "test".into(),
        workspace_root: ".".into(),
        settings: serde_json::json!({}),
    }
}

async fn probe(
    host: &Arc<HostHandle>,
    transport: &dyn PluginTransport,
) -> Result<serde_json::Value, TransportError> {
    host.run_in_authorized_callback_context(
        &"test.binding".parse().unwrap(),
        HostCallbackContext {
            session_id: Some(41),
            call_id: Some(7),
            workspace_root: Some("/test/workspace".into()),
            ..Default::default()
        },
        transport.dispatch("test/probe", serde_json::json!({})),
    )
    .await
}

#[tokio::test]
async fn fork_keeps_each_authority_and_old_retirement_preserves_successor() {
    let (peer, first, old_host, old_scope) = Peer::start().await;
    let (new_host, new_scope) = owner();
    let second = first
        .fork_host(
            new_host.clone(),
            new_scope.clone(),
            old_scope.clone(),
            initialization(&new_host),
        )
        .await
        .unwrap()
        .unwrap();
    let old = probe(&old_host, first.as_ref()).await.unwrap();
    let new = probe(&new_host, second.as_ref()).await.unwrap();
    assert_ne!(old["binding"], new["binding"]);
    let before = peer.state.calls.load(Ordering::Acquire);
    let wrong = probe(&new_host, first.as_ref()).await.unwrap_err();
    let TransportError::Plugin(wrong) = wrong else {
        panic!("expected an authority failure");
    };
    assert_eq!(wrong.kind, crate::sdk::PluginErrorKind::PolicyDenied);
    assert!(wrong.diagnostic_message().contains("unknown or expired"));
    assert_eq!(peer.state.calls.load(Ordering::Acquire), before);
    first.shutdown().await.unwrap();
    assert!(!old_scope.is_accepting());
    assert!(new_scope.is_accepting());
    assert_eq!(peer.state.shutdowns.load(Ordering::Acquire), 0);
    assert_eq!(probe(&new_host, second.as_ref()).await.unwrap(), new);
    second.shutdown().await.unwrap();
    first.shutdown().await.unwrap();
    assert_eq!(peer.state.shutdowns.load(Ordering::Acquire), 1);
    peer.stop().await;
}

#[tokio::test]
async fn descendants_retire_in_reverse_order_with_one_physical_shutdown() {
    let (peer, first, first_host, first_scope) = Peer::start().await;
    let (second_host, second_scope) = owner();
    let second = first
        .fork_host(
            second_host.clone(),
            second_scope.clone(),
            first_scope,
            initialization(&second_host),
        )
        .await
        .unwrap()
        .unwrap();
    let (third_host, third_scope) = owner();
    let third = second
        .fork_host(
            third_host.clone(),
            third_scope,
            second_scope,
            initialization(&third_host),
        )
        .await
        .unwrap()
        .unwrap();
    third.shutdown().await.unwrap();
    second.shutdown().await.unwrap();
    assert_eq!(peer.state.shutdowns.load(Ordering::Acquire), 0);
    probe(&first_host, first.as_ref()).await.unwrap();
    first.shutdown().await.unwrap();
    third.shutdown().await.unwrap();
    assert_eq!(peer.state.shutdowns.load(Ordering::Acquire), 1);
    peer.stop().await;
}

#[tokio::test]
async fn unsupported_fork_preserves_the_predecessor_instead_of_destructive_fallback() {
    let (peer, first, host, scope) = Peer::start().await;
    peer.state.reject_fork.store(true, Ordering::Release);
    let (candidate, candidate_scope) = owner();
    let error = first
        .fork_host(
            candidate.clone(),
            candidate_scope.clone(),
            scope.clone(),
            initialization(&candidate),
        )
        .await
        .err()
        .expect("unsupported fork must fail");
    let TransportError::Plugin(error) = error else {
        panic!("expected unsupported fork error");
    };
    assert_eq!(error.kind, crate::sdk::PluginErrorKind::NotImplemented);
    assert!(error.diagnostic_message().contains("snapshot-preserving"));
    assert!(scope.is_accepting());
    assert!(!candidate_scope.is_accepting());
    probe(&host, first.as_ref()).await.unwrap();
    assert_eq!(peer.state.shutdowns.load(Ordering::Acquire), 0);
    first.shutdown().await.unwrap();
    peer.stop().await;
}

#[tokio::test]
async fn cancelled_fork_waiter_retires_the_committed_candidate() {
    let (peer, first, host, scope) = Peer::start().await;
    peer.state.hold_fork.store(true, Ordering::Release);
    let (candidate, candidate_scope) = owner();
    let candidate_scope_check = candidate_scope.clone();
    let fork = {
        let first = first.clone();
        tokio::spawn(async move {
            first
                .fork_host(
                    candidate.clone(),
                    candidate_scope,
                    scope,
                    initialization(&candidate),
                )
                .await
        })
    };
    tokio::time::timeout(Duration::from_secs(5), peer.state.entered.notified())
        .await
        .unwrap();
    fork.abort();
    let _ = fork.await;
    peer.state.release.add_permits(1);
    tokio::time::timeout(Duration::from_secs(5), peer.state.retired.notified())
        .await
        .unwrap();
    probe(&host, first.as_ref()).await.unwrap();
    first.shutdown().await.unwrap();
    assert!(!candidate_scope_check.is_accepting());
    assert_eq!(peer.state.shutdowns.load(Ordering::Acquire), 1);
    peer.stop().await;
}

#[tokio::test]
async fn a_peer_reporting_other_bindings_prevents_physical_shutdown() {
    let (peer, first, _host, scope) = Peer::start().await;
    let (candidate, candidate_scope) = owner();
    let second = first
        .fork_host(
            candidate.clone(),
            candidate_scope,
            scope,
            initialization(&candidate),
        )
        .await
        .unwrap()
        .unwrap();
    first.shutdown().await.unwrap();
    peer.state.deny_last.store(true, Ordering::Release);
    let error = second.shutdown().await.unwrap_err();
    assert!(error.to_string().contains("final live binding"));
    assert_eq!(peer.state.shutdowns.load(Ordering::Acquire), 0);
    peer.stop().await;
}

#[tokio::test]
async fn final_shutdown_awaits_failed_candidate_disposers_and_preserves_their_error() {
    let (peer, first, _host, scope) = Peer::start().await;
    peer.state.reject_fork.store(true, Ordering::Release);
    let (candidate, candidate_scope) = owner();
    let (entered, entered_rx) = tokio::sync::oneshot::channel();
    let (release, released) = tokio::sync::oneshot::channel();
    candidate_scope
        .own_async(
            "test.disposer",
            "gated candidate resource",
            move || async move {
                let _ = entered.send(());
                let _ = released.await;
                Err("candidate-disposer-failure-marker".to_owned())
            },
        )
        .unwrap();
    let fork = {
        let first = first.clone();
        tokio::spawn(async move {
            first
                .fork_host(
                    candidate.clone(),
                    candidate_scope,
                    scope,
                    initialization(&candidate),
                )
                .await
        })
    };
    tokio::time::timeout(Duration::from_secs(5), entered_rx)
        .await
        .unwrap()
        .unwrap();
    fork.abort();
    let _ = fork.await;
    let mut shutdown = {
        let first = first.clone();
        tokio::spawn(async move { first.shutdown().await })
    };
    tokio::time::timeout(Duration::from_secs(5), peer.state.stopped.notified())
        .await
        .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(50), &mut shutdown)
            .await
            .is_err(),
        "physical shutdown must still await the candidate's gated disposer"
    );
    release.send(()).unwrap();
    let error = tokio::time::timeout(Duration::from_secs(5), shutdown)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("candidate-disposer-failure-marker"),
        "{error}"
    );
    assert_eq!(
        first.shutdown().await.unwrap_err().to_string(),
        error.to_string()
    );
    assert_eq!(peer.state.shutdowns.load(Ordering::Acquire), 1);
    peer.stop().await;
}
