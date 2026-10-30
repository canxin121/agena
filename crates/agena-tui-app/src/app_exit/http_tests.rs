//! Exercise the real App -> backend -> HTTP cancellation path on loopback.
//!
//! No production endpoint, environment credential, model, or artificial
//! ExitRunCancelled acknowledgement is used by these tests.

use super::*;
use crate::Route;
use agena_api::resource::{ActiveExecutionResource, SessionExecutionResource, SessionState};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio::task::{JoinHandle, JoinSet};

const SESSION_ID: i64 = 17;

#[derive(Clone, Debug)]
struct RecordedRequest {
    method: String,
    target: String,
    body: serde_json::Value,
}

#[derive(Clone, Copy)]
enum MockReply {
    Outcome(CancellationResult),
    NoResponse,
}

struct MockCancellationServer {
    endpoint: String,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
    received: mpsc::UnboundedReceiver<RecordedRequest>,
    task: JoinHandle<()>,
}

impl MockCancellationServer {
    async fn start(reply: MockReply) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&requests);
        let (tx, received) = mpsc::unbounded_channel();
        let task = tokio::spawn(async move {
            let mut connections = JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let (mut socket, _) = accepted.unwrap();
                        let recorded = Arc::clone(&recorded);
                        let tx = tx.clone();
                        connections.spawn(async move {
                            let request = read_request(&mut socket).await;
                            let is_cancel = request.method == "POST"
                                && request.target == "/api/v1/sessions/17/cancel";
                            recorded.lock().unwrap().push(request.clone());
                            let _ = tx.send(request);
                            if is_cancel && matches!(reply, MockReply::NoResponse) {
                                // Keep the TCP connection open and send no
                                // HTTP headers. Only the real App deadline
                                // may end this request.
                                let mut byte = [0_u8; 1];
                                let _ = socket.read(&mut byte).await;
                                return;
                            }
                            let body = if is_cancel {
                                let MockReply::Outcome(result) = reply else {
                                    unreachable!();
                                };
                                serde_json::to_vec(&CancellationOutcome::from(result)).unwrap()
                            } else {
                                // Acknowledgements may start unrelated read
                                // refreshes. Record them, but never synthesize
                                // another cancellation acknowledgement.
                                b"{}".to_vec()
                            };
                            let response = format!(
                                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                                body.len()
                            );
                            socket.write_all(response.as_bytes()).await.unwrap();
                            socket.write_all(&body).await.unwrap();
                        });
                    }
                    result = connections.join_next(), if !connections.is_empty() => {
                        result.unwrap().expect("mock HTTP connection must not panic");
                    }
                }
            }
        });
        Self {
            endpoint,
            requests,
            received,
            task,
        }
    }

    async fn first_request(&mut self) -> RecordedRequest {
        tokio::time::timeout(Duration::from_secs(5), self.received.recv())
            .await
            .expect("the App must issue its HTTP request")
            .expect("mock listener remains connected")
    }

    async fn assert_only_exact_cancellation(&self, execution_id: uuid::Uuid) {
        // Let any erroneously spawned fallback request reach the listener.
        // The cancellation future has already delivered its actual result.
        tokio::time::sleep(Duration::from_millis(250)).await;
        assert!(
            !self.task.is_finished(),
            "mock listener must remain healthy"
        );
        let requests = self.requests.lock().unwrap();
        let mutations = requests
            .iter()
            .filter(|request| !matches!(request.method.as_str(), "GET" | "HEAD" | "OPTIONS"))
            .collect::<Vec<_>>();
        assert_eq!(
            mutations.len(),
            1,
            "neither an acknowledgement nor a timeout may retry cancellation: {requests:?}"
        );
        assert_exact_cancellation(mutations[0], execution_id);
    }
}

impl Drop for MockCancellationServer {
    fn drop(&mut self) {
        // Dropping the accept loop's JoinSet also drops any unanswered socket.
        self.task.abort();
    }
}

