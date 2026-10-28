use super::*;
use agena_domain::InlinePhase;
use ratatui::{Terminal, backend::TestBackend};
use unicode_width::UnicodeWidthStr;

fn fixture() -> InlineSnapshot {
    let document = InlineDocument::from_value(
        serde_json::from_str(include_str!(
            "../../../../examples/inline-ui/live-dashboard.json"
        ))
        .unwrap(),
    )
    .unwrap();
    InlineSnapshot {
        reference: "ui_123_0123456789abcdef0123456789abcdef".into(),
        revision: 1,
        phase: InlinePhase::Working,
        updated_at_ms: 1,
        last_call_id: 123,
        document,
    }
}
fn reader() -> (InlineUiRead, watch::Sender<LiveUpdate>) {
    let snapshot = fixture();
    let (tx, receiver) = watch::channel(LiveUpdate {
        snapshot: Some(snapshot.clone()),
        ..Default::default()
    });
    let task = tokio::spawn(std::future::pending::<()>());
    (
        InlineUiRead {
            receiver,
            task: task.abort_handle(),
            snapshot: Some(snapshot),
            seed: None,
            states: Default::default(),
            error: None,
            unavailable: false,
        },
        tx,
    )
}
fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}
fn press(
    dialog: &mut InlineUiDialog,
    read: &mut InlineUiRead,
    id: &str,
    code: KeyCode,
) -> InlineKeyEffect {
    dialog.focused = id.into();
    handle_dialog_key(dialog, read, key(code), &agena_tui::i18n::I18n::english())
}

#[tokio::test]
async fn stateful_ui_terminal_keys_link_tasks_filters_simulation_and_copy() {
    let (mut read, _tx) = reader();
    let mut dialog = InlineUiDialog::default();
    assert!(press(&mut dialog, &mut read, "navigation", KeyCode::Right).changed);
    assert!(
        press(
            &mut dialog,
            &mut read,
            "taskList:design",
            KeyCode::Char(' ')
        )
        .changed
    );
    assert!(press(&mut dialog, &mut read, "tasksReturn", KeyCode::Enter).changed);
    let (_, _, view) = panel(&read, 0).unwrap();
    assert!(
        (view.root.find("taskProgress").unwrap().props["value"]
            .as_f64()
            .unwrap()
            - 200.0 / 3.0)
            .abs()
            < 1e-8
    );
    press(&mut dialog, &mut read, "channel", KeyCode::Right);
    press(&mut dialog, &mut read, "channel", KeyCode::Right);
    let (_, _, view) = panel(&read, 0).unwrap();
    assert_eq!(
        view.root.find("revenue").unwrap().props["value"],
        json!("39000")
    );
    press(&mut dialog, &mut read, "navigation", KeyCode::Left);
    press(&mut dialog, &mut read, "budget", KeyCode::End);
    press(&mut dialog, &mut read, "formulaToggle", KeyCode::Char(' '));
    let (_, _, view) = panel(&read, 0).unwrap();
    assert_eq!(
        view.root.find("scenarioProfit").unwrap().props["value"],
        json!("-9700")
    );
    assert!(view.root.find("formula").is_some());
    let copied = press(&mut dialog, &mut read, "copy", KeyCode::Enter)
        .copy
        .unwrap();
    assert!(copied.contains("39000") && copied.contains("-9700") && copied.contains("2/3"));
    assert!(
        handle_dialog_key(
            &mut dialog,
            &mut read,
            key(KeyCode::Esc),
            &agena_tui::i18n::I18n::english()
        )
        .close
    );
}
#[tokio::test]
async fn stateful_ui_terminal_revisions_preserve_input_and_no_side_effect_occurs_on_update() {
    let (mut read, tx) = reader();
    let mut dialog = InlineUiDialog::default();
    press(&mut dialog, &mut read, "navigation", KeyCode::Left);
    press(&mut dialog, &mut read, "budget", KeyCode::End);
    let before = read.states.clone();
    let mut updated = fixture();
    updated.revision = 2;
    updated.phase = InlinePhase::Complete;
    let InlineBlock::Panel { program, .. } = &mut updated.document.blocks[0] else {
        panic!("panel")
    };
    program.data.insert("processed".into(), json!(6));
    program.data.insert("workLabel".into(), json!("完成"));
    tx.send_replace(LiveUpdate {
        snapshot: Some(updated),
        ..Default::default()
    });
    assert!(read.sync());
    assert_eq!(read.states, before);
    assert!(read.markdown().unwrap().contains("-9700"));
    // A removed input must not commit its edit into some other control.
    dialog.focused = "removed-control".into();
    dialog.editing = Some("42".into());
    let effect = handle_dialog_key(
        &mut dialog,
        &mut read,
        key(KeyCode::Enter),
        &agena_tui::i18n::I18n::english(),
    );
    assert!(!effect.changed);
    assert!(effect.copy.is_none());
    assert!(dialog.error.is_some());
    tx.send_replace(LiveUpdate {
        unavailable: true,
        error: Some("Not in this session".into()),
        snapshot: None,
    });
    assert!(read.sync());
    assert!(read.document().is_none());
    assert!(read.states.is_empty());
}
#[tokio::test]
async fn stateful_ui_terminal_controls_remain_visible_at_narrow_and_wide_sizes() {
    let (read, _tx) = reader();
    let dialog = InlineUiDialog {
        focused: "navigation".into(),
        ..Default::default()
    };
    let i18n = agena_tui::i18n::I18n::english();
    for (width, height) in [(40, 24), (80, 30), (120, 40)] {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| render_dialog(frame, frame.area(), &dialog, Some(&read), &i18n))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let mut text = String::new();
        for y in 0..height {
            let mut x = 0;
            while x < width {
                let symbol = buffer[(x, y)].symbol();
                text.push_str(symbol);
                x = x.saturating_add(symbol.width().max(1) as u16);
            }
            text.push('\n');
        }
        assert!(text.contains("Interactive reply"), "{width}: {text}");
        assert!(text.contains("Controls"), "{width}: {text}");
        assert!(text.contains("Tab"), "{width}: {text}");
        assert!(text.contains("经营总览"), "{width}: {text}");
    }
}
#[tokio::test]
async fn stateful_ui_terminal_current_state_is_projected_into_the_reply_body() {
    let (mut read, _tx) = reader();
    let mut dialog = InlineUiDialog::default();
    press(&mut dialog, &mut read, "channel", KeyCode::Right);
    press(&mut dialog, &mut read, "channel", KeyCode::Right);
    let reference = read.snapshot.as_ref().unwrap().reference.clone();
    let mut transcript = TranscriptState::new(agena_tui::i18n::I18n::english(), Default::default());
    transcript.inline_reads.insert(reference.clone(), read);
    let source = format!("Before\n\n```inline-ui\n{reference}\n```\n\nAfter");
    let part = agena_api::part::PartResource {
        kind: "text".into(),
        role: "assistant".into(),
        state: "completed".into(),
        content: json!({"text":source}),
        ..Default::default()
    };
    let projected = transcript.inline_ui_part(&part);
    let text = projected.content["text"].as_str().unwrap();
    assert!(text.contains("39000"));
    assert!(text.starts_with("Before"));
    assert!(text.ends_with("After"));
    assert!(
        part.content["text"]
            .as_str()
            .unwrap()
            .contains("```inline-ui")
    );
}

