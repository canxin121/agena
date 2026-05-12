use std::{
    collections::BTreeMap,
    io::Write,
    path::Path,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use agena::{
    agent::Agent,
    config::{ConfigEnvironment, ConfigLoader, LoadConfigRequest},
    db,
    message::{Message, PartContent},
    model::ModelRef,
    permission::PermissionPolicy,
    provider::{
        CompletionRequest, CompletionResponse, CompletionStreamEvent, CompletionToolCall,
        CompletionUsage, ProviderRegistry,
    },
    role::Role,
    session::{
        ContextGovernor, ContextPolicy, SessionContinueRequest, SessionCreateRequest,
        SessionForkRequest, SessionListRequest, SessionManager, SessionProcessor,
        SessionRewindRequest, SessionRunOptions, SessionUnrewindRequest, SessionUserTurnRequest,
    },
    tool::{EntryBehavior, EntryDefinition, ToolExecutor},
};
use futures_util::StreamExt;
use sea_orm::{Database, DatabaseConnection};

const LIVE_BASE_URL: &str = "https://api.cxits.cn";
const LIVE_MODEL: &str = "gpt-5.4";
const LIVE_KEY_ENV: &str = "CX_API_KEY";
const CACHE_PROBE_ATTEMPTS: usize = 8;
const CACHE_PROBE_PREFIX_REPETITIONS: usize = 4000;
const CACHE_PROBE_RETRY_DELAY: Duration = Duration::from_secs(5);
const LIVE_REQUEST_RETRY_MAX_RETRIES: u32 = 6;
const LIVE_REQUEST_RETRY_BASE_DELAY_MS: u64 = 1_000;
const LIVE_REQUEST_RETRY_MAX_DELAY_MS: u64 = 10_000;
const LIVE_STREAM_REPLAY_MAX_RETRIES_AFTER_OUTPUT: u32 = 3;
const LIVE_STREAM_REPLAY_MAX_TRACKED_EVENTS: usize = 256;

const PROVIDERS: &[LiveProviderCase] = &[
    LiveProviderCase::new("openai_live", "openai"),
    LiveProviderCase::new("compat_live", "compat"),
    LiveProviderCase::new("anthropic_live", "anthropic"),
    LiveProviderCase::new("gemini_live", "gemini"),
];

#[derive(Clone)]
struct TestEnv {
    vars: BTreeMap<String, String>,
}

impl ConfigEnvironment for TestEnv {
    fn var(&self, key: &str) -> Option<String> {
        self.vars.get(key).cloned()
    }

    fn vars(&self) -> Vec<(String, String)> {
        self.vars
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect()
    }
}

#[derive(Clone, Copy)]
struct LiveProviderCase {
    provider_id: &'static str,
    slug: &'static str,
}

impl LiveProviderCase {
    const fn new(provider_id: &'static str, slug: &'static str) -> Self {
        Self { provider_id, slug }
    }

    fn model_ref(self) -> ModelRef {
        ModelRef::new(self.provider_id, LIVE_MODEL)
    }
}

fn live_key() -> String {
    std::env::var(LIVE_KEY_ENV)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .expect("CX_API_KEY must be set for cliproxy live tests")
}

fn env_usize_or(key: &str, default: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .unwrap_or(default)
}

fn env_u32_or(key: &str, default: u32) -> u32 {
    std::env::var(key)
        .ok()
        .and_then(|value| value.trim().parse::<u32>().ok())
        .unwrap_or(default)
}

fn env_u64_or(key: &str, default: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(default)
}

fn live_cache_probe_attempts() -> usize {
    env_usize_or(
        "AGENA_CLIPROXY_LIVE_CACHE_PROBE_ATTEMPTS",
        CACHE_PROBE_ATTEMPTS,
    )
}

fn live_cache_probe_retry_delay() -> Duration {
    Duration::from_millis(env_u64_or(
        "AGENA_CLIPROXY_LIVE_CACHE_PROBE_RETRY_DELAY_MS",
        CACHE_PROBE_RETRY_DELAY.as_millis() as u64,
    ))
}

fn live_request_retry_max_retries() -> u32 {
    env_u32_or(
        "AGENA_CLIPROXY_LIVE_REQUEST_MAX_RETRIES",
        LIVE_REQUEST_RETRY_MAX_RETRIES,
    )
}

fn live_request_retry_base_delay_ms() -> u64 {
    env_u64_or(
        "AGENA_CLIPROXY_LIVE_REQUEST_BASE_DELAY_MS",
        LIVE_REQUEST_RETRY_BASE_DELAY_MS,
    )
}

fn live_request_retry_max_delay_ms() -> u64 {
    env_u64_or(
        "AGENA_CLIPROXY_LIVE_REQUEST_MAX_DELAY_MS",
        LIVE_REQUEST_RETRY_MAX_DELAY_MS,
    )
}

fn live_stream_replay_max_retries_after_output() -> u32 {
    env_u32_or(
        "AGENA_CLIPROXY_LIVE_STREAM_REPLAY_MAX_RETRIES_AFTER_OUTPUT",
        LIVE_STREAM_REPLAY_MAX_RETRIES_AFTER_OUTPUT,
    )
}

fn live_stream_replay_max_tracked_events() -> usize {
    env_usize_or(
        "AGENA_CLIPROXY_LIVE_STREAM_REPLAY_MAX_TRACKED_EVENTS",
        LIVE_STREAM_REPLAY_MAX_TRACKED_EVENTS,
    )
}

fn required_test_env() -> TestEnv {
    let mut vars = BTreeMap::new();
    vars.insert(LIVE_KEY_ENV.to_owned(), live_key());
    TestEnv { vars }
}

fn placeholder_test_env() -> TestEnv {
    let mut vars = BTreeMap::new();
    vars.insert(LIVE_KEY_ENV.to_owned(), "test-key".to_owned());
    TestEnv { vars }
}

fn write_temp_config(content: &str) -> tempfile::NamedTempFile {
    let mut file = tempfile::NamedTempFile::new().expect("create temp config");
    file.write_all(content.as_bytes())
        .expect("write temp config");
    file
}

fn live_config_text(auth_path: &Path) -> String {
    let request_retry_max_retries = live_request_retry_max_retries();
    let request_retry_base_delay_ms = live_request_retry_base_delay_ms();
    let request_retry_max_delay_ms = live_request_retry_max_delay_ms();
    let stream_replay_max_retries_after_output = live_stream_replay_max_retries_after_output();
    let stream_replay_max_tracked_events = live_stream_replay_max_tracked_events();

    format!(
        r#"
[auth]
store_backend = "file"
store_path = "{auth_path}"

[runtime.request_retry]
max_retries = {request_retry_max_retries}
base_delay_ms = {request_retry_base_delay_ms}
max_delay_ms = {request_retry_max_delay_ms}

[runtime.stream_replay]
max_retries_after_output = {stream_replay_max_retries_after_output}
max_tracked_events = {stream_replay_max_tracked_events}

[providers.openai_live]
kind = "openai"
base_url = "{base_url}/api/provider/openai/v1"
default_model = "{model}"
api_key_env = "{key_env}"

[providers.compat_live]
kind = "openai_compatible"
base_url = "{base_url}/api/provider/openai/v1"
default_model = "{model}"
api_key_env = "{key_env}"
auth_header = "authorization"
auth_scheme = "Bearer"

[providers.anthropic_live]
kind = "anthropic"
base_url = "{base_url}/api/provider/anthropic/v1"
default_model = "{model}"
api_key_env = "{key_env}"
auth_header = "authorization"
auth_scheme = "Bearer"

[providers.gemini_live]
kind = "gemini"
base_url = "{base_url}/api/provider/google/v1beta"
default_model = "{model}"
api_key_env = "{key_env}"
"#,
        auth_path = auth_path.display(),
        base_url = LIVE_BASE_URL,
        model = LIVE_MODEL,
        key_env = LIVE_KEY_ENV,
        request_retry_max_retries = request_retry_max_retries,
        request_retry_base_delay_ms = request_retry_base_delay_ms,
        request_retry_max_delay_ms = request_retry_max_delay_ms,
        stream_replay_max_retries_after_output = stream_replay_max_retries_after_output,
        stream_replay_max_tracked_events = stream_replay_max_tracked_events,
    )
}

fn load_live_registry() -> ProviderRegistry {
    let auth_dir = tempfile::tempdir().expect("create auth tempdir");
    let auth_path = auth_dir.path().join("auth.json");
    let config = live_config_text(&auth_path);
    let file = write_temp_config(config.as_str());
    let loader = ConfigLoader::new(required_test_env());
    let resolution = loader
        .load(&LoadConfigRequest {
            config_path: Some(file.path().to_path_buf()),
            overrides: Vec::new(),
        })
        .expect("live config should load");

    resolution
        .config
        .build_provider_registry_with_env(loader.environment())
        .expect("live provider registry should build")
}

async fn open_live_database(path: &Path) -> DatabaseConnection {
    let url = format!("sqlite://{}?mode=rwc", path.display());
    let db = Database::connect(url)
        .await
        .expect("connect live sqlite db");
    db::init_schema(&db).await.expect("init live sqlite schema");
    db
}

async fn build_live_session_manager(
    workspace_root: &Path,
    db: DatabaseConnection,
) -> Arc<SessionManager> {
    let registry = load_live_registry();
    let processor = SessionProcessor::new(
        Arc::new(registry),
        ContextGovernor::new(ContextPolicy::default()),
    );
    let executor = ToolExecutor::new(
        workspace_root.to_path_buf(),
        Agent::new("cliproxy-live-test", PermissionPolicy::allow_all()),
    );
    Arc::new(SessionManager::new(db, processor, executor))
}

fn hi_request() -> CompletionRequest {
    CompletionRequest {
        model: agena::provider::ModelId::new(LIVE_MODEL),
        system: Some("Reply with short outputs only.".to_owned()),
        messages: vec![Message::prompt_text(Role::User, "hi")],
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
    }
}

fn tool_request() -> CompletionRequest {
    CompletionRequest {
        model: agena::provider::ModelId::new(LIVE_MODEL),
        system: Some("Use the tool exactly once and do not answer directly.".to_owned()),
        messages: vec![Message::prompt_text(
            Role::User,
            "Use the provided tool exactly once and do not answer directly.",
        )],
        tools: vec![
            EntryDefinition::plugin(
                "echo",
                "Echo input",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "value": { "type": "string" }
                    },
                    "required": ["value"],
                    "additionalProperties": false
                }),
                EntryBehavior::ReadOnly,
                "live-test",
            )
            .with_strict(true),
        ],
        temperature: None,
        max_output_tokens: Some(128),
        prompt_cache_key: None,
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

