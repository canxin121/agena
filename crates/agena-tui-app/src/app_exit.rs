//! Client exit menu. Detaching a TUI is never a request to stop server work.

use crate::{App, ChoiceItem, ChoiceOverlayAction, Overlay, Route, ui_text};
use agena_tui::choice::{ChoicePresentationStyle, ChoiceSelection};
use agena_tui::i18n::I18n;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExitChoice {
    Detach,
    StopAndExit,
    Cancel,
}

impl ExitChoice {
    fn from_selection(selection: &ChoiceSelection) -> Self {
        match selection {
            ChoiceSelection::Item { value } if value == "detach" => Self::Detach,
            ChoiceSelection::Item { value } if value == "stop-and-exit" => Self::StopAndExit,
            // Malformed or no selection must never cause cancellation or exit.
            _ => Self::Cancel,
        }
    }
}

fn exit_menu_items(i18n: &I18n) -> Vec<ChoiceItem> {
    [
        ("detach", "exit-menu-detach", "exit-menu-detach-detail"),
        (
            "stop-and-exit",
            "exit-menu-stop-and-exit",
            "exit-menu-stop-and-exit-detail",
        ),
        ("cancel", "exit-menu-cancel", "exit-menu-cancel-detail"),
    ]
    .into_iter()
    .map(|(value, label, detail)| {
        let label = ui_text::t(i18n, label);
        ChoiceItem {
            search_text: label.clone(),
            label,
            detail: ui_text::t(i18n, detail),
            value: value.to_owned(),
            current: false,
        }
    })
    .collect()
}

impl App {
    pub(crate) fn open_exit_menu(&mut self) {
        if self.exit_after_cancel_session_id.is_some() {
            self.flash_info(ui_text::t(&self.i18n, "exit-menu-stopping"));
            return;
        }
        if matches!(
            &self.overlay,
            Some(Overlay::Choice(dialog)) if matches!(dialog.action, ChoiceOverlayAction::Exit)
        ) {
            // Repeated Ctrl+C cannot bypass the choice or cancel a run.
            return;
        }

        let dialog = self.build_choice_overlay(
            ui_text::t(&self.i18n, "exit-menu-title"),
            ui_text::t(&self.i18n, "exit-menu-prompt"),
            None,
            exit_menu_items(&self.i18n),
            ChoiceOverlayAction::Exit,
            false,
            ChoicePresentationStyle::SelectOnly,
        );
        if let Some(previous) = self.overlay.take() {
            self.overlay_stack.push(previous);
        }
        // Contextual help otherwise renders above the choice picker. Restore
        // it if the user closes the picker without leaving the client.
        self.exit_suspended_help = self.context_help.take();
        self.overlay = Some(Overlay::Choice(dialog));
    }

