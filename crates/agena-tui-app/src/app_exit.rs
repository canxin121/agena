//! Context-aware client exit. Leaving a TUI never implicitly stops server work.

use crate::{
    App, AppMessage, ChoiceItem, ChoiceOverlay, ChoiceOverlayAction, Overlay, RunActivityTarget,
    UiResult, ui_text,
};
use agena_domain::{CancellationOutcome, CancellationResult, ExecutionId, ExecutionPhase};
use agena_tui::choice::{self, ChoicePresentationStyle, ChoiceSelection};
use agena_tui::i18n::I18n;
use agena_tui::main_focus::Focus;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[cfg(test)]
mod http_tests;
#[cfg(test)]
mod tests;

/// Captured when the menu opens; never retarget an open menu to a newer run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ExitCancelTarget {
    pub(crate) session_id: i64,
    pub(crate) execution_id: ExecutionId,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ExitCancellation {
    pub(crate) target: ExitCancelTarget,
    pub(crate) request_id: uuid::Uuid,
    pub(crate) awaiting_ack: bool,
    pub(crate) quit_on_ack: bool,
}

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

fn exit_menu_items(i18n: &I18n, can_stop: bool) -> Vec<ChoiceItem> {
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
    .filter(|(value, _, _)| can_stop || *value != "stop-and-exit")
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
    /// Interrupt belongs to the topmost local interaction before the client.
    pub(crate) fn handle_interrupt(&mut self, key: KeyEvent) {
        if self.context_help.take().is_some() {
            return;
        }
        if self.exit_menu_is_open() {
            // A repeated key cannot choose an action or bypass the menu.
            self.refresh_exit_menu();
            return;
        }
        if self.overlay.is_some() {
            // Reuse each modal's Esc cleanup (including native inline editors)
            // instead of stacking a second, unrelated modal above it.
            self.handle_overlay_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
            return;
        }
        if self.btw_has_focus() {
            self.handle_btw_route_key(key);
            return;
        }
        if self.current_route_is_main() && self.focus == Focus::Composer {
            if self.handle_prompt_history_search_key(key) {
                return;
            }
            if self.interaction_editing.is_none()
                && (!self.composer.text().is_empty() || !self.composer_items.is_empty())
            {
                self.reset_prompt_history_recall();
                self.clear_composer_state();
                return;
            }
        }
        if self.current_route_is_main()
            && self.focus == Focus::Transcript
            && self.interaction_editing.is_some()
            && self
                .handle_active_interaction_action(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        {
            return;
        }
        self.open_exit_menu();
    }

    fn exit_menu_is_open(&self) -> bool {
        matches!(
            &self.overlay,
            Some(Overlay::Choice(dialog)) if matches!(dialog.action, ChoiceOverlayAction::Exit)
        )
    }

    fn current_exit_execution(&self) -> Option<(ExitCancelTarget, ExecutionPhase)> {
        if !self.current_route_is_main() {
            return None;
        }
        let session_id = self.transcript.session_id?;
        let snapshot = self.transcript.execution.as_ref()?;
        if snapshot.session.id != session_id {
            return None;
        }
        let execution = snapshot.session.state.active_execution()?;
        Some((
            ExitCancelTarget {
                session_id,
                execution_id: execution.execution_id.into(),
            },
            execution.phase,
        ))
    }

    fn current_exit_cancel_target(&self) -> Option<ExitCancelTarget> {
        let (target, phase) = self.current_exit_execution()?;
        if phase == ExecutionPhase::Cancelling
            || self
                .exit_cancellation
                .is_some_and(|request| request.awaiting_ack || request.target == target)
        {
            return None;
        }
        Some(target)
    }

    fn exit_is_stopping(&self) -> bool {
        self.exit_cancellation.is_some_and(|request| {
            request.awaiting_ack
                || self
                    .current_exit_execution()
                    .is_some_and(|(target, _)| target == request.target)
        }) || self
            .current_exit_execution()
            .is_some_and(|(_, phase)| phase == ExecutionPhase::Cancelling)
    }

    fn exit_has_unsent_input(&self) -> bool {
        !self.composer.text().is_empty()
            || !self.composer_items.is_empty()
            || self.session_composer.pending_submit.is_some()
            || self
                .session_composer
                .pending_restore_draft
                .as_ref()
                .is_some_and(|draft| !draft.is_empty())
    }

    fn exit_has_unsettled_work(&self) -> bool {
        if !self.current_route_is_main() {
            return false;
        }
        let target = self
            .transcript
            .session_id
            .map(RunActivityTarget::Session)
            .unwrap_or(RunActivityTarget::NewSession);
        if self.run_activity.is_active(target) {
            return true;
        }
        let Some(session_id) = self.transcript.session_id else {
            return false;
        };
        let Some(snapshot) = self
            .transcript
            .execution
            .as_ref()
            .filter(|snapshot| snapshot.session.id == session_id)
        else {
            // A selected session without its state is unknown, not idle.
            return true;
        };
        !snapshot.queued_messages.is_empty()
            || matches!(
                snapshot.session.state,
                agena_api::resource::SessionState::Creating
                    | agena_api::resource::SessionState::Running { .. }
                    | agena_api::resource::SessionState::AwaitingInteraction { .. }
            )
    }

    fn exit_menu_prompt_key(&self) -> &'static str {
        if self.exit_is_stopping() {
            "exit-menu-stopping"
        } else if self.exit_menu_target.is_some() {
            "exit-menu-prompt"
        } else if self.exit_has_unsent_input() {
            "exit-menu-draft-prompt"
        } else if self.exit_has_unsettled_work() {
            "exit-menu-pending-prompt"
        } else {
            "exit-menu-idle-prompt"
        }
    }

    fn exit_tui(&mut self) {
        // Preserve the existing best-effort persistence contract. A storage
        // failure must not make it impossible to leave the client; Drop also
        // attempts the final flush and records any remaining error.
        self.sync_current_draft_slot();
        self.persist_draft_store_with_feedback(true);
        self.should_quit = true;
    }

    pub(crate) fn open_exit_menu(&mut self) {
        if self.exit_menu_is_open() {
            self.refresh_exit_menu();
            return;
        }
        self.exit_menu_target = self.current_exit_cancel_target();
        if self.exit_menu_target.is_none()
            && !self.exit_is_stopping()
            && !self.exit_has_unsent_input()
            && !self.exit_has_unsettled_work()
        {
            self.exit_tui();
            return;
        }
        let dialog = self.build_choice_overlay(
            ui_text::t(&self.i18n, "exit-menu-title"),
            ui_text::t(&self.i18n, self.exit_menu_prompt_key()),
            None,
            exit_menu_items(&self.i18n, self.exit_menu_target.is_some()),
            ChoiceOverlayAction::Exit,
            false,
            ChoicePresentationStyle::SelectOnly,
        );
        self.overlay = Some(Overlay::Choice(dialog));
    }

    pub(crate) fn refresh_exit_menu(&mut self) {
        if !self.exit_menu_is_open() {
            return;
        }
        let Some(Overlay::Choice(mut dialog)) = self.overlay.take() else {
            unreachable!();
        };
        self.refresh_exit_menu_dialog(&mut dialog);
        self.overlay = Some(Overlay::Choice(dialog));
    }

    pub(crate) fn refresh_exit_menu_dialog(&mut self, dialog: &mut ChoiceOverlay) {
        if self.exit_menu_target != self.current_exit_cancel_target() {
            // Once invalidated, this menu cannot silently acquire another run.
            // Reopening is a new, deliberate choice about the current state.
            self.exit_menu_target = None;
        }
        let selected = choice::selection(&dialog.presentation);
        dialog.presentation.prompt = ui_text::t(&self.i18n, self.exit_menu_prompt_key());
        dialog
            .presentation
            .replace_items(exit_menu_items(&self.i18n, self.exit_menu_target.is_some()));
        choice::refresh(&mut dialog.presentation);
        let value = match selected {
            Some(ChoiceSelection::Item { value }) => value,
            _ => "cancel".to_owned(),
        };
        if !dialog
            .presentation
            .select_item_where(|item| item.value == value)
        {
            dialog
                .presentation
                .select_item_where(|item| item.value == "cancel");
        }
    }

    pub(crate) fn close_exit_menu(&mut self) {
        self.exit_menu_target = None;
        if let Some(request) = &mut self.exit_cancellation {
            // Return/Esc cancels the exit intention, not a cancellation that
            // has already been sent to the server.
            request.quit_on_ack = false;
        }
    }

    /// Whether the choice overlay should close.
    pub(crate) fn commit_exit_menu(&mut self, selection: &ChoiceSelection) -> bool {
        match ExitChoice::from_selection(selection) {
            ExitChoice::Detach => {
                self.exit_tui();
                true
            }
            ExitChoice::StopAndExit => {
                let target = self.exit_menu_target.take();
                if let Some(target) = target
                    && self.current_exit_cancel_target() == Some(target)
                {
                    self.request_exit_cancel(target);
                } else {
                    self.flash_warning(ui_text::t(&self.i18n, "exit-menu-target-changed"));
                }
                // Keep an actionable Exit/Return picker while waiting.
                false
            }
            ExitChoice::Cancel => {
                self.close_exit_menu();
                true
            }
        }
    }

    fn request_exit_cancel(&mut self, target: ExitCancelTarget) {
        let request_id = uuid::Uuid::new_v4();
        self.exit_cancellation = Some(ExitCancellation {
            target,
            request_id,
            awaiting_ack: true,
            quit_on_ack: true,
        });
        let application = self.application.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = tokio::time::timeout(
                std::time::Duration::from_secs(30),
                application.cancel_run(target.session_id, Some(target.execution_id)),
            )
            .await
            .unwrap_or_else(|_| {
                Err(anyhow::anyhow!(
                    "timed out waiting for cancellation acknowledgement"
                ))
            })
            .map_err(crate::UiFailure::from_backend);
            // Never retry with None: that would cancel an unrelated new run.
            let _ = tx
                .send(AppMessage::ExitRunCancelled {
                    request_id,
                    target,
                    result,
                })
                .await;
        });
    }

    pub(crate) fn handle_exit_run_cancelled(
        &mut self,
        request_id: uuid::Uuid,
        target: ExitCancelTarget,
        result: UiResult<CancellationOutcome>,
    ) {
        let Some(request) = self.exit_cancellation.filter(|request| {
            request.request_id == request_id && request.target == target && request.awaiting_ack
        }) else {
            return;
        };
        if self.transcript.session_id == Some(target.session_id) {
            self.request_refresh(target.session_id, true);
        }
        match result {
            Ok(outcome)
                if matches!(
                    outcome.result,
                    CancellationResult::CancellationRequested
                        | CancellationResult::AlreadyTerminal
                        | CancellationResult::NotFound
                ) =>
            {
                if let Some(pending) = &mut self.exit_cancellation {
                    pending.awaiting_ack = false;
                    pending.quit_on_ack = false;
                }
                // Keep the acknowledged target blocked until its state changes,
                // even if the last snapshot has not reached Cancelling yet.
                self.restore_cancelled_input(target.session_id, outcome);
                if request.quit_on_ack {
                    self.exit_tui();
                } else {
                    self.flash_info(ui_text::t(&self.i18n, "flash-run-cancelled"));
                }
            }
            Ok(_) => {
                self.exit_cancellation = None;
                self.flash_warning(ui_text::t(&self.i18n, "exit-menu-target-changed"));
            }
            Err(error) => {
                self.exit_cancellation = None;
                self.flash_warning(format!(
                    "{}: {}",
                    ui_text::t(&self.i18n, "flash-cancel-failed"),
                    error
                ));
            }
        }
    }
}