fn unique_nonce(label: &str) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after UNIX_EPOCH")
        .as_nanos();
    format!("{label}-{now}")
}

fn cache_probe_prefix(label: &str) -> String {
    let token = format!("CACHE-MARKER-{label}");
    let mut prefix = String::new();
    for _ in 0..CACHE_PROBE_PREFIX_REPETITIONS {
        prefix.push_str(token.as_str());
        prefix.push(' ');
    }
    prefix
}

fn openai_cache_probe_request(nonce: &str) -> CompletionRequest {
    CompletionRequest {
        model: agena::provider::ModelId::new(LIVE_MODEL),
        system: None,
        messages: vec![Message::prompt_text(
            Role::User,
            format!(
                "{}Reply with exactly OK.",
                cache_probe_prefix(format!("OPENAI-{nonce}").as_str())
            ),
        )],
        tools: Vec::new(),
        temperature: None,
        max_output_tokens: Some(32),
        prompt_cache_key: Some(format!("agena-openai-live-cache-{nonce}")),
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

fn anthropic_cache_probe_request(nonce: &str) -> CompletionRequest {
    CompletionRequest {
        model: agena::provider::ModelId::new(LIVE_MODEL),
        system: Some(cache_probe_prefix(
            format!("ANTHROPIC-SYSTEM-{nonce}").as_str(),
        )),
        messages: vec![Message::prompt_text(Role::User, "Reply with exactly OK.")],
        tools: vec![
            EntryDefinition::plugin(
                "project_search",
                "Search project files for matches.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "query": { "type": "string" }
                    },
                    "required": ["query"],
                    "additionalProperties": false
                }),
                EntryBehavior::ReadOnly,
                "live-cache-test",
            )
            .with_strict(true),
        ],
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
    }
}

