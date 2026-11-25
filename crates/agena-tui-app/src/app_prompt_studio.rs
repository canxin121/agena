//! System prompt order, library browsing, and session previews.

use agena_prompt_layers::{BuiltinPrompt, FragmentId, LayerMode, PromptFragment, PromptLayer};
use serde::{Deserialize, Serialize};
use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet},
};

use agena_tui_components::editor_state::{
    EditorDialogKeyResult, EditorDialogState, drive_editor_dialog_key,
};

use crate::{
    App, Editor, KeyEvent, Overlay, PromptStudioDraft, PromptStudioEditorAction,
    PromptStudioOverlay, PromptStudioPickerAction, PromptStudioScope,
};

pub(crate) mod view;

const PROMPTS_SETTING_PATH: &str = "prompts";

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
struct PromptScopeSetting {
    mode: LayerMode,
    order: Vec<FragmentId>,
    disabled: Vec<FragmentId>,
    inline: Vec<PromptFragment>,
    library: Vec<PromptFragment>,
    models: Vec<serde_json::Value>,
    categories: Vec<String>,
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
            library: self.library,
            models: self.models,
            categories: self.categories,
        }
    }
}

#[derive(Debug, Clone)]
struct PromptStudioEntry {
    id: FragmentId,
    inline_index: Option<usize>,
    order_index: Option<usize>,
}

/// Workspace definitions shadow global ones. A session-local inline definition wins last.
fn prompt_library(dialog: &PromptStudioOverlay) -> BTreeMap<FragmentId, &PromptFragment> {
    let mut available: BTreeMap<_, _> = dialog
        .reusable
        .iter()
        .map(|prompt| (prompt.id.clone(), prompt))
        .collect();
    for scope in
        std::iter::once(&dialog.global).chain(dialog.has_workspace().then_some(&dialog.workspace))
    {
        for fragment in scope.library.iter().chain(&scope.layer.inline) {
            available.insert(fragment.id.clone(), fragment);
        }
    }
    for fragment in &dialog.draft().layer.inline {
        available.insert(fragment.id.clone(), fragment);
    }
    available
}

pub(crate) fn split_prompt_catalog(
    catalog: Vec<BuiltinPrompt>,
) -> (Vec<BuiltinPrompt>, Vec<PromptFragment>) {
    let (reusable, required): (Vec<_>, Vec<_>) =
        catalog.into_iter().partition(|prompt| prompt.reusable);
    (
        required,
        reusable
            .iter()
            .filter_map(BuiltinPrompt::reusable_fragment)
            .collect(),
    )
}

fn saved_prompt_choices(
    dialog: &PromptStudioOverlay,
    action: PromptStudioPickerAction,
) -> Vec<Cow<'_, PromptFragment>> {
    let available = prompt_library(dialog);
    let layer = &dialog.draft().layer;
    let mut candidates: Vec<_> = match action {
        PromptStudioPickerAction::Unhide => layer
            .disabled
            .iter()
            .map(|id| {
                available
                    .get(id)
                    .copied()
                    .map(Cow::Borrowed)
                    .unwrap_or_else(|| Cow::Owned(PromptFragment::new(id.clone(), "")))
            })
            .collect(),
        PromptStudioPickerAction::Browse => available.into_values().map(Cow::Borrowed).collect(),
        PromptStudioPickerAction::Order | PromptStudioPickerAction::Hide => {
            let blocked: BTreeSet<_> = inherited_prompt_layers(dialog)
                .flat_map(|parent| &parent.disabled)
                .collect();
            available
                .into_values()
                .filter(|fragment| {
                    !fragment.id.is_builtin()
                        && !layer.order.contains(&fragment.id)
                        && !blocked.contains(&fragment.id)
                        && (action == PromptStudioPickerAction::Order
                            || (!layer.disabled.contains(&fragment.id)
                                && !layer.inline.iter().any(|prompt| prompt.id == fragment.id)
                                && dialog.scope != PromptStudioScope::Global))
                })
                .map(Cow::Borrowed)
                .collect()
        }
    };
    candidates.sort_by_key(|prompt| (prompt_title(prompt).to_lowercase(), prompt.id.clone()));
    candidates
}

