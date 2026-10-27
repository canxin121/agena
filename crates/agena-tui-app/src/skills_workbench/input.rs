use std::sync::Arc;

use agena_api::skills::{SkillInstallSource, SkillScope};
use agena_tui_components::{
    ConfirmDialogState, Editor, EditorDialogKeyResult, EditorDialogState, drive_editor_dialog_key,
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::state::{SkillConfirmAction, SkillEditAction, scope_label};
use super::{App, SkillMutationResult, SkillPane, SkillsWorkbench};

pub(super) enum SkillMutation {
    Create {
        scope: SkillScope,
        document: String,
    },
    Document {
        id: String,
        document: String,
        revision: String,
    },
    Enabled {
        id: String,
        enabled: bool,
        revision: String,
    },
    Delete {
        id: String,
        revision: String,
    },
    Resource {
        id: String,
        path: String,
        content: String,
        revision: Option<String>,
    },
    DeleteResource {
        id: String,
        path: String,
        revision: String,
    },
    Install {
        scope: SkillScope,
        source: Arc<SkillInstallSource>,
        selected: Vec<String>,
        replace: bool,
        source_revision: String,
    },
}

impl App {
    pub(crate) fn handle_skills_workbench_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut SkillsWorkbench,
    ) -> bool {
        if self.application.workspace_id() != dialog.workspace_id && key.code != KeyCode::Esc {
            dialog.error = Some("This workbench belongs to another workspace. Press Esc, then reopen Skills for the current workspace.".to_owned());
            return false;
        }
        if dialog.busy {
            return false;
        }
        if dialog.confirm.is_some() {
            match key.code {
                KeyCode::Esc | KeyCode::Char('n') => dialog.confirm = None,
                KeyCode::Char('y') | KeyCode::Enter => {
                    let action = dialog.confirm.take().expect("confirmation exists").action;
                    match action {
                        SkillConfirmAction::DiscardEditor => {
                            dialog.editor = None;
                            dialog.error = None;
                        }
                        SkillConfirmAction::DeleteSkill { id, revision } => {
                            self.submit_skill_mutation(
                                dialog,
                                SkillMutation::Delete { id, revision },
                            );
                        }
                        SkillConfirmAction::DeleteResource { id, path, revision } => {
                            self.submit_skill_mutation(
                                dialog,
                                SkillMutation::DeleteResource { id, path, revision },
                            );
                        }
                        SkillConfirmAction::Install => self.install_selected_skills(dialog),
                    }
                }
                _ => {}
            }
            return false;
        }
        if let Some(editor) = dialog.editor.as_mut() {
            match drive_editor_dialog_key(editor, key) {
                EditorDialogKeyResult::Continue => {}
                EditorDialogKeyResult::Close => {
                    if editor.input.text() == dialog.editor_original {
                        dialog.editor = None;
                        dialog.error = None;
                    } else {
                        dialog.confirm = Some(ConfirmDialogState::new(
                            "Discard changes?".to_owned(),
                            vec!["The unsaved text in this editor will be discarded.".to_owned()],
                            "Enter / Y discard · Esc keep editing".to_owned(),
                            SkillConfirmAction::DiscardEditor,
                        ));
                    }
                }
                EditorDialogKeyResult::Submit(action, text) => {
                    self.submit_skill_editor(dialog, action, text);
                }
            }
            return false;
        }
        if dialog.installation.is_some() {
            self.handle_skill_installation_key(dialog, key);
            return false;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            if key.code == KeyCode::Char('r') {
                self.request_skills_refresh(dialog);
            }
            return false;
        }
        match key.code {
            KeyCode::Esc => return true,
            KeyCode::Tab | KeyCode::BackTab => {
                dialog.pane = match dialog.pane {
                    SkillPane::Library => SkillPane::Files,
                    SkillPane::Files => SkillPane::Library,
                };
            }
            KeyCode::Up | KeyCode::Char('k') => self.move_skill_selection(dialog, -1),
            KeyCode::Down | KeyCode::Char('j') => self.move_skill_selection(dialog, 1),
            KeyCode::Home => self.move_skill_selection(dialog, isize::MIN),
            KeyCode::End => self.move_skill_selection(dialog, isize::MAX),
            KeyCode::PageUp => dialog.scroll = dialog.scroll.saturating_sub(10),
            KeyCode::PageDown => dialog.scroll = dialog.scroll.saturating_add(10),
            KeyCode::Enter => match dialog.pane {
                SkillPane::Library => {
                    dialog.pane = SkillPane::Files;
                    if dialog.detail.is_none() {
                        self.request_selected_skill(dialog);
                    }
                }
                SkillPane::Files => self.request_selected_skill_resource(dialog),
            },
            KeyCode::Char('/') => dialog.open_editor(
                "Search skills", "Filter by name, description, or path.", false,
                dialog.query.clone(), SkillEditAction::Search,
            ),
            KeyCode::Char('g') => {
                dialog.scope_filter = match dialog.scope_filter {
                    None => Some(SkillScope::Workspace),
                    Some(SkillScope::Workspace) => Some(SkillScope::Global),
                    Some(SkillScope::Global) => None,
                };
                dialog.selected = 0;
                self.request_selected_skill(dialog);
            }
            KeyCode::Char('s') => {
                dialog.target_scope = match dialog.target_scope {
                    SkillScope::Global => SkillScope::Workspace,
                    SkillScope::Workspace => SkillScope::Global,
                };
            }
            KeyCode::Char('n') => dialog.open_editor(
                "Create skill",
                &format!("Create in {}. Use a lowercase name with digits and single hyphens, up to 64 characters.", scope_label(dialog.target_scope)),
                false, String::new(), SkillEditAction::CreateName,
            ),
            KeyCode::Char('i') => dialog.open_editor(
                "Install skills",
                "Enter a GitHub URL, archive URL, or server folder/archive path. Prefix a local folder or archive with @ to upload from this computer.",
                false, String::new(), SkillEditAction::InstallSource,
            ),
            KeyCode::Char('e') => self.open_skill_editor(dialog),
            KeyCode::Char('f') => {
                if let Some(detail) = dialog.editable_detail() {
                    let id = detail.summary.id.clone();
                    dialog.open_editor("New resource", "Relative path, such as references/guide.md or scripts/check.py.", false, String::new(), SkillEditAction::ResourceName { id });
                }
            }
            KeyCode::Char(' ') => {
                if let Some(detail) = dialog.editable_detail() {
                    self.submit_skill_mutation(dialog, SkillMutation::Enabled {
                        id: detail.summary.id.clone(),
                        enabled: !detail.summary.enabled,
                        revision: detail.summary.revision.clone(),
                    });
                }
            }
            KeyCode::Char('d') | KeyCode::Delete => self.confirm_skill_removal(dialog),
            KeyCode::Char('r') => self.request_skills_refresh(dialog),
            KeyCode::Char('u') => self.preview_skill_source_update(dialog),
            KeyCode::Char('c') => self.preview_skill_copy(dialog),
            _ => {}
        }
        false
    }

    fn move_skill_selection(&mut self, dialog: &mut SkillsWorkbench, delta: isize) {
        if dialog.pane == SkillPane::Library {
            let count = dialog.visible_skills().len();
            let selected = move_index(dialog.selected, delta, count);
            if selected != dialog.selected {
                dialog.selected = selected;
                self.request_selected_skill(dialog);
            }
        } else {
            let count = dialog.file_paths().len();
            let selected = move_index(dialog.file_index, delta, count);
            if selected != dialog.file_index {
                dialog.file_index = selected;
                self.request_selected_skill_resource(dialog);
            }
        }
    }

    fn submit_skill_editor(
        &mut self,
        dialog: &mut SkillsWorkbench,
        action: SkillEditAction,
        text: String,
    ) {
        dialog.error = None;
        match action {
            SkillEditAction::Search => {
                dialog.query = text.trim().to_owned();
                dialog.selected = 0;
                dialog.editor = None;
                self.request_selected_skill(dialog);
            }
            SkillEditAction::CreateName => {
                let name = text.trim();
                if !valid_skill_name(name) {
                    dialog.error = Some("Use 1–64 lowercase letters, digits, and single hyphens; do not start or end with a hyphen.".to_owned());
                    return;
                }
                let document = format!(
                    "---\nname: {name}\ndescription: Describe when this skill should be used.\n---\n\n# {name}\n\nWrite the instructions, examples, and references for this skill.\n"
                );
                dialog.open_editor(
                    "Create SKILL.md",
                    "Edit the YAML metadata and instructions before saving.",
                    true,
                    document,
                    SkillEditAction::CreateDocument(dialog.target_scope),
                );
            }
            SkillEditAction::CreateDocument(scope) => {
                self.submit_skill_mutation(
                    dialog,
                    SkillMutation::Create {
                        scope,
                        document: text,
                    },
                );
            }
            SkillEditAction::EditDocument { id, revision } => {
                self.submit_skill_mutation(
                    dialog,
                    SkillMutation::Document {
                        id,
                        document: text,
                        revision,
                    },
                );
            }
            SkillEditAction::ResourceName { id } => {
                let path = text.trim();
                if !valid_resource_path(path) {
                    dialog.error = Some("Use a relative file path inside the skill; parent paths and internal state files are not editable.".to_owned());
                    return;
                }
                if dialog.file_paths().contains(&path) {
                    dialog.error = Some(
                        "That resource exists. Select it in Files and press E to edit it."
                            .to_owned(),
                    );
                    return;
                }
                dialog.open_editor(
                    "Create resource",
                    path,
                    true,
                    String::new(),
                    SkillEditAction::Resource {
                        id,
                        path: path.to_owned(),
                        revision: None,
                    },
                );
            }
            SkillEditAction::Resource { id, path, revision } => {
                self.submit_skill_mutation(
                    dialog,
                    SkillMutation::Resource {
                        id,
                        path,
                        content: text,
                        revision,
                    },
                );
            }
            SkillEditAction::InstallSource => self.preview_skill_install_input(dialog, text),
        }
    }

    fn open_skill_editor(&mut self, dialog: &mut SkillsWorkbench) {
        let Some(detail) = dialog.editable_detail() else {
            return;
        };
        if dialog.pane == SkillPane::Library || dialog.selected_file() == Some("SKILL.md") {
            dialog.open_editor(
                "Edit SKILL.md",
                "Update metadata and instructions. The skill name must match its directory.",
                true,
                detail.document.clone(),
                SkillEditAction::EditDocument {
                    id: detail.summary.id.clone(),
                    revision: detail.summary.revision.clone(),
                },
            );
        } else if let Some(resource) = dialog.resource.as_ref() {
            dialog.open_editor(
                "Edit resource",
                &resource.path.clone(),
                true,
                resource.content.clone(),
                SkillEditAction::Resource {
                    id: detail.summary.id.clone(),
                    path: resource.path.clone(),
                    revision: Some(resource.revision.clone()),
                },
            );
        } else {
            dialog.error = Some("Load the selected text resource before editing it.".to_owned());
        }
    }

    fn confirm_skill_removal(&mut self, dialog: &mut SkillsWorkbench) {
        let Some(detail) = dialog.editable_detail() else {
            return;
        };
        let (title, body, action) = if dialog.pane == SkillPane::Files {
            if dialog.selected_file() == Some("SKILL.md") {
                dialog.error = Some("SKILL.md cannot be removed separately. Focus the library and delete the skill to remove its directory.".to_owned());
                return;
            }
            let Some(path) = dialog.selected_file().map(str::to_owned) else {
                dialog.error = Some("Select a resource to delete.".to_owned());
                return;
            };
            (
                "Delete resource?",
                format!("Remove {} from {}?", path, detail.summary.name),
                SkillConfirmAction::DeleteResource {
                    id: detail.summary.id.clone(),
                    path,
                    revision: detail.summary.revision.clone(),
                },
            )
        } else {
            (
                "Delete skill?",
                format!(
                    "Remove {} and all of its files from {}?",
                    detail.summary.name,
                    scope_label(detail.summary.scope)
                ),
                SkillConfirmAction::DeleteSkill {
                    id: detail.summary.id.clone(),
                    revision: detail.summary.revision.clone(),
                },
            )
        };
        dialog.confirm = Some(ConfirmDialogState::new(
            title.to_owned(), vec![body, "This removes the installed files. It does not delete the original installation source.".to_owned()],
            "Enter / Y delete · Esc cancel".to_owned(), action,
        ));
    }

    pub(super) fn submit_skill_mutation(
        &mut self,
        dialog: &mut SkillsWorkbench,
        mutation: SkillMutation,
    ) {
        let instance = dialog.instance;
        let workspace_id = dialog.workspace_id;
        let request = dialog.begin_operation();
        self.dispatch_backend_operation(
            move |application| async move {
                let client = application.client();
                Ok::<_, anyhow::Error>(match mutation {
                    SkillMutation::Create { scope, document } => SkillMutationResult::detail(
                        client.create_skill(workspace_id, scope, &document).await?,
                        "Skill created.",
                    ),
                    SkillMutation::Document {
                        id,
                        document,
                        revision,
                    } => SkillMutationResult::detail(
                        client
                            .update_skill(workspace_id, &id, &document, &revision)
                            .await?,
                        "Skill saved.",
                    ),
                    SkillMutation::Enabled {
                        id,
                        enabled,
                        revision,
                    } => SkillMutationResult::detail(
                        client
                            .set_skill_enabled(workspace_id, &id, enabled, &revision)
                            .await?,
                        if enabled {
                            "Skill enabled."
                        } else {
                            "Skill disabled."
                        },
                    ),
                    SkillMutation::Delete { id, revision } => {
                        client.delete_skill(workspace_id, &id, &revision).await?;
                        SkillMutationResult::message("Skill removed.")
                    }
                    SkillMutation::Resource {
                        id,
                        path,
                        content,
                        revision,
                    } => {
                        client
                            .write_skill_resource(
                                workspace_id,
                                &id,
                                &path,
                                &content,
                                revision.as_deref(),
                            )
                            .await?;
                        SkillMutationResult::message("Resource saved.")
                    }
                    SkillMutation::DeleteResource { id, path, revision } => {
                        SkillMutationResult::detail(
                            client
                                .delete_skill_resource(workspace_id, &id, &path, &revision)
                                .await?,
                            "Resource removed.",
                        )
                    }
                    SkillMutation::Install {
                        scope,
                        source,
                        selected,
                        replace,
                        source_revision,
                    } => {
                        let result = client
                            .install_skills(
                                workspace_id,
                                scope,
                                &source,
                                &selected,
                                replace,
                                &source_revision,
                            )
                            .await?;
                        let message = format!(
                            "Installed {} skill(s) in {}.",
                            result.installed.len(),
                            scope_label(scope)
                        );
                        let detail = if let Some(skill) = result.installed.first() {
                            client.get_skill(workspace_id, &skill.id).await.ok()
                        } else {
                            None
                        };
                        SkillMutationResult { message, detail }
                    }
                })
            },
            move |app, result| {
                app.with_skills_workbench(instance, |app, dialog| {
                    if dialog.operation_request == request {
                        app.finish_skill_mutation(dialog, result);
                    }
                });
            },
        );
    }
}

