//! Open-ended local HTTP fixtures for plugins, other model protocols, OAuth,
//! MCP, and application-specific APIs. These services stay alive until the
//! product server has drained, so shutdown callbacks still use real transport.

use std::{
    net::{Shutdown, SocketAddr},
    pin::Pin,
    task::{Context as TaskContext, Poll},
    time::Duration,
};

use anyhow::{Context, Result, anyhow};
use axum::{Router, body::Body};
use hyper::{server::conn::http1, service::service_fn};
use hyper_util::rt::TokioIo;
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::{TcpListener, TcpStream},
    sync::{oneshot, watch},
    task::JoinSet,
};
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;

use crate::case::TestCase;

type FailureReporter = std::sync::Arc<dyn Fn(String) + Send + Sync>;

/// Axum WebSocket upgrade whose handshake and protocol worker belong to the
/// serving HttpFixture. Use this extractor inside TestCase::serve_http routes.
///
/// The callback returns Result<()> and receives this fixture's cancellation
/// token. Its errors and panics fail the case; fixture shutdown closes sockets,
/// cancels the callback and joins it before returning.
pub struct FixtureWebSocketUpgrade {
    upgrade: axum::extract::WebSocketUpgrade,
    owner: UpgradeOwner,
}

impl<S: Send + Sync> axum::extract::FromRequestParts<S> for FixtureWebSocketUpgrade {
    type Rejection = axum::response::Response;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        state: &S,
    ) -> std::result::Result<Self, Self::Rejection> {
        use axum::response::IntoResponse;
        let owner = parts
            .extensions
            .get::<UpgradeOwner>()
            .cloned()
            .ok_or_else(|| {
                (
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    "FixtureWebSocketUpgrade requires a router served by TestCase::serve_http",
                )
                    .into_response()
            })?;
        let upgrade = axum::extract::WebSocketUpgrade::from_request_parts(parts, state)
            .await
            .map_err(IntoResponse::into_response)?;
        Ok(Self { upgrade, owner })
    }
}

impl FixtureWebSocketUpgrade {
    /// Configure the underlying Axum handshake, protocols and frame limits.
    pub fn configure(
        mut self,
        configure: impl FnOnce(axum::extract::WebSocketUpgrade) -> axum::extract::WebSocketUpgrade,
    ) -> Self {
        self.upgrade = configure(self.upgrade);
        self
    }

    /// Register the owned worker before returning the HTTP upgrade response.
    /// Axum's short detached callback only transfers its socket to that worker.
    pub fn on_upgrade<F, Fut>(self, handler: F) -> axum::response::Response
    where
        F: FnOnce(axum::extract::ws::WebSocket, CancellationToken) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Result<()>> + Send + 'static,
    {
        use axum::response::IntoResponse;
        let (transferred, received) = oneshot::channel();
        let cancel = self.owner.inner.stop.child_token();
        let future = async move {
            let socket = tokio::select! {
                biased;
                () = cancel.cancelled() => return Ok(()),
                socket = received => socket.context("managed WebSocket handshake did not complete")?,
            };
            // Calling handler inside the observed future also catches a panic
            // while constructing its future, before its first poll.
            handler(socket, cancel).await
        };
        if !self.owner.spawn(future) {
            return (
                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                "HTTP fixture is shutting down",
            )
                .into_response();
        }
        self.upgrade.on_upgrade(move |socket| async move {
            // A cancelled worker drops the transferred socket immediately.
            let _ = transferred.send(socket);
        })
    }
}

#[derive(Clone)]
struct UpgradeOwner {
    inner: std::sync::Arc<UpgradeInner>,
}

struct UpgradeInner {
    tasks: std::sync::Mutex<UpgradeTasks>,
    changed: tokio::sync::Notify,
    stop: CancellationToken,
    report: Option<FailureReporter>,
}

#[derive(Default)]
struct UpgradeTasks {
    closed: bool,
    pending: JoinSet<Result<()>>,
}

impl UpgradeOwner {
    fn new(stop: CancellationToken, report: Option<FailureReporter>) -> Self {
        Self {
            inner: std::sync::Arc::new(UpgradeInner {
                tasks: std::sync::Mutex::new(UpgradeTasks::default()),
                changed: tokio::sync::Notify::new(),
                stop,
                report,
            }),
        }
    }

