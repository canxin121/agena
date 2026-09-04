const MAX_RESOURCE_ATTACHMENTS_PER_MESSAGE: usize = 8;
const MAX_CLIPBOARD_UPLOAD_BYTES: u64 = 50 * 1024 * 1024;
const MAX_CLIPBOARD_UPLOAD_TOTAL_BYTES: u64 = 200 * 1024 * 1024;

#[derive(Debug)]
struct ClipboardUploadCandidate {
    path: PathBuf,
    temporary: bool,
    image_info: Option<PastedImageInfo>,
}

#[derive(Debug)]
struct UploadedClipboardAttachment {
    resource: agena_application::dto::WorkspaceFileUploadResource,
    image_info: Option<PastedImageInfo>,
}

#[derive(Debug, Default)]
struct ClipboardUploadBatch {
    uploaded: Vec<UploadedClipboardAttachment>,
    failures: Vec<String>,
}

impl App {
    pub(crate) fn jump_search_match(&mut self, forward: bool) {
        self.transcript.jump_search_match(
            self.layout.transcript_body.width,
            self.layout.transcript_body.height,
            forward,
        );
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
                    Self::refresh_path_browser_overlay(&self.application, dialog);
                }
                _ => {}
            }
        }
    }

    pub(crate) fn try_stage_pasted_path(&mut self, pasted: &str) -> bool {
        let Some(path) = normalize_pasted_path(pasted) else {
            return false;
        };
        let resolved = self.resolve_workspace_path(path.as_path());
        if self
            .application
            .workspace_path_metadata(resolved.as_path())
            .is_none_or(|metadata| metadata.is_directory)
        {
            return false;
        }

        match self.stage_attachment_from_path(path.as_path()) {
            Ok(()) => true,
            Err(error) => {
                self.flash_warning(error);
                true
            }
        }
    }

    pub(crate) fn stage_attachment_from_path(&mut self, path: &Path) -> UiResult<()> {
        let resource = self.prepare_workspace_resource(path, false)?;
        self.stage_resource(path, resource)
    }

    /// Commits a browser choice as a server workspace path reference. Neither files nor
    /// directories are read, archived, or Base64-encoded into the message.
    pub(crate) fn stage_file_browser_attachment(
        &mut self,
        path: &Path,
        images_only: bool,
    ) -> UiResult<()> {
        let resource = self.prepare_workspace_resource(path, images_only)?;
        self.stage_resource(path, resource)
    }

    fn prepare_workspace_resource(
        &self,
        path: &Path,
        images_only: bool,
    ) -> UiResult<agena_domain::ResourceActivity> {
        let resolved = self.resolve_workspace_path(path);
        let metadata = self
            .application
            .workspace_path_metadata(resolved.as_path())
            .ok_or_else(|| {
                crate::UiFailure::message(format!(
                    "attachment path is not a regular server workspace file or directory: {}",
                    resolved.display()
                ))
            })?;
        let is_directory = metadata.is_directory;
        let kind = if is_directory {
            AttachmentKind::File
        } else {
            AttachmentKind::detect("", resolved.file_name().and_then(|name| name.to_str()))
        };
        // A selected directory remains a workspace path reference even from the
        // image browser; it is not image content and must not be read.
        if images_only && !is_directory && kind != AttachmentKind::Image {
            return Err(crate::UiFailure::message(ui_text::t(
                &self.i18n,
                "flash-attach-images-only",
            )));
        }
        let relative = resolved
            .strip_prefix(self.application.workspace_root())
            .map_err(|_| {
                crate::UiFailure::message("The attachment must be inside the active workspace.")
            })?;
        let name = resolved
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| !name.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| relative.display().to_string());
        Ok(agena_domain::ResourceActivity {
            kind: if is_directory {
                agena_domain::ResourceKind::Directory
            } else {
                match kind {
                    AttachmentKind::Image => agena_domain::ResourceKind::Image,
                    AttachmentKind::Audio => agena_domain::ResourceKind::Audio,
                    AttachmentKind::Video => agena_domain::ResourceKind::Video,
                    AttachmentKind::Pdf => agena_domain::ResourceKind::Pdf,
                    AttachmentKind::File => agena_domain::ResourceKind::File,
                }
            },
            reference: agena_domain::ResourceReference::WorkspacePath {
                path: relative.to_string_lossy().replace('\\', "/"),
            },
            name,
            media_type: is_directory.then_some("inode/directory".to_owned()),
            size_bytes: (!is_directory).then_some(metadata.size.unwrap_or_default()),
            width: None,
            height: None,
            duration_ms: None,
            page_count: None,
        })
    }

    pub(crate) fn stage_skill_reference(&mut self, mut item: ComposerItem) {
        let placeholder = self.make_unique_composer_placeholder(item.placeholder.clone());
        item.placeholder = placeholder.clone();
        let name = match item.payload() {
            agena_domain::ActivityPayload::SkillReference(skill) => skill.name.clone(),
            _ => return,
        };
        self.composer.insert_element(placeholder.as_str());
        self.composer_items.push(item);
        self.flash_success(
            self.i18n
                .text_args("flash-skill-attached", &agena_tui::fl_args!("name" => name)),
        );
    }

    pub(crate) fn stage_long_paste_text_file(&mut self, text: String) {
        if self.remaining_resource_attachment_slots() == 0 {
            self.flash_warning(self.i18n.text_args(
                "flash-attachment-count-limit",
                &agena_tui::fl_args!("count" => MAX_RESOURCE_ATTACHMENTS_PER_MESSAGE as i64),
            ));
            return;
        }
        if text.len() as u64 > MAX_CLIPBOARD_UPLOAD_BYTES {
            self.flash_warning(self.i18n.text_args(
                "flash-clipboard-paste-failed",
                &agena_tui::fl_args!("error" => "pasted text exceeds the 50 MiB upload limit"),
            ));
            return;
        }

        let filename = format!("clipboard-paste-{}.txt", uuid::Uuid::new_v4().simple());
        self.dispatch_backend_operation(
            move |application| async move {
                application
                    .upload_workspace_attachment(
                        filename.as_str(),
                        text.as_bytes(),
                        Some("text/plain; charset=utf-8"),
                    )
                    .await
            },
            |app, result| match result {
                Ok(uploaded) => match app.stage_uploaded_attachment(uploaded, None) {
                    Ok(()) => app.after_composer_text_mutated(),
                    Err(error) => app.flash_warning(error),
                },
                Err(error) => app.flash_error(app.i18n.text_args(
                    "flash-clipboard-paste-failed",
                    &agena_tui::fl_args!("error" => error.to_string()),
                )),
            },
        );
    }

    fn stage_resource(
        &mut self,
        path: &Path,
        resource: agena_domain::ResourceActivity,
    ) -> UiResult<()> {
        let resolved = self.resolve_workspace_path(path);
        let metadata = self
            .application
            .workspace_path_metadata(resolved.as_path())
            .ok_or_else(|| {
                crate::UiFailure::message(format!(
                    "attachment path is no longer available on the server: {}",
                    resolved.display()
                ))
            })?;
        self.stage_resource_with_metadata(
            resolved.as_path(),
            resource,
            metadata.is_directory,
            metadata.size.unwrap_or_default(),
            "workspace",
            resolved.display().to_string(),
        )
    }

    pub(crate) fn stage_uploaded_attachment(
        &mut self,
        uploaded: agena_application::dto::WorkspaceFileUploadResource,
        image_info: Option<&agena_tui_platform::clipboard::PastedImageInfo>,
    ) -> UiResult<()> {
        let mime = uploaded.mime.clone().unwrap_or_default();
        let kind = AttachmentKind::detect(mime.as_str(), Some(uploaded.name.as_str()));
        let display_name = uploaded.name.clone();
        let resource = agena_domain::ResourceActivity {
            kind: match kind {
                AttachmentKind::Image => agena_domain::ResourceKind::Image,
                AttachmentKind::Audio => agena_domain::ResourceKind::Audio,
                AttachmentKind::Video => agena_domain::ResourceKind::Video,
                AttachmentKind::Pdf => agena_domain::ResourceKind::Pdf,
                AttachmentKind::File => agena_domain::ResourceKind::File,
            },
            reference: agena_domain::ResourceReference::WorkspacePath {
                path: uploaded.path.clone(),
            },
            name: uploaded.name,
            media_type: uploaded.mime,
            size_bytes: Some(uploaded.size_bytes),
            width: image_info.map(|info| info.width),
            height: image_info.map(|info| info.height),
            duration_ms: None,
            page_count: None,
        };
        self.stage_resource_with_metadata(
            Path::new(display_name.as_str()),
            resource,
            false,
            uploaded.size_bytes,
            "clipboard",
            uploaded.path,
        )
    }

    pub(crate) fn remaining_resource_attachment_slots(&self) -> usize {
        let used = self
            .composer_items
            .iter()
            .filter(|item| matches!(item.payload(), agena_domain::ActivityPayload::Resource(_)))
            .count();
        MAX_RESOURCE_ATTACHMENTS_PER_MESSAGE.saturating_sub(used)
    }

    fn stage_resource_with_metadata(
        &mut self,
        display_path: &Path,
        resource: agena_domain::ResourceActivity,
        is_directory: bool,
        size_bytes: u64,
        provenance_source: &str,
        attached_path: String,
    ) -> UiResult<()> {
        if self.remaining_resource_attachment_slots() == 0 {
            return Err(crate::UiFailure::message(self.i18n.text_args(
                "flash-attachment-count-limit",
                &agena_tui::fl_args!("count" => MAX_RESOURCE_ATTACHMENTS_PER_MESSAGE as i64),
            )));
        }
        let kind = match resource.kind {
            agena_domain::ResourceKind::Image => AttachmentKind::Image,
            agena_domain::ResourceKind::Audio => AttachmentKind::Audio,
            agena_domain::ResourceKind::Video => AttachmentKind::Video,
            agena_domain::ResourceKind::Pdf => AttachmentKind::Pdf,
            _ => AttachmentKind::File,
        };
        let label = attachment_chip_label(
            &self.i18n,
            display_path,
            kind,
            is_directory,
            resource.width,
            resource.height,
            size_bytes,
        );
        let placeholder = self.make_unique_composer_placeholder(attachment_placeholder_base(
            &self.i18n,
            display_path,
            kind,
            is_directory,
        ));

        self.composer.insert_element(placeholder.as_str());
        self.composer_items.push(ComposerItem {
            placeholder,
            label,
            activity: agena_domain::ComposerActivity {
                id: agena_domain::ActivityId::new(),
                payload: agena_domain::ActivityPayload::Resource(resource),
                provenance: agena_domain::ActivityProvenance {
                    source: Some(provenance_source.to_owned()),
                    content_hash: None,
                    plugin_id: None,
                },
            },
        });
        self.flash_success(self.i18n.text_args(
            "flash-attached",
            &agena_tui::fl_args!("path" => attached_path),
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
        unique_composer_placeholder_text(base.as_str(), &mut existing)
    }

    pub(crate) fn sync_composer_items_with_editor(&mut self) {
        let items = std::mem::take(&mut self.composer_items);
        self.composer_items =
            sync_composer_items_with_editor_texts(&self.composer.element_texts(), items);
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
            document: composer_document_from_editor(
                self.composer.text(),
                self.composer.draft_elements().as_slice(),
                self.composer_items.as_slice(),
            ),
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

        self.draft_store.persist(&self.draft_store_path)?;
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

    pub(crate) fn report_draft_store_error(&mut self, error: crate::UiFailure) {
        let message = error.to_string();
        let should_report = self.draft_store_reported_error.as_deref() != Some(message.as_str());
        self.draft_store_reported_error = Some(message.clone());
        if should_report {
            self.flash_error(message);
        }
    }

    pub(crate) fn record_prompt_history_from_draft(&mut self, draft: &ComposerDraft) {
        if draft.activities().next().is_some() {
            return;
        }
        let text = draft.text();
        let Some(text) = PromptHistory::normalized_text(text.as_str()) else {
            return;
        };
        self.reset_prompt_history_recall();
        if !self.prompt_history.push(text) {
            return;
        }
        if let Err(error) = self.prompt_history.persist(&self.prompt_history_path) {
            self.report_prompt_history_error(error);
        } else {
            self.prompt_history_reported_error = None;
        }
    }

    pub(crate) fn report_prompt_history_error(&mut self, error: crate::UiFailure) {
        let message = error.to_string();
        let should_report = self.prompt_history_reported_error.as_deref() != Some(message.as_str());
        self.prompt_history_reported_error = Some(message.clone());
        if should_report {
            self.flash_error(message);
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
        // Resource activities persist references, never temporary payloads.
    }

    pub(crate) fn take_composer_draft(&mut self) -> ComposerDraft {
        let draft = self.current_composer_draft();
        self.clear_composer_state();
        draft
    }

    pub(crate) fn restore_composer_draft(&mut self, draft: ComposerDraft) {
        if self.composer.text().trim().is_empty() && self.composer_items.is_empty() {
            let (text, elements, items) = rebuild_placeholders(draft.document);
            self.composer.set_text(text);
            self.composer.set_elements(elements);
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

    pub(crate) fn build_submission_document(
        &self,
        draft: &ComposerDraft,
    ) -> UiResult<agena_domain::ComposerDocument> {
        if draft.document.is_empty() {
            return Err(crate::UiFailure::message("The message is empty."));
        }
        Ok(draft.document.clone())
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
                    terminal.write_protocol(sequence).map_err(|error| {
                        ClipboardTextError(agena_failure::diagnostic::format_error_chain(
                            error.as_ref(),
                        ))
                    })
                }) {
                    Ok(method) => self.flash_clipboard_copy_success(method, success),
                    Err(error) => self.flash_error(self.i18n.text_args(
                        "flash-clipboard-copy-failed",
                        &agena_tui::fl_args!("error" => error.to_string()),
                    )),
                }
                Ok(())
            }
            UiAction::PasteClipboard => self.paste_from_clipboard(terminal),
            UiAction::EditComposerExternally => self.edit_composer_externally(terminal),
            UiAction::DownloadTerminalFile { path, remove_after } => {
                self.download_terminal_file(terminal, &path, remove_after)
            }
            UiAction::ExportTranscript { path } => {
                self.export_transcript_to_editor(terminal, path.as_deref())
            }
            UiAction::OpenPath { path } => self.open_path_in_editor(terminal, path.as_path()),
            UiAction::PageTranscript => self.page_transcript(terminal),
        }
    }

    fn paste_from_clipboard(&mut self, terminal: &mut TerminalRuntime) -> Result<()> {
        self.reset_prompt_history_recall();
        self.focus = Focus::Composer;
        let context = terminal.context().clone();
        let available_slots = self.remaining_resource_attachment_slots();
        let mut failures = Vec::new();

        if context.capabilities.clipboard_read_native.is_operational() {
            match clipboard_file_list() {
                Ok(paths) => {
                    let mut candidates = paths
                        .into_iter()
                        .filter(|path| path.is_file())
                        .map(|path| ClipboardUploadCandidate {
                            path,
                            temporary: false,
                            image_info: None,
                        })
                        .collect::<Vec<_>>();
                    if !candidates.is_empty() {
                        if available_slots == 0 {
                            self.flash_warning(self.i18n.text_args(
                                "flash-attachment-count-limit",
                                &agena_tui::fl_args!("count" => MAX_RESOURCE_ATTACHMENTS_PER_MESSAGE as i64),
                            ));
                            return Ok(());
                        }
                        if candidates.len() > available_slots {
                            candidates.truncate(available_slots);
                            self.flash_warning(self.i18n.text_args(
                                "flash-clipboard-attachments-truncated",
                                &agena_tui::fl_args!("count" => available_slots as i64),
                            ));
                        }
                        self.dispatch_clipboard_uploads(candidates, None);
                        return Ok(());
                    }
                }
                Err(error) => failures.push(format!("clipboard files: {error}")),
            }
        }

        match acquire_clipboard_image(&context, terminal)? {
            Ok(acquisition) => {
                if available_slots == 0 {
                    if let Some(root) = acquisition.cleanup_root.as_ref() {
                        let _ = std::fs::remove_dir_all(root);
                    }
                    for item in &acquisition.items {
                        if item.temporary {
                            let _ = std::fs::remove_file(&item.path);
                        }
                    }
                    self.flash_warning(self.i18n.text_args(
                        "flash-attachment-count-limit",
                        &agena_tui::fl_args!("count" => MAX_RESOURCE_ATTACHMENTS_PER_MESSAGE as i64),
                    ));
                    return Ok(());
                }
                let candidates = acquisition
                    .items
                    .into_iter()
                    .take(available_slots)
                    .map(|item| ClipboardUploadCandidate {
                        path: item.path,
                        temporary: item.temporary,
                        image_info: item.image_info,
                    })
                    .collect::<Vec<_>>();
                if !candidates.is_empty() {
                    self.dispatch_clipboard_uploads(candidates, acquisition.cleanup_root);
                    return Ok(());
                }
            }
            Err(error) => failures.push(format!("clipboard image: {error}")),
        }

        match get_clipboard_text(&context) {
            Ok(text) if !text.is_empty() => {
                self.handle_paste(text);
                return Ok(());
            }
            Ok(_) => failures.push("clipboard text is empty".to_owned()),
            Err(error) => failures.push(format!("clipboard text: {error}")),
        }

        self.flash_warning(self.i18n.text_args(
            "flash-clipboard-paste-failed",
            &agena_tui::fl_args!("error" => failures.join("; ")),
        ));
        Ok(())
    }

    fn dispatch_clipboard_uploads(
        &mut self,
        candidates: Vec<ClipboardUploadCandidate>,
        cleanup_root: Option<PathBuf>,
    ) {
        let cleanup_files = candidates
            .iter()
            .filter(|candidate| candidate.temporary)
            .map(|candidate| candidate.path.clone())
            .collect::<Vec<_>>();
        self.dispatch_backend_operation(
            move |application| async move {
                let mut batch = ClipboardUploadBatch::default();
                let mut accepted_bytes = 0u64;
                for candidate in candidates {
                    let upload = async {
                        let metadata = tokio::fs::metadata(candidate.path.as_path()).await?;
                        if !metadata.is_file() {
                            anyhow::bail!(
                                "clipboard attachment is not a regular file: {}",
                                candidate.path.display()
                            );
                        }
                        if metadata.len() > MAX_CLIPBOARD_UPLOAD_BYTES {
                            anyhow::bail!(
                                "clipboard attachment exceeds the 50 MiB upload limit: {}",
                                candidate.path.display()
                            );
                        }
                        if accepted_bytes.saturating_add(metadata.len())
                            > MAX_CLIPBOARD_UPLOAD_TOTAL_BYTES
                        {
                            anyhow::bail!(
                                "clipboard attachments exceed the 200 MiB total upload limit"
                            );
                        }
                        let bytes = tokio::fs::read(candidate.path.as_path()).await?;
                        if bytes.is_empty() {
                            anyhow::bail!(
                                "clipboard attachment is empty: {}",
                                candidate.path.display()
                            );
                        }
                        let filename = candidate
                            .path
                            .file_name()
                            .and_then(|name| name.to_str())
                            .map(str::to_owned)
                            .ok_or_else(|| {
                                anyhow::anyhow!(
                                    "clipboard attachment has no usable filename: {}",
                                    candidate.path.display()
                                )
                            })?;
                        let mime = mime_guess::from_path(candidate.path.as_path())
                            .first_raw()
                            .map(str::to_owned);
                        application
                            .upload_workspace_attachment(
                                filename.as_str(),
                                bytes.as_slice(),
                                mime.as_deref(),
                            )
                            .await
                    }
                    .await;
                    match upload {
                        Ok(resource) => {
                            accepted_bytes = accepted_bytes.saturating_add(resource.size_bytes);
                            batch.uploaded.push(UploadedClipboardAttachment {
                                resource,
                                image_info: candidate.image_info,
                            });
                        }
                        Err(error) => {
                            batch
                                .failures
                                .push(format!("{}: {}", candidate.path.display(), error))
                        }
                    }
                }

                for path in cleanup_files {
                    let _ = tokio::fs::remove_file(path).await;
                }
                if let Some(root) = cleanup_root {
                    let _ = tokio::fs::remove_dir_all(root).await;
                }
                Ok::<_, anyhow::Error>(batch)
            },
            |app, result| match result {
                Ok(mut batch) => {
                    let mut staged = 0usize;
                    for attachment in batch.uploaded {
                        match app.stage_uploaded_attachment(
                            attachment.resource,
                            attachment.image_info.as_ref(),
                        ) {
                            Ok(()) => staged += 1,
                            Err(error) => batch.failures.push(error.to_string()),
                        }
                    }
                    if staged > 0 {
                        app.after_composer_text_mutated();
                    }
                    if !batch.failures.is_empty() {
                        app.flash_warning(app.i18n.text_args(
                            "flash-clipboard-paste-failed",
                            &agena_tui::fl_args!("error" => batch.failures.join("; ")),
                        ));
                    }
                }
                Err(error) => app.flash_error(app.i18n.text_args(
                    "flash-clipboard-paste-failed",
                    &agena_tui::fl_args!("error" => error.to_string()),
                )),
            },
        );
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
        remove_after: bool,
    ) -> Result<()> {
        let _cleanup = TemporaryDownloadCleanup::new(path, remove_after);
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
                    self.flash_error(crate::UiFailure::internal(error));
                    return Ok(());
                }
            }
        }
        self.flash_error(crate::UiFailure::internal(failures.join("; ")));
        Ok(())
    }
}

