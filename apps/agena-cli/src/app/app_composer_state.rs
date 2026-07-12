impl App {
    pub(in crate::app) fn jump_search_match(&mut self, forward: bool) {
        self.transcript.jump_search_match(
            self.layout.transcript_body.width,
            self.layout.transcript_body.height,
            forward,
        );
    }

    pub(in crate::app) fn jump_to_message(&mut self, message_id: i64) {
        self.transcript.jump_to_message(
            self.layout.transcript_body.width,
            self.layout.transcript_body.height,
            message_id,
        );
        self.focus = Focus::Transcript;
    }

    pub(in crate::app) fn refresh_input_derived_state(&mut self) {
        self.sync_composer_suggestions();
        if let Route::SessionModelChooser(dialog) = &mut self.current_route {
            Self::refresh_session_model_chooser_overlay(dialog, false, None);
        }
        if let Some(overlay) = &mut self.overlay {
            match overlay {
                Overlay::Choice(dialog) => {
                    Self::sync_choice_overlay_input(dialog);
                }
                Overlay::PathBrowser(dialog) => {
                    Self::refresh_path_browser_overlay_with_root(
                        self.backend.workspace_root(),
                        dialog,
                    );
                }
                _ => {}
            }
        }
    }

    pub(in crate::app) fn refresh_file_attach_overlay(&self, dialog: &mut FileAttachOverlay) {
        let items = self
            .backend
            .search_workspace_files(dialog.input.text(), 24)
            .unwrap_or_default();
        dialog.replace_items(items);
    }

    pub(in crate::app) fn try_stage_pasted_path(&mut self, pasted: &str) -> bool {
        let Some(path) = normalize_pasted_path(pasted) else {
            return false;
        };
        let resolved = self.backend.resolve_workspace_path(path.as_path());
        if !resolved.exists() || !resolved.is_file() {
            return false;
        }

        match self.stage_attachment_from_path(path.as_path(), false) {
            Ok(()) => true,
            Err(error) => {
                self.flash_warning(error);
                true
            }
        }
    }

    pub(in crate::app) fn stage_attachment_from_path(
        &mut self,
        path: &Path,
        is_temp: bool,
    ) -> UiResult<()> {
        let prepared = self
            .backend
            .prepare_attachment_from_path(path)
            .map_err(|error| error.to_string())?;
        self.stage_prepared_attachment(path, is_temp, None, prepared)
    }

    fn stage_prepared_attachment(
        &mut self,
        path: &Path,
        is_temp: bool,
        cleanup_root: Option<&Path>,
        prepared: AttachmentItem,
    ) -> UiResult<()> {
        let resolved = self.backend.resolve_workspace_path(path);
        let metadata = std::fs::metadata(&resolved).map_err(|error| {
            ui_text::attachment_inspect_failed_message(
                &self.i18n,
                resolved.as_path(),
                error.to_string().as_str(),
            )
        })?;
        let label = attachment_chip_label(
            &self.i18n,
            resolved.as_path(),
            prepared.kind,
            prepared.width,
            prepared.height,
            metadata.len(),
        );
        let placeholder = self.make_unique_composer_placeholder(attachment_placeholder_base(
            &self.i18n,
            resolved.as_path(),
            prepared.kind,
        ));

        self.composer.insert_element(placeholder.as_str());
        self.composer_items
            .push(ComposerItem::Attachment(Box::new(StagedAttachment {
                path: resolved.clone(),
                prepared: Some(std::sync::Arc::new(prepared)),
                cleanup_root: cleanup_root.map(Path::to_path_buf),
                placeholder,
                label,
                is_temp,
            })));
        self.flash_success(self.i18n.text_args(
            "flash-attached",
            &crate::fl_args!("path" => resolved.display().to_string()),
        ));
        Ok(())
    }

    pub(in crate::app) fn make_unique_composer_placeholder(&self, base: String) -> String {
        let mut existing = self
            .composer_items
            .iter()
            .map(|item| item.placeholder().to_string())
            .collect::<HashSet<_>>();
        existing.extend(self.composer.element_texts());
        if !existing.contains(base.as_str()) {
            return base;
        }

        let stem = base.strip_suffix(']').unwrap_or(base.as_str());
        for index in 2.. {
            let candidate = if base.ends_with(']') {
                format!("{stem} #{index}]")
            } else {
                format!("{stem} #{index}")
            };
            if !existing.contains(candidate.as_str()) {
                return candidate;
            }
        }

        base
    }

    pub(in crate::app) fn sync_composer_items_with_editor(&mut self) {
        let mut by_placeholder = std::mem::take(&mut self.composer_items)
            .into_iter()
            .map(|item| (item.placeholder().to_string(), item))
            .collect::<BTreeMap<_, _>>();

        let mut synced = Vec::new();
        for placeholder in self.composer.element_texts() {
            if let Some(item) = by_placeholder.remove(placeholder.as_str()) {
                synced.push(item);
            }
        }

        for (_, item) in by_placeholder {
            cleanup_temporary_composer_item(&item);
        }

        self.composer_items = synced;
    }

    pub(in crate::app) fn current_draft_slot(&self) -> DraftSlot {
        self.transcript
            .session_id
            .map(DraftSlot::Session)
            .unwrap_or(DraftSlot::NewSession)
    }

    pub(in crate::app) fn current_slot_has_in_flight_draft(&self) -> bool {
        if !self.composer.text().trim().is_empty() || !self.composer_items.is_empty() {
            return false;
        }

        match self.current_draft_slot() {
            DraftSlot::Session(session_id) => self.run_activity.has_operation(
                RunActivityTarget::Session(session_id),
                RunOperation::SubmitMessage,
            ),
            DraftSlot::NewSession => self
                .run_activity
                .has_operation(RunActivityTarget::NewSession, RunOperation::CreateSession),
        }
    }

    pub(in crate::app) fn clear_composer_state(&mut self) {
        self.composer.clear();
        self.composer_items.clear();
        self.slash_command_suggestions = None;
        self.dismissed_slash_command_suggestions_for = None;
        self.file_mention_suggestions = None;
        self.dismissed_file_mention_suggestions_for = None;
        self.prompt_history_search = None;
        self.selected_composer_item = None;
    }

    pub(in crate::app) fn current_composer_draft(&mut self) -> ComposerDraft {
        self.sync_composer_items_with_editor();
        ComposerDraft {
            text: self.composer.text().to_string(),
            items: self.composer_items.clone(),
            elements: self
                .composer
                .draft_elements()
                .into_iter()
                .filter_map(|range| {
                    self.composer.text().get(range.clone()).map(|placeholder| {
                        ComposerDraftElement {
                            placeholder: placeholder.to_string(),
                            range,
                        }
                    })
                })
                .collect(),
        }
    }

    pub(in crate::app) fn sync_current_draft_slot(&mut self) {
        if self.current_slot_has_in_flight_draft() {
            return;
        }
        let slot = self.current_draft_slot();
        let draft = self.current_composer_draft();
        self.set_draft_for_slot(slot, draft);
    }

    pub(in crate::app) fn set_draft_for_slot(&mut self, slot: DraftSlot, draft: ComposerDraft) {
        if self.draft_store.set(slot, draft) {
            self.draft_store_dirty = true;
        }
    }

    pub(in crate::app) fn clear_draft_for_slot(&mut self, slot: DraftSlot) {
        if self.draft_store.clear(slot) {
            self.draft_store_dirty = true;
        }
    }

    pub(in crate::app) fn restore_draft_for_slot(&mut self, slot: DraftSlot) {
        if let DraftSlot::Session(session_id) = slot
            && self.run_activity.has_operation(
                RunActivityTarget::Session(session_id),
                RunOperation::SubmitMessage,
            )
        {
            return;
        }
        if let Some(draft) = self.draft_store.get(slot).cloned() {
            self.restore_composer_draft(draft);
        }
    }

    pub(in crate::app) fn try_persist_draft_store(&mut self, force: bool) -> UiResult<()> {
        if !self.draft_store_dirty {
            return Ok(());
        }
        if !force
            && self.draft_store_last_persist_at.elapsed()
                < Duration::from_millis(DRAFT_PERSIST_INTERVAL_MS)
        {
            return Ok(());
        }

        self.draft_store
            .persist(&self.draft_store_path)
            .map_err(|error| {
                ui_text::composer_drafts_save_failed_message(&self.i18n, error.to_string().as_str())
            })?;
        self.draft_store_dirty = false;
        self.draft_store_last_persist_at = Instant::now();
        self.draft_store_reported_error = None;
        Ok(())
    }

    pub(in crate::app) fn persist_draft_store_with_feedback(&mut self, force: bool) {
        if let Err(error) = self.try_persist_draft_store(force) {
            self.report_draft_store_error(error);
        }
    }

    pub(in crate::app) fn report_draft_store_error(&mut self, error: String) {
        let should_report = self.draft_store_reported_error.as_deref() != Some(error.as_str());
        self.draft_store_reported_error = Some(error.clone());
        if should_report {
            self.flash_error(error);
        }
    }

    pub(in crate::app) fn record_prompt_history_from_draft(&mut self, draft: &ComposerDraft) {
        if !draft.items.is_empty() || !draft.elements.is_empty() {
            return;
        }
        let Some(text) = PromptHistory::normalized_text(draft.text.as_str()) else {
            return;
        };
        self.reset_prompt_history_recall();
        if !self.prompt_history.push(text) {
            return;
        }
        if let Err(error) = self.prompt_history.persist(&self.prompt_history_path) {
            self.report_prompt_history_error(ui_text::prompt_history_save_failed_message(
                &self.i18n,
                error.to_string().as_str(),
            ));
        } else {
            self.prompt_history_reported_error = None;
        }
    }

    pub(in crate::app) fn report_prompt_history_error(&mut self, error: String) {
        let should_report = self.prompt_history_reported_error.as_deref() != Some(error.as_str());
        self.prompt_history_reported_error = Some(error.clone());
        if should_report {
            self.flash_error(error);
        }
    }

    pub(in crate::app) fn reset_prompt_history_recall(&mut self) {
        self.prompt_history_search = None;
    }

    pub(in crate::app) fn replace_composer_draft(&mut self, draft: ComposerDraft) {
        cleanup_temporary_composer_items(self.composer_items.as_slice());
        self.clear_composer_state();
        self.restore_composer_draft(draft);
    }

    pub(in crate::app) fn cleanup_temporary_draft_store_items(&self) {
        for draft in self.draft_store.drafts.values() {
            cleanup_temporary_composer_items(draft.items.as_slice());
        }
    }

    pub(in crate::app) fn take_composer_draft(&mut self) -> ComposerDraft {
        let draft = self.current_composer_draft();
        self.clear_composer_state();
        draft
    }

    pub(in crate::app) fn restore_composer_draft(&mut self, draft: ComposerDraft) {
        if self.composer.text().trim().is_empty() && self.composer_items.is_empty() {
            let ComposerDraft {
                text,
                items,
                elements,
            } = draft;
            self.composer.set_text(text);
            self.composer
                .set_elements(elements.into_iter().map(|element| element.range).collect());
            self.composer_items = items;
            self.sync_composer_items_with_editor();
            self.sync_composer_suggestions();
        }
    }

    pub(in crate::app) fn apply_external_editor_text(&mut self, text: String) {
        let mut occupied = Vec::new();
        let mut retained = Vec::new();
        for item in std::mem::take(&mut self.composer_items) {
            if let Some(range) =
                find_placeholder_occurrence(text.as_str(), item.placeholder(), &occupied)
            {
                occupied.push(range.clone());
                retained.push((range, item));
            } else {
                cleanup_temporary_composer_item(&item);
            }
        }

        retained.sort_by_key(|(range, _)| range.start);
        let ranges = retained
            .iter()
            .map(|(range, _)| range.clone())
            .collect::<Vec<_>>();
        let kept = retained
            .into_iter()
            .map(|(_, item)| item)
            .collect::<Vec<_>>();

        self.composer.set_text(text);
        self.composer.set_elements(ranges);
        self.composer_items = kept;
        self.sync_composer_suggestions();
    }

    pub(in crate::app) fn build_submission_parts(
        &self,
        draft: &ComposerDraft,
    ) -> UiResult<Vec<PartContent>> {
        let mut parts = Vec::new();

        let mut items_by_placeholder = draft
            .items
            .iter()
            .map(|item| (item.placeholder().to_string(), item))
            .collect::<BTreeMap<_, _>>();
        let mut elements = draft.elements.clone();
        elements.sort_by_key(|element| element.range.start);

        let mut cursor = 0;
        for element in elements {
            let start = min(element.range.start, draft.text.len());
            let end = min(element.range.end, draft.text.len());
            if cursor < start {
                push_submission_text(&mut parts, &draft.text[cursor..start]);
            }

            let actual_placeholder = draft
                .text
                .get(start..end)
                .ok_or_else(|| ui_text::composer_placeholder_range_invalid_error(&self.i18n))?;
            if actual_placeholder != element.placeholder {
                return Err(ui_text::composer_placeholder_out_of_sync_error(&self.i18n));
            }

            let item = items_by_placeholder
                .remove(element.placeholder.as_str())
                .ok_or_else(|| {
                    ui_text::composer_missing_staged_item_error(
                        &self.i18n,
                        element.placeholder.as_str(),
                    )
                })?;
            match item {
                ComposerItem::Attachment(attachment) => {
                    let prepared = match attachment.prepared.as_ref() {
                        Some(prepared) => prepared.as_ref().clone(),
                        None => self
                            .backend
                            .prepare_attachment_from_path(attachment.path.as_path())
                            .map_err(|error| error.to_string())?,
                    };
                    parts.push(PartContent::attachments(vec![prepared]));
                }
                ComposerItem::LargePaste(paste) => {
                    push_submission_text(&mut parts, paste.text.as_str());
                }
            }
            cursor = end;
        }

        if cursor < draft.text.len() {
            push_submission_text(&mut parts, &draft.text[cursor..]);
        }

        Ok(parts)
    }

    pub(in crate::app) fn run_ui_action(
        &mut self,
        action: UiAction,
        terminal: &mut TerminalRuntime,
    ) -> Result<()> {
        match action {
            UiAction::CopyText { text, success } => {
                let context = terminal.context().clone();
                match set_clipboard_text(text.as_str(), &context, |sequence| {
                    terminal
                        .write_protocol(sequence)
                        .map_err(|error| ClipboardTextError(error.to_string()))
                }) {
                    Ok(method) => self.flash_clipboard_copy_success(method, success),
                    Err(error) => self.flash_error(self.i18n.text_args(
                        "flash-clipboard-copy-failed",
                        &crate::fl_args!("error" => error.to_string()),
                    )),
                }
                Ok(())
            }
            UiAction::EditComposerExternally => self.edit_composer_externally(terminal),
            UiAction::AttachClipboardImage => self.attach_clipboard_image(terminal),
            UiAction::AttachTerminalFiles {
                source,
                images_only,
            } => self.attach_terminal_files(terminal, source, images_only),
            UiAction::DownloadTerminalFile { path } => self.download_terminal_file(terminal, &path),
            UiAction::ExportTranscript { path } => {
                self.export_transcript_to_editor(terminal, path.as_deref())
            }
            UiAction::OpenPath { path } => self.open_path_in_editor(terminal, path.as_path()),
            UiAction::PageTranscript => self.page_transcript(terminal),
        }
    }

    pub(in crate::app) fn edit_composer_externally(
        &mut self,
        terminal: &mut TerminalRuntime,
    ) -> Result<()> {
        let result = terminal.with_suspended(SuspendReason::ExternalEditor, || {
            edit_text(self.composer.text())
        })?;
        match result {
            Ok(text) => {
                self.apply_external_editor_text(text);
                self.focus = Focus::Composer;
                self.flash_success(ui_text::t(&self.i18n, "flash-composer-updated"));
            }
            Err(error) => self.flash_error(self.i18n.text_args(
                "flash-external-editor-failed",
                &crate::fl_args!("error" => error.to_string()),
            )),
        }
        Ok(())
    }

    pub(in crate::app) fn open_path_in_editor(
        &mut self,
        terminal: &mut TerminalRuntime,
        path: &Path,
    ) -> Result<()> {
        let result = terminal.with_suspended(SuspendReason::OpenPath, || open_path(path))?;
        if let Err(error) = result {
            self.flash_error(self.i18n.text_args(
                "flash-external-editor-failed",
                &crate::fl_args!("error" => error.to_string()),
            ));
        }
        Ok(())
    }

    pub(in crate::app) fn attach_clipboard_image(
        &mut self,
        terminal: &mut TerminalRuntime,
    ) -> Result<()> {
        let context = terminal.context().clone();
        let acquisition = match acquire_clipboard_image(&context, terminal)? {
            Ok(acquisition) => acquisition,
            Err(error) => {
                self.flash_error(self.i18n.text_args(
                    "flash-clipboard-image-attach-failed",
                    &crate::fl_args!("error" => error.to_string()),
                ));
                return Ok(());
            }
        };
        let AttachmentAcquisition {
            mut items,
            cleanup_root,
        } = acquisition;
        let Some(item) = items.pop() else {
            if let Some(root) = cleanup_root {
                let _ = std::fs::remove_dir_all(root);
            }
            self.flash_warning("Clipboard did not provide an image.".to_string());
            return Ok(());
        };
        let info = item.image_info.clone();
        let format_label = pasted_image_format(item.path.as_path()).label();
        let prepared = self
            .backend
            .prepare_attachment_from_path(item.path.as_path())
            .map_err(|error| error.to_string());
        let staged = prepared.and_then(|prepared| {
            self.stage_prepared_attachment(
                item.path.as_path(),
                item.temporary,
                cleanup_root.as_deref(),
                prepared,
            )
        });
        if let Err(error) = staged {
            let _ = std::fs::remove_file(item.path);
            if let Some(root) = cleanup_root {
                let _ = std::fs::remove_dir_all(root);
            }
            self.flash_error(error);
        } else if let Some(info) = info {
            self.flash_success(self.i18n.text_args(
                "flash-clipboard-image-attached",
                &crate::fl_args!(
                    "width" => info.width as i64,
                    "height" => info.height as i64,
                    "format" => format_label,
                ),
            ));
        }
        Ok(())
    }

    pub(in crate::app) fn attach_terminal_files(
        &mut self,
        terminal: &mut TerminalRuntime,
        request: TerminalUploadRequest,
        images_only: bool,
    ) -> Result<()> {
        let source: Box<dyn AttachmentSource> = match request {
            TerminalUploadRequest::Iterm2 => Box::new(Iterm2UploadSource::new()),
            TerminalUploadRequest::Kitty { local_sources } => {
                Box::new(KittyUploadSource::new(local_sources))
            }
        };
        let provider = source.label();
        let context = terminal.context().clone();
        let acquisition = match acquire_from_source(source.as_ref(), &context, terminal)? {
            Ok(acquisition) => acquisition,
            Err(error) => {
                self.flash_warning(error.to_string());
                return Ok(());
            }
        };

        let mut attached = 0_usize;
        let mut skipped = 0_usize;
        for item in acquisition.items {
            let prepared = match self
                .backend
                .prepare_attachment_from_path(item.path.as_path())
            {
                Ok(attachment) => attachment,
                Err(error) => {
                    skipped += 1;
                    self.flash_warning(error.to_string());
                    let _ = std::fs::remove_file(item.path);
                    continue;
                }
            };
            if images_only {
                match prepared.kind {
                    AttachmentKind::Image => {}
                    _ => {
                        skipped += 1;
                        let _ = std::fs::remove_file(item.path);
                        continue;
                    }
                }
            }
            match self.stage_prepared_attachment(
                item.path.as_path(),
                item.temporary,
                acquisition.cleanup_root.as_deref(),
                prepared,
            ) {
                Ok(()) => attached += 1,
                Err(error) => {
                    skipped += 1;
                    self.flash_warning(error);
                    let _ = std::fs::remove_file(item.path);
                }
            }
        }

        if attached == 0 {
            if let Some(root) = acquisition.cleanup_root {
                let _ = std::fs::remove_dir_all(root);
            }
            self.flash_warning(if images_only {
                format!("No supported image was received through {provider}.")
            } else {
                format!("No supported file was received through {provider}.")
            });
        } else if skipped > 0 {
            self.flash_warning(format!(
                "Attached {attached} file(s); skipped {skipped} unsupported file(s)."
            ));
        }
        Ok(())
    }

    pub(in crate::app) fn download_terminal_file(
        &mut self,
        terminal: &mut TerminalRuntime,
        path: &Path,
    ) -> Result<()> {
        let context = terminal.context().clone();
        let providers = download_providers(&context);
        if providers.is_empty() {
            self.flash_warning(format!(
                "No verified terminal download provider is available. {}",
                context.diagnostic_summary()
            ));
            return Ok(());
        }
        let mut failures = Vec::new();
        for provider in providers {
            let result = terminal.with_suspended(SuspendReason::FileDownload, || {
                request_download(provider, path)
            })?;
            match result {
                Ok(()) => {
                    self.flash_success(format!(
                        "Downloaded {} through {}.",
                        path.file_name()
                            .and_then(|name| name.to_str())
                            .unwrap_or("file"),
                        provider.label(),
                    ));
                    return Ok(());
                }
                Err(error) if error.allows_fallback() => {
                    failures.push(format!("{}: {error}", provider.label()));
                }
                Err(error) => {
                    self.flash_error(error.to_string());
                    return Ok(());
                }
            }
        }
        self.flash_error(failures.join("; "));
        Ok(())
    }
}
use crate::app::Result;
use crate::app::{
    App, AttachmentAcquisition, AttachmentItem, AttachmentKind, AttachmentSource, BTreeMap,
    ClipboardTextError, ComposerDraft, ComposerDraftElement, ComposerItem,
    DRAFT_PERSIST_INTERVAL_MS, DraftSlot, Duration, FileAttachOverlay, Focus, HashSet, Instant,
    Iterm2UploadSource, KittyUploadSource, Overlay, PartContent, Path, PromptHistory, Route,
    RunActivityTarget, RunOperation, StagedAttachment, SuspendReason, TerminalRuntime,
    TerminalUploadRequest, UiAction, UiResult, acquire_clipboard_image, acquire_from_source,
    attachment_chip_label, attachment_placeholder_base, cleanup_temporary_composer_item,
    cleanup_temporary_composer_items, download_providers, edit_text, find_placeholder_occurrence,
    min, normalize_pasted_path, open_path, pasted_image_format, push_submission_text,
    request_download, set_clipboard_text, ui_text,
};
