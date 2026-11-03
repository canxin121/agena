//! Prompt studio: layered prompt fragments, edited without JSON.

use agena_api::resource::ConfigJsonSources;
use agena_prompt_layers::{FragmentId, LayerMode, PromptFragment, PromptLayer};
use serde::{Deserialize, Serialize};

use agena_tui_components::editor_state::{
    EditorDialogKeyResult, EditorDialogState, drive_editor_dialog_key,
};

use crate::{
    App, Editor, KeyEvent, Overlay, PromptStudioDraft, PromptStudioEditorAction,
    PromptStudioOverlay, PromptStudioScope,
};

const PROMPTS_SETTING_PATH: &str = "prompts";

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
struct PromptScopeSetting {
    mode: LayerMode,
    order: Vec<FragmentId>,
    disabled: Vec<FragmentId>,
    inline: Vec<PromptFragment>,
    models: Vec<serde_json::Value>,
}

impl PromptScopeSetting {
    fn into_draft(self) -> PromptStudioDraft {
        PromptStudioDraft {
            layer: PromptLayer {
                mode: self.mode,
                order: self.order,
                disabled: self.disabled,
                inline: self.inline,
            },
            models: self.models,
        }
    }
}

#[derive(Debug, Clone)]
struct PromptStudioEntry {
    id: FragmentId,
    inline_index: Option<usize>,
    order_index: Option<usize>,
}

impl App {
    /// Render the layered prompt editor inside the standard dialog surface.
    pub(crate) fn render_prompt_layer_list(
        &self,
        frame: &mut ratatui::Frame,
        area: ratatui::layout::Rect,
        dialog: &PromptStudioOverlay,
    ) {
        let draft = dialog.draft();
        let entries = prompt_studio_entries(draft);
        let mode = match draft.layer.mode {
            LayerMode::Append => "append to inherited layers",
            LayerMode::Replace => "replace inherited layers",
        };
        let mut body = format!(
            "Scope: {}  (Tab switches scope)\nMode: {mode}\n\n",
            dialog.scope.as_str()
        );
        if entries.is_empty() {
            body.push_str(
                "No ordered fragments yet. Press `a` to add one or `o` to order an existing id.\n",
            );
        }
        for (index, entry) in entries.iter().enumerate() {
            let marker = if index == dialog.selected { ">" } else { " " };
            let source = match entry.inline_index {
                Some(inline_index) => {
                    let fragment = &draft.layer.inline[inline_index];
                    let title = if fragment.title.trim().is_empty() {
                        String::new()
                    } else {
                        format!(" · {}", fragment.title.trim())
                    };
                    format!("inline{title} · {} bytes", fragment.body.len())
                }
                None => "library".to_owned(),
            };
            body.push_str(&format!("{marker} {}  {source}\n", entry.id));
        }
        if !draft.layer.disabled.is_empty() {
            body.push_str("\nHidden inherited fragments:\n");
            for id in &draft.layer.disabled {
                body.push_str(&format!("- {id}\n"));
            }
        }
        if !draft.models.is_empty() {
            body.push_str(&format!(
                "\n{} model-scoped overlay(s) are preserved.\n",
                draft.models.len()
            ));
        }
        if let Some(preview) = &dialog.preview {
            body.push_str(&format!(
                "\nResolved preview (saved server state; unsaved draft edits are excluded):\n{} bytes, {} fragment(s), {} warning(s)\n",
                preview.total_bytes,
                preview.fragments.len(),
                preview.warnings.len()
            ));
            if preview.over_budget {
                body.push_str("WARNING: the resolved prompt is over budget.\n");
            }
            let preview_text = preview.text.chars().take(4_000).collect::<String>();
            body.push_str(&preview_text);
            if preview.text.chars().count() > 4_000 {
                body.push_str("\n… preview truncated in this dialog …");
            }
            body.push('\n');
        }
        if !dialog.status.is_empty() {
            body.push_str(&format!("\n{}\n", dialog.status));
        }
        agena_tui_components::render_workbench_editor_dialog(
            frame,
            area,
            agena_tui_components::SurfaceMode::Overlay,
            "Prompt layers".to_owned().into(),
            body.into(),
            "Tab scope · a add · e body · t title · d delete · Ctrl+Up/Down order · o order id · h hide id · u unhide · i rename · m mode · p preview(saved) · x clear · Ctrl+S save · Esc close".into(),
            true,
            &Editor::from_text(String::new()),
        );
    }

