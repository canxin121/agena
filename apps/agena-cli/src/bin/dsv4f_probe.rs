use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

use agena::{
    AppError,
    agent::{
        NetworkPermissionConfig, PathAccessModes, PathPermissionConfig, PermissionConfig,
        ToolPermissionConfig,
    },
    config::{ConfigOverride, LoadConfigRequest},
    message::{
        PartContent, PendingInteractiveRequest, ProcessShell, ProcessToolInput, ShellCommandInput,
        UserInputReply, UserInputReplyKind,
    },
    model::ModelRef,
    permission::{PermissionMode, PermissionReply, PermissionReplyKind},
    role::Role,
    runtime::AgenaRuntime,
    session::{
        RunStatus, Session, SessionCreateRequest, SessionExecutionReplyRequest, SessionManager,
        SessionPermissionReplyRequest, SessionRunOptions, SessionUserMessageRequest,
    },
    tool::ToolPayloadInput,
};
use agena_plugin_sdk::schema_example_texts;
use clap::Parser;
use serde_json::{Value, json};

const CASE_TIMEOUT_SECS: u64 = 45;
const INTERACTIVE_CASE_TIMEOUT_SECS: u64 = 30;
const CASE_TOOL_RETRY_LIMIT: usize = 3;
const CASE_SETTLE_MS: u64 = 2_000;
const FS_READ_TOKEN: &str = "RUNTIME_TOKEN_7f3d98c2";

#[derive(Debug, Parser)]
struct Args {
    #[arg(long, default_value = "cline")]
    target: String,
    #[arg(long)]
    model: Option<String>,
    #[arg(long)]
    workspace: Option<PathBuf>,
    #[arg(long)]
    list: bool,
    #[arg(long)]
    tool: Option<String>,
    #[arg(long)]
    limit: Option<usize>,
}

#[derive(Debug, Clone)]
struct ProbeCase {
    tool_name: String,
    label: String,
    input: Value,
}

#[derive(Debug)]
struct ProbeOutcome {
    tool_name: String,
    label: String,
    ok: bool,
    reason: Option<String>,
}

fn main() -> Result<(), AppError> {
    agena::runtime::build_app_runtime()?.block_on(async_main())
}

