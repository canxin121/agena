//! End-to-end verification that creating a plan registers the `plan:{session}`
//! status-line display contribution the TUI reads for its bottom-right chip.
//!
//! Regression guard for "plan created but the bottom-right progress chip never
//! appears": composes a real `PluginHost` with the in-process `agena.plan`
//! plugin and a fake host client, invokes `agena.plan.set` through the normal
//! tool path, and asserts the contribution lands in
//! `PluginHost::display_contributions()` keyed by the callback-context session.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use agena_plugin_host::registry::RegisteredTool;
use agena_plugin_host::sdk::Result as SdkResult;
use agena_plugin_host::sdk::host_api::{
    AskUserRequest, AskUserResponse, EventSubscription, HostClient, HostGetSessionRequest,
    HostGetSessionResponse, HostSession, HostStorageDeleteRequest, HostStorageGetRequest,
    HostStorageGetResponse, HostStorageScope, HostStorageSetRequest, HostStorageVisibility,
    LogLevel, current_host_callback_context,
};
use agena_plugin_host::sdk::{
    ContributionKind, EventEnvelope, EventFilter, PluginDisplayContent, ToolInvokeInput,
    ToolInvokeOutput,
};
use agena_plugin_host::{
    ConfiguredPlugin, PluginHost, PluginHostBuildConfig, PluginsConfig, StaticPluginRegistration,
};

/// Fake host. `get_session` resolves the session id from the plugin's callback
/// context, mirroring `RuntimeHostClient`; storage is an in-memory map; the
/// plan review is auto-approved so `set` moves the plan to `active`.
#[derive(Default)]
struct FakeHostClient {
    storage: Mutex<BTreeMap<String, String>>,
    ask_user_calls: Mutex<usize>,
}

fn storage_key(
    scope: HostStorageScope,
    visibility: HostStorageVisibility,
    namespace: &str,
    key: &str,
) -> String {
    format!("{scope:?}:{visibility:?}:{namespace}:{key}")
}

#[async_trait::async_trait]
impl HostClient for FakeHostClient {
    async fn log(&self, _level: LogLevel, _message: String, _fields: serde_json::Value) {}

    async fn publish_event(&self, _env: EventEnvelope) -> SdkResult<()> {
        Ok(())
    }

    async fn subscribe_events(&self, _filter: EventFilter) -> SdkResult<EventSubscription> {
        Ok(EventSubscription {
            id: "unused".to_string(),
        })
    }

    async fn read_config(&self, _path: Option<String>) -> SdkResult<serde_json::Value> {
        Ok(serde_json::Value::Null)
    }

    async fn invoke_tool(
        &self,
        _tool: String,
        _input: serde_json::Value,
    ) -> SdkResult<ToolInvokeOutput> {
        Ok(ToolInvokeOutput::text("unused"))
    }

    async fn ask_user(&self, _req: AskUserRequest) -> SdkResult<AskUserResponse> {
        *self.ask_user_calls.lock().unwrap() += 1;
        Ok(AskUserResponse {
            answers: BTreeMap::from([("decision".to_string(), vec!["Approve".to_string()])]),
            ..AskUserResponse::default()
        })
    }

    async fn get_session(&self, _req: HostGetSessionRequest) -> SdkResult<HostGetSessionResponse> {
        let session_id = current_host_callback_context()
            .and_then(|ctx| ctx.session_id)
            .unwrap_or(42);
        Ok(HostGetSessionResponse {
            session: HostSession {
                id: session_id,
                parent_id: None,
                root_id: session_id,
                workspace_id: 1,
                title: "test session".to_string(),
                is_subagent: false,
            },
        })
    }

    async fn storage_get(&self, req: HostStorageGetRequest) -> SdkResult<HostStorageGetResponse> {
        let key = storage_key(req.scope, req.visibility, &req.namespace, &req.key);
        let value = self.storage.lock().unwrap().get(&key).cloned();
        Ok(HostStorageGetResponse { value })
    }

