// Exhaustive real-provider regression suite for the Cline dsv4f gateway.
//
// The suite intentionally uses the public session/model path.  Each real
// plugin target is discovered with `tools_help` and then invoked with
// `tools_call` by the configured Cline model; it never bypasses the gateway
// by calling plugin implementations directly.

use std::{
    collections::BTreeMap,
    env as std_env, fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use agena::{
    agent::{
        NetworkPermissionConfig, PathAccessModes, PathPermissionConfig, PermissionConfig,
        ToolPermissionConfig,
    },
    config::LoadConfigRequest,
    event::{EventFilter, EventKind, Scope, Subscription, bus::SubscriptionItem},
    message::{
        ExecutionStatus, OperationPart, PartContent, PendingInteractiveRequest, UserInputReply,
        UserInputReplyKind,
    },
    model::ModelRef,
    permission::{PermissionMode, PermissionReply, PermissionReplyKind},
    runtime::{AgenaRuntime, AgenaRuntimeConfig},
    session::{
        Session, SessionCreateRequest, SessionExecutionReplyRequest, SessionManager,
        SessionPermissionReplyRequest, SessionRunOptions, SessionUserMessageRequest,
    },
    tool,
};
use anyhow::{Context, bail, ensure};
use clap::Parser;
use serde_json::{Value, json};
use tempfile::TempDir;

const DEFAULT_MODEL: &str = "cline/cline-pass/deepseek-v4-flash";
const GATEWAY_HELP: &str = "agena.tools.help";
const GATEWAY_CALL: &str = "agena.tools.call";
const GATEWAY_LIST: &str = "agena.tools.list";
const GATEWAY_SEARCH: &str = "agena.tools.search";
const GATEWAY_TAGS: &str = "agena.tools.tags";
const MAX_EXACT_INVOCATION_ATTEMPTS: usize = 3;

#[derive(Debug, Parser)]
#[command(about = "Exercise every plugin tool through a real Cline dsv4f gateway session")]
struct Args {
    /// Provider/model reference accepted by `agena exec`.
    #[arg(long, default_value = DEFAULT_MODEL)]
    model: String,

    /// Repository root used to find compiled external-plugin fixture artifacts.
    #[arg(long, default_value = ".")]
    repo_root: PathBuf,

    /// Home config to copy into the isolated test HOME. Defaults to $HOME/agena/agena.json.
    #[arg(long)]
    config_source: Option<PathBuf>,

    /// Run only named cases or groups (for example `fs.read` or `web`). Repeatable.
    #[arg(long = "case", value_name = "NAME")]
    cases: Vec<String>,

    /// Maximum duration of one ordinary model turn.
    #[arg(long, default_value_t = 180)]
    case_timeout_secs: u64,

    /// Maximum end-to-end duration of a tasks.run parent gateway turn and child run.
    #[arg(long, default_value_t = 600)]
    task_timeout_secs: u64,

    /// Keep the isolated HOME/workspace fixture for postmortem inspection.
    #[arg(long)]
    keep_fixture: bool,

    /// Optional JSON summary path written only after every selected case passes.
    #[arg(long)]
    report_path: Option<PathBuf>,
}

fn main() -> anyhow::Result<()> {
    // The application helper deliberately gives the model/session stack a
    // larger worker stack than Tokio's default. Complex provider callbacks can
    // otherwise overflow a default test-runtime worker stack.
    agena::runtime::build_app_runtime()
        .context("build dsv4f suite Tokio runtime")?
        .block_on(async_main())
}

async fn async_main() -> anyhow::Result<()> {
    let args = Args::parse();
    let original_home = std_env::var_os("HOME")
        .map(PathBuf::from)
        .context("HOME is required to locate the configured Cline credentials")?;
    let repo_root = args
        .repo_root
        .canonicalize()
        .with_context(|| format!("resolve repository root {}", args.repo_root.display()))?;
    let config_source = args
        .config_source
        .clone()
        .unwrap_or_else(|| original_home.join("agena/agena.json"));
    ensure!(
        config_source.is_file(),
        "Cline configuration source does not exist: {}",
        config_source.display()
    );
    let model = args
        .model
        .parse::<ModelRef>()
        .with_context(|| format!("parse model reference `{}`", args.model))?;

    let fixture = Fixture::create(&repo_root, &config_source, args.keep_fixture)?;
    configure_isolated_environment(&fixture)?;

    let runtime = AgenaRuntime::new(AgenaRuntimeConfig {
        load_request: LoadConfigRequest {
            overrides: Vec::new(),
            workspace_root: Some(fixture.workspace.clone()),
        },
        workspace_root: Some(fixture.workspace.clone()),
        database_connection: None,
        database_url: Some("sqlite::memory:".to_string()),
        auto_migrate: true,
        tracing_reload_handle: None,
    })
    .await
    .context("start isolated dsv4f suite runtime")?;
    let manager = runtime
        .session_manager()
        .context("runtime does not provide a session manager")?;

    assert_gateway_surface(manager.as_ref())?;
    let harness = Harness {
        manager,
        options: run_options(model),
        case_timeout: Duration::from_secs(args.case_timeout_secs),
        task_timeout: Duration::from_secs(args.task_timeout_secs),
        selector: CaseSelector::new(args.cases),
    };
    let mut report = SuiteReport::default();

    run_gateway_meta_suite(&harness, &mut report).await?;
    run_builtin_suite(&harness, &fixture, &mut report).await?;
    run_external_plugin_suite(&harness, &fixture, &mut report).await?;
    run_nested_permission_suite(&harness, &fixture, &mut report).await?;

    runtime.shutdown();
    let output = json!({
        "ok": true,
        "provider": DEFAULT_MODEL,
        "passed": report.passed,
        "count": report.passed.len(),
        "fixture_kept": fixture.keep,
        "fixture_root": fixture.keep.then(|| fixture.root.display().to_string()),
    });
    let output = serde_json::to_string(&output)?;
    if let Some(path) = args.report_path {
        fs::write(&path, &output)
            .with_context(|| format!("write suite report {}", path.display()))?;
    }
    println!("{output}");
    Ok(())
}

fn run_options(model: ModelRef) -> SessionRunOptions {
    let mut request_override = agena::model::ModelSpeedModeRequestOverride::default();
    // The suite exercises one gateway operation at a time. Explicitly disable
    // parallel native calls so a provider cannot fan out duplicate retries for
    // a single strict probe instruction.
    request_override.set_parallel_tool_calls(Some(false));
    SessionRunOptions {
        model,
        thinking_mode: None,
        speed_mode: None,
        verbosity: None,
        thinking: None,
        request_override,
        system: Some(
            "You are a deterministic integration-test driver. When a user gives an exact native tool invocation JSON, copy every key and value exactly, including optional fields. Never make a preliminary/default tool call, never omit supplied fields, and never retry a completed tool call unless the user explicitly asks for a retry."
                .to_string(),
        ),
        temperature: Some(0.0),
        max_output_tokens: Some(1_024),
        agent_profile: None,
    }
}

fn assert_gateway_surface(manager: &SessionManager) -> anyhow::Result<()> {
    let specs = tool::model_tool_specs(&manager.tool_executor().available_model_tools());
    let mut names = specs
        .into_iter()
        .map(|spec| spec.model_name)
        .collect::<Vec<_>>();
    names.sort();
    let expected = [
        "tools_call",
        "tools_help",
        "tools_list",
        "tools_search",
        "tools_tags",
    ]
    .into_iter()
    .map(str::to_string)
    .collect::<Vec<_>>();
    ensure!(
        names == expected,
        "Cline model surface must contain only the five gateway functions, found {names:?}"
    );
    Ok(())
}

/// The suite changes process-wide variables exactly once before the Tokio
/// runtime is constructed and before any child/plugin task is spawned.
fn configure_isolated_environment(fixture: &Fixture) -> anyhow::Result<()> {
    fs::create_dir_all(&fixture.plugin_storage)
        .with_context(|| format!("create {}", fixture.plugin_storage.display()))?;
    // SAFETY: this function runs in `async_main` before creating AgenaRuntime,
    // its Tokio worker threads, or any plugin child process. No concurrent
    // environment access exists yet, and the values remain fixed thereafter.
    unsafe {
        std_env::set_var("HOME", &fixture.home);
        std_env::set_var("AGENA_PLUGIN_STORAGE_DIR", &fixture.plugin_storage);
        std_env::set_var("AGENA_RIFT_BIN", fixture.root.join("missing-rift"));
    }
    Ok(())
}

#[derive(Default)]
struct SuiteReport {
    passed: Vec<String>,
}

impl SuiteReport {
    fn pass(&mut self, name: impl Into<String>) {
        self.passed.push(name.into());
    }
}

#[derive(Debug, Clone)]
struct CaseSelector {
    requested: Vec<String>,
}

impl CaseSelector {
    fn new(requested: Vec<String>) -> Self {
        Self {
            requested: requested
                .into_iter()
                .map(|name| name.trim().to_ascii_lowercase())
                .filter(|name| !name.is_empty())
                .collect(),
        }
    }

    fn enabled(&self, case: &str) -> bool {
        if self.requested.is_empty() || self.requested.iter().any(|name| name == "all") {
            return true;
        }
        let case = case.to_ascii_lowercase();
        self.requested.iter().any(|requested| {
            requested == &case
                || case
                    .strip_prefix(requested.as_str())
                    .is_some_and(|suffix| suffix.starts_with('.'))
        })
    }

    fn any_in_group(&self, group: &str) -> bool {
        self.enabled(group)
            || self
                .requested
                .iter()
                .any(|requested| requested.starts_with(&format!("{group}.")))
    }
}

struct Harness {
    manager: Arc<SessionManager>,
    options: SessionRunOptions,
    case_timeout: Duration,
    task_timeout: Duration,
    selector: CaseSelector,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingReply {
    None,
    Input,
    Permission(PermissionReplyKind),
}

#[derive(Clone)]
struct GatewayOutcome {
    session: Session,
    call: OperationPart,
}

impl GatewayOutcome {
    fn payload(&self) -> Value {
        self.call
            .result
            .structured
            .clone()
            .or_else(|| self.call.structured.clone())
            .unwrap_or(Value::Null)
    }

    fn visible_text(&self) -> String {
        let mut text = self.call.output_text().unwrap_or_default().to_string();
        let payload = self.payload();
        if !payload.is_null() {
            text.push('\n');
            text.push_str(&payload.to_string());
        }
        text
    }
}

impl Harness {
    async fn create_session(
        &self,
        title: &str,
        target_names: &[&str],
        permission: PermissionConfig,
    ) -> anyhow::Result<i64> {
        let session = self
            .manager
            .create_session(SessionCreateRequest {
                title: title.to_string(),
                parent_session_id: None,
            })
            .await
            .with_context(|| format!("create session for {title}"))?;
        let mut allowed = vec![GATEWAY_HELP.to_string(), GATEWAY_CALL.to_string()];
        allowed.extend(target_names.iter().map(|name| (*name).to_string()));
        allowed.sort();
        allowed.dedup();
        self.manager
            .set_session_allowed_tools(session.id, allowed)
            .await
            .with_context(|| format!("allow gateway + target tools for {title}"))?;
        self.manager
            .set_session_permission(session.id, permission)
            .await
            .with_context(|| format!("set baseline permission for {title}"))?;
        Ok(session.id)
    }

    async fn run_gateway_target(
        &self,
        session_id: i64,
        case: &str,
        target: &str,
        input: Value,
        pending_reply: PendingReply,
        expect_success: bool,
    ) -> anyhow::Result<GatewayOutcome> {
        self.run_gateway_target_with_timeout(
            session_id,
            case,
            target,
            input,
            pending_reply,
            expect_success,
            self.case_timeout,
        )
        .await
    }

    /// Run a target that implements the plugin streaming protocol and prove
    /// that its chunks become a live update on the model-visible outer
    /// `agena.tools.call` operation. The subscription is installed before
    /// the provider turn begins so this exercises the complete real-provider
    /// gateway path rather than inspecting only the final persisted result.
    async fn run_gateway_streaming_target(
        &self,
        session_id: i64,
        case: &str,
        target: &str,
        input: Value,
        expected_text: &str,
    ) -> anyhow::Result<GatewayOutcome> {
        let mut subscription = self
            .manager
            .event_bus()
            .subscribe(EventFilter::new(Scope::Session { session_id }));
        let outcome = self
            .run_gateway_target(
                session_id,
                case,
                target,
                input.clone(),
                PendingReply::None,
                true,
            )
            .await?;
        assert_outer_gateway_stream_update(&mut subscription, target, &input, expected_text)
            .await?;
        Ok(outcome)
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_gateway_target_with_timeout(
        &self,
        session_id: i64,
        case: &str,
        target: &str,
        input: Value,
        pending_reply: PendingReply,
        expect_success: bool,
        timeout: Duration,
    ) -> anyhow::Result<GatewayOutcome> {
        let marker = format!("DSV4F_{}_OK", marker_for(case));
        let input_text = serde_json::to_string(&input)?;
        let outcome_instruction = if expect_success {
            "The requested invocation must succeed."
        } else {
            "The requested invocation is intentionally expected to fail after the test denies its nested permission. Once it fails, do not retry the call and do not request another permission; reply with the terminal marker."
        };
        for attempt in 1..=MAX_EXACT_INVOCATION_ATTEMPTS {
            let start_message_count = self.manager.get_session(session_id).await?.messages.len();
            let retry_notice = (attempt > 1).then_some(
                "A prior attempt omitted the exact required input and did not execute it. Correct that now; do not reuse an empty or partial object.",
            );
            let prompt = format!(
                "This is an automated gateway integration test. Call the native function tools_help exactly once with {{\"tool\":{target:?}}}. Then call the native function tools_call exactly once with this exact JSON object: {{\"tool\":{target:?},\"input\":{input_text}}}. Every supplied key is mandatory even if the schema marks it optional. A preliminary/default call, an empty input object, a modified value, or any second call is a test failure. Do not call any other function. {outcome_instruction} {} After that one tool result, reply exactly {marker}.",
                retry_notice.unwrap_or_default(),
            );
            let session = self
                .run_model_turn(session_id, prompt, pending_reply, timeout)
                .await
                .with_context(|| format!("run model turn for {case} (attempt {attempt})"))?;
            match extract_gateway_outcome(
                &session,
                start_message_count,
                target,
                &input,
                &marker,
                expect_success,
            ) {
                Ok(outcome) => return Ok(outcome),
                Err(error)
                    if attempt < MAX_EXACT_INVOCATION_ATTEMPTS
                        && can_retry_missing_gateway_invocation(
                            &session,
                            start_message_count,
                            target,
                            &input,
                        ) =>
                {
                    let _ = error;
                }
                Err(error) => {
                    return Err(error).with_context(|| format!("verify gateway trace for {case}"));
                }
            }
        }
        unreachable!("the exact-invocation retry loop either returns or errors")
    }

    async fn run_native_gateway_function(
        &self,
        session_id: i64,
        case: &str,
        model_function: &str,
        canonical_function: &str,
        input: Value,
        target_marker: Option<&str>,
    ) -> anyhow::Result<GatewayOutcome> {
        let marker = format!("DSV4F_{}_OK", marker_for(case));
        let input_text = serde_json::to_string(&input)?;
        for attempt in 1..=MAX_EXACT_INVOCATION_ATTEMPTS {
            let start_message_count = self.manager.get_session(session_id).await?.messages.len();
            let retry_notice = (attempt > 1).then_some(
                "A prior attempt did not execute the exact supplied JSON. Correct that now and do not use defaults or partial arguments.",
            );
            let prompt = format!(
                "This is an automated native gateway test. Call the native function {model_function} exactly once with {input_text}. Do not call any other function. {} After the function result, reply exactly {marker}.",
                retry_notice.unwrap_or_default(),
            );
            let session = self
                .run_model_turn(session_id, prompt, PendingReply::None, self.case_timeout)
                .await
                .with_context(|| format!("run model turn for {case} (attempt {attempt})"))?;
            match extract_native_outcome(
                &session,
                start_message_count,
                canonical_function,
                &input,
                &marker,
                target_marker,
            ) {
                Ok(outcome) => return Ok(outcome),
                Err(error)
                    if attempt < MAX_EXACT_INVOCATION_ATTEMPTS
                        && can_retry_missing_native_invocation(
                            &session,
                            start_message_count,
                            canonical_function,
                            &input,
                        ) =>
                {
                    let _ = error;
                }
                Err(error) => {
                    return Err(error)
                        .with_context(|| format!("verify native gateway trace for {case}"));
                }
            }
        }
        unreachable!("the exact-invocation retry loop either returns or errors")
    }

    async fn run_model_turn(
        &self,
        session_id: i64,
        prompt: String,
        pending_reply: PendingReply,
        timeout: Duration,
    ) -> anyhow::Result<Session> {
        let manager = Arc::clone(&self.manager);
        let options = self.options.clone();
        let mut run = tokio::spawn(async move {
            manager
                .submit_user_message(SessionUserMessageRequest::new(
                    session_id,
                    options,
                    vec![PartContent::text(prompt)],
                ))
                .await
        });
        let deadline = Instant::now() + timeout;
        let mut replied = false;
        loop {
            if run.is_finished() {
                break;
            }
            if Instant::now() >= deadline {
                run.abort();
                bail!("model turn exceeded {} seconds", timeout.as_secs());
            }
            let session = self.manager.get_session(session_id).await?;
            let pending = session.pending_interactive_requests().into_iter().next();
            if let Some(pending) = pending {
                if replied {
                    run.abort();
                    bail!("model turn requested more than one interactive reply: {pending:?}");
                }
                match (pending_reply, pending) {
                    (PendingReply::Input, PendingInteractiveRequest::UserInput { request }) => {
                        ensure!(
                            request.request_id.starts_with("host-input:"),
                            "expected host-input request, got {}",
                            request.request_id
                        );
                        let answers = request
                            .questions
                            .iter()
                            .map(|question| (question.id.clone(), vec!["TEST_OK".to_string()]))
                            .collect::<BTreeMap<_, _>>();
                        self.manager
                            .reply_user_input(SessionExecutionReplyRequest::new(
                                session_id,
                                self.options.clone(),
                                UserInputReply {
                                    request_id: request.request_id,
                                    kind: UserInputReplyKind::Submit,
                                    answers,
                                    reason: None,
                                },
                            ))
                            .await
                            .context("reply to host user-input request")?;
                        replied = true;
                    }
                    (
                        PendingReply::Permission(kind),
                        PendingInteractiveRequest::Permission { request },
                    ) => {
                        ensure!(
                            request.request_id.starts_with("host-permission:"),
                            "expected nested host permission request, got {}",
                            request.request_id
                        );
                        self.manager
                            .reply_permission(SessionPermissionReplyRequest::new(
                                session_id,
                                self.options.clone(),
                                PermissionReply {
                                    request_id: request.request_id,
                                    kind,
                                    reason: Some("dsv4f exhaustive gateway suite".to_string()),
                                    scope: None,
                                },
                                Some("dsv4f_gateway_suite".to_string()),
                            ))
                            .await
                            .context("reply to nested host permission request")?;
                        replied = true;
                    }
                    (PendingReply::None, pending) => {
                        run.abort();
                        bail!("unexpected interactive request in model turn: {pending:?}");
                    }
                    (_, pending) => {
                        run.abort();
                        bail!("unexpected interactive request type: {pending:?}");
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        if !matches!(pending_reply, PendingReply::None) && !replied {
            bail!("expected an interactive reply, but the model turn completed without one");
        }
        tokio::time::timeout(deadline.saturating_duration_since(Instant::now()), &mut run)
            .await
            .context("model run join exceeded deadline")?
            .context("join model task")?
            .context("complete model run")
    }
}

fn baseline_permission(loopback: PermissionMode) -> PermissionConfig {
    PermissionConfig {
        path: Some(PathPermissionConfig {
            workspace: Some(PathAccessModes {
                read: Some(PermissionMode::Allow),
                write: Some(PermissionMode::Allow),
            }),
            external: Some(PathAccessModes {
                read: Some(PermissionMode::Allow),
                write: Some(PermissionMode::Allow),
            }),
            ..Default::default()
        }),
        network: Some(NetworkPermissionConfig {
            internet: Some(PermissionMode::Allow),
            private: Some(PermissionMode::Allow),
            loopback: Some(loopback),
            ..Default::default()
        }),
        tools: Some(ToolPermissionConfig {
            default: Some(PermissionMode::Allow),
            ..Default::default()
        }),
    }
}

fn marker_for(case: &str) -> String {
    case.chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect()
}

fn operations_since(session: &Session, start_message_count: usize) -> Vec<OperationPart> {
    session
        .messages
        .iter()
        .skip(start_message_count)
        .flat_map(|message| message.parts.iter())
        .filter_map(|part| match part.content.as_ref() {
            Some(PartContent::Operation(operation)) => Some(operation.clone()),
            _ => None,
        })
        .collect()
}

fn transcript_since(session: &Session, start_message_count: usize) -> String {
    session
        .messages
        .iter()
        .skip(start_message_count)
        .map(|message| message.as_text_lossy())
        .collect::<Vec<_>>()
        .join("\n")
}

/// A real provider can occasionally emit only malformed/default function
/// arguments despite an exact prompt. Retrying is safe only if it never
/// reached the requested invocation *and* none of its attempted gateway calls
/// completed, so a stateful target cannot be executed twice by the harness.
fn can_retry_missing_gateway_invocation(
    session: &Session,
    start_message_count: usize,
    target: &str,
    input: &Value,
) -> bool {
    if session.blocked() {
        return false;
    }
    let expected = json!({"tool": target, "input": input});
    let calls = operations_since(session, start_message_count)
        .into_iter()
        .filter(|operation| operation.invocation.name == GATEWAY_CALL)
        .collect::<Vec<_>>();
    !calls
        .iter()
        .any(|call| Value::from(call.invocation.input.clone()) == expected)
        && calls.iter().all(operation_failed_or_incomplete)
}

/// Same retry rule for the five native gateway functions.
fn can_retry_missing_native_invocation(
    session: &Session,
    start_message_count: usize,
    canonical_function: &str,
    input: &Value,
) -> bool {
    if session.blocked() {
        return false;
    }
    let calls = operations_since(session, start_message_count)
        .into_iter()
        .filter(|operation| operation.invocation.name == canonical_function)
        .collect::<Vec<_>>();
    !calls
        .iter()
        .any(|call| Value::from(call.invocation.input.clone()) == *input)
        && calls.iter().all(operation_failed_or_incomplete)
}

fn operation_failed_or_incomplete(operation: &OperationPart) -> bool {
    operation.status() != ExecutionStatus::Completed || operation.error_message().is_some()
}

fn extract_gateway_outcome(
    session: &Session,
    start_message_count: usize,
    target: &str,
    input: &Value,
    marker: &str,
    expect_success: bool,
) -> anyhow::Result<GatewayOutcome> {
    ensure!(!session.blocked(), "session remained blocked");
    let operations = operations_since(session, start_message_count);
    let helped = operations
        .iter()
        .filter(|operation| operation.invocation.name == GATEWAY_HELP)
        .collect::<Vec<_>>();
    ensure!(
        !helped.is_empty(),
        "model did not call tools.help; operations: {}",
        operation_trace_with_ids(session, start_message_count)
    );
    ensure!(
        helped.iter().all(|help| {
            serde_json::Value::from(help.invocation.input.clone()) == json!({"tool": target})
        }),
        "tools.help input did not target {target}; operations: {}",
        operation_trace_with_ids(session, start_message_count)
    );
    // Some Cline responses repeat the same read-only discovery call after a
    // rejected malformed target invocation. That is provider behavior, not a
    // second target execution: require every discovery call to be exact and
    // require exactly one exact target invocation below.
    ensure!(
        helped.len() <= 16,
        "excessive tools.help retries for {target} ({}); operations: {}",
        helped.len(),
        operation_trace_with_ids(session, start_message_count)
    );
    let calls = operations
        .iter()
        .filter(|operation| operation.invocation.name == GATEWAY_CALL)
        .collect::<Vec<_>>();
    ensure!(
        !calls.is_empty(),
        "model did not call tools.call; operations: {}",
        operation_trace_with_ids(session, start_message_count)
    );
    ensure!(
        calls.iter().all(|call| {
            Value::from(call.invocation.input.clone())
                .get("tool")
                .and_then(Value::as_str)
                == Some(target)
        }),
        "model invoked a target other than {target}; operations: {}",
        operation_trace_with_ids(session, start_message_count)
    );
    let expected_call_input = json!({"tool": target, "input": input});
    let matching_calls = calls
        .into_iter()
        .filter(|call| Value::from(call.invocation.input.clone()) == expected_call_input)
        .collect::<Vec<_>>();
    ensure!(
        matching_calls.len() == 1,
        "expected exactly one tools.call with the requested target/input, found {}; operations: {}",
        matching_calls.len(),
        operation_trace_with_ids(session, start_message_count)
    );
    let call = matching_calls[0].clone();
    ensure!(
        serde_json::Value::from(call.invocation.input.clone()) == expected_call_input,
        "tools.call input was not the supplied exact target/input"
    );
    if expect_success {
        ensure!(
            call.status() == ExecutionStatus::Completed && call.error_message().is_none(),
            "tools.call failed: {}",
            call.error_message().unwrap_or("unknown failure")
        );
    } else {
        ensure!(
            call.status() == ExecutionStatus::Failed || call.error_message().is_some(),
            "tools.call unexpectedly succeeded while failure was expected"
        );
    }
    let terminal_text = transcript_since(session, start_message_count);
    if expect_success {
        ensure!(
            terminal_text.contains(marker),
            "model did not emit terminal marker {marker}: {terminal_text}"
        );
    }
    Ok(GatewayOutcome {
        session: session.clone(),
        call,
    })
}

fn extract_native_outcome(
    session: &Session,
    start_message_count: usize,
    canonical_function: &str,
    input: &Value,
    marker: &str,
    target_marker: Option<&str>,
) -> anyhow::Result<GatewayOutcome> {
    ensure!(!session.blocked(), "session remained blocked");
    let matching = operations_since(session, start_message_count)
        .into_iter()
        .filter(|operation| operation.invocation.name == canonical_function)
        .collect::<Vec<_>>();
    ensure!(
        matching.len() == 1,
        "expected exactly one {canonical_function}, found {}; operations: {}",
        matching.len(),
        operation_trace_with_ids(session, start_message_count)
    );
    let call = matching[0].clone();
    ensure!(
        serde_json::Value::from(call.invocation.input.clone()) == *input,
        "{canonical_function} input differed from the supplied exact input"
    );
    ensure!(
        call.status() == ExecutionStatus::Completed && call.error_message().is_none(),
        "{canonical_function} failed: {}",
        call.error_message().unwrap_or("unknown failure")
    );
    let terminal_text = transcript_since(session, start_message_count);
    ensure!(
        terminal_text.contains(marker),
        "model did not emit terminal marker {marker}: {terminal_text}"
    );
    let outcome = GatewayOutcome {
        session: session.clone(),
        call,
    };
    if let Some(expected) = target_marker {
        ensure!(
            outcome.visible_text().contains(expected),
            "{canonical_function} result did not contain {expected}: {}",
            outcome.visible_text()
        );
    }
    Ok(outcome)
}

fn operation_trace_with_ids(session: &Session, start_message_count: usize) -> String {
    session
        .messages
        .iter()
        .skip(start_message_count)
        .flat_map(|message| message.parts.iter())
        .filter_map(|part| match part.content.as_ref() {
            Some(PartContent::Operation(operation)) => {
                Some((part.operation_id.as_deref(), operation))
            }
            _ => None,
        })
        .map(|(operation_id, operation)| {
            let input = Value::from(operation.invocation.input.clone());
            format!(
                "{}({input}) operation_id={} status={:?} error={}",
                operation.invocation.name,
                operation_id.unwrap_or("<none>"),
                operation.status(),
                operation.error_message().unwrap_or("<none>")
            )
        })
        .collect::<Vec<_>>()
        .join("; ")
}

fn assert_contains(outcome: &GatewayOutcome, expected: &str) -> anyhow::Result<()> {
    ensure!(
        outcome.visible_text().contains(expected),
        "tool result did not contain {expected}: {}",
        outcome.visible_text()
    );
    Ok(())
}

/// The final operation result is insufficient evidence that a plugin's
/// streaming handler ran: the ordinary non-streaming handler may return the
/// same text. Require a live `MessagePartUpdated` snapshot where the outer
/// gateway operation is still in progress and contains a streamed chunk.
async fn assert_outer_gateway_stream_update(
    subscription: &mut Subscription<EventKind>,
    target: &str,
    input: &Value,
    expected_text: &str,
) -> anyhow::Result<()> {
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut observed_operations = Vec::new();
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            bail!(
                "did not receive an in-progress streamed update for outer {GATEWAY_CALL} targeting {target}; observed operation updates: {}",
                if observed_operations.is_empty() {
                    "<none>".to_string()
                } else {
                    observed_operations.join("; ")
                }
            );
        }
        let item = match tokio::time::timeout(remaining, subscription.recv()).await {
            Ok(item) => {
                item.context("session event bus closed while awaiting streamed gateway update")?
            }
            Err(_) => continue,
        };
        match item {
            SubscriptionItem::Lagged(count) => {
                bail!(
                    "session event subscription lagged by {count} event(s) while checking streamed gateway output"
                );
            }
            SubscriptionItem::Event(event) => {
                let EventKind::MessagePartUpdated(update) = &event.kind else {
                    continue;
                };
                let Some(PartContent::Operation(operation)) = update.part.content.as_ref() else {
                    continue;
                };
                if observed_operations.len() < 12 {
                    observed_operations.push(format!(
                        "name={} input={} status={:?} output={}",
                        operation.invocation.name,
                        Value::from(operation.invocation.input.clone()),
                        update.part.status,
                        operation.output_text().unwrap_or_default()
                    ));
                }
                if update.part.status != ExecutionStatus::InProgress {
                    continue;
                }
                if operation.invocation.name != GATEWAY_CALL
                    || Value::from(operation.invocation.input.clone())
                        != json!({"tool": target, "input": input})
                    || !operation
                        .output_text()
                        .is_some_and(|text| text.contains(expected_text))
                {
                    continue;
                }
                return Ok(());
            }
        }
    }
}

fn payload_string(value: &Value, field: &str) -> anyhow::Result<String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .with_context(|| format!("payload is missing string field `{field}`: {value}"))
}

struct Fixture {
    temp: Option<TempDir>,
    root: PathBuf,
    home: PathBuf,
    workspace: PathBuf,
    plugin_storage: PathBuf,
    web: LocalWebServer,
    keep: bool,
}

impl Fixture {
    fn create(repo_root: &Path, config_source: &Path, keep: bool) -> anyhow::Result<Self> {
        let temp = tempfile::Builder::new()
            .prefix("agena-dsv4f-suite-")
            .tempdir()?;
        let root = temp.path().to_path_buf();
        let home = root.join("home");
        let workspace = root.join("workspace");
        let plugin_storage = root.join("plugin-storage");
        fs::create_dir_all(home.join("agena"))?;
        fs::create_dir_all(workspace.join(".agena"))?;
        fs::create_dir_all(workspace.join("src"))?;
        fs::copy(config_source, home.join("agena/agena.json")).with_context(|| {
            format!(
                "copy configured Cline home config from {} into isolated HOME",
                config_source.display()
            )
        })?;

        fs::write(
            workspace.join("Cargo.toml"),
            "[package]\nname = \"dsv4f-fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[lib]\npath = \"src/lib.rs\"\n",
        )?;
        fs::write(
            workspace.join("src/lib.rs"),
            "pub fn probe() -> u32 { 7 }\n\npub fn use_probe() -> u32 { probe() }\n",
        )?;
        fs::write(workspace.join("README.md"), "DSV4F workspace fixture\n")?;
        initialize_git_fixture(&workspace)?;

        let web = LocalWebServer::start()?;
        write_project_config(repo_root, &workspace)?;
        Ok(Self {
            temp: Some(temp),
            root,
            home,
            workspace,
            plugin_storage,
            web,
            keep,
        })
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if self.keep {
            // Preserve the fixture by retaining ownership until process exit.
            if let Some(temp) = self.temp.take() {
                let _ = temp.keep();
            }
        }
    }
}

fn initialize_git_fixture(workspace: &Path) -> anyhow::Result<()> {
    for args in [
        ["init", "-q"].as_slice(),
        ["config", "user.email", "dsv4f@example.test"].as_slice(),
        ["config", "user.name", "DSV4F Fixture"].as_slice(),
        ["add", "."].as_slice(),
        ["commit", "--quiet", "-m", "initial fixture"].as_slice(),
    ] {
        let status = Command::new("git")
            .arg("-C")
            .arg(workspace)
            .args(args)
            .status()
            .with_context(|| format!("run git {:?}", args))?;
        ensure!(status.success(), "git {:?} failed", args);
    }
    Ok(())
}

fn commit_fixture_change(workspace: &Path, message: &str) -> anyhow::Result<()> {
    for args in [
        ["add", "."].as_slice(),
        ["commit", "--quiet", "-m", message].as_slice(),
    ] {
        let status = Command::new("git")
            .arg("-C")
            .arg(workspace)
            .args(args)
            .status()
            .with_context(|| format!("commit fixture change with git {:?}", args))?;
        ensure!(status.success(), "git {:?} failed", args);
    }
    Ok(())
}

fn artifact_path(repo_root: &Path, name: &str) -> PathBuf {
    repo_root.join("target/debug").join(name)
}

fn dynamic_library_name() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "libagena_echo_plugin.dylib"
    }
    #[cfg(target_os = "windows")]
    {
        "agena_echo_plugin.dll"
    }
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    {
        "libagena_echo_plugin.so"
    }
}

fn rust_analyzer_path() -> anyhow::Result<PathBuf> {
    let toolchains = PathBuf::from(
        std_env::var_os("RUSTUP_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                std_env::var_os("HOME")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| PathBuf::from("/nonexistent"))
                    .join(".rustup")
            })
            .join("toolchains"),
    );
    let mut candidates = fs::read_dir(&toolchains)
        .with_context(|| format!("read Rust toolchains under {}", toolchains.display()))?
        .filter_map(Result::ok)
        .map(|entry| entry.path().join("bin/rust-analyzer"))
        .filter(|path| path.is_file())
        .collect::<Vec<_>>();
    candidates.sort();
    candidates
        .pop()
        .context("no installed rust-analyzer binary was found in RUSTUP_HOME")
}

/// The suite isolates `HOME` so its model credentials, database, and plugin
/// state cannot leak into the developer's normal agena installation. Rust
/// Analyzer invokes the `cargo`/`rustc` rustup proxies, however, and those
/// proxies locate their selected toolchain through `RUSTUP_HOME` (which
/// otherwise follows the now-isolated `HOME`). Preserve just the host's Rust
/// toolchain roots for the fixture language-server process.
fn rust_toolchain_environment() -> anyhow::Result<BTreeMap<String, String>> {
    let host_home = std_env::var_os("HOME")
        .map(PathBuf::from)
        .context("HOME is required to configure rust-analyzer for the isolated fixture")?;
    Ok(rust_toolchain_environment_for(
        &host_home,
        std_env::var_os("RUSTUP_HOME").map(PathBuf::from),
        std_env::var_os("CARGO_HOME").map(PathBuf::from),
    ))
}

fn rust_toolchain_environment_for(
    host_home: &Path,
    rustup_home: Option<PathBuf>,
    cargo_home: Option<PathBuf>,
) -> BTreeMap<String, String> {
    let rustup_home = rustup_home.unwrap_or_else(|| host_home.join(".rustup"));
    let cargo_home = cargo_home.unwrap_or_else(|| host_home.join(".cargo"));
    BTreeMap::from([
        ("RUSTUP_HOME".to_string(), rustup_home.display().to_string()),
        ("CARGO_HOME".to_string(), cargo_home.display().to_string()),
    ])
}

fn write_project_config(repo_root: &Path, workspace: &Path) -> anyhow::Result<()> {
    let cdylib = artifact_path(repo_root, dynamic_library_name());
    let echo_stdio = artifact_path(repo_root, "agena-echo-plugin-stdio");
    let notes_stdio = artifact_path(repo_root, "agena-multi-tool-plugin-stdio");
    let mcp_fixture = artifact_path(repo_root, "dsv4f_mcp_fixture");
    for path in [&cdylib, &echo_stdio, &notes_stdio, &mcp_fixture] {
        ensure!(
            path.is_file(),
            "required fixture artifact is missing: {} (build the required agena-cli/example binaries first)",
            path.display()
        );
    }
    let rust_analyzer = rust_analyzer_path()?;
    let rust_analyzer_env = rust_toolchain_environment()?;
    let config = json!({
        "plugins": {
            "list": {
                "agena.lsp": {
                    "package": { "kind": "static" },
                    "config": {
                        "servers": {
                            "rust": {
                                "process": { "command": rust_analyzer, "args": [], "env": rust_analyzer_env },
                                "routing": { "file_extensions": ["rs"], "root_markers": ["Cargo.toml"] },
                                "session": {}
                            }
                        }
                    }
                },
                "agena.mcp": {
                    "package": { "kind": "static" },
                    "config": {
                        "runtime": { "token_store": { "enabled": false } },
                        "servers": {
                            "fixture": {
                                "transport": "stdio",
                                "process": {
                                    "command": mcp_fixture,
                                    "args": [],
                                    "env": {},
                                    "cwd": workspace
                                }
                            }
                        }
                    }
                },
                "example.echo": {
                    "package": { "kind": "cdylib", "path": cdylib },
                    "config": { "uppercase": true }
                },
                "example.echo_stdio": {
                    "package": {
                        "kind": "stdio",
                        "command": echo_stdio,
                        "args": [],
                        "env": {},
                        "cwd": workspace
                    }
                },
                "example.notes": {
                    "package": {
                        "kind": "stdio",
                        "command": notes_stdio,
                        "args": [],
                        "env": {},
                        "cwd": workspace
                    },
                    "config": { "prefix": "[probe] ", "uppercase": false }
                }
            }
        }
    });
    fs::write(
        workspace.join(".agena/agena.json"),
        serde_json::to_vec_pretty(&config)?,
    )?;
    Ok(())
}

struct LocalWebServer {
    address: String,
    hits: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl LocalWebServer {
    fn start() -> anyhow::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").context("bind web test fixture")?;
        listener
            .set_nonblocking(true)
            .context("make web test fixture nonblocking")?;
        let address = listener.local_addr()?.to_string();
        let hits = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let thread_hits = Arc::clone(&hits);
        let thread_stop = Arc::clone(&stop);
        let thread = thread::spawn(move || {
            while !thread_stop.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        thread_hits.fetch_add(1, Ordering::Relaxed);
                        let _ = respond_to_fixture_http(stream);
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => break,
                }
            }
        });
        Ok(Self {
            address,
            hits,
            stop,
            thread: Some(thread),
        })
    }

    fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.address)
    }

    fn hits(&self) -> usize {
        self.hits.load(Ordering::Relaxed)
    }
}

