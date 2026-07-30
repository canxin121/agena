impl DraftStore {
    pub(crate) fn load(path: &Path) -> UiResult<Self> {
        let raw = match fs::read_to_string(path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => return Err(error.to_string()),
        };
        let persistent = serde_json::from_str::<PersistentDraftStore>(raw.as_str())
            .map_err(|error| format!("invalid draft store {}: {error}", path.display()))?;
        if persistent.version != crate::persistent_draft_store_version() {
            return Err(format!(
                "unsupported draft schema {}; expected {}",
                persistent.version,
                crate::persistent_draft_store_version()
            ));
        }
        Ok(persistent.into_store())
    }

    pub(crate) fn persist(&self, path: &Path) -> UiResult<()> {
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

    pub(crate) fn persist(&self, path: &Path) -> UiResult<()> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_pre_typed_terminal_response_draft_schema() {
        let directory = tempfile::tempdir().expect("temporary draft directory");
        let path = directory.path().join("tui-drafts.json");
        fs::write(
            &path,
            r#"{
                "version": 1,
                "sessions": {},
                "new_session": {
                    "text": "4;-2;rgb:fae0/fae0/fae0",
                    "items": [],
                    "elements": []
                }
            }"#,
        )
        .expect("write legacy draft store");

        assert!(DraftStore::load(&path).is_err());
    }

    #[test]
    fn rejects_legacy_or_forward_compatible_draft_shapes() {
        let directory = tempfile::tempdir().expect("temporary draft directory");

        let missing_version = directory.path().join("missing-version.json");
        fs::write(&missing_version, r#"{"sessions": {}, "new_session": null}"#)
            .expect("write versionless draft store");
        assert!(DraftStore::load(&missing_version).is_err());

        let unknown_field = directory.path().join("unknown-field.json");
        fs::write(
            &unknown_field,
            format!(
                r#"{{"version": {}, "sessions": {{}}, "new_session": null, "legacy": true}}"#,
                crate::persistent_draft_store_version()
            ),
        )
        .expect("write draft store with an unknown field");
        assert!(DraftStore::load(&unknown_field).is_err());
    }

    #[test]
    fn current_draft_schema_preserves_arbitrary_user_text() {
        let directory = tempfile::tempdir().expect("temporary draft directory");
        let path = directory.path().join("tui-drafts.json");
        let mut store = DraftStore::default();
        assert!(store.set(
            DraftSlot::NewSession,
            ComposerDraft {
                // The persistence layer intentionally has no protocol-payload
                // classifier. Once bytes are typed correctly, this is valid
                // user text like any other string.
                document: agena_domain::ComposerDocument(vec![agena_domain::ComposerNode::Text {
                    text: "4;-2;rgb:fae0/fae0/fae0".to_string(),
                },]),
            }
        ));
        store.persist(&path).expect("persist current draft store");

        let restored = DraftStore::load(&path).expect("load current draft store");
        assert_eq!(
            restored.get(DraftSlot::NewSession).map(ComposerDraft::text),
            Some("4;-2;rgb:fae0/fae0/fae0".to_owned())
        );
    }
}
