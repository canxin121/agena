use super::*;
use crate::local_api::SessionCreateRequest;
use agena::config::LoadConfigRequest;
use agena::db::entities::{activity_message, activity_part};
use agena::message::{ExecutionStatus, MessageMetadata, PartContent, PartKind};
use agena::model::ModelRef;
use agena::runtime::AgenaRuntime;
use agena_api::resource::RunOptions;
use sea_orm::{ActiveModelTrait, ActiveValue::Set};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

fn unique_test_dir(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be after unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("agena-api-server-{label}-{nanos}"))
}

async fn create_session(state: &AppState, workspace_root: &Path, title: &str) -> i64 {
    let workspace = state
        .service()
        .resolve_workspace(crate::local_api::WorkspaceResolveRequest {
            path: workspace_root.display().to_string(),
            create_if_missing: true,
        })
        .await
        .expect("workspace should resolve");
    let session = state
        .service()
        .create_session(SessionCreateRequest {
            workspace_id: workspace.id,
            title: title.to_string(),
            parent_id: None,
        })
        .await
        .expect("session should be created");
    session.id
}

async fn test_state_with_config(config: &str, label: &str) -> (AppState, PathBuf) {
    let root = unique_test_dir(label);
    let workspace_root = root.join("workspace");
    fs::create_dir_all(&workspace_root).expect("create workspace root");
    let config_path = root.join("config.toml");
    fs::write(&config_path, config).expect("write config");

    let db = Arc::new(
        sea_orm::Database::connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite should connect"),
    );
    agena::db::init_schema(db.as_ref())
        .await
        .expect("schema init should succeed");

    let runtime = AgenaRuntime::builder()
        .with_load_request(LoadConfigRequest {
            config_path: Some(config_path),
            overrides: Vec::new(),
        })
        .with_workspace_root(workspace_root.clone())
        .with_database_connection(db.as_ref().clone())
        .build()
        .await
        .expect("runtime build should succeed");

    (AppState::new(runtime, db), workspace_root)
}

async fn insert_projected_message_with_text_part(
    state: &AppState,
    session_id: i64,
    message_id: i64,
    part_id: i64,
    created_at_ms: i64,
    summary: &str,
    text: &str,
) {
    let db = state.service().clone_db();

    activity_message::ActiveModel {
        message_id: Set(message_id),
        session_id: Set(session_id),
        role: Set(agena::role::Role::Assistant),
        state: Set(ExecutionStatus::Completed),
        created_at_ms: Set(created_at_ms),
        updated_at_ms: Set(created_at_ms),
        metadata: Set(MessageMetadata::default()),
        usage: Set(None),
        finish: Set(None),
        part_count: Set(1),
        is_compacted: Set(false),
    }
    .insert(db.as_ref())
    .await
    .expect("activity message projection should insert");

    activity_part::ActiveModel {
        part_id: Set(part_id),
        message_id: Set(message_id),
        session_id: Set(session_id),
        part_index: Set(0),
        status: Set(ExecutionStatus::Completed),
        kind: Set(PartKind::Text),
        name: Set(None),
        summary: Set(Some(summary.to_string())),
        has_detail: Set(true),
        operation_id: Set(None),
        created_at_ms: Set(created_at_ms),
        content: Set(Some(PartContent::text(text))),
    }
    .insert(db.as_ref())
    .await
    .expect("activity part projection should insert");
}

