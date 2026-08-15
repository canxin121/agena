#![cfg(unix)]

use std::{
    collections::VecDeque,
    fs::OpenOptions,
    io::{Read as _, Write as _},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command as ProcessCommand, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

use agena_api::{
    commands::{Command, CommandResult, ResolveWorkspaceParams, SubmitRunParams},
    resource::{ServerEndpointRecord, RunOptions, SessionExecutionResource, SessionState},
};
use agena_client::AgenaClient;
use portable_pty::{CommandBuilder, MasterPty, PtySize, native_pty_system};
use serde::Deserialize;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::mpsc,
};

struct ServerProcess {
    child: Child,
    log_path: PathBuf,
}

impl ServerProcess {
    fn pid(&self) -> u32 {
        self.child.id()
    }

    fn crash(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    fn log(&self) -> String {
        std::fs::read_to_string(&self.log_path).unwrap_or_default()
    }
}

impl Drop for ServerProcess {
    fn drop(&mut self) {
        self.crash();
    }
}

const RUNTIME_OWNERSHIP_AUDIT_ENV: &str = "AGENA_RUNTIME_OWNERSHIP_AUDIT_PATH";
const RUNTIME_BOOTSTRAP_FORBIDDEN_ENV: &str = "AGENA_RUNTIME_BOOTSTRAP_FORBIDDEN";

#[derive(Clone, Copy)]
enum StdioThinClientKind {
    RpcServer,
    McpServer,
}

struct StdioThinClient {
    child: Child,
    stdin: Option<ChildStdin>,
    log_path: PathBuf,
}

impl StdioThinClient {
    fn write_line(&mut self, value: serde_json::Value) {
        let stdin = self.stdin.as_mut().expect("thin client stdin is open");
        serde_json::to_writer(&mut *stdin, &value).expect("write thin-client request");
        stdin
            .write_all(b"\n")
            .expect("terminate thin-client request");
        stdin.flush().expect("flush thin-client request");
    }

    fn log(&self) -> String {
        std::fs::read_to_string(&self.log_path).unwrap_or_default()
    }

    fn assert_running(&mut self, label: &str) {
        let status = self.child.try_wait().expect("poll thin client");
        assert!(
            status.is_none(),
            "{label} exited unexpectedly with {status:?}; log:\n{}",
            self.log()
        );
    }

    fn close(mut self, label: &str) {
        drop(self.stdin.take());
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(status) = self.child.try_wait().expect("poll closing thin client") {
                assert!(
                    status.success(),
                    "{label} failed while closing stdio: {status:?}; log:\n{}",
                    self.log()
                );
                return;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "{label} did not exit after stdio EOF; log:\n{}",
                self.log()
            );
            std::thread::sleep(Duration::from_millis(25));
        }
    }
}

impl Drop for StdioThinClient {
    fn drop(&mut self) {
        drop(self.stdin.take());
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

fn spawn_stdio_thin_client(
    kind: StdioThinClientKind,
    server_url: &str,
    workspace: &Path,
    audit_path: &Path,
    log_path: &Path,
) -> StdioThinClient {
    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
        .expect("open stdio thin-client log");
    let stderr = log.try_clone().expect("clone stdio thin-client log");
    let mut command = ProcessCommand::new(env!("CARGO_BIN_EXE_agena"));
    command
        .arg("--server")
        .arg(server_url)
        .current_dir(workspace)
        .env(RUNTIME_OWNERSHIP_AUDIT_ENV, audit_path)
        .env(RUNTIME_BOOTSTRAP_FORBIDDEN_ENV, "1")
        .env_remove("AGENA_SERVER_TOKEN")
        .env_remove("AGENA_SERVER_PASSWORD")
        .env_remove("AGENA_DATABASE_URL")
        .env_remove("AGENA_DATABASE_PATH")
        .stdin(Stdio::piped())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(stderr));
    match kind {
        StdioThinClientKind::RpcServer => {
            command.arg("rpc-server").arg("--workspace").arg(workspace);
        }
        StdioThinClientKind::McpServer => {
            command.arg("mcp-server").arg("--workspace").arg(workspace);
        }
    }
    let mut child = command.spawn().expect("spawn stdio thin client");
    let stdin = child.stdin.take().expect("capture stdio thin-client stdin");
    StdioThinClient {
        child,
        stdin: Some(stdin),
        log_path: log_path.to_owned(),
    }
}

struct PtyThinClient {
    child: Box<dyn portable_pty::Child + Send>,
    master: Option<Box<dyn MasterPty + Send>>,
    output: Arc<Mutex<Vec<u8>>>,
    stop_reader: Arc<AtomicBool>,
    reader: Option<thread::JoinHandle<()>>,
}

impl PtyThinClient {
    fn output(&self) -> String {
        let bytes = self.output.lock().expect("lock TUI output");
        String::from_utf8_lossy(&bytes).into_owned()
    }

