impl App {
    pub(in crate::app) fn execute_command(&mut self, spec: &'static CommandSpec, args: &str) {
        match spec.id {
            CommandId::Help => {
                self.open_context_help();
            }
            CommandId::Commands => self.open_command_palette(),
            CommandId::New => self.create_session(None),
            CommandId::Sessions => self.handle_sessions_command(spec, args),
            CommandId::Lineage => self.open_lineage_picker(),
            CommandId::Rewind => self.open_rewind_messages_picker(),
            CommandId::Rename => self.handle_rename_command(spec, args),
            CommandId::Timeline => self.handle_timeline_command(spec, args),
            CommandId::Settings => self.handle_settings_command(args),
            CommandId::Model => self.open_session_model_chooser(),
            CommandId::Agent => self.open_session_agent_chooser(),
            CommandId::Review => self.handle_review_command(args),
            CommandId::Snapshot => self.handle_snapshot_command(args),
            CommandId::Commit => self.handle_commit_command(args),
            CommandId::Pr => self.handle_pr_command(args),
            CommandId::Export => self.handle_export_command(args),
            CommandId::Memory => self.handle_memory_command(spec, args),
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
                self.request_file_attachment_from_terminal(false, args);
            }
            CommandId::Download => self.request_terminal_download(args),
            CommandId::Editor => {
                self.pending_ui_action = Some(UiAction::EditComposerExternally);
            }
            CommandId::Image => {
                self.request_file_attachment_from_terminal(true, args);
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
            CommandId::Usage => self.open_usage_dashboard(args),
            CommandId::Btw => self.handle_btw_command(args),
            CommandId::Queue => self.handle_queue_command(args),
        }
    }

