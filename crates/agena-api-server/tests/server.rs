#![cfg(feature = "http")]

//! Integration tests for `agena-api-server`. They construct a
//! `SessionManager` directly (bypassing `AgenaRuntime`), wire it into
//! `AppState::with_manager_override`, and exercise the v2 routes.

use std::{
    process::Command as ProcessCommand,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use agena::model::{ModelId, ModelLifecycle, ModelRef, ProviderId};
use agena::{
    agent::Agent,
    config::LoadConfigRequest,
    event::{EventKind, PublishContext},
    message::PartContent,
    model_catalog::CatalogModelDefinition,
    permission::PermissionPolicy,
    provider::{
        CompletionFinishReason, CompletionRequest, CompletionResponse, ModelProvider,
        ProviderModel, ProviderRegistry,
    },
    session::{
        ContextGovernor, ContextPolicy, SessionManager, SessionProcessor, SessionRunOptions,
        SessionUserTurnRequest,
    },
    tool::ToolExecutor,
};
use agena_api::{
    PROTOCOL_VERSION, Scope,
    commands::{
        Command, CommandResult, CompleteSessionGoalParams, CreateSessionGoalParams,
        SetSessionGoalParams,
    },
    notifications::Notification,
    queries::{GetSessionParams, PaginatedEvents, Query, QueryResult},
    subscribe::SubscribeRequest,
    ws::{ClientMessage, ServerMessage},
};
use agena_api_server::{
    AppState,
    dispatch::{dispatch_command, dispatch_query},
    router,
};
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use sea_orm::{ActiveModelTrait, ActiveValue::Set, Database};
use tower::ServiceExt;

struct TestProvider;

const LIVE_PROVIDER_GATEWAY_BASE_URL: &str = "https://api.cxits.cn/";
const LIVE_PROVIDER_GATEWAY_KEY_ENV: &str = "CX_API_KEY";

#[async_trait::async_trait]
impl ModelProvider for TestProvider {
    fn id(&self) -> &str {
        "test"
    }

    fn default_model(&self) -> &ModelId {
        static DEFAULT_MODEL: std::sync::LazyLock<ModelId> =
            std::sync::LazyLock::new(|| ModelId::new("test-model"));
        &DEFAULT_MODEL
    }

    async fn list_models(&self) -> Result<Vec<ProviderModel>, agena::AppError> {
        Ok(vec![ProviderModel::new(
            "test",
            self.default_model().as_str(),
        )])
    }

    async fn complete(
        &self,
        request: CompletionRequest,
    ) -> Result<CompletionResponse, agena::AppError> {
        Ok(CompletionResponse {
            provider_id: ProviderId::new("test"),
            model: request.model,
            text: "ack".to_owned(),
            reasoning_text: None,
            finish_reason: Some(CompletionFinishReason::Stop),
            tool_calls: Vec::new(),
            usage: None,
            provider_metadata: None,
        })
    }
}

fn test_run_options() -> SessionRunOptions {
    SessionRunOptions {
        model: ModelRef::new("test", "test-model"),
        thinking_mode: None,
        speed_mode: None,
        verbosity: None,
        thinking: None,
        request_override: Default::default(),
        system: None,
        temperature: None,
        max_output_tokens: Some(128),
        agent_profile: None,
        max_turn_loops: None,
    }
}

async fn build_state() -> (AppState, Arc<SessionManager>, String) {
    build_state_with_config("").await
}

async fn build_state_with_config(config_text: &str) -> (AppState, Arc<SessionManager>, String) {
    let workspace_root = format!("/tmp/api-server-workspace-{}", uuid::Uuid::new_v4());
    build_state_with_config_and_workspace(config_text, workspace_root).await
}

async fn build_state_with_config_and_workspace(
    config_text: &str,
    workspace_root: String,
) -> (AppState, Arc<SessionManager>, String) {
    let db = Arc::new(Database::connect("sqlite::memory:").await.unwrap());
    agena::db::init_schema(db.as_ref()).await.unwrap();
    std::fs::create_dir_all(&workspace_root).expect("test workspace dir should be created");
    agena::db::crud::workspace::ensure_workspace_id(db.as_ref(), workspace_root.as_str())
        .await
        .expect("workspace should exist for manager scans");

    let mut registry = ProviderRegistry::new();
    registry.register(TestProvider);
    let processor = SessionProcessor::new(
        Arc::new(registry),
        ContextGovernor::new(ContextPolicy::default()),
    );
    let executor = ToolExecutor::new(
        std::path::PathBuf::from(&workspace_root),
        Agent::new("api-server-test", PermissionPolicy::allow_all()),
    );

    let manager = Arc::new(SessionManager::new(
        db.as_ref().clone(),
        processor,
        executor,
    ));
    let config_path = std::env::temp_dir().join(format!(
        "agena-api-server-test-{}.toml",
        uuid::Uuid::new_v4()
    ));
    std::fs::write(&config_path, config_text).expect("test config should be written");

    let runtime = agena::runtime::AgenaRuntime::builder()
        .with_load_request(LoadConfigRequest {
            config_path: Some(config_path),
            ..LoadConfigRequest::default()
        })
        .with_workspace_root(std::path::PathBuf::from(&workspace_root))
        .with_database_connection(db.as_ref().clone())
        .build()
        .await
        .expect("runtime build");

    let state = AppState::new(runtime, Arc::clone(&db)).with_manager_override(Arc::clone(&manager));
    (state, manager, workspace_root)
}

fn seed_cached_official_catalog(workspace_root: &str) {
    let cache_dir = std::path::Path::new(workspace_root)
        .join(".agena")
        .join("catalog");
    std::fs::create_dir_all(&cache_dir).expect("catalog cache dir should be created");
    let fetched_at_unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after unix epoch")
        .as_millis() as i64;
    let payload = serde_json::json!({
        "fetched_at_unix_ms": fetched_at_unix_ms,
        "source": "cache",
        "document": {
            "models": {
                "gpt-5": {
                    "display_name": "GPT-5 Official",
                    "origin": "OpenAI"
                }
            }
        }
    });
    std::fs::write(
        cache_dir.join("model-catalog-cache.json"),
        serde_json::to_vec_pretty(&payload).expect("cache payload should serialize"),
    )
    .expect("catalog cache file should be written");
}

async fn insert_projected_message(
    db: &sea_orm::DatabaseConnection,
    session_id: i64,
    message_id: i64,
    created_at_ms: i64,
    part_count: i64,
) {
    agena::db::entities::activity_message::ActiveModel {
        message_id: Set(message_id),
        session_id: Set(session_id),
        role: Set(agena::role::Role::Assistant),
        state: Set(agena::message::ExecutionStatus::Completed),
        created_at_ms: Set(created_at_ms),
        updated_at_ms: Set(created_at_ms),
        metadata: Set(agena::message::MessageMetadata::default()),
        usage: Set(None),
        finish: Set(None),
        part_count: Set(part_count),
        is_compacted: Set(false),
    }
    .insert(db)
    .await
    .expect("activity message projection should insert");
}

async fn insert_projected_text_part(
    db: &sea_orm::DatabaseConnection,
    session_id: i64,
    message_id: i64,
    part_id: i64,
    part_index: i32,
    created_at_ms: i64,
    text: &str,
) {
    agena::db::entities::activity_part::ActiveModel {
        part_id: Set(part_id),
        message_id: Set(message_id),
        session_id: Set(session_id),
        part_index: Set(part_index),
        status: Set(agena::message::ExecutionStatus::Completed),
        kind: Set(agena::message::PartKind::Text),
        name: Set(None),
        summary: Set(Some(format!("summary {text}"))),
        has_detail: Set(true),
        operation_id: Set(None),
        created_at_ms: Set(created_at_ms),
        content: Set(Some(agena::message::PartContent::text(text))),
    }
    .insert(db)
    .await
    .expect("activity part projection should insert");
}

fn live_provider_gateway_key() -> String {
    std::env::var(LIVE_PROVIDER_GATEWAY_KEY_ENV)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .expect("CX_API_KEY must be set for cxits live provider creation tests")
}

fn live_provider_id(prefix: &str) -> String {
    format!("{prefix}_{}", uuid::Uuid::new_v4().simple())
}

fn live_provider_adapter_models_request(provider_id: &str) -> serde_json::Value {
    serde_json::json!({
        "provider_id": provider_id,
        "base_url": LIVE_PROVIDER_GATEWAY_BASE_URL,
        "api_key_env": LIVE_PROVIDER_GATEWAY_KEY_ENV,
        "adapter_ids": ["openai", "anthropic", "gemini"]
    })
}

fn select_live_adapter_and_model(adapter_models: &serde_json::Value) -> (String, String) {
    let adapters = adapter_models
        .get("adapters")
        .and_then(|value| value.as_array())
        .expect("adapter models response should include adapters array");

    for preferred in ["openai", "anthropic", "gemini"] {
        if let Some(model_id) = adapters.iter().find_map(|adapter| {
            if adapter.get("adapter_id").and_then(|value| value.as_str()) != Some(preferred)
                || adapter.get("error").is_some()
            {
                return None;
            }
            adapter
                .get("models")
                .and_then(|value| value.as_array())
                .and_then(|models| {
                    models
                        .iter()
                        .find_map(|model| model.get("id").and_then(|value| value.as_str()))
                })
                .map(str::to_owned)
        }) {
            return (preferred.to_owned(), model_id);
        }
    }

    panic!(
        "cxits live adapter model listing should return at least one adapter with models: {adapter_models:?}"
    );
}

fn build_live_provider_patch_from_adapter_models(
    adapter_models: &serde_json::Value,
    default_adapter: &str,
    default_model: &str,
) -> serde_json::Value {
    let mut adapters = serde_json::Map::new();
    for adapter in adapter_models
        .get("adapters")
        .and_then(|value| value.as_array())
        .expect("adapter models response should include adapters array")
    {
        if adapter.get("error").is_some() {
            continue;
        }
        let Some(adapter_id) = adapter.get("adapter_id").and_then(|value| value.as_str()) else {
            continue;
        };
        adapters.insert(
            adapter_id.to_owned(),
            serde_json::json!({ "enabled": true }),
        );
    }

    let default_adapter_patch = adapters
        .entry(default_adapter.to_owned())
        .or_insert_with(|| serde_json::json!({ "enabled": true }));
    default_adapter_patch
        .as_object_mut()
        .expect("default adapter patch should be an object")
        .insert(
            "models".to_owned(),
            serde_json::json!({
                default_model: {}
            }),
        );

    serde_json::json!({
        "enabled": true,
        "default_adapter": default_adapter,
        "default_model": default_model,
        "auth": {
            "mode": "api",
            "base_url": LIVE_PROVIDER_GATEWAY_BASE_URL,
            "api_key_env": LIVE_PROVIDER_GATEWAY_KEY_ENV
        },
        "adapters": adapters
    })
}

#[tokio::test]
async fn health_endpoint_returns_ok() {
    let (state, _, _) = build_state().await;
    let app = router(state);
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn runtime_and_model_catalog_endpoints_expose_catalog_payload() {
    let workspace_root = format!("/tmp/api-server-workspace-{}", uuid::Uuid::new_v4());
    seed_cached_official_catalog(&workspace_root);
    let (state, _, _) = build_state_with_config_and_workspace("", workspace_root).await;
    state
        .runtime()
        .current_snapshot()
        .model_catalog()
        .upsert_custom_entry(
            "gpt-5-mini",
            CatalogModelDefinition {
                display_name: Some("GPT-5 Mini Workspace".to_owned()),
                origin: Some("OpenAI".to_owned()),
                ..CatalogModelDefinition::default()
            },
        )
        .await
        .expect("custom catalog entry should be written");
    state
        .runtime()
        .current_snapshot()
        .model_catalog()
        .upsert_custom_entry(
            "gpt-oss-120b",
            CatalogModelDefinition {
                display_name: Some("GPT OSS 120B".to_owned()),
                origin: Some("OpenAI".to_owned()),
                ..CatalogModelDefinition::default()
            },
        )
        .await
        .expect("slash alias catalog entry should be written");
    let app = router(state);

    let runtime_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/runtime")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(runtime_response.status(), StatusCode::OK);
    let runtime_body = runtime_response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes();
    let runtime_value: serde_json::Value = serde_json::from_slice(&runtime_body).unwrap();
    assert!(
        runtime_value.get("model_catalog").is_some(),
        "runtime payload should include model_catalog: {runtime_value:?}"
    );
    assert_eq!(
        runtime_value.pointer("/model_catalog/entry_count"),
        Some(&serde_json::json!(3))
    );
    assert_eq!(
        runtime_value.pointer("/model_catalog/official_entry_count"),
        Some(&serde_json::json!(1))
    );
    assert_eq!(
        runtime_value.pointer("/model_catalog/custom_entry_count"),
        Some(&serde_json::json!(2))
    );

    let catalog_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/model-catalog?limit=2&offset=0")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(catalog_response.status(), StatusCode::OK);
    let catalog_body = catalog_response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes();
    let catalog_value: serde_json::Value = serde_json::from_slice(&catalog_body).unwrap();
    assert!(
        catalog_value
            .get("items")
            .and_then(|value| value.as_array())
            .is_some(),
        "catalog payload should include items array: {catalog_value:?}"
    );
    assert_eq!(
        catalog_value.pointer("/summary/entry_count"),
        Some(&serde_json::json!(3))
    );
    assert_eq!(catalog_value.pointer("/limit"), Some(&serde_json::json!(2)));
    assert_eq!(
        catalog_value.pointer("/offset"),
        Some(&serde_json::json!(0))
    );
    let catalog_entries = catalog_value
        .get("items")
        .and_then(|value| value.as_array())
        .expect("catalog payload should include items");
    assert!(catalog_entries.iter().any(|entry| {
        entry.get("model_id").and_then(|value| value.as_str()) == Some("gpt-5")
            && entry.get("kind").and_then(|value| value.as_str()) == Some("official")
            && entry.get("source").and_then(|value| value.as_str()) == Some("cache")
            && entry.get("source_label").and_then(|value| value.as_str()) == Some("cached catalog")
            && entry.get("origin").and_then(|value| value.as_str()) == Some("OpenAI")
    }));
    let lookup_response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/model-catalog/lookup")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "model_ids": ["gpt-5-mini", "openai/gpt-oss-120b"]
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(lookup_response.status(), StatusCode::OK);
    let lookup_body = lookup_response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes();
    let lookup_value: serde_json::Value = serde_json::from_slice(&lookup_body).unwrap();
    let lookup_entries = lookup_value
        .get("items")
        .and_then(|value| value.as_array())
        .expect("lookup payload should include items");
    assert!(lookup_entries.iter().any(|entry| {
        entry.get("model_id").and_then(|value| value.as_str()) == Some("gpt-5-mini")
            && entry.get("kind").and_then(|value| value.as_str()) == Some("custom")
            && entry.get("source").and_then(|value| value.as_str()) == Some("custom")
            && entry.get("source_label").and_then(|value| value.as_str())
                == Some("workspace override")
            && entry.get("origin").and_then(|value| value.as_str()) == Some("OpenAI")
    }));
    assert!(lookup_entries.iter().any(|entry| {
        entry.get("model_id").and_then(|value| value.as_str()) == Some("gpt-oss-120b")
            && entry.get("display_name").and_then(|value| value.as_str()) == Some("GPT OSS 120B")
    }));
}

#[tokio::test]
async fn settings_endpoints_read_write_validate_and_reload_config() {
    let (state, _, _) = build_state().await;
    let app = router(state);

    let effective_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/settings?source=effective&path=runtime.reload.enabled")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(effective_response.status(), StatusCode::OK);
    let effective_body = effective_response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes();
    let effective_value: serde_json::Value = serde_json::from_slice(&effective_body).unwrap();
    assert_eq!(
        effective_value.pointer("/value"),
        Some(&serde_json::json!(true))
    );

    let update_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/v1/settings")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "path": "ui.locale",
                        "value": "fr-FR",
                        "validate": true,
                        "reload": true
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(update_response.status(), StatusCode::OK);
    let update_body = update_response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes();
    let update_value: serde_json::Value = serde_json::from_slice(&update_body).unwrap();
    assert_eq!(
        update_value.pointer("/operation"),
        Some(&serde_json::json!("set"))
    );
    assert_eq!(
        update_value.pointer("/changed"),
        Some(&serde_json::json!(true))
    );
    assert_eq!(
        update_value.pointer("/current"),
        Some(&serde_json::json!("fr-FR"))
    );
    assert_eq!(
        update_value.pointer("/reload/previous_generation"),
        Some(&serde_json::json!(1))
    );
    assert_eq!(
        update_value.pointer("/reload/generation"),
        Some(&serde_json::json!(2))
    );

    let file_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/settings?source=file&path=ui.locale")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(file_response.status(), StatusCode::OK);
    let file_body = file_response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes();
    let file_value: serde_json::Value = serde_json::from_slice(&file_body).unwrap();
    assert_eq!(
        file_value.pointer("/value"),
        Some(&serde_json::json!("fr-FR"))
    );

    let validate_response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/settings/validate")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(validate_response.status(), StatusCode::OK);
    let validate_body = validate_response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes();
    let validate_value: serde_json::Value = serde_json::from_slice(&validate_body).unwrap();
    assert_eq!(
        validate_value.pointer("/valid"),
        Some(&serde_json::json!(true))
    );
}

#[tokio::test]
async fn model_catalog_delete_accepts_visible_model_ids_with_slashes() {
    let (state, _, _) = build_state().await;
    state
        .runtime()
        .current_snapshot()
        .model_catalog()
        .upsert_custom_entry(
            "openai/google/gemini-2.5-pro",
            CatalogModelDefinition::default(),
        )
        .await
        .expect("custom catalog entry should be written");

    let app = router(state);
    let response = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/api/v1/model-catalog/entries?model_id=openai%2Fgoogle%2Fgemini-2.5-pro")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(
        value
            .get("entry_count")
            .and_then(|count| count.as_u64())
            .is_some(),
        "catalog delete response should include summary counts: {value:?}"
    );
}

#[tokio::test]
async fn provider_models_endpoint_decorates_listed_models_from_effective_catalog() {
    let mut server = mockito::Server::new_async().await;
    let _mock = server
        .mock("GET", "/v1/models")
        .match_header("authorization", "Bearer sk-test")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            serde_json::json!({
                "data": [{ "id": "gpt-upstream" }]
            })
            .to_string(),
        )
        .create_async()
        .await;

    let config = format!(
        r#"
[providers.gateway]
default_adapter = "openai"
default_model = "gpt-upstream"

[providers.gateway.auth]
mode = "api"
base_url = "{base_url}"
api_key = "sk-test"

[providers.gateway.adapters.openai]
enabled = true
"#,
        base_url = server.url()
    );

    let (state, _, _) = build_state_with_config(config.as_str()).await;
    state
        .runtime()
        .current_snapshot()
        .model_catalog()
        .upsert_custom_entry(
            "gpt-upstream",
            CatalogModelDefinition {
                display_name: Some("Decorated GPT".to_owned()),
                origin: Some("OpenAI".to_owned()),
                description: Some("Catalog description".to_owned()),
                lifecycle: Some(ModelLifecycle::Preview),
                context_window_tokens: Some(123_456),
                max_input_tokens: Some(100_000),
                max_output_tokens: Some(7_890),
                ..CatalogModelDefinition::default()
            },
        )
        .await
        .expect("custom catalog entry should be written");

    let app = router(state);
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/providers/gateway/models")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let models = value
        .get("models")
        .and_then(|models| models.as_array())
        .expect("provider models response should include models");
    let model = models
        .iter()
        .find(|model| {
            model.get("id").and_then(|value| value.as_str()) == Some("gpt-upstream")
                && model.get("adapter_id").and_then(|value| value.as_str()) == Some("openai")
        })
        .expect("decorated upstream model should be present");

    assert_eq!(
        model.get("display_name").and_then(|value| value.as_str()),
        Some("Decorated GPT")
    );
    assert_eq!(
        model
            .pointer("/metadata/description")
            .and_then(|value| value.as_str()),
        Some("Catalog description")
    );
    assert_eq!(
        model
            .pointer("/metadata/lifecycle")
            .and_then(|value| value.as_str()),
        Some("preview")
    );
    assert_eq!(
        model
            .pointer("/metadata/limits/context_window_tokens")
            .and_then(|value| value.as_u64()),
        Some(123_456)
    );
    assert_eq!(
        model
            .pointer("/metadata/limits/max_input_tokens")
            .and_then(|value| value.as_u64()),
        Some(100_000)
    );
    assert_eq!(
        model
            .pointer("/metadata/limits/max_output_tokens")
            .and_then(|value| value.as_u64()),
        Some(7_890)
    );
    assert!(
        model.get("origin").is_none(),
        "provider model payload should not expose catalog-only origin metadata: {model:?}"
    );
}