    fn spawn(
        &self,
        future: impl std::future::Future<Output = Result<()>> + Send + 'static,
    ) -> bool {
        use futures_util::FutureExt;
        let mut tasks = self
            .inner
            .tasks
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if tasks.closed {
            return false;
        }
        let report = self.inner.report.clone();
        tasks.pending.spawn(async move {
            let result = match std::panic::AssertUnwindSafe(future).catch_unwind().await {
                Ok(result) => result,
                Err(payload) => Err(anyhow!(
                    "managed WebSocket handler panicked: {}",
                    crate::case::panic_message(payload.as_ref())
                )),
            };
            if let Err(error) = &result
                && let Some(report) = report
            {
                report(format!("{error:#}"));
            }
            result
        });
        drop(tasks);
        self.inner.changed.notify_one();
        true
    }

    fn is_empty(&self) -> bool {
        self.inner
            .tasks
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .pending
            .is_empty()
    }

    async fn join_next(&self) -> Option<std::result::Result<Result<()>, tokio::task::JoinError>> {
        std::future::poll_fn(|cx| {
            self.inner
                .tasks
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .pending
                .poll_join_next(cx)
        })
        .await
    }

    fn close(&self) {
        self.inner
            .tasks
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .closed = true;
        self.inner.stop.cancel();
        self.inner.changed.notify_waiters();
    }

    fn abort_all(&self) {
        self.inner
            .tasks
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .pending
            .abort_all();
    }

    async fn shutdown(&self) -> Result<()> {
        self.close();
        let mut failures = Vec::new();
        let collect = |failures: &mut Vec<String>, result| match result {
            Ok(Ok(())) => {}
            Ok(Err(error)) => failures.push(format!("{error:#}")),
            Err(error) => failures.push(format!("managed WebSocket worker failed: {error}")),
        };
        let drain = async {
            while let Some(result) = self.join_next().await {
                collect(&mut failures, result);
            }
        };
        if tokio::time::timeout(Duration::from_secs(2), drain)
            .await
            .is_err()
        {
            self.abort_all();
            failures.push(
                "managed WebSocket handlers did not stop before their cleanup deadline".into(),
            );
            let reap = async {
                while let Some(result) = self.join_next().await {
                    collect(&mut failures, result);
                }
            };
            if tokio::time::timeout(Duration::from_secs(1), reap)
                .await
                .is_err()
            {
                failures.push("aborted WebSocket workers did not yield for reaping".into());
            }
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(anyhow!(failures.join("\n")))
        }
    }
}

struct AbortUpgradesOnDrop(UpgradeOwner);

impl Drop for AbortUpgradesOnDrop {
    fn drop(&mut self) {
        self.0.close();
        self.0.abort_all();
    }
}

/// A loopback service owned by a case. Dropping this address handle does not
/// stop the service; case cleanup joins it after the product exits. Call
/// `shutdown()` to deliberately disconnect it during a fault scenario.
#[derive(Clone)]
pub struct HttpFixture {
    address: SocketAddr,
    stop: CancellationToken,
    ended: watch::Receiver<Option<std::result::Result<(), String>>>,
}

impl HttpFixture {
    pub fn address(&self) -> SocketAddr {
        self.address
    }
    pub fn base_url(&self) -> String {
        format!("http://{}", self.address)
    }
    pub fn url(&self, path: &str) -> String {
        format!("{}/{}", self.base_url(), path.trim_start_matches('/'))
    }

    pub async fn shutdown(&self) -> Result<()> {
        self.stop.cancel();
        let mut ended = self.ended.clone();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Some(result) = ended.borrow_and_update().clone() {
                    return result.map_err(anyhow::Error::msg);
                }
                ended
                    .changed()
                    .await
                    .context("HTTP fixture owner disappeared")?;
            }
        })
        .await
        .context("HTTP fixture did not stop within five seconds")?
    }
}