impl Drop for LocalWebServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = TcpStream::connect(&self.address);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn respond_to_fixture_http(mut stream: TcpStream) -> std::io::Result<()> {
    let mut request = [0_u8; 4_096];
    let count = stream.read(&mut request)?;
    let request = String::from_utf8_lossy(&request[..count]);
    let path = request
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or("/");
    let body = match path {
        "/robots.txt" => "User-agent: *\nAllow: /\n".to_string(),
        "/page" => "<html><body>DSV4F_WEB_MARKER</body></html>".to_string(),
        _ => "<html><body>DSV4F_ROOT <a href=\"/page\">page</a></body></html>".to_string(),
    };
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    stream.write_all(response.as_bytes())?;
    stream.flush()
}

async fn run_gateway_meta_suite(harness: &Harness, report: &mut SuiteReport) -> anyhow::Result<()> {
    if harness.selector.enabled("tools.list") {
        let session = harness
            .create_session(
                "dsv4f tools.list",
                &[GATEWAY_LIST],
                baseline_permission(PermissionMode::Allow),
            )
            .await?;
        let outcome = harness
            .run_native_gateway_function(
                session,
                "tools.list",
                "tools_list",
                GATEWAY_LIST,
                json!({"offset": 0, "limit": 5, "tags": ["read_only"]}),
                None,
            )
            .await?;
        ensure!(
            outcome.payload().get("tools").is_some(),
            "tools.list did not return a tools payload"
        );
        report.pass("tools.list");
    }
    if harness.selector.enabled("tools.search") {
        let session = harness
            .create_session(
                "dsv4f tools.search",
                &[GATEWAY_SEARCH],
                baseline_permission(PermissionMode::Allow),
            )
            .await?;
        let outcome = harness
            .run_native_gateway_function(
                session,
                "tools.search",
                "tools_search",
                GATEWAY_SEARCH,
                json!({"query": "schema_lab", "offset": 0, "limit": 5}),
                None,
            )
            .await?;
        assert_contains(&outcome, "schema_lab")?;
        report.pass("tools.search");
    }
    if harness.selector.enabled("tools.tags") {
        let session = harness
            .create_session(
                "dsv4f tools.tags",
                &[GATEWAY_TAGS],
                baseline_permission(PermissionMode::Allow),
            )
            .await?;
        let outcome = harness
            .run_native_gateway_function(
                session,
                "tools.tags",
                "tools_tags",
                GATEWAY_TAGS,
                json!({"offset": 0, "limit": 10}),
                None,
            )
            .await?;
        ensure!(
            outcome.payload().get("tags").is_some(),
            "tools.tags did not return a tags payload"
        );
        report.pass("tools.tags");
    }

    if harness.selector.any_in_group("tools.help") || harness.selector.any_in_group("tools.call") {
        let session = harness
            .create_session(
                "dsv4f native tools help/call",
                &[GATEWAY_HELP, GATEWAY_CALL, "agena.schema_lab.echo"],
                baseline_permission(PermissionMode::Allow),
            )
            .await?;
        let help = harness
            .run_native_gateway_function(
                session,
                "tools.help",
                "tools_help",
                GATEWAY_HELP,
                json!({"tool": "schema_lab.echo"}),
                Some("schema_lab.echo"),
            )
            .await?;
        if harness.selector.enabled("tools.help") {
            report.pass("tools.help");
        }
        if harness.selector.enabled("tools.call") {
            let call = harness
                .run_native_gateway_function(
                    help.session.id,
                    "tools.call",
                    "tools_call",
                    GATEWAY_CALL,
                    json!({
                        "tool": "schema_lab.echo",
                        "input": {"label": "gateway", "payload": {"marker": "GATEWAY_CALL_OK", "n": 1}}
                    }),
                    Some("GATEWAY_CALL_OK"),
                )
                .await?;
            assert_contains(&call, "GATEWAY_CALL_OK")?;
            report.pass("tools.call");
        }
    }
    Ok(())
}