    fn assert_running(&mut self) {
        let status = self.child.try_wait().expect("poll TUI thin client");
        assert!(
            status.is_none(),
            "remote TUI exited unexpectedly with {status:?}; output:\n{}",
            self.output()
        );
    }
}

impl Drop for PtyThinClient {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
        self.stop_reader.store(true, Ordering::Release);
        drop(self.master.take());
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

fn spawn_remote_tui(
    server_url: &str,
    workspace: &Path,
    session_id: i64,
    audit_path: &Path,
) -> PtyThinClient {
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 30,
            cols: 120,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("open TUI integration PTY");
    let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_agena"));
    command.arg("--server");
    command.arg(server_url);
    command.arg("tui");
    command.arg("--workspace");
    command.arg(workspace);
    command.arg("--session");
    command.arg(session_id.to_string());
    command.cwd(workspace);
    command.env("TERM", "xterm-256color");
    command.env("COLORTERM", "truecolor");
    command.env(RUNTIME_OWNERSHIP_AUDIT_ENV, audit_path);
    command.env(RUNTIME_BOOTSTRAP_FORBIDDEN_ENV, "1");
    for name in [
        "AGENA_SERVER_TOKEN",
        "AGENA_SERVER_PASSWORD",
        "AGENA_DATABASE_URL",
        "AGENA_DATABASE_PATH",
    ] {
        command.env_remove(name);
    }
    let child = pair
        .slave
        .spawn_command(command)
        .expect("spawn remote TUI in PTY");
    drop(pair.slave);
    let mut reader = pair
        .master
        .try_clone_reader()
        .expect("clone remote TUI PTY reader");
    let mut writer = pair
        .master
        .take_writer()
        .expect("take remote TUI PTY writer");
    let output = Arc::new(Mutex::new(Vec::new()));
    let reader_output = Arc::clone(&output);
    let stop_reader = Arc::new(AtomicBool::new(false));
    let reader_stop = Arc::clone(&stop_reader);
    let reader = thread::spawn(move || {
        let mut buffer = [0_u8; 8192];
        loop {
            let read = match reader.read(&mut buffer) {
                Ok(read) => read,
                Err(error)
                    if !reader_stop.load(Ordering::Acquire)
                        && (error.kind() == std::io::ErrorKind::Interrupted
                            || error.kind() == std::io::ErrorKind::WouldBlock
                            || error.raw_os_error() == Some(libc::EIO)) =>
                {
                    std::thread::sleep(Duration::from_millis(10));
                    continue;
                }
                Err(_) => return,
            };
            if read == 0 {
                return;
            }
            let chunk = &buffer[..read];
            if chunk.windows(4).any(|window| window == b"\x1b[6n") {
                let _ = writer.write_all(b"\x1b[1;1R");
                let _ = writer.flush();
            }
            if chunk.windows(6).any(|window| window == b"\x1b]11;?") {
                let _ = writer.write_all(b"\x1b]11;rgb:1010/1010/1010\x1b\\");
                let _ = writer.flush();
            }
            let mut output = reader_output.lock().expect("lock remote TUI output");
            output.extend_from_slice(chunk);
            if output.len() > 2 * 1024 * 1024 {
                let remove = output.len() - 1024 * 1024;
                output.drain(..remove);
            }
        }
    });
    PtyThinClient {
        child,
        master: Some(pair.master),
        output,
        stop_reader,
        reader: Some(reader),
    }
}

#[derive(Debug, Deserialize)]
struct RuntimeOwnershipAuditRecord {
    schema: u32,
    pid: u32,
    workspace_root: PathBuf,
    components: Vec<String>,
}

fn runtime_ownership_records(path: &Path) -> Vec<RuntimeOwnershipAuditRecord> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).expect("decode Runtime ownership audit record"))
        .collect()
}