#[tokio::test]
async fn message_queries_respect_none_parts_mode() {
    let (state, workspace_root) = test_state_with_config(
        r#"
[providers.openai]
default_model = "openai/gpt-5.4"

[providers.openai.auth]
mode = "api"
base_url = "https://api.openai.com"
api_key = "dummy"

[providers.openai.adapters.openai]
enabled = true
"#,
        "message-parts-none",
    )
    .await;
    let session_id = create_session(&state, &workspace_root, "message none").await;
    let db = state.service().clone_db();
    let created_at = chrono::Utc::now().timestamp_millis();
    let message_id = 7001;
    let part_id = 7101;

    activity_message::ActiveModel {
        message_id: Set(message_id),
        session_id: Set(session_id),
        role: Set(agena::role::Role::Assistant),
        state: Set(ExecutionStatus::Completed),
        created_at_ms: Set(created_at),
        updated_at_ms: Set(created_at),
        metadata: Set(MessageMetadata::default()),
        usage: Set(None),
        finish: Set(None),
        part_count: Set(1),
        is_compacted: Set(false),
    }
    .insert(db.as_ref())
    .await
    .expect("activity message projection should insert");

    activity_part::ActiveModel {
        part_id: Set(part_id),
        message_id: Set(message_id),
        session_id: Set(session_id),
        part_index: Set(0),
        status: Set(ExecutionStatus::Completed),
        kind: Set(PartKind::Text),
        name: Set(None),
        summary: Set(Some("hello from dispatch".to_string())),
        has_detail: Set(true),
        operation_id: Set(None),
        created_at_ms: Set(created_at),
        content: Set(Some(PartContent::text("hello from dispatch"))),
    }
    .insert(db.as_ref())
    .await
    .expect("activity part projection should insert");

    let result = dispatch_query(
        &state,
        Query::ListMessages(ListMessagesParams {
            session_id,
            cursor: None,
            limit: None,
            parts: agena_api::resource::PartLoadMode::None,
        }),
    )
    .await
    .expect("list messages query should succeed");
    let QueryResult::Messages(page) = result else {
        panic!("expected message list result");
    };
    let message = page
        .items
        .first()
        .expect("message list should not be empty");
    assert_eq!(message.id, message_id);
    assert!(message.parts.is_none(), "none mode should omit parts");
    assert_eq!(message.part_count, 1);

    let result = dispatch_query(
        &state,
        Query::GetMessage(GetMessageParams {
            message_id,
            parts: agena_api::resource::PartLoadMode::None,
        }),
    )
    .await
    .expect("get message query should succeed");
    let QueryResult::Message(message) = result else {
        panic!("expected message result");
    };
    assert_eq!(message.id, message_id);
    assert!(message.parts.is_none(), "none mode should omit parts");
    assert_eq!(message.part_count, 1);
}

#[tokio::test]
async fn message_queries_none_use_projected_part_count_without_part_rows() {
    let (state, workspace_root) = test_state_with_config(
        r#"
[providers.openai]
default_model = "openai/gpt-5.4"

[providers.openai.auth]
mode = "api"
base_url = "https://api.openai.com"
api_key = "dummy"

[providers.openai.adapters.openai]
enabled = true
"#,
        "message-none-part-count",
    )
    .await;
    let session_id = create_session(&state, &workspace_root, "message none headers").await;
    let db = state.service().clone_db();
    let created_at = chrono::Utc::now().timestamp_millis();
    let message_id = 7002;

    activity_message::ActiveModel {
        message_id: Set(message_id),
        session_id: Set(session_id),
        role: Set(agena::role::Role::Assistant),
        state: Set(ExecutionStatus::Completed),
        created_at_ms: Set(created_at),
        updated_at_ms: Set(created_at),
        metadata: Set(MessageMetadata::default()),
        usage: Set(None),
        finish: Set(None),
        part_count: Set(3),
        is_compacted: Set(false),
    }
    .insert(db.as_ref())
    .await
    .expect("activity message projection should insert");

    let result = dispatch_query(
        &state,
        Query::ListMessages(ListMessagesParams {
            session_id,
            cursor: None,
            limit: None,
            parts: agena_api::resource::PartLoadMode::None,
        }),
    )
    .await
    .expect("list messages query should succeed");
    let QueryResult::Messages(page) = result else {
        panic!("expected message list result");
    };
    let message = page
        .items
        .first()
        .expect("message list should not be empty");
    assert_eq!(message.id, message_id);
    assert!(message.parts.is_none(), "none mode should omit parts");
    assert_eq!(message.part_count, 3);

    let result = dispatch_query(
        &state,
        Query::GetMessage(GetMessageParams {
            message_id,
            parts: agena_api::resource::PartLoadMode::None,
        }),
    )
    .await
    .expect("get message query should succeed");
    let QueryResult::Message(message) = result else {
        panic!("expected message result");
    };
    assert_eq!(message.id, message_id);
    assert!(message.parts.is_none(), "none mode should omit parts");
    assert_eq!(message.part_count, 3);
}

