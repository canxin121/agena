use super::*;
use agena_api::part::PartResource;
use agena_domain::InlinePhase;
use agena_tui_transcript::{TranscriptControlAction, inline_ui::InlineEvent};
use crossterm::event::{Event, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{Terminal, backend::TestBackend};

const REFERENCE: &str = "ui_123_0123456789abcdef0123456789abcdef";
fn fixture() -> InlineSnapshot {
    InlineSnapshot {
        reference: REFERENCE.into(),
        revision: 1,
        phase: InlinePhase::Working,
        updated_at_ms: 1,
        last_call_id: 123,
        document: InlineDocument::from_value(
            serde_json::from_str(include_str!(
                "../../../../examples/inline-ui/live-dashboard.json"
            ))
            .unwrap(),
        )
        .unwrap(),
    }
}
fn application() -> (App, watch::Sender<LiveUpdate>, tempfile::TempDir) {
    let temp = tempfile::tempdir().unwrap();
    let backend = crate::TuiBackend::isolated_test_backend(temp.path().to_path_buf());
    let mut app = App::new_with_backend(
        backend,
        crate::LaunchOptions::default(),
        agena_tui::i18n::I18n::english(),
    );
    app.current_route = crate::Route::Main;
    app.focus = agena_tui::main_focus::Focus::Composer;
    app.layout.transcript_body = Rect::new(2, 2, 116, 40);
    let snapshot = fixture();
    let (tx, receiver) = watch::channel(LiveUpdate {
        snapshot: Some(snapshot.clone()),
        ..Default::default()
    });
    let task = tokio::spawn(std::future::pending::<()>());
    app.transcript.inline_reads.insert(
        REFERENCE.into(),
        InlineUiRead {
            receiver,
            task: task.abort_handle(),
            snapshot: Some(snapshot.clone()),
            seed: None,
            states: Default::default(),
            presentation: Default::default(),
            error: None,
            unavailable: false,
        },
    );
    app.transcript.session_id = Some(1);
    app.transcript.parts = vec![PartResource {
        part_id: 10,
        run_id: Some(10),
        message_id: Some(10),
        message_boundary: true,
        role: "assistant".into(),
        kind: "text".into(),
        state: "completed".into(),
        run_state: Some("completed".into()),
        content: json!({"text":format!("Before\n\n```inline-ui\n{REFERENCE}\n```\n\nAfter")}),
        inline_ui: Some([(REFERENCE.into(), snapshot.document)].into()),
        ..Default::default()
    }];
    (app, tx, temp)
}
fn control(
    app: &mut App,
    node: &str,
    event: Option<&InlineEvent>,
) -> (usize, agena_tui_transcript::RenderedTranscriptControl) {
    let width = app.layout.transcript_body.width;
    app.transcript.rendered(width).lines.iter().enumerate().find_map(|(line,row)|row.controls.iter().find(|control|matches!(&control.action,TranscriptControlAction::InlineUi(target) if target.node==node&&event.is_none_or(|event|&target.event==event))).map(|control|(line,control.clone()))).unwrap_or_else(||panic!("missing interactive node {node}"))
}
fn click(app: &mut App, node: &str, event: Option<&InlineEvent>) {
    let (line, ctrl) = control(app, node, event);
    let width = app.layout.transcript_body.width;
    let height = app.layout.transcript_body.height;
    app.transcript.focus_pointer_control(
        width,
        height,
        agena_tui_transcript::TranscriptTextPosition {
            line,
            column: ctrl.columns.start,
        },
    );
    let x = app.layout.transcript_body.x + ctrl.columns.start as u16;
    let y = app.layout.transcript_body.y + (line - app.transcript.viewport_top()) as u16;
    for kind in [
        MouseEventKind::Down(MouseButton::Left),
        MouseEventKind::Up(MouseButton::Left),
    ] {
        app.handle_terminal_event(Event::Mouse(MouseEvent {
            kind,
            column: x,
            row: y,
            modifiers: KeyModifiers::NONE,
        }));
    }
}
fn key(app: &mut App, code: KeyCode) {
    app.handle_terminal_event(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)));
}
fn state(app: &App) -> UiState {
    let read = &app.transcript.inline_reads[REFERENCE];
    let program = read
        .document()
        .unwrap()
        .blocks
        .iter()
        .find_map(|block| match block {
            InlineBlock::Panel { program, .. } => Some(program),
            _ => None,
        })
        .unwrap();
    program
        .render(read.states.get("workspace").unwrap_or(&UiState::new()))
        .unwrap()
        .state
}
fn computed(app: &App) -> UiState {
    let read = &app.transcript.inline_reads[REFERENCE];
    let program = read
        .document()
        .unwrap()
        .blocks
        .iter()
        .find_map(|block| match block {
            InlineBlock::Panel { program, .. } => Some(program),
            _ => None,
        })
        .unwrap();
    program
        .render(read.states.get("workspace").unwrap_or(&UiState::new()))
        .unwrap()
        .computed
}
fn redraw_modal(app: &mut App) {
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    terminal
        .draw(|frame| {
            if let Some(Overlay::InlineUi(dialog)) = &app.overlay {
                app.render_inline_ui_overlay(frame, frame.area(), dialog);
            }
        })
        .unwrap();
}

