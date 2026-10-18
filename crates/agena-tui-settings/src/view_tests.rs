use super::*;
use agena_tui_components::{
    SurfaceMode,
    pointer::{self, PointerAction},
};
use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::{Terminal, backend::TestBackend, buffer::Buffer, text::Text};

fn fixture(i18n: &I18n) -> SettingsStudioPresentation<usize> {
    let sections = [
        (
            SettingsStudioSectionId::ModelsProviders,
            "overlay-settings-section-providers-label",
            "overlay-settings-section-providers-description",
        ),
        (
            SettingsStudioSectionId::Permissions,
            "overlay-settings-section-permissions-label",
            "overlay-settings-section-permissions-description",
        ),
        (
            SettingsStudioSectionId::PluginsTools,
            "overlay-settings-section-plugins-label",
            "overlay-settings-section-plugins-description",
        ),
        (
            SettingsStudioSectionId::RuntimeSession,
            "overlay-settings-section-runtime-session-label",
            "overlay-settings-section-runtime-session-description",
        ),
        (
            SettingsStudioSectionId::Interface,
            "overlay-settings-section-ui-label",
            "overlay-settings-section-ui-description",
        ),
        (
            SettingsStudioSectionId::Diagnostics,
            "terminal-diagnostics-title",
            "overlay-settings-section-tracing-summary",
        ),
    ]
    .into_iter()
    .map(|(id, label, description)| SettingsStudioSection {
        id,
        label: i18n.text(label),
        summary: String::new(),
        description: i18n.text(description),
        items: [
            ("ui-locale", "English", "ui.locale"),
            ("tui-color-scheme", "auto", "ui.tui.color_scheme"),
            ("tui-graphics", "auto", "ui.tui.graphics"),
            ("tui-theme", "default", "ui.tui.theme"),
            (
                "activity-default-expanded",
                "false",
                "ui.transcript.activity.default_expanded",
            ),
        ]
        .into_iter()
        .enumerate()
        .map(|(index, (key, value, path))| {
            let mut item = SettingsStudioItem::new(
                i18n.text(&format!("settings-field-{key}-label")),
                value,
                i18n.text(&format!("settings-field-{key}-description")),
                index,
            );
            item.path = Some(path.to_owned());
            item
        })
        .collect(),
    })
    .collect();
    SettingsStudioPresentation::new(sections, 4, 1, SettingsStudioFocus::Items)
}

fn render(
    state: &SettingsStudioPresentation<usize>,
    i18n: &I18n,
    width: u16,
    height: u16,
) -> (Buffer, pointer::PointerMap) {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    let mut targets = pointer::PointerMap::default();
    terminal
        .draw(|frame| {
            targets = pointer::capture(|| {
                render_settings(
                    frame,
                    frame.area(),
                    SurfaceMode::Route,
                    state,
                    i18n,
                    &SettingsViewSpec {
                        title: &i18n.text("overlay-settings-title"),
                        detail_title: "Terminal Color Scheme",
                        detail: Text::from(
                            (0..80)
                                .map(|index| {
                                    format!(
                                        "Configuration source {index}: /workspace/设置/agena.json\n"
                                    )
                                })
                                .collect::<String>(),
                        ),
                    },
                )
            });
        })
        .unwrap();
    (terminal.backend().buffer().clone(), targets)
}

