#![cfg(unix)]

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use agena_plugin_host::config::{DurationSpec, RestartMode, RestartPolicy};
use agena_plugin_host::logs::PluginLogStore;
use agena_plugin_host::sdk::{PluginKey, ToolInvokeInput};
use agena_plugin_host::status::{PluginRunState, PluginStatus, StatusRegistry};
use agena_plugin_host::transport::{
    PluginTransport,
    stdio::{HostHandler, StdioTransport},
};

struct Fixture {
    root: tempfile::TempDir,
    status: Arc<StatusRegistry>,
    logs: Arc<PluginLogStore>,
    key: PluginKey,
    transport: StdioTransport,
}

impl Fixture {
    async fn new(policy: RestartMode, handler: Option<HostHandler>) -> Self {
        Self::with_backoff(policy, handler, Duration::from_millis(5)).await
    }

    async fn with_backoff(
        policy: RestartMode,
        handler: Option<HostHandler>,
        backoff: Duration,
    ) -> Self {
        let root = tempfile::tempdir().unwrap();
        let script = root.path().join("plugin.py");
        std::fs::write(&script, include_str!("fixtures/stdio_lifecycle.py")).unwrap();
        let status = Arc::new(StatusRegistry::new());
        let logs = Arc::new(PluginLogStore::default());
        let key: PluginKey = "test.lifecycle".parse().unwrap();
        status.set(PluginStatus::initial(&key, "stdio"));
        let env = HashMap::from([
            (
                "AGENA_TEST_STARTS".into(),
                root.path().join("starts").to_str().unwrap().into(),
            ),
            (
                "AGENA_TEST_REPLIES".into(),
                root.path().join("replies").to_str().unwrap().into(),
            ),
        ]);
        let transport = StdioTransport::spawn_with_policy_and_status(
            "/usr/bin/python3",
            &[script.to_str().unwrap().into()],
            &env,
            Some(&root.path().to_path_buf()),
            handler,
            RestartPolicy {
                policy,
                max_retries: 2,
                min_backoff: DurationSpec(backoff),
                max_backoff: DurationSpec(backoff),
            },
            Some(key.clone()),
            Some(status.clone()),
            Some(logs.clone()),
            None,
        )
        .await
        .unwrap();
        transport
            .dispatch("meta/ping", serde_json::json!({}))
            .await
            .unwrap();
        Self {
            root,
            status,
            logs,
            key,
            transport,
        }
    }

    async fn request_exit(&self, method: &str, params: serde_json::Value) {
        let result = tokio::time::timeout(
            Duration::from_secs(3),
            self.transport.dispatch(method, params),
        )
        .await;
        assert!(
            result.is_ok(),
            "pending requests must be released after the process exits"
        );
        assert!(result.unwrap().is_err());
    }