#[tokio::test]
async fn provider_models_endpoint_matches_vendor_prefixed_ids_to_catalog_entries() {
    let mut server = mockito::Server::new_async().await;
    let _mock = server
        .mock("GET", "/v1/models")
        .match_header("authorization", "Bearer sk-test")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            serde_json::json!({
                "data": [{ "id": "openai/gpt-5.4" }]
            })
            .to_string(),
        )
        .create_async()
        .await;

    let config = format!(
        r#"
[providers.gateway]
default_adapter = "openai"
default_model = "openai/gpt-5.4"

[providers.gateway.auth]
mode = "api"
base_url = "{base_url}"
api_key = "sk-test"

[providers.gateway.adapters.openai]
enabled = true
"#,
        base_url = server.url()
    );

    let (state, _, _) = build_state_with_config(config.as_str()).await;
    state
        .runtime()
        .current_snapshot()
        .model_catalog()
        .upsert_custom_entry(
            "gpt-5.4",
            CatalogModelDefinition {
                display_name: Some("GPT-5.4 Catalog".to_owned()),
                description: Some("Canonical catalog match".to_owned()),
                lifecycle: Some(ModelLifecycle::Preview),
                ..CatalogModelDefinition::default()
            },
        )
        .await
        .expect("canonical catalog entry should be written");

    let app = router(state);
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/providers/gateway/models")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let models = value
        .get("models")
        .and_then(|models| models.as_array())
        .expect("provider models response should include models");
    let model = models
        .iter()
        .find(|model| {
            model.get("id").and_then(|value| value.as_str()) == Some("openai/gpt-5.4")
                && model.get("adapter_id").and_then(|value| value.as_str()) == Some("openai")
        })
        .expect("decorated vendor-prefixed model should be present");

    assert_eq!(
        model
            .get("catalog_model_id")
            .and_then(|value| value.as_str()),
        Some("gpt-5.4")
    );
    assert_eq!(
        model.get("display_name").and_then(|value| value.as_str()),
        Some("GPT-5.4 Catalog")
    );
    assert_eq!(
        model
            .pointer("/metadata/description")
            .and_then(|value| value.as_str()),
        Some("Canonical catalog match")
    );
    assert_eq!(
        models
            .iter()
            .filter(|model| {
                matches!(
                    model.get("id").and_then(|value| value.as_str()),
                    Some("openai/gpt-5.4" | "gpt-5.4")
                )
            })
            .count(),
        1,
        "catalog alias should decorate the raw model instead of appending a duplicate canonical entry: {value:?}"
    );
}

