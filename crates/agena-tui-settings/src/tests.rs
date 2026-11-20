use super::*;

fn sections() -> Vec<SettingsStudioSection<&'static str>> {
    vec![
        SettingsStudioSection {
            id: SettingsStudioSectionId::ModelsProviders,
            label: "Models".into(),
            summary: "Provider routing".into(),
            description: "Choose default models".into(),
            items: vec![
                SettingsStudioItem::from_parts(
                    "Default Model",
                    "model-one",
                    "Main conversation model",
                    Some("providers.default".into()),
                    Some("model-one".into()),
                    None,
                    Vec::new(),
                    "edit-model",
                ),
                SettingsStudioItem::new("Temperature", "0.5", "Sampling value", "edit-temp"),
            ],
        },
        SettingsStudioSection {
            id: SettingsStudioSectionId::RuntimeSession,
            label: "Sessions".into(),
            summary: "Run limits".into(),
            description: "Session execution controls".into(),
            items: vec![
                SettingsStudioItem::new("Max Turns", "12", "Execution budget", "edit-turns"),
                SettingsStudioItem::new("Timeout", "60", "Time budget", "edit-timeout"),
            ],
        },
    ]
}

fn fixture() -> SettingsStudioPresentation<&'static str> {
    SettingsStudioPresentation::new(sections(), 0, 0, SettingsStudioFocus::Navigation)
}

#[test]
fn edits_remain_bound_to_their_original_config_path_and_can_save_empty_strings() {
    let mut state = SettingsState::default();
    assert_eq!(state.reduce(SettingsAction::Save), None);
    state.reduce(SettingsAction::EditField {
        path: "providers.default".into(),
        value: "model-two".into(),
    });
    assert_eq!(
        state.reduce(SettingsAction::Save),
        Some(SettingsEffect::SaveField {
            path: "providers.default".into(),
            value: "model-two".into(),
        })
    );
    // Reselecting the same field does not lose its edit.
    state.reduce(SettingsAction::SelectField {
        path: "providers.default".into(),
    });
    assert_eq!(state.pending_value.as_deref(), Some("model-two"));
    // Switching away cannot reuse the old editor's value for a different setting.
    state.reduce(SettingsAction::SelectField {
        path: "session.max_turns".into(),
    });
    assert_eq!(state.reduce(SettingsAction::Save), None);
    assert_eq!(state.selected_path.as_deref(), Some("session.max_turns"));
    assert!(state.pending_value.is_none());

    state.reduce(SettingsAction::EditField {
        path: "session.max_turns".into(),
        value: String::new(),
    });
    assert_eq!(
        state.reduce(SettingsAction::Save),
        Some(SettingsEffect::SaveField {
            path: "session.max_turns".into(),
            value: String::new(),
        }),
        "explicit empty values must not be confused with unsupplied edits"
    );
    assert_eq!(
        state.reduce(SettingsAction::Reload),
        Some(SettingsEffect::ReloadRuntime)
    );
}

#[test]
fn search_filters_sections_and_items_by_terms_without_losing_the_selection() {
    let mut view = fixture();
    assert_eq!(view.visible_section_indices(), vec![0, 1]);
    assert_eq!(view.visible_item_indices(0), vec![0, 1]);
    view.set_query(" default MODEL\n");
    assert_eq!(view.visible_section_indices(), vec![0]);
    assert_eq!(view.visible_item_indices(0), vec![0]);
    assert!(view.visible_item_indices(1).is_empty());

    view.set_query("budget");
    assert_eq!(view.visible_section_indices(), vec![1]);
    assert_eq!(view.visible_item_indices(1), vec![0, 1]);
    assert_eq!(view.selected_section_index(), 1);

    view.set_query("no possible match");
    assert!(view.visible_section_indices().is_empty());
    assert!(!view.select_visible(1, 0));
    view.set_query("");
    assert_eq!(view.visible_section_indices(), vec![0, 1]);
}

