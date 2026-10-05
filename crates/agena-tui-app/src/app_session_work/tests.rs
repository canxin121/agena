use super::*;
use crate::{I18n, LaunchOptions, TuiBackend};
use ratatui::{Terminal, backend::TestBackend};

fn app() -> App {
    let mut app = App::new_with_backend(
        TuiBackend::remote_mock(),
        LaunchOptions::default(),
        I18n::english(),
    );
    app.transcript.session_id = Some(7);
    app
}
fn files() -> FilePage {
    serde_json::from_value(serde_json::json!({"files":[{"path":"src/main.rs","index":" ","workingDir":"M"}],"totalFiles":1,"hasMore":false})).unwrap()
}

#[tokio::test]
async fn tool_execution_stays_out_of_accessories_and_resolved_retry_collapses() {
    let mut app = app();
    app.transcript.execution = Some(
        serde_json::from_value(serde_json::json!({
            "session": {
                "id": 7, "depth": 0, "root_id": 7, "workspace_id": 1,
                "title": "Tool execution", "version": 1,
                "relation_kind": "root", "lifecycle_state": "ready",
                "state": {"kind": "running", "data": {"workflow": "tool_pending"}},
                "created_at": "2026-10-01T00:00:00Z", "updated_at": "2026-10-01T00:00:00Z",
                "message_count": 0, "child_session_count": 0
            },
            "parts": [], "execution": {"agent_id": "default"}, "usage": {"current_tokens": 0}
        }))
        .unwrap(),
    );
    let mut terminal = Terminal::new(TestBackend::new(100, 14)).unwrap();
    let render = |app: &mut App, terminal: &mut Terminal<TestBackend>| {
        terminal
            .draw(|frame| app.render_session_work(frame, frame.area()))
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>()
    };
    let normal = render(&mut app, &mut terminal);
    assert!(!normal.contains(&app.i18n.text("session-work-waiting")));
    assert!(!normal.contains(&app.i18n.text("session-work-stop")));

    if let agena_api::resource::SessionState::Running { requests, .. } =
        &mut app.transcript.execution.as_mut().unwrap().session.state
    {
        requests.push(agena_api::resource::PendingInteractiveRequestResource {
            session_id: 7,
            parent_session_id: None,
            task_id: None,
            request: agena_api::resource::PendingInteractiveRequest::Permission {
                request: agena_api::resource::PermissionRequest {
                    request_id: "permission-7".into(),
                    session_id: Some(7),
                    action: agena_api::resource::PermissionActionResource::Tool {
                        tool_name: "fs.write".into(),
                        qualifier: None,
                    },
                    related_actions: Vec::new(),
                    requested_actions: Vec::new(),
                    reason: "write the requested file".into(),
                    explanation: String::new(),
                    source: None,
                    scope: None,
                    operator: None,
                    trace: Vec::new(),
                    created_at: chrono::Utc::now(),
                },
            },
        });
    }
    assert!(!render(&mut app, &mut terminal).contains(&app.i18n.text("session-work-requests")));

    app.transcript
        .execution
        .as_mut()
        .unwrap()
        .execution
        .provider_retry = Some(agena_domain::ProviderRetryStatus {
        attempt: 2,
        max_retries: 5,
        next_at_ms: chrono::Utc::now().timestamp_millis() + 10_000,
        message: "Provider unavailable; retrying shortly".into(),
    });
    assert!(render(&mut app, &mut terminal).contains("↻ 2"));
    app.handle_session_work_action("work-status");
    assert!(render(&mut app, &mut terminal).contains("Provider unavailable; retrying shortly"));

    app.transcript
        .execution
        .as_mut()
        .unwrap()
        .execution
        .provider_retry = None;
    app.heal_session_work();
    assert_eq!(app.session_work_height(30), 1);
    assert_eq!(app.work_focus, None);
    let resumed = render(&mut app, &mut terminal);
    assert!(!resumed.contains("retrying shortly"));
    assert!(!resumed.contains(&app.i18n.text("session-work-waiting")));
    app.handle_session_work_action("work-toggle");
    assert_eq!(app.session_work[&7].tab, Tab::Files);
    assert!(app.session_work[&7].expanded);
}