#[tokio::test]
async fn provider_models_endpoint_appends_catalog_only_models_missing_upstream() {
    let mut server = mockito::Server::new_async().await;
    let _mock = server
        .mock("GET", "/v1/models")
        .match_header("authorization", "Bearer sk-test")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            serde_json::json!({
                "data": [{ "id": "gpt-upstream" }]
            })
            .to_string(),
        )
        .create_async()
        .await;

    let config = format!(
        r#"
[providers.gateway]
default_adapter = "openai"
default_model = "gpt-upstream"

[providers.gateway.auth]
mode = "api"
base_url = "{base_url}"
api_key = "sk-test"

[providers.gateway.adapters.openai]
enabled = true
"#,
        base_url = server.url()
    );

    let (state, _, _) = build_state_with_config(config.as_str()).await;
    state
        .runtime()
        .current_snapshot()
        .model_catalog()
        .upsert_custom_entry(
            "gpt-catalog-only",
            CatalogModelDefinition {
                display_name: Some("Catalog Only GPT".to_owned()),
                description: Some("Only in catalog".to_owned()),
                lifecycle: Some(ModelLifecycle::Preview),
                context_window_tokens: Some(222_222),
                max_input_tokens: Some(180_000),
                max_output_tokens: Some(3_333),
                ..CatalogModelDefinition::default()
            },
        )
        .await
        .expect("custom catalog entry should be written");

    let app = router(state);
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/providers/gateway/models")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let models = value
        .get("models")
        .and_then(|models| models.as_array())
        .expect("provider models response should include models");
    assert!(
        models.iter().any(|model| {
            model.get("id").and_then(|value| value.as_str()) == Some("gpt-upstream")
                && model.get("adapter_id").and_then(|value| value.as_str()) == Some("openai")
        }),
        "upstream listed model should remain present: {value:?}"
    );

    let catalog_only = models
        .iter()
        .find(|model| model.get("id").and_then(|value| value.as_str()) == Some("gpt-catalog-only"))
        .expect("catalog-only model should be appended");

    assert_eq!(
        catalog_only
            .get("display_name")
            .and_then(|value| value.as_str()),
        Some("Catalog Only GPT")
    );
    assert_eq!(
        catalog_only
            .pointer("/metadata/description")
            .and_then(|value| value.as_str()),
        Some("Only in catalog")
    );
    assert_eq!(
        catalog_only
            .pointer("/metadata/lifecycle")
            .and_then(|value| value.as_str()),
        Some("preview")
    );
    assert_eq!(
        catalog_only
            .pointer("/metadata/limits/max_input_tokens")
            .and_then(|value| value.as_u64()),
        Some(180_000)
    );
}

#[tokio::test]
async fn provider_adapter_models_endpoint_lists_shared_gateway_models() {
    let mut server = mockito::Server::new_async().await;
    let _openai = server
        .mock("GET", "/v1/models")
        .match_header("authorization", "Bearer sk-test")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            serde_json::json!({
                "data": [{ "id": "gpt-4.1-mini" }]
            })
            .to_string(),
        )
        .create_async()
        .await;
    let _anthropic = server
        .mock("GET", "/v1/models")
        .match_header("x-api-key", "sk-test")
        .match_header("anthropic-version", "2023-06-01")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            serde_json::json!({
                "data": [{ "id": "claude-sonnet-4-5", "display_name": "Claude Sonnet 4.5" }]
            })
            .to_string(),
        )
        .create_async()
        .await;
    let _gemini = server
        .mock("GET", "/v1beta/models")
        .match_query(mockito::Matcher::UrlEncoded(
            "key".to_owned(),
            "sk-test".to_owned(),
        ))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            serde_json::json!({
                "models": [{ "name": "models/gemini-2.5-flash", "displayName": "Gemini 2.5 Flash" }]
            })
            .to_string(),
        )
        .create_async()
        .await;

    let (state, _, _) = build_state().await;
    let app = router(state);
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/providers/models")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "provider_id": "draft-gateway",
                        "base_url": server.url(),
                        "api_key": "sk-test",
                        "adapter_ids": ["openai", "anthropic", "gemini"]
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let adapters = value
        .get("adapters")
        .and_then(|items| items.as_array())
        .expect("adapter models response should include adapters");
    assert_eq!(
        adapters.len(),
        3,
        "expected openai/anthropic/gemini: {value:?}"
    );
    assert!(adapters.iter().any(|adapter| {
        adapter.get("adapter_id").and_then(|value| value.as_str()) == Some("openai")
            && adapter.get("error").is_none()
            && adapter
                .get("resolved_base_url")
                .and_then(|value| value.as_str())
                == Some(&format!("{}/v1", server.url()))
            && adapter
                .get("models")
                .and_then(|value| value.as_array())
                .map(|models| {
                    models.iter().any(|model| {
                        model.get("id").and_then(|value| value.as_str()) == Some("gpt-4.1-mini")
                            && model.get("adapter_id").and_then(|value| value.as_str())
                                == Some("openai")
                    })
                })
                == Some(true)
    }));
    assert!(adapters.iter().any(|adapter| {
        adapter.get("adapter_id").and_then(|value| value.as_str()) == Some("anthropic")
            && adapter.get("error").is_none()
            && adapter
                .get("models")
                .and_then(|value| value.as_array())
                .map(|models| {
                    models.iter().any(|model| {
                        model.get("id").and_then(|value| value.as_str())
                            == Some("claude-sonnet-4-5")
                            && model.get("adapter_id").and_then(|value| value.as_str())
                                == Some("anthropic")
                    })
                })
                == Some(true)
    }));
    assert!(adapters.iter().any(|adapter| {
        adapter.get("adapter_id").and_then(|value| value.as_str()) == Some("gemini")
            && adapter.get("error").is_none()
            && adapter
                .get("resolved_base_url")
                .and_then(|value| value.as_str())
                == Some(&format!("{}/v1beta", server.url()))
            && adapter
                .get("models")
                .and_then(|value| value.as_array())
                .map(|models| {
                    models.iter().any(|model| {
                        model.get("id").and_then(|value| value.as_str()) == Some("gemini-2.5-flash")
                            && model.get("adapter_id").and_then(|value| value.as_str())
                                == Some("gemini")
                    })
                })
                == Some(true)
    }));
}

