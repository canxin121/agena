use std::{collections::BTreeMap, ops::Range, path::PathBuf, sync::Arc};

use serde::{Deserialize, Serialize};

use agena_plugin_sdk::AttachmentItem;

use super::{PermissionMode, PermissionRuleDraft, PermissionRuleSubjectKind};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ComposerDraft {
    pub text: String,
    pub items: Vec<ComposerItem>,
    pub elements: Vec<ComposerDraftElement>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComposerItem {
    Attachment(Box<StagedAttachment>),
    LargePaste(StagedPaste),
    SkillReference(StagedSkillReference),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagedAttachment {
    pub(crate) path: PathBuf,
    /// Immutable content snapshot created when the attachment is staged.
    /// Legacy drafts loaded from disk may leave this empty and are upgraded on
    /// their next successful staging/submission path.
    pub(crate) prepared: Option<Arc<AttachmentItem>>,
    pub(crate) cleanup_root: Option<PathBuf>,
    pub(crate) placeholder: String,
    pub(crate) label: String,
    pub(crate) is_temp: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagedPaste {
    pub(crate) placeholder: String,
    pub(crate) label: String,
    pub(crate) text: String,
}

/// Immutable Skill text selected for this one outgoing message. This is a
/// composer attachment only: it cannot activate a Skill, modify permissions,
/// or select a model for the session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagedSkillReference {
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) instructions: String,
    pub(crate) content_hash: String,
    pub(crate) source: String,
    pub(crate) aliases: Vec<String>,
    pub(crate) placeholder: String,
    pub(crate) label: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposerDraftElement {
    pub(crate) placeholder: String,
    pub range: Range<usize>,
}

#[derive(Debug, Clone)]
pub(crate) struct SlashCommandSuggestionAction {
    pub(crate) slash_name: String,
    pub(crate) can_submit_without_arguments: bool,
}

pub(crate) use agena_tui::slash_commands::{
    SlashCommandSuggestionItem, SlashCommandSuggestionMeta, SlashCommandSuggestionState,
};

#[derive(Debug, Clone)]
pub(crate) struct SlashCommandSuggestionContext {
    pub(crate) query: String,
    pub(crate) fingerprint: String,
    pub(crate) name_range: Range<usize>,
}

#[derive(Debug, Clone)]
pub(crate) struct FileMentionSuggestionAction {
    pub(crate) path: PathBuf,
}

pub(crate) use agena_tui::file_mentions::{FileMentionSuggestionItem, FileMentionSuggestionState};

#[derive(Debug, Clone)]
pub(crate) struct FileMentionSuggestionContext {
    pub(crate) query: String,
    pub(crate) fingerprint: String,
    pub(crate) mention_range: Range<usize>,
}

pub(crate) use agena_tui::prompt_history::{PromptHistorySearchResult, PromptHistorySearchState};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub(crate) struct PersistentDraftStore {
    pub(crate) version: u32,
    pub(crate) sessions: BTreeMap<i64, PersistentComposerDraft>,
    pub(crate) new_session: Option<PersistentComposerDraft>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct PersistentComposerDraft {
    pub(crate) text: String,
    pub(crate) items: Vec<PersistentComposerItem>,
    pub(crate) elements: Vec<PersistentComposerDraftElement>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) enum PersistentComposerItem {
    Attachment(PersistentAttachment),
    LargePaste(PersistentPaste),
    SkillReference(PersistentSkillReference),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct PersistentAttachment {
    pub(crate) path: PathBuf,
    pub(crate) placeholder: String,
    pub(crate) label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct PersistentPaste {
    pub(crate) placeholder: String,
    pub(crate) label: String,
    pub(crate) text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct PersistentSkillReference {
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) instructions: String,
    pub(crate) content_hash: String,
    pub(crate) source: String,
    pub(crate) aliases: Vec<String>,
    pub(crate) placeholder: String,
    pub(crate) label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct PersistentComposerDraftElement {
    pub(crate) placeholder: String,
    pub(crate) start: usize,
    pub(crate) end: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct PromptHistoryRecord {
    pub(crate) text: String,
}

impl Default for PermissionRuleDraft {
    fn default() -> Self {
        Self {
            subject_kind: PermissionRuleSubjectKind::Tool,
            tool_name: String::new(),
            qualifier: String::new(),
            path_access_kind: "read".to_string(),
            workspace_root: String::new(),
            target_path: String::new(),
            network_target: String::new(),
            network_host: String::new(),
            network_port: String::new(),
            scope: "workspace".to_string(),
            session_id: String::new(),
            mode: PermissionMode::Ask,
        }
    }
}