#[tokio::test]
async fn accessories_align_with_composer_and_leave_room_for_chat() {
    let mut app = app();
    app.composer.insert_str("MAIN DRAFT");
    app.session_work.insert(
        7,
        SessionWorkState {
            expanded: true,
            files: Some(files()),
            ..Default::default()
        },
    );
    for (width, height) in [(100, 35), (70, 24), (40, 20), (28, 12)] {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        assert_eq!(app.work_area.width, app.surface_layout.composer_outer.width);
        assert_eq!(app.work_area.x, app.surface_layout.composer_outer.x);
        assert!(app.layout.transcript_body.height > 0);
        let text: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(text.contains("MAIN DRAFT"));
    }
    app.work_focus = Some(7);
    assert!(app.handle_session_work_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
    assert_eq!(app.focus, Focus::Composer);
    app.open_btw("");
    assert!(!app.session_work[&7].expanded);
}

#[test]
fn switched_sessions_and_changed_selections_reject_old_reads() {
    let mut app = app();
    app.session_work.insert(
        7,
        SessionWorkState {
            file_request: 11,
            detail_request: 12,
            ..Default::default()
        },
    );
    app.transcript.session_id = Some(8);
    app.handle_session_work_loaded(7, 10, 0, Ok(WorkResult::Files(files())));
    assert!(app.session_work[&7].files.is_none());
    app.handle_session_work_loaded(7, 11, 0, Ok(WorkResult::Files(files())));
    assert_eq!(app.session_work[&7].files.as_ref().unwrap().total_files, 1);
    assert!(!app.session_work.contains_key(&8));
    app.session_work
        .get_mut(&7)
        .unwrap()
        .select_detail(Some(Detail::File("new.rs".into(), false)));
    app.handle_session_work_loaded(7, 12, 1, Ok(WorkResult::Diff("old content".into(), false)));
    assert!(app.session_work[&7].diff.is_empty());
    app.transcript.session_id = Some(7);
    let state = app.session_work.get_mut(&7).unwrap();
    state.tab = Tab::Status;
    state.expanded = true;
    app.work_focus = Some(7);
    app.heal_session_work();
    assert!(
        !app.session_work[&7].expanded,
        "resolved status must stop occupying the transcript"
    );
    assert_eq!(app.work_focus, None);
}

#[test]
fn log_tails_deduplicate_and_bound_retained_output() {
    let mut app = app();
    app.session_work.insert(
        7,
        SessionWorkState {
            detail_request: 1,
            ..Default::default()
        },
    );
    let read = |start, end| BackgroundActivityLogResource {
        activity_id: "task_a".into(),
        status: "running".into(),
        lines: (start..=end)
            .map(
                |seq| agena_api::resource::BackgroundActivityLogLineResource {
                    seq,
                    stream: "stdout".into(),
                    ts_ms: 0,
                    text: seq.to_string(),
                },
            )
            .collect(),
        last_seq: end,
        has_more: false,
        dropped_lines: 0,
        exit_code: None,
        completion_reason: None,
    };
    app.handle_session_work_loaded(7, 1, 1, Ok(WorkResult::Logs(read(1, 190))));
    app.handle_session_work_loaded(7, 1, 1, Ok(WorkResult::Logs(read(180, 310))));
    let lines = &app.session_work[&7].logs.as_ref().unwrap().lines;
    assert_eq!(lines.len(), 200);
    assert_eq!(lines[0].seq, 111);
    assert_eq!(lines[199].seq, 310);
    let mut huge = read(311, 311);
    huge.lines[0].text = "文🙂".repeat(80_000);
    app.handle_session_work_loaded(7, 1, 1, Ok(WorkResult::Logs(huge)));
    let lines = &app.session_work[&7].logs.as_ref().unwrap().lines;
    assert!(lines.iter().map(|l| l.text.len()).sum::<usize>() <= 128 * 1024);
    assert!(lines.last().unwrap().text.starts_with('…'));
}

#[tokio::test]
async fn visible_file_rows_are_clickable_without_leaving_the_conversation() {
    use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    let mut app = app();
    app.composer.insert_str("unsent draft");
    app.session_work.insert(
        7,
        SessionWorkState {
            expanded: true,
            files: Some(files()),
            ..Default::default()
        },
    );
    let mut terminal = Terminal::new(TestBackend::new(70, 26)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let mut target = None;
    for row in 0..26 {
        for column in 0..70 {
            let event = MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column,
                row,
                modifiers: KeyModifiers::NONE,
            };
            if app.pointer_targets.action(event) == Some(PointerAction::NamedIndex("work-row", 0)) {
                target = Some(event);
            }
        }
    }
    app.handle_mouse_event(target.expect("file row must own a pointer target"));
    assert_eq!(
        app.session_work[&7].detail,
        Some(Detail::File("src/main.rs".into(), false))
    );
    assert!(matches!(app.current_route, Route::Main));
    assert_eq!(app.composer.text(), "unsent draft");
    assert_eq!(app.work_focus, Some(7));
}

#[tokio::test]
async fn panel_header_uses_full_width_and_keyboard_entry_needs_no_function_key() {
    let mut app = app();
    app.composer.insert_str("draft to preserve");
    for width in [28, 40, 80] {
        let mut terminal = Terminal::new(TestBackend::new(width, 1)).unwrap();
        terminal
            .draw(|frame| app.render_session_work(frame, frame.area()))
            .unwrap();
        let cells = terminal.backend().buffer().content();
        let text: String = cells.iter().map(|c| c.symbol()).collect();
        assert!(text.contains("Workspace") && text.contains("Files"));
        assert!(!text.contains("F6") && !text.contains('…') && !text.contains("Ctrl"));
        assert_eq!(cells[0].bg, cells[usize::from(width) - 1].bg);
    }
    assert!(!app.handle_session_work_key(KeyEvent::new(KeyCode::F(6), KeyModifiers::NONE)));
    let toggle = KeyEvent::new(KeyCode::Char(']'), KeyModifiers::CONTROL);
    assert!(app.handle_session_work_key(toggle));
    assert!(app.session_work[&7].expanded);
    assert_eq!(app.work_focus, Some(7));
    assert!(app.handle_session_work_key(toggle));
    assert!(!app.session_work[&7].expanded);
    assert_eq!(app.composer.text(), "draft to preserve");
    app.interaction_editing = Some("ask".into());
    assert!(!app.handle_session_work_key(toggle));
    assert!(
        app.composer_status_parts()
            .iter()
            .all(|p| !matches!(p.kind, "btw" | "side" | "side-parent"))
    );
}
