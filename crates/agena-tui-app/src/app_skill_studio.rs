use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    text::{Line, Span, Text},
};
use serde_json::Value;

use crate::{
    App, ConfirmAction, ConfirmOverlay, Editor, EditorDialogKeyResult, Overlay, Route,
    SkillStudioDetail, SkillStudioEditor, SkillStudioEditorAction, SkillStudioItem,
    SkillStudioOverlay, UiResult, drive_editor_dialog_key, ui_text,
};
use agena_tui::keymap::{KeyAction, KeyContext, resolve as resolve_tui_key};
use agena_tui::selection_picker::SelectionPickerItem;
use agena_tui_components::{
    LineTextDialogSpec, SurfaceMode, TextDialogLine, render_editor_dialog, render_line_text_dialog,
    wrapped_text_height_for_text,
};

const SKILL_STUDIO_PAGE_SIZE: usize = 100;
const NEW_SKILL_TEMPLATE: &str = "---\nname: new_skill\ndescription: Describe this reusable workflow\naliases: []\n---\n\nWrite the Skill instructions here.\n";

impl App {
    /// Open the local Skill workbench. Runtime plugin invocation is attached
    /// to an active session so its writes keep the ordinary Agena permission
    /// and event/audit path instead of bypassing the execution layer.
    pub(crate) fn open_skill_studio(&mut self) {
        if self.skill_studio_session_id().is_err() {
            self.flash_warning(ui_text::t(&self.i18n, "flash-command-requires-session"));
            return;
        }

        let presentation = agena_tui::selection_picker::new_presentation(
            ui_text::t(&self.i18n, "overlay-skill-studio-title"),
            ui_text::t(&self.i18n, "overlay-skill-studio-prompt"),
            String::new(),
            ui_text::t(&self.i18n, "overlay-skill-studio-empty"),
            String::new(),
        );
        let mut dialog = SkillStudioOverlay {
            presentation,
            actions: Default::default(),
            detail: None,
            editor: None,
            offset: 0,
            total: 0,
            limit: SKILL_STUDIO_PAGE_SIZE,
        };
        self.load_skill_studio_page(&mut dialog, 0);
        self.current_route = Route::SkillStudio(dialog);
    }

