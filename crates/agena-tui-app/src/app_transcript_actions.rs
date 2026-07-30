impl App {
    pub(crate) fn copy_loaded_transcript(&mut self) {
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

    pub(crate) fn copy_last_assistant_message(&mut self) {
        let Some(response) = self
            .transcript
            .snapshot
            .turns
            .last()
            .map(|turn| &turn.response)
        else {
            self.flash_warning(ui_text::t(&self.i18n, "flash-no-assistant-message"));
            return;
        };
        let text = response.content.text();
        if text.trim().is_empty() {
            self.flash_warning(ui_text::t(&self.i18n, "flash-no-assistant-message-text"));
            return;
        }

        self.request_clipboard_copy(
            text,
            ui_text::t(&self.i18n, "flash-copied-assistant-message"),
        );
    }

    pub(crate) fn copy_visible_transcript(&mut self) {
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

    pub(crate) fn export_transcript_to_editor(
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
                    &agena_tui::fl_args!("error" => error),
                ));
                return Ok(());
            }
        };

        if let Some(parent) = path.parent()
            && let Err(error) = std::fs::create_dir_all(parent)
        {
            self.flash_error(self.i18n.text_args(
                "flash-transcript-export-failed",
                &agena_tui::fl_args!("error" => error.to_string()),
            ));
            return Ok(());
        }

        if let Err(error) = std::fs::write(&path, text) {
            self.flash_error(self.i18n.text_args(
                "flash-transcript-export-failed",
                &agena_tui::fl_args!("error" => error.to_string()),
            ));
            return Ok(());
        }

        let result =
            terminal.with_suspended(SuspendReason::OpenPath, || open_path(path.as_path()))?;

        match result {
            Ok(()) => self.flash_success(self.i18n.text_args(
                "flash-transcript-exported",
                &agena_tui::fl_args!("path" => path.display().to_string()),
            )),
            Err(error) => self.flash_error(self.i18n.text_args(
                "flash-transcript-export-failed",
                &agena_tui::fl_args!("error" => error.to_string()),
            )),
        }
        Ok(())
    }

    pub(crate) fn page_transcript(&mut self, terminal: &mut TerminalRuntime) -> Result<()> {
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
                &agena_tui::fl_args!("error" => error.to_string()),
            ));
        }

        Ok(())
    }

    pub(crate) fn transcript_export_text(&self) -> String {
        let entries = transcript_entries(&self.transcript.snapshot);
        if entries.is_empty() {
            return String::new();
        }

        entries
            .iter()
            .map(|entry| {
                render_entry_export(
                    entry,
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

    pub(crate) fn transcript_pager_text(&self) -> String {
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
                    &agena_tui::fl_args!("id" => parent_id),
                ));
            }
            if execution.session.child_session_count > 0 {
                meta.push(self.i18n.text_args(
                    "session-summary-children",
                    &agena_tui::fl_args!("count" => execution.session.child_session_count as i64),
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

    pub(crate) fn transcript_export_markdown(&self) -> String {
        render_transcript_snapshot_export_markdown(
            &self.i18n,
            self.transcript.session_id,
            self.transcript.session_title.as_str(),
            self.transcript.execution.as_ref(),
            &self.transcript.snapshot,
        )
    }

    pub(crate) fn resolve_transcript_export_path(
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

    pub(crate) fn visible_transcript_text(&mut self) -> String {
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

    pub(crate) fn semantic_loaded_transcript_text(&mut self) -> String {
        let width = self.layout.transcript_body.width.max(1);
        self.transcript
            .rendered(width)
            .nodes
            .iter()
            .filter(|node| node.key.is_entry_container() && !node.copy_text.trim().is_empty())
            .map(|node| node.copy_text.clone())
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    pub(crate) fn flash(&mut self, level: FlashLevel, text: impl Into<String>) {
        self.flash = Some(FlashMessage::new(level, text));
    }

    pub(crate) fn flash_error(&mut self, text: impl Into<String>) {
        self.flash(FlashLevel::Error, text);
    }

    pub(crate) fn flash_warning(&mut self, text: impl Into<String>) {
        self.flash(FlashLevel::Warning, text);
    }

    pub(crate) fn flash_success(&mut self, text: impl Into<String>) {
        self.flash(FlashLevel::Success, text);
    }

    pub(crate) fn flash_info(&mut self, text: impl Into<String>) {
        self.flash(FlashLevel::Info, text);
    }
}
use crate::Result;
use crate::{
    App, FlashLevel, FlashMessage, Local, Path, PathBuf, TerminalRuntime, TranscriptDetailDefaults,
    UiResult, min, open_path, page_text, render_entry_export,
    render_transcript_snapshot_export_markdown, transcript_entries, ui_text,
};
use agena_tui::terminal_lifecycle::SuspendReason;
