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
