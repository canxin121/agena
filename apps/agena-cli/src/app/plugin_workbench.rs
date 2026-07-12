use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

use crossterm::event::KeyEvent;
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Paragraph, Wrap},
};
use regex::Regex;
use serde_json::{Map as JsonMap, Number as JsonNumber, Value as JsonValue, json};

use agena_tui_components::{
    Editor, EditorDialogKeyResult, EditorDialogSpec, EditorDialogState, FramedSurfaceSpec,
    SectionedListFocus, SectionedListSection, SectionedListState, SurfaceMode, TextPanelSpec,
    drive_editor_dialog_key, render_editor_dialog, render_framed_surface, render_text_panel,
};

mod plugin_workbench_policy;
mod workbench_config;
mod workbench_config_actions;
mod workbench_config_sections;
mod workbench_config_state;
mod workbench_display;
mod workbench_editor;
mod workbench_input;
mod workbench_navigation;
mod workbench_policy_builder;
mod workbench_render;
mod workbench_render_helpers;
mod workbench_schema_resolution;
mod workbench_schema_util;
mod workbench_schema_validation;
mod workbench_text_render;

pub(super) use self::workbench_config_actions::*;
pub(super) use self::workbench_config_sections::*;
pub(super) use self::workbench_config_state::*;
pub(super) use self::workbench_display::*;
pub(super) use self::workbench_policy_builder::*;
pub(super) use self::workbench_render_helpers::*;
pub(super) use self::workbench_schema_resolution::*;
pub(super) use self::workbench_schema_util::*;
pub(super) use self::workbench_schema_validation::*;
pub(super) use self::workbench_text_render::*;

const PLUGIN_WORKBENCH_LOG_LIMIT: usize = 80;

#[derive(Debug, Clone)]
pub(in crate::app) struct PluginWorkbenchOverlay {
    title: String,
    query: Editor,
    mode: PluginWorkbenchMode,
    transport_filter: PluginTransportFilter,
    config_filter: PluginConfigFilter,
    plugins: Vec<PluginWorkbenchPlugin>,
    visible_plugins: Vec<usize>,
    selected_plugin: usize,
    list_controls_focused: bool,
    selected_list_control: usize,
    detail_tab: PluginDetailTab,
    config_view: PluginConfigView,
    config_focus: PluginConfigFocus,
    selected_toolbar_action: usize,
    selected_section: usize,
    selected_node: usize,
    selected_cell: ConfigRowCell,
    selected_diagnostic: usize,
    selected_diff_row: usize,
    config_scroll: usize,
    diagnostics_scroll: usize,
    show_diff: bool,
    drilldown_stack: Vec<PluginConfigDrilldownOverlay>,
    actions: Option<PluginConfigActionOverlay>,
    selection: Option<PluginConfigSelectionOverlay>,
    editor: Option<PluginConfigEditOverlay>,
}

#[derive(Debug, Clone)]
pub(in crate::app) struct PluginPolicyStudioOverlay {
    title: String,
    footer: String,
    config_path: String,
    config_found: bool,
    selected_column: PluginPolicyColumn,
    visible_section_page_size: Cell<usize>,
    visible_item_page_size: Cell<usize>,
    state: SectionedListState<PluginPolicySection>,
}

#[derive(Debug, Clone)]
pub(super) struct PluginPolicySection {
    plugin_id: String,
    label: String,
    summary: String,
    description: String,
    items: Vec<PluginPolicyItem>,
}

impl SectionedListSection for PluginPolicySection {
    type Item = PluginPolicyItem;

    fn items(&self) -> &[Self::Item] {
        self.items.as_slice()
    }
}

