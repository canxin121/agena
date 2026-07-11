impl App {
    pub(in crate::app) fn open_selected_permission_rule_path_browser(
        &mut self,
        dialog: &PermissionRuleStudioOverlay,
    ) {
        let Some(item) = dialog.workbench.list.selected_item() else {
            return;
        };
        match item.action {
            PermissionRuleStudioAction::BrowseWorkspaceRoot => self
                .open_permission_rule_path_browser(
                    dialog,
                    PermissionRuleStudioPathField::WorkspaceRoot,
                ),
            PermissionRuleStudioAction::BrowseTargetPath => self.open_permission_rule_path_browser(
                dialog,
                PermissionRuleStudioPathField::TargetPath,
            ),
            _ => {}
        }
    }

    pub(in crate::app) fn open_permission_rule_path_browser(
        &mut self,
        dialog: &PermissionRuleStudioOverlay,
        field: PermissionRuleStudioPathField,
    ) {
        let (title, prompt, mode, initial) =
            permission_rule_path_browser_spec(&self.i18n, &dialog.draft, field);
        self.overlay = Some(Overlay::PathBrowser(self.build_path_browser_overlay(
            title,
            prompt,
            mode,
            initial,
            PathBrowserTarget::PermissionRuleStudio(field),
        )));
    }

    pub(in crate::app) fn refresh_path_browser_overlay_with_root(
        workspace_root: &Path,
        dialog: &mut PathBrowserOverlay,
    ) {
        dialog.items = Self::path_browser_entries_with_root(workspace_root, dialog);
        dialog.clamp_selection();
    }

    pub(in crate::app) fn handle_path_browser_overlay_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut PathBrowserOverlay,
    ) -> bool {
        match resolve_tui_key(KeyContext::PathBrowser, key) {
            Some(KeyAction::Fill) => {
                dialog.fill_input_from_selected();
                false
            }
            Some(KeyAction::Back) => {
                self.path_browser_navigate_parent(dialog);
                false
            }
            Some(KeyAction::Open) => {
                self.path_browser_open_entry(dialog);
                false
            }
            Some(KeyAction::Accept) => self.path_browser_activate(dialog),
            _ => match dialog.handle_filter_input_key(key, 10) {
                SearchInputKeyResult::Close => true,
                SearchInputKeyResult::Navigated => false,
                SearchInputKeyResult::Edited { changed } => {
                    if changed {
                        Self::refresh_path_browser_overlay_with_root(
                            self.backend.workspace_root(),
                            dialog,
                        );
                    }
                    false
                }
            },
        }
    }

    pub(in crate::app) fn path_browser_activate(
        &mut self,
        dialog: &mut PathBrowserOverlay,
    ) -> bool {
        if let Some(selection) = dialog.selected_row() {
            return match selection {
                SearchListRow::Item(entry) => {
                    self.commit_path_browser_selection(dialog, entry.path)
                }
                SearchListRow::Custom(value) => {
                    let path = Self::resolve_browser_input_path_with_root(
                        self.backend.workspace_root(),
                        value.raw.as_str(),
                    );
                    self.commit_path_browser_selection(dialog, path)
                }
                SearchListRow::Clear(_) => false,
            };
        }
        let raw = dialog.input.text().trim();
        if raw.is_empty() {
            return false;
        }
        let path = Self::resolve_browser_input_path_with_root(self.backend.workspace_root(), raw);
        self.commit_path_browser_selection(dialog, path)
    }

    pub(in crate::app) fn path_browser_open_entry(&self, dialog: &mut PathBrowserOverlay) {
        if let Some(entry) = dialog.items.get(dialog.selected) {
            if entry.is_dir {
                dialog.input.set_text(entry.path.display().to_string());
                Self::refresh_path_browser_overlay_with_root(self.backend.workspace_root(), dialog);
            }
        }
    }

    pub(in crate::app) fn path_browser_navigate_parent(&self, dialog: &mut PathBrowserOverlay) {
        let current = Self::resolve_browser_input_path_with_root(
            self.backend.workspace_root(),
            dialog.input.text().trim(),
        );
        let parent = current.parent().map(Path::to_path_buf);
        if let Some(parent) = parent {
            dialog.input.set_text(parent.display().to_string());
            Self::refresh_path_browser_overlay_with_root(self.backend.workspace_root(), dialog);
        }
    }

    pub(in crate::app) fn commit_path_browser_selection(
        &mut self,
        dialog: &PathBrowserOverlay,
        path: PathBuf,
    ) -> bool {
        match dialog.meta.target {
            PathBrowserTarget::PermissionRuleStudio(field) => {
                let workspace_root = self.backend.workspace_root();
                let value = match field {
                    PermissionRuleStudioPathField::WorkspaceRoot => path.display().to_string(),
                    PermissionRuleStudioPathField::TargetPath => path
                        .strip_prefix(workspace_root)
                        .ok()
                        .map(|relative| relative.display().to_string())
                        .filter(|relative| !relative.is_empty())
                        .unwrap_or_else(|| path.display().to_string()),
                };
                match &mut self.current_route {
                    Route::PermissionRuleStudio(route) => {
                        match field {
                            PermissionRuleStudioPathField::WorkspaceRoot => {
                                route.draft.workspace_root = value;
                            }
                            PermissionRuleStudioPathField::TargetPath => {
                                route.draft.target_path = value;
                            }
                        }
                        refresh_permission_rule_studio_dialog(&self.i18n, route);
                    }
                    _ => self
                        .flash_error(ui_text::t(&self.i18n, "flash-permission-rule-context-lost")),
                }
                true
            }
        }
    }

    pub(in crate::app) fn resolve_browser_input_path_with_root(
        workspace_root: &Path,
        raw: &str,
    ) -> PathBuf {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return workspace_root.to_path_buf();
        }
        let path = PathBuf::from(trimmed);
        if path.is_absolute() {
            path
        } else {
            workspace_root.join(path)
        }
    }

    pub(in crate::app) fn path_browser_entries_with_root(
        workspace_root: &Path,
        dialog: &PathBrowserOverlay,
    ) -> Vec<PathBrowserItem> {
        let raw = dialog.input.text().trim();
        let resolved = Self::resolve_browser_input_path_with_root(workspace_root, raw);
        let (directory, needle) = if resolved.is_dir() {
            (resolved, String::new())
        } else {
            (
                resolved
                    .parent()
                    .map(Path::to_path_buf)
                    .unwrap_or_else(|| workspace_root.to_path_buf()),
                resolved
                    .file_name()
                    .and_then(|name| name.to_str())
                    .map(str::to_ascii_lowercase)
                    .unwrap_or_default(),
            )
        };

        let mut entries = Vec::new();
        if let Some(parent) = directory.parent() {
            entries.push(PathBrowserItem {
                path: parent.to_path_buf(),
                label: "../".to_string(),
                detail: parent.display().to_string(),
                is_dir: true,
            });
        }
        let Ok(read_dir) = fs::read_dir(&directory) else {
            return entries;
        };

        let mut children = read_dir
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| {
                let path = entry.path();
                let is_dir = path.is_dir();
                if matches!(dialog.meta.mode, PathBrowserMode::DirectoryOnly) && !is_dir {
                    return None;
                }
                let name = entry.file_name();
                let name = name.to_string_lossy().to_string();
                if !needle.is_empty() && !name.to_ascii_lowercase().contains(needle.as_str()) {
                    return None;
                }
                Some(PathBrowserItem {
                    label: if is_dir {
                        format!("{name}/")
                    } else {
                        name.clone()
                    },
                    detail: path.display().to_string(),
                    path,
                    is_dir,
                })
            })
            .collect::<Vec<_>>();
        children.sort_by(|left, right| {
            (!left.is_dir, left.label.to_ascii_lowercase())
                .cmp(&(!right.is_dir, right.label.to_ascii_lowercase()))
        });
        entries.extend(children);
        entries
    }
}
use crate::app::{
    App, KeyEvent, Overlay, Path, PathBrowserItem, PathBrowserMode, PathBrowserOverlay,
    PathBrowserTarget, PathBuf, PermissionRuleStudioAction, PermissionRuleStudioOverlay,
    PermissionRuleStudioPathField, Route, SearchInputKeyResult, SearchListRow, fs,
    permission_rule_path_browser_spec, refresh_permission_rule_studio_dialog, ui_text,
};
use crate::tui_keymap::{KeyAction, KeyContext, resolve as resolve_tui_key};