#[test]
fn navigation_remembers_each_sections_item_and_rejects_filtered_outside_indices() {
    let mut view = fixture();
    view.set_focus(SettingsStudioFocus::Items);
    view.move_selection(1);
    assert_eq!(view.selected_item_index(), 1);
    view.set_focus(SettingsStudioFocus::Navigation);
    view.move_selection(1);
    assert_eq!(view.selected_section_index(), 1);
    view.set_focus(SettingsStudioFocus::Items);
    assert_eq!(view.selected_item_index(), 0);
    view.move_selection(1);
    assert_eq!(view.selected_item_index(), 1);
    view.set_focus(SettingsStudioFocus::Navigation);
    view.move_selection(-1);
    assert_eq!(view.selected_section_index(), 0);
    assert_eq!(view.selected_item_index(), 1);
    assert!(!view.select_visible(1, 100));
    assert!(view.select_visible(1, 0));
    assert_eq!(view.selected_item_index(), 0);
}

#[test]
fn keyboard_search_details_and_close_follow_production_key_mapping() {
    let mut view = fixture();
    let key = |code| KeyEvent::new(code, KeyModifiers::NONE);
    assert_eq!(
        handle_key(&mut view, key(KeyCode::Char('/'))),
        SettingsStudioEffect::KeepOpen
    );
    assert!(view.is_searching());
    assert_eq!(
        handle_key(&mut view, key(KeyCode::Char('m'))),
        SettingsStudioEffect::KeepOpen
    );
    assert!(!view.query().is_empty());
    assert_eq!(
        handle_key(&mut view, key(KeyCode::Enter)),
        SettingsStudioEffect::KeepOpen
    );
    assert!(!view.is_searching());
    assert_eq!(view.focus(), SettingsStudioFocus::Items);
    assert_eq!(
        handle_key(&mut view, key(KeyCode::Esc)),
        SettingsStudioEffect::KeepOpen
    );
    assert_eq!(view.query(), "");
    assert_eq!(
        handle_key(
            &mut view,
            KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL)
        ),
        SettingsStudioEffect::KeepOpen
    );
    assert!(view.details_open());
    assert_eq!(
        handle_key(&mut view, key(KeyCode::Esc)),
        SettingsStudioEffect::KeepOpen
    );
    assert!(!view.details_open());
    assert_eq!(
        handle_key(&mut view, key(KeyCode::Esc)),
        SettingsStudioEffect::Close
    );
}

#[test]
fn dense_setting_rows_keep_source_visible_for_long_values_and_show_source_shortcut() {
    use agena_tui::i18n::I18n;
    use ratatui::{Terminal, backend::TestBackend, text::Text};
    for width in [80, 100] {
        for locale in ["en-US", "zh-CN"] {
            let i18n = I18n::resolve(Some(locale), None);
            let inherited = i18n.text("permission-studio-origin-inherited");
            let local = i18n.text("permission-studio-origin-current");
            let state = SettingsStudioPresentation::new(
                vec![SettingsStudioSection {
                    id: SettingsStudioSectionId::RuntimeSession,
                    label: "Runtime".into(),
                    summary: String::new(),
                    description: String::new(),
                    items: vec![
                        SettingsStudioItem::new(
                            "Long value",
                            "server/workspace/".repeat(20),
                            "",
                            "path",
                        )
                        .with_source(inherited.clone()),
                        SettingsStudioItem::new("Enabled", "true", "", "enabled")
                            .with_source(local.clone()),
                    ],
                }],
                0,
                0,
                SettingsStudioFocus::Items,
            );
            let mut terminal = Terminal::new(TestBackend::new(width, 24)).unwrap();
            terminal
                .draw(|frame| {
                    crate::render::render_settings(
                        frame,
                        frame.area(),
                        agena_tui_components::SurfaceMode::Route,
                        &state,
                        &i18n,
                        &crate::render::SettingsViewSpec {
                            title: "Settings",
                            detail_title: "Details",
                            detail: Text::default(),
                        },
                    )
                })
                .unwrap();
            let buffer = terminal.backend().buffer();
            let text = buffer
                .content
                .iter()
                .map(|cell| cell.symbol())
                .collect::<String>()
                .replace(' ', "");
            assert!(
                text.contains(&format!("[{inherited}]")),
                "source clipped at {width} in {locale}"
            );
            assert!(text.contains(&format!("[{local}]")));
            assert!(
                text.contains(&i18n.text("value-on")),
                "boolean value display lost"
            );
            assert!(text.contains(&i18n.text("settings-key-source").replace(' ', "")));
            assert!(text.contains(if width >= 100 { "Ctrl+O" } else { "^O" }));
        }
    }
}

