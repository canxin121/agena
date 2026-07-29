impl App {
    pub(crate) fn open_permission_rule_path_browser(
        &mut self,
        dialog: &PermissionRuleStudioOverlay,
        field: PermissionRuleStudioPathField,
    ) {
        let (title, prompt, mode, initial) =
            permission_rule_path_browser_spec(&self.i18n, &dialog.draft, field);
        self.overlay = Some(Overlay::PathBrowser(self.build_path_browser_overlay(
            title,
            prompt,
            ui_text::t(&self.i18n, "overlay-permission-rule-browser-footer"),
            ui_text::t(&self.i18n, "overlay-permission-rule-browser-empty"),
            mode,
            initial,
            PathBrowserTarget::PermissionRuleStudio(field),
        )));
    }

    pub(crate) fn refresh_path_browser_overlay_with_root(
        workspace_root: &Path,
        dialog: &mut PathBrowserOverlay,
    ) {
        let (items, actions) = Self::path_browser_entries_with_root(workspace_root, dialog);
        dialog.presentation.replace_items(items);
        dialog.path_actions = actions;
    }

    pub(crate) fn handle_path_browser_overlay_key(
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
            agena_tui::path_browser::PathBrowserEffect::Parent => {
                let (directory, _) = Self::path_browser_directory_and_needle_for_overlay(
                    self.backend.workspace_root(),
                    dialog,
                );
                if let Some(parent) = directory.parent() {
                    Self::navigate_path_browser_to_with_root(
                        self.backend.workspace_root(),
                        dialog,
                        parent.to_path_buf(),
                    );
                }
                false
            }
            agena_tui::path_browser::PathBrowserEffect::EnterDirectory { key } => {
                let Some(path) = dialog.path_actions.get(key.as_str()).cloned() else {
                    return false;
                };
                if path.is_dir() {
                    Self::navigate_path_browser_to_with_root(
                        self.backend.workspace_root(),
                        dialog,
                        path,
                    );
                }
                false
            }
            agena_tui::path_browser::PathBrowserEffect::SelectItem { key, is_dir } => {
                let Some(path) = dialog.path_actions.get(key.as_str()).cloned() else {
                    return false;
                };
                if path_browser_enter_navigates(dialog.target, is_dir) {
                    Self::navigate_path_browser_to_with_root(
                        self.backend.workspace_root(),
                        dialog,
                        path,
                    );
                    false
                } else {
                    self.commit_path_browser_selection(dialog, path)
                }
            }
            agena_tui::path_browser::PathBrowserEffect::SelectCustom { raw } => {
                let path = Self::resolve_path_browser_input_with_root(
                    self.backend.workspace_root(),
                    dialog,
                    &raw,
                );
                if path_browser_enter_navigates(dialog.target, path.is_dir()) {
                    Self::navigate_path_browser_to_with_root(
                        self.backend.workspace_root(),
                        dialog,
                        path,
                    );
                    false
                } else {
                    self.commit_path_browser_selection(dialog, path)
                }
            }
        }
    }

    pub(crate) fn commit_path_browser_selection(
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
            PathBrowserTarget::FileAttachment { images_only } => {
                match self.stage_file_browser_attachment(path.as_path(), images_only) {
                    Ok(()) => true,
                    Err(error) => {
                        self.flash_error(error);
                        false
                    }
                }
            }
        }
    }

    fn navigate_path_browser_to_with_root(
        workspace_root: &Path,
        dialog: &mut PathBrowserOverlay,
        path: PathBuf,
    ) {
        dialog.current_directory = path.clone();
        dialog
            .presentation
            .input
            .set_text(path_browser_directory_input(path.as_path()));
        Self::refresh_path_browser_overlay_with_root(workspace_root, dialog);
    }

    pub(crate) fn resolve_browser_input_path_with_root(
        workspace_root: &Path,
        raw: &str,
    ) -> PathBuf {
        Self::resolve_browser_input_path_with_base(workspace_root, raw)
    }

    fn resolve_path_browser_input_with_root(
        workspace_root: &Path,
        dialog: &PathBrowserOverlay,
        raw: &str,
    ) -> PathBuf {
        let base = if matches!(dialog.target, PathBrowserTarget::FileAttachment { .. }) {
            dialog.current_directory.as_path()
        } else {
            workspace_root
        };
        Self::resolve_browser_input_path_with_base(base, raw)
    }

    fn resolve_browser_input_path_with_base(base: &Path, raw: &str) -> PathBuf {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return base.to_path_buf();
        }
        let path = PathBuf::from(trimmed);
        if path.is_absolute() {
            path
        } else {
            base.join(path)
        }
    }

    pub(crate) fn path_browser_entries_with_root(
        workspace_root: &Path,
        dialog: &PathBrowserOverlay,
    ) -> (
        Vec<agena_tui::path_browser::PathBrowserItem>,
        BTreeMap<String, PathBuf>,
    ) {
        let (directory, needle) =
            Self::path_browser_directory_and_needle_for_overlay(workspace_root, dialog);

        let mut entries = Vec::new();
        let mut actions = BTreeMap::new();
        if !matches!(dialog.target, PathBrowserTarget::FileAttachment { .. })
            && let Some(parent) = directory.parent()
        {
            let path = parent.to_path_buf();
            let key = format!("path:{}", actions.len());
            actions.insert(key.clone(), path.clone());
            entries.push(agena_tui::path_browser::PathBrowserItem {
                key,
                label: "../".to_string(),
                search_text: parent.display().to_string(),
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
                if matches!(
                    dialog.target,
                    PathBrowserTarget::FileAttachment { images_only: true }
                ) && !is_dir
                    && !path_is_likely_image(path.as_path())
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
                search_text: path.display().to_string(),
                is_dir,
            });
        }
        (entries, actions)
    }

    #[cfg(test)]
    fn path_browser_directory_and_needle_with_root(
        workspace_root: &Path,
        raw: &str,
    ) -> (PathBuf, String) {
        Self::path_browser_directory_and_needle_with_base(workspace_root, raw)
    }

    fn path_browser_directory_and_needle_for_overlay(
        workspace_root: &Path,
        dialog: &PathBrowserOverlay,
    ) -> (PathBuf, String) {
        let base = if matches!(dialog.target, PathBrowserTarget::FileAttachment { .. }) {
            dialog.current_directory.as_path()
        } else {
            workspace_root
        };
        Self::path_browser_directory_and_needle_with_base(base, dialog.presentation.input.text())
    }

    fn path_browser_directory_and_needle_with_base(base: &Path, raw: &str) -> (PathBuf, String) {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return (base.to_path_buf(), String::new());
        }
        let typed_path = Path::new(trimmed);
        // A bare name is a directory-local search term. A path expression
        // (absolute, ./, ../, or containing a separator) instead drives the
        // displayed directory directly. This keeps filename search immediate
        // while retaining a single field for direct path entry.
        if !typed_path.is_absolute()
            && !trimmed.starts_with('.')
            && !trimmed.contains('/')
            && !trimmed.contains('\\')
        {
            return (base.to_path_buf(), trimmed.to_ascii_lowercase());
        }
        let resolved = Self::resolve_browser_input_path_with_base(base, trimmed);
        if resolved.is_dir() {
            return (resolved, String::new());
        }
        (
            resolved
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| base.to_path_buf()),
            resolved
                .file_name()
                .and_then(|name| name.to_str())
                .map(str::to_ascii_lowercase)
                .unwrap_or_default(),
        )
    }
}