fn prompt_title(prompt: &PromptFragment) -> &str {
    if prompt.title.trim().is_empty() {
        prompt.id.as_str()
    } else {
        prompt.title.trim()
    }
}

fn apply_saved_prompt_choice(
    dialog: &mut PromptStudioOverlay,
    action: PromptStudioPickerAction,
    fragment: &PromptFragment,
) {
    let id = &fragment.id;
    if id.is_builtin() {
        return;
    }
    let draft = dialog.draft_mut();
    let layer = &mut draft.layer;
    match action {
        PromptStudioPickerAction::Browse => return,
        PromptStudioPickerAction::Order => {
            if layer.order.contains(id) {
                return;
            }
            layer.disabled.retain(|item| item != id);
            layer.order.push(id.clone());
            dialog.selected = configured_order(dialog).len().saturating_sub(1);
        }
        PromptStudioPickerAction::Hide => {
            if layer.order.contains(id) || layer.inline.iter().any(|item| &item.id == id) {
                return;
            }
            if !layer.disabled.contains(id) {
                layer.disabled.push(id.clone());
            }
        }
        PromptStudioPickerAction::Unhide => {
            layer.disabled.retain(|item| item != id);
        }
    }
    dialog.status = format!(
        "Updated the layer using saved prompt `{}`.",
        if fragment.title.is_empty() {
            id.as_str()
        } else {
            &fragment.title
        }
    );
    dialog.preview = None;
}

impl App {
    pub(crate) fn render_prompt_layer_list(
        &self,
        frame: &mut ratatui::Frame,
        area: ratatui::layout::Rect,
        dialog: &PromptStudioOverlay,
    ) {
        view::render(frame, area, dialog, &self.i18n);
    }

    pub(crate) fn open_prompt_studio(&mut self, session_id: i64) {
        self.dispatch_backend_operation(
            move |application| async move {
                let client = application.client();
                let (global, workspace, builtins, state) = tokio::try_join!(
                    client.settings_layer_value("global", PROMPTS_SETTING_PATH),
                    client.settings_layer_value("workspace", PROMPTS_SETTING_PATH),
                    client.builtin_prompt_catalog(),
                    client.get_session_state(session_id),
                )?;
                let (global, global_revision) = read_prompt_draft(&global)?;
                let (workspace, workspace_revision) = read_prompt_draft(&workspace)?;
                let session = PromptStudioDraft {
                    layer: state.execution.prompts,
                    ..Default::default()
                };
                let (builtins, reusable) = split_prompt_catalog(builtins);
                Ok::<_, anyhow::Error>(PromptStudioOverlay {
                    session_id,
                    session_has_workspace: state.session.workspace_id.is_some(),
                    builtins,
                    reusable,
                    saved: [global.clone(), workspace.clone(), session.clone()],
                    global,
                    workspace,
                    session,
                    global_revision,
                    workspace_revision,
                    ..Default::default()
                })
            },
            |app, result| match result {
                Ok(dialog) => app.overlay = Some(Overlay::SessionPromptEdit(dialog)),
                Err(error) => app.flash_error(error),
            },
        );
    }

