use super::*;
use crate::{I18n, LaunchOptions, TuiBackend};
use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::{Terminal, backend::TestBackend, style::Modifier};

fn app() -> App {
    let mut app = App::new_with_backend(
        TuiBackend::remote_mock(),
        LaunchOptions::default(),
        I18n::english(),
    );
    app.transcript.session_id = Some(7);
    app
}

fn data() -> InlinePlanData {
    InlinePlanData::from_response("Revision: v1\n# Plan title\n\n**Strong** and `code`\n\n| Key | Value |\n|---|---|\n|one|two|\n\n```rust\nlet answer = 42;\n```".into(), Some(&serde_json::json!({
        "plan": {"title": "Plan title", "phase": "active", "autorun": true,
            "steps": [{"status":"completed"}, {"status":"skipped"}, {"status":"in_progress"}]},
        "current_step": {"title":"Verify 中文"}
    })))
}

fn click(app: &mut App, terminal: &mut Terminal<TestBackend>, action: &'static str) {
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let area = terminal.backend().buffer().area;
    for y in 0..area.height {
        for x in 0..area.width {
            let event = MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: x,
                row: y,
                modifiers: KeyModifiers::NONE,
            };
            if app.pointer_targets.action(event) == Some(PointerAction::Named(action)) {
                app.handle_mouse_event(event);
                return;
            }
        }
    }
    panic!("missing action {action}");
}

#[test]
fn durable_projection_and_refresh_backoff() {
    let data = data();
    assert_eq!(data.summary, "▶ 2/3 ↻");
    assert_eq!(data.current_step, "Verify 中文");
    assert!(!data.markdown.contains("Revision:"));
    assert!(
        InlinePlanData::from_response("No plan".into(), None)
            .summary
            .is_empty()
    );
    let now = Instant::now();
    let mut state = InlinePlanState::default();
    assert!(state.refresh_due(false, now));
    state.refreshed_at = Some(now);
    assert!(!state.refresh_due(false, now + Duration::from_secs(59)));
    assert!(state.refresh_due(false, now + Duration::from_secs(60)));
    state.data = Some(data);
    assert!(!state.refresh_due(false, now + Duration::from_secs(29)));
    assert!(state.refresh_due(false, now + Duration::from_secs(30)));
    assert!(state.refresh_due(true, now + Duration::from_secs(5)));
    state.expanded = true;
    assert!(state.refresh_due(false, now + Duration::from_secs(5)));
}

#[tokio::test]
async fn inline_plan_and_btw_align_with_the_editor_and_preserve_keyboard_input() {
    let mut app = app();
    app.composer.insert_str("MAIN DRAFT");
    app.open_btw("");
    app.inline_plans.insert(
        7,
        InlinePlanState {
            data: Some(data()),
            ..Default::default()
        },
    );
    for (width, height) in [(100, 30), (70, 24), (40, 20)] {
        app.handle_inline_plan_action("plan-toggle");
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        assert!(matches!(app.current_route, Route::Main));
        assert!(app.layout.transcript_body.height > 0);
        assert_eq!(app.plan_area.x, app.surface_layout.composer_outer.x);
        assert_eq!(app.plan_area.width, app.surface_layout.composer_outer.width);
        assert_eq!(app.btw_area.width, app.surface_layout.composer_outer.width);
        let text: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(text.contains("MAIN DRAFT"), "{width}x{height}: {text}");
        assert!(text.contains("2/3"));
        assert!(
            text.contains("Verify ") && text.contains('中') && text.contains('文'),
            "{text}"
        );
        assert_eq!(app.btw_area.height, 1);
        let state = &app.inline_plans[&7];
        assert!(
            state
                .rendered
                .iter()
                .flat_map(|line| &line.spans)
                .any(|span| span.content.contains("Strong")
                    && span.style.add_modifier.contains(Modifier::BOLD))
        );
        assert!(
            state
                .rendered
                .iter()
                .any(|line| line.to_string().contains("Key"))
        );
        let wheel = MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: app.plan_area.x + 2,
            row: app.plan_area.y + 2,
            modifiers: KeyModifiers::NONE,
        };
        app.handle_mouse_event(wheel);
        assert!(app.inline_plans[&7].scroll > 0);
        app.handle_key_event(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(app.focus, Focus::Composer);
        app.handle_paste(" continued".into());
        assert!(app.composer.text().ends_with(" continued"));
        app.composer.set_text("MAIN DRAFT".into());
        click(&mut app, &mut terminal, "btw-toggle");
        assert!(!app.inline_plans[&7].expanded);
        terminal.draw(|frame| app.draw(frame)).unwrap();
        assert_eq!(app.plan_area.height, 1);
        assert!(app.btw_area.height >= 7);
    }
}

#[tokio::test]
async fn requests_are_coalesced_and_late_results_cannot_replace_another_sessions_plan() {
    let mut app = app();
    app.open_plan_viewer();
    let first = app.inline_plans[&7].request_id;
    app.request_plan_display_refresh(7);
    assert_eq!(app.inline_plans[&7].request_id, first);
    app.transcript.session_id = Some(9);
    app.open_plan_viewer();
    let second = app.inline_plans[&9].request_id;
    app.handle_inline_plan_loaded(7, first, Ok(data()));
    assert!(app.inline_plans[&9].data.is_none());
    assert!(app.inline_plans[&7].expanded);
    app.handle_inline_plan_loaded(9, second, Ok(InlinePlanData::default()));
    app.request_plan_display_refresh(9);
    let third = app.inline_plans[&9].request_id;
    app.handle_inline_plan_loaded(9, second, Ok(data()));
    assert!(
        app.inline_plans[&9]
            .data
            .as_ref()
            .unwrap()
            .summary
            .is_empty()
    );
    assert_eq!(app.inline_plans[&9].request_id, third);
    app.transcript.session_id = Some(7);
    app.heal_plan_display_refresh();
    assert!(app.inline_plans[&9].request.is_none());
    app.handle_inline_plan_loaded(9, third, Ok(data()));
    assert!(
        app.inline_plans[&9]
            .data
            .as_ref()
            .unwrap()
            .summary
            .is_empty()
    );
    assert_eq!(
        app.composer_plan_progress_part().as_deref(),
        Some("▶ 2/3 ↻")
    );
}

#[tokio::test]
async fn a_short_terminal_returns_plan_focus_to_the_main_editor() {
    let mut app = app();
    app.composer.set_text("main draft".into());
    app.inline_plans.insert(
        7,
        InlinePlanState {
            expanded: true,
            data: Some(data()),
            ..Default::default()
        },
    );
    app.plan_focus = Some(7);
    let mut terminal = Terminal::new(TestBackend::new(40, 10)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    assert_eq!(app.plan_area.height, 1);
    assert_eq!(app.plan_focus, None);
    assert_eq!(app.focus, Focus::Composer);
    assert_eq!(app.composer.text(), "main draft");
}