    pub(crate) fn open_prompt_studio(&mut self, session_id: i64) {
        self.dispatch_backend_operation(
            move |application| async move {
                let sources = application.refresh_config_sources().await?;
                let state = application.get_session_state(session_id).await?;
                let result: anyhow::Result<_> = Ok((sources, state));
                result
            },
            move |app, result| match result {
                Ok((sources, state)) => {
                    let global =
                        match prompt_draft_from_sources(&sources, PromptStudioScope::Global) {
                            Ok(draft) => draft,
                            Err(error) => {
                                app.flash_error(crate::UiFailure::from_backend(error));
                                return;
                            }
                        };
                    let workspace =
                        match prompt_draft_from_sources(&sources, PromptStudioScope::Workspace) {
                            Ok(draft) => draft,
                            Err(error) => {
                                app.flash_error(crate::UiFailure::from_backend(error));
                                return;
                            }
                        };
                    app.overlay = Some(Overlay::SessionPromptEdit(PromptStudioOverlay {
                        session_id,
                        scope: PromptStudioScope::Session,
                        global,
                        workspace,
                        session: PromptStudioDraft {
                            layer: state.execution.prompts,
                            models: Vec::new(),
                        },
                        selected: 0,
                        editor: None,
                        status: String::new(),
                        preview: None,
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
        if dialog.editor.is_some() {
            let result = drive_editor_dialog_key(
                dialog.editor.as_mut().expect("checked prompt editor"),
                key,
            );
            match result {
                EditorDialogKeyResult::Close => {
                    dialog.editor = None;
                }
                EditorDialogKeyResult::Submit(action, text) => {
                    dialog.editor = None;
                    apply_prompt_editor_submission(dialog, action, text);
                }
                EditorDialogKeyResult::Continue => {}
            }
            return false;
        }
        let ctrl = key
            .modifiers
            .contains(crossterm::event::KeyModifiers::CONTROL);
        if ctrl && key.code == crossterm::event::KeyCode::Char('s') {
            self.save_prompt_studio(dialog);
            return true;
        }
        if ctrl && key.code == crossterm::event::KeyCode::Up {
            move_selected_prompt_order(dialog, -1);
            return false;
        }
        if ctrl && key.code == crossterm::event::KeyCode::Down {
            move_selected_prompt_order(dialog, 1);
            return false;
        }
        match key.code {
            crossterm::event::KeyCode::Esc => true,
            crossterm::event::KeyCode::Tab => {
                dialog.scope = match dialog.scope {
                    PromptStudioScope::Global => PromptStudioScope::Workspace,
                    PromptStudioScope::Workspace => PromptStudioScope::Session,
                    PromptStudioScope::Session => PromptStudioScope::Global,
                };
                dialog.selected = 0;
                dialog.preview = None;
                dialog.status = format!("Editing the {} layer.", dialog.scope.as_str());
                false
            }
            crossterm::event::KeyCode::Up => {
                dialog.selected = dialog.selected.saturating_sub(1);
                false
            }
            crossterm::event::KeyCode::Down => {
                let count = prompt_studio_entries(dialog.draft()).len();
                if dialog.selected + 1 < count {
                    dialog.selected += 1;
                }
                false
            }
            crossterm::event::KeyCode::Char('m') => {
                let draft = dialog.draft_mut();
                draft.layer.mode = match draft.layer.mode {
                    LayerMode::Append => LayerMode::Replace,
                    LayerMode::Replace => LayerMode::Append,
                };
                mark_prompt_edited(dialog, "Changed the layer mode.");
                false
            }
            crossterm::event::KeyCode::Char('a') => {
                let (selected, editor) = {
                    let draft = dialog.draft_mut();
                    let id = next_fragment_id(&draft.layer);
                    draft
                        .layer
                        .inline
                        .push(PromptFragment::new(id.clone(), String::new()));
                    draft.layer.order.push(id.clone());
                    (draft.layer.order.len() - 1, prompt_body_editor(draft, &id))
                };
                dialog.selected = selected;
                dialog.editor = editor;
                dialog.status.clear();
                dialog.preview = None;
                false
            }
            crossterm::event::KeyCode::Enter | crossterm::event::KeyCode::Char('e') => {
                match selected_prompt_entry(dialog) {
                    Some(entry) if entry.inline_index.is_some() => {
                        dialog.editor = prompt_body_editor(dialog.draft(), &entry.id);
                    }
                    Some(_) => {
                        dialog.status = "That ordered fragment comes from the library and has no inline body; edit its file outside this dialog, or press `d` to remove it from this layer.".to_owned();
                    }
                    None => {}
                }
                false
            }
            crossterm::event::KeyCode::Char('t') => {
                match selected_prompt_entry(dialog) {
                    Some(entry) if entry.inline_index.is_some() => {
                        dialog.editor = prompt_title_editor(dialog.draft(), &entry.id);
                    }
                    Some(_) => {
                        dialog.status = "That ordered fragment comes from the library and has no inline title; edit its file outside this dialog, or press `d` to remove it from this layer.".to_owned();
                    }
                    None => {}
                }
                false
            }
            crossterm::event::KeyCode::Char('i') => {
                if let Some(entry) = selected_prompt_entry(dialog)
                    && entry.inline_index.is_some()
                {
                    dialog.editor = Some(prompt_line_editor(
                        format!("Rename fragment `{}`", entry.id),
                        "New fragment id. Lowercase letters, digits, dots, underscores, or hyphens.".to_owned(),
                        entry.id.to_string(),
                        PromptStudioEditorAction::Rename(entry.id.to_string()),
                    ));
                }
                false
            }
            crossterm::event::KeyCode::Char('d') => {
                if let Some(entry) = selected_prompt_entry(dialog) {
                    let draft = dialog.draft_mut();
                    remove_prompt_fragment(draft, &entry.id);
                    let count = prompt_studio_entries(draft).len();
                    dialog.selected = dialog.selected.min(count.saturating_sub(1));
                    dialog.status = format!("Removed `{}` from this layer.", entry.id);
                    dialog.preview = None;
                }
                false
            }
            crossterm::event::KeyCode::Char('o') => {
                dialog.editor = Some(prompt_line_editor(
                    "Order a fragment".to_owned(),
                    "Fragment id to append to this layer's order.".to_owned(),
                    String::new(),
                    PromptStudioEditorAction::AddOrder,
                ));
                false
            }
            crossterm::event::KeyCode::Char('h') => {
                dialog.editor = Some(prompt_line_editor(
                    "Hide inherited fragment".to_owned(),
                    "Fragment id inherited from a broader scope.".to_owned(),
                    String::new(),
                    PromptStudioEditorAction::AddDisabled,
                ));
                false
            }
            crossterm::event::KeyCode::Char('u') => {
                dialog.editor = Some(prompt_line_editor(
                    "Unhide inherited fragment".to_owned(),
                    "Fragment id currently listed as hidden.".to_owned(),
                    String::new(),
                    PromptStudioEditorAction::EnableDisabled,
                ));
                false
            }
            crossterm::event::KeyCode::Char('p') => {
                self.load_prompt_preview(dialog.session_id);
                false
            }
            crossterm::event::KeyCode::Char('x') => {
                dialog.draft_mut().layer = PromptLayer::default();
                dialog.selected = 0;
                dialog.preview = None;
                dialog.status = "Cleared the top-level layer in the draft; model-scoped overlays are kept. Ctrl+S saves (an empty append layer with no overlays is deleted).".to_owned();
                false
            }
            _ => false,
        }
    }

    fn load_prompt_preview(&mut self, session_id: i64) {
        self.dispatch_backend_operation(
            move |application| async move {
                application.session_prompt_preview(session_id).await
            },
            |app, result| match result {
                Ok(preview) => {
                    if let Some(Overlay::SessionPromptEdit(dialog)) = &mut app.overlay {
                        dialog.preview = Some(preview);
                        dialog.status = "Preview loaded from the saved server session layers; unsaved draft edits are not included.".to_owned();
                    }
                }
                Err(error) => app.flash_error(error),
            },
        );
    }

    fn save_prompt_studio(&mut self, dialog: &mut PromptStudioOverlay) {
        let draft = dialog.draft().clone();
        if let Err(error) = draft.layer.validate() {
            dialog.status = format!("Invalid prompt layer: {error}");
            return;
        }
        let scope = dialog.scope;
        let session_id = dialog.session_id;
        let empty = draft.is_empty();
        let setting = prompt_setting_value(&draft);
        let layer = draft.layer;
        dialog.status = "Saving prompt layers…".to_owned();
        self.dispatch_backend_operation(
            move |application| async move {
                match scope {
                    PromptStudioScope::Session => {
                        application
                            .set_session_prompts(session_id, Some(layer))
                            .await?;
                    }
                    PromptStudioScope::Global => {
                        if empty {
                            application
                                .delete_config_setting(PROMPTS_SETTING_PATH)
                                .await?;
                        } else {
                            application
                                .set_config_setting(PROMPTS_SETTING_PATH, setting)
                                .await?;
                        }
                    }
                    PromptStudioScope::Workspace => {
                        if empty {
                            application
                                .delete_workspace_config_setting(PROMPTS_SETTING_PATH)
                                .await?;
                        } else {
                            application
                                .set_workspace_config_setting(PROMPTS_SETTING_PATH, setting)
                                .await?;
                        }
                    }
                }
                Ok::<(), anyhow::Error>(())
            },
            move |app, result| match result {
                Ok(()) => {
                    if let Some(Overlay::SessionPromptEdit(dialog)) = &mut app.overlay {
                        dialog.status = format!("Saved the {} prompt layer.", scope.as_str());
                        dialog.preview = None;
                    }
                    app.flash_success(format!("Saved the {} prompt layer.", scope.as_str()));
                }
                Err(error) => app.flash_error(error),
            },
        );
    }
}

fn prompt_studio_entries(draft: &PromptStudioDraft) -> Vec<PromptStudioEntry> {
    let mut entries = draft
        .layer
        .order
        .iter()
        .enumerate()
        .map(|(order_index, id)| PromptStudioEntry {
            id: id.clone(),
            inline_index: draft
                .layer
                .inline
                .iter()
                .position(|fragment| &fragment.id == id),
            order_index: Some(order_index),
        })
        .collect::<Vec<_>>();
    for (inline_index, fragment) in draft.layer.inline.iter().enumerate() {
        if !draft.layer.order.contains(&fragment.id) {
            entries.push(PromptStudioEntry {
                id: fragment.id.clone(),
                inline_index: Some(inline_index),
                order_index: None,
            });
        }
    }
    entries
}

fn selected_prompt_entry(dialog: &PromptStudioOverlay) -> Option<PromptStudioEntry> {
    prompt_studio_entries(dialog.draft())
        .get(dialog.selected)
        .cloned()
}

fn mark_prompt_edited(dialog: &mut PromptStudioOverlay, status: &str) {
    dialog.status = status.to_owned();
    dialog.preview = None;
}

fn move_selected_prompt_order(dialog: &mut PromptStudioOverlay, delta: isize) {
    let Some(entry) = selected_prompt_entry(dialog) else {
        return;
    };
    let Some(current) = entry.order_index else {
        dialog.status =
            "The selected inline fragment is not in the order yet; press `o` to add it.".to_owned();
        return;
    };
    let draft = dialog.draft_mut();
    let destination = current.saturating_add_signed(delta);
    if destination >= draft.layer.order.len() || destination == current {
        return;
    }
    draft.layer.order.swap(current, destination);
    dialog.selected = destination;
    mark_prompt_edited(dialog, "Moved the fragment in the order.");
}

fn apply_prompt_editor_submission(
    dialog: &mut PromptStudioOverlay,
    action: PromptStudioEditorAction,
    text: String,
) {
    match action {
        PromptStudioEditorAction::Body(id) => {
            let Ok(id) = FragmentId::parse(&id) else {
                dialog.status = "The fragment id is no longer valid.".to_owned();
                return;
            };
            let Some(fragment) = dialog
                .draft_mut()
                .layer
                .inline
                .iter_mut()
                .find(|fragment| fragment.id == id)
            else {
                dialog.status = "The inline fragment no longer exists.".to_owned();
                return;
            };
            fragment.body = text;
            mark_prompt_edited(dialog, "Updated the fragment body.");
        }
        PromptStudioEditorAction::Title(id) => {
            let Ok(id) = FragmentId::parse(&id) else {
                dialog.status = "The fragment id is no longer valid.".to_owned();
                return;
            };
            let Some(fragment) = dialog
                .draft_mut()
                .layer
                .inline
                .iter_mut()
                .find(|fragment| fragment.id == id)
            else {
                dialog.status = "The inline fragment no longer exists.".to_owned();
                return;
            };
            fragment.title = text.trim().to_owned();
            mark_prompt_edited(dialog, "Updated the fragment title.");
        }
        PromptStudioEditorAction::Rename(old_id) => {
            let old_id = match FragmentId::parse(&old_id) {
                Ok(id) => id,
                Err(error) => {
                    dialog.status = error.to_string();
                    return;
                }
            };
            let new_id = match FragmentId::parse(text.trim()) {
                Ok(id) => id,
                Err(error) => {
                    dialog.status = error.to_string();
                    return;
                }
            };
            match rename_prompt_fragment(dialog.draft_mut(), &old_id, new_id.clone()) {
                Ok(()) => mark_prompt_edited(dialog, &format!("Renamed to `{new_id}`.")),
                Err(error) => dialog.status = error,
            }
        }
        PromptStudioEditorAction::AddOrder => match FragmentId::parse(text.trim()) {
            Ok(id) => {
                let draft = dialog.draft_mut();
                if draft.layer.order.contains(&id) {
                    dialog.status = format!("`{id}` is already in the order.");
                    return;
                }
                draft
                    .layer
                    .disabled
                    .retain(|fragment_id| fragment_id != &id);
                draft.layer.order.push(id.clone());
                dialog.selected = draft.layer.order.len() - 1;
                mark_prompt_edited(dialog, &format!("Added `{id}` to the order."));
            }
            Err(error) => dialog.status = error.to_string(),
        },
        PromptStudioEditorAction::AddDisabled => match FragmentId::parse(text.trim()) {
            Ok(id) => {
                let draft = dialog.draft_mut();
                if draft.layer.order.contains(&id)
                    || draft.layer.inline.iter().any(|fragment| fragment.id == id)
                {
                    dialog.status = format!(
                        "`{id}` is defined in this layer; remove or edit it instead of hiding it."
                    );
                    return;
                }
                if !draft.layer.disabled.contains(&id) {
                    draft.layer.disabled.push(id.clone());
                }
                mark_prompt_edited(dialog, &format!("Hid inherited fragment `{id}`."));
            }
            Err(error) => dialog.status = error.to_string(),
        },
        PromptStudioEditorAction::EnableDisabled => match FragmentId::parse(text.trim()) {
            Ok(id) => {
                let draft = dialog.draft_mut();
                let before = draft.layer.disabled.len();
                draft
                    .layer
                    .disabled
                    .retain(|fragment_id| fragment_id != &id);
                if draft.layer.disabled.len() == before {
                    dialog.status = format!("`{id}` is not currently hidden.");
                } else {
                    mark_prompt_edited(dialog, &format!("Unhid `{id}`."));
                }
            }
            Err(error) => dialog.status = error.to_string(),
        },
    }
}

fn rename_prompt_fragment(
    draft: &mut PromptStudioDraft,
    old_id: &FragmentId,
    new_id: FragmentId,
) -> Result<(), String> {
    if old_id == &new_id {
        return Ok(());
    }
    if draft.layer.order.contains(&new_id)
        || draft.layer.disabled.contains(&new_id)
        || draft
            .layer
            .inline
            .iter()
            .any(|fragment| fragment.id == new_id)
    {
        return Err(format!(
            "Fragment id `{new_id}` is already used in this layer."
        ));
    }
    let Some(fragment) = draft
        .layer
        .inline
        .iter_mut()
        .find(|fragment| &fragment.id == old_id)
    else {
        return Err(format!("Inline fragment `{old_id}` no longer exists."));
    };
    fragment.id = new_id.clone();
    for id in &mut draft.layer.order {
        if id == old_id {
            *id = new_id.clone();
        }
    }
    for id in &mut draft.layer.disabled {
        if id == old_id {
            *id = new_id.clone();
        }
    }
    transform_model_references(&mut draft.models, old_id, Some(&new_id));
    Ok(())
}

fn remove_prompt_fragment(draft: &mut PromptStudioDraft, id: &FragmentId) {
    draft.layer.inline.retain(|fragment| &fragment.id != id);
    draft.layer.order.retain(|fragment_id| fragment_id != id);
    draft.layer.disabled.retain(|fragment_id| fragment_id != id);
    transform_model_references(&mut draft.models, id, None);
}

fn transform_model_references(
    models: &mut [serde_json::Value],
    old_id: &FragmentId,
    new_id: Option<&FragmentId>,
) {
    for model in models {
        let Some(model) = model.as_object_mut() else {
            continue;
        };
        if model
            .get("inline")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|inline| {
                inline.iter().any(|fragment| {
                    fragment.get("id").and_then(serde_json::Value::as_str) == Some(old_id.as_str())
                })
            })
        {
            continue;
        }
        for field in ["order", "disabled"] {
            let Some(ids) = model
                .get_mut(field)
                .and_then(serde_json::Value::as_array_mut)
            else {
                continue;
            };
            ids.retain_mut(|value| {
                if value.as_str() != Some(old_id.as_str()) {
                    return true;
                }
                match new_id {
                    Some(new_id) => {
                        *value = serde_json::Value::String(new_id.to_string());
                        true
                    }
                    None => false,
                }
            });
        }
    }
}

fn prompt_draft_from_sources(
    sources: &ConfigJsonSources,
    scope: PromptStudioScope,
) -> anyhow::Result<PromptStudioDraft> {
    let root = match scope {
        PromptStudioScope::Global => &sources.file,
        PromptStudioScope::Workspace => &sources.project_file,
        PromptStudioScope::Session => {
            anyhow::bail!("session prompt layers are loaded from the session state")
        }
    };
    let value = root
        .get(PROMPTS_SETTING_PATH)
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    if value.is_null() {
        return Ok(PromptStudioDraft::default());
    }
    let setting: PromptScopeSetting = serde_json::from_value(value).map_err(|error| {
        anyhow::anyhow!(
            "invalid {} prompt layer configuration: {error}",
            scope.as_str()
        )
    })?;
    Ok(setting.into_draft())
}

fn prompt_setting_value(draft: &PromptStudioDraft) -> serde_json::Value {
    let setting = PromptScopeSetting {
        mode: draft.layer.mode,
        order: draft.layer.order.clone(),
        disabled: draft.layer.disabled.clone(),
        inline: draft.layer.inline.clone(),
        models: draft.models.clone(),
    };
    serde_json::to_value(setting).expect("prompt scope settings always serialize")
}

fn next_fragment_id(layer: &PromptLayer) -> FragmentId {
    for index in 1..=999 {
        let candidate = format!("fragment-{index}");
        if let Ok(id) = FragmentId::parse(&candidate)
            && !layer.inline.iter().any(|fragment| fragment.id == id)
            && !layer.order.contains(&id)
            && !layer.disabled.contains(&id)
        {
            return id;
        }
    }
    FragmentId::parse("fragment-extra").unwrap_or_else(|_| unreachable!())
}

fn prompt_body_editor(
    draft: &PromptStudioDraft,
    id: &FragmentId,
) -> Option<EditorDialogState<PromptStudioEditorAction>> {
    let fragment = draft
        .layer
        .inline
        .iter()
        .find(|fragment| &fragment.id == id)?;
    Some(EditorDialogState::new(
        format!("Fragment `{}`", fragment.id),
        "Markdown instructions for this scope. Ctrl+S save, Esc cancel.".to_owned(),
        "Ctrl+S save · Enter newline · Esc cancel".to_owned(),
        true,
        Editor::from_text(fragment.body.clone()),
        PromptStudioEditorAction::Body(fragment.id.to_string()),
    ))
}

fn prompt_title_editor(
    draft: &PromptStudioDraft,
    id: &FragmentId,
) -> Option<EditorDialogState<PromptStudioEditorAction>> {
    let fragment = draft
        .layer
        .inline
        .iter()
        .find(|fragment| &fragment.id == id)?;
    Some(prompt_line_editor(
        format!("Title for fragment `{}`", fragment.id),
        "Optional short title shown in lists and previews. Enter submit, Esc cancel.".to_owned(),
        fragment.title.clone(),
        PromptStudioEditorAction::Title(fragment.id.to_string()),
    ))
}

fn prompt_line_editor(
    title: String,
    prompt: String,
    initial: String,
    action: PromptStudioEditorAction,
) -> EditorDialogState<PromptStudioEditorAction> {
    EditorDialogState::new(
        title,
        prompt,
        "Enter submit · Esc cancel".to_owned(),
        false,
        Editor::from_text(initial),
        action,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(value: &str) -> FragmentId {
        FragmentId::parse(value).expect("test fragment id")
    }

    fn fragment(value: &str, title: &str, body: &str) -> PromptFragment {
        let mut fragment = PromptFragment::new(id(value), body);
        fragment.title = title.to_owned();
        fragment
    }

    fn draft(
        inline: Vec<PromptFragment>,
        order: &[&str],
        disabled: &[&str],
        models: Vec<serde_json::Value>,
    ) -> PromptStudioDraft {
        PromptStudioDraft {
            layer: PromptLayer {
                mode: LayerMode::Append,
                order: order.iter().map(|value| id(value)).collect(),
                disabled: disabled.iter().map(|value| id(value)).collect(),
                inline,
            },
            models,
        }
    }

    fn overlay(scope: PromptStudioScope, prompt_draft: PromptStudioDraft) -> PromptStudioOverlay {
        PromptStudioOverlay {
            session_id: 7,
            scope,
            global: PromptStudioDraft::default(),
            workspace: PromptStudioDraft::default(),
            session: prompt_draft,
            selected: 0,
            editor: None,
            status: String::new(),
            preview: None,
        }
    }

    fn order_ids(layer: &PromptLayer) -> Vec<String> {
        layer.order.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn move_reorders_and_respects_bounds() {
        let prompt_draft = draft(
            vec![
                fragment("a", "", ""),
                fragment("b", "", ""),
                fragment("c", "", ""),
            ],
            &["a", "b", "c"],
            &[],
            Vec::new(),
        );
        let mut dialog = overlay(PromptStudioScope::Session, prompt_draft);
        dialog.selected = 1;

        move_selected_prompt_order(&mut dialog, 1);
        assert_eq!(order_ids(&dialog.draft().layer), ["a", "c", "b"]);
        assert_eq!(dialog.selected, 2);
        assert!(dialog.preview.is_none());

        move_selected_prompt_order(&mut dialog, 1);
        assert_eq!(order_ids(&dialog.draft().layer), ["a", "c", "b"]);
        assert_eq!(dialog.selected, 2);

        dialog.selected = 0;
        move_selected_prompt_order(&mut dialog, -1);
        assert_eq!(order_ids(&dialog.draft().layer), ["a", "c", "b"]);
        assert_eq!(dialog.selected, 0);
    }

    #[test]
    fn rename_updates_scope_and_model_references() {
        let shadowed = serde_json::json!({
            "match": "local/*",
            "inline": [{"id": "old", "body": "private"}],
            "order": ["old"],
            "disabled": ["old"]
        });
        let models = vec![
            serde_json::json!({
                "match": "anthropic/*",
                "order": ["old", "other"],
                "disabled": ["old"]
            }),
            shadowed.clone(),
            serde_json::json!({
                "match": "other/*",
                "order": ["other"],
                "disabled": ["old"]
            }),
        ];
        let mut prompt_draft = draft(
            vec![fragment("old", "", "")],
            &["old", "other"],
            &[],
            models,
        );

        rename_prompt_fragment(&mut prompt_draft, &id("old"), id("new")).expect("rename succeeds");

        assert_eq!(prompt_draft.layer.inline[0].id, id("new"));
        assert_eq!(order_ids(&prompt_draft.layer), ["new", "other"]);
        assert_eq!(
            prompt_draft.models[0]["order"],
            serde_json::json!(["new", "other"])
        );
        assert_eq!(
            prompt_draft.models[0]["disabled"],
            serde_json::json!(["new"])
        );
        assert_eq!(
            prompt_draft.models[1], shadowed,
            "a model layer that defines the id inline must not have its own references rewritten"
        );
        assert_eq!(
            prompt_draft.models[2]["disabled"],
            serde_json::json!(["new"])
        );
    }

    #[test]
    fn rename_rejects_ids_already_used_in_the_layer() {
        let mut prompt_draft = draft(
            vec![fragment("old", "", ""), fragment("other", "", "")],
            &["old", "other"],
            &[],
            Vec::new(),
        );

        let error = rename_prompt_fragment(&mut prompt_draft, &id("old"), id("other"))
            .expect_err("duplicate ids are rejected");

        assert!(error.contains("already used"));
        assert_eq!(prompt_draft.layer.inline[0].id, id("old"));
        assert_eq!(order_ids(&prompt_draft.layer), ["old", "other"]);
    }

    #[test]
    fn remove_updates_scope_and_model_references() {
        let shadowed = serde_json::json!({
            "match": "local/*",
            "inline": [{"id": "old", "body": "private"}],
            "order": ["old"],
            "disabled": ["old"]
        });
        let mut prompt_draft = draft(
            vec![fragment("old", "", "")],
            &["old", "other"],
            &["inherited"],
            vec![
                serde_json::json!({
                    "match": "anthropic/*",
                    "order": ["old", "other"],
                    "disabled": ["old"]
                }),
                shadowed.clone(),
            ],
        );

        remove_prompt_fragment(&mut prompt_draft, &id("old"));

        assert!(prompt_draft.layer.inline.is_empty());
        assert_eq!(order_ids(&prompt_draft.layer), ["other"]);
        assert_eq!(
            prompt_draft.models[0]["order"],
            serde_json::json!(["other"])
        );
        assert_eq!(prompt_draft.models[0]["disabled"], serde_json::json!([]));
        assert_eq!(prompt_draft.models[1], shadowed);
    }

    #[test]
    fn order_and_disabled_actions_keep_the_layer_valid() {
        let prompt_draft = draft(
            vec![fragment("local", "", "")],
            &["local"],
            &["inherited"],
            Vec::new(),
        );
        let mut dialog = overlay(PromptStudioScope::Session, prompt_draft);

        apply_prompt_editor_submission(
            &mut dialog,
            PromptStudioEditorAction::AddDisabled,
            "local".to_owned(),
        );
        assert!(dialog.status.contains("defined in this layer"));
        assert!(!dialog.draft().layer.disabled.contains(&id("local")));

        apply_prompt_editor_submission(
            &mut dialog,
            PromptStudioEditorAction::AddOrder,
            "inherited".to_owned(),
        );
        assert!(dialog.draft().layer.order.contains(&id("inherited")));
        assert!(!dialog.draft().layer.disabled.contains(&id("inherited")));

        apply_prompt_editor_submission(
            &mut dialog,
            PromptStudioEditorAction::AddDisabled,
            "hidden".to_owned(),
        );
        assert!(dialog.draft().layer.disabled.contains(&id("hidden")));
        assert!(dialog.draft().layer.validate().is_ok());
    }

    #[test]
    fn title_edit_updates_fragment_and_clears_preview() {
        let prompt_draft = draft(
            vec![fragment("one", "Old", "Body")],
            &["one"],
            &[],
            Vec::new(),
        );
        let mut dialog = overlay(PromptStudioScope::Session, prompt_draft);
        dialog.preview = Some(agena_prompt_layers::PromptResolution::default());

        apply_prompt_editor_submission(
            &mut dialog,
            PromptStudioEditorAction::Title("one".to_owned()),
            "  New title  ".to_owned(),
        );

        assert_eq!(dialog.draft().layer.inline[0].title, "New title");
        assert!(dialog.preview.is_none());
        assert_eq!(dialog.status, "Updated the fragment title.");
    }

    #[test]
    fn empty_draft_detection_covers_mode_and_model_overlays() {
        assert!(PromptStudioDraft::default().is_empty());

        let mut replace_only = PromptStudioDraft::default();
        replace_only.layer.mode = LayerMode::Replace;
        assert!(
            !replace_only.is_empty(),
            "an empty replace layer still suppresses inherited fragments"
        );

        let mut models_only = PromptStudioDraft::default();
        models_only
            .models
            .push(serde_json::json!({"match": "anthropic/*"}));
        assert!(!models_only.is_empty());

        let inline_only = draft(vec![fragment("one", "", "")], &[], &[], Vec::new());
        assert!(!inline_only.is_empty());
    }

    #[test]
    fn setting_round_trip_preserves_mode_and_model_overlays() {
        let models = vec![serde_json::json!({
            "match": "anthropic/*",
            "order": ["house-style"],
            "disabled": ["legacy"]
        })];
        let mut prompt_draft = draft(Vec::new(), &[], &[], models.clone());
        prompt_draft.layer.mode = LayerMode::Replace;

        let value = prompt_setting_value(&prompt_draft);
        assert_eq!(value["mode"], serde_json::json!("replace"));
        assert_eq!(value["models"], serde_json::Value::Array(models.clone()));

        let setting: PromptScopeSetting =
            serde_json::from_value(value).expect("editor output is a valid scope setting");
        let round_trip = setting.into_draft();
        assert_eq!(round_trip.layer.mode, LayerMode::Replace);
        assert_eq!(round_trip.models, models);
        assert!(!round_trip.is_empty());
    }
}