async fn run_single(
    harness: &Harness,
    report: &mut SuiteReport,
    label: &str,
    target: &str,
    canonical_target: &str,
    input: Value,
    expected_text: Option<&str>,
) -> anyhow::Result<Option<GatewayOutcome>> {
    if !harness.selector.enabled(label) {
        return Ok(None);
    }
    let session = harness
        .create_session(
            label,
            &[canonical_target],
            baseline_permission(PermissionMode::Allow),
        )
        .await?;
    let outcome = harness
        .run_gateway_target(session, label, target, input, PendingReply::None, true)
        .await?;
    if let Some(expected) = expected_text {
        assert_contains(&outcome, expected)?;
    }
    report.pass(label);
    Ok(Some(outcome))
}

async fn run_builtin_suite(
    harness: &Harness,
    fixture: &Fixture,
    report: &mut SuiteReport,
) -> anyhow::Result<()> {
    run_skills_cases(harness, report).await?;
    run_code_cases(harness, report).await?;
    run_lsp_cases(harness, report).await?;
    run_cron_cases(harness, report).await?;
    run_fs_cases(harness, fixture, report).await?;
    run_settings_cases(harness, report).await?;
    run_process_cases(harness, report).await?;
    run_runtime_cases(harness, report).await?;
    run_plan_cases(harness, report).await?;
    run_memory_cases(harness, report).await?;
    run_schema_lab_cases(harness, report).await?;
    run_mcp_cases(harness, report).await?;
    run_snapshot_cases(harness, report).await?;
    run_task_case(harness, report).await?;
    run_web_cases(harness, fixture, report).await?;
    Ok(())
}