    pub(crate) fn handle_skill_studio_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut SkillStudioOverlay,
    ) -> bool {
        if dialog.editor.is_some() {
            return self.handle_skill_studio_editor_key(key, dialog);
        }

        if dialog.detail.is_some() {
            return self.handle_skill_studio_detail_key(key, dialog);
        }

        if key.modifiers == KeyModifiers::CONTROL {
            match key.code {
                KeyCode::Char('n') => {
                    self.open_skill_studio_create_editor(dialog);
                    return false;
                }
                KeyCode::Char('r') => {
                    self.load_skill_studio_page(dialog, dialog.offset);
                    self.flash_success(ui_text::t(&self.i18n, "flash-skill-studio-refreshed"));
                    return false;
                }
                KeyCode::PageUp if dialog.offset > 0 => {
                    self.load_skill_studio_page(dialog, dialog.offset.saturating_sub(dialog.limit));
                    return false;
                }
                KeyCode::PageDown
                    if dialog
                        .offset
                        .saturating_add(dialog.presentation.items.len())
                        < dialog.total =>
                {
                    self.load_skill_studio_page(dialog, dialog.offset.saturating_add(dialog.limit));
                    return false;
                }
                _ => {}
            }
        }

        let action = match resolve_tui_key(KeyContext::Picker, key) {
            Some(KeyAction::Accept) => agena_tui::selection_picker::SelectionPickerAction::Accept,
            _ => agena_tui::selection_picker::SelectionPickerAction::Input(key),
        };
        match agena_tui::selection_picker::reduce(&mut dialog.presentation, action) {
            agena_tui::selection_picker::SelectionPickerEffect::Close => true,
            agena_tui::selection_picker::SelectionPickerEffect::KeepOpen => false,
            agena_tui::selection_picker::SelectionPickerEffect::Activate { key } => {
                let Some(item) = dialog.actions.get(key.as_str()).cloned() else {
                    return false;
                };
                self.open_skill_studio_detail(dialog, item);
                false
            }
        }
    }

    fn handle_skill_studio_editor_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut SkillStudioOverlay,
    ) -> bool {
        let Some(mut editor) = dialog.editor.take() else {
            return false;
        };
        match drive_editor_dialog_key(&mut editor, key) {
            EditorDialogKeyResult::Continue => {
                dialog.editor = Some(editor);
                false
            }
            EditorDialogKeyResult::Close => false,
            EditorDialogKeyResult::Submit(action, document) => {
                match self.commit_skill_studio_editor(action, document.as_str()) {
                    Ok(()) => {
                        dialog.editor = None;
                    }
                    Err(error) => self.flash_error(error),
                }
                false
            }
        }
    }

    fn handle_skill_studio_detail_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut SkillStudioOverlay,
    ) -> bool {
        let Some(detail) = dialog.detail.clone() else {
            return false;
        };
        if key.modifiers == KeyModifiers::NONE {
            match key.code {
                KeyCode::Esc => {
                    dialog.detail = None;
                    return false;
                }
                KeyCode::Char('e') => {
                    if detail.item.editable {
                        self.open_skill_studio_update_editor(dialog);
                    } else {
                        self.flash_warning(ui_text::t(&self.i18n, "flash-skill-studio-read-only"));
                    }
                    return false;
                }
                KeyCode::Char('d') => {
                    if detail.item.editable {
                        self.confirm_skill_studio_delete(detail.item.name.as_str());
                    } else {
                        self.flash_warning(ui_text::t(&self.i18n, "flash-skill-studio-read-only"));
                    }
                    return false;
                }
                KeyCode::Char('r') => {
                    self.open_skill_studio_detail(dialog, detail.item);
                    return false;
                }
                KeyCode::Up => {
                    if let Some(detail) = dialog.detail.as_mut() {
                        detail.scroll = detail.scroll.saturating_sub(1);
                    }
                    return false;
                }
                KeyCode::Down => {
                    if let Some(detail) = dialog.detail.as_mut() {
                        detail.scroll = detail.scroll.saturating_add(1);
                    }
                    return false;
                }
                KeyCode::PageUp => {
                    let page_size = self.skill_studio_detail_page_size(dialog);
                    if let Some(detail) = dialog.detail.as_mut() {
                        detail.scroll = detail.scroll.saturating_sub(page_size as u16);
                    }
                    return false;
                }
                KeyCode::PageDown => {
                    let page_size = self.skill_studio_detail_page_size(dialog);
                    if let Some(detail) = dialog.detail.as_mut() {
                        detail.scroll = detail.scroll.saturating_add(page_size as u16);
                    }
                    return false;
                }
                _ => {}
            }
        }
        false
    }

    fn load_skill_studio_page(&mut self, dialog: &mut SkillStudioOverlay, offset: usize) {
        dialog.presentation.set_loading(true);
        dialog.presentation.error_message = None;
        dialog.actions.clear();
        dialog.offset = offset;
        let session_id = match self.skill_studio_session_id() {
            Ok(session_id) => session_id,
            Err(error) => {
                dialog.presentation.error_message = Some(error.to_string());
                dialog.presentation.set_loading(false);
                return;
            }
        };
        let limit = dialog.limit;
        self.dispatch_backend_operation(
            move |application| async move {
                application
                    .invoke_plugin_tool(
                        "agena.skills",
                        "list",
                        serde_json::json!({
                            "kind": "skill",
                            "offset": offset,
                            "limit": limit,
                            "verbose": true,
                        }),
                        Some(session_id),
                    )
                    .await
            },
            move |app, result| {
                let route = std::mem::replace(&mut app.current_route, Route::Main);
                app.current_route = match route {
                    Route::SkillStudio(mut dialog) if dialog.offset == offset => {
                        app.apply_skill_studio_page(&mut dialog, result);
                        Route::SkillStudio(dialog)
                    }
                    route => route,
                };
            },
        );
    }

    fn apply_skill_studio_page(
        &mut self,
        dialog: &mut SkillStudioOverlay,
        result: UiResult<agena_plugin_host::PluginToolInvokeResponse>,
    ) {
        match result.and_then(|response| skill_studio_page(response.payload)) {
            Ok(page) => {
                dialog.offset = page.offset;
                dialog.total = page.total;
                let mut items = Vec::with_capacity(page.items.len());
                for (index, item) in page.items.into_iter().enumerate() {
                    let key = format!("skill-studio:{}:{}", dialog.offset, index);
                    let access = if item.editable {
                        "workspace"
                    } else {
                        "read-only"
                    };
                    let aliases = if item.aliases.is_empty() {
                        String::new()
                    } else {
                        format!(" · aliases: {}", item.aliases.join(", "))
                    };
                    let detail = format!("{} · {} · {}{}", item.kind, item.source, access, aliases);
                    items.push(SelectionPickerItem::new(
                        key.clone(),
                        item.name.clone(),
                        detail.clone(),
                        format!(
                            "{} {} {} {}",
                            item.name,
                            item.summary,
                            detail,
                            item.aliases.join(" ")
                        ),
                    ));
                    dialog.actions.insert(key, item);
                }
                dialog.presentation.replace_items(items);
                dialog.presentation.footer = self.skill_studio_footer(dialog);
                dialog.presentation.set_loading(false);
            }
            Err(error) => {
                dialog.presentation.replace_items(Vec::new());
                dialog.presentation.error_message = Some(error.to_string());
                dialog.presentation.footer = self.skill_studio_footer(dialog);
                dialog.presentation.set_loading(false);
            }
        }
    }

    fn skill_studio_footer(&self, dialog: &SkillStudioOverlay) -> String {
        let page = dialog.offset / dialog.limit + 1;
        let pages = dialog.total.max(1).div_ceil(dialog.limit);
        self.i18n.text_args(
            "overlay-skill-studio-footer",
            &agena_tui::fl_args!(
                "page" => page as i64,
                "pages" => pages as i64,
                "total" => dialog.total as i64,
            ),
        )
    }

    fn open_skill_studio_detail(
        &mut self,
        _dialog: &mut SkillStudioOverlay,
        item: SkillStudioItem,
    ) {
        let session_id = match self.skill_studio_session_id() {
            Ok(session_id) => session_id,
            Err(error) => {
                self.flash_error(error);
                return;
            }
        };
        let requested_name = item.name.clone();
        self.dispatch_backend_operation(
            move |application| async move {
                application
                    .invoke_plugin_tool(
                        "agena.skills",
                        "get",
                        serde_json::json!({ "name": requested_name }),
                        Some(session_id),
                    )
                    .await
            },
            move |app, result| {
                let result =
                    result.and_then(|response| skill_studio_detail(response.payload, item));
                let route = std::mem::replace(&mut app.current_route, Route::Main);
                app.current_route = match route {
                    Route::SkillStudio(mut dialog) => {
                        match result {
                            Ok(detail) => dialog.detail = Some(detail),
                            Err(error) => app.flash_error(error),
                        }
                        Route::SkillStudio(dialog)
                    }
                    route => route,
                };
            },
        );
    }

    fn open_skill_studio_create_editor(&mut self, dialog: &mut SkillStudioOverlay) {
        dialog.editor = Some(SkillStudioEditor::new(
            ui_text::t(&self.i18n, "overlay-skill-studio-create-title"),
            ui_text::t(&self.i18n, "overlay-skill-studio-editor-prompt"),
            ui_text::t(&self.i18n, "overlay-skill-studio-editor-footer"),
            true,
            Editor::from_text(NEW_SKILL_TEMPLATE.to_owned()),
            SkillStudioEditorAction::Create,
        ));
    }

    fn open_skill_studio_update_editor(&mut self, dialog: &mut SkillStudioOverlay) {
        let Some(detail) = dialog.detail.as_ref() else {
            return;
        };
        dialog.editor = Some(SkillStudioEditor::new(
            self.i18n.text_args(
                "overlay-skill-studio-edit-title",
                &agena_tui::fl_args!("name" => detail.item.name.clone()),
            ),
            ui_text::t(&self.i18n, "overlay-skill-studio-editor-prompt"),
            ui_text::t(&self.i18n, "overlay-skill-studio-editor-footer"),
            true,
            Editor::from_text(detail.document.clone()),
            SkillStudioEditorAction::Update {
                name: detail.item.name.clone(),
            },
        ));
    }

    fn commit_skill_studio_editor(
        &mut self,
        action: SkillStudioEditorAction,
        document: &str,
    ) -> UiResult<()> {
        let session_id = self.skill_studio_session_id()?;
        let (tool, input) = match action {
            SkillStudioEditorAction::Create => {
                ("create", serde_json::json!({ "document": document }))
            }
            SkillStudioEditorAction::Update { name } => (
                "update",
                serde_json::json!({ "name": name, "document": document }),
            ),
        };
        self.dispatch_backend_operation(
            move |application| async move {
                application
                    .invoke_plugin_tool("agena.skills", tool, input, Some(session_id))
                    .await
            },
            |app, result| match result {
                Ok(response) => {
                    let message = response
                        .payload
                        .as_ref()
                        .and_then(|payload| payload.get("operation"))
                        .and_then(Value::as_str)
                        .map(|operation| format!("Skill {operation}."))
                        .unwrap_or_else(|| ui_text::t(&app.i18n, "flash-skill-studio-saved"));
                    app.flash_success(message);
                    let route = std::mem::replace(&mut app.current_route, Route::Main);
                    app.current_route = match route {
                        Route::SkillStudio(mut dialog) => {
                            dialog.editor = None;
                            dialog.detail = None;
                            let offset = dialog.offset;
                            app.load_skill_studio_page(&mut dialog, offset);
                            Route::SkillStudio(dialog)
                        }
                        route => route,
                    };
                }
                Err(error) => app.flash_error(error),
            },
        );
        Ok(())
    }

    fn confirm_skill_studio_delete(&mut self, name: &str) {
        self.overlay = Some(Overlay::Confirm(ConfirmOverlay::new(
            ui_text::t(&self.i18n, "overlay-skill-studio-delete-title"),
            vec![self.i18n.text_args(
                "overlay-skill-studio-delete-body",
                &agena_tui::fl_args!("name" => name.to_owned()),
            )],
            ui_text::t(&self.i18n, "overlay-skill-studio-delete-footer"),
            ConfirmAction::SkillStudioDelete {
                name: name.to_owned(),
            },
        )));
    }

    pub(crate) fn delete_skill_studio_skill(&mut self, name: &str) {
        let session_id = match self.skill_studio_session_id() {
            Ok(session_id) => session_id,
            Err(error) => {
                self.flash_error(error);
                return;
            }
        };
        let name = name.to_string();
        self.dispatch_backend_operation(
            move |application| async move {
                application
                    .invoke_plugin_tool(
                        "agena.skills",
                        "delete",
                        serde_json::json!({ "name": name }),
                        Some(session_id),
                    )
                    .await
            },
            |app, result| match result {
                Ok(_) => {
                    let route = std::mem::replace(&mut app.current_route, Route::Main);
                    app.current_route = match route {
                        Route::SkillStudio(mut dialog) => {
                            dialog.detail = None;
                            let offset = dialog.offset;
                            app.load_skill_studio_page(&mut dialog, offset);
                            Route::SkillStudio(dialog)
                        }
                        route => route,
                    };
                    app.flash_success(ui_text::t(&app.i18n, "flash-skill-studio-deleted"));
                }
                Err(error) => app.flash_error(error),
            },
        );
    }

    fn skill_studio_session_id(&self) -> UiResult<i64> {
        self.transcript
            .session_id
            .or_else(|| self.sessions.current_selected_id())
            .ok_or_else(|| {
                crate::UiFailure::message(ui_text::t(&self.i18n, "flash-command-requires-session"))
            })
    }

    /// The number of lines one PageUp/PageDown moves the Skill detail body,
    /// matching the dialog's actual body height instead of a hard-coded 12.
    fn skill_studio_detail_page_size(&self, dialog: &SkillStudioOverlay) -> usize {
        let Some(detail) = dialog.detail.as_ref() else {
            return 12;
        };
        let content_width = SurfaceMode::Overlay.content_width(self.layout.overlay_area, 104);
        let body = Text::from(
            skill_studio_detail_lines(detail)
                .iter()
                .map(|line| Line::from(Span::styled(line.text.clone(), line.style)))
                .collect::<Vec<_>>(),
        );
        usize::from(
            wrapped_text_height_for_text(&body, content_width)
                .clamp(8, 26)
                .max(1),
        )
    }

    pub(crate) fn render_skill_studio(
        &self,
        frame: &mut Frame,
        area: Rect,
        dialog: &SkillStudioOverlay,
    ) {
        agena_tui::selection_picker::render_overlay(frame, area, &dialog.presentation, &self.i18n);

        if let Some(detail) = dialog.detail.as_ref() {
            let lines = skill_studio_detail_lines(detail);
            let footer = if detail.item.editable {
                ui_text::t(&self.i18n, "overlay-skill-studio-detail-footer-editable")
            } else {
                ui_text::t(&self.i18n, "overlay-skill-studio-detail-footer-read-only")
            };
            let spec = LineTextDialogSpec::new(
                self.i18n
                    .text_args(
                        "overlay-skill-studio-detail-title",
                        &agena_tui::fl_args!("name" => detail.item.name.clone()),
                    )
                    .into(),
                lines.as_slice(),
                Some(footer.into()),
                104,
                true,
                Some((detail.scroll, 0)),
                None,
                (8, 26),
                (1, 2),
                None,
                Style::default(),
            );
            render_line_text_dialog(frame, area, SurfaceMode::Overlay, &spec);
        }

        if let Some(editor) = dialog.editor.as_ref() {
            render_editor_dialog(
                frame,
                area,
                SurfaceMode::Overlay,
                &agena_tui_components::EditorDialogSpec {
                    title: editor.title.as_str().into(),
                    prompt: editor.prompt.as_str().into(),
                    footer: editor.footer.as_str().into(),
                    target_width: 104,
                    multiline: editor.multiline,
                    prompt_height_bounds: (1, 3),
                    footer_height_bounds: (1, 2),
                },
                &editor.input,
            );
        }
    }
}