    pub(in crate::app) fn execute_plugin_slash_command(
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
                    text: prompt,
                    ..ComposerDraft::default()
                };
                match session_id {
                    Some(session_id) => self.request_submit_message(session_id, draft),
                    None => self.create_session(Some(draft)),
                }
            }
            PluginCommandEffect::OpenRoute(route) => {
                self.flash_info(format!("Plugin command route: {route}"));
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
    pub(in crate::app) fn handle_btw_command(&mut self, args: &str) {
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
                    let parts = vec![MessagePartContent::Text(MessageTextPart {
                        text: prompt,
                        synthetic: false,
                        ignored: false,
                    })];
                    let result = backend
                        .submit_parts_message_with_options(session_id, parts, options)
                        .await
                        .map_err(|error| error.to_string());
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
                        result: Err(err.to_string()),
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
    pub(in crate::app) fn handle_queue_command(&mut self, args: &str) {
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

    pub(in crate::app) fn handle_rename_command(&mut self, spec: &'static CommandSpec, args: &str) {
        let trimmed = args.trim();
        if trimmed.is_empty() {
            if self.current_or_selected_session_id().is_some() {
                self.open_rename_session_overlay();
            } else {
                self.flash_warning(self.i18n.text_args(
                    "flash-command-usage",
                    &agena_tui::fl_args!("usage" => spec.invocation()),
                ));
            }
            return;
        }
        self.submit_session_rename(trimmed);
    }

    pub(in crate::app) fn handle_timeline_command(
        &mut self,
        spec: &'static CommandSpec,
        args: &str,
    ) {
        let trimmed = args.trim();
        let limit = if trimmed.is_empty() {
            TIMELINE_EVENT_LIMIT
        } else {
            match trimmed.parse::<u64>() {
                Ok(value) if value > 0 => value,
                _ => {
                    self.flash_warning(self.i18n.text_args(
                        "flash-command-usage",
                        &agena_tui::fl_args!("usage" => spec.invocation()),
                    ));
                    return;
                }
            }
        };
        self.open_timeline_overlay(limit);
    }

    pub(in crate::app) fn handle_settings_command(&mut self, args: &str) {
        self.open_settings_studio(args.trim());
    }

    pub(in crate::app) fn handle_review_command(&mut self, args: &str) {
        let target_session_id = self
            .transcript
            .session_id
            .or_else(|| self.sessions.current_selected_id())
            .unwrap_or(-1);
        let prompt = match self
            .backend
            .render_skill_prompt(target_session_id, "review", args)
        {
            Ok(prompt) => prompt,
            Err(error) => {
                self.flash_error(error.to_string());
                return;
            }
        };
        if prompt.trim().is_empty() {
            self.flash_warning(ui_text::t(&self.i18n, "flash-user-command-empty"));
            return;
        }
        let draft = ComposerDraft {
            text: prompt,
            ..ComposerDraft::default()
        };
        match self
            .transcript
            .session_id
            .or_else(|| self.sessions.current_selected_id())
        {
            Some(session_id) => self.request_submit_message(session_id, draft),
            None => self.create_session(Some(draft)),
        }
    }

    pub(in crate::app) fn handle_snapshot_command(&mut self, args: &str) {
        let trimmed = args.trim();
        if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("list") {
            self.open_inspector_picker(
                ui_text::snapshot_picker_title(&self.i18n),
                ui_text::snapshot_picker_prompt(&self.i18n),
                "",
                self.backend.snapshot_inspector_rows(),
            );
            return;
        }

        let Some(session_id) = self.current_or_selected_session_id() else {
            self.flash_warning(ui_text::t(&self.i18n, "flash-command-requires-session"));
            return;
        };
        let (action, rest) = split_command_args_once(trimmed).unwrap_or((trimmed, ""));
        match action.to_ascii_lowercase().as_str() {
            "enter" => {
                let argument = rest.trim();
                let result = if argument.is_empty() {
                    self.backend.enter_snapshot(session_id, None, None)
                } else {
                    self.backend
                        .enter_snapshot(session_id, Some(argument.to_string()), None)
                };
                match result {
                    Ok(output) => {
                        let mut message = ui_text::snapshot_ready_message(
                            &self.i18n,
                            output.path.as_str(),
                            output.branch.as_deref(),
                        );
                        if let Some(backend) = output.backend.as_deref() {
                            message.push_str(format!(" | backend={backend}").as_str());
                        }
                        if let Some(note) = output.note.as_deref() {
                            message.push_str(format!(" | {note}").as_str());
                        }
                        self.flash_success(message)
                    }
                    Err(error) => self.flash_error(error.to_string()),
                }
            }
            "attach" => {
                let path = rest.trim();
                if path.is_empty() {
                    self.flash_warning(self.i18n.text_args(
                        "flash-command-usage",
                        &agena_tui::fl_args!("usage" => "/snapshot attach <path>"),
                    ));
                    return;
                }
                match self
                    .backend
                    .enter_snapshot(session_id, None, Some(path.to_string()))
                {
                    Ok(output) => {
                        let mut message = ui_text::snapshot_attached_message(
                            &self.i18n,
                            output.path.as_str(),
                            output.branch.as_deref(),
                        );
                        if let Some(backend) = output.backend.as_deref() {
                            message.push_str(format!(" | backend={backend}").as_str());
                        }
                        if let Some(note) = output.note.as_deref() {
                            message.push_str(format!(" | {note}").as_str());
                        }
                        self.flash_success(message)
                    }
                    Err(error) => self.flash_error(error.to_string()),
                }
            }
            "exit" | "leave" => {
                let exit_args = rest.trim();
                let (mode, extra) = split_command_args_once(exit_args).unwrap_or((exit_args, ""));
                match mode.to_ascii_lowercase().as_str() {
                    "" | "keep" => match self.backend.exit_snapshot(session_id, "keep".to_string(), false) {
                        Ok(output) => self.flash_success(ui_text::snapshot_exit_message(
                            &self.i18n,
                            output.action.as_deref(),
                            output.path.as_str(),
                        )),
                        Err(error) => self.flash_error(error.to_string()),
                    },
                    "remove" => {
                        let discard_changes =
                            matches!(extra.trim().to_ascii_lowercase().as_str(), "force" | "discard");
                        self.open_snapshot_remove_confirm(session_id, discard_changes);
                    }
                    _ => {
                        self.flash_warning(self.i18n.text_args(
                            "flash-command-usage",
                            &agena_tui::fl_args!("usage" => "/snapshot exit [keep|remove [force]]"),
                        ));
                    }
                }
            }
            _ => self.flash_warning(self.i18n.text_args(
                "flash-command-usage",
                &agena_tui::fl_args!("usage" => "/snapshot [list|enter [name]|attach <path>|exit [keep|remove [force]]]"),
            )),
        }
    }

    pub(in crate::app) fn handle_commit_command(&mut self, args: &str) {
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

    pub(in crate::app) fn handle_pr_command(&mut self, args: &str) {
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

    pub(in crate::app) fn handle_export_command(&mut self, args: &str) {
        let requested_path = non_empty_owned(args.to_string()).map(|value| {
            self.backend
                .resolve_workspace_path(Path::new(value.as_str()))
        });
        self.pending_ui_action = Some(UiAction::ExportTranscript {
            path: requested_path,
        });
    }

    pub(in crate::app) fn handle_memory_command(&mut self, spec: &'static CommandSpec, args: &str) {
        let trimmed = args.trim();
        if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("list") {
            match self.backend.memory_index_path() {
                Ok(path) => self.pending_ui_action = Some(UiAction::OpenPath { path }),
                Err(error) => self.flash_error(error.to_string()),
            }
            return;
        }

        let (action, rest) = split_command_args_once(trimmed).unwrap_or((trimmed, ""));
        let action = action.to_ascii_lowercase();
        match action.as_str() {
            "list" if rest.is_empty() => match self.backend.memory_index_path() {
                Ok(path) => self.pending_ui_action = Some(UiAction::OpenPath { path }),
                Err(error) => self.flash_error(error.to_string()),
            },
            "edit" | "open" => {
                let result = if rest.is_empty() {
                    self.backend.memory_index_path()
                } else {
                    self.backend.memory_entry_path(rest)
                };
                match result {
                    Ok(path) => self.pending_ui_action = Some(UiAction::OpenPath { path }),
                    Err(error) => self.flash_error(error.to_string()),
                }
            }
            "forget" | "rm" | "remove" | "delete" if !rest.is_empty() => {
                match self.backend.forget_memory(rest) {
                    Ok(()) => self.flash_success(self.i18n.text_args(
                        "flash-memory-forgotten",
                        &agena_tui::fl_args!("name" => rest),
                    )),
                    Err(error) => self.flash_error(error.to_string()),
                }
            }
            _ => self.flash_warning(self.i18n.text_args(
                "flash-command-usage",
                &agena_tui::fl_args!("usage" => spec.invocation()),
            )),
        }
    }

    pub(in crate::app) fn handle_sessions_command(
        &mut self,
        _spec: &'static CommandSpec,
        args: &str,
    ) {
        let trimmed = args.trim();
        if trimmed.is_empty() {
            self.open_resume_session_picker();
            return;
        }

        let next_mode = match trimmed.to_ascii_lowercase().as_str() {
            "all" | "recent" => SessionViewMode::All,
            "roots" | "root" => SessionViewMode::Roots,
            "subtree" | "tree" | "branch" => SessionViewMode::Subtree,
            _ => {
                self.open_resume_session_picker_with_query(trimmed);
                return;
            }
        };
        self.set_session_view_mode(next_mode);
        self.open_resume_session_picker();
    }

    pub(in crate::app) fn set_session_view_mode(&mut self, mode: SessionViewMode) {
        if mode == SessionViewMode::Subtree && self.current_or_selected_session_id().is_none() {
            self.flash_warning(ui_text::t(&self.i18n, "flash-command-requires-session"));
            return;
        }
        let effect = self
            .sessions
            .update(agena_tui::session_list::SessionListAction::SetViewMode(
                mode,
            ));
        self.flash_success(self.i18n.text_args(
            "flash-session-view-mode",
            &agena_tui::fl_args!("mode" => self.current_session_view_summary()),
        ));
        if effect == agena_tui::session_list::SessionListEffect::Reload {
            self.request_sessions(false);
        }
    }

    pub(in crate::app) fn cycle_session_view_mode(&mut self) {
        self.set_session_view_mode(self.sessions.view_mode().next());
    }

    pub(in crate::app) fn submit_session_rename(&mut self, title: &str) -> bool {
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
use crate::app::{
    App, AppMessage, CommandId, CommandSpec, ComposerDraft, Path, PermissionReplyKind,
    TIMELINE_EVENT_LIMIT, UiAction, derive_session_title, non_empty_owned, parse_pr_command_args,
    split_command_args_once, ui_text,
};
use crate::backend::PluginCommandEffect;
use agena_api::resource::{MessagePartContent, MessageTextPart};
use agena_tui::main_focus::Focus;
use agena_tui::session_view::SessionViewMode;
