use agena_domain::{
    ExternalAgent, ExternalAgentOptionChange, ExternalAgentOptions, ExternalAgentWorkItem,
    ExternalAgentWorkKind,
};
use agena_tui::choice::ChoicePresentationStyle;

use crate::{App, ChoiceItem, ChoiceOverlayAction, ui_text};

impl App {
    pub(crate) fn load_external_agent_command_preview(&mut self) {
        let Some((session_id, _, _)) = self.external_agent_commands() else {
            return;
        };
        if !self.can_preview_external_agent_commands(session_id)
            || self
                .external_agent_command_preview
                .as_ref()
                .is_some_and(|(id, _)| *id == session_id)
        {
            return;
        }
        self.external_agent_command_preview = Some((session_id, Vec::new()));
        self.dispatch_backend_operation(
            move |backend| async move {
                backend
                    .client()
                    .preview_external_agent_options(session_id)
                    .await
            },
            move |app, result| {
                if app
                    .external_agent_command_preview
                    .as_ref()
                    .is_none_or(|(id, _)| *id != session_id)
                {
                    return;
                }
                app.external_agent_command_preview = result
                    .ok()
                    .filter(|_| app.can_preview_external_agent_commands(session_id))
                    .map(|options| (session_id, options.preview_commands));
                if app.external_agent_command_preview.is_some() {
                    app.refresh_command_palette();
                    app.refresh_slash_command_suggestions();
                }
            },
        );
    }

    pub(crate) fn request_external_agent_work(
        &mut self,
        session_id: i64,
        cancel_id: Option<uuid::Uuid>,
    ) {
        if self.current_or_selected_session_id() != Some(session_id) {
            return;
        }
        self.dispatch_backend_operation(
            move |backend| async move {
                backend
                    .client()
                    .external_agent_work(session_id, cancel_id)
                    .await
            },
            move |app, result| {
                if app.current_or_selected_session_id() != Some(session_id) {
                    return;
                }
                match result {
                    Ok(_) if cancel_id.is_some() => {
                        app.flash_info(ui_text::t(&app.i18n, "external-agent-work-requested"));
                    }
                    Ok(items) => app.open_external_agent_work(session_id, items),
                    Err(error) => app.flash_error(error),
                }
            },
        );
    }

    fn open_external_agent_work(&mut self, session_id: i64, work: Vec<ExternalAgentWorkItem>) {
        if work.is_empty() {
            self.flash_info(ui_text::t(&self.i18n, "external-agent-work-empty"));
            return;
        }
        let items = work
            .into_iter()
            .map(|item| {
                let label = item.title.unwrap_or_else(|| item.target_id.clone());
                let kind = ui_text::t(
                    &self.i18n,
                    match item.kind {
                        ExternalAgentWorkKind::ToolCall => "external-agent-work-tool-call",
                        ExternalAgentWorkKind::Terminal => "external-agent-work-terminal",
                        ExternalAgentWorkKind::Subagent => "external-agent-work-subagent",
                        ExternalAgentWorkKind::AsyncTask => "external-agent-work-async-task",
                    },
                );
                ChoiceItem {
                    search_text: format!("{label} {kind} {} {}", item.session_id, item.target_id),
                    value: item.cancel_id.to_string(),
                    detail: format!("{kind} · {}", item.session_id),
                    label,
                    current: false,
                }
            })
            .collect();
        self.open_choice_overlay(self.build_choice_overlay(
            ui_text::t(&self.i18n, "external-agent-work-cancel"),
            String::new(),
            None,
            items,
            ChoiceOverlayAction::ExternalAgentWork { session_id },
            false,
            ChoicePresentationStyle::SearchableSelect,
        ));
    }

    pub(crate) fn request_external_agent_options(
        &mut self,
        session_id: i64,
        agent: ExternalAgent,
        change: Option<ExternalAgentOptionChange>,
    ) {
        if self.transcript.session_id != Some(session_id) {
            return;
        }
        self.dispatch_backend_operation(
            move |backend| async move {
                backend
                    .client()
                    .external_agent_options(session_id, change)
                    .await
            },
            move |app, result| {
                if app.transcript.session_id != Some(session_id) {
                    return;
                }
                match result {
                    Ok(options) => app.open_external_agent_options(session_id, agent, options),
                    Err(error) => app.flash_error(error),
                }
            },
        );
    }

    fn open_external_agent_options(
        &mut self,
        session_id: i64,
        agent: ExternalAgent,
        state: ExternalAgentOptions,
    ) {
        let mut items = Vec::new();
        let mut changes = Vec::new();
        let default_label = ui_text::t(&self.i18n, "value-default");
        for option in state.options {
            let selected = state.selection.get(&option.kind);
            let current_name = option
                .choices
                .iter()
                .find(|choice| choice.value == option.current_value)
                .map(|choice| choice.name.as_str())
                .unwrap_or(&option.current_value);
            let label = format!("{} · {default_label}", option.name);
            items.push(ChoiceItem {
                label: label.clone(),
                search_text: label,
                value: items.len().to_string(),
                detail: current_name.to_owned(),
                current: selected.is_none(),
            });
            changes.push(ExternalAgentOptionChange {
                kind: option.kind.clone(),
                value: None,
            });
            for choice in option.choices {
                let current = selected == Some(&choice.value);
                let label = format!("{} · {}", option.name, choice.name);
                items.push(ChoiceItem {
                    label: label.clone(),
                    search_text: label,
                    value: items.len().to_string(),
                    detail: choice.description.unwrap_or_default(),
                    current,
                });
                changes.push(ExternalAgentOptionChange {
                    kind: option.kind.clone(),
                    value: Some(choice.value),
                });
            }
        }
        for (kind, value) in state.selection {
            if changes.iter().any(|change| change.kind == kind) {
                continue;
            }
            let label = format!("{} · {default_label}", kind.label());
            items.push(ChoiceItem {
                label: label.clone(),
                search_text: label,
                value: items.len().to_string(),
                detail: value,
                current: false,
            });
            changes.push(ExternalAgentOptionChange { kind, value: None });
        }

        self.open_choice_overlay(self.build_choice_overlay(
            agent.label().to_owned(),
            String::new(),
            None,
            items,
            ChoiceOverlayAction::ExternalAgentOptions {
                session_id,
                agent,
                changes,
            },
            false,
            ChoicePresentationStyle::SearchableSelect,
        ));
    }
}
