use super::*;
use agena_domain::InlinePhase;
use agena_tui_transcript::{TranscriptControlAction, inline_ui::InlineEvent};
use ratatui::{Terminal, backend::TestBackend};

fn fixture() -> InlineSnapshot {
    InlineSnapshot {
        reference: "ui_123_0123456789abcdef0123456789abcdef".into(),
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
            presentation: Default::default(),
            error: None,
            unavailable: false,
        },
        tx,
    )
}
fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}
fn draw(
    read: &InlineUiRead,
    dialog: &InlineUiDialog,
    width: u16,
    height: u16,
) -> Terminal<TestBackend> {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| {
            render_dialog(
                frame,
                frame.area(),
                dialog,
                Some(read),
                &agena_tui::i18n::I18n::english(),
            )
        })
        .unwrap();
    terminal
}
fn terminal_text(terminal: &Terminal<TestBackend>) -> String {
    let buffer = terminal.backend().buffer();
    let mut lines = Vec::new();
    for y in buffer.area.y..buffer.area.bottom() {
        let mut line = String::new();
        let mut x = buffer.area.x;
        while x < buffer.area.right() {
            let symbol = buffer[(x, y)].symbol();
            line.push_str(symbol);
            x = x.saturating_add(symbol.width().max(1) as u16);
        }
        lines.push(line.trim_end().to_owned());
    }
    lines.join("\n")
}
fn focus(dialog: &mut InlineUiDialog, read: &InlineUiRead, node: &str, event: Option<InlineEvent>) {
    draw(read, dialog, 120, 36);
    let selected = dialog
        .native_layout
        .borrow()
        .rows
        .iter()
        .flat_map(|row| &row.controls)
        .find_map(|ctrl| match &ctrl.action {
            TranscriptControlAction::InlineUi(target)
                if target.node == node
                    && event.as_ref().is_none_or(|event| event == &target.event) =>
            {
                Some(target.clone())
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("missing node {node}"));
    dialog.native_focus = Some(selected);
    dialog.focused = node.into();
    dialog.reveal_native.set(true);
}
fn press(
    dialog: &mut InlineUiDialog,
    read: &mut InlineUiRead,
    node: &str,
    event: Option<InlineEvent>,
    code: KeyCode,
) -> InlineKeyEffect {
    focus(dialog, read, node, event);
    native::handle_modal_key(dialog, read, key(code), &agena_tui::i18n::I18n::english())
}
fn small(read: &mut InlineUiRead, nodes: Value, state: Value, root: &str) {
    read.snapshot.as_mut().unwrap().document=InlineDocument::from_value(json!({"version":1,"title":"Small result","blocks":[{"type":"panel","id":"workspace","program":{"root":root,"state":state,"nodes":nodes}}]})).unwrap();
}

#[tokio::test]
async fn native_ui_focused_surface_renders_real_components_and_keyboard_focus() {
    let (read, _tx) = reader();
    let dialog = InlineUiDialog {
        reference: fixture().reference,
        ..Default::default()
    };
    for (width, height) in [(40, 18), (80, 24), (120, 36)] {
        let terminal = draw(&read, &dialog, width, height);
        let text = terminal_text(&terminal);
        assert!(text.contains("Interactive reply"));
        assert!(text.contains("经营总览"));
        assert!(text.contains("Tab"));
        assert!(!text.contains("Controls"));
        for line in text.lines() {
            assert!(line.width() <= usize::from(width));
        }
        if let Ok(path) = std::env::var("AGENA_INLINE_UI_NATIVE_ARTIFACTS") {
            std::fs::create_dir_all(&path).unwrap();
            std::fs::write(
                std::path::Path::new(&path).join(format!("focused-{width}x{height}.txt")),
                text,
            )
            .unwrap();
        }
    }
}
#[tokio::test]
async fn native_ui_small_surface_and_controls_have_no_empty_side_pane() {
    let (mut read, _tx) = reader();
    small(
        &mut read,
        json!([{"id":"text","kind":"text","props":{"text":"Result: 42"}}]),
        json!({}),
        "text",
    );
    let dialog = InlineUiDialog {
        reference: fixture().reference,
        ..Default::default()
    };
    let terminal = draw(&read, &dialog, 120, 40);
    let text = terminal_text(&terminal);
    assert!(text.contains("Result: 42"));
    assert!(text.lines().filter(|line| !line.trim().is_empty()).count() <= 9);
    assert!(!text.contains("Controls"));
    let buffer = terminal.backend().buffer();
    let corner = buffer
        .content
        .iter()
        .find(|cell| cell.symbol() == "╔")
        .unwrap();
    assert_eq!(
        corner.fg,
        agena_tui_components::theme::modal_border_style()
            .fg
            .unwrap()
    );
    small(
        &mut read,
        json!([{"id":"enabled","kind":"checkbox","bind":"enabled","props":{"label":"Enable"}}]),
        json!({"enabled":{"kind":"boolean","initial":false}}),
        "enabled",
    );
    for (width, height) in [(24, 10), (40, 18), (80, 24)] {
        let text = terminal_text(&draw(&read, &dialog, width, height));
        assert!(text.contains("Enable"), "{width}x{height}: {text}");
        assert!(text.contains("[ ]"));
        assert!(text.contains("Esc"));
    }
    for (width, height) in [(1, 1), (8, 3), (16, 6)] {
        draw(&read, &dialog, width, height);
    }
}
#[tokio::test]
async fn native_ui_revision_updates_keep_local_input_but_never_execute_buttons() {
    let (mut read, tx) = reader();
    let mut dialog = InlineUiDialog {
        reference: fixture().reference,
        ..Default::default()
    };
    assert!(
        press(
            &mut dialog,
            &mut read,
            "navigation",
            Some(InlineEvent::Choose("simulation".into())),
            KeyCode::Enter
        )
        .changed
    );
    assert!(press(&mut dialog, &mut read, "budget", None, KeyCode::Right).changed);
    let before = read.states.clone();
    let mut snapshot = fixture();
    snapshot.revision = 2;
    snapshot.document.title = "New data".into();
    tx.send(LiveUpdate {
        snapshot: Some(snapshot),
        ..Default::default()
    })
    .unwrap();
    assert!(read.sync());
    assert_eq!(before, read.states);
    let text = read.markdown().unwrap();
    assert!(text.contains("13000"));
    assert!(text.contains("New data"));
    let effect = press(&mut dialog, &mut read, "copy", None, KeyCode::Enter);
    assert!(effect.copy.is_some());
    assert!(!effect.changed);
}
#[tokio::test]
async fn native_ui_arrows_on_buttons_do_not_activate_actions() {
    let (mut read, _tx) = reader();
    let mut dialog = InlineUiDialog {
        reference: fixture().reference,
        ..Default::default()
    };
    let effect = press(&mut dialog, &mut read, "viewTasks", None, KeyCode::Right);
    assert!(!effect.changed);
    assert!(effect.copy.is_none());
    let effect = press(&mut dialog, &mut read, "viewTasks", None, KeyCode::Enter);
    assert!(effect.changed);
    assert_eq!(read.states["workspace"]["page"], "tasks");
}
#[tokio::test]
async fn native_ui_disclosures_and_chart_legend_are_real_local_actions() {
    let (mut read, _tx) = reader();
    let mut dialog = InlineUiDialog {
        reference: fixture().reference,
        ..Default::default()
    };
    let chart = read
        .document()
        .unwrap()
        .blocks
        .iter()
        .find_map(|block| match block {
            InlineBlock::Panel { program, .. } => program
                .nodes
                .iter()
                .find(|node| node.kind == UiKind::Chart)
                .map(|node| node.id.clone()),
            _ => None,
        })
        .unwrap();
    assert!(
        press(
            &mut dialog,
            &mut read,
            &chart,
            Some(InlineEvent::Data),
            KeyCode::Enter
        )
        .changed
    );
    assert!(read.presentation["workspace"].data.contains(&chart));
    assert!(
        press(
            &mut dialog,
            &mut read,
            &chart,
            Some(InlineEvent::Series(0)),
            KeyCode::Char(' ')
        )
        .changed
    );
    assert!(read.presentation["workspace"].hidden_series[&chart].contains(&0));
    let original = read.markdown().unwrap();
    assert!(original.contains("24000"), "copy retains hidden plot data");
}
#[tokio::test]
async fn native_ui_utf8_editor_paste_delete_and_validation_are_atomic() {
    let (mut read, _tx) = reader();
    small(
        &mut read,
        json!([{"id":"field","kind":"input","bind":"query","props":{"label":"Query"}}]),
        json!({"query":{"kind":"text","initial":"中文🌱","max_length":8}}),
        "field",
    );
    let mut dialog = InlineUiDialog {
        reference: fixture().reference,
        ..Default::default()
    };
    press(&mut dialog, &mut read, "field", None, KeyCode::Enter);
    native::handle_modal_key(
        &mut dialog,
        &mut read,
        key(KeyCode::Home),
        &agena_tui::i18n::I18n::english(),
    );
    native::handle_modal_key(
        &mut dialog,
        &mut read,
        key(KeyCode::Right),
        &agena_tui::i18n::I18n::english(),
    );
    native::paste_editor(&mut dialog, "插入\u{1b}");
    assert_eq!(dialog.editing.as_deref(), Some("中插入文🌱"));
    native::handle_modal_key(
        &mut dialog,
        &mut read,
        key(KeyCode::Delete),
        &agena_tui::i18n::I18n::english(),
    );
    native::handle_modal_key(
        &mut dialog,
        &mut read,
        key(KeyCode::End),
        &agena_tui::i18n::I18n::english(),
    );
    native::handle_modal_key(
        &mut dialog,
        &mut read,
        key(KeyCode::Backspace),
        &agena_tui::i18n::I18n::english(),
    );
    assert_eq!(dialog.editing.as_deref(), Some("中插入"));
    let effect = native::handle_modal_key(
        &mut dialog,
        &mut read,
        key(KeyCode::Enter),
        &agena_tui::i18n::I18n::english(),
    );
    assert!(effect.changed);
    assert_eq!(read.states["workspace"]["query"], "中插入");
    press(&mut dialog, &mut read, "field", None, KeyCode::Enter);
    native::paste_editor(&mut dialog, "超长不可提交内容");
    let before = read.states.clone();
    let effect = native::handle_modal_key(
        &mut dialog,
        &mut read,
        key(KeyCode::Enter),
        &agena_tui::i18n::I18n::english(),
    );
    assert!(!effect.changed);
    assert!(dialog.error.is_some());
    assert_eq!(before, read.states);
}
#[tokio::test]
async fn native_ui_live_shrink_clamps_focused_surface_scroll() {
    let (mut read, _tx) = reader();
    let mut dialog = InlineUiDialog {
        reference: fixture().reference,
        scroll: u16::MAX,
        ..Default::default()
    };
    draw(&read, &dialog, 80, 24);
    let max = dialog.scroll_limit.get();
    assert!(max > 0);
    native::handle_modal_key(
        &mut dialog,
        &mut read,
        key(KeyCode::PageUp),
        &agena_tui::i18n::I18n::english(),
    );
    assert_eq!(dialog.scroll, max.saturating_sub(8));
    small(
        &mut read,
        json!([{"id":"text","kind":"text","props":{"text":"Finished"}}]),
        json!({}),
        "text",
    );
    let text = terminal_text(&draw(&read, &dialog, 80, 24));
    assert!(text.contains("Finished"));
    assert_eq!(dialog.scroll_limit.get(), 0);
}
#[tokio::test]
async fn native_ui_disabled_controls_and_removed_fields_cannot_execute_stale_events() {
    let (mut read, _tx) = reader();
    small(
        &mut read,
        json!([{"id":"field","kind":"input","bind":"query","props":{"label":"Query"}}]),
        json!({"query":{"kind":"text","initial":"safe"}}),
        "field",
    );
    let mut dialog = InlineUiDialog {
        reference: fixture().reference,
        ..Default::default()
    };
    press(&mut dialog, &mut read, "field", None, KeyCode::Enter);
    small(
        &mut read,
        json!([{"id":"text","kind":"text","props":{"text":"Removed"}}]),
        json!({}),
        "text",
    );
    let before = read.states.clone();
    let outcome = native::handle_modal_key(
        &mut dialog,
        &mut read,
        key(KeyCode::Enter),
        &agena_tui::i18n::I18n::english(),
    );
    assert!(!outcome.changed);
    assert!(dialog.error.is_some());
    assert_eq!(before, read.states);
}

#[tokio::test]
async fn native_ui_number_controls_respect_their_declared_range_not_a_default_slider_range() {
    let (mut read, _tx) = reader();
    small(
        &mut read,
        json!([{"id":"field","kind":"number","bind":"value","props":{"label":"Value"}}]),
        json!({"value":{"kind":"number","initial":250,"min":-1000,"max":1000}}),
        "field",
    );
    let mut dialog = InlineUiDialog {
        reference: fixture().reference,
        ..Default::default()
    };
    let effect = press(&mut dialog, &mut read, "field", None, KeyCode::Right);
    assert!(effect.changed);
    assert_eq!(read.states["workspace"]["value"], 251.0);
    let effect = press(&mut dialog, &mut read, "field", None, KeyCode::Home);
    assert!(effect.changed);
    assert_eq!(read.states["workspace"]["value"], -1000.0);
}

#[tokio::test]
async fn native_ui_long_select_options_keep_the_current_choice_visible() {
    let (mut read, _tx) = reader();
    let options = (0..30)
        .map(|index| json!({"value":format!("v{index}"),"label":format!("选项 {index}")}))
        .collect::<Vec<_>>();
    small(
        &mut read,
        json!([{"id":"field","kind":"select","bind":"choice","props":{"label":"Choose"}}]),
        json!({"choice":{"kind":"choice","initial":"v29","options":options}}),
        "field",
    );
    let mut dialog = InlineUiDialog {
        reference: fixture().reference,
        ..Default::default()
    };
    press(&mut dialog, &mut read, "field", None, KeyCode::Enter);
    let text = terminal_text(&draw(&read, &dialog, 40, 12));
    assert!(text.contains("● 选项 29"), "{text}");
    native::handle_modal_key(
        &mut dialog,
        &mut read,
        key(KeyCode::Home),
        &agena_tui::i18n::I18n::english(),
    );
    let text = terminal_text(&draw(&read, &dialog, 40, 12));
    assert!(text.contains("● 选项 0"), "{text}");
    native::handle_modal_key(
        &mut dialog,
        &mut read,
        key(KeyCode::Enter),
        &agena_tui::i18n::I18n::english(),
    );
    assert_eq!(read.states["workspace"]["choice"], "v0");
}

#[tokio::test]
async fn native_ui_data_revision_error_has_a_real_reset_control_and_recovers() {
    let (mut read, tx) = reader();
    let mut document=InlineDocument::from_value(json!({"version":1,"blocks":[{"type":"panel","id":"workspace","program":{
        "root":"root","data":{"threshold":2},"state":{"value":{"kind":"number","initial":0,"min":0,"max":4}},
        "computed":{"result":{"$op":"div","args":[10,{"$op":"sub","args":[{"$get":"data.threshold"},{"$get":"state.value"}]}]}},
        "nodes":[{"id":"root","kind":"stack","children":["field","result"]},{"id":"field","kind":"number","bind":"value","props":{"label":"Value"}},{"id":"result","kind":"metric","props":{"label":"Result","value":{"$get":"computed.result"}}}]
    }}]})).unwrap();
    read.snapshot.as_mut().unwrap().document = document.clone();
    let mut dialog = InlineUiDialog {
        reference: fixture().reference,
        ..Default::default()
    };
    assert!(press(&mut dialog, &mut read, "field", None, KeyCode::Right).changed);
    let InlineBlock::Panel { program, .. } = &mut document.blocks[0] else {
        panic!("panel")
    };
    program.data.insert("threshold".into(), json!(1));
    let mut revision = fixture();
    revision.document = document;
    revision.revision = 2;
    tx.send(LiveUpdate {
        snapshot: Some(revision),
        ..Default::default()
    })
    .unwrap();
    assert!(read.sync());
    assert!(read.error.is_some());
    let effect = press(
        &mut dialog,
        &mut read,
        "",
        Some(InlineEvent::Reset),
        KeyCode::Enter,
    );
    assert!(effect.changed);
    assert!(read.error.is_none());
    let text = terminal_text(&draw(&read, &dialog, 80, 24));
    assert!(text.contains("10"));
}
