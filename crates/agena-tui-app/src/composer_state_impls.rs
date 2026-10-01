use agena_domain::{ActivityPayload, ComposerNode};
use agena_plugin_sdk::AttachmentKind;
use agena_tui::i18n::I18n;

use crate::{
    BTreeMap, ComposerDraft, ComposerItem, DraftSlot, DraftStore, PersistentComposerDraft,
    PersistentDraftStore,
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

/// The composer item's human label, derived from its payload on demand. The
/// placeholder owns identity (editor ranges, draft persistence); the label is
/// only ever a projection of the activity facts.
pub(crate) fn composer_item_label(item: &ComposerItem, i18n: &I18n) -> String {
    match &item.activity.payload {
        ActivityPayload::Resource(resource) => {
            let mut label = crate::app_session_helpers::attachment_chip_label(
                i18n,
                resource_display_path(resource).as_path(),
                attachment_kind_for_resource(resource.kind),
                resource.kind == agena_domain::ResourceKind::Directory,
                resource.width,
                resource.height,
                resource.size_bytes.unwrap_or_default(),
            );
            if resource.delivery == agena_domain::ResourceDelivery::ModelInput {
                label.push_str(" · send contents to selected model");
            }
            label
        }
        ActivityPayload::SkillReference(command) => format!("Command: {}", command.name),
        ActivityPayload::TextArtifact(artifact) => crate::ui_text::text_artifact_display_label(
            artifact.text.as_str(),
            artifact.label.as_deref(),
        ),
        _ => composer_activity_presentation(&item.activity.payload).1,
    }
}

fn resource_display_path(resource: &agena_domain::ResourceActivity) -> std::path::PathBuf {
    match &resource.reference {
        agena_domain::ResourceReference::WorkspacePath { path } => std::path::PathBuf::from(path),
        agena_domain::ResourceReference::Url { url } => std::path::PathBuf::from(url),
        agena_domain::ResourceReference::Artifact { uri, .. } => std::path::PathBuf::from(uri),
        agena_domain::ResourceReference::ProviderFile { file_id, .. } => {
            std::path::PathBuf::from(file_id)
        }
    }
}

fn attachment_kind_for_resource(kind: agena_domain::ResourceKind) -> AttachmentKind {
    match kind {
        agena_domain::ResourceKind::Image => AttachmentKind::Image,
        agena_domain::ResourceKind::Audio => AttachmentKind::Audio,
        agena_domain::ResourceKind::Video => AttachmentKind::Video,
        agena_domain::ResourceKind::Pdf => AttachmentKind::Pdf,
        _ => AttachmentKind::File,
    }
}

impl ComposerItem {
    pub(crate) fn placeholder(&self) -> &str {
        self.placeholder.as_str()
    }

    pub(crate) fn is_completed(&self) -> bool {
        self.state == agena_api::part::PartExecutionStatusResource::Completed
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
        ActivityPayload::SkillReference(command) => (
            format!("[Command: {}]", command.name),
            format!("Command: {}", command.name),
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