async fn read_request(socket: &mut TcpStream) -> RecordedRequest {
    let mut request = Vec::new();
    let mut buffer = [0_u8; 2048];
    let body_start = loop {
        assert!(request.len() < 16 * 1024, "bounded mock request headers");
        let count = socket.read(&mut buffer).await.unwrap();
        assert_ne!(count, 0, "HTTP request must include complete headers");
        request.extend_from_slice(&buffer[..count]);
        if let Some(end) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
            break end + 4;
        }
    };
    let headers = std::str::from_utf8(&request[..body_start]).unwrap();
    let mut lines = headers.split("\r\n");
    let start = lines
        .next()
        .unwrap()
        .split_ascii_whitespace()
        .collect::<Vec<_>>();
    assert_eq!(start.len(), 3);
    assert_eq!(start[2], "HTTP/1.1");
    let method = start[0].to_owned();
    let target = start[1].to_owned();
    let mut lengths = Vec::new();
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            assert!(
                !name.eq_ignore_ascii_case("transfer-encoding"),
                "JSON cancellation must have a known Content-Length"
            );
            if name.eq_ignore_ascii_case("content-length") {
                lengths.push(value.trim().parse::<usize>().unwrap());
            }
        }
    }
    assert!(lengths.len() <= 1);
    let length = lengths.first().copied().unwrap_or(0);
    assert!(length < 16 * 1024, "bounded mock request body");
    while request.len() < body_start + length {
        let count = socket.read(&mut buffer).await.unwrap();
        assert_ne!(count, 0, "HTTP request body must be complete");
        request.extend_from_slice(&buffer[..count]);
    }
    let body = if length == 0 {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&request[body_start..body_start + length]).unwrap()
    };
    RecordedRequest {
        method,
        target,
        body,
    }
}

fn assert_exact_cancellation(request: &RecordedRequest, execution_id: uuid::Uuid) {
    assert_eq!(request.method, "POST");
    assert_eq!(request.target, "/api/v1/sessions/17/cancel");
    assert_eq!(
        request.body,
        serde_json::json!({"execution_id": execution_id}),
        "the exact active execution must be serialized; null/session-wide cancellation is forbidden"
    );
}

fn app_with_endpoint(endpoint: &str, execution_id: uuid::Uuid) -> (tempfile::TempDir, App) {
    let temp = tempfile::tempdir().unwrap();
    let backend =
        crate::TuiBackend::isolated_test_backend_with_endpoint(temp.path().to_path_buf(), endpoint);
    let mut app = App::new_with_backend(backend, crate::LaunchOptions::default(), I18n::english());
    // Keep persistence inside this fixture even when the test process has a
    // real Agena data directory in its environment.
    app.draft_store_path = temp.path().join("tui-drafts.json");
    app.prompt_history_path = temp.path().join("tui-prompt-history.jsonl");
    app.current_route = Route::Main;
    app.focus = Focus::Composer;
    app.transcript.session_id = Some(SESSION_ID);
    let state = SessionState::Running {
        execution: Some(ActiveExecutionResource {
            execution_id,
            phase: ExecutionPhase::StreamingModel,
        }),
        workflow: agena_domain::WorkflowState::Quiescent,
        requests: Vec::new(),
    };
    let snapshot: SessionExecutionResource = serde_json::from_value(serde_json::json!({
        "session": {
            "id": SESSION_ID, "parent_id": null, "root_id": SESSION_ID, "depth": 0,
            "workspace_id": 1, "title": "Isolated HTTP exit test", "version": 1,
            "relation_kind": "root", "lifecycle_state": "ready", "state": state,
            "created_at": "2026-01-01T00:00:00Z", "updated_at": "2026-01-01T00:00:00Z",
            "message_count": 0, "child_session_count": 0
        },
        "parts": [], "part_page": null,
        "execution": {
            "agent_id": "test", "selected_permission": {},
            "effective_permission": {}, "permission_ceiling": {}
        },
        "usage": { "current_tokens": 0 }
    }))
    .unwrap();
    app.transcript.execution = Some(snapshot);
    (temp, app)
}

fn choose(app: &mut App, value: &str) {
    let Some(Overlay::Choice(dialog)) = &mut app.overlay else {
        panic!("expected the actual exit picker");
    };
    assert!(matches!(dialog.action, ChoiceOverlayAction::Exit));
    assert!(
        dialog
            .presentation
            .select_item_where(|item| item.value == value)
    );
    app.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
}