async fn async_main() -> Result<(), AppError> {
    let args = Args::parse();
    let workspace = args
        .workspace
        .clone()
        .unwrap_or_else(default_probe_workspace);
    prepare_probe_workspace(&workspace)?;

    let load_request = LoadConfigRequest {
        overrides: vec![
            format!("providers.default={}", args.target).parse::<ConfigOverride>()?,
            "runtime.providers.http.timeout_secs=30".parse::<ConfigOverride>()?,
            "runtime.providers.retry.max_retries=1".parse::<ConfigOverride>()?,
        ],
        workspace_root: Some(workspace.clone()),
    };
    let runtime = AgenaRuntime::builder()
        .with_load_request(load_request)
        .with_workspace_root(workspace.clone())
        .with_database_url("sqlite::memory:")
        .build()
        .await?;
    let manager = runtime
        .session_manager()
        .ok_or_else(|| AppError::Config("session storage unavailable".to_string()))?;
    let model = resolve_model(&runtime, &args)?;
    let executor = manager
        .tool_executor()
        .with_model_id(model.model_id.to_string());
    let mut tools = executor.available_model_tools();
    tools.sort_by(|left, right| left.exposed_name.cmp(&right.exposed_name));

    if let Some(name) = args.tool.as_deref() {
        tools.retain(|tool| tool.exposed_name == name);
    }
    if let Some(limit) = args.limit {
        tools.truncate(limit);
    }

    if args.list {
        for tool in &tools {
            println!("{}", tool.exposed_name);
        }
        return Ok(());
    }

    let mut cases = Vec::new();
    for tool in &tools {
        cases.extend(build_probe_cases(
            tool.exposed_name.as_str(),
            &tool.sanitized_input_schema(),
        ));
    }

    let mut outcomes = Vec::new();
    for case in cases {
        println!("RUN {} [{}]", case.tool_name, case.label);
        let tool_name = case.tool_name.clone();
        let label = case.label.clone();
        match run_probe_case(&manager, &model, case).await {
            Ok(outcome) => outcomes.push(outcome),
            Err(err) => {
                println!("BAD {} [{}] {}", tool_name, label, err);
                outcomes.push(ProbeOutcome {
                    tool_name,
                    label,
                    ok: false,
                    reason: Some(err.to_string()),
                });
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(CASE_SETTLE_MS)).await;
    }

    let failures = outcomes
        .iter()
        .filter(|outcome| !outcome.ok)
        .collect::<Vec<_>>();
    println!("workspace: {}", workspace.display());
    println!("model: {}", model);
    println!("total: {}", outcomes.len());
    println!("failed: {}", failures.len());
    for failure in &failures {
        println!(
            "FAIL {} [{}] {}",
            failure.tool_name,
            failure.label,
            failure.reason.as_deref().unwrap_or("unknown failure")
        );
    }

    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    if failures.is_empty() {
        Ok(())
    } else {
        Err(AppError::Config(format!(
            "{} probe case(s) failed",
            failures.len()
        )))
    }
}

fn resolve_model(runtime: &AgenaRuntime, args: &Args) -> Result<ModelRef, AppError> {
    match args.model.as_deref() {
        Some(model) => runtime
            .current_snapshot()
            .resolve_model_target(args.target.as_str(), Some(model)),
        None => runtime
            .current_snapshot()
            .resolve_model_target(args.target.as_str(), None),
    }
}

fn default_probe_workspace() -> PathBuf {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    std::env::temp_dir().join(format!(
        "agena-dsv4f-probe-{}-{}",
        std::process::id(),
        millis
    ))
}

fn prepare_probe_workspace(root: &Path) -> Result<(), AppError> {
    if root.exists() {
        fs::remove_dir_all(root)?;
    }
    fs::create_dir_all(root.join("src"))?;
    fs::create_dir_all(root.join("notes"))?;
    fs::create_dir_all(root.join("data"))?;
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"probe\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )?;
    fs::write(root.join("README.md"), "# Probe Workspace\n")?;
    fs::write(
        root.join("src/main.rs"),
        "fn main() { println!(\"probe\"); }\n",
    )?;
    fs::write(
        root.join("src/lib.rs"),
        "pub fn probe_value() -> &'static str { \"probe\" }\n",
    )?;
    fs::write(root.join("notes/todo.txt"), "probe task\n")?;
    fs::write(
        root.join("notes/runtime_token.txt"),
        format!("{FS_READ_TOKEN}\n"),
    )?;
    fs::write(root.join("data/sample.json"), "{\n  \"probe\": true\n}\n")?;

    run_git(root, ["init"])?;
    run_git(root, ["config", "user.name", "Agena Probe"])?;
    run_git(root, ["config", "user.email", "probe@example.com"])?;
    run_git(root, ["add", "."])?;
    run_git(root, ["commit", "-m", "probe baseline"])?;
    Ok(())
}

fn run_git<const N: usize>(root: &Path, args: [&str; N]) -> Result<(), AppError> {
    let status = Command::new("git").args(args).current_dir(root).status()?;
    if status.success() {
        Ok(())
    } else {
        Err(AppError::Config(format!(
            "git command failed: git {}",
            args.join(" ")
        )))
    }
}

fn build_probe_cases(tool_name: &str, schema: &Value) -> Vec<ProbeCase> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    let manual_examples = tool_specific_examples(tool_name);
    let raw_examples = if manual_examples.is_empty() {
        schema_example_texts(schema)
            .into_iter()
            .enumerate()
            .map(|(idx, example)| (format!("schema-{idx}"), example))
            .collect::<Vec<_>>()
    } else {
        manual_examples
    };

    for (label, raw) in raw_examples {
        let Ok(mut value) = serde_json::from_str::<Value>(raw.as_str()) else {
            continue;
        };
        sanitize_value(tool_name, None, &mut value);
        canonicalize_probe_case(tool_name, label.as_str(), &mut value);
        let key = format!(
            "{}:{}",
            tool_name,
            serde_json::to_string(&value).unwrap_or_default()
        );
        if seen.insert(key) {
            out.push(ProbeCase {
                tool_name: tool_name.to_string(),
                label,
                input: value,
            });
        }
    }

    if out.is_empty() {
        let mut input = Value::Object(Default::default());
        sanitize_value(tool_name, None, &mut input);
        out.push(ProbeCase {
            tool_name: tool_name.to_string(),
            label: "empty".to_string(),
            input,
        });
    }

    out
}