    pub(crate) fn commit_exit_menu(&mut self, selection: &ChoiceSelection) {
        match ExitChoice::from_selection(selection) {
            ExitChoice::Detach => {
                // Only the TUI exits; the server retains every running task.
                self.should_quit = true;
            }
            ExitChoice::StopAndExit => {
                // A Hub has no current session to stop. On other screens ask
                // the server even if the local activity indicator is stale.
                let session_id = if matches!(self.current_route, Route::Hub(_)) {
                    None
                } else {
                    self.transcript.session_id
                };
                if let Some(session_id) = session_id {
                    self.exit_after_cancel_session_id = Some(session_id);
                    self.exit_suspended_help = None;
                    self.request_cancel_run(session_id);
                    self.flash_info(ui_text::t(&self.i18n, "exit-menu-stopping"));
                } else {
                    self.should_quit = true;
                }
            }
            ExitChoice::Cancel => {
                // Esc and this explicit option both restore the previous UI.
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agena_tui::choice::{self, ChoicePresentationAction, ChoicePresentationEffect};
    use agena_tui::main_focus::Focus;
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

    fn composer_app() -> (tempfile::TempDir, App) {
        let temp = tempfile::tempdir().unwrap();
        let backend = crate::TuiBackend::isolated_test_backend(temp.path().to_path_buf());
        let mut app =
            App::new_with_backend(backend, crate::LaunchOptions::default(), I18n::english());
        app.current_route = Route::Main;
        app.focus = Focus::Composer;
        (temp, app)
    }

    fn ctrl_c() -> KeyEvent {
        KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)
    }

    fn assert_exit_menu(app: &App) {
        assert!(matches!(
            &app.overlay,
            Some(Overlay::Choice(dialog)) if matches!(dialog.action, ChoiceOverlayAction::Exit)
        ));
        assert!(!app.should_quit);
        assert!(app.exit_after_cancel_session_id.is_none());
    }

    #[tokio::test]
    async fn ctrl_c_clears_text_and_whitespace_before_opening_exit_menu() {
        for draft in ["unfinished draft", " \n "] {
            let (_temp, mut app) = composer_app();
            app.transcript.session_id = Some(17);
            app.composer.set_text(draft.to_owned());

            app.handle_key_event(ctrl_c());

            assert!(app.composer.text().is_empty());
            assert!(app.overlay.is_none());
            assert!(!app.should_quit);
            assert!(app.exit_after_cancel_session_id.is_none());

            app.handle_key_event(ctrl_c());
            assert_exit_menu(&app);
        }
    }

    #[tokio::test]
    async fn ctrl_c_clears_attachments_and_invalidates_pending_media() {
        use agena_api::part::PartExecutionStatusResource;
        use agena_domain::{ResourceActivity, ResourceDelivery, ResourceKind, ResourceReference};

        for state in [
            PartExecutionStatusResource::InProgress,
            PartExecutionStatusResource::Completed,
        ] {
            let (_temp, mut app) = composer_app();
            app.reserve_pending_composer_media(
                ResourceActivity {
                    delivery: ResourceDelivery::Reference,
                    kind: ResourceKind::File,
                    reference: ResourceReference::WorkspacePath {
                        path: "draft.txt".to_owned(),
                    },
                    name: "draft.txt".to_owned(),
                    media_type: None,
                    size_bytes: Some(1),
                    width: None,
                    height: None,
                    duration_ms: None,
                    page_count: None,
                },
                None,
            );
            app.composer_items[0].state = state;
            assert!(app.toggle_composer_item_selection());
            let media_epoch = app.composer_media_epoch;

            app.handle_key_event(ctrl_c());

            assert!(app.composer.text().is_empty());
            assert!(app.composer.draft_elements().is_empty());
            assert!(app.composer_items.is_empty());
            assert!(!app.composer_item_selection.is_active());
            assert_eq!(app.composer_media_epoch, media_epoch.wrapping_add(1));
            assert!(app.overlay.is_none());

            app.handle_key_event(ctrl_c());
            assert_exit_menu(&app);
        }
    }

    #[tokio::test]
    async fn ctrl_c_preserves_drafts_outside_the_active_composer() {
        for surface in ["transcript", "help", "command palette", "exit menu"] {
            let (_temp, mut app) = composer_app();
            app.composer.set_text("keep this draft".to_owned());
            match surface {
                "transcript" => app.focus = Focus::Transcript,
                "help" => {
                    app.toggle_context_help();
                }
                "command palette" => {
                    app.open_command_palette();
                }
                "exit menu" => {
                    app.open_exit_menu();
                }
                _ => unreachable!(),
            }

            app.handle_key_event(ctrl_c());

            assert_eq!(app.composer.text(), "keep this draft", "{surface}");
            assert_exit_menu(&app);
            let overlay_stack_len = app.overlay_stack.len();

            app.handle_key_event(ctrl_c());

            assert_eq!(app.composer.text(), "keep this draft", "{surface}");
            assert_exit_menu(&app);
            assert_eq!(app.overlay_stack.len(), overlay_stack_len);
        }
    }

    #[tokio::test]
    async fn ctrl_c_release_does_not_clear_or_open_exit_menu() {
        let (_temp, mut app) = composer_app();
        app.composer.set_text("unfinished draft".to_owned());
        let mut release = ctrl_c();
        release.kind = KeyEventKind::Release;

        app.handle_key_event(release);

        assert_eq!(app.composer.text(), "unfinished draft");
        assert!(app.overlay.is_none());
        assert!(!app.should_quit);
    }

    #[test]
    fn exit_picker_defaults_to_detach_and_shows_three_actions() {
        let i18n = I18n::english();
        let items = exit_menu_items(&i18n);
        assert_eq!(
            items
                .iter()
                .map(|item| item.value.as_str())
                .collect::<Vec<_>>(),
            ["detach", "stop-and-exit", "cancel"]
        );
        let mut picker = choice::new_presentation(
            i18n.text("exit-menu-title"),
            i18n.text("exit-menu-prompt"),
            "Enter select / Esc cancel".into(),
            "No choices".into(),
            items,
            None,
            None,
            ChoicePresentationStyle::SelectOnly,
            String::new(),
            String::new(),
            String::new(),
        );
        assert_eq!(
            choice::selection(&picker),
            Some(ChoiceSelection::Item {
                value: "detach".into()
            })
        );
        assert_eq!(
            choice::reduce(&mut picker, ChoicePresentationAction::Accept),
            ChoicePresentationEffect::Commit(ChoiceSelection::Item {
                value: "detach".into(),
            })
        );
    }

    #[test]
    fn exit_picker_stop_requires_explicit_selection_and_escape_only_closes() {
        let i18n = I18n::english();
        let mut picker = choice::new_presentation(
            i18n.text("exit-menu-title"),
            i18n.text("exit-menu-prompt"),
            String::new(),
            String::new(),
            exit_menu_items(&i18n),
            None,
            None,
            ChoicePresentationStyle::SelectOnly,
            String::new(),
            String::new(),
            String::new(),
        );

        let down = KeyEvent::new(KeyCode::Down, KeyModifiers::NONE);
        assert_eq!(
            choice::reduce(&mut picker, ChoicePresentationAction::Input(down)),
            ChoicePresentationEffect::KeepOpen
        );
        assert_eq!(
            ExitChoice::from_selection(&choice::selection(&picker).unwrap()),
            ExitChoice::StopAndExit
        );
        assert_eq!(
            choice::reduce(&mut picker, ChoicePresentationAction::Accept),
            ChoicePresentationEffect::Commit(ChoiceSelection::Item {
                value: "stop-and-exit".into(),
            })
        );
        assert_eq!(
            choice::reduce(
                &mut picker,
                ChoicePresentationAction::Input(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            ),
            ChoicePresentationEffect::Close
        );
    }

    #[test]
    fn exit_menu_labels_exist_in_all_supported_locales() {
        for (locale, _) in agena_tui::i18n::SUPPORTED_LOCALES {
            let i18n = I18n::resolve(Some(locale), None);
            for key in [
                "exit-menu-title",
                "exit-menu-prompt",
                "exit-menu-detach",
                "exit-menu-detach-detail",
                "exit-menu-stop-and-exit",
                "exit-menu-stop-and-exit-detail",
                "exit-menu-cancel",
                "exit-menu-cancel-detail",
                "exit-menu-stopping",
                "context-help-key-exit-menu",
            ] {
                let value = i18n.text(key);
                assert!(
                    !value.is_empty() && !value.contains("Unknown localization key"),
                    "missing or empty {key} for {locale}: {value}"
                );
            }
        }
    }

    #[test]
    fn cancel_and_unknown_selections_are_safe() {
        assert_eq!(
            ExitChoice::from_selection(&ChoiceSelection::Item {
                value: "cancel".into(),
            }),
            ExitChoice::Cancel
        );
        assert_eq!(
            ExitChoice::from_selection(&ChoiceSelection::Clear),
            ExitChoice::Cancel
        );
        assert_eq!(
            ExitChoice::from_selection(&ChoiceSelection::Item {
                value: "unexpected".into(),
            }),
            ExitChoice::Cancel
        );
    }
}