async fn run_skills_cases(harness: &Harness, report: &mut SuiteReport) -> anyhow::Result<()> {
    let _ = run_single(
        harness,
        report,
        "skills.list",
        "skills.list",
        "agena.skills.list",
        json!({"kind": "skill", "offset": 0, "limit": 20, "verbose": true}),
        Some("init"),
    )
    .await?;
    let _ = run_single(
        harness,
        report,
        "skills.get",
        "skills.get",
        "agena.skills.get",
        json!({"name": "init"}),
        Some("init"),
    )
    .await?;
    let _ = run_single(
        harness,
        report,
        "skills.run",
        "skills.run",
        "agena.skills.run",
        json!({"name": "init", "args": "DSV4F_SKILL_ARG"}),
        Some("DSV4F_SKILL_ARG"),
    )
    .await?;
    Ok(())
}

async fn run_code_cases(harness: &Harness, report: &mut SuiteReport) -> anyhow::Result<()> {
    let _ = run_single(
        harness,
        report,
        "code.search_ast",
        "code.search_ast",
        "agena.code.search_ast",
        json!({
            "path": "src/lib.rs",
            "pattern": "probe()",
            "language": "rust",
            "limit": 5
        }),
        Some("probe"),
    )
    .await?;
    let _ = run_single(
        harness,
        report,
        "code.syntax_tree",
        "code.syntax_tree",
        "agena.code.syntax_tree",
        json!({"path": "src/lib.rs", "language": "rust", "max_depth": 3}),
        Some("function_item"),
    )
    .await?;
    Ok(())
}

