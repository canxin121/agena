use std::collections::{BTreeMap, BTreeSet};

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

use agena_tui::plugin_workbench::{
    PluginConfigFilterValue, PluginDetailTab, PluginWorkbenchListItem,
    PluginWorkbenchListPresentation, PluginWorkbenchMode, PluginWorkbenchNavigation,
};

use agena_tui_components::{
    Editor, EditorDialogKeyResult, EditorDialogSpec, EditorDialogState, FramedSurfaceSpec,
    SurfaceMode, drive_editor_dialog_key, render_editor_dialog, render_framed_surface,
};

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
pub(crate) struct PluginWorkbenchOverlay {
    pub(super) title: String,
    pub(super) list: PluginWorkbenchListPresentation,
    pub(super) navigation: PluginWorkbenchNavigation,
    plugins: Vec<PluginWorkbenchPlugin>,
    config_view: PluginConfigView,
    config_focus: PluginConfigFocus,
    selected_section: usize,
    selected_node: usize,
    selected_cell: ConfigRowCell,
    selected_diagnostic: usize,
    selected_diff_row: usize,
    config_scroll: usize,
    diagnostics_scroll: usize,
    pub(super) show_diff: bool,
    pub(super) drilldown_stack: Vec<PluginConfigDrilldownOverlay>,
    pub(super) actions: Option<PluginConfigActionOverlay>,
    pub(super) selection: Option<PluginConfigSelectionOverlay>,
    pub(super) editor: Option<PluginConfigEditOverlay>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PluginConfigView {
    Effective,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PluginTextDisplayMode {
    Detailed,
    Summary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PluginTextDisplaySource {
    ToolPolicy,
    PluginPolicy,
    ToolManifest,
    PluginManifest,
    GlobalPolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PluginConfigFocus {
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
    tools: Vec<agena_plugin_host::ToolDefinition>,
    commands: Vec<agena_plugin_host::PluginCommandDefinition>,
    config_status: PluginConfigStatus,
    status: agena_plugin_host::status::PluginStatus,
    inspect: Option<agena_plugin_host::PluginInspect>,
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
    logs: Vec<agena_plugin_host::PluginLogRecord>,
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

#[derive(Debug, Clone)]
pub(super) enum ConfigRowEditor {
    Bool {
        path: ConfigPath,
    },
    Null,
    ReadOnly,
    Scalar,
    NullableString {
        path: ConfigPath,
    },
    Enum,
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
    pub(super) presentation: agena_tui::plugin_workbench::PluginConfigPickerPresentation,
    pub(super) actions: BTreeMap<String, PluginConfigAction>,
}

#[derive(Debug, Clone)]
pub(super) struct PluginConfigActionCandidate {
    label: String,
    description: String,
    action: PluginConfigAction,
}

#[derive(Debug, Clone)]
pub(super) struct PluginConfigSelectionOverlay {
    pub(super) presentation: agena_tui::plugin_workbench::PluginConfigPickerPresentation,
    pub(super) action: PluginConfigSelectionAction,
    pub(super) values: BTreeMap<String, PluginConfigSelectionValue>,
}

#[derive(Debug, Clone)]
pub(super) struct PluginConfigSelectionCandidate {
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
    Type { plugin_id: String, path: ConfigPath },
    Branch { plugin_id: String, path: ConfigPath },
    Enum { plugin_id: String, path: ConfigPath },
    MultiEnum { plugin_id: String, path: ConfigPath },
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
        let key = self.list.selected_key()?;
        self.plugins.iter().find(|plugin| plugin.plugin_id == key)
    }

    fn selected_plugin_mut(&mut self) -> Option<&mut PluginWorkbenchPlugin> {
        let key = self.list.selected_key()?.to_owned();
        self.plugins
            .iter_mut()
            .find(|plugin| plugin.plugin_id == key)
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
            actions.presentation.clamp_selection();
        }
        if let Some(selection) = self.selection.as_mut() {
            selection.presentation.clamp_selection();
        }
        if self
            .selected_plugin()
            .is_some_and(plugin_uses_compact_config_layout)
            && !matches!(
                self.config_focus,
                PluginConfigFocus::Structure | PluginConfigFocus::Editor
            )
        {
            self.config_focus = PluginConfigFocus::Structure;
        }
        for overlay in &mut self.drilldown_stack {
            overlay.selected_cell =
                drilldown_selected_row_cell(overlay, self.config_view, overlay.selected_cell);
        }
    }
}

pub(crate) fn plugin_workbench_list_items(
    plugins: &[PluginWorkbenchPlugin],
) -> Vec<PluginWorkbenchListItem> {
    plugins
        .iter()
        .map(|plugin| PluginWorkbenchListItem {
            key: plugin.plugin_id.clone(),
            search_text: vec![
                plugin.plugin_id.clone(),
                plugin.visible_tool.clone(),
                plugin.transport.clone(),
                plugin.config_status.label.clone(),
            ],
            transport: plugin.transport.clone(),
            config_filter_value: match plugin.config_status.kind {
                PluginConfigStatusKind::Valid => PluginConfigFilterValue::Valid,
                PluginConfigStatusKind::Missing => PluginConfigFilterValue::Missing,
                PluginConfigStatusKind::SchemaMissing => PluginConfigFilterValue::SchemaMissing,
                PluginConfigStatusKind::Invalid | PluginConfigStatusKind::Warning => {
                    PluginConfigFilterValue::Issues
                }
                PluginConfigStatusKind::NeedsRestart => PluginConfigFilterValue::NeedsRestart,
                PluginConfigStatusKind::RuntimeIssue => PluginConfigFilterValue::RuntimeIssue,
            },
        })
        .collect()
}

use crate::{App, UiResult, editor_save_footer};
