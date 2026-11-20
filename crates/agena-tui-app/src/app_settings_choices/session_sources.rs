use crate::{App, ChoiceOverlayAction, I18n, ModelRef, RunOptions, SessionModelModeStep};
use agena_domain::ExecutionSelection;

pub(crate) fn model_selection_only(selection: &ExecutionSelection) -> ExecutionSelection {
    let mut selection = selection.clone();
    selection.permission = Default::default();
    selection
}

pub(crate) fn session_model_route_is_local(selection: &ExecutionSelection) -> bool {
    selection.provider.is_some() || selection.adapter.is_some() || selection.model.is_some()
}

pub(crate) fn session_model_mode_is_local(
    selection: &ExecutionSelection,
    step: SessionModelModeStep,
) -> bool {
    session_model_route_is_local(selection) || session_model_mode_value(selection, step).is_some()
}

pub(crate) fn session_model_has_override(selection: &ExecutionSelection) -> bool {
    !model_selection_only(selection).is_empty()
}

/// The public write accepts a complete model identity or mode-only overrides.
/// Partial legacy identities must be chosen explicitly, never filled from the
/// effective model and silently pinned into this layer.
pub(crate) fn session_model_selection_request(
    selection: &ExecutionSelection,
) -> Option<RunOptions> {
    let model = match (selection.provider.as_ref(), selection.model.as_ref()) {
        (Some(provider), Some(model)) => Some(agena_api::resource::ModelRef {
            provider_id: provider.clone(),
            adapter_id: selection.adapter.clone(),
            model_id: model.clone(),
        }),
        (None, None) if selection.adapter.is_none() => None,
        _ => return None,
    };
    Some(RunOptions {
        model,
        thinking_mode: selection.thinking_mode.clone(),
        speed_mode: selection.speed_mode.clone(),
        verbosity: selection.verbosity.clone(),
        parallel_tool_calls: selection.parallel_tool_calls,
        ..Default::default()
    })
}

pub(crate) fn session_model_mode_value(
    selection: &ExecutionSelection,
    step: SessionModelModeStep,
) -> Option<String> {
    match step {
        SessionModelModeStep::ThinkingMode => selection.thinking_mode.clone(),
        SessionModelModeStep::SpeedMode => selection.speed_mode.clone(),
        SessionModelModeStep::Verbosity => selection.verbosity.clone(),
    }
}

pub(crate) fn inherited_session_model_mode(
    selected: &ExecutionSelection,
    parent: &ExecutionSelection,
    step: SessionModelModeStep,
) -> Option<String> {
    let mut reset = selected.clone();
    match step {
        SessionModelModeStep::ThinkingMode => reset.thinking_mode = None,
        SessionModelModeStep::SpeedMode => reset.speed_mode = None,
        SessionModelModeStep::Verbosity => reset.verbosity = None,
    }
    session_model_mode_value(&parent.overlay_with_cascade(&reset), step)
}

fn selection_summary(i18n: &I18n, selection: &ExecutionSelection) -> String {
    let selection = model_selection_only(selection);
    if selection.is_empty() {
        return i18n.text("settings-source-unset");
    }
    let mut parts = Vec::new();
    if let (Some(provider), Some(model)) = (&selection.provider, &selection.model) {
        parts.push(if let Some(adapter) = &selection.adapter {
            format!("{provider}/{adapter}/{model}")
        } else {
            format!("{provider}/{model}")
        });
    } else {
        parts.extend(
            [
                selection.provider.clone(),
                selection.adapter.clone(),
                selection.model.clone(),
            ]
            .into_iter()
            .flatten(),
        );
    }
    for step in [
        SessionModelModeStep::ThinkingMode,
        SessionModelModeStep::SpeedMode,
        SessionModelModeStep::Verbosity,
    ] {
        if let Some(value) = session_model_mode_value(&selection, step) {
            parts.push(format!(
                "{}={value}",
                crate::model_mode_display_label(i18n, step)
            ));
        }
    }
    if let Some(parallel) = selection.parallel_tool_calls {
        parts.push(format!("parallel={parallel}"));
    }
    parts.join(" · ")
}

