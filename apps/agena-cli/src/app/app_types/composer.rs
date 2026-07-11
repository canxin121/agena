use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Range,
    path::PathBuf,
};

use agena_tui_components::{Editor, QuerySuggestionState, SuggestionPopupState};
use ratatui::{layout::Rect, style::Style, text::Line};
use serde::{Deserialize, Serialize};

use crate::commands::CommandSpec;

use super::{
    PermissionMode, PermissionRuleDraft, PermissionRuleSubjectKind, RenderedTranscriptNode,
    persistent_draft_store_version,
};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ComposerDraft {
    pub text: String,
    pub items: Vec<ComposerItem>,
    pub elements: Vec<ComposerDraftElement>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComposerItem {
    Attachment(StagedAttachment),
    LargePaste(StagedPaste),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagedAttachment {
    pub(crate) path: PathBuf,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposerDraftElement {
    pub(crate) placeholder: String,
    pub range: Range<usize>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct SlashCommandSuggestionMeta;

pub(crate) type SlashCommandSuggestionState =
    SuggestionPopupState<SlashCommandSuggestionItem, SlashCommandSuggestionMeta>;

#[derive(Debug, Clone)]
pub(crate) struct SlashCommandSuggestionItem {
    pub(crate) label: String,
    pub(crate) detail: String,
    pub(crate) value: SlashCommandSuggestionValue,
}

#[derive(Debug, Clone)]
pub(crate) enum SlashCommandSuggestionValue {
    Command(&'static CommandSpec),
    RuntimeTool(String),
}

#[derive(Debug, Clone)]
pub(crate) struct SlashCommandSuggestionContext {
    pub(crate) query: String,
    pub(crate) fingerprint: String,
    pub(crate) name_range: Range<usize>,
}

#[derive(Debug, Clone)]
pub(crate) struct FileMentionSuggestionMeta {
    pub(crate) mention_range: Range<usize>,
}

pub(crate) type FileMentionSuggestionState =
    SuggestionPopupState<FileMentionSuggestionItem, FileMentionSuggestionMeta>;

#[derive(Debug, Clone)]
pub(crate) struct FileMentionSuggestionItem {
    pub(crate) path: PathBuf,
    pub(crate) label: String,
    pub(crate) detail: String,
}

#[derive(Debug, Clone)]
pub(crate) struct FileMentionSuggestionContext {
    pub(crate) query: String,
    pub(crate) fingerprint: String,
    pub(crate) mention_range: Range<usize>,
}

#[derive(Debug, Clone)]
pub(crate) struct PromptHistorySearchMeta {
    pub(crate) original: ComposerDraft,
}

pub(crate) type PromptHistorySearchState =
    QuerySuggestionState<PromptHistorySearchResult, PromptHistorySearchMeta, Editor>;

#[derive(Debug, Clone)]
pub(crate) struct PromptHistorySearchResult {
    pub(crate) history_index: usize,
    pub(crate) text: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct UserInputAnswerDraft {
    pub(crate) option_indexes: BTreeSet<usize>,
    pub(crate) custom_values: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub(crate) struct PersistentDraftStore {
    #[serde(default = "persistent_draft_store_version")]
    pub(crate) version: u32,
    #[serde(default)]
    pub(crate) sessions: BTreeMap<i64, PersistentComposerDraft>,
    #[serde(default)]
    pub(crate) new_session: Option<PersistentComposerDraft>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct PersistentComposerDraft {
    pub(crate) text: String,
    pub(crate) items: Vec<PersistentComposerItem>,
    pub(crate) elements: Vec<PersistentComposerDraftElement>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) enum PersistentComposerItem {
    Attachment(PersistentAttachment),
    LargePaste(PersistentPaste),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct PersistentAttachment {
    pub(crate) path: PathBuf,
    pub(crate) placeholder: String,
    pub(crate) label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct PersistentPaste {
    pub(crate) placeholder: String,
    pub(crate) label: String,
    pub(crate) text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct PersistentComposerDraftElement {
    pub(crate) placeholder: String,
    pub(crate) start: usize,
    pub(crate) end: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct PromptHistoryRecord {
    pub(crate) text: String,
}

#[derive(Debug, Clone)]
pub(crate) struct RenderedTranscript {
    pub(crate) width: u16,
    pub(crate) lines: Vec<RenderedLine>,
    pub(crate) search_matches: Vec<usize>,
    pub(crate) message_line_starts: Vec<(i64, usize)>,
    pub(crate) nodes: Vec<RenderedTranscriptNode>,
    pub(crate) line_nodes: Vec<Option<usize>>,
}

#[derive(Debug, Clone)]
pub(crate) struct RenderedLine {
    pub(crate) text: String,
    pub(crate) style: Style,
    pub(crate) rich_line: Option<Line<'static>>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct TranscriptDetailDefaults {
    pub(crate) tool_output_expanded: bool,
    pub(crate) thinking_expanded: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TranscriptMoveDirection {
    Up,
    Down,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct LayoutCache {
    pub(crate) transcript_body: Rect,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ToolOutputPreview {
    pub(crate) text: String,
    pub(crate) omitted_lines: usize,
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