#[tokio::test]
async fn native_ui_real_transcript_clicks_toggle_tasks_and_update_overview() {
    let (mut app, _tx, _temp) = application();
    let rows = app.transcript.rendered(116).lines.clone();
    assert!(rows.iter().any(|line| line.text.contains('┌')));
    assert!(rows.iter().any(|line| {
        line.text
            .chars()
            .any(|c| c == '█' || ('\u{2801}'..='\u{28ff}').contains(&c))
    }));
    click(
        &mut app,
        "navigation",
        Some(&InlineEvent::Choose("tasks".into())),
    );
    assert_eq!(state(&app)["page"], "tasks");
    assert!(app.overlay.is_none());
    click(
        &mut app,
        "taskList",
        Some(&InlineEvent::Task("design".into())),
    );
    assert_eq!(state(&app)["done"], json!(["research", "design"]));
    click(&mut app, "tasksReturn", None);
    assert_eq!(state(&app)["page"], "overview");
    assert_eq!(computed(&app)["done"], 2);
    let text = app
        .transcript
        .rendered(116)
        .lines
        .iter()
        .map(|line| line.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!text.contains("```inline-ui"));
    assert!(text.contains("67"));
}

#[tokio::test]
async fn native_ui_real_mouse_drag_and_keyboard_adjust_use_the_same_state() {
    let (mut app, _tx, _temp) = application();
    click(
        &mut app,
        "navigation",
        Some(&InlineEvent::Choose("simulation".into())),
    );
    let (line, ctrl) = control(&mut app, "budget", Some(&InlineEvent::Slider));
    app.transcript.focus_pointer_control(
        116,
        40,
        agena_tui_transcript::TranscriptTextPosition {
            line,
            column: ctrl.columns.start,
        },
    );
    let y = app.layout.transcript_body.y + (line - app.transcript.viewport_top()) as u16;
    let x = app.layout.transcript_body.x + ctrl.columns.start as u16;
    let right = app.layout.transcript_body.x + ctrl.columns.end.saturating_sub(1) as u16;
    for (kind, column) in [
        (MouseEventKind::Down(MouseButton::Left), x),
        (MouseEventKind::Drag(MouseButton::Left), right),
        (MouseEventKind::Up(MouseButton::Left), right),
    ] {
        app.handle_terminal_event(Event::Mouse(MouseEvent {
            kind,
            column,
            row: y,
            modifiers: KeyModifiers::NONE,
        }));
    }
    let high = state(&app)["budget"].as_f64().unwrap();
    assert!(high > 10000.0);
    key(&mut app, KeyCode::Left);
    assert!(state(&app)["budget"].as_f64().unwrap() < high);
    key(&mut app, KeyCode::Home);
    assert_eq!(state(&app)["budget"], 0.0);
    key(&mut app, KeyCode::End);
    assert_eq!(state(&app)["budget"].as_f64().unwrap(), high);
    assert!(app.overlay.is_none());
    key(&mut app, KeyCode::Tab);
    let ctrl = app.transcript.control_at_cursor(116).unwrap();
    assert!(matches!(ctrl.action, TranscriptControlAction::InlineUi(_)));
}