async fn wait_for_process_output(process: &mut StdioThinClient, needle: &str, label: &str) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        process.assert_running(label);
        if process.log().contains(needle) {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "{label} did not produce `{needle}`; log:\n{}",
            process.log()
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

fn spawn_server(
    workspace: &Path,
    database_path: &Path,
    record_path: &Path,
    server_data_dir: &Path,
    log_path: &Path,
    port: u16,
    extra_environment: &[(&str, &Path)],
) -> ServerProcess {
    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
        .expect("open server integration log");
    let stderr = log.try_clone().expect("clone server integration log");
    let mut command = ProcessCommand::new(env!("CARGO_BIN_EXE_agena"));
    command
        .arg("--database-path")
        .arg(database_path)
        .arg("server")
        .arg("--host")
        .arg("127.0.0.1")
        .arg("--port")
        .arg(port.to_string())
        .arg("--workspace")
        .arg(workspace)
        .env("AGENA_SERVER_RECORD", record_path)
        .env("AGENA_SERVER_DATA_DIR", server_data_dir)
        .env_remove("AGENA_SERVER_UI_PASSWORD")
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(stderr));
    for (name, value) in extra_environment {
        command.env(name, value);
    }
    let child = command.spawn().expect("spawn foreground server");
    ServerProcess {
        child,
        log_path: log_path.to_owned(),
    }
}

async fn wait_for_server(
    process: &ServerProcess,
    record_path: &Path,
) -> (AgenaClient, agena_api::resource::ServerIdentityResource) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        if let Ok(bytes) = std::fs::read(record_path)
            && let Ok(record) = serde_json::from_slice::<ServerEndpointRecord>(&bytes)
            && record.pid == process.pid()
            && let Ok(client) = AgenaClient::new(record.url.as_str())
            && let Ok(identity) = client.server_identity().await
            && identity.pid == process.pid()
            && identity.id == record.server_id
        {
            return (client, identity);
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "server {} did not become ready; log:\n{}",
            process.pid(),
            process.log()
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn read_http_json_request(stream: &mut TcpStream) -> (String, serde_json::Value) {
    let mut request = Vec::new();
    let mut buffer = [0_u8; 4096];
    loop {
        let read = stream
            .read(&mut buffer)
            .await
            .expect("read provider request");
        assert!(read > 0, "provider request closed before headers");
        request.extend_from_slice(&buffer[..read]);
        let Some(header_end) = request
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .map(|index| index + 4)
        else {
            continue;
        };
        let headers = String::from_utf8_lossy(&request[..header_end]);
        let request_line = headers.lines().next().unwrap_or_default().trim().to_owned();
        let content_length = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().ok())
                    .flatten()
            })
            .unwrap_or_default();
        while request.len() < header_end + content_length {
            let read = stream.read(&mut buffer).await.expect("read provider body");
            assert!(read > 0, "provider request closed before body");
            request.extend_from_slice(&buffer[..read]);
        }
        let body = if content_length == 0 {
            serde_json::Value::Null
        } else {
            serde_json::from_slice(&request[header_end..header_end + content_length])
                .expect("decode provider request JSON")
        };
        return (request_line, body);
    }
}