fn canonicalize_probe_case(tool_name: &str, label: &str, value: &mut Value) {
    if tool_name == "fs.read" && label.starts_with("schema-") {
        *value = json!({"path":"README.md"});
    } else if tool_name == "snapshot.exit" && label.starts_with("schema-") {
        *value = json!({"action":"keep","discard_changes":false});
    }
}

fn tool_specific_examples(tool_name: &str) -> Vec<(String, String)> {
    let cases: Vec<Value> = match tool_name {
        "tools.usage" => vec![json!({})],
        "tools.list" => vec![json!({"limit":10,"verbose":true})],
        "tools.search" => vec![json!({"query":"fs","limit":5})],
        "tools.help" => vec![json!({"tool":"fs.read","include_schema":true})],
        "tool.help" => vec![json!({"tool":"tool.help","include_schema":true})],
        "tool_catalog" => vec![json!({"query":"fs","limit":5})],
        "fs.read" => vec![json!({"path":"notes/runtime_token.txt"})],
        "fs.glob" => vec![json!({"pattern":"src/**/*.rs"})],
        "fs.grep" => vec![json!({"pattern":"probe","path":"src"})],
        "fs.apply_patch" => vec![
            json!({"patch":"*** Begin Patch\n*** Add File: probe_note.txt\n+probe\n*** End Patch\n"}),
        ],
        "code.syntax_tree" => vec![json!({"path":"src/main.rs"})],
        "code.search_ast" => vec![json!({"path":"src/main.rs","pattern":"main"})],
        "lsp.definition" => vec![json!({"file_path":"src/lib.rs","line":1,"character":4})],
        "lsp.references" => vec![
            json!({"file_path":"src/lib.rs","line":1,"character":4,"include_declaration":true}),
        ],
        "lsp.hover" => vec![json!({"file_path":"src/lib.rs","line":1,"character":4})],
        "lsp.diagnostics" => vec![json!({"file_path":"src/lib.rs"})],
        "lsp.servers" => vec![json!({})],
        "process.run" => vec![
            json!({"command":"printf probe","description":"print probe","workdir":".","filesystem_effects":[],"network_effects":[]}),
        ],
        "process.list" => vec![json!({})],
        "process.logs" => vec![json!({"process_id":"probe","since_seq":0,"wait_ms":0})],
        "process.stop" => vec![json!({"process_id":"probe"})],
        "settings.list" => vec![json!({})],
        "settings.get" => vec![json!({"path":"agents.default","source":"file"})],
        "settings.validate" => vec![json!({})],
        "settings.set" => {
            vec![json!({"path":"session.compaction.auto","value":true,"dry_run":true})]
        }
        "settings.patch" => {
            vec![json!({"changes":{"session":{"compaction":{"auto":true}}},"dry_run":true})]
        }
        "settings.delete" => {
            vec![json!({"path":"session.compaction.reserved_tokens","dry_run":true})]
        }
        "web.fetch" => vec![json!({"url":"https://example.com"})],
        "web.search" => vec![json!({"query":"example domain","limit":3})],
        "web.crawl" => vec![json!({"start_url":"https://example.com","max_pages":1})],
        "plan.get" => vec![json!({})],
        "plan.clear" => vec![json!({})],
        "plan.set" => vec![
            json!({"objective":"probe","title":"Probe Plan","steps":[{"id":"step_probe","title":"step one","status":"pending"}]}),
        ],
        "plan.update" => vec![json!({"phase":"completed"})],
        "snapshot.enter.new" => vec![json!({"name":"probe-snapshot"})],
        "snapshot.enter.existing" => vec![json!({"path":"."})],
        "snapshot.exit" => vec![json!({"action":"keep","discard_changes":false})],
        "memory.search" => vec![json!({"query":"probe","limit":3})],
        "memory.write" => {
            vec![json!({"name":"probe","content":"probe memory","description":"probe"})]
        }
        "memory.list" => vec![json!({})],
        "memory.get" => vec![json!({"name":"probe"})],
        "memory.delete" => vec![json!({"name":"probe"})],
        "session.get" => vec![json!({})],
        "session.rename" => vec![json!({"title":"probe-session"})],
        "agent.restore" => vec![json!({})],
        "agent.switch" => vec![json!({"agent":"planner","push_previous":true})],
        "schedule.list" => vec![json!({})],
        "schedule.create" => {
            vec![json!({"expression":"0 0 * * * *","prompt":"probe schedule","max_age_days":1})]
        }
        "schedule.delete" => vec![json!({"id":"probe"})],
        "schedule.wakeup" => {
            vec![json!({"delay_seconds":60,"prompt":"probe wakeup","reason":"probe"})]
        }
        "schema_lab.inspect" => vec![json!({"section":"identity","include_defaults":true})],
        "schema_lab.echo" => vec![json!({"label":"probe","payload":{"ok":true}})],
        "skills.bootstrap" | "skills.init" => vec![json!({})],
        "skills.review" => vec![json!({"args":"auth handlers"})],
        "skills.security-review" | "skills.security_review" => vec![json!({"args":"auth layer"})],
        "task.run" => vec![
            json!({"description":"probe task","prompt":"Read README.md and reply with its heading.","subagent_type":"explore","task_id":"probe-task"}),
        ],
        "user.request_input" => vec![
            json!({"questions":[{"id":"confirm","header":"Confirm","question":"continue?","options":[{"label":"yes","description":"continue"}]}]}),
        ],
        _ => Vec::new(),
    };

    cases
        .into_iter()
        .enumerate()
        .map(|(idx, value): (usize, Value)| (format!("manual-{idx}"), value.to_string()))
        .collect()
}