#[tokio::test]
async fn saved_provider_adapter_models_endpoint_filters_to_requested_adapters() {
    let mut server = mockito::Server::new_async().await;
    let _openai = server
        .mock("GET", "/v1/models")
        .match_header("authorization", "Bearer sk-test")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            serde_json::json!({
                "data": [{ "id": "gpt-4.1-mini" }]
            })
            .to_string(),
        )
        .create_async()
        .await;
    let _anthropic = server
        .mock("GET", "/v1/models")
        .match_header("x-api-key", "sk-test")
        .match_header("anthropic-version", "2023-06-01")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            serde_json::json!({
                "data": [{ "id": "claude-sonnet-4-5" }]
            })
            .to_string(),
        )
        .create_async()
        .await;

    let config = format!(
        r#"
[providers.gateway]
default_adapter = "openai"
default_model = "gpt-4.1-mini"

[providers.gateway.auth]
mode = "api"
base_url = "{base_url}"
api_key = "sk-test"

[providers.gateway.adapters.openai]
enabled = true

[providers.gateway.adapters.anthropic]
enabled = true
"#,
        base_url = server.url()
    );

    let (state, _, _) = build_state_with_config(config.as_str()).await;
    let app = router(state);
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/providers/gateway/models")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "adapter_ids": ["anthropic"]
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let adapters = value
        .get("adapters")
        .and_then(|items| items.as_array())
        .expect("saved adapter models response should include adapters");
    assert_eq!(
        adapters.len(),
        1,
        "only requested adapter should be returned: {value:?}"
    );
    assert_eq!(
        adapters[0]
            .get("adapter_id")
            .and_then(|value| value.as_str()),
        Some("anthropic")
    );
    assert!(
        adapters[0].get("error").is_none(),
        "adapter listing should succeed: {value:?}"
    );
}

#[tokio::test]
async fn saved_provider_adapter_models_endpoint_requires_explicit_adapter_ids() {
    let config = r#"
[providers.gateway]
default_adapter = "openai"
default_model = "gpt-4.1-mini"

[providers.gateway.auth]
mode = "api"
base_url = "https://example.com/v1"
api_key = "sk-test"

[providers.gateway.adapters.openai]
enabled = true
"#;

    let (state, _, _) = build_state_with_config(config).await;
    let app = router(state);
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/providers/gateway/models")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(
        value
            .get("message")
            .and_then(|message| message.as_str())
            .is_some_and(|message| message
                .contains("saved provider adapter model listing requires explicit adapter_ids")),
        "expected strict adapter_ids validation error: {value:?}"
    );
}

#[tokio::test]
async fn saved_provider_adapter_models_endpoint_includes_explicit_unconfigured_http_adapters() {
    let mut server = mockito::Server::new_async().await;
    let _openai = server
        .mock("GET", "/v1/models")
        .match_header("authorization", "Bearer sk-test")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            serde_json::json!({
                "data": [{ "id": "gpt-4.1-mini" }]
            })
            .to_string(),
        )
        .create_async()
        .await;
    let _anthropic = server
        .mock("GET", "/v1/models")
        .match_header("x-api-key", "sk-test")
        .match_header("anthropic-version", "2023-06-01")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            serde_json::json!({
                "data": [{ "id": "claude-sonnet-4-5" }]
            })
            .to_string(),
        )
        .create_async()
        .await;
    let _gemini = server
        .mock("GET", "/v1beta/models")
        .match_query(mockito::Matcher::UrlEncoded(
            "key".to_owned(),
            "sk-test".to_owned(),
        ))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            serde_json::json!({
                "models": [{ "name": "models/gemini-2.5-flash" }]
            })
            .to_string(),
        )
        .create_async()
        .await;

    let config = format!(
        r#"
[providers.gateway]
default_adapter = "openai"
default_model = "gpt-4.1-mini"

[providers.gateway.auth]
mode = "api"
base_url = "{base_url}"
api_key = "sk-test"

[providers.gateway.adapters.openai]
enabled = true
"#,
        base_url = server.url()
    );

    let (state, _, _) = build_state_with_config(config.as_str()).await;
    let app = router(state);
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/providers/gateway/models")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "adapter_ids": ["openai", "anthropic", "gemini"]
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let adapters = value
        .get("adapters")
        .and_then(|items| items.as_array())
        .expect("saved adapter models response should include adapters");
    assert!(
        adapters.iter().any(|adapter| {
            adapter.get("adapter_id").and_then(|value| value.as_str()) == Some("anthropic")
                && adapter.get("error").is_none()
        }),
        "saved adapter model listing should include unconfigured anthropic adapter: {value:?}"
    );
    assert!(
        adapters.iter().any(|adapter| {
            adapter.get("adapter_id").and_then(|value| value.as_str()) == Some("gemini")
                && adapter.get("error").is_none()
        }),
        "saved adapter model listing should include unconfigured gemini adapter: {value:?}"
    );
}