    pub(crate) fn handle_prompt_studio_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut PromptStudioOverlay,
    ) -> bool {
        use crossterm::event::{KeyCode, KeyModifiers};

        if dialog.saving {
            return false;
        }
        if let Some(editor) = dialog.editor.as_mut() {
            match drive_editor_dialog_key(editor, key) {
                EditorDialogKeyResult::Close => dialog.editor = None,
                EditorDialogKeyResult::Submit(action, text) => {
                    dialog.editor = None;
                    apply_prompt_editor_submission(dialog, action, text);
                }
                EditorDialogKeyResult::Continue => {}
            }
            return false;
        }
        if dialog.detail_open {
            let scroll = dialog.detail_scroll.get();
            match key.code {
                KeyCode::Esc | KeyCode::Enter => dialog.detail_open = false,
                KeyCode::Up => dialog.detail_scroll.set(scroll.saturating_sub(1)),
                KeyCode::Down => dialog.detail_scroll.set(scroll.saturating_add(1)),
                KeyCode::PageUp => dialog.detail_scroll.set(scroll.saturating_sub(10)),
                KeyCode::PageDown => dialog.detail_scroll.set(scroll.saturating_add(10)),
                KeyCode::Home => dialog.detail_scroll.set(0),
                KeyCode::End => dialog.detail_scroll.set(u16::MAX),
                _ => {}
            }
            return false;
        }
        if dialog.preview.is_some() || dialog.preview_request.is_some() {
            match key.code {
                KeyCode::Esc | KeyCode::Char('p') => {
                    dialog.preview = None;
                    dialog.preview_request = None;
                }
                KeyCode::Enter if dialog.preview.is_some() => show_prompt_detail(dialog),
                code => {
                    let count = dialog
                        .preview
                        .as_ref()
                        .map_or(0, |preview| preview.fragments.len());
                    move_list_selection(code, &mut dialog.preview_selected, count);
                }
            }
            return false;
        }
        if let Some(action) = dialog.picker {
            let choices = saved_prompt_choices(dialog, action);
            let builtin_count = if action == PromptStudioPickerAction::Browse {
                dialog.builtins.len()
            } else {
                0
            };
            let count = builtin_count + choices.len();
            match key.code {
                KeyCode::Esc => dialog.picker = None,
                KeyCode::Enter if action == PromptStudioPickerAction::Browse && count > 0 => {
                    show_prompt_detail(dialog)
                }
                KeyCode::Enter => {
                    if let Some(fragment) = choices
                        .into_iter()
                        .nth(dialog.picker_selected)
                        .map(Cow::into_owned)
                    {
                        apply_saved_prompt_choice(dialog, action, &fragment);
                    }
                    dialog.picker = None;
                }
                code => move_list_selection(code, &mut dialog.picker_selected, count),
            }
            return false;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('s') => self.save_prompt_studio(dialog),
                KeyCode::Up => move_selected_prompt_order(dialog, -1),
                KeyCode::Down => move_selected_prompt_order(dialog, 1),
                _ => {}
            }
            return false;
        }
        match key.code {
            KeyCode::Esc => return true,
            KeyCode::Tab | KeyCode::BackTab => {
                let scopes = [
                    PromptStudioScope::Global,
                    PromptStudioScope::Workspace,
                    PromptStudioScope::Session,
                ];
                let offset = if key.code == KeyCode::Tab { 1 } else { 2 };
                dialog.scope = scopes[(dialog.scope_index() + offset) % scopes.len()];
                dialog.selected = 0;
                dialog.status.clear();
            }
            KeyCode::Char('[') => move_selected_prompt_order(dialog, -1),
            KeyCode::Char(']') => move_selected_prompt_order(dialog, 1),
            KeyCode::Char('m') => {
                let draft = dialog.draft_mut();
                draft.layer.mode = match draft.layer.mode {
                    LayerMode::Append => LayerMode::Replace,
                    LayerMode::Replace => LayerMode::Append,
                };
                mark_prompt_edited(dialog, "");
            }
            KeyCode::Char('a') => {
                let id = next_fragment_id(dialog);
                dialog.editor = Some(inline_body_editor(
                    &id,
                    "",
                    PromptStudioEditorAction::Add(id.clone()),
                ));
            }
            KeyCode::Enter if selected_prompt_entry(dialog).is_some() => show_prompt_detail(dialog),
            KeyCode::Char('e' | 't' | 'i') => {
                if let Some(entry) = selected_prompt_entry(dialog) {
                    dialog.editor = match key.code {
                        KeyCode::Char('e') => prompt_body_editor(dialog.draft(), &entry.id),
                        KeyCode::Char('t') => prompt_title_editor(dialog.draft(), &entry.id),
                        _ if entry.inline_index.is_some() => Some(prompt_line_editor(
                            format!("Rename fragment `{}`", entry.id),
                            "Use lowercase letters, digits, dots, underscores, or hyphens."
                                .to_owned(),
                            entry.id.to_string(),
                            PromptStudioEditorAction::Rename(entry.id.to_string()),
                        )),
                        _ => None,
                    };
                    if dialog.editor.is_none() {
                        dialog.status = self.i18n.text("prompt-studio-read-only");
                    }
                }
            }
            KeyCode::Char('d') => {
                if let Some(entry) = selected_prompt_entry(dialog) {
                    if entry.id.is_builtin() {
                        dialog.status = self.i18n.text("prompt-studio-required");
                    } else {
                        remove_prompt_fragment(dialog.draft_mut(), &entry.id);
                        dialog.selected = dialog
                            .selected
                            .min(prompt_studio_entries(dialog).len().saturating_sub(1));
                        mark_prompt_edited(dialog, "");
                    }
                }
            }
            KeyCode::Char('l' | 'o' | 'h' | 'u') => {
                dialog.picker = Some(match key.code {
                    KeyCode::Char('l') => PromptStudioPickerAction::Browse,
                    KeyCode::Char('o') => PromptStudioPickerAction::Order,
                    KeyCode::Char('h') => PromptStudioPickerAction::Hide,
                    _ => PromptStudioPickerAction::Unhide,
                });
                dialog.picker_selected = 0;
                dialog.status.clear();
            }
            KeyCode::Char('p') => self.load_prompt_preview(dialog),
            KeyCode::Char('x') => {
                dialog.draft_mut().layer = PromptLayer::default();
                dialog.selected = 0;
                mark_prompt_edited(dialog, self.i18n.text("prompt-studio-cleared").as_str());
            }
            KeyCode::Char('r') => {
                *dialog.draft_mut() = dialog.saved[dialog.scope_index()].clone();
                dialog.selected = 0;
                mark_prompt_edited(dialog, "");
            }
            KeyCode::Char('?') => dialog.status = self.i18n.text("prompt-studio-extra-keys"),
            code => {
                let count = prompt_studio_entries(dialog).len();
                move_list_selection(code, &mut dialog.selected, count);
            }
        }
        false
    }

    fn load_prompt_preview(&mut self, dialog: &mut PromptStudioOverlay) {
        let session_id = dialog.session_id;
        let request = uuid::Uuid::new_v4();
        dialog.preview_request = Some(request);
        dialog.preview_selected = 0;
        dialog.status.clear();
        self.dispatch_backend_operation(
            move |application| async move { application.session_prompt_preview(session_id).await },
            move |app, result| {
                let Some(Overlay::SessionPromptEdit(dialog)) = &mut app.overlay else {
                    return;
                };
                if dialog.preview_request != Some(request) {
                    return;
                }
                dialog.preview_request = None;
                match result {
                    Ok(preview) => dialog.preview = Some(preview),
                    Err(error) => dialog.status = error.to_string(),
                }
            },
        );
    }

    fn save_prompt_studio(&mut self, dialog: &mut PromptStudioOverlay) {
        let index = dialog.scope_index();
        let draft = dialog.draft().clone();
        if draft == dialog.saved[index] {
            return;
        }
        if let Err(error) = draft.layer.validate() {
            dialog.status = error.to_string();
            return;
        }
        let scope = dialog.scope;
        let session_id = dialog.session_id;
        let revision = match scope {
            PromptStudioScope::Global => dialog.global_revision.clone(),
            PromptStudioScope::Workspace => dialog.workspace_revision.clone(),
            PromptStudioScope::Session => None,
        };
        let saved = draft.clone();
        dialog.saving = true;
        dialog.status = self.i18n.text("prompt-studio-saving");
        self.dispatch_backend_operation(
            move |application| async move {
                if scope == PromptStudioScope::Session {
                    application.set_session_prompts(session_id, Some(draft.layer)).await?;
                    return Ok(None);
                }
                let revision = revision.ok_or_else(|| anyhow::anyhow!("Configuration revision is missing. Reopen the prompt editor and try again."))?;
                let response = application.client().set_settings_layer_value_if_revision(
                    scope.as_str(),
                    PROMPTS_SETTING_PATH,
                    prompt_setting_value(&draft),
                    false,
                    true,
                    Some(&revision),
                ).await?;
                if let Err(error) = application.refresh_after_config_edit(PROMPTS_SETTING_PATH).await {
                    tracing::warn!(%error, "could not refresh TUI settings after saving system prompts");
                }
                Ok::<_, anyhow::Error>(Some(response.after_revision))
            },
            move |app, result| {
                let Some(Overlay::SessionPromptEdit(dialog)) = &mut app.overlay else { return };
                if dialog.session_id != session_id || !dialog.saving {
                    return;
                }
                dialog.saving = false;
                match result {
                    Ok(revision) => {
                        dialog.saved[index] = saved;
                        match scope {
                            PromptStudioScope::Global => dialog.global_revision = revision,
                            PromptStudioScope::Workspace => dialog.workspace_revision = revision,
                            PromptStudioScope::Session => {}
                        }
                        dialog.status = app.i18n.text("prompt-studio-saved");
                    }
                    Err(error) => dialog.status = error.to_string(),
                }
            },
        );
    }
}

