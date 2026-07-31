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
            CommandId::Fork => {
                let Some(session_id) = self.transcript.session_id else {
                    self.flash_warning(ui_text::t(&self.i18n, "flash-command-requires-session"));
                    return;
                };
                self.create_session_with_parent(None, Some(session_id));
            }
            CommandId::Children => self.open_child_sessions_picker(),
            CommandId::Parent => self.open_parent_session(),
            CommandId::Diagnostics => {
                self.open_terminal_diagnostics();
            }
            CommandId::Status => {
                self.flash_success(self.current_runtime_status_summary());
            }
            CommandId::Usage => self.open_usage_dashboard(),
            CommandId::Btw => self.handle_btw_command(args),
            CommandId::Queue => self.handle_queue_command(args),
        }
    }

    pub(crate) fn execute_plugin_slash_command(
        &mut self,
        entry: agena_plugin_host::PluginCommandCatalogItem,
        args: &str,
    ) {
        let session_id = self
            .transcript
            .session_id
            .or_else(|| self.sessions.current_selected_id());
        let effect = match self.block_on_async(
            self.backend
                .invoke_plugin_slash_command(&entry, session_id, args),
        ) {
            Ok(effect) => effect,
            Err(error) => {
                self.flash_error(error);
                return;
            }
        };
        match effect {
            PluginCommandEffect::None => {}
            PluginCommandEffect::Message(message) => {
                if !message.trim().is_empty() {
                    self.flash_info(message);
                }
            }
            PluginCommandEffect::SubmitPrompt(prompt) => {
                if prompt.trim().is_empty() {
                    self.flash_warning(ui_text::t(&self.i18n, "flash-user-command-empty"));
                    return;
                }
                let draft = ComposerDraft {
                    document: agena_domain::ComposerDocument(vec![
                        agena_domain::ComposerNode::Text { text: prompt },
                    ]),
                };
                match session_id {
                    Some(session_id) => self.request_submit_message(session_id, draft),
                    None => self.create_session(Some(draft)),
                }
            }
            PluginCommandEffect::OpenPluginWorkbench { plugin_id, tab } => {
                self.open_plugin_workbench_detail(plugin_id.as_str(), tab.as_deref());
            }
            PluginCommandEffect::OpenUrl(url) => {
                self.flash_info(format!("Plugin command URL: {url}"));
            }
        }
    }

    /// `/btw <question>` forks a child session and submits the question
    /// there without touching the parent transcript. The parent run keeps
    /// running (or stays idle) untouched; the user can switch to the new
    /// session via the sessions pane to read the answer.
    pub(crate) fn handle_btw_command(&mut self, args: &str) {
        let question = args.trim();
        if question.is_empty() {
            self.flash_warning(self.i18n.text_args(
                "flash-command-usage",
                &agena_tui::fl_args!("usage" => "/btw <question>"),
            ));
            return;
        }
        let parent_id = self
            .transcript
            .session_id
            .or_else(|| self.sessions.current_selected_id());
        let title = format!("btw: {}", derive_session_title(&self.i18n, question));
        let prompt = question.to_string();
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        let options = self.run_options.to_request();
        tokio::spawn(async move {
            let create = backend.create_session(title, parent_id).await;
            match create {
                Ok(session) => {
                    let session_id = session.id;
                    let document =
                        agena_domain::ComposerDocument(vec![agena_domain::ComposerNode::Text {
                            text: prompt,
                        }]);
                    let result = backend
                        .submit_document_with_options(session_id, document, options)
                        .await
                        .map_err(crate::UiFailure::internal);
                    // Reuse the existing run-submitted message — the
                    // handler will route the new session into the UI if
                    // appropriate, otherwise just refresh the list.
                    let _ = tx.send(AppMessage::SessionMessageSubmitted {
                        session_id,
                        pending_message_id: 0,
                        draft: ComposerDraft::default(),
                        result,
                    });
                }
                Err(err) => {
                    let _ = tx.send(AppMessage::SessionCreated {
                        submit_draft: None,
                        pending_message_id: None,
                        result: Err(crate::UiFailure::internal(err)),
                    });
                }
            }
        });
        self.flash_info(ui_text::t(&self.i18n, "flash-btw-spawned"));
    }

    /// `/queue [list|clear|pop]` — inspect or manage the pending message
    /// queue.
    ///   * `list` (default): flash a one-liner showing how many entries
    ///     and the first preview.
    ///   * `clear`: drop every queued message.
    ///   * `pop`: pull the head editable entry back into the editor.
    pub(crate) fn handle_queue_command(&mut self, args: &str) {
        let action = args.trim().to_lowercase();
        match action.as_str() {
            "" | "list" | "ls" | "show" => {
                if self.queue.is_empty() {
                    self.flash_info(ui_text::t(&self.i18n, "flash-queue-empty"));
                    return;
                }
                let preview = self.queue.first_preview(60).unwrap_or_default();
                self.flash_info(self.i18n.text_args(
                    "flash-queue-list",
                    &agena_tui::fl_args!(
                        "count" => self.queue.len() as i64,
                        "preview" => preview,
                    ),
                ));
            }
            "clear" | "drop" => {
                if self.queue.is_empty() {
                    self.flash_info(ui_text::t(&self.i18n, "flash-queue-empty"));
                    return;
                }
                let count = self.queue.len();
                self.queue.clear();
                self.flash_success(self.i18n.text_args(
                    "flash-queue-cleared",
                    &agena_tui::fl_args!("count" => count as i64),
                ));
            }
            "pop" | "edit" => {
                if !self.try_pop_queue_into_editor() {
                    self.flash_info(ui_text::t(&self.i18n, "flash-queue-empty"));
                }
            }
            _ => {
                self.flash_warning(self.i18n.text_args(
                    "flash-command-usage",
                    &agena_tui::fl_args!("usage" => "/queue [list|clear|pop]"),
                ));
            }
        }
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
        let prompt = match self.block_on_async(self.backend.invoke_plugin_ui_tool(
            "agena.skills",
            "get",
            serde_json::json!({ "name": "review" }),
            Some(session_id),
        )) {
            Ok(response) => response
                .payload
                .as_ref()
                .and_then(|payload| payload.get("body"))
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|body| !body.is_empty())
                .map(ToOwned::to_owned)
                .ok_or_else(|| "review Skill did not return instructions".to_owned()),
            Err(error) => {
                self.notify_ui_failure(error, NoticeScope::Session);
                return;
            }
        };
        let prompt = match prompt {
            Ok(prompt) if args.trim().is_empty() => prompt,
            Ok(prompt) => format!("{prompt}\n\nReview focus:\n{}", args.trim()),
            Err(error) => {
                self.flash_error(error);
                return;
            }
        };
        if prompt.trim().is_empty() {
            self.flash_warning(ui_text::t(&self.i18n, "flash-user-command-empty"));
            return;
        }
        let draft = ComposerDraft {
            document: agena_domain::ComposerDocument(vec![agena_domain::ComposerNode::Text {
                text: prompt,
            }]),
        };
        self.request_submit_message(session_id, draft);
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

        match self.block_on_async(self.backend.create_commit(message.to_string())) {
            Ok((commit, summary)) => {
                self.flash_success(ui_text::commit_created_message(
                    &self.i18n,
                    &commit[..commit.len().min(12)],
                    summary.as_str(),
                ));
            }
            Err(error) => self.flash_error(error),
        }
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
            Err(_) => {
                self.flash_warning(self.i18n.text_args(
                    "flash-command-usage",
                    &agena_tui::fl_args!("usage" => "/pr <title> [--body <text>] [--base <branch>] [--head <branch>]"),
                ));
                return;
            }
        };

        match self.block_on_async(self.backend.create_pr(title, body, base, head)) {
            Ok(url) => self.flash_success(ui_text::pull_request_created_message(
                &self.i18n,
                url.as_str(),
            )),
            Err(error) => self.flash_error(error),
        }
    }

    pub(crate) fn handle_export_command(&mut self, args: &str) {
        let requested_path = non_empty_owned(args.to_string()).map(|value| {
            self.backend
                .resolve_workspace_path(Path::new(value.as_str()))
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

fn command_opens_interactive_surface_without_arguments(id: CommandId) -> bool {
    matches!(
        id,
        CommandId::Sessions
            | CommandId::Rename
            | CommandId::Timeline
            | CommandId::Settings
            | CommandId::Attach
            | CommandId::Skill
            | CommandId::SkillStudio
            | CommandId::Image
            | CommandId::Usage
    )
}
use crate::{
    App, AppMessage, CommandId, CommandSpec, ComposerDraft, NoticeScope, Path, PermissionReplyKind,
    TIMELINE_EVENT_LIMIT, UiAction, derive_session_title, non_empty_owned, parse_pr_command_args,
    ui_text,
};
use agena_tui::main_focus::Focus;
use agena_tui_backend::PluginCommandEffect;
use agena_tui_session::session_view::SessionViewMode;