async fn run_lsp_cases(harness: &Harness, report: &mut SuiteReport) -> anyhow::Result<()> {
    let _ = run_single(
        harness,
        report,
        "lsp.servers",
        "lsp.servers",
        "agena.lsp.servers",
        json!({}),
        Some("rust"),
    )
    .await?;
    let position = json!({"file_path": "src/lib.rs", "line": 2, "character": 28});
    if harness.selector.enabled("lsp.definition") {
        let definition = harness
            .run_gateway_target(
                harness
                    .create_session(
                        "dsv4f lsp definition",
                        &["agena.lsp.definition"],
                        baseline_permission(PermissionMode::Allow),
                    )
                    .await?,
                "lsp.definition",
                "lsp.definition",
                position.clone(),
                PendingReply::None,
                true,
            )
            .await?;
        assert_contains(&definition, "src/lib.rs")?;
        report.pass("lsp.definition");
    }
    if harness.selector.enabled("lsp.references") {
        let references = harness
            .run_gateway_target(
                harness
                    .create_session(
                        "dsv4f lsp references",
                        &["agena.lsp.references"],
                        baseline_permission(PermissionMode::Allow),
                    )
                    .await?,
                "lsp.references",
                "lsp.references",
                json!({"file_path": "src/lib.rs", "line": 2, "character": 28, "include_declaration": true}),
                PendingReply::None,
                true,
            )
            .await?;
        assert_contains(&references, "src/lib.rs")?;
        report.pass("lsp.references");
    }
    if harness.selector.enabled("lsp.hover") {
        let hover = harness
            .run_gateway_target(
                harness
                    .create_session(
                        "dsv4f lsp hover",
                        &["agena.lsp.hover"],
                        baseline_permission(PermissionMode::Allow),
                    )
                    .await?,
                "lsp.hover",
                "lsp.hover",
                position,
                PendingReply::None,
                true,
            )
            .await?;
        assert_contains(&hover, "probe")?;
        report.pass("lsp.hover");
    }
    if harness.selector.enabled("lsp.diagnostics") {
        let diagnostics = harness
            .run_gateway_target(
                harness
                    .create_session(
                        "dsv4f lsp diagnostics",
                        &["agena.lsp.diagnostics"],
                        baseline_permission(PermissionMode::Allow),
                    )
                    .await?,
                "lsp.diagnostics",
                "lsp.diagnostics",
                json!({"file_path": "src/lib.rs"}),
                PendingReply::None,
                true,
            )
            .await?;
        ensure!(
            diagnostics.payload().get("entries").is_some(),
            "lsp.diagnostics did not return an entries payload"
        );
        report.pass("lsp.diagnostics");
    }
    Ok(())
}