    async fn storage_set(&self, req: HostStorageSetRequest) -> SdkResult<()> {
        let key = storage_key(req.scope, req.visibility, &req.namespace, &req.key);
        self.storage.lock().unwrap().insert(key, req.value);
        Ok(())
    }

    async fn storage_delete(&self, req: HostStorageDeleteRequest) -> SdkResult<()> {
        let key = storage_key(req.scope, req.visibility, &req.namespace, &req.key);
        self.storage.lock().unwrap().remove(&key);
        Ok(())
    }
}

async fn build_host(tmp: &tempfile::TempDir, host_client: Arc<dyn HostClient>) -> Arc<PluginHost> {
    build_host_with_previous(tmp, host_client, None, HashMap::new()).await
}

/// Like [`build_host`] but allows simulating a runtime reload / process
/// restart by building a successor host from a previous one. The successor
/// shares the same host client (durable storage) but gets a fresh in-memory
/// display contribution map. `previous_plugins` mirrors what
/// `RuntimeSnapshot::build_inner` derives from the previous `PluginsConfig`
/// (`PluginHostBuildConfig::previous_plugins`) so the hot-reload transport
/// reuse path is exercised when the config is byte-identical.
async fn build_host_with_previous(
    tmp: &tempfile::TempDir,
    host_client: Arc<dyn HostClient>,
    previous: Option<Arc<PluginHost>>,
    previous_plugins: HashMap<String, ConfiguredPlugin>,
) -> Arc<PluginHost> {
    let mut list = BTreeMap::new();
    list.insert("agena.plan".to_string(), ConfiguredPlugin::static_default());
    PluginHost::new(PluginHostBuildConfig {
        static_plugins: vec![StaticPluginRegistration::new(
            "agena.plan".parse().unwrap(),
            agena_bundled_plugins::tool::new_plan_plugin(),
        )],
        config: PluginsConfig {
            list,
            ..Default::default()
        },
        workspace_root: tmp.path().to_path_buf(),
        agena_version: "test".to_string(),
        callback_base_url: None,
        host_client: Some(host_client),
        previous,
        previous_plugins,
    })
    .await
    .unwrap()
}

/// `agena.plan.set` with two AI steps must register a `plan:{session_id}`
/// status-line contribution while the plan is still in `planning` (default:
/// no review). A follow-up `agena.plan.review` — the only tool that requests
/// user approval — is auto-approved by the fake host and moves the plan to
/// `active`, updating the chip to show active progress.
#[tokio::test]
async fn plan_set_registers_status_line_contribution() {
    let tmp = tempfile::tempdir().unwrap();
    let client = Arc::new(FakeHostClient::default());
    let host = build_host(&tmp, Arc::clone(&client) as Arc<dyn HostClient>).await;

    let registered: RegisteredTool = host.lookup_tool("agena.plan.set").unwrap();
    let input = ToolInvokeInput {
        tool_name: "set".to_string(),
        session_id: 42,
        call_id: 7,
        workspace_root: tmp.path().to_string_lossy().to_string(),
        input: serde_json::json!({
            "objective": "Build a widget",
            "steps": [
                { "title": "Design the widget" },
                { "title": "Implement the widget" },
            ],
        }),
    };

    host.invoke_tool(&registered, input, None)
        .await
        .expect("agena.plan.set must succeed");

    let contributions = host.display_contributions();
    let plan_contribution = contributions
        .iter()
        .find(|c| {
            c.plugin_id.to_string() == "agena.plan"
                && c.contribution.id == "plan:42"
                && c.contribution.kind == ContributionKind::StatusLineText
        })
        .unwrap_or_else(|| {
            panic!("expected plan:42 status-line contribution, got: {contributions:#?}")
        });

    match &plan_contribution.contribution.content {
        PluginDisplayContent::Text { text } => {
            assert_eq!(text.trim(), "⏳ 0/2 ↻");
        }
        other => panic!("expected Text content, got {other:?}"),
    }

    // plan.review requests approval and (auto-approved here) moves to active.
    assert_eq!(
        *client.ask_user_calls.lock().unwrap(),
        0,
        "plan.set must not request approval by itself"
    );
    let review: RegisteredTool = host.lookup_tool("agena.plan.review").unwrap();
    host.invoke_tool(
        &review,
        ToolInvokeInput {
            tool_name: "review".to_string(),
            session_id: 42,
            call_id: 8,
            workspace_root: tmp.path().to_string_lossy().to_string(),
            input: serde_json::json!({}),
        },
        None,
    )
    .await
    .expect("agena.plan.review must succeed");

    let contributions = host.display_contributions();
    let plan_contribution = contributions
        .iter()
        .find(|c| {
            c.plugin_id.to_string() == "agena.plan"
                && c.contribution.id == "plan:42"
                && c.contribution.kind == ContributionKind::StatusLineText
        })
        .unwrap_or_else(|| {
            panic!("expected plan:42 status-line contribution, got: {contributions:#?}")
        });
    match &plan_contribution.contribution.content {
        PluginDisplayContent::Text { text } => {
            assert_eq!(text.trim(), "▶ 0/2 ↻");
        }
        other => panic!("expected Text content, got {other:?}"),
    }
}