fn sanitize_value(tool_name: &str, key: Option<&str>, value: &mut Value) {
    match value {
        Value::Object(map) => {
            for (child_key, child_value) in map.iter_mut() {
                let key_name: &str = child_key.as_ref();
                sanitize_value(tool_name, Some(key_name), child_value);
            }
            apply_object_defaults(tool_name, map);
        }
        Value::Array(items) => {
            for item in items {
                sanitize_value(tool_name, key, item);
            }
        }
        Value::String(text) => {
            let replacement = sanitize_string(tool_name, key, text);
            *text = replacement;
        }
        _ => {}
    }
}

fn sanitize_string(tool_name: &str, key: Option<&str>, text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return default_string_for_key(key);
    }

    if trimmed.starts_with('<') && trimmed.ends_with('>') {
        return default_string_for_key(key);
    }

    match key {
        Some("path") | Some("file") | Some("file_path") => {
            if trimmed.contains("Cargo.toml")
                || trimmed.contains("main.rs")
                || trimmed.contains("lib.rs")
                || trimmed.contains("README")
            {
                trimmed.to_string()
            } else {
                if tool_name.starts_with("lsp.") {
                    "src/lib.rs".to_string()
                } else {
                    "src/main.rs".to_string()
                }
            }
        }
        Some("cwd") => ".".to_string(),
        Some("command") => "printf probe".to_string(),
        Some("pattern") => "probe".to_string(),
        Some("query") => "probe".to_string(),
        Some("url") => "https://example.com".to_string(),
        Some("tool") => "fs.read".to_string(),
        Some("process_id") | Some("id") => "probe".to_string(),
        Some("expression") => "0 0 * * * *".to_string(),
        Some("reason") => "probe".to_string(),
        Some("section") => "identity".to_string(),
        Some("label") => "probe".to_string(),
        Some("title") | Some("objective") | Some("description") | Some("prompt") => {
            "probe".to_string()
        }
        Some("key") if tool_name.starts_with("settings.") => "session.compaction.auto".to_string(),
        Some("name") if tool_name.starts_with("snapshot.") => "probe-snapshot".to_string(),
        Some("agent") if tool_name == "agent.switch" => "planner".to_string(),
        _ => trimmed.to_string(),
    }
}

