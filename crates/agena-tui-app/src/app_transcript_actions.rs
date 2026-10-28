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
        let Some(session_id) = self.transcript.session_id else {
            self.flash_warning(ui_text::t(&self.i18n, "flash-no-assistant-message"));
            return;
        };
        let inline_ui = self.transcript.inline_ui_copy_state();
        self.dispatch_backend_operation(
            move |application| async move {
                application
                    .last_assistant_message_text(session_id, inline_ui)
                    .await
            },
            move |app, result| {
                if app.transcript.session_id != Some(session_id) {
                    return;
                }
                match result {
                    Ok(Some(text)) => app.request_clipboard_copy(
                        text,
                        ui_text::t(&app.i18n, "flash-copied-assistant-message"),
                    ),
                    Ok(None) => {
                        app.flash_warning(ui_text::t(&app.i18n, "flash-no-assistant-message-text"))
                    }
                    Err(error) => app.flash_error(app.i18n.text_args(
                        "flash-clipboard-copy-failed",
                        &agena_tui::fl_args!("error" => error.to_string()),
                    )),
                }
            },
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

    pub(crate) fn request_transcript_export(&mut self, requested_path: Option<&Path>) {
        let path = match self.resolve_transcript_export_path(requested_path) {
            Ok(path) => path,
            Err(error) => {
                self.flash_error(self.i18n.text_args(
                    "flash-transcript-export-failed",
                    &agena_tui::fl_args!("error" => error.to_string()),
                ));
                return;
            }
        };
        self.prepare_full_transcript(Some(path));
    }

    pub(crate) fn request_transcript_pager(&mut self) {
        self.prepare_full_transcript(None);
    }

    fn prepare_full_transcript(&mut self, destination: Option<PathBuf>) {
        let Some(session_id) = self.transcript.session_id else {
            self.flash_warning(ui_text::t(&self.i18n, "flash-no-loaded-transcript"));
            return;
        };
        let request = crate::app_backend::transcript_export::TranscriptExportRequest {
            session_id,
            session_title: self.current_or_selected_session_title().unwrap_or_default(),
            execution: self.transcript.execution.clone(),
            i18n: self.i18n.clone(),
            markdown: destination.is_some(),
            destination: destination.clone(),
            inline_ui: self.transcript.inline_ui_copy_state(),
            pager_context: self
                .current_lineage_context_parts()
                .into_iter()
                .chain([self.current_session_view_summary()])
                .chain(self.run_options.summary(&self.i18n))
                .collect::<Vec<_>>()
                .join(" | "),
        };
        self.dispatch_backend_operation(
            move |application| async move { application.prepare_transcript_export(request).await },
            move |app, result| {
                if app.transcript.session_id != Some(session_id) {
                    return;
                }
                match result {
                    Ok(Some(file)) => {
                        app.pending_ui_action = Some(crate::UiAction::PreparedTranscript {
                            session_id,
                            file,
                            destination,
                        });
                    }
                    Ok(None) => {
                        app.flash_warning(ui_text::t(&app.i18n, "flash-no-loaded-transcript"))
                    }
                    Err(error) => app.flash_error(app.i18n.text_args(
                        if destination.is_some() {
                            "flash-transcript-export-failed"
                        } else {
                            "flash-transcript-pager-failed"
                        },
                        &agena_tui::fl_args!("error" => error.to_string()),
                    )),
                }
            },
        );
    }

    pub(crate) fn open_prepared_transcript(
        &mut self,
        terminal: &mut TerminalRuntime,
        session_id: i64,
        file: tempfile::NamedTempFile,
        destination: Option<PathBuf>,
    ) -> Result<()> {
        if self.transcript.session_id != Some(session_id) {
            return Ok(());
        }
        if let Some(path) = destination {
            if let Err(error) = file.persist(&path) {
                self.flash_error(self.i18n.text_args(
                    "flash-transcript-export-failed",
                    &agena_tui::fl_args!("error" => error.to_string()),
                ));
                return Ok(());
            }
            let result = terminal.with_suspended(SuspendReason::OpenPath, || open_path(&path))?;
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
        } else {
            let result =
                terminal.with_suspended(SuspendReason::ExternalPager, || page_file(file.path()))?;
            if let Err(error) = result {
                self.flash_error(self.i18n.text_args(
                    "flash-transcript-pager-failed",
                    &agena_tui::fl_args!("error" => error.to_string()),
                ));
            }
        }
        Ok(())
    }

    pub(crate) fn resolve_transcript_export_path(
        &self,
        requested_path: Option<&Path>,
    ) -> UiResult<PathBuf> {
        if let Some(path) = requested_path {
            if path.exists() && path.is_dir() {
                return Err(crate::UiFailure::message(
                    ui_text::transcript_export_path_is_directory_error(&self.i18n, path),
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
            .map(|node| node.copy_text.to_string())
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
            .map(|node| node.copy_text.to_string())
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    pub(crate) fn notify(&mut self, severity: NoticeSeverity, text: impl Into<String>) {
        self.notifications
            .push(crate::notifications::notice_notification(
                severity,
                text,
                NoticeScope::Global,
                NotificationSurface::Banner,
                None,
                0,
            ));
        self.queue_notice_notification(severity);
    }

    /// Queues a terminal attention notification for the next frame when the
    /// notice severity warrants one. Only error, warning, and success events
    /// raise attention; routine info flashes stay silent.
    pub(crate) fn queue_notice_notification(&mut self, severity: NoticeSeverity) {
        use agena_tui_platform::terminal::integration::NotificationMethod;
        if !matches!(
            severity,
            NoticeSeverity::Error | NoticeSeverity::Warning | NoticeSeverity::Success
        ) {
            return;
        }
        self.terminal_integration
            .queue_notification(NotificationMethod::Bell);
    }

    pub(crate) fn notify_failure(&mut self, failure: &agena_failure::Failure, scope: NoticeScope) {
        if !self.seen_failure_ids.insert(failure.id) {
            return;
        }
        if self.seen_failure_ids.len() > 512 {
            self.seen_failure_ids.clear();
            self.seen_failure_ids.insert(failure.id);
        }
        self.notifications
            .push(crate::notifications::failure_notification(failure, scope));
    }

    pub(crate) fn notify_ui_failure(&mut self, error: crate::UiFailure, scope: NoticeScope) {
        self.notify_failure(&error.failure, scope);
    }

    pub(crate) fn flash_error(&mut self, notice: impl Into<crate::app_types::UiErrorNotice>) {
        match notice.into() {
            crate::app_types::UiErrorNotice::Message(text) => {
                self.notify(NoticeSeverity::Error, text);
            }
            crate::app_types::UiErrorNotice::Failure(error) => {
                self.notify_ui_failure(error, NoticeScope::Global);
            }
        }
    }

    pub(crate) fn flash_warning(&mut self, notice: impl Into<crate::app_types::UiErrorNotice>) {
        match notice.into() {
            crate::app_types::UiErrorNotice::Message(text) => {
                self.notify(NoticeSeverity::Warning, text);
            }
            crate::app_types::UiErrorNotice::Failure(error) => {
                self.notify_ui_failure(error, NoticeScope::Global);
            }
        }
    }

    pub(crate) fn flash_success(&mut self, text: impl Into<String>) {
        self.notify(NoticeSeverity::Success, text);
    }

    pub(crate) fn flash_info(&mut self, text: impl Into<String>) {
        self.notify(NoticeSeverity::Info, text);
    }
}
use crate::Result;
use crate::{
    App, Local, NoticeScope, NoticeSeverity, NotificationSurface, Path, PathBuf, TerminalRuntime,
    UiResult, min, open_path, ui_text,
};
use agena_tui::terminal_lifecycle::SuspendReason;
use agena_tui_platform::external_pager::page_file;