/// An active autorun plan's `agent.stop` hook blocks the stop and returns a
/// continuation. The continuation must be carried by the recorded hook run's
/// `message` field (and its detail), so the session runtime can surface it
/// inside the hook activity instead of injecting a separate assistant message.
#[tokio::test]
async fn agent_stop_continuation_rides_the_hook_run_message() {
    let tmp = tempfile::tempdir().unwrap();
    let client = Arc::new(FakeHostClient::default());
    let host = build_host(&tmp, Arc::clone(&client) as Arc<dyn HostClient>).await;

    let set: RegisteredTool = host.lookup_tool("agena.plan.set").unwrap();
    host.invoke_tool(
        &set,
        ToolInvokeInput {
            tool_name: "set".to_string(),
            session_id: 42,
            call_id: 7,
            workspace_root: tmp.path().to_string_lossy().to_string(),
            input: serde_json::json!({
                "objective": "Build a widget",
                "steps": [{ "title": "Design the widget" }],
            }),
        },
        None,
    )
    .await
    .expect("agena.plan.set must succeed");
    let review: RegisteredTool = host.lookup_tool("agena.plan.review").unwrap();
    host.invoke_tool(
        &review,
        ToolInvokeInput {
            tool_name: "review".to_string(),
            session_id: 42,
            call_id: 8,
            workspace_root: tmp.path().to_string_lossy().to_string(),
            input: serde_json::json!({}),
        },
        None,
    )
    .await
    .expect("agena.plan.review must succeed");

    let patch = host
        .dispatch_agent_stop(agena_plugin_host::AgentStopInput {
            session_id: 42,
            stop_hook_active: false,
            last_assistant_message: None,
            run_error: None,
        })
        .await
        .expect("agent.stop dispatch must succeed");

    let continuation = patch
        .continue_with_message
        .expect("an active autorun plan blocks the stop with a continuation");
    assert!(
        continuation.contains("<plan_context>") && continuation.contains("Design the widget"),
        "the continuation is the plan autorun prompt: {continuation}"
    );

    let drained = host.drain_hook_runs(42);
    let blocking = drained
        .iter()
        .find(|record| record.hook == "agent.stop")
        .expect("the agent.stop hook run was recorded");
    assert_eq!(
        blocking.message.as_deref(),
        Some(continuation.as_str()),
        "the continuation rides the hook run's message field"
    );
    assert_eq!(
        blocking.detail.as_deref(),
        Some(continuation.as_str()),
        "the continuation stays visible as the activity detail"
    );
}