fn default_string_for_key(key: Option<&str>) -> String {
    match key {
        Some("path") | Some("file") | Some("file_path") => "src/main.rs".to_string(),
        Some("cwd") => ".".to_string(),
        Some("command") => "printf probe".to_string(),
        Some("pattern") => "probe".to_string(),
        Some("query") => "probe".to_string(),
        Some("url") => "https://example.com".to_string(),
        Some("tool") => "fs.read".to_string(),
        Some("process_id") | Some("id") => "probe".to_string(),
        Some("expression") => "0 0 * * * *".to_string(),
        Some("key") => "session.compaction.auto".to_string(),
        Some("title") | Some("objective") | Some("description") | Some("prompt") => {
            "probe".to_string()
        }
        _ => "probe".to_string(),
    }
}

fn apply_object_defaults(tool_name: &str, map: &mut serde_json::Map<String, Value>) {
    match tool_name {
        "process.run" => {
            map.entry("command".to_string())
                .or_insert_with(|| Value::String("printf probe".to_string()));
            map.entry("description".to_string())
                .or_insert_with(|| Value::String("print probe".to_string()));
            map.entry("workdir".to_string())
                .or_insert_with(|| Value::String(".".to_string()));
            map.entry("filesystem_effects".to_string())
                .or_insert_with(|| Value::Array(Vec::new()));
            map.entry("network_effects".to_string())
                .or_insert_with(|| Value::Array(Vec::new()));
        }
        "fs.read" => {
            map.entry("path".to_string())
                .or_insert_with(|| Value::String("README.md".to_string()));
        }
        "fs.glob" => {
            map.entry("pattern".to_string())
                .or_insert_with(|| Value::String("src/**/*.rs".to_string()));
        }
        "fs.grep" => {
            map.entry("pattern".to_string())
                .or_insert_with(|| Value::String("probe".to_string()));
            map.entry("path".to_string())
                .or_insert_with(|| Value::String("src".to_string()));
        }
        "code.syntax_tree" => {
            map.entry("path".to_string())
                .or_insert_with(|| Value::String("src/main.rs".to_string()));
        }
        "code.search_ast" => {
            map.entry("path".to_string())
                .or_insert_with(|| Value::String("src/main.rs".to_string()));
            map.entry("pattern".to_string())
                .or_insert_with(|| Value::String("main".to_string()));
        }
        "lsp.definition" | "lsp.references" | "lsp.hover" => {
            map.entry("file_path".to_string())
                .or_insert_with(|| Value::String("src/lib.rs".to_string()));
            map.entry("line".to_string())
                .or_insert_with(|| Value::Number(1.into()));
            map.entry("character".to_string())
                .or_insert_with(|| Value::Number(4.into()));
            if tool_name == "lsp.references" {
                map.entry("include_declaration".to_string())
                    .or_insert_with(|| Value::Bool(true));
            }
        }
        "lsp.diagnostics" => {
            map.entry("file_path".to_string())
                .or_insert_with(|| Value::String("src/lib.rs".to_string()));
        }
        "process.logs" => {
            map.entry("process_id".to_string())
                .or_insert_with(|| Value::String("probe".to_string()));
            map.entry("since_seq".to_string())
                .or_insert_with(|| Value::Number(0.into()));
            map.entry("wait_ms".to_string())
                .or_insert_with(|| Value::Number(0.into()));
        }
        "process.stop" => {
            map.entry("process_id".to_string())
                .or_insert_with(|| Value::String("probe".to_string()));
        }
        _ => {}
    }
}

async fn run_probe_case(
    manager: &SessionManager,
    model: &ModelRef,
    case: ProbeCase,
) -> Result<ProbeOutcome, AppError> {
    let mut last = run_probe_case_once(manager, model, case.clone(), false).await?;
    for _ in 0..CASE_TOOL_RETRY_LIMIT {
        let should_retry = !last.ok;
        if !should_retry {
            return Ok(last);
        }
        last = run_probe_case_once(manager, model, case.clone(), true).await?;
    }
    Ok(last)
}