#[tokio::test]
async fn native_ui_select_popup_keyboard_and_pointer_choices_update_the_actual_reply() {
    let (mut app, _tx, _temp) = application();
    click(&mut app, "channel", None);
    assert!(matches!(app.overlay, Some(Overlay::InlineUi(_))));
    redraw_modal(&mut app);
    key(&mut app, KeyCode::Down);
    redraw_modal(&mut app);
    key(&mut app, KeyCode::Enter);
    assert!(app.overlay.is_none());
    assert_eq!(state(&app)["channel"], "organic");
    click(&mut app, "channel", None);
    redraw_modal(&mut app);
    let (body, line, columns) = if let Some(Overlay::InlineUi(dialog)) = &app.overlay {
        let layout = dialog.native_layout.borrow();
        let (line,ctrl)=layout.rows.iter().enumerate().find_map(|(i,row)|row.controls.iter().find(|ctrl|matches!(&ctrl.action,TranscriptControlAction::InlineUi(target)if target.event==InlineEvent::Choose("paid".into()))).map(|ctrl|(i,ctrl))).unwrap();
        (layout.body, line, ctrl.columns.clone())
    } else {
        panic!("missing popup")
    };
    app.handle_terminal_event(Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: body.x + columns.start as u16,
        row: body.y + line as u16,
        modifiers: KeyModifiers::NONE,
    }));
    assert!(app.overlay.is_none());
    assert_eq!(state(&app)["channel"], "paid");
    assert_eq!(computed(&app)["revenue"], 39000.0);
}

#[tokio::test]
async fn native_ui_actual_chat_edit_live_revision_and_copy_remain_consistent() {
    let (mut app, tx, _temp) = application();
    click(
        &mut app,
        "navigation",
        Some(&InlineEvent::Choose("simulation".into())),
    );
    let (line, ctrl) = control(&mut app, "budget", Some(&InlineEvent::Slider));
    app.transcript.focus_pointer_control(
        116,
        40,
        agena_tui_transcript::TranscriptTextPosition {
            line,
            column: ctrl.columns.start,
        },
    );
    app.focus = agena_tui::main_focus::Focus::Transcript;
    key(&mut app, KeyCode::Enter);
    redraw_modal(&mut app);
    key(&mut app, KeyCode::Home);
    for _ in 0..8 {
        key(&mut app, KeyCode::Delete);
    }
    for ch in "25000".chars() {
        key(&mut app, KeyCode::Char(ch));
    }
    key(&mut app, KeyCode::Enter);
    assert!(app.overlay.is_none());
    assert_eq!(state(&app)["budget"], 25000.0);
    let mut snapshot = fixture();
    snapshot.revision = 2;
    snapshot.document.title = "Agent updated the source".into();
    tx.send(LiveUpdate {
        snapshot: Some(snapshot),
        ..Default::default()
    })
    .unwrap();
    assert!(
        app.transcript
            .inline_reads
            .get_mut(REFERENCE)
            .unwrap()
            .sync()
    );
    app.transcript.invalidate_render();
    assert_eq!(state(&app)["budget"], 25000.0);
    let rows = app.transcript.rendered(116).lines.clone();
    assert!(rows.iter().any(|line| line.text.contains("Agent updated")));
    assert!(
        app.transcript.inline_ui_copy_state()[REFERENCE]
            .0
            .to_markdown_with_states(&app.transcript.inline_ui_copy_state()[REFERENCE].1)
            .contains("25000")
    );
    // Retraction invalidates stale pointer controls instead of executing them.
    tx.send(LiveUpdate {
        unavailable: true,
        ..Default::default()
    })
    .unwrap();
    assert!(
        app.transcript
            .inline_reads
            .get_mut(REFERENCE)
            .unwrap()
            .sync()
    );
    app.transcript.invalidate_render();
    assert!(app.transcript.rendered(116).lines.iter().all(|line| {
        line.controls
            .iter()
            .all(|c| !matches!(c.action, TranscriptControlAction::InlineUi(_)))
    }));
}