fn plain(buffer: &Buffer) -> String {
    buffer
        .content()
        .chunks(buffer.area.width.max(1) as usize)
        .map(|cells| {
            let mut skip = 0;
            let mut text = String::new();
            for cell in cells {
                if skip > 0 {
                    skip -= 1;
                    continue;
                }
                text.push_str(cell.symbol());
                skip = unicode_width::UnicodeWidthStr::width(cell.symbol()).saturating_sub(1);
            }
            text
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn key(state: &mut SettingsStudioPresentation<usize>, code: KeyCode) -> SettingsStudioEffect {
    handle_key(state, KeyEvent::new(code, KeyModifiers::NONE))
}

fn click(x: u16, y: u16) -> MouseEvent {
    MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: x,
        row: y,
        modifiers: KeyModifiers::NONE,
    }
}

#[test]
fn settings_search_filters_paths_and_keeps_original_action_indices() {
    let mut state = fixture(&I18n::english());
    key(&mut state, KeyCode::Char('/'));
    state.paste("appearance ui.tui.graphics");
    assert_eq!(state.visible_section_indices(), vec![4]);
    assert_eq!(state.visible_item_indices(4), vec![2]);
    assert_eq!(state.selected_item().unwrap().action, 2);
    assert_eq!(
        key(&mut state, KeyCode::Enter),
        SettingsStudioEffect::KeepOpen
    );
    assert!(!state.is_searching());
    assert!(state.select_visible(1, 0));
    assert_eq!(state.selected_item_index(), 2);
    assert_eq!(
        key(&mut state, KeyCode::Enter),
        SettingsStudioEffect::Activate
    );
    assert_eq!(
        key(&mut state, KeyCode::Esc),
        SettingsStudioEffect::KeepOpen
    );
    assert!(state.query().is_empty());
    assert_eq!(key(&mut state, KeyCode::Esc), SettingsStudioEffect::Close);
}

#[test]
fn settings_search_does_not_match_every_row_from_a_category_description() {
    let mut state = fixture(&I18n::english());
    state.set_query("appearance graphics");
    assert_eq!(state.visible_item_indices(4), vec![2]);
    assert_eq!(state.selected_item_index(), 2);
}

#[test]
fn settings_no_results_cannot_activate_a_hidden_setting() {
    let mut state = fixture(&I18n::english());
    state.set_query("does-not-exist");
    assert!(state.visible_section_indices().is_empty());
    assert!(state.selected_item().is_none());
    assert_eq!(
        key(&mut state, KeyCode::Enter),
        SettingsStudioEffect::KeepOpen
    );
    assert!(!state.select_visible(1, 0));
    key(&mut state, KeyCode::End);
    assert!(state.selected_item().is_none());
    let (buffer, _) = render(&state, &I18n::english(), 80, 24);
    assert!(plain(&buffer).contains("No matching settings"));
}

#[test]
fn settings_search_handles_unicode_input_and_snapshot_refresh() {
    let i18n = I18n::resolve(Some("zh-CN"), None);
    let mut state = fixture(&i18n);
    key(&mut state, KeyCode::Char('/'));
    state.paste("ui.tui.\n色彩");
    assert!(!state.query().contains('\n'));
    state.set_query("ui.tui.graphics");
    let mut rebuilt = fixture(&i18n);
    rebuilt.restore_view(&state);
    assert!(rebuilt.is_searching());
    assert_eq!(rebuilt.query(), "ui.tui.graphics");
    assert_eq!(rebuilt.selected_item_index(), 2);
    assert!(rebuilt.selected_item().is_some());
}

#[test]
fn settings_remembers_selection_per_section_and_pages_to_edges() {
    let mut state = fixture(&I18n::english());
    state.set_indices(4, 3);
    state.set_focus(SettingsStudioFocus::Navigation);
    state.move_selection(-1);
    assert_eq!(state.selected_item_index(), 0);
    state.move_selection(1);
    assert_eq!(state.selected_item_index(), 3);
    state.set_focus(SettingsStudioFocus::Items);
    render(&state, &I18n::english(), 80, 18);
    key(&mut state, KeyCode::Home);
    key(&mut state, KeyCode::PageDown);
    assert_eq!(state.selected_item_index(), state.page_size.get().min(4));
    key(&mut state, KeyCode::End);
    assert_eq!(state.selected_item_index(), 4);
    key(&mut state, KeyCode::PageDown);
    assert_eq!(state.selected_item_index(), 4);
}

#[test]
fn settings_layout_survives_resizes_and_all_supported_locales() {
    for (locale, _) in agena_tui::i18n::SUPPORTED_LOCALES {
        let i18n = I18n::resolve(Some(locale), None);
        for (width, height) in [
            (1, 1),
            (12, 4),
            (40, 12),
            (60, 20),
            (80, 24),
            (100, 30),
            (150, 40),
        ] {
            let mut state = fixture(&i18n);
            for focus in [SettingsStudioFocus::Items, SettingsStudioFocus::Navigation] {
                state.set_focus(focus);
                let (buffer, _) = render(&state, &i18n, width, height);
                let text = plain(&buffer);
                assert!(
                    !text.contains("Unknown localization key"),
                    "{locale}: {text}"
                );
                if width >= 80 {
                    assert!(
                        text.contains(&i18n.text("settings-key-search")),
                        "{locale}: {text}"
                    );
                }
            }
            state.details = true;
            render(&state, &i18n, width, height);
        }
    }
}

#[test]
fn settings_details_are_scrollable_and_own_pointer_targets() {
    let mut state = fixture(&I18n::english());
    handle_key(
        &mut state,
        KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL),
    );
    let (_, targets) = render(&state, &I18n::english(), 80, 24);
    assert!(state.detail_max_scroll.get() > 50);
    key(&mut state, KeyCode::End);
    assert_eq!(state.detail_scroll.get(), state.detail_max_scroll.get());
    let (buffer, _) = render(&state, &I18n::english(), 80, 24);
    assert!(plain(&buffer).contains("Configuration source 79"));
    for y in 0..24 {
        for x in 0..80 {
            assert!(!matches!(
                targets.action(click(x, y)),
                Some(PointerAction::List { .. })
            ));
        }
    }
    assert_eq!(
        key(&mut state, KeyCode::Esc),
        SettingsStudioEffect::KeepOpen
    );
    assert!(!state.details_open());
    assert_eq!(state.selected_item_index(), 1);
}

#[test]
fn settings_pointer_targets_follow_filtered_rows_and_narrow_navigation() {
    let i18n = I18n::english();
    let mut state = fixture(&i18n);
    state.set_query("appearance ui.tui.graphics");
    let (buffer, targets) = render(&state, &i18n, 100, 30);
    let text = plain(&buffer);
    assert!(text.contains("Rich Terminal Graphics"));
    assert!(!text.contains("Terminal Color Scheme"));
    let mut hits = 0;
    for y in 0..30 {
        for x in 0..100 {
            if let Some(PointerAction::List {
                panel: 1, index, ..
            }) = targets.action(click(x, y))
            {
                assert_eq!(index, 0);
                hits += 1;
            }
        }
    }
    assert!(hits > 0);
    state.set_query("");
    state.set_focus(SettingsStudioFocus::Navigation);
    let (buffer, _) = render(&state, &i18n, 60, 20);
    assert!(plain(&buffer).contains("Models & Providers"));
    assert!(!plain(&buffer).contains("Terminal Color Scheme"));
    state.select_visible(0, 4);
    let (buffer, _) = render(&state, &i18n, 60, 20);
    assert!(plain(&buffer).contains("Terminal Color Scheme"));
}

#[test]
fn settings_external_labels_are_terminal_safe() {
    let i18n = I18n::english();
    let mut state = fixture(&i18n);
    state.state = SectionedListState::new(
        vec![SettingsStudioSection {
            id: SettingsStudioSectionId::Interface,
            label: "\x1b[31mAppearance\x1b[0m".into(),
            summary: String::new(),
            description: "設定\u{2066}".into(),
            items: vec![SettingsStudioItem::new(
                "\x1b]0;bad\x07Language",
                "English",
                "設定 🧑‍💻 e\u{301}",
                0,
            )],
        }],
        0,
        0,
        SettingsStudioFocus::Items,
    );
    let (buffer, _) = render(&state, &i18n, 80, 24);
    let text = plain(&buffer);
    assert!(text.contains("Language"));
    assert!(!text.contains("bad"));
    assert!(!text.contains('\x1b'));
    assert!(!text.contains('\u{2066}'));
}

/// Export the actual Ratatui cells, so visual review exercises the production renderer.
/// Run with AGENA_SETTINGS_PREVIEW_DIR=/path cargo test -p agena-tui-settings settings_write_previews -- --ignored.
#[test]
#[ignore = "writes visual review artifacts when explicitly requested"]
fn settings_write_previews() {
    use agena_tui_components::theme::{self, ColorScheme, ThemePalette};
    use ratatui::style::{Color, Modifier};
    use std::fmt::Write;
    let directory = std::path::PathBuf::from(
        std::env::var("AGENA_SETTINGS_PREVIEW_DIR").expect("preview directory"),
    );
    std::fs::create_dir_all(&directory).unwrap();
    for (name, locale, width, height, light, query, detail) in [
        ("settings-dark", "en-US", 120, 34, false, "", false),
        ("settings-light", "en-US", 120, 34, true, "", false),
        ("settings-chinese", "zh-CN", 120, 34, false, "", false),
        ("settings-narrow", "zh-CN", 60, 22, false, "", false),
        (
            "settings-search",
            "en-US",
            100,
            28,
            false,
            "appearance graphics",
            false,
        ),
        ("settings-details", "en-US", 100, 28, false, "", true),
    ] {
        let palette = ThemePalette::for_scheme(if light {
            ColorScheme::Light
        } else {
            ColorScheme::Dark
        });
        theme::set_active_palette(palette);
        let i18n = I18n::resolve(Some(locale), None);
        let mut state = fixture(&i18n);
        state.set_query(query);
        state.details = detail;
        let (buffer, _) = render(&state, &i18n, width, height);
        std::fs::write(directory.join(format!("{name}.txt")), plain(&buffer)).unwrap();
        let background = if light { "#fafafa" } else { "#18181b" };
        let foreground = if light { "#1f2328" } else { "#f0f6fc" };
        let color = |color, fallback: &str| match color {
            Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
            _ => fallback.to_owned(),
        };
        let escape = |text: &str| {
            text.replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;")
        };
        let mut svg = format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\" viewBox=\"0 0 {} {}\"><rect width=\"100%\" height=\"100%\" fill=\"{background}\"/>",
            width * 10 + 32,
            height * 22 + 32,
            width * 10 + 32,
            height * 22 + 32
        );
        for y in 0..height {
            for x in 0..width {
                let cell = &buffer[(x, y)];
                let left = x * 10 + 16;
                let top = y * 22 + 16;
                if cell.bg != Color::Reset {
                    write!(
                        svg,
                        "<rect x=\"{left}\" y=\"{top}\" width=\"10\" height=\"22\" fill=\"{}\"/>",
                        color(cell.bg, background)
                    )
                    .unwrap();
                }
            }
        }
        svg.push_str("<g font-family=\"Menlo, 'PingFang SC', monospace\" font-size=\"16\">");
        for y in 0..height {
            for x in 0..width {
                let cell = &buffer[(x, y)];
                if cell.symbol().trim().is_empty() {
                    continue;
                }
                write!(
                    svg,
                    "<text x=\"{}\" y=\"{}\" fill=\"{}\" font-weight=\"{}\">{}</text>",
                    x * 10 + 16,
                    y * 22 + 33,
                    color(cell.fg, foreground),
                    if cell.modifier.contains(Modifier::BOLD) {
                        "700"
                    } else {
                        "400"
                    },
                    escape(cell.symbol())
                )
                .unwrap();
            }
        }
        svg.push_str("</g></svg>");
        std::fs::write(directory.join(format!("{name}.svg")), svg).unwrap();
    }
}
