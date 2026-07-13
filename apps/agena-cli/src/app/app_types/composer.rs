use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet},
    ops::Range,
    path::PathBuf,
    sync::Arc,
};

use agena_tui_components::{Editor, SearchPicker, SearchPickerItem, SearchPickerNoCustom, theme};
use ratatui::{layout::Rect, style::Style, text::Line};
use serde::{Deserialize, Serialize};

use crate::{
    commands::CommandSpec,
    math_render::{MathLinePlacement, TranscriptMathPlacement},
};
use agena::message::AttachmentItem;

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
    Attachment(Box<StagedAttachment>),
    LargePaste(StagedPaste),
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposerDraftElement {
    pub(crate) placeholder: String,
    pub range: Range<usize>,
}

#[derive(Debug, Clone)]
pub(crate) struct SlashCommandSuggestionMeta {
    pub(crate) fingerprint: String,
}

pub(crate) type SlashCommandSuggestionState = SearchPicker<
    SlashCommandSuggestionItem,
    SearchPickerNoCustom,
    SlashCommandSuggestionMeta,
    Editor,
>;

#[derive(Debug, Clone)]
pub(crate) struct SlashCommandSuggestionItem {
    pub(crate) label: String,
    pub(crate) detail: String,
    pub(crate) value: SlashCommandSuggestionValue,
}

#[derive(Debug, Clone)]
pub(crate) enum SlashCommandSuggestionValue {
    Command(&'static CommandSpec),
    PluginCommand(Box<agena::plugin::PluginCommandCatalogItem>),
}

impl SearchPickerItem for SlashCommandSuggestionItem {
    fn search_picker_key(&self) -> Cow<'_, str> {
        match &self.value {
            SlashCommandSuggestionValue::Command(spec) => {
                Cow::Owned(format!("command:{}", spec.name))
            }
            SlashCommandSuggestionValue::PluginCommand(entry) => Cow::Owned(format!(
                "plugin-command:{}:{}",
                entry.plugin_id, entry.command.id
            )),
        }
    }

    fn search_picker_label(&self) -> Cow<'_, str> {
        Cow::Borrowed(self.label.as_str())
    }

    fn search_picker_detail(&self) -> Option<Cow<'_, str>> {
        Some(Cow::Borrowed(self.detail.as_str()))
    }

    fn search_picker_label_style(&self) -> Style {
        Style::default()
            .fg(theme::accent_color())
            .add_modifier(ratatui::style::Modifier::BOLD)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct SlashCommandSuggestionContext {
    pub(crate) query: String,
    pub(crate) fingerprint: String,
    pub(crate) name_range: Range<usize>,
}

#[derive(Debug, Clone)]
pub(crate) struct FileMentionSuggestionMeta {
    pub(crate) fingerprint: String,
    pub(crate) mention_range: Range<usize>,
}

pub(crate) type FileMentionSuggestionState = SearchPicker<
    FileMentionSuggestionItem,
    SearchPickerNoCustom,
    FileMentionSuggestionMeta,
    Editor,
>;

#[derive(Debug, Clone)]
pub(crate) struct FileMentionSuggestionItem {
    pub(crate) path: PathBuf,
    pub(crate) label: String,
    pub(crate) detail: String,
}

impl SearchPickerItem for FileMentionSuggestionItem {
    fn search_picker_key(&self) -> Cow<'_, str> {
        Cow::Owned(self.path.display().to_string())
    }

    fn search_picker_label(&self) -> Cow<'_, str> {
        Cow::Borrowed(self.label.as_str())
    }

    fn search_picker_detail(&self) -> Option<Cow<'_, str>> {
        Some(Cow::Borrowed(self.detail.as_str()))
    }

    fn search_picker_label_style(&self) -> Style {
        Style::default()
            .fg(theme::info_color())
            .add_modifier(ratatui::style::Modifier::BOLD)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct FileMentionSuggestionContext {
    pub(crate) query: String,
    pub(crate) fingerprint: String,
    pub(crate) mention_range: Range<usize>,
}

pub(crate) type PromptHistorySearchState =
    SearchPicker<PromptHistorySearchResult, SearchPickerNoCustom, (), Editor>;

#[derive(Debug, Clone)]
pub(crate) struct PromptHistorySearchResult {
    pub(crate) history_index: usize,
    pub(crate) text: String,
}

impl SearchPickerItem for PromptHistorySearchResult {
    fn search_picker_key(&self) -> Cow<'_, str> {
        Cow::Owned(self.history_index.to_string())
    }

    fn search_picker_label(&self) -> Cow<'_, str> {
        Cow::Borrowed(self.text.as_str())
    }

    fn search_picker_detail(&self) -> Option<Cow<'_, str>> {
        None
    }

    fn search_picker_prefix(&self) -> Option<Cow<'_, str>> {
        Some(Cow::Owned(format!("#{:<3} ", self.history_index + 1)))
    }

    fn search_picker_fill_value(&self) -> Cow<'_, str> {
        Cow::Borrowed(self.text.as_str())
    }

    fn search_picker_search_text(&self) -> Cow<'_, str> {
        Cow::Borrowed(self.text.as_str())
    }
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
    pub(crate) math: Vec<TranscriptMathPlacement>,
}

#[derive(Debug, Clone)]
pub(crate) struct RenderedLine {
    pub(crate) text: String,
    pub(crate) style: Style,
    pub(crate) rich_line: Option<Line<'static>>,
    pub(crate) math: Vec<MathLinePlacement>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct TranscriptDetailDefaults {
    pub(crate) activity_expanded: bool,
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