#[tokio::test]
async fn list_messages_query_uses_paginated_service_metadata() {
    let (state, workspace_root) = test_state_with_config(
        r#"
[providers.openai]
default_model = "openai/gpt-5.4"

[providers.openai.auth]
mode = "api"
base_url = "https://api.openai.com"
api_key = "dummy"

[providers.openai.adapters.openai]
enabled = true
"#,
        "message-pagination-dispatch",
    )
    .await;
    let session_id = create_session(&state, &workspace_root, "message pagination").await;
    let created_at = chrono::Utc::now().timestamp_millis();

    insert_projected_message_with_text_part(
        &state,
        session_id,
        7201,
        7301,
        created_at,
        "older summary",
        "older body",
    )
    .await;
    insert_projected_message_with_text_part(
        &state,
        session_id,
        7202,
        7302,
        created_at + 1,
        "newer summary",
        "newer body",
    )
    .await;

    let result = dispatch_query(
        &state,
        Query::ListMessages(ListMessagesParams {
            session_id,
            cursor: None,
            limit: Some(1),
            parts: agena_api::resource::PartLoadMode::Summary,
        }),
    )
    .await
    .expect("first list messages query should succeed");
    let QueryResult::Messages(first_page) = result else {
        panic!("expected message list result");
    };
    assert_eq!(first_page.items.len(), 1);
    assert_eq!(first_page.page.returned, 1);
    assert!(
        first_page.page.has_more,
        "first page should report more rows"
    );
    let next_cursor = first_page
        .page
        .next_cursor
        .clone()
        .expect("first page should include a cursor");
    assert_eq!(first_page.items[0].id, 7202);

    let first_part = first_page.items[0]
        .parts
        .as_ref()
        .and_then(|parts| parts.first())
        .expect("summary mode should include part headers");
    assert_eq!(first_part.summary.as_deref(), Some("newer summary"));
    assert!(
        first_part.content.is_none(),
        "summary mode should omit full content"
    );

    let result = dispatch_query(
        &state,
        Query::ListMessages(ListMessagesParams {
            session_id,
            cursor: Some(next_cursor),
            limit: Some(1),
            parts: agena_api::resource::PartLoadMode::Summary,
        }),
    )
    .await
    .expect("second list messages query should succeed");
    let QueryResult::Messages(second_page) = result else {
        panic!("expected message list result");
    };
    assert_eq!(second_page.items.len(), 1);
    assert_eq!(second_page.items[0].id, 7201);
    assert!(
        !second_page.page.has_more,
        "cursor should advance to the final page"
    );
}

#[tokio::test]
async fn list_messages_query_preserves_parts_modes() {
    let (state, workspace_root) = test_state_with_config(
        r#"
[providers.openai]
default_model = "openai/gpt-5.4"

[providers.openai.auth]
mode = "api"
base_url = "https://api.openai.com"
api_key = "dummy"

[providers.openai.adapters.openai]
enabled = true
"#,
        "message-parts-dispatch",
    )
    .await;
    let session_id = create_session(&state, &workspace_root, "message parts").await;
    let created_at = chrono::Utc::now().timestamp_millis();

    insert_projected_message_with_text_part(
        &state,
        session_id,
        7401,
        7501,
        created_at,
        "dispatch summary",
        "dispatch full text",
    )
    .await;

    let result = dispatch_query(
        &state,
        Query::ListMessages(ListMessagesParams {
            session_id,
            cursor: None,
            limit: Some(1),
            parts: agena_api::resource::PartLoadMode::None,
        }),
    )
    .await
    .expect("none list messages query should succeed");
    let QueryResult::Messages(none_page) = result else {
        panic!("expected message list result");
    };
    assert!(
        none_page.items[0].parts.is_none(),
        "none mode should omit parts"
    );
    assert_eq!(none_page.items[0].part_count, 1);

    let result = dispatch_query(
        &state,
        Query::ListMessages(ListMessagesParams {
            session_id,
            cursor: None,
            limit: Some(1),
            parts: agena_api::resource::PartLoadMode::Summary,
        }),
    )
    .await
    .expect("summary list messages query should succeed");
    let QueryResult::Messages(summary_page) = result else {
        panic!("expected message list result");
    };
    let summary_part = summary_page.items[0]
        .parts
        .as_ref()
        .and_then(|parts| parts.first())
        .expect("summary mode should include part headers");
    assert_eq!(summary_part.summary.as_deref(), Some("dispatch summary"));
    assert!(
        summary_part.content.is_none(),
        "summary mode should omit full content"
    );

    let result = dispatch_query(
        &state,
        Query::ListMessages(ListMessagesParams {
            session_id,
            cursor: None,
            limit: Some(1),
            parts: agena_api::resource::PartLoadMode::Full,
        }),
    )
    .await
    .expect("full list messages query should succeed");
    let QueryResult::Messages(full_page) = result else {
        panic!("expected message list result");
    };
    let full_part = full_page.items[0]
        .parts
        .as_ref()
        .and_then(|parts| parts.first())
        .expect("full mode should include full parts");
    assert_eq!(full_part.summary.as_deref(), Some("dispatch summary"));
    assert_eq!(full_part.text(), Some("dispatch full text"));
}

