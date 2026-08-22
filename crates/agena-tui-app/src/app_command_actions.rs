impl App {
    pub(crate) fn execute_command(&mut self, spec: &'static CommandSpec, args: &str) {
        if command_opens_interactive_surface_without_arguments(spec.id) && !args.trim().is_empty() {
            self.flash_warning(self.i18n.text_args(
                "flash-command-usage",
                &agena_tui::fl_args!("usage" => spec.invocation()),
            ));
            return;
        }
        match spec.id {
            CommandId::Help => {
                self.open_context_help();
            }
            CommandId::Commands => self.open_command_palette(),
            CommandId::New => self.create_session(None),
            CommandId::Sessions => self.open_resume_session_picker(),
            CommandId::Hub => self.open_hub(),
            CommandId::Lineage => self.open_lineage_picker(),
            CommandId::Rewind => self.open_rewind_messages_picker(),
            CommandId::Rename => self.open_rename_session_overlay(),
            CommandId::Timeline => self.open_timeline_overlay(TIMELINE_EVENT_LIMIT),
            CommandId::Settings => self.open_settings_studio(),
            CommandId::Model => self.open_session_model_chooser(),
            CommandId::Review => self.handle_review_command(args),
            CommandId::Commit => self.handle_commit_command(args),
            CommandId::Pr => self.handle_pr_command(args),
            CommandId::Export => self.handle_export_command(args),
            CommandId::Pager => self.pending_ui_action = Some(UiAction::PageTranscript),
            CommandId::Continue => self.continue_current_session(),
            CommandId::Compact => self.compact_current_session(),
            CommandId::UserInput => self.open_user_input_overlay(),
            CommandId::Allow => self.reply_permission(PermissionReplyKind::AllowOnce),
            CommandId::AllowAlways => self.reply_permission(PermissionReplyKind::AllowAlways),
            CommandId::Deny => self.reply_permission(PermissionReplyKind::DenyOnce),
            CommandId::DenyAlways => self.reply_permission(PermissionReplyKind::DenyAlways),
            CommandId::Attach => {
                self.focus = Focus::Composer;
                self.request_file_attachment(false);
            }
            CommandId::Skill => self.open_skill_picker(),
            CommandId::SkillStudio => self.open_skill_studio(),
            CommandId::Download => self.request_terminal_download(args),
            CommandId::Editor => {
                self.pending_ui_action = Some(UiAction::EditComposerExternally);
            }
            CommandId::Image => {
                self.request_file_attachment(true);
            }
            CommandId::Copy => self.copy_loaded_transcript(),
            CommandId::CopyMessage => self.copy_last_assistant_message(),
            CommandId::CopyVisible => self.copy_visible_transcript(),
            CommandId::Fork => self.handle_fork_command(args),
            CommandId::Children => self.open_child_sessions_picker(),
            CommandId::Parent => self.open_parent_session(),
            CommandId::Diagnostics => {
                self.open_terminal_diagnostics();
            }
            CommandId::Status => {
                self.flash_success(self.current_runtime_status_summary());
            }
            CommandId::Usage => self.open_usage_dashboard(),
            CommandId::Activities => self.open_activities_panel(),
            CommandId::Background => {
                // Return to the session hub and leave the TUI. The server owns
                // the session independently of this client, so nothing is
                // stopped: the session keeps running and can be re-attached
                // from the hub next launch.
                self.open_hub();
                self.should_quit = true;
            }
            CommandId::Plan => self.open_plan_viewer(),
            CommandId::Side => self.handle_side_command(args),
        }
    }

    pub(crate) fn execute_plugin_slash_operation(
        &mut self,
        entry: agena_plugin_host::PluginOperationCatalogItem,
        args: &str,
    ) {
        let session_id = self
            .transcript
            .session_id
            .or_else(|| self.sessions.current_selected_id());
        let args = args.to_string();
        self.dispatch_backend_operation(
            move |application| async move {
                crate::app_backend::plugin_effects::invoke_plugin_slash_operation(
                    &application,
                    &entry,
                    session_id,
                    &args,
                )
                .await
            },
            move |app, result| match result {
                Ok(result) => app.apply_plugin_operation_result(result, session_id),
                Err(error) => app.flash_error(error),
            },
        );
    }

    fn apply_plugin_operation_result(
        &mut self,
        result: PluginOperationEffect,
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
            agena_plugin_host::sdk::PluginOperationStatus::Succeeded => {
                self.flash_success(feedback)
            }
            agena_plugin_host::sdk::PluginOperationStatus::Failed => self.flash_error(
                result
                    .detail
                    .clone()
                    .filter(|detail| !detail.trim().is_empty())
                    .unwrap_or(feedback),
            ),
            agena_plugin_host::sdk::PluginOperationStatus::Unavailable
            | agena_plugin_host::sdk::PluginOperationStatus::PermissionRequired
            | agena_plugin_host::sdk::PluginOperationStatus::Cancelled => self.flash_warning(
                result
                    .detail
                    .clone()
                    .filter(|detail| !detail.trim().is_empty())
                    .unwrap_or(feedback),
            ),
        }

        for effect in result.effects {
            match effect {
                agena_plugin_host::sdk::PluginHostEffect::InsertPrompt { prompt } => {
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
                agena_plugin_host::sdk::PluginHostEffect::Navigate { path } => {
                    if !self.apply_plugin_navigation(path.as_str()) {
                        self.flash_info(format!("Plugin navigation: {path}"));
                    }
                }
                agena_plugin_host::sdk::PluginHostEffect::OpenUrl { url } => {
                    self.flash_info(format!("Plugin operation URL: {url}"));
                }
                agena_plugin_host::sdk::PluginHostEffect::RefreshPluginSurface { .. } => {}
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

    pub(crate) fn handle_review_command(&mut self, args: &str) {
        let Some(session_id) = self
            .transcript
            .session_id
            .or_else(|| self.sessions.current_selected_id())
        else {
            self.flash_warning(ui_text::t(&self.i18n, "flash-command-requires-session"));
            return;
        };
        let review_focus = args.trim().to_string();
        self.dispatch_backend_operation(
            move |application| async move {
                application
                    .invoke_plugin_tool(
                        "agena.skills",
                        "get",
                        serde_json::json!({ "name": "review" }),
                        Some(session_id),
                    )
                    .await
            },
            move |app, result| {
                let response = match result {
                    Ok(response) => response,
                    Err(error) => {
                        app.notify_ui_failure(error, NoticeScope::Session(session_id));
                        return;
                    }
                };
                let Some(prompt) = response
                    .payload
                    .as_ref()
                    .and_then(|payload| payload.get("body"))
                    .and_then(serde_json::Value::as_str)
                    .map(str::trim)
                    .filter(|body| !body.is_empty())
                else {
                    app.flash_error("review Skill did not return instructions");
                    return;
                };
                let prompt = if review_focus.is_empty() {
                    prompt.to_string()
                } else {
                    format!("{prompt}\n\nReview focus:\n{review_focus}")
                };
                let draft = ComposerDraft {
                    document: agena_domain::ComposerDocument(vec![
                        agena_domain::ComposerNode::Text { text: prompt },
                    ]),
                };
                app.request_submit_message_with_pending(session_id, draft, None);
            },
        );
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

    pub(crate) fn set_session_view_mode(&mut self, mode: SessionViewMode) {
        if mode == SessionViewMode::Subtree && self.current_or_selected_session_id().is_none() {
            self.flash_warning(ui_text::t(&self.i18n, "flash-command-requires-session"));
            return;
        }
        let effect = self
            .sessions
            .update(agena_tui_session::session_list::SessionListAction::SetViewMode(mode));
        self.flash_success(self.i18n.text_args(
            "flash-session-view-mode",
            &agena_tui::fl_args!("mode" => self.current_session_view_summary()),
        ));
        if effect == agena_tui_session::session_list::SessionListEffect::Reload {
            self.request_sessions(false);
        }
    }

    pub(crate) fn cycle_session_view_mode(&mut self) {
        self.set_session_view_mode(self.sessions.view_mode().next());
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

fn command_opens_interactive_surface_without_arguments(id: CommandId) -> bool {
    matches!(
        id,
        CommandId::Sessions
            | CommandId::Hub
            | CommandId::Background
            | CommandId::Rename
            | CommandId::Timeline
            | CommandId::Settings
            | CommandId::Attach
            | CommandId::Skill
            | CommandId::SkillStudio
            | CommandId::Image
            | CommandId::Usage
            | CommandId::Activities
            | CommandId::Plan
            | CommandId::Fork
            | CommandId::Side
    )
}
use crate::app_backend::PluginOperationEffect;
use crate::{
    App, AppMessage, CommandId, CommandSpec, ComposerDraft, NoticeScope, Path, PermissionReplyKind,
    TIMELINE_EVENT_LIMIT, UiAction, non_empty_owned, parse_pr_command_args, ui_text,
};
use agena_tui::main_focus::Focus;
use agena_tui_session::session_view::SessionViewMode;
