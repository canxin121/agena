use std::path::PathBuf;

use agena_domain::{
    ExternalAgentAuth, ExternalAgentAuthFlow, ExternalAgentAuthStatus, ExternalAgentProviders,
    ExternalAgentSessionResult as SessionResult,
};
use agena_tui_components::{EditorDialogKeyResult, EditorDialogState, drive_editor_dialog_key};
use crossterm::event::KeyEvent;

use super::{
    AuthAction, NativeAgentChoice, NativeAgentEdit, NativeAgentEditKind, NativeAgentPosition,
    SessionAction, menu_row,
};
use crate::{App, Editor, Overlay, UiFailure, ui_text};

impl App {
    pub(crate) fn open_native_agent_management(&mut self, session_id: i64) {
        if self.current_or_selected_session_id() != Some(session_id) {
            return;
        }
        self.native_agent_request(
            "external-agent-manage",
            None,
            move |backend| async move {
                Ok(backend
                    .client()
                    .external_agent_session(session_id, SessionAction::Status)
                    .await?)
            },
            move |app, result| app.show_native_management(session_id, result),
        );
    }

    pub(super) fn show_native_management(&mut self, session_id: i64, result: SessionResult) {
        let SessionResult::Status {
            session,
            native_status,
            additional_directories,
            capabilities,
            fork_points,
            rewind_points,
        } = result
        else {
            self.flash_error(UiFailure::message("Expected a native ACP session status."));
            return;
        };
        let mut rows = Vec::new();
        let mut description = self.i18n.text("external-agent-not-started");
        if let Some(session) = session {
            description = session.native_id;
            if let Some(status) = native_status {
                description.push_str(&format!(" · {status}"));
            }
            if !capabilities.continue_conversation {
                description.push('\n');
                description.push_str(&self.i18n.text("external-agent-continuation-unavailable"));
            }
            if capabilities.title {
                let initial = session
                    .title
                    .or_else(|| self.current_or_selected_session_title())
                    .unwrap_or_default();
                rows.push(self.native_agent_row(
                    "external-agent-rename",
                    NativeAgentChoice::Edit {
                        session_id,
                        kind: NativeAgentEditKind::Title,
                        initial,
                    },
                ));
            }
            if capabilities.fork {
                rows.push(self.native_agent_row(
                    "external-agent-fork",
                    NativeAgentChoice::Change {
                        session_id,
                        action: SessionAction::Fork {
                            title: None,
                            before_message_id: None,
                        },
                    },
                ));
            }
            for (available, points, operation) in [
                (
                    capabilities.fork_before_message,
                    fork_points,
                    NativeAgentPosition::Fork,
                ),
                (
                    capabilities.rewind_before_message,
                    rewind_points,
                    NativeAgentPosition::Rewind,
                ),
            ] {
                if available && !points.is_empty() {
                    rows.push(self.native_agent_row(
                        operation.title(),
                        NativeAgentChoice::Positions {
                            session_id,
                            points,
                            operation,
                        },
                    ));
                }
            }
            if capabilities.close {
                rows.push(self.native_agent_row(
                    "external-agent-close",
                    NativeAgentChoice::Change {
                        session_id,
                        action: SessionAction::Close,
                    },
                ));
            }
            if capabilities.delete {
                rows.push(self.native_agent_row(
                    "external-agent-delete",
                    NativeAgentChoice::Confirm {
                        session_id,
                        action: SessionAction::Delete,
                        title: "external-agent-delete",
                        prompt: "external-agent-delete-prompt",
                        label: None,
                    },
                ));
            }
        }
        if capabilities.additional_directories || !additional_directories.is_empty() {
            let initial = additional_directories
                .iter()
                .map(|path| path.to_string_lossy())
                .collect::<Vec<_>>()
                .join("\n");
            rows.push(self.native_agent_row(
                "external-agent-directories",
                NativeAgentChoice::Edit {
                    session_id,
                    kind: NativeAgentEditKind::Directories,
                    initial,
                },
            ));
        }
        rows.push(self.native_agent_row(
            "external-agent-providers",
            NativeAgentChoice::Providers(session_id),
        ));
        rows.push(self.native_agent_row(
            "external-agent-refresh",
            NativeAgentChoice::Manage(session_id),
        ));
        self.open_native_agent_menu("external-agent-manage", description, rows, None);
    }

    pub(super) fn open_native_agent_providers(&mut self, session_id: i64) {
        if self.current_or_selected_session_id() != Some(session_id) {
            return;
        }
        self.native_agent_request(
            "external-agent-providers",
            None,
            move |backend| async move {
                Ok(backend
                    .client()
                    .external_agent_providers(session_id)
                    .await?)
            },
            move |app, providers| app.show_native_providers(session_id, providers),
        );
    }

