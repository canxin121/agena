use super::*;

impl App {
    pub(crate) fn open_message_queue(&mut self) {
        let Some(session_id) = self.current_or_selected_session_id() else {
            return;
        };
        self.request_message_queue(session_id, false);
    }

    pub(crate) fn request_message_queue(&mut self, session_id: i64, stopped: bool) {
        self.dispatch_backend_operation(
            move |application| async move {
                let mut execution = application.get_session_state(session_id).await?;
                let ids = execution
                    .queued_messages
                    .iter()
                    .map(|item| item.message_id)
                    .collect::<Vec<_>>();
                for chunk in ids.chunks(64) {
                    let page = application
                        .client()
                        .session_parts_by_ids(session_id, chunk)
                        .await?;
                    execution.parts.extend(page.parts);
                }
                Ok::<_, anyhow::Error>(execution)
            },
            move |app, result| match result {
                Ok(execution) if app.transcript.session_id == Some(session_id) => {
                    let has_queue = !execution.queued_messages.is_empty();
                    let paused = execution.queue_paused;
                    app.handle_session_execution_updated(session_id, execution, false);
                    if stopped {
                        if paused && has_queue {
                            app.show_stopped_message_queue(session_id);
                        }
                    } else {
                        app.show_message_queue(session_id);
                    }
                }
                Ok(_) => {}
                Err(error) => app.flash_error(error),
            },
        );
    }

    fn queue_choice(&self, label_key: &str, detail: String, value: String) -> ChoiceItem {
        let label = ui_text::t(&self.i18n, label_key);
        ChoiceItem {
            search_text: format!("{label} {detail}"),
            label,
            detail,
            value,
            current: false,
        }
    }

    fn show_message_queue(&mut self, session_id: i64) {
        let Some(execution) = self.transcript.execution.as_ref() else {
            return;
        };
        let paused = execution.queue_paused;
        let mut items = vec![self.queue_choice(
            if paused {
                "queue-resume"
            } else {
                "queue-pause"
            },
            ui_text::t(
                &self.i18n,
                if paused {
                    "queue-paused-hint"
                } else {
                    "queue-running-hint"
                },
            ),
            if paused { "resume" } else { "pause" }.into(),
        )];
        if !execution.queued_messages.is_empty() {
            items.push(self.queue_choice(
                "queue-clear",
                ui_text::t(&self.i18n, "queue-clear-hint"),
                "clear".into(),
            ));
        }
        for (index, queued) in execution.queued_messages.iter().enumerate() {
            let preview = execution
                .parts
                .iter()
                .find(|part| part.part_id == queued.message_id)
                .map(queue_part_preview)
                .filter(|text| !text.is_empty())
                .unwrap_or_else(|| ui_text::t(&self.i18n, "queue-message"));
            items.push(ChoiceItem {
                label: format!("{}. {preview}", index + 1),
                detail: ui_text::t(&self.i18n, "queue-item-hint"),
                search_text: preview,
                value: queued.execution_id.to_string(),
                current: false,
            });
        }
        self.open_choice_overlay(self.build_choice_overlay(
            ui_text::t(&self.i18n, "queue-title"),
            ui_text::t(
                &self.i18n,
                if paused {
                    "queue-paused-hint"
                } else {
                    "queue-running-hint"
                },
            ),
            None,
            items,
            ChoiceOverlayAction::MessageQueue { session_id },
            false,
            agena_tui::choice::ChoicePresentationStyle::SelectOnly,
        ));
    }

    pub(crate) fn show_queued_message_actions(
        &mut self,
        session_id: i64,
        execution_id: agena_domain::ExecutionId,
    ) {
        let items = vec![
            self.queue_choice(
                "queue-restore",
                ui_text::t(&self.i18n, "queue-restore-hint"),
                "restore".into(),
            ),
            self.queue_choice("queue-remove", String::new(), "remove".into()),
            self.queue_choice("queue-back", String::new(), "back".into()),
        ];
        self.open_choice_overlay(self.build_choice_overlay(
            ui_text::t(&self.i18n, "queue-message"),
            ui_text::t(&self.i18n, "queue-item-hint"),
            None,
            items,
            ChoiceOverlayAction::QueuedMessage {
                session_id,
                execution_id,
            },
            false,
            agena_tui::choice::ChoicePresentationStyle::SelectOnly,
        ));
    }

    fn show_stopped_message_queue(&mut self, session_id: i64) {
        let items = vec![
            self.queue_choice(
                "queue-keep-stopped",
                ui_text::t(&self.i18n, "queue-paused-hint"),
                "keep".into(),
            ),
            self.queue_choice(
                "queue-send-next",
                ui_text::t(&self.i18n, "queue-running-hint"),
                "resume".into(),
            ),
        ];
        self.open_choice_overlay(self.build_choice_overlay(
            ui_text::t(&self.i18n, "queue-stopped-title"),
            ui_text::t(&self.i18n, "queue-stopped-prompt"),
            None,
            items,
            ChoiceOverlayAction::StoppedMessageQueue { session_id },
            false,
            agena_tui::choice::ChoicePresentationStyle::SelectOnly,
        ));
    }

