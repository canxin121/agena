#![cfg(unix)]

mod support;

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
    commands::{
        Command, CommandResult, ForkSessionParams, ReplyPermissionParams, ResolveWorkspaceParams,
        SubmitRunParams, UpdateSessionParams,
    },
    live::SessionChangeResource,
    part::PartResource,
    resource::{
        PathAccessModesResource, PathPermissionConfigResource, PermissionConfigResource,
        PermissionMode, PermissionReply, PermissionReplyKind, RunOptions, ServerEndpointRecord,
        SessionExecutionResource, SessionState,
    },
};
use agena_client::{AgenaClient, NotificationSubscription, SubscriptionEvent};
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
    let mut command = support::isolated_server_command(server_data_dir);
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

struct ProcessProviderReply {
    events: Vec<serde_json::Value>,
    wait_for_disconnect: bool,
}

fn process_text_events(text: &str, response_id: &str) -> Vec<serde_json::Value> {
    vec![
        serde_json::json!({"type":"response.output_text.delta","delta":text}),
        serde_json::json!({"type":"response.completed","response":{
            "id":response_id,"status":"completed",
            "usage":{"input_tokens":4,"output_tokens":3,"total_tokens":7}
        }}),
    ]
}

fn process_permission_events() -> Vec<serde_json::Value> {
    let call = serde_json::json!({"type":"function_call","id":"fc_process_read","call_id":"call_process_read","name":"tools_call"});
    let mut added = call.clone();
    added["arguments"] = serde_json::json!("");
    let mut done = call;
    done["arguments"] = serde_json::json!(
        serde_json::json!({
            "tool":"fs.read","input":{"file_path":"permission-fixture.txt"}
        })
        .to_string()
    );
    vec![
        serde_json::json!({"type":"response.output_item.added","output_index":0,"item":added}),
        serde_json::json!({"type":"response.output_item.done","output_index":0,"item":done}),
        serde_json::json!({"type":"response.completed","response":{
            "id":"resp_process_read","status":"completed","stop_reason":"tool_calls",
            "usage":{"input_tokens":5,"output_tokens":3,"total_tokens":8}
        }}),
    ]
}

async fn spawn_process_provider(
    plans: Vec<ProcessProviderReply>,
    repeat: Option<Vec<serde_json::Value>>,
) -> (
    String,
    mpsc::UnboundedReceiver<serde_json::Value>,
    tokio::task::JoinHandle<()>,
) {
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("bind controlled process-test provider");
    let address = listener.local_addr().expect("fake provider address");
    let (request_tx, request_rx) = mpsc::unbounded_channel();
    let server = tokio::spawn(async move {
        let mut plans = VecDeque::from(plans);
        while !plans.is_empty() || repeat.is_some() {
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

            let plan = plans.pop_front().unwrap_or_else(|| ProcessProviderReply {
                events: repeat
                    .as_ref()
                    .expect("provider plan or repeating reply")
                    .clone(),
                wait_for_disconnect: false,
            });
            request_tx
                .send(request)
                .expect("record fake provider request");
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncache-control: no-cache\r\nconnection: close\r\n\r\n",
                )
                .await
                .expect("write provider SSE headers");
            for event in plan.events {
                let frame = format!("data: {event}\n\n");
                if stream.write_all(frame.as_bytes()).await.is_err() {
                    break;
                }
            }
            if plan.wait_for_disconnect {
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

            let _ = stream.shutdown().await;
        }
    });
    (format!("http://{address}/v1"), request_rx, server)
}

async fn spawn_crash_recovery_provider() -> (
    String,
    mpsc::UnboundedReceiver<serde_json::Value>,
    tokio::task::JoinHandle<()>,
) {
    spawn_process_provider(
        vec![ProcessProviderReply {
            events: Vec::new(),
            wait_for_disconnect: true,
        }],
        Some(process_text_events(
            "recovered after server restart",
            "resp_restart_recovered",
        )),
    )
    .await
}