    pub(super) fn show_native_providers(
        &mut self,
        session_id: i64,
        providers: ExternalAgentProviders,
    ) {
        let mut description = if !providers.supported {
            self.i18n.text("external-agent-providers-unsupported")
        } else if providers.providers.is_empty() {
            self.i18n.text("external-agent-providers-empty")
        } else {
            providers
                .providers
                .iter()
                .map(|provider| {
                    let mut header = provider.provider_id.clone();
                    if provider.required {
                        header.push_str(&format!(
                            " ({})",
                            self.i18n.text("external-agent-provider-required")
                        ));
                    }
                    let target = match &provider.current {
                        Some(current) => format!("{} · {}", current.api_type, current.base_url),
                        None => self.i18n.text("external-agent-provider-unconfigured"),
                    };
                    if provider.supported.is_empty() {
                        format!("{header}: {target}")
                    } else {
                        format!(
                            "{header} [{}]: {target}",
                            provider.supported.join(", ")
                        )
                    }
                })
                .collect::<Vec<_>>()
                .join("\n")
        };
        if description.is_empty() {
            description = self.i18n.text("external-agent-providers-empty");
        }
        let rows = vec![self.native_agent_row(
            "external-agent-refresh",
            NativeAgentChoice::Providers(session_id),
        )];
        self.open_native_agent_menu("external-agent-providers", description, rows, None);
    }

    pub(crate) fn change_native_management(
        &mut self,
        session_id: i64,
        action: SessionAction,
        fallback: Option<Overlay>,
    ) {
        let request = self.begin_native_agent_request("external-agent-manage");
        self.dispatch_backend_operation(
            move |backend| async move {
                backend
                    .client()
                    .external_agent_session(session_id, action)
                    .await
            },
            move |app, result| {
                let show = app.finish_native_agent_request(request);
                match result {
                    Ok(result) => app.apply_native_management_change(session_id, result, show),
                    Err(error) if show => app.restore_native_agent_menu(fallback, error),
                    Err(error) => app.flash_error(error),
                }
            },
        );
    }

    fn apply_native_management_change(
        &mut self,
        session_id: i64,
        result: SessionResult,
        show: bool,
    ) {
        match result {
            SessionResult::Forked { session_id } if show => {
                let title = ui_text::session_fallback_title(&self.i18n, session_id);
                self.enter_native_view(session_id, title);
            }
            SessionResult::Rewound {
                session_id: rewound,
            } => {
                self.transcript_cache.remove(&rewound);
                if show || self.transcript.session_id == Some(session_id) {
                    let title = self
                        .current_or_selected_session_title()
                        .unwrap_or_else(|| ui_text::session_fallback_title(&self.i18n, rewound));
                    if show {
                        self.enter_native_view(rewound, title);
                    } else {
                        self.open_session(rewound, title);
                    }
                }
            }
            SessionResult::Deleted => {
                if show {
                    self.clear_native_agent_overlays();
                }
                self.leave_deleted_native_view(session_id);
                self.flash_info(self.i18n.text("external-agent-deleted"));
            }
            SessionResult::Title { .. }
            | SessionResult::Directories { .. }
            | SessionResult::Closed => {
                self.request_refresh(session_id, true);
                if show {
                    self.clear_native_agent_overlays();
                    self.open_native_agent_management(session_id);
                }
            }
            SessionResult::Forked { .. } => {}
            _ => self.flash_error(UiFailure::message("Unexpected native ACP session result.")),
        }
    }

    pub(super) fn edit_native_management(
        &mut self,
        session_id: i64,
        kind: NativeAgentEditKind,
        initial: String,
    ) {
        let multiline = matches!(kind, NativeAgentEditKind::Directories);
        let (title, prompt, footer) = if multiline {
            (
                "external-agent-directories",
                "external-agent-directories-prompt",
                "overlay-editor-footer-multiline",
            )
        } else {
            (
                "external-agent-rename",
                "external-agent-rename-prompt",
                "overlay-line-footer",
            )
        };
        self.overlay = Some(Overlay::NativeAgentEdit(Box::new(EditorDialogState::new(
            self.i18n.text(title),
            self.i18n.text(prompt),
            self.i18n.text(footer),
            multiline,
            Editor::from_text(initial),
            NativeAgentEdit { session_id, kind },
        ))));
    }

    pub(crate) fn handle_native_agent_edit(
        &mut self,
        key: KeyEvent,
        editor: &mut EditorDialogState<NativeAgentEdit>,
    ) -> bool {
        match drive_editor_dialog_key(editor, key) {
            EditorDialogKeyResult::Continue => false,
            EditorDialogKeyResult::Close => true,
            EditorDialogKeyResult::Submit(edit, value) => {
                let action = match edit.kind {
                    NativeAgentEditKind::Title if value.trim().is_empty() => {
                        self.flash_warning(self.i18n.text("external-agent-title-required"));
                        return false;
                    }
                    NativeAgentEditKind::Title => SessionAction::SetTitle {
                        title: value.trim().to_owned(),
                    },
                    NativeAgentEditKind::Directories => SessionAction::SetDirectories {
                        directories: value
                            .lines()
                            .map(str::trim)
                            .filter(|line| !line.is_empty())
                            .map(PathBuf::from)
                            .collect(),
                    },
                };
                self.change_native_management(
                    edit.session_id,
                    action,
                    Some(Overlay::NativeAgentEdit(Box::new(editor.clone()))),
                );
                true
            }
        }
    }