#[tokio::test]
async fn run_options_to_core_uses_single_provider_default_when_model_absent() {
    let (state, workspace_root) = test_state_with_config(
        r#"
[providers.openai]
default_model = "openai/gpt-5.4"

[providers.openai.auth]
mode = "api"
base_url = "https://api.openai.com"
api_key = "dummy"

[providers.openai.adapters.openai]
enabled = true
"#,
        "single-provider-default",
    )
    .await;
    let session_id = create_session(&state, &workspace_root, "single provider").await;
    let options = RunOptions {
        model: None,
        thinking_mode: None,
        speed_mode: None,
        verbosity: None,
        parallel_tool_calls: None,
        agent_profile: None,
        system: None,
        temperature: None,
        max_output_tokens: None,
        max_turn_loops: None,
    };
    let core = run_options_to_core(&state, session_id, &options)
        .await
        .expect("single provider should resolve default model");
    assert_eq!(core.model.provider_id.as_str(), "openai");
    assert_eq!(core.model.model_id.as_str(), "openai/gpt-5.4");
}

#[tokio::test]
async fn run_options_to_core_errors_when_model_absent_and_multiple_providers_exist() {
    let (state, workspace_root) = test_state_with_config(
        r#"
[providers.openai]
default_model = "openai/gpt-5.4"

[providers.openai.auth]
mode = "api"
base_url = "https://api.openai.com"
api_key = "dummy"

[providers.openai.adapters.openai]
enabled = true

[providers.ollama]
default_model = "ollama/qwen3:14b"

[providers.ollama.adapters.ollama]
enabled = true
base_url = "http://localhost:11434"
"#,
        "multiple-provider-default",
    )
    .await;
    let session_id = create_session(&state, &workspace_root, "multiple providers").await;
    let options = RunOptions {
        model: None,
        thinking_mode: None,
        speed_mode: None,
        verbosity: None,
        parallel_tool_calls: None,
        agent_profile: None,
        system: None,
        temperature: None,
        max_output_tokens: None,
        max_turn_loops: None,
    };
    let error = run_options_to_core(&state, session_id, &options)
        .await
        .expect_err("multiple providers should require an explicit or inferred model");
    assert!(
        matches!(error, ServerError::BadRequest(message) if message.contains("model is required"))
    );
}

#[tokio::test]
async fn run_options_to_core_round_trips_explicit_model() {
    let (state, workspace_root) = test_state_with_config(
        r#"
[providers.openai]
default_model = "openai/gpt-5.4"

[providers.openai.auth]
mode = "api"
base_url = "https://api.openai.com"
api_key = "dummy"

[providers.openai.adapters.openai]
enabled = true

[providers.ollama]
default_model = "ollama/qwen3:14b"

[providers.ollama.adapters.ollama]
enabled = true
base_url = "http://localhost:11434"
"#,
        "explicit-model",
    )
    .await;
    let session_id = create_session(&state, &workspace_root, "explicit model").await;
    let options = RunOptions {
        model: Some(ModelRef::new("openai", "openai/gpt-5.4")),
        thinking_mode: None,
        speed_mode: None,
        verbosity: None,
        parallel_tool_calls: None,
        agent_profile: None,
        system: Some("be concise".into()),
        temperature: Some(0.7),
        max_output_tokens: Some(256),
        max_turn_loops: None,
    };
    let core = run_options_to_core(&state, session_id, &options)
        .await
        .expect("explicit model should bypass default inference");
    assert_eq!(core.model.provider_id.as_str(), "openai");
    assert_eq!(core.model.model_id.as_str(), "openai/gpt-5.4");
    assert_eq!(core.system.as_deref(), Some("be concise"));
    assert_eq!(core.temperature, Some(0.7));
    assert_eq!(core.max_output_tokens, Some(256));
}