#[derive(Debug, Clone)]
pub(super) struct PluginPolicyItem {
    key: String,
    label: String,
    scope_label: String,
    description: String,
    prompt_tool_default: Option<agena::plugin::ToolDescriptionMode>,
    prompt_plugin_declared_default: Option<agena::plugin::ToolDescriptionMode>,
    prompt_file_override: Option<agena::plugin::ToolDescriptionOverride>,
    prompt_effective_mode: agena::plugin::ToolDescriptionMode,
    prompt_source: PluginTextDisplaySource,
    prompt_path: String,
    ui_tool_default: Option<PluginTextDisplayMode>,
    ui_plugin_declared_default: Option<PluginTextDisplayMode>,
    ui_file_override: Option<agena::plugin::UiPresentationOverride>,
    ui_effective_mode: PluginTextDisplayMode,
    ui_source: PluginTextDisplaySource,
    ui_path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PluginPolicyColumn {
    Prompt,
    Ui,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PluginWorkbenchMode {
    List,
    Detail,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PluginTransportFilter {
    All,
    Static,
    Stdio,
    Cdylib,
    Http,
    Wasm,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PluginConfigFilter {
    All,
    Valid,
    Missing,
    SchemaMissing,
    Issues,
    NeedsRestart,
    RuntimeIssue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PluginDetailTab {
    Config,
    Tools,
    Commands,
    Capabilities,
    Logs,
    Diagnostics,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PluginConfigView {
    Effective,
}

impl PluginDetailTab {
    const ALL: [Self; 6] = [
        Self::Config,
        Self::Tools,
        Self::Commands,
        Self::Capabilities,
        Self::Logs,
        Self::Diagnostics,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Config => "Config",
            Self::Tools => "Tools",
            Self::Commands => "Commands",
            Self::Capabilities => "Capabilities",
            Self::Logs => "Logs",
            Self::Diagnostics => "Diagnostics",
        }
    }

    fn move_by(self, delta: isize) -> Self {
        let index = Self::ALL
            .iter()
            .position(|tab| *tab == self)
            .unwrap_or_default();
        let last = Self::ALL.len().saturating_sub(1) as isize;
        let next = (index as isize + delta).clamp(0, last) as usize;
        Self::ALL[next]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PluginTextDisplayMode {
    Detailed,
    Summary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PluginTextDisplaySource {
    ToolOverride,
    PluginOverride,
    ToolDefault,
    PluginDefault,
    GlobalDefault,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PluginConfigFocus {
    Toolbar,
    Structure,
    Editor,
    Diagnostics,
}

#[derive(Debug, Clone)]
pub(super) struct PluginWorkbenchPlugin {
    plugin_id: String,
    visible_tool: String,
    version: String,
    transport: String,
    ui_display_mode: PluginTextDisplayMode,
    ui_display_source: PluginTextDisplaySource,
    tool_ui_display_modes: BTreeMap<String, PluginTextDisplayMode>,
    tool_ui_display_defaults: BTreeMap<String, PluginTextDisplayMode>,
    tool_ui_display_sources: BTreeMap<String, PluginTextDisplaySource>,
    tools: Vec<agena::plugin::ToolDefinition>,
    commands: Vec<agena::plugin::PluginCommandDefinition>,
    config_status: PluginConfigStatus,
    status: agena::plugin::status::PluginStatus,
    inspect: Option<agena::plugin::PluginInspect>,
    configured_plugin_value: Option<JsonValue>,
    saved_override: JsonValue,
    draft_override: JsonValue,
    default_config: JsonValue,
    saved_config: JsonValue,
    draft_config: JsonValue,
    schema: Option<JsonValue>,
    schema_missing: bool,
    diagnostics: Vec<ConfigDiagnostic>,
    runtime_diagnostics: Vec<ConfigDiagnostic>,
    diff: Vec<ConfigDiffRow>,
    sections: Vec<ConfigSectionView>,
    logs: Vec<agena::plugin::PluginLogRecord>,
    dirty: bool,
    branch_drafts: BTreeMap<String, JsonValue>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum PluginConfigStatusKind {
    Valid,
    Missing,
    SchemaMissing,
    Invalid,
    Warning,
    NeedsRestart,
    RuntimeIssue,
}

#[derive(Debug, Clone)]
pub(super) struct PluginConfigStatus {
    kind: PluginConfigStatusKind,
    label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum PathSegment {
    Key(String),
    Index(usize),
}

pub(super) type ConfigPath = Vec<PathSegment>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DiagnosticSeverity {
    Error,
    Warning,
}

#[derive(Debug, Clone)]
pub(super) struct ConfigDiagnostic {
    severity: DiagnosticSeverity,
    source: String,
    path: ConfigPath,
    field: String,
    message: String,
}

#[derive(Debug, Clone)]
pub(super) struct ConfigDiffRow {
    path: ConfigPath,
    before: String,
    after: String,
    summary: String,
}

#[derive(Debug, Clone)]
pub(super) struct ConfigSectionView {
    key: String,
    title: String,
    issue_count: usize,
    dirty: bool,
    body: ConfigSectionBody,
}

#[derive(Debug, Clone)]
pub(super) enum ConfigSectionBody {
    Overview {
        cards: Vec<ConfigOverviewCard>,
        lines: Vec<String>,
    },
    Form {
        notice: Option<String>,
        groups: Vec<ConfigGroupView>,
    },
}

#[derive(Debug, Clone)]
pub(super) struct ConfigOverviewCard {
    title: String,
    summary: String,
    issue_label: Option<String>,
}

#[derive(Debug, Clone)]
pub(super) struct ConfigGroupView {
    title: String,
    layout: ConfigGroupLayout,
    rows: Vec<ConfigRowView>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ConfigGroupLayout {
    Standard,
    Pair {
        left_label: &'static str,
        right_label: &'static str,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ConfigRowCell {
    Type,
    Value,
    SecondaryValue,
    Default,
    Action,
    State,
}

#[derive(Debug, Clone)]
pub(super) struct ConfigRowView {
    title: String,
    primary_path: ConfigPath,
    additional_paths: Vec<ConfigPath>,
    editor: ConfigRowEditor,
    description: Option<String>,
    constraints: Vec<String>,
    type_display: String,
    type_mode: ConfigRowTypeMode,
    value_display: String,
    default_display: String,
    secondary_value_display: Option<String>,
    action_display: Option<String>,
    state: ConfigRowState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ConfigRowTypeMode {
    Fixed,
    SelectType,
    SelectShape,
}

impl ConfigRowTypeMode {
    fn is_switchable(self) -> bool {
        !matches!(self, Self::Fixed)
    }

    fn action_label(self) -> &'static str {
        match self {
            Self::Fixed => "Type",
            Self::SelectType => "Choose Type",
            Self::SelectShape => "Choose Shape",
        }
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub(super) enum ConfigRowEditor {
    Bool {
        path: ConfigPath,
    },
    Null {
        path: ConfigPath,
    },
    ReadOnly {
        path: ConfigPath,
    },
    Scalar {
        path: ConfigPath,
        kind: ScalarEditKind,
    },
    NullableString {
        path: ConfigPath,
    },
    Enum {
        path: ConfigPath,
        variants: Vec<JsonValue>,
    },
    MultiEnum {
        path: ConfigPath,
        variants: Vec<JsonValue>,
    },
    PairInteger {
        left_path: ConfigPath,
        right_path: ConfigPath,
    },
    Structured {
        path: ConfigPath,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ConfigRowState {
    Default,
    Override,
    Dirty,
    Error,
    Inactive,
}

impl ConfigRowState {
    fn label(self) -> &'static str {
        match self {
            Self::Default => "Default",
            Self::Override => "Changed",
            Self::Dirty => "Dirty",
            Self::Error => "Error",
            Self::Inactive => "Inactive",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CompactToolbarAction {
    Validate,
    ResetAll,
    Diff,
    Save,
    Restart,
}

const COMPACT_TOOLBAR_ACTIONS: [CompactToolbarAction; 5] = [
    CompactToolbarAction::Validate,
    CompactToolbarAction::ResetAll,
    CompactToolbarAction::Diff,
    CompactToolbarAction::Save,
    CompactToolbarAction::Restart,
];

impl CompactToolbarAction {
    fn label(self) -> &'static str {
        match self {
            Self::Validate => "Validate",
            Self::ResetAll => "Reset All",
            Self::Diff => "Diff",
            Self::Save => "Save",
            Self::Restart => "Restart",
        }
    }
}

pub(super) type PluginConfigEditOverlay = EditorDialogState<PluginConfigEditAction>;

#[derive(Debug, Clone)]
pub(super) struct PluginConfigDrilldownOverlay {
    plugin_id: String,
    path: ConfigPath,
    title: String,
    groups: Vec<ConfigGroupView>,
    selected_row: usize,
    selected_cell: ConfigRowCell,
}

#[derive(Debug, Clone)]
pub(super) struct PluginConfigActionOverlay {
    title: String,
    subject: String,
    footer: String,
    actions: Vec<PluginConfigActionItem>,
    selected_action: usize,
}

#[derive(Debug, Clone)]
pub(super) struct PluginConfigActionItem {
    label: String,
    description: String,
    action: PluginConfigAction,
}

#[derive(Debug, Clone)]
pub(super) struct PluginConfigSelectionOverlay {
    title: String,
    prompt: String,
    footer: String,
    multi: bool,
    items: Vec<PluginConfigSelectionItem>,
    selected_item: usize,
    action: PluginConfigSelectionAction,
}

#[derive(Debug, Clone)]
pub(super) struct PluginConfigSelectionItem {
    label: String,
    description: Option<String>,
    checked: bool,
    value: PluginConfigSelectionValue,
}

#[derive(Debug, Clone)]
pub(super) enum PluginConfigSelectionValue {
    Named(String),
    Branch(BranchChoice),
    Json(JsonValue),
}

#[derive(Debug, Clone)]
pub(super) enum PluginConfigSelectionAction {
    SelectType { plugin_id: String, path: ConfigPath },
    SelectBranch { plugin_id: String, path: ConfigPath },
    SelectEnum { plugin_id: String, path: ConfigPath },
    SelectMultiEnum { plugin_id: String, path: ConfigPath },
}

#[derive(Debug, Clone)]
pub(super) enum PluginConfigAction {
    SelectType {
        plugin_id: String,
        path: ConfigPath,
    },
    AppendArrayItem {
        plugin_id: String,
        path: ConfigPath,
    },
    PromptAddObjectField {
        plugin_id: String,
        path: ConfigPath,
    },
    InsertArrayItemBefore {
        plugin_id: String,
        path: ConfigPath,
    },
    InsertArrayItemAfter {
        plugin_id: String,
        path: ConfigPath,
    },
    DuplicateArrayItem {
        plugin_id: String,
        path: ConfigPath,
    },
    MoveArrayItem {
        plugin_id: String,
        path: ConfigPath,
        direction: isize,
    },
    RemoveArrayItem {
        plugin_id: String,
        path: ConfigPath,
    },
    RenameField {
        plugin_id: String,
        path: ConfigPath,
    },
    ResetField {
        plugin_id: String,
        paths: Vec<ConfigPath>,
        focus_path: ConfigPath,
    },
    ResetGroup {
        plugin_id: String,
        paths: Vec<ConfigPath>,
        focus_path: ConfigPath,
    },
}

#[derive(Debug, Clone)]
pub(super) enum PluginConfigEditAction {
    SetScalar {
        plugin_id: String,
        path: ConfigPath,
        kind: ScalarEditKind,
    },
    SetNullableString {
        plugin_id: String,
        path: ConfigPath,
    },
    SetPairIntegers {
        plugin_id: String,
        left_path: ConfigPath,
        right_path: ConfigPath,
    },
    AddObjectField {
        plugin_id: String,
        path: ConfigPath,
    },
    RenameObjectField {
        plugin_id: String,
        path: ConfigPath,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ScalarEditKind {
    String,
    Number,
    Integer,
}

#[derive(Debug, Clone)]
pub(super) struct BranchChoice {
    id: String,
    label: String,
    schema: JsonValue,
}

impl PluginWorkbenchOverlay {
    fn selected_plugin(&self) -> Option<&PluginWorkbenchPlugin> {
        let visible_index = *self.visible_plugins.get(self.selected_plugin)?;
        self.plugins.get(visible_index)
    }

    fn selected_plugin_mut(&mut self) -> Option<&mut PluginWorkbenchPlugin> {
        let visible_index = *self.visible_plugins.get(self.selected_plugin)?;
        self.plugins.get_mut(visible_index)
    }

    fn selected_section(&self) -> Option<&ConfigSectionView> {
        self.selected_plugin()
            .and_then(|plugin| plugin.sections.get(self.selected_section))
    }

    fn current_drilldown(&self) -> Option<&PluginConfigDrilldownOverlay> {
        self.drilldown_stack.last()
    }

    fn current_drilldown_mut(&mut self) -> Option<&mut PluginConfigDrilldownOverlay> {
        self.drilldown_stack.last_mut()
    }

    fn selected_row(&self) -> Option<&ConfigRowView> {
        let section = self.selected_section()?;
        section_row_at(section, self.config_view, self.selected_node)
    }

    fn clamp_selection(&mut self) {
        if self.visible_plugins.is_empty() {
            self.selected_plugin = 0;
        } else {
            self.selected_plugin = self
                .selected_plugin
                .min(self.visible_plugins.len().saturating_sub(1));
        }
        let section_count = self
            .selected_plugin()
            .map(|plugin| plugin.sections.len())
            .unwrap_or_default();
        if section_count == 0 {
            self.selected_section = 0;
        } else {
            self.selected_section = self.selected_section.min(section_count.saturating_sub(1));
        }
        let row_count = self
            .selected_section()
            .map(|section| section_row_count(section, self.config_view))
            .unwrap_or_default();
        if row_count == 0 {
            self.selected_node = 0;
        } else {
            self.selected_node = self.selected_node.min(row_count.saturating_sub(1));
        }
        self.selected_cell = self
            .selected_section()
            .map(|section| {
                section_selected_row_cell(
                    section,
                    self.config_view,
                    self.selected_node,
                    self.selected_cell,
                )
            })
            .unwrap_or(ConfigRowCell::Value);
        let diagnostic_count = self
            .selected_plugin()
            .map(plugin_all_diagnostics)
            .map(|diagnostics| diagnostics.len())
            .unwrap_or_default();
        if diagnostic_count == 0 {
            self.selected_diagnostic = 0;
        } else {
            self.selected_diagnostic = self.selected_diagnostic.min(diagnostic_count - 1);
        }
        let diff_count = self
            .selected_plugin()
            .map(|plugin| plugin.diff.len())
            .unwrap_or_default();
        if diff_count == 0 {
            self.selected_diff_row = 0;
        } else {
            self.selected_diff_row = self.selected_diff_row.min(diff_count - 1);
        }
        if let Some(actions) = self.actions.as_mut() {
            if actions.actions.is_empty() {
                actions.selected_action = 0;
            } else {
                actions.selected_action = actions
                    .selected_action
                    .min(actions.actions.len().saturating_sub(1));
            }
        }
        if let Some(selection) = self.selection.as_mut() {
            if selection.items.is_empty() {
                selection.selected_item = 0;
            } else {
                selection.selected_item = selection
                    .selected_item
                    .min(selection.items.len().saturating_sub(1));
            }
        }
        if self
            .selected_plugin()
            .is_some_and(plugin_uses_compact_config_layout)
            && !matches!(
                self.config_focus,
                PluginConfigFocus::Toolbar
                    | PluginConfigFocus::Structure
                    | PluginConfigFocus::Editor
            )
        {
            self.config_focus = PluginConfigFocus::Structure;
        }
        self.selected_toolbar_action = self
            .selected_toolbar_action
            .min(COMPACT_TOOLBAR_ACTIONS.len().saturating_sub(1));
        for overlay in &mut self.drilldown_stack {
            overlay.selected_cell =
                drilldown_selected_row_cell(overlay, self.config_view, overlay.selected_cell);
        }
    }
}

impl PluginPolicyStudioOverlay {
    fn selected_section(&self) -> Option<&PluginPolicySection> {
        self.state.selected_section()
    }

    fn selected_item(&self) -> Option<&PluginPolicyItem> {
        self.state.selected_item()
    }
}
use crate::app::{
    App, PLUGIN_TOOL_PRESENTATION_PATH, PLUGIN_UI_PRESENTATION_DEFAULT_MODE_PATH,
    PLUGIN_UI_PRESENTATION_PATH, Route, UiResult, editor_save_footer,
};