pub(crate) fn move_list_selection(
    code: crossterm::event::KeyCode,
    selected: &mut usize,
    count: usize,
) {
    use crossterm::event::KeyCode;
    let last = count.saturating_sub(1);
    *selected = match code {
        KeyCode::Up => selected.saturating_sub(1),
        KeyCode::Down => selected.saturating_add(1).min(last),
        KeyCode::PageUp => selected.saturating_sub(8),
        KeyCode::PageDown => selected.saturating_add(8).min(last),
        KeyCode::Home => 0,
        KeyCode::End => last,
        _ => *selected,
    };
}

fn show_prompt_detail(dialog: &mut PromptStudioOverlay) {
    dialog.detail_open = true;
    dialog.detail_scroll.set(0);
}

fn inherited_prompt_layers(dialog: &PromptStudioOverlay) -> impl Iterator<Item = &PromptLayer> {
    (dialog.scope != PromptStudioScope::Global)
        .then_some(&dialog.global.layer)
        .into_iter()
        .chain(
            (dialog.scope == PromptStudioScope::Session && dialog.session_has_workspace)
                .then_some(&dialog.workspace.layer),
        )
}

fn configured_order(dialog: &PromptStudioOverlay) -> Vec<FragmentId> {
    let mut order: Vec<_> = dialog
        .builtins
        .iter()
        .map(|prompt| prompt.id.clone())
        .collect();
    let inherited = inherited_prompt_layers(dialog)
        .flat_map(|layer| layer.order.iter().filter(|id| id.is_builtin()));
    for id in inherited.chain(&dialog.draft().layer.order) {
        let id = dialog
            .builtins
            .iter()
            .find(|prompt| prompt.matches_id(id.as_str()))
            .map_or(id, |prompt| &prompt.id);
        order.retain(|existing| existing != id);
        order.push(id.clone());
    }
    order
}