#[tokio::test]
async fn run_options_to_core_resolves_model_thinking_mode() {
    let (state, workspace_root) = test_state_with_config(
        r#"
[providers.openai]
default_model = "openai/gpt-5.4"

[providers.openai.auth]
mode = "api"
base_url = "https://api.openai.com"
api_key = "dummy"

[providers.openai.adapters.openai]
enabled = true

[providers.openai.adapters.openai.models."gpt-5.4".thinking_modes.light]
thinking = { type = "budget", budget_tokens = 3000 }

[providers.openai.adapters.openai.models."gpt-5.4".thinking_modes.deep]
thinking = { type = "budget", budget_tokens = 30000 }
"#,
        "model-thinking-mode",
    )
    .await;
    let session_id = create_session(&state, &workspace_root, "thinking mode").await;
    let options = RunOptions {
        model: Some(ModelRef::new("openai", "openai/gpt-5.4")),
        thinking_mode: Some("deep".to_string()),
        speed_mode: None,
        verbosity: None,
        parallel_tool_calls: None,
        agent_profile: None,
        system: None,
        temperature: None,
        max_output_tokens: None,
        max_turn_loops: None,
    };

    let core = run_options_to_core(&state, session_id, &options)
        .await
        .expect("thinking mode should resolve");

    assert_eq!(core.thinking_mode.as_deref(), Some("deep"));
    assert_eq!(
        core.thinking,
        Some(agena::provider::ThinkingRequest::Budget {
            budget_tokens: 30000
        })
    );
}

#[tokio::test]
async fn run_options_to_core_resolves_model_thinking_mode_and_merges_adapter_override() {
    let (state, workspace_root) = test_state_with_config(
        r#"
[providers.openai]
default_adapter = "openai"
default_model = "openai/gpt-5.4"

[providers.openai.auth]
mode = "api"
base_url = "https://api.openai.com"
api_key = "dummy"

[providers.openai.adapters.openai]
enabled = true

[providers.openai.adapters.openai.models."gpt-5.4".thinking_modes.deep]
thinking = { type = "effort", effort = "high" }

[providers.openai.adapters.openai.models."gpt-5.4".thinking_modes.deep.request_override]
body_patch = { reasoning = { summary = "auto" } }

[providers.openai.adapters.openai.models."gpt-5.4".thinking_modes.deep.adapter_overrides.openai]
headers = { x_reasoning_profile = "deep" }
"#,
        "model-thinking-mode-override",
    )
    .await;
    let session_id = create_session(&state, &workspace_root, "thinking mode override").await;
    let options = RunOptions {
        model: Some(ModelRef::new("openai", "openai/gpt-5.4")),
        thinking_mode: Some("deep".to_string()),
        speed_mode: None,
        verbosity: None,
        parallel_tool_calls: None,
        agent_profile: None,
        system: None,
        temperature: None,
        max_output_tokens: None,
        max_turn_loops: None,
    };

    let core = run_options_to_core(&state, session_id, &options)
        .await
        .expect("thinking mode with override should resolve");

    assert_eq!(core.thinking_mode.as_deref(), Some("deep"));
    assert_eq!(
        core.thinking,
        Some(agena::provider::ThinkingRequest::Effort {
            effort: agena::provider::ReasoningEffort::High
        })
    );
    assert_eq!(
        core.request_override.body_patch.get("reasoning"),
        Some(&serde_json::json!({ "summary": "auto" }))
    );
    assert_eq!(
        core.request_override
            .headers
            .get("x_reasoning_profile")
            .map(String::as_str),
        Some("deep")
    );
}

#[tokio::test]
async fn run_options_to_core_resolves_model_speed_mode_and_merges_adapter_override() {
    let (state, workspace_root) = test_state_with_config(
        r#"
[providers.openai]
default_adapter = "openai"
default_model = "openai/gpt-5.4"

[providers.openai.auth]
mode = "api"
base_url = "https://api.openai.com"
api_key = "dummy"

[providers.openai.adapters.openai]
enabled = true

[providers.openai.adapters.openai.models."gpt-5.4".speed_modes.fast]
request_override = { headers = { x-mode = "fast" }, body_patch = { service_tier = "priority" } }

[providers.openai.adapters.openai.models."gpt-5.4".speed_modes.fast.adapter_overrides.openai]
headers = { x-openai-mode = "fast" }
body_patch = { response_format = "json_schema" }
"#,
        "model-speed-mode",
    )
    .await;
    let session_id = create_session(&state, &workspace_root, "speed mode").await;
    let options = RunOptions {
        model: Some(ModelRef::new_with_adapter(
            "openai",
            "openai",
            "openai/gpt-5.4",
        )),
        thinking_mode: None,
        speed_mode: Some("fast".to_string()),
        verbosity: None,
        parallel_tool_calls: None,
        agent_profile: None,
        system: None,
        temperature: None,
        max_output_tokens: None,
        max_turn_loops: None,
    };

    let core = run_options_to_core(&state, session_id, &options)
        .await
        .expect("speed mode should resolve");

    assert_eq!(core.speed_mode.as_deref(), Some("fast"));
    assert_eq!(
        core.request_override
            .headers
            .get("x-mode")
            .map(String::as_str),
        Some("fast")
    );
    assert_eq!(
        core.request_override
            .headers
            .get("x-openai-mode")
            .map(String::as_str),
        Some("fast")
    );
    assert_eq!(
        core.request_override.body_patch.get("service_tier"),
        Some(&serde_json::Value::String("priority".to_string()))
    );
    assert_eq!(
        core.request_override.body_patch.get("response_format"),
        Some(&serde_json::Value::String("json_schema".to_string()))
    );
}

