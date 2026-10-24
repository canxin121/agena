use std::io::Write as _;

fn persist_atomically(path: &Path, raw: &[u8]) -> UiResult<()> {
    let parent = path.parent().ok_or_else(|| {
        crate::UiFailure::message(format!(
            "persistent TUI state path has no parent: {}",
            path.display()
        ))
    })?;
    agena_storage_layout::secure_directory(parent).map_err(crate::UiFailure::internal)?;
    let permissions = match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {
            Some(metadata.permissions())
        }
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(crate::UiFailure::message(
                "persistent TUI state cannot be a symbolic link",
            ));
        }
        Ok(_) => None,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(crate::UiFailure::internal(error)),
    };
    let mut builder = tempfile::Builder::new();
    builder.prefix(".agena-tui-state-").suffix(".tmp");
    #[cfg(unix)]
    if permissions.is_none() {
        use std::os::unix::fs::PermissionsExt as _;
        builder.permissions(fs::Permissions::from_mode(0o600));
    }
    let mut staged = builder
        .tempfile_in(parent)
        .map_err(crate::UiFailure::internal)?;
    staged.write_all(raw).map_err(crate::UiFailure::internal)?;
    staged.flush().map_err(crate::UiFailure::internal)?;
    staged
        .as_file()
        .sync_all()
        .map_err(crate::UiFailure::internal)?;
    if let Some(permissions) = permissions {
        staged
            .as_file()
            .set_permissions(permissions)
            .map_err(crate::UiFailure::internal)?;
    }
    agena_storage_layout::secure_file(staged.path()).map_err(crate::UiFailure::internal)?;
    staged
        .persist(path)
        .map_err(|error| crate::UiFailure::internal(error.error))?;
    Ok(())
}

impl DraftStore {
    pub(crate) fn load(path: &Path) -> UiResult<Self> {
        let raw = match fs::read_to_string(path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => return Err(crate::UiFailure::internal(error)),
        };
        let persistent = serde_json::from_str::<PersistentDraftStore>(raw.as_str())
            .map_err(crate::UiFailure::internal)?;
        Ok(persistent.into_store())
    }

    pub(crate) fn persist(&self, path: &Path) -> UiResult<()> {
        let persistent = PersistentDraftStore::from_store(self);
        if persistent.is_empty() {
            match fs::remove_file(path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(crate::UiFailure::internal(error)),
            }
            return Ok(());
        }

        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            agena_storage_layout::secure_directory(parent).map_err(crate::UiFailure::internal)?;
        }

        let raw = serde_json::to_string_pretty(&persistent).map_err(crate::UiFailure::internal)?;
        persist_atomically(path, raw.as_bytes())
    }

    pub(crate) fn get(&self, slot: DraftSlot) -> Option<&ComposerDraft> {
        self.drafts.get(&slot)
    }

    pub(crate) fn set(&mut self, slot: DraftSlot, draft: ComposerDraft) -> bool {
        if draft.is_empty() {
            return self.clear(slot);
        }
        match self.drafts.get(&slot) {
            Some(existing) if existing == &draft => false,
            _ => {
                self.drafts.insert(slot, draft);
                true
            }
        }
    }

    pub(crate) fn clear(&mut self, slot: DraftSlot) -> bool {
        self.drafts.remove(&slot).is_some()
    }
}

impl PromptHistory {
    pub(crate) fn load(path: &Path) -> UiResult<Self> {
        let raw = match fs::read_to_string(path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => return Err(crate::UiFailure::internal(error)),
        };

        let mut items = Vec::new();
        for line in raw.lines() {
            let line = line.trim();
            if line.is_empty() {
                return Err(crate::UiFailure::message(
                    "prompt history contains an empty record; delete the file and recreate current history",
                ));
            }
            let entry = serde_json::from_str::<PromptHistoryRecord>(line)
                .map_err(crate::UiFailure::internal)?;
            let text = Self::normalized_text(entry.text.as_str()).ok_or_else(|| {
                crate::UiFailure::message(
                    "prompt history contains an empty prompt; delete the file and recreate current history",
                )
            })?;
            if text != entry.text {
                return Err(crate::UiFailure::message(
                    "prompt history contains a non-canonical prompt; delete the file and recreate current history",
                ));
            }
            if items.iter().any(|item| item == &text) {
                return Err(crate::UiFailure::message(
                    "prompt history contains duplicate prompts; delete the file and recreate current history",
                ));
            }
            items.push(text);
            if items.len() > MAX_PROMPT_HISTORY_ENTRIES {
                return Err(crate::UiFailure::message(
                    "prompt history exceeds the current entry limit; delete the file and recreate current history",
                ));
            }
        }
        Ok(Self { items })
    }

    pub(crate) fn persist(&self, path: &Path) -> UiResult<()> {
        if self.items.is_empty() {
            match fs::remove_file(path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(crate::UiFailure::internal(error)),
            }
            return Ok(());
        }

        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            agena_storage_layout::secure_directory(parent).map_err(crate::UiFailure::internal)?;
        }

        let mut raw = String::new();
        for text in &self.items {
            let line = serde_json::to_string(&PromptHistoryRecord { text: text.clone() })
                .map_err(crate::UiFailure::internal)?;
            raw.push_str(line.as_str());
            raw.push('\n');
        }

        persist_atomically(path, raw.as_bytes())
    }

    pub(crate) fn normalized_text(text: &str) -> Option<String> {
        let trimmed = text.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    }

    pub(crate) fn push(&mut self, text: String) -> bool {
        if self.items.last().is_some_and(|item| item == &text) {
            return false;
        }
        self.items.retain(|item| item != &text);
        self.items.push(text);
        if self.items.len() > MAX_PROMPT_HISTORY_ENTRIES {
            let excess = self.items.len() - MAX_PROMPT_HISTORY_ENTRIES;
            self.items.drain(0..excess);
        }
        true
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}
use crate::{
    ComposerDraft, DraftSlot, DraftStore, MAX_PROMPT_HISTORY_ENTRIES, Path, PersistentDraftStore,
    PromptHistory, PromptHistoryRecord, UiResult, fs,
};