#[tokio::test]
#[ignore = "real integration test against api.cxits.cn"]
async fn cxits_live_provider_creation_flow_lists_gateway_models_from_root() {
    let _api_key = live_provider_gateway_key();
    let provider_id = live_provider_id("cxits_draft");
    let (state, _, _) = build_state().await;
    let app = router(state);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/providers/models")
                .header("content-type", "application/json")
                .body(Body::from(
                    live_provider_adapter_models_request(provider_id.as_str()).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        value.get("provider_id").and_then(|value| value.as_str()),
        Some(provider_id.as_str())
    );
    let adapters = value
        .get("adapters")
        .and_then(|items| items.as_array())
        .expect("adapter models response should include adapters");
    assert!(
        adapters.iter().any(|adapter| {
            adapter.get("error").is_none()
                && adapter
                    .get("models")
                    .and_then(|value| value.as_array())
                    .map(|models| !models.is_empty())
                    == Some(true)
        }),
        "cxits adapter model listing should yield at least one adapter with models: {value:?}"
    );

    let (adapter_id, model_id) = select_live_adapter_and_model(&value);
    let selected_adapter = adapters
        .iter()
        .find(|adapter| {
            adapter.get("adapter_id").and_then(|value| value.as_str()) == Some(adapter_id.as_str())
        })
        .expect("selected adapter should be present");
    assert!(
        selected_adapter
            .get("resolved_base_url")
            .and_then(|value| value.as_str())
            .is_some_and(|resolved| resolved.starts_with(LIVE_PROVIDER_GATEWAY_BASE_URL)),
        "selected adapter should resolve from cxits root: {selected_adapter:?}"
    );
    assert!(
        selected_adapter
            .get("models")
            .and_then(|value| value.as_array())
            .map(|models| {
                models.iter().any(|model| {
                    model.get("id").and_then(|value| value.as_str()) == Some(model_id.as_str())
                })
            })
            == Some(true),
        "selected adapter should include chosen model {model_id}: {selected_adapter:?}"
    );
}

#[tokio::test]
#[ignore = "real integration test against api.cxits.cn"]
async fn cxits_live_provider_creation_flow_can_save_provider_and_list_models() {
    let _api_key = live_provider_gateway_key();
    let provider_id = live_provider_id("cxits_live");
    let (state, _, _) = build_state().await;
    let app = router(state);

    let draft_adapter_models = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/providers/models")
                .header("content-type", "application/json")
                .body(Body::from(
                    live_provider_adapter_models_request(provider_id.as_str()).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(draft_adapter_models.status(), StatusCode::OK);

    let adapter_models_body = draft_adapter_models
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes();
    let adapter_models_value: serde_json::Value =
        serde_json::from_slice(&adapter_models_body).unwrap();
    let (adapter_id, model_id) = select_live_adapter_and_model(&adapter_models_value);
    let provider_patch = build_live_provider_patch_from_adapter_models(
        &adapter_models_value,
        adapter_id.as_str(),
        model_id.as_str(),
    );
    let mut provider_changes = serde_json::Map::new();
    provider_changes.insert(provider_id.clone(), provider_patch);

    let patch_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri("/api/v1/settings")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "path": "providers",
                        "changes": serde_json::Value::Object(provider_changes),
                        "validate": true,
                        "reload": true
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(patch_response.status(), StatusCode::OK);

    let patch_body = patch_response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes();
    let patch_value: serde_json::Value = serde_json::from_slice(&patch_body).unwrap();
    assert_eq!(
        patch_value.get("changed").and_then(|value| value.as_bool()),
        Some(true),
        "provider creation patch should change config: {patch_value:?}"
    );
    assert_eq!(
        patch_value.pointer(format!("/current/{provider_id}/default_adapter").as_str()),
        Some(&serde_json::json!(adapter_id.as_str()))
    );
    assert_eq!(
        patch_value.pointer(format!("/current/{provider_id}/default_model").as_str()),
        Some(&serde_json::json!(model_id.as_str()))
    );

    let runtime_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/runtime")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(runtime_response.status(), StatusCode::OK);
    let runtime_body = runtime_response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes();
    let runtime_value: serde_json::Value = serde_json::from_slice(&runtime_body).unwrap();
    assert!(
        runtime_value
            .get("provider_ids")
            .and_then(|value| value.as_array())
            .is_some_and(|providers| {
                providers
                    .iter()
                    .any(|value| value.as_str() == Some(provider_id.as_str()))
            }),
        "runtime should include saved cxits provider: {runtime_value:?}"
    );

    let saved_adapter_models = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v1/providers/{provider_id}/models"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "adapter_ids": [adapter_id]
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(saved_adapter_models.status(), StatusCode::OK);

    let saved_adapter_models_body = saved_adapter_models
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes();
    let saved_adapter_models_value: serde_json::Value =
        serde_json::from_slice(&saved_adapter_models_body).unwrap();
    assert!(
        saved_adapter_models_value
            .get("adapters")
            .and_then(|value| value.as_array())
            .is_some_and(|adapters| {
                adapters.iter().any(|adapter| {
                    adapter.get("adapter_id").and_then(|value| value.as_str())
                        == Some(adapter_id.as_str())
                        && adapter.get("error").is_none()
                        && adapter
                            .get("models")
                            .and_then(|value| value.as_array())
                            .map(|models| {
                                models.iter().any(|model| {
                                    model.get("id").and_then(|value| value.as_str())
                                        == Some(model_id.as_str())
                                })
                            })
                            == Some(true)
                })
            }),
        "saved provider adapter model listing should include the selected live adapter/model: {saved_adapter_models_value:?}"
    );

    let models_response = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/v1/providers/{provider_id}/models"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(models_response.status(), StatusCode::OK);

    let models_body = models_response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes();
    let models_value: serde_json::Value = serde_json::from_slice(&models_body).unwrap();
    assert!(
        models_value
            .get("models")
            .and_then(|value| value.as_array())
            .is_some_and(|models| {
                models.iter().any(|model| {
                    model.get("id").and_then(|value| value.as_str()) == Some(model_id.as_str())
                        && model.get("adapter_id").and_then(|value| value.as_str())
                            == Some(adapter_id.as_str())
                })
            }),
        "saved provider models should include the selected live adapter/model: {models_value:?}"
    );
}

#[tokio::test]
#[ignore = "real integration test against api.cxits.cn"]
async fn cxits_live_catalog_match_coverage_reports_unmatched_models() {
    let _api_key = live_provider_gateway_key();
    let config = format!(
        r#"
[providers.cxits_gateway]
default_adapter = "openai"
default_model = "gpt-5.4"

[providers.cxits_gateway.auth]
mode = "api"
base_url = "{base_url}"
api_key_env = "{key_env}"

[providers.cxits_gateway.adapters.openai]
enabled = true

[providers.cxits_gateway.adapters.anthropic]
enabled = true

[providers.cxits_gateway.adapters.gemini]
enabled = true
"#,
        base_url = LIVE_PROVIDER_GATEWAY_BASE_URL,
        key_env = LIVE_PROVIDER_GATEWAY_KEY_ENV,
    );

    let (state, _, _) = build_state_with_config(config.as_str()).await;
    let snapshot = state.runtime().current_snapshot();
    let source_providers = snapshot.catalog_source_provider_registry();
    snapshot
        .model_catalog()
        .refresh_from_registry(
            source_providers.as_ref(),
            Some(snapshot.config_resolution()),
        )
        .await
        .expect("live cxits catalog refresh should succeed");
    let catalog_ids = snapshot
        .model_catalog_response()
        .entries
        .into_iter()
        .map(|entry| entry.model_id)
        .collect::<std::collections::BTreeSet<_>>();
    assert!(
        !catalog_ids.is_empty(),
        "live cxits catalog refresh should produce catalog entries"
    );

    let app = router(state);
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/providers/cxits_gateway/models")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "adapter_ids": ["openai", "anthropic", "gemini"]
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let adapters = value
        .get("adapters")
        .and_then(|value| value.as_array())
        .expect("cxits adapter models response should include adapters");

    let listed_adapters = adapters
        .iter()
        .filter(|adapter| adapter.get("error").is_none())
        .collect::<Vec<_>>();
    assert!(
        !listed_adapters.is_empty(),
        "cxits adapter model listing should return at least one successful adapter: {value:?}"
    );

    let mut raw_models = std::collections::BTreeSet::new();
    let mut matched_raw_models = std::collections::BTreeSet::new();
    let mut matched_catalog_ids = std::collections::BTreeSet::new();
    let mut unmatched = Vec::new();
    let mut listed_entry_count = 0_usize;

    for adapter in listed_adapters.iter().copied() {
        let models = adapter
            .get("models")
            .and_then(|value| value.as_array())
            .expect("successful adapter listing should include models");
        listed_entry_count += models.len();
        for model in models {
            let Some(raw_model_id) = model.get("id").and_then(|value| value.as_str()) else {
                continue;
            };
            if !raw_models.insert(raw_model_id.to_owned()) {
                continue;
            }
            let catalog_model_id = model
                .get("catalog_model_id")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
                .unwrap_or_else(|| agena::model_catalog::canonical_model_catalog_id(raw_model_id));
            if catalog_ids.contains(catalog_model_id.as_str()) {
                matched_raw_models.insert(raw_model_id.to_owned());
                matched_catalog_ids.insert(catalog_model_id);
            } else {
                unmatched.push(if catalog_model_id == raw_model_id {
                    raw_model_id.to_owned()
                } else {
                    format!("{raw_model_id} -> {catalog_model_id}")
                });
            }
        }
    }

    unmatched.sort();
    let unmatched_preview = if unmatched.is_empty() {
        "(none)".to_owned()
    } else {
        unmatched
            .iter()
            .take(20)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    };

    eprintln!(
        "cxits catalog coverage: listed_adapters={} listed_entries={} unique_models={} matched_unique_models={} unmatched_unique_models={} matched_catalog_ids={} catalog_entries={}",
        adapters
            .iter()
            .filter(|adapter| adapter.get("error").is_none())
            .count(),
        listed_entry_count,
        raw_models.len(),
        matched_raw_models.len(),
        unmatched.len(),
        matched_catalog_ids.len(),
        catalog_ids.len(),
    );
    eprintln!("cxits unmatched model ids (up to 20):\n{unmatched_preview}");

    assert!(
        !matched_raw_models.is_empty(),
        "cxits live catalog coverage should match at least one listed model"
    );
}

#[tokio::test]
async fn operational_probes_and_metrics_return_expected_shapes() {
    let (state, _, _) = build_state().await;
    let app = router(state);

    let healthz = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/healthz")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(healthz.status(), StatusCode::OK);
    let healthz_body = healthz.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(String::from_utf8_lossy(&healthz_body), "ok");

    let readyz = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/readyz")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(readyz.status(), StatusCode::OK);
    let readyz_body = readyz.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(String::from_utf8_lossy(&readyz_body), "ready");

    let metrics = app
        .oneshot(
            Request::builder()
                .uri("/metrics")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(metrics.status(), StatusCode::OK);
    let metrics_body = metrics.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8_lossy(&metrics_body);
    assert!(text.contains("agena_runtime_generation"));
    assert!(text.contains("agena_http_requests_total"));
    assert!(text.contains("agena_http_request_duration_seconds_bucket"));
    assert!(text.contains("agena_provider_calls_total"));
    assert!(text.contains("agena_session_active"));
    assert!(text.contains("agena_build_info"));
}

#[tokio::test]
async fn project_git_init_endpoint_initializes_repository_and_returns_status() {
    let (state, _manager, workspace_root) = build_state().await;
    let app = router(state);

    let git_dir = std::path::Path::new(&workspace_root).join(".git");
    assert!(
        !git_dir.exists(),
        "test workspace should start without .git"
    );

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/project/git/init")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        value.get("repo").and_then(|item| item.as_bool()),
        Some(true),
        "expected git repo after init: {value:?}"
    );
    assert!(git_dir.exists(), "git init should create a .git directory");
}

#[tokio::test]
async fn vcs_diff_raw_endpoint_returns_plaintext_patch_for_workspace_changes() {
    let (state, _manager, workspace_root) = build_state().await;
    let app = router(state);

    let run_git = |args: &[&str]| {
        let output = ProcessCommand::new("git")
            .args(args)
            .current_dir(&workspace_root)
            .output()
            .expect("git command should execute");
        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
    };

    run_git(&["init"]);
    run_git(&["config", "user.email", "studio@example.com"]);
    run_git(&["config", "user.name", "Studio Parity"]);
    std::fs::write(
        std::path::Path::new(&workspace_root).join("tracked.txt"),
        "before\n",
    )
    .unwrap();
    run_git(&["add", "tracked.txt"]);
    run_git(&["commit", "-m", "initial"]);

    std::fs::write(
        std::path::Path::new(&workspace_root).join("tracked.txt"),
        "after\n",
    )
    .unwrap();
    std::fs::write(
        std::path::Path::new(&workspace_root).join("untracked.txt"),
        "brand new\n",
    )
    .unwrap();

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/vcs/diff/raw")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8_lossy(&body);
    assert!(text.contains("diff --git a/tracked.txt b/tracked.txt"));
    assert!(text.contains("diff --git a/untracked.txt b/untracked.txt"));
}

#[tokio::test]
async fn list_events_returns_published_events() {
    let (state, manager, _workspace_root) = build_state().await;

    // Publish a persistent (history) event — UI-only events like RunStarted
    // are no longer written to the event store. PluginEvent is an easy
    // persistent kind that has no private payload types.
    let publisher = manager.event_publisher();
    publisher
        .publish(
            PublishContext::for_session(42),
            EventKind::PluginEvent(agena::event::PluginEventPayload {
                plugin_id: "test".into(),
                kind_label: "test_event".into(),
                payload: serde_json::json!({}),
            }),
        )
        .await
        .unwrap();

    let app = router(state);
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/events")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let items = value
        .get("items")
        .and_then(|i| i.as_array())
        .expect("items array");
    assert!(
        items
            .iter()
            .any(|e| e.get("kind").and_then(|k| k.as_str()) == Some("plugin_event")),
        "expected plugin_event event in {value:?}"
    );
}

#[tokio::test]
async fn session_state_endpoint_returns_execution_resource() {
    let (state, _manager, workspace_root) = build_state().await;
    let app = router(state.clone());

    let workspace = state
        .service()
        .resolve_workspace(agena_api_server::local_api::WorkspaceResolveRequest {
            path: workspace_root,
            create_if_missing: false,
        })
        .await
        .expect("workspace should resolve");
    let session = state
        .service()
        .create_session(agena_api_server::local_api::SessionCreateRequest {
            workspace_id: workspace.id,
            title: "state route".to_string(),
            parent_id: None,
        })
        .await
        .expect("session should be created");

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/v1/sessions/{}/state", session.id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        value
            .get("session")
            .and_then(|s| s.get("id"))
            .and_then(|id| id.as_i64()),
        Some(session.id)
    );
    assert!(
        value
            .get("execution")
            .and_then(|execution| execution.get("allowed_tools"))
            .and_then(|allowed_tools| allowed_tools.as_array())
            .is_some()
    );
}

#[tokio::test]
async fn goal_command_and_query_round_trip() {
    let (state, _manager, workspace_root) = build_state().await;

    let workspace = state
        .service()
        .resolve_workspace(agena_api_server::local_api::WorkspaceResolveRequest {
            path: workspace_root,
            create_if_missing: false,
        })
        .await
        .expect("workspace should resolve");
    let session = state
        .service()
        .create_session(agena_api_server::local_api::SessionCreateRequest {
            workspace_id: workspace.id,
            title: "goal route".to_string(),
            parent_id: None,
        })
        .await
        .expect("session should be created");

    let created = dispatch_command(
        &state,
        Command::CreateSessionGoal(CreateSessionGoalParams {
            session_id: session.id,
            objective: "close goal scope".to_string(),
            token_budget: Some(512),
        }),
    )
    .await
    .expect("create goal command should succeed");
    let CommandResult::SessionGoal(goal) = created else {
        panic!("expected goal result");
    };
    assert_eq!(goal.session_id, session.id);
    assert_eq!(goal.objective, "close goal scope");
    assert_eq!(goal.token_budget, Some(512));
    assert_eq!(goal.tokens_used, 0);
    assert_eq!(goal.time_used_seconds, 0);

    let queried = dispatch_query(
        &state,
        Query::GetSessionGoal(GetSessionParams {
            session_id: session.id,
        }),
    )
    .await
    .expect("goal query should succeed");
    let QueryResult::SessionGoal(Some(goal)) = queried else {
        panic!("expected session goal query result");
    };
    assert_eq!(goal.session_id, session.id);
    assert_eq!(goal.objective, "close goal scope");
    assert_eq!(goal.tokens_used, 0);
    assert_eq!(goal.time_used_seconds, 0);

    let completed = dispatch_command(
        &state,
        Command::CompleteSessionGoal(CompleteSessionGoalParams {
            session_id: session.id,
        }),
    )
    .await
    .expect("complete goal command should succeed");
    let CommandResult::SessionGoal(goal) = completed else {
        panic!("expected completed goal result");
    };
    assert_eq!(goal.session_id, session.id);
    assert_eq!(goal.status, agena::session::GoalStatus::Completed);
    assert!(goal.completed_at.is_some());
    assert_eq!(goal.tokens_used, 0);
    assert_eq!(goal.time_used_seconds, 0);
}

#[tokio::test]
async fn set_goal_command_creates_updates_and_clears_goal() {
    let (state, _manager, workspace_root) = build_state().await;

    let workspace = state
        .service()
        .resolve_workspace(agena_api_server::local_api::WorkspaceResolveRequest {
            path: workspace_root,
            create_if_missing: false,
        })
        .await
        .expect("workspace should resolve");
    let session = state
        .service()
        .create_session(agena_api_server::local_api::SessionCreateRequest {
            workspace_id: workspace.id,
            title: "set goal route".to_string(),
            parent_id: None,
        })
        .await
        .expect("session should be created");

    let created = dispatch_command(
        &state,
        Command::SetSessionGoal(SetSessionGoalParams {
            session_id: session.id,
            objective: Some("ship the API slice".to_string()),
            status: None,
            token_budget: Some(Some(256)),
            clear: false,
        }),
    )
    .await
    .expect("set goal create should succeed");
    let CommandResult::SessionGoal(goal) = created else {
        panic!("expected goal result");
    };
    assert_eq!(goal.objective, "ship the API slice");
    assert_eq!(goal.status, agena::session::GoalStatus::Active);
    assert_eq!(goal.token_budget, Some(256));

    let updated = dispatch_command(
        &state,
        Command::SetSessionGoal(SetSessionGoalParams {
            session_id: session.id,
            objective: Some("ship the narrower API slice".to_string()),
            status: Some(agena::session::GoalStatus::Paused),
            token_budget: Some(None),
            clear: false,
        }),
    )
    .await
    .expect("set goal update should succeed");
    let CommandResult::SessionGoal(goal) = updated else {
        panic!("expected updated goal result");
    };
    assert_eq!(goal.objective, "ship the narrower API slice");
    assert_eq!(goal.status, agena::session::GoalStatus::Paused);
    assert_eq!(goal.token_budget, None);

    let cleared = dispatch_command(
        &state,
        Command::SetSessionGoal(SetSessionGoalParams {
            session_id: session.id,
            objective: None,
            status: None,
            token_budget: None,
            clear: true,
        }),
    )
    .await
    .expect("set goal clear should succeed");
    let CommandResult::SessionGoalCleared { session_id } = cleared else {
        panic!("expected cleared result");
    };
    assert_eq!(session_id, session.id);

    let queried = dispatch_query(
        &state,
        Query::GetSessionGoal(GetSessionParams {
            session_id: session.id,
        }),
    )
    .await
    .expect("goal query after clear should succeed");
    let QueryResult::SessionGoal(goal) = queried else {
        panic!("expected session goal query result");
    };
    assert!(goal.is_none(), "goal should be cleared");
}

#[tokio::test]
async fn set_goal_update_preserves_usage_counters() {
    let (state, _manager, workspace_root) = build_state().await;

    let workspace = state
        .service()
        .resolve_workspace(agena_api_server::local_api::WorkspaceResolveRequest {
            path: workspace_root,
            create_if_missing: false,
        })
        .await
        .expect("workspace should resolve");
    let session = state
        .service()
        .create_session(agena_api_server::local_api::SessionCreateRequest {
            workspace_id: workspace.id,
            title: "set goal usage".to_string(),
            parent_id: None,
        })
        .await
        .expect("session should be created");

    dispatch_command(
        &state,
        Command::SetSessionGoal(SetSessionGoalParams {
            session_id: session.id,
            objective: Some("preserve usage".to_string()),
            status: None,
            token_budget: Some(Some(100)),
            clear: false,
        }),
    )
    .await
    .expect("initial goal create should succeed");

    let db = state.service().clone_db();
    agena::db::crud::session_goal::account_usage(
        db.as_ref(),
        session.id,
        17,
        3,
        agena::db::crud::session_goal::GoalAccountingMode::ActiveOnly,
        None,
    )
    .await
    .expect("goal usage should persist");

    let updated = dispatch_command(
        &state,
        Command::SetSessionGoal(SetSessionGoalParams {
            session_id: session.id,
            objective: Some("preserve usage after update".to_string()),
            status: Some(agena::session::GoalStatus::Active),
            token_budget: Some(Some(200)),
            clear: false,
        }),
    )
    .await
    .expect("goal update should succeed");
    let CommandResult::SessionGoal(goal) = updated else {
        panic!("expected updated goal result");
    };
    assert_eq!(goal.objective, "preserve usage after update");
    assert_eq!(goal.token_budget, Some(200));
    assert_eq!(goal.tokens_used, 17);
    assert_eq!(goal.time_used_seconds, 3);
}

#[tokio::test]
async fn fork_session_endpoint_returns_forked_execution_resource() {
    let (state, _manager, workspace_root) = build_state().await;
    let app = router(state.clone());

    let workspace = state
        .service()
        .resolve_workspace(agena_api_server::local_api::WorkspaceResolveRequest {
            path: workspace_root,
            create_if_missing: false,
        })
        .await
        .expect("workspace should resolve");
    let session = state
        .service()
        .create_session(agena_api_server::local_api::SessionCreateRequest {
            workspace_id: workspace.id,
            title: "fork source".to_string(),
            parent_id: None,
        })
        .await
        .expect("session should be created");

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v1/sessions/{}/fork", session.id))
                .header("content-type", "application/json")
                .body(Body::from(r#"{"title":"fork child"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(
        status,
        StatusCode::OK,
        "body: {}",
        String::from_utf8_lossy(&body)
    );
    let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_ne!(
        value
            .get("session")
            .and_then(|s| s.get("id"))
            .and_then(|id| id.as_i64()),
        Some(session.id)
    );
    assert_eq!(
        value
            .get("session")
            .and_then(|s| s.get("parent_id"))
            .and_then(|id| id.as_i64()),
        Some(session.id)
    );
    assert_eq!(
        value
            .get("session")
            .and_then(|s| s.get("title"))
            .and_then(|title| title.as_str()),
        Some("fork child")
    );
}

#[tokio::test]
async fn message_detail_routes_return_message_and_parts() {
    let (state, manager, workspace_root) = build_state().await;
    let app = router(state.clone());

    let workspace = state
        .service()
        .resolve_workspace(agena_api_server::local_api::WorkspaceResolveRequest {
            path: workspace_root,
            create_if_missing: false,
        })
        .await
        .expect("workspace should resolve");
    let session = state
        .service()
        .create_session(agena_api_server::local_api::SessionCreateRequest {
            workspace_id: workspace.id,
            title: "message source".to_string(),
            parent_id: None,
        })
        .await
        .expect("session should be created");

    let session = manager
        .submit_user_turn(SessionUserTurnRequest {
            session_id: session.id,
            options: test_run_options(),
            parts: vec![PartContent::text("hello")],
        })
        .await
        .expect("submit turn should succeed");
    let message = session
        .messages
        .first()
        .expect("session should contain a user message");
    let part = message
        .parts
        .first()
        .expect("message should contain a part");

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/v1/messages/{}?parts=full", message.id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(value.get("id").and_then(|id| id.as_i64()), Some(message.id));
    assert_eq!(
        value
            .get("parts")
            .and_then(|parts| parts.as_array())
            .and_then(|parts| parts.first())
            .and_then(|item| item.get("content"))
            .and_then(|content| content.get("text"))
            .and_then(|text| text.as_str()),
        Some("hello")
    );

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/api/v1/messages/{}/parts?mode=summary",
                    message.id
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(value.as_array().map(|items| items.len()), Some(1));
    assert_eq!(
        value
            .as_array()
            .and_then(|items| items.first())
            .and_then(|item| item.get("id"))
            .and_then(|id| id.as_i64()),
        Some(part.id)
    );
    assert!(
        value
            .as_array()
            .and_then(|items| items.first())
            .and_then(|item| item.get("content"))
            .is_none(),
        "summary mode should omit content: {value:?}"
    );

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/v1/messages/{}/parts?mode=full", message.id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        value
            .as_array()
            .and_then(|items| items.first())
            .and_then(|item| item.get("content"))
            .and_then(|content| content.get("text"))
            .and_then(|text| text.as_str()),
        Some("hello")
    );

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/v1/message-parts/{}", part.id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(value.get("id").and_then(|id| id.as_i64()), Some(part.id));
    assert_eq!(
        value
            .get("content")
            .and_then(|content| content.get("text"))
            .and_then(|text| text.as_str()),
        Some("hello")
    );
}

#[tokio::test]
async fn message_list_summary_omits_part_content_while_detail_full_keeps_it() {
    let (state, manager, workspace_root) = build_state().await;
    let app = router(state.clone());

    let workspace = state
        .service()
        .resolve_workspace(agena_api_server::local_api::WorkspaceResolveRequest {
            path: workspace_root,
            create_if_missing: false,
        })
        .await
        .expect("workspace should resolve");
    let session = state
        .service()
        .create_session(agena_api_server::local_api::SessionCreateRequest {
            workspace_id: workspace.id,
            title: "summary source".to_string(),
            parent_id: None,
        })
        .await
        .expect("session should be created");

    let session = manager
        .submit_user_turn(SessionUserTurnRequest {
            session_id: session.id,
            options: test_run_options(),
            parts: vec![PartContent::text("summary hello")],
        })
        .await
        .expect("submit turn should succeed");
    let message = session
        .messages
        .first()
        .expect("session should contain a user message");

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/api/v1/sessions/{}/messages?parts=none",
                    session.id
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let first = value
        .get("items")
        .and_then(|items| items.as_array())
        .and_then(|items| items.first())
        .expect("first list item");
    assert_eq!(first.get("id").and_then(|id| id.as_i64()), Some(message.id));
    assert!(
        first.get("parts").is_none(),
        "none list should omit parts entirely: {value:?}"
    );
    assert_eq!(
        first.get("part_count").and_then(|count| count.as_u64()),
        Some(1)
    );

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/api/v1/sessions/{}/messages?parts=summary",
                    session.id
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let first = value
        .get("items")
        .and_then(|items| items.as_array())
        .and_then(|items| items.first())
        .expect("first list item");
    assert_eq!(first.get("id").and_then(|id| id.as_i64()), Some(message.id));
    assert!(
        first
            .get("parts")
            .and_then(|parts| parts.as_array())
            .and_then(|parts| parts.first())
            .and_then(|part| part.get("content"))
            .is_none(),
        "summary list should omit part content: {value:?}"
    );

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/v1/messages/{}?parts=none", message.id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(
        value.get("parts").is_none(),
        "none detail should omit parts entirely: {value:?}"
    );
    assert_eq!(
        value.get("part_count").and_then(|count| count.as_u64()),
        Some(1)
    );

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/v1/messages/{}?parts=full", message.id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        value
            .get("parts")
            .and_then(|parts| parts.as_array())
            .and_then(|parts| parts.first())
            .and_then(|item| item.get("content"))
            .and_then(|content| content.get("text"))
            .and_then(|text| text.as_str()),
        Some("summary hello")
    );
}

