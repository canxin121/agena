//! Prompt studio: session-scoped prompt fragments, edited without JSON.

use agena_prompt_layers::{FragmentId, LayerMode, PromptFragment, PromptLayer};

use agena_tui_components::editor_state::{
    EditorDialogKeyResult, EditorDialogState, drive_editor_dialog_key,
};

use crate::{App, Editor, KeyEvent, Overlay, PromptStudioOverlay};
impl App {
    /// Render the session layer list inside the standard dialog surface.
    pub(crate) fn render_prompt_layer_list(
        &self,
        frame: &mut ratatui::Frame,
        area: ratatui::layout::Rect,
        dialog: &PromptStudioOverlay,
    ) {
        let mode = match dialog.layer.mode {
            LayerMode::Append => "append to inherited",
            LayerMode::Replace => "replace inherited",
        };
        let mut body = format!("Mode: {mode}\n\n");
        if dialog.layer.inline.is_empty() {
            body.push_str("No session fragments yet. Press `a` to add one.\n");
        }
        for (index, fragment) in dialog.layer.inline.iter().enumerate() {
            let marker = if index == dialog.selected { ">" } else { " " };
            body.push_str(&format!(
                "{marker} {}  {} bytes\n",
                fragment.id,
                fragment.body.len()
            ));
        }
        if !dialog.status.is_empty() {
            body.push_str(&format!("\n{}\n", dialog.status));
        }
        agena_tui_components::render_workbench_editor_dialog(
            frame,
            area,
            agena_tui_components::SurfaceMode::Overlay,
            "Session prompt layers".to_owned().into(),
            body.into(),
            "a add · e edit · d delete · m mode · Ctrl+S save · Esc close"
                .to_owned()
                .into(),
            true,
            &Editor::from_text(String::new()),
        );
    }
    pub(crate) fn open_prompt_studio(&mut self, session_id: i64) {
        self.dispatch_backend_operation(
            move |application| async move { application.get_session_state(session_id).await },
            move |app, result| match result {
                Ok(state) => {
                    let layer = state.execution.prompts;
                    app.overlay = Some(Overlay::SessionPromptEdit(PromptStudioOverlay {
                        session_id,
                        layer,
                        selected: 0,
                        editor: None,
                        status: String::new(),
                    }));
                }
                Err(error) => app.flash_error(error),
            },
        );
    }

    pub(crate) fn handle_prompt_studio_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut PromptStudioOverlay,
    ) -> bool {
        if let Some(editor) = dialog.editor.as_mut() {
            return match drive_editor_dialog_key(editor, key) {
                EditorDialogKeyResult::Close => {
                    dialog.editor = None;
                    false
                }
                EditorDialogKeyResult::Submit(index, text) => {
                    dialog.editor = None;
                    if let Some(fragment) = dialog.layer.inline.get_mut(index) {
                        fragment.body = text;
                        dialog.status = format!("Updated `{}`.", fragment.id);
                    }
                    false
                }
                EditorDialogKeyResult::Continue => false,
            };
        }
        let ctrl = key
            .modifiers
            .contains(crossterm::event::KeyModifiers::CONTROL);
        if ctrl && key.code == crossterm::event::KeyCode::Char('s') {
            self.save_prompt_studio(dialog);
            return true;
        }
        match key.code {
            crossterm::event::KeyCode::Esc => true,
            crossterm::event::KeyCode::Up => {
                dialog.selected = dialog.selected.saturating_sub(1);
                false
            }
            crossterm::event::KeyCode::Down => {
                if dialog.selected + 1 < dialog.layer.inline.len() {
                    dialog.selected += 1;
                }
                false
            }
            crossterm::event::KeyCode::Char('m') => {
                dialog.layer.mode = match dialog.layer.mode {
                    LayerMode::Append => LayerMode::Replace,
                    LayerMode::Replace => LayerMode::Append,
                };
                false
            }
            crossterm::event::KeyCode::Char('a') => {
                let id = next_fragment_id(&dialog.layer);
                dialog
                    .layer
                    .inline
                    .push(PromptFragment::new(id, String::new()));
                let index = dialog.layer.inline.len() - 1;
                dialog.layer.order.push(index_id(&dialog.layer, index));
                dialog.selected = index;
                dialog.editor = Some(editor_for(&dialog.layer, index));
                false
            }
            crossterm::event::KeyCode::Enter | crossterm::event::KeyCode::Char('e') => {
                if dialog.selected < dialog.layer.inline.len() {
                    dialog.editor = Some(editor_for(&dialog.layer, dialog.selected));
                }
                false
            }
            crossterm::event::KeyCode::Char('d') => {
                if dialog.selected < dialog.layer.inline.len() {
                    let removed = dialog.layer.inline.remove(dialog.selected);
                    dialog.layer.order.retain(|id| id != &removed.id);
                    dialog.selected = dialog.selected.saturating_sub(1);
                    dialog.status = format!("Removed `{}`.", removed.id);
                }
                false
            }
            _ => false,
        }
    }

    fn save_prompt_studio(&mut self, dialog: &PromptStudioOverlay) {
        let session_id = dialog.session_id;
        let layer = dialog.layer.clone();
        self.dispatch_backend_operation(
            move |application| async move {
                application
                    .set_session_prompts(session_id, Some(layer))
                    .await
            },
            |app, result| match result {
                Ok(_) => app.flash_success("Session prompt layers saved."),
                Err(error) => app.flash_error(error),
            },
        );
    }
}

fn next_fragment_id(layer: &PromptLayer) -> FragmentId {
    for index in 1..=999 {
        let candidate = format!("session-{index}");
        if let Ok(id) = FragmentId::parse(&candidate)
            && !layer.inline.iter().any(|fragment| fragment.id == id)
        {
            return id;
        }
    }
    FragmentId::parse("session-extra").unwrap_or_else(|_| unreachable!())
}

fn index_id(layer: &PromptLayer, index: usize) -> FragmentId {
    layer.inline[index].id.clone()
}

fn editor_for(layer: &PromptLayer, index: usize) -> EditorDialogState<usize> {
    let fragment = &layer.inline[index];
    EditorDialogState::new(
        format!("Session fragment `{}`", fragment.id),
        "Markdown instructions for this session only. Ctrl+S save, Esc cancel.".to_owned(),
        "Ctrl+S save · Enter newline · Esc cancel".to_owned(),
        true,
        Editor::from_text(fragment.body.clone()),
        index,
    )
}
