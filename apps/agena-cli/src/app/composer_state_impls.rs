impl ComposerDraft {
    pub(in crate::app) fn is_empty(&self) -> bool {
        self.text.trim().is_empty() && self.items.is_empty()
    }

    pub(in crate::app) fn persistent_snapshot(&self) -> Option<PersistentComposerDraft> {
        let mut items_by_placeholder = self
            .items
            .iter()
            .filter_map(|item| {
                item.persistent_item()
                    .map(|persistent| (item.placeholder().to_string(), persistent))
            })
            .collect::<BTreeMap<_, _>>();
        let mut elements = self.elements.clone();
        elements.sort_by_key(|element| element.range.start);

        let mut text = String::new();
        let mut persistent_items = Vec::new();
        let mut persistent_elements = Vec::new();
        let mut cursor = 0;

        for element in elements {
            let start = min(element.range.start, self.text.len());
            let end = min(element.range.end, self.text.len());
            if cursor < start {
                text.push_str(&self.text[cursor..start]);
            }

            if let Some(placeholder) = self.text.get(start..end)
                && let Some(item) = items_by_placeholder.remove(placeholder)
            {
                let range = text.len()..text.len() + placeholder.len();
                text.push_str(placeholder);
                persistent_items.push(item);
                persistent_elements.push(PersistentComposerDraftElement {
                    placeholder: placeholder.to_string(),
                    start: range.start,
                    end: range.end,
                });
            }

            cursor = end;
        }

        if cursor < self.text.len() {
            text.push_str(&self.text[cursor..]);
        }

        let draft = PersistentComposerDraft {
            text,
            items: persistent_items,
            elements: persistent_elements,
        };
        (!draft.text.trim().is_empty() || !draft.items.is_empty()).then_some(draft)
    }
}

impl ComposerItem {
    pub(in crate::app) fn placeholder(&self) -> &str {
        match self {
            Self::Attachment(attachment) => attachment.placeholder.as_str(),
            Self::LargePaste(paste) => paste.placeholder.as_str(),
        }
    }

    pub(in crate::app) fn short_label(&self) -> &str {
        match self {
            Self::Attachment(attachment) => attachment.label.as_str(),
            Self::LargePaste(paste) => paste.label.as_str(),
        }
    }

    pub(in crate::app) fn persistent_item(&self) -> Option<PersistentComposerItem> {
        match self {
            Self::Attachment(attachment) => (!attachment.is_temp).then(|| {
                PersistentComposerItem::Attachment(PersistentAttachment {
                    path: attachment.path.clone(),
                    placeholder: attachment.placeholder.clone(),
                    label: attachment.label.clone(),
                })
            }),
            Self::LargePaste(paste) => Some(PersistentComposerItem::LargePaste(PersistentPaste {
                placeholder: paste.placeholder.clone(),
                label: paste.label.clone(),
                text: paste.text.clone(),
            })),
        }
    }
}

impl PersistentDraftStore {
    pub(in crate::app) fn is_empty(&self) -> bool {
        self.new_session.is_none() && self.sessions.is_empty()
    }

    pub(in crate::app) fn from_store(store: &DraftStore) -> Self {
        let mut sessions = BTreeMap::new();
        let mut new_session = None;

        for (slot, draft) in &store.drafts {
            let Some(persistent) = draft.persistent_snapshot() else {
                continue;
            };
            match slot {
                DraftSlot::Session(session_id) => {
                    sessions.insert(*session_id, persistent);
                }
                DraftSlot::NewSession => {
                    new_session = Some(persistent);
                }
            }
        }

        Self {
            version: persistent_draft_store_version(),
            sessions,
            new_session,
        }
    }

    pub(in crate::app) fn into_store(self) -> DraftStore {
        let mut drafts = BTreeMap::new();
        if let Some(draft) = self.new_session {
            drafts.insert(DraftSlot::NewSession, draft.into_draft());
        }
        for (session_id, draft) in self.sessions {
            drafts.insert(DraftSlot::Session(session_id), draft.into_draft());
        }
        DraftStore { drafts }
    }
}

impl PersistentComposerDraft {
    pub(in crate::app) fn into_draft(self) -> ComposerDraft {
        ComposerDraft {
            text: self.text,
            items: self
                .items
                .into_iter()
                .map(|item| item.into_item())
                .collect(),
            elements: self
                .elements
                .into_iter()
                .map(|element| ComposerDraftElement {
                    placeholder: element.placeholder,
                    range: element.start..element.end,
                })
                .collect(),
        }
    }
}

impl PersistentComposerItem {
    pub(in crate::app) fn into_item(self) -> ComposerItem {
        match self {
            Self::Attachment(attachment) => ComposerItem::Attachment(StagedAttachment {
                path: attachment.path,
                placeholder: attachment.placeholder,
                label: attachment.label,
                is_temp: false,
            }),
            Self::LargePaste(paste) => ComposerItem::LargePaste(StagedPaste {
                placeholder: paste.placeholder,
                label: paste.label,
                text: paste.text,
            }),
        }
    }
}
use crate::app::{
    BTreeMap, ComposerDraft, ComposerDraftElement, ComposerItem, DraftSlot, DraftStore,
    PersistentAttachment, PersistentComposerDraft, PersistentComposerDraftElement,
    PersistentComposerItem, PersistentDraftStore, PersistentPaste, StagedAttachment, StagedPaste,
    min, persistent_draft_store_version,
};