async fn spawn_crash_recovery_provider() -> (
    String,
    mpsc::UnboundedReceiver<serde_json::Value>,
    tokio::task::JoinHandle<()>,
) {
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("bind crash-recovery fake provider");
    let address = listener.local_addr().expect("fake provider address");
    let (request_tx, request_rx) = mpsc::unbounded_channel();
    let server = tokio::spawn(async move {
        let mut post_index = 0_u64;
        loop {
            let (mut stream, _) = listener.accept().await.expect("accept provider request");
            let (request_line, request) = read_http_json_request(&mut stream).await;
            if !request_line.starts_with("POST /v1/responses ") {
                let body = serde_json::json!({
                    "object": "list",
                    "data": [{"id": "fake-model", "object": "model"}]
                })
                .to_string();
                let response = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                stream
                    .write_all(response.as_bytes())
                    .await
                    .expect("write discovery response");
                let _ = stream.shutdown().await;
                continue;
            }

            post_index += 1;
            request_tx
                .send(request)
                .expect("record fake provider request");
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncache-control: no-cache\r\nconnection: close\r\n\r\n",
                )
                .await
                .expect("write provider SSE headers");
            if post_index == 1 {
                // Keep the original model turn in flight until the server is
                // killed. EOF proves the provider socket belonged to the
                // crashed process and lets this loop serve the restarted one.
                let mut byte = [0_u8; 1];
                loop {
                    match stream.read(&mut byte).await {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {}
                    }
                }
                continue;
            }

            let events = VecDeque::from([
                serde_json::json!({
                    "type": "response.output_text.delta",
                    "delta": "recovered after server restart"
                }),
                serde_json::json!({
                    "type": "response.completed",
                    "response": {
                        "id": "resp_restart_recovered",
                        "status": "completed",
                        "usage": {
                            "input_tokens": 4,
                            "output_tokens": 3,
                            "total_tokens": 7
                        }
                    }
                }),
            ]);
            for event in events {
                let frame = format!("data: {event}\n\n");
                if stream.write_all(frame.as_bytes()).await.is_err() {
                    break;
                }
            }
            let _ = stream.shutdown().await;
        }
    });
    (format!("http://{address}/v1"), request_rx, server)
}

fn write_isolated_project_config(workspace: &Path, provider_base_url: &str) {
    let config_dir = workspace.join(".agena");
    std::fs::create_dir_all(&config_dir).expect("create project config directory");
    let config = serde_json::json!({
        "providers": {
            "default": "fake",
            "default_selection": {
                "provider": "fake",
                "adapter": "openai_responses",
                "model": "fake-model"
            },
            "fake": {
                "defaults": {
                    "adapter": "openai_responses",
                    "model": "fake-model"
                },
                "auth": {
                    "mode": "api",
                    "subtype": "custom",
                    "base_url": provider_base_url,
                    "api_key": {"kind": "inline", "value": "fake-process-test-key"}
                },
                "adapters": {
                    "openai_responses": {
                        "enabled": true,
                        "models": {
                            "fake-model": {
                                "agena_tools": {"mode": "provider_protocol"}
                            }
                        }
                    }
                }
            }
        }
    });
    std::fs::write(
        config_dir.join("agena.json"),
        serde_json::to_vec_pretty(&config).expect("encode project config"),
    )
    .expect("write project config");
}