fn start_exit_cancellation(app: &mut App, execution_id: uuid::Uuid) -> ExitCancellation {
    app.handle_key_event(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
    choose(app, "stop-and-exit");
    let pending = app
        .exit_cancellation
        .expect("the real request must become pending");
    assert_eq!(
        pending.target,
        ExitCancelTarget {
            session_id: SESSION_ID,
            execution_id: execution_id.into()
        }
    );
    assert!(pending.awaiting_ack && pending.quit_on_ack);
    assert!(!app.should_quit);
    pending
}

async fn receive_actual_ack(
    app: &mut App,
    expected: ExitCancellation,
    deadline: Duration,
) -> AppMessage {
    let message = tokio::time::timeout(deadline, app.rx.recv())
        .await
        .expect("the actual HTTP operation must produce an App result")
        .expect("App message channel remains connected");
    let AppMessage::ExitRunCancelled {
        request_id, target, ..
    } = &message
    else {
        panic!("expected the result produced by request_exit_cancel");
    };
    assert_eq!(*request_id, expected.request_id);
    assert_eq!(*target, expected.target);
    message
}

fn assert_return_and_exit_remain_usable(app: &mut App) {
    assert!(!app.should_quit);
    assert!(
        app.exit_cancellation.is_none(),
        "failure must clear the pending operation"
    );
    app.refresh_exit_menu();
    let Some(Overlay::Choice(dialog)) = &app.overlay else {
        panic!("failure must leave an actionable exit picker");
    };
    let mut picker = dialog.presentation.clone();
    assert!(picker.select_item_where(|item| item.value == "detach"));
    assert!(picker.select_item_where(|item| item.value == "cancel"));
    assert!(!picker.select_item_where(|item| item.value == "stop-and-exit"));
    choose(app, "cancel");
    assert!(!app.should_quit);
    assert!(app.overlay.is_none(), "Return must restore the UI");
    app.handle_key_event(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
    choose(app, "detach");
    assert!(
        app.should_quit,
        "Exit TUI remains usable after a failed stop"
    );
    assert!(app.exit_cancellation.is_none());
}

#[tokio::test]
async fn execution_mismatch_over_http_never_retries_without_the_exact_id() {
    let mut server =
        MockCancellationServer::start(MockReply::Outcome(CancellationResult::ExecutionMismatch))
            .await;
    let execution_id = uuid::Uuid::new_v4();
    let (_temp, mut app) = app_with_endpoint(&server.endpoint, execution_id);
    let pending = start_exit_cancellation(&mut app, execution_id);
    assert_exact_cancellation(&server.first_request().await, execution_id);
    let message = receive_actual_ack(&mut app, pending, Duration::from_secs(5)).await;
    let AppMessage::ExitRunCancelled { result, .. } = &message else {
        unreachable!();
    };
    assert_eq!(
        result.as_ref().unwrap().result,
        CancellationResult::ExecutionMismatch
    );
    app.handle_message(message);
    assert!(
        !app.should_quit,
        "a parsed execution mismatch must not exit"
    );
    assert_return_and_exit_remain_usable(&mut app);
    server.assert_only_exact_cancellation(execution_id).await;
}

#[tokio::test]
async fn unanswered_http_stop_times_out_after_the_real_deadline_and_releases_the_ui() {
    let mut server = MockCancellationServer::start(MockReply::NoResponse).await;
    let execution_id = uuid::Uuid::new_v4();
    let (_temp, mut app) = app_with_endpoint(&server.endpoint, execution_id);
    let started = Instant::now();
    let pending = start_exit_cancellation(&mut app, execution_id);
    assert_exact_cancellation(&server.first_request().await, execution_id);
    assert!(app.exit_cancellation.unwrap().awaiting_ack);
    assert!(!app.should_quit);
    // Deliberately use the production 30-second timeout. No injected duration,
    // paused test clock, synthetic ACK, or HTTP connection failure can pass.
    let message = receive_actual_ack(&mut app, pending, Duration::from_secs(40)).await;
    assert!(
        started.elapsed() >= Duration::from_secs(29),
        "an immediate transport failure is not the cancellation timeout path"
    );
    let AppMessage::ExitRunCancelled { result, .. } = &message else {
        unreachable!();
    };
    assert!(
        result.is_err(),
        "an unanswered socket must not produce a success ACK"
    );
    app.handle_message(message);
    assert_return_and_exit_remain_usable(&mut app);
    server.assert_only_exact_cancellation(execution_id).await;
}

#[tokio::test]
async fn matching_successful_http_ack_exits_once_without_a_second_cancel() {
    let mut server = MockCancellationServer::start(MockReply::Outcome(
        CancellationResult::CancellationRequested,
    ))
    .await;
    let execution_id = uuid::Uuid::new_v4();
    let (_temp, mut app) = app_with_endpoint(&server.endpoint, execution_id);
    let pending = start_exit_cancellation(&mut app, execution_id);
    assert_exact_cancellation(&server.first_request().await, execution_id);
    let message = receive_actual_ack(&mut app, pending, Duration::from_secs(5)).await;
    let AppMessage::ExitRunCancelled { result, .. } = &message else {
        unreachable!();
    };
    assert_eq!(
        result.as_ref().unwrap().result,
        CancellationResult::CancellationRequested
    );
    app.handle_message(message);
    assert!(app.should_quit);
    assert!(!app.exit_cancellation.unwrap().awaiting_ack);
    server.assert_only_exact_cancellation(execution_id).await;
}
