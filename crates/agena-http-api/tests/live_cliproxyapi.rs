#![allow(clippy::await_holding_lock)]

use std::{
    collections::BTreeSet,
    fs,
    path::PathBuf,
    process::Command,
    sync::{Arc, LazyLock, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use agena::{
    config::LoadConfigRequest,
    db::init_schema,
    message::Message,
    model::ModelRef,
    provider::{CompletionRequest, CompletionStreamEvent, CompletionUsage, ProviderRegistry},
    role::Role,
    runtime::AgenaRuntime,
};
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Method, Request, StatusCode},
};
use futures_util::{FutureExt, StreamExt};
use sea_orm::Database;
use serde_json::{Value, json};
use tower::ServiceExt;

use agena_http_api::{ApiState, router};

#[derive(Clone)]
struct LiveConfig {
    base_url: String,
    api_key: String,
    model: String,
}

struct LiveHarness {
    config: LiveConfig,
    provider_registry: Arc<ProviderRegistry>,
    app: Router,
    workspace_root: PathBuf,
}

#[derive(Debug)]
struct StreamObservation {
    text: String,
    text_delta_count: usize,
    completion_count: usize,
    usage: Option<CompletionUsage>,
    provider_metadata: Option<Value>,
}

static LIVE_TEST_SERIAL: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

