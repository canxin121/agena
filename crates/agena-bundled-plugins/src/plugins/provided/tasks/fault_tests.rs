use super::*;
use agena_plugin_host::sdk::host_api::*;
use agena_plugin_host::sdk::{EventEnvelope, EventFilter};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

#[derive(Default)]
struct FaultHost {
    fail: AtomicBool,
    runs: AtomicUsize,
    storage: Mutex<BTreeMap<String, String>>,
    block: AtomicBool,
    entered: Notify,
}
#[async_trait::async_trait]
impl HostClient for FaultHost {
    async fn log(&self, _: LogLevel, _: String, _: serde_json::Value) {}
    async fn publish_event(&self, _: EventEnvelope) -> SdkResult<()> {
        Ok(())
    }
    async fn subscribe_events(&self, _: EventFilter) -> SdkResult<EventSubscription> {
        Ok(EventSubscription { id: "test".into() })
    }
    async fn read_config(&self, _: Option<String>) -> SdkResult<serde_json::Value> {
        Ok(serde_json::Value::Null)
    }
    async fn invoke_tool(&self, _: String, _: serde_json::Value) -> SdkResult<ToolInvokeOutput> {
        unreachable!()
    }
    async fn storage_list(&self, _: HostStorageListRequest) -> SdkResult<HostStorageListResponse> {
        Ok(HostStorageListResponse::default())
    }
    async fn storage_set(&self, req: HostStorageSetRequest) -> SdkResult<()> {
        if self.block.load(Ordering::SeqCst) {
            self.entered.notify_one();
            std::future::pending::<()>().await;
        }
        if self.fail.load(Ordering::SeqCst) {
            return Err(agena_plugin_host::PluginError::internal(
                "injected persistence failure",
            ));
        }
        self.storage.lock().unwrap().insert(req.key, req.value);
        Ok(())
    }
    async fn run_subtask(&self, _: RunSubtaskRequest) -> SdkResult<RunSubtaskResponse> {
        self.runs.fetch_add(1, Ordering::SeqCst);
        std::future::pending().await
    }
    async fn cancel_subtask(&self, req: CancelSubtaskRequest) -> SdkResult<SubtaskControlResponse> {
        Ok(SubtaskControlResponse {
            task_id: req.task_id,
            session_id: 90,
            accepted: false,
        })
    }
}
async fn fixture() -> (tempfile::TempDir, Arc<TasksPlugin>, Arc<FaultHost>) {
    let dir = tempfile::tempdir().unwrap();
    let host = Arc::new(FaultHost::default());
    let plugin = Arc::new(TasksPlugin::new());
    plugin
        .init(
            InitContext {
                agena_version: "test".into(),
                workspace_root: dir.path().to_path_buf(),
                plugin_id: TASKS_PLUGIN_ID.parse().unwrap(),
                host_callback_url: None,
                host_callback_token: None,
                settings: serde_json::Value::Null,
                protocol_version: agena_plugin_host::sdk::rpc::PROTOCOL_VERSION,
            },
            host.clone(),
        )
        .await
        .unwrap();
    (dir, plugin, host)
}
fn state(parent: i64, id: &str, status: &str) -> AsyncTaskState {
    serde_json::from_value(serde_json::json!({"task_id":id,"parent_session_id":parent,"description":"fixture","prompt":"original","access":"read_only","status":status,"started_at_ms":1,"finished_at_ms":null,"response":null,"error":null,"run_epoch":1})).unwrap()
}
fn context<'a>(dir: &'a std::path::Path, method: &'static str) -> ToolInvokeContext<'a> {
    ToolInvokeContext {
        tool_name: method,
        session_id: 41,
        call_id: 1,
        workspace_root: dir.to_str().unwrap(),
    }
}
#[tokio::test]
async fn failed_run_and_followup_persistence_restore_registry() {
    let (dir, plugin, host) = fixture().await;
    host.fail.store(true, Ordering::SeqCst);
    let input=TaskToolInput::parse_input(serde_json::json!({"description":"fixture","prompt":"not executed","task_id":"same","run_in_background":true})).unwrap();
    assert!(
        plugin
            .run(&input, &context(dir.path(), "run"))
            .await
            .is_err()
    );
    assert!(plugin.tasks.lock().unwrap().is_empty());
    assert_eq!(host.runs.load(Ordering::SeqCst), 0);
    let prior = plugin
        .reserve_task(state(41, "same", "completed"), false)
        .unwrap()
        .commit();
    let followup: TaskFollowupInput = serde_json::from_value(
        serde_json::json!({"task_id":"same","prompt":"new","timeout_ms":456}),
    )
    .unwrap();
    assert!(
        plugin
            .followup(&followup, &context(dir.path(), "followup"))
            .await
            .is_err()
    );
    assert!(Arc::ptr_eq(
        &task_entry_for_parent(&plugin.tasks, "same", 41).unwrap(),
        &prior
    ));
    assert_eq!(recover_task_state(&prior).run_epoch, 1);
    assert_eq!(host.runs.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn cancelled_admission_releases_reserved_capacity() {
    let (dir, plugin, host) = fixture().await;
    host.block.store(true, Ordering::SeqCst);
    let p = plugin.clone();
    let root = dir.path().to_path_buf();
    let task = tokio::spawn(async move {
        let input=TaskToolInput::parse_input(serde_json::json!({"description":"fixture","prompt":"not executed","run_in_background":true})).unwrap();
        p.run(&input, &context(&root, "run")).await
    });
    tokio::time::timeout(std::time::Duration::from_secs(2), host.entered.notified())
        .await
        .unwrap();
    task.abort();
    let _ = task.await;
    assert!(plugin.tasks.lock().unwrap().is_empty());
    assert_eq!(host.runs.load(Ordering::SeqCst), 0);
}
#[test]
fn concurrent_reservations_cannot_overbook_or_collide_across_parents() {
    let plugin = Arc::new(TasksPlugin::new());
    let mut threads = Vec::new();
    for i in 0..32 {
        let p = plugin.clone();
        threads.push(std::thread::spawn(move || {
            p.reserve_task(state(41, &format!("task-{i}"), "running"), false)
                .map(|r| r.commit())
                .is_ok()
        }));
    }
    assert_eq!(
        threads
            .into_iter()
            .map(|t| t.join().unwrap())
            .filter(|ok| *ok)
            .count(),
        8
    );
    plugin
        .reserve_task(state(42, "task-0", "running"), false)
        .unwrap()
        .commit();
    assert!(task_entry_for_parent(&plugin.tasks, "task-0", 43).is_err());
}
#[tokio::test]
async fn rejected_cancel_preserves_running_state() {
    let (dir, plugin, _) = fixture().await;
    plugin
        .reserve_task(state(41, "task", "running"), false)
        .unwrap()
        .commit();
    plugin
        .cancel(
            &TaskIdInput {
                task_id: "task".into(),
            },
            &context(dir.path(), "cancel"),
        )
        .await
        .unwrap();
    assert_eq!(
        entry_state_for_parent(&plugin.tasks, "task", 41)
            .unwrap()
            .status,
        "running"
    );
}