async fn run_probe_case_once(
    manager: &SessionManager,
    model: &ModelRef,
    case: ProbeCase,
    retry: bool,
) -> Result<ProbeOutcome, AppError> {
    let mut case = case;
    let timeout_secs = case_timeout_secs(case.tool_name.as_str());
    let mut options = SessionRunOptions::new(model.clone());
    options.max_output_tokens = Some(128);
    if !case.tool_name.starts_with("skills.") && case.tool_name != "fs.read" {
        options.temperature = Some(0.0);
    }
    let session = manager
        .create_session(SessionCreateRequest {
            title: format!("probe {}", case.tool_name),
            parent_session_id: None,
        })
        .await?;
    manager
        .set_session_allowed_tools(session.id, vec![case.tool_name.clone()])
        .await?;
    manager
        .set_session_permission(session.id, allow_all_permission())
        .await?;
    prepare_case_runtime_state(manager, session.id, &mut case).await?;

    let prompt = probe_prompt(&case, retry);
    let mut session = match tokio::time::timeout(
        std::time::Duration::from_secs(timeout_secs),
        manager.submit_user_message(SessionUserMessageRequest::new(
            session.id,
            options.clone(),
            vec![PartContent::text(prompt)],
        )),
    )
    .await
    {
        Ok(result) => result?,
        Err(_) => match recover_timed_out_session(manager, session.id).await? {
            Some(recovered) => recovered,
            None => {
                let _ = manager.cancel_active_run(session.id).await;
                return Ok(ProbeOutcome {
                    tool_name: case.tool_name,
                    label: case.label,
                    ok: false,
                    reason: Some("submit timeout".to_string()),
                });
            }
        },
    };

    for _ in 0..8 {
        if session.runtime().run.status != RunStatus::Blocked {
            break;
        }
        let Some(request) = session.pending_interactive_requests().into_iter().next() else {
            break;
        };
        if case.tool_name == "user.request_input" && request.as_user_input().is_some() {
            break;
        }
        session = match tokio::time::timeout(
            std::time::Duration::from_secs(timeout_secs),
            continue_blocked_probe(manager, &options, session.id, request),
        )
        .await
        {
            Ok(result) => result?,
            Err(_) => {
                let _ = manager.cancel_active_run(session.id).await;
                return Ok(ProbeOutcome {
                    tool_name: case.tool_name,
                    label: case.label,
                    ok: false,
                    reason: Some("continuation timeout".to_string()),
                });
            }
        };
    }

    let operations = observed_tool_names(&session);
    let mut failures = Vec::new();
    if session.runtime().run.status == RunStatus::Blocked
        && !(case.tool_name == "user.request_input" && has_pending_user_input(&session))
    {
        failures.push("still blocked".to_string());
    }
    let expected_names = acceptable_tool_names(case.tool_name.as_str());
    if !operations
        .iter()
        .any(|name| expected_names.iter().any(|expected| name == expected))
    {
        failures.push(format!(
            "expected tool call count {} (saw: {})",
            operations.len(),
            operations.join(", ")
        ));
        failures.push(format!("tool not called: {}", case.tool_name));
    }
    let assistant_text = latest_assistant_visible_text(&session);
    if case.tool_name != "user.request_input" && assistant_text.trim().is_empty() {
        failures.push("empty assistant reply".to_string());
    }
    if case.tool_name == "fs.read" && !assistant_text.contains(FS_READ_TOKEN) {
        failures.push("fs.read reply did not include runtime token".to_string());
    }

    let outcome = ProbeOutcome {
        tool_name: case.tool_name,
        label: case.label,
        ok: failures.is_empty(),
        reason: (!failures.is_empty()).then(|| failures.join("; ")),
    };
    if outcome.ok {
        println!("OK  {} [{}]", outcome.tool_name, outcome.label);
    } else {
        println!(
            "BAD {} [{}] {}",
            outcome.tool_name,
            outcome.label,
            outcome.reason.as_deref().unwrap_or("unknown failure")
        );
    }
    Ok(outcome)
}

fn case_timeout_secs(tool_name: &str) -> u64 {
    match tool_name {
        "user.request_input" => INTERACTIVE_CASE_TIMEOUT_SECS,
        _ => CASE_TIMEOUT_SECS,
    }
}