fn skill_studio_detail_lines(detail: &SkillStudioDetail) -> Vec<TextDialogLine<'static>> {
    let access = if detail.item.editable {
        "workspace-managed"
    } else {
        "read-only"
    };
    let mut lines = vec![
        TextDialogLine::plain(format!("Name: {}", detail.item.name)),
        TextDialogLine::plain(format!("Summary: {}", detail.item.summary)),
        TextDialogLine::plain(format!("Source: {} · {access}", detail.item.source)),
        TextDialogLine::plain(format!(
            "Aliases: {}",
            if detail.item.aliases.is_empty() {
                "none".to_owned()
            } else {
                detail.item.aliases.join(", ")
            }
        )),
        TextDialogLine::plain(String::new()),
        TextDialogLine::plain("SKILL.md:"),
    ];
    lines.extend(
        detail
            .document
            .lines()
            .map(|line| TextDialogLine::plain(line.to_owned())),
    );
    lines
}

#[derive(Debug)]
struct SkillStudioPage {
    items: Vec<SkillStudioItem>,
    total: usize,
    offset: usize,
}

fn skill_studio_page(payload: Option<Value>) -> UiResult<SkillStudioPage> {
    let payload = payload.as_ref().and_then(Value::as_object).ok_or_else(|| {
        crate::UiFailure::message("Skill catalog returned no structured payload.")
    })?;
    let tools = payload
        .get("tools")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            crate::UiFailure::message("Skill catalog payload is missing its tools list.")
        })?;
    let items = tools
        .iter()
        .filter_map(|value| {
            let value = value.as_object()?;
            (json_string(value.get("kind")) == "skill").then_some(())?;
            let name = json_string(value.get("name"));
            (!name.is_empty()).then(|| SkillStudioItem {
                name,
                kind: json_string(value.get("kind")),
                summary: json_string(value.get("summary")),
                aliases: json_strings(value.get("aliases")),
                source: json_string(value.get("source")),
                editable: value
                    .get("editable")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            })
        })
        .collect();
    Ok(SkillStudioPage {
        items,
        total: json_count(payload.get("total")),
        offset: json_count(payload.get("offset")),
    })
}