/// A user cancellation must turn off plan autorun before the next execution
/// can be started. Otherwise the runtime's cancellation signal is followed by
/// the old `agent.stop` autorun decision and the plan immediately relaunches.
#[tokio::test]
async fn agent_cancel_disables_plan_autorun() {
    let tmp = tempfile::tempdir().unwrap();
    let client = Arc::new(FakeHostClient::default());
    let host = build_host(&tmp, Arc::clone(&client) as Arc<dyn HostClient>).await;

    let set: RegisteredTool = host.lookup_tool("agena.plan.set").unwrap();
    host.invoke_tool(
        &set,
        ToolInvokeInput {
            tool_name: "set".to_string(),
            session_id: 42,
            call_id: 7,
            workspace_root: tmp.path().to_string_lossy().to_string(),
            input: serde_json::json!({
                "objective": "Build a widget",
                "request_approval": false,
                "steps": [{ "title": "Design the widget" }],
            }),
        },
        None,
    )
    .await
    .expect("active autorun plan must be created");

    host.dispatch_agent_cancel(agena_plugin_host::AgentCancelInput {
        session_id: 42,
        execution_id: "execution-42".to_string(),
    })
    .await;

    let key = storage_key(
        HostStorageScope::Session,
        HostStorageVisibility::Shared,
        "workflow_plan",
        "active",
    );
    let stored = client
        .storage
        .lock()
        .unwrap()
        .get(&key)
        .cloned()
        .expect("the active plan remains stored");
    let stored_plan: serde_json::Value = serde_json::from_str(&stored).unwrap();
    assert_eq!(stored_plan["autorun"], serde_json::Value::Bool(false));

    let patch = host
        .dispatch_agent_stop(agena_plugin_host::AgentStopInput {
            session_id: 42,
            stop_hook_active: false,
            last_assistant_message: None,
            run_error: None,
        })
        .await
        .expect("agent.stop dispatch must succeed after cancellation");
    assert!(
        patch.continue_with_message.is_none(),
        "cancelled plan autorun must not relaunch the plan"
    );
}

/// A hot-reload (`Runtime::reload_with_cause` with byte-identical config)
/// builds a successor host whose `previous_plugins` match the current config,
/// so the host's transport-reuse path is taken. In-proc `Static` plugins must
/// NOT be reused: a static plugin instance binds its host during `meta/init`,
/// and reusing the transport keeps every display write on the detached
/// previous handle. The successor must recreate the plan plugin against its
/// own handle, so `plan.set` through the SECOND host lands `plan:{session}` on
/// the SECOND host's contribution map (this failed before the static-reuse
/// guard: the plugin wrote to the first host instead).
#[tokio::test]
async fn hot_reload_recreates_static_plan_plugin_against_successor_host() {
    let tmp = tempfile::tempdir().unwrap();
    let shared_client = Arc::new(FakeHostClient::default());

    // First host: create a plan so durable storage holds it.
    let first = build_host(&tmp, Arc::clone(&shared_client) as Arc<dyn HostClient>).await;
    let set: RegisteredTool = first.lookup_tool("agena.plan.set").unwrap();
    first
        .invoke_tool(
            &set,
            ToolInvokeInput {
                tool_name: "set".to_string(),
                session_id: 42,
                call_id: 7,
                workspace_root: tmp.path().to_string_lossy().to_string(),
                input: serde_json::json!({
                    "objective": "Build a widget",
                    "request_approval": false,
                    "steps": [
                        { "title": "Design the widget" },
                        { "title": "Implement the widget" },
                    ],
                }),
            },
            None,
        )
        .await
        .expect("agena.plan.set must succeed");
    assert!(
        first
            .display_contributions()
            .iter()
            .any(|c| c.contribution.id == "plan:42"),
        "precondition: plan:42 must exist on the first host"
    );

    // Hot-reload: byte-identical config + the previous host, exactly how
    // RuntimeSnapshot::build_inner derives previous_plugins.
    let mut list = BTreeMap::new();
    list.insert("agena.plan".to_string(), ConfiguredPlugin::static_default());
    let previous_config = PluginsConfig {
        list,
        ..Default::default()
    };
    let second = build_host_with_previous(
        &tmp,
        Arc::clone(&shared_client) as Arc<dyn HostClient>,
        Some(Arc::clone(&first)),
        PluginHostBuildConfig::previous_plugins(&previous_config),
    )
    .await;
    assert!(
        !second
            .display_contributions()
            .iter()
            .any(|c| c.contribution.id == "plan:42"),
        "precondition: a rebuilt host starts without the in-memory plan contribution"
    );

    // Update the plan through the SECOND host. With the reuse bug the plugin
    // would still write to the FIRST host's map and this lookup would fail.
    let set2: RegisteredTool = second.lookup_tool("agena.plan.set").unwrap();
    second
        .invoke_tool(
            &set2,
            ToolInvokeInput {
                tool_name: "set".to_string(),
                session_id: 42,
                call_id: 9,
                workspace_root: tmp.path().to_string_lossy().to_string(),
                input: serde_json::json!({
                    "objective": "Rebuild the widget",
                    "request_approval": false,
                    "steps": [
                        { "title": "Design the rebuild" },
                        { "title": "Implement the rebuild" },
                    ],
                }),
            },
            None,
        )
        .await
        .expect("agena.plan.set on the successor host must succeed");

    let contributions = second.display_contributions();
    let plan_contribution = contributions
        .iter()
        .find(|c| {
            c.plugin_id.to_string() == "agena.plan"
                && c.contribution.id == "plan:42"
                && c.contribution.kind == ContributionKind::StatusLineText
        })
        .unwrap_or_else(|| {
            panic!(
                "plan.set on the successor host must register plan:42 on the LIVE host (not the detached previous one), got: {contributions:#?}"
            )
        });
    match &plan_contribution.contribution.content {
        PluginDisplayContent::Text { text } => {
            assert_eq!(text.trim(), "▶ 0/2 ↻");
        }
        other => panic!("expected Text content, got {other:?}"),
    }
}