#[tokio::test(flavor = "current_thread")]
async fn cliproxyapi_multi_provider_real_smoke() {
    let _serial = acquire_live_test_lock();
    let Some(harness) = build_live_harness().await else {
        eprintln!(
            "skipping live cliproxyapi test: set AGENA_REAL_CLIPROXY_API_KEY or CLIPROXY_API_KEY"
        );
        return;
    };
    let config = harness.config.clone();
    let provider_registry = harness.provider_registry.clone();
    let app = harness.app.clone();
    let workspace_root = harness.workspace_root.clone();

    for provider_id in ["openai", "claude", "gemini"] {
        let response = provider_registry
            .complete(
                &ModelRef::new(provider_id, config.model.clone()),
                CompletionRequest {
                    model: agena::model::ModelId::new(config.model.clone()),
                    system: None,
                    messages: vec![Message::prompt_text(
                        Role::User,
                        format!(
                            "You are being tested directly through agena provider {provider_id}. Return a brief acknowledgement."
                        ),
                    )],
                    tools: Vec::new(),
                    temperature: None,
                    max_output_tokens: Some(64),
                    prompt_cache_key: None,
                    previous_response_id: None,
                    prompt_window_generation: None,
                    stop_sequences: Vec::new(),
                    top_p: None,
                    top_k: None,
                    seed: None,
                    thinking: None,
                    response_format: None,
                },
            )
            .await
            .unwrap_or_else(|error| panic!("direct provider complete should succeed for {provider_id}: {error}"));
        assert!(
            !response.text.trim().is_empty(),
            "expected direct provider text for {provider_id}, got {response:?}"
        );
        if provider_id == "openai" {
            assert!(
                response
                    .provider_metadata
                    .as_ref()
                    .and_then(|value| value.get("response_id"))
                    .and_then(|value| value.as_str())
                    .is_some(),
                "expected openai response metadata in {response:?}"
            );
        }
    }

    let (providers_status, providers_json) = json_response(
        app.clone(),
        Request::builder()
            .method(Method::GET)
            .uri("/api/v1/providers")
            .body(Body::empty())
            .expect("request should build"),
    )
    .await;
    assert_eq!(
        providers_status,
        StatusCode::OK,
        "unexpected body: {providers_json}"
    );
    let provider_ids = providers_json
        .as_array()
        .expect("providers should be an array")
        .iter()
        .filter_map(|item| item["provider_id"].as_str().map(ToOwned::to_owned))
        .collect::<Vec<_>>();
    assert_eq!(provider_ids, vec!["claude", "gemini", "openai"]);

    for provider_id in ["openai", "claude", "gemini"] {
        let (status, models_json) = json_response(
            app.clone(),
            Request::builder()
                .method(Method::GET)
                .uri(format!("/api/v1/providers/{provider_id}/models"))
                .body(Body::empty())
                .expect("request should build"),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "unexpected body: {models_json}");
        let models = models_json["models"]
            .as_array()
            .expect("models should be an array");
        assert!(
            models.iter().any(|item| {
                item["id"] == json!("gpt-5.4")
                    || item["model"] == json!("gpt-5.4")
                    || item["name"] == json!("gpt-5.4")
            }),
            "expected gpt-5.4 in {provider_id} models, got {models_json}"
        );
    }

    let (runtime_before_status, runtime_before_json) = json_response(
        app.clone(),
        Request::builder()
            .method(Method::GET)
            .uri("/api/v1/runtime")
            .body(Body::empty())
            .expect("request should build"),
    )
    .await;
    assert_eq!(
        runtime_before_status,
        StatusCode::OK,
        "unexpected body: {runtime_before_json}"
    );

    let (workspace_status, workspace_json) = json_response(
        app.clone(),
        Request::builder()
            .method(Method::POST)
            .uri("/api/v1/workspaces")
            .header("content-type", "application/json")
            .body(Body::from(
                json!({ "path": workspace_root.display().to_string() }).to_string(),
            ))
            .expect("request should build"),
    )
    .await;
    assert_eq!(
        workspace_status,
        StatusCode::OK,
        "unexpected body: {workspace_json}"
    );
    let workspace_id = workspace_json["id"]
        .as_i64()
        .expect("workspace id should exist");

    let mut session_ids = std::collections::BTreeMap::new();
    for provider_id in ["openai", "claude", "gemini"] {
        let (session_status, session_json) = json_response(
            app.clone(),
            Request::builder()
                .method(Method::POST)
                .uri("/api/v1/sessions")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "workspace_id": workspace_id,
                        "title": format!("live-{provider_id}")
                    })
                    .to_string(),
                ))
                .expect("request should build"),
        )
        .await;
        assert_eq!(
            session_status,
            StatusCode::OK,
            "unexpected body: {session_json}"
        );
        let session_id = session_json["id"]
            .as_i64()
            .expect("session id should exist");
        session_ids.insert(provider_id.to_string(), session_id);

        let (turn_status, turn_json) = json_response(
            app.clone(),
            Request::builder()
                .method(Method::POST)
                .uri(format!("/api/v1/sessions/{session_id}/turns"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "model": {
                            "provider_id": provider_id,
                            "model_id": config.model
                        },
                        "max_output_tokens": 64,
                        "parts": [
                            {
                                "type": "text",
                                "text": format!(
                                    "You are being tested through agena using provider {provider_id}. Return a brief acknowledgement."
                                )
                            }
                        ]
                    })
                    .to_string(),
                ))
                .expect("request should build"),
        )
        .await;
        assert_eq!(turn_status, StatusCode::OK, "unexpected body: {turn_json}");
        assert_eq!(turn_json["blocked"], json!(false));

        let (messages_status, messages_json) = json_response(
            app.clone(),
            Request::builder()
                .method(Method::GET)
                .uri(format!(
                    "/api/v1/sessions/{session_id}/messages?limit=50&parts=full"
                ))
                .body(Body::empty())
                .expect("request should build"),
        )
        .await;
        assert_eq!(
            messages_status,
            StatusCode::OK,
            "unexpected body: {messages_json}"
        );
        let items = messages_json["items"]
            .as_array()
            .expect("message items should be an array");
        let assistant_message = items.iter().find(|item| {
            item["role"] == json!("assistant")
                && item["metadata"]["model_provider_id"] == json!(provider_id)
                && item["metadata"]["model_id"] == json!(config.model)
        });
        let Some(assistant_message) = assistant_message else {
            panic!("expected assistant message for {provider_id}, got {messages_json}");
        };
        assert!(
            json_message_has_non_empty_text(assistant_message),
            "expected non-empty assistant text for {provider_id}, got {messages_json}"
        );
    }

    let openai_session_id = *session_ids
        .get("openai")
        .expect("openai session should exist");
    let (follow_up_status, follow_up_json) = json_response(
        app.clone(),
        Request::builder()
            .method(Method::POST)
            .uri(format!("/api/v1/sessions/{openai_session_id}/turns"))
            .header("content-type", "application/json")
            .body(Body::from(
                json!({
                    "model": {
                        "provider_id": "openai",
                        "model_id": config.model
                    },
                    "max_output_tokens": 64,
                    "parts": [
                        {
                            "type": "text",
                            "text": "Follow up in the same session and keep the answer short."
                        }
                    ]
                })
                .to_string(),
            ))
            .expect("request should build"),
    )
    .await;
    assert_eq!(
        follow_up_status,
        StatusCode::OK,
        "unexpected body: {follow_up_json}"
    );
    assert_eq!(follow_up_json["blocked"], json!(false));

    let (openai_messages_status, openai_messages_json) = json_response(
        app.clone(),
        Request::builder()
            .method(Method::GET)
            .uri(format!(
                "/api/v1/sessions/{openai_session_id}/messages?limit=100&parts=full"
            ))
            .body(Body::empty())
            .expect("request should build"),
    )
    .await;
    assert_eq!(
        openai_messages_status,
        StatusCode::OK,
        "unexpected body: {openai_messages_json}"
    );
    let openai_assistants = openai_messages_json["items"]
        .as_array()
        .expect("message items should be an array")
        .iter()
        .filter(|item| item["role"] == json!("assistant"))
        .collect::<Vec<_>>();
    assert!(
        openai_assistants.len() >= 2,
        "expected at least two assistant messages, got {openai_messages_json}"
    );
    let latest_openai = openai_assistants
        .last()
        .expect("latest openai assistant should exist");
    assert!(
        json_message_has_non_empty_text(latest_openai),
        "expected non-empty openai follow-up assistant text, got {openai_messages_json}"
    );
    println!(
        "live cliproxy openai cache observation: cache_read_tokens={}, cache_write_tokens={}, usage={}",
        latest_openai["usage"]["cache_read_tokens"]
            .as_u64()
            .unwrap_or_default(),
        latest_openai["usage"]["cache_write_tokens"]
            .as_u64()
            .unwrap_or_default(),
        latest_openai["usage"]
    );

    for _ in 0..2 {
        let (state_status, state_json) = json_response(
            app.clone(),
            Request::builder()
                .method(Method::GET)
                .uri(format!("/api/v1/sessions/{openai_session_id}/state"))
                .body(Body::empty())
                .expect("request should build"),
        )
        .await;
        assert_eq!(
            state_status,
            StatusCode::OK,
            "unexpected body: {state_json}"
        );
    }

    let (events_status, events_json) = json_response(
        app.clone(),
        Request::builder()
            .method(Method::GET)
            .uri(format!(
                "/api/v1/sessions/{openai_session_id}/events?limit=50"
            ))
            .body(Body::empty())
            .expect("request should build"),
    )
    .await;
    assert_eq!(
        events_status,
        StatusCode::OK,
        "unexpected body: {events_json}"
    );
    assert!(
        events_json["items"]
            .as_array()
            .is_some_and(|items| !items.is_empty()),
        "expected session events, got {events_json}"
    );

    let (runtime_after_status, runtime_after_json) = json_response(
        app,
        Request::builder()
            .method(Method::GET)
            .uri("/api/v1/runtime")
            .body(Body::empty())
            .expect("request should build"),
    )
    .await;
    assert_eq!(
        runtime_after_status,
        StatusCode::OK,
        "unexpected body: {runtime_after_json}"
    );
    println!(
        "live session cache stats before={} after={}",
        runtime_before_json["session_cache"], runtime_after_json["session_cache"]
    );
    assert!(
        runtime_after_json["session_cache"]["entry_count"]
            .as_u64()
            .unwrap_or_default()
            >= 1,
        "unexpected runtime body: {runtime_after_json}"
    );
    assert!(
        runtime_after_json["session_cache"]["hits"]
            .as_u64()
            .unwrap_or_default()
            >= 1,
        "unexpected runtime body: {runtime_after_json}"
    );
    assert!(
        runtime_after_json["session_cache"]["misses"]
            .as_u64()
            .unwrap_or_default()
            >= 1,
        "unexpected runtime body: {runtime_after_json}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn cliproxyapi_multi_provider_real_streaming_smoke() {
    let _serial = acquire_live_test_lock();
    let Some(harness) = build_live_harness().await else {
        eprintln!(
            "skipping live cliproxyapi stream test: set AGENA_REAL_CLIPROXY_API_KEY or CLIPROXY_API_KEY"
        );
        return;
    };

    for provider_id in ["openai", "claude", "gemini"] {
        let observation = collect_stream_observation(
            &harness.provider_registry,
            provider_id,
            harness.config.model.as_str(),
            format!(
                "You are being tested through agena streaming using provider {provider_id}. Return a brief acknowledgement."
            ),
            None,
        )
        .await;
        assert!(
            observation.completion_count >= 1,
            "expected completed event for {provider_id}, got {observation:?}"
        );
        assert!(
            observation.text_delta_count >= 1,
            "expected at least one text delta for {provider_id}, got {observation:?}"
        );
        assert!(
            !observation.text.trim().is_empty(),
            "expected non-empty streamed text for {provider_id}, got {observation:?}"
        );
        if provider_id == "openai" {
            assert!(
                observation
                    .provider_metadata
                    .as_ref()
                    .and_then(|value| value.get("response_id"))
                    .and_then(|value| value.as_str())
                    .is_some(),
                "expected openai streamed response_id metadata, got {observation:?}"
            );
        }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn cliproxyapi_live_http_crud_roundtrip_with_real_generation() {
    let _serial = acquire_live_test_lock();
    let Some(harness) = build_live_harness().await else {
        eprintln!(
            "skipping live cliproxyapi CRUD test: set AGENA_REAL_CLIPROXY_API_KEY or CLIPROXY_API_KEY"
        );
        return;
    };

    let app = harness.app.clone();
    let renamed_workspace_root = temp_dir("agena-live-workspace-renamed");
    fs::create_dir_all(&renamed_workspace_root).expect("renamed workspace should exist");
    fs::write(
        renamed_workspace_root.join("README.md"),
        "# Renamed Live Workspace\n",
    )
    .expect("renamed workspace README should be written");

    let (health_status, health_json) = json_response(
        app.clone(),
        Request::builder()
            .method(Method::GET)
            .uri("/api/v1/health")
            .body(Body::empty())
            .expect("request should build"),
    )
    .await;
    assert_eq!(
        health_status,
        StatusCode::OK,
        "unexpected body: {health_json}"
    );
    assert_eq!(health_json["status"], json!("ok"));

    let (runtime_before_status, runtime_before_json) = json_response(
        app.clone(),
        Request::builder()
            .method(Method::GET)
            .uri("/api/v1/runtime")
            .body(Body::empty())
            .expect("request should build"),
    )
    .await;
    assert_eq!(
        runtime_before_status,
        StatusCode::OK,
        "unexpected body: {runtime_before_json}"
    );
    let runtime_generation_before = runtime_before_json["generation"]
        .as_u64()
        .expect("runtime generation should exist");

    let (workspace_status, workspace_json) = json_response(
        app.clone(),
        Request::builder()
            .method(Method::POST)
            .uri("/api/v1/workspaces")
            .header("content-type", "application/json")
            .body(Body::from(
                json!({ "path": harness.workspace_root.display().to_string() }).to_string(),
            ))
            .expect("request should build"),
    )
    .await;
    assert_eq!(
        workspace_status,
        StatusCode::OK,
        "unexpected body: {workspace_json}"
    );
    let workspace_id = workspace_json["id"]
        .as_i64()
        .expect("workspace id should exist");

    let (fetched_workspace_status, fetched_workspace_json) = json_response(
        app.clone(),
        Request::builder()
            .method(Method::GET)
            .uri(format!("/api/v1/workspaces/{workspace_id}"))
            .body(Body::empty())
            .expect("request should build"),
    )
    .await;
    assert_eq!(
        fetched_workspace_status,
        StatusCode::OK,
        "unexpected body: {fetched_workspace_json}"
    );
    assert_eq!(fetched_workspace_json["id"], json!(workspace_id));

    let (resolved_workspace_status, resolved_workspace_json) = json_response(
        app.clone(),
        Request::builder()
            .method(Method::POST)
            .uri("/api/v1/workspaces/resolve")
            .header("content-type", "application/json")
            .body(Body::from(
                json!({
                    "path": harness.workspace_root.display().to_string(),
                    "create_if_missing": true
                })
                .to_string(),
            ))
            .expect("request should build"),
    )
    .await;
    assert_eq!(
        resolved_workspace_status,
        StatusCode::OK,
        "unexpected body: {resolved_workspace_json}"
    );
    assert_eq!(resolved_workspace_json["id"], json!(workspace_id));

    let (replaced_workspace_status, replaced_workspace_json) = json_response(
        app.clone(),
        Request::builder()
            .method(Method::PUT)
            .uri(format!("/api/v1/workspaces/{workspace_id}"))
            .header("content-type", "application/json")
            .body(Body::from(
                json!({ "path": renamed_workspace_root.display().to_string() }).to_string(),
            ))
            .expect("request should build"),
    )
    .await;
    assert_eq!(
        replaced_workspace_status,
        StatusCode::OK,
        "unexpected body: {replaced_workspace_json}"
    );
    assert_eq!(
        replaced_workspace_json["path"],
        json!(renamed_workspace_root.display().to_string())
    );

    let (root_session_status, root_session_json) = json_response(
        app.clone(),
        Request::builder()
            .method(Method::POST)
            .uri("/api/v1/sessions")
            .header("content-type", "application/json")
            .body(Body::from(
                json!({
                    "workspace_id": workspace_id,
                    "title": "Live Root Session"
                })
                .to_string(),
            ))
            .expect("request should build"),
    )
    .await;
    assert_eq!(
        root_session_status,
        StatusCode::OK,
        "unexpected body: {root_session_json}"
    );
    let root_session_id = root_session_json["id"]
        .as_i64()
        .expect("root session id should exist");

    let (child_session_status, child_session_json) = json_response(
        app.clone(),
        Request::builder()
            .method(Method::POST)
            .uri("/api/v1/sessions")
            .header("content-type", "application/json")
            .body(Body::from(
                json!({
                    "workspace_id": workspace_id,
                    "title": "Live Child Session",
                    "parent_id": root_session_id
                })
                .to_string(),
            ))
            .expect("request should build"),
    )
    .await;
    assert_eq!(
        child_session_status,
        StatusCode::OK,
        "unexpected body: {child_session_json}"
    );
    let child_session_id = child_session_json["id"]
        .as_i64()
        .expect("child session id should exist");

    let (listed_workspaces_status, listed_workspaces_json) = json_response(
        app.clone(),
        Request::builder()
            .method(Method::GET)
            .uri("/api/v1/workspaces?include_session_count=true")
            .body(Body::empty())
            .expect("request should build"),
    )
    .await;
    assert_eq!(
        listed_workspaces_status,
        StatusCode::OK,
        "unexpected body: {listed_workspaces_json}"
    );
    let listed_workspace = listed_workspaces_json["items"]
        .as_array()
        .expect("workspace items should be an array")
        .iter()
        .find(|item| item["id"] == json!(workspace_id))
        .expect("created workspace should be listed");
    assert_eq!(listed_workspace["session_count"], json!(2));

    let (listed_sessions_status, listed_sessions_json) = json_response(
        app.clone(),
        Request::builder()
            .method(Method::GET)
            .uri(format!(
                "/api/v1/sessions?workspace_id={workspace_id}&limit=10"
            ))
            .body(Body::empty())
            .expect("request should build"),
    )
    .await;
    assert_eq!(
        listed_sessions_status,
        StatusCode::OK,
        "unexpected body: {listed_sessions_json}"
    );
    assert_eq!(
        listed_sessions_json["items"]
            .as_array()
            .expect("session items should be an array")
            .len(),
        2
    );

    let (root_only_status, root_only_json) = json_response(
        app.clone(),
        Request::builder()
            .method(Method::GET)
            .uri(format!(
                "/api/v1/sessions?workspace_id={workspace_id}&roots=true&limit=10"
            ))
            .body(Body::empty())
            .expect("request should build"),
    )
    .await;
    assert_eq!(
        root_only_status,
        StatusCode::OK,
        "unexpected body: {root_only_json}"
    );
    let root_only_items = root_only_json["items"]
        .as_array()
        .expect("root items should be an array");
    assert!(
        root_only_items
            .iter()
            .any(|item| item["id"] == json!(root_session_id)),
        "expected root session in {root_only_json}"
    );
    assert!(
        root_only_items
            .iter()
            .all(|item| item["parent_id"].is_null()),
        "expected only root sessions in {root_only_json}"
    );

    let (child_list_status, child_list_json) = json_response(
        app.clone(),
        Request::builder()
            .method(Method::GET)
            .uri(format!(
                "/api/v1/sessions?parent_id={root_session_id}&limit=10"
            ))
            .body(Body::empty())
            .expect("request should build"),
    )
    .await;
    assert_eq!(
        child_list_status,
        StatusCode::OK,
        "unexpected body: {child_list_json}"
    );
    let child_items = child_list_json["items"]
        .as_array()
        .expect("child items should be an array");
    assert_eq!(child_items.len(), 1, "unexpected body: {child_list_json}");
    assert_eq!(child_items[0]["id"], json!(child_session_id));

    let (replaced_child_status, replaced_child_json) = json_response(
        app.clone(),
        Request::builder()
            .method(Method::PUT)
            .uri(format!("/api/v1/sessions/{child_session_id}"))
            .header("content-type", "application/json")
            .body(Body::from(
                json!({
                    "title": "Live Child Session Updated",
                    "parent_id": root_session_id
                })
                .to_string(),
            ))
            .expect("request should build"),
    )
    .await;
    assert_eq!(
        replaced_child_status,
        StatusCode::OK,
        "unexpected body: {replaced_child_json}"
    );
    assert_eq!(
        replaced_child_json["title"],
        json!("Live Child Session Updated")
    );

    let (fetched_child_status, fetched_child_json) = json_response(
        app.clone(),
        Request::builder()
            .method(Method::GET)
            .uri(format!("/api/v1/sessions/{child_session_id}"))
            .body(Body::empty())
            .expect("request should build"),
    )
    .await;
    assert_eq!(
        fetched_child_status,
        StatusCode::OK,
        "unexpected body: {fetched_child_json}"
    );
    assert_eq!(
        fetched_child_json["title"],
        json!("Live Child Session Updated")
    );

    let (turn_status, turn_json) = json_response(
        app.clone(),
        Request::builder()
            .method(Method::POST)
            .uri(format!("/api/v1/sessions/{root_session_id}/turns"))
            .header("content-type", "application/json")
            .body(Body::from(
                json!({
                    "model": {
                        "provider_id": "openai",
                        "model_id": harness.config.model
                    },
                    "max_output_tokens": 96,
                    "parts": [
                        {
                            "type": "text",
                            "text": "This is a live CRUD integration test. Reply with a short acknowledgement that includes the phrase 'live crud'."
                        }
                    ]
                })
                .to_string(),
            ))
            .expect("request should build"),
    )
    .await;
    assert_eq!(turn_status, StatusCode::OK, "unexpected body: {turn_json}");
    assert_eq!(turn_json["blocked"], json!(false));
    assert_eq!(turn_json["run_state"], json!("idle"));
    assert!(turn_json["latest_event_seq"].as_i64().is_some());

    let (state_status, state_json) = json_response(
        app.clone(),
        Request::builder()
            .method(Method::GET)
            .uri(format!("/api/v1/sessions/{root_session_id}/state"))
            .body(Body::empty())
            .expect("request should build"),
    )
    .await;
    assert_eq!(
        state_status,
        StatusCode::OK,
        "unexpected body: {state_json}"
    );
    assert_eq!(state_json["blocked"], json!(false));
    assert_eq!(state_json["run_state"], json!("idle"));
    assert_eq!(state_json["pending_permission_requests"], json!([]));
    assert_eq!(state_json["pending_user_input_requests"], json!([]));

    let (messages_status, messages_json) = json_response(
        app.clone(),
        Request::builder()
            .method(Method::GET)
            .uri(format!(
                "/api/v1/sessions/{root_session_id}/messages?limit=50&parts=full"
            ))
            .body(Body::empty())
            .expect("request should build"),
    )
    .await;
    assert_eq!(
        messages_status,
        StatusCode::OK,
        "unexpected body: {messages_json}"
    );
    let assistant_message = latest_assistant_message(&messages_json);
    assert_eq!(
        assistant_message["metadata"]["model_provider_id"],
        json!("openai")
    );
    assert_eq!(
        assistant_message["metadata"]["model_id"],
        json!(harness.config.model)
    );
    assert!(
        json_message_has_non_empty_text(assistant_message),
        "expected non-empty assistant text, got {messages_json}"
    );
    let assistant_message_id = assistant_message["id"]
        .as_i64()
        .expect("assistant message id should exist");
    let assistant_text_part_id =
        first_text_part_id(assistant_message).expect("assistant text part should exist");

    let (message_summary_status, message_summary_json) = json_response(
        app.clone(),
        Request::builder()
            .method(Method::GET)
            .uri(format!(
                "/api/v1/messages/{assistant_message_id}?parts=summary"
            ))
            .body(Body::empty())
            .expect("request should build"),
    )
    .await;
    assert_eq!(
        message_summary_status,
        StatusCode::OK,
        "unexpected body: {message_summary_json}"
    );
    assert!(
        message_summary_json["parts"]
            .as_array()
            .is_some_and(|parts| parts.iter().all(|part| part["content"].is_null())),
        "expected summary parts without detail, got {message_summary_json}"
    );

    let (message_full_status, message_full_json) = json_response(
        app.clone(),
        Request::builder()
            .method(Method::GET)
            .uri(format!(
                "/api/v1/messages/{assistant_message_id}?parts=full"
            ))
            .body(Body::empty())
            .expect("request should build"),
    )
    .await;
    assert_eq!(
        message_full_status,
        StatusCode::OK,
        "unexpected body: {message_full_json}"
    );
    assert!(
        json_message_has_non_empty_text(&message_full_json),
        "expected full message text, got {message_full_json}"
    );

    let (message_parts_status, message_parts_json) = json_response(
        app.clone(),
        Request::builder()
            .method(Method::GET)
            .uri(format!(
                "/api/v1/messages/{assistant_message_id}/parts?mode=full"
            ))
            .body(Body::empty())
            .expect("request should build"),
    )
    .await;
    assert_eq!(
        message_parts_status,
        StatusCode::OK,
        "unexpected body: {message_parts_json}"
    );
    let message_parts = message_parts_json
        .as_array()
        .expect("message parts should be an array");
    assert!(
        message_parts
            .iter()
            .any(|part| part["id"] == json!(assistant_text_part_id)),
        "expected assistant text part in {message_parts_json}"
    );

    let (message_part_status, message_part_json) = json_response(
        app.clone(),
        Request::builder()
            .method(Method::GET)
            .uri(format!("/api/v1/message-parts/{assistant_text_part_id}"))
            .body(Body::empty())
            .expect("request should build"),
    )
    .await;
    assert_eq!(
        message_part_status,
        StatusCode::OK,
        "unexpected body: {message_part_json}"
    );
    assert_eq!(message_part_json["id"], json!(assistant_text_part_id));
    assert_eq!(message_part_json["content"]["type"], json!("text"));

    let (events_status, events_json) = json_response(
        app.clone(),
        Request::builder()
            .method(Method::GET)
            .uri(format!(
                "/api/v1/sessions/{root_session_id}/events?limit=50"
            ))
            .body(Body::empty())
            .expect("request should build"),
    )
    .await;
    assert_eq!(
        events_status,
        StatusCode::OK,
        "unexpected body: {events_json}"
    );
    assert!(
        events_json["items"]
            .as_array()
            .is_some_and(|items| !items.is_empty()),
        "expected session events, got {events_json}"
    );

    let (stream_status, stream_body) = text_response(
        app.clone(),
        Request::builder()
            .method(Method::GET)
            .uri(format!(
                "/api/v1/sessions/{root_session_id}/events/stream?after_seq=0&poll_interval_ms=10&idle_timeout_ms=40"
            ))
            .body(Body::empty())
            .expect("request should build"),
    )
    .await;
    assert_eq!(
        stream_status,
        StatusCode::OK,
        "unexpected body: {stream_body}"
    );
    assert!(stream_body.contains("event: session_event"));
    assert!(stream_body.contains(&format!("\"session_id\":{root_session_id}")));

    let (created_rule_status, created_rule_json) = json_response(
        app.clone(),
        Request::builder()
            .method(Method::POST)
            .uri("/api/v1/permission-rules")
            .header("content-type", "application/json")
            .body(Body::from(
                json!({
                    "action_key": "tool:bash",
                    "mode": "ask"
                })
                .to_string(),
            ))
            .expect("request should build"),
    )
    .await;
    assert_eq!(
        created_rule_status,
        StatusCode::OK,
        "unexpected body: {created_rule_json}"
    );
    let rule_id = created_rule_json["id"]
        .as_i64()
        .expect("rule id should exist");

    let (listed_rules_status, listed_rules_json) = json_response(
        app.clone(),
        Request::builder()
            .method(Method::GET)
            .uri("/api/v1/permission-rules?search=tool%3Abash")
            .body(Body::empty())
            .expect("request should build"),
    )
    .await;
    assert_eq!(
        listed_rules_status,
        StatusCode::OK,
        "unexpected body: {listed_rules_json}"
    );
    assert!(
        listed_rules_json["items"]
            .as_array()
            .is_some_and(|items| items
                .iter()
                .any(|item| { item["id"] == json!(rule_id) && item["mode"] == json!("ask") })),
        "expected created rule in {listed_rules_json}"
    );

    let (replaced_rule_status, replaced_rule_json) = json_response(
        app.clone(),
        Request::builder()
            .method(Method::PUT)
            .uri(format!("/api/v1/permission-rules/{rule_id}"))
            .header("content-type", "application/json")
            .body(Body::from(
                json!({
                    "action_key": "tool:bash",
                    "mode": "allow"
                })
                .to_string(),
            ))
            .expect("request should build"),
    )
    .await;
    assert_eq!(
        replaced_rule_status,
        StatusCode::OK,
        "unexpected body: {replaced_rule_json}"
    );
    assert_eq!(replaced_rule_json["mode"], json!("allow"));

    let (fetched_rule_status, fetched_rule_json) = json_response(
        app.clone(),
        Request::builder()
            .method(Method::GET)
            .uri(format!("/api/v1/permission-rules/{rule_id}"))
            .body(Body::empty())
            .expect("request should build"),
    )
    .await;
    assert_eq!(
        fetched_rule_status,
        StatusCode::OK,
        "unexpected body: {fetched_rule_json}"
    );
    assert_eq!(fetched_rule_json["mode"], json!("allow"));

    let (deleted_rule_status, deleted_rule_json) = json_response(
        app.clone(),
        Request::builder()
            .method(Method::DELETE)
            .uri(format!("/api/v1/permission-rules/{rule_id}"))
            .body(Body::empty())
            .expect("request should build"),
    )
    .await;
    assert_eq!(
        deleted_rule_status,
        StatusCode::OK,
        "unexpected body: {deleted_rule_json}"
    );
    assert_eq!(deleted_rule_json["id"], json!(rule_id));

    let (reloaded_status, reloaded_json) = json_response(
        app.clone(),
        Request::builder()
            .method(Method::POST)
            .uri("/api/v1/runtime/reload")
            .body(Body::empty())
            .expect("request should build"),
    )
    .await;
    assert_eq!(
        reloaded_status,
        StatusCode::OK,
        "unexpected body: {reloaded_json}"
    );
    assert!(
        reloaded_json["generation"]
            .as_u64()
            .is_some_and(|generation| generation > runtime_generation_before),
        "expected runtime generation to advance, before={runtime_before_json} after={reloaded_json}"
    );

    let (deleted_child_status, deleted_child_json) = json_response(
        app.clone(),
        Request::builder()
            .method(Method::DELETE)
            .uri(format!("/api/v1/sessions/{child_session_id}"))
            .body(Body::empty())
            .expect("request should build"),
    )
    .await;
    assert_eq!(
        deleted_child_status,
        StatusCode::OK,
        "unexpected body: {deleted_child_json}"
    );
    assert_eq!(deleted_child_json["id"], json!(child_session_id));

    let (deleted_root_status, deleted_root_json) = json_response(
        app.clone(),
        Request::builder()
            .method(Method::DELETE)
            .uri(format!("/api/v1/sessions/{root_session_id}"))
            .body(Body::empty())
            .expect("request should build"),
    )
    .await;
    assert_eq!(
        deleted_root_status,
        StatusCode::OK,
        "unexpected body: {deleted_root_json}"
    );
    assert_eq!(deleted_root_json["id"], json!(root_session_id));

    let (deleted_workspace_status, deleted_workspace_json) = json_response(
        app.clone(),
        Request::builder()
            .method(Method::DELETE)
            .uri(format!("/api/v1/workspaces/{workspace_id}"))
            .body(Body::empty())
            .expect("request should build"),
    )
    .await;
    assert_eq!(
        deleted_workspace_status,
        StatusCode::OK,
        "unexpected body: {deleted_workspace_json}"
    );
    assert_eq!(deleted_workspace_json["id"], json!(workspace_id));
}

#[tokio::test(flavor = "current_thread")]
async fn cliproxyapi_openai_real_prompt_cache_roundtrip() {
    let _serial = acquire_live_test_lock();
    let Some(harness) = build_live_harness().await else {
        eprintln!(
            "skipping live cliproxyapi openai cache test: set AGENA_REAL_CLIPROXY_API_KEY or CLIPROXY_API_KEY"
        );
        return;
    };

    let cache_nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time should advance")
        .as_nanos();
    let prompt_cache_key = format!("agena-live-openai-cache-{cache_nonce}");
    let prompt = format!(
        "{} Respond with OK only.",
        format!("CACHE-MARKER-OPENAI-REAL-{cache_nonce} ").repeat(4000)
    );

    let first = harness
        .provider_registry
        .complete(
            &ModelRef::new("openai", harness.config.model.clone()),
            completion_request(
                harness.config.model.as_str(),
                prompt.clone(),
                Some(prompt_cache_key.clone()),
            ),
        )
        .await
        .expect("first openai cache probe should succeed");
    let second = harness
        .provider_registry
        .complete(
            &ModelRef::new("openai", harness.config.model.clone()),
            completion_request(
                harness.config.model.as_str(),
                prompt,
                Some(prompt_cache_key),
            ),
        )
        .await
        .expect("second openai cache probe should succeed");

    assert!(
        !first.text.trim().is_empty(),
        "expected first openai text, got {first:?}"
    );
    assert!(
        !second.text.trim().is_empty(),
        "expected second openai text, got {second:?}"
    );

    let first_usage = first
        .usage
        .expect("first openai response should include usage");
    let second_usage = second
        .usage
        .expect("second openai response should include usage");
    assert!(
        second_usage.cache_read_tokens > 0,
        "expected cached second request, first={first_usage:?} second={second_usage:?}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn cliproxyapi_live_cursor_pagination_roundtrip() {
    let _serial = acquire_live_test_lock();
    let Some(harness) = build_live_harness().await else {
        eprintln!(
            "skipping live cliproxyapi pagination test: set AGENA_REAL_CLIPROXY_API_KEY or CLIPROXY_API_KEY"
        );
        return;
    };

    let app = harness.app.clone();
    let (workspace_status, workspace_json) = json_response(
        app.clone(),
        Request::builder()
            .method(Method::POST)
            .uri("/api/v1/workspaces")
            .header("content-type", "application/json")
            .body(Body::from(
                json!({ "path": harness.workspace_root.display().to_string() }).to_string(),
            ))
            .expect("request should build"),
    )
    .await;
    assert_eq!(
        workspace_status,
        StatusCode::OK,
        "unexpected body: {workspace_json}"
    );
    let workspace_id = workspace_json["id"]
        .as_i64()
        .expect("workspace id should exist");

    let mut created_session_ids = Vec::new();
    for index in 0..3 {
        let (status, payload) = json_response(
            app.clone(),
            Request::builder()
                .method(Method::POST)
                .uri("/api/v1/sessions")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "workspace_id": workspace_id,
                        "title": format!("live-pagination-{index}")
                    })
                    .to_string(),
                ))
                .expect("request should build"),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "unexpected body: {payload}");
        created_session_ids.push(payload["id"].as_i64().expect("session id should exist"));
    }

    let (sessions_first_status, sessions_first_json) = json_response(
        app.clone(),
        Request::builder()
            .method(Method::GET)
            .uri(format!(
                "/api/v1/sessions?workspace_id={workspace_id}&limit=2"
            ))
            .body(Body::empty())
            .expect("request should build"),
    )
    .await;
    assert_eq!(
        sessions_first_status,
        StatusCode::OK,
        "unexpected body: {sessions_first_json}"
    );
    assert_eq!(sessions_first_json["page"]["has_more"], json!(true));
    let sessions_first_cursor = sessions_first_json["page"]["next_cursor"]
        .as_str()
        .expect("next cursor should exist")
        .to_string();
    let sessions_first_ids = json_array_i64_field(&sessions_first_json["items"], "id");
    assert_eq!(sessions_first_ids.len(), 2);

    let (sessions_second_status, sessions_second_json) = json_response(
        app.clone(),
        Request::builder()
            .method(Method::GET)
            .uri(format!(
                "/api/v1/sessions?workspace_id={workspace_id}&limit=2&cursor={sessions_first_cursor}"
            ))
            .body(Body::empty())
            .expect("request should build"),
    )
    .await;
    assert_eq!(
        sessions_second_status,
        StatusCode::OK,
        "unexpected body: {sessions_second_json}"
    );
    assert_eq!(sessions_second_json["page"]["has_more"], json!(false));
    let sessions_second_ids = json_array_i64_field(&sessions_second_json["items"], "id");
    assert_eq!(sessions_second_ids.len(), 1);
    let paged_session_ids = sessions_first_ids
        .iter()
        .chain(sessions_second_ids.iter())
        .copied()
        .collect::<BTreeSet<_>>();
    assert_eq!(
        paged_session_ids,
        created_session_ids.iter().copied().collect::<BTreeSet<_>>()
    );

    let paged_session_id = created_session_ids[0];
    for turn_index in 0..2 {
        let (turn_status, turn_json) = json_response(
            app.clone(),
            Request::builder()
                .method(Method::POST)
                .uri(format!("/api/v1/sessions/{paged_session_id}/turns"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "model": {
                            "provider_id": "openai",
                            "model_id": harness.config.model
                        },
                        "max_output_tokens": 64,
                        "parts": [
                            {
                                "type": "text",
                                "text": format!("Live pagination turn {turn_index}. Reply briefly.")
                            }
                        ]
                    })
                    .to_string(),
                ))
                .expect("request should build"),
        )
        .await;
        assert_eq!(turn_status, StatusCode::OK, "unexpected body: {turn_json}");
        assert_eq!(turn_json["blocked"], json!(false));
    }

    let (messages_full_status, messages_full_json) = json_response(
        app.clone(),
        Request::builder()
            .method(Method::GET)
            .uri(format!(
                "/api/v1/sessions/{paged_session_id}/messages?parts=summary&limit=20"
            ))
            .body(Body::empty())
            .expect("request should build"),
    )
    .await;
    assert_eq!(
        messages_full_status,
        StatusCode::OK,
        "unexpected body: {messages_full_json}"
    );
    let all_message_ids = json_array_i64_field(&messages_full_json["items"], "id");
    assert_eq!(
        all_message_ids.len(),
        4,
        "expected exactly two user+assistant turns, got {messages_full_json}"
    );

    let (messages_first_status, messages_first_json) = json_response(
        app.clone(),
        Request::builder()
            .method(Method::GET)
            .uri(format!(
                "/api/v1/sessions/{paged_session_id}/messages?parts=summary&limit=2"
            ))
            .body(Body::empty())
            .expect("request should build"),
    )
    .await;
    assert_eq!(
        messages_first_status,
        StatusCode::OK,
        "unexpected body: {messages_first_json}"
    );
    assert_eq!(messages_first_json["page"]["order"], json!("asc"));
    assert_eq!(messages_first_json["page"]["has_more"], json!(true));
    let messages_first_cursor = messages_first_json["page"]["next_cursor"]
        .as_str()
        .expect("next cursor should exist")
        .to_string();
    let messages_first_ids = json_array_i64_field(&messages_first_json["items"], "id");
    assert_eq!(
        messages_first_ids,
        all_message_ids[all_message_ids.len() - 2..].to_vec(),
        "unexpected newest message window: {messages_first_json}"
    );

    let (messages_second_status, messages_second_json) = json_response(
        app.clone(),
        Request::builder()
            .method(Method::GET)
            .uri(format!(
                "/api/v1/sessions/{paged_session_id}/messages?parts=summary&limit=2&cursor={messages_first_cursor}"
            ))
            .body(Body::empty())
            .expect("request should build"),
    )
    .await;
    assert_eq!(
        messages_second_status,
        StatusCode::OK,
        "unexpected body: {messages_second_json}"
    );
    assert_eq!(messages_second_json["page"]["order"], json!("asc"));
    assert_eq!(messages_second_json["page"]["has_more"], json!(false));
    let messages_second_ids = json_array_i64_field(&messages_second_json["items"], "id");
    assert_eq!(
        messages_second_ids,
        all_message_ids[..all_message_ids.len() - 2].to_vec(),
        "unexpected older message window: {messages_second_json}"
    );

    let (events_full_status, events_full_json) = json_response(
        app.clone(),
        Request::builder()
            .method(Method::GET)
            .uri(format!(
                "/api/v1/sessions/{paged_session_id}/events?limit=200"
            ))
            .body(Body::empty())
            .expect("request should build"),
    )
    .await;
    assert_eq!(
        events_full_status,
        StatusCode::OK,
        "unexpected body: {events_full_json}"
    );
    let all_event_seqs = json_array_i64_field(&events_full_json["items"], "seq");
    assert!(
        all_event_seqs.len() >= 4,
        "expected multiple events for paged session, got {events_full_json}"
    );

    let (events_first_status, events_first_json) = json_response(
        app.clone(),
        Request::builder()
            .method(Method::GET)
            .uri(format!(
                "/api/v1/sessions/{paged_session_id}/events?limit=2"
            ))
            .body(Body::empty())
            .expect("request should build"),
    )
    .await;
    assert_eq!(
        events_first_status,
        StatusCode::OK,
        "unexpected body: {events_first_json}"
    );
    assert_eq!(events_first_json["page"]["order"], json!("asc"));
    assert_eq!(events_first_json["page"]["has_more"], json!(true));
    let events_first_cursor = events_first_json["page"]["next_cursor"]
        .as_str()
        .expect("next cursor should exist")
        .to_string();
    let events_first_seqs = json_array_i64_field(&events_first_json["items"], "seq");
    assert_eq!(
        events_first_seqs,
        all_event_seqs[all_event_seqs.len() - 2..].to_vec(),
        "unexpected newest event window: {events_first_json}"
    );

    let (events_second_status, events_second_json) = json_response(
        app,
        Request::builder()
            .method(Method::GET)
            .uri(format!(
                "/api/v1/sessions/{paged_session_id}/events?limit=2&cursor={events_first_cursor}"
            ))
            .body(Body::empty())
            .expect("request should build"),
    )
    .await;
    assert_eq!(
        events_second_status,
        StatusCode::OK,
        "unexpected body: {events_second_json}"
    );
    assert_eq!(events_second_json["page"]["order"], json!("asc"));
    let events_second_seqs = json_array_i64_field(&events_second_json["items"], "seq");
    assert_eq!(
        events_second_seqs,
        all_event_seqs[all_event_seqs.len() - 4..all_event_seqs.len() - 2].to_vec(),
        "unexpected older event window: {events_second_json}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn cliproxyapi_openai_real_tool_flow_executes_builtin_tools() {
    let _serial = acquire_live_test_lock();
    if live_config_from_env().is_none() {
        eprintln!(
            "skipping live cliproxyapi tool-flow test: set AGENA_REAL_CLIPROXY_API_KEY or CLIPROXY_API_KEY"
        );
        return;
    }

    for attempt in 1..=3 {
        let result = std::panic::AssertUnwindSafe(assert_openai_real_tool_flow_once())
            .catch_unwind()
            .await;
        match result {
            Ok(()) => return,
            Err(payload) if attempt == 3 => std::panic::resume_unwind(payload),
            Err(_) => {
                eprintln!("live tool-flow attempt {attempt} failed; retrying");
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            }
        }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn cliproxyapi_authoritative_cache_probe_matches_current_provider_behavior() {
    let _serial = acquire_live_test_lock();
    let Some(config) = live_config_from_env() else {
        eprintln!(
            "skipping live cliproxyapi cache probe: set AGENA_REAL_CLIPROXY_API_KEY or CLIPROXY_API_KEY"
        );
        return;
    };

    let cache_nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time should advance")
        .as_millis()
        .to_string();
    let probe = run_cache_probe_with_nonce(&config, cache_nonce.as_str(), 3, 2.0);
    let openai_second_cached = probe["openai"]["second_cached_tokens"]
        .as_u64()
        .unwrap_or_default();

    assert!(
        probe["openai"]["cache_hit_observed"] == json!(true) && openai_second_cached > 0,
        "expected authoritative OpenAI cache hit on second probe, got {probe}"
    );

    let claude_second_cached = probe["claude"]["second_cache_read_input_tokens"]
        .as_u64()
        .unwrap_or_default();
    assert!(
        probe["claude"]["first_input_tokens"]
            .as_u64()
            .unwrap_or_default()
            > 0
            && probe["claude"]["second_input_tokens"]
                .as_u64()
                .unwrap_or_default()
                > 0,
        "expected Claude probe requests to succeed, got {probe}"
    );
    println!(
        "live authoritative claude cache observation: second_cache_read_input_tokens={claude_second_cached}"
    );

    let gemini_first_prompt_tokens = probe["gemini"]["first_prompt_tokens"]
        .as_u64()
        .unwrap_or_default();
    let gemini_second_prompt_tokens = probe["gemini"]["second_prompt_tokens"]
        .as_u64()
        .unwrap_or_default();
    assert!(
        gemini_first_prompt_tokens > 0 && gemini_second_prompt_tokens > 0,
        "expected Gemini probe requests to succeed, got {probe}"
    );

    println!("live authoritative cache probe: {probe}");
}

fn acquire_live_test_lock() -> std::sync::MutexGuard<'static, ()> {
    LIVE_TEST_SERIAL
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

async fn build_live_harness() -> Option<LiveHarness> {
    let config = live_config_from_env()?;

    let workspace_root = temp_dir("agena-live-workspace");
    fs::create_dir_all(workspace_root.join("docs")).expect("workspace directories should exist");
    fs::write(
        workspace_root.join("README.md"),
        "# Live Workspace\ncache marker from live test\n",
    )
    .expect("README should be written");
    fs::write(
        workspace_root.join("docs/live.md"),
        "live cache marker in docs\n",
    )
    .expect("live docs should be written");

    let db = Arc::new(
        Database::connect("sqlite::memory:")
            .await
            .expect("database should connect"),
    );
    init_schema(db.as_ref())
        .await
        .expect("schema should initialize");

    let auth_store_path = temp_dir("agena-live-auth").join("auth.json");
    let config_path = write_temp_config(
        format!(
            r#"
[tracing]
filter = "info"

[auth]
store_path = {auth_store_path:?}

[runtime.session_cache]
max_sessions = 32
ttl_secs = 900
max_bytes = 16777216

[providers.openai]
kind = "openai"
base_url = {openai_base_url:?}
default_model = {model:?}
api_key = {api_key:?}
api_mode = "responses"
stream_mode = "sse"

[providers.claude]
kind = "anthropic"
base_url = {claude_base_url:?}
default_model = {model:?}
api_key = {api_key:?}
auth_header = "x-api-key"

[providers.gemini]
kind = "gemini"
base_url = {gemini_base_url:?}
default_model = {model:?}
api_key = {api_key:?}
"#,
            auth_store_path = auth_store_path.display().to_string(),
            openai_base_url = format!("{}/api/provider/openai/v1", config.base_url.clone()),
            claude_base_url = format!("{}/api/provider/claude/v1", config.base_url.clone()),
            gemini_base_url = format!("{}/api/provider/gemini/v1beta", config.base_url.clone()),
            model = config.model.clone(),
            api_key = config.api_key.clone(),
        )
        .as_str(),
    );

    let runtime = AgenaRuntime::builder()
        .with_load_request(LoadConfigRequest {
            config_path: Some(config_path),
            ..Default::default()
        })
        .with_workspace_root(workspace_root.clone())
        .with_database_connection(db.as_ref().clone())
        .build()
        .await
        .expect("runtime should build");
    let provider_registry = runtime.current_snapshot().provider_registry();
    let app = router(ApiState::new(runtime, db));

    Some(LiveHarness {
        config,
        provider_registry,
        app,
        workspace_root,
    })
}

fn live_config_from_env() -> Option<LiveConfig> {
    let api_key = std::env::var("AGENA_REAL_CLIPROXY_API_KEY")
        .ok()
        .or_else(|| std::env::var("CLIPROXY_API_KEY").ok())?;
    let base_url = std::env::var("AGENA_REAL_CLIPROXY_BASE_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "https://api.cxits.cn".to_string());
    let model = std::env::var("AGENA_REAL_CLIPROXY_MODEL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "gpt-5.4".to_string());
    Some(LiveConfig {
        base_url: base_url.trim_end_matches('/').to_string(),
        api_key,
        model,
    })
}

fn completion_request(
    model: &str,
    prompt: String,
    prompt_cache_key: Option<String>,
) -> CompletionRequest {
    CompletionRequest {
        model: agena::model::ModelId::new(model.to_owned()),
        system: None,
        messages: vec![Message::prompt_text(Role::User, prompt)],
        tools: Vec::new(),
        temperature: None,
        max_output_tokens: Some(64),
        prompt_cache_key,
        previous_response_id: None,
        prompt_window_generation: None,
        stop_sequences: Vec::new(),
        top_p: None,
        top_k: None,
        seed: None,
        thinking: None,
        response_format: None,
    }
}

async fn collect_stream_observation(
    provider_registry: &Arc<ProviderRegistry>,
    provider_id: &str,
    model: &str,
    prompt: String,
    prompt_cache_key: Option<String>,
) -> StreamObservation {
    let mut stream = provider_registry
        .complete_stream(
            &ModelRef::new(provider_id, model.to_owned()),
            completion_request(model, prompt, prompt_cache_key),
        )
        .await
        .unwrap_or_else(|error| panic!("stream request should succeed for {provider_id}: {error}"));

    let mut observation = StreamObservation {
        text: String::new(),
        text_delta_count: 0,
        completion_count: 0,
        usage: None,
        provider_metadata: None,
    };

    while let Some(item) = stream.next().await {
        match item.unwrap_or_else(|error| panic!("stream chunk should succeed: {error}")) {
            CompletionStreamEvent::TextDelta { delta, .. } => {
                observation.text_delta_count += 1;
                observation.text.push_str(delta.as_str());
            }
            CompletionStreamEvent::ThinkingDelta { .. } => {}
            CompletionStreamEvent::ToolCallDelta { .. } => {}
            CompletionStreamEvent::Completed {
                usage,
                provider_metadata,
                ..
            } => {
                observation.completion_count += 1;
                observation.usage = usage;
                observation.provider_metadata = provider_metadata;
            }
        }
    }

    observation
}

fn run_cache_probe_with_nonce(
    config: &LiveConfig,
    nonce: &str,
    attempts: u32,
    retry_delay_secs: f64,
) -> Value {
    let script_path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../scripts/cliproxy_cache_probe.py");
    let output = Command::new("python3")
        .arg(script_path)
        .arg("--base-url")
        .arg(config.base_url.as_str())
        .arg("--api-key")
        .arg(config.api_key.as_str())
        .arg("--model")
        .arg(config.model.as_str())
        .arg("--nonce")
        .arg(nonce)
        .arg("--attempts")
        .arg(attempts.to_string())
        .arg("--retry-delay-secs")
        .arg(retry_delay_secs.to_string())
        .output()
        .expect("python3 should be available for cache probe");
    assert!(
        output.status.success(),
        "cache probe should succeed: status={} stderr={}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "cache probe should emit json: {error}; stdout={}",
            String::from_utf8_lossy(&output.stdout)
        )
    })
}

fn json_array_i64_field(items: &Value, field: &str) -> Vec<i64> {
    items
        .as_array()
        .expect("items should be an array")
        .iter()
        .map(|item| {
            item[field]
                .as_i64()
                .unwrap_or_else(|| panic!("field `{field}` should be an i64 in {item}"))
        })
        .collect()
}

fn json_message_has_non_empty_text(message: &Value) -> bool {
    message["parts"].as_array().is_some_and(|parts| {
        parts.iter().any(|part| {
            part["content"]["type"] == json!("text")
                && part["content"]["text"]
                    .as_str()
                    .is_some_and(|text| !text.trim().is_empty())
        })
    })
}

async fn json_response(app: Router, request: Request<Body>) -> (StatusCode, Value) {
    let response = app.oneshot(request).await.expect("request should succeed");
    let status = response.status();
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body should read");
    let json = serde_json::from_slice(&body).unwrap_or_else(|error| {
        panic!(
            "response should be json: {error}; body={}",
            String::from_utf8_lossy(&body)
        )
    });
    (status, json)
}

async fn text_response(app: Router, request: Request<Body>) -> (StatusCode, String) {
    let response = app.oneshot(request).await.expect("request should succeed");
    let status = response.status();
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body should read");
    (status, String::from_utf8_lossy(&body).to_string())
}

fn latest_assistant_message(messages_json: &Value) -> &Value {
    messages_json["items"]
        .as_array()
        .expect("message items should be an array")
        .iter()
        .rev()
        .find(|item| item["role"] == json!("assistant"))
        .expect("assistant message should exist")
}

fn first_text_part_id(message: &Value) -> Option<i64> {
    message["parts"].as_array().and_then(|parts| {
        parts.iter().find_map(|part| {
            (part["content"]["type"] == json!("text"))
                .then(|| part["id"].as_i64())
                .flatten()
        })
    })
}

fn json_message_text(message: &Value) -> Option<String> {
    message["parts"].as_array().and_then(|parts| {
        parts.iter().find_map(|part| {
            if part["content"]["type"] == json!("text") {
                part["content"]["text"].as_str().map(ToOwned::to_owned)
            } else {
                None
            }
        })
    })
}

fn completed_builtin_tools(messages_json: &Value) -> BTreeSet<String> {
    let mut tools = BTreeSet::new();
    if let Some(items) = messages_json["items"].as_array() {
        for message in items {
            let Some(parts) = message["parts"].as_array() else {
                continue;
            };
            for part in parts {
                if part["content"]["type"] != json!("tool_execution")
                    || part["content"]["state"] != json!("completed")
                    || part["content"]["invocation"]["source"] != json!("builtin")
                {
                    continue;
                }
                if let Some(tool_name) = part["content"]["invocation"]["input"]["tool"].as_str() {
                    tools.insert(tool_name.to_string());
                }
            }
        }
    }
    tools
}

async fn assert_openai_real_tool_flow_once() {
    let harness = build_live_harness()
        .await
        .expect("live harness should exist when env is configured");
    let app = harness.app.clone();
    let bash_output_path = harness.workspace_root.join("bash_live.txt");
    if bash_output_path.exists() {
        fs::remove_file(&bash_output_path).expect("existing bash output should be removable");
    }

    let (workspace_status, workspace_json) = json_response(
        app.clone(),
        Request::builder()
            .method(Method::POST)
            .uri("/api/v1/workspaces")
            .header("content-type", "application/json")
            .body(Body::from(
                json!({ "path": harness.workspace_root.display().to_string() }).to_string(),
            ))
            .expect("request should build"),
    )
    .await;
    assert_eq!(
        workspace_status,
        StatusCode::OK,
        "unexpected body: {workspace_json}"
    );
    let workspace_id = workspace_json["id"]
        .as_i64()
        .expect("workspace id should exist");

    let (session_status, session_json) = json_response(
        app.clone(),
        Request::builder()
            .method(Method::POST)
            .uri("/api/v1/sessions")
            .header("content-type", "application/json")
            .body(Body::from(
                json!({
                    "workspace_id": workspace_id,
                    "title": "live-openai-tool-flow"
                })
                .to_string(),
            ))
            .expect("request should build"),
    )
    .await;
    assert_eq!(
        session_status,
        StatusCode::OK,
        "unexpected body: {session_json}"
    );
    let session_id = session_json["id"]
        .as_i64()
        .expect("session id should exist");

    let prompt = r#"You are running a deterministic integration test inside agena.

Use agena built-in tools in this exact order, and every tool call must use valid JSON arguments:
1. Call `glob` with pattern `**/*.md`.
2. Call `grep` with pattern `cache marker`.
3. Call `read` with file_path `README.md`.
4. Call `tool_search` and load `bash`.
5. Call `bash` with description `write bash output` and command `printf 'bash-live\n' > bash_live.txt`.
6. After every tool call succeeds, answer with exactly `tool flow complete`.

Do not call `apply_patch`, `todo_write`, `ask_user`, `task`, or any other tool. Do not skip steps. Do not explain anything."#;

    let (turn_status, turn_json) = json_response(
        app.clone(),
        Request::builder()
            .method(Method::POST)
            .uri(format!("/api/v1/sessions/{session_id}/turns"))
            .header("content-type", "application/json")
            .body(Body::from(
                json!({
                    "model": {
                        "provider_id": "openai",
                        "model_id": harness.config.model
                    },
                    "max_output_tokens": 192,
                    "parts": [
                        {
                            "type": "text",
                            "text": prompt
                        }
                    ]
                })
                .to_string(),
            ))
            .expect("request should build"),
    )
    .await;
    assert_eq!(turn_status, StatusCode::OK, "unexpected body: {turn_json}");
    assert_eq!(turn_json["blocked"], json!(false));

    let bash_output = fs::read_to_string(&bash_output_path)
        .expect("bash tool should create the expected output file");
    assert_eq!(bash_output, "bash-live\n");

    let (messages_status, messages_json) = json_response(
        app.clone(),
        Request::builder()
            .method(Method::GET)
            .uri(format!(
                "/api/v1/sessions/{session_id}/messages?limit=200&parts=full"
            ))
            .body(Body::empty())
            .expect("request should build"),
    )
    .await;
    assert_eq!(
        messages_status,
        StatusCode::OK,
        "unexpected body: {messages_json}"
    );
    let completed_tools = completed_builtin_tools(&messages_json);
    for required in ["glob", "grep", "read", "tool_search", "bash"] {
        assert!(
            completed_tools.contains(required),
            "expected completed tool `{required}`, got tools={completed_tools:?} messages={messages_json}"
        );
    }

    let assistant_message = latest_assistant_message(&messages_json);
    assert!(
        json_message_text(assistant_message)
            .is_some_and(|text| text.to_ascii_lowercase().contains("tool flow complete")),
        "expected final assistant acknowledgement, got {messages_json}"
    );

    let (events_status, events_json) = json_response(
        app.clone(),
        Request::builder()
            .method(Method::GET)
            .uri(format!("/api/v1/sessions/{session_id}/events?limit=200"))
            .body(Body::empty())
            .expect("request should build"),
    )
    .await;
    assert_eq!(
        events_status,
        StatusCode::OK,
        "unexpected body: {events_json}"
    );
    assert!(
        events_json["items"]
            .as_array()
            .is_some_and(|items| items.len() >= 2),
        "expected tool-flow session events, got {events_json}"
    );
}

fn write_temp_config(contents: &str) -> PathBuf {
    let path = temp_dir("agena-live-config").join("config.toml");
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("config directory should exist");
    }
    fs::write(&path, contents).expect("config should be written");
    path
}

fn temp_dir(prefix: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!(
        "{prefix}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should advance")
            .as_nanos(),
    ));
    path
}