fn skill_studio_detail(
    payload: Option<Value>,
    item: SkillStudioItem,
) -> UiResult<SkillStudioDetail> {
    let payload = payload
        .as_ref()
        .and_then(Value::as_object)
        .ok_or_else(|| crate::UiFailure::message("Skill detail returned no structured payload."))?;
    if json_string(payload.get("kind")) != "skill" {
        return Err(crate::UiFailure::message(
            "The selected catalog entry is not a Skill.",
        ));
    }
    let document = payload
        .get("document")
        .and_then(Value::as_str)
        .filter(|document| !document.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| {
            crate::UiFailure::message("Skill detail is missing its editable SKILL.md document.")
        })?;
    let name = json_string(payload.get("name"));
    if name != item.name {
        return Err(crate::UiFailure::message(
            "Skill catalog changed while opening details; refresh and retry.",
        ));
    }
    Ok(SkillStudioDetail {
        item: SkillStudioItem {
            summary: json_string(payload.get("summary")),
            aliases: json_strings(payload.get("aliases")),
            source: json_string(payload.get("source")),
            editable: payload
                .get("editable")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            ..item
        },
        document,
        scroll: 0,
    })
}

fn json_string(value: Option<&Value>) -> String {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or_default()
        .to_owned()
}

fn json_strings(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn json_count(value: Option<&Value>) -> usize {
    value
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::{skill_studio_detail, skill_studio_page};

    #[test]
    fn catalog_page_preserves_workspace_editability() {
        let page = skill_studio_page(Some(serde_json::json!({
            "tools": [{
                "name": "review",
                "kind": "skill",
                "summary": "Review a change",
                "aliases": ["check"],
                "source": "filesystem",
                "editable": true,
            }],
            "total": 1,
            "offset": 0,
        })))
        .expect("valid page");
        assert!(page.items[0].editable);
        assert_eq!(page.items[0].aliases, ["check"]);
    }

    #[test]
    fn detail_rejects_catalog_races_and_retains_document() {
        let page = skill_studio_page(Some(serde_json::json!({
            "tools": [{"name": "review", "kind": "skill", "editable": true}],
            "total": 1,
            "offset": 0,
        })))
        .expect("valid page");
        let detail = skill_studio_detail(
            Some(serde_json::json!({
                "name": "review",
                "kind": "skill",
                "document": "---\nname: review\n---\nReview it.",
                "editable": true,
            })),
            page.items.into_iter().next().expect("item"),
        )
        .expect("valid detail");
        assert!(detail.document.contains("Review it."));
    }
}
