//! Exercise the real host draw path, not a test-only fixed transcript rectangle.
use super::*;
fn draw_host(app: &mut App, width: u16, height: u16) -> Terminal<TestBackend> {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    terminal
}
fn capture(app: &App, terminal: &Terminal<TestBackend>, name: &str) {
    if let Ok(directory) = std::env::var("AGENA_INLINE_UI_RESPONSIVE_ARTIFACTS") {
        std::fs::create_dir_all(&directory).unwrap();
        let buffer = terminal.backend().buffer();
        let mut text = String::new();
        for y in buffer.area.y..buffer.area.bottom() {
            let mut x = buffer.area.x;
            while x < buffer.area.right() {
                let value = buffer[(x, y)].symbol();
                text.push_str(value);
                x += value.width().max(1) as u16;
            }
            text.push('\n');
        }
        std::fs::write(
            std::path::Path::new(&directory).join(format!("{name}.txt")),
            text,
        )
        .unwrap();
        let modal = if let Some(Overlay::InlineUi(dialog)) = &app.overlay {
            Some(dialog.native_layout.borrow().body.width)
        } else {
            None
        };
        std::fs::write(std::path::Path::new(&directory).join(format!("{name}.json")),serde_json::to_string_pretty(&json!({"terminal_width":buffer.area.width,"transcript_width":app.layout.transcript_body.width,"inline_modal_width":modal})).unwrap()).unwrap();
    }
}
#[tokio::test]
async fn responsive_ui_real_main_and_focused_surfaces_use_the_available_terminal_width() {
    let (mut app, _tx, _temp) = application();
    for width in [40, 60, 72, 73, 80, 96, 97, 120, 160, 200, 240, 320] {
        app.handle_terminal_event(Event::Resize(width, 44));
        let terminal = draw_host(&mut app, width, 44);
        assert_eq!(app.layout.transcript_body.width, width - 1);
        let rows = app.transcript.rendered(width - 1).lines.clone();
        assert!(
            rows.iter().any(|row| row.text.contains('┌')
                && row.text.width() >= usize::from(width.saturating_sub(4))),
            "full-width sections unexpectedly shrink at {width}"
        );
        capture(&app, &terminal, &format!("main-{width}"));
        app.open_inline_ui(REFERENCE);
        let terminal = draw_host(&mut app, width, 44);
        let Some(Overlay::InlineUi(dialog)) = &app.overlay else {
            panic!("surface")
        };
        assert_eq!(
            dialog.native_layout.borrow().body.width,
            width - 4,
            "focused UI must not retain 150-column or narrow-dialog caps at {width}"
        );
        capture(&app, &terminal, &format!("focused-{width}"));
        key(&mut app, KeyCode::Esc);
    }
}
#[tokio::test]
async fn responsive_ui_focus_scroll_survives_redraw_and_resizes() {
    let (mut app, _tx, _temp) = application();
    draw_host(&mut app, 120, 24);
    click(
        &mut app,
        "navigation",
        Some(&InlineEvent::Choose("simulation".into())),
    );
    app.open_inline_ui(REFERENCE);
    draw_host(&mut app, 120, 24);
    if let Some(Overlay::InlineUi(dialog)) = &mut app.overlay {
        let target = dialog
            .native_layout
            .borrow()
            .rows
            .iter()
            .flat_map(|row| &row.controls)
            .find_map(|control| match &control.action {
                TranscriptControlAction::InlineUi(target) if target.node == "reset" => {
                    Some(target.clone())
                }
                _ => None,
            })
            .unwrap();
        dialog.native_focus = Some(target);
        dialog.reveal_native.set(true);
    }
    draw_host(&mut app, 120, 24);
    let scroll = match &app.overlay {
        Some(Overlay::InlineUi(dialog)) => dialog.native_layout.borrow().scroll,
        _ => unreachable!(),
    };
    assert!(scroll > 0);
    draw_host(&mut app, 120, 24);
    assert_eq!(
        match &app.overlay {
            Some(Overlay::InlineUi(dialog)) => dialog.native_layout.borrow().scroll,
            _ => unreachable!(),
        },
        scroll,
        "paint-only redraw must not reset auto-revealed scrolling"
    );
    for width in [60, 240, 80, 320, 120] {
        app.handle_terminal_event(Event::Resize(width, 24));
        draw_host(&mut app, width, 24);
        let Some(Overlay::InlineUi(dialog)) = &app.overlay else {
            panic!("surface")
        };
        let layout = dialog.native_layout.borrow();
        assert!(layout.rows.iter().skip(usize::from(layout.scroll)).take(usize::from(layout.body.height)).flat_map(|row|&row.controls).any(|control|matches!(&control.action,TranscriptControlAction::InlineUi(target)if target.node=="reset")),"focus was lost after resize to {width}");
    }
}
#[tokio::test]
async fn responsive_ui_mouse_slider_uses_reflowed_coordinates_after_resize() {
    let (mut app, _tx, _temp) = application();
    draw_host(&mut app, 120, 44);
    click(
        &mut app,
        "navigation",
        Some(&InlineEvent::Choose("simulation".into())),
    );
    for width in [80, 200, 50, 320, 120] {
        app.handle_terminal_event(Event::Resize(width, 44));
        draw_host(&mut app, width, 44);
        let (line, control) = super::control(&mut app, "budget", Some(&InlineEvent::Slider));
        let body = app.layout.transcript_body;
        app.transcript.focus_pointer_control(
            body.width,
            body.height,
            agena_tui_transcript::TranscriptTextPosition {
                line,
                column: control.columns.start,
            },
        );
        draw_host(&mut app, width, 44);
        let (line, control) = super::control(&mut app, "budget", Some(&InlineEvent::Slider));
        let body = app.layout.transcript_body;
        let y = body.y + (line - app.transcript.viewport_top()) as u16;
        let x = body.x + control.columns.end.saturating_sub(1) as u16;
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
        assert_eq!(
            state(&app)["budget"],
            60000.0,
            "wrong resized track at {width}"
        );
        key(&mut app, KeyCode::Home);
        assert_eq!(state(&app)["budget"], 0.0);
    }
}