#[tokio::test]
async fn native_ui_actual_content_resource_preserves_hit_targets_and_stream_bytes() {
    use agena_domain::{
        ContentChunk, ContentCursor, ContentKind, ContentPage, ContentPayload, ContentResource,
        ContentState,
    };
    let (mut app, _tx, _temp) = application();
    let document = fixture().document;
    let source = format!("Before\n\n```inline-ui\n{REFERENCE}\n```\n\nAfter");
    let id = agena_domain::ContentId::new();
    let cursor = ContentCursor {
        epoch: uuid::Uuid::new_v4(),
        sequence: source.len() as u64,
    };
    let mut view = agena_tui_transcript::content::ContentView::default();
    assert!(view.apply(ContentPage {
        inline_ui: Some([(REFERENCE.into(), document)].into()),
        resource: ContentResource {
            resource_id: id,
            owner_session_id: 1,
            part_id: 10,
            kind: ContentKind::Text,
            state: ContentState::Complete,
            cursor,
            committed_cursor: cursor,
            total_bytes: Some(cursor.sequence),
            dropped_bytes: 0,
            dropped_packets: 0,
            committed_dropped_bytes: 0,
            committed_dropped_packets: 0,
            retained_ranges: vec![],
            capture_error: None
        },
        chunks: vec![ContentChunk {
            cursor,
            captured_at_ms: 0,
            payload: ContentPayload::Text {
                text: source.clone()
            }
        }],
        next_cursor: cursor,
        has_more: false,
        gap: false
    }));
    app.transcript.parts[0].content =
        json!({"text":"","resources":[{"resource_id":id,"kind":"text"}]});
    app.transcript.parts[0].inline_ui = None;
    let frame = view.frame.clone().unwrap();
    assert_eq!(frame.source_text.as_ref(), source);
    app.transcript.content_reads.insert(
        id,
        crate::app_session_events::content_reads::ContentRead::native_ui_test_frame(frame),
    );
    app.transcript.invalidate_render();
    let rows = app.transcript.rendered(116).lines.clone();
    assert!(rows.iter().any(|row| !row.controls.is_empty()));
    click(
        &mut app,
        "navigation",
        Some(&InlineEvent::Choose("tasks".into())),
    );
    click(
        &mut app,
        "taskList",
        Some(&InlineEvent::Task("design".into())),
    );
    assert_eq!(state(&app)["done"], json!(["research", "design"]));
    assert_eq!(view.cursor.unwrap().sequence, source.len() as u64);
    assert_eq!(view.frame.as_ref().unwrap().source_text.as_ref(), source);
}