async fn prepare_case_runtime_state(
    manager: &SessionManager,
    session_id: i64,
    case: &mut ProbeCase,
) -> Result<(), AppError> {
    if !matches!(case.tool_name.as_str(), "process.logs" | "process.stop") {
        return Ok(());
    }

    let invocation = ToolPayloadInput::Process(ProcessToolInput::Run {
        shell: ProcessShell::Bash,
        command: ShellCommandInput {
            command: "printf probe-log && sleep 5".to_string(),
            description: "probe background process".to_string(),
            timeout_ms: None,
            workdir: Some(".".to_string()),
            filesystem_effects: Vec::new(),
            network_effects: Vec::new(),
        },
        background: true,
    })
    .into_invocation();
    let execution = manager
        .tool_executor()
        .execute_invocation_detailed(&invocation, session_id, -1)
        .map_err(|err| {
            AppError::Config(format!("probe setup failed for {}: {err}", case.tool_name))
        })?;
    let payload = execution
        .output
        .to_json_payload()
        .ok_or_else(|| AppError::Config("probe setup returned empty process output".to_string()))?;
    let process_id = payload
        .get("process_id")
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::Config("probe setup did not return process_id".to_string()))?
        .to_string();

    if let Some(map) = case.input.as_object_mut() {
        map.insert("process_id".to_string(), Value::String(process_id));
    }
    Ok(())
}

async fn recover_timed_out_session(
    manager: &SessionManager,
    session_id: i64,
) -> Result<Option<Session>, AppError> {
    for _ in 0..10 {
        let session = manager.get_session(session_id).await?;
        let status = session.runtime().run.status;
        if matches!(status, RunStatus::Blocked | RunStatus::Idle) {
            return Ok(Some(session));
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    Ok(None)
}

fn acceptable_tool_names(tool_name: &str) -> Vec<&str> {
    match tool_name {
        "tool.help" => vec!["tool.help", "tools.help"],
        "tool_catalog" => vec!["tool_catalog", "tools.search"],
        "skills.bootstrap" => vec!["skills.bootstrap", "skills.init"],
        "skills.security-review" => vec!["skills.security-review", "skills.security_review"],
        other => vec![other],
    }
}

fn probe_prompt(case: &ProbeCase, retry: bool) -> String {
    let payload = serde_json::to_string_pretty(&case.input).unwrap_or_else(|_| "{}".to_string());
    let task = probe_task_hint(case.tool_name.as_str());
    let alias_guard = probe_alias_guard(case.tool_name.as_str());
    let completion = probe_completion_instruction(case.tool_name.as_str());
    if retry {
        return format!(
            "这是失败重试。上一轮你没有按要求完成。现在只做这一件事：{}。你必须在第一条 assistant 响应里先调用工具 `{}`。参数对象必须与下面 JSON 完全一致。不要调用其他工具，不要先解释，不要先输出文本。{}{}\n\n```json\n{}\n```",
            task, case.tool_name, alias_guard, completion, payload
        );
    }

    format!(
        "这是协议探测。请完成这件事：{}。你必须调用工具 `{}`。参数对象必须与下面 JSON 完全一致。不要调用其他工具。{}{}\n\n```json\n{}\n```",
        task, case.tool_name, alias_guard, completion, payload
    )
}

fn probe_task_hint(tool_name: &str) -> &'static str {
    match tool_name {
        "fs.read" => "读取目标文件并返回文件首行",
        "process.run" => "运行一个最小 shell 命令并返回结果",
        "settings.get" => "读取一个 Agena 配置项",
        "settings.set" => "设置一个 Agena 配置项",
        "settings.delete" => "删除一个 Agena 配置项",
        "settings.validate" => "校验当前 Agena 配置",
        "skills.bootstrap" | "skills.init" => "生成初始化 AGENA.md 的项目引导提示",
        "skills.review" => "生成当前分支代码评审提示",
        "skills.security-review" | "skills.security_review" => "生成当前分支安全评审提示",
        "snapshot.exit" => "退出当前 snapshot",
        "tool.help" => "读取一个工具的详细帮助",
        "user.request_input" => "向用户发起一个结构化确认问题",
        _ => "按要求调用目标工具",
    }
}

fn probe_completion_instruction(tool_name: &str) -> &'static str {
    match tool_name {
        "fs.read" => "工具结束后只回复读取到的文件首行，不要回复 OK。",
        _ => "工具结束后只回复 OK。",
    }
}

