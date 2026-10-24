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
                "queue-edit",
                ui_text::t(&self.i18n, "queue-edit-hint"),
                "edit".into(),
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