#[tokio::test]
async fn native_ui_actual_heal_preserves_focus_and_cancels_stale_drag_geometry() {
    let (mut app, tx, _temp) = application();
    click(
        &mut app,
        "navigation",
        Some(&InlineEvent::Choose("simulation".into())),
    );
    let (line, ctrl) = control(&mut app, "budget", Some(&InlineEvent::Slider));
    app.transcript.focus_pointer_control(
        116,
        40,
        agena_tui_transcript::TranscriptTextPosition {
            line,
            column: ctrl.columns.start,
        },
    );
    let y = app.layout.transcript_body.y + (line - app.transcript.viewport_top()) as u16;
    app.handle_terminal_event(Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: app.layout.transcript_body.x + ctrl.columns.start as u16,
        row: y,
        modifiers: KeyModifiers::NONE,
    }));
    assert!(app.transcript.inline_pointer.is_some());
    let mut snapshot = fixture();
    snapshot.revision = 2;
    let InlineBlock::Panel { program, .. } = &mut snapshot.document.blocks[0] else {
        panic!("panel")
    };
    let text = program
        .nodes
        .iter_mut()
        .find(|node| node.kind == UiKind::Text)
        .unwrap();
    text.props.insert(
        "text".into(),
        json!("新数据说明占据不同的空间，重新计算焦点位置。".repeat(12)),
    );
    tx.send(LiveUpdate {
        snapshot: Some(snapshot),
        ..Default::default()
    })
    .unwrap();
    app.heal_inline_ui_reads();
    assert!(app.transcript.inline_pointer.is_none());
    assert!(
        matches!(app.transcript.control_at_cursor(116).unwrap().action,TranscriptControlAction::InlineUi(ref target) if target.node=="budget")
    );
    let value = state(&app)["budget"].clone();
    key(&mut app, KeyCode::Right);
    assert!(state(&app)["budget"].as_f64().unwrap() > value.as_f64().unwrap());
}

#[tokio::test]
async fn native_ui_focused_surface_mouse_uses_the_same_live_tree() {
    let (mut app, _tx, _temp) = application();
    app.open_inline_ui(REFERENCE);
    redraw_modal(&mut app);
    let (x, y) = if let Some(Overlay::InlineUi(dialog)) = &app.overlay {
        let layout = dialog.native_layout.borrow();
        let (line,control)=layout.rows.iter().enumerate().find_map(|(line,row)|row.controls.iter().find(|control|matches!(&control.action,TranscriptControlAction::InlineUi(target) if target.node=="navigation" && target.event==InlineEvent::Choose("tasks".into()))).map(|control|(line,control))).unwrap();
        (
            layout.body.x + control.columns.start as u16,
            layout.body.y + (line - usize::from(layout.scroll)) as u16,
        )
    } else {
        panic!("focused surface")
    };
    app.handle_terminal_event(Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: x,
        row: y,
        modifiers: KeyModifiers::NONE,
    }));
    assert_eq!(state(&app)["page"], "tasks");
    redraw_modal(&mut app);
    let (x, y) = if let Some(Overlay::InlineUi(dialog)) = &app.overlay {
        let layout = dialog.native_layout.borrow();
        let(line,control)=layout.rows.iter().enumerate().find_map(|(line,row)|row.controls.iter().find(|control|matches!(&control.action,TranscriptControlAction::InlineUi(target) if target.node=="taskList" && target.event==InlineEvent::Task("design".into()))).map(|control|(line,control))).unwrap();
        (
            layout.body.x + control.columns.start as u16,
            layout.body.y + (line - usize::from(layout.scroll)) as u16,
        )
    } else {
        panic!("focused surface")
    };
    app.handle_terminal_event(Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: x,
        row: y,
        modifiers: KeyModifiers::NONE,
    }));
    assert_eq!(state(&app)["done"], json!(["research", "design"]));
    key(&mut app, KeyCode::Esc);
    assert!(app.overlay.is_none());
    assert_eq!(state(&app)["done"], json!(["research", "design"]));
    click(&mut app, "tasksReturn", None);
    assert_eq!(computed(&app)["done"], 2);
}