impl SkillsWorkbench {
    pub(super) fn open_editor(
        &mut self,
        title: &str,
        prompt: &str,
        multiline: bool,
        text: String,
        action: SkillEditAction,
    ) {
        self.error = None;
        self.editor_original = text.clone();
        self.editor = Some(EditorDialogState::new(
            title.to_owned(),
            prompt.to_owned(),
            if multiline {
                "Ctrl+S save · Enter newline · Esc cancel"
            } else {
                "Enter continue · Esc cancel"
            }
            .to_owned(),
            multiline,
            Editor::from_text(text),
            action,
        ));
    }

    fn editable_detail(&mut self) -> Option<agena_api::skills::SkillDetail> {
        match self.detail.as_ref() {
            Some(detail) if detail.summary.editable => Some(detail.clone()),
            Some(_) => {
                self.error = Some("This discovered skill is read-only. Press C to create a managed copy in the selected target scope.".to_owned());
                None
            }
            None => {
                self.error = Some("Select a skill and wait for its details to load.".to_owned());
                None
            }
        }
    }
}

fn move_index(index: usize, delta: isize, count: usize) -> usize {
    index
        .saturating_add_signed(delta)
        .min(count.saturating_sub(1))
}

fn valid_skill_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && !name.starts_with('-')
        && !name.ends_with('-')
        && !name.contains("--")
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn valid_resource_path(path: &str) -> bool {
    !path.is_empty()
        && path != "SKILL.md"
        && !path.contains('\\')
        && path.split('/').all(|component| {
            !component.is_empty()
                && component != "."
                && component != ".."
                && component != ".agena-skill.json"
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_create_names_and_resource_paths_before_opening_editors() {
        for name in ["a", "rust-tests", "v2-tools"] {
            assert!(valid_skill_name(name));
        }
        for name in ["", "../x", "Uppercase", "two--hyphens", "-start", "end-"] {
            assert!(!valid_skill_name(name));
        }
        assert!(valid_resource_path("references/guide.md"));
        assert!(valid_resource_path("scripts/run.py"));
        for path in [
            "",
            "/tmp/other",
            "../other",
            "a/../b",
            "SKILL.md",
            ".agena-skill.json",
            "x\\y",
        ] {
            assert!(!valid_resource_path(path));
        }
    }
}