impl TestCase {
    /// Serve a real Axum router on an ephemeral loopback port. For example,
    /// pass an SDK plugin HTTP router and configure its `/rpc` URL through the
    /// product settings API. HTTP upgrades (including WebSocket) are supported;
    /// their sockets remain owned and are disconnected on fixture shutdown.
    /// Use FixtureWebSocketUpgrade in WebSocket routes to own the handshake and
    /// protocol worker as well: errors/panics fail the case, and shutdown joins
    /// the handler. Raw Axum on_upgrade callbacks still only have their sockets
    /// tracked. No handler or HostClient behavior is simulated here.
    pub async fn serve_http(&self, router: Router) -> Result<HttpFixture> {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await?;
        let address = listener.local_addr()?;
        let stop = self.service_cancellation_token().child_token();
        let (finished, ended) = watch::channel(None);
        let task_stop = stop.clone();
        let report = self.task_failure_reporter("WebSocket fixture");
        self.spawn_service(async move {
            let mut completion = Completion {
                finished,
                done: false,
            };
            let result = serve_owned(listener, router, task_stop, Some(report)).await;
            completion.finished.send_replace(Some(
                result
                    .as_ref()
                    .map(|_| ())
                    .map_err(|error| format!("{error:#}")),
            ));
            completion.done = true;
            result
        });
        Ok(HttpFixture {
            address,
            stop,
            ended,
        })
    }
}

struct Completion {
    finished: watch::Sender<Option<std::result::Result<(), String>>>,
    done: bool,
}

impl Drop for Completion {
    fn drop(&mut self) {
        if !self.done {
            self.finished.send_replace(Some(Err(
                "HTTP fixture task was aborted or panicked".into()
            )));
        }
    }
}

#[cfg(test)]
async fn serve(listener: TcpListener, router: Router, stop: CancellationToken) -> Result<()> {
    serve_owned(listener, router, stop, None).await
}

async fn serve_owned(
    listener: TcpListener,
    router: Router,
    stop: CancellationToken,
    report: Option<FailureReporter>,
) -> Result<()> {
    let _cancel_connections = stop.clone().drop_guard();
    let upgrades = UpgradeOwner::new(stop.clone(), report);
    let _abort_upgrades = AbortUpgradesOnDrop(upgrades.clone());
    let router = router.layer(axum::Extension(upgrades.clone()));
    let mut connections = JoinSet::new();
    let mut failure = None;
    loop {
        tokio::select! {
            biased;
            () = stop.cancelled() => break,
            () = upgrades.inner.changed.notified() => {},
            result = upgrades.join_next(), if !upgrades.is_empty() => {
                match result {
                    Some(Ok(Err(error))) => { failure = Some(error); break; }
                    Some(Err(error)) => {
                        failure = Some(anyhow!("managed WebSocket worker failed: {error}"));
                        break;
                    }
                    _ => {}
                }
            },
            result = connections.join_next(), if !connections.is_empty() => {
                match result {
                    Some(Ok(Err(error))) => { failure = Some(error); break; }
                    Some(Err(error)) => {
                        failure = Some(anyhow!("HTTP fixture connection handler panicked: {error}"));
                        break;
                    }
                    _ => {}
                }
            },
            accepted = listener.accept() => {
                let (socket, _) = match accepted {
                    Ok(accepted) => accepted,
                    Err(error) => {
                        failure = Some(anyhow!("accept local fixture connection: {error}"));
                        break;
                    }
                };
                let router = router.clone();
                connections.spawn(serve_connection(socket, router, stop.clone()));
            },
        }
    }
    // On every exit path, close the listener, cancel all accepted connections,
    // and join them before reporting fixture completion. JoinSet's Drop alone
    // would abort them without waiting for their sockets and bodies to drop.
    drop(listener);
    upgrades.close();
    // The two drains run together: cancellation wakes managed handlers while
    // connection owners close their sockets, including handlers blocked on IO.
    let drain_connections = async {
        let mut failures = Vec::new();
        while let Some(result) = connections.join_next().await {
            match result {
                Ok(Ok(())) => {}
                Ok(Err(error)) => failures.push(format!("{error:#}")),
                Err(error) => {
                    failures.push(format!("HTTP fixture connection task failed: {error}"))
                }
            }
        }
        failures
    };
    let (connection_failures, upgrade_result) =
        tokio::join!(drain_connections, upgrades.shutdown());
    let mut failures = failure
        .take()
        .into_iter()
        .map(|error| format!("{error:#}"))
        .collect::<Vec<_>>();
    failures.extend(connection_failures);
    if let Err(error) = upgrade_result {
        failures.push(format!("{error:#}"));
    }
    if !failures.is_empty() {
        return Err(anyhow!(failures.join("\n")));
    }
    Ok(())
}