pub(crate) fn model_source_context_matches(
    expected_session: Option<i64>,
    current_session: Option<i64>,
    expected_model: &Option<ModelRef>,
    current_model: &Option<ModelRef>,
) -> bool {
    expected_session == current_session && expected_model == current_model
}

impl App {
    pub(crate) fn current_raw_model_selection(&self) -> ExecutionSelection {
        self.run_options
            .raw_selection
            .clone()
            .or_else(|| {
                self.transcript
                    .execution
                    .as_ref()
                    .map(|execution| execution.execution.selected_selection.clone())
            })
            .unwrap_or_else(|| self.run_options.raw_model_selection())
    }

    pub(crate) fn session_model_source_prompt(&self) -> String {
        let raw = self.current_raw_model_selection();
        let (source, value) = if session_model_route_is_local(&raw) {
            (
                self.i18n.text("settings-layer-local"),
                selection_summary(&self.i18n, &raw),
            )
        } else {
            let value = self
                .transcript
                .execution
                .as_ref()
                .map(|execution| {
                    selection_summary(&self.i18n, &execution.execution.inherited_selection)
                })
                .unwrap_or_else(|| self.i18n.text("settings-source-unavailable"));
            (
                crate::settings_inherited_source_label(&self.i18n, true),
                value,
            )
        };
        format!(
            "{}: {source} · {value}",
            self.i18n.text("settings-layer-source-title")
        )
    }

    pub(crate) fn open_session_model_mode_source(&mut self, step: SessionModelModeStep) {
        let Some(session_id) = self.transcript.session_id else {
            return;
        };
        let selected = self.current_raw_model_selection();
        if session_model_route_is_local(&selected) {
            self.open_session_model_source();
            return;
        }
        if self.session_is_busy(session_id) {
            self.flash_warning(self.i18n.text("flash-session-busy"));
            return;
        }
        let inherited = self
            .transcript
            .execution
            .as_ref()
            .map(|execution| {
                session_model_mode_value(&execution.execution.inherited_selection, step)
                    .unwrap_or_else(|| self.i18n.text("value-default"))
            })
            .unwrap_or_else(|| self.i18n.text("settings-source-unavailable"));
        let local = session_model_mode_value(&selected, step);
        let picker = self.build_choice_overlay(
            format!(
                "{} · {}",
                crate::model_mode_display_label(&self.i18n, step),
                self.i18n.text("settings-layer-source-title")
            ),
            crate::settings_inherited_locked_message(&self.i18n, true),
            Some(if local.is_some() { "local" } else { "inherit" }.into()),
            super::layers::source_items(
                &self.i18n,
                true,
                format!(
                    "{} · {inherited}",
                    self.i18n.text("settings-layer-parent-config")
                ),
                local.unwrap_or(inherited),
            ),
            ChoiceOverlayAction::SessionModelModeSource {
                step,
                session_id,
                model: self.current_session_model_ref(),
                selected,
            },
            false,
            agena_tui::choice::ChoicePresentationStyle::SelectOnly,
        );
        self.open_choice_overlay(picker);
    }

    pub(crate) fn commit_session_model_mode_source(
        &mut self,
        step: SessionModelModeStep,
        session_id: i64,
        model: Option<ModelRef>,
        selected: ExecutionSelection,
        value: &str,
    ) -> bool {
        if !model_source_context_matches(
            Some(session_id),
            self.transcript.session_id,
            &model,
            &self.current_session_model_ref(),
        ) || selected != self.current_raw_model_selection()
        {
            self.flash_warning(self.i18n.text("settings-model-source-changed"));
            return true;
        }
        if self.session_is_busy(session_id) {
            self.flash_warning(self.i18n.text("flash-session-busy"));
            return false;
        }
        match value {
            "local" => {
                if let Err(error) = self.open_session_model_mode_overlay_with_source(step, true) {
                    self.flash_warning(error);
                }
                true
            }
            "inherit" => {
                let previous = self.run_options.clone();
                self.run_options.raw_selection = Some(selected);
                self.run_options.apply_model_mode_input(step, "");
                if !self.persist_current_session_model_stack(previous.clone()) {
                    self.run_options = previous;
                    return false;
                }
                true
            }
            _ => false,
        }
    }

