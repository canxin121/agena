impl App {
    /// Run a built-in command locally.
    ///
    /// `command` is the published declaration this client resolved the
    /// invocation to, so the usage line and summary shown to the user come from
    /// the same place the palette read them. The match on the command's action
    /// stays exhaustive over [`ClientCommandAction`]: a declaration naming an
    /// action this build does not implement never reaches here, because
    /// resolving an invocation drops it.
    pub(crate) fn execute_command(&mut self, command: &ClientCommand, args: &str) {
        let action = command.action();
        if !command.takes_arguments() && !args.trim().is_empty() {
            self.flash_warning(self.i18n.text_args(
                "flash-command-usage",
                &agena_tui::fl_args!("usage" => command.invocation()),
            ));
            return;
        }
        match action {
            ClientCommandAction::Help => {
                self.open_context_help();
            }
            ClientCommandAction::Commands => self.open_command_palette(),
            ClientCommandAction::New => self.create_session(None),
            ClientCommandAction::Sessions => self.open_resume_session_picker(),
            ClientCommandAction::Hub => self.open_hub(),
            ClientCommandAction::Lineage => self.open_lineage_picker(),
            ClientCommandAction::Rewind => self.open_rewind_messages_picker(),
            ClientCommandAction::Rename => self.open_rename_session_overlay(),
            ClientCommandAction::Favorite => {
                if !args.trim().is_empty() {
                    self.flash_warning(self.i18n.text_args(
                        "flash-command-usage",
                        &agena_tui::fl_args!("usage" => command.invocation()),
                    ));
                } else {
                    self.toggle_current_session_favorite();
                }
            }
            ClientCommandAction::Timeline => self.open_timeline_overlay(TIMELINE_EVENT_LIMIT),
            ClientCommandAction::Settings => self.open_settings_studio(),
            ClientCommandAction::Model => self.open_session_model_chooser(),
            ClientCommandAction::Commit => self.handle_commit_command(args),
            ClientCommandAction::Pr => self.handle_pr_command(args),
            ClientCommandAction::Export => self.handle_export_command(args),
            ClientCommandAction::Pager => self.pending_ui_action = Some(UiAction::PageTranscript),
            ClientCommandAction::Continue => self.continue_current_session(),
            ClientCommandAction::Compact => self.compact_current_session(),
            ClientCommandAction::UserInput => self.open_user_input_overlay(),
            ClientCommandAction::Allow => self.reply_permission(PermissionReplyKind::AllowOnce),
            ClientCommandAction::AllowAlways => {
                self.reply_permission(PermissionReplyKind::AllowAlways)
            }
            ClientCommandAction::Deny => self.reply_permission(PermissionReplyKind::DenyOnce),
            ClientCommandAction::DenyAlways => {
                self.reply_permission(PermissionReplyKind::DenyAlways)
            }
            ClientCommandAction::Attach => {
                self.focus = Focus::Composer;
                self.request_file_attachment(false);
            }
            ClientCommandAction::Download => self.request_terminal_download(args),
            ClientCommandAction::Editor => {
                self.pending_ui_action = Some(UiAction::EditComposerExternally);
            }
            ClientCommandAction::Image => {
                self.request_file_attachment(true);
            }
            ClientCommandAction::Paste => {
                self.focus = Focus::Composer;
                self.pending_ui_action = Some(UiAction::PasteClipboard);
            }
            ClientCommandAction::Copy => self.copy_loaded_transcript(),
            ClientCommandAction::CopyMessage => self.copy_last_assistant_message(),
            ClientCommandAction::CopyVisible => self.copy_visible_transcript(),
            ClientCommandAction::Fork => self.handle_fork_command(args),
            ClientCommandAction::Children => self.open_child_sessions_picker(),
            ClientCommandAction::Parent => self.open_parent_session(),
            ClientCommandAction::Diagnostics => {
                self.open_terminal_diagnostics();
            }
            ClientCommandAction::Status => {
                self.flash_success(self.current_runtime_status_summary());
            }
            ClientCommandAction::Usage => self.open_usage_dashboard(),
            ClientCommandAction::Activities => self.open_activities_panel(),
            ClientCommandAction::Background => {
                // Return to the session hub and leave the TUI. The server owns
                // the session independently of this client, so nothing is
                // stopped: the session keeps running and can be re-attached
                // from the hub next launch.
                self.open_hub();
                self.should_quit = true;
            }
            ClientCommandAction::Plan => self.open_plan_viewer(),
            ClientCommandAction::Side => self.handle_side_command(args),
        }
    }

    pub(crate) fn execute_plugin_slash_command(
        &mut self,
        entry: agena_plugin_host::CommandCatalogItem,
        args: &str,
    ) {
        let session_id = self
            .transcript
            .session_id
            .or_else(|| self.sessions.current_selected_id());
        let args = args.to_string();
        self.dispatch_backend_operation(
            move |application| async move {
                crate::app_backend::plugin_effects::invoke_plugin_slash_command(
                    &application,
                    &entry,
                    session_id,
                    &args,
                )
                .await
            },
            move |app, result| match result {
                Ok(result) => app.apply_plugin_command_result(result, session_id),
                Err(error) => app.flash_error(error),
            },
        );
    }

    fn apply_plugin_command_result(
        &mut self,
        result: PluginCommandEffect,
        session_id: Option<i64>,
    ) {
        let feedback = if result.summary.trim().is_empty() {
            result.title.clone()
        } else if result.title.trim().is_empty() {
            result.summary.clone()
        } else {
            format!("{}: {}", result.title, result.summary)
        };
        match result.status {
            agena_plugin_host::sdk::CommandStatus::Succeeded => self.flash_success(feedback),
            agena_plugin_host::sdk::CommandStatus::Failed => self.flash_error(
                result
                    .detail
                    .clone()
                    .filter(|detail| !detail.trim().is_empty())
                    .unwrap_or(feedback),
            ),
            agena_plugin_host::sdk::CommandStatus::Unavailable
            | agena_plugin_host::sdk::CommandStatus::PermissionRequired
            | agena_plugin_host::sdk::CommandStatus::Cancelled => self.flash_warning(
                result
                    .detail
                    .clone()
                    .filter(|detail| !detail.trim().is_empty())
                    .unwrap_or(feedback),
            ),
        }

        for effect in result.effects {
            match effect {
                agena_plugin_host::sdk::CommandHostEffect::InsertPrompt { prompt } => {
                    if prompt.trim().is_empty() {
                        self.flash_warning(ui_text::t(&self.i18n, "flash-user-command-empty"));
                        continue;
                    }
                    let draft = ComposerDraft {
                        document: agena_domain::ComposerDocument(vec![
                            agena_domain::ComposerNode::Text { text: prompt },
                        ]),
                    };
                    match session_id {
                        Some(session_id) => {
                            self.request_submit_message_with_pending(session_id, draft, None)
                        }
                        None => self.create_session(Some(draft)),
                    }
                }
                agena_plugin_host::sdk::CommandHostEffect::Navigate { path } => {
                    if !self.apply_plugin_navigation(path.as_str()) {
                        self.flash_info(format!("Plugin navigation: {path}"));
                    }
                }
                agena_plugin_host::sdk::CommandHostEffect::OpenUrl { url } => {
                    self.flash_info(format!("Plugin operation URL: {url}"));
                }
                agena_plugin_host::sdk::CommandHostEffect::RefreshPluginSurface { .. } => {}
            }
        }
    }

    fn apply_plugin_navigation(&mut self, path: &str) -> bool {
        let Ok(url) = url::Url::parse(&format!("http://agena.local{path}")) else {
            return false;
        };
        if url.path() != "/settings/plugins" {
            return false;
        }
        let mut plugin_id = None;
        let mut tab = None;
        for (key, value) in url.query_pairs() {
            match key.as_ref() {
                "plugin" => plugin_id = Some(value.into_owned()),
                "pluginTab" => tab = Some(value.into_owned()),
                _ => {}
            }
        }
        let Some(plugin_id) = plugin_id else {
            return false;
        };
        self.open_plugin_workbench_detail(plugin_id.as_str(), tab.as_deref());
        true
    }

    /// `/fork` forks the current session (full history clone) and opens the
    /// fork so the user can start working in it. The parent session is
    /// untouched and keeps running. `/branch` is an alias.
    pub(crate) fn handle_fork_command(&mut self, _args: &str) {
        self.open_fork(ForkKind::Fork);
    }

    /// `/side` is `/fork` plus the side-conversation identity: the fork is
    /// registered in `side_sessions` (footer indicator, already-open gate,
    /// marker dropped on navigation) while the main session keeps running in
    /// the background — the only difference from plain `/fork`. `/btw` and
    /// `/aside` are aliases.
    pub(crate) fn handle_side_command(&mut self, _args: &str) {
        self.open_fork(ForkKind::Side);
    }

    /// Shared `/fork` / `/side` implementation: fork the current session with
    /// full history (`ForkSession`, `at_message_id: None`) and open the fork
    /// through the normal session-created path so the composer is ready for
    /// the user's next message. The fork itself is a permanent child session;
    /// side tracking only affects the TUI identity.
    fn open_fork(&mut self, kind: ForkKind) {
        let Some(parent_id) = self
            .transcript
            .session_id
            .or_else(|| self.sessions.current_selected_id())
        else {
            let key = match kind {
                ForkKind::Fork => "flash-command-requires-session",
                ForkKind::Side => "flash-side-requires-session",
            };
            self.flash_warning(ui_text::t(&self.i18n, key));
            return;
        };
        if kind == ForkKind::Side && !self.side_sessions.is_empty() {
            self.flash_warning(ui_text::t(&self.i18n, "flash-side-already-open"));
            return;
        }
        let title = match kind {
            ForkKind::Fork => ui_text::default_session_title(&self.i18n),
            ForkKind::Side => format!("side: {}", ui_text::default_session_title(&self.i18n)),
        };
        let track_as_side = kind == ForkKind::Side;
        let application = self.application.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let forked =
                crate::app_backend::operations::fork_session(&application, parent_id, Some(title))
                    .await;
            match forked {
                Ok(state) => {
                    let session_id = state.session.id;
                    // Register the side conversation before the open result
                    // routes the UI into the fork, so the open_session switch
                    // keeps the side marker (same-channel ordering).
                    if track_as_side {
                        let _ = tx
                            .send(AppMessage::SideSessionOpened {
                                session_id,
                                parent_id,
                            })
                            .await;
                    }
                    let _ = tx
                        .send(AppMessage::SessionCreated {
                            submit_draft: None,
                            pending_message_id: None,
                            model_stack: None,
                            result: Ok(state.session),
                        })
                        .await;
                }
                Err(err) => {
                    let _ = tx
                        .send(AppMessage::SessionCreated {
                            submit_draft: None,
                            pending_message_id: None,
                            model_stack: None,
                            result: Err(crate::UiFailure::internal(err)),
                        })
                        .await;
                }
            }
        });
    }

    pub(crate) fn handle_commit_command(&mut self, args: &str) {
        let message = args.trim();
        if message.is_empty() {
            self.flash_warning(self.i18n.text_args(
                "flash-command-usage",
                &agena_tui::fl_args!("usage" => "/commit <message>"),
            ));
            return;
        }

        let message = message.to_string();
        self.dispatch_backend_operation(
            move |application| async move {
                crate::app_backend::plugin_effects::create_commit(&application, message).await
            },
            |app, result| match result {
                Ok((commit, summary)) => app.flash_success(ui_text::commit_created_message(
                    &app.i18n,
                    &commit[..commit.len().min(12)],
                    summary.as_str(),
                )),
                Err(error) => app.flash_error(error),
            },
        );
    }

    pub(crate) fn handle_pr_command(&mut self, args: &str) {
        let trimmed = args.trim();
        if trimmed.is_empty() {
            self.flash_warning(self.i18n.text_args(
                "flash-command-usage",
                &agena_tui::fl_args!("usage" => "/pr <title> [--body <text>] [--base <branch>] [--head <branch>]"),
            ));
            return;
        }

        let (title, body, base, head) = match parse_pr_command_args(trimmed) {
            Ok(parsed) => parsed,
            Err(error) => {
                let usage = self.i18n.text_args(
                    "flash-command-usage",
                    &agena_tui::fl_args!("usage" => "/pr <title> [--body <text>] [--base <branch>] [--head <branch>]"),
                );
                self.flash_warning(format!(
                    "{usage}: {}",
                    agena_failure::diagnostic::format_error_chain(error.as_ref())
                ));
                return;
            }
        };

        self.dispatch_backend_operation(
            move |application| async move {
                crate::app_backend::plugin_effects::create_pr(&application, title, body, base, head)
                    .await
            },
            |app, result| match result {
                Ok(url) => app.flash_success(ui_text::pull_request_created_message(
                    &app.i18n,
                    url.as_str(),
                )),
                Err(error) => app.flash_error(error),
            },
        );
    }

    pub(crate) fn handle_export_command(&mut self, args: &str) {
        let requested_path = non_empty_owned(args.to_string()).map(|value| {
            let path = Path::new(value.as_str());
            if path.is_absolute() {
                path.to_path_buf()
            } else {
                std::env::current_dir()
                    .unwrap_or_else(|_| std::env::temp_dir())
                    .join(path)
            }
        });
        self.pending_ui_action = Some(UiAction::ExportTranscript {
            path: requested_path,
        });
    }

    pub(crate) fn submit_session_rename(&mut self, title: &str) -> bool {
        let trimmed = title.trim();
        if trimmed.is_empty() {
            self.flash_warning(ui_text::t(&self.i18n, "flash-session-title-empty"));
            return false;
        }
        let Some(session_id) = self.current_or_selected_session_id() else {
            self.flash_warning(ui_text::t(&self.i18n, "flash-command-requires-session"));
            return false;
        };
        self.request_session_rename(session_id, trimmed.to_string());
        true
    }

    pub(crate) fn toggle_current_session_favorite(&mut self) {
        let selected = self
            .sessions
            .current_selected()
            .map(|session| (session.session_id, session.favorite, session.title.clone()));
        let current = self.transcript.execution.as_ref().map(|execution| {
            (
                execution.session.id,
                execution.session.favorite,
                execution.session.title.clone(),
            )
        });
        let target = current.or(selected);
        let Some((session_id, favorite, _title)) = target else {
            self.flash_warning(ui_text::t(&self.i18n, "flash-command-requires-session"));
            return;
        };
        self.request_session_favorite(session_id, !favorite);
    }
}

/// Which fork flavor a `/fork` or `/side` invocation runs. Both commands
/// create the same real fork (full history clone) and open it for the user;
/// `/side` additionally tracks the fork as an open side conversation so the
/// main session keeps running untouched in the background.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ForkKind {
    Fork,
    Side,
}

use crate::app_backend::PluginCommandEffect;
use crate::commands::ClientCommand;
use crate::{
    App, AppMessage, ComposerDraft, Path, PermissionReplyKind, TIMELINE_EVENT_LIMIT, UiAction,
    non_empty_owned, parse_pr_command_args, ui_text,
};
use agena_api::client_command::ClientCommandAction;
use agena_tui::main_focus::Focus;
