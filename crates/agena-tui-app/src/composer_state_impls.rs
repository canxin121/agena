use agena_domain::{ActivityPayload, ComposerNode};

use crate::{
    BTreeMap, ComposerDraft, ComposerItem, DraftSlot, DraftStore, PersistentComposerDraft,
    PersistentDraftStore, persistent_draft_store_version,
};

impl ComposerDraft {
    pub(crate) fn is_empty(&self) -> bool {
        self.document.is_empty()
    }

    pub(crate) fn text(&self) -> String {
        self.document.text()
    }

    pub(crate) fn render_text(&self) -> String {
        self.document
            .0
            .iter()
            .map(|node| match node {
                ComposerNode::Text { text } => text.clone(),
                ComposerNode::Activity { activity } => {
                    composer_activity_presentation(&activity.payload).0
                }
            })
            .collect::<Vec<_>>()
            .concat()
    }

    pub(crate) fn activities(&self) -> impl Iterator<Item = &agena_domain::ComposerActivity> {
        self.document.0.iter().filter_map(|node| match node {
            ComposerNode::Activity { activity } => Some(activity.as_ref()),
            ComposerNode::Text { .. } => None,
        })
    }

    pub(crate) fn persistent_snapshot(&self) -> Option<PersistentComposerDraft> {
        (!self.is_empty()).then(|| PersistentComposerDraft {
            document: self.document.clone(),
        })
    }
}

impl ComposerItem {
    pub(crate) fn placeholder(&self) -> &str {
        self.placeholder.as_str()
    }

    pub(crate) fn short_label(&self) -> &str {
        self.label.as_str()
    }

    pub(crate) fn payload(&self) -> &ActivityPayload {
        &self.activity.payload
    }
}

pub(crate) fn composer_activity_presentation(payload: &ActivityPayload) -> (String, String) {
    match payload {
        ActivityPayload::Resource(resource) => {
            let noun = if resource.kind == agena_domain::ResourceKind::Directory {
                "folder"
            } else {
                "file"
            };
            (
                format!("[{noun}: {}]", resource.name),
                format!("{noun}: {}", resource.name),
            )
        }
        ActivityPayload::SkillReference(skill) => (
            format!("[Skill: {}]", skill.name),
            format!("Skill: {}", skill.name),
        ),
        ActivityPayload::TextArtifact(artifact) => {
            let label = crate::ui_text::text_artifact_display_label(
                artifact.text.as_str(),
                artifact.label.as_deref(),
            );
            (format!("[{label}]"), label)
        }
        _ => ("[activity]".to_owned(), "activity".to_owned()),
    }
}

impl PersistentDraftStore {
    pub(crate) fn is_empty(&self) -> bool {
        self.new_session.is_none() && self.sessions.is_empty()
    }

    pub(crate) fn from_store(store: &DraftStore) -> Self {
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

    pub(crate) fn into_store(self) -> DraftStore {
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
    pub(crate) fn into_draft(self) -> ComposerDraft {
        ComposerDraft {
            document: self.document,
        }
    }
}