async fn run_cron_cases(harness: &Harness, report: &mut SuiteReport) -> anyhow::Result<()> {
    if !harness.selector.any_in_group("cron") {
        return Ok(());
    }
    let session = harness
        .create_session(
            "dsv4f cron chain",
            &[
                "agena.cron.list",
                "agena.cron.create",
                "agena.cron.wakeup",
                "agena.cron.delete",
            ],
            baseline_permission(PermissionMode::Allow),
        )
        .await?;
    let list = harness
        .run_gateway_target(
            session,
            "cron.list",
            "cron.list",
            json!({}),
            PendingReply::None,
            true,
        )
        .await?;
    ensure!(
        list.payload().get("jobs").is_some(),
        "cron.list payload lacks jobs"
    );
    report.pass("cron.list");
    let created = harness
        .run_gateway_target(
            session,
            "cron.create",
            "cron.create",
            json!({"expression": "0 0 0 1 1 *", "prompt": "DSV4F cron probe", "max_age_days": 1}),
            PendingReply::None,
            true,
        )
        .await?;
    let cron_id = payload_string(&created.payload(), "id")?;
    report.pass("cron.create");
    let wakeup = harness
        .run_gateway_target(
            session,
            "cron.wakeup",
            "cron.wakeup",
            json!({"delay_seconds": 3600, "prompt": "DSV4F wakeup probe", "reason": "gateway suite"}),
            PendingReply::None,
            true,
        )
        .await?;
    let wakeup_id = payload_string(&wakeup.payload(), "id")?;
    report.pass("cron.wakeup");
    let deleted = harness
        .run_gateway_target(
            session,
            "cron.delete",
            "cron.delete",
            json!({"id": cron_id}),
            PendingReply::None,
            true,
        )
        .await?;
    ensure!(
        deleted.payload().get("removed").and_then(Value::as_bool) == Some(true),
        "cron.delete did not report removed=true"
    );
    report.pass("cron.delete");
    let cleanup = harness
        .run_gateway_target(
            session,
            "cron.delete.wakeup_cleanup",
            "cron.delete",
            json!({"id": wakeup_id}),
            PendingReply::None,
            true,
        )
        .await?;
    ensure!(
        cleanup.payload().get("removed").and_then(Value::as_bool) == Some(true),
        "wakeup cleanup did not report removed=true"
    );
    Ok(())
}

async fn run_fs_cases(
    harness: &Harness,
    fixture: &Fixture,
    report: &mut SuiteReport,
) -> anyhow::Result<()> {
    let _ = run_single(
        harness,
        report,
        "fs.glob",
        "fs.glob",
        "agena.fs.glob",
        json!({"pattern": "**/*.rs", "path": "src"}),
        Some("lib.rs"),
    )
    .await?;
    let _ = run_single(
        harness,
        report,
        "fs.grep",
        "fs.grep",
        "agena.fs.grep",
        json!({"pattern": "pub fn probe", "path": "src", "include": "*.rs"}),
        Some("pub fn probe"),
    )
    .await?;
    let _ = run_single(
        harness,
        report,
        "fs.read",
        "fs.read",
        "agena.fs.read",
        json!({"file_path": "src/lib.rs", "mode": "text", "offset": 1, "limit": 20}),
        Some("pub fn probe"),
    )
    .await?;
    if harness.selector.enabled("fs.apply_patch") {
        let session = harness
            .create_session(
                "dsv4f fs.apply_patch",
                &["agena.fs.apply_patch"],
                baseline_permission(PermissionMode::Allow),
            )
            .await?;
        let outcome = harness
            .run_gateway_target(
                session,
                "fs.apply_patch",
                "fs.apply_patch",
                json!({
                    "patch": "*** Begin Patch\n*** Add File: probe/patch.txt\n+DSV4F_PATCH_MARKER\n*** End Patch"
                }),
                PendingReply::None,
                true,
            )
            .await?;
        assert_contains(&outcome, "probe/patch.txt")?;
        ensure!(
            fs::read_to_string(fixture.workspace.join("probe/patch.txt"))?
                .contains("DSV4F_PATCH_MARKER"),
            "fs.apply_patch did not create the expected file"
        );
        commit_fixture_change(&fixture.workspace, "exercise fs.apply_patch")?;
        report.pass("fs.apply_patch");
    }
    Ok(())
}

async fn run_settings_cases(harness: &Harness, report: &mut SuiteReport) -> anyhow::Result<()> {
    if !harness.selector.any_in_group("settings") {
        return Ok(());
    }
    let session = harness
        .create_session(
            "dsv4f settings chain",
            &[
                "agena.settings.set",
                "agena.settings.get",
                "agena.settings.patch",
                "agena.settings.list",
                "agena.settings.delete",
                "agena.settings.validate",
            ],
            baseline_permission(PermissionMode::Allow),
        )
        .await?;
    let set = harness
        .run_gateway_target(
            session,
            "settings.set",
            "settings.set",
            json!({
                "path": "runtime.reload.poll_interval_secs",
                "value": 13,
                "dry_run": false,
                "validate": true,
                "reload": false
            }),
            PendingReply::None,
            true,
        )
        .await?;
    ensure!(
        set.payload().get("changed").and_then(Value::as_bool) == Some(true),
        "settings.set did not report a persistent change"
    );
    report.pass("settings.set");
    let get = harness
        .run_gateway_target(
            session,
            "settings.get",
            "settings.get",
            json!({"path": "runtime.reload.poll_interval_secs", "source": "file"}),
            PendingReply::None,
            true,
        )
        .await?;
    ensure!(
        get.payload().to_string().contains("13"),
        "settings.get did not observe the value written by settings.set"
    );
    report.pass("settings.get");
    let patch = harness
        .run_gateway_target(
            session,
            "settings.patch",
            "settings.patch",
            json!({
                "path": "runtime.reload",
                "changes": {"poll_interval_secs": 14},
                "dry_run": false,
                "validate": true,
                "reload": false
            }),
            PendingReply::None,
            true,
        )
        .await?;
    ensure!(
        patch.payload().get("changed").and_then(Value::as_bool) == Some(true),
        "settings.patch did not report a persistent change"
    );
    report.pass("settings.patch");
    let list = harness
        .run_gateway_target(
            session,
            "settings.list",
            "settings.list",
            json!({"path": "runtime.reload", "source": "file", "recursive": true}),
            PendingReply::None,
            true,
        )
        .await?;
    ensure!(
        list.payload().to_string().contains("poll_interval_secs"),
        "settings.list did not enumerate the patched setting"
    );
    report.pass("settings.list");
    let delete = harness
        .run_gateway_target(
            session,
            "settings.delete",
            "settings.delete",
            json!({
                "path": "runtime.reload.poll_interval_secs",
                "dry_run": false,
                "validate": true,
                "reload": false
            }),
            PendingReply::None,
            true,
        )
        .await?;
    ensure!(
        delete.payload().get("deleted").and_then(Value::as_bool) == Some(true),
        "settings.delete did not report deleted=true"
    );
    report.pass("settings.delete");
    let validate = harness
        .run_gateway_target(
            session,
            "settings.validate",
            "settings.validate",
            json!({}),
            PendingReply::None,
            true,
        )
        .await?;
    ensure!(
        validate.payload().get("valid").and_then(Value::as_bool) == Some(true),
        "settings.validate did not report valid=true"
    );
    report.pass("settings.validate");
    Ok(())
}

async fn run_process_cases(harness: &Harness, report: &mut SuiteReport) -> anyhow::Result<()> {
    if !harness.selector.any_in_group("process") {
        return Ok(());
    }
    let session = harness
        .create_session(
            "dsv4f process chain",
            &[
                "agena.process.run",
                "agena.process.list",
                "agena.process.logs",
                "agena.process.stop",
            ],
            baseline_permission(PermissionMode::Allow),
        )
        .await?;
    let foreground = harness
        .run_gateway_target(
            session,
            "process.run.foreground",
            "process.run",
            json!({
                "shell": "bash",
                "command": "printf PROCESS_FG_OK",
                "description": "dsv4f foreground process",
                "timeout_ms": 5000,
                "filesystem_effects": [],
                "network_effects": [],
                "background": false
            }),
            PendingReply::None,
            true,
        )
        .await?;
    assert_contains(&foreground, "PROCESS_FG_OK")?;
    report.pass("process.run");
    let background = harness
        .run_gateway_target(
            session,
            "process.run.background",
            "process.run",
            json!({
                "shell": "bash",
                "command": "printf 'PROCESS_BG_OK\\n'; sleep 300",
                "description": "dsv4f background process",
                "timeout_ms": 310000,
                "filesystem_effects": [],
                "network_effects": [],
                "background": true
            }),
            PendingReply::None,
            true,
        )
        .await?;
    let process_id = payload_string(&background.payload(), "process_id")?;
    let list = harness
        .run_gateway_target(
            session,
            "process.list",
            "process.list",
            json!({}),
            PendingReply::None,
            true,
        )
        .await?;
    assert_contains(&list, &process_id)?;
    report.pass("process.list");
    let logs = harness
        .run_gateway_target(
            session,
            "process.logs",
            "process.logs",
            json!({"process_id": process_id, "since_seq": 0, "limit": 20, "wait_ms": 1500}),
            PendingReply::None,
            true,
        )
        .await?;
    assert_contains(&logs, "PROCESS_BG_OK")?;
    report.pass("process.logs");
    let stop = harness
        .run_gateway_target(
            session,
            "process.stop",
            "process.stop",
            json!({"process_id": payload_string(&background.payload(), "process_id")?}),
            PendingReply::None,
            true,
        )
        .await?;
    ensure!(
        stop.payload().get("status").and_then(Value::as_str) == Some("stopped"),
        "process.stop did not terminate the still-running fixture process: {}",
        stop.visible_text()
    );
    report.pass("process.stop");
    Ok(())
}

