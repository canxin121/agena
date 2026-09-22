//! Delayed human approval must never restore an edited or deleted plan.
use super::*;
use agena_plugin_host::sdk::host_api::*;
use agena_plugin_host::sdk::{EventEnvelope, EventFilter};
use serde_json::json;
use std::{collections::BTreeMap, sync::Mutex};

struct ReviewHost {
    storage: Mutex<BTreeMap<String, String>>,
    entered: tokio::sync::Notify,
    answer: tokio::sync::Semaphore,
}
impl Default for ReviewHost {
    fn default() -> Self {
        Self {
            storage: Mutex::new(BTreeMap::new()),
            entered: tokio::sync::Notify::new(),
            answer: tokio::sync::Semaphore::new(0),
        }
    }
}
#[async_trait::async_trait]
impl HostClient for ReviewHost {
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
    async fn get_session(&self, _: HostGetSessionRequest) -> SdkResult<HostGetSessionResponse> {
        Ok(HostGetSessionResponse {
            session: HostSession {
                id: 41,
                parent_id: None,
                root_id: 41,
                workspace_id: 1,
                title: "test".into(),
                is_subagent: false,
            },
        })
    }
    async fn storage_get(&self, r: HostStorageGetRequest) -> SdkResult<HostStorageGetResponse> {
        Ok(HostStorageGetResponse {
            value: self.storage.lock().unwrap().get(&r.key).cloned(),
        })
    }
    async fn storage_set(&self, r: HostStorageSetRequest) -> SdkResult<()> {
        self.storage.lock().unwrap().insert(r.key, r.value);
        Ok(())
    }
    async fn storage_delete(&self, r: HostStorageDeleteRequest) -> SdkResult<()> {
        self.storage.lock().unwrap().remove(&r.key);
        Ok(())
    }
    async fn ask_user(&self, _: AskUserRequest) -> SdkResult<AskUserResponse> {
        self.entered.notify_one();
        self.answer.acquire().await.unwrap().forget();
        Ok(AskUserResponse {
            answers: BTreeMap::from([("decision".into(), vec!["Approve".into()])]),
            ..Default::default()
        })
    }
    // Display intentionally unavailable; a durable commit must still succeed.
}
async fn fixture(allow: bool) -> (tempfile::TempDir, Arc<PlanPlugin>, Arc<ReviewHost>) {
    let dir = tempfile::tempdir().unwrap();
    let plugin = Arc::new(PlanPlugin::new());
    let host = Arc::new(ReviewHost::default());
    plugin
        .init(
            InitContext {
                agena_version: "test".into(),
                workspace_root: dir.path().to_path_buf(),
                plugin_id: PLAN_PLUGIN_ID.parse().unwrap(),
                host_callback_url: None,
                host_callback_token: None,
                settings: json!({"allow_unreviewed_activation":allow}),
                protocol_version: agena_plugin_host::sdk::rpc::PROTOCOL_VERSION,
            },
            host.clone(),
        )
        .await
        .unwrap();
    (dir, plugin, host)
}
async fn create(plugin: &PlanPlugin) -> serde_json::Value {
    let input =
        PlanSetInput::parse_input(json!({"objective":"test","steps":[{"title":"one"}]})).unwrap();
    plugin.set(&input).await.unwrap().payload.unwrap()
}
#[tokio::test]
async fn old_approval_cannot_overwrite_an_edited_plan() {
    let (_dir, plugin, host) = fixture(false).await;
    create(&plugin).await;
    let p = plugin.clone();
    let review = tokio::spawn(async move { p.review(&PlanReviewInput::default()).await });
    tokio::time::timeout(std::time::Duration::from_secs(2), host.entered.notified())
        .await
        .unwrap();
    plugin
        .edit(
            &PlanEditInput::parse_input(
                json!({"step":1,"status":"in_progress","note":"new requirement"}),
            )
            .unwrap(),
        )
        .await
        .unwrap();
    host.answer.add_permits(1);
    assert!(review.await.unwrap().is_err());
    let output = plugin
        .get(&PlanGetInput::default())
        .await
        .unwrap()
        .payload
        .unwrap();
    assert_eq!(output["plan"]["phase"], "planning");
    assert_eq!(output["plan"]["steps"][0]["note"], "new requirement");
}
#[tokio::test]
async fn old_approval_cannot_resurrect_a_cleared_plan() {
    let (_dir, plugin, host) = fixture(false).await;
    create(&plugin).await;
    let p = plugin.clone();
    let review = tokio::spawn(async move { p.review(&PlanReviewInput::default()).await });
    tokio::time::timeout(std::time::Duration::from_secs(2), host.entered.notified())
        .await
        .unwrap();
    plugin.clear().await.unwrap();
    host.answer.add_permits(1);
    assert!(review.await.unwrap().is_err());
    assert!(
        plugin
            .get(&PlanGetInput::default())
            .await
            .unwrap()
            .payload
            .unwrap()["plan"]
            .is_null()
    );
}
#[tokio::test]
async fn skip_review_requires_configuration_grant_and_display_failure_does_not_undo_commit() {
    let (_dir, plugin, _) = fixture(false).await;
    let input = PlanSetInput::parse_input(
        json!({"objective":"test","steps":[{"title":"one"}],"request_approval":false}),
    )
    .unwrap();
    assert!(plugin.set(&input).await.is_err());
    assert!(
        plugin
            .get(&PlanGetInput::default())
            .await
            .unwrap()
            .payload
            .unwrap()["plan"]
            .is_null()
    );
    let (_dir, plugin, _) = fixture(true).await;
    let output = plugin.set(&input).await.unwrap();
    assert!(output.output_text.contains("committed"));
    let plan = plugin
        .get(&PlanGetInput::default())
        .await
        .unwrap()
        .payload
        .unwrap();
    assert_eq!(plan["plan"]["phase"], "active");
}
#[tokio::test]
async fn two_edits_of_one_revision_cannot_both_commit() {
    let (_dir, plugin, _) = fixture(false).await;
    let initial = create(&plugin).await;
    let revision = initial
        .get("revision")
        .or_else(|| initial.pointer("/plan/revision"))
        .unwrap()
        .clone();
    let a = PlanEditInput::parse_input(json!({"step":1,"note":"a","expected_revision":revision}))
        .unwrap();
    let b = PlanEditInput::parse_input(json!({"step":1,"note":"b","expected_revision":revision}))
        .unwrap();
    let (a, b) = tokio::join!(plugin.edit(&a), plugin.edit(&b));
    assert_ne!(a.is_ok(), b.is_ok());
}
