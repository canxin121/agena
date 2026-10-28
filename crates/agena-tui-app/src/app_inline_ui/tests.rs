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