#[tokio::test]
async fn native_ui_reported_simulation_works_inside_the_real_transcript() {
    let (mut app, _tx, _temp) = application();
    let document = InlineDocument::from_value(
        serde_json::from_str(include_str!(
            "../../../../examples/inline-ui/terminal-simulation.json"
        ))
        .unwrap(),
    )
    .unwrap();
    app.transcript
        .inline_reads
        .get_mut(REFERENCE)
        .unwrap()
        .snapshot
        .as_mut()
        .unwrap()
        .document = document.clone();
    app.transcript.parts[0].inline_ui = Some([(REFERENCE.into(), document)].into());
    app.transcript.invalidate_render();
    assert_eq!(computed(&app)["base"], 565.0);
    assert_eq!(computed(&app)["projection"], 678.0);
    let (line, control) = control(&mut app, "growth", Some(&InlineEvent::Slider));
    app.transcript.focus_pointer_control(
        116,
        40,
        agena_tui_transcript::TranscriptTextPosition {
            line,
            column: control.columns.start,
        },
    );
    let y = app.layout.transcript_body.y + (line - app.transcript.viewport_top()) as u16;
    let midpoint = app.layout.transcript_body.x
        + (control.columns.start + (control.columns.len() - 1) / 2) as u16;
    for kind in [
        MouseEventKind::Down(MouseButton::Left),
        MouseEventKind::Up(MouseButton::Left),
    ] {
        app.handle_terminal_event(Event::Mouse(MouseEvent {
            kind,
            column: midpoint,
            row: y,
            modifiers: KeyModifiers::NONE,
        }));
    }
    assert_eq!(state(&app)["growth"], 30.0);
    assert_eq!(computed(&app)["projection"], 735.0);
    assert_eq!(computed(&app)["delta"], 170.0);
    for task in ["slider", "tasks", "toggle"] {
        click(&mut app, "tasks", Some(&InlineEvent::Task(task.into())));
    }
    assert_eq!(computed(&app)["progress"], 100.0);
    click(&mut app, "assumptions", None);
    assert_eq!(state(&app)["showFormula"], true);
    let rows = app.transcript.rendered(116).lines.clone();
    assert!(rows.iter().any(|row| row.text.contains("计算假设")));
    let result = rows
        .iter()
        .map(|row| row.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(result.contains("735"));
    assert!(result.contains("100%"));
    assert!(result.contains('✓'));
    if let Ok(directory) = std::env::var("AGENA_INLINE_UI_NATIVE_ARTIFACTS") {
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(
            std::path::Path::new(&directory).join("reported-simulation-actual-chat.txt"),
            result,
        )
        .unwrap();
    }
    click(&mut app, "trend", Some(&InlineEvent::Data));
    let rows = app.transcript.rendered(116).lines.clone();
    let table = rows
        .iter()
        .map(|row| row.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(table.contains("218"));
    click(&mut app, "reset", None);
    assert_eq!(state(&app)["growth"], 20);
    assert_eq!(state(&app)["done"], json!([]));
    assert_eq!(state(&app)["showFormula"], false);
}

#[tokio::test]
async fn native_ui_tab_navigation_keeps_the_selected_repeated_embed() {
    let (mut app, _tx, _temp) = application();
    let source = app.transcript.parts[0].content["text"]
        .as_str()
        .unwrap()
        .to_owned();
    app.transcript.parts[0].content["text"] = json!(format!("{source}\n\n{source}"));
    app.transcript.invalidate_render();
    let(line,column)=app.transcript.rendered(116).lines.iter().enumerate().filter_map(|(line,row)|row.controls.iter().find(|control|matches!(&control.action,TranscriptControlAction::InlineUi(target) if target.node=="navigation"&&target.event==InlineEvent::Choose("overview".into()))).map(|control|(line,control.columns.start))).next_back().unwrap();
    app.transcript.focus_pointer_control(
        116,
        40,
        agena_tui_transcript::TranscriptTextPosition { line, column },
    );
    app.focus = agena_tui::main_focus::Focus::Transcript;

    key(&mut app, KeyCode::Tab);

    assert!(app.transcript.interaction.cursor.as_ref().unwrap().line >= line);
    assert!(
        matches!(app.transcript.control_at_cursor(116).unwrap().action,TranscriptControlAction::InlineUi(ref target) if target.event==InlineEvent::Choose("tasks".into()))
    );
    key(&mut app, KeyCode::Enter);
    assert_eq!(state(&app)["page"], "tasks");
}