fn gemini_cache_probe_request(nonce: &str) -> CompletionRequest {
    CompletionRequest {
        model: agena::provider::ModelId::new(LIVE_MODEL),
        system: None,
        messages: vec![Message::prompt_text(
            Role::User,
            format!(
                "{}Reply with exactly OK.",
                cache_probe_prefix(format!("GEMINI-{nonce}").as_str())
            ),
        )],
        tools: Vec::new(),
        temperature: None,
        max_output_tokens: Some(32),
        prompt_cache_key: None,
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

fn cache_probe_request_for_provider(provider_id: &str, nonce: &str) -> CompletionRequest {
    match provider_id {
        "openai_live" | "compat_live" => openai_cache_probe_request(nonce),
        "anthropic_live" => anthropic_cache_probe_request(nonce),
        "gemini_live" => gemini_cache_probe_request(nonce),
        other => panic!("unsupported live provider for cache probe: {other}"),
    }
}

fn assert_cache_probe_response_correct(provider_id: &str, response: &CompletionResponse) {
    assert_eq!(response.provider_id.as_str(), provider_id);
    assert_eq!(response.model.as_str(), LIVE_MODEL);
    assert!(
        response.text.contains("OK"),
        "{provider_id} cache probe response should contain OK, got {:?}",
        response.text
    );
}

fn cache_read_tokens(usage: &CompletionUsage) -> u64 {
    usage.cache_read_tokens
}

fn assistant_cache_read_tokens(session: &agena::session::Session) -> Option<u64> {
    session
        .messages
        .iter()
        .rev()
        .find(|message| message.role == Role::Assistant)
        .and_then(|message| message.usage.as_ref())
        .map(|usage| usage.cache_read_tokens)
}

fn assistant_message(session: &agena::session::Session) -> &Message {
    session
        .messages
        .iter()
        .rev()
        .find(|message| message.role == Role::Assistant)
        .expect("assistant message should exist")
}

fn user_message_ids(session: &agena::session::Session) -> Vec<i64> {
    session
        .messages
        .iter()
        .filter(|message| message.role == Role::User)
        .map(|message| message.id)
        .collect()
}

fn assert_message_has_reply_text(context: &str, text: &str) {
    assert!(
        !text.trim().is_empty(),
        "{context} should not be empty, got {:?}",
        text
    );
}

fn assert_session_has_nonempty_assistant_reply(context: &str, session: &agena::session::Session) {
    let assistant = assistant_message(session);
    assert_message_has_reply_text(context, assistant.as_text_lossy().as_str());
}

fn run_options(case: LiveProviderCase, system: &str, max_output_tokens: u32) -> SessionRunOptions {
    SessionRunOptions {
        model: case.model_ref(),
        system: Some(system.to_owned()),
        temperature: None,
        max_output_tokens: Some(max_output_tokens),
        agent_profile: None,
        max_turn_loops: None,
    }
}

async fn provider_cache_probe_with_retries(
    registry: &ProviderRegistry,
    provider_id: &str,
) -> Result<(CompletionResponse, CompletionResponse, usize), String> {
    let model = ModelRef::new(provider_id, LIVE_MODEL);
    let nonce = unique_nonce(provider_id);
    let attempts = live_cache_probe_attempts();
    let retry_delay = live_cache_probe_retry_delay();
    let mut last_pair = None;

    for attempt in 1..=attempts {
        let request = cache_probe_request_for_provider(provider_id, nonce.as_str());
        let first = registry
            .complete(&model, request.clone())
            .await
            .map_err(|err| format!("cache warm-up complete failed: {err}"))?;
        let second = registry
            .complete(&model, request)
            .await
            .map_err(|err| format!("cache replay complete failed: {err}"))?;
        let Some(second_usage) = second.usage.as_ref() else {
            return Err("cache replay should carry usage".to_owned());
        };
        if cache_read_tokens(second_usage) > 0 {
            return Ok((first, second, attempt));
        }
        last_pair = Some((first, second, attempt));
        if attempt < attempts {
            tokio::time::sleep(retry_delay).await;
        }
    }

    last_pair.ok_or_else(|| "cache probe produced no attempts".to_owned())
}

async fn session_cache_probe_with_retries(
    manager: &Arc<SessionManager>,
    case: LiveProviderCase,
) -> Result<(agena::session::Session, agena::session::Session, usize), String> {
    let attempts = live_cache_probe_attempts();
    let retry_delay = live_cache_probe_retry_delay();
    let mut last_pair = None;

    for attempt in 1..=attempts {
        let session = manager
            .create_session(SessionCreateRequest {
                title: format!(
                    "cliproxy cache probe {} attempt {}",
                    case.provider_id, attempt
                ),
                parent_session_id: None,
            })
            .await
            .map_err(|err| format!("create live cache probe session failed: {err}"))?;
        let nonce = unique_nonce(format!("{}-{attempt}", case.provider_id).as_str());
        let cache_filler =
            cache_probe_prefix(format!("SESSION-{}-{nonce}", case.provider_id).as_str());
        let first_prompt = format!("{cache_filler} Reply with exactly OK.");
        let second_prompt = "Reply with exactly OK again.";
        let first = manager
            .submit_user_turn(SessionUserTurnRequest {
                session_id: session.id,
                options: run_options(case, "Reply with exactly OK.", 64),
                parts: vec![PartContent::text(first_prompt)],
            })
            .await
            .map_err(|err| format!("submit live cache warm-up turn failed: {err}"))?;
        let second = manager
            .submit_user_turn(SessionUserTurnRequest {
                session_id: session.id,
                options: run_options(case, "Reply with exactly OK.", 65),
                parts: vec![PartContent::text(second_prompt)],
            })
            .await
            .map_err(|err| format!("submit live cache replay turn failed: {err}"))?;
        if assistant_cache_read_tokens(&second).unwrap_or_default() > 0 {
            return Ok((first, second, attempt));
        }
        last_pair = Some((first, second, attempt));
        if attempt < attempts {
            tokio::time::sleep(retry_delay).await;
        }
    }

    last_pair.ok_or_else(|| "session cache probe produced no attempts".to_owned())
}

async fn collect_stream_text(
    stream: std::pin::Pin<
        Box<dyn futures_core::Stream<Item = Result<CompletionStreamEvent, agena::AppError>> + Send>,
    >,
) -> (String, bool) {
    let mut stream = stream;
    let mut text = String::new();
    let mut completed = false;
    while let Some(event) = stream.next().await {
        match event.expect("stream event should parse") {
            CompletionStreamEvent::TextDelta { delta, .. } => text.push_str(&delta),
            CompletionStreamEvent::Completed { .. } => completed = true,
            CompletionStreamEvent::ThinkingDelta { .. }
            | CompletionStreamEvent::ToolCallDelta { .. } => {}
        }
    }
    (text, completed)
}

async fn assert_registry_single_hi(case: LiveProviderCase) {
    let registry = load_live_registry();
    let response = registry
        .complete(&case.model_ref(), hi_request())
        .await
        .unwrap_or_else(|err| {
            panic!(
                "single hi completion failed for {}: {err}",
                case.provider_id
            )
        });
    assert_eq!(response.provider_id.as_str(), case.provider_id);
    assert_eq!(response.model.as_str(), LIVE_MODEL);
    assert_message_has_reply_text(
        format!("{} single hi completion", case.provider_id).as_str(),
        response.text.as_str(),
    );
    let usage = response
        .usage
        .unwrap_or_else(|| panic!("{} single hi should carry usage", case.provider_id));
    assert!(
        usage.input_tokens > 0,
        "{} input tokens should be > 0",
        case.provider_id
    );
    assert!(
        usage.output_tokens > 0,
        "{} output tokens should be > 0",
        case.provider_id
    );
}

async fn assert_registry_stream_hi(case: LiveProviderCase) {
    let registry = load_live_registry();
    let stream = registry
        .complete_stream(&case.model_ref(), hi_request())
        .await
        .unwrap_or_else(|err| panic!("stream hi failed for {}: {err}", case.provider_id));
    let (text, completed) = collect_stream_text(stream).await;
    assert!(
        completed,
        "{} stream should emit Completed",
        case.provider_id
    );
    assert_message_has_reply_text(
        format!("{} stream hi", case.provider_id).as_str(),
        text.as_str(),
    );
}

async fn assert_registry_tool_call(case: LiveProviderCase) {
    let registry = load_live_registry();
    let response = registry
        .complete(&case.model_ref(), tool_request())
        .await
        .unwrap_or_else(|err| panic!("tool complete failed for {}: {err}", case.provider_id));
    assert_eq!(
        response.tool_calls.len(),
        1,
        "{} should return exactly one tool call",
        case.provider_id
    );
    match &response.tool_calls[0] {
        CompletionToolCall::Function {
            name,
            arguments_json,
            ..
        } => {
            assert_eq!(name, "echo", "{} tool name mismatch", case.provider_id);
            let args: serde_json::Value =
                serde_json::from_str(arguments_json).unwrap_or_else(|err| {
                    panic!("{} tool args should be valid JSON: {err}", case.provider_id)
                });
            let echoed = args
                .get("value")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            assert!(
                !echoed.trim().is_empty(),
                "{} tool args should contain non-empty value: {}",
                case.provider_id,
                arguments_json
            );
        }
    }
}

async fn assert_session_single_hi(case: LiveProviderCase) {
    let workspace = tempfile::tempdir().expect("create workspace tempdir");
    let db_path = workspace.path().join(format!("{}-single-hi.db", case.slug));
    let db = open_live_database(&db_path).await;
    let manager = build_live_session_manager(workspace.path(), db).await;

    let session = manager
        .create_session(SessionCreateRequest {
            title: format!("cliproxy live {} single hi", case.provider_id),
            parent_session_id: None,
        })
        .await
        .expect("create live session");
    let updated = manager
        .submit_user_turn(SessionUserTurnRequest {
            session_id: session.id,
            options: run_options(case, "Reply with a brief greeting.", 64),
            parts: vec![PartContent::text("hi")],
        })
        .await
        .unwrap_or_else(|err| {
            panic!(
                "submit single hi session turn failed for {}: {err}",
                case.provider_id
            )
        });

    assert_session_has_nonempty_assistant_reply(
        format!("{} single hi session", case.provider_id).as_str(),
        &updated,
    );
}

async fn assert_session_multi_turn(case: LiveProviderCase) {
    let workspace = tempfile::tempdir().expect("create workspace tempdir");
    let db_path = workspace
        .path()
        .join(format!("{}-multi-turn.db", case.slug));
    let db = open_live_database(&db_path).await;
    let manager = build_live_session_manager(workspace.path(), db).await;

    let session = manager
        .create_session(SessionCreateRequest {
            title: format!("cliproxy live {} multi turn", case.provider_id),
            parent_session_id: None,
        })
        .await
        .expect("create live session");

    let first = manager
        .submit_user_turn(SessionUserTurnRequest {
            session_id: session.id,
            options: run_options(case, "Reply briefly and clearly.", 64),
            parts: vec![PartContent::text("hi, this is turn one")],
        })
        .await
        .unwrap_or_else(|err| panic!("submit first turn failed for {}: {err}", case.provider_id));
    assert_session_has_nonempty_assistant_reply(
        format!("{} first session turn", case.provider_id).as_str(),
        &first,
    );

    let second = manager
        .submit_user_turn(SessionUserTurnRequest {
            session_id: session.id,
            options: run_options(case, "Reply briefly and clearly.", 64),
            parts: vec![PartContent::text("now answer turn two briefly")],
        })
        .await
        .unwrap_or_else(|err| panic!("submit second turn failed for {}: {err}", case.provider_id));
    assert_session_has_nonempty_assistant_reply(
        format!("{} second session turn", case.provider_id).as_str(),
        &second,
    );

    let user_messages = second
        .messages
        .iter()
        .filter(|message| message.role == Role::User)
        .count();
    assert!(
        user_messages >= 2,
        "{} session should retain at least two user turns, got {}",
        case.provider_id,
        user_messages
    );
}

async fn assert_registry_cache_hit(case: LiveProviderCase) {
    let registry = load_live_registry();
    let (first, second, attempts_used) =
        provider_cache_probe_with_retries(&registry, case.provider_id)
            .await
            .unwrap_or_else(|err| panic!("cache probe failed for {}: {err}", case.provider_id));

    assert_cache_probe_response_correct(case.provider_id, &first);
    assert_cache_probe_response_correct(case.provider_id, &second);

    let first_usage = first
        .usage
        .as_ref()
        .unwrap_or_else(|| panic!("{} cache warm-up should carry usage", case.provider_id));
    let second_usage = second
        .usage
        .as_ref()
        .unwrap_or_else(|| panic!("{} cache replay should carry usage", case.provider_id));

    assert_eq!(
        first_usage.cache_read_tokens, 0,
        "{} first request should start cold for a unique nonce",
        case.provider_id
    );
    assert!(
        second_usage.cache_read_tokens > 0,
        "{} second request should observe cache reads after {} attempts; usage={second_usage:?}",
        case.provider_id,
        attempts_used
    );
}

async fn assert_session_cache_hit(case: LiveProviderCase) {
    let workspace = tempfile::tempdir().expect("create workspace tempdir");
    let db_path = workspace
        .path()
        .join(format!("{}-session-cache.db", case.slug));
    let db = open_live_database(&db_path).await;
    let manager = build_live_session_manager(workspace.path(), db).await;

    let (first, second, attempts_used) = session_cache_probe_with_retries(&manager, case)
        .await
        .unwrap_or_else(|err| panic!("session cache probe failed for {}: {err}", case.provider_id));

    let first_assistant = assistant_message(&first);
    let second_assistant = assistant_message(&second);
    assert_message_has_reply_text(
        format!("{} session cache warm-up", case.provider_id).as_str(),
        first_assistant.as_text_lossy().as_str(),
    );
    assert_message_has_reply_text(
        format!("{} session cache replay", case.provider_id).as_str(),
        second_assistant.as_text_lossy().as_str(),
    );

    let first_cache_read = first_assistant
        .usage
        .as_ref()
        .map(|usage| usage.cache_read_tokens)
        .unwrap_or_default();
    let second_cache_read = second_assistant
        .usage
        .as_ref()
        .map(|usage| usage.cache_read_tokens)
        .unwrap_or_default();
    assert!(
        second_cache_read > 0,
        "{} second session turn should observe cache reads after {} attempts; first_cache_read={}, second_cache_read={}",
        case.provider_id,
        attempts_used,
        first_cache_read,
        second_cache_read
    );
}

async fn assert_session_lifecycle(case: LiveProviderCase) {
    let workspace = tempfile::tempdir().expect("create workspace tempdir");
    let db_path = workspace.path().join(format!("{}-lifecycle.db", case.slug));
    let first_db = open_live_database(&db_path).await;
    let first = build_live_session_manager(workspace.path(), first_db).await;

    let source = first
        .create_session(SessionCreateRequest {
            title: format!("cliproxy live {} lifecycle", case.provider_id),
            parent_session_id: None,
        })
        .await
        .expect("create lifecycle session");

    let first_turn = first
        .submit_user_turn(SessionUserTurnRequest {
            session_id: source.id,
            options: run_options(case, "Reply briefly and clearly.", 64),
            parts: vec![PartContent::text("hi from lifecycle turn one")],
        })
        .await
        .unwrap_or_else(|err| {
            panic!(
                "first lifecycle turn failed for {}: {err}",
                case.provider_id
            )
        });
    assert_session_has_nonempty_assistant_reply(
        format!("{} lifecycle first turn", case.provider_id).as_str(),
        &first_turn,
    );

    let first_user_message_id = user_message_ids(&first_turn)
        .into_iter()
        .last()
        .expect("first lifecycle turn should append a user message");

    let second_turn = first
        .submit_user_turn(SessionUserTurnRequest {
            session_id: source.id,
            options: run_options(case, "Reply briefly and clearly.", 64),
            parts: vec![PartContent::text("this is lifecycle turn two")],
        })
        .await
        .unwrap_or_else(|err| {
            panic!(
                "second lifecycle turn failed for {}: {err}",
                case.provider_id
            )
        });
    assert_session_has_nonempty_assistant_reply(
        format!("{} lifecycle second turn", case.provider_id).as_str(),
        &second_turn,
    );

    let source_after_two = first
        .get_session(source.id)
        .await
        .expect("reload lifecycle source after two turns");
    assert!(
        source_after_two
            .messages
            .iter()
            .any(|message| message.role == Role::User
                && message.as_text_lossy().contains("lifecycle turn two")),
        "{} source session should retain second turn before rewind",
        case.provider_id
    );

    let forked = first
        .fork_session(SessionForkRequest {
            session_id: source.id,
            at_message_id: Some(first_user_message_id),
            title: Some(format!("forked {}", case.provider_id)),
            expected_version: None,
        })
        .await
        .unwrap_or_else(|err| {
            panic!(
                "fork lifecycle session failed for {}: {err}",
                case.provider_id
            )
        });
    assert_eq!(forked.parent_id, Some(source.id));
    assert_eq!(forked.root_id, source.root_id);
    assert!(
        forked
            .messages
            .iter()
            .any(|message| message.role == Role::User
                && message.as_text_lossy().contains("turn one")),
        "{} forked session should retain turn one",
        case.provider_id
    );
    assert!(
        !forked
            .messages
            .iter()
            .any(|message| message.role == Role::User
                && message.as_text_lossy().contains("turn two")),
        "{} forked session should exclude turn two",
        case.provider_id
    );

    let tree = first
        .list_session_tree(source.root_id)
        .await
        .expect("list lifecycle session tree");
    assert!(
        tree.iter().any(|summary| summary.id == source.id),
        "{} source session should appear in session tree",
        case.provider_id
    );
    assert!(
        tree.iter().any(|summary| summary.id == forked.id),
        "{} forked session should appear in session tree",
        case.provider_id
    );

    let summaries = first
        .list_session_summaries(SessionListRequest::default())
        .await
        .expect("list lifecycle session summaries");
    assert!(
        summaries.iter().any(|summary| summary.id == source.id),
        "{} source session should appear in summaries",
        case.provider_id
    );
    assert!(
        summaries.iter().any(|summary| summary.id == forked.id),
        "{} forked session should appear in summaries",
        case.provider_id
    );

    let rewound = first
        .rewind_session(SessionRewindRequest {
            session_id: source.id,
            message_id: first_user_message_id,
            expected_version: None,
        })
        .await
        .unwrap_or_else(|err| {
            panic!(
                "rewind lifecycle session failed for {}: {err}",
                case.provider_id
            )
        });
    assert!(
        !rewound
            .messages
            .iter()
            .any(|message| message.role == Role::User
                && message.as_text_lossy().contains("turn two")),
        "{} rewound session should hide turn two",
        case.provider_id
    );

    let checkpoints = first
        .list_rewind_checkpoints(source.id)
        .await
        .expect("list rewind checkpoints");
    let checkpoint = checkpoints
        .last()
        .unwrap_or_else(|| panic!("{} rewind should record a checkpoint", case.provider_id));
    assert_eq!(checkpoint.target_message_id, first_user_message_id);
    assert!(
        checkpoint
            .dropped
            .iter()
            .any(|entry| entry.preview.contains("turn two")),
        "{} rewind checkpoint should include the dropped second turn",
        case.provider_id
    );

    let unrewound = first
        .unrewind_session(SessionUnrewindRequest {
            session_id: source.id,
            message_id: first_user_message_id,
            expected_version: None,
        })
        .await
        .unwrap_or_else(|err| {
            panic!(
                "unrewind lifecycle session failed for {}: {err}",
                case.provider_id
            )
        });
    assert!(
        unrewound
            .messages
            .iter()
            .any(|message| message.role == Role::User
                && message.as_text_lossy().contains("turn two")),
        "{} unrewound session should restore turn two",
        case.provider_id
    );

    drop(first);

    let second_db = open_live_database(&db_path).await;
    let second = build_live_session_manager(workspace.path(), second_db).await;
    second
        .event_publisher()
        .resume_from_store()
        .await
        .expect("resume live event publisher from store");

    let reloaded = second.get_session(source.id).await.unwrap_or_else(|err| {
        panic!(
            "reload lifecycle session after resume failed for {}: {err}",
            case.provider_id
        )
    });
    assert!(
        reloaded
            .messages
            .iter()
            .any(|message| message.role == Role::User
                && message.as_text_lossy().contains("turn two")),
        "{} resumed session should still contain restored turn two",
        case.provider_id
    );

    let continued = second
        .continue_session(SessionContinueRequest {
            session_id: source.id,
            options: run_options(case, "Reply briefly and clearly.", 64),
        })
        .await
        .unwrap_or_else(|err| {
            panic!(
                "continue lifecycle session failed for {}: {err}",
                case.provider_id
            )
        });
    assert_session_has_nonempty_assistant_reply(
        format!("{} lifecycle continue after resume", case.provider_id).as_str(),
        &continued,
    );
}

macro_rules! live_provider_tests {
    ($provider_id:ident, $slug:literal) => {
        mod $provider_id {
            use super::*;

            const CASE: LiveProviderCase = LiveProviderCase::new(stringify!($provider_id), $slug);

            #[tokio::test]
            #[ignore = "real integration test against deployed CLIProxyAPI"]
            async fn single_hi_completion() {
                assert_registry_single_hi(CASE).await;
            }

            #[tokio::test]
            #[ignore = "real integration test against deployed CLIProxyAPI"]
            async fn single_hi_stream() {
                assert_registry_stream_hi(CASE).await;
            }

            #[tokio::test]
            #[ignore = "real integration test against deployed CLIProxyAPI"]
            async fn tool_call_roundtrip() {
                assert_registry_tool_call(CASE).await;
            }

            #[tokio::test]
            #[ignore = "real integration test against deployed CLIProxyAPI"]
            async fn session_single_hi() {
                assert_session_single_hi(CASE).await;
            }

            #[tokio::test]
            #[ignore = "real integration test against deployed CLIProxyAPI"]
            async fn session_multi_turn() {
                assert_session_multi_turn(CASE).await;
            }

            #[tokio::test]
            #[ignore = "real integration test against deployed CLIProxyAPI"]
            async fn registry_cache_hit() {
                assert_registry_cache_hit(CASE).await;
            }

            #[tokio::test]
            #[ignore = "real integration test against deployed CLIProxyAPI"]
            async fn session_cache_hit() {
                assert_session_cache_hit(CASE).await;
            }

            #[tokio::test]
            #[ignore = "real integration test against deployed CLIProxyAPI"]
            async fn session_lifecycle_fork_rewind_resume() {
                assert_session_lifecycle(CASE).await;
            }
        }
    };
}

live_provider_tests!(openai_live, "openai");
live_provider_tests!(compat_live, "compat");
live_provider_tests!(anthropic_live, "anthropic");
live_provider_tests!(gemini_live, "gemini");

#[tokio::test]
#[ignore = "real integration test against deployed CLIProxyAPI"]
async fn cliproxy_live_registry_lists_models_for_all_protocols() {
    let registry = load_live_registry();

    for case in PROVIDERS {
        let models = registry
            .list_models(case.provider_id)
            .await
            .unwrap_or_else(|err| panic!("list_models failed for {}: {err}", case.provider_id));
        assert!(
            models.iter().any(|model| model.id.as_str() == LIVE_MODEL),
            "{} model list should include {}, got {:?}",
            case.provider_id,
            LIVE_MODEL,
            models
                .iter()
                .map(|model| model.id.as_str().to_owned())
                .collect::<Vec<_>>()
        );
    }
}

#[test]
fn cliproxy_live_test_config_uses_real_proxy_endpoint() {
    let auth_dir = tempfile::tempdir().expect("create auth tempdir");
    let auth_path = auth_dir.path().join("auth.json");
    let config = live_config_text(&auth_path);
    let file = write_temp_config(config.as_str());
    let loader = ConfigLoader::new(placeholder_test_env());
    let resolution = loader
        .load(&LoadConfigRequest {
            config_path: Some(file.path().to_path_buf()),
            overrides: Vec::new(),
        })
        .expect("load config");

    let provider = resolution
        .config
        .providers
        .get("openai_live")
        .expect("provider should exist");
    let serialized = serde_json::to_string(provider).expect("serialize provider config");
    assert!(
        serialized.contains("/api/provider/openai/v1"),
        "config should point to live proxy endpoint: {serialized}"
    );
}