struct TemporaryDownloadCleanup(Option<std::path::PathBuf>);

impl TemporaryDownloadCleanup {
    fn new(path: &Path, remove_after: bool) -> Self {
        Self(remove_after.then(|| path.to_path_buf()))
    }
}

impl Drop for TemporaryDownloadCleanup {
    fn drop(&mut self) {
        if let Some(path) = self.0.take()
            && let Err(error) = std::fs::remove_file(&path)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(
                path = %path.display(),
                diagnostic = %agena_failure::diagnostic::format_error_chain(&error),
                "failed to remove a temporary TUI download"
            );
        }
    }
}

/// Keep only the composer items whose placeholder still exists as an editor
/// element. Items whose placeholder was deleted or edited away are removed so
/// their activity cannot leak a stale reference into the submitted document;
/// the placeholder then degrades to ordinary body text, which is the
/// user-visible signal that the attachment was removed.
pub(crate) fn sync_composer_items_with_editor_texts(
    element_texts: &[String],
    items: Vec<ComposerItem>,
) -> Vec<ComposerItem> {
    let mut by_placeholder = items
        .into_iter()
        .map(|item| (item.placeholder().to_string(), item))
        .collect::<BTreeMap<_, _>>();

    let mut synced = Vec::new();
    for placeholder in element_texts {
        if let Some(item) = by_placeholder.remove(placeholder.as_str()) {
            synced.push(item);
        }
    }

    for (_, item) in by_placeholder {
        cleanup_temporary_composer_item(&item);
    }

    synced
}

