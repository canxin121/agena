impl App {
    pub(in crate::app) fn copy_loaded_transcript(&mut self) {
        let text = self.semantic_loaded_transcript_text();
        if text.trim().is_empty() {
            self.flash_warning(ui_text::t(&self.i18n, "flash-no-loaded-transcript"));
            return;
        }

        self.request_clipboard_copy(
            text,
            ui_text::t(&self.i18n, "flash-copied-loaded-transcript"),
        );
    }

    pub(in crate::app) fn copy_last_assistant_message(&mut self) {
        let Some(message) = self
            .transcript
            .messages
            .iter()
            .rev()
            .find(|message| message.role == MessageRole::Assistant)
        else {
            self.flash_warning(ui_text::t(&self.i18n, "flash-no-assistant-message"));
            return;
        };

        let Some(text) = assistant_message_text(message) else {
            self.flash_warning(ui_text::t(&self.i18n, "flash-no-assistant-message-text"));
            return;
        };

        self.request_clipboard_copy(
            text,
            ui_text::t(&self.i18n, "flash-copied-assistant-message"),
        );
    }

    pub(in crate::app) fn copy_visible_transcript(&mut self) {
        let text = self.visible_transcript_text();
        if text.trim().is_empty() {
            self.flash_warning(ui_text::t(&self.i18n, "flash-no-visible-transcript"));
            return;
        }

        self.request_clipboard_copy(
            text,
            ui_text::t(&self.i18n, "flash-copied-visible-transcript"),
        );
    }

    pub(in crate::app) fn export_transcript_to_editor(
        &mut self,
        terminal: &mut TerminalRuntime,
        requested_path: Option<&Path>,
    ) -> Result<()> {
        let text = self.transcript_export_markdown();
        if text.trim().is_empty() {
            self.flash_warning(ui_text::t(&self.i18n, "flash-no-loaded-transcript"));
            return Ok(());
        }

        let path = match self.resolve_transcript_export_path(requested_path) {
            Ok(path) => path,
            Err(error) => {
                self.flash_error(self.i18n.text_args(
                    "flash-transcript-export-failed",
                    &crate::fl_args!("error" => error),
                ));
                return Ok(());
            }
        };

        if let Some(parent) = path.parent()
            && let Err(error) = std::fs::create_dir_all(parent)
        {
            self.flash_error(self.i18n.text_args(
                "flash-transcript-export-failed",
                &crate::fl_args!("error" => error.to_string()),
            ));
            return Ok(());
        }

        if let Err(error) = std::fs::write(&path, text) {
            self.flash_error(self.i18n.text_args(
                "flash-transcript-export-failed",
                &crate::fl_args!("error" => error.to_string()),
            ));
            return Ok(());
        }

        let result =
            terminal.with_suspended(SuspendReason::OpenPath, || open_path(path.as_path()))?;

        match result {
            Ok(()) => self.flash_success(self.i18n.text_args(
                "flash-transcript-exported",
                &crate::fl_args!("path" => path.display().to_string()),
            )),
            Err(error) => self.flash_error(self.i18n.text_args(
                "flash-transcript-export-failed",
                &crate::fl_args!("error" => error.to_string()),
            )),
        }
        Ok(())
    }

    pub(in crate::app) fn page_transcript(&mut self, terminal: &mut TerminalRuntime) -> Result<()> {
        let text = self.transcript_pager_text();
        if text.trim().is_empty() {
            self.flash_warning(ui_text::t(&self.i18n, "flash-no-loaded-transcript"));
            return Ok(());
        }

        let result =
            terminal.with_suspended(SuspendReason::ExternalPager, || page_text(text.as_str()))?;

        if let Err(error) = result {
            self.flash_error(self.i18n.text_args(
                "flash-transcript-pager-failed",
                &crate::fl_args!("error" => error.to_string()),
            ));
        }

        Ok(())
    }

    pub(in crate::app) fn transcript_export_text(&self) -> String {
        if self.transcript.messages.is_empty() {
            return String::new();
        }

        self.transcript
            .messages
            .iter()
            .map(|message| {
                render_message_export(
                    message,
                    &self.i18n,
                    TranscriptDetailDefaults {
                        activity_expanded: true,
                    },
                )
                .into_iter()
                .map(|line| line.text)
                .collect::<Vec<_>>()
                .join("\n")
            })
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    pub(in crate::app) fn transcript_pager_text(&self) -> String {
        let body = self.transcript_export_text();
        if body.trim().is_empty() {
            return String::new();
        }

        let mut lines = Vec::new();
        lines.push(
            self.current_or_selected_session_title()
                .unwrap_or_else(|| ui_text::t(&self.i18n, "pane-transcript")),
        );
        if let Some(session_id) = self.transcript.session_id {
            lines.push(format!("#{}", session_id));
        }

        let mut meta = Vec::new();
        if let Some(execution) = self.transcript.execution.as_ref() {
            if let Some(parent_id) = execution.session.parent_id {
                meta.push(self.i18n.text_args(
                    "session-summary-parent",
                    &crate::fl_args!("id" => parent_id),
                ));
            }
            if execution.session.child_session_count > 0 {
                meta.push(self.i18n.text_args(
                    "session-summary-children",
                    &crate::fl_args!("count" => execution.session.child_session_count as i64),
                ));
            }
        }
        meta.extend(self.current_lineage_context_parts());
        meta.push(self.current_session_view_summary());
        if let Some(summary) = self.run_options.summary(&self.i18n) {
            meta.push(summary);
        }
        if !meta.is_empty() {
            lines.push(meta.join(" | "));
        }
        lines.push(String::new());
        lines.push(body);
        lines.join("\n")
    }

    pub(in crate::app) fn transcript_export_markdown(&self) -> String {
        render_transcript_export_markdown(
            &self.i18n,
            self.transcript.session_id,
            self.transcript.session_title.as_str(),
            self.transcript.execution.as_ref(),
            self.transcript.messages.as_slice(),
            self.transcript.has_more_older,
        )
    }

    pub(in crate::app) fn resolve_transcript_export_path(
        &self,
        requested_path: Option<&Path>,
    ) -> UiResult<PathBuf> {
        if let Some(path) = requested_path {
            if path.exists() && path.is_dir() {
                return Err(ui_text::transcript_export_path_is_directory_error(
                    &self.i18n, path,
                ));
            }
            return Ok(path.to_path_buf());
        }

        let session_id = self.transcript.session_id.unwrap_or_default();
        let timestamp = Local::now().format("%Y%m%d-%H%M%S");
        Ok(std::env::temp_dir().join(format!("agena-session-{session_id}-{timestamp}.md")))
    }

    pub(in crate::app) fn visible_transcript_text(&mut self) -> String {
        let width = self.layout.transcript_body.width.max(1);
        let height = self.layout.transcript_body.height.max(1) as usize;
        if self.transcript.session_id.is_none() {
            return ui_text::no_session_selected_text(&self.i18n);
        }

        let viewport_top = self.transcript.viewport_top();
        let rendered = self.transcript.rendered(width);
        let start = min(viewport_top, rendered.lines.len());
        let end = min(start.saturating_add(height), rendered.lines.len());
        rendered
            .nodes
            .iter()
            .filter(|node| {
                node.contributes_to_aggregate_copy()
                    && node.start_line < end
                    && node.end_line > start
            })
            .map(|node| node.copy_text.clone())
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    pub(in crate::app) fn semantic_loaded_transcript_text(&mut self) -> String {
        let width = self.layout.transcript_body.width.max(1);
        self.transcript
            .rendered(width)
            .nodes
            .iter()
            .filter(|node| node.key.is_message_container() && !node.copy_text.trim().is_empty())
            .map(|node| node.copy_text.clone())
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    pub(in crate::app) fn maybe_request_more_sessions(&mut self) {
        if self.sessions.should_load_more() {
            self.request_sessions(true);
        }
    }

    pub(in crate::app) fn maybe_request_older_messages(&mut self) {
        if self.transcript.should_load_older()
            && let Some(session_id) = self.transcript.session_id
        {
            self.request_messages(session_id, MessageLoadMode::Prepend);
        }
    }

    pub(in crate::app) fn flash(&mut self, level: FlashLevel, text: impl Into<String>) {
        self.flash = Some(FlashMessage {
            text: text.into(),
            level,
            expires_at: Instant::now() + Duration::from_secs(5),
        });
    }

    pub(in crate::app) fn flash_error(&mut self, text: impl Into<String>) {
        self.flash(FlashLevel::Error, text);
    }

    pub(in crate::app) fn flash_warning(&mut self, text: impl Into<String>) {
        self.flash(FlashLevel::Warning, text);
    }

    pub(in crate::app) fn flash_success(&mut self, text: impl Into<String>) {
        self.flash(FlashLevel::Success, text);
    }

    pub(in crate::app) fn flash_info(&mut self, text: impl Into<String>) {
        self.flash(FlashLevel::Info, text);
    }
}
use crate::app::Result;
use crate::app::{
    App, Duration, FlashLevel, FlashMessage, Instant, Local, MessageLoadMode, MessageRole, Path,
    PathBuf, SuspendReason, TerminalRuntime, TranscriptDetailDefaults, UiResult,
    assistant_message_text, min, open_path, page_text, render_message_export,
    render_transcript_export_markdown, ui_text,
};
