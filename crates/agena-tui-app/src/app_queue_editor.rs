use super::*;
use crate::app_composer_state::{composer_document_from_editor, rebuild_placeholders};
use crate::view::sanitize_display_text;
use agena_domain::{ActivityId, ActivityPayload, ComposerDocument, ComposerNode, ResourceDelivery};
use agena_tui_components::Editor;
use agena_tui_components::SurfaceMode;

#[derive(Debug, Clone)]
pub(crate) struct QueueEditorOverlay {
    pub session_id: i64,
    pub snapshot: agena_api::resource::QueuedMessageEditResource,
    pub input: Editor,
    pub items: Vec<ComposerItem>,
    pub attachments_focus: bool,
    pub selected: usize,
    pub field: Option<QueueAttachmentField>,
    pub busy: bool,
    pub error: String,
}
#[derive(Debug, Clone)]
pub(crate) struct QueueAttachmentField {
    pub input: Editor,
    pub target: Option<ActivityId>,
    pub rename: bool,
}
impl QueueEditorOverlay {
    pub(crate) fn new(
        session_id: i64,
        snapshot: agena_api::resource::QueuedMessageEditResource,
        i18n: &I18n,
    ) -> Self {
        let (text, elements, items) = rebuild_placeholders(snapshot.document.clone(), i18n);
        let mut input = Editor::from_text(text);
        input.set_elements(elements);
        Self {
            session_id,
            snapshot,
            input,
            items,
            attachments_focus: false,
            selected: 0,
            field: None,
            busy: false,
            error: String::new(),
        }
    }
    pub(crate) fn document(&self) -> ComposerDocument {
        composer_document_from_editor(self.input.text(), &self.input.draft_elements(), &self.items)
    }
    fn replace_document(&mut self, document: ComposerDocument, i18n: &I18n) {
        let (text, elements, items) = rebuild_placeholders(document, i18n);
        self.input.set_text(text);
        self.input.set_elements(elements);
        self.items = items;
    }
    pub(crate) fn paste(&mut self, text: &str) {
        if self.busy {
            return;
        }
        if let Some(field) = self.field.as_mut() {
            field.input.insert_str(text);
        } else if !self.attachments_focus {
            self.input.insert_str(text);
        }
    }
}
impl App {
    pub(crate) fn request_edit_queued_message(
        &mut self,
        session_id: i64,
        execution: agena_domain::ExecutionId,
    ) {
        self.dispatch_backend_operation(
            move |application| async move {
                application
                    .client()
                    .queued_message_for_edit(session_id, execution)
                    .await
            },
            move |app, result| match result {
                Ok(snapshot)
                    if app.transcript.session_id == Some(session_id)
                        && !matches!(app.overlay, Some(Overlay::QueueEditor(_))) =>
                {
                    app.overlay = Some(Overlay::QueueEditor(QueueEditorOverlay::new(
                        session_id, snapshot, &app.i18n,
                    )));
                }
                Ok(_) => {}
                Err(error) => {
                    app.flash_error(error);
                    app.request_refresh(session_id, true);
                }
            },
        );
    }
    pub(crate) fn handle_queue_editor_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut QueueEditorOverlay,
    ) -> bool {
        use crossterm::event::{KeyCode, KeyModifiers};
        if dialog.busy {
            return false;
        }
        if let Some(mut field) = dialog.field.take() {
            match key.code {
                KeyCode::Esc => return false,
                KeyCode::Enter => {
                    let value = field.input.text().trim();
                    if value.is_empty() {
                        dialog.field = Some(field);
                        return false;
                    }
                    let mut document = dialog.document();
                    if field.rename {
                        for node in &mut document.0 {
                            if let ComposerNode::Activity { activity } = node
                                && Some(activity.id) == field.target
                                && let ActivityPayload::Resource(resource) = &mut activity.payload
                            {
                                resource.name = value.to_owned();
                            }
                        }
                    } else {
                        let activity = queue_file_activity(value);
                        if let Some(target) = field.target {
                            for node in &mut document.0 {
                                if let ComposerNode::Activity { activity: old } = node
                                    && old.id == target
                                {
                                    let mut replacement = activity.clone();
                                    if let (
                                        ActivityPayload::Resource(previous),
                                        ActivityPayload::Resource(next),
                                    ) = (&old.payload, &mut replacement.payload)
                                    {
                                        next.delivery = previous.delivery;
                                    }
                                    **old = replacement;
                                }
                            }
                        } else {
                            document.0.push(ComposerNode::activity(activity));
                        }
                    }
                    dialog.replace_document(document, &self.i18n);
                    return false;
                }
                _ => {
                    field.input.handle_line_input_key(key);
                    dialog.field = Some(field);
                    return false;
                }
            }
        }
        if key.code == KeyCode::Esc {
            return true;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(key.code, KeyCode::Char('s') | KeyCode::Enter)
        {
            let document = dialog.document();
            if document.is_empty() {
                return false;
            }
            dialog.busy = true;
            dialog.error.clear();
            let session_id = dialog.session_id;
            let execution = agena_domain::ExecutionId(dialog.snapshot.execution_id);
            let revision = dialog.snapshot.revision;
            self.dispatch_backend_operation(move |application| async move {
                application.client().edit_queued_message(session_id, execution, revision, document).await
            }, move |app, result| {
                let matches = matches!(&app.overlay, Some(Overlay::QueueEditor(editor)) if editor.session_id == session_id && editor.snapshot.execution_id == execution.0);
                match result {
                    Ok(state) => {
                        app.handle_session_execution_updated(session_id, state, true);
                        if matches { app.overlay = None; app.request_message_queue(session_id, false); }
                    }
                    Err(error) => {
                        if let Some(Overlay::QueueEditor(editor)) = app.overlay.as_mut() && matches {
                            editor.busy = false;
                            editor.error = error.to_string();
                        } else { app.flash_error(error); }
                    }
                }
            });
            return false;
        }
        if key.code == KeyCode::Tab || key.code == KeyCode::BackTab {
            dialog.attachments_focus = !dialog.attachments_focus;
            return false;
        }
        if !dialog.attachments_focus {
            dialog.input.handle_multiline_input_key(key);
            return false;
        }
        let mut document = dialog.document();
        let activities = document
            .0
            .iter()
            .filter_map(|node| {
                if let ComposerNode::Activity { activity } = node {
                    Some(activity.as_ref())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        dialog.selected = dialog.selected.min(activities.len());
        let selected = activities.get(dialog.selected).copied();
        match key.code {
            KeyCode::Up => dialog.selected = dialog.selected.saturating_sub(1),
            KeyCode::Down => dialog.selected = (dialog.selected + 1).min(activities.len()),
            KeyCode::Delete | KeyCode::Backspace => {
                let id = selected.map(|activity| activity.id);
                document.0.retain(|node| !matches!(node, ComposerNode::Activity { activity } if Some(activity.id) == id));
                dialog.replace_document(document, &self.i18n);
            }
            KeyCode::Char('p') => {
                let id = selected.map(|activity| activity.id);
                for node in &mut document.0 {
                    if let ComposerNode::Activity { activity } = node
                        && Some(activity.id) == id
                        && let ActivityPayload::Resource(resource) = &mut activity.payload
                    {
                        resource.delivery = if resource.delivery == ResourceDelivery::ModelInput {
                            ResourceDelivery::Reference
                        } else {
                            ResourceDelivery::ModelInput
                        };
                    }
                }
                dialog.replace_document(document, &self.i18n);
            }
            KeyCode::Enter | KeyCode::Char('r') | KeyCode::Char('a') | KeyCode::Char('n') => {
                if key.code != KeyCode::Char('a')
                    && selected.is_some_and(|activity| {
                        !matches!(activity.payload, ActivityPayload::Resource(_))
                    })
                {
                    return false;
                }
                let add = key.code == KeyCode::Char('a') || selected.is_none();
                let rename = key.code == KeyCode::Char('n') && !add;
                let text = if rename {
                    selected
                        .and_then(|activity| match &activity.payload {
                            ActivityPayload::Resource(resource) => Some(resource.name.clone()),
                            _ => None,
                        })
                        .unwrap_or_default()
                } else {
                    String::new()
                };
                dialog.field = Some(QueueAttachmentField {
                    input: Editor::from_text(text),
                    target: if add {
                        None
                    } else {
                        selected.map(|activity| activity.id)
                    },
                    rename,
                });
            }
            _ => {}
        }
        false
    }

    pub(crate) fn render_queue_editor(
        &self,
        frame: &mut Frame,
        area: Rect,
        dialog: &QueueEditorOverlay,
    ) {
        use agena_tui_components::{
            EditorPanelSpec, FramedSurfaceSpec, render_editor_panel, render_framed_surface,
        };
        use ratatui::{
            layout::{Constraint, Layout},
            widgets::{Borders, Paragraph, Wrap},
        };
        let surface = render_framed_surface(
            frame,
            area,
            SurfaceMode::Overlay,
            &FramedSurfaceSpec {
                title: ui_text::t(&self.i18n, "queue-edit").into(),
                target_width: 96,
                target_height: 28,
            },
        );
        let rows = Layout::vertical([
            Constraint::Length(2),
            Constraint::Min(5),
            Constraint::Length(7),
            Constraint::Length(3),
        ])
        .split(surface.inner);
        let hint = if !dialog.error.is_empty() {
            dialog.error.clone()
        } else if let Some(field) = &dialog.field {
            ui_text::t(
                &self.i18n,
                if field.rename {
                    "queue-attachment-name-hint"
                } else {
                    "queue-attachment-path-hint"
                },
            )
        } else {
            ui_text::t(&self.i18n, "queue-edit-hint")
        };
        frame.render_widget(
            Paragraph::new(sanitize_display_text(hint)).wrap(Wrap { trim: false }),
            rows[0],
        );
        let input = dialog
            .field
            .as_ref()
            .map(|field| &field.input)
            .unwrap_or(&dialog.input);
        let result = render_editor_panel(
            frame,
            rows[1],
            &EditorPanelSpec {
                title: None,
                borders: Borders::ALL,
            },
            input,
        );
        let document = dialog.document();
        let mut labels = document
            .0
            .iter()
            .filter_map(|node| match node {
                ComposerNode::Activity { activity } => Some(activity),
                _ => None,
            })
            .map(|activity| match &activity.payload {
                ActivityPayload::Resource(resource) => format!(
                    "{} · {}",
                    resource.name,
                    if resource.delivery == ResourceDelivery::ModelInput {
                        ui_text::t(&self.i18n, "queue-model-input")
                    } else {
                        ui_text::t(&self.i18n, "queue-reference")
                    }
                ),
                _ => {
                    crate::composer_state_impls::composer_activity_presentation(
                        &activity.payload,
                        &self.i18n,
                    )
                    .1
                }
            })
            .collect::<Vec<_>>();
        labels.push(ui_text::t(&self.i18n, "queue-add-attachment"));
        let count = labels.len();
        let visible = usize::from(rows[2].height);
        let selected = dialog.selected.min(count.saturating_sub(1));
        let start = selected.saturating_sub(visible.saturating_sub(1));
        let lines = labels
            .into_iter()
            .enumerate()
            .skip(start)
            .take(visible)
            .map(|(index, label)| {
                format!(
                    "{} {}",
                    if dialog.attachments_focus && index == selected {
                        "›"
                    } else {
                        " "
                    },
                    sanitize_display_text(label)
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        frame.render_widget(Paragraph::new(lines), rows[2]);
        frame.render_widget(
            Paragraph::new(ui_text::t(
                &self.i18n,
                if dialog.busy {
                    "queue-saving"
                } else {
                    "queue-editor-keys"
                },
            ))
            .wrap(Wrap { trim: false }),
            rows[3],
        );
        if !dialog.attachments_focus || dialog.field.is_some() {
            frame.set_cursor_position(result.cursor);
        }
    }
}
fn queue_file_activity(path: &str) -> agena_domain::ComposerActivity {
    let name = Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(path)
        .to_owned();
    let extension = Path::new(path)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let (kind, mime) = match extension.as_str() {
        "png" => ("image", "image/png"),
        "jpg" | "jpeg" => ("image", "image/jpeg"),
        "webp" => ("image", "image/webp"),
        "gif" => ("image", "image/gif"),
        "pdf" => ("pdf", "application/pdf"),
        "mp3" => ("audio", "audio/mpeg"),
        "wav" => ("audio", "audio/wav"),
        "mp4" => ("video", "video/mp4"),
        _ => ("file", "application/octet-stream"),
    };
    serde_json::from_value(serde_json::json!({"id":ActivityId::new(),"payload":{
        "activity_type":"resource","kind":kind,"name":name,"media_type":mime,
        "delivery": if kind == "file" { "reference" } else { "model_input" },
        "reference":{"reference_type":"workspace_path","path":path}
    }}))
    .expect("queue resource activity")
}