fn write_isolated_project_config(workspace: &Path, provider_base_url: &str) {
    let config_dir = workspace.join(".agena");
    std::fs::create_dir_all(&config_dir).expect("create project config directory");
    let config = serde_json::json!({
        "providers": {
            "default_selection": {
                "provider": "fake",
                "adapter": "openai_responses",
                "model": "fake-model"
            },
            "fake": {
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
        let mut execution = client
            .get_session_state(session_id)
            .await
            .expect("read process-test session state");
        execution.parts = client
            .session_all_parts(session_id)
            .await
            .expect("read process-test session parts");
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

async fn execution_text(client: &AgenaClient, execution: &SessionExecutionResource) -> String {
    let mut text = String::new();
    for part in &execution.parts {
        text.push_str(
            &client
                .part_text(execution.session.id, part, 1024 * 1024)
                .await
                .expect("read canonical process-test text resources"),
        );
    }
    text
}

async fn wait_for_session_change(
    subscription: &mut NotificationSubscription,
    predicate: impl Fn(&SessionChangeResource) -> bool,
) -> SessionChangeResource {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match subscription
                .recv()
                .await
                .expect("process-test SSE stays connected")
                .expect("decode process-test SSE")
            {
                SubscriptionEvent::SessionChanged(change) if predicate(&change) => return change,
                SubscriptionEvent::Lagged(skipped) => {
                    panic!("bounded process fixture unexpectedly lagged by {skipped}")
                }
                _ => {}
            }
        }
    })
    .await
    .expect("receive committed process-test change over SSE")
}

fn changed_part(change: &SessionChangeResource) -> Option<&PartResource> {
    match change {
        SessionChangeResource::PartAdded { part, .. }
        | SessionChangeResource::PartUpdated { part, .. } => Some(part),
        _ => None,
    }
}

fn permission_replies<'a>(
    parts: &'a [PartResource],
    request_id: &str,
) -> Vec<&'a serde_json::Value> {
    parts
        .iter()
        .filter_map(|part| {
            part.content
                .pointer("/authorization/permissions")
                .and_then(serde_json::Value::as_array)
        })
        .flatten()
        .filter(|permission| {
            permission
                .pointer("/request/request_id")
                .and_then(serde_json::Value::as_str)
                == Some(request_id)
        })
        .filter_map(|permission| permission.get("reply"))
        .collect()
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
async fn killed_server_restores_queued_messages_in_order_without_a_connected_sender() {
    let fixture = tempfile::tempdir().unwrap();
    let workspace = fixture.path().join("workspace");
    let server_data = fixture.path().join("server-data");
    let database_path = fixture.path().join("sessions.db");
    let record_path = fixture.path().join("server.json");
    let log_path = fixture.path().join("server.log");
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::create_dir_all(&server_data).unwrap();
    let (provider_url, mut requests, provider) = spawn_process_provider(
        vec![
            ProcessProviderReply {
                events: Vec::new(),
                wait_for_disconnect: true,
            },
            ProcessProviderReply {
                events: process_text_events("first queued answer", "queue-one"),
                wait_for_disconnect: false,
            },
            ProcessProviderReply {
                events: process_text_events("second queued answer", "queue-two"),
                wait_for_disconnect: false,
            },
        ],
        None,
    )
    .await;
    write_isolated_project_config(&workspace, &provider_url);
    let mut first_server = spawn_server(
        &workspace,
        &database_path,
        &record_path,
        &server_data,
        &log_path,
        0,
        &[],
    );
    let (sender, _) = wait_for_server(&first_server, &record_path).await;
    let CommandResult::Workspace(resolved) = sender
        .command(Command::ResolveWorkspace(ResolveWorkspaceParams {
            path: workspace.to_string_lossy().into_owned(),
            create_if_missing: true,
        }))
        .await
        .unwrap()
    else {
        panic!("expected workspace")
    };
    let session = sender
        .create_session(resolved.id, "durable message queue", None)
        .await
        .unwrap();
    for (index, text) in [
        "active before crash",
        "first queued prompt",
        "second queued prompt",
    ]
    .into_iter()
    .enumerate()
    {
        tokio::time::timeout(
            Duration::from_secs(5),
            sender.submit_message(SubmitRunParams {
                session_id: session.id,
                options: RunOptions::default(),
                document: agena_domain::ComposerDocument(vec![agena_domain::ComposerNode::Text {
                    text: text.into(),
                }]),
            }),
        )
        .await
        .expect("admission cannot wait for the provider")
        .unwrap();
        if index == 0 {
            tokio::time::timeout(Duration::from_secs(10), requests.recv())
                .await
                .unwrap()
                .unwrap();
        }
    }
    assert_eq!(
        sender
            .get_session_state(session.id)
            .await
            .unwrap()
            .queued_messages
            .len(),
        2
    );
    drop(sender);
    first_server.crash();
    let mut second_server = spawn_server(
        &workspace,
        &database_path,
        &record_path,
        &server_data,
        &log_path,
        0,
        &[],
    );
    let (observer, _) = wait_for_server(&second_server, &record_path).await;
    let first = tokio::time::timeout(Duration::from_secs(15), requests.recv())
        .await
        .unwrap()
        .unwrap()
        .to_string();
    assert!(first.contains("first queued prompt"));
    assert!(!first.contains("second queued prompt"));
    let second = tokio::time::timeout(Duration::from_secs(15), requests.recv())
        .await
        .unwrap()
        .unwrap()
        .to_string();
    assert!(
        second.find("first queued answer").unwrap() < second.find("second queued prompt").unwrap()
    );
    let completed = wait_for_execution(&observer, session.id, |state| {
        state.session.state.active_execution().is_none()
    })
    .await;
    assert!(completed.queued_messages.is_empty());
    assert!(
        execution_text(&observer, &completed)
            .await
            .contains("second queued answer")
    );
    let pool = sqlx::SqlitePool::connect(&format!("sqlite://{}?mode=rw", database_path.display()))
        .await
        .unwrap();
    let phases = sqlx::query_scalar::<_, String>("SELECT a.phase FROM agena_executions e JOIN agena_attempts a ON a.execution_id=e.id AND a.generation=e.current_generation WHERE e.session_id=? ORDER BY e.message_id")
        .bind(session.id).fetch_all(&pool).await.unwrap();
    assert_eq!(phases, ["interrupted", "succeeded", "succeeded"]);
    pool.close().await;
    provider.await.unwrap();
    assert!(requests.try_recv().is_err());
    second_server.crash();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn killed_server_restart_reconciles_the_abandoned_run_before_first_read() {
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
    assert!(
        status
            .provider_ids
            .iter()
            .any(|provider| provider == "fake")
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
    client_a
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
    wait_for_execution(&client_a, session.id, |execution| {
        execution.session.state.is_running() && execution.session.state.active_execution().is_some()
    })
    .await;
    first_server.crash();
    drop(client_a);

    // The crash left the attempt in flight with nobody executing it. That
    // durable fact is the entire recovery input: the process that opens this
    // data directory next terminalizes it (17.4). No clock is consulted, so
    // the regression is deterministic rather than time-dependent.
    let database_url = format!("sqlite://{}?mode=rwc", database_path.display());
    let pool = sqlx::SqlitePool::connect(database_url.as_str())
        .await
        .expect("open killed server database");
    let orphaned = sqlx::query_scalar::<_, i64>(
        "SELECT e.message_id FROM agena_executions e JOIN agena_attempts a \
         ON a.execution_id=e.id AND a.generation=e.current_generation \
         WHERE a.phase IN ('queued','running')",
    )
    .fetch_all(&pool)
    .await
    .expect("read the crashed server's in-flight run markers");
    assert_eq!(
        orphaned.len(),
        1,
        "the crashed server must leave exactly one ownerless in-flight run"
    );
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
        "an abandoned run must never remain visible as running after restart"
    );
    assert!(
        overview.attention.iter().any(|item| item.id == session.id),
        "an interrupted attempt requires attention until explicitly continued"
    );

    let mut reconciled = client_b
        .get_session_state(session.id)
        .await
        .expect("open the reconciled session");
    reconciled.parts = client_b
        .session_all_parts(session.id)
        .await
        .expect("read reconciled session parts");
    assert!(matches!(
        reconciled.session.state,
        SessionState::Failed { .. }
    ));
    assert!(reconciled.session.state.active_execution().is_none());
    assert!(reconciled.parts.iter().any(|part| {
        part.is_message_boundary()
            && part.role == "user"
            // The submitted document is complete; the interrupted execution
            // is reported separately from its immutable input content.
            && part.state == "completed"
            && part.content["execution_phase"] == "interrupted"
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
        matches!(execution.session.state, SessionState::Ready { .. })
            && execution.session.state.active_execution().is_none()
    })
    .await;
    assert!(
        execution_text(&client_b, &completed)
            .await
            .contains("recovered after server restart")
    );
    assert!(
        completed
            .session
            .state
            .pending_interactive_requests()
            .is_empty()
    );
    assert_eq!(
        completed
            .parts
            .iter()
            .filter(|part| part.is_message_boundary() && part.role == "assistant")
            .count(),
        1,
        "the interrupted attempt has no fabricated assistant marker; recovery creates one real response"
    );
    let pool = sqlx::SqlitePool::connect(database_url.as_str())
        .await
        .expect("inspect recovery attempt facts");
    let attempts=sqlx::query_as::<_,(i64,String)>("SELECT a.generation,a.phase FROM agena_attempts a JOIN agena_executions e ON e.id=a.execution_id WHERE e.session_id=? ORDER BY a.generation")
        .bind(session.id).fetch_all(&pool).await.expect("read interrupted and recovery attempts");
    assert_eq!(
        attempts,
        vec![(1, "interrupted".into()), (2, "succeeded".into())],
        "explicit recovery creates a fenced generation without losing the interrupted attempt"
    );
    pool.close().await;

    second_server.crash();
    provider.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn native_permission_fork_cancel_and_sse_reconnect_survive_a_real_server_restart() {
    let fixture = tempfile::tempdir().expect("create native HTTP process fixture");
    let workspace = fixture.path().join("workspace");
    let server_data = fixture.path().join("server-data");
    let database_path = fixture.path().join("sessions.db");
    let record_path = fixture.path().join("server.json");
    let log_path = fixture.path().join("server.log");
    std::fs::create_dir_all(&workspace).expect("create native process workspace");
    std::fs::create_dir_all(&server_data).expect("create native process server data");
    std::fs::write(
        workspace.join("permission-fixture.txt"),
        "permission fixture from the real tool\n",
    )
    .expect("write controlled permission target");
    let answer = "permission allowed 中文 🧪  \n";
    let partial = "partial branch output 中文 🧪  \n";
    let (provider_url, mut requests, provider) = spawn_process_provider(
        vec![
            ProcessProviderReply {
                events: process_permission_events(),
                wait_for_disconnect: false,
            },
            ProcessProviderReply {
                events: process_text_events(answer, "resp_process_permission_done"),
                wait_for_disconnect: false,
            },
            ProcessProviderReply {
                events: vec![
                    serde_json::json!({"type":"response.output_text.delta","delta":partial}),
                ],
                wait_for_disconnect: true,
            },
        ],
        None,
    )
    .await;
    write_isolated_project_config(&workspace, &provider_url);
    let mut first_server = spawn_server(
        &workspace,
        &database_path,
        &record_path,
        &server_data,
        &log_path,
        0,
        &[],
    );
    let (client, first_identity) = wait_for_server(&first_server, &record_path).await;
    let CommandResult::Workspace(directory) = client
        .command(Command::ResolveWorkspace(ResolveWorkspaceParams {
            path: workspace.to_string_lossy().into_owned(),
            create_if_missing: true,
        }))
        .await
        .expect("resolve native process workspace")
    else {
        panic!("workspace result");
    };
    let parent = client
        .create_session(directory.id, "native process parent", None)
        .await
        .expect("create process session");
    client
        .set_session_permission(
            parent.id,
            PermissionConfigResource {
                path: Some(PathPermissionConfigResource {
                    workspace: Some(PathAccessModesResource {
                        read: Some(PermissionMode::Ask),
                        write: Some(PermissionMode::Ask),
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            },
        )
        .await
        .expect("configure a real path permission gate");
    let mut original = client
        .connect_session(parent.id)
        .await
        .expect("connect before submitting");
    assert!(original.snapshot.session.state.active_execution().is_none());
    client
        .submit_message(SubmitRunParams {
            session_id: parent.id,
            options: RunOptions::default(),
            document: agena_domain::ComposerDocument(vec![agena_domain::ComposerNode::Text {
                text: "read the controlled permission fixture".into(),
            }]),
        })
        .await
        .expect("submit through the real HTTP server");
    tokio::time::timeout(Duration::from_secs(10), requests.recv())
        .await
        .expect("provider receives the user message")
        .expect("record the permission-producing request");
    let input_change = wait_for_session_change(&mut original.subscription, |change| {
        changed_part(change).is_some_and(|part| part.is_message_boundary() && part.role == "user")
    })
    .await;
    let input_id = changed_part(&input_change)
        .expect("native user entry in SSE")
        .part_id;
    let pending = wait_for_execution(&client, parent.id, |execution| {
        execution
            .session
            .state
            .pending_interactive_requests()
            .iter()
            .any(|pending| pending.request.as_permission().is_some())
    })
    .await;
    let permission = pending
        .session
        .state
        .pending_interactive_requests()
        .iter()
        .find_map(|pending| pending.request.as_permission())
        .expect("native permission in authoritative shell");
    let request_id = permission.request_id.clone();
    let interaction_id = request_id
        .parse::<i64>()
        .expect("permission identity is the real native interaction id");
    assert!(interaction_id > 0);
    let execution_id = pending
        .session
        .state
        .active_execution()
        .expect("permission wait retains its execution")
        .execution_id;
    original.subscription.close();
    drop(original);

    // The first SSE consumer is gone. Another client consumes the native gate
    // while the same server-owned execution remains active.
    let record: ServerEndpointRecord =
        serde_json::from_slice(&std::fs::read(&record_path).unwrap()).unwrap();
    let other = AgenaClient::new(record.url.as_str()).expect("attach the second HTTP client");
    assert_eq!(
        other
            .get_session_state(parent.id)
            .await
            .unwrap()
            .session
            .state
            .active_execution()
            .map(|active| active.execution_id),
        Some(execution_id)
    );
    other
        .reply_permission(ReplyPermissionParams {
            session_id: parent.id,
            options: RunOptions::default(),
            reply: PermissionReply {
                request_id: request_id.clone(),
                kind: PermissionReplyKind::AllowOnce,
                reason: Some("approved from a different process client".into()),
                scope: None,
            },
        })
        .await
        .expect("answer the durable permission through HTTP");
    let resumed = tokio::time::timeout(Duration::from_secs(10), requests.recv())
        .await
        .expect("provider receives the permission continuation")
        .expect("record continuation");
    assert!(
        serde_json::to_string(&resumed)
            .unwrap()
            .contains("permission fixture from the real tool"),
        "the provider continuation must contain the actual fs.read result: {resumed}"
    );
    let completed = wait_for_execution(&other, parent.id, |execution| {
        execution.session.state.active_execution().is_none()
            && execution
                .session
                .state
                .pending_interactive_requests()
                .is_empty()
            && execution.parts.iter().any(|part| {
                part.role == "assistant" && part.kind == "text" && part.state == "completed"
            })
    })
    .await;
    let response = completed
        .parts
        .iter()
        .find(|part| part.role == "assistant" && part.kind == "text")
        .unwrap();
    assert_eq!(
        other
            .part_text(parent.id, response, 64 * 1024)
            .await
            .unwrap(),
        answer
    );
    assert!(completed.parts.iter().all(|part| part.kind != "run"));
    let tool = completed
        .parts
        .iter()
        .find(|part| part.kind == "tool_call")
        .expect("real tool entry");
    assert!(
        tool.is_message_boundary(),
        "the first real assistant entry owns the response boundary"
    );
    let replies = permission_replies(&completed.parts, &request_id);
    assert_eq!(replies.len(), 1);
    assert_eq!(replies[0]["kind"], "allow_once");
    assert_eq!(
        replies[0]["reason"],
        "approved from a different process client"
    );
    let expected_reply = replies[0].clone();
    let parent_ids = completed
        .parts
        .iter()
        .map(|part| part.part_id)
        .collect::<Vec<_>>();
    assert!(parent_ids.contains(&input_id));

    // Reconnect after missed terminal changes, then consume a fresh live
    // change. The snapshot and stream must both belong to the current owner.
    let mut reconnected = other
        .connect_session(parent.id)
        .await
        .expect("reconnect snapshot plus SSE");
    assert!(
        reconnected
            .snapshot
            .session
            .state
            .pending_interactive_requests()
            .is_empty()
    );
    assert!(
        reconnected
            .snapshot
            .session
            .state
            .active_execution()
            .is_none()
    );
    assert!(
        reconnected
            .snapshot
            .latest_event_seq
            .expect("reconnected native control version")
            > pending
                .latest_event_seq
                .expect("permission-wait native control version")
    );
    other
        .command(Command::UpdateSession(UpdateSessionParams {
            session_id: parent.id,
            title: "parent after SSE reconnect".into(),
            expected_version: None,
        }))
        .await
        .expect("mutate after SSE reconnect");
    wait_for_session_change(&mut reconnected.subscription,|change|matches!(change,
        SessionChangeResource::SessionMetaUpdated{session_id,title,..} if *session_id==parent.id&&title=="parent after SSE reconnect"
    )).await;

    let CommandResult::Execution(fork) = other
        .command(Command::ForkSession(ForkSessionParams {
            session_id: parent.id,
            at_message_id: None,
            title: Some("native process child".into()),
            conversation_mode: None,
        }))
        .await
        .expect("fork the completed native history through HTTP")
    else {
        panic!("fork execution result");
    };
    let child_id = fork.session.id;
    assert_eq!(fork.session.parent_id, Some(parent.id));
    let inherited = other
        .session_all_parts(child_id)
        .await
        .expect("read inherited membership");
    assert_eq!(
        inherited
            .iter()
            .map(|part| part.part_id)
            .collect::<Vec<_>>(),
        parent_ids
    );
    assert!(
        inherited
            .iter()
            .all(|part| part.origin_session_id == parent.id)
    );
    assert_eq!(permission_replies(&inherited, &request_id).len(), 1);
    let inherited_response = inherited
        .iter()
        .find(|part| part.part_id == response.part_id)
        .unwrap();
    assert_eq!(
        other
            .part_text(child_id, inherited_response, 64 * 1024)
            .await
            .unwrap(),
        answer
    );

    let mut child_connection = other
        .connect_session(child_id)
        .await
        .expect("subscribe before child execution");
    other
        .submit_message(SubmitRunParams {
            session_id: child_id,
            options: RunOptions::default(),
            document: agena_domain::ComposerDocument(vec![agena_domain::ComposerNode::Text {
                text: "child to cancel".into(),
            }]),
        })
        .await
        .expect("submit the independent child execution");
    tokio::time::timeout(Duration::from_secs(10), requests.recv())
        .await
        .expect("provider receives child request")
        .expect("record child request");
    let child_running = wait_for_execution(&other, child_id, |execution| {
        execution.session.state.active_execution().is_some()
            && execution.parts.iter().any(|part| {
                part.origin_session_id == child_id
                    && part.role == "assistant"
                    && part.kind == "text"
            })
    })
    .await;
    let partial_entry = child_running
        .parts
        .iter()
        .find(|part| {
            part.origin_session_id == child_id && part.role == "assistant" && part.kind == "text"
        })
        .unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if other
                .part_text(child_id, partial_entry, 64 * 1024)
                .await
                .unwrap()
                == partial
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("accept native text bytes before cancellation");
    let child_execution = child_running
        .session
        .state
        .active_execution()
        .unwrap()
        .execution_id;
    let cancellation = client
        .cancel_run(child_id, Some(agena_domain::ExecutionId(child_execution)))
        .await
        .expect("cancel from the other HTTP client");
    assert_eq!(
        cancellation.result,
        agena_domain::CancellationResult::CancellationRequested
    );
    let cancelled = wait_for_execution(&other, child_id, |execution| {
        execution.session.state.active_execution().is_none()
            && execution.parts.iter().any(|part| {
                part.part_id == partial_entry.part_id
                    && part.state == "completed"
                    && part.run_state.as_deref() == Some("cancelled")
            })
    })
    .await;
    wait_for_session_change(&mut child_connection.subscription, |change| {
        changed_part(change).is_some_and(|part| {
            part.part_id == partial_entry.part_id
                && part.state == "completed"
                && part.run_state.as_deref() == Some("cancelled")
        })
    })
    .await;
    assert_eq!(
        other
            .part_text(
                child_id,
                cancelled
                    .parts
                    .iter()
                    .find(|part| part.part_id == partial_entry.part_id)
                    .unwrap(),
                64 * 1024
            )
            .await
            .unwrap(),
        partial,
        "cancellation must seal accepted bytes"
    );
    let child_ids = cancelled
        .parts
        .iter()
        .map(|part| part.part_id)
        .collect::<Vec<_>>();
    assert_eq!(
        other
            .session_all_parts(parent.id)
            .await
            .unwrap()
            .iter()
            .map(|part| part.part_id)
            .collect::<Vec<_>>(),
        parent_ids,
        "the child's send and cancellation must not mutate the parent's membership"
    );
    tokio::time::timeout(Duration::from_secs(5), provider)
        .await
        .expect("cancellation closes the provider socket")
        .expect("controlled provider exits");
    assert!(
        requests.recv().await.is_none(),
        "permission, fork and cancel must not duplicate provider continuations"
    );
    drop(child_connection);
    drop(reconnected);
    drop(other);
    drop(client);
    first_server.crash();

    let mut second_server = spawn_server(
        &workspace,
        &database_path,
        &record_path,
        &server_data,
        &log_path,
        0,
        &[],
    );
    let (restarted, second_identity) = wait_for_server(&second_server, &record_path).await;
    assert_ne!(first_identity.id, second_identity.id);
    let restored_parent = wait_for_execution(&restarted, parent.id, |execution| {
        execution.session.state.active_execution().is_none()
    })
    .await;
    let restored_child = wait_for_execution(&restarted, child_id, |execution| {
        execution.session.state.active_execution().is_none()
    })
    .await;
    assert_eq!(restored_parent.session.title, "parent after SSE reconnect");
    assert_eq!(restored_child.session.parent_id, Some(parent.id));
    assert_eq!(
        restored_parent
            .parts
            .iter()
            .map(|part| part.part_id)
            .collect::<Vec<_>>(),
        parent_ids
    );
    assert_eq!(
        restored_child
            .parts
            .iter()
            .map(|part| part.part_id)
            .collect::<Vec<_>>(),
        child_ids
    );
    assert_eq!(
        permission_replies(&restored_parent.parts, &request_id),
        vec![&expected_reply]
    );
    assert_eq!(
        permission_replies(&restored_child.parts, &request_id),
        vec![&expected_reply]
    );
    assert!(
        restored_parent
            .session
            .state
            .pending_interactive_requests()
            .is_empty()
    );
    assert!(
        restored_child
            .session
            .state
            .pending_interactive_requests()
            .is_empty()
    );
    assert_eq!(
        restarted
            .part_text(
                child_id,
                restored_child
                    .parts
                    .iter()
                    .find(|part| part.part_id == partial_entry.part_id)
                    .unwrap(),
                64 * 1024
            )
            .await
            .unwrap(),
        partial
    );
    let pool = sqlx::SqlitePool::connect(&format!("sqlite://{}?mode=rwc", database_path.display()))
        .await
        .expect("inspect native facts after restart");
    let phases=sqlx::query_as::<_,(i64,String)>("SELECT e.session_id,a.phase FROM agena_executions e JOIN agena_attempts a ON a.execution_id=e.id AND a.generation=e.current_generation WHERE e.session_id IN (?,?) ORDER BY e.session_id")
        .bind(parent.id).bind(child_id).fetch_all(&pool).await.unwrap();
    assert_eq!(
        phases,
        vec![
            (parent.id, "succeeded".into()),
            (child_id, "cancelled".into())
        ]
    );
    let decisions=sqlx::query_scalar::<_,i64>("SELECT count(*) FROM agena_interactions WHERE id=? AND session_id=? AND decision_json IS NOT NULL")
        .bind(interaction_id).bind(parent.id).fetch_one(&pool).await.unwrap();
    assert_eq!(
        decisions, 1,
        "the completed permission is one native interaction, including after process restart"
    );
    pool.close().await;
    second_server.crash();
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
    assert!(
        status
            .provider_ids
            .iter()
            .any(|provider| provider == "fake")
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
    web_client
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
        execution.session.state.is_running() && execution.session.state.active_execution().is_some()
    })
    .await;
    let running_execution_id = running
        .session
        .state
        .active_execution()
        .expect("running session has an active execution")
        .execution_id;
    // This snapshot-plus-SSE attachment uses the same public transport as the
    // Web conversation runtime and remains connected while the native clients
    // below start. The submitted HTTP request itself is already detached from
    // the server-owned execution task.
    let web_connection = web_client
        .connect_session(session.id)
        .await
        .expect("attach Web-style session snapshot plus SSE");
    assert!(web_connection.snapshot.session.state.is_running());

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
    // Ownership is observable as the durable in-flight run itself: exactly one
    // marker exists, it belongs to the one session under execution, and it is
    // owned by the single server Runtime that is holding the provider request
    // open. No client can add a second one (17.3).
    let in_flight_runs = sqlx::query_scalar::<_, i64>(
        "SELECT e.message_id FROM agena_executions e JOIN agena_attempts a \
         ON a.execution_id=e.id AND a.generation=e.current_generation \
         WHERE a.phase IN ('queued','running') AND e.session_id=?",
    )
    .bind(session.id)
    .fetch_all(&pool)
    .await
    .expect("read the server-owned in-flight run markers");
    assert_eq!(
        in_flight_runs.len(),
        1,
        "all active session execution must belong to one server Runtime"
    );
    pool.close().await;

    // Every client disconnects without sending cancel. The provider remains
    // blocked and the same server execution must still be observable.
    drop(web_connection);
    drop(tui);
    rpc.close("IDE rpc-server");
    mcp.close("MCP stdio bridge");
    tokio::time::sleep(Duration::from_millis(100)).await;
    let after_disconnect = web_client
        .get_session_state(session.id)
        .await
        .expect("observe execution after every client disconnected");
    assert!(after_disconnect.session.state.is_running());
    assert_eq!(
        after_disconnect
            .session
            .state
            .active_execution()
            .map(|active| active.execution_id),
        Some(running_execution_id),
        "disconnecting every client must not replace the server-owned execution"
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