#[cfg(test)]
fn text_artifact_composer_activity(text: String) -> agena_domain::ComposerActivity {
    let count = text.chars().count();
    agena_domain::ComposerActivity {
        id: agena_domain::ActivityId::new(),
        payload: agena_domain::ActivityPayload::TextArtifact(agena_domain::TextArtifactActivity {
            text,
            language: None,
            label: Some(format!("paste {count} chars")),
        }),
        provenance: agena_domain::ActivityProvenance {
            source: Some("legacy-test-fixture".to_owned()),
            content_hash: None,
            plugin_id: None,
        },
    }
}

fn composer_document_from_editor(
    text: &str,
    element_ranges: &[std::ops::Range<usize>],
    items: &[ComposerItem],
) -> agena_domain::ComposerDocument {
    let mut nodes = Vec::new();
    let mut cursor = 0usize;
    for (range, item) in element_ranges.iter().zip(items) {
        let start = range.start.min(text.len());
        let end = range.end.min(text.len());
        if cursor < start {
            nodes.push(agena_domain::ComposerNode::Text {
                text: text[cursor..start].to_owned(),
            });
        }
        nodes.push(agena_domain::ComposerNode::activity(item.activity.clone()));
        cursor = end;
    }
    if cursor < text.len() {
        nodes.push(agena_domain::ComposerNode::Text {
            text: text[cursor..].to_owned(),
        });
    }
    agena_domain::ComposerDocument(nodes)
}