#[tokio::test]
async fn run_options_to_core_resolves_adaptive_thinking_mode() {
    let (state, workspace_root) = test_state_with_config(
        r#"
[providers.bedrock]
default_adapter = "amazon_bedrock"
default_model = "anthropic.claude-opus-4-7"

[providers.bedrock.auth]
mode = "bedrock_sigv4"
region = "us-east-1"
access_key_id = "AKIDEXAMPLE"
secret_access_key = "secret"

[providers.bedrock.adapters.amazon_bedrock]
enabled = true

[providers.bedrock.adapters.amazon_bedrock.models."anthropic.claude-opus-4-7".thinking_modes.light]
thinking = { type = "adaptive", effort = "low" }
"#,
        "adaptive-thinking-mode",
    )
    .await;
    let session_id = create_session(&state, &workspace_root, "adaptive thinking mode").await;
    let options = RunOptions {
        model: Some(ModelRef::new("bedrock", "anthropic.claude-opus-4-7")),
        thinking_mode: Some("light".to_string()),
        speed_mode: None,
        verbosity: None,
        parallel_tool_calls: None,
        agent_profile: None,
        system: None,
        temperature: None,
        max_output_tokens: None,
        max_turn_loops: None,
    };

    let core = run_options_to_core(&state, session_id, &options)
        .await
        .expect("adaptive thinking mode should resolve");

    assert_eq!(core.thinking_mode.as_deref(), Some("light"));
    assert_eq!(
        core.thinking,
        Some(agena::provider::ThinkingRequest::Adaptive {
            effort: Some(agena::provider::ReasoningEffort::Low),
            display: None,
        })
    );
}

#[tokio::test]
async fn run_options_to_core_resolves_default_and_explicit_verbosity() {
    let (state, workspace_root) = test_state_with_config(
        r#"
[providers.openai]
default_model = "openai/gpt-5.4"

[providers.openai.auth]
mode = "api"
base_url = "https://api.openai.com"
api_key = "dummy"

[providers.openai.adapters.openai]
enabled = true

[providers.openai.adapters.openai.models."gpt-5.4"]
supports_verbosity = true
default_verbosity = "low"
"#,
        "model-verbosity",
    )
    .await;
    let session_id = create_session(&state, &workspace_root, "verbosity").await;

    let default_options = RunOptions {
        model: Some(ModelRef::new("openai", "openai/gpt-5.4")),
        thinking_mode: None,
        speed_mode: None,
        verbosity: None,
        parallel_tool_calls: None,
        agent_profile: None,
        system: None,
        temperature: None,
        max_output_tokens: None,
        max_turn_loops: None,
    };
    let default_core = run_options_to_core(&state, session_id, &default_options)
        .await
        .expect("default verbosity should resolve");
    assert_eq!(default_core.verbosity.as_deref(), Some("low"));

    let explicit_options = RunOptions {
        model: Some(ModelRef::new("openai", "openai/gpt-5.4")),
        thinking_mode: None,
        speed_mode: None,
        verbosity: Some("HIGH".to_string()),
        parallel_tool_calls: None,
        agent_profile: None,
        system: None,
        temperature: None,
        max_output_tokens: None,
        max_turn_loops: None,
    };
    let explicit_core = run_options_to_core(&state, session_id, &explicit_options)
        .await
        .expect("explicit verbosity should resolve");
    assert_eq!(explicit_core.verbosity.as_deref(), Some("high"));
}

