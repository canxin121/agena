use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;

async fn task_fixture(protocol: &str) -> (HttpFixture, McpConnectionManager) {
    let fixture = HttpFixture::start_process(protocol, &["--tasks"]).await;
    let manager = manager();
    manager
        .add_server("http", fixture.spec(None))
        .await
        .unwrap();
    (fixture, manager)
}

fn method_count(root: &Path, method: &str) -> usize {
    records(root)
        .into_iter()
        .filter(|message| message["method"] == method)
        .count()
}

#[tokio::test]
async fn tasks_negotiate_both_versions_and_return_tool_results_and_errors() {
    for protocol in ["modern", "legacy"] {
        let (fixture, manager) = task_fixture(protocol).await;
        let success = manager
            .call_tool("http", "task", Some(json!({"label": "ready"})))
            .await
            .unwrap();
        assert_eq!(text_content(success), "ready");
        let failure = manager
            .call_tool("http", "task-optional", Some(json!({"kind": "tool-error"})))
            .await
            .unwrap();
        assert!(failure.is_error);
        let error = manager
            .call_tool("http", "task", Some(json!({"kind": "rpc-error"})))
            .await
            .unwrap_err();
        assert!(matches!(error, McpError::Rpc { code: -32042, message }
            if message == "fixture task failed"));
        for name in ["ping", "task-forbidden"] {
            assert!(
                !manager
                    .call_tool("http", name, None)
                    .await
                    .unwrap()
                    .is_error
            );
        }
        let rpc = records(fixture.root.path());
        let first_call = rpc
            .iter()
            .find(|entry| entry["method"] == "tools/call")
            .unwrap();
        let first_poll = rpc
            .iter()
            .find(|entry| entry["method"] == "tasks/get")
            .unwrap();
        assert!(
            first_poll["metadata"]["time"].as_f64().unwrap()
                - first_call["metadata"]["time"].as_f64().unwrap()
                >= 0.015,
            "client polled before the suggested interval: {protocol}"
        );
        let origin = &first_call["metadata"];
        if protocol == "modern" {
            assert!(origin["params"].get("task").is_none());
            let capabilities = &origin["capabilities"];
            assert_eq!(
                capabilities["extensions"]["io.modelcontextprotocol/tasks"],
                json!({})
            );
            assert!(capabilities.get("tasks").is_none());
            assert!(!has_method(fixture.root.path(), "tasks/result"));
        } else {
            assert_eq!(origin["params"]["task"], json!({}));
            assert_eq!(method_count(fixture.root.path(), "tasks/result"), 3);
        }
        assert_eq!(method_count(fixture.root.path(), "tools/call"), 5);
        assert!(!has_method(fixture.root.path(), "tasks/cancel"));
        for terminal in ["wrong-id", "cancelled"] {
            let result = manager
                .call_tool("http", "task", Some(json!({"kind": terminal})))
                .await;
            if terminal == "wrong-id" {
                assert!(matches!(result, Err(McpError::Malformed(reason))
                    if reason.contains("different task")));
                assert!(has_method(fixture.root.path(), "tasks/cancel"));
            } else {
                assert!(result.is_err());
            }
        }
        manager.shutdown_all().await;
    }
}

struct TaskForm<'a> {
    label: &'a str,
    content: Value,
    count: AtomicUsize,
}

impl<'a> TaskForm<'a> {
    fn new(label: &'a str, content: Value) -> Self {
        Self {
            label,
            content,
            count: AtomicUsize::new(0),
        }
    }
}

#[async_trait::async_trait]
impl McpElicitationHandler for TaskForm<'_> {
    async fn elicit(
        &self,
        _: &str,
        request: McpElicitation,
        _: CancellationToken,
    ) -> McpElicitationReply {
        let McpElicitation::Form { message, .. } = request else {
            panic!("fixture should send a form");
        };
        assert_eq!(message, self.label);
        self.count.fetch_add(1, Ordering::Relaxed);
        McpElicitationReply {
            action: McpElicitationAction::Accept,
            content: Some(self.content.clone()),
        }
    }
}

async fn call_task(
    manager: &McpConnectionManager,
    kind: &str,
    label: &str,
    handler: Option<&dyn McpElicitationHandler>,
) -> Value {
    let result = manager
        .call_tool_with_elicitation(
            "http",
            "task",
            Some(json!({"kind": kind, "label": label})),
            handler,
        )
        .await
        .unwrap();
    serde_json::from_str(&text_content(result)).unwrap()
}

#[tokio::test]
async fn modern_tasks_deliver_forms_and_roots_once_per_input_key() {
    let (fixture, manager) = task_fixture("modern").await;
    let form = TaskForm::new("form", json!({"answer": "approved"}));
    let picker = TaskForm::new("picker", json!({"template": "week"}));
    let (form_result, picker_result) = tokio::join!(
        call_task(&manager, "input", form.label, Some(&form)),
        call_task(&manager, "picker", picker.label, Some(&picker)),
    );
    assert_eq!(
        form_result,
        json!({"action": "accept", "content": form.content})
    );
    assert_eq!(
        picker_result,
        json!({"action": "accept", "content": picker.content})
    );
    assert_eq!(form.count.load(Ordering::Relaxed), 1);
    assert_eq!(picker.count.load(Ordering::Relaxed), 1);
    assert_eq!(method_count(fixture.root.path(), "tasks/update"), 2);
    assert_eq!(method_count(fixture.root.path(), "tasks/get"), 8);
    assert_eq!(
        call_task(&manager, "input", "no handler", None).await,
        json!({"action": "decline"})
    );
    manager.shutdown_all().await;
}