#[test]
fn source_shortcut_requires_a_selected_setting_and_does_not_interrupt_search_or_details() {
    let mut view = fixture();
    let source = KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL);
    assert_eq!(
        handle_key(&mut view, source),
        SettingsStudioEffect::KeepOpen
    );
    view.set_focus(SettingsStudioFocus::Items);
    assert_eq!(handle_key(&mut view, source), SettingsStudioEffect::Source);

    handle_key(
        &mut view,
        KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE),
    );
    assert_eq!(
        handle_key(&mut view, source),
        SettingsStudioEffect::KeepOpen
    );
    assert!(view.is_searching());
    handle_key(&mut view, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

    handle_key(
        &mut view,
        KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL),
    );
    assert_eq!(
        handle_key(&mut view, source),
        SettingsStudioEffect::KeepOpen
    );
    assert!(view.details_open());
    handle_key(&mut view, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    view.set_query("missing setting");
    assert_eq!(
        handle_key(&mut view, source),
        SettingsStudioEffect::KeepOpen
    );
}

#[test]
fn global_rows_offer_reset_actions_without_source_states() {
    use agena_tui::i18n::I18n;
    use ratatui::{Terminal, backend::TestBackend, text::Text};
    for width in [80, 100] {
        for locale in ["en-US", "zh-CN"] {
            let i18n = I18n::resolve(Some(locale), None);
            for enabled in [false, true] {
                let item = SettingsStudioItem::new("Tuning", "false", "", "toggle")
                    .with_control(SettingsStudioControl::RestoreDefault { enabled });
                assert!(item.source.is_none());
                let mut state = SettingsStudioPresentation::new(
                    vec![SettingsStudioSection {
                        id: SettingsStudioSectionId::RuntimeSession,
                        label: "Runtime".into(),
                        summary: String::new(),
                        description: String::new(),
                        items: vec![item.clone(), item],
                    }],
                    0,
                    0,
                    SettingsStudioFocus::Items,
                );
                let mut terminal = Terminal::new(TestBackend::new(width, 24)).unwrap();
                let pointer_map = agena_tui_components::pointer::capture(|| {
                    terminal
                        .draw(|frame| {
                            crate::render::render_settings(
                                frame,
                                frame.area(),
                                agena_tui_components::SurfaceMode::Route,
                                &state,
                                &i18n,
                                &crate::render::SettingsViewSpec {
                                    title: "Settings",
                                    detail_title: "Details",
                                    detail: Text::default(),
                                },
                            )
                        })
                        .unwrap();
                });
                let restore_cells = terminal
                    .backend()
                    .buffer()
                    .content
                    .iter()
                    .enumerate()
                    .filter(|(_, cell)| cell.symbol() == "↺")
                    .collect::<Vec<_>>();
                assert_eq!(restore_cells.len(), 2);
                for (item, (cell, _)) in restore_cells.into_iter().enumerate() {
                    let action = pointer_map.action(crossterm::event::MouseEvent {
                        kind: crossterm::event::MouseEventKind::Down(
                            crossterm::event::MouseButton::Left,
                        ),
                        column: (cell % width as usize) as u16,
                        row: (cell / width as usize) as u16,
                        modifiers: KeyModifiers::NONE,
                    });
                    assert_eq!(
                        action,
                        Some(if enabled {
                            agena_tui_components::pointer::PointerAction::NamedIndex(
                                "settings-restore-default",
                                item,
                            )
                        } else {
                            agena_tui_components::pointer::PointerAction::Named(
                                "settings-restore-disabled",
                            )
                        })
                    );
                }
                let text = terminal
                    .backend()
                    .buffer()
                    .content
                    .iter()
                    .map(|cell| cell.symbol())
                    .collect::<String>()
                    .replace(' ', "");
                assert!(text.contains("[↺]"));
                for key in [
                    "permission-studio-origin-current",
                    "permission-studio-origin-default",
                    "permission-studio-origin-inherited",
                ] {
                    assert!(!text.contains(&format!("[{}]", i18n.text(key))));
                }
                if enabled {
                    assert!(
                        text.contains(&i18n.text("settings-key-restore-default").replace(' ', ""))
                    );
                }
                assert_eq!(
                    handle_key(
                        &mut state,
                        KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL)
                    ),
                    if enabled {
                        SettingsStudioEffect::Source
                    } else {
                        SettingsStudioEffect::KeepOpen
                    }
                );
            }
        }
    }
}

