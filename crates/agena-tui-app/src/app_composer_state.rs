impl App {
    pub(crate) fn jump_search_match(&mut self, forward: bool) {
        self.transcript.jump_search_match(
            self.layout.transcript_body.width,
            self.layout.transcript_body.height,
            forward,
        );
    }

    pub(crate) fn jump_to_message(&mut self, message_id: i64) {
        self.transcript.jump_to_message(
            self.layout.transcript_body.width,
            self.layout.transcript_body.height,
            message_id,
        );
        self.focus = Focus::Transcript;
    }

    pub(crate) fn refresh_input_derived_state(&mut self) {
        self.sync_composer_suggestions();
        if let Route::SessionModelChooser(dialog) = &mut self.current_route {
            agena_tui::model_chooser::refresh(dialog, false);
        }
        if let Some(overlay) = &mut self.overlay {
            match overlay {
                Overlay::Choice(dialog) => {
                    // Ticks and unrelated key events must not re-apply the committed
                    // value after the user has moved the result selection.
                    agena_tui::choice::refresh(&mut dialog.presentation);
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

    pub(crate) fn try_stage_pasted_path(&mut self, pasted: &str) -> bool {
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

    pub(crate) fn stage_attachment_from_path(
        &mut self,
        path: &Path,
        is_temp: bool,
    ) -> UiResult<()> {
        let prepared = self.prepare_local_path_attachment(path, false)?;
        self.stage_prepared_attachment(path, is_temp, None, prepared)
    }

    /// Commits a browser choice as a local path reference. Neither files nor
    /// directories are read, archived, or Base64-encoded into the message.
    pub(crate) fn stage_file_browser_attachment(
        &mut self,
        path: &Path,
        images_only: bool,
    ) -> UiResult<()> {
        let prepared = self.prepare_local_path_attachment(path, images_only)?;
        self.stage_prepared_attachment(path, false, None, prepared)
    }

    fn prepare_local_path_attachment(
        &self,
        path: &Path,
        images_only: bool,
    ) -> UiResult<AttachmentItem> {
        let resolved = self.backend.resolve_workspace_path(path);
        let metadata = std::fs::metadata(&resolved).map_err(|error| error.to_string())?;
        let is_directory = metadata.is_dir();
        if !is_directory && !metadata.is_file() {
            return Err(format!(
                "attachment path is neither a file nor a directory: {}",
                resolved.display()
            ));
        }
        let kind = if is_directory {
            AttachmentKind::File
        } else {
            AttachmentKind::detect("", resolved.file_name().and_then(|name| name.to_str()))
        };
        // A selected directory remains a local path reference even from the
        // image browser; it is not image content and must not be read.
        if images_only && !is_directory && kind != AttachmentKind::Image {
            return Err(ui_text::t(&self.i18n, "flash-attach-images-only"));
        }
        Ok(local_path_attachment_reference(
            resolved.as_path(),
            kind,
            is_directory,
        ))
    }

    pub(crate) fn stage_skill_reference(&mut self, skill: StagedSkillReference) {
        let placeholder = self.make_unique_composer_placeholder(skill.placeholder.clone());
        let mut skill = skill;
        skill.placeholder = placeholder.clone();
        let name = skill.name.clone();
        self.composer.insert_element(placeholder.as_str());
        self.composer_items
            .push(ComposerItem::SkillReference(skill));
        self.flash_success(
            self.i18n
                .text_args("flash-skill-attached", &agena_tui::fl_args!("name" => name)),
        );
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
            &agena_tui::fl_args!("path" => resolved.display().to_string()),
        ));
        Ok(())
    }

    pub(crate) fn make_unique_composer_placeholder(&self, base: String) -> String {
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

    pub(crate) fn sync_composer_items_with_editor(&mut self) {
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

    pub(crate) fn current_draft_slot(&self) -> DraftSlot {
        self.transcript
            .session_id
            .map(DraftSlot::Session)
            .unwrap_or(DraftSlot::NewSession)
    }

    pub(crate) fn current_slot_has_in_flight_draft(&self) -> bool {
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

    pub(crate) fn clear_composer_state(&mut self) {
        self.composer.clear();
        self.composer_items.clear();
        self.slash_command_suggestions = None;
        self.slash_command_suggestion_actions.clear();
        self.dismissed_slash_command_suggestions_for = None;
        self.file_mention_suggestions = None;
        self.file_mention_suggestion_actions.clear();
        self.dismissed_file_mention_suggestions_for = None;
        self.prompt_history_search = None;
        self.composer_item_selection.clear();
    }

    pub(crate) fn current_composer_draft(&mut self) -> ComposerDraft {
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

    pub(crate) fn sync_current_draft_slot(&mut self) {
        if self.current_slot_has_in_flight_draft() {
            return;
        }
        let slot = self.current_draft_slot();
        let draft = self.current_composer_draft();
        self.set_draft_for_slot(slot, draft);
    }

    pub(crate) fn set_draft_for_slot(&mut self, slot: DraftSlot, draft: ComposerDraft) {
        if self.draft_store.set(slot, draft) {
            self.draft_store_dirty = true;
        }
    }

    pub(crate) fn clear_draft_for_slot(&mut self, slot: DraftSlot) {
        if self.draft_store.clear(slot) {
            self.draft_store_dirty = true;
        }
    }

    pub(crate) fn restore_draft_for_slot(&mut self, slot: DraftSlot) {
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

    pub(crate) fn try_persist_draft_store(&mut self, force: bool) -> UiResult<()> {
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

    pub(crate) fn persist_draft_store_with_feedback(&mut self, force: bool) {
        if let Err(error) = self.try_persist_draft_store(force) {
            self.report_draft_store_error(error);
        }
    }

    pub(crate) fn report_draft_store_error(&mut self, error: String) {
        let should_report = self.draft_store_reported_error.as_deref() != Some(error.as_str());
        self.draft_store_reported_error = Some(error.clone());
        if should_report {
            self.flash_error(error);
        }
    }

    pub(crate) fn record_prompt_history_from_draft(&mut self, draft: &ComposerDraft) {
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

    pub(crate) fn report_prompt_history_error(&mut self, error: String) {
        let should_report = self.prompt_history_reported_error.as_deref() != Some(error.as_str());
        self.prompt_history_reported_error = Some(error.clone());
        if should_report {
            self.flash_error(error);
        }
    }

    pub(crate) fn reset_prompt_history_recall(&mut self) {
        self.prompt_history_search = None;
    }

    pub(crate) fn replace_composer_draft(&mut self, draft: ComposerDraft) {
        cleanup_temporary_composer_items(self.composer_items.as_slice());
        self.clear_composer_state();
        self.restore_composer_draft(draft);
    }

    pub(crate) fn cleanup_temporary_draft_store_items(&self) {
        for draft in self.draft_store.drafts.values() {
            cleanup_temporary_composer_items(draft.items.as_slice());
        }
    }

    pub(crate) fn take_composer_draft(&mut self) -> ComposerDraft {
        let draft = self.current_composer_draft();
        self.clear_composer_state();
        draft
    }

    pub(crate) fn restore_composer_draft(&mut self, draft: ComposerDraft) {
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

    pub(crate) fn apply_external_editor_text(&mut self, text: String) {
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

    pub(crate) fn build_submission_parts(
        &self,
        draft: &ComposerDraft,
    ) -> UiResult<Vec<MessagePartContent>> {
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
                    push_submission_text(
                        &mut parts,
                        format!("【Path: {}】", attachment.path.display()).as_str(),
                    );
                }
                ComposerItem::LargePaste(paste) => {
                    push_submission_text(&mut parts, paste.text.as_str());
                }
                ComposerItem::SkillReference(skill) => {
                    push_submission_text(&mut parts, format!("【Skill: {}】", skill.name).as_str());
                    parts.push(MessagePartContent::SkillReference(
                        MessageSkillReferencePart {
                            skills: vec![MessageSkillReference {
                                name: skill.name.clone(),
                                description: skill.description.clone(),
                                instructions: skill.instructions.clone(),
                                content_hash: skill.content_hash.clone(),
                                source: skill.source.clone(),
                                aliases: skill.aliases.clone(),
                            }],
                        },
                    ));
                }
            }
            cursor = end;
        }

        if cursor < draft.text.len() {
            push_submission_text(&mut parts, &draft.text[cursor..]);
        }

        Ok(parts)
    }

    pub(crate) fn run_ui_action(
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
                        &agena_tui::fl_args!("error" => error.to_string()),
                    )),
                }
                Ok(())
            }
            UiAction::EditComposerExternally => self.edit_composer_externally(terminal),
            UiAction::DownloadTerminalFile { path } => self.download_terminal_file(terminal, &path),
            UiAction::ExportTranscript { path } => {
                self.export_transcript_to_editor(terminal, path.as_deref())
            }
            UiAction::OpenPath { path } => self.open_path_in_editor(terminal, path.as_path()),
            UiAction::PageTranscript => self.page_transcript(terminal),
        }
    }

    pub(crate) fn edit_composer_externally(
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
                &agena_tui::fl_args!("error" => error.to_string()),
            )),
        }
        Ok(())
    }

    pub(crate) fn open_path_in_editor(
        &mut self,
        terminal: &mut TerminalRuntime,
        path: &Path,
    ) -> Result<()> {
        let result = terminal.with_suspended(SuspendReason::OpenPath, || open_path(path))?;
        if let Err(error) = result {
            self.flash_error(self.i18n.text_args(
                "flash-external-editor-failed",
                &agena_tui::fl_args!("error" => error.to_string()),
            ));
        }
        Ok(())
    }

    pub(crate) fn download_terminal_file(
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

fn local_path_attachment_reference(
    path: &Path,
    kind: AttachmentKind,
    is_directory: bool,
) -> AttachmentItem {
    let filename = path
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| path.display().to_string());
    AttachmentItem {
        kind,
        mime: is_directory
            .then_some("inode/directory".to_owned())
            .unwrap_or_default(),
        source: AttachmentSource::LocalPath {
            path: path.display().to_string(),
        },
        filename: Some(filename),
        title: None,
        size_bytes: None,
        sha256: None,
        width: None,
        height: None,
        duration_ms: None,
        page_count: None,
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use agena_plugin_sdk::{AttachmentKind, AttachmentSource};

    use super::local_path_attachment_reference;

    #[test]
    fn local_attachment_references_paths_without_embedding_file_content() {
        let attachment = local_path_attachment_reference(
            Path::new("/workspace/notes.md"),
            AttachmentKind::File,
            false,
        );
        assert_eq!(attachment.filename.as_deref(), Some("notes.md"));
        assert!(attachment.mime.is_empty());
        assert_eq!(
            attachment.source,
            AttachmentSource::LocalPath {
                path: "/workspace/notes.md".to_owned()
            }
        );
    }

    #[test]
    fn local_directory_attachment_is_not_flattened_into_file_content() {
        let attachment = local_path_attachment_reference(
            Path::new("/workspace/project"),
            AttachmentKind::File,
            true,
        );
        assert_eq!(attachment.filename.as_deref(), Some("project"));
        assert_eq!(attachment.mime, "inode/directory");
        assert_eq!(
            attachment.source,
            AttachmentSource::LocalPath {
                path: "/workspace/project".to_owned()
            }
        );
    }
}

use crate::Result;
use crate::{
    App, AttachmentItem, AttachmentKind, AttachmentSource, BTreeMap, ClipboardTextError,
    ComposerDraft, ComposerDraftElement, ComposerItem, DRAFT_PERSIST_INTERVAL_MS, DraftSlot,
    Duration, HashSet, Instant, Overlay, Path, PromptHistory, Route, RunActivityTarget,
    RunOperation, StagedAttachment, StagedSkillReference, TerminalRuntime, UiAction, UiResult,
    attachment_chip_label, attachment_placeholder_base, cleanup_temporary_composer_item,
    cleanup_temporary_composer_items, download_providers, edit_text, find_placeholder_occurrence,
    min, normalize_pasted_path, open_path, push_submission_text, request_download,
    set_clipboard_text, ui_text,
};
use agena_api::resource::{MessagePartContent, MessageSkillReference, MessageSkillReferencePart};
use agena_tui::main_focus::Focus;
use agena_tui::terminal_lifecycle::SuspendReason;