fn probe_alias_guard(tool_name: &str) -> &'static str {
    match tool_name {
        "tool.help" => "工具名必须精确是 `tool.help`，不要改成 `tools.help` 或 `tool_catalog`。",
        "skills.bootstrap" => {
            "工具名必须精确是 `skills.bootstrap`，不要改成 `skills.init`、`skills.review`、`skills.security-review` 或 `skills.security_review`。"
        }
        "skills.init" => {
            "工具名必须精确是 `skills.init`，不要改成 `skills.bootstrap`、`skills.review`、`skills.security-review` 或 `skills.security_review`。"
        }
        "skills.review" => {
            "工具名必须精确是 `skills.review`，不要改成 `skills.bootstrap`、`skills.init`、`skills.security-review` 或 `skills.security_review`。"
        }
        "skills.security-review" => {
            "工具名必须精确是 `skills.security-review`，不要改成 `skills.security_review`、`skills.review`、`skills.bootstrap` 或 `skills.init`。"
        }
        "skills.security_review" => {
            "工具名必须精确是 `skills.security_review`，不要改成 `skills.security-review`、`skills.review`、`skills.bootstrap` 或 `skills.init`。"
        }
        _ => "",
    }
}

async fn continue_blocked_probe(
    manager: &SessionManager,
    options: &SessionRunOptions,
    session_id: i64,
    request: PendingInteractiveRequest,
) -> Result<Session, AppError> {
    if let Some(permission) = request.as_permission() {
        return manager
            .reply_permission(SessionPermissionReplyRequest::new(
                session_id,
                options.clone(),
                PermissionReply {
                    request_id: permission.request_id.clone(),
                    kind: PermissionReplyKind::AllowOnce,
                    reason: None,
                    scope: None,
                },
                Some("dsv4f_probe".to_string()),
            ))
            .await;
    }

    let user_input = request
        .as_user_input()
        .ok_or_else(|| AppError::Config("unsupported pending request".to_string()))?;
    let mut answers = BTreeMap::new();
    for question in &user_input.questions {
        let answer = question
            .options
            .first()
            .map(|option| option.label.clone())
            .unwrap_or_else(|| "probe".to_string());
        answers.insert(question.id.clone(), vec![answer]);
    }
    manager
        .reply_user_input(SessionExecutionReplyRequest::new(
            session_id,
            options.clone(),
            UserInputReply {
                request_id: user_input.request_id.clone(),
                kind: UserInputReplyKind::Submit,
                answers,
                reason: None,
            },
        ))
        .await
}

fn observed_tool_names(session: &Session) -> Vec<String> {
    let mut names = Vec::new();
    for message in &session.messages {
        if message.role != Role::Assistant {
            continue;
        }
        for part in &message.parts {
            let Some(PartContent::Operation(operation)) = part.content.as_ref() else {
                continue;
            };
            if operation.is_provider_native_only() {
                continue;
            }
            names.push(operation.invocation.name.clone());
        }
    }
    names
}

fn latest_assistant_visible_text(session: &Session) -> String {
    session
        .messages
        .iter()
        .rev()
        .find(|message| message.role == Role::Assistant)
        .map(|message| message.visible_text_lossy())
        .unwrap_or_default()
}

fn has_pending_user_input(session: &Session) -> bool {
    session
        .pending_interactive_requests()
        .into_iter()
        .any(|request| request.as_user_input().is_some())
}

fn allow_all_permission() -> PermissionConfig {
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
            rules: Default::default(),
        }),
        network: Some(NetworkPermissionConfig {
            internet: Some(PermissionMode::Allow),
            private: Some(PermissionMode::Allow),
            loopback: Some(PermissionMode::Allow),
            rules: Default::default(),
        }),
        tools: Some(ToolPermissionConfig {
            default: Some(PermissionMode::Allow),
            tags: Default::default(),
            names: Default::default(),
            plugin: Default::default(),
            rules: Default::default(),
        }),
    }
}