fn terminal_text(terminal: &Terminal<TestBackend>) -> String {
    let buffer = terminal.backend().buffer();
    let mut lines = Vec::new();
    for y in buffer.area.y..buffer.area.bottom() {
        let mut text = String::new();
        let mut x = buffer.area.x;
        while x < buffer.area.right() {
            let symbol = buffer[(x, y)].symbol();
            text.push_str(symbol);
            x = x.saturating_add(symbol.width().max(1) as u16);
        }
        lines.push(text.trim_end().to_owned());
    }
    lines.join("\n")
}

#[tokio::test]
async fn stateful_ui_compact_preview_preserves_results_without_duplicating_control_labels() {
    let (read, _tx) = reader();
    let (_, _, view) = panel(&read, 0).unwrap();
    let lines = compact_preview(&view, 64);
    let text = lines
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("118500"));
    assert!(
        !text.contains("复制当前摘要"),
        "buttons are already shown in the control list"
    );
    assert!(!text.starts_with('\n') && !text.ends_with('\n'));
    assert!(!text.contains("\n\n\n"), "avoid doubled spacer rows");
    assert!(
        view.to_markdown().contains("复制当前摘要"),
        "copy/export must keep the full semantic contents"
    );
}

#[tokio::test]
async fn stateful_ui_compact_surface_is_content_sized_and_uses_host_theme() {
    let (mut read, _tx) = reader();
    read.snapshot.as_mut().unwrap().document = InlineDocument::from_value(json!({
        "version":1,"title":"Small result","blocks":[{"type":"panel","id":"small","program":{
            "root":"main","nodes":[{"id":"main","kind":"text","props":{"text":"Result: 42"}}]
        }}]
    }))
    .unwrap();
    let dialog = InlineUiDialog::default();
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    terminal
        .draw(|frame| {
            render_dialog(
                frame,
                frame.area(),
                &dialog,
                Some(&read),
                &agena_tui::i18n::I18n::english(),
            )
        })
        .unwrap();
    let text = terminal_text(&terminal);
    let occupied = text.lines().filter(|line| !line.trim().is_empty()).count();
    assert!(
        occupied <= 8,
        "a one-line result must not reserve a screen of empty rows: {text}"
    );
    assert!(text.contains("Result: 42"));
    assert!(
        !text.contains("Controls"),
        "read-only results have no empty control pane"
    );
    let buffer = terminal.backend().buffer();
    let corner = buffer
        .content
        .iter()
        .find(|cell| cell.symbol() == "╔")
        .expect("native double modal border");
    assert_eq!(
        corner.fg,
        agena_tui_components::theme::modal_border_style()
            .fg
            .unwrap()
    );
    assert_eq!(
        corner.bg,
        agena_tui_components::theme::modal_surface_style()
            .bg
            .unwrap()
    );
}