/// `request_approval: false` must move the plan straight to `active` without
/// asking the user: the review path (host ask_user) is never entered. This is
/// the "user pre-declared no approval needed, start immediately" path.
#[tokio::test]
async fn plan_set_skips_review_when_request_approval_is_false() {
    let tmp = tempfile::tempdir().unwrap();
    let client = Arc::new(FakeHostClient::default());
    let host = build_host(&tmp, Arc::clone(&client) as Arc<dyn HostClient>).await;

    let registered: RegisteredTool = host.lookup_tool("agena.plan.set").unwrap();
    let input = ToolInvokeInput {
        tool_name: "set".to_string(),
        session_id: 42,
        call_id: 7,
        workspace_root: tmp.path().to_string_lossy().to_string(),
        input: serde_json::json!({
            "objective": "Build a widget",
            "request_approval": false,
            "steps": [
                { "title": "Design the widget" },
                { "title": "Implement the widget" },
            ],
        }),
    };

    host.invoke_tool(&registered, input, None)
        .await
        .expect("agena.plan.set with request_approval=false must succeed");

    assert_eq!(
        *client.ask_user_calls.lock().unwrap(),
        0,
        "no review should be requested when request_approval is false"
    );

    let contributions = host.display_contributions();
    let plan_contribution = contributions
        .iter()
        .find(|c| {
            c.plugin_id.to_string() == "agena.plan"
                && c.contribution.id == "plan:42"
                && c.contribution.kind == ContributionKind::StatusLineText
        })
        .unwrap_or_else(|| {
            panic!("expected plan:42 status-line contribution, got: {contributions:#?}")
        });
    match &plan_contribution.contribution.content {
        PluginDisplayContent::Text { text } => {
            assert_eq!(text.trim(), "▶ 0/2 ↻");
        }
        other => panic!("expected Text content, got {other:?}"),
    }
}