#[tokio::test]
async fn legacy_task_inputs_route_over_concurrent_post_and_standalone_streams() {
    struct HeldForm {
        started: CancellationToken,
        release: CancellationToken,
    }
    #[async_trait::async_trait]
    impl McpElicitationHandler for HeldForm {
        async fn elicit(
            &self,
            _: &str,
            request: McpElicitation,
            _: CancellationToken,
        ) -> McpElicitationReply {
            assert!(
                matches!(request, McpElicitation::Form { message, .. } if message == "standalone")
            );
            self.started.cancel();
            self.release.cancelled().await;
            McpElicitationReply {
                action: McpElicitationAction::Accept,
                content: Some(json!({"answer":"standalone answer"})),
            }
        }
    }

    let (fixture, manager) = task_fixture("legacy").await;
    let standalone = HeldForm {
        started: CancellationToken::new(),
        release: CancellationToken::new(),
    };
    let post = TaskForm::new("post", json!({"answer":"post answer"}));
    let (standalone_result, ()) = tokio::join!(
        call_task(&manager, "standalone", "standalone", Some(&standalone)),
        async {
            tokio::time::timeout(Duration::from_secs(1), standalone.started.cancelled())
                .await
                .expect("a standalone stream should deliver the task input");
            let (post_result, cross_result) = tokio::join!(
                call_task(&manager, "input", post.label, Some(&post)),
                call_task(&manager, "cross", "crossed stream", Some(&post)),
            );
            assert_eq!(
                post_result,
                json!({"action": "accept", "content": post.content})
            );
            assert_eq!(cross_result["code"], -32602);
            assert_eq!(post.count.load(Ordering::Relaxed), 1);
            standalone.release.cancel();
        },
    );
    assert_eq!(
        standalone_result,
        json!({"action":"accept","content":{"answer":"standalone answer"}}),
    );
    assert!(has_method(fixture.root.path(), "GET /mcp"));
    assert!(!has_method(fixture.root.path(), "tasks/update"));
    manager.shutdown_all().await;
}

#[tokio::test]
async fn legacy_stdio_tasks_use_the_exact_id_with_multiple_pending_results() {
    let root = tempfile::tempdir().unwrap();
    let mut spec = stdio_spec(root.path(), "legacy", "tools", false);
    let ServerSpec::Stdio { args, .. } = &mut spec else {
        unreachable!()
    };
    args.push("--tasks".into());
    let manager = manager();
    manager.add_server("http", spec).await.unwrap();
    let first = TaskForm::new("first", json!({"answer":"one"}));
    let second = TaskForm::new("second", json!({"answer":"two"}));
    let (one, two) = tokio::join!(
        call_task(&manager, "input", first.label, Some(&first)),
        call_task(&manager, "input", second.label, Some(&second)),
    );
    assert_eq!(one, json!({"action": "accept", "content": first.content}));
    assert_eq!(two, json!({"action": "accept", "content": second.content}));
    let untagged = call_task(&manager, "untagged", first.label, Some(&first)).await;
    assert_eq!(untagged["code"], -32602);
    assert_eq!(first.count.load(Ordering::Relaxed), 1);
    manager.shutdown_all().await;
}

#[tokio::test]
async fn task_input_wait_does_not_use_the_active_timeout() {
    struct UserAnswer;
    #[async_trait::async_trait]
    impl McpElicitationHandler for UserAnswer {
        async fn elicit(
            &self,
            _: &str,
            _: McpElicitation,
            _: CancellationToken,
        ) -> McpElicitationReply {
            agena_async::permission_wait::waiting(tokio::time::sleep(Duration::from_millis(700)))
                .await;
            McpElicitationReply {
                action: McpElicitationAction::Accept,
                content: Some(json!({"answer":"ready"})),
            }
        }
    }
    for protocol in ["modern", "legacy"] {
        let fixture = HttpFixture::start_process(protocol, &["--tasks"]).await;
        let manager = manager();
        manager
            .add_server_with_timeouts(
                "http",
                fixture.spec(None),
                Duration::from_secs(4),
                Duration::from_millis(500),
            )
            .await
            .unwrap();
        let answer = agena_async::permission_wait::scope(
            Some((1278, 47)),
            call_task(&manager, "input", "slow form", Some(&UserAnswer)),
        )
        .await;
        assert_eq!(
            answer,
            json!({"action":"accept","content":{"answer":"ready"}})
        );
        assert!(!has_method(fixture.root.path(), "tasks/cancel"));
        manager.shutdown_all().await;
    }
}

#[tokio::test]
async fn tasks_cancel_after_a_local_timeout_or_a_dropped_tool_call() {
    for protocol in ["modern", "legacy"] {
        let fixture = HttpFixture::start_process(protocol, &["--tasks"]).await;
        let manager = Arc::new(manager());
        manager
            .add_server_with_timeouts(
                "http",
                fixture.spec(None),
                Duration::from_secs(4),
                Duration::from_millis(500),
            )
            .await
            .unwrap();
        let pending = json!({"kind":"working"});
        assert!(matches!(
            manager
                .call_tool("http", "task", Some(pending.clone()))
                .await,
            Err(McpError::Timeout(_))
        ));
        let root = fixture.root.path();
        assert_eq!(method_count(root, "tasks/cancel"), 1);
        let initial_gets = method_count(root, "tasks/get");
        let client = Arc::clone(&manager);
        let call =
            tokio::spawn(async move { client.call_tool("http", "task", Some(pending)).await });
        tokio::time::timeout(Duration::from_secs(1), async {
            while method_count(root, "tasks/get") == initial_gets {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        call.abort();
        assert!(call.await.unwrap_err().is_cancelled());
        tokio::time::timeout(Duration::from_secs(1), async {
            while method_count(root, "tasks/cancel") < 2 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(method_count(root, "tools/call"), 2);
        manager.shutdown_all().await;
    }
}