async fn wait_for_execution(
    client: &AgenaClient,
    session_id: i64,
    predicate: impl Fn(&SessionExecutionResource) -> bool,
) -> SessionExecutionResource {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    loop {
        let execution = client
            .get_session_state(session_id)
            .await
            .expect("read process-test session state");
        if predicate(&execution) {
            return execution;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for session {session_id}: {execution:#?}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn execution_text(execution: &SessionExecutionResource) -> String {
    execution
        .parts
        .iter()
        .filter_map(|part| part.content.get("text").and_then(serde_json::Value::as_str))
        .collect::<Vec<_>>()
        .join("")
}

fn unused_loopback_port() -> u16 {
    let listener =
        std::net::TcpListener::bind(("127.0.0.1", 0)).expect("reserve server restart auth port");
    listener
        .local_addr()
        .expect("reserved server restart auth address")
        .port()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn killed_server_restarts_with_interrupted_then_reconciled_session() {
    let fixture = tempfile::tempdir().expect("create server restart fixture");
    let workspace = fixture.path().join("workspace");
    let server_data = fixture.path().join("server-data");
    let database_path = fixture.path().join("sessions.db");
    let record_path = fixture.path().join("server.json");
    let log_path = fixture.path().join("server.log");
    std::fs::create_dir_all(&workspace).expect("create process-test workspace");
    std::fs::create_dir_all(&server_data).expect("create process-test server data");

    let (provider_url, mut provider_requests, provider) = spawn_crash_recovery_provider().await;
    write_isolated_project_config(&workspace, provider_url.as_str());

    let mut first_server = spawn_server(
        &workspace,
        &database_path,
        &record_path,
        &server_data,
        &log_path,
        0,
        &[],
    );
    let (client_a, first_identity) = wait_for_server(&first_server, &record_path).await;
    let status = client_a
        .runtime_status()
        .await
        .expect("read first server runtime status");
    assert_eq!(
        status
            .default_selection
            .as_ref()
            .and_then(|selection| selection.provider.as_deref()),
        Some("fake"),
        "test isolation failed: refusing to submit through a non-fake provider"
    );
    let workspace_result = client_a
        .command(Command::ResolveWorkspace(ResolveWorkspaceParams {
            path: workspace.to_string_lossy().into_owned(),
            create_if_missing: true,
        }))
        .await
        .expect("resolve server restart workspace");
    let CommandResult::Workspace(workspace_resource) = workspace_result else {
        panic!("server returned the wrong workspace result");
    };
    let session = client_a
        .create_session(workspace_resource.id, "crash recovery", None)
        .await
        .expect("create crash-recovery session");
    let submitted = client_a
        .submit_message(SubmitRunParams {
            session_id: session.id,
            options: RunOptions::default(),
            document: agena_domain::ComposerDocument(vec![agena_domain::ComposerNode::Text {
                text: "hold until the server crashes".to_owned(),
            }]),
        })
        .await
        .expect("submit hanging fake-provider run");
    tokio::time::timeout(Duration::from_secs(10), provider_requests.recv())
        .await
        .expect("fake provider receives hanging request")
        .expect("hanging provider request exists");
    let running = wait_for_execution(&client_a, session.id, |execution| {
        execution.session.state == SessionState::Running && execution.active_execution.is_some()
    })
    .await;
    assert_eq!(
        running
            .active_execution
            .as_ref()
            .map(|active| active.execution_id),
        submitted
            .active_execution
            .as_ref()
            .map(|active| active.execution_id)
    );

    first_server.crash();
    drop(client_a);

    // Advance only the durable lease clock. This is equivalent to waiting
    // past LEASE_STALENESS_MS but keeps the process-level regression fast.
    let database_url = format!("sqlite://{}?mode=rwc", database_path.display());
    let pool = sqlx::SqlitePool::connect(database_url.as_str())
        .await
        .expect("open killed server database");
    let aged =
        sqlx::query("UPDATE agena_execution_leases SET heartbeat_at_ms = heartbeat_at_ms - 60000")
            .execute(&pool)
            .await
            .expect("age killed server lease");
    assert_eq!(aged.rows_affected(), 1, "one hanging lease must be durable");
    pool.close().await;

    let mut second_server = spawn_server(
        &workspace,
        &database_path,
        &record_path,
        &server_data,
        &log_path,
        0,
        &[],
    );
    let (client_b, second_identity) = wait_for_server(&second_server, &record_path).await;
    assert_ne!(first_identity.id, second_identity.id);
    assert_ne!(first_identity.pid, second_identity.pid);

    let overview = client_b
        .session_overview(Some(workspace_resource.id), 10)
        .await
        .expect("read restarted server overview");
    assert!(
        overview.running.iter().all(|item| item.id != session.id),
        "a stale lease must never remain visible as running after restart"
    );
    assert!(
        overview
            .attention
            .iter()
            .any(|item| item.id == session.id && item.state == SessionState::Interrupted),
        "the restarted server must publish the stale run as interrupted before opening it"
    );

    let reconciled = client_b
        .get_session_state(session.id)
        .await
        .expect("open and reconcile interrupted session");
    assert_eq!(reconciled.session.state, SessionState::Ready);
    assert!(reconciled.active_execution.is_none());
    assert!(reconciled.parts.iter().any(|part| {
        part.kind == "run"
            && part.role == "assistant"
            && part.state == "failed"
            && part
                .content
                .get("abort_reason")
                .and_then(serde_json::Value::as_str)
                == Some("process_restart")
    }));

    client_b
        .continue_run(session.id, RunOptions::default())
        .await
        .expect("explicitly continue reconciled session");
    tokio::time::timeout(Duration::from_secs(10), provider_requests.recv())
        .await
        .expect("fake provider receives explicit recovery request")
        .expect("recovery provider request exists");
    let completed = wait_for_execution(&client_b, session.id, |execution| {
        execution.session.state == SessionState::Ready
            && execution.active_execution.is_none()
            && execution_text(execution).contains("recovered after server restart")
    })
    .await;
    assert!(completed.pending_interactive_requests.is_empty());
    assert_eq!(
        completed
            .parts
            .iter()
            .filter(|part| part.kind == "run" && part.role == "assistant")
            .count(),
        2,
        "the interrupted run and explicit recovery run must remain distinct"
    );

    second_server.crash();
    provider.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn simultaneous_web_tui_cli_ide_clients_leave_runtime_ownership_in_server() {
    let fixture = tempfile::tempdir().expect("create process-ownership fixture");
    let workspace = fixture.path().join("workspace");
    let server_data = fixture.path().join("server-data");
    let database_path = fixture.path().join("sessions.db");
    let scheduler_database_path = fixture.path().join("scheduler.db");
    let record_path = fixture.path().join("server.json");
    let audit_path = fixture.path().join("runtime-ownership.jsonl");
    let server_log_path = fixture.path().join("server.log");
    let rpc_log_path = fixture.path().join("rpc-server.log");
    let mcp_log_path = fixture.path().join("mcp-server.log");
    std::fs::create_dir_all(&workspace).expect("create ownership-test workspace");
    std::fs::create_dir_all(&server_data).expect("create ownership-test server data");

    let (provider_url, mut provider_requests, provider) = spawn_crash_recovery_provider().await;
    write_isolated_project_config(&workspace, provider_url.as_str());

    let server_environment = [
        (RUNTIME_OWNERSHIP_AUDIT_ENV, audit_path.as_path()),
        (
            "AGENA_SCHEDULER_DATABASE_PATH",
            scheduler_database_path.as_path(),
        ),
    ];
    let mut server = spawn_server(
        &workspace,
        &database_path,
        &record_path,
        &server_data,
        &server_log_path,
        0,
        &server_environment,
    );
    let (web_client, server_identity) = wait_for_server(&server, &record_path).await;
    let status = web_client
        .runtime_status()
        .await
        .expect("read process-ownership Runtime status");
    assert_eq!(
        status
            .default_selection
            .as_ref()
            .and_then(|selection| selection.provider.as_deref()),
        Some("fake"),
        "test isolation failed: refusing to submit through a non-fake provider"
    );

    let workspace_result = web_client
        .command(Command::ResolveWorkspace(ResolveWorkspaceParams {
            path: workspace.to_string_lossy().into_owned(),
            create_if_missing: true,
        }))
        .await
        .expect("resolve process-ownership workspace");
    let CommandResult::Workspace(workspace_resource) = workspace_result else {
        panic!("server returned the wrong ownership workspace result");
    };
    let title = "ownership-process-gate";
    let session = web_client
        .create_session(workspace_resource.id, title, None)
        .await
        .expect("create process-ownership session");
    let submitted = web_client
        .submit_message(SubmitRunParams {
            session_id: session.id,
            options: RunOptions::default(),
            document: agena_domain::ComposerDocument(vec![agena_domain::ComposerNode::Text {
                text: "keep the server-owned execution running while clients attach".to_owned(),
            }]),
        })
        .await
        .expect("submit server-owned process-ownership run");
    tokio::time::timeout(Duration::from_secs(10), provider_requests.recv())
        .await
        .expect("fake provider receives ownership-test request")
        .expect("ownership-test provider request exists");
    let running = wait_for_execution(&web_client, session.id, |execution| {
        execution.session.state == SessionState::Running && execution.active_execution.is_some()
    })
    .await;
    assert_eq!(
        running
            .active_execution
            .as_ref()
            .map(|active| active.execution_id),
        submitted
            .active_execution
            .as_ref()
            .map(|active| active.execution_id)
    );

    // This snapshot-plus-SSE attachment uses the same public transport as the
    // Web conversation runtime and remains connected while the native clients
    // below start. The submitted HTTP request itself is already detached from
    // the server-owned execution task.
    let web_connection = web_client
        .connect_session(session.id)
        .await
        .expect("attach Web-style session snapshot plus SSE");
    assert_eq!(web_connection.snapshot.session.state, SessionState::Running);

    let server_url = record_path
        .exists()
        .then(|| {
            std::fs::read(&record_path)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<ServerEndpointRecord>(&bytes).ok())
                .map(|record| record.url)
        })
        .flatten()
        .expect("read ownership-test server URL");
    let mut tui = spawn_remote_tui(server_url.as_str(), &workspace, session.id, &audit_path);
    tokio::time::sleep(Duration::from_millis(1_500)).await;
    tui.assert_running();

    let mut rpc = spawn_stdio_thin_client(
        StdioThinClientKind::RpcServer,
        server_url.as_str(),
        &workspace,
        &audit_path,
        &rpc_log_path,
    );
    rpc.write_line(serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "sessions/list",
        "params": {"offset": 0, "limit": 20}
    }));
    wait_for_process_output(&mut rpc, title, "IDE rpc-server").await;

    let mut mcp = spawn_stdio_thin_client(
        StdioThinClientKind::McpServer,
        server_url.as_str(),
        &workspace,
        &audit_path,
        &mcp_log_path,
    );
    mcp.write_line(serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {"name": "agena-ownership-test", "version": "1.0"}
        }
    }));
    wait_for_process_output(&mut mcp, "protocolVersion", "MCP stdio bridge").await;
    mcp.write_line(serde_json::json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized",
        "params": {}
    }));

    let cli = ProcessCommand::new(env!("CARGO_BIN_EXE_agena"))
        .arg("--server")
        .arg(server_url.as_str())
        .arg("sessions")
        .arg("list")
        .arg("--format")
        .arg("json")
        .current_dir(&workspace)
        .env(RUNTIME_OWNERSHIP_AUDIT_ENV, &audit_path)
        .env(RUNTIME_BOOTSTRAP_FORBIDDEN_ENV, "1")
        .env_remove("AGENA_SERVER_TOKEN")
        .env_remove("AGENA_SERVER_PASSWORD")
        .env_remove("AGENA_DATABASE_URL")
        .env_remove("AGENA_DATABASE_PATH")
        .output()
        .expect("run one-shot CLI beside other clients");
    assert!(
        cli.status.success(),
        "one-shot CLI failed: {}",
        String::from_utf8_lossy(&cli.stderr)
    );
    assert!(
        String::from_utf8_lossy(&cli.stdout).contains(title),
        "one-shot CLI did not observe the shared session: {}",
        String::from_utf8_lossy(&cli.stdout)
    );
    tui.assert_running();
    rpc.assert_running("IDE rpc-server");
    mcp.assert_running("MCP stdio bridge");
    assert_eq!(
        web_client
            .server_identity()
            .await
            .expect("server survives simultaneous clients")
            .id,
        server_identity.id
    );

    let records = runtime_ownership_records(&audit_path);
    assert_eq!(
        records.len(),
        1,
        "only the server may compose Runtime: {records:#?}"
    );
    let record = &records[0];
    assert_eq!(record.schema, 1);
    assert_eq!(record.pid, server.pid());
    assert_eq!(
        record.workspace_root,
        std::fs::canonicalize(&workspace).expect("canonical ownership-test workspace")
    );
    assert_eq!(
        record.components,
        [
            "runtime",
            "provider_clients",
            "scheduler",
            "plugin_host",
            "execution_registry",
            "session_database",
        ]
    );

    let database_url = format!("sqlite://{}?mode=rwc", database_path.display());
    let pool = sqlx::SqlitePool::connect(database_url.as_str())
        .await
        .expect("open live server database for ownership assertion");
    let lease_owners = sqlx::query_scalar::<_, String>(
        "SELECT DISTINCT owner_id FROM agena_execution_leases ORDER BY owner_id",
    )
    .fetch_all(&pool)
    .await
    .expect("read active execution lease owners");
    assert_eq!(
        lease_owners.len(),
        1,
        "all active session execution must belong to one server Runtime"
    );
    let session_lease_owner = sqlx::query_scalar::<_, String>(
        "SELECT owner_id FROM agena_execution_leases WHERE session_id = ?",
    )
    .bind(session.id)
    .fetch_one(&pool)
    .await
    .expect("read ownership-test session lease");
    assert_eq!(session_lease_owner, lease_owners[0]);
    pool.close().await;

    // Every client disconnects without sending cancel. The provider remains
    // blocked and the same server execution/lease must still be observable.
    drop(web_connection);
    drop(tui);
    rpc.close("IDE rpc-server");
    mcp.close("MCP stdio bridge");
    tokio::time::sleep(Duration::from_millis(100)).await;
    let after_disconnect = web_client
        .get_session_state(session.id)
        .await
        .expect("observe execution after every client disconnected");
    assert_eq!(after_disconnect.session.state, SessionState::Running);
    assert_eq!(
        after_disconnect
            .active_execution
            .as_ref()
            .map(|active| active.execution_id),
        submitted
            .active_execution
            .as_ref()
            .map(|active| active.execution_id)
    );
    assert_eq!(runtime_ownership_records(&audit_path).len(), 1);
    assert!(
        provider_requests.try_recv().is_err(),
        "thin clients must not start a second provider continuation"
    );

    server.crash();
    provider.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn password_client_reauthenticates_after_server_restart_without_reconstruction() {
    let fixture = tempfile::tempdir().expect("create server reauthentication fixture");
    let workspace = fixture.path().join("workspace");
    let server_data = fixture.path().join("server-data");
    let database_path = fixture.path().join("sessions.db");
    let scheduler_database_path = fixture.path().join("scheduler.db");
    let record_path = fixture.path().join("server.json");
    let log_path = fixture.path().join("server.log");
    std::fs::create_dir_all(&workspace).expect("create reauthentication workspace");
    std::fs::create_dir_all(&server_data).expect("create reauthentication server data");

    let (provider_url, mut provider_requests, provider) = spawn_crash_recovery_provider().await;
    write_isolated_project_config(&workspace, provider_url.as_str());
    let password = "server-restart-password-secret";
    let server_environment = [
        ("AGENA_SERVER_UI_PASSWORD", Path::new(password)),
        (
            "AGENA_SCHEDULER_DATABASE_PATH",
            scheduler_database_path.as_path(),
        ),
    ];
    let port = unused_loopback_port();

    let mut first_server = spawn_server(
        &workspace,
        &database_path,
        &record_path,
        &server_data,
        &log_path,
        port,
        &server_environment,
    );
    let (_, first_identity) = wait_for_server(&first_server, &record_path).await;
    let record = std::fs::read_to_string(&record_path).expect("read first auth server record");
    assert!(!record.contains(password));
    let server_url = format!("http://127.0.0.1:{port}");
    let client = AgenaClient::connect_server(server_url.as_str(), None, Some(password))
        .await
        .expect("connect password client to first server");
    client
        .runtime_status()
        .await
        .expect("password token accesses first server");
    let debug = format!("{client:?}");
    assert!(debug.contains("password-refreshable"));
    assert!(!debug.contains(password));

    first_server.crash();
    let mut second_server = spawn_server(
        &workspace,
        &database_path,
        &record_path,
        &server_data,
        &log_path,
        port,
        &server_environment,
    );
    let (_, second_identity) = wait_for_server(&second_server, &record_path).await;
    assert_ne!(second_identity.id, first_identity.id);
    assert_ne!(second_identity.pid, first_identity.pid);

    // The same client still holds the first process's now-invalid bearer. Its
    // first protected request must exchange the retained in-memory password,
    // install the second process's token, and replay without reconstruction.
    client
        .runtime_status()
        .await
        .expect("same password client reauthenticates after server restart");
    assert_eq!(
        client
            .server_identity()
            .await
            .expect("read restarted server identity from same client")
            .id,
        second_identity.id
    );
    assert!(
        provider_requests.try_recv().is_err(),
        "authentication refresh must not start a provider request"
    );

    second_server.crash();
    provider.abort();
}