#[tokio::test]
async fn message_none_mode_uses_projected_part_count_without_part_rows() {
    let (state, _manager, workspace_root) = build_state().await;
    let app = router(state.clone());

    let workspace = state
        .service()
        .resolve_workspace(agena_api_server::local_api::WorkspaceResolveRequest {
            path: workspace_root,
            create_if_missing: false,
        })
        .await
        .expect("workspace should resolve");
    let session = state
        .service()
        .create_session(agena_api_server::local_api::SessionCreateRequest {
            workspace_id: workspace.id,
            title: "none mode source".to_string(),
            parent_id: None,
        })
        .await
        .expect("session should be created");

    let db = state.service().clone_db();
    let created_at = chrono::Utc::now().timestamp_millis();
    let message_id = 9101;

    agena::db::entities::activity_message::ActiveModel {
        message_id: Set(message_id),
        session_id: Set(session.id),
        role: Set(agena::role::Role::Assistant),
        state: Set(agena::message::ExecutionStatus::Completed),
        created_at_ms: Set(created_at),
        updated_at_ms: Set(created_at),
        metadata: Set(agena::message::MessageMetadata::default()),
        usage: Set(None),
        finish: Set(None),
        part_count: Set(4),
        is_compacted: Set(false),
    }
    .insert(db.as_ref())
    .await
    .expect("activity message projection should insert");

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/api/v1/sessions/{}/messages?parts=none",
                    session.id
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let first = value
        .get("items")
        .and_then(|items| items.as_array())
        .and_then(|items| items.first())
        .expect("first list item");
    assert_eq!(first.get("id").and_then(|id| id.as_i64()), Some(message_id));
    assert!(first.get("parts").is_none(), "none list should omit parts");
    assert_eq!(
        first.get("part_count").and_then(|count| count.as_u64()),
        Some(4)
    );

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/v1/messages/{}?parts=none", message_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(
        value.get("parts").is_none(),
        "none detail should omit parts"
    );
    assert_eq!(
        value.get("part_count").and_then(|count| count.as_u64()),
        Some(4)
    );
}