/// Rebuild the editor projection (text, element ranges, composer items) from a
/// saved draft document. Placeholders are regenerated from activity payloads,
/// so identical activities (e.g. the same large block pasted twice) must be
/// disambiguated here — otherwise `sync_composer_items_with_editor` would match
/// both elements to one item and the second placeholder would degrade into
/// literal body text on send.
fn rebuild_placeholders(
    document: agena_domain::ComposerDocument,
) -> (String, Vec<std::ops::Range<usize>>, Vec<ComposerItem>) {
    let mut text = String::new();
    let mut items = Vec::new();
    let mut elements = Vec::new();
    let mut used = std::collections::HashSet::new();
    for node in document.0 {
        match node {
            agena_domain::ComposerNode::Text { text: value } => text.push_str(&value),
            agena_domain::ComposerNode::Activity { activity } => {
                let (base_placeholder, label) =
                    crate::composer_state_impls::composer_activity_presentation(&activity.payload);
                let placeholder = unique_composer_placeholder_text(&base_placeholder, &mut used);
                let start = text.len();
                text.push_str(&placeholder);
                elements.push(start..text.len());
                items.push(ComposerItem {
                    activity: *activity,
                    placeholder,
                    label,
                });
            }
        }
    }
    (text, elements, items)
}

/// Pick `base` unless it is already taken, then `stem #2]`, `stem #3]`, …
/// matching `make_unique_composer_placeholder`'s suffix scheme.
///
/// The base is sanitized exactly like the editor sanitizes inserted text
/// (`sanitize_editor_text` strips ANSI escapes, control characters, tabs and
/// Unicode bidi isolation marks). i18n placeholders such as the zh-CN
/// `attachment-placeholder` wrap their interpolated variables in U+2068/U+2069
/// isolation marks, so without this step the stored item placeholder would
/// never equal the editor element text and `sync_composer_items_with_editor`
/// would drop the attachment, leaking the literal placeholder into the body.
fn unique_composer_placeholder_text(
    base: &str,
    used: &mut std::collections::HashSet<String>,
) -> String {
    let base = sanitize_editor_text(base);
    if used.insert(base.clone()) {
        return base;
    }
    let stem = base.strip_suffix(']').unwrap_or(&base);
    for index in 2.. {
        let candidate = if base.ends_with(']') {
            format!("{stem} #{index}]")
        } else {
            format!("{stem} #{index}")
        };
        if used.insert(candidate.clone()) {
            return candidate;
        }
    }
    unreachable!("at least one numbered suffix is always available")
}