async fn run_runtime_cases(harness: &Harness, report: &mut SuiteReport) -> anyhow::Result<()> {
    if !harness.selector.any_in_group("runtime") {
        return Ok(());
    }
    let session = harness
        .create_session(
            "dsv4f runtime chain",
            &[
                "agena.runtime.get",
                "agena.runtime.rename",
                "agena.runtime.switch",
                "agena.runtime.restore",
                "agena.runtime.request_input",
            ],
            baseline_permission(PermissionMode::Allow),
        )
        .await?;
    let current = harness
        .run_gateway_target(
            session,
            "runtime.get",
            "runtime.get",
            json!({}),
            PendingReply::None,
            true,
        )
        .await?;
    ensure!(
        current.payload().get("session").is_some(),
        "runtime.get lacks session payload"
    );
    report.pass("runtime.get");
    let renamed = harness
        .run_gateway_target(
            session,
            "runtime.rename",
            "runtime.rename",
            json!({"title": "dsv4f-runtime-renamed"}),
            PendingReply::None,
            true,
        )
        .await?;
    assert_contains(&renamed, "dsv4f-runtime-renamed")?;
    report.pass("runtime.rename");
    let switched = harness
        .run_gateway_target(
            session,
            "runtime.switch",
            "runtime.switch",
            json!({"agent": "verify", "push_previous": true}),
            PendingReply::None,
            true,
        )
        .await?;
    assert_contains(&switched, "verify")?;
    report.pass("runtime.switch");
    let restored = harness
        .run_gateway_target(
            session,
            "runtime.restore",
            "runtime.restore",
            json!({}),
            PendingReply::None,
            true,
        )
        .await?;
    ensure!(
        restored.payload().get("restored").and_then(Value::as_bool) == Some(true),
        "runtime.restore did not report restored=true"
    );
    report.pass("runtime.restore");
    let requested = harness
        .run_gateway_target(
            session,
            "runtime.request_input",
            "runtime.request_input",
            json!({
                "title": "Gateway input probe",
                "body_markdown": "Return TEST_OK.",
                "kind": "text",
                "submit_label": "Submit",
                "cancel_label": "Cancel",
                "questions": [{
                    "id": "answer",
                    "header": "Answer",
                    "question": "Reply TEST_OK",
                    "allow_custom": true
                }]
            }),
            PendingReply::Input,
            true,
        )
        .await?;
    assert_contains(&requested, "TEST_OK")?;
    report.pass("runtime.request_input");
    Ok(())
}

async fn run_plan_cases(harness: &Harness, report: &mut SuiteReport) -> anyhow::Result<()> {
    if !harness.selector.any_in_group("plan") {
        return Ok(());
    }
    let session = harness
        .create_session(
            "dsv4f plan chain",
            &[
                "agena.plan.set",
                "agena.plan.get",
                "agena.plan.update",
                "agena.plan.clear",
            ],
            baseline_permission(PermissionMode::Allow),
        )
        .await?;
    let set = harness
        .run_gateway_target(
            session,
            "plan.set",
            "plan.set",
            json!({
                "objective": "Exercise dsv4f plan gateway",
                "title": "DSV4F plan",
                "steps": [{
                    "id": "probe-step",
                    "title": "Probe step",
                    "checks": [{"id": "probe-check", "text": "Check gateway"}]
                }],
                "autorun": false
            }),
            PendingReply::None,
            true,
        )
        .await?;
    assert_contains(&set, "DSV4F plan")?;
    report.pass("plan.set");
    let get = harness
        .run_gateway_target(
            session,
            "plan.get",
            "plan.get",
            json!({"view": "full"}),
            PendingReply::None,
            true,
        )
        .await?;
    assert_contains(&get, "probe-step")?;
    report.pass("plan.get");
    let update = harness
        .run_gateway_target(
            session,
            "plan.update",
            "plan.update",
            json!({"step_id": "probe-step", "check_id": "probe-check", "status": "completed"}),
            PendingReply::None,
            true,
        )
        .await?;
    assert_contains(&update, "completed")?;
    report.pass("plan.update");
    let clear = harness
        .run_gateway_target(
            session,
            "plan.clear",
            "plan.clear",
            json!({}),
            PendingReply::None,
            true,
        )
        .await?;
    ensure!(
        clear.payload().get("cleared").and_then(Value::as_bool) == Some(true),
        "plan.clear did not report cleared=true"
    );
    report.pass("plan.clear");
    Ok(())
}

async fn run_memory_cases(harness: &Harness, report: &mut SuiteReport) -> anyhow::Result<()> {
    if !harness.selector.any_in_group("memory") {
        return Ok(());
    }
    let session = harness
        .create_session(
            "dsv4f memory chain",
            &[
                "agena.memory.write",
                "agena.memory.get",
                "agena.memory.list",
                "agena.memory.search",
                "agena.memory.delete",
            ],
            baseline_permission(PermissionMode::Allow),
        )
        .await?;
    let write = harness
        .run_gateway_target(
            session,
            "memory.write",
            "memory.write",
            json!({
                "name": "probe-memory",
                "description": "dsv4f durable probe",
                "memory_type": "project",
                "content": "DSV4F_MEMORY_MARKER"
            }),
            PendingReply::None,
            true,
        )
        .await?;
    assert_contains(&write, "probe-memory")?;
    report.pass("memory.write");
    let get = harness
        .run_gateway_target(
            session,
            "memory.get",
            "memory.get",
            json!({"name": "probe-memory"}),
            PendingReply::None,
            true,
        )
        .await?;
    assert_contains(&get, "DSV4F_MEMORY_MARKER")?;
    report.pass("memory.get");
    let list = harness
        .run_gateway_target(
            session,
            "memory.list",
            "memory.list",
            json!({"limit": 20}),
            PendingReply::None,
            true,
        )
        .await?;
    assert_contains(&list, "probe-memory")?;
    report.pass("memory.list");
    let search = harness
        .run_gateway_target(
            session,
            "memory.search",
            "memory.search",
            json!({"query": "DSV4F_MEMORY_MARKER", "limit": 10}),
            PendingReply::None,
            true,
        )
        .await?;
    assert_contains(&search, "probe-memory")?;
    report.pass("memory.search");
    let delete = harness
        .run_gateway_target(
            session,
            "memory.delete",
            "memory.delete",
            json!({"name": "probe-memory"}),
            PendingReply::None,
            true,
        )
        .await?;
    assert_contains(&delete, "probe-memory")?;
    report.pass("memory.delete");
    Ok(())
}

async fn run_schema_lab_cases(harness: &Harness, report: &mut SuiteReport) -> anyhow::Result<()> {
    let _ = run_single(
        harness,
        report,
        "schema_lab.inspect",
        "schema_lab.inspect",
        "agena.schema_lab.inspect",
        json!({"section": "basics", "include_defaults": true}),
        Some("inspect"),
    )
    .await?;
    let _ = run_single(
        harness,
        report,
        "schema_lab.echo",
        "schema_lab.echo",
        "agena.schema_lab.echo",
        json!({"label": "dsv4f", "payload": {"marker": "SCHEMA_OK", "n": 1}}),
        Some("SCHEMA_OK"),
    )
    .await?;
    Ok(())
}

async fn run_mcp_cases(harness: &Harness, report: &mut SuiteReport) -> anyhow::Result<()> {
    if !harness.selector.any_in_group("mcp") {
        return Ok(());
    }
    let session = harness
        .create_session(
            "dsv4f mcp bridge chain",
            &[
                "agena.mcp.resources.list",
                "agena.mcp.resources.read",
                "agena.mcp.prompts.list",
                "agena.mcp.prompts.get",
                "agena.mcp.tools.call",
            ],
            baseline_permission(PermissionMode::Allow),
        )
        .await?;
    let resources = harness
        .run_gateway_target(
            session,
            "mcp.resources.list",
            "mcp.resources.list",
            json!({"server": "fixture"}),
            PendingReply::None,
            true,
        )
        .await?;
    assert_contains(&resources, "fixture://hello")?;
    report.pass("mcp.resources.list");
    let resource = harness
        .run_gateway_target(
            session,
            "mcp.resources.read",
            "mcp.resources.read",
            json!({"server": "fixture", "uri": "fixture://hello"}),
            PendingReply::None,
            true,
        )
        .await?;
    assert_contains(&resource, "MCP_RESOURCE_OK")?;
    report.pass("mcp.resources.read");
    let prompts = harness
        .run_gateway_target(
            session,
            "mcp.prompts.list",
            "mcp.prompts.list",
            json!({"server": "fixture"}),
            PendingReply::None,
            true,
        )
        .await?;
    assert_contains(&prompts, "probe")?;
    report.pass("mcp.prompts.list");
    let prompt = harness
        .run_gateway_target(
            session,
            "mcp.prompts.get",
            "mcp.prompts.get",
            json!({"server": "fixture", "name": "probe", "arguments": {"name": "dsv4f"}}),
            PendingReply::None,
            true,
        )
        .await?;
    assert_contains(&prompt, "MCP_PROMPT_OK")?;
    report.pass("mcp.prompts.get");
    let called = harness
        .run_gateway_target(
            session,
            "mcp.tools.call",
            "mcp.tools.call",
            json!({"server": "fixture", "name": "echo", "arguments": {"value": "MCP_OK"}}),
            PendingReply::None,
            true,
        )
        .await?;
    assert_contains(&called, "MCP_ECHO_OK")?;
    report.pass("mcp.tools.call");
    Ok(())
}

async fn run_snapshot_cases(harness: &Harness, report: &mut SuiteReport) -> anyhow::Result<()> {
    if !harness.selector.any_in_group("snapshot") {
        return Ok(());
    }
    let session = harness
        .create_session(
            "dsv4f snapshot chain",
            &["agena.snapshot.enter", "agena.snapshot.exit"],
            baseline_permission(PermissionMode::Allow),
        )
        .await?;
    let entered = harness
        .run_gateway_target(
            session,
            "snapshot.enter",
            "snapshot.enter",
            json!({"target": "new", "name": "dsv4f-snapshot"}),
            PendingReply::None,
            true,
        )
        .await?;
    let snapshot_path = payload_string(&entered.payload(), "path")?;
    ensure!(
        Path::new(&snapshot_path).is_dir(),
        "snapshot path was not created: {snapshot_path}"
    );
    report.pass("snapshot.enter");
    let exited = harness
        .run_gateway_target(
            session,
            "snapshot.exit",
            "snapshot.exit",
            json!({"exit_action": "remove", "discard_changes": false}),
            PendingReply::None,
            true,
        )
        .await?;
    assert_contains(&exited, "remove")?;
    ensure!(
        !Path::new(&snapshot_path).exists(),
        "snapshot.exit did not remove {snapshot_path}"
    );
    report.pass("snapshot.exit");
    Ok(())
}