#[tokio::test]
async fn run_options_to_core_resolves_default_and_explicit_temperature() {
    let (state, workspace_root) = test_state_with_config(
        r#"
[providers.openai]
default_model = "qwen/qwen3-next-80b-a3b"

[providers.openai.auth]
mode = "api"
base_url = "https://api.openai.com"
api_key = "dummy"

[providers.openai.adapters.openai]
enabled = true
"#,
        "model-temperature",
    )
    .await;
    let session_id = create_session(&state, &workspace_root, "temperature").await;

    let default_options = RunOptions {
        model: Some(ModelRef::new("openai", "qwen/qwen3-next-80b-a3b")),
        thinking_mode: None,
        speed_mode: None,
        verbosity: None,
        parallel_tool_calls: None,
        agent_profile: None,
        system: None,
        temperature: None,
        max_output_tokens: None,
        max_turn_loops: None,
    };
    let default_core = run_options_to_core(&state, session_id, &default_options)
        .await
        .expect("default temperature should resolve");
    assert_eq!(default_core.temperature, Some(0.55));

    let explicit_options = RunOptions {
        temperature: Some(0.2),
        ..default_options
    };
    let explicit_core = run_options_to_core(&state, session_id, &explicit_options)
        .await
        .expect("explicit temperature should resolve");
    assert_eq!(explicit_core.temperature, Some(0.2));
}

#[tokio::test]
async fn run_options_to_core_rejects_unsupported_chat_model_verbosity() {
    let (state, workspace_root) = test_state_with_config(
        r#"
[providers.openai]
default_model = "openai/gpt-5.2-chat-latest"

[providers.openai.auth]
mode = "api"
base_url = "https://api.openai.com"
api_key = "dummy"

[providers.openai.adapters.openai]
enabled = true

[providers.openai.adapters.openai.models."gpt-5.2-chat-latest"]
supports_verbosity = true
default_verbosity = "medium"
"#,
        "chat-model-verbosity",
    )
    .await;
    let session_id = create_session(&state, &workspace_root, "chat verbosity").await;

    let allowed_options = RunOptions {
        model: Some(ModelRef::new("openai", "openai/gpt-5.2-chat-latest")),
        thinking_mode: None,
        speed_mode: None,
        verbosity: Some("medium".to_string()),
        parallel_tool_calls: None,
        agent_profile: None,
        system: None,
        temperature: None,
        max_output_tokens: None,
        max_turn_loops: None,
    };
    let allowed_core = run_options_to_core(&state, session_id, &allowed_options)
        .await
        .expect("chat model medium verbosity should resolve");
    assert_eq!(allowed_core.verbosity.as_deref(), Some("medium"));

    let rejected = run_options_to_core(
        &state,
        session_id,
        &RunOptions {
            verbosity: Some("high".to_string()),
            ..allowed_options
        },
    )
    .await
    .expect_err("chat model high verbosity should be rejected");
    assert!(
        rejected.to_string().contains("supported values: medium"),
        "unexpected error: {rejected}"
    );
}

#[tokio::test]
async fn run_options_to_core_threads_parallel_tool_calls_into_request_override() {
    let (state, workspace_root) = test_state_with_config(
        r#"
[providers.openai]
default_model = "openai/gpt-5.4"

[providers.openai.auth]
mode = "api"
base_url = "https://api.openai.com"
api_key = "dummy"

[providers.openai.adapters.openai]
enabled = true

[providers.openai.adapters.openai.models."gpt-5.4"]
supports_parallel_tool_calls = true
"#,
        "parallel-tool-calls",
    )
    .await;
    let session_id = create_session(&state, &workspace_root, "parallel tool calls").await;

    let enabled_options = RunOptions {
        model: Some(ModelRef::new("openai", "openai/gpt-5.4")),
        thinking_mode: None,
        speed_mode: None,
        verbosity: None,
        parallel_tool_calls: Some(true),
        agent_profile: None,
        system: None,
        temperature: None,
        max_output_tokens: None,
        max_turn_loops: None,
    };
    let enabled_core = run_options_to_core(&state, session_id, &enabled_options)
        .await
        .expect("parallel tool calls enabled should resolve");
    assert_eq!(
        enabled_core.request_override.parallel_tool_calls(),
        Some(true)
    );

    let disabled_options = RunOptions {
        parallel_tool_calls: Some(false),
        ..enabled_options
    };
    let disabled_core = run_options_to_core(&state, session_id, &disabled_options)
        .await
        .expect("parallel tool calls disabled should resolve");
    assert_eq!(
        disabled_core.request_override.parallel_tool_calls(),
        Some(false)
    );
}