/// Attachment browser directories are added by `Enter`; only horizontal
/// motions navigate there. Permission-rule path editing keeps its existing
/// enter-a-directory behavior.
fn path_browser_enter_navigates(target: PathBrowserTarget, is_dir: bool) -> bool {
    matches!(target, PathBrowserTarget::PermissionRuleStudio(_)) && is_dir
}

fn path_is_likely_image(path: &Path) -> bool {
    agena_plugin_sdk::AttachmentKind::detect("", path.file_name().and_then(|name| name.to_str()))
        == agena_plugin_sdk::AttachmentKind::Image
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{
        App, PathBrowserTarget, PermissionRuleStudioPathField, path_browser_enter_navigates,
        path_is_likely_image,
    };

    #[test]
    fn browser_input_uses_existing_directories_or_filters_their_children() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let source = root.join("src");
        let source_file = source.join("app_types.rs");

        let (directory, needle) = App::path_browser_directory_and_needle_with_root(root, "./src");
        assert_eq!(directory, source);
        assert!(needle.is_empty());

        let (directory, needle) =
            App::path_browser_directory_and_needle_with_root(root, "app_types");
        assert_eq!(directory, root);
        assert_eq!(needle, "app_types");

        let (directory, needle) = App::path_browser_directory_and_needle_with_root(
            root,
            source_file.to_string_lossy().as_ref(),
        );
        assert_eq!(directory, source);
        assert_eq!(needle, "app_types.rs");
    }

    #[test]
    fn image_browser_accepts_common_image_extensions_only() {
        assert!(path_is_likely_image(Path::new("diagram.svg")));
        assert!(path_is_likely_image(Path::new("photo.JPEG")));
        assert!(!path_is_likely_image(Path::new("notes.md")));
        assert!(!path_is_likely_image(Path::new("archive.pdf")));
    }

    #[test]
    fn attachment_enter_adds_directories_while_arrows_handle_navigation() {
        assert!(!path_browser_enter_navigates(
            PathBrowserTarget::FileAttachment { images_only: false },
            true,
        ));
        assert!(!path_browser_enter_navigates(
            PathBrowserTarget::FileAttachment { images_only: true },
            true,
        ));
        assert!(path_browser_enter_navigates(
            PathBrowserTarget::PermissionRuleStudio(PermissionRuleStudioPathField::TargetPath),
            true,
        ));
        assert!(!path_browser_enter_navigates(
            PathBrowserTarget::PermissionRuleStudio(PermissionRuleStudioPathField::TargetPath),
            false,
        ));
    }
}

use crate::{
    App, BTreeMap, KeyEvent, Overlay, Path, PathBrowserMode, PathBrowserOverlay, PathBrowserTarget,
    PathBuf, PermissionRuleStudioOverlay, PermissionRuleStudioPathField, Route, fs,
    path_browser_directory_input, permission_rule_path_browser_spec,
    refresh_permission_rule_studio_dialog, ui_text,
};
