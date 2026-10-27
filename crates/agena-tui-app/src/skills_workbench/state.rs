use std::{cell::Cell, collections::BTreeSet, sync::Arc};

use agena_api::skills::{
    SkillCatalog, SkillDetail, SkillInstallPreview, SkillInstallSource, SkillResource, SkillScope,
    SkillSummary,
};
use agena_tui_components::{ConfirmDialogState, EditorDialogState};
use portable_atomic::{AtomicU64, Ordering};

static NEXT_WORKBENCH_INSTANCE: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SkillPane {
    Library,
    Files,
}

#[derive(Debug, Clone)]
pub(crate) struct SkillsWorkbench {
    pub(super) instance: u64,
    pub(super) workspace_id: i64,
    pub(super) catalog_request: u64,
    pub(super) detail_request: u64,
    pub(super) resource_request: u64,
    pub(super) operation_request: u64,
    pub(super) catalog: SkillCatalog,
    pub(super) selected: usize,
    pub(super) query: String,
    pub(super) scope_filter: Option<SkillScope>,
    pub(super) target_scope: SkillScope,
    pub(super) pane: SkillPane,
    pub(super) compact_layout: Cell<bool>,
    pub(super) detail: Option<SkillDetail>,
    pub(super) file_index: usize,
    pub(super) resource: Option<SkillResource>,
    pub(super) scroll: u16,
    pub(super) loading: bool,
    pub(super) detail_loading: bool,
    pub(super) resource_loading: bool,
    pub(super) busy: bool,
    pub(super) error: Option<String>,
    pub(crate) editor: Option<EditorDialogState<SkillEditAction>>,
    pub(super) editor_original: String,
    pub(super) confirm: Option<ConfirmDialogState<SkillConfirmAction>>,
    pub(super) installation: Option<SkillInstallation>,
}

#[derive(Debug, Clone)]
pub(crate) enum SkillEditAction {
    Search,
    CreateName,
    CreateDocument(SkillScope),
    EditDocument {
        id: String,
        revision: String,
    },
    ResourceName {
        id: String,
    },
    Resource {
        id: String,
        path: String,
        revision: Option<String>,
    },
    InstallSource,
}

#[derive(Debug, Clone)]
pub(super) enum SkillConfirmAction {
    DiscardEditor,
    DeleteSkill {
        id: String,
        revision: String,
    },
    DeleteResource {
        id: String,
        path: String,
        revision: String,
    },
    Install,
}

#[derive(Debug, Clone)]
pub(super) struct SkillInstallation {
    pub(super) scope: SkillScope,
    pub(super) source: Arc<SkillInstallSource>,
    pub(super) preview: SkillInstallPreview,
    pub(super) selected: BTreeSet<String>,
    pub(super) cursor: usize,
    pub(super) replace: bool,
}

impl SkillsWorkbench {
    pub(super) fn new(workspace_id: i64) -> Self {
        Self {
            instance: NEXT_WORKBENCH_INSTANCE.fetch_add(1, Ordering::Relaxed),
            workspace_id,
            catalog_request: 0,
            detail_request: 0,
            resource_request: 0,
            operation_request: 0,
            catalog: SkillCatalog {
                skills: Vec::new(),
                diagnostics: Vec::new(),
                roots: Vec::new(),
            },
            selected: 0,
            query: String::new(),
            scope_filter: None,
            target_scope: SkillScope::Workspace,
            pane: SkillPane::Library,
            compact_layout: Cell::new(false),
            detail: None,
            file_index: 0,
            resource: None,
            scroll: 0,
            loading: false,
            detail_loading: false,
            resource_loading: false,
            busy: false,
            error: None,
            editor: None,
            editor_original: String::new(),
            confirm: None,
            installation: None,
        }
    }

    pub(super) fn visible_skills(&self) -> Vec<&SkillSummary> {
        let query = self.query.to_lowercase();
        self.catalog
            .skills
            .iter()
            .filter(|skill| {
                self.scope_filter.is_none_or(|scope| scope == skill.scope)
                    && (query.is_empty()
                        || skill.name.to_lowercase().contains(&query)
                        || skill.description.to_lowercase().contains(&query)
                        || skill.path.to_lowercase().contains(&query))
            })
            .collect()
    }

    pub(super) fn selected_skill(&self) -> Option<&SkillSummary> {
        self.visible_skills().get(self.selected).copied()
    }

    pub(super) fn reselect(&mut self, id: Option<&str>) {
        let visible = self.visible_skills();
        self.selected = id
            .and_then(|id| visible.iter().position(|skill| skill.id == id))
            .unwrap_or(self.selected.min(visible.len().saturating_sub(1)));
    }