// Hyper transfers the IO into an upgraded handler and completes its connection
// future. Keep a shutdown handle and a release receipt so that completion does
// not accidentally detach an open WebSocket from the fixture's JoinSet.
struct SocketOwner(std::net::TcpStream);

impl SocketOwner {
    fn close(&self) {
        let _ = self.0.shutdown(Shutdown::Both);
    }
}

impl Drop for SocketOwner {
    fn drop(&mut self) {
        self.close();
    }
}

struct TrackedSocket {
    socket: TcpStream,
    released: Option<oneshot::Sender<()>>,
}

impl Drop for TrackedSocket {
    fn drop(&mut self) {
        if let Some(released) = self.released.take() {
            let _ = released.send(());
        }
    }
}

impl AsyncRead for TrackedSocket {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.socket).poll_read(cx, buffer)
    }
}

impl AsyncWrite for TrackedSocket {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        bytes: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.socket).poll_write(cx, bytes)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.socket).poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.socket).poll_shutdown(cx)
    }
}

async fn serve_connection(
    socket: TcpStream,
    router: Router,
    stop: CancellationToken,
) -> Result<()> {
    let socket = socket.into_std()?;
    let owner = SocketOwner(socket.try_clone()?);
    let (released, mut release) = oneshot::channel();
    let io = TrackedSocket {
        socket: TcpStream::from_std(socket)?,
        released: Some(released),
    };
    {
        let service = service_fn(move |request: hyper::Request<hyper::body::Incoming>| {
            router.clone().oneshot(request.map(Body::new))
        });
        let connection = http1::Builder::new()
            .serve_connection(TokioIo::new(io), service)
            .with_upgrades();
        tokio::pin!(connection);
        tokio::select! {
            () = stop.cancelled() => {},
            // Disconnects and malformed HTTP are valid fault scenarios.
            _ = &mut connection => {},
        }
    }
    tokio::select! {
        result = &mut release => result.context("HTTP fixture socket lost its release receipt")?,
        () = stop.cancelled() => {
            // Closing a cloned TCP handle also interrupts reads in an upgraded
            // Axum handler. Wait for that handler to drop the actual IO.
            owner.close();
            tokio::time::timeout(Duration::from_secs(3), &mut release)
                .await.context("HTTP upgrade handler retained its socket after fixture shutdown")?
                .context("HTTP fixture socket lost its release receipt")?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    use axum::{http::StatusCode, routing::get};
    use tokio::sync::Notify;

    use super::*;

    struct Dropped(Arc<AtomicBool>);

    impl Drop for Dropped {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Release);
        }
    }

    #[tokio::test]
    async fn websocket_upgrades_exchange_real_frames_and_shutdown_releases_the_socket() -> Result<()>
    {
        use axum::extract::{WebSocketUpgrade, ws::Message};
        use futures_util::{SinkExt, StreamExt};

        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await?;
        let address = listener.local_addr()?;
        let stop = CancellationToken::new();
        let _stop_on_test_exit = stop.clone().drop_guard();
        let dropped = Arc::new(AtomicBool::new(false));
        let router = Router::new().route(
            "/ws",
            get({
                let dropped = dropped.clone();
                move |upgrade: WebSocketUpgrade| {
                    let dropped = dropped.clone();
                    async move {
                        upgrade.on_upgrade(move |mut socket| async move {
                            let _guard = Dropped(dropped);
                            while let Some(Ok(message)) = socket.next().await {
                                if let Message::Text(text) = message
                                    && socket.send(Message::Text(text)).await.is_err()
                                {
                                    break;
                                }
                            }
                        })
                    }
                }
            }),
        );
        let server = tokio::spawn(serve(listener, router, stop.clone()));
        let (mut client, _) =
            tokio_tungstenite::connect_async(format!("ws://{address}/ws")).await?;
        client
            .send(tokio_tungstenite::tungstenite::Message::Text(
                "upgrade-round-trip".into(),
            ))
            .await?;
        let echoed = tokio::time::timeout(Duration::from_secs(2), client.next())
            .await?
            .context("upgraded service disconnected before echo")??;
        assert_eq!(echoed.to_text()?, "upgrade-round-trip");
        assert!(!dropped.load(Ordering::Acquire));

        // Keep the client open. Fixture cleanup must close the transferred socket
        // and wait for its owner, rather than only dropping the HTTP listener.
        stop.cancel();
        tokio::time::timeout(Duration::from_secs(5), server).await???;
        assert!(
            dropped.load(Ordering::Acquire),
            "upgrade escaped fixture cleanup"
        );
        let terminal = tokio::time::timeout(Duration::from_secs(2), client.next()).await?;
        assert!(
            terminal.is_none()
                || terminal
                    .is_some_and(|message| message.is_err()
                        || message.is_ok_and(|message| message.is_close()))
        );
        assert!(TcpStream::connect(address).await.is_err());
        Ok(())
    }

    #[tokio::test]
    async fn a_panicking_connection_still_joins_other_inflight_handlers() -> Result<()> {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await?;
        let address = listener.local_addr()?;
        let stop = CancellationToken::new();
        let _stop_on_test_exit = stop.clone().drop_guard();
        let entered = Arc::new(Notify::new());
        let dropped = Arc::new(AtomicBool::new(false));
        let router = Router::new()
            .route(
                "/held",
                get({
                    let entered = entered.clone();
                    let dropped = dropped.clone();
                    move || {
                        let entered = entered.clone();
                        let dropped = dropped.clone();
                        async move {
                            let _guard = Dropped(dropped);
                            entered.notify_one();
                            std::future::pending::<StatusCode>().await
                        }
                    }
                }),
            )
            .route(
                "/panic",
                get(|| async {
                    panic!("intentional fixture handler panic");
                    #[allow(unreachable_code)]
                    StatusCode::OK
                }),
            );
        let owner = tokio::spawn(serve(listener, router, stop));
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(5))
            .build()?;
        let held = tokio::spawn({
            let client = client.clone();
            async move { client.get(format!("http://{address}/held")).send().await }
        });
        tokio::time::timeout(Duration::from_secs(2), entered.notified()).await?;
        assert!(
            client
                .get(format!("http://{address}/panic"))
                .send()
                .await
                .is_err()
        );
        let result = tokio::time::timeout(Duration::from_secs(2), owner).await??;
        assert!(result.is_err());
        assert!(
            dropped.load(Ordering::Acquire),
            "fixture completion must follow handler drop"
        );
        assert!(
            tokio::time::timeout(Duration::from_secs(2), held)
                .await??
                .is_err()
        );
        assert!(tokio::net::TcpStream::connect(address).await.is_err());
        Ok(())
    }
    async fn prepared_case() -> Result<TestCase> {
        crate::TestAgena::builder(std::env::current_exe()?)
            .raw()
            .shutdown_timeout(Duration::from_secs(4))
            .prepare()
            .await
    }

    async fn ended_result(fixture: &HttpFixture) -> Result<std::result::Result<(), String>> {
        let mut ended = fixture.ended.clone();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Some(result) = ended.borrow_and_update().clone() {
                    return Ok::<_, anyhow::Error>(result);
                }
                ended.changed().await?;
            }
        })
        .await
        .context("fixture did not report its worker outcome")?
    }

    #[tokio::test]
    async fn managed_upgrade_shutdown_awaits_callback_cleanup_without_socket_reads() -> Result<()> {
        let case = prepared_case().await?;
        let entered = Arc::new(Notify::new());
        let cancelled = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let dropped = Arc::new(AtomicBool::new(false));
        let fixture = case
            .serve_http(Router::new().route(
                "/ws",
                get({
                    let entered = entered.clone();
                    let cancelled = cancelled.clone();
                    let release = release.clone();
                    let dropped = dropped.clone();
                    move |upgrade: FixtureWebSocketUpgrade| {
                        let entered = entered.clone();
                        let cancelled = cancelled.clone();
                        let release = release.clone();
                        let dropped = dropped.clone();
                        async move {
                            upgrade.on_upgrade(move |socket, cancel| async move {
                                let _socket = socket;
                                let _guard = Dropped(dropped);
                                entered.notify_one();
                                cancel.cancelled().await;
                                cancelled.notify_one();
                                release.notified().await;
                                Ok(())
                            })
                        }
                    }
                }),
            ))
            .await?;
        let (_client, _) = tokio_tungstenite::connect_async(
            fixture.base_url().replacen("http:", "ws:", 1) + "/ws",
        )
        .await?;
        tokio::time::timeout(Duration::from_secs(2), entered.notified()).await?;
        let shutdown = tokio::spawn({
            let fixture = fixture.clone();
            async move { fixture.shutdown().await }
        });
        tokio::time::timeout(Duration::from_secs(2), cancelled.notified()).await?;
        assert!(
            !shutdown.is_finished(),
            "shutdown skipped the callback's cleanup"
        );
        assert!(!dropped.load(Ordering::Acquire));
        release.notify_one();
        tokio::time::timeout(Duration::from_secs(4), shutdown).await???;
        assert!(dropped.load(Ordering::Acquire));
        assert!(TcpStream::connect(fixture.address()).await.is_err());
        fixture.shutdown().await?;
        case.finish().await?;
        Ok(())
    }

    #[derive(Clone, Copy)]
    enum WorkerFailure {
        Error,
        ConstructPanic,
        PollPanic,
    }

    #[tokio::test]
    async fn managed_upgrade_errors_and_both_panic_boundaries_fail_before_case_finish() -> Result<()>
    {
        for failure in [
            WorkerFailure::Error,
            WorkerFailure::ConstructPanic,
            WorkerFailure::PollPanic,
        ] {
            let case = prepared_case().await?;
            let fixture = case
                .serve_http(Router::new().route(
                    "/ws",
                    get(move |upgrade: FixtureWebSocketUpgrade| async move {
                        upgrade.on_upgrade(move |_socket, _cancel| {
                            if matches!(failure, WorkerFailure::ConstructPanic) {
                                panic!("construct-upgrade-panic-marker");
                            }
                            async move {
                                if matches!(failure, WorkerFailure::PollPanic) {
                                    panic!("poll-upgrade-panic-marker");
                                }
                                Err(anyhow!("upgrade-error-marker"))
                            }
                        })
                    }),
                ))
                .await?;
            let client = tokio_tungstenite::connect_async(
                fixture.base_url().replacen("http:", "ws:", 1) + "/ws",
            )
            .await;
            drop(client);
            let error = ended_result(&fixture)
                .await?
                .expect_err("handler failure was detached");
            let marker = match failure {
                WorkerFailure::Error => "upgrade-error-marker",
                WorkerFailure::ConstructPanic => "construct-upgrade-panic-marker",
                WorkerFailure::PollPanic => "poll-upgrade-panic-marker",
            };
            assert!(error.contains(marker), "{error}");
            let cleanup = case.finish().await.unwrap_err().to_string();
            assert!(cleanup.contains(marker), "{cleanup}");
            assert!(TcpStream::connect(fixture.address()).await.is_err());
            tokio::fs::remove_dir_all(&case.paths().root).await?;
        }
        Ok(())
    }

    #[tokio::test]
    async fn managed_upgrade_registered_before_handshake_is_reaped_when_request_is_cancelled()
    -> Result<()> {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let case = prepared_case().await?;
        let registered = Arc::new(Notify::new());
        let handler_called = Arc::new(AtomicBool::new(false));
        let http_dropped = Arc::new(AtomicBool::new(false));
        let fixture = case
            .serve_http(Router::new().route(
                "/ws",
                get({
                    let registered = registered.clone();
                    let handler_called = handler_called.clone();
                    let http_dropped = http_dropped.clone();
                    move |upgrade: FixtureWebSocketUpgrade| {
                        let registered = registered.clone();
                        let handler_called = handler_called.clone();
                        let http_dropped = http_dropped.clone();
                        async move {
                            let _http = Dropped(http_dropped);
                            let _response =
                                upgrade.on_upgrade(move |_socket, _cancel| async move {
                                    handler_called.store(true, Ordering::Release);
                                    Ok(())
                                });
                            registered.notify_one();
                            std::future::pending::<axum::response::Response>().await
                        }
                    }
                }),
            ))
            .await?;
        let mut client = TcpStream::connect(fixture.address()).await?;
        client.write_all(b"GET /ws HTTP/1.1\r\nHost: localhost\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n").await?;
        tokio::time::timeout(Duration::from_secs(2), registered.notified()).await?;
        fixture.shutdown().await?;
        assert!(http_dropped.load(Ordering::Acquire));
        assert!(!handler_called.load(Ordering::Acquire));
        let mut byte = [0];
        let terminal = tokio::time::timeout(Duration::from_secs(2), client.read(&mut byte)).await?;
        assert!(matches!(terminal, Ok(0) | Err(_)), "{terminal:?}");
        case.finish().await?;
        Ok(())
    }

    #[tokio::test]
    async fn managed_upgrade_held_worker_is_aborted_joined_and_reported_as_failure() -> Result<()> {
        let case = prepared_case().await?;
        let entered = Arc::new(Notify::new());
        let dropped = Arc::new(AtomicBool::new(false));
        let fixture = case
            .serve_http(Router::new().route(
                "/ws",
                get({
                    let entered = entered.clone();
                    let dropped = dropped.clone();
                    move |upgrade: FixtureWebSocketUpgrade| {
                        let entered = entered.clone();
                        let dropped = dropped.clone();
                        async move {
                            upgrade.on_upgrade(move |socket, _cancel| async move {
                                let _socket = socket;
                                let _guard = Dropped(dropped);
                                entered.notify_one();
                                std::future::pending::<Result<()>>().await
                            })
                        }
                    }
                }),
            ))
            .await?;
        let (_client, _) = tokio_tungstenite::connect_async(
            fixture.base_url().replacen("http:", "ws:", 1) + "/ws",
        )
        .await?;
        tokio::time::timeout(Duration::from_secs(2), entered.notified()).await?;
        let error = fixture.shutdown().await.unwrap_err().to_string();
        assert!(error.contains("cleanup deadline"), "{error}");
        assert!(
            dropped.load(Ordering::Acquire),
            "shutdown did not join the aborted handler"
        );
        assert!(TcpStream::connect(fixture.address()).await.is_err());
        assert!(case.finish().await.is_err());
        tokio::fs::remove_dir_all(&case.paths().root).await?;
        Ok(())
    }

    #[tokio::test]
    async fn managed_upgrade_requires_fixture_context_and_rejects_non_upgrade_requests()
    -> Result<()> {
        let handler_called = Arc::new(AtomicBool::new(false));
        let router = Router::new().route(
            "/ws",
            get({
                let called = handler_called.clone();
                move |upgrade: FixtureWebSocketUpgrade| {
                    let called = called.clone();
                    async move {
                        upgrade.on_upgrade(move |_socket, _cancel| async move {
                            called.store(true, Ordering::Release);
                            Ok(())
                        })
                    }
                }
            }),
        );
        let response = router
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri("/ws")
                    .body(Body::empty())?,
            )
            .await?;
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = axum::body::to_bytes(response.into_body(), 1024).await?;
        assert!(std::str::from_utf8(&body)?.contains("TestCase::serve_http"));

        let case = prepared_case().await?;
        let fixture = case.serve_http(router).await?;
        let response = reqwest::Client::builder()
            .no_proxy()
            .build()?
            .get(fixture.url("/ws"))
            .send()
            .await?;
        assert!(response.status().is_client_error());
        assert!(!handler_called.load(Ordering::Acquire));
        fixture.shutdown().await?;
        case.finish().await?;
        Ok(())
    }
}