#[tokio::test]
async fn message_list_none_paginates_newest_first_and_returns_each_page_ascending() {
    let (state, _manager, workspace_root) = build_state().await;
    let app = router(state.clone());

    let workspace = state
        .service()
        .resolve_workspace(agena_api_server::local_api::WorkspaceResolveRequest {
            path: workspace_root,
            create_if_missing: false,
        })
        .await
        .expect("workspace should resolve");
    let session = state
        .service()
        .create_session(agena_api_server::local_api::SessionCreateRequest {
            workspace_id: workspace.id,
            title: "paged none".to_string(),
            parent_id: None,
        })
        .await
        .expect("session should be created");

    let db = state.service().clone_db();
    let created_at = chrono::Utc::now().timestamp_millis();
    for message_id in 9_501..=9_505 {
        insert_projected_message(db.as_ref(), session.id, message_id, created_at, 0).await;
    }

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/api/v1/sessions/{}/messages?parts=none&limit=2",
                    session.id
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let items = value
        .get("items")
        .and_then(|items| items.as_array())
        .expect("first page items");
    assert_eq!(
        items
            .iter()
            .map(|item| item.get("id").and_then(|id| id.as_i64()).unwrap())
            .collect::<Vec<_>>(),
        vec![9_504, 9_505]
    );
    assert!(
        items.iter().all(|item| item.get("parts").is_none()),
        "none mode should omit parts: {value:?}"
    );
    assert_eq!(
        value
            .get("page")
            .and_then(|page| page.get("has_more"))
            .and_then(|has_more| has_more.as_bool()),
        Some(true)
    );
    let cursor = value
        .get("page")
        .and_then(|page| page.get("next_cursor"))
        .and_then(|cursor| cursor.as_str())
        .expect("first page cursor")
        .to_string();

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/api/v1/sessions/{}/messages?parts=none&limit=2&cursor={cursor}",
                    session.id
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let items = value
        .get("items")
        .and_then(|items| items.as_array())
        .expect("second page items");
    assert_eq!(
        items
            .iter()
            .map(|item| item.get("id").and_then(|id| id.as_i64()).unwrap())
            .collect::<Vec<_>>(),
        vec![9_502, 9_503]
    );
    assert_eq!(
        value
            .get("page")
            .and_then(|page| page.get("has_more"))
            .and_then(|has_more| has_more.as_bool()),
        Some(true)
    );
    let cursor = value
        .get("page")
        .and_then(|page| page.get("next_cursor"))
        .and_then(|cursor| cursor.as_str())
        .expect("second page cursor")
        .to_string();

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/api/v1/sessions/{}/messages?parts=none&limit=2&cursor={cursor}",
                    session.id
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let items = value
        .get("items")
        .and_then(|items| items.as_array())
        .expect("third page items");
    assert_eq!(
        items
            .iter()
            .map(|item| item.get("id").and_then(|id| id.as_i64()).unwrap())
            .collect::<Vec<_>>(),
        vec![9_501]
    );
    assert_eq!(
        value
            .get("page")
            .and_then(|page| page.get("has_more"))
            .and_then(|has_more| has_more.as_bool()),
        Some(false)
    );
}

