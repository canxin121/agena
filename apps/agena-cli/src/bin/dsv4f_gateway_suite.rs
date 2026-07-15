//! Exhaustive real-provider regression suite for the Cline dsv4f gateway.
//!
//! The suite intentionally uses the public session/model path.  Each real
//! plugin target is discovered with `tools_help` and then invoked with
//! `tools_call` by the configured Cline model; it never bypasses the gateway
//! by calling plugin implementations directly.

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
    time::Duration,
};

use agena::{
    config::LoadConfigRequest,
    message::OperationPart,
    model::ModelRef,
    permission::PermissionReplyKind,
    runtime::{AgenaRuntime, AgenaRuntimeConfig},
    session::{Session, SessionManager, SessionRunOptions},
    tool,
};
use anyhow::{Context, ensure};
use clap::Parser;
use serde_json::{Value, json};
use tempfile::TempDir;

mod dsv4f_gateway_suite_cases;
mod dsv4f_gateway_suite_support;

use self::dsv4f_gateway_suite_cases::*;
use self::dsv4f_gateway_suite_support::*;

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
            "You are a deterministic integration-test driver. When a user gives an exact provider tool invocation JSON, copy every key and value exactly, including optional fields. Never make a preliminary/default tool call, never omit supplied fields, and never retry a completed tool call unless the user explicitly asks for a retry."
                .to_string(),
        ),
        temperature: Some(0.0),
        max_output_tokens: Some(1_024),
        agent_profile: None,
    }
}

fn assert_gateway_surface(manager: &SessionManager) -> anyhow::Result<()> {
    let specs = tool::gateway_function_specs(&manager.tool_executor().available_gateway_tools());
    let mut names = specs
        .into_iter()
        .map(|spec| spec.protocol_name)
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
    let toolchains = std_env::var_os("RUSTUP_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std_env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("/nonexistent"))
                .join(".rustup")
        })
        .join("toolchains");
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

#[cfg(test)]
mod tests {
    use super::{Path, PathBuf, rust_toolchain_environment_for};

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
