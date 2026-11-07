//! Actual product-process ownership when a Rust test future is cancelled.
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use agena_testkit::{FixtureWebSocketUpgrade, HttpFixture, TestCase, TestResult};
use anyhow::{Context, ensure};
use axum::{Router, routing::get};
use futures_util::StreamExt;
use tokio::sync::{Notify, oneshot};

use super::super::fixture;

struct DropFlag(Arc<AtomicBool>);

impl Drop for DropFlag {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

struct AbortOnDrop(tokio::task::AbortHandle);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

struct ActiveCase {
    case: TestCase,
    service: HttpFixture,
    pid: u32,
    product_url: String,
    model_url: String,
}

async fn assert_listener_closed(url: &str) -> TestResult {
    let url = reqwest::Url::parse(url)?;
    let host = url.host_str().context("owned listener host")?;
    let port = url.port_or_known_default().context("owned listener port")?;
    let probe = tokio::time::timeout(
        Duration::from_secs(2),
        tokio::net::TcpStream::connect((host, port)),
    )
    .await?;
    ensure!(
        probe.is_err(),
        "owned listener still accepts connections: {url}"
    );
    Ok(())
}

fn assert_process_reaped(pid: u32) -> TestResult {
    #[cfg(unix)]
    {
        // SAFETY: signal 0 only observes the exact fixture-owned leader PID.
        let exists = unsafe { libc::kill(pid as libc::pid_t, 0) };
        ensure!(
            exists == -1 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH),
            "owned Agena PID {pid} still exists after finish"
        );
    }
    #[cfg(not(unix))]
    let _ = pid;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn aborting_run_drains_real_product_and_fixtures_even_when_they_retain_case_clones()
-> TestResult {
    tokio::time::timeout(Duration::from_secs(90), async {
        let (ready, active) = oneshot::channel();
        let (service_done, service_stopped) = oneshot::channel();
        let websocket_entered = Arc::new(Notify::new());
        let websocket_dropped = Arc::new(AtomicBool::new(false));
        let entered = websocket_entered.clone();
        let dropped = websocket_dropped.clone();
        let owner = tokio::spawn(fixture().run(move |case| async move {
            let retained = case.clone();
            let cancel = case.service_cancellation_token();
            case.spawn_service(async move {
                cancel.cancelled().await;
                // A case clone held by a service forms the ownership cycle that
                // previously prevented CaseInner::drop from initiating cleanup.
                ensure!(retained.workspace().is_dir());
                let _ = service_done.send(());
                Ok(())
            });
            let router_case = case.clone();
            let service = case
                .serve_http(
                    Router::new()
                        .route(
                            "/held-case",
                            get(move || {
                                let retained = router_case.clone();
                                async move { retained.workspace().display().to_string() }
                            }),
                        )
                        .route(
                            "/ws",
                            get(move |upgrade: FixtureWebSocketUpgrade| {
                                let entered = entered.clone();
                                let dropped = dropped.clone();
                                async move {
                                    upgrade.on_upgrade(move |socket, cancel| async move {
                                        let _socket = socket;
                                        let _guard = DropFlag(dropped);
                                        entered.notify_one();
                                        cancel.cancelled().await;
                                        Ok(())
                                    })
                                }
                            }),
                        ),
                )
                .await?;
            let proof = ActiveCase {
                pid: case.process_id().await.context("live owned product PID")?,
                product_url: case.endpoint().await,
                model_url: case.model().base_url().to_owned(),
                case,
                service,
            };
            ready
                .send(proof)
                .map_err(|_| anyhow::anyhow!("test owner disappeared"))?;
            std::future::pending::<TestResult>().await
        }));
        let _abort_owner = AbortOnDrop(owner.abort_handle());
        let proof = active.await?;
        let client = reqwest::Client::builder().no_proxy().build()?;
        ensure!(
            client
                .get(proof.service.url("/held-case"))
                .send()
                .await?
                .status()
                .is_success()
        );
        let (mut websocket, _) = tokio_tungstenite::connect_async(
            proof.service.base_url().replacen("http:", "ws:", 1) + "/ws",
        )
        .await?;
        tokio::time::timeout(Duration::from_secs(2), websocket_entered.notified()).await?;
        owner.abort();
        match owner.await {
            Err(error) => ensure!(error.is_cancelled(), "{error}"),
            Ok(_) => anyhow::bail!("the cancelled run unexpectedly completed"),
        }

        // Observe automatic cleanup before calling finish ourselves. Otherwise
        // this assertion could pass even if cancelling run leaked every owner.
        tokio::time::timeout(Duration::from_secs(25), service_stopped)
            .await
            .context("run cancellation did not initiate owned teardown")??;
        tokio::time::timeout(Duration::from_secs(20), proof.case.finish()).await??;
        ensure!(proof.case.process_id().await.is_none());
        ensure!(
            proof.case.last_exit().await.is_some(),
            "owned child was not waited"
        );
        assert_process_reaped(proof.pid)?;
        assert_listener_closed(&proof.product_url).await?;
        assert_listener_closed(&proof.model_url).await?;
        assert_listener_closed(&proof.service.base_url()).await?;
        ensure!(websocket_dropped.load(Ordering::Acquire));
        let terminal = tokio::time::timeout(Duration::from_secs(2), websocket.next()).await?;
        ensure!(
            terminal.is_none()
                || terminal.is_some_and(|frame| {
                    frame.is_err() || frame.is_ok_and(|frame| frame.is_close())
                }),
            "owned WebSocket remained usable after cleanup"
        );
        let diagnostic =
            tokio::fs::read_to_string(proof.case.paths().diagnostics.join("case.txt")).await?;
        ensure!(
            diagnostic.contains("test body future was cancelled"),
            "{diagnostic}"
        );
        // The cancellation contract retained these diagnostics; remove only
        // this regression's own successful negative-test artifact directory.
        tokio::fs::remove_dir_all(&proof.case.paths().root).await?;
        Ok(())
    })
    .await
    .context("external cancellation ownership regression exceeded its deadline")?
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelling_a_finish_waiter_keeps_the_real_cleanup_owner_and_all_joins() -> TestResult {
    fixture().run(|case| async move {
        let pid = case.process_id().await.context("owned product PID")?;
        let product_url = case.endpoint().await;
        let model_url = case.model().base_url().to_owned();
        let service = case.serve_http(Router::new().route("/", get(|| async { "live" }))).await?;
        let entered = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let completed = Arc::new(AtomicBool::new(false));
        let cancel = case.service_cancellation_token();
        let entered_worker = entered.clone();
        let release_worker = release.clone();
        let completion = completed.clone();
        case.spawn_service(async move {
            cancel.cancelled().await;
            entered_worker.notify_one();
            release_worker.notified().await;
            completion.store(true, Ordering::Release);
            Ok(())
        });
        let first = tokio::spawn({
            let case = case.clone();
            async move { case.finish().await }
        });
        tokio::time::timeout(Duration::from_secs(20), entered.notified()).await?;
        first.abort();
        ensure!(first.await.unwrap_err().is_cancelled());
        ensure!(!completed.load(Ordering::Acquire));
        let second = case.finish();
        tokio::pin!(second);
        tokio::select! {
            biased;
            result = &mut second => anyhow::bail!("finish returned before the service release: {result:?}"),
            () = tokio::task::yield_now() => {}
        }
        release.notify_one();
        tokio::time::timeout(Duration::from_secs(20), second).await??;
        ensure!(completed.load(Ordering::Acquire));
        ensure!(case.process_id().await.is_none());
        assert_process_reaped(pid)?;
        assert_listener_closed(&product_url).await?;
        assert_listener_closed(&model_url).await?;
        assert_listener_closed(&service.base_url()).await?;
        Ok(())
    }).await
}

/// PTYs establish an independent Unix session. Killing the product's process
/// group must also reap test-owned shell descendants outside that group.
#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn crashing_agena_cleans_independent_pty_shell_and_its_child() -> TestResult {
    fn is_live(pid: i32, start_ticks: u64) -> bool {
        std::fs::read_to_string(format!("/proc/{pid}/stat"))
            .ok()
            .and_then(|stat| {
                let fields = stat
                    .rsplit_once(") ")?
                    .1
                    .split_whitespace()
                    .collect::<Vec<_>>();
                let state = fields.first()?.chars().next()?;
                let start = fields.get(19)?.parse::<u64>().ok()?;
                (start == start_ticks && !matches!(state, 'Z' | 'X')).then_some(true)
            })
            .unwrap_or(false)
    }
    fn start_ticks(pid: i32) -> std::io::Result<u64> {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat"))?;
        stat.rsplit_once(") ")
            .and_then(|(_, rest)| rest.split_whitespace().nth(19))
            .and_then(|field| field.parse().ok())
            .ok_or_else(|| std::io::Error::other("missing process start time"))
    }
    fixture()
        .run(|case| async move {
            let terminal = case.terminal().create(case.workspace()).await?;
            let marker = case.workspace().join("owned-pty-shell.pid");
            terminal
                .input(b"trap '' HUP; echo $$ > owned-pty-shell.pid; sleep 25\n".to_vec())
                .await?;
            let (pid, sleep_pid) = tokio::time::timeout(Duration::from_secs(8), async {
                loop {
                    if let Ok(pid_text) = tokio::fs::read_to_string(&marker).await
                        && let Ok(pid) = pid_text.trim().parse::<i32>()
                        && let Ok(children) =
                            tokio::fs::read_to_string(format!("/proc/{pid}/task/{pid}/children"))
                                .await
                        && let Some(sleep_pid) = children
                            .split_whitespace()
                            .find_map(|value| value.parse::<i32>().ok())
                    {
                        break Ok::<_, anyhow::Error>((pid, sleep_pid));
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await
            .context("PTY shell did not spawn its foreground child")??;
            let start_shell = start_ticks(pid)?;
            let start_sleep = start_ticks(sleep_pid)?;
            ensure!(is_live(pid, start_shell) && is_live(sleep_pid, start_sleep));

            case.crash().await?;
            tokio::time::timeout(Duration::from_secs(4), async {
                while is_live(pid, start_shell) || is_live(sleep_pid, start_sleep) {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await
            .context("product crash left an independent PTY session running")?;
            case.restart().await?;
            case.user().new_session().await?;
            Ok(())
        })
        .await
}