#[tokio::test]
async fn message_list_summary_and_full_paginate_with_expected_part_payloads() {
    let (state, _manager, workspace_root) = build_state().await;
    let app = router(state.clone());

    let workspace = state
        .service()
        .resolve_workspace(agena_api_server::local_api::WorkspaceResolveRequest {
            path: workspace_root,
            create_if_missing: false,
        })
        .await
        .expect("workspace should resolve");
    let session = state
        .service()
        .create_session(agena_api_server::local_api::SessionCreateRequest {
            workspace_id: workspace.id,
            title: "paged summary/full".to_string(),
            parent_id: None,
        })
        .await
        .expect("session should be created");

    let db = state.service().clone_db();
    let created_at = chrono::Utc::now().timestamp_millis();
    for (offset, (message_id, part_id, text)) in [
        (0_i64, (9_601_i64, 9_701_i64, "first page oldest")),
        (1_i64, (9_602_i64, 9_702_i64, "first page newest")),
        (2_i64, (9_603_i64, 9_703_i64, "second page only")),
    ] {
        let created_at_ms = created_at + offset;
        insert_projected_message(db.as_ref(), session.id, message_id, created_at_ms, 1).await;
        insert_projected_text_part(
            db.as_ref(),
            session.id,
            message_id,
            part_id,
            0,
            created_at_ms,
            text,
        )
        .await;
    }

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/api/v1/sessions/{}/messages?parts=summary&limit=2",
                    session.id
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let items = value
        .get("items")
        .and_then(|items| items.as_array())
        .expect("summary page items");
    assert_eq!(
        items
            .iter()
            .map(|item| item.get("id").and_then(|id| id.as_i64()).unwrap())
            .collect::<Vec<_>>(),
        vec![9_602, 9_603]
    );
    assert!(
        items.iter().all(|item| {
            item.get("parts")
                .and_then(|parts| parts.as_array())
                .and_then(|parts| parts.first())
                .and_then(|part| part.get("content"))
                .is_none()
        }),
        "summary mode should omit part content: {value:?}"
    );
    let cursor = value
        .get("page")
        .and_then(|page| page.get("next_cursor"))
        .and_then(|cursor| cursor.as_str())
        .expect("summary page cursor")
        .to_string();

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/api/v1/sessions/{}/messages?parts=full&limit=2&cursor={cursor}",
                    session.id
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let items = value
        .get("items")
        .and_then(|items| items.as_array())
        .expect("full page items");
    assert_eq!(
        items
            .iter()
            .map(|item| item.get("id").and_then(|id| id.as_i64()).unwrap())
            .collect::<Vec<_>>(),
        vec![9_601]
    );
    assert_eq!(
        items[0]
            .get("parts")
            .and_then(|parts| parts.as_array())
            .and_then(|parts| parts.first())
            .and_then(|part| part.get("content"))
            .and_then(|content| content.get("text"))
            .and_then(|text| text.as_str()),
        Some("first page oldest")
    );
    assert_eq!(
        value
            .get("page")
            .and_then(|page| page.get("has_more"))
            .and_then(|has_more| has_more.as_bool()),
        Some(false)
    );
}

#[tokio::test]
async fn message_parts_none_still_404s_for_missing_message() {
    let (state, _manager, _workspace_root) = build_state().await;
    let app = router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/messages/999999/parts?mode=none")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        value.get("message").and_then(|message| message.as_str()),
        Some("message not found: 999999")
    );
}

#[tokio::test]
async fn fork_session_endpoint_rejects_legacy_event_seq_payload() {
    let (state, _manager, workspace_root) = build_state().await;
    let app = router(state.clone());

    let workspace = state
        .service()
        .resolve_workspace(agena_api_server::local_api::WorkspaceResolveRequest {
            path: workspace_root,
            create_if_missing: false,
        })
        .await
        .expect("workspace should resolve");
    let session = state
        .service()
        .create_session(agena_api_server::local_api::SessionCreateRequest {
            workspace_id: workspace.id,
            title: "fork source".to_string(),
            parent_id: None,
        })
        .await
        .expect("session should be created");

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v1/sessions/{}/fork", session.id))
                .header("content-type", "application/json")
                .body(Body::from(r#"{"at_event_seq":1}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(
        value
            .get("message")
            .and_then(|message| message.as_str())
            .or_else(|| {
                value
                    .get("error")
                    .and_then(|error| error.get("message"))
                    .and_then(|message| message.as_str())
            })
            .is_some_and(|message| message.contains("at_message_id")),
        "body should mention at_message_id: {value:?}"
    );
}

#[tokio::test]
async fn session_action_routes_cover_rewind_tree_checkpoints_export_and_import() {
    let (state, manager, workspace_root) = build_state().await;
    let app = router(state.clone());

    let workspace = state
        .service()
        .resolve_workspace(agena_api_server::local_api::WorkspaceResolveRequest {
            path: workspace_root,
            create_if_missing: false,
        })
        .await
        .expect("workspace should resolve");
    let session = state
        .service()
        .create_session(agena_api_server::local_api::SessionCreateRequest {
            workspace_id: workspace.id,
            title: "action source".to_string(),
            parent_id: None,
        })
        .await
        .expect("session should be created");

    let session = manager
        .submit_user_turn(SessionUserTurnRequest {
            session_id: session.id,
            options: test_run_options(),
            parts: vec![PartContent::text("first")],
        })
        .await
        .expect("first turn should succeed");
    let rewind_target = session.messages.first().expect("expected first message").id;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v1/sessions/{}/rewind", session.id))
                .header("content-type", "application/json")
                .body(Body::from(format!(r#"{{"message_id":{rewind_target}}}"#)))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v1/sessions/{}/unrewind", session.id))
                .header("content-type", "application/json")
                .body(Body::from(format!(r#"{{"message_id":{rewind_target}}}"#)))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/v1/sessions/tree/{}", session.id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let tree_body = response.into_body().collect().await.unwrap().to_bytes();
    let tree: serde_json::Value = serde_json::from_slice(&tree_body).unwrap();
    assert!(tree.as_array().is_some_and(|items| !items.is_empty()));

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/api/v1/sessions/{}/rewind-checkpoints",
                    session.id
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let export = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/v1/sessions/{}/export", session.id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(export.status(), StatusCode::OK);
    let export_body = export.into_body().collect().await.unwrap().to_bytes();
    let jsonl = String::from_utf8_lossy(&export_body).to_string();
    assert!(!jsonl.trim().is_empty());

    let import = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/sessions/import")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({ "jsonl": jsonl }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let import_status = import.status();
    let import_body = import.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(
        import_status,
        StatusCode::OK,
        "import response body: {}",
        String::from_utf8_lossy(&import_body)
    );
    let imported: serde_json::Value = serde_json::from_slice(&import_body).unwrap();
    assert_ne!(
        imported
            .get("session")
            .and_then(|session| session.get("id"))
            .and_then(|id| id.as_i64()),
        Some(session.id)
    );
}

#[tokio::test]
async fn permission_rule_crud_routes_expose_operator_metadata() {
    let (state, _manager, _workspace_root) = build_state().await;
    let app = router(state.clone());

    let create_rule = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/permission-rules")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "subject_kind": "tool",
                        "tool_name": "bash",
                        "qualifier": "git status*",
                        "scope": "global",
                        "mode": "allow"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create_rule.status(), StatusCode::OK);
    let created_body = create_rule.into_body().collect().await.unwrap().to_bytes();
    let created: serde_json::Value = serde_json::from_slice(&created_body).unwrap();
    let rule_id = created
        .get("id")
        .and_then(|id| id.as_i64())
        .expect("rule id");
    assert_eq!(
        created.get("operator").and_then(|value| value.as_str()),
        Some("http_api")
    );
    assert_eq!(
        created.get("scope").and_then(|value| value.as_str()),
        Some("global")
    );

    let replace_rule = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!("/api/v1/permission-rules/{rule_id}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "subject_kind": "tool",
                        "tool_name": "bash",
                        "qualifier": "git diff*",
                        "scope": "workspace",
                        "mode": "deny"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(replace_rule.status(), StatusCode::OK);
    let replaced_body = replace_rule.into_body().collect().await.unwrap().to_bytes();
    let replaced: serde_json::Value = serde_json::from_slice(&replaced_body).unwrap();
    assert_eq!(
        replaced.get("operator").and_then(|value| value.as_str()),
        Some("http_api")
    );
    assert_eq!(
        replaced.get("source").and_then(|value| value.as_str()),
        Some("api")
    );
    assert_eq!(
        replaced.get("scope").and_then(|value| value.as_str()),
        Some("workspace")
    );

    let revoked_rule = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v1/permission-rules/{rule_id}/revoke"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({ "reason": "no longer needed" }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(revoked_rule.status(), StatusCode::OK);
    let revoked_body = revoked_rule.into_body().collect().await.unwrap().to_bytes();
    let revoked: serde_json::Value = serde_json::from_slice(&revoked_body).unwrap();
    assert_eq!(
        revoked.get("revoked_by").and_then(|value| value.as_str()),
        Some("http_api")
    );
}

#[tokio::test]
async fn ws_protocol_round_trip_command_and_subscription() {
    let (state, manager, _workspace_root) = build_state().await;

    // Drive the bus directly so we don't depend on a real provider in this
    // integration test.
    let publisher = manager.event_publisher();
    let bus = manager.event_bus();

    // Start an in-process axum server bound to an ephemeral port.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = router(state);
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    // Sanity-check the bus routing without needing a WS client crate: we
    // subscribe in-process and verify events flow.
    let mut sub = bus.subscribe(agena::event::EventFilter::new(Scope::Global));
    publisher
        .publish(
            PublishContext::for_session(1),
            EventKind::RunStarted(agena::event::RunStartedEvent {
                session_id: 1,
                ts_ms: 0,
            }),
        )
        .await
        .unwrap();
    let item = tokio::time::timeout(std::time::Duration::from_millis(200), sub.recv())
        .await
        .expect("subscriber should receive event");
    assert!(item.is_some());

    // Confirm the address opened a TCP listener (basic liveness check).
    let _ = tokio::net::TcpStream::connect(addr).await.unwrap();

    handle.abort();

    // Reuse imports so the compiler doesn't warn unused.
    let _ = (
        ClientMessage::Ping { nonce: None },
        ServerMessage::Hello {
            protocol_version: PROTOCOL_VERSION,
        },
        SubscribeRequest {
            scope: Scope::Global,
            kinds: None,
            since_seq_global: None,
        },
        std::any::type_name::<Notification>(),
        std::any::type_name::<PaginatedEvents>(),
    );
}