    pub(super) fn upsert_summary(&mut self, skill: SkillSummary) {
        if let Some(current) = self
            .catalog
            .skills
            .iter_mut()
            .find(|current| current.id == skill.id)
        {
            *current = skill;
        } else {
            self.catalog.skills.push(skill);
        }
    }

    pub(super) fn install_detail(&mut self, detail: SkillDetail) {
        self.upsert_summary(detail.summary.clone());
        self.resource = Some(SkillResource {
            path: "SKILL.md".to_owned(),
            content: detail.document.clone(),
            revision: detail.summary.revision.clone(),
        });
        self.detail = Some(detail);
        self.file_index = 0;
        self.scroll = 0;
        self.detail_loading = false;
        self.resource_loading = false;
    }

    pub(super) fn file_paths(&self) -> Vec<&str> {
        let Some(detail) = &self.detail else {
            return Vec::new();
        };
        std::iter::once("SKILL.md")
            .chain(
                detail
                    .files
                    .iter()
                    .map(|file| file.path.as_str())
                    .filter(|path| *path != "SKILL.md"),
            )
            .collect()
    }

    pub(super) fn selected_file(&self) -> Option<&str> {
        self.file_paths().get(self.file_index).copied()
    }

    pub(super) fn begin_catalog_request(&mut self) -> u64 {
        self.catalog_request = self.catalog_request.wrapping_add(1);
        self.loading = true;
        self.error = None;
        self.catalog_request
    }

    pub(super) fn begin_operation(&mut self) -> u64 {
        self.operation_request = self.operation_request.wrapping_add(1);
        self.catalog_request = self.catalog_request.wrapping_add(1);
        self.detail_request = self.detail_request.wrapping_add(1);
        self.resource_request = self.resource_request.wrapping_add(1);
        self.loading = false;
        self.detail_loading = false;
        self.resource_loading = false;
        self.busy = true;
        self.error = None;
        self.operation_request
    }

    pub(crate) fn paste(&mut self, text: &str) {
        if !self.busy
            && self.confirm.is_none()
            && let Some(editor) = &mut self.editor
        {
            editor.input.insert_str(text);
        }
    }

    pub(crate) fn accepts_text_input(&self) -> bool {
        !self.busy && self.confirm.is_none() && self.editor.is_some()
    }

    pub(crate) fn focus_list(&mut self, panel: usize) {
        if self.editor.is_none()
            && self.confirm.is_none()
            && self.installation.is_none()
            && !self.compact_layout.get()
        {
            self.pane = if panel == 0 {
                SkillPane::Library
            } else {
                SkillPane::Files
            };
        }
    }
}

pub(super) fn scope_label(scope: SkillScope) -> &'static str {
    match scope {
        SkillScope::Global => "Global",
        SkillScope::Workspace => "Workspace",
    }
}

pub(super) fn status_label(skill: &SkillSummary) -> &'static str {
    if !skill.valid {
        "Invalid"
    } else if !skill.enabled {
        "Disabled"
    } else if skill.shadowed {
        "Shadowed"
    } else {
        "Enabled"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn skill(id: &str, name: &str, scope: &str) -> SkillSummary {
        serde_json::from_value(json!({
            "id": id, "name": name, "description": "Useful deployment instructions",
            "scope": scope, "path": format!("/skills/{name}"), "editable": true,
            "enabled": true, "valid": true, "shadowed": false, "revision": "revision",
            "source": null, "diagnostics": []
        }))
        .unwrap()
    }

    #[test]
    fn filters_and_reselects_by_identity_instead_of_row_number() {
        let mut state = SkillsWorkbench::new(1);
        state.catalog.skills = vec![
            skill("a", "alpha", "global"),
            skill("b", "beta", "workspace"),
        ];
        state.reselect(Some("b"));
        assert_eq!(state.selected_skill().unwrap().id, "b");
        state.scope_filter = Some(SkillScope::Workspace);
        state.reselect(Some("b"));
        assert_eq!(state.selected, 0);
        state.query = "DEPLOYMENT".to_owned();
        assert_eq!(state.visible_skills().len(), 1);
        state.query = "missing".to_owned();
        state.reselect(Some("b"));
        assert!(state.selected_skill().is_none());
    }

    #[test]
    fn reopening_uses_a_distinct_instance_for_async_response_guards() {
        assert_ne!(
            SkillsWorkbench::new(1).instance,
            SkillsWorkbench::new(1).instance
        );
    }
}