    pub(crate) fn open_session_model_source(&mut self) {
        let Some(session_id) = self.transcript.session_id else {
            return;
        };
        if self.session_is_busy(session_id) {
            self.flash_warning(self.i18n.text("flash-session-busy"));
            return;
        }
        let parent = self
            .transcript
            .execution
            .as_ref()
            .map(|execution| model_selection_only(&execution.execution.inherited_selection));
        let selected = model_selection_only(&self.current_raw_model_selection());
        let picker = self.build_session_model_source_picker(session_id, selected, parent);
        self.open_choice_overlay(picker);
    }

    pub(crate) fn build_session_model_source_picker(
        &self,
        session_id: i64,
        selected: ExecutionSelection,
        parent: Option<ExecutionSelection>,
    ) -> crate::ChoiceOverlay {
        let local = session_model_route_is_local(&selected);
        let local_value = parent
            .as_ref()
            .map(|parent| parent.overlay_with_cascade(&selected))
            .unwrap_or_else(|| selected.clone());
        self.build_choice_overlay(
            format!(
                "{} · {}",
                self.i18n.text("overlay-session-model-title"),
                self.i18n.text("settings-layer-source-title")
            ),
            format!(
                "{}\n{}",
                self.i18n.text("settings-layer-source-description"),
                self.i18n.text("settings-layer-group-model")
            ),
            Some(if local { "local" } else { "inherit" }.into()),
            super::layers::source_items(
                &self.i18n,
                true,
                format!(
                    "{} · {}",
                    self.i18n.text("settings-layer-parent-config"),
                    parent
                        .as_ref()
                        .map(|parent| selection_summary(&self.i18n, parent))
                        .unwrap_or_else(|| self.i18n.text("settings-source-unavailable"))
                ),
                selection_summary(&self.i18n, &local_value),
            ),
            ChoiceOverlayAction::SessionModelSource {
                session_id,
                model: self.current_session_model_ref(),
                selected,
                local_value,
            },
            false,
            agena_tui::choice::ChoicePresentationStyle::SelectOnly,
        )
    }