#[tokio::test]
async fn stateful_ui_only_controls_and_small_terminals_do_not_disappear() {
    let (mut read, _tx) = reader();
    read.snapshot.as_mut().unwrap().document = InlineDocument::from_value(json!({
        "version":1,"blocks":[{"type":"panel","id":"control","program":{
            "root":"field","state":{"value":{"kind":"boolean","initial":false}},
            "nodes":[{"id":"field","kind":"checkbox","bind":"value","props":{"label":"Enable"}}]
        }}]
    }))
    .unwrap();
    let dialog = InlineUiDialog {
        focused: "field".into(),
        ..Default::default()
    };
    for (width, height) in [(24, 10), (40, 18), (80, 24), (120, 32)] {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| {
                render_dialog(
                    frame,
                    frame.area(),
                    &dialog,
                    Some(&read),
                    &agena_tui::i18n::I18n::english(),
                )
            })
            .unwrap();
        let text = terminal_text(&terminal);
        assert!(text.contains("Enable"), "{width}x{height}: {text}");
        assert!(text.contains("☐"), "the value must remain visible: {text}");
        assert!(
            text.contains("Esc"),
            "a short window still needs exit help: {text}"
        );
    }
    // Extreme dimensions must not underflow/panic even when no content fits.
    for (width, height) in [(1, 1), (8, 3), (16, 6)] {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| {
                render_dialog(
                    frame,
                    frame.area(),
                    &dialog,
                    Some(&read),
                    &agena_tui::i18n::I18n::english(),
                )
            })
            .unwrap();
    }
}

#[tokio::test]
async fn stateful_ui_live_content_shrink_clamps_scroll_instead_of_showing_blank_space() {
    let (mut read, _tx) = reader();
    let mut dialog = InlineUiDialog {
        scroll: u16::MAX,
        ..Default::default()
    };
    let i18n = agena_tui::i18n::I18n::english();
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal
        .draw(|frame| render_dialog(frame, frame.area(), &dialog, Some(&read), &i18n))
        .unwrap();
    let maximum = dialog.scroll_limit.get();
    assert!(maximum > 0);
    handle_dialog_key(&mut dialog, &mut read, key(KeyCode::PageUp), &i18n);
    assert_eq!(dialog.scroll, maximum.saturating_sub(8));
    for _ in 0..20 {
        handle_dialog_key(&mut dialog, &mut read, key(KeyCode::PageDown), &i18n);
    }
    assert_eq!(dialog.scroll, maximum);
    read.snapshot.as_mut().unwrap().document = InlineDocument::from_value(json!({
        "version":1,"blocks":[{"type":"panel","id":"short","program":{
            "root":"done","nodes":[{"id":"done","kind":"text","props":{"text":"Finished"}}]
        }}]
    }))
    .unwrap();
    terminal
        .draw(|frame| render_dialog(frame, frame.area(), &dialog, Some(&read), &i18n))
        .unwrap();
    assert_eq!(dialog.scroll_limit.get(), 0);
    assert!(terminal_text(&terminal).contains("Finished"));
}

#[tokio::test]
async fn stateful_ui_compact_layout_captures_are_bounded_and_controls_are_aligned() {
    let (read, _tx) = reader();
    let dialog = InlineUiDialog {
        focused: "navigation".into(),
        ..Default::default()
    };
    for (width, height) in [(40, 18), (80, 24), (120, 32)] {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| {
                render_dialog(
                    frame,
                    frame.area(),
                    &dialog,
                    Some(&read),
                    &agena_tui::i18n::I18n::english(),
                )
            })
            .unwrap();
        let text = terminal_text(&terminal);
        assert!(text.contains("Controls"));
        assert!(text.contains("经营总览"));
        for line in text.lines() {
            assert!(line.width() <= usize::from(width));
        }
        if let Ok(path) = std::env::var("AGENA_INLINE_UI_LAYOUT_ARTIFACTS") {
            std::fs::create_dir_all(&path).unwrap();
            std::fs::write(
                std::path::Path::new(&path).join(format!("terminal-{width}x{height}.txt")),
                text,
            )
            .unwrap();
        }
    }
    let control = Control {
        id: "long".into(),
        node: "long".into(),
        item: None,
        label: "这是很长的中文控件名称".repeat(5),
        value: "超长当前值🌱".repeat(5),
        kind: UiKind::Select,
        disabled: false,
    };
    let mut terminal = Terminal::new(TestBackend::new(30, 1)).unwrap();
    terminal
        .draw(|frame| {
            frame.render_widget(
                List::new(vec![control_row(&control, true, 30)]),
                frame.area(),
            )
        })
        .unwrap();
    let text = terminal_text(&terminal);
    assert!(text.contains("这是"));
    assert!(text.contains("超长"));
    assert!(text.width() <= 30);
}