fn prompt_studio_entries(dialog: &PromptStudioOverlay) -> Vec<PromptStudioEntry> {
    let inline = &dialog.draft().layer.inline;
    let order = configured_order(dialog);
    let mut entries = Vec::with_capacity(order.len() + inline.len());
    for (index, id) in order.iter().enumerate() {
        entries.push(PromptStudioEntry {
            id: id.clone(),
            inline_index: inline.iter().position(|fragment| &fragment.id == id),
            order_index: Some(index),
        });
    }
    for (index, fragment) in inline.iter().enumerate() {
        if !order.contains(&fragment.id) {
            entries.push(PromptStudioEntry {
                id: fragment.id.clone(),
                inline_index: Some(index),
                order_index: None,
            });
        }
    }
    entries
}

fn selected_prompt_entry(dialog: &PromptStudioOverlay) -> Option<PromptStudioEntry> {
    prompt_studio_entries(dialog).get(dialog.selected).cloned()
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
    let mut order = configured_order(dialog);
    let destination = current.saturating_add_signed(delta);
    if destination >= order.len() || destination == current {
        return;
    }
    order.swap(current, destination);
    dialog.draft_mut().layer.order = order;
    dialog.selected = destination;
    mark_prompt_edited(dialog, "Moved the fragment in the order.");
}