#[cfg(test)]
mod tests {
    use agena_domain::{
        ActivityId, ActivityPayload, ComposerActivity, ComposerDocument, ComposerNode,
        ResourceActivity, ResourceKind, ResourceReference, SkillReferenceActivity,
    };
    use agena_tui::i18n::I18n;
    use agena_tui_components::Editor;

    use super::{
        AttachmentKind, ComposerItem, attachment_placeholder_base, composer_document_from_editor,
        rebuild_placeholders, sync_composer_items_with_editor_texts,
        text_artifact_composer_activity, unique_composer_placeholder_text,
    };

    #[test]
    fn mixed_document_preserves_inline_order_without_placeholder_text() {
        let skill_placeholder = "[Skill: doctor]";
        let directory_placeholder = "[folder apps]";
        let skill_id = ActivityId::new();
        let directory_id = ActivityId::new();
        let source = format!("hi {skill_placeholder} hi {directory_placeholder}");
        let skill_start = 3;
        let directory_start = skill_start + skill_placeholder.len() + 4;
        let document = composer_document_from_editor(
            source.as_str(),
            &[
                skill_start..skill_start + skill_placeholder.len(),
                directory_start..directory_start + directory_placeholder.len(),
            ],
            &[
                ComposerItem {
                    placeholder: skill_placeholder.to_owned(),
                    label: "Skill: doctor".to_owned(),
                    activity: ComposerActivity {
                        id: skill_id,
                        payload: ActivityPayload::SkillReference(SkillReferenceActivity {
                            name: "doctor".to_owned(),
                            description: String::new(),
                            content_hash: "hash".to_owned(),
                            source: "workspace".to_owned(),
                            aliases: Vec::new(),
                        }),
                        provenance: Default::default(),
                    },
                },
                ComposerItem {
                    placeholder: directory_placeholder.to_owned(),
                    label: "folder apps".to_owned(),
                    activity: ComposerActivity {
                        id: directory_id,
                        payload: ActivityPayload::Resource(ResourceActivity {
                            kind: ResourceKind::Directory,
                            reference: ResourceReference::WorkspacePath {
                                path: "apps".to_owned(),
                            },
                            name: "apps".to_owned(),
                            media_type: None,
                            size_bytes: None,
                            width: None,
                            height: None,
                            duration_ms: None,
                            page_count: None,
                        }),
                        provenance: Default::default(),
                    },
                },
            ],
        );
        assert!(matches!(&document.0[0], ComposerNode::Text { text } if text == "hi "));
        assert!(
            matches!(&document.0[1], ComposerNode::Activity { activity } if activity.id == skill_id)
        );
        assert!(matches!(&document.0[2], ComposerNode::Text { text } if text == " hi "));
        assert!(matches!(
            &document.0[3],
            ComposerNode::Activity { activity }
                if activity.id == directory_id
                    && matches!(
                        &activity.payload,
                        ActivityPayload::Resource(ResourceActivity {
                            kind: ResourceKind::Directory,
                            reference: ResourceReference::WorkspacePath { path },
                            ..
                        }) if path == "apps"
                    )
        ));
        assert_eq!(document.text(), "hi  hi ");
    }

