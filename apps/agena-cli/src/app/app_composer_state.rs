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

    pub(in crate::app) fn flush_input_buffers_if_due(&mut self, now: Instant) {
        self.composer.flush_pending_input_if_due(now);
        if let Some(search) = self.prompt_history_search.as_mut() {
            search.query.flush_pending_input_if_due(now);
            Self::refresh_prompt_history_search(&self.prompt_history, search);
        }
        self.sync_composer_suggestions();
        match &mut self.current_route {
            Route::Main => {}
            Route::Help(_) => {}
            Route::SettingsStudio(_) => {}
            Route::AgentStudio(dialog) => {
                if let Some(editor) = dialog.workbench.editor.as_mut() {
                    editor.input.flush_pending_input_if_due(now);
                }
            }
            Route::PermissionStudio(dialog) => {
                if let Some(editor) = dialog.editor.as_mut() {
                    editor.input.flush_pending_input_if_due(now);
                }
            }
            Route::PermissionRuleStudio(dialog) => {
                if let Some(editor) = dialog.workbench.editor.as_mut() {
                    editor.input.flush_pending_input_if_due(now);
                }
            }
            Route::SessionSearch(dialog) => dialog.input.flush_pending_input_if_due(now),
            Route::Picker(dialog) => dialog.input.flush_pending_input_if_due(now),
            Route::SessionModelChooser(dialog) => {
                dialog.input.flush_pending_input_if_due(now);
                Self::refresh_session_model_chooser_overlay(dialog, false, None);
            }
            Route::Timeline(dialog) => dialog.input.flush_pending_input_if_due(now),
            Route::PluginPolicyStudio(_) => {}
            Route::PluginWorkbench(dialog) => Self::flush_plugin_workbench_input(dialog, now),
            Route::ProviderStudio(dialog) => {
                if let Some(editor) = dialog.editor.as_mut() {
                    editor.input.flush_pending_input_if_due(now);
                }
            }
            Route::ModelCatalogStudio(dialog) => {
                if let Some(editor) = dialog.workbench.editor.as_mut() {
                    editor.input.flush_pending_input_if_due(now);
                }
            }
        }
        if let Some(overlay) = &mut self.overlay {
            match overlay {
                Overlay::TranscriptSearch(dialog) | Overlay::SessionRename(dialog) => {
                    dialog.input.flush_pending_input_if_due(now);
                }
                Overlay::AgentCreate(dialog) => {
                    dialog.input.flush_pending_input_if_due(now);
                }
                Overlay::SettingsValueEdit(dialog) => {
                    dialog.input.flush_pending_input_if_due(now);
                }
                Overlay::RuntimeSettingEdit(dialog) => {
                    dialog.input.flush_pending_input_if_due(now);
                }
                Overlay::Choice(dialog) => {
                    dialog.input.flush_pending_input_if_due(now);
                    Self::sync_choice_overlay_input(dialog, false);
                }
                Overlay::FileAttach(dialog) => dialog.input.flush_pending_input_if_due(now),
                Overlay::PathBrowser(dialog) => {
                    dialog.input.flush_pending_input_if_due(now);
                    Self::refresh_path_browser_overlay_with_root(
                        self.backend.workspace_root(),
                        dialog,
                    );
                }
                Overlay::UserInputReply(dialog) => {
                    if dialog.editing_custom {
                        dialog.custom_input.flush_pending_input_if_due(now);
                    }
                }
                Overlay::SessionSearch(dialog) => dialog.input.flush_pending_input_if_due(now),
                Overlay::Picker(dialog) => dialog.input.flush_pending_input_if_due(now),
                Overlay::Timeline(dialog) => dialog.input.flush_pending_input_if_due(now),
                Overlay::ProviderStudio(dialog) => {
                    if let Some(editor) = dialog.editor.as_mut() {
                        editor.input.flush_pending_input_if_due(now);
                    }
                }
                Overlay::ModelCatalogStudio(dialog) => {
                    if let Some(editor) = dialog.workbench.editor.as_mut() {
                        editor.input.flush_pending_input_if_due(now);
                    }
                }
                Overlay::Confirm(_) => {}
                Overlay::Permission(_) => {}
            }
        }
    }

    pub(in crate::app) fn refresh_file_attach_overlay(&self, dialog: &mut FileAttachOverlay) {
        dialog.items = self
            .backend
            .search_workspace_files(dialog.input.text(), 24)
            .unwrap_or_default();
        dialog.clamp_selection();
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
        let resolved = self.backend.resolve_workspace_path(path);
        let metadata = std::fs::metadata(&resolved).map_err(|error| {
            ui_text::attachment_inspect_failed_message(
                &self.i18n,
                resolved.as_path(),
                error.to_string().as_str(),
            )
        })?;
        let prepared = self
            .backend
            .prepare_attachment_from_path(path)
            .map_err(|error| error.to_string())?;
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
            .push(ComposerItem::Attachment(StagedAttachment {
                path: resolved.clone(),
                placeholder,
                label,
                is_temp,
            }));
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
            DraftSlot::Session(session_id) => self.submitting_session_ids.contains(&session_id),
            DraftSlot::NewSession => {
                self.transcript.submitting && self.transcript.pending_restore_draft.is_some()
            }
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
        self.composer.flush_all_pending_input();
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
            && self.submitting_session_ids.contains(&session_id)
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
                    let prepared = self
                        .backend
                        .prepare_attachment_from_path(attachment.path.as_path())
                        .map_err(|error| error.to_string())?;
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

    pub(in crate::app) fn run_ui_action<B: RatatuiBackend>(
        &mut self,
        action: UiAction,
        terminal: &mut Terminal<B>,
    ) -> Result<()> {
        match action {
            UiAction::EditComposerExternally => self.edit_composer_externally(terminal),
            UiAction::AttachClipboardImage => {
                self.attach_clipboard_image();
                Ok(())
            }
            UiAction::AttachIterm2Files { images_only } => {
                self.attach_iterm2_files(terminal, images_only)
            }
            UiAction::DownloadIterm2File { path } => self.download_iterm2_file(terminal, &path),
            UiAction::ExportTranscript { path } => {
                self.export_transcript_to_editor(terminal, path.as_deref())
            }
            UiAction::OpenPath { path } => self.open_path_in_editor(terminal, path.as_path()),
            UiAction::PageTranscript => self.page_transcript(terminal),
        }
    }

    pub(in crate::app) fn edit_composer_externally<B: RatatuiBackend>(
        &mut self,
        terminal: &mut Terminal<B>,
    ) -> Result<()> {
        self.composer.flush_all_pending_input();
        terminal
            .flush()
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        terminal::suspend_stdio_terminal()?;
        let result = edit_text(self.composer.text());
        terminal::resume_terminal(terminal)?;
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

    pub(in crate::app) fn open_path_in_editor<B: RatatuiBackend>(
        &mut self,
        terminal: &mut Terminal<B>,
        path: &Path,
    ) -> Result<()> {
        terminal
            .flush()
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        terminal::suspend_stdio_terminal()?;
        let result = open_path(path);
        terminal::resume_terminal(terminal)?;
        if let Err(error) = result {
            self.flash_error(self.i18n.text_args(
                "flash-external-editor-failed",
                &crate::fl_args!("error" => error.to_string()),
            ));
        }
        Ok(())
    }

    pub(in crate::app) fn attach_clipboard_image(&mut self) {
        match paste_image_to_temp_png() {
            Ok((path, info)) => {
                let format_label = pasted_image_format(path.as_path()).label();
                if let Err(error) = self.stage_attachment_from_path(path.as_path(), true) {
                    let _ = std::fs::remove_file(path);
                    self.flash_error(error);
                } else {
                    self.flash_success(self.i18n.text_args(
                        "flash-clipboard-image-attached",
                        &crate::fl_args!(
                            "width" => info.width as i64,
                            "height" => info.height as i64,
                            "format" => format_label,
                        ),
                    ));
                }
            }
            Err(error) => self.flash_error(self.i18n.text_args(
                "flash-clipboard-image-attach-failed",
                &crate::fl_args!("error" => error.to_string()),
            )),
        }
    }

    pub(in crate::app) fn attach_iterm2_files<B: RatatuiBackend>(
        &mut self,
        terminal: &mut Terminal<B>,
        images_only: bool,
    ) -> Result<()> {
        let destination =
            env::temp_dir().join(format!("agena-iterm-upload-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&destination).map_err(|error| {
            anyhow::anyhow!("could not create iTerm2 upload directory: {error}")
        })?;

        terminal
            .flush()
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        terminal::suspend_stdio_terminal()?;
        let result = iterm2::request_upload(destination.as_path())
            .and_then(|()| iterm2::uploaded_regular_files(destination.as_path()));
        terminal::resume_terminal(terminal)?;

        let files = match result {
            Ok(files) => files,
            Err(error) => {
                let _ = fs::remove_dir_all(&destination);
                self.flash_warning(error);
                return Ok(());
            }
        };

        let mut attached = 0_usize;
        let mut skipped = 0_usize;
        for path in files {
            if images_only {
                match self.backend.prepare_attachment_from_path(path.as_path()) {
                    Ok(attachment) if attachment.kind == AttachmentKind::Image => {}
                    Ok(_) => {
                        skipped += 1;
                        let _ = fs::remove_file(path);
                        continue;
                    }
                    Err(error) => {
                        skipped += 1;
                        self.flash_warning(error.to_string());
                        let _ = fs::remove_file(path);
                        continue;
                    }
                }
            }
            match self.stage_attachment_from_path(path.as_path(), true) {
                Ok(()) => attached += 1,
                Err(error) => {
                    skipped += 1;
                    self.flash_warning(error);
                    let _ = fs::remove_file(path);
                }
            }
        }

        if attached == 0 {
            let _ = fs::remove_dir_all(&destination);
            self.flash_warning(if images_only {
                "No supported image was selected in iTerm2.".to_string()
            } else {
                "No supported file was selected in iTerm2.".to_string()
            });
        } else if skipped > 0 {
            self.flash_warning(format!(
                "Attached {attached} file(s); skipped {skipped} unsupported file(s)."
            ));
        }
        Ok(())
    }

    pub(in crate::app) fn download_iterm2_file<B: RatatuiBackend>(
        &mut self,
        terminal: &mut Terminal<B>,
        path: &Path,
    ) -> Result<()> {
        terminal
            .flush()
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        terminal::suspend_stdio_terminal()?;
        let result = iterm2::request_download(path);
        terminal::resume_terminal(terminal)?;
        match result {
            Ok(()) => self.flash_success(format!(
                "Downloaded {} through iTerm2.",
                path.file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("file")
            )),
            Err(error) => self.flash_error(error),
        }
        Ok(())
    }
}
use crate::app::RatatuiBackend;
use crate::app::Result;
use crate::app::{
    App, AttachmentKind, BTreeMap, ComposerDraft, ComposerDraftElement, ComposerItem,
    DRAFT_PERSIST_INTERVAL_MS, DraftSlot, Duration, FileAttachOverlay, Focus, HashSet, Instant,
    Overlay, PartContent, Path, PromptHistory, Route, StagedAttachment, Terminal, UiAction,
    UiResult, attachment_chip_label, attachment_placeholder_base, cleanup_temporary_composer_item,
    cleanup_temporary_composer_items, edit_text, env, find_placeholder_occurrence, fs, iterm2, min,
    normalize_pasted_path, open_path, paste_image_to_temp_png, pasted_image_format,
    push_submission_text, terminal, ui_text,
};