/// Clearing the plan (via plan.set after approval path is not used here — this
/// exercises `plan:get` after `set` plus a second `set` to confirm the same
/// contribution id is reused for the same session rather than duplicated).
#[tokio::test]
async fn plan_contribution_is_keyed_per_session() {
    let tmp = tempfile::tempdir().unwrap();
    let host = build_host(&tmp, Arc::new(FakeHostClient::default())).await;

    let registered: RegisteredTool = host.lookup_tool("agena.plan.set").unwrap();
    for call_id in [7i64, 8i64] {
        let input = ToolInvokeInput {
            tool_name: "set".to_string(),
            session_id: 42,
            call_id,
            workspace_root: tmp.path().to_string_lossy().to_string(),
            input: serde_json::json!({
                "objective": "Build a widget",
                "steps": [
                    { "title": "Design the widget" },
                    { "title": "Implement the widget" },
                ],
            }),
        };
        host.invoke_tool(&registered, input, None)
            .await
            .expect("agena.plan.set must succeed");
    }

    let plan_contributions = host
        .display_contributions()
        .into_iter()
        .filter(|c| c.contribution.id == "plan:42")
        .collect::<Vec<_>>();
    assert_eq!(plan_contributions.len(), 1, "one contribution per session");
}

/// A different session id yields a different contribution id, so the TUI never
/// renders another session's plan in the active session's chip.
#[tokio::test]
async fn plan_contribution_is_qualified_by_session() {
    let tmp = tempfile::tempdir().unwrap();
    let host = build_host(&tmp, Arc::new(FakeHostClient::default())).await;

    let registered: RegisteredTool = host.lookup_tool("agena.plan.set").unwrap();
    for session_id in [42i64, 43i64] {
        let input = ToolInvokeInput {
            tool_name: "set".to_string(),
            session_id,
            call_id: session_id,
            workspace_root: tmp.path().to_string_lossy().to_string(),
            input: serde_json::json!({
                "objective": "Build a widget",
                "steps": [
                    { "title": "Design the widget" },
                    { "title": "Implement the widget" },
                ],
            }),
        };
        host.invoke_tool(&registered, input, None)
            .await
            .expect("agena.plan.set must succeed");
    }

    let ids = host
        .display_contributions()
        .into_iter()
        .filter(|c| c.plugin_id.to_string() == "agena.plan")
        .map(|c| c.contribution.id)
        .collect::<Vec<_>>();
    assert!(ids.contains(&"plan:42".to_string()));
    assert!(ids.contains(&"plan:43".to_string()));
}

/// Routing through the real `ToolExecutor::execute_invocation_detailed` — the
/// exact method `SessionManager::execute_session_tool` uses to run application
/// tools (plan viewer autorun toggle, session tool execution) — must still land
/// the plan contribution on the shared host.
#[tokio::test]
async fn plan_contribution_survives_real_tool_executor_route() {
    use agena_domain::{StructuredObject, ToolInvocation};
    use agena_runtime_tools::tool::ToolExecutor;
    use agena_runtime_tools::{authorization, permission};

    let tmp = tempfile::tempdir().unwrap();
    let host = build_host(&tmp, Arc::new(FakeHostClient::default())).await;

    let executor = ToolExecutor::new(
        tmp.path().to_path_buf(),
        authorization::ExecutionPrincipal::new(
            permission::PermissionPolicy::allow_all(),
            permission::ToolPermissionPolicy::allow_all(),
        ),
        Arc::clone(&host),
        None,
        None,
        None,
    );

    let invocation = ToolInvocation::plugin_named(
        "plan.set",
        "agena.plan",
        StructuredObject::try_from(serde_json::json!({
            "objective": "Build a widget",
            "request_approval": false,
            "steps": [
                { "title": "Design the widget" },
                { "title": "Implement the widget" },
            ],
        }))
        .expect("valid structured plan input"),
    );
    let execution = executor
        .execute_invocation_detailed(&invocation, 42, 7)
        .await
        .expect("plan.set through the real tool executor must succeed");
    assert!(
        execution.summary().summary.contains("Active"),
        "plan.set with request_approval=false should go straight to active through the executor, got: {:?}",
        execution.summary().summary
    );

    let contributions = host.display_contributions();
    let plan_contribution = contributions
        .iter()
        .find(|c| {
            c.plugin_id.to_string() == "agena.plan"
                && c.contribution.id == "plan:42"
                && c.contribution.kind == ContributionKind::StatusLineText
        })
        .unwrap_or_else(|| {
            panic!(
                "expected plan:42 status-line contribution after real executor route, got: {contributions:#?}"
            )
        });
    match &plan_contribution.contribution.content {
        PluginDisplayContent::Text { text } => {
            assert_eq!(text.trim(), "▶ 0/2 ↻");
        }
        other => panic!("expected Text content, got {other:?}"),
    }
}

