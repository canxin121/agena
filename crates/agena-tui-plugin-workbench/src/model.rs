use std::collections::{BTreeMap, BTreeSet};

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Paragraph, Wrap},
};
use regex::Regex;
use serde_json::{Map as JsonMap, Number as JsonNumber, Value as JsonValue, json};

use crate::{
    PluginConfigFilterValue, PluginDetailTab, PluginWorkbenchListItem,
    PluginWorkbenchListPresentation, PluginWorkbenchMode, PluginWorkbenchNavigation,
};

use agena_tui_components::{
    EditorDialogSpec, EditorDialogState, FramedSurfaceSpec, SurfaceMode, render_editor_dialog,
    render_framed_surface,
};

pub type UiResult<T> = std::result::Result<T, String>;

mod workbench_config_actions;
mod workbench_config_sections;
mod workbench_config_state;
mod workbench_display;
mod workbench_policy_builder;
mod workbench_render_helpers;
mod workbench_schema_resolution;
mod workbench_schema_util;
mod workbench_schema_validation;
mod workbench_text_render;

pub(crate) use self::workbench_config_actions::*;
pub(crate) use self::workbench_config_sections::*;
pub(crate) use self::workbench_config_state::*;
pub(crate) use self::workbench_display::*;
pub(crate) use self::workbench_schema_resolution::*;
pub(crate) use self::workbench_schema_util::*;
pub(crate) use self::workbench_schema_validation::*;
pub(crate) use self::workbench_text_render::*;
/// Public presentation/model operations consumed by the application adapter.
///
/// The implementation modules remain private; this namespace exposes only the
/// cross-crate operations needed to connect the neutral workbench model to the
/// application shell.
pub mod api {
    pub use super::workbench_config_actions::ResetPathsOutcome;
    pub use super::workbench_config_actions::{
        append_default_array_item_at_path, apply_reset_paths, apply_staged_config_value_updates,
        array_item_action_info, can_append_array_item, clear_branch_drafts_for_structural_change,
        config_actions_overlay_footer, duplicate_array_item_at_path, field_prompt_for_path,
        field_prompt_for_row, insert_default_array_item_at_path, move_array_item_at_path,
        object_add_field_block_reason, parse_pair_integer_editor_values, parse_scalar_editor_value,
        path_display, preview_value, prioritize_config_actions, remove_array_item_at_path,
        rename_object_field_at_path, reset_paths_warning_message, schema_string_is_multiline,
        title_for_schema_or_key, title_from_key, validate_new_object_field_key,
        validate_schema_value_for_path,
    };
    pub use super::workbench_config_sections::api::config_path;
    pub use super::workbench_config_state::SelectedConfigRowContext;
    pub use super::workbench_config_state::{
        build_drilldown_groups, config_row_primary_action, drilldown_group_for_row,
        drilldown_row_at, drilldown_row_count, drilldown_selected_row_cell,
        find_best_drilldown_row_for_path, find_best_section_row_for_path, find_row_position,
        move_config_row_cell, move_selected_bottom_panel_row, move_selected_config_section,
        normalize_config_row_cell, persisted_plugin_config_value, plugin_save_block_reason,
        rebuild_drilldown_stack, recompute_plugin_config_state, row_paths,
        row_rename_action_allowed, section_group_for_row, section_row_at, select_config_path,
        selected_config_row_context,
    };
    pub use super::workbench_display::plugin_uses_compact_config_layout;
    pub use super::workbench_policy_builder::build_plugin_workbench_plugin;
    pub use super::workbench_render_helpers::{
        render_plugin_detail_page, render_plugin_list_page, render_plugin_workbench_editor_overlay,
    };
    pub use super::workbench_schema_resolution::{ArrayItemActionInfo, ConfigRowPrimaryAction};
    pub use super::workbench_schema_resolution::{
        active_branch_id, branch_choices, declared_schema_for_path, default_value_for_schema,
        default_value_for_type, get_value_at_path, path_key_info, plugin_branch_draft_key,
        schema_for_path, schema_kind_label, schema_type_selector_choices, set_value_at_path,
        value_matches_type,
    };
    pub use super::workbench_schema_util::{
        clean, move_detail_scroll, move_index, move_selected_config_node,
        plugin_config_record_value, plugin_workbench_summary, quote_settings_segment,
    };
    pub use super::workbench_schema_validation::api::{
        merge_multi_enum_selection, next_config_focus, previous_config_focus, schema_enum_values,
    };
    pub use super::workbench_text_render::api::{pair_editor_labels, plugin_all_diagnostics};
}

pub const PLUGIN_WORKBENCH_LOG_LIMIT: usize = 80;