#[test]
fn inherited_values_render_muted_with_parent_and_accept_only_source_actions() {
    use agena_tui::i18n::I18n;
    use agena_tui_components::theme;
    use ratatui::{Terminal, backend::TestBackend, style::Modifier, text::Text};
    let i18n = I18n::english();
    let mut state = SettingsStudioPresentation::new(
        vec![SettingsStudioSection {
            id: SettingsStudioSectionId::RuntimeSession,
            label: "Runtime".into(),
            summary: String::new(),
            description: String::new(),
            items: vec![
                SettingsStudioItem::new("Inherited", "λParentValue", "", "inherited")
                    .with_control(SettingsStudioControl::Inherited("inherit · Global".into())),
                SettingsStudioItem::new("Local", "ΩLocalValue", "", "local")
                    .with_control(SettingsStudioControl::Source("local".into())),
            ],
        }],
        0,
        0,
        SettingsStudioFocus::Items,
    );
    assert_eq!(
        handle_key(
            &mut state,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)
        ),
        SettingsStudioEffect::KeepOpen
    );
    assert_eq!(
        handle_key(
            &mut state,
            KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE)
        ),
        SettingsStudioEffect::KeepOpen
    );
    assert_eq!(
        handle_key(
            &mut state,
            KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL)
        ),
        SettingsStudioEffect::Source
    );
    let mut terminal = Terminal::new(TestBackend::new(120, 24)).unwrap();
    terminal
        .draw(|frame| {
            crate::render::render_settings(
                frame,
                frame.area(),
                agena_tui_components::SurfaceMode::Route,
                &state,
                &i18n,
                &crate::render::SettingsViewSpec {
                    title: "Settings",
                    detail_title: "Details",
                    detail: Text::default(),
                },
            )
        })
        .unwrap();
    let buffer = terminal.backend().buffer();
    let text = buffer
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(text.contains("inherit · Global"));
    assert!(text.contains("λParentValue"));
    let inherited = buffer
        .content
        .iter()
        .find(|cell| cell.symbol() == "λ")
        .unwrap();
    assert_eq!(inherited.fg, theme::muted_color());
    assert!(inherited.modifier.contains(Modifier::DIM));
    let local = buffer
        .content
        .iter()
        .find(|cell| cell.symbol() == "Ω")
        .unwrap();
    assert!(!local.modifier.contains(Modifier::DIM));
    state.move_selection(1);
    assert_eq!(
        handle_key(
            &mut state,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)
        ),
        SettingsStudioEffect::Activate
    );
}