    pub(crate) fn open_native_agent_auth(&mut self, session_id: i64) {
        self.request_native_auth(session_id, AuthAction::Inspect { terminal: false });
    }

    pub(crate) fn request_native_auth(&mut self, session_id: i64, action: AuthAction) {
        if self.current_or_selected_session_id() != Some(session_id) {
            return;
        }
        if matches!(action, AuthAction::Login { .. }) {
            // Login can request input through the existing ACP interaction feed.
            self.clear_native_agent_overlays();
            self.flash_info(self.i18n.text("external-agent-auth-started"));
            self.dispatch_backend_operation(
                move |backend| async move {
                    backend
                        .client()
                        .external_agent_auth(session_id, action)
                        .await
                },
                move |app, result| match result {
                    Ok(auth)
                        if app.current_or_selected_session_id() == Some(session_id)
                            && app.overlay.is_none()
                            && app.current_route_is_main() =>
                    {
                        app.show_native_auth(session_id, auth)
                    }
                    Ok(_) => app.flash_info(app.i18n.text("external-agent-auth-completed")),
                    Err(error) => app.flash_error(error),
                },
            );
        } else {
            self.native_agent_request(
                "external-agent-auth",
                None,
                move |backend| async move {
                    let client = backend.client();
                    let auth = client.external_agent_auth(session_id, action).await?;
                    let live = client
                        .external_agent_live_auth(session_id)
                        .await
                        .ok()
                        .flatten();
                    Ok((auth, live))
                },
                move |app, (auth, live)| app.show_native_auth_with_live(session_id, auth, live),
            );
        }
    }

    pub(super) fn show_native_auth(&mut self, session_id: i64, auth: ExternalAgentAuth) {
        self.show_native_auth_with_live(session_id, auth, None);
    }

    pub(super) fn show_native_auth_with_live(
        &mut self,
        session_id: i64,
        auth: ExternalAgentAuth,
        live: Option<ExternalAgentAuthStatus>,
    ) {
        let state_str = live
            .as_ref()
            .map(|status| status.state.as_str())
            .or(auth.state.as_deref());
        let state = self.i18n.text(match state_str {
            Some("authenticated") => "external-agent-auth-signed-in",
            Some("unauthenticated") => "external-agent-auth-required",
            Some("not_required") => "external-agent-auth-not-required",
            _ => "external-agent-auth-unknown",
        });
        let mut description = if live.is_some() {
            format!(
                "{} · {} · {state}",
                auth.agent.label(),
                self.i18n.text("external-agent-auth-connection")
            )
        } else {
            format!("{} · {state}", auth.agent.label())
        };
        let accounts = live
            .as_ref()
            .map(|status| status.accounts.as_slice())
            .filter(|accounts| !accounts.is_empty())
            .unwrap_or(auth.accounts.as_slice());
        for account in accounts.iter().filter(|account| account.active || live.is_some()) {
            let line = [
                Some(account.label.as_str()),
                account.name.as_deref(),
                account.email.as_deref(),
                account.organization.as_deref(),
            ]
            .into_iter()
            .flatten()
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>()
            .join(" · ");
            if !line.is_empty() {
                description.push_str(&format!("\n{line}"));
            }
            if let Some(detail) = account.description.as_deref().filter(|d| !d.is_empty()) {
                description.push_str(&format!("\n{detail}"));
            }
        }
        description.push('\n');
        description.push_str(&self.i18n.text_args(
            "external-agent-auth-terminal-help",
            &agena_tui::fl_args!("session" => session_id.to_string()),
        ));
        let mut rows = auth
            .methods
            .into_iter()
            .filter(|method| matches!(method.flow, ExternalAgentAuthFlow::Agent))
            .map(|method| {
                menu_row(
                    method.name,
                    method.description.unwrap_or_default(),
                    NativeAgentChoice::Auth {
                        session_id,
                        action: AuthAction::Login {
                            method_id: method.id,
                        },
                    },
                )
            })
            .collect::<Vec<_>>();
        if auth.logout_supported {
            rows.push(self.native_agent_row(
                "external-agent-auth-logout",
                NativeAgentChoice::Auth {
                    session_id,
                    action: AuthAction::Logout,
                },
            ));
        }
        rows.push(self.native_agent_row(
            "external-agent-refresh",
            NativeAgentChoice::Auth {
                session_id,
                action: AuthAction::Inspect { terminal: false },
            },
        ));
        self.open_native_agent_menu("external-agent-auth", description, rows, None);
    }
}