/// A plan persisted in durable storage must not silently lose its composer
/// chip when the in-memory display contribution map starts empty — exactly
/// what happens after a process restart or a runtime reload, where a fresh
/// `PluginHost` is built with the same session storage. Reading the plan
/// (`agena.plan.get`, which the TUI also calls to heal the chip) re-publishes
/// the `plan:{session}` contribution.
#[tokio::test]
async fn plan_get_restores_status_line_contribution_after_host_rebuild() {
    let tmp = tempfile::tempdir().unwrap();
    let shared_client = Arc::new(FakeHostClient::default());

    // First host: create a plan. The contribution lands in memory.
    let first = build_host(&tmp, Arc::clone(&shared_client) as Arc<dyn HostClient>).await;
    let registered: RegisteredTool = first.lookup_tool("agena.plan.set").unwrap();
    first
        .invoke_tool(
            &registered,
            ToolInvokeInput {
                tool_name: "set".to_string(),
                session_id: 42,
                call_id: 7,
                workspace_root: tmp.path().to_string_lossy().to_string(),
                input: serde_json::json!({
                    "objective": "Build a widget",
                    "request_approval": false,
                    "steps": [{ "title": "Design the widget" }, { "title": "Implement the widget" }],
                }),
            },
            None,
        )
        .await
        .expect("agena.plan.set must succeed");
    assert!(
        first
            .display_contributions()
            .iter()
            .any(|c| c.contribution.id == "plan:42"),
        "plan:42 contribution must be present right after creation"
    );

    // Simulate restart / reload: a successor host shares the same storage
    // (the fake host client) but starts with an empty contribution map.
    let second = build_host_with_previous(
        &tmp,
        Arc::clone(&shared_client) as Arc<dyn HostClient>,
        Some(Arc::clone(&first)),
        HashMap::new(),
    )
    .await;
    assert!(
        !second
            .display_contributions()
            .iter()
            .any(|c| c.contribution.id == "plan:42"),
        "precondition: a rebuilt host starts without the in-memory plan contribution"
    );

    // Reading the plan through the normal tool path (what the TUI does to
    // heal the chip) must restore the contribution from durable storage.
    let get: RegisteredTool = second.lookup_tool("agena.plan.get").unwrap();
    second
        .invoke_tool(
            &get,
            ToolInvokeInput {
                tool_name: "get".to_string(),
                session_id: 42,
                call_id: 8,
                workspace_root: tmp.path().to_string_lossy().to_string(),
                input: serde_json::json!({ "view": "summary" }),
            },
            None,
        )
        .await
        .expect("agena.plan.get must succeed");

    let contributions = second.display_contributions();
    let plan_contribution = contributions
        .iter()
        .find(|c| {
            c.plugin_id.to_string() == "agena.plan"
                && c.contribution.id == "plan:42"
                && c.contribution.kind == ContributionKind::StatusLineText
        })
        .unwrap_or_else(|| {
            panic!(
                "plan.get must restore the plan:42 contribution on a rebuilt host, got: {contributions:#?}"
            )
        });
    match &plan_contribution.contribution.content {
        PluginDisplayContent::Text { text } => {
            assert_eq!(text.trim(), "▶ 0/2 ↻");
        }
        other => panic!("expected Text content, got {other:?}"),
    }
}
