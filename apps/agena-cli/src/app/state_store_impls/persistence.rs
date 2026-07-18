impl DraftStore {
    pub(in crate::app) fn load(path: &Path) -> UiResult<Self> {
        let raw = match fs::read_to_string(path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => return Err(error.to_string()),
        };
        let persistent = serde_json::from_str::<PersistentDraftStore>(raw.as_str())
            .map_err(|error| format!("invalid draft store {}: {error}", path.display()))?;
        Ok(persistent.into_store())
    }

    pub(in crate::app) fn persist(&self, path: &Path) -> UiResult<()> {
        let persistent = PersistentDraftStore::from_store(self);
        if persistent.is_empty() {
            match fs::remove_file(path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.to_string()),
            }
            return Ok(());
        }

        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }

        let raw = serde_json::to_string_pretty(&persistent).map_err(|error| error.to_string())?;
        let tmp_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| format!("{name}.tmp"))
            .unwrap_or_else(|| "tui-drafts.json.tmp".to_string());
        let tmp_path = path.with_file_name(tmp_name);
        fs::write(&tmp_path, raw).map_err(|error| error.to_string())?;
        fs::rename(&tmp_path, path).map_err(|error| error.to_string())?;
        Ok(())
    }

    pub(in crate::app) fn get(&self, slot: DraftSlot) -> Option<&ComposerDraft> {
        self.drafts.get(&slot)
    }

    pub(in crate::app) fn set(&mut self, slot: DraftSlot, draft: ComposerDraft) -> bool {
        if draft.is_empty()
            || (draft.items.is_empty()
                && draft.elements.is_empty()
                && crate::terminal::is_terminal_color_response_text(&draft.text))
        {
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

    pub(in crate::app) fn clear(&mut self, slot: DraftSlot) -> bool {
        self.drafts.remove(&slot).is_some()
    }
}

impl PromptHistory {
    pub(in crate::app) fn load(path: &Path) -> UiResult<Self> {
        let raw = match fs::read_to_string(path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => return Err(error.to_string()),
        };

        let mut items = Vec::new();
        for (index, line) in raw.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let entry = serde_json::from_str::<PromptHistoryRecord>(line).map_err(|error| {
                format!(
                    "invalid prompt history {}:{}: {error}",
                    path.display(),
                    index + 1
                )
            })?;
            if let Some(text) = Self::normalized_text(entry.text.as_str()) {
                if items.last().is_some_and(|item| item == &text) {
                    continue;
                }
                items.retain(|item| item != &text);
                items.push(text);
                if items.len() > MAX_PROMPT_HISTORY_ENTRIES {
                    let excess = items.len() - MAX_PROMPT_HISTORY_ENTRIES;
                    items.drain(0..excess);
                }
            }
        }
        Ok(Self { items })
    }

    pub(in crate::app) fn persist(&self, path: &Path) -> UiResult<()> {
        if self.items.is_empty() {
            match fs::remove_file(path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.to_string()),
            }
            return Ok(());
        }

        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }

        let mut raw = String::new();
        for text in &self.items {
            let line = serde_json::to_string(&PromptHistoryRecord { text: text.clone() })
                .map_err(|error| error.to_string())?;
            raw.push_str(line.as_str());
            raw.push('\n');
        }

        let tmp_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| format!("{name}.tmp"))
            .unwrap_or_else(|| "tui-prompt-history.jsonl.tmp".to_string());
        let tmp_path = path.with_file_name(tmp_name);
        fs::write(&tmp_path, raw).map_err(|error| error.to_string())?;
        fs::rename(&tmp_path, path).map_err(|error| error.to_string())?;
        Ok(())
    }

    pub(in crate::app) fn normalized_text(text: &str) -> Option<String> {
        let trimmed = text.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    }

    pub(in crate::app) fn push(&mut self, text: String) -> bool {
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

    pub(in crate::app) fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}
use crate::app::{
    ComposerDraft, DraftSlot, DraftStore, MAX_PROMPT_HISTORY_ENTRIES, Path, PersistentDraftStore,
    PromptHistory, PromptHistoryRecord, UiResult, fs,
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::PersistentComposerDraft;

    #[test]
    fn terminal_color_residue_is_never_restored_or_persisted_as_a_draft() {
        let residue = ComposerDraft {
            text: "4;-2;rgb:fae0/fae0/fae0".to_string(),
            ..ComposerDraft::default()
        };
        let mut store = DraftStore::default();
        assert!(store.set(
            DraftSlot::Session(7),
            ComposerDraft {
                text: "real draft".to_string(),
                ..ComposerDraft::default()
            }
        ));
        assert!(store.set(DraftSlot::Session(7), residue));
        assert!(store.get(DraftSlot::Session(7)).is_none());

        let restored = PersistentDraftStore {
            sessions: [(
                9,
                PersistentComposerDraft {
                    text: "11;rgb:ffff/eeee/dddd".to_string(),
                    items: Vec::new(),
                    elements: Vec::new(),
                },
            )]
            .into_iter()
            .collect(),
            ..PersistentDraftStore::default()
        }
        .into_store();
        assert!(restored.get(DraftSlot::Session(9)).is_none());

        assert!(store.set(
            DraftSlot::Session(10),
            ComposerDraft {
                text: "4;-2;rgb:fae0/fae0/not-a-color".to_string(),
                ..ComposerDraft::default()
            }
        ));
        assert!(store.get(DraftSlot::Session(10)).is_some());
    }
}