    pub(crate) fn request_queue_control(&mut self, session_id: i64, action: &str) {
        let clear = action == "clear";
        let paused = action == "pause";
        self.dispatch_backend_operation(
            move |application| async move {
                if clear {
                    application.client().clear_message_queue(session_id).await
                } else {
                    application
                        .client()
                        .set_message_queue_paused(session_id, paused)
                        .await
                }
            },
            move |app, result| match result {
                Ok(execution) => {
                    app.handle_session_execution_updated(session_id, execution, true);
                    app.flash_success(ui_text::t(&app.i18n, "queue-updated"));
                }
                Err(error) => {
                    app.flash_error(error);
                    app.request_refresh(session_id, true);
                }
            },
        );
    }
}

fn queue_part_preview(part: &agena_api::part::PartResource) -> String {
    let text = part
        .content
        .get("text")
        .and_then(JsonValue::as_str)
        .unwrap_or("");
    let preview = if text.trim().is_empty() {
        part.content
            .get("document")
            .and_then(JsonValue::as_array)
            .into_iter()
            .flatten()
            .filter_map(|node| {
                node.pointer("/activity/payload/name")
                    .and_then(JsonValue::as_str)
            })
            .collect::<Vec<_>>()
            .join(" · ")
    } else {
        text.split_whitespace().collect::<Vec<_>>().join(" ")
    };
    preview.chars().take(160).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use agena_tui::choice::ChoiceSelection;

    fn app() -> App {
        let mut app = App::new_with_backend(
            TuiBackend::remote_mock(),
            LaunchOptions::default(),
            I18n::english(),
        );
        let execution: SessionExecutionResource = serde_json::from_value(serde_json::json!({
            "session": { "id":7,"depth":0,"root_id":7,"workspace_id":1,"title":"Queue test","version":1,
                "relation_kind":"root","lifecycle_state":"ready","created_at":"2026-10-03T00:00:00Z","updated_at":"2026-10-03T00:00:00Z","message_count":1,"child_session_count":0 },
            "parts":[],"part_page":null,"execution":{"agent_id":"test"},"usage":{"current_tokens":0},"queue_paused":true,
            "queued_messages":[{"execution_id":"00000000-0000-0000-0000-000000000002","message_id":20},{"execution_id":"00000000-0000-0000-0000-000000000003","message_id":30}]
        })).unwrap();
        app.transcript.session_id = Some(7);
        app.transcript.apply_execution(execution);
        app
    }

    #[test]
    fn queue_manager_lists_each_message_and_selects_an_individual_item() {
        let mut app = app();
        app.show_message_queue(7);
        let Some(Overlay::Choice(mut dialog)) = app.overlay.take() else {
            panic!("queue chooser")
        };
        let values = dialog
            .presentation
            .items
            .iter()
            .map(|item| item.value.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            values,
            [
                "resume",
                "clear",
                "00000000-0000-0000-0000-000000000002",
                "00000000-0000-0000-0000-000000000003"
            ]
        );
        let selected = values[3].to_owned();
        app.commit_choice_overlay(&mut dialog, ChoiceSelection::Item { value: selected });
        let Some(Overlay::Choice(dialog)) = app.overlay.take() else {
            panic!("item actions")
        };
        assert!(
            matches!(dialog.action, ChoiceOverlayAction::QueuedMessage { session_id:7, execution_id } if execution_id.to_string().ends_with("0003"))
        );
        assert_eq!(
            dialog
                .presentation
                .items
                .iter()
                .map(|item| item.value.as_str())
                .collect::<Vec<_>>(),
            ["restore", "remove", "back"]
        );
    }

    #[test]
    fn keeping_stopped_is_a_safe_default_and_preserves_queue_and_composer() {
        let mut app = app();
        app.composer.set_text("new draft".to_owned());
        app.show_stopped_message_queue(7);
        let Some(Overlay::Choice(mut dialog)) = app.overlay.take() else {
            panic!("stop choices")
        };
        assert_eq!(dialog.presentation.items[0].value, "keep");
        app.commit_choice_overlay(
            &mut dialog,
            ChoiceSelection::Item {
                value: "keep".into(),
            },
        );
        assert!(app.transcript.execution.as_ref().unwrap().queue_paused);
        assert_eq!(
            app.transcript
                .execution
                .as_ref()
                .unwrap()
                .queued_messages
                .len(),
            2
        );
        assert_eq!(app.composer.text(), "new draft");
        assert!(app.transcript_footer_text().contains("Queue paused"));
        assert!(app.transcript_footer_text().contains("/queue"));
    }
}

#[cfg(test)]
mod restoration_tests {
    use super::*;

    #[test]
    fn returning_stored_media_keeps_the_new_draft_and_uses_a_readable_attachment_label() {
        let mut app = App::new_with_backend(
            TuiBackend::remote_mock(),
            LaunchOptions::default(),
            I18n::english(),
        );
        app.composer.set_text("new draft".to_owned());
        let document = serde_json::from_value(serde_json::json!([
            {"type":"text","text":"returned text"},
            {"type":"activity","activity":{"id":"00000000-0000-0000-0000-000000000003","payload":{
                "activity_type":"resource","delivery":"model_input","kind":"image","name":"saved-image.png",
                "reference":{"reference_type":"artifact","uri":"agena:media:v1:internal-binding","sha256":"hash"}
            }}}
        ])).unwrap();
        app.restore_queued_message_into_editor(ComposerDraft { document });
        let restored = app.current_composer_draft();
        assert!(restored.text().contains("new draft"));
        assert!(restored.text().contains("returned text"));
        assert!(app.composer.text().contains("saved-image.png"));
        assert!(!app.composer.text().contains("internal-binding"));
        assert!(
            serde_json::to_string(&restored.document)
                .unwrap()
                .contains("agena:media:v1:internal-binding")
        );
    }
}