#[derive(Debug, Clone)]
/// An overlay of the plugin workbench.
pub struct PluginWorkbenchOverlay {
    pub title: String,
    pub list: PluginWorkbenchListPresentation,
    pub navigation: PluginWorkbenchNavigation,
    pub plugins: Vec<PluginWorkbenchPlugin>,
    pub config_view: PluginConfigView,
    pub config_focus: PluginConfigFocus,
    pub selected_section: usize,
    pub selected_node: usize,
    pub selected_cell: ConfigRowCell,
    pub selected_diagnostic: usize,
    pub selected_diff_row: usize,
    pub selected_tool: usize,
    pub config_scroll: usize,
    pub diagnostics_scroll: usize,
    pub show_diff: bool,
    pub drilldown_stack: Vec<PluginConfigDrilldownOverlay>,
    pub actions: Option<PluginConfigActionOverlay>,
    pub selection: Option<PluginConfigSelectionOverlay>,
    pub editor: Option<PluginConfigEditOverlay>,
    pub tool_editor: Option<PluginToolInvocationOverlay>,
    pub tool_result: Option<PluginToolInvocationResult>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// View of a plugin config.
pub enum PluginConfigView {
    Effective,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Focus of a plugin config.
pub enum PluginConfigFocus {
    Structure,
    Editor,
    Diagnostics,
}

#[derive(Debug, Clone)]
/// A plugin shown in the workbench.
pub struct PluginWorkbenchPlugin {
    pub plugin_id: String,
    pub visible_tool: String,
    pub version: String,
    pub transport: String,
    pub tools: Vec<agena_plugin_host::ToolDefinition>,
    pub operations: Vec<agena_plugin_host::PluginOperationDefinition>,
    pub config_status: PluginConfigStatus,
    pub status: agena_plugin_host::status::PluginStatus,
    pub inspect: Option<agena_plugin_host::PluginInspect>,
    pub configured_plugin_value: Option<JsonValue>,
    pub saved_override: JsonValue,
    pub draft_override: JsonValue,
    pub default_config: JsonValue,
    pub saved_config: JsonValue,
    pub draft_config: JsonValue,
    pub schema: Option<JsonValue>,
    pub schema_missing: bool,
    pub diagnostics: Vec<ConfigDiagnostic>,
    pub runtime_diagnostics: Vec<ConfigDiagnostic>,
    pub diff: Vec<ConfigDiffRow>,
    pub sections: Vec<ConfigSectionView>,
    pub logs: Vec<agena_plugin_host::PluginLogRecord>,
    pub dirty: bool,
    pub branch_drafts: BTreeMap<String, JsonValue>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
/// Status kind of a plugin config.
pub enum PluginConfigStatusKind {
    Valid,
    Missing,
    SchemaMissing,
    Invalid,
    Warning,
    NeedsRestart,
    RuntimeIssue,
}

#[derive(Debug, Clone)]
/// Status of a plugin config.
pub struct PluginConfigStatus {
    pub kind: PluginConfigStatusKind,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
/// A path segment in a config path.
pub enum PathSegment {
    Key(String),
    Index(usize),
}

pub type ConfigPath = Vec<PathSegment>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Severity of a config diagnostic.
pub enum DiagnosticSeverity {
    Error,
    Warning,
}

#[derive(Debug, Clone)]
/// A config diagnostic.
pub struct ConfigDiagnostic {
    pub severity: DiagnosticSeverity,
    pub source: String,
    pub path: ConfigPath,
    pub field: String,
    pub message: String,
}

#[derive(Debug, Clone)]
/// A diff row of a config section.
pub struct ConfigDiffRow {
    pub path: ConfigPath,
    pub before: String,
    pub after: String,
    pub summary: String,
}

#[derive(Debug, Clone)]
/// View of a config section.
pub struct ConfigSectionView {
    pub key: String,
    pub title: String,
    pub issue_count: usize,
    pub dirty: bool,
    pub body: ConfigSectionBody,
}

#[derive(Debug, Clone)]
/// Body of a config section view.
pub enum ConfigSectionBody {
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
/// An overview card of a config group.
pub struct ConfigOverviewCard {
    pub title: String,
    pub summary: String,
    pub issue_label: Option<String>,
}

#[derive(Debug, Clone)]
/// View of a config group.
pub struct ConfigGroupView {
    pub title: String,
    pub layout: ConfigGroupLayout,
    pub rows: Vec<ConfigRowView>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Layout of a config group.
pub enum ConfigGroupLayout {
    Standard,
    Pair {
        left_label: &'static str,
        right_label: &'static str,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// A cell of a config row.
pub enum ConfigRowCell {
    Type,
    Value,
    SecondaryValue,
    Default,
    Action,
    State,
}

#[derive(Debug, Clone)]
/// View of a config row.
pub struct ConfigRowView {
    pub title: String,
    pub primary_path: ConfigPath,
    pub additional_paths: Vec<ConfigPath>,
    pub editor: ConfigRowEditor,
    pub description: Option<String>,
    pub constraints: Vec<String>,
    pub type_display: String,
    pub type_mode: ConfigRowTypeMode,
    pub value_display: String,
    pub default_display: String,
    pub secondary_value_display: Option<String>,
    pub action_display: Option<String>,
    pub state: ConfigRowState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Type mode of a config row.
pub enum ConfigRowTypeMode {
    Fixed,
    SelectType,
    SelectShape,
}

impl ConfigRowTypeMode {
    pub fn is_switchable(self) -> bool {
        !matches!(self, Self::Fixed)
    }

    pub fn action_label(self) -> &'static str {
        match self {
            Self::Fixed => "Type",
            Self::SelectType => "Choose Type",
            Self::SelectShape => "Choose Shape",
        }
    }
}

#[derive(Debug, Clone)]
/// Editor of a config row.
pub enum ConfigRowEditor {
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
/// State of a config row.
pub enum ConfigRowState {
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
/// Action of the compact toolbar.
pub enum CompactToolbarAction {
    Validate,
    ResetAll,
    Diff,
    Save,
    Restart,
}

pub type PluginConfigEditOverlay = EditorDialogState<PluginConfigEditAction>;

#[derive(Debug, Clone)]
/// Drilldown overlay of a plugin config.
pub struct PluginConfigDrilldownOverlay {
    pub plugin_id: String,
    pub path: ConfigPath,
    pub title: String,
    pub groups: Vec<ConfigGroupView>,
    pub selected_row: usize,
    pub selected_cell: ConfigRowCell,
}

#[derive(Debug, Clone)]
/// Action overlay of a plugin config.
pub struct PluginConfigActionOverlay {
    pub presentation: crate::PluginConfigPickerPresentation,
    pub actions: BTreeMap<String, PluginConfigAction>,
}

#[derive(Debug, Clone)]
/// A candidate action of a plugin config.
pub struct PluginConfigActionCandidate {
    pub label: String,
    pub description: String,
    pub action: PluginConfigAction,
}

#[derive(Debug, Clone)]
/// Selection overlay of a plugin config.
pub struct PluginConfigSelectionOverlay {
    pub presentation: crate::PluginConfigPickerPresentation,
    pub action: PluginConfigSelectionAction,
    pub values: BTreeMap<String, PluginConfigSelectionValue>,
}

#[derive(Debug, Clone)]
/// A candidate of a plugin config selection.
pub struct PluginConfigSelectionCandidate {
    pub label: String,
    pub description: Option<String>,
    pub checked: bool,
    pub value: PluginConfigSelectionValue,
}

#[derive(Debug, Clone)]
/// Value of a plugin config selection.
pub enum PluginConfigSelectionValue {
    Named(String),
    Branch(BranchChoice),
    Json(JsonValue),
}

#[derive(Debug, Clone)]
/// Action of a plugin config selection.
pub enum PluginConfigSelectionAction {
    Type { plugin_id: String, path: ConfigPath },
    Branch { plugin_id: String, path: ConfigPath },
    Enum { plugin_id: String, path: ConfigPath },
    MultiEnum { plugin_id: String, path: ConfigPath },
}

#[derive(Debug, Clone)]
/// Action on a plugin config.
pub enum PluginConfigAction {
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
/// Edit action on a plugin config.
pub enum PluginConfigEditAction {
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

#[derive(Debug, Clone, PartialEq, Eq)]
/// Tool invocation action of the workbench.
pub struct PluginToolInvocationAction {
    pub plugin_id: String,
    pub tool_name: String,
}

pub type PluginToolInvocationOverlay = EditorDialogState<PluginToolInvocationAction>;

#[derive(Debug, Clone, PartialEq, Eq)]
/// Result of a workbench tool invocation.
pub struct PluginToolInvocationResult {
    pub plugin_id: String,
    pub tool_name: String,
    pub output: String,
    pub succeeded: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Kind of a scalar edit.
pub enum ScalarEditKind {
    String,
    Number,
    Integer,
}

#[derive(Debug, Clone)]
/// A branch choice of a config edit.
pub struct BranchChoice {
    pub id: String,
    pub label: String,
    pub schema: JsonValue,
}

impl PluginWorkbenchOverlay {
    pub fn open_plugin_detail(&mut self, plugin_id: &str, tab: PluginDetailTab) -> bool {
        self.list.select_key(plugin_id);
        if self.list.selected_key() != Some(plugin_id) {
            return false;
        }
        self.navigation.mode = PluginWorkbenchMode::Detail;
        self.navigation.detail_tab = tab;
        self.selected_section = 0;
        self.selected_node = 0;
        self.selected_cell = ConfigRowCell::Value;
        self.selected_diagnostic = 0;
        self.selected_diff_row = 0;
        self.selected_tool = 0;
        self.config_scroll = 0;
        self.diagnostics_scroll = 0;
        self.show_diff = false;
        self.drilldown_stack.clear();
        self.actions = None;
        self.selection = None;
        self.editor = None;
        self.tool_editor = None;
        self.tool_result = None;
        self.clamp_selection();
        true
    }

    pub fn selected_plugin(&self) -> Option<&PluginWorkbenchPlugin> {
        let key = self.list.selected_key()?;
        self.plugins.iter().find(|plugin| plugin.plugin_id == key)
    }

    pub fn selected_plugin_mut(&mut self) -> Option<&mut PluginWorkbenchPlugin> {
        let key = self.list.selected_key()?.to_owned();
        self.plugins
            .iter_mut()
            .find(|plugin| plugin.plugin_id == key)
    }

    pub fn selected_tool(&self) -> Option<&agena_plugin_host::ToolDefinition> {
        self.selected_plugin()
            .and_then(|plugin| plugin.tools.get(self.selected_tool))
    }

    pub fn selected_section(&self) -> Option<&ConfigSectionView> {
        self.selected_plugin()
            .and_then(|plugin| plugin.sections.get(self.selected_section))
    }

    pub fn current_drilldown(&self) -> Option<&PluginConfigDrilldownOverlay> {
        self.drilldown_stack.last()
    }

    pub fn current_drilldown_mut(&mut self) -> Option<&mut PluginConfigDrilldownOverlay> {
        self.drilldown_stack.last_mut()
    }

    pub fn selected_row(&self) -> Option<&ConfigRowView> {
        let section = self.selected_section()?;
        section_row_at(section, self.config_view, self.selected_node)
    }

    pub fn clamp_selection(&mut self) {
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
        let tool_count = self
            .selected_plugin()
            .map(|plugin| plugin.tools.len())
            .unwrap_or_default();
        if tool_count == 0 {
            self.selected_tool = 0;
        } else {
            self.selected_tool = self.selected_tool.min(tool_count - 1);
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

pub fn plugin_workbench_list_items(
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

#[cfg(test)]
mod tests {
    use super::workbench_policy_builder::build_plugin_workbench_plugin;
    use super::*;

    fn workbench_with_plugin(plugin_id: &str) -> PluginWorkbenchOverlay {
        let key = plugin_id.parse().expect("plugin key");
        let sources = agena_application::dto::ConfigJsonSources {
            config_path: std::path::PathBuf::new(),
            config_found: false,
            project_config_path: std::path::PathBuf::new(),
            project_config_found: false,
            applied_layers: Vec::new(),
            file: json!({}),
            project_file: json!({}),
            effective: json!({}),
        };
        let plugin = build_plugin_workbench_plugin(
            &sources,
            "en-US",
            agena_plugin_host::status::PluginStatus::initial(&key, "static"),
            None,
            Vec::new(),
        );
        let plugins = vec![plugin];
        PluginWorkbenchOverlay {
            title: "Plugins".to_owned(),
            list: PluginWorkbenchListPresentation::new(plugin_workbench_list_items(&plugins), ""),
            navigation: PluginWorkbenchNavigation::new(),
            plugins,
            config_view: PluginConfigView::Effective,
            config_focus: PluginConfigFocus::Structure,
            selected_section: 0,
            selected_node: 0,
            selected_cell: ConfigRowCell::Value,
            selected_diagnostic: 0,
            selected_diff_row: 0,
            selected_tool: 0,
            config_scroll: 0,
            diagnostics_scroll: 0,
            show_diff: false,
            drilldown_stack: Vec::new(),
            actions: None,
            selection: None,
            editor: None,
            tool_editor: None,
            tool_result: None,
        }
    }

    #[test]
    fn opening_plugin_detail_selects_the_plugin_and_standard_tab() {
        let mut workbench = workbench_with_plugin("agena.memory");

        assert!(workbench.open_plugin_detail("agena.memory", PluginDetailTab::Tools));
        assert_eq!(workbench.navigation.mode, PluginWorkbenchMode::Detail);
        assert_eq!(workbench.navigation.detail_tab, PluginDetailTab::Tools);
        assert_eq!(workbench.list.selected_key(), Some("agena.memory"));
    }

    #[test]
    fn opening_an_unavailable_plugin_leaves_the_workbench_closed() {
        let mut workbench = workbench_with_plugin("agena.memory");

        assert!(!workbench.open_plugin_detail("agena.snapshot", PluginDetailTab::Tools));
        assert_eq!(workbench.navigation.mode, PluginWorkbenchMode::List);
    }
}
