use std::{collections::BTreeMap, ops::Range, path::PathBuf};

use serde::{Deserialize, Serialize};

use agena_domain::{ComposerActivity, ComposerDocument};

use super::{PermissionMode, PermissionRuleDraft, PermissionRuleSubjectKind};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ComposerDraft {
    pub document: ComposerDocument,
}

/// One structured composer node. The payload is the same Activity payload
/// submitted to Runtime; the editor token is merely its inline rendering.
#[derive(Debug, Clone, PartialEq)]
pub struct ComposerItem {
    pub(crate) activity: ComposerActivity,
    pub(crate) placeholder: String,
    pub(crate) label: String,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub(crate) struct PersistentDraftStore {
    pub(crate) version: u32,
    pub(crate) sessions: BTreeMap<i64, PersistentComposerDraft>,
    pub(crate) new_session: Option<PersistentComposerDraft>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct PersistentComposerDraft {
    pub(crate) document: ComposerDocument,
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
            mode: PermissionMode::Auto,
        }
    }
}