fn apply_prompt_editor_submission(
    dialog: &mut PromptStudioOverlay,
    action: PromptStudioEditorAction,
    text: String,
) {
    match action {
        PromptStudioEditorAction::Add(id) => {
            let layer = &mut dialog.draft_mut().layer;
            layer.inline.push(PromptFragment::new(id.clone(), text));
            layer.order.push(id);
            dialog.selected = configured_order(dialog).len().saturating_sub(1);
            mark_prompt_edited(dialog, "");
        }
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
    if new_id.is_builtin() {
        return Err("Built-in prompt IDs are reserved.".to_owned());
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
    let removed_inline = draft.layer.inline.iter().any(|fragment| &fragment.id == id);
    draft.layer.inline.retain(|fragment| &fragment.id != id);
    draft.layer.order.retain(|fragment_id| fragment_id != id);
    draft.layer.disabled.retain(|fragment_id| fragment_id != id);
    // Removing a reference to a saved library prompt must not silently edit
    // a model overlay that also references the same reusable prompt.
    if removed_inline {
        transform_model_references(&mut draft.models, id, None);
    }
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

pub(crate) fn read_prompt_draft(
    value: &serde_json::Value,
) -> anyhow::Result<(PromptStudioDraft, Option<String>)> {
    let revision = value["revision"].as_str().map(str::to_owned);
    let draft = if value["value"].is_null() {
        PromptStudioDraft::default()
    } else {
        serde_json::from_value::<PromptScopeSetting>(value["value"].clone())?.into_draft()
    };
    Ok((draft, revision))
}

fn prompt_setting_value(draft: &PromptStudioDraft) -> serde_json::Value {
    let setting = PromptScopeSetting {
        mode: draft.layer.mode,
        order: draft.layer.order.clone(),
        disabled: draft.layer.disabled.clone(),
        inline: draft.layer.inline.clone(),
        library: draft.library.clone(),
        models: draft.models.clone(),
        categories: draft.categories.clone(),
    };
    serde_json::to_value(setting).expect("prompt scope settings always serialize")
}

fn next_fragment_id(dialog: &PromptStudioOverlay) -> FragmentId {
    let layer = &dialog.draft().layer;
    let library = prompt_library(dialog);
    let inherited_orders: BTreeSet<_> = inherited_prompt_layers(dialog)
        .flat_map(|layer| layer.order.iter().chain(&layer.disabled))
        .collect();
    (1..)
        .map(|index| {
            FragmentId::parse(&format!("fragment-{index}")).expect("generated fragment ID")
        })
        .find(|id| {
            !library.contains_key(id)
                && !inherited_orders.contains(id)
                && !layer.order.contains(id)
                && !layer.disabled.contains(id)
        })
        .expect("an unused inline fragment ID")
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
    Some(inline_body_editor(
        id,
        &fragment.body,
        PromptStudioEditorAction::Body(id.to_string()),
    ))
}

fn inline_body_editor(
    id: &FragmentId,
    body: &str,
    action: PromptStudioEditorAction,
) -> EditorDialogState<PromptStudioEditorAction> {
    EditorDialogState::new(
        format!("Fragment `{id}`"),
        "Markdown instructions for this scope.".to_owned(),
        "Ctrl+S apply · Enter newline · Esc cancel".to_owned(),
        true,
        Editor::from_text(body.to_owned()),
        action,
    )
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
            library: Vec::new(),
            models,
            categories: Vec::new(),
        }
    }

    fn overlay(scope: PromptStudioScope, prompt_draft: PromptStudioDraft) -> PromptStudioOverlay {
        PromptStudioOverlay {
            session_id: 7,
            scope,
            session_has_workspace: true,
            session: prompt_draft,
            ..Default::default()
        }
    }

    fn order_ids(layer: &PromptLayer) -> Vec<String> {
        layer.order.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn the_order_and_library_surfaces_keep_required_entries_readable() {
        use agena_prompt_layers::{BuiltinPrompt, BuiltinPromptWhen};
        use agena_tui::i18n::I18n;
        use ratatui::{Terminal, backend::TestBackend};

        let mut dialog = overlay(PromptStudioScope::Session, PromptStudioDraft::default());
        dialog.builtins = ["identity", "tools"]
            .into_iter()
            .map(|name| BuiltinPrompt {
                id: id(&format!("agena.system.{name}")),
                title: name.to_owned(),
                body: format!("Read the {name} instructions."),
                when: BuiltinPromptWhen::Always,
                aliases: Vec::new(),
                reusable: false,
            })
            .collect();
        dialog
            .global
            .library
            .push(fragment("style", "Team style", "Use examples."));
        dialog.session.layer.order.push(id("style"));
        dialog.selected = 2;
        move_selected_prompt_order(&mut dialog, -1);
        assert_eq!(
            order_ids(&dialog.session.layer),
            ["agena.system.identity", "style", "agena.system.tools"]
        );

        let render = |width, height, dialog: &PromptStudioOverlay| {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal
                .draw(|frame| view::render(frame, frame.area(), dialog, &I18n::english()))
                .unwrap();
            let buffer = terminal.backend().buffer();
            (0..height)
                .map(|y| {
                    (0..width)
                        .map(|x| buffer[(x, y)].symbol())
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("\n")
        };
        let wide = render(108, 28, &dialog);
        assert!(
            wide.contains("Prompt order")
                && wide.contains("Team style")
                && wide.contains("Use examples.")
        );

        dialog.picker = Some(PromptStudioPickerAction::Browse);
        let narrow = render(42, 18, &dialog);
        assert!(narrow.contains("Prompt library") && narrow.contains("◆ identity"));
        show_prompt_detail(&mut dialog);
        let detail = render(42, 18, &dialog);
        assert!(
            detail.contains("Required built-in")
                && detail.contains("Read the identity instructions.")
        );
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

        apply_saved_prompt_choice(
            &mut dialog,
            PromptStudioPickerAction::Hide,
            &fragment("local", "Local", ""),
        );
        assert!(!dialog.draft().layer.disabled.contains(&id("local")));

        apply_saved_prompt_choice(
            &mut dialog,
            PromptStudioPickerAction::Order,
            &fragment("inherited", "Inherited", ""),
        );
        assert!(dialog.draft().layer.order.contains(&id("inherited")));
        assert!(!dialog.draft().layer.disabled.contains(&id("inherited")));

        apply_saved_prompt_choice(
            &mut dialog,
            PromptStudioPickerAction::Hide,
            &fragment("hidden", "Hidden", ""),
        );
        assert!(dialog.draft().layer.disabled.contains(&id("hidden")));
        apply_saved_prompt_choice(
            &mut dialog,
            PromptStudioPickerAction::Unhide,
            &fragment("hidden", "Hidden", ""),
        );
        assert!(!dialog.draft().layer.disabled.contains(&id("hidden")));
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
    fn setting_round_trip_preserves_mode_and_model_overlays() {
        let models = vec![serde_json::json!({
            "match": "anthropic/*",
            "order": ["house-style"],
            "disabled": ["legacy"]
        })];
        let mut prompt_draft = draft(Vec::new(), &[], &[], models.clone());
        prompt_draft.layer.mode = LayerMode::Replace;
        prompt_draft
            .library
            .push(fragment("saved", "Saved", "Instructions"));

        prompt_draft.categories = vec!["Workflow".to_owned()];
        let value = prompt_setting_value(&prompt_draft);
        assert_eq!(value["mode"], serde_json::json!("replace"));
        assert_eq!(value["models"], serde_json::Value::Array(models.clone()));

        let setting: PromptScopeSetting =
            serde_json::from_value(value).expect("editor output is a valid scope setting");
        let round_trip = setting.into_draft();
        assert_eq!(round_trip.layer.mode, LayerMode::Replace);
        assert_eq!(round_trip.models, models);
        assert_eq!(round_trip.library[0].title, "Saved");
        assert_eq!(round_trip.categories, ["Workflow"]);
    }

    #[test]
    fn saved_prompt_picker_inherits_and_shadows_by_scope() {
        let mut dialog = overlay(PromptStudioScope::Session, PromptStudioDraft::default());
        dialog
            .global
            .library
            .push(fragment("shared", "Global prompt", "global"));
        dialog
            .global
            .library
            .push(fragment("global", "Global only", "global"));
        dialog
            .workspace
            .library
            .push(fragment("shared", "Workspace prompt", "workspace"));
        dialog
            .workspace
            .library
            .push(fragment("workspace", "Workspace only", "workspace"));
        let items = saved_prompt_choices(&dialog, PromptStudioPickerAction::Order);
        assert_eq!(items.len(), 3);
        assert_eq!(
            items
                .iter()
                .find(|item| item.id == id("shared"))
                .unwrap()
                .title,
            "Workspace prompt"
        );
        dialog.session.layer.order.push(id("shared"));
        let remaining = saved_prompt_choices(&dialog, PromptStudioPickerAction::Order);
        assert!(!remaining.iter().any(|item| item.id == id("shared")));
        dialog.session.layer.disabled.push(id("global"));
        let hidden = saved_prompt_choices(&dialog, PromptStudioPickerAction::Unhide);
        assert_eq!(hidden.len(), 1);
        assert_eq!(hidden[0].title, "Global only");
    }
}
