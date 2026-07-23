impl App {
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
        let (items, actions) = Self::path_browser_entries_with_root(workspace_root, dialog);
        dialog.presentation.replace_items(items);
        dialog.path_actions = actions;
    }

    pub(in crate::app) fn handle_path_browser_overlay_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut PathBrowserOverlay,
    ) -> bool {
        match agena_tui::path_browser::handle_key(&mut dialog.presentation, key) {
            agena_tui::path_browser::PathBrowserEffect::Close => true,
            agena_tui::path_browser::PathBrowserEffect::KeepOpen => false,
            agena_tui::path_browser::PathBrowserEffect::Refresh => {
                Self::refresh_path_browser_overlay_with_root(self.backend.workspace_root(), dialog);
                false
            }
            agena_tui::path_browser::PathBrowserEffect::SelectItem { key, is_dir } => {
                let Some(path) = dialog.path_actions.get(key.as_str()).cloned() else {
                    return false;
                };
                if is_dir {
                    dialog
                        .presentation
                        .input
                        .set_text(path.display().to_string());
                    Self::refresh_path_browser_overlay_with_root(
                        self.backend.workspace_root(),
                        dialog,
                    );
                    false
                } else {
                    self.commit_path_browser_selection(dialog, path)
                }
            }
            agena_tui::path_browser::PathBrowserEffect::SelectCustom { raw } => {
                let path =
                    Self::resolve_browser_input_path_with_root(self.backend.workspace_root(), &raw);
                self.commit_path_browser_selection(dialog, path)
            }
        }
    }

    pub(in crate::app) fn commit_path_browser_selection(
        &mut self,
        dialog: &PathBrowserOverlay,
        path: PathBuf,
    ) -> bool {
        match dialog.target {
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
    ) -> (
        Vec<agena_tui::path_browser::PathBrowserItem>,
        BTreeMap<String, PathBuf>,
    ) {
        let raw = dialog.presentation.input.text().trim();
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
        let mut actions = BTreeMap::new();
        if let Some(parent) = directory.parent() {
            let path = parent.to_path_buf();
            let key = format!("path:{}", actions.len());
            actions.insert(key.clone(), path.clone());
            entries.push(agena_tui::path_browser::PathBrowserItem {
                key,
                label: "../".to_string(),
                detail: parent.display().to_string(),
                is_dir: true,
            });
        }
        let Ok(read_dir) = fs::read_dir(&directory) else {
            return (entries, actions);
        };

        let mut children = read_dir
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| {
                let path = entry.path();
                let is_dir = path.is_dir();
                if matches!(
                    dialog.presentation.meta.mode,
                    PathBrowserMode::DirectoryOnly
                ) && !is_dir
                {
                    return None;
                }
                let name = entry.file_name();
                let name = name.to_string_lossy().to_string();
                if !needle.is_empty() && !name.to_ascii_lowercase().contains(needle.as_str()) {
                    return None;
                }
                Some((path, is_dir, name))
            })
            .collect::<Vec<_>>();
        children.sort_by(|left, right| {
            (!left.1, left.2.to_ascii_lowercase()).cmp(&(!right.1, right.2.to_ascii_lowercase()))
        });
        for (path, is_dir, name) in children {
            let key = format!("path:{}", actions.len());
            actions.insert(key.clone(), path.clone());
            entries.push(agena_tui::path_browser::PathBrowserItem {
                key,
                label: if is_dir {
                    format!("{name}/")
                } else {
                    name.clone()
                },
                detail: path.display().to_string(),
                is_dir,
            });
        }
        (entries, actions)
    }
}
use crate::app::{
    App, BTreeMap, KeyEvent, Overlay, Path, PathBrowserMode, PathBrowserOverlay, PathBrowserTarget,
    PathBuf, PermissionRuleStudioOverlay, PermissionRuleStudioPathField, Route, fs,
    permission_rule_path_browser_spec, refresh_permission_rule_studio_dialog, ui_text,
};