    pub(crate) fn commit_session_model_source(
        &mut self,
        session_id: i64,
        model: Option<ModelRef>,
        selected: ExecutionSelection,
        local_value: ExecutionSelection,
        value: &str,
    ) -> bool {
        if !model_source_context_matches(
            Some(session_id),
            self.transcript.session_id,
            &model,
            &self.current_session_model_ref(),
        ) || selected != model_selection_only(&self.current_raw_model_selection())
        {
            self.flash_warning(self.i18n.text("settings-model-source-changed"));
            return true;
        }
        if self.session_is_busy(session_id) {
            self.flash_warning(self.i18n.text("flash-session-busy"));
            return false;
        }
        let selection = match value {
            "inherit" => ExecutionSelection::default(),
            "local"
                if session_model_route_is_local(&local_value)
                    && session_model_selection_request(&local_value)
                        .is_some_and(|request| request.model.is_some()) =>
            {
                local_value
            }
            "local" => {
                self.open_model_chooser_with_source(
                    agena_tui::model_chooser::SessionModelChooserPurpose::RuntimeOverride,
                    true,
                );
                return true;
            }
            _ => return false,
        };
        let previous = self.run_options.clone();
        self.run_options.raw_selection = Some(selection);
        self.run_options.pending_mode_clears = [false; 3];
        self.run_options.explicit_model_stack = true;
        if !self.persist_current_session_model_stack(previous.clone()) {
            self.run_options = previous;
            return false;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn selection(value: serde_json::Value) -> ExecutionSelection {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn session_route_and_modes_have_independent_inherited_editability() {
        let parent = selection(
            json!({"provider":"p","model":"m","thinking_mode":"medium","verbosity":"low","parallel_tool_calls":false}),
        );
        let raw = selection(json!({"thinking_mode":"high"}));
        assert!(!session_model_route_is_local(&raw));
        assert!(session_model_mode_is_local(
            &raw,
            SessionModelModeStep::ThinkingMode
        ));
        assert!(!session_model_mode_is_local(
            &raw,
            SessionModelModeStep::SpeedMode
        ));
        assert!(!session_model_mode_is_local(
            &raw,
            SessionModelModeStep::Verbosity
        ));
        let pinned = parent.overlay_with_cascade(&raw);
        assert!(session_model_route_is_local(&pinned));
        assert_eq!(pinned.thinking_mode.as_deref(), Some("high"));
        assert_eq!(pinned.verbosity.as_deref(), Some("low"));
        assert_eq!(pinned.parallel_tool_calls, Some(false));
        let route = selection(json!({"provider":"own","model":"m"}));
        assert!(session_model_mode_is_local(
            &route,
            SessionModelModeStep::SpeedMode
        ));
        assert!(
            inherited_session_model_mode(&route, &parent, SessionModelModeStep::ThinkingMode)
                .is_none()
        );
    }

    #[test]
    fn inherited_session_mode_opens_source_and_rejects_direct_value_commit() {
        let temp = tempfile::tempdir().unwrap();
        let backend = crate::TuiBackend::isolated_test_backend(temp.path().to_path_buf());
        let mut app =
            App::new_with_backend(backend, crate::LaunchOptions::default(), I18n::english());
        app.transcript.session_id = Some(10);
        app.run_options.raw_selection = Some(selection(json!({"thinking_mode":"high"})));
        assert!(
            app.open_session_model_mode_overlay(SessionModelModeStep::Verbosity)
                .unwrap()
        );
        assert!(matches!(
            app.overlay,
            Some(crate::Overlay::Choice(crate::ChoiceOverlay {
                action: ChoiceOverlayAction::SessionModelModeSource { .. },
                ..
            }))
        ));
        let Some(crate::Overlay::Choice(mut picker)) = app.overlay.take() else {
            panic!("source chooser missing");
        };
        assert!(picker.presentation.prompt.contains("Parent configuration"));
        picker.action = ChoiceOverlayAction::SessionModelMode {
            step: SessionModelModeStep::Verbosity,
            session_id: Some(10),
            model: app.current_session_model_ref(),
            explicit_local: false,
        };
        assert!(!app.commit_choice_overlay(
            &mut picker,
            agena_tui::choice::ChoiceSelection::Item {
                value: "low".into()
            }
        ));
        assert_eq!(app.current_raw_model_selection().verbosity, None);
        assert_eq!(
            app.current_raw_model_selection().thinking_mode.as_deref(),
            Some("high")
        );
    }

    #[test]
    fn reselecting_current_model_preserves_modes_and_new_model_keeps_parallel_false() {
        let temp = tempfile::tempdir().unwrap();
        let backend = crate::TuiBackend::isolated_test_backend(temp.path().to_path_buf());
        let mut app =
            App::new_with_backend(backend, crate::LaunchOptions::default(), I18n::english());
        let model = ModelRef::new("p", "same");
        app.run_options.replace_model_stack(
            Some(model.clone()),
            Some("high".into()),
            Some("fast".into()),
            Some("low".into()),
            Some(false),
        );
        assert!(app.apply_model_override_from_source(model, false));
        let same = app.current_raw_model_selection();
        assert_eq!(same.thinking_mode.as_deref(), Some("high"));
        assert_eq!(same.speed_mode.as_deref(), Some("fast"));
        assert_eq!(same.verbosity.as_deref(), Some("low"));
        assert_eq!(same.parallel_tool_calls, Some(false));
        assert!(app.apply_model_override_from_source(ModelRef::new("p", "next"), false));
        let next = app.current_raw_model_selection();
        assert!(
            next.thinking_mode.is_none() && next.speed_mode.is_none() && next.verbosity.is_none()
        );
        assert_eq!(next.parallel_tool_calls, Some(false));
    }

    #[test]
    fn unknown_parent_source_can_open_explicit_local_model_draft_without_copying_values() {
        let temp = tempfile::tempdir().unwrap();
        let backend = crate::TuiBackend::isolated_test_backend(temp.path().to_path_buf());
        let mut app =
            App::new_with_backend(backend, crate::LaunchOptions::default(), I18n::english());
        app.transcript.session_id = Some(10);
        let raw = selection(json!({"thinking_mode":"high","parallel_tool_calls":false}));
        app.run_options.raw_selection = Some(raw.clone());
        app.open_session_model_source();
        let Some(crate::Overlay::Choice(mut picker)) = app.overlay.take() else {
            panic!("unknown parent still needs a source chooser");
        };
        assert!(
            picker
                .presentation
                .items
                .iter()
                .any(|item| item.label.contains("Unavailable"))
        );
        assert!(app.commit_choice_overlay(
            &mut picker,
            agena_tui::choice::ChoiceSelection::Item {
                value: "local".into()
            }
        ));
        let crate::Route::SessionModelChooser(dialog) = &app.current_route else {
            panic!("explicit local model draft missing");
        };
        assert!(dialog.meta.editable && dialog.meta.explicit_local);
        assert_eq!(app.current_raw_model_selection(), raw);
    }

    #[test]
    fn session_source_pin_keeps_complete_parent_and_false_then_clear_is_empty_request() {
        let parent = selection(
            json!({"provider":"p","adapter":"a","model":"m","thinking_mode":"high","speed_mode":"fast","verbosity":"low","parallel_tool_calls":false}),
        );
        let request = session_model_selection_request(&parent).unwrap();
        assert_eq!(request.model.as_ref().unwrap().model_id, "m");
        assert_eq!(
            request.model.as_ref().unwrap().adapter_id.as_deref(),
            Some("a")
        );
        assert_eq!(request.thinking_mode.as_deref(), Some("high"));
        assert_eq!(request.speed_mode.as_deref(), Some("fast"));
        assert_eq!(request.verbosity.as_deref(), Some("low"));
        assert_eq!(request.parallel_tool_calls, Some(false));
        let clear = session_model_selection_request(&ExecutionSelection::default()).unwrap();
        assert!(
            clear.model.is_none()
                && clear.thinking_mode.is_none()
                && clear.speed_mode.is_none()
                && clear.verbosity.is_none()
                && clear.parallel_tool_calls.is_none()
        );
        assert!(
            session_model_selection_request(&selection(json!({"provider":"partial"}))).is_none()
        );
    }

    #[test]
    fn session_mode_only_edit_does_not_pin_effective_identity_or_unrelated_modes() {
        let mut state = crate::RunOptionsState::default();
        state.replace_model_stack(
            Some(ModelRef::new("effective", "m")),
            Some("low".into()),
            Some("fast".into()),
            Some("high".into()),
            Some(true),
        );
        state.raw_selection = Some(ExecutionSelection::default());
        state.mark_model_stack_resolved();
        state.apply_model_mode_input(SessionModelModeStep::ThinkingMode, "high");
        let request = state.model_stack_request().unwrap();
        assert_eq!(request.thinking_mode.as_deref(), Some("high"));
        assert!(request.model.is_none());
        assert!(
            request.speed_mode.is_none()
                && request.verbosity.is_none()
                && request.parallel_tool_calls.is_none()
        );
        assert_eq!(
            state.model.as_ref().unwrap().provider_id.as_ref(),
            "effective"
        );
        assert!(state.submission_request().model.is_none());
        state.apply_model_mode_input(SessionModelModeStep::ThinkingMode, "");
        assert!(!session_model_has_override(&state.raw_model_selection()));
    }

    #[test]
    fn session_mode_pending_snapshot_preserves_other_raw_modes_and_parallel_false() {
        let mut state = crate::RunOptionsState::default();
        state.raw_selection = Some(selection(
            json!({"speed_mode":"slow","parallel_tool_calls":false}),
        ));
        state.apply_model_mode_input(SessionModelModeStep::ThinkingMode, "high");
        state.apply_model_mode_input(SessionModelModeStep::Verbosity, "low");
        let request = state.model_stack_request().unwrap();
        assert_eq!(request.thinking_mode.as_deref(), Some("high"));
        assert_eq!(request.speed_mode.as_deref(), Some("slow"));
        assert_eq!(request.verbosity.as_deref(), Some("low"));
        assert_eq!(request.parallel_tool_calls, Some(false));
        assert!(request.model.is_none());
    }

    #[test]
    fn session_mode_clear_is_explicit_in_a_nonempty_patch_and_survives_followup_modes() {
        let mut state = crate::RunOptionsState::default();
        state.raw_selection = Some(selection(
            json!({"thinking_mode":"high","parallel_tool_calls":false}),
        ));
        state.apply_model_mode_input(SessionModelModeStep::ThinkingMode, "");
        let cleared = state.model_stack_request().unwrap();
        assert_eq!(cleared.thinking_mode.as_deref(), Some(""));
        assert_eq!(cleared.parallel_tool_calls, Some(false));
        assert_eq!(state.raw_model_selection().thinking_mode, None);
        state.apply_model_mode_input(SessionModelModeStep::Verbosity, "low");
        let followup = state.model_stack_request().unwrap();
        assert_eq!(followup.thinking_mode.as_deref(), Some(""));
        assert_eq!(followup.verbosity.as_deref(), Some("low"));
        assert_eq!(followup.parallel_tool_calls, Some(false));
        state.apply_model_mode_input(SessionModelModeStep::ThinkingMode, "medium");
        assert_eq!(
            state
                .model_stack_request()
                .unwrap()
                .thinking_mode
                .as_deref(),
            Some("medium")
        );
    }

    #[test]
    fn pending_model_source_snapshot_survives_refresh_only_in_its_own_session() {
        let temp = tempfile::tempdir().unwrap();
        let backend = crate::TuiBackend::isolated_test_backend(temp.path().to_path_buf());
        let mut app =
            App::new_with_backend(backend, crate::LaunchOptions::default(), I18n::english());
        let pending = selection(json!({"thinking_mode":"high","parallel_tool_calls":false}));
        app.run_options.raw_selection = Some(pending.clone());
        app.transcript.session_id = Some(10);
        app.pending_session_model_write = Some((10, 1));
        app.sync_run_model_stack_from_execution();
        assert_eq!(app.run_options.raw_selection, Some(pending));
        app.transcript.session_id = Some(11);
        app.sync_run_model_stack_from_execution();
        assert_eq!(app.run_options.raw_selection, None);
    }

    #[test]
    fn a_pending_session_model_write_blocks_a_second_concurrent_snapshot() {
        let temp = tempfile::tempdir().unwrap();
        let backend = crate::TuiBackend::isolated_test_backend(temp.path().to_path_buf());
        let mut app =
            App::new_with_backend(backend, crate::LaunchOptions::default(), I18n::english());
        app.transcript.session_id = Some(10);
        app.session_selection_revision = 7;
        app.pending_session_model_write = Some((10, 7));
        app.run_options.raw_selection = Some(selection(json!({"thinking_mode":"high"})));
        assert!(!app.persist_current_session_model_stack(app.run_options.clone()));
        assert_eq!(app.session_selection_revision, 7);
        assert_eq!(app.pending_session_model_write, Some((10, 7)));
    }

    #[test]
    fn session_mode_inherited_preview_respects_identity_cascade() {
        let parent = selection(json!({"provider":"p","model":"parent","thinking_mode":"low"}));
        let mode_only = selection(json!({"thinking_mode":"high"}));
        assert_eq!(
            inherited_session_model_mode(&mode_only, &parent, SessionModelModeStep::ThinkingMode)
                .as_deref(),
            Some("low")
        );
        for model in ["parent", "other"] {
            let own_model = selection(json!({"provider":"p","model":model,"thinking_mode":"high"}));
            assert_eq!(
                inherited_session_model_mode(
                    &own_model,
                    &parent,
                    SessionModelModeStep::ThinkingMode
                ),
                None
            );
        }
    }

    #[test]
    fn session_source_guard_rejects_another_session_or_changed_model() {
        let model = Some(ModelRef::new("p", "m"));
        assert!(model_source_context_matches(
            Some(10),
            Some(10),
            &model,
            &model
        ));
        assert!(!model_source_context_matches(
            Some(10),
            Some(11),
            &model,
            &model
        ));
        assert!(!model_source_context_matches(
            Some(10),
            Some(10),
            &model,
            &Some(ModelRef::new("p", "other"))
        ));
    }

    #[test]
    fn session_source_picker_keeps_route_inherited_for_mode_only_and_pins_complete_group() {
        let temp = tempfile::tempdir().unwrap();
        let backend = crate::TuiBackend::isolated_test_backend(temp.path().to_path_buf());
        let app = App::new_with_backend(backend, crate::LaunchOptions::default(), I18n::english());
        let parent = selection(
            json!({"provider":"p","model":"m","thinking_mode":"high","parallel_tool_calls":false}),
        );
        for (selected, source) in [
            (ExecutionSelection::default(), "inherit"),
            (selection(json!({"thinking_mode":"high"})), "inherit"),
        ] {
            let picker =
                app.build_session_model_source_picker(10, selected.clone(), Some(parent.clone()));
            assert_eq!(
                agena_tui::choice::selection(&picker.presentation),
                Some(agena_tui::choice::ChoiceSelection::Item {
                    value: source.into()
                })
            );
            let ChoiceOverlayAction::SessionModelSource { local_value, .. } = picker.action else {
                panic!("wrong action");
            };
            assert_eq!(local_value, parent.overlay_with_cascade(&selected));
        }
        let permissions_only = selection(json!({"permission":{"tools":{"default":"deny"}}}));
        assert!(!session_model_has_override(&permissions_only));
    }

    #[test]
    fn session_model_chooser_shortcut_opens_source_intent_and_prompt_reports_raw_source() {
        use agena_tui::model_chooser::*;
        let temp = tempfile::tempdir().unwrap();
        let backend = crate::TuiBackend::isolated_test_backend(temp.path().to_path_buf());
        let mut app =
            App::new_with_backend(backend, crate::LaunchOptions::default(), I18n::english());
        app.transcript.session_id = Some(10);
        app.run_options.replace_model_stack(
            Some(ModelRef::new("effective", "m")),
            None,
            None,
            None,
            None,
        );
        app.run_options.raw_selection = Some(ExecutionSelection::default());
        let mut picker =
            app.build_session_model_chooser_overlay(SessionModelChooserPurpose::RuntimeOverride);
        assert!(picker.prompt.contains("inherit"));
        assert!(picker.footer.contains("Ctrl+O"));
        assert_eq!(
            reduce(
                &mut picker,
                SessionModelChooserAction::Input(crossterm::event::KeyEvent::new(
                    crossterm::event::KeyCode::Char('o'),
                    crossterm::event::KeyModifiers::CONTROL
                ))
            ),
            SessionModelChooserReducerEffect::Source
        );
        app.run_options.raw_selection = Some(selection(json!({"parallel_tool_calls":false})));
        assert!(
            app.session_model_source_prompt()
                .contains("Parent configuration")
        );
        assert!(app.session_model_source_prompt().contains("inherit"));
    }
}