    async fn wait_for(&self, condition: impl Fn(&PluginStatus) -> bool) -> PluginStatus {
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let status = self.status.get(&self.key).unwrap();
                if condition(&status) {
                    return status;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap_or_else(|_| {
            panic!(
                "unexpected final status: {:?}; logs: {:?}",
                self.status.get(&self.key),
                self.logs.list(&self.key, None, 0)
            )
        })
    }

    fn starts(&self) -> usize {
        std::fs::read_to_string(self.root.path().join("starts"))
            .unwrap()
            .lines()
            .count()
    }
}

#[tokio::test]
async fn on_failure_keeps_clean_exits_stopped_including_early_stdout_eof() {
    for (method, params) in [
        ("test/exit", serde_json::json!({"code": 0})),
        (
            "test/close_stdout",
            serde_json::json!({"delay_ms": 200, "code": 0}),
        ),
    ] {
        let fixture = Fixture::new(RestartMode::OnFailure, None).await;
        fixture.request_exit(method, params).await;
        let status = fixture
            .wait_for(|status| status.state == PluginRunState::Stopped)
            .await;
        assert_eq!(status.last_exit_code, Some(0));
        assert_eq!(status.restart_count, 0);
        assert!(status.last_failure.is_none());
        assert_eq!(fixture.starts(), 1);
        fixture.transport.close().await.unwrap();
    }
}

#[tokio::test]
async fn restart_modes_distinguish_clean_and_failed_exits() {
    for (mode, code, restarts) in [
        (RestartMode::Always, 0, true),
        (RestartMode::OnFailure, 7, true),
        (RestartMode::Never, 7, false),
    ] {
        let fixture = Fixture::new(mode, None).await;
        fixture
            .request_exit("test/exit", serde_json::json!({"code": code}))
            .await;
        let status = fixture
            .wait_for(|status| {
                if restarts {
                    status.restart_count == 1 && status.state == PluginRunState::Running
                } else {
                    status.state == PluginRunState::Failed
                }
            })
            .await;
        assert_eq!(status.last_exit_code, Some(code));
        if restarts {
            fixture
                .transport
                .dispatch("meta/ping", serde_json::json!({}))
                .await
                .unwrap();
        }
        assert_eq!(fixture.starts(), if restarts { 2 } else { 1 });
        fixture.transport.close().await.unwrap();
    }
}

#[tokio::test]
async fn malformed_protocol_is_a_failure_even_with_exit_code_zero() {
    let fixture = Fixture::new(RestartMode::Never, None).await;
    fixture
        .request_exit("test/malformed", serde_json::json!({}))
        .await;
    let status = fixture
        .wait_for(|status| status.state == PluginRunState::Failed)
        .await;
    assert!(
        status.last_failure.is_some(),
        "protocol failure must be observable in lifecycle status"
    );
    assert!(
        fixture
            .logs
            .list(&fixture.key, None, 0)
            .iter()
            .any(|record| record.message.contains("JSON-RPC decode error"))
    );
    fixture.transport.close().await.unwrap();
}

#[tokio::test]
async fn malformed_protocol_restarts_under_on_failure() {
    let fixture = Fixture::new(RestartMode::OnFailure, None).await;
    fixture
        .request_exit("test/malformed", serde_json::json!({}))
        .await;
    fixture
        .wait_for(|status| status.restart_count == 1 && status.state == PluginRunState::Running)
        .await;
    fixture
        .transport
        .dispatch("meta/ping", serde_json::json!({}))
        .await
        .unwrap();
    fixture.transport.close().await.unwrap();
}

#[tokio::test]
async fn complete_response_immediately_before_exit_is_delivered() {
    for _ in 0..10 {
        let fixture = Fixture::new(RestartMode::OnFailure, None).await;
        let result = tokio::time::timeout(
            Duration::from_secs(3),
            fixture
                .transport
                .dispatch("test/reply_then_exit", serde_json::json!({})),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(result, serde_json::json!({"ok": true}));
        fixture
            .wait_for(|status| status.state == PluginRunState::Stopped)
            .await;
        fixture.transport.close().await.unwrap();
    }
}

#[tokio::test]
async fn signal_exit_clears_the_previous_numeric_exit_code() {
    let fixture = Fixture::new(RestartMode::OnFailure, None).await;
    fixture
        .request_exit("test/exit", serde_json::json!({"code": 7}))
        .await;
    let status = fixture
        .wait_for(|status| status.restart_count == 1 && status.state == PluginRunState::Running)
        .await;
    assert_eq!(status.last_exit_code, Some(7));
    fixture
        .request_exit("test/signal", serde_json::json!({}))
        .await;
    let status = fixture
        .wait_for(|status| status.restart_count == 2 && status.state == PluginRunState::Running)
        .await;
    assert_eq!(status.last_exit_code, None);
    fixture
        .transport
        .dispatch("meta/ping", serde_json::json!({}))
        .await
        .unwrap();
    fixture.transport.close().await.unwrap();
}

#[tokio::test]
async fn stdout_eof_with_a_live_child_is_a_failure() {
    let fixture = Fixture::new(RestartMode::Never, None).await;
    fixture
        .request_exit("test/close_stdout", serde_json::json!({"delay_ms": 30_000}))
        .await;
    let status = fixture
        .wait_for(|status| status.state == PluginRunState::Failed)
        .await;
    assert!(status.last_failure.is_some());
    fixture.transport.close().await.unwrap();
}

#[tokio::test]
async fn close_cancels_a_long_restart_backoff() {
    let fixture = Fixture::with_backoff(RestartMode::Always, None, Duration::from_secs(30)).await;
    fixture
        .request_exit("test/exit", serde_json::json!({"code": 0}))
        .await;
    fixture
        .wait_for(|status| status.state == PluginRunState::Restarting)
        .await;
    tokio::time::timeout(Duration::from_secs(1), fixture.transport.close())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        fixture.status.get(&fixture.key).unwrap().state,
        PluginRunState::Stopped
    );
    assert_eq!(fixture.starts(), 1);
    assert!(
        fixture
            .transport
            .dispatch("meta/ping", serde_json::json!({}))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn stderr_is_bounded_and_keeps_draining_after_invalid_utf8() {
    let fixture = Fixture::new(RestartMode::Never, None).await;
    tokio::time::timeout(
        Duration::from_secs(3),
        fixture
            .transport
            .dispatch("test/noisy_stderr", serde_json::json!({})),
    )
    .await
    .unwrap()
    .unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let records = fixture.logs.list(&fixture.key, None, 0);
            if records.iter().any(|record| {
                record.source == "stderr" && record.message.contains("stderr-finished")
            }) {
                let stderr: Vec<_> = records
                    .iter()
                    .filter(|record| record.source == "stderr")
                    .collect();
                assert!(
                    stderr
                        .iter()
                        .all(|record| record.message.len() <= 3 * 16 * 1024)
                );
                assert!(
                    stderr
                        .iter()
                        .any(|record| record.message.contains('\u{fffd}'))
                );
                assert!(
                    stderr
                        .iter()
                        .any(|record| record.fields["continues"] == true)
                );
                let text = stderr
                    .iter()
                    .map(|record| record.message.as_str())
                    .collect::<String>();
                assert_eq!(text.matches('x').count(), 116_383);
                assert_eq!(text.matches('y').count(), 100_000);
                assert_eq!(text.matches('😀').count(), 1);
                assert_eq!(text.matches('\u{fffd}').count(), 2);
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    fixture
        .transport
        .dispatch("meta/ping", serde_json::json!({}))
        .await
        .unwrap();
    fixture.transport.close().await.unwrap();
}

fn stream_input(fixture: &Fixture, input: serde_json::Value) -> ToolInvokeInput {
    ToolInvokeInput {
        tool_name: "fixture".into(),
        session_id: 1,
        call_id: 1,
        workspace_root: fixture.root.path().to_str().unwrap().into(),
        input,
    }
}

async fn open_stream(
    fixture: &Fixture,
    complete: bool,
) -> agena_plugin_host::transport::ToolStreamHandle {
    tokio::time::timeout(
        Duration::from_secs(3),
        fixture.transport.invoke_stream(stream_input(
            fixture,
            serde_json::json!({"complete": complete}),
        )),
    )
    .await
    .unwrap()
    .unwrap()
    .unwrap()
}

#[tokio::test]
async fn early_stream_terminal_does_not_discard_buffered_chunks_or_hide_overflow() {
    for count in [64, 65, 128] {
        let fixture = Fixture::new(RestartMode::Never, None).await;
        let mut stream = tokio::time::timeout(
            Duration::from_secs(3),
            fixture.transport.invoke_stream(stream_input(
                &fixture,
                serde_json::json!({"early_chunks": count}),
            )),
        )
        .await
        .unwrap()
        .unwrap()
        .unwrap();
        let mut chunks = Vec::new();
        tokio::time::timeout(Duration::from_secs(3), async {
            while let Some(chunk) = stream.chunks.recv().await {
                chunks.push(chunk.text_delta.unwrap());
            }
        })
        .await
        .unwrap();
        let end = stream.end.await.unwrap();
        if count == 64 {
            assert_eq!(
                chunks,
                (0..count)
                    .map(|index| index.to_string())
                    .collect::<Vec<_>>()
            );
            assert_eq!(end.unwrap().output_text, "done");
        } else {
            assert!(
                end.is_err(),
                "a success marker cannot erase an earlier buffer overflow"
            );
        }
        fixture.transport.close().await.unwrap();
    }
}

#[tokio::test]
async fn duplicate_active_stream_id_is_rejected_without_replacing_the_first_stream() {
    let fixture = Fixture::new(RestartMode::Never, None).await;
    let mut first = open_stream(&fixture, false).await;
    first.chunks.recv().await.unwrap();
    let duplicate = fixture
        .transport
        .invoke_stream(stream_input(
            &fixture,
            serde_json::json!({"complete": false}),
        ))
        .await;
    assert!(duplicate.is_err());
    fixture
        .transport
        .dispatch("test/end_stream", serde_json::json!({}))
        .await
        .unwrap();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(3), first.end)
            .await
            .unwrap()
            .unwrap()
            .unwrap()
            .output_text,
        "done"
    );
    fixture.transport.close().await.unwrap();
    fixture.transport.close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn restart_fails_old_stream_and_allows_the_new_process_to_reuse_its_stream_id() {
    let fixture = Fixture::new(RestartMode::OnFailure, None).await;
    let mut old = open_stream(&fixture, false).await;
    assert_eq!(
        old.chunks.recv().await.unwrap().text_delta.as_deref(),
        Some("first")
    );
    fixture
        .request_exit("test/exit", serde_json::json!({"code": 7}))
        .await;
    assert!(
        tokio::time::timeout(Duration::from_secs(3), old.end)
            .await
            .unwrap()
            .unwrap()
            .is_err()
    );
    fixture
        .wait_for(|status| status.restart_count == 1 && status.state == PluginRunState::Running)
        .await;
    let mut new = open_stream(&fixture, true).await;
    assert_eq!(new.stream_id, old.stream_id);
    let mut chunks = Vec::new();
    tokio::time::timeout(Duration::from_secs(3), async {
        while let Some(chunk) = new.chunks.recv().await {
            chunks.push(chunk.text_delta.unwrap());
        }
    })
    .await
    .unwrap();
    assert_eq!(chunks, ["first", "second"]);
    assert_eq!(new.end.await.unwrap().unwrap().output_text, "done");
    fixture.transport.close().await.unwrap();
}

#[tokio::test]
async fn a_descendant_holding_stdout_cannot_hide_the_main_process_exit() {
    let fixture = Fixture::new(RestartMode::Always, None).await;
    fixture
        .request_exit("test/descendant_stdout", serde_json::json!({}))
        .await;
    fixture
        .wait_for(|status| status.restart_count == 1 && status.state == PluginRunState::Running)
        .await;
    fixture.transport.close().await.unwrap();
}

#[tokio::test]
async fn closed_stdin_triggers_supervision_instead_of_leaving_a_running_dead_transport() {
    let fixture = Fixture::new(RestartMode::OnFailure, None).await;
    fixture
        .transport
        .dispatch("test/close_stdin", serde_json::json!({}))
        .await
        .unwrap();
    fixture
        .request_exit("meta/ping", serde_json::json!({}))
        .await;
    fixture
        .wait_for(|status| status.restart_count == 1 && status.state == PluginRunState::Running)
        .await;
    fixture
        .transport
        .dispatch("meta/ping", serde_json::json!({}))
        .await
        .unwrap();
    fixture.transport.close().await.unwrap();
}

struct Dropped(Arc<AtomicBool>);
impl Drop for Dropped {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

#[tokio::test]
async fn restart_cancels_callbacks_and_notifications_from_the_old_process() {
    for method in ["test/callback", "test/notification"] {
        let entered = Arc::new(tokio::sync::Notify::new());
        let dropped = Arc::new(AtomicBool::new(false));
        let handler: HostHandler = {
            let entered = entered.clone();
            let dropped = dropped.clone();
            Arc::new(move |_, _| {
                let entered = entered.clone();
                let dropped = dropped.clone();
                Box::pin(async move {
                    let _guard = Dropped(dropped);
                    entered.notify_one();
                    std::future::pending().await
                })
            })
        };
        let fixture = Fixture::new(RestartMode::OnFailure, Some(handler)).await;
        fixture
            .transport
            .dispatch(method, serde_json::json!({}))
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(1), entered.notified())
            .await
            .unwrap();
        fixture
            .request_exit("test/exit", serde_json::json!({"code": 1}))
            .await;
        fixture
            .wait_for(|status| status.restart_count == 1 && status.state == PluginRunState::Running)
            .await;
        assert!(
            dropped.load(Ordering::SeqCst),
            "a disconnected process must release its pending host work"
        );
        fixture.transport.close().await.unwrap();
    }
}
