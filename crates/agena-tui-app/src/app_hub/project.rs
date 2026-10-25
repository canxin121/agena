use super::*;
use crate::{Editor, InputDialogKeyResult, LineInputOverlay, Overlay, drive_input_dialog_key};
use agena_api::resource::WorkspaceResource;
use crossterm::event::{KeyCode, KeyModifiers};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub(crate) struct HubProjectDialog {
    pub(crate) editor: LineInputOverlay,
    pub(crate) submitting: bool,
    pub(crate) error: Option<String>,
    request_id: u64,
}

/// Paths belong to the server. Resolve relative paths against its known
/// launch workspace, without reading the client's filesystem or environment.
fn project_path(root: &Path, input: &str) -> Result<String, &'static str> {
    let input = input.trim();
    if input.is_empty() {
        return Err("hub-project-path-required");
    }
    if input.chars().any(char::is_control) {
        return Err("hub-project-path-single-line");
    }
    if input.starts_with('~') {
        return Err("hub-project-path-home");
    }
    let path = PathBuf::from(input);
    let windows_absolute = input.as_bytes().get(1) == Some(&b':')
        && input
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphabetic)
        && matches!(input.as_bytes().get(2), Some(b'/' | b'\\'));
    if path.is_absolute() || windows_absolute || input.starts_with("\\\\") {
        Ok(input.to_owned())
    } else {
        Ok(root.join(path).to_string_lossy().into_owned())
    }
}

impl App {
    pub(crate) fn open_hub_project_dialog(&mut self) {
        self.next_hub_request_id = self.next_hub_request_id.saturating_add(1);
        self.overlay = Some(Overlay::HubProject(HubProjectDialog {
            editor: self.build_line_input_overlay(
                self.i18n.text("hub-project-title"),
                self.i18n.text_args(
                    "hub-project-prompt",
                    &agena_tui::fl_args!(
                        "root" => self.application.workspace_root().display().to_string()
                    ),
                ),
                Editor::default(),
            ),
            submitting: false,
            error: None,
            request_id: self.next_hub_request_id,
        }));
    }

    pub(crate) fn handle_hub_project_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut HubProjectDialog,
    ) -> bool {
        if key.code == KeyCode::Esc && key.modifiers == KeyModifiers::NONE {
            return true;
        }
        if dialog.submitting {
            return false;
        }
        match drive_input_dialog_key(&mut dialog.editor, key) {
            InputDialogKeyResult::Close => true,
            InputDialogKeyResult::Continue => {
                dialog.error = None;
                false
            }
            InputDialogKeyResult::Submit(_, input) => {
                let path = match project_path(self.application.workspace_root(), &input) {
                    Ok(path) => path,
                    Err(key) => {
                        dialog.error = Some(self.i18n.text(key));
                        return false;
                    }
                };
                self.next_hub_request_id = self.next_hub_request_id.saturating_add(1);
                dialog.request_id = self.next_hub_request_id;
                dialog.submitting = true;
                dialog.error = None;
                let request_id = dialog.request_id;
                let application = self.application.clone();
                let tx = self.tx.clone();
                let timeout_message = self.i18n.text("hub-project-timeout");
                tokio::spawn(async move {
                    let result = tokio::time::timeout(
                        std::time::Duration::from_secs(30),
                        application.add_hub_project(path),
                    )
                    .await
                    .map_err(|_| crate::UiFailure::internal(timeout_message))
                    .and_then(|result| result.map_err(crate::UiFailure::from_backend));
                    let _ = tx
                        .send(AppMessage::HubProjectAdded { request_id, result })
                        .await;
                });
                false
            }
        }
    }

    pub(crate) fn handle_hub_project_added(
        &mut self,
        request_id: u64,
        result: crate::UiResult<WorkspaceResource>,
    ) {
        if !matches!(&self.overlay, Some(Overlay::HubProject(dialog)) if dialog.request_id == request_id)
        {
            // Dismissing a submitted dialog cannot undo a server commit, but
            // its response must never close or replace a subsequently opened dialog.
            if result.is_ok()
                && let Route::Hub(state) = &mut self.current_route
            {
                state.dirty = true;
            }
            return;
        }
        let workspace = match result {
            Ok(workspace) => workspace,
            Err(error) => {
                if let Some(Overlay::HubProject(dialog)) = &mut self.overlay {
                    dialog.submitting = false;
                    dialog.error = Some(error.to_string());
                }
                return;
            }
        };
        self.overlay = None;
        if !matches!(&self.current_route, Route::Hub(state) if state.presentation.view() == HubView::Projects)
        {
            return;
        }
        let Route::Hub(mut state) = std::mem::replace(&mut self.current_route, Route::Main) else {
            return;
        };
        state.query.clear();
        state.search_active = false;
        state.search_changed_at = None;
        state.presentation.set_query("");
        state.pages.clear();
        state.pending_page = None;
        state.revealed_workspace = Some(workspace.id);
        state.pending_project = Some(workspace.id);
        self.spawn_hub_overview_request(&mut state);
        self.current_route = Route::Hub(state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_use_the_server_workspace_and_preserve_spaces_and_unicode() {
        let root = Path::new("/server/workspace");
        assert_eq!(
            project_path(root, " ../项目 space "),
            Ok("/server/workspace/../项目 space".into())
        );
        assert_eq!(
            project_path(root, " /srv/my project "),
            Ok("/srv/my project".into())
        );
        assert_eq!(
            project_path(root, "C:\\work\\project"),
            Ok("C:\\work\\project".into())
        );
    }

    #[test]
    fn invalid_inputs_cannot_register_an_unintended_path() {
        for path in ["", "   ", "a\nb", "a\0b", "~/project"] {
            assert!(project_path(Path::new("/server"), path).is_err());
        }
    }
}