    #[test]
    fn restoring_duplicate_activities_keeps_unique_placeholders_and_no_body_text() {
        let first = text_artifact_composer_activity("x".repeat(1_000));
        let second = text_artifact_composer_activity("x".repeat(1_000));
        let document = ComposerDocument(vec![
            ComposerNode::activity(first.clone()),
            ComposerNode::activity(second.clone()),
        ]);

        let (text, elements, items) = rebuild_placeholders(document);
        // Both artifacts survive with distinct placeholders so the editor never
        // degrades one of them into literal body text.
        assert_eq!(items.len(), 2);
        assert_eq!(elements.len(), 2);
        assert_ne!(items[0].placeholder(), items[1].placeholder());
        assert!(items[1].placeholder().contains(" #2"));

        // Round-tripping the rebuilt projection keeps two Activities and zero
        // literal placeholder text in the message body.
        let rebuilt =
            composer_document_from_editor(text.as_str(), elements.as_slice(), items.as_slice());
        assert!(matches!(
            rebuilt.0.as_slice(),
            [
                ComposerNode::Activity { activity: first_activity },
                ComposerNode::Activity { activity: second_activity },
            ] if first_activity.id == first.id && second_activity.id == second.id
        ));
        assert!(rebuilt.text().is_empty());
        let json = serde_json::to_string(&rebuilt).unwrap();
        // The serialized document must carry the two full artifacts and no
        // literal placeholder text (a placeholder ends with `… +N chars]`).
        assert!(
            !json.contains("chars]"),
            "body must not leak a placeholder literal"
        );
        assert_eq!(json.matches("x".repeat(1_000).as_str()).count(), 2);
    }