#[tokio::test]
async fn run_options_to_core_rejects_parallel_tool_calls_for_unsupported_model() {
    let (state, workspace_root) = test_state_with_config(
        r#"
[providers.openai]
default_model = "openai/gpt-4.1-mini"

[providers.openai.auth]
mode = "api"
base_url = "https://api.openai.com"
api_key = "dummy"

[providers.openai.adapters.openai]
enabled = true

[providers.openai.adapters.openai.models."gpt-4.1-mini"]
supports_parallel_tool_calls = false
"#,
        "parallel-tool-calls-unsupported",
    )
    .await;
    let session_id =
        create_session(&state, &workspace_root, "parallel tool calls unsupported").await;

    let rejected = run_options_to_core(
        &state,
        session_id,
        &RunOptions {
            model: Some(ModelRef::new("openai", "openai/gpt-4.1-mini")),
            thinking_mode: None,
            speed_mode: None,
            verbosity: None,
            parallel_tool_calls: Some(true),
            agent_profile: None,
            system: None,
            temperature: None,
            max_output_tokens: None,
            max_turn_loops: None,
        },
    )
    .await
    .expect_err("unsupported model should reject parallel tool calls");
    assert!(
        rejected
            .to_string()
            .contains("does not support parallel tool calls"),
        "unexpected error: {rejected}"
    );
}

#[tokio::test]
async fn run_options_to_core_rejects_unknown_model_thinking_mode() {
    let (state, workspace_root) = test_state_with_config(
        r#"
[providers.openai]
default_model = "openai/gpt-5.4"

[providers.openai.auth]
mode = "api"
base_url = "https://api.openai.com"
api_key = "dummy"

[providers.openai.adapters.openai]
enabled = true

[providers.openai.adapters.openai.models."gpt-5.4".thinking_modes.light]
thinking = { type = "budget", budget_tokens = 3000 }
"#,
        "unknown-model-thinking-mode",
    )
    .await;
    let session_id = create_session(&state, &workspace_root, "thinking mode").await;
    let options = RunOptions {
        model: Some(ModelRef::new("openai", "openai/gpt-5.4")),
        thinking_mode: Some("deep".to_string()),
        speed_mode: None,
        verbosity: None,
        parallel_tool_calls: None,
        agent_profile: None,
        system: None,
        temperature: None,
        max_output_tokens: None,
        max_turn_loops: None,
    };

    let error = run_options_to_core(&state, session_id, &options)
        .await
        .expect_err("unknown thinking mode should be rejected");

    assert!(
        matches!(error, ServerError::BadRequest(message) if message.contains("has no thinking mode `deep`"))
    );
}

#[tokio::test]
async fn runtime_query_includes_agent_inventory() {
    let (state, _) = test_state_with_config(
        r#"
[providers.openai]
default_model = "openai/gpt-5.4"

[providers.openai.auth]
mode = "api"
base_url = "https://api.openai.com"
api_key = "dummy"

[providers.openai.adapters.openai]
enabled = true
"#,
        "runtime-agent-inventory",
    )
    .await;

    let result = dispatch_query(&state, Query::Runtime)
        .await
        .expect("runtime query should succeed");
    let QueryResult::Runtime(runtime) = result else {
        panic!("expected runtime query result");
    };

    assert_eq!(runtime.operator.agents.default_agent, "build");
    assert!(runtime.operator.agents.total_count >= 1);
    assert!(runtime.operator.agents.primary_count >= 1);
    assert!(
        runtime
            .operator
            .agents
            .agents
            .iter()
            .any(|agent| agent.name == "build" && agent.mode.allows_root())
    );
    assert!(
        runtime
            .operator
            .agents
            .agents
            .iter()
            .any(|agent| agent.name == "planner")
    );
    assert!(
        runtime
            .operator
            .agents
            .agents
            .iter()
            .any(|agent| agent.name == "scout")
    );
}

#[test]
fn server_error_from_http_preserves_bad_request() {
    let err = crate::local_api::ApiError::bad_request("boom");
    let server_err = server_error_from_http(err);
    assert!(matches!(server_err, ServerError::BadRequest(message) if message == "boom"));
}
