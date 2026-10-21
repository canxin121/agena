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
    serde_json::from_value(serde_json::json!({"files":[{"path":"src/main.rs","operation_count":1,"operation_history":false,"operations":[]}],"total_files":1,"has_more":false,"offset":0,"recording_incomplete":false})).unwrap()
}

fn buffer_text(terminal: &Terminal<TestBackend>) -> String {
    terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}

fn retry_status() -> agena_domain::ProviderRetryStatus {
    agena_domain::ProviderRetryStatus {
        attempt: 2,
        max_retries: 5,
        next_at_ms: chrono::Utc::now().timestamp_millis() + 10_000,
        message: "Provider unavailable; retrying shortly".into(),
    }
}

#[tokio::test]
async fn provider_retry_status_is_reachable_from_the_full_screen_panel() {
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
    app.transcript
        .execution
        .as_mut()
        .unwrap()
        .execution
        .provider_retry = Some(retry_status());
    app.open_session_work_panel(Tab::Status);
    assert!(matches!(app.current_route, Route::SessionWork(7)));
    let mut terminal = Terminal::new(TestBackend::new(100, 14)).unwrap();
    terminal
        .draw(|frame| app.render_session_work_panel(frame, frame.area()))
        .unwrap();
    let text = buffer_text(&terminal);
    assert!(text.contains("Provider unavailable; retrying shortly"));
    assert!(text.contains("↻ 2"));
}

#[tokio::test]
async fn the_chat_surface_keeps_no_docked_rows_above_the_composer() {
    let mut app = app();
    app.composer.insert_str("MAIN DRAFT");
    app.session_work.insert(
        7,
        SessionWorkState {
            files: Some(files()),
            ..Default::default()
        },
    );
    for (width, height) in [(100, 35), (70, 24), (40, 20), (28, 12)] {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        assert_eq!(
            app.layout.transcript_body.bottom(),
            app.surface_layout.composer_outer.y,
            "{width}x{height}: transcript ends where the composer starts"
        );
        assert_eq!(app.surface_layout.composer_outer.bottom(), height);
        assert!(app.layout.transcript_body.height > 0);
        assert!(buffer_text(&terminal).contains("MAIN DRAFT"));
    }
}