#[tokio::test]
async fn responsive_ui_modal_resize_discards_old_drag_hits_and_retains_the_editor() {
    let (mut app, _tx, _temp) = application();
    draw_host(&mut app, 120, 44);
    click(
        &mut app,
        "navigation",
        Some(&InlineEvent::Choose("simulation".into())),
    );
    app.open_inline_ui(REFERENCE);
    draw_host(&mut app, 120, 44);
    if let Some(Overlay::InlineUi(dialog)) = &mut app.overlay {
        let target = dialog
            .native_layout
            .borrow()
            .rows
            .iter()
            .flat_map(|row| &row.controls)
            .find_map(|control| match &control.action {
                TranscriptControlAction::InlineUi(target)
                    if target.node == "budget" && target.event == InlineEvent::Slider =>
                {
                    Some(target.clone())
                }
                _ => None,
            })
            .unwrap();
        dialog.native_focus = Some(target);
        dialog.reveal_native.set(true);
    }
    draw_host(&mut app, 120, 44);
    let (x, y) = match &app.overlay {
        Some(Overlay::InlineUi(dialog)) => {
            let layout = dialog.native_layout.borrow();
            let(line,control)=layout.rows.iter().enumerate().find_map(|(line,row)|row.controls.iter().find(|control|matches!(&control.action,TranscriptControlAction::InlineUi(target)if target.node=="budget"&&target.event==InlineEvent::Slider)).map(|control|(line,control))).unwrap();
            (
                layout.body.x + control.columns.start as u16,
                layout.body.y + (line - usize::from(layout.scroll)) as u16,
            )
        }
        _ => unreachable!(),
    };
    app.handle_terminal_event(Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: x,
        row: y,
        modifiers: KeyModifiers::NONE,
    }));
    assert!(matches!(&app.overlay,Some(Overlay::InlineUi(dialog))if dialog.pointer.is_some()));
    let value = state(&app)["budget"].clone();
    app.handle_terminal_event(Event::Resize(60, 32));
    assert!(
        matches!(&app.overlay,Some(Overlay::InlineUi(dialog))if dialog.pointer.is_none()&&!dialog.native_layout.borrow().valid)
    );
    app.handle_terminal_event(Event::Mouse(MouseEvent {
        kind: MouseEventKind::Drag(MouseButton::Left),
        column: 100,
        row: y,
        modifiers: KeyModifiers::NONE,
    }));
    assert_eq!(
        state(&app)["budget"],
        value,
        "an old coordinate must not update the resized control"
    );
    draw_host(&mut app, 60, 32);
    key(&mut app, KeyCode::Enter);
    assert!(matches!(&app.overlay,Some(Overlay::InlineUi(dialog))if dialog.editing.is_some()));
    app.handle_terminal_event(Event::Key(KeyEvent::new(
        KeyCode::Char('u'),
        KeyModifiers::CONTROL,
    )));
    for character in "25000".chars() {
        key(&mut app, KeyCode::Char(character));
    }
    for width in [40, 240, 80] {
        app.handle_terminal_event(Event::Resize(width, 32));
        draw_host(&mut app, width, 32);
        assert!(
            matches!(&app.overlay,Some(Overlay::InlineUi(dialog))if dialog.editing.as_deref()==Some("25000")&&dialog.native_layout.borrow().body.width<=70)
        );
    }
    key(&mut app, KeyCode::Enter);
    assert_eq!(state(&app)["budget"], 25000.0);
    key(&mut app, KeyCode::Esc);
    assert!(app.overlay.is_none());
}
