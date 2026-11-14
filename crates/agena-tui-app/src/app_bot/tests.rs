use super::*;
use agena_tui::i18n::I18n;
use ratatui::{Terminal, backend::TestBackend, layout::Rect};
use std::sync::{Arc, Mutex};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    time::{Duration, timeout},
};

struct Server {
    endpoint: String,
    requests: Arc<Mutex<Vec<String>>>,
    bodies: Arc<Mutex<Vec<(String, serde_json::Value)>>>,
    profiles: Arc<Mutex<Vec<BotProfile>>>,
    fail: Arc<std::sync::atomic::AtomicBool>,
    busy: Arc<std::sync::atomic::AtomicBool>,
    next_binding: Arc<Mutex<Option<BotSession>>>,
    task: tokio::task::JoinHandle<()>,
}
impl Server {
    async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let bodies = Arc::new(Mutex::new(Vec::new()));
        let profiles = Arc::new(Mutex::new(vec![profile(1, "AI Bot")]));
        let fail = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let busy = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let next_binding = Arc::new(Mutex::new(None));
        let bindings = next_binding.clone();
        let (recorded, mutations, registry, failures, idle) = (
            requests.clone(),
            bodies.clone(),
            profiles.clone(),
            fail.clone(),
            busy.clone(),
        );
        let task = tokio::spawn(async move {
            let mut connections = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let (mut socket, _) = accepted.unwrap();
                        let (recorded, mutations, registry, failures, idle) = (recorded.clone(), mutations.clone(), registry.clone(), failures.clone(), idle.clone());
                        let bindings = bindings.clone();
                        connections.spawn(async move {
                            let mut bytes = Vec::new();
                            let header_end = loop {
                                if let Some(pos) = bytes.windows(4).position(|b| b == b"\r\n\r\n") { break pos + 4; }
                                let mut buf = [0; 4096];
                                let n = socket.read(&mut buf).await.unwrap();
                                if n == 0 { return; }
                                bytes.extend_from_slice(&buf[..n]);
                                assert!(bytes.len() < 65536);
                            };
                            let headers = String::from_utf8(bytes[..header_end].to_vec()).unwrap();
                            let line = headers.lines().next().unwrap().to_owned();
                            let length: usize = headers.lines().find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|n| n.trim().parse().unwrap())).unwrap_or(0);
                            while bytes.len() < header_end + length {
                                let mut buf = [0; 4096]; let n = socket.read(&mut buf).await.unwrap(); if n == 0 { return; } bytes.extend_from_slice(&buf[..n]);
                            }
                            let body = serde_json::from_slice(&bytes[header_end..header_end+length]).unwrap_or(serde_json::Value::Null);
                            recorded.lock().unwrap().push(line.clone());
                            mutations.lock().unwrap().push((line.clone(), body.clone()));
                            let (status, response) = {
                                let mut profiles = registry.lock().unwrap();
                                if failures.load(std::sync::atomic::Ordering::SeqCst) {
                                    ("503 Service Unavailable", serde_json::json!({"message":"offline"}))
                                } else {
                                    let binding = if line.starts_with("POST /api/v1/bots/") && line.contains("/session ") { bindings.lock().unwrap().take() } else { None };
                                    mock_response(&line, body, &mut profiles, idle.load(std::sync::atomic::Ordering::SeqCst), binding)
                                }
                            };
                            let body = response.to_string();
                            let response = format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                            let _ = socket.write_all(response.as_bytes()).await;
                        });
                    }
                    _ = connections.join_next(), if !connections.is_empty() => {}
                }
            }
        });
        Self {
            endpoint,
            requests,
            bodies,
            profiles,
            fail,
            busy,
            next_binding,
            task,
        }
    }
    fn bot_requests(&self) -> usize {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|line| {
                line.as_str() == "POST /api/v1/bot/session HTTP/1.1"
                    || (line.starts_with("POST /api/v1/bots/") && line.contains("/session "))
            })
            .count()
    }
}
fn mock_response(
    line: &str,
    body: serde_json::Value,
    profiles: &mut Vec<BotProfile>,
    busy: bool,
    binding: Option<BotSession>,
) -> (&'static str, serde_json::Value) {
    use serde_json::{json, to_value};
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap();
    let path = parts.next().unwrap();
    let default = profiles.iter().find(|p| !p.archived).map(|p| p.id);
    let ok = |value| ("200 OK", value);
    if method == "GET" && path.starts_with("/api/v1/sessions?") {
        // Owner-filtered session list: one local and one group conversation so
        // the sidebar has two surfaces to group.
        let owner = path
            .split('&')
            .find_map(|part| part.trim_start_matches('?').strip_prefix("owner_bot_id="));
        let items: Vec<serde_json::Value> = match owner {
            Some(id) if !id.is_empty() => vec![
                json!({"id": 901, "title": "Local chat", "depth": 0, "root_id": 901, "version": 1, "child_session_count": 0, "relation_kind": "root", "lifecycle_state": "ready", "owner_bot_id": id, "owner_surface": "local",
                       "message_count": 2, "favorite": false, "pinned": false, "state": {"kind": "ready", "data": {}},
                       "created_at": "2026-01-01T00:00:00Z", "updated_at": "2026-01-01T00:00:00Z",
                       "last_message_at": "2026-01-01T00:01:00Z"}),
                json!({"id": 902, "title": "Team room", "depth": 0, "root_id": 902, "version": 1, "child_session_count": 0, "relation_kind": "root", "lifecycle_state": "ready", "owner_bot_id": id, "owner_surface": "group",
                       "message_count": 5, "favorite": true, "pinned": false, "state": {"kind": "ready", "data": {}},
                       "created_at": "2026-01-01T00:00:00Z", "updated_at": "2026-01-01T00:00:00Z",
                       "last_message_at": "2026-01-01T00:02:00Z"}),
            ],
            _ => vec![],
        };
        let total = items.len();
        return ok(json!({
            "items": items,
            "page": {"has_more": false, "returned": total},
            "total": total,
        }));
    }
    if method == "GET" && path.starts_with("/api/v1/bots?") {
        return ok(to_value(BotList {
            items: profiles.clone(),
            default_bot_id: default,
        })
        .unwrap());
    }
    if method == "POST" && path == "/api/v1/bot/session" {
        if profiles.is_empty() {
            profiles.push(profile(1, "AI Bot"));
        }
        return profiles
            .iter()
            .find(|p| !p.archived)
            .map(|p| ok(to_value(BotSession::new(900 + p.id.as_u128() as i64, p)).unwrap()))
            .unwrap_or(("404 Not Found", json!({"message":"All Bots are archived"})));
    }
    if method == "POST" && path == "/api/v1/bots" {
        let create: BotCreate = serde_json::from_value(body).unwrap();
        let mut p = profile(profiles.len() as u128 + 1, &create.name);
        p.persona = create.persona.unwrap_or_default();
        profiles.push(p.clone());
        return ok(to_value(p).unwrap());
    }
    if let Some(path) = path.strip_prefix("/api/v1/bots/") {
        let mut segments = path.split('/');
        if let Ok(id) = Uuid::parse_str(segments.next().unwrap())
            && let Some(p) = profiles.iter_mut().find(|p| p.id == id)
        {
            match (method, segments.next()) {
                ("GET", None) => return ok(to_value(p).unwrap()),
                ("POST", Some("session")) if !p.archived => {
                    // Return the staged binding verbatim, even when it names a
                    // different Bot: the client must reject that mismatch.
                    if let Some(session) = binding {
                        return ok(to_value(session).unwrap());
                    }
                    return ok(to_value(BotSession::new(900 + p.id.as_u128() as i64, p)).unwrap());
                }
                ("PUT", None) => {
                    let update: BotUpdate = serde_json::from_value(body).unwrap();
                    if update.expected_revision != p.revision {
                        return (
                            "409 Conflict",
                            to_value(agena_api::error::ApiError::conflict(
                                "Bot revision conflict",
                            ))
                            .unwrap(),
                        );
                    }
                    p.name = update.name;
                    p.persona = update.persona;
                    p.revision += 1;
                    return ok(to_value(p).unwrap());
                }
                ("POST", Some("archive")) => {
                    let update: BotArchive = serde_json::from_value(body).unwrap();
                    if busy || update.expected_revision != p.revision {
                        return (
                            "409 Conflict",
                            to_value(agena_api::error::ApiError::conflict(
                                "Bot must be idle; revision conflict",
                            ))
                            .unwrap(),
                        );
                    }
                    p.archived = true;
                    p.revision += 1;
                    return ok(to_value(p).unwrap());
                }
                _ => {}
            }
        }
    }
    ("404 Not Found", json!({"message":"not found"}))
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn app(endpoint: Option<&str>) -> (tempfile::TempDir, App) {
    let temp = tempfile::tempdir().unwrap();
    let backend = match endpoint {
        Some(endpoint) => {
            crate::TuiBackend::isolated_test_backend_with_endpoint(temp.path().into(), endpoint)
        }
        None => crate::TuiBackend::isolated_test_backend(temp.path().into()),
    };
    let mut app = App::new_with_backend(backend, crate::LaunchOptions::default(), I18n::english());
    app.draft_store_path = temp.path().join("drafts.json");
    app.prompt_history_path = temp.path().join("history.jsonl");
    app.start_command_actor();
    (temp, app)
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}
fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

async fn resolve(app: &mut App) {
    let expected = app.bot.request_id;
    timeout(Duration::from_secs(10), async {
        loop {
            let message = app.rx.recv().await.unwrap();
            let ready = matches!(&message, AppMessage::BotSessionReady { request_id, .. } if *request_id == expected);
            app.handle_message(message);
            if ready { return; }
        }
    }).await.unwrap();
}

#[tokio::test]
async fn bot_shortcut_works_from_initial_empty_hub_without_selected_session() {
    let server = Server::start().await;
    let (_temp, mut app) = app(Some(&server.endpoint));
    app.current_route = Route::Hub(crate::HubState::new());
    assert!(
        matches!(&app.current_route, Route::Hub(state) if state.presentation.selected_workspace().is_none())
    );
    assert!(app.transcript.session_id.is_none());
    app.handle_key_event(ctrl('b'));
    assert!(matches!(app.current_route, Route::BotEntry(_)));
    resolve(&mut app).await;
    assert_eq!(app.transcript.session_id, Some(901));
    assert!(app.current_route_is_main());
    assert_eq!(app.focus, Focus::Composer);
    assert!(app.is_bot_session());
    assert_eq!(app.current_session_path_label(), None);
    assert_eq!(server.bot_requests(), 1);
    assert!(app.transcript_surface_title().contains("AI Bot"));
}

#[tokio::test]
async fn hub_letter_entry_opens_bot_before_catalog_loads() {
    let (_temp, mut app) = app(None);
    app.open_hub();
    app.handle_key_event(key(KeyCode::Char('b')));
    assert!(matches!(app.current_route, Route::BotEntry(_)));
    assert!(
        matches!(app.route_stack.last(), Some(Route::Hub(state)) if state.request_task.is_none())
    );
}

#[tokio::test]
async fn repeat_open_coalesces_in_flight_and_revalidates_server_identity_on_reopen() {
    let server = Server::start().await;
    let (_temp, mut app) = app(Some(&server.endpoint));
    app.open_bot();
    let request_id = app.bot.request_id;
    app.open_bot();
    app.handle_key_event(ctrl('b'));
    assert_eq!(app.bot.request_id, request_id);
    resolve(&mut app).await;
    app.composer.insert_str("Bot draft stays here");
    app.handle_key_event(ctrl('g'));
    assert!(matches!(app.current_route, Route::Hub(_)));
    app.open_bot();
    resolve(&mut app).await;
    assert_eq!(app.transcript.session_id, Some(901));
    assert_eq!(app.composer.text(), "Bot draft stays here");
    assert_eq!(server.bot_requests(), 2);
}

#[tokio::test]
async fn failure_is_retryable_and_keeps_original_session_draft() {
    let server = Server::start().await;
    server.fail.store(true, std::sync::atomic::Ordering::SeqCst);
    let (_temp, mut app) = app(Some(&server.endpoint));
    app.transcript.session_id = Some(77);
    app.composer.insert_str("unfinished project work");
    app.open_bot();
    resolve(&mut app).await;
    assert!(app.bot.error.is_some());
    assert!(app.bot.task.is_none());
    assert_eq!(app.transcript.session_id, Some(77));
    assert_eq!(app.composer.text(), "unfinished project work");
    server
        .fail
        .store(false, std::sync::atomic::Ordering::SeqCst);
    app.handle_key_event(key(KeyCode::Enter));
    resolve(&mut app).await;
    assert_eq!(app.transcript.session_id, Some(901));
    assert_eq!(server.bot_requests(), 1);
    app.open_session(77, "Project".into());
    assert_eq!(app.composer.text(), "unfinished project work");
    assert!(!app.is_bot_session());
}

#[tokio::test]
async fn escape_and_reopen_reject_stale_success_and_failure() {
    let (_temp, mut app) = app(None);
    app.open_hub();
    app.composer.insert_str("keep draft");
    let hub_request_id = app.next_hub_request_id;
    app.open_bot();
    let old = app.bot.request_id;
    app.handle_key_event(key(KeyCode::Esc));
    assert!(matches!(&app.current_route, Route::Hub(state) if state.request_id > hub_request_id));
    app.handle_bot_session_ready(old, Ok(opened()));
    assert!(matches!(app.current_route, Route::Hub(_)));
    app.open_bot();
    let latest = app.bot.request_id;
    app.handle_bot_session_ready(old, Err(UiFailure::internal("stale failure")));
    assert!(app.bot.error.is_none());
    assert_eq!(app.bot.request_id, latest);
    app.handle_key_event(ctrl('g'));
    app.handle_bot_session_ready(latest, Ok(opened()));
    assert!(matches!(app.current_route, Route::Hub(_)));
    assert_eq!(app.composer.text(), "keep draft");
}

#[tokio::test]
async fn loading_and_error_fit_a_narrow_terminal_with_retry_and_back_visible() {
    let (_temp, mut app) = app(None);
    app.open_bot();
    for failed in [false, true] {
        if failed {
            app.handle_bot_session_ready(app.bot.request_id, Err(UiFailure::internal("offline")));
        }
        let mut terminal = Terminal::new(TestBackend::new(42, 18)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let buffer = terminal.backend().buffer();
        let text = (0..18)
            .map(|y| (0..42).map(|x| buffer[(x, y)].symbol()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("Named Bots"), "{text}");
        assert!(text.contains("Esc"), "{text}");
        assert!(
            text.contains(if failed { "retry" } else { "Enter" }),
            "{text}"
        );
    }
}

#[tokio::test]
async fn slash_bot_action_opens_canonical_session_without_a_selected_session() {
    let server = Server::start().await;
    let (_temp, mut app) = app(Some(&server.endpoint));
    let catalog = crate::commands::bot_tests::bot_catalog();
    let command = crate::commands::find_client_command(Some(&catalog), "bot", false).unwrap();
    assert!(app.transcript.session_id.is_none());
    app.execute_command(&command, "");
    assert!(matches!(app.current_route, Route::BotEntry(_)));
    resolve(&mut app).await;
    assert_eq!(app.transcript.session_id, Some(901));
    assert!(app.assistant_panels.is_empty());
    assert_eq!(server.bot_requests(), 1);
}

#[tokio::test]
async fn hub_search_keeps_plain_b_as_text_and_global_shortcut_still_opens_bot() {
    let (_temp, mut app) = app(None);
    app.open_hub();
    app.handle_key_event(key(KeyCode::Char('/')));
    app.handle_key_event(key(KeyCode::Char('b')));
    assert!(matches!(&app.current_route, Route::Hub(state) if state.query == "b"));
    app.handle_key_event(ctrl('b'));
    assert!(matches!(app.current_route, Route::BotEntry(_)));
}

#[tokio::test]
async fn navigation_keeps_file_attachments_in_the_original_draft() {
    let server = Server::start().await;
    let (_temp, mut app) = app(Some(&server.endpoint));
    app.transcript.session_id = Some(77);
    let resource = serde_json::from_value(serde_json::json!({
        "kind": "file", "reference": {"reference_type": "workspace_path", "path": "notes.md"},
        "name": "notes.md"
    }))
    .unwrap();
    let draft = crate::ComposerDraft {
        document: agena_domain::ComposerDocument(vec![
            agena_domain::ComposerNode::Text {
                text: "read these notes".into(),
            },
            agena_domain::ComposerNode::activity(agena_domain::ComposerActivity {
                id: agena_domain::ActivityId::new(),
                payload: agena_domain::ActivityPayload::Resource(resource),
                provenance: Default::default(),
            }),
        ]),
    };
    app.replace_composer_draft(draft.clone());
    let before = app.current_composer_draft();
    app.open_bot();
    resolve(&mut app).await;
    app.open_session(77, "Project".into());
    assert_eq!(app.current_composer_draft(), before);
}

#[tokio::test]
async fn bot_uses_full_session_surface_and_keeps_composer_and_transcript_at_42_by_18() {
    let (_temp, mut app) = app(None);
    app.open_bot();
    app.handle_bot_session_ready(app.bot.request_id, Ok(opened()));
    app.composer.insert_str("Plan my next project");
    let mut terminal = Terminal::new(TestBackend::new(42, 18)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    assert!(app.current_route_is_main());
    assert!(app.surface_layout.composer_editor.height >= 1);
    assert!(app.layout.transcript_body.height >= 6);
    let buffer = terminal.backend().buffer();
    let text = (0..18)
        .map(|y| (0..42).map(|x| buffer[(x, y)].symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("AI Bot"), "{text}");
    assert!(text.contains("Ctrl+G"), "{text}");
    assert!(text.contains("Plan my next project"), "{text}");
    println!("Bot session at 42x18:\n{text}");
}

#[tokio::test]
async fn clicking_the_fixed_hub_entry_opens_bot_without_a_catalog() {
    let (_temp, mut app) = app(None);
    app.current_route = Route::Hub(crate::HubState::new());
    app.layout.overlay_area = Rect::new(0, 0, 42, 18);
    app.handle_mouse_event(crossterm::event::MouseEvent {
        kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
        column: 5,
        row: 1,
        modifiers: KeyModifiers::NONE,
    });
    assert!(matches!(app.current_route, Route::BotEntry(_)));
}

#[tokio::test]
async fn empty_hub_bot_keyboard_and_mouse_paths_work_at_42_by_18_in_every_locale() {
    for (locale, _) in agena_tui::i18n::SUPPORTED_LOCALES {
        for input in 0..4 {
            let (_temp, mut app) = app(None);
            app.i18n = I18n::resolve(Some(locale), None);
            app.current_route = Route::Hub(crate::HubState::new());
            let mut terminal = Terminal::new(TestBackend::new(42, 18)).unwrap();
            terminal.draw(|frame| app.draw(frame)).unwrap();
            match input {
                0 => app.handle_key_event(key(KeyCode::Char('b'))),
                1 => app.handle_key_event(ctrl('b')),
                2 => app.handle_mouse_event(crossterm::event::MouseEvent {
                    kind: crossterm::event::MouseEventKind::Down(
                        crossterm::event::MouseButton::Left,
                    ),
                    column: app.layout.overlay_area.x + 5,
                    row: app.layout.overlay_area.y + 1,
                    modifiers: KeyModifiers::NONE,
                }),
                _ => {
                    let catalog = crate::commands::bot_tests::bot_catalog();
                    let command =
                        crate::commands::find_client_command(Some(&catalog), "bot", false).unwrap();
                    app.execute_command(&command, "");
                }
            }
            assert!(
                matches!(app.current_route, Route::BotEntry(_)),
                "{locale}: input {input}"
            );
            let request = app.bot.request_id;
            app.handle_bot_session_ready(request, Ok(opened()));
            app.composer.insert_str("Session draft");
            terminal.draw(|frame| app.draw(frame)).unwrap();
            assert!(app.current_route_is_main(), "{locale}: input {input}");
            assert_eq!(app.transcript.session_id, Some(901));
            assert!(app.surface_layout.composer_editor.height >= 1, "{locale}");
            assert!(app.layout.transcript_body.height >= 6, "{locale}");
        }
    }
}

#[tokio::test]
async fn assistant_uses_own_persona_for_bot_and_workspace_for_project() {
    let server = Server::start().await;
    let (_temp, mut app) = app(Some(&server.endpoint));
    app.open_session(77, "Project".into());
    app.open_assistant();
    timeout(Duration::from_secs(10), async {
        while !matches!(app.current_route, Route::Assistant(77)) {
            let message = app.rx.recv().await.unwrap();
            app.handle_message(message);
        }
    })
    .await
    .unwrap();
    assert!(app.assistant_panels.contains_key(&77));
    app.current_route = Route::Main;
    app.open_bot();
    resolve(&mut app).await;
    app.open_assistant();
    resolve(&mut app).await;
    assert!(matches!(app.bot.view, BotView::Form));
    assert_eq!(
        app.bot.form.as_ref().unwrap().base.as_ref().unwrap().id,
        Uuid::from_u128(1)
    );
    assert!(!app.assistant_panels.contains_key(&901));
}

fn profile(id: u128, name: &str) -> BotProfile {
    BotProfile {
        id: Uuid::from_u128(id),
        name: name.into(),
        persona: BotPersona::default(),
        revision: 1,
        archived: false,
        local_session_id: None,
        created_at_ms: 0,
        updated_at_ms: 0,
    }
}
fn opened() -> BotReply {
    let bot = profile(1, "AI Bot");
    BotReply::Opened {
        session: agena_api::bot::BotSession::new(901, &bot),
        profile: bot,
        list: BotList {
            items: vec![profile(1, "AI Bot")],
            default_bot_id: Some(Uuid::from_u128(1)),
        },
    }
}

fn list_reply(items: Vec<BotProfile>) -> BotReply {
    BotReply::List(BotList {
        default_bot_id: items.iter().find(|p| !p.archived).map(|p| p.id),
        items,
    })
}
fn form_field(app: &mut App, index: usize, value: &str) {
    let form = app.bot.form.as_mut().unwrap();
    form.index = index;
    let mut editor = Editor::default();
    editor.set_text(value.into());
    form.edit = Some(editor);
    app.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::CONTROL));
}
async fn wait_main(app: &mut App) {
    timeout(Duration::from_secs(10), async {
        while !app.current_route_is_main() {
            let msg = app.rx.recv().await.unwrap();
            app.handle_message(msg);
        }
    })
    .await
    .unwrap();
}
fn screen(app: &mut App, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|f| app.draw(f)).unwrap();
    let buffer = terminal.backend().buffer();
    (0..height)
        .map(|y| {
            (0..width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[tokio::test]
async fn two_profiles_switch_independent_sessions_drafts_and_reenter_selected_uuid() {
    let server = Server::start().await;
    server.profiles.lock().unwrap().push(profile(2, "Research"));
    let (_temp, mut app) = app(Some(&server.endpoint));
    app.open_bot();
    resolve(&mut app).await;
    app.composer.insert_str("first draft");
    app.open_bot_picker();
    resolve(&mut app).await;
    assert_eq!(app.bot.profiles.len(), 2);
    let rendered = screen(&mut app, 64, 22);
    assert!(rendered.contains("AI Bot (current)"), "{rendered}");
    assert!(
        rendered.contains("No additional system instructions"),
        "{rendered}"
    );
    app.handle_key_event(key(KeyCode::Down));
    app.handle_key_event(key(KeyCode::Enter));
    resolve(&mut app).await;
    assert_eq!(app.transcript.session_id, Some(902));
    assert_eq!(app.bot.selected, Some(Uuid::from_u128(2)));
    assert!(app.composer.text().is_empty());
    app.composer.insert_str("second draft");
    app.open_hub();
    app.open_bot();
    resolve(&mut app).await;
    assert_eq!(app.transcript.session_id, Some(902));
    assert_eq!(app.composer.text(), "second draft");
    app.execute_bot_command(&Uuid::from_u128(1).to_string());
    resolve(&mut app).await;
    assert_eq!(app.transcript.session_id, Some(901));
    assert_eq!(app.composer.text(), "first draft");
    assert!(
        server
            .requests
            .lock()
            .unwrap()
            .iter()
            .any(|s| s.contains("/00000000-0000-0000-0000-000000000002/session"))
    );
}

#[tokio::test]
async fn create_edit_rename_instructions_use_typed_profile_and_keep_native_title() {
    let server = Server::start().await;
    let (_temp, mut app) = app(Some(&server.endpoint));
    let command = crate::commands::find_client_command(
        Some(&crate::commands::bot_tests::bot_catalog()),
        "bot",
        false,
    )
    .unwrap();
    app.execute_command(&command, "create");
    assert!(matches!(app.bot.view, BotView::Form));
    assert_eq!(
        app.bot.form.as_ref().unwrap().persona,
        BotPersona::default()
    );
    form_field(&mut app, 0, "Planner");
    form_field(&mut app, 1, "先听完\n再提出具体建议");
    app.handle_key_event(key(KeyCode::Char('s')));
    wait_main(&mut app).await;
    assert_eq!(app.bot.selected, Some(Uuid::from_u128(2)));
    assert_eq!(app.transcript.session_id, Some(902));
    let created = server.profiles.lock().unwrap()[1].clone();
    assert_eq!(created.persona.instructions, "先听完\n再提出具体建议");
    assert_eq!(
        server.profiles.lock().unwrap()[0].persona,
        BotPersona::default()
    );
    app.transcript.session_title = "My custom conversation title".into();
    app.open_assistant();
    resolve(&mut app).await;
    form_field(&mut app, 0, "Renamed Planner");
    form_field(&mut app, 1, "New\nindependent persona");
    app.handle_key_event(key(KeyCode::Char('s')));
    resolve(&mut app).await;
    resolve(&mut app).await;
    assert_eq!(app.transcript.session_title, "My custom conversation title");
    assert!(app.transcript_surface_title().contains("Renamed Planner"));
    let mutations = server.bodies.lock().unwrap();
    let (_, update) = mutations
        .iter()
        .find(|(s, _)| s.starts_with("PUT /api/v1/bots/"))
        .unwrap();
    assert_eq!(update["expected_revision"], 1);
    assert_eq!(
        update["persona"],
        serde_json::json!({"instructions": "New\nindependent persona"})
    );
    assert_eq!(
        update["persona"]["instructions"],
        "New\nindependent persona"
    );
    assert!(
        update["request_key"]
            .as_str()
            .is_some_and(|s| !s.is_empty())
    );
    assert!(
        !mutations
            .iter()
            .any(|(s, _)| s.contains("/assistant/preferences"))
    );
}

#[tokio::test]
async fn revision_conflict_preserves_draft_and_requires_explicit_reload_confirmation() {
    let server = Server::start().await;
    let (_temp, mut app) = app(Some(&server.endpoint));
    app.open_bot();
    resolve(&mut app).await;
    app.open_assistant();
    resolve(&mut app).await;
    form_field(&mut app, 0, "My unsaved name");
    {
        let mut registry = server.profiles.lock().unwrap();
        registry[0].revision = 2;
        registry[0].name = "Other client edit".into();
    }
    app.handle_key_event(key(KeyCode::Char('s')));
    resolve(&mut app).await;
    assert!(app.bot.error.is_some());
    assert_eq!(app.bot.form.as_ref().unwrap().name, "My unsaved name");
    assert_eq!(
        app.bot
            .form
            .as_ref()
            .unwrap()
            .base
            .as_ref()
            .unwrap()
            .revision,
        1
    );
    assert!(screen(&mut app, 42, 18).to_lowercase().contains("conflict"));
    app.handle_key_event(key(KeyCode::Char('r')));
    assert!(matches!(app.bot.view, BotView::Reload));
    app.handle_key_event(key(KeyCode::Esc));
    assert_eq!(app.bot.form.as_ref().unwrap().name, "My unsaved name");
    app.handle_key_event(key(KeyCode::Char('r')));
    app.handle_key_event(key(KeyCode::Enter));
    resolve(&mut app).await;
    assert_eq!(app.bot.form.as_ref().unwrap().name, "Other client edit");
    assert_eq!(
        app.bot
            .form
            .as_ref()
            .unwrap()
            .base
            .as_ref()
            .unwrap()
            .revision,
        2
    );
}

#[tokio::test]
async fn archive_confirmation_idle_error_default_fallback_and_all_archived_create_state() {
    let server = Server::start().await;
    server.profiles.lock().unwrap().push(profile(2, "Research"));
    let (_temp, mut app) = app(Some(&server.endpoint));
    app.open_bot();
    resolve(&mut app).await;
    app.open_bot_picker();
    resolve(&mut app).await;
    app.handle_key_event(key(KeyCode::Char('a')));
    assert!(matches!(app.bot.view, BotView::Archive));
    assert!(screen(&mut app, 42, 18).contains("history"));
    app.handle_key_event(key(KeyCode::Esc));
    assert!(
        !server
            .requests
            .lock()
            .unwrap()
            .iter()
            .any(|s| s.contains("/archive"))
    );
    server.busy.store(true, std::sync::atomic::Ordering::SeqCst);
    app.handle_key_event(key(KeyCode::Char('a')));
    app.handle_key_event(key(KeyCode::Enter));
    resolve(&mut app).await;
    assert!(app.bot.error.is_some());
    assert!(!server.profiles.lock().unwrap()[0].archived);
    server
        .busy
        .store(false, std::sync::atomic::Ordering::SeqCst);
    app.handle_key_event(key(KeyCode::Enter));
    resolve(&mut app).await;
    resolve(&mut app).await;
    assert!(server.profiles.lock().unwrap()[0].archived);
    assert_eq!(app.bot.selected, None);
    app.open_bot();
    resolve(&mut app).await;
    assert_eq!(app.transcript.session_id, Some(902));
    app.open_bot_picker();
    resolve(&mut app).await;
    app.handle_key_event(key(KeyCode::Char('a')));
    app.handle_key_event(key(KeyCode::Enter));
    resolve(&mut app).await;
    resolve(&mut app).await;
    let requests = server.bot_requests();
    app.open_bot();
    resolve(&mut app).await;
    assert!(matches!(app.bot.view, BotView::List));
    assert!(screen(&mut app, 42, 18).contains("Press n"));
    assert_eq!(server.bot_requests(), requests);
    assert_eq!(server.profiles.lock().unwrap().len(), 2);
    app.handle_key_event(key(KeyCode::Char('n')));
    assert!(app.bot.form.as_ref().unwrap().base.is_none());
}

#[tokio::test]
async fn invalid_archived_ids_and_stale_switch_response_never_focus_another_bot() {
    let server = Server::start().await;
    let (_temp, mut app) = app(Some(&server.endpoint));
    app.transcript.session_id = Some(77);
    app.composer.insert_str("project draft");
    app.execute_bot_command(&Uuid::from_u128(999).to_string());
    resolve(&mut app).await;
    assert!(app.bot.error.is_some());
    assert_eq!(app.transcript.session_id, Some(77));
    assert_eq!(server.bot_requests(), 0);
    app.execute_bot_command(&Uuid::from_u128(1).to_string());
    let old = app.bot.request_id;
    app.execute_bot_command(&Uuid::from_u128(2).to_string());
    let latest = app.bot.request_id;
    app.handle_bot_session_ready(old, Ok(opened()));
    assert_eq!(app.transcript.session_id, Some(77));
    assert_eq!(app.bot.request_id, latest);
    app.handle_bot_session_ready(latest, Err(UiFailure::internal("unavailable")));
    app.handle_bot_session_ready(old, Err(UiFailure::internal("stale failure")));
    assert_eq!(app.bot.error.as_deref(), Some("unavailable"));
    assert_eq!(app.composer.text(), "project draft");
    server.profiles.lock().unwrap()[0].archived = true;
    app.execute_bot_command(&Uuid::from_u128(1).to_string());
    resolve(&mut app).await;
    assert!(app.bot.error.is_some());
    assert_eq!(server.bot_requests(), 0);
}

#[tokio::test]
async fn empty_registry_uses_server_default_and_recovered_session_stays_same_uuid() {
    let server = Server::start().await;
    server.profiles.lock().unwrap().clear();
    let (_temp, mut app) = app(Some(&server.endpoint));
    app.open_bot();
    resolve(&mut app).await;
    assert_eq!(app.bot.selected, Some(Uuid::from_u128(1)));
    assert_eq!(server.bot_requests(), 1);
    let original = server.profiles.lock().unwrap()[0].clone();
    *server.next_binding.lock().unwrap() = Some(BotSession {
        session_id: 1001,
        bot_id: original.id,
        name: original.name.clone(),
    });
    app.open_hub();
    app.open_bot();
    resolve(&mut app).await;
    assert_eq!(app.bot.selected, Some(Uuid::from_u128(1)));
    assert_eq!(app.transcript.session_id, Some(1001));
    assert_eq!(app.current_bot_profile().unwrap().name, original.name);
    assert_eq!(app.current_bot_profile().unwrap().persona, original.persona);
}

#[tokio::test]
async fn list_create_editor_confirm_and_error_layouts_keep_hints_in_all_eight_locales() {
    for (locale, _) in agena_tui::i18n::SUPPORTED_LOCALES {
        let (_temp, mut app) = app(None);
        app.i18n = I18n::resolve(Some(locale), None);
        app.open_bot_picker();
        let request = app.bot.request_id;
        let mut first = profile(1, "小鱼 Research");
        first.persona.instructions = "保持中文、自然、简洁\nSecond line".into();
        app.handle_bot_session_ready(request, Ok(list_reply(vec![first, profile(2, "Planner")])));
        for (width, height) in [(42, 18), (64, 24), (110, 30)] {
            app.bot.view = BotView::List;
            let rendered = screen(&mut app, width, height);
            assert!(rendered.contains("Research"), "{locale} {rendered}");
            assert!(rendered.contains("Esc"), "{locale} {rendered}");
            assert!(rendered.contains("Planner"), "{locale} {rendered}");
            app.new_bot_form();
            form_field(&mut app, 0, "小鱼");
            app.bot.form.as_mut().unwrap().index = 1;
            app.handle_key_event(key(KeyCode::Enter));
            assert!(app.has_active_text_input());
            app.handle_paste("第一行\nsecond line".into());
            let rendered = screen(&mut app, width, height);
            assert!(rendered.contains("second line"), "{locale} {rendered}");
            assert!(rendered.contains("Ctrl+Enter"), "{locale} {rendered}");
            app.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT));
            assert_eq!(
                app.bot.form.as_ref().unwrap().persona.instructions,
                "第一行\nsecond line"
            );
            app.bot.view = BotView::Archive;
            assert!(screen(&mut app, width, height).contains("Enter"));
            app.bot.view = BotView::Form;
            app.bot.error = Some("409 Conflict: another client changed this Bot".into());
            let rendered = screen(&mut app, width, height);
            assert!(rendered.contains("409"), "{locale} {rendered}");
        }
    }
}

#[tokio::test]
async fn hub_explicit_list_keyboard_and_click_work_without_project_or_session() {
    for mouse in [false, true] {
        let (_temp, mut app) = app(None);
        app.current_route = Route::Hub(crate::HubState::new());
        let text = screen(&mut app, 42, 18);
        assert!(text.contains("l Bot list"), "{text}");
        if mouse {
            app.handle_mouse_event(crossterm::event::MouseEvent {
                kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
                column: app.layout.overlay_area.x + 25,
                row: app.layout.overlay_area.y + 1,
                modifiers: KeyModifiers::NONE,
            });
        } else {
            app.handle_key_event(key(KeyCode::Char('l')));
        }
        assert!(matches!(app.current_route, Route::BotEntry(_)));
        assert!(matches!(app.bot.view, BotView::List));
        assert!(app.transcript.session_id.is_none());
    }
}

#[tokio::test]
async fn ordinary_session_registry_read_does_not_resolve_or_select_a_bot() {
    let server = Server::start().await;
    let (_temp, mut app) = app(Some(&server.endpoint));
    app.open_session(77, "Ordinary project".into());
    let request = app.bot.scope_request;
    timeout(Duration::from_secs(10), async {
        while app.bot.profiles.is_empty() {
            let message = app.rx.recv().await.unwrap();
            app.handle_message(message);
        }
    })
    .await
    .unwrap();
    assert_eq!(app.bot.scope_request, request);
    assert_eq!(app.bot.selected, None);
    assert_eq!(app.transcript.session_id, Some(77));
    assert_eq!(server.bot_requests(), 0);
    assert!(!app.is_bot_session());
}

#[tokio::test]
async fn stale_profile_and_archive_results_cannot_replace_new_picker_or_session() {
    let (_temp, mut app) = app(None);
    app.open_bot_picker();
    let old = app.bot.request_id;
    app.open_bot();
    let newer = app.bot.request_id;
    app.handle_bot_session_ready(old, Ok(BotReply::Saved(profile(2, "Old save"))));
    app.handle_bot_session_ready(
        old,
        Ok(BotReply::Archived({
            let mut p = profile(1, "AI Bot");
            p.archived = true;
            p
        })),
    );
    app.handle_bot_session_ready(old, Ok(list_reply(vec![profile(3, "Old list")])));
    assert_eq!(app.bot.request_id, newer);
    assert!(app.bot.profiles.is_empty());
    app.handle_bot_session_ready(newer, Ok(opened()));
    let active = app.bot.selected;
    app.handle_bot_session_ready(old, Ok(BotReply::Edit(profile(2, "Old editor"))));
    assert!(app.current_route_is_main());
    assert_eq!(app.bot.selected, active);
}

#[tokio::test]
async fn generic_session_entry_resolves_registry_name_and_persona_from_its_owner() {
    let server = Server::start().await;
    server
        .profiles
        .lock()
        .unwrap()
        .push(profile(2, "Research persona"));
    let (_temp, mut app) = app(Some(&server.endpoint));
    app.open_session(902, "Custom durable chat title".into());
    // Ownership arrives on the session resource (`owner_bot_id`), not from a
    // registry lookup by session id: the registry no longer names a session.
    app.transcript.owner_bot_id = Some(Uuid::from_u128(2));
    timeout(Duration::from_secs(10), async {
        while !app.is_bot_session() {
            let message = app.rx.recv().await.unwrap();
            app.handle_message(message);
        }
    })
    .await
    .unwrap();
    assert_eq!(app.bot.selected, Some(Uuid::from_u128(2)));
    assert_eq!(app.transcript.session_title, "Custom durable chat title");
    assert!(app.transcript_surface_title().contains("Research persona"));
    app.open_assistant();
    resolve(&mut app).await;
    assert_eq!(
        app.bot.form.as_ref().unwrap().base.as_ref().unwrap().id,
        Uuid::from_u128(2)
    );
    assert!(app.assistant_panels.is_empty());
}

#[tokio::test]
async fn long_profile_names_keep_current_marker_and_chat_navigation_hints_visible() {
    let (_temp, mut app) = app(None);
    let p = profile(1, "Research persona with a very long descriptive name 小鱼");
    app.open_bot();
    app.handle_bot_session_ready(
        app.bot.request_id,
        Ok(BotReply::Opened {
            session: agena_api::bot::BotSession::new(901, &p),
            profile: p.clone(),
            list: BotList {
                items: vec![p.clone()],
                default_bot_id: Some(p.id),
            },
        }),
    );
    for width in [42, 64, 110] {
        let rendered = screen(&mut app, width, 18);
        assert!(rendered.contains("Research"), "{rendered}");
        assert!(rendered.contains("/bot list"), "{rendered}");
        assert!(rendered.contains("Ctrl+G"), "{rendered}");
    }
    app.open_bot_picker();
    let request = app.bot.request_id;
    app.handle_bot_session_ready(request, Ok(list_reply(vec![p])));
    let rendered = screen(&mut app, 42, 18);
    assert!(rendered.contains("(current)"), "{rendered}");
    assert!(rendered.contains('…'), "{rendered}");
}

#[tokio::test]
async fn mismatched_named_session_reply_cannot_bind_another_bot_or_consume_draft() {
    let server = Server::start().await;
    let (_temp, mut app) = app(Some(&server.endpoint));
    app.open_session(77, "Project".into());
    app.composer.insert_str("keep this draft");
    *server.next_binding.lock().unwrap() = Some(BotSession {
        session_id: 999,
        bot_id: Uuid::from_u128(999),
        name: "Another Bot".into(),
    });
    app.execute_bot_command(&Uuid::from_u128(1).to_string());
    resolve(&mut app).await;
    assert!(
        app.bot
            .error
            .as_ref()
            .is_some_and(|e| e.contains("another Bot identity"))
    );
    assert_eq!(app.transcript.session_id, Some(77));
    assert_eq!(app.composer.text(), "keep this draft");
}

#[tokio::test]
async fn two_fields_cycle_and_enforce_utf8_byte_limit() {
    let server = Server::start().await;
    let (_temp, mut app) = app(Some(&server.endpoint));
    app.execute_bot_command("create");
    assert_eq!(app.bot.form.as_ref().unwrap().index, 0);
    app.handle_key_event(key(KeyCode::Tab));
    assert_eq!(app.bot.form.as_ref().unwrap().index, 1);
    app.handle_key_event(key(KeyCode::Down));
    assert_eq!(app.bot.form.as_ref().unwrap().index, 0);
    app.handle_key_event(key(KeyCode::Up));
    assert_eq!(app.bot.form.as_ref().unwrap().index, 1);
    let rendered = screen(&mut app, 42, 18);
    assert!(
        rendered.contains("Additional system instructions"),
        "{rendered}"
    );
    assert!(!rendered.contains("Tone"), "{rendered}");
    form_field(&mut app, 0, "Bytes");
    // Only the storage ceiling is refused; the save keeps the form open.
    form_field(&mut app, 1, &"a".repeat(MAX_BOT_INSTRUCTIONS_BYTES + 1));
    app.handle_key_event(key(KeyCode::Char('s')));
    assert!(app.bot.error.is_some());
    assert!(server.profiles.lock().unwrap().len() == 1);
    // A multi-kilobyte character sheet is ordinary and saves verbatim.
    form_field(&mut app, 1, &"界".repeat(2000));
    app.handle_key_event(key(KeyCode::Char('s')));
    wait_main(&mut app).await;
    assert_eq!(
        server.profiles.lock().unwrap()[1]
            .persona
            .instructions
            .len(),
        "界".repeat(2000).len()
    );
}

#[tokio::test]
async fn narrow_list_clips_long_utf8_instructions_and_shows_empty_hint() {
    let (_temp, mut app) = app(None);
    app.open_bot_picker();
    let mut first = profile(1, "Long");
    first.persona.instructions = "指令".repeat(1000);
    app.handle_bot_session_ready(
        app.bot.request_id,
        Ok(list_reply(vec![first, profile(2, "Empty")])),
    );
    let rendered = screen(&mut app, 42, 18);
    assert!(rendered.contains("指 令"), "{rendered}");
    assert!(rendered.contains('…'), "{rendered}");
    assert!(
        rendered.contains("No additional system instructions"),
        "{rendered}"
    );
}

#[tokio::test]
async fn tab_loads_one_bots_conversations_grouped_by_surface() {
    let server = Server::start().await;
    let (_temp, mut app) = app(Some(&server.endpoint));
    app.open_bot_picker();
    resolve(&mut app).await;
    assert!(matches!(app.bot.view, BotView::List));
    assert!(
        app.bot.conversations.is_empty(),
        "the Bot list loads before any rooms"
    );

    // Tab fetches only the highlighted Bot's conversations.
    app.handle_key_event(key(KeyCode::Tab));
    resolve(&mut app).await;
    let surfaces: Vec<&str> = app
        .bot
        .conversations
        .iter()
        .filter(|group| !group.rows.is_empty())
        .map(|group| group.surface)
        .collect();
    assert_eq!(
        surfaces,
        vec!["local", "group"],
        "surfaces keep the sidebar order"
    );
    assert_eq!(
        app.bot
            .conversations
            .iter()
            .map(|g| g.rows.len())
            .sum::<usize>(),
        2
    );
    assert!(
        app.bot.focus_conversations,
        "Tab moves the cursor into the rooms"
    );

    // Newest first inside a surface.
    let group = app
        .bot
        .conversations
        .iter()
        .find(|g| g.surface == "group")
        .unwrap();
    assert_eq!(group.rows[0].session_id, 902);
    assert!(group.rows[0].favorite);

    // Moving off the Bot must not leave another Bot's rooms on screen.
    app.bot.focus_conversations = false;
    app.handle_key_event(key(KeyCode::Down));
    assert!(app.bot.conversations.is_empty(), "rooms are per Bot");
}

#[tokio::test]
async fn new_and_delete_act_on_one_conversation_not_the_bot() {
    let server = Server::start().await;
    let (_temp, mut app) = app(Some(&server.endpoint));
    app.open_bot_picker();
    resolve(&mut app).await;
    app.handle_key_event(key(KeyCode::Tab));
    resolve(&mut app).await;
    assert!(app.bot.focus_conversations);

    // `n` while the cursor is in the rooms starts a conversation, not a Bot.
    app.handle_key_event(key(KeyCode::Char('n')));
    resolve(&mut app).await;
    assert!(
        !server
            .requests
            .lock()
            .unwrap()
            .iter()
            .any(|line| line.contains("POST /api/v1/bots ")),
        "the room list must not create a Bot"
    );
    assert!(
        server
            .requests
            .lock()
            .unwrap()
            .iter()
            .any(|line| line.contains("/sessions")),
        "the room list asks the server for a new conversation"
    );

    // `d` asks for confirmation before deleting.
    app.bot.view = BotView::List;
    app.bot.focus_conversations = true;
    app.bot.conversation_index = 0;
    app.handle_key_event(key(KeyCode::Char('d')));
    assert!(matches!(app.bot.view, BotView::DeleteConversation));
    let pending = app.bot.delete_conversation.clone().expect("pending row");
    // Escape keeps it.
    app.handle_key_event(key(KeyCode::Esc));
    assert!(app.bot.delete_conversation.is_none());
    assert!(matches!(app.bot.view, BotView::List));
    // Enter deletes exactly that conversation.
    app.bot.focus_conversations = true;
    app.bot.delete_conversation = Some(pending.clone());
    app.bot.view = BotView::DeleteConversation;
    app.handle_key_event(key(KeyCode::Enter));
    resolve(&mut app).await;
    assert!(
        server
            .requests
            .lock()
            .unwrap()
            .iter()
            .any(|line| line.contains(&format!("/api/v1/sessions/{}", pending.session_id))),
        "the delete targets the conversation, not the Bot"
    );
}