#[tokio::test]
async fn opening_and_leaving_the_panel_preserves_the_composer_draft() {
    let mut app = app();
    app.composer.insert_str("draft to preserve");
    app.session_work.entry(7).or_default().files = Some(files());
    app.open_session_work_panel(Tab::Files);
    assert!(matches!(app.current_route, Route::SessionWork(7)));
    assert_eq!(app.focus, Focus::Transcript);
    let mut terminal = Terminal::new(TestBackend::new(90, 24)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let text = buffer_text(&terminal);
    assert!(text.contains(&app.i18n.text("session-work-title")));
    app.handle_key_event(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(matches!(app.current_route, Route::Main));
    assert_eq!(app.focus, Focus::Composer);
    assert_eq!(app.composer.text(), "draft to preserve");
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
        .select_detail(Some(Detail::File("new.rs".into())));
    app.handle_session_work_loaded(7, 12, 1, Ok(WorkResult::Diff("old content".into(), false)));
    assert!(app.session_work[&7].diff.is_empty());
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
                    chunk: false,
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
async fn visible_file_rows_are_clickable_inside_the_panel() {
    use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    let mut app = app();
    app.composer.insert_str("unsent draft");
    app.session_work.insert(
        7,
        SessionWorkState {
            files: Some(files()),
            ..Default::default()
        },
    );
    app.open_session_work_panel(Tab::Files);
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
        Some(Detail::File("src/main.rs".into()))
    );
    assert!(matches!(app.current_route, Route::SessionWork(7)));
    assert_eq!(app.composer.text(), "unsent draft");
}

#[tokio::test]
async fn composer_chip_counts_files_tasks_and_btw_and_opens_the_matching_surface() {
    let mut app = app();
    app.session_work.entry(7).or_default().files = Some(files());
    app.transcript.execution = Some(
        serde_json::from_value::<crate::SessionExecutionResource>(serde_json::json!({
            "session": {
                "id": 7, "depth": 0, "root_id": 7, "workspace_id": 1,
                "title": "Tasks", "version": 1,
                "relation_kind": "root", "lifecycle_state": "ready",
                "state": {"kind": "running", "data": {"workflow": "tool_pending"}},
                "created_at": "2026-10-01T00:00:00Z", "updated_at": "2026-10-01T00:00:00Z",
                "message_count": 0, "child_session_count": 0
            },
            "parts": [], "execution": {"agent_id": "default"}, "usage": {"current_tokens": 0},
            "background_activities": [{"id": "act_1", "kind": "shell", "title": "Build", "description": "Build the workspace", "status": "running", "created_at_ms": 0, "started_at_ms": 0, "last_seq": 0, "has_more": false, "dropped_lines": 0, "cancellable": true, "dismissible": true}]
        }))
        .unwrap(),
    );
    let segments = app.composer_bottom_left_segments();
    assert_eq!(segments.len(), 2, "files + tasks");
    assert_eq!(segments[0].1, "work-files");
    assert_eq!(segments[1].1, "work-tasks");
    app.handle_session_work_action("work-tasks");
    assert!(matches!(app.current_route, Route::SessionWork(7)));
    assert_eq!(app.session_work[&7].tab, Tab::Tasks);
    app.handle_session_work_action("work-leave");
    assert!(matches!(app.current_route, Route::Main));
}

#[tokio::test]
async fn composer_bottom_border_chip_lists_files_and_tasks_and_opens_the_panel() {
    use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    let mut app = app();
    app.session_work.entry(7).or_default().files = Some(files());
    app.transcript.execution = Some(
        serde_json::from_value::<crate::SessionExecutionResource>(serde_json::json!({
            "session": {
                "id": 7, "depth": 0, "root_id": 7, "workspace_id": 1,
                "title": "Tasks", "version": 1,
                "relation_kind": "root", "lifecycle_state": "ready",
                "state": {"kind": "running", "data": {"workflow": "tool_pending"}},
                "created_at": "2026-10-01T00:00:00Z", "updated_at": "2026-10-01T00:00:00Z",
                "message_count": 0, "child_session_count": 0
            },
            "parts": [], "execution": {"agent_id": "default"}, "usage": {"current_tokens": 0},
            "background_activities": [{"id": "act_1", "kind": "shell", "title": "Build", "description": "Build the workspace", "status": "running", "created_at_ms": 0, "started_at_ms": 0, "last_seq": 0, "has_more": false, "dropped_lines": 0, "cancellable": true, "dismissible": true}]
        }))
        .unwrap(),
    );
    let mut terminal = Terminal::new(TestBackend::new(80, 20)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let row = app.surface_layout.composer_outer.bottom().saturating_sub(1);
    let text = buffer_text(&terminal);
    let line: String = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .skip(usize::from(row) * 80)
        .take(80)
        .map(|cell| cell.symbol())
        .collect();
    assert!(
        text.contains("±1 files"),
        "bottom border carries the file count"
    );
    assert!(line.contains("±1 files") && line.contains("●1 tasks"));

    let mut target = None;
    for column in 0..80 {
        let event = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column,
            row,
            modifiers: KeyModifiers::NONE,
        };
        if app.pointer_targets.action(event) == Some(PointerAction::Named("work-files")) {
            target = Some(event);
        }
    }
    app.handle_mouse_event(target.expect("file chip segment must be clickable"));
    assert!(matches!(app.current_route, Route::SessionWork(7)));
    assert_eq!(app.session_work[&7].tab, Tab::Files);
}

#[tokio::test]
async fn incomplete_recording_is_visible_without_a_git_workspace_and_has_no_staged_switch() {
    let mut app = app();
    app.session_work.insert(
        7,
        SessionWorkState {
            files: Some(FilePage {
                recording_incomplete: true,
                ..Default::default()
            }),
            ..Default::default()
        },
    );
    assert_eq!(app.session_work_file_count(), Some(0));
    app.open_session_work_panel(Tab::Files);
    let mut terminal = Terminal::new(TestBackend::new(120, 8)).unwrap();
    terminal
        .draw(|frame| app.render_session_work_panel(frame, frame.area()))
        .unwrap();
    let text = buffer_text(&terminal);
    assert!(text.contains("Session recorded edits"));
    assert!(text.contains("Recording may be incomplete"));
    app.transcript.session_id = Some(8);
    assert_eq!(
        app.session_work_file_count(),
        None,
        "a different session cannot inherit cached edit rows"
    );
}