    #[test]
    fn zh_cn_file_attachment_survives_editor_sync_and_builds_activity_document() {
        let i18n = I18n::resolve(Some("zh-CN"), None);
        // The zh-CN attachment-placeholder wraps its interpolated variables in
        // Unicode bidi isolation marks (U+2068/U+2069); the editor strips those
        // marks on insert, so the unique placeholder must be computed from the
        // sanitized base or the composer item would never match the editor
        // element and the attachment would degrade to literal body text.
        let base = attachment_placeholder_base(
            &i18n,
            std::path::Path::new("LICENSE"),
            AttachmentKind::File,
            false,
        );
        assert!(
            base.contains('\u{2068}'),
            "i18n placeholders carry bidi marks"
        );
        let placeholder =
            unique_composer_placeholder_text(&base, &mut std::collections::HashSet::new());
        assert_eq!(placeholder, "[文件 LICENSE]");

        // The editor flow that stage_resource performs when the user attaches
        // a file, followed by typing a body after the placeholder.
        let mut editor = Editor::default();
        editor.insert_element(placeholder.as_str());
        editor.insert_str(" aaa");
        assert_eq!(editor.text(), "[文件 LICENSE] aaa");
        assert_eq!(editor.element_texts(), vec![placeholder.clone()]);

        let item = ComposerItem {
            placeholder: placeholder.clone(),
            label: "文件: LICENSE".to_owned(),
            activity: ComposerActivity {
                id: ActivityId::new(),
                payload: ActivityPayload::Resource(ResourceActivity {
                    kind: ResourceKind::File,
                    reference: ResourceReference::WorkspacePath {
                        path: "LICENSE".to_owned(),
                    },
                    name: "LICENSE".to_owned(),
                    media_type: None,
                    size_bytes: None,
                    width: None,
                    height: None,
                    duration_ms: None,
                    page_count: None,
                }),
                provenance: Default::default(),
            },
        };

        // submit_composer → current_composer_draft syncs items against the
        // editor elements; the zh-CN placeholder must match exactly so the
        // Resource activity survives into the submitted document.
        let synced = sync_composer_items_with_editor_texts(&editor.element_texts(), vec![item]);
        assert_eq!(synced.len(), 1);

        let document = composer_document_from_editor(
            editor.text(),
            editor.draft_elements().as_slice(),
            synced.as_slice(),
        );
        assert!(matches!(
            &document.0[0],
            ComposerNode::Activity { activity }
                if matches!(
                    &activity.payload,
                    ActivityPayload::Resource(ResourceActivity { name, .. }) if name == "LICENSE"
                )
        ));
        assert!(matches!(&document.0[1], ComposerNode::Text { text } if text == " aaa"));
        assert_eq!(document.text(), " aaa");
        assert!(
            !document.text().contains("[文件 LICENSE]"),
            "the placeholder must never leak into the submitted body text"
        );
    }
}

use crate::Result;
use crate::{
    App, AttachmentKind, BTreeMap, ClipboardTextError, ComposerDraft, ComposerItem,
    DRAFT_PERSIST_INTERVAL_MS, DraftSlot, Duration, HashSet, Instant, Overlay, Path, PathBuf,
    PromptHistory, Route, RunActivityTarget, RunOperation, TerminalRuntime, UiAction, UiResult,
    attachment_chip_label, attachment_placeholder_base, cleanup_temporary_composer_item,
    cleanup_temporary_composer_items, download_providers, edit_text, find_placeholder_occurrence,
    normalize_pasted_path, open_path, request_download, set_clipboard_text, ui_text,
};
use agena_tui::main_focus::Focus;
use agena_tui::terminal_lifecycle::SuspendReason;
use agena_tui_components::sanitize_editor_text;
use agena_tui_platform::{
    attachment_source::acquire_clipboard_image,
    clipboard::{PastedImageInfo, clipboard_file_list, get_clipboard_text},
};