async fn run_task_case(harness: &Harness, report: &mut SuiteReport) -> anyhow::Result<()> {
    if !harness.selector.enabled("tasks.run") {
        return Ok(());
    }
    let session = harness
        .create_session(
            "dsv4f task run",
            &["agena.tasks.run"],
            baseline_permission(PermissionMode::Allow),
        )
        .await?;
    let outcome = harness
        .run_gateway_target_with_timeout(
            session,
            "tasks.run",
            "tasks.run",
            json!({
                "description": "dsv4f gateway task",
                "prompt": "Reply with exactly SUBTASK_OK. Do not call any tools.",
                "subagent_type": "verify"
            }),
            PendingReply::None,
            true,
            harness.task_timeout,
        )
        .await?;
    let child_id = outcome
        .payload()
        .get("session_id")
        .and_then(Value::as_str)
        .context("tasks.run payload lacks session_id")?
        .parse::<i64>()
        .context("parse tasks.run child session id")?;
    let child = tokio::time::timeout(harness.task_timeout, async {
        loop {
            let child = harness.manager.get_session(child_id).await?;
            if !harness.manager.is_run_active(child_id).await {
                return Ok::<_, anyhow::Error>(child);
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    })
    .await
    .context("tasks.run child session did not complete")??;
    ensure!(
        transcript_since(&child, 0).contains("SUBTASK_OK"),
        "tasks.run child transcript did not contain SUBTASK_OK: {}",
        transcript_since(&child, 0)
    );
    report.pass("tasks.run");
    Ok(())
}

async fn run_web_cases(
    harness: &Harness,
    fixture: &Fixture,
    report: &mut SuiteReport,
) -> anyhow::Result<()> {
    if !harness.selector.any_in_group("web") {
        return Ok(());
    }
    let session = harness
        .create_session(
            "dsv4f web chain",
            &["agena.web.fetch", "agena.web.crawl", "agena.web.search"],
            baseline_permission(PermissionMode::Allow),
        )
        .await?;
    let fetched = harness
        .run_gateway_target(
            session,
            "web.fetch",
            "web.fetch",
            json!({
                "url": fixture.web.url("/page"),
                "prompt": "return DSV4F_WEB_MARKER",
                "use_cache": false,
                "render_js": false
            }),
            PendingReply::None,
            true,
        )
        .await?;
    assert_contains(&fetched, "DSV4F_WEB_MARKER")?;
    report.pass("web.fetch");
    let crawled = harness
        .run_gateway_target(
            session,
            "web.crawl",
            "web.crawl",
            json!({
                "start_url": fixture.web.url("/"),
                "max_pages": 2,
                "max_depth": 1,
                "same_host_only": true,
                "use_cache": false,
                "render_js": false
            }),
            PendingReply::None,
            true,
        )
        .await?;
    let crawl_payload = crawled.payload();
    ensure!(
        crawl_payload
            .get("stored_count")
            .and_then(Value::as_u64)
            .is_some_and(|count| count >= 2),
        "web.crawl did not index both fixture pages: {crawl_payload}"
    );
    ensure!(
        crawl_payload.get("failure_count").and_then(Value::as_u64) == Some(0),
        "web.crawl reported failures for the local fixture: {crawl_payload}"
    );
    report.pass("web.crawl");
    let search = harness
        .run_gateway_target(
            session,
            "web.search",
            "web.search",
            json!({"query": "Agena DSV4F gateway probe", "engine": "auto", "limit": 1}),
            PendingReply::None,
            true,
        )
        .await?;
    ensure!(
        search
            .payload()
            .get("attempted_engines")
            .and_then(Value::as_array)
            .is_some_and(|engines| !engines.is_empty()),
        "web.search did not report attempted engines"
    );
    report.pass("web.search");
    Ok(())
}

async fn run_external_plugin_suite(
    harness: &Harness,
    fixture: &Fixture,
    report: &mut SuiteReport,
) -> anyhow::Result<()> {
    if !harness.selector.any_in_group("external") {
        return Ok(());
    }
    let _ = run_single(
        harness,
        report,
        "external.echo",
        "echo",
        "example.echo.echo",
        json!({"text": "hello external"}),
        Some("[PREPARED] HELLO EXTERNAL"),
    )
    .await?;
    if harness.selector.enabled("external.echo_stdio") {
        let session = harness
            .create_session(
                "dsv4f external echo_stdio stream",
                &["example.echo_stdio.echo"],
                baseline_permission(PermissionMode::Allow),
            )
            .await?;
        let outcome = harness
            .run_gateway_streaming_target(
                session,
                "external.echo_stdio",
                "echo_stdio.echo",
                json!({"text": "streaming external"}),
                "stdio-echo: [prepared] streaming external",
            )
            .await?;
        assert_contains(&outcome, "stdio-echo: [prepared] streaming external")?;
        report.pass("external.echo_stdio");
    }
    let _ = run_single(
        harness,
        report,
        "external.notes.format",
        "notes.format",
        "example.notes.format",
        json!({"text": "streaming note\n"}),
        Some("[probe] streaming note"),
    )
    .await?;
    if harness.selector.enabled("external.notes.write") {
        let session = harness
            .create_session(
                "dsv4f external notes.write",
                &["example.notes.write"],
                baseline_permission(PermissionMode::Allow),
            )
            .await?;
        let outcome = harness
            .run_gateway_target(
                session,
                "external.notes.write",
                "notes.write",
                json!({"path": "probe/note.txt", "text": "EXTERNAL_NOTE_OK", "append": false}),
                PendingReply::None,
                true,
            )
            .await?;
        let write_payload = outcome.payload();
        ensure!(
            write_payload.get("path").and_then(Value::as_str) == Some("probe/note.txt")
                && write_payload.get("append").and_then(Value::as_bool) == Some(false)
                && write_payload
                    .get("bytes")
                    .and_then(Value::as_u64)
                    .is_some_and(|bytes| bytes > 0),
            "notes.write returned an unexpected payload: {write_payload}"
        );
        ensure!(
            fs::read_to_string(fixture.workspace.join("probe/note.txt"))?
                .contains("[probe] EXTERNAL_NOTE_OK"),
            "notes.write did not create the formatted fixture note"
        );
        report.pass("external.notes.write");
    }
    Ok(())
}

async fn run_nested_permission_suite(
    harness: &Harness,
    fixture: &Fixture,
    report: &mut SuiteReport,
) -> anyhow::Result<()> {
    if !harness.selector.any_in_group("permission") {
        return Ok(());
    }
    run_nested_permission_mode(
        harness,
        fixture,
        "permission.allow_once",
        PermissionReplyKind::AllowOnce,
        true,
        true,
    )
    .await?;
    run_nested_permission_mode(
        harness,
        fixture,
        "permission.allow_always",
        PermissionReplyKind::AllowAlways,
        true,
        false,
    )
    .await?;
    run_nested_permission_mode(
        harness,
        fixture,
        "permission.deny_once",
        PermissionReplyKind::DenyOnce,
        false,
        true,
    )
    .await?;
    run_nested_permission_mode(
        harness,
        fixture,
        "permission.deny_always",
        PermissionReplyKind::DenyAlways,
        false,
        false,
    )
    .await?;
    report.pass("permission.nested_host_gateway");
    Ok(())
}

/// Exercise the four persistence modes for a dynamic permission discovered by
/// a target *inside* tools.call. The second invocation distinguishes once from
/// always without granting broad gateway permissions.
async fn run_nested_permission_mode(
    harness: &Harness,
    fixture: &Fixture,
    label: &str,
    first_reply: PermissionReplyKind,
    first_succeeds: bool,
    second_requires_reply: bool,
) -> anyhow::Result<()> {
    let session = harness
        .create_session(
            label,
            &["agena.web.fetch"],
            baseline_permission(PermissionMode::Ask),
        )
        .await?;
    let input = json!({
        "url": fixture.web.url("/page"),
        "prompt": "return DSV4F_WEB_MARKER",
        "use_cache": false,
        "render_js": false
    });
    let hits_before = fixture.web.hits();
    let first = harness
        .run_gateway_target(
            session,
            &format!("{label}.first"),
            "web.fetch",
            input.clone(),
            PendingReply::Permission(first_reply),
            first_succeeds,
        )
        .await?;
    if first_succeeds {
        assert_contains(&first, "DSV4F_WEB_MARKER")?;
    } else {
        ensure!(
            fixture.web.hits() == hits_before,
            "denied host permission reached the web server"
        );
    }

    let second_reply = if second_requires_reply {
        if first_succeeds {
            PendingReply::Permission(PermissionReplyKind::DenyOnce)
        } else {
            PendingReply::Permission(PermissionReplyKind::AllowOnce)
        }
    } else {
        PendingReply::None
    };
    let second_succeeds = if second_requires_reply {
        !first_succeeds
    } else {
        first_succeeds
    };
    let hits_before_second = fixture.web.hits();
    let second = harness
        .run_gateway_target(
            session,
            &format!("{label}.second"),
            "web.fetch",
            input,
            second_reply,
            second_succeeds,
        )
        .await?;
    if second_succeeds {
        assert_contains(&second, "DSV4F_WEB_MARKER")?;
    } else {
        ensure!(
            fixture.web.hits() == hits_before_second,
            "denied persistent host permission reached the web server"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_analyzer_environment_keeps_host_toolchain_when_fixture_home_isolated() {
        let environment = rust_toolchain_environment_for(Path::new("/host/home"), None, None);

        assert_eq!(
            environment.get("RUSTUP_HOME").map(String::as_str),
            Some("/host/home/.rustup")
        );
        assert_eq!(
            environment.get("CARGO_HOME").map(String::as_str),
            Some("/host/home/.cargo")
        );
    }

    #[test]
    fn rust_analyzer_environment_respects_explicit_toolchain_roots() {
        let environment = rust_toolchain_environment_for(
            Path::new("/host/home"),
            Some(PathBuf::from("/toolchains/rustup")),
            Some(PathBuf::from("/toolchains/cargo")),
        );

        assert_eq!(
            environment.get("RUSTUP_HOME").map(String::as_str),
            Some("/toolchains/rustup")
        );
        assert_eq!(
            environment.get("CARGO_HOME").map(String::as_str),
            Some("/toolchains/cargo")
        );
    }
}
