use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Paragraph, Wrap},
};
use regex::Regex;
use serde_json::{Map as JsonMap, Number as JsonNumber, Value as JsonValue, json};
use unicode_width::UnicodeWidthStr;

use agena_tui_components::{
    Editor, EditorDialogKeyResult, EditorDialogSpec, EditorDialogState, FramedSurfaceSpec,
    SurfaceMode, drive_editor_dialog_key, render_editor_dialog, render_framed_surface,
};

use super::*;

const PLUGIN_WORKBENCH_LOG_LIMIT: usize = 80;
const CONFIG_EDITOR_PAGE_SIZE: usize = 10;

#[derive(Debug, Clone)]
pub(super) struct PluginWorkbenchOverlay {
    title: String,
    query: Editor,
    mode: PluginWorkbenchMode,
    transport_filter: PluginTransportFilter,
    config_filter: PluginConfigFilter,
    plugins: Vec<PluginWorkbenchPlugin>,
    visible_plugins: Vec<usize>,
    selected_plugin: usize,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PluginWorkbenchMode {
    List,
    Detail,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PluginTransportFilter {
    All,
    Static,
    Stdio,
    Cdylib,
    Http,
    Wasm,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PluginConfigFilter {
    All,
    Valid,
    Missing,
    SchemaMissing,
    Issues,
    NeedsRestart,
    RuntimeIssue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PluginDetailTab {
    Config,
    Tools,
    Commands,
    Capabilities,
    Logs,
    Diagnostics,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PluginConfigView {
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
enum PluginConfigFocus {
    Toolbar,
    Structure,
    Editor,
    Diagnostics,
}

#[derive(Debug, Clone)]
struct PluginWorkbenchPlugin {
    plugin_id: String,
    visible_tool: String,
    version: String,
    transport: String,
    tools: Vec<agena::plugin::PluginToolDecl>,
    commands: Vec<agena::plugin::PluginStudioCommand>,
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
enum PluginConfigStatusKind {
    Valid,
    Missing,
    SchemaMissing,
    Invalid,
    Warning,
    NeedsRestart,
    RuntimeIssue,
}

#[derive(Debug, Clone)]
struct PluginConfigStatus {
    kind: PluginConfigStatusKind,
    label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum PathSegment {
    Key(String),
    Index(usize),
}

type ConfigPath = Vec<PathSegment>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DiagnosticSeverity {
    Error,
    Warning,
}

#[derive(Debug, Clone)]
struct ConfigDiagnostic {
    severity: DiagnosticSeverity,
    source: String,
    path: ConfigPath,
    field: String,
    message: String,
}

#[derive(Debug, Clone)]
struct ConfigDiffRow {
    path: ConfigPath,
    before: String,
    after: String,
    summary: String,
}

#[derive(Debug, Clone)]
struct ConfigSectionView {
    key: String,
    title: String,
    issue_count: usize,
    dirty: bool,
    body: ConfigSectionBody,
}

#[derive(Debug, Clone)]
enum ConfigSectionBody {
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
struct ConfigOverviewCard {
    title: String,
    summary: String,
    issue_label: Option<String>,
}

#[derive(Debug, Clone)]
struct ConfigGroupView {
    title: String,
    layout: ConfigGroupLayout,
    rows: Vec<ConfigRowView>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConfigGroupLayout {
    Standard,
    Pair {
        left_label: &'static str,
        right_label: &'static str,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConfigRowCell {
    Type,
    Value,
    SecondaryValue,
    Default,
    Action,
    State,
}

#[derive(Debug, Clone)]
struct ConfigRowView {
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
enum ConfigRowTypeMode {
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
enum ConfigRowEditor {
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
enum ConfigRowState {
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
enum CompactToolbarAction {
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

    fn shortcut(self) -> &'static str {
        match self {
            Self::Validate => "V",
            Self::ResetAll => "R",
            Self::Diff => "D",
            Self::Save => "S",
            Self::Restart => "^R",
        }
    }
}

type PluginConfigEditOverlay = EditorDialogState<PluginConfigEditAction>;

#[derive(Debug, Clone)]
struct PluginConfigDrilldownOverlay {
    plugin_id: String,
    path: ConfigPath,
    title: String,
    groups: Vec<ConfigGroupView>,
    selected_row: usize,
    selected_cell: ConfigRowCell,
}

#[derive(Debug, Clone)]
struct PluginConfigActionOverlay {
    title: String,
    subject: String,
    footer: String,
    actions: Vec<PluginConfigActionItem>,
    selected_action: usize,
}

#[derive(Debug, Clone)]
struct PluginConfigActionItem {
    label: String,
    description: String,
    action: PluginConfigAction,
}

#[derive(Debug, Clone)]
struct PluginConfigSelectionOverlay {
    title: String,
    prompt: String,
    footer: String,
    multi: bool,
    items: Vec<PluginConfigSelectionItem>,
    selected_item: usize,
    action: PluginConfigSelectionAction,
}

#[derive(Debug, Clone)]
struct PluginConfigSelectionItem {
    label: String,
    description: Option<String>,
    checked: bool,
    value: PluginConfigSelectionValue,
}

#[derive(Debug, Clone)]
enum PluginConfigSelectionValue {
    Named(String),
    Branch(BranchChoice),
    Json(JsonValue),
}

#[derive(Debug, Clone)]
enum PluginConfigSelectionAction {
    SelectType { plugin_id: String, path: ConfigPath },
    SelectBranch { plugin_id: String, path: ConfigPath },
    SelectEnum { plugin_id: String, path: ConfigPath },
    SelectMultiEnum { plugin_id: String, path: ConfigPath },
}

#[derive(Debug, Clone)]
enum PluginConfigAction {
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
enum PluginConfigEditAction {
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
enum ScalarEditKind {
    String,
    Number,
    Integer,
}

#[derive(Debug, Clone)]
struct BranchChoice {
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

impl App {
    pub(super) fn open_plugin_workbench(&mut self, query: &str) {
        match self.build_plugin_workbench(query) {
            Ok(dialog) => {
                self.route_stack.clear();
                self.current_route = Route::PluginWorkbench(Box::new(dialog));
            }
            Err(error) => self.flash_error(error),
        }
    }

    pub(super) fn build_plugin_workbench(&self, query: &str) -> UiResult<PluginWorkbenchOverlay> {
        let sources = self
            .backend
            .config_json_sources()
            .map_err(|error| error.to_string())?;
        let locale = self.i18n.locale_tag();
        let statuses = self.backend.plugin_statuses();
        let mut plugins = statuses
            .into_iter()
            .map(|status| {
                let plugin_id = status.plugin_id.clone();
                let inspect = self.backend.plugin_inspect(plugin_id.as_str());
                let logs =
                    self.backend
                        .plugin_logs(plugin_id.as_str(), None, PLUGIN_WORKBENCH_LOG_LIMIT);
                build_plugin_workbench_plugin(&sources, locale.as_str(), status, inspect, logs)
            })
            .collect::<Vec<_>>();
        plugins.sort_by(|left, right| left.plugin_id.cmp(&right.plugin_id));

        let mut dialog = PluginWorkbenchOverlay {
            title: "Plugins".to_owned(),
            query: Editor::from_text(query.to_owned()),
            mode: PluginWorkbenchMode::List,
            transport_filter: PluginTransportFilter::All,
            config_filter: PluginConfigFilter::All,
            plugins,
            visible_plugins: Vec::new(),
            selected_plugin: 0,
            detail_tab: PluginDetailTab::Config,
            config_view: PluginConfigView::Effective,
            config_focus: PluginConfigFocus::Structure,
            selected_toolbar_action: 0,
            selected_section: 0,
            selected_node: 0,
            selected_cell: ConfigRowCell::Value,
            selected_diagnostic: 0,
            selected_diff_row: 0,
            config_scroll: 0,
            diagnostics_scroll: 0,
            show_diff: false,
            drilldown_stack: Vec::new(),
            actions: None,
            selection: None,
            editor: None,
        };
        refresh_plugin_workbench_filter(&mut dialog);
        Ok(dialog)
    }

    pub(super) fn refresh_plugin_workbench(&mut self, dialog: &mut PluginWorkbenchOverlay) {
        let query = dialog.query.text().to_owned();
        let selected_plugin_id = dialog
            .selected_plugin()
            .map(|plugin| plugin.plugin_id.clone());
        let selected_section_key = dialog.selected_section().map(|section| section.key.clone());
        let selected_path = dialog.selected_row().map(|row| row.primary_path.clone());
        match self.build_plugin_workbench(query.as_str()) {
            Ok(mut refreshed) => {
                refreshed.mode = dialog.mode;
                refreshed.transport_filter = dialog.transport_filter;
                refreshed.config_filter = dialog.config_filter;
                refreshed.detail_tab = dialog.detail_tab;
                refreshed.config_view = dialog.config_view;
                refreshed.config_focus = dialog.config_focus;
                refreshed.selected_toolbar_action = dialog.selected_toolbar_action;
                refreshed.selected_cell = dialog.selected_cell;
                refreshed.show_diff = dialog.show_diff;
                refreshed.drilldown_stack =
                    rebuild_drilldown_stack(&refreshed, dialog.drilldown_stack.as_slice());
                refresh_plugin_workbench_filter(&mut refreshed);
                if let Some(plugin_id) = selected_plugin_id {
                    if let Some(index) = refreshed.visible_plugins.iter().position(|visible| {
                        refreshed
                            .plugins
                            .get(*visible)
                            .is_some_and(|plugin| plugin.plugin_id == plugin_id)
                    }) {
                        refreshed.selected_plugin = index;
                    }
                }
                if let Some(section_key) = selected_section_key {
                    refreshed.selected_section = refreshed
                        .selected_plugin()
                        .and_then(|plugin| {
                            plugin
                                .sections
                                .iter()
                                .position(|section| section.key == section_key)
                        })
                        .unwrap_or_default();
                }
                if let Some(path) = selected_path {
                    if let Some((section_index, row_index)) = refreshed
                        .selected_plugin()
                        .and_then(|plugin| find_row_position(plugin, refreshed.config_view, &path))
                    {
                        refreshed.selected_section = section_index;
                        refreshed.selected_node = row_index;
                    }
                }
                refreshed.clamp_selection();
                *dialog = refreshed;
            }
            Err(error) => self.flash_error(error),
        }
    }

    pub(super) fn refresh_restored_plugin_workbench(
        &self,
        dialog: PluginWorkbenchOverlay,
    ) -> PluginWorkbenchOverlay {
        let query = dialog.query.text().to_owned();
        let selected_plugin_id = dialog
            .selected_plugin()
            .map(|plugin| plugin.plugin_id.clone());
        let selected_section_key = dialog.selected_section().map(|section| section.key.clone());
        let selected_path = dialog.selected_row().map(|row| row.primary_path.clone());
        let Ok(mut refreshed) = self.build_plugin_workbench(query.as_str()) else {
            return dialog;
        };
        refreshed.mode = dialog.mode;
        refreshed.transport_filter = dialog.transport_filter;
        refreshed.config_filter = dialog.config_filter;
        refreshed.detail_tab = dialog.detail_tab;
        refreshed.config_view = dialog.config_view;
        refreshed.config_focus = dialog.config_focus;
        refreshed.selected_toolbar_action = dialog.selected_toolbar_action;
        refreshed.selected_cell = dialog.selected_cell;
        refreshed.show_diff = dialog.show_diff;
        refreshed.selected_diagnostic = dialog.selected_diagnostic;
        refreshed.selected_diff_row = dialog.selected_diff_row;
        refreshed.drilldown_stack =
            rebuild_drilldown_stack(&refreshed, dialog.drilldown_stack.as_slice());
        refreshed.actions = dialog.actions.clone();
        refreshed.selection = dialog.selection.clone();
        refresh_plugin_workbench_filter(&mut refreshed);
        if let Some(plugin_id) = selected_plugin_id {
            if let Some(index) = refreshed.visible_plugins.iter().position(|visible| {
                refreshed
                    .plugins
                    .get(*visible)
                    .is_some_and(|plugin| plugin.plugin_id == plugin_id)
            }) {
                refreshed.selected_plugin = index;
            }
        }
        if let Some(section_key) = selected_section_key {
            refreshed.selected_section = refreshed
                .selected_plugin()
                .and_then(|plugin| {
                    plugin
                        .sections
                        .iter()
                        .position(|section| section.key == section_key)
                })
                .unwrap_or_default();
        }
        if let Some(path) = selected_path {
            if let Some((section_index, row_index)) = refreshed
                .selected_plugin()
                .and_then(|plugin| find_row_position(plugin, refreshed.config_view, &path))
            {
                refreshed.selected_section = section_index;
                refreshed.selected_node = row_index;
            }
        }
        refreshed.clamp_selection();
        refreshed
    }

    pub(super) fn handle_plugin_workbench_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut PluginWorkbenchOverlay,
    ) -> bool {
        if dialog.actions.is_some() {
            return self.handle_plugin_config_actions_key(key, dialog);
        }
        if dialog.selection.is_some() {
            return self.handle_plugin_config_selection_key(key, dialog);
        }
        if let Some(editor) = dialog.editor.as_mut() {
            match drive_editor_dialog_key(editor, key) {
                EditorDialogKeyResult::Continue => return false,
                EditorDialogKeyResult::Close => {
                    dialog.editor = None;
                    return false;
                }
                EditorDialogKeyResult::Submit(action, input) => {
                    if let Err(error) =
                        self.commit_plugin_config_editor(dialog, action, input.as_str())
                    {
                        self.flash_error(error);
                    } else {
                        dialog.editor = None;
                    }
                    return false;
                }
            }
        }
        if dialog.current_drilldown().is_some() {
            return self.handle_plugin_config_drilldown_key(key, dialog);
        }

        match dialog.mode {
            PluginWorkbenchMode::List => self.handle_plugin_workbench_list_key(key, dialog),
            PluginWorkbenchMode::Detail => self.handle_plugin_workbench_detail_key(key, dialog),
        }
    }

    fn handle_plugin_workbench_list_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut PluginWorkbenchOverlay,
    ) -> bool {
        match key.code {
            KeyCode::Esc => true,
            KeyCode::Enter => {
                dialog.mode = PluginWorkbenchMode::Detail;
                dialog.detail_tab = PluginDetailTab::Config;
                dialog.selected_section = 0;
                dialog.selected_node = 0;
                false
            }
            KeyCode::Char('r') | KeyCode::Char('R') => {
                self.refresh_plugin_workbench(dialog);
                false
            }
            KeyCode::Char('t') => {
                dialog.transport_filter = next_transport_filter(dialog.transport_filter);
                refresh_plugin_workbench_filter(dialog);
                false
            }
            KeyCode::Char('c') => {
                dialog.config_filter = next_config_filter(dialog.config_filter);
                refresh_plugin_workbench_filter(dialog);
                false
            }
            KeyCode::PageUp => {
                move_index_page(
                    &mut dialog.selected_plugin,
                    dialog.visible_plugins.len(),
                    -1,
                    CONFIG_EDITOR_PAGE_SIZE,
                );
                false
            }
            KeyCode::PageDown => {
                move_index_page(
                    &mut dialog.selected_plugin,
                    dialog.visible_plugins.len(),
                    1,
                    CONFIG_EDITOR_PAGE_SIZE,
                );
                false
            }
            KeyCode::Home => {
                dialog.selected_plugin = 0;
                false
            }
            KeyCode::End => {
                dialog.selected_plugin = dialog.visible_plugins.len().saturating_sub(1);
                false
            }
            KeyCode::Up | KeyCode::Char('k') => {
                move_index(
                    &mut dialog.selected_plugin,
                    dialog.visible_plugins.len(),
                    -1,
                );
                false
            }
            KeyCode::Down | KeyCode::Char('j') => {
                move_index(&mut dialog.selected_plugin, dialog.visible_plugins.len(), 1);
                false
            }
            _ => {
                let before = dialog.query.text().to_owned();
                dialog.query.handle_line_input_key(key);
                dialog.query.flush_all_pending_input();
                if dialog.query.text() != before {
                    refresh_plugin_workbench_filter(dialog);
                }
                false
            }
        }
    }

    fn handle_plugin_workbench_detail_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut PluginWorkbenchOverlay,
    ) -> bool {
        if dialog.detail_tab == PluginDetailTab::Config {
            return match key.code {
                KeyCode::Esc => {
                    dialog.mode = PluginWorkbenchMode::List;
                    false
                }
                KeyCode::Left | KeyCode::Char('h')
                    if key.modifiers.contains(KeyModifiers::CONTROL) =>
                {
                    dialog.detail_tab = dialog.detail_tab.move_by(-1);
                    false
                }
                KeyCode::Right | KeyCode::Char('l')
                    if key.modifiers.contains(KeyModifiers::CONTROL) =>
                {
                    dialog.detail_tab = dialog.detail_tab.move_by(1);
                    false
                }
                _ => self.handle_plugin_config_key(key, dialog),
            };
        }

        match key.code {
            KeyCode::Esc => {
                dialog.mode = PluginWorkbenchMode::List;
                false
            }
            KeyCode::Char('r') | KeyCode::Char('R') => {
                self.refresh_plugin_workbench(dialog);
                false
            }
            KeyCode::Tab if key.modifiers.is_empty() => {
                dialog.detail_tab = dialog.detail_tab.move_by(1);
                false
            }
            KeyCode::BackTab => {
                dialog.detail_tab = dialog.detail_tab.move_by(-1);
                false
            }
            KeyCode::Down | KeyCode::Char('j') => {
                dialog.detail_tab = dialog.detail_tab.move_by(1);
                false
            }
            KeyCode::Up | KeyCode::Char('k') => {
                dialog.detail_tab = dialog.detail_tab.move_by(-1);
                false
            }
            KeyCode::PageUp if dialog.detail_tab != PluginDetailTab::Config => {
                move_detail_scroll(dialog, -10);
                false
            }
            KeyCode::PageDown if dialog.detail_tab != PluginDetailTab::Config => {
                move_detail_scroll(dialog, 10);
                false
            }
            _ => false,
        }
    }

    fn handle_plugin_config_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut PluginWorkbenchOverlay,
    ) -> bool {
        let compact_layout = dialog
            .selected_plugin()
            .is_some_and(plugin_uses_compact_config_layout);
        if compact_layout && dialog.show_diff {
            match key.code {
                KeyCode::Esc | KeyCode::Char('d') | KeyCode::Char('D') => {
                    dialog.show_diff = false;
                    return false;
                }
                _ => return false,
            }
        }
        if compact_layout {
            match key.code {
                KeyCode::Enter if dialog.config_focus == PluginConfigFocus::Toolbar => {
                    self.run_compact_toolbar_action(dialog);
                    return false;
                }
                KeyCode::Enter if dialog.config_focus == PluginConfigFocus::Structure => {
                    dialog.config_focus = PluginConfigFocus::Editor;
                    return false;
                }
                KeyCode::Right | KeyCode::Char('l')
                    if dialog.config_focus == PluginConfigFocus::Toolbar =>
                {
                    move_index(
                        &mut dialog.selected_toolbar_action,
                        COMPACT_TOOLBAR_ACTIONS.len(),
                        1,
                    );
                    return false;
                }
                KeyCode::Left | KeyCode::Char('h')
                    if dialog.config_focus == PluginConfigFocus::Toolbar =>
                {
                    move_index(
                        &mut dialog.selected_toolbar_action,
                        COMPACT_TOOLBAR_ACTIONS.len(),
                        -1,
                    );
                    return false;
                }
                KeyCode::Right | KeyCode::Char('l')
                    if dialog.config_focus == PluginConfigFocus::Structure =>
                {
                    dialog.config_focus = PluginConfigFocus::Editor;
                    return false;
                }
                KeyCode::Left | KeyCode::Char('h')
                    if dialog.config_focus == PluginConfigFocus::Structure =>
                {
                    return false;
                }
                KeyCode::Left | KeyCode::Char('h')
                    if dialog.config_focus == PluginConfigFocus::Editor =>
                {
                    if !self.move_selected_main_config_cell(dialog, -1) {
                        dialog.config_focus = PluginConfigFocus::Structure;
                    }
                    return false;
                }
                KeyCode::Right | KeyCode::Char('l')
                    if dialog.config_focus == PluginConfigFocus::Editor =>
                {
                    self.move_selected_main_config_cell(dialog, 1);
                    return false;
                }
                KeyCode::Up | KeyCode::Char('k')
                    if dialog.config_focus == PluginConfigFocus::Toolbar =>
                {
                    dialog.config_focus = PluginConfigFocus::Structure;
                    return false;
                }
                KeyCode::Down | KeyCode::Char('j')
                    if dialog.config_focus == PluginConfigFocus::Toolbar =>
                {
                    dialog.config_focus = PluginConfigFocus::Structure;
                    return false;
                }
                _ => {}
            }
        }
        match key.code {
            KeyCode::Tab if key.modifiers.is_empty() => {
                dialog.config_focus = next_config_focus(dialog.config_focus, compact_layout);
                false
            }
            KeyCode::BackTab => {
                dialog.config_focus = previous_config_focus(dialog.config_focus, compact_layout);
                false
            }
            KeyCode::Char('s') | KeyCode::Char('S') => {
                self.save_selected_plugin_config(dialog);
                false
            }
            KeyCode::Char('v') | KeyCode::Char('V') => {
                self.validate_selected_plugin_config(dialog);
                false
            }
            KeyCode::Char('i') | KeyCode::Char('I') => {
                self.insert_selected_plugin_defaults(dialog);
                false
            }
            KeyCode::Char('x') | KeyCode::Char('X') => {
                self.open_selected_config_actions(dialog);
                false
            }
            KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if dialog.config_focus == PluginConfigFocus::Diagnostics {
                    self.jump_to_selected_bottom_item(dialog);
                } else {
                    self.delete_selected_config_node(dialog);
                }
                false
            }
            KeyCode::Char('D') => {
                dialog.show_diff = !dialog.show_diff;
                if !compact_layout {
                    dialog.clamp_selection();
                }
                false
            }
            KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.restart_selected_plugin(dialog);
                false
            }
            KeyCode::Char('r') => {
                self.reset_selected_plugin_config_to_defaults(dialog);
                false
            }
            KeyCode::Char('a') | KeyCode::Char('A') => {
                self.open_add_config_value_editor(dialog);
                false
            }
            KeyCode::Char('t') | KeyCode::Char('T') => {
                self.open_config_type_selector(dialog);
                false
            }
            KeyCode::Enter | KeyCode::Char('e') => {
                if dialog.config_focus == PluginConfigFocus::Diagnostics {
                    self.jump_to_selected_bottom_item(dialog);
                } else {
                    self.open_selected_config_value_editor(dialog);
                }
                false
            }
            KeyCode::PageUp => {
                match dialog.config_focus {
                    PluginConfigFocus::Toolbar => {}
                    PluginConfigFocus::Structure => {
                        move_selected_config_section(dialog, -(CONFIG_EDITOR_PAGE_SIZE as isize));
                    }
                    PluginConfigFocus::Diagnostics => {
                        move_selected_bottom_panel_row(dialog, -(CONFIG_EDITOR_PAGE_SIZE as isize));
                    }
                    _ => move_selected_config_node(dialog, -(CONFIG_EDITOR_PAGE_SIZE as isize)),
                }
                false
            }
            KeyCode::PageDown => {
                match dialog.config_focus {
                    PluginConfigFocus::Toolbar => {}
                    PluginConfigFocus::Structure => {
                        move_selected_config_section(dialog, CONFIG_EDITOR_PAGE_SIZE as isize);
                    }
                    PluginConfigFocus::Diagnostics => {
                        move_selected_bottom_panel_row(dialog, CONFIG_EDITOR_PAGE_SIZE as isize);
                    }
                    _ => move_selected_config_node(dialog, CONFIG_EDITOR_PAGE_SIZE as isize),
                }
                false
            }
            KeyCode::Home => {
                match dialog.config_focus {
                    PluginConfigFocus::Toolbar => dialog.selected_toolbar_action = 0,
                    PluginConfigFocus::Structure => dialog.selected_section = 0,
                    PluginConfigFocus::Diagnostics => match dialog.show_diff {
                        true => dialog.selected_diff_row = 0,
                        false => dialog.selected_diagnostic = 0,
                    },
                    _ => dialog.selected_node = 0,
                }
                dialog.clamp_selection();
                false
            }
            KeyCode::End => {
                match dialog.config_focus {
                    PluginConfigFocus::Toolbar => {
                        dialog.selected_toolbar_action =
                            COMPACT_TOOLBAR_ACTIONS.len().saturating_sub(1);
                    }
                    PluginConfigFocus::Structure => {
                        dialog.selected_section = dialog
                            .selected_plugin()
                            .map(|plugin| plugin.sections.len().saturating_sub(1))
                            .unwrap_or_default();
                    }
                    PluginConfigFocus::Diagnostics => {
                        if dialog.show_diff {
                            dialog.selected_diff_row = dialog
                                .selected_plugin()
                                .map(|plugin| plugin.diff.len().saturating_sub(1))
                                .unwrap_or_default();
                        } else {
                            dialog.selected_diagnostic = dialog
                                .selected_plugin()
                                .map(plugin_all_diagnostics)
                                .map(|diagnostics| diagnostics.len().saturating_sub(1))
                                .unwrap_or_default();
                        }
                    }
                    _ => {
                        dialog.selected_node = dialog
                            .selected_section()
                            .map(|section| {
                                section_row_count(section, dialog.config_view).saturating_sub(1)
                            })
                            .unwrap_or_default();
                    }
                }
                dialog.clamp_selection();
                false
            }
            KeyCode::Left | KeyCode::Char('h') => {
                dialog.config_focus = previous_config_focus(dialog.config_focus, compact_layout);
                false
            }
            KeyCode::Right | KeyCode::Char('l') => {
                dialog.config_focus = next_config_focus(dialog.config_focus, compact_layout);
                false
            }
            KeyCode::Up | KeyCode::Char('k') => {
                match dialog.config_focus {
                    PluginConfigFocus::Structure => move_selected_config_section(dialog, -1),
                    PluginConfigFocus::Diagnostics => move_selected_bottom_panel_row(dialog, -1),
                    _ => move_selected_config_node(dialog, -1),
                }
                false
            }
            KeyCode::Down | KeyCode::Char('j') => {
                match dialog.config_focus {
                    PluginConfigFocus::Structure => move_selected_config_section(dialog, 1),
                    PluginConfigFocus::Diagnostics => move_selected_bottom_panel_row(dialog, 1),
                    _ => move_selected_config_node(dialog, 1),
                }
                false
            }
            _ => false,
        }
    }

    fn handle_plugin_config_actions_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut PluginWorkbenchOverlay,
    ) -> bool {
        let Some(overlay) = dialog.actions.as_mut() else {
            return false;
        };
        match key.code {
            KeyCode::Esc => {
                dialog.actions = None;
                false
            }
            KeyCode::Up | KeyCode::Char('k') => {
                move_index(&mut overlay.selected_action, overlay.actions.len(), -1);
                false
            }
            KeyCode::Down | KeyCode::Char('j') => {
                move_index(&mut overlay.selected_action, overlay.actions.len(), 1);
                false
            }
            KeyCode::Home => {
                overlay.selected_action = 0;
                false
            }
            KeyCode::End => {
                overlay.selected_action = overlay.actions.len().saturating_sub(1);
                false
            }
            KeyCode::Enter => {
                self.commit_plugin_config_action(dialog);
                false
            }
            _ => false,
        }
    }

    fn handle_plugin_config_selection_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut PluginWorkbenchOverlay,
    ) -> bool {
        let Some(overlay) = dialog.selection.as_mut() else {
            return false;
        };
        match key.code {
            KeyCode::Esc => {
                dialog.selection = None;
                false
            }
            KeyCode::Up | KeyCode::Char('k') => {
                move_index(&mut overlay.selected_item, overlay.items.len(), -1);
                false
            }
            KeyCode::Down | KeyCode::Char('j') => {
                move_index(&mut overlay.selected_item, overlay.items.len(), 1);
                false
            }
            KeyCode::Home => {
                overlay.selected_item = 0;
                false
            }
            KeyCode::End => {
                overlay.selected_item = overlay.items.len().saturating_sub(1);
                false
            }
            KeyCode::Char(' ') if overlay.multi => {
                if let Some(item) = overlay.items.get_mut(overlay.selected_item) {
                    item.checked = !item.checked;
                }
                false
            }
            KeyCode::Enter => {
                if let Err(error) = self.commit_plugin_config_selection(dialog) {
                    self.flash_error(error);
                } else {
                    dialog.selection = None;
                }
                false
            }
            _ => false,
        }
    }

    fn handle_plugin_config_drilldown_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut PluginWorkbenchOverlay,
    ) -> bool {
        let Some(overlay_snapshot) = dialog.current_drilldown().cloned() else {
            return false;
        };
        let view = dialog.config_view;
        match key.code {
            KeyCode::Esc => {
                dialog.drilldown_stack.pop();
                false
            }
            KeyCode::Up | KeyCode::Char('k') => {
                let count = drilldown_row_count(&overlay_snapshot, dialog.config_view);
                let Some(overlay) = dialog.current_drilldown_mut() else {
                    return false;
                };
                move_index(&mut overlay.selected_row, count, -1);
                overlay.selected_cell =
                    drilldown_selected_row_cell(overlay, view, overlay.selected_cell);
                false
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let count = drilldown_row_count(&overlay_snapshot, dialog.config_view);
                let Some(overlay) = dialog.current_drilldown_mut() else {
                    return false;
                };
                move_index(&mut overlay.selected_row, count, 1);
                overlay.selected_cell =
                    drilldown_selected_row_cell(overlay, view, overlay.selected_cell);
                false
            }
            KeyCode::PageUp => {
                let count = drilldown_row_count(&overlay_snapshot, dialog.config_view);
                let Some(overlay) = dialog.current_drilldown_mut() else {
                    return false;
                };
                move_index_page(
                    &mut overlay.selected_row,
                    count,
                    -1,
                    CONFIG_EDITOR_PAGE_SIZE,
                );
                overlay.selected_cell =
                    drilldown_selected_row_cell(overlay, view, overlay.selected_cell);
                false
            }
            KeyCode::PageDown => {
                let count = drilldown_row_count(&overlay_snapshot, dialog.config_view);
                let Some(overlay) = dialog.current_drilldown_mut() else {
                    return false;
                };
                move_index_page(&mut overlay.selected_row, count, 1, CONFIG_EDITOR_PAGE_SIZE);
                overlay.selected_cell =
                    drilldown_selected_row_cell(overlay, view, overlay.selected_cell);
                false
            }
            KeyCode::Home => {
                let Some(overlay) = dialog.current_drilldown_mut() else {
                    return false;
                };
                overlay.selected_row = 0;
                overlay.selected_cell =
                    drilldown_selected_row_cell(overlay, view, overlay.selected_cell);
                false
            }
            KeyCode::End => {
                let count = drilldown_row_count(&overlay_snapshot, dialog.config_view);
                let Some(overlay) = dialog.current_drilldown_mut() else {
                    return false;
                };
                overlay.selected_row = count.saturating_sub(1);
                overlay.selected_cell =
                    drilldown_selected_row_cell(overlay, view, overlay.selected_cell);
                false
            }
            KeyCode::Left | KeyCode::Char('h') => {
                self.move_selected_drilldown_cell(dialog, -1);
                false
            }
            KeyCode::Right | KeyCode::Char('l') => {
                self.move_selected_drilldown_cell(dialog, 1);
                false
            }
            KeyCode::Char('a') | KeyCode::Char('A') => {
                self.open_add_config_value_editor_for_path(
                    dialog,
                    overlay_snapshot.plugin_id.clone(),
                    overlay_snapshot.path.clone(),
                );
                false
            }
            KeyCode::Char('x') | KeyCode::Char('X') => {
                self.open_selected_config_actions(dialog);
                false
            }
            KeyCode::Char('t') | KeyCode::Char('T') => {
                self.open_config_type_selector(dialog);
                false
            }
            KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.delete_drilldown_selected_row(dialog);
                false
            }
            KeyCode::Enter | KeyCode::Char('e') => {
                self.open_drilldown_selected_row_editor(dialog);
                false
            }
            _ => false,
        }
    }

    fn save_selected_plugin_config(&mut self, dialog: &mut PluginWorkbenchOverlay) {
        let save_block = {
            let Some(plugin) = dialog.selected_plugin_mut() else {
                return;
            };
            recompute_plugin_config_state(plugin);
            plugin_save_block_reason(plugin)
        };
        if let Some(reason) = save_block {
            self.flash_error(reason);
            return;
        }
        let Some(plugin) = dialog.selected_plugin().cloned() else {
            return;
        };
        let mut configured_plugin_value = plugin_config_record_value(&plugin);
        let Some(plugin_object) = configured_plugin_value.as_object_mut() else {
            self.flash_error(format!(
                "plugin `{}` config record is not an object",
                plugin.plugin_id
            ));
            return;
        };
        plugin_object.insert("config".to_owned(), persisted_plugin_config_value(&plugin));
        let path = format!(
            "plugins.list.{}",
            quote_settings_segment(plugin.plugin_id.as_str())
        );
        match self.block_on_async(
            self.backend
                .set_config_setting(path.as_str(), configured_plugin_value),
        ) {
            Ok(_) => {
                self.flash_success(format!("saved plugin config for {}", plugin.plugin_id));
                self.refresh_plugin_workbench(dialog);
            }
            Err(error) => self.flash_error(error),
        }
    }

    fn validate_selected_plugin_config(&mut self, dialog: &mut PluginWorkbenchOverlay) {
        let Some(plugin) = dialog.selected_plugin_mut() else {
            return;
        };
        recompute_plugin_config_state(plugin);
        if plugin
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error)
        {
            self.flash_warning(format!(
                "{} config has {} issue(s)",
                plugin.plugin_id,
                plugin.diagnostics.len()
            ));
        } else {
            self.flash_success(format!("{} config is valid", plugin.plugin_id));
        }
    }

    fn insert_selected_plugin_defaults(&mut self, dialog: &mut PluginWorkbenchOverlay) {
        let Some(plugin) = dialog.selected_plugin_mut() else {
            return;
        };
        let Some(schema) = plugin.schema.clone() else {
            self.flash_warning("schema missing; defaults are unavailable".to_owned());
            return;
        };
        let before = plugin.draft_config.clone();
        insert_schema_defaults(&mut plugin.draft_config, &schema, &schema);
        if plugin.draft_config == before {
            self.flash_info("no missing defaults to insert".to_owned());
        } else {
            clear_branch_drafts_for_structural_change(plugin);
            recompute_plugin_config_state(plugin);
            self.flash_success(format!("inserted defaults for {}", plugin.plugin_id));
        }
    }

    fn run_compact_toolbar_action(&mut self, dialog: &mut PluginWorkbenchOverlay) {
        let action = COMPACT_TOOLBAR_ACTIONS
            .get(dialog.selected_toolbar_action)
            .copied()
            .unwrap_or(CompactToolbarAction::Validate);
        match action {
            CompactToolbarAction::Validate => self.validate_selected_plugin_config(dialog),
            CompactToolbarAction::ResetAll => self.reset_selected_plugin_config_to_defaults(dialog),
            CompactToolbarAction::Diff => {
                dialog.show_diff = !dialog.show_diff;
                dialog.clamp_selection();
            }
            CompactToolbarAction::Save => self.save_selected_plugin_config(dialog),
            CompactToolbarAction::Restart => self.restart_selected_plugin(dialog),
        }
    }

    fn restart_selected_plugin(&mut self, dialog: &mut PluginWorkbenchOverlay) {
        let Some(plugin) = dialog.selected_plugin() else {
            return;
        };
        self.flash_info(format!(
            "restart is not available for {} from this screen",
            plugin.plugin_id
        ));
    }

    fn reset_selected_plugin_config_to_defaults(&mut self, dialog: &mut PluginWorkbenchOverlay) {
        let Some(plugin) = dialog.selected_plugin_mut() else {
            return;
        };
        plugin.draft_config = plugin.default_config.clone();
        plugin.draft_override = JsonValue::Null;
        plugin.branch_drafts.clear();
        recompute_plugin_config_state(plugin);
        self.flash_success(format!(
            "reset {} config to plugin defaults",
            plugin.plugin_id
        ));
    }

    fn delete_selected_config_node(&mut self, dialog: &mut PluginWorkbenchOverlay) {
        let Some(row) = dialog.selected_row().cloned() else {
            return;
        };
        if row.primary_path.is_empty() {
            self.flash_warning("root config cannot be deleted".to_owned());
            return;
        }
        let Some(selected_plugin_id) = dialog
            .selected_plugin()
            .map(|plugin| plugin.plugin_id.clone())
        else {
            return;
        };
        let (changed, blocked) = if let Some(plugin) = dialog.selected_plugin_mut() {
            let outcome = apply_reset_paths(
                &mut plugin.draft_config,
                &plugin.default_config,
                plugin.schema.as_ref(),
                &row_paths(&row).into_iter().cloned().collect::<Vec<_>>(),
            );
            if outcome.changed {
                clear_branch_drafts_for_structural_change(plugin);
                recompute_plugin_config_state(plugin);
            }
            (outcome.changed, outcome.blocked)
        } else {
            (false, Vec::new())
        };
        if changed {
            select_config_path(
                dialog,
                selected_plugin_id.as_str(),
                row.primary_path.as_slice(),
            );
        }
        if let Some(message) = reset_paths_warning_message(blocked.as_slice()) {
            self.flash_warning(message);
        }
    }

    fn open_selected_config_actions(&mut self, dialog: &mut PluginWorkbenchOverlay) {
        let Some(context) = selected_config_row_context(dialog) else {
            return;
        };
        let plugin = dialog
            .plugins
            .iter()
            .find(|plugin| plugin.plugin_id == context.plugin_id);
        let primary_action = plugin.and_then(|plugin| {
            config_row_primary_action(
                plugin,
                &context.row.editor,
                context.row.primary_path.as_slice(),
                context.row.additional_paths.as_slice(),
            )
        });
        let mut actions = Vec::new();
        if context.row.type_mode.is_switchable() {
            let description = if context.row.type_mode == ConfigRowTypeMode::SelectShape {
                format!("Switch the active shape for {}.", context.row.title)
            } else {
                format!("Switch the active type for {}.", context.row.title)
            };
            actions.push(PluginConfigActionItem {
                label: context.row.type_mode.action_label().to_owned(),
                description,
                action: PluginConfigAction::SelectType {
                    plugin_id: context.plugin_id.clone(),
                    path: context.row.primary_path.clone(),
                },
            });
        }
        if let Some(plugin) = plugin
            && let ConfigRowEditor::Structured { path } = &context.row.editor
            && let Some(value) = get_value_at_path(&plugin.draft_config, path)
        {
            if value.is_array() && can_append_array_item(plugin, path.as_slice()) {
                actions.push(PluginConfigActionItem {
                    label: "Add Item".to_owned(),
                    description: format!("Append a new default item to {}.", context.row.title),
                    action: PluginConfigAction::AppendArrayItem {
                        plugin_id: context.plugin_id.clone(),
                        path: path.clone(),
                    },
                });
            }
            if value.is_object()
                && object_add_field_block_reason(plugin.schema.as_ref(), &plugin.draft_config, path)
                    .is_none()
            {
                actions.push(PluginConfigActionItem {
                    label: "Add Field".to_owned(),
                    description: format!("Add a new field inside {}.", context.row.title),
                    action: PluginConfigAction::PromptAddObjectField {
                        plugin_id: context.plugin_id.clone(),
                        path: path.clone(),
                    },
                });
            }
        }
        if let Some(plugin) = plugin
            && let Some(info) = array_item_action_info(plugin, context.row.primary_path.as_slice())
        {
            if info.can_insert_before {
                actions.push(PluginConfigActionItem {
                    label: "Insert Before".to_owned(),
                    description: "Insert a new default item before this array item.".to_owned(),
                    action: PluginConfigAction::InsertArrayItemBefore {
                        plugin_id: context.plugin_id.clone(),
                        path: context.row.primary_path.clone(),
                    },
                });
            }
            if info.can_insert_after {
                actions.push(PluginConfigActionItem {
                    label: "Insert After".to_owned(),
                    description: "Insert a new default item after this array item.".to_owned(),
                    action: PluginConfigAction::InsertArrayItemAfter {
                        plugin_id: context.plugin_id.clone(),
                        path: context.row.primary_path.clone(),
                    },
                });
            }
            if info.can_duplicate {
                actions.push(PluginConfigActionItem {
                    label: "Duplicate Item".to_owned(),
                    description: format!("Duplicate {} inside this array.", context.row.title),
                    action: PluginConfigAction::DuplicateArrayItem {
                        plugin_id: context.plugin_id.clone(),
                        path: context.row.primary_path.clone(),
                    },
                });
            }
            if info.can_move_up {
                actions.push(PluginConfigActionItem {
                    label: "Move Up".to_owned(),
                    description: "Move this array item one position earlier.".to_owned(),
                    action: PluginConfigAction::MoveArrayItem {
                        plugin_id: context.plugin_id.clone(),
                        path: context.row.primary_path.clone(),
                        direction: -1,
                    },
                });
            }
            if info.can_move_down {
                actions.push(PluginConfigActionItem {
                    label: "Move Down".to_owned(),
                    description: "Move this array item one position later.".to_owned(),
                    action: PluginConfigAction::MoveArrayItem {
                        plugin_id: context.plugin_id.clone(),
                        path: context.row.primary_path.clone(),
                        direction: 1,
                    },
                });
            }
            if info.can_remove {
                actions.push(PluginConfigActionItem {
                    label: "Remove Item".to_owned(),
                    description: format!("Remove {} from this array.", context.row.title),
                    action: PluginConfigAction::RemoveArrayItem {
                        plugin_id: context.plugin_id.clone(),
                        path: context.row.primary_path.clone(),
                    },
                });
            }
        }
        let rename_allowed = context.row.additional_paths.is_empty()
            && match plugin {
                Some(plugin) => {
                    row_rename_action_allowed(plugin, context.row.primary_path.as_slice())
                }
                None => path_key_info(context.row.primary_path.as_slice()).is_some(),
            };
        if rename_allowed {
            actions.push(PluginConfigActionItem {
                label: "Rename Field".to_owned(),
                description: format!("Rename the key for {}.", context.row.title),
                action: PluginConfigAction::RenameField {
                    plugin_id: context.plugin_id.clone(),
                    path: context.row.primary_path.clone(),
                },
            });
        }
        let field_paths = row_paths(&context.row)
            .into_iter()
            .cloned()
            .collect::<Vec<_>>();
        actions.push(PluginConfigActionItem {
            label: "Reset Field".to_owned(),
            description: format!("Restore {} to the plugin default value.", context.row.title),
            action: PluginConfigAction::ResetField {
                plugin_id: context.plugin_id.clone(),
                paths: field_paths,
                focus_path: context.row.primary_path.clone(),
            },
        });
        actions.push(PluginConfigActionItem {
            label: "Reset Group".to_owned(),
            description: format!(
                "Restore every field in {} to the plugin defaults.",
                context.group_title
            ),
            action: PluginConfigAction::ResetGroup {
                plugin_id: context.plugin_id,
                paths: context.group_paths,
                focus_path: context.row.primary_path,
            },
        });
        let selected_action =
            prioritize_config_actions(actions.as_mut_slice(), context.cell, primary_action);
        dialog.actions = Some(PluginConfigActionOverlay {
            title: if primary_action.is_some() {
                "More Actions".to_owned()
            } else {
                "Field Actions".to_owned()
            },
            subject: context.row.title,
            footer: config_actions_overlay_footer(primary_action),
            actions,
            selected_action,
        });
    }

    fn commit_plugin_config_action(&mut self, dialog: &mut PluginWorkbenchOverlay) {
        let Some(overlay) = dialog.actions.clone() else {
            return;
        };
        let Some(item) = overlay.actions.get(overlay.selected_action).cloned() else {
            dialog.actions = None;
            return;
        };
        match item.action {
            PluginConfigAction::SelectType { plugin_id, path } => {
                self.focus_config_path(dialog, plugin_id.as_str(), path.as_slice());
                dialog.clamp_selection();
                self.open_config_type_selector(dialog);
            }
            PluginConfigAction::AppendArrayItem { plugin_id, path } => {
                self.append_config_array_item(dialog, plugin_id.as_str(), path.as_slice());
            }
            PluginConfigAction::PromptAddObjectField { plugin_id, path } => {
                self.open_add_config_value_editor_for_path(dialog, plugin_id, path);
            }
            PluginConfigAction::InsertArrayItemBefore { plugin_id, path } => {
                self.insert_array_item(dialog, plugin_id.as_str(), path.as_slice(), false);
            }
            PluginConfigAction::InsertArrayItemAfter { plugin_id, path } => {
                self.insert_array_item(dialog, plugin_id.as_str(), path.as_slice(), true);
            }
            PluginConfigAction::DuplicateArrayItem { plugin_id, path } => {
                self.duplicate_array_item(dialog, plugin_id.as_str(), path.as_slice());
            }
            PluginConfigAction::MoveArrayItem {
                plugin_id,
                path,
                direction,
            } => {
                self.move_array_item(dialog, plugin_id.as_str(), path.as_slice(), direction);
            }
            PluginConfigAction::RemoveArrayItem { plugin_id, path } => {
                self.remove_array_item(dialog, plugin_id.as_str(), path.as_slice());
            }
            PluginConfigAction::RenameField { plugin_id, path } => {
                self.open_rename_field_editor(dialog, plugin_id, path);
            }
            PluginConfigAction::ResetField {
                plugin_id,
                paths,
                focus_path,
            }
            | PluginConfigAction::ResetGroup {
                plugin_id,
                paths,
                focus_path,
            } => {
                self.reset_config_paths(dialog, plugin_id.as_str(), paths.as_slice(), &focus_path);
            }
        }
        dialog.actions = None;
    }

    fn jump_to_selected_bottom_item(&mut self, dialog: &mut PluginWorkbenchOverlay) {
        let Some(plugin) = dialog.selected_plugin() else {
            return;
        };
        let target_path = if dialog.show_diff {
            plugin
                .diff
                .get(dialog.selected_diff_row)
                .map(|row| row.path.clone())
        } else {
            plugin_all_diagnostics(plugin)
                .get(dialog.selected_diagnostic)
                .map(|diagnostic| diagnostic.path.clone())
        };
        let Some(target_path) = target_path else {
            return;
        };
        let plugin_id = plugin.plugin_id.clone();
        if dialog
            .selected_plugin()
            .and_then(|plugin| find_row_position(plugin, dialog.config_view, &target_path))
            .is_none()
        {
            dialog.config_view = PluginConfigView::Effective;
        }
        self.focus_config_path(dialog, plugin_id.as_str(), target_path.as_slice());
        dialog.config_focus = PluginConfigFocus::Editor;
    }

    fn reset_config_paths(
        &mut self,
        dialog: &mut PluginWorkbenchOverlay,
        plugin_id: &str,
        paths: &[ConfigPath],
        focus_path: &ConfigPath,
    ) {
        let (changed, blocked) = if let Some(plugin) = dialog
            .plugins
            .iter_mut()
            .find(|plugin| plugin.plugin_id == plugin_id)
        {
            let outcome = apply_reset_paths(
                &mut plugin.draft_config,
                &plugin.default_config,
                plugin.schema.as_ref(),
                paths,
            );
            if outcome.changed {
                clear_branch_drafts_for_structural_change(plugin);
                recompute_plugin_config_state(plugin);
            }
            (outcome.changed, outcome.blocked)
        } else {
            (false, Vec::new())
        };
        if changed {
            self.focus_config_path(dialog, plugin_id, focus_path.as_slice());
            dialog.drilldown_stack =
                rebuild_drilldown_stack(dialog, dialog.drilldown_stack.as_slice());
        }
        if let Some(message) = reset_paths_warning_message(blocked.as_slice()) {
            self.flash_warning(message);
        }
    }

    fn duplicate_array_item(
        &mut self,
        dialog: &mut PluginWorkbenchOverlay,
        plugin_id: &str,
        path: &[PathSegment],
    ) {
        let next_focus = if let Some(plugin) = dialog
            .plugins
            .iter_mut()
            .find(|plugin| plugin.plugin_id == plugin_id)
        {
            if !array_item_action_info(plugin, path).is_some_and(|info| info.can_duplicate) {
                self.flash_warning("cannot duplicate this array item".to_owned());
                return;
            }
            let focus = duplicate_array_item_at_path(&mut plugin.draft_config, path);
            if focus.is_some() {
                clear_branch_drafts_for_structural_change(plugin);
                recompute_plugin_config_state(plugin);
            }
            focus
        } else {
            None
        };
        if let Some(focus) = next_focus {
            self.focus_config_path(dialog, plugin_id, focus.as_slice());
            dialog.drilldown_stack =
                rebuild_drilldown_stack(dialog, dialog.drilldown_stack.as_slice());
        }
    }

    fn insert_array_item(
        &mut self,
        dialog: &mut PluginWorkbenchOverlay,
        plugin_id: &str,
        path: &[PathSegment],
        after: bool,
    ) {
        let next_focus = if let Some(plugin) = dialog
            .plugins
            .iter_mut()
            .find(|plugin| plugin.plugin_id == plugin_id)
        {
            let focus = insert_default_array_item_at_path(
                &mut plugin.draft_config,
                plugin.schema.as_ref(),
                path,
                after,
            );
            if focus.is_some() {
                clear_branch_drafts_for_structural_change(plugin);
                recompute_plugin_config_state(plugin);
            }
            focus
        } else {
            None
        };
        if let Some(focus) = next_focus {
            self.focus_config_path(dialog, plugin_id, focus.as_slice());
            dialog.drilldown_stack =
                rebuild_drilldown_stack(dialog, dialog.drilldown_stack.as_slice());
            self.maybe_open_type_selector_for_selected_row(dialog, plugin_id, focus.as_slice());
        } else {
            self.flash_warning("cannot insert an item at this array position".to_owned());
        }
    }

    fn move_array_item(
        &mut self,
        dialog: &mut PluginWorkbenchOverlay,
        plugin_id: &str,
        path: &[PathSegment],
        direction: isize,
    ) {
        let next_focus = if let Some(plugin) = dialog
            .plugins
            .iter_mut()
            .find(|plugin| plugin.plugin_id == plugin_id)
        {
            let focus = move_array_item_at_path(&mut plugin.draft_config, path, direction);
            if focus.is_some() {
                clear_branch_drafts_for_structural_change(plugin);
                recompute_plugin_config_state(plugin);
            }
            focus
        } else {
            None
        };
        if let Some(focus) = next_focus {
            self.focus_config_path(dialog, plugin_id, focus.as_slice());
            dialog.drilldown_stack =
                rebuild_drilldown_stack(dialog, dialog.drilldown_stack.as_slice());
        }
    }

    fn remove_array_item(
        &mut self,
        dialog: &mut PluginWorkbenchOverlay,
        plugin_id: &str,
        path: &[PathSegment],
    ) {
        let next_focus = if let Some(plugin) = dialog
            .plugins
            .iter_mut()
            .find(|plugin| plugin.plugin_id == plugin_id)
        {
            if !array_item_action_info(plugin, path).is_some_and(|info| info.can_remove) {
                self.flash_warning("cannot remove this array item".to_owned());
                return;
            }
            let focus = remove_array_item_at_path(&mut plugin.draft_config, path);
            if focus.is_some() {
                clear_branch_drafts_for_structural_change(plugin);
                recompute_plugin_config_state(plugin);
            }
            focus
        } else {
            None
        };
        if let Some(focus) = next_focus {
            self.focus_config_path(dialog, plugin_id, focus.as_slice());
            dialog.drilldown_stack =
                rebuild_drilldown_stack(dialog, dialog.drilldown_stack.as_slice());
        }
    }

    fn open_rename_field_editor(
        &mut self,
        dialog: &mut PluginWorkbenchOverlay,
        plugin_id: String,
        path: ConfigPath,
    ) {
        let Some((_, key)) = path_key_info(path.as_slice()) else {
            self.flash_warning("selected row does not point to an object field".to_owned());
            return;
        };
        dialog.editor = Some(EditorDialogState::new(
            format!("Rename {}", title_from_key(key.as_str())),
            "Enter the new field name.".to_owned(),
            "Enter rename  Esc cancel".to_owned(),
            false,
            Editor::from_text(key),
            PluginConfigEditAction::RenameObjectField { plugin_id, path },
        ));
    }

    fn focus_config_path(
        &mut self,
        dialog: &mut PluginWorkbenchOverlay,
        plugin_id: &str,
        target_path: &[PathSegment],
    ) {
        dialog.drilldown_stack.clear();
        select_config_path(dialog, plugin_id, target_path);
        if dialog
            .selected_plugin()
            .and_then(|plugin| find_row_position(plugin, dialog.config_view, target_path))
            .is_some()
        {
            return;
        }
        let Some((section_index, row_index, row)) = dialog.selected_plugin().and_then(|plugin| {
            find_best_section_row_for_path(plugin, dialog.config_view, target_path)
        }) else {
            return;
        };
        dialog.selected_section = section_index;
        dialog.selected_node = row_index;
        dialog.clamp_selection();
        let ConfigRowEditor::Structured { path } = &row.editor else {
            return;
        };
        self.open_structured_row_drilldown(
            dialog,
            plugin_id.to_owned(),
            path.clone(),
            row.title.clone(),
        );
        self.focus_drilldown_path(dialog, target_path);
    }

    fn focus_drilldown_path(
        &mut self,
        dialog: &mut PluginWorkbenchOverlay,
        target_path: &[PathSegment],
    ) {
        loop {
            let Some(overlay) = dialog.current_drilldown().cloned() else {
                return;
            };
            let Some((row_index, row)) =
                find_best_drilldown_row_for_path(&overlay, dialog.config_view, target_path)
            else {
                return;
            };
            if let Some(current) = dialog.current_drilldown_mut() {
                current.selected_row = row_index;
            }
            if row.primary_path.as_slice() == target_path
                || row
                    .additional_paths
                    .iter()
                    .any(|candidate| candidate.as_slice() == target_path)
            {
                return;
            }
            let ConfigRowEditor::Structured { path } = &row.editor else {
                return;
            };
            self.open_structured_row_drilldown(
                dialog,
                overlay.plugin_id.clone(),
                path.clone(),
                row.title.clone(),
            );
        }
    }

    fn open_add_config_value_editor(&mut self, dialog: &mut PluginWorkbenchOverlay) {
        let Some(row) = dialog.selected_row().cloned() else {
            return;
        };
        let Some(plugin) = dialog.selected_plugin() else {
            return;
        };
        self.open_add_config_value_editor_for_path(
            dialog,
            plugin.plugin_id.clone(),
            row.primary_path,
        );
    }

    fn open_add_config_value_editor_for_path(
        &mut self,
        dialog: &mut PluginWorkbenchOverlay,
        plugin_id: String,
        path: ConfigPath,
    ) {
        let Some(plugin) = dialog
            .plugins
            .iter()
            .find(|plugin| plugin.plugin_id == plugin_id)
        else {
            return;
        };
        let value = get_value_at_path(&plugin.draft_config, &path).unwrap_or(&JsonValue::Null);
        if value.is_object() {
            if let Some(reason) =
                object_add_field_block_reason(plugin.schema.as_ref(), &plugin.draft_config, &path)
            {
                self.flash_warning(reason);
                return;
            }
            dialog.editor = Some(EditorDialogState::new(
                "Add Field".to_owned(),
                format!(
                    "Enter a field name for {}. If the schema allows multiple value types or shapes, the editor will prompt you after create.",
                    path_display(&path)
                ),
                "Enter create  Esc cancel".to_owned(),
                false,
                Editor::default(),
                PluginConfigEditAction::AddObjectField { plugin_id, path },
            ));
        } else if value.is_array() {
            self.append_config_array_item(dialog, plugin_id.as_str(), path.as_slice());
        } else {
            self.flash_warning("add is available for object and array nodes".to_owned());
        }
    }

    fn append_config_array_item(
        &mut self,
        dialog: &mut PluginWorkbenchOverlay,
        plugin_id: &str,
        path: &[PathSegment],
    ) {
        let Some(plugin_index) = dialog
            .plugins
            .iter()
            .position(|plugin| plugin.plugin_id == plugin_id)
        else {
            return;
        };
        let (plugin_id, focus_path, can_append) = {
            let plugin = &mut dialog.plugins[plugin_index];
            let can_append = can_append_array_item(plugin, path);
            let focus_path = append_default_array_item_at_path(
                &mut plugin.draft_config,
                plugin.schema.as_ref(),
                path,
            );
            if focus_path.is_some() {
                clear_branch_drafts_for_structural_change(plugin);
                recompute_plugin_config_state(plugin);
            }
            (plugin.plugin_id.clone(), focus_path, can_append)
        };
        if let Some(focus_path) = focus_path {
            self.focus_config_path(dialog, plugin_id.as_str(), focus_path.as_slice());
            dialog.clamp_selection();
            self.maybe_open_type_selector_for_selected_row(
                dialog,
                plugin_id.as_str(),
                focus_path.as_slice(),
            );
        } else if !can_append {
            self.flash_warning("cannot add another item at this array position".to_owned());
        } else {
            self.flash_warning("failed to append array item".to_owned());
        }
    }

    fn move_selected_main_config_cell(
        &mut self,
        dialog: &mut PluginWorkbenchOverlay,
        delta: isize,
    ) -> bool {
        let Some(section) = dialog.selected_section() else {
            return false;
        };
        let Some(row) = section_row_at(section, dialog.config_view, dialog.selected_node) else {
            return false;
        };
        let layout = section_group_for_row(section, dialog.config_view, dialog.selected_node)
            .map(|group| group.layout)
            .unwrap_or(ConfigGroupLayout::Standard);
        let Some(next) = move_config_row_cell(row, layout, dialog.selected_cell, delta) else {
            return false;
        };
        dialog.selected_cell = next;
        true
    }

    fn move_selected_drilldown_cell(
        &mut self,
        dialog: &mut PluginWorkbenchOverlay,
        delta: isize,
    ) -> bool {
        let Some(overlay_snapshot) = dialog.current_drilldown().cloned() else {
            return false;
        };
        let Some(row) = drilldown_row_at(
            &overlay_snapshot,
            dialog.config_view,
            overlay_snapshot.selected_row,
        ) else {
            return false;
        };
        let layout = drilldown_group_for_row(
            &overlay_snapshot,
            dialog.config_view,
            overlay_snapshot.selected_row,
        )
        .map(|group| group.layout)
        .unwrap_or(ConfigGroupLayout::Standard);
        let Some(next) = move_config_row_cell(row, layout, overlay_snapshot.selected_cell, delta)
        else {
            return false;
        };
        let Some(overlay) = dialog.current_drilldown_mut() else {
            return false;
        };
        overlay.selected_cell = next;
        true
    }

    fn open_config_type_selector(&mut self, dialog: &mut PluginWorkbenchOverlay) {
        let Some(context) = selected_config_row_context(dialog) else {
            return;
        };
        if dialog.current_drilldown().is_some() {
            if let Some(overlay) = dialog.current_drilldown_mut() {
                overlay.selected_cell = ConfigRowCell::Type;
            }
        } else {
            dialog.selected_cell = ConfigRowCell::Type;
        }
        self.open_type_selector_for_row(dialog, context.plugin_id, context.row);
    }

    fn maybe_open_type_selector_for_selected_row(
        &mut self,
        dialog: &mut PluginWorkbenchOverlay,
        plugin_id: &str,
        path: &[PathSegment],
    ) {
        let Some(context) = selected_config_row_context(dialog) else {
            return;
        };
        if context.plugin_id != plugin_id
            || context.row.primary_path.as_slice() != path
            || !context.row.type_mode.is_switchable()
        {
            return;
        }
        self.open_type_selector_for_row(dialog, context.plugin_id, context.row);
    }

    fn open_type_selector_for_row(
        &mut self,
        dialog: &mut PluginWorkbenchOverlay,
        plugin_id: String,
        row: ConfigRowView,
    ) {
        if dialog.current_drilldown().is_some() {
            if let Some(overlay) = dialog.current_drilldown_mut() {
                overlay.selected_cell = ConfigRowCell::Type;
            }
        } else {
            dialog.selected_cell = ConfigRowCell::Type;
        }
        let Some(plugin) = dialog
            .plugins
            .iter()
            .find(|plugin| plugin.plugin_id == plugin_id)
        else {
            return;
        };
        let schema = plugin.schema.as_ref().and_then(|schema| {
            declared_schema_for_path(schema, schema, &plugin.draft_config, &row.primary_path)
        });
        if let Some(branches) = schema.as_ref().and_then(|schema| {
            plugin
                .schema
                .as_ref()
                .and_then(|root| branch_choices(root, schema))
        }) {
            self.open_branch_selection_overlay(
                dialog,
                "Select Branch".to_owned(),
                "Choose schema shape".to_owned(),
                plugin.plugin_id.clone(),
                row.primary_path.clone(),
                branches,
                get_value_at_path(&plugin.draft_config, &row.primary_path)
                    .cloned()
                    .unwrap_or(JsonValue::Null),
            );
            return;
        }
        let current_value = get_value_at_path(&plugin.draft_config, &row.primary_path)
            .cloned()
            .unwrap_or(JsonValue::Null);
        let choices = schema_type_selector_choices(schema.as_ref());
        if choices.is_empty() {
            self.flash_warning(format!("{} has no selectable schema type", row.title));
            return;
        }
        if choices.len() == 1 {
            let choice = choices[0].clone();
            if value_matches_type(&current_value, choice.as_str()) {
                self.flash_warning(format!("{} has a fixed schema type", row.title));
                return;
            }
            let value = schema
                .as_ref()
                .map(|schema| {
                    default_value_for_schema(schema, plugin.schema.as_ref().unwrap_or(schema))
                })
                .unwrap_or_else(|| JsonValue::Null);
            self.set_config_value_at(
                dialog,
                plugin.plugin_id.clone(),
                row.primary_path.clone(),
                value,
            );
            return;
        }
        self.open_named_selection_overlay(
            dialog,
            "Select Type".to_owned(),
            format!("Choose JSON type for {}", path_display(&row.primary_path)),
            "Enter apply  Esc cancel  Up/Down move".to_owned(),
            false,
            choices
                .into_iter()
                .map(|choice| PluginConfigSelectionItem {
                    checked: value_matches_type(&current_value, choice.as_str()),
                    label: choice.clone(),
                    description: None,
                    value: PluginConfigSelectionValue::Named(choice),
                })
                .collect(),
            PluginConfigSelectionAction::SelectType {
                plugin_id: plugin.plugin_id.clone(),
                path: row.primary_path.clone(),
            },
        );
    }

    fn open_selected_config_value_editor(&mut self, dialog: &mut PluginWorkbenchOverlay) {
        let Some(context) = selected_config_row_context(dialog) else {
            return;
        };
        self.open_row_cell_editor(
            dialog,
            context.plugin_id,
            context.row,
            context.layout,
            context.cell,
        );
    }

    fn open_drilldown_selected_row_editor(&mut self, dialog: &mut PluginWorkbenchOverlay) {
        self.open_selected_config_value_editor(dialog);
    }

    fn open_row_cell_editor(
        &mut self,
        dialog: &mut PluginWorkbenchOverlay,
        plugin_id: String,
        row: ConfigRowView,
        layout: ConfigGroupLayout,
        cell: ConfigRowCell,
    ) {
        let cell = normalize_config_row_cell(&row, layout, cell);
        match cell {
            ConfigRowCell::Default => {
                if dialog.current_drilldown().is_some() {
                    self.delete_drilldown_selected_row(dialog);
                } else {
                    self.delete_selected_config_node(dialog);
                }
                return;
            }
            ConfigRowCell::State => {
                self.open_selected_config_actions(dialog);
                return;
            }
            ConfigRowCell::Action => {
                let Some(primary_action) = dialog
                    .plugins
                    .iter()
                    .find(|plugin| plugin.plugin_id == plugin_id)
                    .and_then(|plugin| {
                        config_row_primary_action(
                            plugin,
                            &row.editor,
                            row.primary_path.as_slice(),
                            row.additional_paths.as_slice(),
                        )
                    })
                else {
                    self.open_selected_config_actions(dialog);
                    return;
                };
                match primary_action {
                    ConfigRowPrimaryAction::InsertAfter => {
                        self.insert_array_item(
                            dialog,
                            plugin_id.as_str(),
                            row.primary_path.as_slice(),
                            true,
                        );
                    }
                    ConfigRowPrimaryAction::Duplicate => {
                        self.duplicate_array_item(
                            dialog,
                            plugin_id.as_str(),
                            row.primary_path.as_slice(),
                        );
                    }
                    ConfigRowPrimaryAction::MoveDown => {
                        self.move_array_item(
                            dialog,
                            plugin_id.as_str(),
                            row.primary_path.as_slice(),
                            1,
                        );
                    }
                    ConfigRowPrimaryAction::MoveUp => {
                        self.move_array_item(
                            dialog,
                            plugin_id.as_str(),
                            row.primary_path.as_slice(),
                            -1,
                        );
                    }
                    ConfigRowPrimaryAction::Remove => {
                        self.remove_array_item(
                            dialog,
                            plugin_id.as_str(),
                            row.primary_path.as_slice(),
                        );
                    }
                    ConfigRowPrimaryAction::AddField | ConfigRowPrimaryAction::AddItem => {
                        if let ConfigRowEditor::Structured { path } = row.editor.clone() {
                            self.open_add_config_value_editor_for_path(dialog, plugin_id, path);
                        }
                    }
                    ConfigRowPrimaryAction::Rename => {
                        self.open_rename_field_editor(dialog, plugin_id, row.primary_path.clone());
                    }
                }
                return;
            }
            _ => {}
        }
        if cell == ConfigRowCell::Type && row.type_mode.is_switchable() {
            self.open_type_selector_for_row(dialog, plugin_id, row);
            return;
        }
        if cell == ConfigRowCell::Value
            && let ConfigRowEditor::NullableString { path } = row.editor.clone()
        {
            self.open_nullable_string_value_editor(dialog, plugin_id, row, path);
            return;
        }
        if let ConfigRowEditor::PairInteger {
            left_path,
            right_path,
        } = &row.editor
        {
            let (left_label, right_label) =
                pair_editor_labels(left_path.as_slice(), right_path.as_slice());
            let (path, label) = if cell == ConfigRowCell::SecondaryValue {
                (right_path.clone(), right_label)
            } else {
                (left_path.clone(), left_label)
            };
            self.open_pair_integer_value_editor(dialog, plugin_id, row, path, label);
            return;
        }
        self.open_row_editor(dialog, plugin_id, row);
    }

    fn open_nullable_string_value_editor(
        &mut self,
        dialog: &mut PluginWorkbenchOverlay,
        plugin_id: String,
        row: ConfigRowView,
        path: ConfigPath,
    ) {
        let Some(plugin) = dialog
            .plugins
            .iter()
            .find(|plugin| plugin.plugin_id == plugin_id)
        else {
            return;
        };
        let current = get_value_at_path(&plugin.draft_config, &path)
            .and_then(JsonValue::as_str)
            .unwrap_or_default()
            .to_owned();
        dialog.editor = Some(EditorDialogState::new(
            format!("Edit {}", row.title),
            field_prompt_for_path(plugin, &path),
            editor_save_footer(&self.i18n, false),
            false,
            Editor::from_text(current),
            PluginConfigEditAction::SetNullableString {
                plugin_id: plugin.plugin_id.clone(),
                path,
            },
        ));
    }

    fn open_pair_integer_value_editor(
        &mut self,
        dialog: &mut PluginWorkbenchOverlay,
        plugin_id: String,
        row: ConfigRowView,
        path: ConfigPath,
        label: &str,
    ) {
        let Some(plugin) = dialog
            .plugins
            .iter()
            .find(|plugin| plugin.plugin_id == plugin_id)
        else {
            return;
        };
        let current = get_value_at_path(&plugin.draft_config, &path)
            .and_then(JsonValue::as_i64)
            .map(|value| value.to_string())
            .unwrap_or_else(|| "0".to_owned());
        dialog.editor = Some(EditorDialogState::new(
            format!("Edit {} · {}", row.title, label),
            field_prompt_for_path(plugin, &path),
            "Enter save  Esc cancel".to_owned(),
            false,
            Editor::from_text(current),
            PluginConfigEditAction::SetScalar {
                plugin_id: plugin.plugin_id.clone(),
                path,
                kind: ScalarEditKind::Integer,
            },
        ));
    }

    fn open_row_editor(
        &mut self,
        dialog: &mut PluginWorkbenchOverlay,
        plugin_id: String,
        row: ConfigRowView,
    ) {
        let Some(plugin) = dialog
            .plugins
            .iter()
            .find(|plugin| plugin.plugin_id == plugin_id)
        else {
            return;
        };
        match row.editor.clone() {
            ConfigRowEditor::Bool { path } => {
                let current = get_value_at_path(&plugin.draft_config, &path)
                    .and_then(JsonValue::as_bool)
                    .unwrap_or(false);
                self.set_config_value_at(dialog, plugin.plugin_id.clone(), path, json!(!current));
                return;
            }
            ConfigRowEditor::ReadOnly { .. } => {
                self.flash_warning(format!("{} is read-only", row.title));
                return;
            }
            ConfigRowEditor::NullableString { path } => {
                self.open_nullable_string_value_editor(dialog, plugin.plugin_id.clone(), row, path);
                return;
            }
            ConfigRowEditor::PairInteger {
                left_path,
                right_path,
            } => {
                let (left_label, right_label) =
                    pair_editor_labels(left_path.as_slice(), right_path.as_slice());
                let left = get_value_at_path(&plugin.draft_config, &left_path)
                    .and_then(JsonValue::as_i64)
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "0".to_owned());
                let right = get_value_at_path(&plugin.draft_config, &right_path)
                    .and_then(JsonValue::as_i64)
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "0".to_owned());
                dialog.editor = Some(EditorDialogState::new(
                    format!("Edit {}", row.title),
                    format!(
                        "Enter the two values for {}.\nFirst line: {}\nSecond line: {}",
                        row.title, left_label, right_label
                    ),
                    editor_save_footer(&self.i18n, true),
                    true,
                    Editor::from_text(format!("{left}\n{right}")),
                    PluginConfigEditAction::SetPairIntegers {
                        plugin_id: plugin.plugin_id.clone(),
                        left_path,
                        right_path,
                    },
                ));
                return;
            }
            ConfigRowEditor::Structured { path } => {
                self.open_structured_row_drilldown(
                    dialog,
                    plugin.plugin_id.clone(),
                    path,
                    row.title.clone(),
                );
                return;
            }
            ConfigRowEditor::MultiEnum { path, variants } => {
                let current = get_value_at_path(&plugin.draft_config, &path)
                    .and_then(JsonValue::as_array)
                    .cloned()
                    .unwrap_or_default();
                self.open_multi_enum_selection_overlay(
                    dialog,
                    row.title.clone(),
                    plugin.plugin_id.clone(),
                    path,
                    variants,
                    current,
                );
                return;
            }
            _ => {}
        }
        let value =
            get_value_at_path(&plugin.draft_config, &row.primary_path).unwrap_or(&JsonValue::Null);
        let schema = plugin.schema.as_ref().and_then(|schema| {
            declared_schema_for_path(schema, schema, &plugin.draft_config, &row.primary_path)
        });
        if let Some(variants) = schema
            .as_ref()
            .and_then(schema_enum_values)
            .filter(|variants| !variants.is_empty())
        {
            self.open_enum_selection_overlay(
                dialog,
                row.title.clone(),
                plugin.plugin_id.clone(),
                row.primary_path.clone(),
                variants,
                value.clone(),
            );
            return;
        }
        if let Some(branches) = schema.as_ref().and_then(|schema| {
            plugin
                .schema
                .as_ref()
                .and_then(|root| branch_choices(root, schema))
        }) {
            self.open_branch_selection_overlay(
                dialog,
                "Select Branch".to_owned(),
                row.title.clone(),
                plugin.plugin_id.clone(),
                row.primary_path.clone(),
                branches,
                value.clone(),
            );
            return;
        }
        match value {
            JsonValue::Bool(current) => {
                self.set_config_value_at(
                    dialog,
                    plugin.plugin_id.clone(),
                    row.primary_path.clone(),
                    json!(!current),
                );
            }
            JsonValue::String(text) => {
                let multiline = schema
                    .as_ref()
                    .is_some_and(|schema| schema_string_is_multiline(schema));
                dialog.editor = Some(EditorDialogState::new(
                    format!("Edit {}", row.title),
                    field_prompt_for_row(schema.as_ref(), &row),
                    editor_save_footer(&self.i18n, multiline),
                    multiline,
                    Editor::from_text(text.clone()),
                    PluginConfigEditAction::SetScalar {
                        plugin_id: plugin.plugin_id.clone(),
                        path: row.primary_path.clone(),
                        kind: ScalarEditKind::String,
                    },
                ));
            }
            JsonValue::Number(number) => {
                dialog.editor = Some(EditorDialogState::new(
                    format!("Edit {}", row.title),
                    field_prompt_for_row(schema.as_ref(), &row),
                    "Enter save  Esc cancel".to_owned(),
                    false,
                    Editor::from_text(number.to_string()),
                    PluginConfigEditAction::SetScalar {
                        plugin_id: plugin.plugin_id.clone(),
                        path: row.primary_path.clone(),
                        kind: if number.as_i64().is_some() || number.as_u64().is_some() {
                            ScalarEditKind::Integer
                        } else {
                            ScalarEditKind::Number
                        },
                    },
                ));
            }
            JsonValue::Null => {
                self.open_type_selector_for_row(dialog, plugin.plugin_id.clone(), row)
            }
            JsonValue::Object(_) | JsonValue::Array(_) => {
                self.open_structured_row_drilldown(
                    dialog,
                    plugin.plugin_id.clone(),
                    row.primary_path.clone(),
                    row.title.clone(),
                );
            }
        }
    }

    fn set_config_value_at(
        &mut self,
        dialog: &mut PluginWorkbenchOverlay,
        plugin_id: String,
        path: ConfigPath,
        value: JsonValue,
    ) -> bool {
        match self.try_set_config_value_at(dialog, plugin_id, path, value) {
            Ok(()) => true,
            Err(error) => {
                self.flash_warning(error);
                false
            }
        }
    }

    fn try_set_config_values_at(
        &mut self,
        dialog: &mut PluginWorkbenchOverlay,
        plugin_id: String,
        updates: Vec<(ConfigPath, JsonValue)>,
        focus_path: ConfigPath,
    ) -> UiResult<()> {
        let Some(plugin_index) = dialog
            .plugins
            .iter()
            .position(|plugin| plugin.plugin_id == plugin_id)
        else {
            return Ok(());
        };
        let next_config = {
            let plugin = &dialog.plugins[plugin_index];
            apply_staged_config_value_updates(
                plugin.schema.as_ref(),
                &plugin.draft_config,
                updates.as_slice(),
            )?
        };
        {
            let plugin = &mut dialog.plugins[plugin_index];
            plugin.draft_config = next_config;
            recompute_plugin_config_state(plugin);
        }
        self.focus_config_path(dialog, plugin_id.as_str(), focus_path.as_slice());
        dialog.clamp_selection();
        Ok(())
    }

    fn try_set_config_value_at(
        &mut self,
        dialog: &mut PluginWorkbenchOverlay,
        plugin_id: String,
        path: ConfigPath,
        value: JsonValue,
    ) -> UiResult<()> {
        self.try_set_config_values_at(dialog, plugin_id, vec![(path.clone(), value)], path)
    }

    fn open_named_selection_overlay(
        &mut self,
        dialog: &mut PluginWorkbenchOverlay,
        title: String,
        prompt: String,
        footer: String,
        multi: bool,
        items: Vec<PluginConfigSelectionItem>,
        action: PluginConfigSelectionAction,
    ) {
        let selected_item = items.iter().position(|item| item.checked).unwrap_or(0);
        dialog.selection = Some(PluginConfigSelectionOverlay {
            title,
            prompt,
            footer,
            multi,
            items,
            selected_item,
            action,
        });
        dialog.clamp_selection();
    }

    fn open_branch_selection_overlay(
        &mut self,
        dialog: &mut PluginWorkbenchOverlay,
        title: String,
        prompt: String,
        plugin_id: String,
        path: ConfigPath,
        branches: Vec<BranchChoice>,
        current: JsonValue,
    ) {
        let active = active_branch_id(branches.as_slice(), &current).to_owned();
        let items = branches
            .into_iter()
            .map(|branch| PluginConfigSelectionItem {
                label: branch.label.clone(),
                description: Some(schema_kind_label(&branch.schema)),
                checked: branch.id == active,
                value: PluginConfigSelectionValue::Branch(branch),
            })
            .collect::<Vec<_>>();
        self.open_named_selection_overlay(
            dialog,
            title,
            prompt,
            "Enter apply  Esc cancel  Up/Down move".to_owned(),
            false,
            items,
            PluginConfigSelectionAction::SelectBranch { plugin_id, path },
        );
    }

    fn open_enum_selection_overlay(
        &mut self,
        dialog: &mut PluginWorkbenchOverlay,
        title: String,
        plugin_id: String,
        path: ConfigPath,
        variants: Vec<JsonValue>,
        current: JsonValue,
    ) {
        let items = variants
            .into_iter()
            .map(|variant| {
                let checked = variant == current;
                PluginConfigSelectionItem {
                    label: preview_value(&variant),
                    description: None,
                    checked,
                    value: PluginConfigSelectionValue::Json(variant),
                }
            })
            .collect::<Vec<_>>();
        self.open_named_selection_overlay(
            dialog,
            format!("Select {title}"),
            "Choose one value".to_owned(),
            "Enter apply  Esc cancel  Up/Down move".to_owned(),
            false,
            items,
            PluginConfigSelectionAction::SelectEnum { plugin_id, path },
        );
    }

    fn open_multi_enum_selection_overlay(
        &mut self,
        dialog: &mut PluginWorkbenchOverlay,
        title: String,
        plugin_id: String,
        path: ConfigPath,
        variants: Vec<JsonValue>,
        current: Vec<JsonValue>,
    ) {
        let items = variants
            .into_iter()
            .map(|variant| PluginConfigSelectionItem {
                label: preview_value(&variant),
                description: None,
                checked: current.iter().any(|item| item == &variant),
                value: PluginConfigSelectionValue::Json(variant),
            })
            .collect::<Vec<_>>();
        self.open_named_selection_overlay(
            dialog,
            format!("Select {title}"),
            "Choose one or more values".to_owned(),
            "Space toggle  Enter apply  Esc cancel  Up/Down move".to_owned(),
            true,
            items,
            PluginConfigSelectionAction::SelectMultiEnum { plugin_id, path },
        );
    }

    fn open_structured_row_drilldown(
        &mut self,
        dialog: &mut PluginWorkbenchOverlay,
        plugin_id: String,
        path: ConfigPath,
        title: String,
    ) {
        let Some(plugin) = dialog
            .plugins
            .iter()
            .find(|plugin| plugin.plugin_id == plugin_id)
        else {
            return;
        };
        let groups = build_drilldown_groups(plugin, &path, title.as_str());
        dialog.drilldown_stack.push(PluginConfigDrilldownOverlay {
            plugin_id,
            path,
            title,
            groups,
            selected_row: 0,
            selected_cell: ConfigRowCell::Value,
        });
    }

    fn delete_drilldown_selected_row(&mut self, dialog: &mut PluginWorkbenchOverlay) {
        let Some(overlay) = dialog.current_drilldown() else {
            return;
        };
        let Some(row) =
            drilldown_row_at(overlay, dialog.config_view, overlay.selected_row).cloned()
        else {
            return;
        };
        if row.primary_path.is_empty() {
            self.flash_warning("root config cannot be deleted".to_owned());
            return;
        }
        let plugin_id = overlay.plugin_id.clone();
        let (changed, blocked) = if let Some(plugin) = dialog
            .plugins
            .iter_mut()
            .find(|plugin| plugin.plugin_id == plugin_id)
        {
            let outcome = apply_reset_paths(
                &mut plugin.draft_config,
                &plugin.default_config,
                plugin.schema.as_ref(),
                &row_paths(&row).into_iter().cloned().collect::<Vec<_>>(),
            );
            if outcome.changed {
                clear_branch_drafts_for_structural_change(plugin);
                recompute_plugin_config_state(plugin);
            }
            (outcome.changed, outcome.blocked)
        } else {
            (false, Vec::new())
        };
        if changed {
            dialog.drilldown_stack =
                rebuild_drilldown_stack(dialog, dialog.drilldown_stack.as_slice());
        }
        if let Some(message) = reset_paths_warning_message(blocked.as_slice()) {
            self.flash_warning(message);
        }
    }

    fn commit_plugin_config_editor(
        &mut self,
        dialog: &mut PluginWorkbenchOverlay,
        action: PluginConfigEditAction,
        input: &str,
    ) -> UiResult<()> {
        match action {
            PluginConfigEditAction::SetScalar {
                plugin_id,
                path,
                kind,
            } => {
                let value = parse_scalar_editor_value(kind, input)?;
                self.try_set_config_value_at(dialog, plugin_id, path, value)?;
            }
            PluginConfigEditAction::SetNullableString { plugin_id, path } => {
                let trimmed = input.trim();
                let value = if trimmed.is_empty() {
                    JsonValue::Null
                } else {
                    JsonValue::String(trimmed.to_owned())
                };
                self.try_set_config_value_at(dialog, plugin_id, path, value)?;
            }
            PluginConfigEditAction::SetPairIntegers {
                plugin_id,
                left_path,
                right_path,
            } => {
                let (left_value, right_value) = parse_pair_integer_editor_values(input)?;
                let selection_plugin_id = plugin_id.clone();
                self.try_set_config_values_at(
                    dialog,
                    plugin_id,
                    vec![
                        (
                            left_path.clone(),
                            JsonValue::Number(JsonNumber::from(left_value)),
                        ),
                        (right_path, JsonValue::Number(JsonNumber::from(right_value))),
                    ],
                    left_path.clone(),
                )?;
                self.focus_config_path(dialog, selection_plugin_id.as_str(), left_path.as_slice());
            }
            PluginConfigEditAction::AddObjectField { plugin_id, path } => {
                let key = input.trim();
                if key.is_empty() {
                    return Err("field name cannot be empty".to_owned());
                }
                let Some(plugin_index) = dialog
                    .plugins
                    .iter()
                    .position(|plugin| plugin.plugin_id == plugin_id)
                else {
                    return Ok(());
                };
                let mut child_path = path.clone();
                child_path.push(PathSegment::Key(key.to_owned()));
                {
                    let plugin = &mut dialog.plugins[plugin_index];
                    if get_value_at_path(&plugin.draft_config, &path)
                        .and_then(JsonValue::as_object)
                        .is_some_and(|object| object.contains_key(key))
                    {
                        return Err(format!("field `{key}` already exists"));
                    }
                    if let Some(reason) = object_add_field_block_reason(
                        plugin.schema.as_ref(),
                        &plugin.draft_config,
                        &path,
                    ) {
                        return Err(reason);
                    }
                    let child_schema = validate_new_object_field_key(
                        plugin.schema.as_ref(),
                        &plugin.draft_config,
                        &path,
                        key,
                    )?;
                    let default = child_schema
                        .as_ref()
                        .map(|schema| {
                            default_value_for_schema(
                                schema,
                                plugin.schema.as_ref().unwrap_or(schema),
                            )
                        })
                        .unwrap_or(JsonValue::Null);
                    set_value_at_path(&mut plugin.draft_config, &child_path, default);
                    clear_branch_drafts_for_structural_change(plugin);
                    recompute_plugin_config_state(plugin);
                }
                self.focus_config_path(dialog, plugin_id.as_str(), child_path.as_slice());
                self.maybe_open_type_selector_for_selected_row(
                    dialog,
                    plugin_id.as_str(),
                    child_path.as_slice(),
                );
            }
            PluginConfigEditAction::RenameObjectField { plugin_id, path } => {
                let new_key = input.trim();
                if new_key.is_empty() {
                    return Err("field name cannot be empty".to_owned());
                }
                let Some(plugin_index) = dialog
                    .plugins
                    .iter()
                    .position(|plugin| plugin.plugin_id == plugin_id)
                else {
                    return Ok(());
                };
                let Some((parent_path, current_key)) = path_key_info(path.as_slice()) else {
                    return Err("selected row does not point to an object field".to_owned());
                };
                if new_key == current_key {
                    return Ok(());
                }
                let parent_path = parent_path.to_vec();
                let new_path = {
                    let plugin = &mut dialog.plugins[plugin_index];
                    if get_value_at_path(&plugin.draft_config, &parent_path)
                        .and_then(JsonValue::as_object)
                        .is_some_and(|object| object.contains_key(new_key))
                    {
                        return Err(format!("field `{new_key}` already exists"));
                    }
                    let child_schema = validate_new_object_field_key(
                        plugin.schema.as_ref(),
                        &plugin.draft_config,
                        &parent_path,
                        new_key,
                    )?;
                    let current_value = get_value_at_path(&plugin.draft_config, &path)
                        .cloned()
                        .unwrap_or(JsonValue::Null);
                    let mut preview_path = parent_path.clone();
                    preview_path.push(PathSegment::Key(new_key.to_owned()));
                    if let Some(schema) = child_schema.as_ref()
                        && let Some(root_schema) = plugin.schema.as_ref()
                    {
                        validate_schema_value_for_path(
                            root_schema,
                            schema,
                            &current_value,
                            &preview_path,
                            title_for_schema_or_key(schema, new_key).as_str(),
                        )?;
                    }
                    let Some(new_path) = rename_object_field_at_path(
                        &mut plugin.draft_config,
                        path.as_slice(),
                        new_key,
                    ) else {
                        return Err("failed to rename field".to_owned());
                    };
                    clear_branch_drafts_for_structural_change(plugin);
                    recompute_plugin_config_state(plugin);
                    new_path
                };
                self.focus_config_path(dialog, plugin_id.as_str(), new_path.as_slice());
            }
        }
        dialog.clamp_selection();
        Ok(())
    }

    fn commit_plugin_config_selection(
        &mut self,
        dialog: &mut PluginWorkbenchOverlay,
    ) -> UiResult<()> {
        let Some(overlay) = dialog.selection.clone() else {
            return Ok(());
        };
        let selected_item = overlay
            .items
            .get(overlay.selected_item)
            .cloned()
            .ok_or_else(|| "no selection available".to_owned())?;
        match overlay.action {
            PluginConfigSelectionAction::SelectType { plugin_id, path } => {
                let PluginConfigSelectionValue::Named(selected) = selected_item.value else {
                    return Err("invalid type selection".to_owned());
                };
                let Some(plugin) = dialog
                    .plugins
                    .iter()
                    .find(|plugin| plugin.plugin_id == plugin_id)
                else {
                    return Ok(());
                };
                let schema = plugin
                    .schema
                    .as_ref()
                    .and_then(|root| schema_for_path(root, root, &plugin.draft_config, &path));
                let value = default_value_for_type(selected.as_str(), schema.as_ref());
                self.try_set_config_value_at(dialog, plugin_id, path, value)?;
            }
            PluginConfigSelectionAction::SelectBranch { plugin_id, path } => {
                let PluginConfigSelectionValue::Branch(branch) = selected_item.value else {
                    return Err("invalid branch selection".to_owned());
                };
                let Some(plugin) = dialog
                    .plugins
                    .iter()
                    .find(|plugin| plugin.plugin_id == plugin_id)
                else {
                    return Ok(());
                };
                let all_branches = overlay
                    .items
                    .iter()
                    .filter_map(|item| match &item.value {
                        PluginConfigSelectionValue::Branch(branch) => Some(branch.clone()),
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                let current = get_value_at_path(&plugin.draft_config, &path)
                    .cloned()
                    .unwrap_or(JsonValue::Null);
                let active_key = plugin_branch_draft_key(
                    plugin.plugin_id.as_str(),
                    &path,
                    active_branch_id(all_branches.as_slice(), &current),
                );
                let target_key =
                    plugin_branch_draft_key(plugin.plugin_id.as_str(), &path, branch.id.as_str());
                let value = plugin
                    .branch_drafts
                    .get(target_key.as_str())
                    .cloned()
                    .unwrap_or_else(|| {
                        default_value_for_schema(
                            &branch.schema,
                            plugin.schema.as_ref().unwrap_or(&branch.schema),
                        )
                    });
                self.try_set_config_value_at(dialog, plugin_id.clone(), path.clone(), value)?;
                if let Some(plugin) = dialog
                    .plugins
                    .iter_mut()
                    .find(|plugin| plugin.plugin_id == plugin_id)
                {
                    plugin.branch_drafts.insert(active_key, current);
                }
            }
            PluginConfigSelectionAction::SelectEnum { plugin_id, path } => {
                let PluginConfigSelectionValue::Json(selected) = selected_item.value else {
                    return Err("invalid enum selection".to_owned());
                };
                self.try_set_config_value_at(dialog, plugin_id, path, selected)?;
            }
            PluginConfigSelectionAction::SelectMultiEnum { plugin_id, path } => {
                let selected_values = overlay
                    .items
                    .iter()
                    .filter(|item| item.checked)
                    .filter_map(|item| match &item.value {
                        PluginConfigSelectionValue::Json(value) => Some(value.clone()),
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                let Some(plugin) = dialog
                    .plugins
                    .iter()
                    .find(|plugin| plugin.plugin_id == plugin_id)
                else {
                    return Ok(());
                };
                let current = get_value_at_path(&plugin.draft_config, &path)
                    .and_then(JsonValue::as_array)
                    .cloned()
                    .unwrap_or_default();
                let values =
                    merge_multi_enum_selection(current.as_slice(), selected_values.as_slice());
                self.try_set_config_value_at(dialog, plugin_id, path, JsonValue::Array(values))?;
            }
        }
        dialog.clamp_selection();
        Ok(())
    }

    pub(super) fn render_plugin_workbench(
        &self,
        frame: &mut Frame,
        area: Rect,
        dialog: &PluginWorkbenchOverlay,
        surface: SurfaceMode,
    ) {
        let surface = render_framed_surface(
            frame,
            area,
            surface,
            &FramedSurfaceSpec {
                title: clean(format!(
                    "{} · {}",
                    dialog.title,
                    plugin_workbench_summary(dialog)
                ))
                .into(),
                target_width: 150,
                target_height: 42,
            },
        );
        match dialog.mode {
            PluginWorkbenchMode::List => render_plugin_list_page(frame, surface.inner, dialog),
            PluginWorkbenchMode::Detail => render_plugin_detail_page(frame, surface.inner, dialog),
        }
        render_plugin_workbench_editor_overlay(frame, area, surface.outer, dialog);
    }

    pub(super) fn paste_plugin_workbench(dialog: &mut PluginWorkbenchOverlay, text: &str) {
        if let Some(editor) = dialog.editor.as_mut() {
            editor.input.flush_all_pending_input();
            editor.input.insert_str(text);
            return;
        }
        if dialog.mode == PluginWorkbenchMode::List {
            dialog.query.flush_all_pending_input();
            dialog.query.insert_str(text);
            refresh_plugin_workbench_filter(dialog);
        }
    }

    pub(super) fn flush_plugin_workbench_input(dialog: &mut PluginWorkbenchOverlay, now: Instant) {
        if let Some(editor) = dialog.editor.as_mut() {
            editor.input.flush_pending_input_if_due(now);
        }
        dialog.query.flush_pending_input_if_due(now);
    }
}

fn render_plugin_list_page(frame: &mut Frame, area: Rect, dialog: &PluginWorkbenchOverlay) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(8),
            Constraint::Length(1),
        ])
        .split(area);
    let filter_line = format!(
        "Search plugins... {}        Transport: {}        Config: {}        {}/{} shown",
        if dialog.query.text().is_empty() {
            "all".to_owned()
        } else {
            clean(dialog.query.text())
        },
        dialog.transport_filter.label(),
        dialog.config_filter.label(),
        dialog.visible_plugins.len(),
        dialog.plugins.len()
    );
    frame.render_widget(
        Paragraph::new(filter_line).wrap(Wrap { trim: false }),
        rows[0],
    );

    let mut lines = Vec::new();
    lines.push(Line::from(Span::styled(
        fixed_columns(
            &[
                ("Plugin", 22),
                ("Visible Tool", 16),
                ("Version", 12),
                ("Transport", 11),
                ("Tools", 7),
                ("Commands", 10),
                ("Config", 16),
            ],
            area.width.saturating_sub(4),
        ),
        Style::default().add_modifier(Modifier::BOLD),
    )));
    if dialog.visible_plugins.is_empty() {
        lines.push(Line::from("No plugins match the current filters."));
    } else {
        for (visible_row, plugin_index) in dialog.visible_plugins.iter().enumerate() {
            let Some(plugin) = dialog.plugins.get(*plugin_index) else {
                continue;
            };
            let selected = visible_row == dialog.selected_plugin;
            let marker = if selected { ">> " } else { "   " };
            let line = format!(
                "{}{}",
                marker,
                fixed_columns(
                    &[
                        (plugin.plugin_id.as_str(), 22),
                        (plugin.visible_tool.as_str(), 16),
                        (plugin.version.as_str(), 12),
                        (transport_display(plugin.transport.as_str()), 11),
                        (plugin.tools.len().to_string().as_str(), 7),
                        (plugin.commands.len().to_string().as_str(), 10),
                        (plugin.config_status.label.as_str(), 16),
                    ],
                    area.width.saturating_sub(7),
                )
            );
            let style = if selected {
                plugin_workbench_selection_highlight_style()
            } else {
                Style::default()
            };
            lines.push(Line::from(Span::styled(clean(line), style)));
        }
    }
    render_plugin_panel(frame, rows[1], "Plugins", Text::from(lines), None);
    render_plugin_footer(
        frame,
        rows[2],
        "Type to search  t transport filter  c config filter  Enter details  r refresh  Esc close",
    );
}

fn render_plugin_detail_page(frame: &mut Frame, area: Rect, dialog: &PluginWorkbenchOverlay) {
    let Some(plugin) = dialog.selected_plugin() else {
        render_plugin_panel(
            frame,
            area,
            "Plugin",
            Text::from("No plugin selected."),
            None,
        );
        return;
    };
    if dialog.detail_tab == PluginDetailTab::Config {
        render_plugin_compact_config_page(frame, area, dialog, plugin);
        return;
    }

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(1),
        ])
        .split(area);
    render_plugin_panel(
        frame,
        rows[0],
        plugin.plugin_id.as_str(),
        plugin_header_text(plugin),
        None,
    );
    render_plugin_tabs(frame, rows[1], dialog.detail_tab);
    let body = match dialog.detail_tab {
        PluginDetailTab::Config => Text::default(),
        PluginDetailTab::Tools => plugin_tools_text(plugin),
        PluginDetailTab::Commands => plugin_commands_text(plugin),
        PluginDetailTab::Capabilities => plugin_capabilities_text(plugin),
        PluginDetailTab::Logs => plugin_logs_text(plugin),
        PluginDetailTab::Diagnostics => plugin_diagnostics_text(plugin),
    };
    render_plugin_panel(frame, rows[2], dialog.detail_tab.label(), body, None);
    render_plugin_footer(
        frame,
        rows[3],
        "Esc plugins  Tab/Down next tab  Shift+Tab/Up previous tab  PageUp/PageDown scroll  r refresh",
    );
}

fn render_plugin_compact_config_page(
    frame: &mut Frame,
    area: Rect,
    dialog: &PluginWorkbenchOverlay,
    plugin: &PluginWorkbenchPlugin,
) {
    let block = Block::default()
        .title(format!(" {} / Config ", clean(plugin.plugin_id.as_str())))
        .borders(Borders::ALL);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(10),
        ])
        .split(inner);

    frame.render_widget(
        Paragraph::new(compact_config_header_line(plugin)).wrap(Wrap { trim: false }),
        rows[0],
    );
    frame.render_widget(
        Paragraph::new(compact_config_view_line(plugin, dialog)).wrap(Wrap { trim: false }),
        rows[1],
    );
    frame.render_widget(
        Paragraph::new(compact_config_toolbar_text(dialog)).wrap(Wrap { trim: false }),
        rows[2],
    );
    frame.render_widget(
        Paragraph::new(compact_config_divider(inner.width)).wrap(Wrap { trim: false }),
        rows[3],
    );

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(20),
            Constraint::Length(1),
            Constraint::Min(28),
        ])
        .split(rows[4]);
    frame.render_widget(
        Paragraph::new(compact_config_sections_text(dialog, plugin, body[0].width))
            .wrap(Wrap { trim: false }),
        body[0],
    );
    frame.render_widget(
        Paragraph::new(compact_vertical_divider(rows[4].height)).wrap(Wrap { trim: false }),
        body[1],
    );
    frame.render_widget(
        Paragraph::new(config_editor_text(dialog, plugin)).wrap(Wrap { trim: false }),
        body[2],
    );
}

fn render_plugin_workbench_editor_overlay(
    frame: &mut Frame,
    area: Rect,
    _workbench_area: Rect,
    dialog: &PluginWorkbenchOverlay,
) {
    if dialog.show_diff
        && dialog
            .selected_plugin()
            .is_some_and(plugin_uses_compact_config_layout)
    {
        if let Some(plugin) = dialog.selected_plugin() {
            render_plugin_config_diff_overlay(frame, area, dialog, plugin);
        }
    }
    if let Some(overlay) = dialog.current_drilldown() {
        render_plugin_config_drilldown_overlay(frame, area, dialog, overlay);
    }
    if let Some(selection) = dialog.selection.as_ref() {
        render_plugin_config_selection_overlay(frame, area, selection);
        if let Some(actions) = dialog.actions.as_ref() {
            render_plugin_config_actions_overlay(frame, area, actions);
        }
        return;
    }
    let Some(editor) = dialog.editor.as_ref() else {
        if let Some(actions) = dialog.actions.as_ref() {
            render_plugin_config_actions_overlay(frame, area, actions);
        }
        return;
    };
    render_editor_dialog(
        frame,
        area,
        SurfaceMode::Overlay,
        &EditorDialogSpec {
            title: clean(editor.title.as_str()).into(),
            prompt: clean(editor.prompt.as_str()).into(),
            footer: clean(editor.footer.as_str()).into(),
            target_width: if editor.multiline { 96 } else { 78 },
            multiline: editor.multiline,
            prompt_height_bounds: (1, 5),
            footer_height_bounds: (1, 2),
        },
        &editor.input,
    );
    if let Some(actions) = dialog.actions.as_ref() {
        render_plugin_config_actions_overlay(frame, area, actions);
    }
}

fn render_plugin_config_selection_overlay(
    frame: &mut Frame,
    area: Rect,
    overlay: &PluginConfigSelectionOverlay,
) {
    let surface = render_framed_surface(
        frame,
        area,
        SurfaceMode::Overlay,
        &FramedSurfaceSpec {
            title: clean(overlay.title.clone()).into(),
            target_width: 86,
            target_height: 20,
        },
    );
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(8), Constraint::Length(1)])
        .split(surface.inner);
    let mut lines = Vec::new();
    if !overlay.prompt.is_empty() {
        lines.push(Line::from(clean(overlay.prompt.clone())));
        lines.push(Line::from(""));
    }
    for (index, item) in overlay.items.iter().enumerate() {
        let marker = if overlay.multi {
            if item.checked { "[x]" } else { "[ ]" }
        } else if item.checked {
            "(*)"
        } else {
            "( )"
        };
        let prefix = if index == overlay.selected_item {
            "> "
        } else {
            "  "
        };
        let style = if index == overlay.selected_item {
            plugin_workbench_selection_highlight_style()
        } else {
            Style::default()
        };
        lines.push(Line::from(Span::styled(
            clean(format!("{prefix}{marker} {}", item.label)),
            style,
        )));
        if let Some(description) = item.description.as_deref()
            && !description.is_empty()
        {
            lines.push(Line::from(clean(format!("    {description}"))));
        }
    }
    render_plugin_panel(
        frame,
        rows[0],
        overlay.title.as_str(),
        Text::from(lines),
        None,
    );
    render_plugin_footer(frame, rows[1], overlay.footer.as_str());
}

fn render_plugin_config_actions_overlay(
    frame: &mut Frame,
    area: Rect,
    overlay: &PluginConfigActionOverlay,
) {
    let surface = render_framed_surface(
        frame,
        area,
        SurfaceMode::Overlay,
        &FramedSurfaceSpec {
            title: clean(overlay.title.clone()).into(),
            target_width: 82,
            target_height: 16,
        },
    );
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(6), Constraint::Length(1)])
        .split(surface.inner);
    let mut lines = Vec::new();
    lines.push(Line::from(Span::styled(
        overlay.subject.clone(),
        Style::default().add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(""));
    for (index, item) in overlay.actions.iter().enumerate() {
        let prefix = if index == overlay.selected_action {
            "> "
        } else {
            "  "
        };
        let style = if index == overlay.selected_action {
            plugin_workbench_selection_highlight_style()
        } else {
            Style::default()
        };
        lines.push(Line::from(Span::styled(
            clean(format!("{prefix}{}", item.label)),
            style,
        )));
        lines.push(Line::from(clean(format!("    {}", item.description))));
    }
    render_plugin_panel(
        frame,
        rows[0],
        overlay.title.as_str(),
        Text::from(lines),
        None,
    );
    render_plugin_footer(frame, rows[1], overlay.footer.as_str());
}

fn render_plugin_config_drilldown_overlay(
    frame: &mut Frame,
    area: Rect,
    dialog: &PluginWorkbenchOverlay,
    overlay: &PluginConfigDrilldownOverlay,
) {
    let surface = render_framed_surface(
        frame,
        area,
        SurfaceMode::Overlay,
        &FramedSurfaceSpec {
            title: clean(format!(
                "{} · {}",
                overlay.title,
                path_display(&overlay.path)
            ))
            .into(),
            target_width: 108,
            target_height: 30,
        },
    );
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(8), Constraint::Length(1)])
        .split(surface.inner);
    let mut lines = Vec::new();
    lines.push(Line::from(Span::styled(
        overlay.title.clone(),
        Style::default().add_modifier(Modifier::BOLD),
    )));
    for group in &overlay.groups {
        if !group
            .rows
            .iter()
            .any(|row| row_visible(row, dialog.config_view))
        {
            continue;
        }
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            group.title.clone(),
            Style::default().add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(Span::styled(
            if group_has_action_column(group, dialog.config_view) {
                standard_config_row_line_with_action(
                    "Setting",
                    "Type",
                    "Value",
                    "Default",
                    "Action",
                    "State",
                    rows[0].width.saturating_sub(4),
                )
            } else {
                standard_config_row_line(
                    "Setting",
                    "Type",
                    "Value",
                    "Default",
                    "State",
                    rows[0].width.saturating_sub(4),
                )
            },
            Style::default().add_modifier(Modifier::BOLD),
        )));
        let mut visible_index = 0usize;
        for drill_group in &overlay.groups {
            for row in &drill_group.rows {
                if !row_visible(row, dialog.config_view) {
                    continue;
                }
                if std::ptr::eq(drill_group, group) {
                    let include_action = group_has_action_column(group, dialog.config_view);
                    let focused_cell = if visible_index == overlay.selected_row {
                        Some(drilldown_selected_row_cell(
                            overlay,
                            dialog.config_view,
                            overlay.selected_cell,
                        ))
                    } else {
                        None
                    };
                    lines.push(standard_config_row_line_with_focus(
                        row,
                        rows[0].width.saturating_sub(4),
                        focused_cell,
                        include_action,
                    ));
                }
                visible_index += 1;
            }
        }
    }
    if lines.len() == 1 {
        lines.push(Line::from(""));
        lines.push(Line::from("No editable rows."));
    }
    render_plugin_panel(
        frame,
        rows[0],
        overlay.title.as_str(),
        Text::from(lines),
        None,
    );
    let footer = drilldown_footer_text(dialog, overlay);
    render_plugin_footer(frame, rows[1], footer.as_str());
}

fn render_plugin_config_diff_overlay(
    frame: &mut Frame,
    area: Rect,
    dialog: &PluginWorkbenchOverlay,
    plugin: &PluginWorkbenchPlugin,
) {
    let surface = render_framed_surface(
        frame,
        area,
        SurfaceMode::Overlay,
        &FramedSurfaceSpec {
            title: clean("Config Diff").into(),
            target_width: 112,
            target_height: 18,
        },
    );
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(6), Constraint::Length(1)])
        .split(surface.inner);
    render_plugin_panel(
        frame,
        rows[0],
        "Config Diff",
        config_diff_text(dialog, plugin),
        None,
    );
    render_plugin_footer(frame, rows[1], "D/Esc close");
}

fn render_plugin_panel(
    frame: &mut Frame,
    area: Rect,
    title: impl Into<String>,
    body: Text<'static>,
    scroll: Option<(u16, u16)>,
) {
    let block = Block::default()
        .title(format!(" {} ", clean(title.into())))
        .borders(Borders::ALL);
    let paragraph = Paragraph::new(body)
        .block(block)
        .wrap(Wrap { trim: false })
        .scroll(scroll.unwrap_or((0, 0)));
    frame.render_widget(paragraph, area);
}

fn render_plugin_footer(frame: &mut Frame, area: Rect, text: &str) {
    frame.render_widget(Paragraph::new(clean(text)).wrap(Wrap { trim: false }), area);
}

fn render_plugin_tabs(frame: &mut Frame, area: Rect, selected: PluginDetailTab) {
    let mut spans = Vec::new();
    for (index, tab) in PluginDetailTab::ALL.iter().copied().enumerate() {
        if index > 0 {
            spans.push(Span::raw(" | "));
        }
        let style = if tab == selected {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD | Modifier::REVERSED)
        } else {
            Style::default()
        };
        spans.push(Span::styled(format!(" {} ", tab.label()), style));
    }
    render_plugin_panel(frame, area, "Tabs", Text::from(Line::from(spans)), None);
}

fn transport_display(transport: &str) -> &str {
    match transport {
        "static" => "native",
        other => other,
    }
}

fn plugin_uses_compact_config_layout(plugin: &PluginWorkbenchPlugin) -> bool {
    let _ = plugin;
    true
}

fn compact_plugin_label(plugin: &PluginWorkbenchPlugin) -> String {
    plugin
        .plugin_id
        .rsplit('.')
        .next()
        .unwrap_or(plugin.plugin_id.as_str())
        .to_owned()
}

fn compact_config_header_line(plugin: &PluginWorkbenchPlugin) -> String {
    let save_state = if plugin.dirty { "Dirty" } else { "Saved" };
    let config_label = format!("Config: {} / {}", plugin.config_status.label, save_state);
    let label = compact_plugin_label(plugin);
    let version_label = format!("v{}", plugin.version);
    fixed_columns(
        &[
            (label.as_str(), 24),
            (version_label.as_str(), 14),
            (transport_display(plugin.transport.as_str()), 14),
            (config_label.as_str(), 30),
        ],
        112,
    )
}

fn compact_config_view_line(
    plugin: &PluginWorkbenchPlugin,
    dialog: &PluginWorkbenchOverlay,
) -> String {
    let (cell_label, enter_hint) = selected_config_row_context(dialog)
        .map(|context| {
            (
                config_row_cell_label(&context.row, context.layout, context.cell).to_owned(),
                config_row_cell_enter_hint(&context.row, context.layout, context.cell),
            )
        })
        .unwrap_or_else(|| ("Value".to_owned(), "edit value".to_owned()));
    format!(
        "Changed: {}  Cell: {}  Left/Right move  Enter {}  T type/shape  A add  X actions",
        override_leaf_count(&plugin.draft_override),
        cell_label,
        enter_hint,
    )
}

fn drilldown_footer_text(
    dialog: &PluginWorkbenchOverlay,
    overlay: &PluginConfigDrilldownOverlay,
) -> String {
    let enter_hint = drilldown_row_at(overlay, dialog.config_view, overlay.selected_row)
        .and_then(|row| {
            drilldown_group_for_row(overlay, dialog.config_view, overlay.selected_row).map(
                |group| {
                    config_row_cell_enter_hint(
                        row,
                        group.layout,
                        drilldown_selected_row_cell(
                            overlay,
                            dialog.config_view,
                            overlay.selected_cell,
                        ),
                    )
                },
            )
        })
        .unwrap_or_else(|| "edit value".to_owned());
    format!(
        "Esc back  Left/Right cell  Enter {}  T type/shape  A add  X actions  Ctrl+d reset row  Up/Down move",
        enter_hint
    )
}

fn compact_config_toolbar_text(dialog: &PluginWorkbenchOverlay) -> Text<'static> {
    let mut spans = Vec::new();
    for (index, action) in COMPACT_TOOLBAR_ACTIONS.iter().copied().enumerate() {
        if index > 0 {
            spans.push(Span::raw(" "));
        }
        let label = format!("[ {} ({}) ]", action.label(), action.shortcut());
        let style = if dialog.config_focus == PluginConfigFocus::Toolbar
            && dialog.selected_toolbar_action == index
        {
            plugin_workbench_selection_highlight_style()
        } else {
            Style::default()
        };
        spans.push(Span::styled(label, style));
    }
    Text::from(Line::from(spans))
}

fn compact_config_divider(width: u16) -> String {
    "─".repeat(width as usize)
}

fn compact_vertical_divider(height: u16) -> Text<'static> {
    Text::from((0..height).map(|_| Line::from("│")).collect::<Vec<_>>())
}

fn compact_config_sections_text(
    dialog: &PluginWorkbenchOverlay,
    plugin: &PluginWorkbenchPlugin,
    width: u16,
) -> Text<'static> {
    let mut lines = vec![Line::from(Span::styled(
        "Sections",
        Style::default().add_modifier(Modifier::BOLD),
    ))];
    lines.push(Line::from(""));
    let content_width = width.saturating_sub(1).max(6) as usize;
    for (index, section) in plugin.sections.iter().enumerate() {
        let mut label = section.title.clone();
        if section.issue_count > 0 {
            label.push_str(format!(" !{}", section.issue_count).as_str());
        } else if section.dirty {
            label.push_str(" dirty");
        }
        let focused =
            dialog.config_focus == PluginConfigFocus::Structure && index == dialog.selected_section;
        let prefixes = if focused { ("> ", "  ") } else { ("  ", "  ") };
        let wrapped = wrap_prefixed_text(label.as_str(), prefixes.0, prefixes.1, content_width);
        for line in wrapped {
            let padded = pad_to_width(line.as_str(), content_width);
            let style = if focused {
                plugin_workbench_selection_highlight_style()
            } else {
                Style::default()
            };
            lines.push(Line::from(Span::styled(padded, style)));
        }
    }
    Text::from(lines)
}

fn build_plugin_workbench_plugin(
    sources: &crate::backend::ConfigJsonSources,
    locale: &str,
    status: agena::plugin::status::PluginStatus,
    inspect: Option<agena::plugin::PluginInspect>,
    logs: Vec<agena::plugin::PluginLogRecord>,
) -> PluginWorkbenchPlugin {
    let manifest = inspect
        .as_ref()
        .and_then(|inspect| inspect.manifest.as_ref());
    let tools = manifest
        .map(|manifest| manifest.tools.clone())
        .unwrap_or_default();
    let commands = manifest
        .map(|manifest| manifest.commands.clone())
        .unwrap_or_default();
    let version = manifest
        .map(|manifest| manifest.version.clone())
        .unwrap_or_else(|| "n/a".to_owned());
    let visible_tool = tools
        .first()
        .map(|tool| tool.name.clone())
        .unwrap_or_else(|| {
            status
                .plugin_id
                .rsplit('.')
                .next()
                .unwrap_or(status.plugin_id.as_str())
                .to_owned()
        });
    let configured_plugin_value = inspect
        .as_ref()
        .and_then(|inspect| inspect.configured_plugin.as_ref())
        .and_then(|configured_plugin| serde_json::to_value(configured_plugin).ok())
        .or_else(|| {
            get_json_path(
                &sources.effective,
                Some(
                    format!(
                        "plugins.list.{}",
                        quote_settings_segment(status.plugin_id.as_str())
                    )
                    .as_str(),
                ),
            )
            .ok()
            .filter(|value| !value.is_null())
        })
        .filter(|value| !value.is_null());
    let raw_config = configured_plugin_value
        .as_ref()
        .and_then(|configured_plugin| configured_plugin.get("config"))
        .cloned()
        .unwrap_or(JsonValue::Null);
    let schema = manifest.and_then(|manifest| localized_config_schema(manifest, locale));
    let schema_missing = schema.is_none();
    let default_config = materialized_config_value(schema.as_ref(), &JsonValue::Null);
    let saved_config = materialized_config_value(schema.as_ref(), &raw_config);
    let saved_override = derive_override_value(&default_config, &saved_config);
    let mut plugin = PluginWorkbenchPlugin {
        plugin_id: status.plugin_id.clone(),
        visible_tool,
        version,
        transport: status.kind.to_owned(),
        tools,
        commands,
        config_status: PluginConfigStatus {
            kind: PluginConfigStatusKind::Valid,
            label: "Valid".to_owned(),
        },
        status,
        inspect,
        configured_plugin_value,
        saved_override: saved_override.clone(),
        draft_override: saved_override,
        default_config,
        saved_config: saved_config.clone(),
        draft_config: saved_config,
        schema,
        schema_missing,
        diagnostics: Vec::new(),
        runtime_diagnostics: Vec::new(),
        diff: Vec::new(),
        sections: Vec::new(),
        logs,
        dirty: false,
        branch_drafts: BTreeMap::new(),
    };
    recompute_plugin_config_state(&mut plugin);
    plugin
}

fn recompute_plugin_config_state(plugin: &mut PluginWorkbenchPlugin) {
    plugin.draft_override = derive_override_value(&plugin.default_config, &plugin.draft_config);
    plugin.dirty = normalize_override_value(plugin.draft_override.clone())
        != normalize_override_value(plugin.saved_override.clone());
    plugin.diagnostics = validate_config_value(
        plugin.schema.as_ref(),
        &plugin.draft_config,
        plugin.schema_missing,
    );
    plugin
        .diagnostics
        .extend(plugin_semantic_diagnostics(plugin));
    plugin.runtime_diagnostics = runtime_diagnostics(&plugin.status);
    plugin.diff = diff_config_values(&plugin.saved_config, &plugin.draft_config);
    plugin.sections = build_config_sections(plugin);
    plugin.config_status = config_status_for_plugin(plugin);
}

fn plugin_config_error_count(plugin: &PluginWorkbenchPlugin) -> usize {
    plugin
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error)
        .count()
}

fn plugin_save_block_reason(plugin: &PluginWorkbenchPlugin) -> Option<String> {
    let errors = plugin_config_error_count(plugin);
    (errors > 0).then(|| {
        format!(
            "cannot save {} config with {errors} error(s)",
            plugin.plugin_id
        )
    })
}

fn config_status_for_plugin(plugin: &PluginWorkbenchPlugin) -> PluginConfigStatus {
    if !plugin.runtime_diagnostics.is_empty() {
        return PluginConfigStatus {
            kind: PluginConfigStatusKind::RuntimeIssue,
            label: format!("Runtime issue {}", plugin.runtime_diagnostics.len()),
        };
    }
    if plugin.dirty {
        return PluginConfigStatus {
            kind: PluginConfigStatusKind::NeedsRestart,
            label: "Needs restart".to_owned(),
        };
    }
    let errors = plugin_config_error_count(plugin);
    if errors > 0 {
        return PluginConfigStatus {
            kind: PluginConfigStatusKind::Invalid,
            label: format!("Invalid {errors}"),
        };
    }
    let warnings = plugin
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == DiagnosticSeverity::Warning)
        .count();
    if warnings > 0 {
        return PluginConfigStatus {
            kind: if plugin.schema_missing {
                PluginConfigStatusKind::SchemaMissing
            } else {
                PluginConfigStatusKind::Warning
            },
            label: if plugin.schema_missing {
                "Schema missing".to_owned()
            } else {
                format!("Warning {warnings}")
            },
        };
    }
    if plugin.configured_plugin_value.is_none() {
        return PluginConfigStatus {
            kind: PluginConfigStatusKind::Missing,
            label: "Missing".to_owned(),
        };
    }
    PluginConfigStatus {
        kind: PluginConfigStatusKind::Valid,
        label: "Valid".to_owned(),
    }
}

fn normalize_override_value(value: JsonValue) -> JsonValue {
    match value {
        JsonValue::Object(object) => JsonValue::Object(
            object
                .into_iter()
                .map(|(key, value)| (key, normalize_override_value(value)))
                .collect(),
        ),
        JsonValue::Array(items) => JsonValue::Array(
            items
                .into_iter()
                .map(normalize_override_value)
                .collect::<Vec<_>>(),
        ),
        other => other,
    }
}

fn persisted_plugin_config_value(plugin: &PluginWorkbenchPlugin) -> JsonValue {
    normalize_override_value(plugin.draft_override.clone())
}

fn derive_override_value(default: &JsonValue, effective: &JsonValue) -> JsonValue {
    derive_override_option(default, effective).unwrap_or(JsonValue::Null)
}

fn derive_override_option(default: &JsonValue, effective: &JsonValue) -> Option<JsonValue> {
    if default == effective {
        return None;
    }
    match (default, effective) {
        (JsonValue::Object(default), JsonValue::Object(effective)) => {
            let mut patch = JsonMap::new();
            let keys = default
                .keys()
                .chain(effective.keys())
                .cloned()
                .collect::<BTreeSet<_>>();
            for key in keys {
                let child_default = default.get(key.as_str()).unwrap_or(&JsonValue::Null);
                let child_effective = effective.get(key.as_str()).unwrap_or(&JsonValue::Null);
                if let Some(child_patch) = derive_override_option(child_default, child_effective) {
                    patch.insert(key, child_patch);
                }
            }
            (!patch.is_empty()).then_some(JsonValue::Object(patch))
        }
        (JsonValue::Array(default), JsonValue::Array(effective)) => {
            (default != effective).then_some(JsonValue::Array(effective.clone()))
        }
        _ => Some(effective.clone()),
    }
}

fn row_paths(row: &ConfigRowView) -> Vec<&ConfigPath> {
    std::iter::once(&row.primary_path)
        .chain(row.additional_paths.iter())
        .collect()
}

fn reset_effective_value_at_path(
    value: &mut JsonValue,
    default_root: &JsonValue,
    path: &[PathSegment],
) -> bool {
    let path = path.to_vec();
    let before = get_value_at_path(value, &path)
        .cloned()
        .unwrap_or(JsonValue::Null);
    if let Some(default_value) = get_value_at_path(default_root, &path).cloned() {
        set_value_at_path(value, &path, default_value);
    } else if remove_value_at_path(value, &path).is_none() {
        return false;
    }
    get_value_at_path(value, &path)
        .cloned()
        .unwrap_or(JsonValue::Null)
        != before
}

fn path_present_in_value(value: &JsonValue, path: &[PathSegment]) -> bool {
    let mut cursor = value;
    for segment in path {
        match segment {
            PathSegment::Key(key) => {
                let Some(next) = cursor.as_object().and_then(|object| object.get(key)) else {
                    return false;
                };
                cursor = next;
            }
            PathSegment::Index(index) => {
                let Some(next) = cursor.as_array().and_then(|items| items.get(*index)) else {
                    return false;
                };
                cursor = next;
            }
        }
    }
    true
}

fn path_is_prefix_of(prefix: &[PathSegment], path: &[PathSegment]) -> bool {
    prefix.len() <= path.len()
        && prefix
            .iter()
            .zip(path.iter())
            .all(|(left, right)| left == right)
}

fn section_row_count(section: &ConfigSectionView, view: PluginConfigView) -> usize {
    match &section.body {
        ConfigSectionBody::Overview { .. } => 0,
        ConfigSectionBody::Form { groups, .. } => groups
            .iter()
            .map(|group| {
                group
                    .rows
                    .iter()
                    .filter(|row| row_visible(row, view))
                    .count()
            })
            .sum(),
    }
}

fn section_row_at(
    section: &ConfigSectionView,
    view: PluginConfigView,
    index: usize,
) -> Option<&ConfigRowView> {
    let ConfigSectionBody::Form { groups, .. } = &section.body else {
        return None;
    };
    let mut visible_index = 0usize;
    for group in groups {
        for row in &group.rows {
            if !row_visible(row, view) {
                continue;
            }
            if visible_index == index {
                return Some(row);
            }
            visible_index += 1;
        }
    }
    None
}

fn find_row_position(
    plugin: &PluginWorkbenchPlugin,
    view: PluginConfigView,
    path: &[PathSegment],
) -> Option<(usize, usize)> {
    for (section_index, section) in plugin.sections.iter().enumerate() {
        let mut row_index = 0usize;
        let ConfigSectionBody::Form { groups, .. } = &section.body else {
            continue;
        };
        for group in groups {
            for row in &group.rows {
                if !row_visible(row, view) {
                    continue;
                }
                if row.primary_path.as_slice() == path
                    || row
                        .additional_paths
                        .iter()
                        .any(|candidate| candidate.as_slice() == path)
                {
                    return Some((section_index, row_index));
                }
                row_index += 1;
            }
        }
    }
    None
}

fn find_best_section_row_for_path(
    plugin: &PluginWorkbenchPlugin,
    view: PluginConfigView,
    target_path: &[PathSegment],
) -> Option<(usize, usize, ConfigRowView)> {
    let mut best: Option<(usize, usize, usize, ConfigRowView)> = None;
    for (section_index, section) in plugin.sections.iter().enumerate() {
        let mut row_index = 0usize;
        let ConfigSectionBody::Form { groups, .. } = &section.body else {
            continue;
        };
        for group in groups {
            for row in &group.rows {
                if !row_visible(row, view) {
                    continue;
                }
                if let Some(prefix_len) = row_best_path_prefix_len(row, target_path) {
                    let replace = best
                        .as_ref()
                        .is_none_or(|(_, _, current_len, _)| prefix_len > *current_len);
                    if replace {
                        best = Some((section_index, row_index, prefix_len, row.clone()));
                    }
                }
                row_index += 1;
            }
        }
    }
    best.map(|(section_index, row_index, _, row)| (section_index, row_index, row))
}

fn find_best_drilldown_row_for_path(
    overlay: &PluginConfigDrilldownOverlay,
    view: PluginConfigView,
    target_path: &[PathSegment],
) -> Option<(usize, ConfigRowView)> {
    let mut best: Option<(usize, usize, ConfigRowView)> = None;
    let mut visible_index = 0usize;
    for group in &overlay.groups {
        for row in &group.rows {
            if !row_visible(row, view) {
                continue;
            }
            if let Some(prefix_len) = row_best_path_prefix_len(row, target_path) {
                let replace = best
                    .as_ref()
                    .is_none_or(|(_, current_len, _)| prefix_len > *current_len);
                if replace {
                    best = Some((visible_index, prefix_len, row.clone()));
                }
            }
            visible_index += 1;
        }
    }
    best.map(|(row_index, _, row)| (row_index, row))
}

fn row_best_path_prefix_len(row: &ConfigRowView, target_path: &[PathSegment]) -> Option<usize> {
    std::iter::once(&row.primary_path)
        .chain(row.additional_paths.iter())
        .filter(|candidate| path_is_prefix_of(candidate.as_slice(), target_path))
        .map(|candidate| candidate.len())
        .max()
}

fn move_selected_config_section(dialog: &mut PluginWorkbenchOverlay, delta: isize) {
    let item_count = dialog
        .selected_plugin()
        .map(|plugin| plugin.sections.len())
        .unwrap_or_default();
    move_index(&mut dialog.selected_section, item_count, delta);
    dialog.selected_node = 0;
    dialog.clamp_selection();
}

fn move_selected_bottom_panel_row(dialog: &mut PluginWorkbenchOverlay, delta: isize) {
    let item_count = if dialog.show_diff {
        dialog
            .selected_plugin()
            .map(|plugin| plugin.diff.len())
            .unwrap_or_default()
    } else {
        dialog
            .selected_plugin()
            .map(plugin_all_diagnostics)
            .map(|diagnostics| diagnostics.len())
            .unwrap_or_default()
    };
    if dialog.show_diff {
        move_index(&mut dialog.selected_diff_row, item_count, delta);
    } else {
        move_index(&mut dialog.selected_diagnostic, item_count, delta);
    }
    dialog.clamp_selection();
}

fn select_config_path(dialog: &mut PluginWorkbenchOverlay, plugin_id: &str, path: &[PathSegment]) {
    let Some(plugin) = dialog
        .plugins
        .iter()
        .find(|plugin| plugin.plugin_id == plugin_id)
    else {
        return;
    };
    if let Some((section_index, row_index)) = find_row_position(plugin, dialog.config_view, path) {
        dialog.selected_section = section_index;
        dialog.selected_node = row_index;
    }
    dialog.clamp_selection();
}

fn row_visible(_row: &ConfigRowView, _view: PluginConfigView) -> bool {
    true
}

fn row_rename_action_allowed(plugin: &PluginWorkbenchPlugin, path: &[PathSegment]) -> bool {
    let Some((parent_path, key)) = path_key_info(path) else {
        return false;
    };
    let Some(root_schema) = plugin.schema.as_ref() else {
        return true;
    };
    let Some(parent_schema) =
        schema_for_path(root_schema, root_schema, &plugin.draft_config, &parent_path)
    else {
        return true;
    };
    !schema_declared_property_keys(&parent_schema).contains(&key)
}

fn array_item_primary_action(info: ArrayItemActionInfo) -> Option<ConfigRowPrimaryAction> {
    if info.can_insert_after {
        Some(ConfigRowPrimaryAction::InsertAfter)
    } else if info.can_duplicate {
        Some(ConfigRowPrimaryAction::Duplicate)
    } else if info.can_move_down {
        Some(ConfigRowPrimaryAction::MoveDown)
    } else if info.can_move_up {
        Some(ConfigRowPrimaryAction::MoveUp)
    } else if info.can_remove {
        Some(ConfigRowPrimaryAction::Remove)
    } else {
        None
    }
}

fn config_row_primary_action(
    plugin: &PluginWorkbenchPlugin,
    editor: &ConfigRowEditor,
    primary_path: &[PathSegment],
    additional_paths: &[ConfigPath],
) -> Option<ConfigRowPrimaryAction> {
    if let Some(info) = array_item_action_info(plugin, primary_path)
        && info.has_any_action()
    {
        return array_item_primary_action(info);
    }
    if let ConfigRowEditor::Structured { path } = editor
        && let Some(value) = get_value_at_path(&plugin.draft_config, path)
    {
        match value {
            JsonValue::Object(_) => {
                if object_add_field_block_reason(plugin.schema.as_ref(), &plugin.draft_config, path)
                    .is_none()
                {
                    return Some(ConfigRowPrimaryAction::AddField);
                }
            }
            JsonValue::Array(_) => {
                if can_append_array_item(plugin, path.as_slice()) {
                    return Some(ConfigRowPrimaryAction::AddItem);
                }
            }
            _ => {}
        }
    }
    (additional_paths.is_empty() && row_rename_action_allowed(plugin, primary_path))
        .then_some(ConfigRowPrimaryAction::Rename)
}

fn config_row_action_display(
    plugin: &PluginWorkbenchPlugin,
    editor: &ConfigRowEditor,
    primary_path: &[PathSegment],
    additional_paths: &[ConfigPath],
) -> Option<String> {
    config_row_primary_action(plugin, editor, primary_path, additional_paths)
        .map(ConfigRowPrimaryAction::label)
        .map(str::to_owned)
}

fn config_row_cell_fallback_order(
    layout: ConfigGroupLayout,
    preferred: ConfigRowCell,
) -> &'static [ConfigRowCell] {
    match layout {
        ConfigGroupLayout::Standard => match preferred {
            ConfigRowCell::Type => &[
                ConfigRowCell::Type,
                ConfigRowCell::Value,
                ConfigRowCell::Default,
                ConfigRowCell::Action,
                ConfigRowCell::State,
            ],
            ConfigRowCell::Value => &[
                ConfigRowCell::Value,
                ConfigRowCell::Default,
                ConfigRowCell::Action,
                ConfigRowCell::State,
                ConfigRowCell::Type,
            ],
            ConfigRowCell::SecondaryValue => &[
                ConfigRowCell::Value,
                ConfigRowCell::Default,
                ConfigRowCell::Action,
                ConfigRowCell::State,
                ConfigRowCell::Type,
            ],
            ConfigRowCell::Default => &[
                ConfigRowCell::Default,
                ConfigRowCell::Action,
                ConfigRowCell::State,
                ConfigRowCell::Value,
                ConfigRowCell::Type,
            ],
            ConfigRowCell::Action => &[
                ConfigRowCell::Action,
                ConfigRowCell::State,
                ConfigRowCell::Default,
                ConfigRowCell::Value,
                ConfigRowCell::Type,
            ],
            ConfigRowCell::State => &[
                ConfigRowCell::State,
                ConfigRowCell::Action,
                ConfigRowCell::Default,
                ConfigRowCell::Value,
                ConfigRowCell::Type,
            ],
        },
        ConfigGroupLayout::Pair { .. } => match preferred {
            ConfigRowCell::Type | ConfigRowCell::Default | ConfigRowCell::Value => &[
                ConfigRowCell::Value,
                ConfigRowCell::SecondaryValue,
                ConfigRowCell::Action,
                ConfigRowCell::State,
            ],
            ConfigRowCell::SecondaryValue => &[
                ConfigRowCell::SecondaryValue,
                ConfigRowCell::Value,
                ConfigRowCell::Action,
                ConfigRowCell::State,
            ],
            ConfigRowCell::Action => &[
                ConfigRowCell::Action,
                ConfigRowCell::State,
                ConfigRowCell::SecondaryValue,
                ConfigRowCell::Value,
            ],
            ConfigRowCell::State => &[
                ConfigRowCell::State,
                ConfigRowCell::Action,
                ConfigRowCell::SecondaryValue,
                ConfigRowCell::Value,
            ],
        },
    }
}

fn row_cells(row: &ConfigRowView, layout: ConfigGroupLayout) -> Vec<ConfigRowCell> {
    match layout {
        ConfigGroupLayout::Standard => {
            let mut cells = Vec::new();
            if row.type_mode.is_switchable() {
                cells.push(ConfigRowCell::Type);
            }
            cells.push(ConfigRowCell::Value);
            cells.push(ConfigRowCell::Default);
            if row.action_display.is_some() {
                cells.push(ConfigRowCell::Action);
            }
            cells.push(ConfigRowCell::State);
            cells
        }
        ConfigGroupLayout::Pair { .. } => {
            let mut cells = vec![ConfigRowCell::Value, ConfigRowCell::SecondaryValue];
            if row.action_display.is_some() {
                cells.push(ConfigRowCell::Action);
            }
            cells.push(ConfigRowCell::State);
            cells
        }
    }
}

fn normalize_config_row_cell(
    row: &ConfigRowView,
    layout: ConfigGroupLayout,
    preferred: ConfigRowCell,
) -> ConfigRowCell {
    let cells = row_cells(row, layout);
    config_row_cell_fallback_order(layout, preferred)
        .iter()
        .copied()
        .find(|candidate| cells.contains(candidate))
        .or_else(|| cells.first().copied())
        .unwrap_or(ConfigRowCell::Value)
}

fn move_config_row_cell(
    row: &ConfigRowView,
    layout: ConfigGroupLayout,
    current: ConfigRowCell,
    delta: isize,
) -> Option<ConfigRowCell> {
    let cells = row_cells(row, layout);
    let current = normalize_config_row_cell(row, layout, current);
    let index = cells
        .iter()
        .position(|cell| *cell == current)
        .unwrap_or_default();
    let next = (index as isize + delta).clamp(0, cells.len().saturating_sub(1) as isize) as usize;
    (next != index).then(|| cells[next])
}

fn config_row_cell_label(
    row: &ConfigRowView,
    layout: ConfigGroupLayout,
    cell: ConfigRowCell,
) -> &'static str {
    match (layout, normalize_config_row_cell(row, layout, cell)) {
        (ConfigGroupLayout::Standard, ConfigRowCell::Type) => "Type",
        (ConfigGroupLayout::Standard, ConfigRowCell::Value) => "Value",
        (ConfigGroupLayout::Standard, ConfigRowCell::Default) => "Default",
        (ConfigGroupLayout::Standard, ConfigRowCell::Action) => "Action",
        (ConfigGroupLayout::Standard, ConfigRowCell::State) => "State",
        (ConfigGroupLayout::Standard, _) => "Value",
        (ConfigGroupLayout::Pair { left_label, .. }, ConfigRowCell::Value) => left_label,
        (ConfigGroupLayout::Pair { right_label, .. }, ConfigRowCell::SecondaryValue) => right_label,
        (ConfigGroupLayout::Pair { .. }, ConfigRowCell::Action) => "Action",
        (ConfigGroupLayout::Pair { .. }, ConfigRowCell::State) => "State",
        (ConfigGroupLayout::Pair { left_label, .. }, _) => left_label,
    }
}

fn config_row_cell_enter_hint(
    row: &ConfigRowView,
    layout: ConfigGroupLayout,
    cell: ConfigRowCell,
) -> String {
    match normalize_config_row_cell(row, layout, cell) {
        ConfigRowCell::Type => {
            if row.type_mode == ConfigRowTypeMode::SelectShape {
                "choose shape".to_owned()
            } else {
                "choose type".to_owned()
            }
        }
        ConfigRowCell::Default => "reset field".to_owned(),
        ConfigRowCell::Action => row
            .action_display
            .as_deref()
            .map(trim_action_display)
            .map(str::to_ascii_lowercase)
            .unwrap_or_else(|| "open actions".to_owned()),
        ConfigRowCell::State => "open actions".to_owned(),
        ConfigRowCell::SecondaryValue => format!(
            "edit {}",
            config_row_cell_label(row, layout, ConfigRowCell::SecondaryValue).to_ascii_lowercase()
        ),
        ConfigRowCell::Value => match &row.editor {
            ConfigRowEditor::Bool { .. } => "toggle value".to_owned(),
            ConfigRowEditor::ReadOnly { .. } => "view value".to_owned(),
            ConfigRowEditor::Null { .. } if row.type_mode.is_switchable() => {
                if row.type_mode == ConfigRowTypeMode::SelectShape {
                    "choose shape".to_owned()
                } else {
                    "choose type".to_owned()
                }
            }
            ConfigRowEditor::Structured { .. } => "configure".to_owned(),
            ConfigRowEditor::Enum { .. } | ConfigRowEditor::MultiEnum { .. } => {
                "select value".to_owned()
            }
            ConfigRowEditor::PairInteger { .. } => format!(
                "edit {}",
                config_row_cell_label(row, layout, ConfigRowCell::Value).to_ascii_lowercase()
            ),
            _ => "edit value".to_owned(),
        },
    }
}

fn group_has_action_column(group: &ConfigGroupView, view: PluginConfigView) -> bool {
    group
        .rows
        .iter()
        .filter(|row| row_visible(row, view))
        .any(|row| row.action_display.is_some())
}

fn section_selected_row_cell(
    section: &ConfigSectionView,
    view: PluginConfigView,
    index: usize,
    preferred: ConfigRowCell,
) -> ConfigRowCell {
    let Some(row) = section_row_at(section, view, index) else {
        return ConfigRowCell::Value;
    };
    let layout = section_group_for_row(section, view, index)
        .map(|group| group.layout)
        .unwrap_or(ConfigGroupLayout::Standard);
    normalize_config_row_cell(row, layout, preferred)
}

fn drilldown_selected_row_cell(
    overlay: &PluginConfigDrilldownOverlay,
    view: PluginConfigView,
    preferred: ConfigRowCell,
) -> ConfigRowCell {
    let Some(row) = drilldown_row_at(overlay, view, overlay.selected_row) else {
        return ConfigRowCell::Value;
    };
    let layout = drilldown_group_for_row(overlay, view, overlay.selected_row)
        .map(|group| group.layout)
        .unwrap_or(ConfigGroupLayout::Standard);
    normalize_config_row_cell(row, layout, preferred)
}

#[derive(Debug, Clone)]
struct SelectedConfigRowContext {
    plugin_id: String,
    row: ConfigRowView,
    layout: ConfigGroupLayout,
    cell: ConfigRowCell,
    group_title: String,
    group_paths: Vec<ConfigPath>,
}

fn selected_config_row_context(
    dialog: &PluginWorkbenchOverlay,
) -> Option<SelectedConfigRowContext> {
    if let Some(overlay) = dialog.current_drilldown() {
        let row = drilldown_row_at(overlay, dialog.config_view, overlay.selected_row)?.clone();
        let group = drilldown_group_for_row(overlay, dialog.config_view, overlay.selected_row)?;
        return Some(SelectedConfigRowContext {
            plugin_id: overlay.plugin_id.clone(),
            row,
            layout: group.layout,
            cell: drilldown_selected_row_cell(overlay, dialog.config_view, overlay.selected_cell),
            group_title: group.title.clone(),
            group_paths: group_row_paths(group),
        });
    }
    let plugin = dialog.selected_plugin()?;
    let section = dialog.selected_section()?;
    let row = section_row_at(section, dialog.config_view, dialog.selected_node)?.clone();
    let group = section_group_for_row(section, dialog.config_view, dialog.selected_node)?;
    Some(SelectedConfigRowContext {
        plugin_id: plugin.plugin_id.clone(),
        row,
        layout: group.layout,
        cell: section_selected_row_cell(
            section,
            dialog.config_view,
            dialog.selected_node,
            dialog.selected_cell,
        ),
        group_title: group.title.clone(),
        group_paths: group_row_paths(group),
    })
}

fn group_row_paths(group: &ConfigGroupView) -> Vec<ConfigPath> {
    let mut paths = Vec::new();
    for row in &group.rows {
        for path in row_paths(row) {
            paths.push(path.clone());
        }
    }
    paths
}

fn section_group_for_row(
    section: &ConfigSectionView,
    view: PluginConfigView,
    index: usize,
) -> Option<&ConfigGroupView> {
    let ConfigSectionBody::Form { groups, .. } = &section.body else {
        return None;
    };
    let mut visible_index = 0usize;
    for group in groups {
        for row in &group.rows {
            if !row_visible(row, view) {
                continue;
            }
            if visible_index == index {
                return Some(group);
            }
            visible_index += 1;
        }
    }
    None
}

fn drilldown_group_for_row(
    overlay: &PluginConfigDrilldownOverlay,
    view: PluginConfigView,
    index: usize,
) -> Option<&ConfigGroupView> {
    let mut visible_index = 0usize;
    for group in &overlay.groups {
        for row in &group.rows {
            if !row_visible(row, view) {
                continue;
            }
            if visible_index == index {
                return Some(group);
            }
            visible_index += 1;
        }
    }
    None
}

fn build_drilldown_groups(
    plugin: &PluginWorkbenchPlugin,
    path: &ConfigPath,
    title: &str,
) -> Vec<ConfigGroupView> {
    let value = get_value_at_path(&plugin.draft_config, path).unwrap_or(&JsonValue::Null);
    match value {
        JsonValue::Object(_) => build_generic_object_groups(plugin, path, title),
        JsonValue::Array(items) => {
            let rows = items
                .iter()
                .enumerate()
                .map(|(index, _)| {
                    let mut item_path = path.clone();
                    item_path.push(PathSegment::Index(index));
                    build_row_for_path(plugin, item_path, format!("Item {index}").as_str(), None)
                })
                .collect::<Vec<_>>();
            vec![ConfigGroupView {
                title: title.to_owned(),
                layout: ConfigGroupLayout::Standard,
                rows,
            }]
        }
        _ => vec![ConfigGroupView {
            title: title.to_owned(),
            layout: ConfigGroupLayout::Standard,
            rows: vec![build_row_for_path(plugin, path.clone(), title, None)],
        }],
    }
}

fn drilldown_row_count(overlay: &PluginConfigDrilldownOverlay, view: PluginConfigView) -> usize {
    overlay
        .groups
        .iter()
        .map(|group| {
            group
                .rows
                .iter()
                .filter(|row| row_visible(row, view))
                .count()
        })
        .sum()
}

fn drilldown_row_at(
    overlay: &PluginConfigDrilldownOverlay,
    view: PluginConfigView,
    index: usize,
) -> Option<&ConfigRowView> {
    let mut visible_index = 0usize;
    for group in &overlay.groups {
        for row in &group.rows {
            if !row_visible(row, view) {
                continue;
            }
            if visible_index == index {
                return Some(row);
            }
            visible_index += 1;
        }
    }
    None
}

fn rebuild_drilldown_overlay(
    dialog: &PluginWorkbenchOverlay,
    previous: &PluginConfigDrilldownOverlay,
) -> Option<PluginConfigDrilldownOverlay> {
    let plugin = dialog
        .plugins
        .iter()
        .find(|plugin| plugin.plugin_id == previous.plugin_id)?;
    let groups = build_drilldown_groups(plugin, &previous.path, previous.title.as_str());
    let mut overlay = PluginConfigDrilldownOverlay {
        plugin_id: previous.plugin_id.clone(),
        path: previous.path.clone(),
        title: previous.title.clone(),
        groups,
        selected_row: previous.selected_row,
        selected_cell: previous.selected_cell,
    };
    if drilldown_row_count(&overlay, dialog.config_view) == 0 {
        overlay.selected_row = 0;
    } else {
        overlay.selected_row = overlay
            .selected_row
            .min(drilldown_row_count(&overlay, dialog.config_view).saturating_sub(1));
    }
    overlay.selected_cell =
        drilldown_selected_row_cell(&overlay, dialog.config_view, overlay.selected_cell);
    Some(overlay)
}

fn rebuild_drilldown_stack(
    dialog: &PluginWorkbenchOverlay,
    previous_stack: &[PluginConfigDrilldownOverlay],
) -> Vec<PluginConfigDrilldownOverlay> {
    previous_stack
        .iter()
        .filter_map(|overlay| rebuild_drilldown_overlay(dialog, overlay))
        .collect()
}

fn plugin_semantic_diagnostics(plugin: &PluginWorkbenchPlugin) -> Vec<ConfigDiagnostic> {
    if plugin.plugin_id != "agena.web" {
        return Vec::new();
    }
    let mut diagnostics = Vec::new();
    for (path, label) in [
        (
            config_path(["fetch", "request", "delay_ms"]),
            "fetch.request.delay_ms",
        ),
        (
            config_path(["fetch", "request", "timeout_secs"]),
            "fetch.request.timeout_secs",
        ),
        (
            config_path(["fetch", "request", "max_body_bytes"]),
            "fetch.request.max_body_bytes",
        ),
        (
            config_path(["fetch", "cache", "ttl_secs"]),
            "fetch.cache.ttl_secs",
        ),
        (
            config_path(["fetch", "cache", "capacity"]),
            "fetch.cache.capacity",
        ),
        (
            config_path(["crawl", "defaults", "max_pages"]),
            "crawl.defaults.max_pages",
        ),
        (
            config_path(["crawl", "limits", "max_pages"]),
            "crawl.limits.max_pages",
        ),
        (
            config_path(["crawl", "limits", "max_depth"]),
            "crawl.limits.max_depth",
        ),
        (
            config_path(["crawl", "indexing", "document_cache_ttl_secs"]),
            "crawl.indexing.document_cache_ttl_secs",
        ),
        (
            config_path(["crawl", "indexing", "chunk_chars"]),
            "crawl.indexing.chunk_chars",
        ),
        (
            config_path(["crawl", "indexing", "near_duplicate_hamming_distance"]),
            "crawl.indexing.near_duplicate_hamming_distance",
        ),
        (
            config_path(["search", "default_limit"]),
            "search.default_limit",
        ),
        (config_path(["search", "max_limit"]), "search.max_limit"),
        (
            config_path(["store", "retention", "max_documents"]),
            "store.retention.max_documents",
        ),
        (
            config_path(["store", "retention", "max_bytes"]),
            "store.retention.max_bytes",
        ),
        (
            config_path(["store", "listing", "default_limit"]),
            "store.listing.default_limit",
        ),
        (
            config_path(["store", "listing", "max_limit"]),
            "store.listing.max_limit",
        ),
        (
            config_path(["browser", "wait", "timeout_secs"]),
            "browser.wait.timeout_secs",
        ),
    ] {
        if get_value_at_path(&plugin.draft_config, &path)
            .and_then(JsonValue::as_u64)
            .is_some_and(|value| value == 0)
        {
            push_diag(
                &mut diagnostics,
                DiagnosticSeverity::Error,
                &path,
                &title_for_config_path(plugin, &path, label),
                "must be greater than 0",
            );
        }
    }
    for (left_path, right_path, message) in [
        (
            config_path(["crawl", "defaults", "max_pages"]),
            config_path(["crawl", "limits", "max_pages"]),
            "default value must be <= limit",
        ),
        (
            config_path(["crawl", "defaults", "max_depth"]),
            config_path(["crawl", "limits", "max_depth"]),
            "default value must be <= limit",
        ),
        (
            config_path(["search", "default_limit"]),
            config_path(["search", "max_limit"]),
            "default value must be <= max",
        ),
        (
            config_path(["store", "listing", "default_limit"]),
            config_path(["store", "listing", "max_limit"]),
            "default value must be <= max",
        ),
    ] {
        let Some(left) =
            get_value_at_path(&plugin.draft_config, &left_path).and_then(JsonValue::as_u64)
        else {
            continue;
        };
        let Some(right) =
            get_value_at_path(&plugin.draft_config, &right_path).and_then(JsonValue::as_u64)
        else {
            continue;
        };
        if left > right {
            push_diag(
                &mut diagnostics,
                DiagnosticSeverity::Error,
                &left_path,
                &title_for_config_path(plugin, &left_path, "Value"),
                message,
            );
        }
    }
    for (path, message) in [
        (
            config_path(["browser", "executable_path"]),
            "executable path cannot be empty when set",
        ),
        (
            config_path(["browser", "wait", "for_selector"]),
            "selector cannot be empty when set",
        ),
    ] {
        if get_value_at_path(&plugin.draft_config, &path)
            .and_then(JsonValue::as_str)
            .is_some_and(|value| value.trim().is_empty())
        {
            push_diag(
                &mut diagnostics,
                DiagnosticSeverity::Error,
                &path,
                &title_for_config_path(plugin, &path, "Value"),
                message,
            );
        }
    }
    diagnostics
}

fn build_config_sections(plugin: &PluginWorkbenchPlugin) -> Vec<ConfigSectionView> {
    if plugin.plugin_id == "agena.web" {
        build_web_config_sections(plugin)
    } else {
        build_generic_config_sections(plugin)
    }
}

fn build_web_config_sections(plugin: &PluginWorkbenchPlugin) -> Vec<ConfigSectionView> {
    let fetch_enabled = get_value_at_path(&plugin.draft_config, &config_path(["fetch", "enabled"]))
        .and_then(JsonValue::as_bool)
        .unwrap_or(true);
    let mut sections = vec![ConfigSectionView {
        key: "overview".to_owned(),
        title: "Overview".to_owned(),
        issue_count: plugin
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error)
            .count(),
        dirty: plugin.dirty,
        body: ConfigSectionBody::Overview {
            cards: vec![
                ConfigOverviewCard {
                    title: "Fetch".to_owned(),
                    summary: format!(
                        "{}, {}, {}",
                        if fetch_enabled { "enabled" } else { "disabled" },
                        compact_duration_summary(
                            get_value_at_path(
                                &plugin.draft_config,
                                &config_path(["fetch", "request", "delay_ms"]),
                            )
                            .and_then(JsonValue::as_u64)
                            .unwrap_or_default(),
                            "ms",
                            "delay",
                        ),
                        compact_duration_summary(
                            get_value_at_path(
                                &plugin.draft_config,
                                &config_path(["fetch", "request", "timeout_secs"]),
                            )
                            .and_then(JsonValue::as_u64)
                            .unwrap_or_default(),
                            "s",
                            "timeout",
                        )
                    ),
                    issue_label: section_issue_label(plugin, &config_path(["fetch"])),
                },
                ConfigOverviewCard {
                    title: "Crawl".to_owned(),
                    summary: format!(
                        "{} / depth {}, limit {} / {}",
                        get_value_at_path(
                            &plugin.draft_config,
                            &config_path(["crawl", "defaults", "max_pages"]),
                        )
                        .and_then(JsonValue::as_u64)
                        .unwrap_or_default(),
                        get_value_at_path(
                            &plugin.draft_config,
                            &config_path(["crawl", "defaults", "max_depth"]),
                        )
                        .and_then(JsonValue::as_u64)
                        .unwrap_or_default(),
                        get_value_at_path(
                            &plugin.draft_config,
                            &config_path(["crawl", "limits", "max_pages"]),
                        )
                        .and_then(JsonValue::as_u64)
                        .unwrap_or_default(),
                        get_value_at_path(
                            &plugin.draft_config,
                            &config_path(["crawl", "limits", "max_depth"]),
                        )
                        .and_then(JsonValue::as_u64)
                        .unwrap_or_default(),
                    ),
                    issue_label: section_issue_label(plugin, &config_path(["crawl"])),
                },
                ConfigOverviewCard {
                    title: "Search".to_owned(),
                    summary: format!(
                        "default {}, max {}",
                        get_value_at_path(
                            &plugin.draft_config,
                            &config_path(["search", "default_limit"]),
                        )
                        .and_then(JsonValue::as_u64)
                        .unwrap_or_default(),
                        get_value_at_path(
                            &plugin.draft_config,
                            &config_path(["search", "max_limit"]),
                        )
                        .and_then(JsonValue::as_u64)
                        .unwrap_or_default(),
                    ),
                    issue_label: section_issue_label(plugin, &config_path(["search"])),
                },
                ConfigOverviewCard {
                    title: "Store".to_owned(),
                    summary: format!(
                        "{} docs, {}, listing {} / {}",
                        get_value_at_path(
                            &plugin.draft_config,
                            &config_path(["store", "retention", "max_documents"]),
                        )
                        .and_then(JsonValue::as_u64)
                        .unwrap_or_default(),
                        format_bytes_summary(
                            get_value_at_path(
                                &plugin.draft_config,
                                &config_path(["store", "retention", "max_bytes"]),
                            )
                            .and_then(JsonValue::as_u64)
                            .unwrap_or_default()
                        ),
                        get_value_at_path(
                            &plugin.draft_config,
                            &config_path(["store", "listing", "default_limit"]),
                        )
                        .and_then(JsonValue::as_u64)
                        .unwrap_or_default(),
                        get_value_at_path(
                            &plugin.draft_config,
                            &config_path(["store", "listing", "max_limit"]),
                        )
                        .and_then(JsonValue::as_u64)
                        .unwrap_or_default(),
                    ),
                    issue_label: section_issue_label(plugin, &config_path(["store"])),
                },
                ConfigOverviewCard {
                    title: "Browser".to_owned(),
                    summary: format!(
                        "{}, wait {}, selector {}",
                        if get_value_at_path(
                            &plugin.draft_config,
                            &config_path(["browser", "enabled"]),
                        )
                        .and_then(JsonValue::as_bool)
                        .unwrap_or(false)
                        {
                            "enabled"
                        } else {
                            "disabled"
                        },
                        compact_duration_summary(
                            get_value_at_path(
                                &plugin.draft_config,
                                &config_path(["browser", "wait", "timeout_secs"]),
                            )
                            .and_then(JsonValue::as_u64)
                            .unwrap_or_default(),
                            "s",
                            "",
                        ),
                        if get_value_at_path(
                            &plugin.draft_config,
                            &config_path(["browser", "wait", "for_selector"]),
                        )
                        .and_then(JsonValue::as_str)
                        .is_some_and(|value| !value.trim().is_empty())
                        {
                            "set"
                        } else {
                            "not set"
                        }
                    ),
                    issue_label: section_issue_label(plugin, &config_path(["browser"])),
                },
            ],
            lines: vec![
                format!(
                    "Schema             {}",
                    if plugin.schema_missing {
                        "Missing"
                    } else {
                        "Available"
                    }
                ),
                "Effective mode     Full config values".to_owned(),
                format!(
                    "Changed            {} field(s)",
                    override_leaf_count(&plugin.draft_override)
                ),
                format!(
                    "Diagnostics        {}",
                    if plugin.diagnostics.is_empty() {
                        "No issues".to_owned()
                    } else {
                        format!("{} issue(s)", plugin.diagnostics.len())
                    }
                ),
            ],
        },
    }];

    sections.push(web_form_section(
        plugin,
        "fetch",
        "Fetch",
        config_path(["fetch"]),
        (!fetch_enabled).then(|| {
            "Fetch disabled: agena.web/fetch and agena.web/crawl will be unavailable".to_owned()
        }),
        vec![
            ConfigGroupView {
                title: "Fetch".to_owned(),
                layout: ConfigGroupLayout::Standard,
                rows: vec![build_bool_row(
                    plugin,
                    "Enabled",
                    config_path(["fetch", "enabled"]),
                    None,
                )],
            },
            ConfigGroupView {
                title: "Request".to_owned(),
                layout: ConfigGroupLayout::Standard,
                rows: vec![
                    build_integer_row(
                        plugin,
                        "Delay",
                        config_path(["fetch", "request", "delay_ms"]),
                        (!fetch_enabled).then(|| "Fetch is disabled".to_owned()),
                    ),
                    build_integer_row(
                        plugin,
                        "Timeout",
                        config_path(["fetch", "request", "timeout_secs"]),
                        (!fetch_enabled).then(|| "Fetch is disabled".to_owned()),
                    ),
                    build_integer_row(
                        plugin,
                        "Max body size",
                        config_path(["fetch", "request", "max_body_bytes"]),
                        (!fetch_enabled).then(|| "Fetch is disabled".to_owned()),
                    ),
                    build_bool_row(
                        plugin,
                        "Respect robots.txt",
                        config_path(["fetch", "request", "respect_robots_txt"]),
                        (!fetch_enabled).then(|| "Fetch is disabled".to_owned()),
                    ),
                ],
            },
            ConfigGroupView {
                title: "Cache".to_owned(),
                layout: ConfigGroupLayout::Standard,
                rows: vec![
                    build_integer_row(
                        plugin,
                        "TTL",
                        config_path(["fetch", "cache", "ttl_secs"]),
                        (!fetch_enabled).then(|| "Fetch is disabled".to_owned()),
                    ),
                    build_integer_row(
                        plugin,
                        "Capacity",
                        config_path(["fetch", "cache", "capacity"]),
                        (!fetch_enabled).then(|| "Fetch is disabled".to_owned()),
                    ),
                ],
            },
        ],
    ));

    sections.push(web_form_section(
        plugin,
        "crawl",
        "Crawl",
        config_path(["crawl"]),
        None,
        vec![
            ConfigGroupView {
                title: "Crawl Range".to_owned(),
                layout: ConfigGroupLayout::Pair {
                    left_label: "Value",
                    right_label: "Limit",
                },
                rows: vec![
                    build_pair_integer_row(
                        plugin,
                        "Max pages",
                        config_path(["crawl", "defaults", "max_pages"]),
                        config_path(["crawl", "limits", "max_pages"]),
                        None,
                    ),
                    build_pair_integer_row(
                        plugin,
                        "Max depth",
                        config_path(["crawl", "defaults", "max_depth"]),
                        config_path(["crawl", "limits", "max_depth"]),
                        None,
                    ),
                    build_bool_row(
                        plugin,
                        "Same host only",
                        config_path(["crawl", "defaults", "same_host_only"]),
                        None,
                    ),
                ],
            },
            ConfigGroupView {
                title: "Indexing".to_owned(),
                layout: ConfigGroupLayout::Standard,
                rows: vec![
                    build_integer_row(
                        plugin,
                        "Document cache TTL",
                        config_path(["crawl", "indexing", "document_cache_ttl_secs"]),
                        None,
                    ),
                    build_integer_row(
                        plugin,
                        "Chunk size",
                        config_path(["crawl", "indexing", "chunk_chars"]),
                        None,
                    ),
                    build_integer_row(
                        plugin,
                        "Near-duplicate distance",
                        config_path(["crawl", "indexing", "near_duplicate_hamming_distance"]),
                        None,
                    ),
                ],
            },
        ],
    ));

    sections.push(web_form_section(
        plugin,
        "search",
        "Search",
        config_path(["search"]),
        None,
        vec![ConfigGroupView {
            title: "Search Results".to_owned(),
            layout: ConfigGroupLayout::Pair {
                left_label: "Value",
                right_label: "Max",
            },
            rows: vec![build_pair_integer_row(
                plugin,
                "Result limit",
                config_path(["search", "default_limit"]),
                config_path(["search", "max_limit"]),
                None,
            )],
        }],
    ));

    sections.push(web_form_section(
        plugin,
        "store",
        "Store",
        config_path(["store"]),
        None,
        vec![
            ConfigGroupView {
                title: "Retention".to_owned(),
                layout: ConfigGroupLayout::Standard,
                rows: vec![
                    build_integer_row(
                        plugin,
                        "Max documents",
                        config_path(["store", "retention", "max_documents"]),
                        None,
                    ),
                    build_integer_row(
                        plugin,
                        "Max bytes",
                        config_path(["store", "retention", "max_bytes"]),
                        None,
                    ),
                ],
            },
            ConfigGroupView {
                title: "Listing".to_owned(),
                layout: ConfigGroupLayout::Pair {
                    left_label: "Value",
                    right_label: "Max",
                },
                rows: vec![build_pair_integer_row(
                    plugin,
                    "List limit",
                    config_path(["store", "listing", "default_limit"]),
                    config_path(["store", "listing", "max_limit"]),
                    None,
                )],
            },
        ],
    ));

    sections.push(web_form_section(
        plugin,
        "browser",
        "Browser",
        config_path(["browser"]),
        None,
        vec![
            ConfigGroupView {
                title: "Browser".to_owned(),
                layout: ConfigGroupLayout::Standard,
                rows: vec![
                    build_bool_row(plugin, "Enabled", config_path(["browser", "enabled"]), None),
                    build_nullable_string_row(
                        plugin,
                        "Executable path",
                        config_path(["browser", "executable_path"]),
                        None,
                    ),
                ],
            },
            ConfigGroupView {
                title: "Wait".to_owned(),
                layout: ConfigGroupLayout::Standard,
                rows: vec![
                    build_bool_row(
                        plugin,
                        "Network idle",
                        config_path(["browser", "wait", "for_network_idle"]),
                        None,
                    ),
                    build_integer_row(
                        plugin,
                        "Timeout",
                        config_path(["browser", "wait", "timeout_secs"]),
                        None,
                    ),
                    build_nullable_string_row(
                        plugin,
                        "Selector",
                        config_path(["browser", "wait", "for_selector"]),
                        None,
                    ),
                    build_integer_row(
                        plugin,
                        "Extra delay",
                        config_path(["browser", "wait", "delay_ms"]),
                        None,
                    ),
                ],
            },
        ],
    ));
    sections
}

fn build_generic_config_sections(plugin: &PluginWorkbenchPlugin) -> Vec<ConfigSectionView> {
    let mut sections = vec![build_generic_overview_section(plugin)];
    let root_value = &plugin.draft_config;
    let root_schema = plugin.schema.as_ref();
    if let Some(object) = root_value.as_object() {
        for key in ordered_object_keys(root_schema, object) {
            let path = vec![PathSegment::Key(key.clone())];
            sections.push(build_generic_section(
                plugin,
                &path,
                title_for_config_path(plugin, &path, key.as_str()),
            ));
        }
    } else {
        sections.push(build_generic_section(
            plugin,
            &Vec::new(),
            "Config".to_owned(),
        ));
    }
    sections
}

fn config_path<const N: usize>(segments: [&str; N]) -> ConfigPath {
    segments
        .into_iter()
        .map(|segment| PathSegment::Key(segment.to_owned()))
        .collect()
}

fn section_issue_label(plugin: &PluginWorkbenchPlugin, path: &[PathSegment]) -> Option<String> {
    let count = plugin
        .diagnostics
        .iter()
        .filter(|diagnostic| {
            diagnostic.severity == DiagnosticSeverity::Error
                && path_is_prefix_of(path, diagnostic.path.as_slice())
        })
        .count();
    (count > 0).then(|| format!("Error {count}"))
}

fn section_issue_count(plugin: &PluginWorkbenchPlugin, path: &[PathSegment]) -> usize {
    plugin
        .diagnostics
        .iter()
        .filter(|diagnostic| {
            diagnostic.severity == DiagnosticSeverity::Error
                && path_is_prefix_of(path, diagnostic.path.as_slice())
        })
        .count()
}

fn section_dirty(plugin: &PluginWorkbenchPlugin, path: &[PathSegment]) -> bool {
    if path.is_empty() {
        return plugin.dirty;
    }
    path_present_in_value(&plugin.draft_override, path)
        || !diff_config_values(
            get_value_at_path(&plugin.saved_config, &path.to_vec()).unwrap_or(&JsonValue::Null),
            get_value_at_path(&plugin.draft_config, &path.to_vec()).unwrap_or(&JsonValue::Null),
        )
        .is_empty()
}

fn web_form_section(
    plugin: &PluginWorkbenchPlugin,
    key: &str,
    title: &str,
    path: ConfigPath,
    notice: Option<String>,
    groups: Vec<ConfigGroupView>,
) -> ConfigSectionView {
    ConfigSectionView {
        key: key.to_owned(),
        title: title.to_owned(),
        issue_count: section_issue_count(plugin, &path),
        dirty: section_dirty(plugin, &path),
        body: ConfigSectionBody::Form { notice, groups },
    }
}

fn build_generic_overview_section(plugin: &PluginWorkbenchPlugin) -> ConfigSectionView {
    let mut cards = Vec::new();
    if let Some(root) = plugin.draft_config.as_object() {
        for key in ordered_object_keys(plugin.schema.as_ref(), root) {
            let path = vec![PathSegment::Key(key.clone())];
            let summary = get_value_at_path(&plugin.draft_config, &path)
                .map(preview_value)
                .unwrap_or_else(|| "missing".to_owned());
            cards.push(ConfigOverviewCard {
                title: title_for_config_path(plugin, &path, key.as_str()),
                summary,
                issue_label: section_issue_label(plugin, &path),
            });
        }
    }
    ConfigSectionView {
        key: "overview".to_owned(),
        title: "Overview".to_owned(),
        issue_count: plugin
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error)
            .count(),
        dirty: plugin.dirty,
        body: ConfigSectionBody::Overview {
            cards,
            lines: vec![
                format!(
                    "Schema             {}",
                    if plugin.schema_missing {
                        "Missing"
                    } else {
                        "Available"
                    }
                ),
                "Effective mode     Full config values".to_owned(),
                format!(
                    "Changed            {} field(s)",
                    override_leaf_count(&plugin.draft_override)
                ),
                format!(
                    "Diagnostics        {}",
                    if plugin.diagnostics.is_empty() {
                        "No issues".to_owned()
                    } else {
                        format!("{} issue(s)", plugin.diagnostics.len())
                    }
                ),
            ],
        },
    }
}

fn build_generic_section(
    plugin: &PluginWorkbenchPlugin,
    path: &ConfigPath,
    title: String,
) -> ConfigSectionView {
    let value = get_value_at_path(&plugin.draft_config, path).unwrap_or(&JsonValue::Null);
    let groups = if value.is_object() {
        build_generic_object_groups(plugin, path, title.as_str())
    } else {
        vec![ConfigGroupView {
            title: title.clone(),
            layout: ConfigGroupLayout::Standard,
            rows: vec![build_row_for_path(
                plugin,
                path.clone(),
                title.as_str(),
                None,
            )],
        }]
    };
    ConfigSectionView {
        key: path
            .last()
            .and_then(|segment| match segment {
                PathSegment::Key(key) => Some(key.clone()),
                PathSegment::Index(_) => None,
            })
            .unwrap_or_else(|| "config".to_owned()),
        title,
        issue_count: section_issue_count(plugin, path),
        dirty: section_dirty(plugin, path),
        body: ConfigSectionBody::Form {
            notice: None,
            groups,
        },
    }
}

fn build_generic_object_groups(
    plugin: &PluginWorkbenchPlugin,
    path: &ConfigPath,
    title: &str,
) -> Vec<ConfigGroupView> {
    let value = get_value_at_path(&plugin.draft_config, path)
        .and_then(JsonValue::as_object)
        .cloned()
        .unwrap_or_default();
    let schema = plugin
        .schema
        .as_ref()
        .and_then(|schema| schema_for_path(schema, schema, &plugin.draft_config, path));
    let mut primitive_rows = Vec::new();
    let mut groups = Vec::new();
    for key in ordered_object_keys(schema.as_ref(), &value) {
        let mut child_path = path.clone();
        child_path.push(PathSegment::Key(key.clone()));
        let child_value = value.get(key.as_str()).unwrap_or(&JsonValue::Null);
        let child_schema = plugin.schema.as_ref().and_then(|root_schema| {
            declared_schema_for_path(root_schema, root_schema, &plugin.draft_config, &child_path)
        });
        if should_expand_object_child(plugin, child_schema.as_ref(), child_value) {
            groups.push(ConfigGroupView {
                title: title_for_config_path(plugin, &child_path, key.as_str()),
                layout: ConfigGroupLayout::Standard,
                rows: flatten_generic_object_rows(plugin, &child_path),
            });
        } else {
            primitive_rows.push(build_row_for_path(
                plugin,
                child_path,
                title_for_schema_or_key(
                    child_schema.as_ref().unwrap_or(&JsonValue::Null),
                    key.as_str(),
                )
                .as_str(),
                None,
            ));
        }
    }
    if !primitive_rows.is_empty() {
        groups.insert(
            0,
            ConfigGroupView {
                title: title.to_owned(),
                layout: ConfigGroupLayout::Standard,
                rows: primitive_rows,
            },
        );
    }
    if groups.is_empty() {
        groups.push(ConfigGroupView {
            title: title.to_owned(),
            layout: ConfigGroupLayout::Standard,
            rows: vec![build_structured_row(plugin, title, path.clone(), None)],
        });
    }
    groups
}

fn should_expand_object_child(
    plugin: &PluginWorkbenchPlugin,
    child_schema: Option<&JsonValue>,
    child_value: &JsonValue,
) -> bool {
    if !child_value.is_object() {
        return false;
    }
    let Some(child_schema) = child_schema else {
        return false;
    };
    let root = plugin.schema.as_ref().unwrap_or(child_schema);
    effective_schema_kind(child_schema).as_deref() == Some("object")
        && !schema_is_map_like(root, child_schema)
}

fn flatten_generic_object_rows(
    plugin: &PluginWorkbenchPlugin,
    path: &ConfigPath,
) -> Vec<ConfigRowView> {
    let value = get_value_at_path(&plugin.draft_config, path)
        .and_then(JsonValue::as_object)
        .cloned()
        .unwrap_or_default();
    let schema = plugin
        .schema
        .as_ref()
        .and_then(|schema| schema_for_path(schema, schema, &plugin.draft_config, path));
    let mut rows = Vec::new();
    for key in ordered_object_keys(schema.as_ref(), &value) {
        let mut child_path = path.clone();
        child_path.push(PathSegment::Key(key.clone()));
        rows.push(build_row_for_path(
            plugin,
            child_path.clone(),
            title_for_config_path(plugin, &child_path, key.as_str()).as_str(),
            None,
        ));
    }
    rows
}

fn build_row_for_path(
    plugin: &PluginWorkbenchPlugin,
    path: ConfigPath,
    title: &str,
    inactive_reason: Option<String>,
) -> ConfigRowView {
    let value = get_value_at_path(&plugin.draft_config, &path).unwrap_or(&JsonValue::Null);
    let schema = plugin
        .schema
        .as_ref()
        .and_then(|schema| schema_for_path(schema, schema, &plugin.draft_config, &path));
    if schema.as_ref().is_some_and(|schema| {
        schema_const_value(schema).is_some() || schema_bool_keyword_any(schema, "readOnly")
    }) {
        return build_read_only_row(plugin, title, path, inactive_reason);
    }
    if let Some(variants) = schema.as_ref().and_then(|schema| {
        plugin
            .schema
            .as_ref()
            .and_then(|root| array_enum_variants(root, schema))
    }) {
        return build_multi_enum_row(plugin, title, path, variants, inactive_reason);
    }
    if schema.as_ref().and_then(schema_enum_values).is_some() {
        return build_enum_row(plugin, title, path, inactive_reason);
    }
    if schema
        .as_ref()
        .is_some_and(schema_uses_nullable_string_editor)
    {
        return build_nullable_string_row(plugin, title, path, inactive_reason);
    }
    match value {
        JsonValue::Bool(_) => build_bool_row(plugin, title, path, inactive_reason),
        JsonValue::Number(number) if number.as_i64().is_some() || number.as_u64().is_some() => {
            build_integer_row(plugin, title, path, inactive_reason)
        }
        JsonValue::Number(_) => build_number_row(plugin, title, path, inactive_reason),
        JsonValue::String(_) => build_string_row(plugin, title, path, inactive_reason),
        JsonValue::Null => build_null_row(plugin, title, path, inactive_reason),
        JsonValue::Object(_) | JsonValue::Array(_) => {
            build_structured_row(plugin, title, path, inactive_reason)
        }
    }
}

fn build_read_only_row(
    plugin: &PluginWorkbenchPlugin,
    title: &str,
    path: ConfigPath,
    inactive_reason: Option<String>,
) -> ConfigRowView {
    let value = get_value_at_path(&plugin.draft_config, &path)
        .cloned()
        .unwrap_or(JsonValue::Null);
    let default = get_value_at_path(&plugin.default_config, &path)
        .cloned()
        .unwrap_or(JsonValue::Null);
    let mut constraints = path_constraints(plugin, &path);
    constraints.push("read-only".to_owned());
    build_config_row(
        plugin,
        title,
        path.clone(),
        Vec::new(),
        ConfigRowEditor::ReadOnly { path: path.clone() },
        preview_value(&value),
        preview_value(&default),
        None,
        None,
        None,
        inactive_reason,
        path_description(plugin, &path),
        constraints,
    )
}

fn build_bool_row(
    plugin: &PluginWorkbenchPlugin,
    title: &str,
    path: ConfigPath,
    inactive_reason: Option<String>,
) -> ConfigRowView {
    let value = get_value_at_path(&plugin.draft_config, &path)
        .cloned()
        .unwrap_or(JsonValue::Bool(false));
    let default = get_value_at_path(&plugin.default_config, &path)
        .cloned()
        .unwrap_or(JsonValue::Bool(false));
    build_config_row(
        plugin,
        title,
        path.clone(),
        Vec::new(),
        ConfigRowEditor::Bool { path: path.clone() },
        format_bool_checkbox(value.as_bool().unwrap_or(false)),
        default
            .as_bool()
            .map(|value| value.to_string())
            .unwrap_or_else(|| "false".to_owned()),
        None,
        None,
        None,
        inactive_reason,
        path_description(plugin, &path),
        path_constraints(plugin, &path),
    )
}

fn build_integer_row(
    plugin: &PluginWorkbenchPlugin,
    title: &str,
    path: ConfigPath,
    inactive_reason: Option<String>,
) -> ConfigRowView {
    build_numeric_row(
        plugin,
        title,
        path,
        ScalarEditKind::Integer,
        inactive_reason,
    )
}

fn build_number_row(
    plugin: &PluginWorkbenchPlugin,
    title: &str,
    path: ConfigPath,
    inactive_reason: Option<String>,
) -> ConfigRowView {
    build_numeric_row(plugin, title, path, ScalarEditKind::Number, inactive_reason)
}

fn build_numeric_row(
    plugin: &PluginWorkbenchPlugin,
    title: &str,
    path: ConfigPath,
    kind: ScalarEditKind,
    inactive_reason: Option<String>,
) -> ConfigRowView {
    let value = get_value_at_path(&plugin.draft_config, &path)
        .cloned()
        .unwrap_or_else(|| JsonValue::Number(JsonNumber::from(0)));
    let default = get_value_at_path(&plugin.default_config, &path)
        .cloned()
        .unwrap_or_else(|| JsonValue::Number(JsonNumber::from(0)));
    build_config_row(
        plugin,
        title,
        path.clone(),
        Vec::new(),
        ConfigRowEditor::Scalar {
            path: path.clone(),
            kind,
        },
        format_value_with_brackets(&path, &value),
        format_default_value(&path, &default),
        None,
        None,
        None,
        inactive_reason,
        path_description(plugin, &path),
        path_constraints(plugin, &path),
    )
}

fn build_string_row(
    plugin: &PluginWorkbenchPlugin,
    title: &str,
    path: ConfigPath,
    inactive_reason: Option<String>,
) -> ConfigRowView {
    let value = get_value_at_path(&plugin.draft_config, &path)
        .cloned()
        .unwrap_or_else(|| JsonValue::String(String::new()));
    let default = get_value_at_path(&plugin.default_config, &path)
        .cloned()
        .unwrap_or_else(|| JsonValue::String(String::new()));
    build_config_row(
        plugin,
        title,
        path.clone(),
        Vec::new(),
        ConfigRowEditor::Scalar {
            path: path.clone(),
            kind: ScalarEditKind::String,
        },
        format_value_with_brackets(&path, &value),
        format_default_value(&path, &default),
        None,
        None,
        None,
        inactive_reason,
        path_description(plugin, &path),
        path_constraints(plugin, &path),
    )
}

fn build_nullable_string_row(
    plugin: &PluginWorkbenchPlugin,
    title: &str,
    path: ConfigPath,
    inactive_reason: Option<String>,
) -> ConfigRowView {
    let value = get_value_at_path(&plugin.draft_config, &path)
        .cloned()
        .unwrap_or(JsonValue::Null);
    let default = get_value_at_path(&plugin.default_config, &path)
        .cloned()
        .unwrap_or(JsonValue::Null);
    build_config_row(
        plugin,
        title,
        path.clone(),
        Vec::new(),
        ConfigRowEditor::NullableString { path: path.clone() },
        format_nullable_value_for_cell(&value),
        format_default_nullable_value(&default),
        None,
        None,
        None,
        inactive_reason,
        path_description(plugin, &path),
        path_constraints(plugin, &path),
    )
}

fn build_null_row(
    plugin: &PluginWorkbenchPlugin,
    title: &str,
    path: ConfigPath,
    inactive_reason: Option<String>,
) -> ConfigRowView {
    let value = get_value_at_path(&plugin.draft_config, &path)
        .cloned()
        .unwrap_or(JsonValue::Null);
    let default = get_value_at_path(&plugin.default_config, &path)
        .cloned()
        .unwrap_or(JsonValue::Null);
    build_config_row(
        plugin,
        title,
        path.clone(),
        Vec::new(),
        ConfigRowEditor::Null { path: path.clone() },
        format_value_with_brackets(&path, &value),
        format_default_value(&path, &default),
        None,
        None,
        None,
        inactive_reason,
        path_description(plugin, &path),
        path_constraints(plugin, &path),
    )
}

fn build_enum_row(
    plugin: &PluginWorkbenchPlugin,
    title: &str,
    path: ConfigPath,
    inactive_reason: Option<String>,
) -> ConfigRowView {
    let schema = plugin
        .schema
        .as_ref()
        .and_then(|schema| schema_for_path(schema, schema, &plugin.draft_config, &path));
    let variants = schema
        .as_ref()
        .and_then(schema_enum_values)
        .unwrap_or_default();
    let value = get_value_at_path(&plugin.draft_config, &path)
        .cloned()
        .unwrap_or(JsonValue::Null);
    let default = get_value_at_path(&plugin.default_config, &path)
        .cloned()
        .unwrap_or(JsonValue::Null);
    build_config_row(
        plugin,
        title,
        path.clone(),
        Vec::new(),
        ConfigRowEditor::Enum {
            path: path.clone(),
            variants,
        },
        format!("[ {} ▾ ]", preview_value(&value)),
        preview_value(&default),
        None,
        None,
        None,
        inactive_reason,
        path_description(plugin, &path),
        path_constraints(plugin, &path),
    )
}

fn build_multi_enum_row(
    plugin: &PluginWorkbenchPlugin,
    title: &str,
    path: ConfigPath,
    variants: Vec<JsonValue>,
    inactive_reason: Option<String>,
) -> ConfigRowView {
    let value = get_value_at_path(&plugin.draft_config, &path)
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    let default = get_value_at_path(&plugin.default_config, &path)
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    build_config_row(
        plugin,
        title,
        path.clone(),
        Vec::new(),
        ConfigRowEditor::MultiEnum {
            path: path.clone(),
            variants,
        },
        format_multi_enum_value_with_selector(value.as_slice()),
        format_multi_enum_default_value(default.as_slice()),
        None,
        None,
        None,
        inactive_reason,
        path_description(plugin, &path),
        path_constraints(plugin, &path),
    )
}

fn build_pair_integer_row(
    plugin: &PluginWorkbenchPlugin,
    title: &str,
    left_path: ConfigPath,
    right_path: ConfigPath,
    inactive_reason: Option<String>,
) -> ConfigRowView {
    let left_value = get_value_at_path(&plugin.draft_config, &left_path)
        .cloned()
        .unwrap_or_else(|| JsonValue::Number(JsonNumber::from(0)));
    let right_value = get_value_at_path(&plugin.draft_config, &right_path)
        .cloned()
        .unwrap_or_else(|| JsonValue::Number(JsonNumber::from(0)));
    let left_default = get_value_at_path(&plugin.default_config, &left_path)
        .cloned()
        .unwrap_or_else(|| JsonValue::Number(JsonNumber::from(0)));
    let right_default = get_value_at_path(&plugin.default_config, &right_path)
        .cloned()
        .unwrap_or_else(|| JsonValue::Number(JsonNumber::from(0)));
    build_config_row(
        plugin,
        title,
        left_path.clone(),
        vec![right_path.clone()],
        ConfigRowEditor::PairInteger {
            left_path: left_path.clone(),
            right_path: right_path.clone(),
        },
        format_value_with_brackets(&left_path, &left_value),
        format_default_value(&left_path, &left_default),
        Some(format_value_with_brackets(&right_path, &right_value)),
        None,
        Some(format_default_value(&right_path, &right_default)),
        inactive_reason,
        path_description(plugin, &left_path),
        pair_constraints(plugin, &left_path, &right_path),
    )
}

fn build_structured_row(
    plugin: &PluginWorkbenchPlugin,
    title: &str,
    path: ConfigPath,
    inactive_reason: Option<String>,
) -> ConfigRowView {
    let value = get_value_at_path(&plugin.draft_config, &path)
        .cloned()
        .unwrap_or(JsonValue::Null);
    let default = get_value_at_path(&plugin.default_config, &path)
        .cloned()
        .unwrap_or(JsonValue::Null);
    build_config_row(
        plugin,
        title,
        path.clone(),
        Vec::new(),
        ConfigRowEditor::Structured { path: path.clone() },
        structured_preview(&value),
        structured_preview(&default),
        None,
        None,
        None,
        inactive_reason,
        path_description(plugin, &path),
        path_constraints(plugin, &path),
    )
}

fn schema_contains_keyword(schema: &JsonValue, key: &str) -> bool {
    if schema
        .as_object()
        .is_some_and(|object| object.contains_key(key))
    {
        return true;
    }
    schema
        .get("allOf")
        .and_then(JsonValue::as_array)
        .is_some_and(|branches| {
            branches
                .iter()
                .any(|branch| schema_contains_keyword(branch, key))
        })
}

fn schema_bool_keyword_any(schema: &JsonValue, key: &str) -> bool {
    if schema.get(key).and_then(JsonValue::as_bool) == Some(true) {
        return true;
    }
    schema
        .get("allOf")
        .and_then(JsonValue::as_array)
        .is_some_and(|branches| {
            branches
                .iter()
                .any(|branch| schema_bool_keyword_any(branch, key))
        })
}

fn schema_first_string_keyword<'a>(schema: &'a JsonValue, key: &str) -> Option<&'a str> {
    if let Some(value) = schema.get(key).and_then(JsonValue::as_str) {
        return Some(value);
    }
    schema
        .get("allOf")
        .and_then(JsonValue::as_array)
        .and_then(|branches| {
            branches
                .iter()
                .find_map(|branch| schema_first_string_keyword(branch, key))
        })
}

fn schema_const_value(schema: &JsonValue) -> Option<JsonValue> {
    if let Some(constant) = schema.get("const") {
        return Some(constant.clone());
    }
    schema
        .get("allOf")
        .and_then(JsonValue::as_array)
        .and_then(|branches| branches.iter().find_map(schema_const_value))
}

fn schema_enum_values(schema: &JsonValue) -> Option<Vec<JsonValue>> {
    let mut combined = schema
        .get("const")
        .map(|constant| vec![constant.clone()])
        .or_else(|| {
            schema
                .get("enum")
                .and_then(JsonValue::as_array)
                .cloned()
                .filter(|variants| !variants.is_empty())
        });
    if let Some(all_of) = schema.get("allOf").and_then(JsonValue::as_array) {
        for branch in all_of {
            let Some(branch_values) = schema_enum_values(branch) else {
                continue;
            };
            combined = Some(match combined.take() {
                Some(current) => current
                    .into_iter()
                    .filter(|value| branch_values.iter().any(|candidate| candidate == value))
                    .collect(),
                None => branch_values,
            });
        }
    }
    combined.filter(|variants| !variants.is_empty())
}

fn schema_description_text(schema: &JsonValue) -> Option<String> {
    schema
        .get("description")
        .and_then(JsonValue::as_str)
        .map(str::to_owned)
        .or_else(|| {
            schema
                .get("allOf")
                .and_then(JsonValue::as_array)
                .and_then(|branches| branches.iter().find_map(schema_description_text))
        })
}

fn schema_examples(schema: &JsonValue) -> Option<Vec<JsonValue>> {
    schema
        .get("examples")
        .and_then(JsonValue::as_array)
        .cloned()
        .filter(|examples| !examples.is_empty())
        .or_else(|| {
            schema
                .get("allOf")
                .and_then(JsonValue::as_array)
                .and_then(|branches| branches.iter().find_map(schema_examples))
        })
}

fn array_enum_variants(root: &JsonValue, schema: &JsonValue) -> Option<Vec<JsonValue>> {
    if effective_schema_kind(schema).as_deref() != Some("array") {
        return None;
    }
    if schema_contains_keyword(schema, "prefixItems") {
        return None;
    }
    if !schema_bool_keyword_any(schema, "uniqueItems") {
        return None;
    }
    let item_schema = array_item_schema(root, schema, 0)?;
    if matches!(item_schema, JsonValue::Bool(false)) {
        return None;
    }
    let variants = schema_enum_values(&item_schema)?;
    (!variants.is_empty()).then_some(variants)
}

fn schema_uses_nullable_string_editor(schema: &JsonValue) -> bool {
    let type_choices = schema_type_choices(schema);
    !type_choices.is_empty()
        && type_choices.iter().any(|kind| kind == "null")
        && type_choices.iter().any(|kind| kind == "string")
        && type_choices
            .iter()
            .all(|kind| matches!(kind.as_str(), "null" | "string"))
}

fn format_multi_enum_value_with_selector(values: &[JsonValue]) -> String {
    if values.is_empty() {
        "[ None ▾ ]".to_owned()
    } else {
        format!(
            "[ {} ▾ ]",
            values
                .iter()
                .map(preview_value)
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}

fn format_multi_enum_default_value(values: &[JsonValue]) -> String {
    if values.is_empty() {
        "None".to_owned()
    } else {
        values
            .iter()
            .map(preview_value)
            .collect::<Vec<_>>()
            .join(", ")
    }
}

fn config_row_type_meta(
    plugin: &PluginWorkbenchPlugin,
    path: &ConfigPath,
    editor: &ConfigRowEditor,
) -> (String, ConfigRowTypeMode) {
    let value = get_value_at_path(&plugin.draft_config, path).unwrap_or(&JsonValue::Null);
    let declared_schema = plugin
        .schema
        .as_ref()
        .and_then(|root| declared_schema_for_path(root, root, &plugin.draft_config, path));
    if !matches!(
        editor,
        ConfigRowEditor::ReadOnly { .. }
            | ConfigRowEditor::Enum { .. }
            | ConfigRowEditor::MultiEnum { .. }
            | ConfigRowEditor::PairInteger { .. }
    ) {
        if let Some(schema) = declared_schema.as_ref()
            && let Some(root) = plugin.schema.as_ref()
            && let Some(branches) = branch_choices(root, schema)
        {
            return (
                format!("[ {} ▾ ]", active_branch_label(branches.as_slice(), value)),
                ConfigRowTypeMode::SelectShape,
            );
        }
        let choices = schema_type_selector_choices(declared_schema.as_ref());
        if choices.len() > 1 {
            return (
                format!("[ {} ▾ ]", json_kind_label(value)),
                ConfigRowTypeMode::SelectType,
            );
        }
    }
    let display = declared_schema
        .as_ref()
        .and_then(|schema| effective_schema_kind(schema))
        .unwrap_or_else(|| json_kind_label(value).to_owned());
    (display, ConfigRowTypeMode::Fixed)
}

#[allow(clippy::too_many_arguments)]
fn build_config_row(
    plugin: &PluginWorkbenchPlugin,
    title: &str,
    primary_path: ConfigPath,
    additional_paths: Vec<ConfigPath>,
    editor: ConfigRowEditor,
    value_display: String,
    default_display: String,
    secondary_value_display: Option<String>,
    action_display: Option<String>,
    _secondary_default_display: Option<String>,
    inactive_reason: Option<String>,
    description: Option<String>,
    constraints: Vec<String>,
) -> ConfigRowView {
    let all_paths = std::iter::once(&primary_path)
        .chain(additional_paths.iter())
        .cloned()
        .collect::<Vec<_>>();
    let diagnostics = plugin
        .diagnostics
        .iter()
        .filter(|diagnostic| {
            all_paths
                .iter()
                .any(|path| path_is_prefix_of(path.as_slice(), diagnostic.path.as_slice()))
        })
        .cloned()
        .collect::<Vec<_>>();
    let dirty = all_paths
        .iter()
        .any(|path| value_changed_at_path(&plugin.saved_config, &plugin.draft_config, path));
    let override_count = all_paths
        .iter()
        .filter(|path| path_present_in_value(&plugin.draft_override, path.as_slice()))
        .count();
    let state = if diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error)
    {
        ConfigRowState::Error
    } else if dirty {
        ConfigRowState::Dirty
    } else if inactive_reason.is_some() {
        ConfigRowState::Inactive
    } else if override_count > 0 {
        ConfigRowState::Override
    } else {
        ConfigRowState::Default
    };
    let (type_display, type_mode) = config_row_type_meta(plugin, &primary_path, &editor);
    let action_display = action_display.or_else(|| {
        config_row_action_display(
            plugin,
            &editor,
            primary_path.as_slice(),
            additional_paths.as_slice(),
        )
    });
    ConfigRowView {
        title: title.to_owned(),
        primary_path,
        additional_paths,
        editor,
        description,
        constraints,
        type_display,
        type_mode,
        value_display,
        default_display,
        secondary_value_display,
        action_display,
        state,
    }
}

fn value_changed_at_path(before: &JsonValue, after: &JsonValue, path: &ConfigPath) -> bool {
    get_value_at_path(before, path) != get_value_at_path(after, path)
}

fn override_leaf_count(value: &JsonValue) -> usize {
    match value {
        JsonValue::Null => 0,
        JsonValue::Object(object) => object.values().map(override_leaf_count).sum(),
        JsonValue::Array(items) => {
            if items.is_empty() {
                1
            } else {
                items.iter().map(override_leaf_count).sum()
            }
        }
        _ => 1,
    }
}

fn title_for_config_path(
    plugin: &PluginWorkbenchPlugin,
    path: &ConfigPath,
    fallback: &str,
) -> String {
    plugin
        .schema
        .as_ref()
        .and_then(|schema| schema_for_path(schema, schema, &plugin.draft_config, path))
        .as_ref()
        .map(|schema| title_for_schema_or_key(schema, fallback))
        .unwrap_or_else(|| title_from_key(fallback))
}

fn path_description(plugin: &PluginWorkbenchPlugin, path: &ConfigPath) -> Option<String> {
    plugin
        .schema
        .as_ref()
        .and_then(|schema| schema_for_path(schema, schema, &plugin.draft_config, path))
        .and_then(|schema| schema_description_text(&schema))
}

fn path_constraints(plugin: &PluginWorkbenchPlugin, path: &ConfigPath) -> Vec<String> {
    plugin
        .schema
        .as_ref()
        .and_then(|schema| schema_for_path(schema, schema, &plugin.draft_config, path))
        .map(|schema| schema_constraints(&schema))
        .unwrap_or_default()
}

fn pair_constraints(
    plugin: &PluginWorkbenchPlugin,
    left_path: &ConfigPath,
    right_path: &ConfigPath,
) -> Vec<String> {
    let mut constraints = path_constraints(plugin, left_path);
    constraints.extend(path_constraints(plugin, right_path));
    constraints.sort();
    constraints.dedup();
    constraints
}

fn format_bool_checkbox(value: bool) -> String {
    if value {
        "[x]".to_owned()
    } else {
        "[ ]".to_owned()
    }
}

fn format_value_with_brackets(path: &ConfigPath, value: &JsonValue) -> String {
    match value {
        JsonValue::Bool(value) => format_bool_checkbox(*value),
        JsonValue::String(text) => format!("[ {} ]", clean(truncate_text(text, 28))),
        JsonValue::Number(number) => format!("[ {} ]", format_number_with_unit(path, number)),
        JsonValue::Null => "[ null ]".to_owned(),
        JsonValue::Array(_) | JsonValue::Object(_) => format!("[ {} ]", structured_preview(value)),
    }
}

fn format_default_value(path: &ConfigPath, value: &JsonValue) -> String {
    match value {
        JsonValue::Bool(value) => value.to_string(),
        JsonValue::String(text) => clean(truncate_text(text, 28)),
        JsonValue::Number(number) => format_number_with_unit(path, number),
        JsonValue::Null => "Not set".to_owned(),
        JsonValue::Array(_) | JsonValue::Object(_) => structured_preview(value),
    }
}

fn format_nullable_value_for_cell(value: &JsonValue) -> String {
    match value {
        JsonValue::Null => "[ Not set ]".to_owned(),
        JsonValue::String(text) => format!("[ {} ]", clean(truncate_text(text, 24))),
        _ => format!("[ {} ]", preview_value(value)),
    }
}

fn format_default_nullable_value(value: &JsonValue) -> String {
    match value {
        JsonValue::Null => "Not set".to_owned(),
        JsonValue::String(text) => clean(truncate_text(text, 28)),
        _ => preview_value(value),
    }
}

fn format_number_with_unit(path: &ConfigPath, number: &JsonNumber) -> String {
    let Some(last) = path.last() else {
        return number.to_string();
    };
    let PathSegment::Key(key) = last else {
        return number.to_string();
    };
    if key.ends_with("_ms") {
        return format!("{} ms", number);
    }
    if key.ends_with("_secs") {
        return format!("{} sec", number);
    }
    if key.ends_with("_chars") {
        return format!("{} ch", number);
    }
    if key.ends_with("_bytes") {
        return format_bytes_summary(number.as_u64().unwrap_or_default());
    }
    number.to_string()
}

fn format_bytes_summary(bytes: u64) -> String {
    const MIB: u64 = 1024 * 1024;
    if bytes >= MIB && bytes % MIB == 0 {
        format!("{} MiB", bytes / MIB)
    } else if bytes >= MIB {
        format!("{:.1} MiB", bytes as f64 / MIB as f64)
    } else {
        format!("{bytes} bytes")
    }
}

fn compact_duration_summary(value: u64, suffix: &str, label: &str) -> String {
    if label.is_empty() {
        format!("{value}{suffix}")
    } else {
        format!("{value}{suffix} {label}")
    }
}

fn runtime_diagnostics(status: &agena::plugin::status::PluginStatus) -> Vec<ConfigDiagnostic> {
    if status.state == agena::plugin::status::PluginRunState::Failed {
        vec![ConfigDiagnostic {
            severity: DiagnosticSeverity::Error,
            source: "runtime".to_owned(),
            path: Vec::new(),
            field: "Process".to_owned(),
            message: status
                .last_error
                .clone()
                .unwrap_or_else(|| "plugin failed".to_owned()),
        }]
    } else {
        Vec::new()
    }
}

fn refresh_plugin_workbench_filter(dialog: &mut PluginWorkbenchOverlay) {
    let query = dialog.query.text().trim().to_ascii_lowercase();
    dialog.visible_plugins = dialog
        .plugins
        .iter()
        .enumerate()
        .filter_map(|(index, plugin)| {
            let matches_query = query.is_empty()
                || plugin
                    .plugin_id
                    .to_ascii_lowercase()
                    .contains(query.as_str())
                || plugin
                    .visible_tool
                    .to_ascii_lowercase()
                    .contains(query.as_str())
                || plugin
                    .transport
                    .to_ascii_lowercase()
                    .contains(query.as_str())
                || plugin
                    .config_status
                    .label
                    .to_ascii_lowercase()
                    .contains(query.as_str());
            let matches_transport = dialog.transport_filter.matches(plugin.transport.as_str());
            let matches_config = dialog.config_filter.matches(plugin.config_status.kind);
            (matches_query && matches_transport && matches_config).then_some(index)
        })
        .collect();
    dialog.clamp_selection();
}

impl PluginTransportFilter {
    fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Static => "native",
            Self::Stdio => "stdio",
            Self::Cdylib => "cdylib",
            Self::Http => "http",
            Self::Wasm => "wasm",
            Self::Other => "other",
        }
    }

    fn matches(self, transport: &str) -> bool {
        match self {
            Self::All => true,
            Self::Static => matches!(transport, "static" | "native"),
            Self::Stdio => transport == "stdio",
            Self::Cdylib => transport == "cdylib",
            Self::Http => transport == "http",
            Self::Wasm => transport == "wasm",
            Self::Other => !matches!(
                transport,
                "static" | "native" | "stdio" | "cdylib" | "http" | "wasm"
            ),
        }
    }
}

impl PluginConfigFilter {
    fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Valid => "Valid",
            Self::Missing => "Missing",
            Self::SchemaMissing => "Schema missing",
            Self::Issues => "Issues",
            Self::NeedsRestart => "Needs restart",
            Self::RuntimeIssue => "Runtime issue",
        }
    }

    fn matches(self, kind: PluginConfigStatusKind) -> bool {
        match self {
            Self::All => true,
            Self::Valid => kind == PluginConfigStatusKind::Valid,
            Self::Missing => kind == PluginConfigStatusKind::Missing,
            Self::SchemaMissing => kind == PluginConfigStatusKind::SchemaMissing,
            Self::Issues => matches!(
                kind,
                PluginConfigStatusKind::Invalid | PluginConfigStatusKind::Warning
            ),
            Self::NeedsRestart => kind == PluginConfigStatusKind::NeedsRestart,
            Self::RuntimeIssue => kind == PluginConfigStatusKind::RuntimeIssue,
        }
    }
}

fn next_transport_filter(filter: PluginTransportFilter) -> PluginTransportFilter {
    match filter {
        PluginTransportFilter::All => PluginTransportFilter::Static,
        PluginTransportFilter::Static => PluginTransportFilter::Stdio,
        PluginTransportFilter::Stdio => PluginTransportFilter::Cdylib,
        PluginTransportFilter::Cdylib => PluginTransportFilter::Http,
        PluginTransportFilter::Http => PluginTransportFilter::Wasm,
        PluginTransportFilter::Wasm => PluginTransportFilter::Other,
        PluginTransportFilter::Other => PluginTransportFilter::All,
    }
}

fn next_config_filter(filter: PluginConfigFilter) -> PluginConfigFilter {
    match filter {
        PluginConfigFilter::All => PluginConfigFilter::Valid,
        PluginConfigFilter::Valid => PluginConfigFilter::Missing,
        PluginConfigFilter::Missing => PluginConfigFilter::SchemaMissing,
        PluginConfigFilter::SchemaMissing => PluginConfigFilter::Issues,
        PluginConfigFilter::Issues => PluginConfigFilter::NeedsRestart,
        PluginConfigFilter::NeedsRestart => PluginConfigFilter::RuntimeIssue,
        PluginConfigFilter::RuntimeIssue => PluginConfigFilter::All,
    }
}

fn next_config_focus(focus: PluginConfigFocus, compact: bool) -> PluginConfigFocus {
    let _ = compact;
    match focus {
        PluginConfigFocus::Toolbar => PluginConfigFocus::Structure,
        PluginConfigFocus::Structure => PluginConfigFocus::Editor,
        _ => PluginConfigFocus::Toolbar,
    }
}

fn previous_config_focus(focus: PluginConfigFocus, compact: bool) -> PluginConfigFocus {
    let _ = compact;
    match focus {
        PluginConfigFocus::Toolbar => PluginConfigFocus::Editor,
        PluginConfigFocus::Editor => PluginConfigFocus::Structure,
        PluginConfigFocus::Structure => PluginConfigFocus::Toolbar,
        _ => PluginConfigFocus::Editor,
    }
}

fn localized_config_schema(
    manifest: &agena::plugin::PluginManifest,
    locale: &str,
) -> Option<JsonValue> {
    let mut schema = manifest.config_schema.clone()?;
    if let Some(overlay) = manifest.config_schema_i18n.get(locale).or_else(|| {
        locale
            .split('-')
            .next()
            .and_then(|language| manifest.config_schema_i18n.get(language))
    }) {
        merge_schema_overlay(&mut schema, overlay);
    }
    Some(schema)
}

fn merge_schema_overlay(target: &mut JsonValue, overlay: &JsonValue) {
    match (target, overlay) {
        (JsonValue::Object(target), JsonValue::Object(overlay)) => {
            for (key, overlay_value) in overlay {
                match target.get_mut(key) {
                    Some(target_value) => merge_schema_overlay(target_value, overlay_value),
                    None => {
                        target.insert(key.clone(), overlay_value.clone());
                    }
                }
            }
        }
        (target, overlay) => *target = overlay.clone(),
    }
}

fn validate_config_value(
    schema: Option<&JsonValue>,
    value: &JsonValue,
    schema_missing: bool,
) -> Vec<ConfigDiagnostic> {
    if value.is_null() {
        return Vec::new();
    }
    if schema_missing {
        return vec![ConfigDiagnostic {
            severity: DiagnosticSeverity::Warning,
            source: "config".to_owned(),
            path: Vec::new(),
            field: "Config".to_owned(),
            message: "schema missing; using generic structured editor".to_owned(),
        }];
    }
    let Some(schema) = schema else {
        return Vec::new();
    };
    let mut diagnostics = Vec::new();
    validate_schema_at(
        &mut diagnostics,
        schema,
        schema,
        value,
        &Vec::new(),
        "Config",
    );
    diagnostics
}

fn materialized_config_value(schema: Option<&JsonValue>, value: &JsonValue) -> JsonValue {
    let Some(schema) = schema else {
        return value.clone();
    };
    let mut materialized = materialized_value_for_schema(schema, schema);
    if !value.is_null() {
        merge_config_override(&mut materialized, value);
    }
    materialize_schema_fields(&mut materialized, schema, schema);
    materialized
}

fn merge_config_override(target: &mut JsonValue, override_value: &JsonValue) {
    match (target, override_value) {
        (JsonValue::Object(target), JsonValue::Object(override_object)) => {
            for (key, value) in override_object {
                match target.get_mut(key) {
                    Some(existing) => merge_config_override(existing, value),
                    None => {
                        target.insert(key.clone(), value.clone());
                    }
                }
            }
        }
        (target, value) => *target = value.clone(),
    }
}

fn materialized_string_value_for_schema(schema: &JsonValue) -> String {
    let min_length = schema
        .get("minLength")
        .and_then(JsonValue::as_u64)
        .unwrap_or_default() as usize;
    if min_length == 0 {
        String::new()
    } else {
        "x".repeat(min_length)
    }
}

fn materialized_numeric_value_for_schema(schema: &JsonValue, integer: bool) -> JsonValue {
    let multiple_of = schema
        .get("multipleOf")
        .and_then(JsonValue::as_f64)
        .filter(|value| *value > 0.0);
    let step = if integer {
        1.0
    } else {
        multiple_of.unwrap_or(1.0)
    };
    let mut candidate = 0.0_f64;
    if let Some(minimum) = schema.get("minimum").and_then(JsonValue::as_f64) {
        candidate = candidate.max(minimum);
    }
    if let Some(exclusive_minimum) = schema.get("exclusiveMinimum").and_then(JsonValue::as_f64) {
        candidate = candidate.max(exclusive_minimum + step);
    }
    if let Some(multiple_of) = multiple_of {
        candidate = (candidate / multiple_of).ceil() * multiple_of;
    }
    if integer {
        JsonValue::Number(JsonNumber::from(candidate.ceil() as i64))
    } else {
        JsonValue::Number(JsonNumber::from_f64(candidate).unwrap_or_else(|| JsonNumber::from(0)))
    }
}

fn materialized_value_for_schema(schema: &JsonValue, root: &JsonValue) -> JsonValue {
    let schema = resolve_schema(root, schema);
    if let Some(default) = schema.get("default") {
        return default.clone();
    }
    if let Some(constant) = schema_const_value(schema) {
        return constant;
    }
    if schema_type_choices(schema)
        .iter()
        .any(|kind| kind == "null")
    {
        return JsonValue::Null;
    }
    if let Some(variants) = schema_enum_values(schema)
        && let Some(first) = variants.first()
    {
        return first.clone();
    }
    if let Some(branches) = schema
        .get("oneOf")
        .or_else(|| schema.get("anyOf"))
        .and_then(JsonValue::as_array)
        && let Some(first) = branches.first()
    {
        return materialized_value_for_schema(first, root);
    }
    if let Some(all_of) = schema.get("allOf").and_then(JsonValue::as_array) {
        let mut value = JsonValue::Object(JsonMap::new());
        for branch in all_of {
            let branch_value = materialized_value_for_schema(branch, root);
            merge_default_value(&mut value, branch_value);
        }
        materialize_schema_fields(&mut value, schema, root);
        return value;
    }
    match effective_schema_kind(schema).as_deref() {
        Some("object") => {
            let mut object = JsonMap::new();
            let required = schema_required_fields(schema);
            for key in schema_declared_property_keys(schema) {
                let Some(child_schema) = object_property_schema(root, schema, key.as_str()) else {
                    continue;
                };
                if required.contains(key.as_str())
                    || schema_prefers_materialized_presence(&child_schema, root)
                {
                    object.insert(
                        key.clone(),
                        materialized_value_for_schema(&child_schema, root),
                    );
                }
            }
            JsonValue::Object(object)
        }
        Some("array") => JsonValue::Array(Vec::new()),
        Some("string") => JsonValue::String(materialized_string_value_for_schema(schema)),
        Some("integer") => materialized_numeric_value_for_schema(schema, true),
        Some("number") => materialized_numeric_value_for_schema(schema, false),
        Some("boolean") => JsonValue::Bool(false),
        Some("null") => JsonValue::Null,
        _ => JsonValue::Null,
    }
}

fn materialize_schema_fields(value: &mut JsonValue, schema: &JsonValue, root: &JsonValue) {
    let schema = active_schema_for_value(root, schema, value);
    match effective_schema_kind(&schema).as_deref() {
        Some("object") => {
            let JsonValue::Object(object) = value else {
                return;
            };
            let required = schema_required_fields(&schema);
            for key in schema_declared_property_keys(&schema) {
                let Some(child_schema) = object_property_schema(root, &schema, key.as_str()) else {
                    continue;
                };
                if required.contains(key.as_str())
                    || schema_prefers_materialized_presence(&child_schema, root)
                {
                    let child = object
                        .entry(key.clone())
                        .or_insert_with(|| materialized_value_for_schema(&child_schema, root));
                    materialize_schema_fields(child, &child_schema, root);
                }
            }
        }
        Some("array") => {
            let JsonValue::Array(items) = value else {
                return;
            };
            for (index, item) in items.iter_mut().enumerate() {
                if let Some(item_schema) = array_item_schema(root, &schema, index) {
                    materialize_schema_fields(item, &item_schema, root);
                }
            }
        }
        _ => {}
    }
}

fn schema_prefers_materialized_presence(schema: &JsonValue, root: &JsonValue) -> bool {
    let schema = resolve_schema(root, schema);
    if schema.get("default").is_some() || schema_const_value(schema).is_some() {
        return true;
    }
    if schema_enum_values(schema).is_some() {
        return true;
    }
    if schema.get("oneOf").is_some()
        || schema.get("anyOf").is_some()
        || schema.get("allOf").is_some()
    {
        return true;
    }
    matches!(
        effective_schema_kind(schema).as_deref(),
        Some("object") | Some("array")
    )
}

fn validate_schema_at(
    diagnostics: &mut Vec<ConfigDiagnostic>,
    root: &JsonValue,
    schema: &JsonValue,
    value: &JsonValue,
    path: &ConfigPath,
    title: &str,
) {
    let schema = resolve_schema(root, schema);
    if matches!(schema, JsonValue::Bool(true)) {
        return;
    }
    if matches!(schema, JsonValue::Bool(false)) {
        push_diag(
            diagnostics,
            DiagnosticSeverity::Error,
            path,
            title,
            "schema rejects this value",
        );
        return;
    }
    let Some(object) = schema.as_object() else {
        return;
    };
    if let Some(all_of) = object.get("allOf").and_then(JsonValue::as_array) {
        for branch in all_of {
            validate_schema_at(diagnostics, root, branch, value, path, title);
        }
    }
    if let Some(any_of) = object.get("anyOf").and_then(JsonValue::as_array)
        && !any_of
            .iter()
            .any(|branch| schema_matches(root, branch, value))
    {
        push_diag(
            diagnostics,
            DiagnosticSeverity::Error,
            path,
            title,
            "value must match at least one allowed shape",
        );
    }
    if let Some(one_of) = object.get("oneOf").and_then(JsonValue::as_array) {
        let count = one_of
            .iter()
            .filter(|branch| schema_matches(root, branch, value))
            .count();
        if count != 1 {
            push_diag(
                diagnostics,
                DiagnosticSeverity::Error,
                path,
                title,
                "value must match exactly one allowed shape",
            );
        }
    }

    if let Some(expected) = object.get("const")
        && expected != value
    {
        push_diag(
            diagnostics,
            DiagnosticSeverity::Error,
            path,
            title,
            "value does not match the required constant",
        );
    }
    if let Some(variants) = object.get("enum").and_then(JsonValue::as_array)
        && !variants.iter().any(|variant| variant == value)
    {
        push_diag(
            diagnostics,
            DiagnosticSeverity::Error,
            path,
            title,
            "value is not one of the allowed options",
        );
    }
    if let Some(schema_type) = object.get("type")
        && !value_matches_schema_type(value, schema_type)
    {
        push_diag(
            diagnostics,
            DiagnosticSeverity::Error,
            path,
            title,
            "value does not match declared type",
        );
        return;
    }
    if let Some(if_schema) = object.get("if") {
        let target = if schema_matches(root, if_schema, value) {
            object.get("then")
        } else {
            object.get("else")
        };
        if let Some(target_schema) = target {
            validate_schema_at(diagnostics, root, target_schema, value, path, title);
        }
    }
    if object.get("deprecated").and_then(JsonValue::as_bool) == Some(true) {
        push_diag(
            diagnostics,
            DiagnosticSeverity::Warning,
            path,
            title,
            "field is deprecated",
        );
    }
    if let Some(object_value) = value.as_object() {
        validate_object_schema(diagnostics, root, &schema, object, object_value, path);
    }
    if let Some(array) = value.as_array() {
        validate_array_schema(diagnostics, root, &schema, object, array, path);
    }
    if let Some(text) = value.as_str() {
        validate_string_schema(diagnostics, object, text, path, title);
    }
    if let Some(number) = value.as_f64() {
        validate_number_schema(diagnostics, object, number, path, title);
    }
}

fn validate_object_schema(
    diagnostics: &mut Vec<ConfigDiagnostic>,
    root: &JsonValue,
    schema: &JsonValue,
    schema_object: &JsonMap<String, JsonValue>,
    value: &JsonMap<String, JsonValue>,
    path: &ConfigPath,
) {
    if let Some(patterns) = schema_object
        .get("patternProperties")
        .and_then(JsonValue::as_object)
    {
        for pattern in patterns.keys() {
            if let Err(error) = validate_regex_pattern(pattern) {
                push_diag(
                    diagnostics,
                    DiagnosticSeverity::Error,
                    path,
                    &title_for_schema_or_key(schema, "Object"),
                    format!("invalid patternProperties regex `{pattern}`: {error}").as_str(),
                );
            }
        }
    }
    if let Some(min_properties) = schema_object
        .get("minProperties")
        .and_then(JsonValue::as_u64)
        && value.len() < min_properties as usize
    {
        push_diag(
            diagnostics,
            DiagnosticSeverity::Error,
            path,
            &title_for_schema_or_key(schema, "Object"),
            format!("object must contain at least {min_properties} field(s)").as_str(),
        );
    }
    if let Some(max_properties) = schema_object
        .get("maxProperties")
        .and_then(JsonValue::as_u64)
        && value.len() > max_properties as usize
    {
        push_diag(
            diagnostics,
            DiagnosticSeverity::Error,
            path,
            &title_for_schema_or_key(schema, "Object"),
            format!("object must contain at most {max_properties} field(s)").as_str(),
        );
    }
    let required = schema_object
        .get("required")
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
        .filter_map(JsonValue::as_str)
        .collect::<BTreeSet<_>>();
    for field in required {
        if !value.contains_key(field) {
            let mut child_path = path.clone();
            child_path.push(PathSegment::Key(field.to_owned()));
            push_diag(
                diagnostics,
                DiagnosticSeverity::Error,
                &child_path,
                &title_for_property(root, schema, field),
                "required field is missing",
            );
        }
    }
    if let Some(property_names_schema) = schema_object.get("propertyNames") {
        for key in value.keys() {
            let mut child_path = path.clone();
            child_path.push(PathSegment::Key(key.clone()));
            validate_schema_at(
                diagnostics,
                root,
                property_names_schema,
                &JsonValue::String(key.clone()),
                &child_path,
                format!("{key} name").as_str(),
            );
        }
    }
    let properties = schema_object
        .get("properties")
        .and_then(JsonValue::as_object);
    for (key, child_value) in value {
        let mut child_path = path.clone();
        child_path.push(PathSegment::Key(key.clone()));
        if let Some(child_schema) = object_property_schema(root, schema, key) {
            validate_schema_at(
                diagnostics,
                root,
                &child_schema,
                child_value,
                &child_path,
                &title_for_schema_or_key(&child_schema, key),
            );
        } else if schema_object.get("additionalProperties") == Some(&JsonValue::Bool(false)) {
            if properties.is_none_or(|properties| !properties.contains_key(key)) {
                push_diag(
                    diagnostics,
                    DiagnosticSeverity::Error,
                    &child_path,
                    key,
                    "unexpected property",
                );
            }
        } else if let Some(additional) = schema_object.get("additionalProperties")
            && !matches!(additional, JsonValue::Bool(true))
        {
            validate_schema_at(diagnostics, root, additional, child_value, &child_path, key);
        }
    }
    if let Some(dependencies) = schema_object
        .get("dependentRequired")
        .and_then(JsonValue::as_object)
    {
        for (trigger, required_fields) in dependencies {
            if !value.contains_key(trigger) {
                continue;
            }
            for required in required_fields
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(JsonValue::as_str)
            {
                if value.contains_key(required) {
                    continue;
                }
                let mut child_path = path.clone();
                child_path.push(PathSegment::Key(required.to_owned()));
                push_diag(
                    diagnostics,
                    DiagnosticSeverity::Error,
                    &child_path,
                    &title_for_property(root, schema, required),
                    format!("required because `{trigger}` is set").as_str(),
                );
            }
        }
    }
    if let Some(dependencies) = schema_object
        .get("dependentSchemas")
        .and_then(JsonValue::as_object)
    {
        for (trigger, dependency_schema) in dependencies {
            if value.contains_key(trigger) {
                validate_schema_at(
                    diagnostics,
                    root,
                    dependency_schema,
                    &JsonValue::Object(value.clone()),
                    path,
                    &title_for_schema_or_key(schema, trigger),
                );
            }
        }
    }
}

fn validate_array_schema(
    diagnostics: &mut Vec<ConfigDiagnostic>,
    root: &JsonValue,
    schema: &JsonValue,
    schema_object: &JsonMap<String, JsonValue>,
    value: &[JsonValue],
    path: &ConfigPath,
) {
    if let Some(min_items) = schema_object.get("minItems").and_then(JsonValue::as_u64)
        && value.len() < min_items as usize
    {
        push_diag(
            diagnostics,
            DiagnosticSeverity::Error,
            path,
            &title_for_schema_or_key(schema, "Array"),
            format!("array must contain at least {min_items} item(s)").as_str(),
        );
    }
    if let Some(max_items) = schema_object.get("maxItems").and_then(JsonValue::as_u64)
        && value.len() > max_items as usize
    {
        push_diag(
            diagnostics,
            DiagnosticSeverity::Error,
            path,
            &title_for_schema_or_key(schema, "Array"),
            format!("array must contain at most {max_items} item(s)").as_str(),
        );
    }
    if schema_object
        .get("uniqueItems")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false)
    {
        let mut seen = BTreeSet::new();
        for item in value {
            if !seen.insert(item.to_string()) {
                push_diag(
                    diagnostics,
                    DiagnosticSeverity::Error,
                    path,
                    &title_for_schema_or_key(schema, "Array"),
                    "array contains duplicate items",
                );
                break;
            }
        }
    }
    if let Some(contains_schema) = schema_object.get("contains") {
        let matches = value
            .iter()
            .filter(|item| schema_matches(root, contains_schema, item))
            .count();
        let min_contains = schema_object
            .get("minContains")
            .and_then(JsonValue::as_u64)
            .unwrap_or(1);
        let max_contains = schema_object.get("maxContains").and_then(JsonValue::as_u64);
        if matches < min_contains as usize {
            push_diag(
                diagnostics,
                DiagnosticSeverity::Error,
                path,
                &title_for_schema_or_key(schema, "Array"),
                format!("array must contain at least {min_contains} matching item(s)").as_str(),
            );
        }
        if let Some(max_contains) = max_contains
            && matches > max_contains as usize
        {
            push_diag(
                diagnostics,
                DiagnosticSeverity::Error,
                path,
                &title_for_schema_or_key(schema, "Array"),
                format!("array must contain at most {max_contains} matching item(s)").as_str(),
            );
        }
    }
    for (index, item) in value.iter().enumerate() {
        if let Some(item_schema) = array_item_schema(root, schema, index) {
            let mut child_path = path.clone();
            child_path.push(PathSegment::Index(index));
            validate_schema_at(
                diagnostics,
                root,
                &item_schema,
                item,
                &child_path,
                format!("Item {index}").as_str(),
            );
        }
    }
}

fn validate_string_schema(
    diagnostics: &mut Vec<ConfigDiagnostic>,
    schema_object: &JsonMap<String, JsonValue>,
    text: &str,
    path: &ConfigPath,
    title: &str,
) {
    if let Some(min_length) = schema_object.get("minLength").and_then(JsonValue::as_u64)
        && text.chars().count() < min_length as usize
    {
        push_diag(
            diagnostics,
            DiagnosticSeverity::Error,
            path,
            title,
            format!("must be at least {min_length} characters").as_str(),
        );
    }
    if let Some(max_length) = schema_object.get("maxLength").and_then(JsonValue::as_u64)
        && text.chars().count() > max_length as usize
    {
        push_diag(
            diagnostics,
            DiagnosticSeverity::Error,
            path,
            title,
            format!("must be at most {max_length} characters").as_str(),
        );
    }
    if let Some(format) = schema_object.get("format").and_then(JsonValue::as_str)
        && !format_is_valid(format, text)
    {
        push_diag(
            diagnostics,
            DiagnosticSeverity::Error,
            path,
            title,
            format!("must match format: {format}").as_str(),
        );
    }
    if let Some(pattern) = schema_object.get("pattern").and_then(JsonValue::as_str) {
        match pattern_matches(pattern, text) {
            Ok(true) => {}
            Ok(false) => {
                push_diag(
                    diagnostics,
                    DiagnosticSeverity::Error,
                    path,
                    title,
                    format!("must match pattern: {pattern}").as_str(),
                );
            }
            Err(error) => {
                push_diag(
                    diagnostics,
                    DiagnosticSeverity::Error,
                    path,
                    title,
                    format!("invalid regex pattern `{pattern}`: {error}").as_str(),
                );
            }
        }
    }
}

fn validate_number_schema(
    diagnostics: &mut Vec<ConfigDiagnostic>,
    schema_object: &JsonMap<String, JsonValue>,
    number: f64,
    path: &ConfigPath,
    title: &str,
) {
    if let Some(minimum) = schema_object.get("minimum").and_then(JsonValue::as_f64)
        && number < minimum
    {
        push_diag(
            diagnostics,
            DiagnosticSeverity::Error,
            path,
            title,
            format!("must be >= {minimum}").as_str(),
        );
    }
    if let Some(maximum) = schema_object.get("maximum").and_then(JsonValue::as_f64)
        && number > maximum
    {
        push_diag(
            diagnostics,
            DiagnosticSeverity::Error,
            path,
            title,
            format!("must be <= {maximum}").as_str(),
        );
    }
    if let Some(minimum) = schema_object
        .get("exclusiveMinimum")
        .and_then(JsonValue::as_f64)
        && number <= minimum
    {
        push_diag(
            diagnostics,
            DiagnosticSeverity::Error,
            path,
            title,
            format!("must be > {minimum}").as_str(),
        );
    }
    if let Some(maximum) = schema_object
        .get("exclusiveMaximum")
        .and_then(JsonValue::as_f64)
        && number >= maximum
    {
        push_diag(
            diagnostics,
            DiagnosticSeverity::Error,
            path,
            title,
            format!("must be < {maximum}").as_str(),
        );
    }
    if let Some(multiple_of) = schema_object.get("multipleOf").and_then(JsonValue::as_f64)
        && multiple_of > 0.0
        && (number / multiple_of).fract().abs() > f64::EPSILON
    {
        push_diag(
            diagnostics,
            DiagnosticSeverity::Error,
            path,
            title,
            format!("must be a multiple of {multiple_of}").as_str(),
        );
    }
}

fn schema_matches(root: &JsonValue, schema: &JsonValue, value: &JsonValue) -> bool {
    let mut diagnostics = Vec::new();
    validate_schema_at(&mut diagnostics, root, schema, value, &Vec::new(), "Value");
    diagnostics
        .iter()
        .all(|diagnostic| diagnostic.severity != DiagnosticSeverity::Error)
}

fn push_diag(
    diagnostics: &mut Vec<ConfigDiagnostic>,
    severity: DiagnosticSeverity,
    path: &ConfigPath,
    field: &str,
    message: &str,
) {
    diagnostics.push(ConfigDiagnostic {
        severity,
        source: "config".to_owned(),
        path: path.clone(),
        field: field.to_owned(),
        message: message.to_owned(),
    });
}

fn hostname_format_is_valid(text: &str) -> bool {
    let text = text.trim_end_matches('.');
    if text.is_empty()
        || text.len() > 253
        || text.contains('/')
        || text.chars().any(char::is_whitespace)
    {
        return false;
    }
    text.split('.').all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label
                .chars()
                .all(|char| char.is_ascii_alphanumeric() || char == '-')
    })
}

fn email_format_is_valid(text: &str) -> bool {
    let Some((local, domain)) = text.split_once('@') else {
        return false;
    };
    if local.is_empty()
        || domain.is_empty()
        || text.chars().any(char::is_whitespace)
        || text.matches('@').count() != 1
        || local.starts_with('.')
        || local.ends_with('.')
        || local.contains("..")
    {
        return false;
    }
    let local_valid = local.chars().all(|char| {
        char.is_ascii_alphanumeric()
            || matches!(
                char,
                '!' | '#'
                    | '$'
                    | '%'
                    | '&'
                    | '\''
                    | '*'
                    | '+'
                    | '-'
                    | '/'
                    | '='
                    | '?'
                    | '^'
                    | '_'
                    | '`'
                    | '{'
                    | '|'
                    | '}'
                    | '~'
                    | '.'
            )
    });
    local_valid && (hostname_format_is_valid(domain) || domain.eq_ignore_ascii_case("localhost"))
}

fn format_is_valid(format: &str, text: &str) -> bool {
    match format {
        "uri" | "url" => url::Url::parse(text).is_ok(),
        "email" => email_format_is_valid(text),
        "hostname" => hostname_format_is_valid(text),
        "ipv4" => text.parse::<std::net::Ipv4Addr>().is_ok(),
        "ipv6" => text.parse::<std::net::Ipv6Addr>().is_ok(),
        "uuid" => uuid::Uuid::parse_str(text).is_ok(),
        _ => true,
    }
}

fn validate_regex_pattern(pattern: &str) -> Result<(), regex::Error> {
    Regex::new(pattern).map(|_| ())
}

fn pattern_matches(pattern: &str, text: &str) -> Result<bool, regex::Error> {
    Regex::new(pattern).map(|regex| regex.is_match(text))
}

fn merge_multi_enum_selection(current: &[JsonValue], selected: &[JsonValue]) -> Vec<JsonValue> {
    let mut values = current
        .iter()
        .filter(|value| {
            selected
                .iter()
                .any(|selected_value| selected_value == *value)
        })
        .cloned()
        .collect::<Vec<_>>();
    for selected_value in selected {
        if !values.iter().any(|value| value == selected_value) {
            values.push(selected_value.clone());
        }
    }
    values
}

fn diff_config_values(before: &JsonValue, after: &JsonValue) -> Vec<ConfigDiffRow> {
    let mut rows = Vec::new();
    collect_diff_rows(&mut rows, before, after, &Vec::new());
    rows
}

fn collect_diff_rows(
    rows: &mut Vec<ConfigDiffRow>,
    before: &JsonValue,
    after: &JsonValue,
    path: &ConfigPath,
) {
    if before == after {
        return;
    }
    match (before, after) {
        (JsonValue::Object(left), JsonValue::Object(right)) => {
            let keys = left
                .keys()
                .chain(right.keys())
                .cloned()
                .collect::<BTreeSet<_>>();
            for key in keys {
                let mut child_path = path.clone();
                child_path.push(PathSegment::Key(key.clone()));
                collect_diff_rows(
                    rows,
                    left.get(key.as_str()).unwrap_or(&JsonValue::Null),
                    right.get(key.as_str()).unwrap_or(&JsonValue::Null),
                    &child_path,
                );
            }
        }
        (JsonValue::Array(left), JsonValue::Array(right)) => {
            let max = left.len().max(right.len());
            for index in 0..max {
                let mut child_path = path.clone();
                child_path.push(PathSegment::Index(index));
                collect_diff_rows(
                    rows,
                    left.get(index).unwrap_or(&JsonValue::Null),
                    right.get(index).unwrap_or(&JsonValue::Null),
                    &child_path,
                );
            }
        }
        _ => rows.push(ConfigDiffRow {
            path: path.clone(),
            before: diff_preview(before),
            after: diff_preview(after),
            summary: diff_summary(before, after),
        }),
    }
}

fn schema_has_direct_defaulted_object_fields(schema: &JsonValue, root: &JsonValue) -> bool {
    let schema = resolve_schema(root, schema);
    schema_declared_property_keys(schema)
        .into_iter()
        .any(|key| {
            object_property_schema(root, schema, key.as_str())
                .as_ref()
                .is_some_and(|child_schema| child_schema.get("default").is_some())
        })
}

fn insert_schema_defaults(value: &mut JsonValue, schema: &JsonValue, root: &JsonValue) {
    let schema = active_schema_for_value(root, schema, value);
    if value.is_null() {
        if let Some(default) = schema.get("default") {
            *value = default.clone();
        } else if effective_schema_kind(&schema).as_deref() == Some("object")
            && schema_has_direct_defaulted_object_fields(&schema, root)
        {
            *value = JsonValue::Object(JsonMap::new());
        } else {
            return;
        }
    }
    let kind = effective_schema_kind(&schema);
    if kind.as_deref() == Some("object") {
        let Some(object) = value.as_object_mut() else {
            return;
        };
        for key in schema_declared_property_keys(&schema) {
            let Some(child_schema) = object_property_schema(root, &schema, key.as_str()) else {
                continue;
            };
            if let Some(default) = child_schema.get("default")
                && !object.contains_key(key.as_str())
            {
                object.insert(key.clone(), default.clone());
            }
            if let Some(child) = object.get_mut(key.as_str()) {
                insert_schema_defaults(child, &child_schema, root);
            }
        }
    } else if kind.as_deref() == Some("array")
        && let Some(array) = value.as_array_mut()
    {
        for (index, item) in array.iter_mut().enumerate() {
            if let Some(item_schema) = array_item_schema(root, &schema, index) {
                insert_schema_defaults(item, &item_schema, root);
            }
        }
    }
}

fn default_value_for_schema(schema: &JsonValue, root: &JsonValue) -> JsonValue {
    materialized_value_for_schema(schema, root)
}

fn merge_default_value(target: &mut JsonValue, patch: JsonValue) {
    match (target, patch) {
        (JsonValue::Object(target), JsonValue::Object(patch)) => {
            for (key, value) in patch {
                match target.get_mut(key.as_str()) {
                    Some(existing) => merge_default_value(existing, value),
                    None => {
                        target.insert(key, value);
                    }
                }
            }
        }
        (target, value) if target.is_null() => *target = value,
        _ => {}
    }
}

fn default_value_for_type(kind: &str, schema: Option<&JsonValue>) -> JsonValue {
    match kind {
        "object" => {
            let mut value = JsonValue::Object(JsonMap::new());
            if let Some(schema) = schema {
                insert_schema_defaults(&mut value, schema, schema);
            }
            value
        }
        "array" => JsonValue::Array(Vec::new()),
        "string" => JsonValue::String(String::new()),
        "integer" => JsonValue::Number(JsonNumber::from(0)),
        "number" => JsonValue::Number(JsonNumber::from(0)),
        "boolean" => JsonValue::Bool(false),
        "null" => JsonValue::Null,
        _ => JsonValue::Null,
    }
}

fn schema_for_path(
    root: &JsonValue,
    schema: &JsonValue,
    value: &JsonValue,
    path: &ConfigPath,
) -> Option<JsonValue> {
    let mut current_schema = schema.clone();
    let mut current_value = value;
    for segment in path {
        current_schema = active_schema_for_value(root, &current_schema, current_value);
        match segment {
            PathSegment::Key(key) => {
                current_schema = object_property_schema(root, &current_schema, key)?;
                current_value = current_value.get(key).unwrap_or(&JsonValue::Null);
            }
            PathSegment::Index(index) => {
                current_schema = array_item_schema(root, &current_schema, *index)?;
                current_value = current_value.get(*index).unwrap_or(&JsonValue::Null);
            }
        }
    }
    Some(active_schema_for_value(
        root,
        &current_schema,
        current_value,
    ))
}

fn declared_schema_for_path(
    root: &JsonValue,
    schema: &JsonValue,
    value: &JsonValue,
    path: &ConfigPath,
) -> Option<JsonValue> {
    let mut current_schema = schema.clone();
    let mut current_value = value;
    for segment in path {
        let parent_schema = active_schema_for_value(root, &current_schema, current_value);
        match segment {
            PathSegment::Key(key) => {
                current_schema = object_property_schema(root, &parent_schema, key)?;
                current_value = current_value.get(key).unwrap_or(&JsonValue::Null);
            }
            PathSegment::Index(index) => {
                current_schema = array_item_schema(root, &parent_schema, *index)?;
                current_value = current_value.get(*index).unwrap_or(&JsonValue::Null);
            }
        }
    }
    Some(resolve_schema(root, &current_schema).clone())
}

fn schema_base_without_applicators(
    schema_object: &JsonMap<String, JsonValue>,
) -> Option<JsonValue> {
    let mut base = schema_object.clone();
    for key in ["allOf", "anyOf", "oneOf", "if", "then", "else"] {
        base.remove(key);
    }
    (!base.is_empty()).then_some(JsonValue::Object(base))
}

fn first_matching_branch<'a>(
    root: &JsonValue,
    branches: &'a [JsonValue],
    value: &JsonValue,
) -> Option<&'a JsonValue> {
    branches
        .iter()
        .find(|branch| schema_matches(root, branch, value))
        .or_else(|| branches.first())
}

fn collect_applicable_schema_fragments(
    root: &JsonValue,
    schema: &JsonValue,
    value: &JsonValue,
    fragments: &mut Vec<JsonValue>,
) {
    let schema = resolve_schema(root, schema);
    match schema {
        JsonValue::Bool(_) => fragments.push(schema.clone()),
        JsonValue::Object(object) => {
            if let Some(base) = schema_base_without_applicators(object) {
                fragments.push(base);
            }
            if let Some(all_of) = object.get("allOf").and_then(JsonValue::as_array) {
                for branch in all_of {
                    collect_applicable_schema_fragments(root, branch, value, fragments);
                }
            }
            for key in ["oneOf", "anyOf"] {
                if let Some(branches) = object.get(key).and_then(JsonValue::as_array)
                    && let Some(branch) = first_matching_branch(root, branches, value)
                {
                    collect_applicable_schema_fragments(root, branch, value, fragments);
                }
            }
            if let Some(if_schema) = object.get("if") {
                let target = if schema_matches(root, if_schema, value) {
                    object.get("then")
                } else {
                    object.get("else")
                };
                if let Some(target_schema) = target {
                    collect_applicable_schema_fragments(root, target_schema, value, fragments);
                }
            }
        }
        _ => fragments.push(schema.clone()),
    }
}

fn compose_schema_fragments(mut fragments: Vec<JsonValue>) -> JsonValue {
    fragments.retain(|fragment| !matches!(fragment, JsonValue::Bool(true)));
    if fragments
        .iter()
        .any(|fragment| matches!(fragment, JsonValue::Bool(false)))
    {
        return JsonValue::Bool(false);
    }
    match fragments.len() {
        0 => JsonValue::Bool(true),
        1 => fragments.pop().unwrap_or(JsonValue::Bool(true)),
        _ => {
            let mut object = JsonMap::new();
            object.insert("allOf".to_owned(), JsonValue::Array(fragments));
            JsonValue::Object(object)
        }
    }
}

fn active_schema_for_value(root: &JsonValue, schema: &JsonValue, value: &JsonValue) -> JsonValue {
    let mut fragments = Vec::new();
    collect_applicable_schema_fragments(root, schema, value, &mut fragments);
    compose_schema_fragments(fragments)
}

fn resolve_schema<'a>(root: &'a JsonValue, schema: &'a JsonValue) -> &'a JsonValue {
    let Some(reference) = schema.get("$ref").and_then(JsonValue::as_str) else {
        return schema;
    };
    if !reference.starts_with("#/") {
        return schema;
    }
    let mut cursor = root;
    for segment in reference.trim_start_matches("#/").split('/') {
        let segment = segment.replace("~1", "/").replace("~0", "~");
        let Some(next) = cursor.get(segment.as_str()) else {
            return schema;
        };
        cursor = next;
    }
    cursor
}

fn combine_schema_constraints(mut schemas: Vec<JsonValue>) -> Option<JsonValue> {
    let had_true = schemas
        .iter()
        .any(|schema| matches!(schema, JsonValue::Bool(true)));
    schemas.retain(|schema| !matches!(schema, JsonValue::Bool(true)));
    if schemas
        .iter()
        .any(|schema| matches!(schema, JsonValue::Bool(false)))
    {
        return Some(JsonValue::Bool(false));
    }
    match schemas.len() {
        0 => had_true.then_some(JsonValue::Bool(true)),
        1 => schemas.pop(),
        _ => {
            let mut object = JsonMap::new();
            object.insert("allOf".to_owned(), JsonValue::Array(schemas));
            Some(JsonValue::Object(object))
        }
    }
}

fn direct_object_property_schema(
    root: &JsonValue,
    schema: &JsonValue,
    key: &str,
) -> Option<JsonValue> {
    let schema = resolve_schema(root, schema);
    let mut matches = Vec::new();
    let mut matched_named_or_pattern = false;

    if let Some(properties) = schema.get("properties").and_then(JsonValue::as_object)
        && let Some(child) = properties.get(key)
    {
        matches.push(child.clone());
        matched_named_or_pattern = true;
    }
    if let Some(patterns) = schema
        .get("patternProperties")
        .and_then(JsonValue::as_object)
    {
        for (pattern, child) in patterns {
            if pattern_key_matches(pattern, key) {
                matches.push(child.clone());
                matched_named_or_pattern = true;
            }
        }
    }
    if !matched_named_or_pattern {
        match schema.get("additionalProperties") {
            Some(JsonValue::Object(object)) => matches.push(JsonValue::Object(object.clone())),
            Some(other) if !matches!(other, JsonValue::Bool(true) | JsonValue::Bool(false)) => {
                matches.push(other.clone());
            }
            _ => {}
        }
    }
    combine_schema_constraints(matches)
}

fn object_property_schema(root: &JsonValue, schema: &JsonValue, key: &str) -> Option<JsonValue> {
    let schema = resolve_schema(root, schema);
    let Some(object) = schema.as_object() else {
        return matches!(schema, JsonValue::Bool(false)).then_some(JsonValue::Bool(false));
    };
    let mut matches = Vec::new();
    if let Some(base_match) = direct_object_property_schema(root, schema, key) {
        matches.push(base_match);
    }
    if let Some(all_of) = object.get("allOf").and_then(JsonValue::as_array) {
        for branch in all_of {
            if let Some(branch_match) = object_property_schema(root, branch, key) {
                matches.push(branch_match);
            }
        }
    }
    combine_schema_constraints(matches)
}

fn direct_array_item_schema(
    root: &JsonValue,
    schema: &JsonValue,
    index: usize,
) -> Option<JsonValue> {
    let schema = resolve_schema(root, schema);
    if let Some(prefix) = schema.get("prefixItems").and_then(JsonValue::as_array)
        && let Some(item) = prefix.get(index)
    {
        return Some(item.clone());
    }
    if let Some(items) = schema.get("items") {
        return Some(items.clone());
    }
    (effective_schema_kind(schema).as_deref() == Some("array")).then_some(JsonValue::Bool(true))
}

fn array_item_schema(root: &JsonValue, schema: &JsonValue, index: usize) -> Option<JsonValue> {
    let schema = resolve_schema(root, schema);
    let Some(object) = schema.as_object() else {
        return matches!(schema, JsonValue::Bool(false)).then_some(JsonValue::Bool(false));
    };
    let mut matches = Vec::new();
    if let Some(base_match) = direct_array_item_schema(root, schema, index) {
        matches.push(base_match);
    }
    if let Some(all_of) = object.get("allOf").and_then(JsonValue::as_array) {
        for branch in all_of {
            if let Some(branch_match) = array_item_schema(root, branch, index) {
                matches.push(branch_match);
            }
        }
    }
    combine_schema_constraints(matches)
}

fn branch_choices(root: &JsonValue, schema: &JsonValue) -> Option<Vec<BranchChoice>> {
    let key = if schema.get("oneOf").is_some() {
        "oneOf"
    } else if schema.get("anyOf").is_some() {
        "anyOf"
    } else {
        return None;
    };
    let branches = schema.get(key)?.as_array()?;
    let choices = branches
        .iter()
        .enumerate()
        .map(|(index, schema)| BranchChoice {
            id: format!("branch-{index}"),
            label: branch_label(index, schema),
            schema: resolve_schema(root, schema).clone(),
        })
        .collect::<Vec<_>>();
    (!choices.is_empty()).then_some(choices)
}

fn branch_label(index: usize, schema: &JsonValue) -> String {
    schema
        .get("title")
        .and_then(JsonValue::as_str)
        .map(str::to_owned)
        .or_else(|| {
            schema
                .get("properties")
                .and_then(JsonValue::as_object)
                .and_then(|properties| {
                    properties.iter().find_map(|(key, value)| {
                        value
                            .get("const")
                            .and_then(JsonValue::as_str)
                            .map(|constant| format!("{key}: {constant}"))
                    })
                })
        })
        .or_else(|| effective_schema_kind(schema))
        .unwrap_or_else(|| format!("Branch {}", index + 1))
}

fn active_branch_choice<'a>(
    branches: &'a [BranchChoice],
    value: &JsonValue,
) -> Option<&'a BranchChoice> {
    branches
        .iter()
        .find(|branch| schema_matches(&branch.schema, &branch.schema, value))
        .or_else(|| branches.first())
}

fn active_branch_id<'a>(branches: &'a [BranchChoice], value: &JsonValue) -> &'a str {
    active_branch_choice(branches, value)
        .map(|branch| branch.id.as_str())
        .unwrap_or("branch")
}

fn active_branch_label<'a>(branches: &'a [BranchChoice], value: &JsonValue) -> &'a str {
    active_branch_choice(branches, value)
        .map(|branch| branch.label.as_str())
        .unwrap_or("branch")
}

fn plugin_branch_draft_key(plugin_id: &str, path: &ConfigPath, branch_id: &str) -> String {
    format!("{plugin_id}:{}:{branch_id}", path_display(path))
}

fn generic_json_type_choices() -> Vec<String> {
    vec![
        "string".to_owned(),
        "number".to_owned(),
        "integer".to_owned(),
        "boolean".to_owned(),
        "object".to_owned(),
        "array".to_owned(),
        "null".to_owned(),
    ]
}

fn schema_type_choices(schema: &JsonValue) -> Vec<String> {
    let direct = match schema.get("type") {
        Some(JsonValue::String(kind)) => BTreeSet::from([kind.clone()]),
        Some(JsonValue::Array(items)) => items
            .iter()
            .filter_map(JsonValue::as_str)
            .map(str::to_owned)
            .collect::<BTreeSet<_>>(),
        _ => BTreeSet::new(),
    };
    let Some(all_of) = schema.get("allOf").and_then(JsonValue::as_array) else {
        return direct.into_iter().collect();
    };
    let mut combined = direct;
    let mut branch_types = None::<BTreeSet<String>>;
    for branch in all_of {
        let choices = schema_type_choices(branch)
            .into_iter()
            .collect::<BTreeSet<_>>();
        if choices.is_empty() {
            continue;
        }
        branch_types = Some(match branch_types {
            Some(current) => current.intersection(&choices).cloned().collect(),
            None => choices,
        });
    }
    if let Some(branch_types) = branch_types {
        if combined.is_empty() {
            combined = branch_types;
        } else {
            combined = combined.intersection(&branch_types).cloned().collect();
        }
    }
    combined.into_iter().collect()
}

fn schema_type_selector_choices(schema: Option<&JsonValue>) -> Vec<String> {
    let Some(schema) = schema else {
        return generic_json_type_choices();
    };
    if matches!(schema, JsonValue::Bool(false)) {
        return Vec::new();
    }
    if matches!(schema, JsonValue::Bool(true)) {
        return generic_json_type_choices();
    }
    let choices = schema_type_choices(schema);
    if !choices.is_empty() {
        return choices;
    }
    if let Some(kind) = effective_schema_kind(schema) {
        return vec![kind];
    }
    generic_json_type_choices()
}

fn schema_has_object_shape(schema: &JsonValue) -> bool {
    if schema.as_object().is_some_and(|object| {
        object.contains_key("properties")
            || object.contains_key("patternProperties")
            || object.contains_key("additionalProperties")
            || object.contains_key("propertyNames")
    }) {
        return true;
    }
    schema
        .get("allOf")
        .and_then(JsonValue::as_array)
        .is_some_and(|branches| branches.iter().any(schema_has_object_shape))
}

fn schema_has_array_shape(schema: &JsonValue) -> bool {
    if schema.as_object().is_some_and(|object| {
        object.contains_key("items")
            || object.contains_key("prefixItems")
            || object.contains_key("contains")
            || object.contains_key("minItems")
            || object.contains_key("maxItems")
            || object.contains_key("uniqueItems")
            || object.contains_key("minContains")
            || object.contains_key("maxContains")
    }) {
        return true;
    }
    schema
        .get("allOf")
        .and_then(JsonValue::as_array)
        .is_some_and(|branches| branches.iter().any(schema_has_array_shape))
}

fn effective_schema_kind(schema: &JsonValue) -> Option<String> {
    let type_choices = schema_type_choices(schema);
    if !type_choices.is_empty() {
        return type_choices
            .into_iter()
            .find(|kind| kind != "null")
            .or_else(|| Some("null".to_owned()));
    }
    if schema_has_object_shape(schema) {
        return Some("object".to_owned());
    }
    if schema_has_array_shape(schema) {
        return Some("array".to_owned());
    }
    None
}

fn value_matches_schema_type(value: &JsonValue, schema_type: &JsonValue) -> bool {
    match schema_type {
        JsonValue::String(kind) => value_matches_type(value, kind),
        JsonValue::Array(kinds) => kinds
            .iter()
            .filter_map(JsonValue::as_str)
            .any(|kind| value_matches_type(value, kind)),
        _ => true,
    }
}

fn value_matches_type(value: &JsonValue, kind: &str) -> bool {
    match kind {
        "object" => value.is_object(),
        "array" => value.is_array(),
        "string" => value.is_string(),
        "number" => value.is_number(),
        "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
        "boolean" => value.is_boolean(),
        "null" => value.is_null(),
        _ => true,
    }
}

fn schema_kind_label(schema: &JsonValue) -> String {
    if schema.get("oneOf").is_some() {
        return "oneOf".to_owned();
    }
    if schema.get("anyOf").is_some() {
        return "anyOf".to_owned();
    }
    if schema.get("allOf").is_some() {
        return "allOf".to_owned();
    }
    effective_schema_kind(schema).unwrap_or_else(|| "value".to_owned())
}

fn json_kind_label(value: &JsonValue) -> &'static str {
    match value {
        JsonValue::Null => "null",
        JsonValue::Bool(_) => "boolean",
        JsonValue::Number(number) if number.as_i64().is_some() || number.as_u64().is_some() => {
            "integer"
        }
        JsonValue::Number(_) => "number",
        JsonValue::String(_) => "string",
        JsonValue::Array(_) => "array",
        JsonValue::Object(_) => "object",
    }
}

fn get_value_at_path<'a>(value: &'a JsonValue, path: &ConfigPath) -> Option<&'a JsonValue> {
    let mut cursor = value;
    for segment in path {
        match segment {
            PathSegment::Key(key) => cursor = cursor.get(key)?,
            PathSegment::Index(index) => cursor = cursor.get(*index)?,
        }
    }
    Some(cursor)
}

fn get_value_mut_at_path<'a>(
    value: &'a mut JsonValue,
    path: &ConfigPath,
) -> Option<&'a mut JsonValue> {
    let mut cursor = value;
    for segment in path {
        match segment {
            PathSegment::Key(key) => cursor = cursor.as_object_mut()?.get_mut(key)?,
            PathSegment::Index(index) => cursor = cursor.as_array_mut()?.get_mut(*index)?,
        }
    }
    Some(cursor)
}

fn set_value_at_path(root: &mut JsonValue, path: &ConfigPath, value: JsonValue) {
    if path.is_empty() {
        *root = value;
        return;
    }
    let mut cursor = root;
    for segment in &path[..path.len().saturating_sub(1)] {
        match segment {
            PathSegment::Key(key) => {
                if !cursor.is_object() {
                    *cursor = JsonValue::Object(JsonMap::new());
                }
                cursor = cursor
                    .as_object_mut()
                    .expect("object initialized")
                    .entry(key.clone())
                    .or_insert(JsonValue::Object(JsonMap::new()));
            }
            PathSegment::Index(index) => {
                if !cursor.is_array() {
                    *cursor = JsonValue::Array(Vec::new());
                }
                let array = cursor.as_array_mut().expect("array initialized");
                while array.len() <= *index {
                    array.push(JsonValue::Null);
                }
                cursor = &mut array[*index];
            }
        }
    }
    match path.last().expect("path checked") {
        PathSegment::Key(key) => {
            if !cursor.is_object() {
                *cursor = JsonValue::Object(JsonMap::new());
            }
            cursor
                .as_object_mut()
                .expect("object initialized")
                .insert(key.clone(), value);
        }
        PathSegment::Index(index) => {
            if !cursor.is_array() {
                *cursor = JsonValue::Array(Vec::new());
            }
            let array = cursor.as_array_mut().expect("array initialized");
            while array.len() <= *index {
                array.push(JsonValue::Null);
            }
            array[*index] = value;
        }
    }
}

fn remove_value_at_path(root: &mut JsonValue, path: &ConfigPath) -> Option<JsonValue> {
    let (last, parent_path) = path.split_last()?;
    let parent = get_value_mut_at_path(root, &parent_path.to_vec())?;
    match last {
        PathSegment::Key(key) => parent.as_object_mut()?.remove(key),
        PathSegment::Index(index) => {
            let array = parent.as_array_mut()?;
            (*index < array.len()).then(|| array.remove(*index))
        }
    }
}

fn array_item_path_info(
    value: &JsonValue,
    path: &[PathSegment],
) -> Option<(ConfigPath, usize, usize)> {
    let (last, parent_path) = path.split_last()?;
    let PathSegment::Index(index) = last else {
        return None;
    };
    let parent_path = parent_path.to_vec();
    let len = get_value_at_path(value, &parent_path)?.as_array()?.len();
    (*index < len).then_some((parent_path, *index, len))
}

fn path_key_info(path: &[PathSegment]) -> Option<(ConfigPath, String)> {
    let (last, parent_path) = path.split_last()?;
    let PathSegment::Key(key) = last else {
        return None;
    };
    Some((parent_path.to_vec(), key.clone()))
}

fn replace_last_index(path: &[PathSegment], new_index: usize) -> ConfigPath {
    let mut next = path.to_vec();
    if let Some(last) = next.last_mut() {
        *last = PathSegment::Index(new_index);
    }
    next
}

#[derive(Debug, Clone, Copy)]
struct ArrayItemActionInfo {
    can_insert_before: bool,
    can_insert_after: bool,
    can_duplicate: bool,
    can_move_up: bool,
    can_move_down: bool,
    can_remove: bool,
}

impl ArrayItemActionInfo {
    fn has_any_action(self) -> bool {
        self.can_insert_before
            || self.can_insert_after
            || self.can_duplicate
            || self.can_move_up
            || self.can_move_down
            || self.can_remove
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConfigRowPrimaryAction {
    InsertAfter,
    Duplicate,
    MoveDown,
    MoveUp,
    Remove,
    AddField,
    AddItem,
    Rename,
}

impl ConfigRowPrimaryAction {
    fn plain_label(self) -> &'static str {
        match self {
            Self::InsertAfter => "Insert",
            Self::Duplicate => "Duplicate",
            Self::MoveDown => "Move down",
            Self::MoveUp => "Move up",
            Self::Remove => "Remove",
            Self::AddField => "Add field",
            Self::AddItem => "Add item",
            Self::Rename => "Rename",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::InsertAfter => "[ Insert ]",
            Self::Duplicate => "[ Duplicate ]",
            Self::MoveDown => "[ Move down ]",
            Self::MoveUp => "[ Move up ]",
            Self::Remove => "[ Remove ]",
            Self::AddField => "[ Add field ]",
            Self::AddItem => "[ Add item ]",
            Self::Rename => "[ Rename ]",
        }
    }
}

fn trim_action_display(label: &str) -> &str {
    label
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .trim()
}

fn action_matches_primary(action: &PluginConfigAction, primary: ConfigRowPrimaryAction) -> bool {
    matches!(
        (action, primary),
        (
            PluginConfigAction::AppendArrayItem { .. },
            ConfigRowPrimaryAction::AddItem
        ) | (
            PluginConfigAction::PromptAddObjectField { .. },
            ConfigRowPrimaryAction::AddField
        ) | (
            PluginConfigAction::InsertArrayItemAfter { .. },
            ConfigRowPrimaryAction::InsertAfter
        ) | (
            PluginConfigAction::DuplicateArrayItem { .. },
            ConfigRowPrimaryAction::Duplicate
        ) | (
            PluginConfigAction::MoveArrayItem { direction: 1, .. },
            ConfigRowPrimaryAction::MoveDown
        ) | (
            PluginConfigAction::MoveArrayItem { direction: -1, .. },
            ConfigRowPrimaryAction::MoveUp
        ) | (
            PluginConfigAction::RemoveArrayItem { .. },
            ConfigRowPrimaryAction::Remove
        ) | (
            PluginConfigAction::RenameField { .. },
            ConfigRowPrimaryAction::Rename
        )
    )
}

fn action_is_select_type(action: &PluginConfigAction) -> bool {
    matches!(action, PluginConfigAction::SelectType { .. })
}

fn action_is_reset_field(action: &PluginConfigAction) -> bool {
    matches!(action, PluginConfigAction::ResetField { .. })
}

fn action_priority_for_focus(
    action: &PluginConfigAction,
    focused_cell: ConfigRowCell,
    primary_action: Option<ConfigRowPrimaryAction>,
) -> (u8, u8) {
    let base = match action {
        PluginConfigAction::SelectType { .. } => 0,
        PluginConfigAction::AppendArrayItem { .. } => 1,
        PluginConfigAction::PromptAddObjectField { .. } => 2,
        PluginConfigAction::InsertArrayItemBefore { .. } => 3,
        PluginConfigAction::InsertArrayItemAfter { .. } => 4,
        PluginConfigAction::DuplicateArrayItem { .. } => 5,
        PluginConfigAction::MoveArrayItem { direction: -1, .. } => 6,
        PluginConfigAction::MoveArrayItem { direction: 1, .. } => 7,
        PluginConfigAction::RemoveArrayItem { .. } => 8,
        PluginConfigAction::RenameField { .. } => 9,
        PluginConfigAction::ResetField { .. } => 10,
        PluginConfigAction::ResetGroup { .. } => 11,
        PluginConfigAction::MoveArrayItem { .. } => 12,
    };
    let rank = if focused_cell == ConfigRowCell::Type && action_is_select_type(action) {
        0
    } else if focused_cell == ConfigRowCell::Default && action_is_reset_field(action) {
        0
    } else if primary_action.is_some_and(|primary| action_matches_primary(action, primary)) {
        if focused_cell == ConfigRowCell::Action || focused_cell == ConfigRowCell::State {
            0
        } else {
            1
        }
    } else if action_is_select_type(action) {
        2
    } else if matches!(
        action,
        PluginConfigAction::AppendArrayItem { .. }
            | PluginConfigAction::PromptAddObjectField { .. }
            | PluginConfigAction::InsertArrayItemBefore { .. }
            | PluginConfigAction::InsertArrayItemAfter { .. }
            | PluginConfigAction::DuplicateArrayItem { .. }
            | PluginConfigAction::MoveArrayItem { .. }
    ) {
        3
    } else if matches!(action, PluginConfigAction::RenameField { .. }) {
        4
    } else if action_is_reset_field(action) {
        5
    } else {
        6
    };
    (rank, base)
}

fn prioritize_config_actions(
    actions: &mut [PluginConfigActionItem],
    focused_cell: ConfigRowCell,
    primary_action: Option<ConfigRowPrimaryAction>,
) -> usize {
    actions
        .sort_by_key(|item| action_priority_for_focus(&item.action, focused_cell, primary_action));
    0
}

fn config_actions_overlay_footer(primary_action: Option<ConfigRowPrimaryAction>) -> String {
    let base = "Enter apply  Esc close  Up/Down move";
    primary_action
        .map(|action| format!("{base}  Action: {}", action.plain_label()))
        .unwrap_or_else(|| base.to_owned())
}

#[derive(Debug, Default)]
struct ResetPathsOutcome {
    changed: bool,
    blocked: Vec<String>,
}

fn schema_unique_items(schema: &JsonValue) -> bool {
    schema_bool_keyword_any(schema, "uniqueItems")
}

fn clear_branch_drafts_for_structural_change(plugin: &mut PluginWorkbenchPlugin) {
    plugin.branch_drafts.clear();
}

fn schema_allows_dynamic_object_keys(schema: &JsonValue) -> bool {
    let direct = match schema.get("additionalProperties") {
        Some(JsonValue::Bool(false)) => schema
            .get("patternProperties")
            .and_then(JsonValue::as_object)
            .is_some_and(|patterns| !patterns.is_empty()),
        _ => true,
    };
    if let Some(all_of) = schema.get("allOf").and_then(JsonValue::as_array) {
        direct && all_of.iter().all(schema_allows_dynamic_object_keys)
    } else {
        direct
    }
}

fn object_add_field_block_reason(
    root_schema: Option<&JsonValue>,
    config: &JsonValue,
    object_path: &ConfigPath,
) -> Option<String> {
    let object = get_value_at_path(config, object_path)?.as_object()?;
    let Some(root_schema) = root_schema else {
        return None;
    };
    let Some(parent_schema) = schema_for_path(root_schema, root_schema, config, object_path) else {
        return None;
    };
    let max_properties =
        schema_max_u64_constraint(&parent_schema, "maxProperties").map(|value| value as usize);
    max_properties
        .filter(|max_properties| object.len() >= *max_properties)
        .map(|max_properties| {
            format!("object already has the maximum of {max_properties} field(s)")
        })
        .or_else(|| {
            (!schema_allows_dynamic_object_keys(&parent_schema))
                .then_some("schema does not allow adding custom field names".to_owned())
        })
}

fn write_path_block_reason(
    root_schema: Option<&JsonValue>,
    config: &JsonValue,
    path: &[PathSegment],
) -> Option<String> {
    let Some(root_schema) = root_schema else {
        return None;
    };
    let mut current_value = config;
    let mut prefix = Vec::new();
    for segment in path {
        match segment {
            PathSegment::Key(key) => {
                let object = current_value.as_object();
                let present = object.is_some_and(|object| object.contains_key(key));
                if !present {
                    let object_len = object.map(|object| object.len()).unwrap_or_default();
                    if let Some(parent_schema) =
                        schema_for_path(root_schema, root_schema, config, &prefix)
                        && let Some(max_properties) =
                            schema_max_u64_constraint(&parent_schema, "maxProperties")
                                .map(|value| value as usize)
                        && object_len >= max_properties
                    {
                        return Some(if prefix.is_empty() {
                            format!("object already has the maximum of {max_properties} field(s)")
                        } else {
                            format!(
                                "{} already has the maximum of {max_properties} field(s)",
                                path_display(&prefix)
                            )
                        });
                    }
                }
                current_value = object
                    .and_then(|object| object.get(key))
                    .unwrap_or(&JsonValue::Null);
            }
            PathSegment::Index(index) => {
                current_value = current_value
                    .as_array()
                    .and_then(|items| items.get(*index))
                    .unwrap_or(&JsonValue::Null);
            }
        }
        prefix.push(segment.clone());
    }
    None
}

fn apply_staged_config_value_updates(
    root_schema: Option<&JsonValue>,
    current_config: &JsonValue,
    updates: &[(ConfigPath, JsonValue)],
) -> UiResult<JsonValue> {
    let mut next_config = current_config.clone();
    for (path, value) in updates {
        if let Some(reason) = write_path_block_reason(root_schema, &next_config, path.as_slice()) {
            return Err(reason);
        }
        set_value_at_path(&mut next_config, path, value.clone());
    }
    Ok(next_config)
}

fn validate_container_candidate(
    root_schema: &JsonValue,
    current_root: &JsonValue,
    container_path: &ConfigPath,
    candidate_container: JsonValue,
) -> Option<String> {
    let mut candidate_root = current_root.clone();
    set_value_at_path(&mut candidate_root, container_path, candidate_container);
    let candidate_schema =
        schema_for_path(root_schema, root_schema, &candidate_root, container_path)
            .or_else(|| schema_for_path(root_schema, root_schema, current_root, container_path))?;
    let candidate_value = get_value_at_path(&candidate_root, container_path)
        .cloned()
        .unwrap_or(JsonValue::Null);
    let title = title_from_path(container_path.as_slice());
    validate_schema_value_for_path(
        root_schema,
        &candidate_schema,
        &candidate_value,
        container_path,
        title.as_str(),
    )
    .err()
}

fn validate_reset_candidate(
    root_schema: Option<&JsonValue>,
    current_root: &JsonValue,
    path: &ConfigPath,
) -> Option<String> {
    let Some(root_schema) = root_schema else {
        return None;
    };
    let (_, parent_path) = path.split_last()?;
    let container_path = parent_path.to_vec();
    let mut candidate_root = current_root.clone();
    remove_value_at_path(&mut candidate_root, path)?;
    let candidate_container = get_value_at_path(&candidate_root, &container_path)
        .cloned()
        .unwrap_or(JsonValue::Null);
    validate_container_candidate(
        root_schema,
        current_root,
        &container_path,
        candidate_container,
    )
}

fn prepared_default_array_item_value(
    current_root: &JsonValue,
    root_schema: Option<&JsonValue>,
    parent_path: &ConfigPath,
    insert_index: usize,
) -> Option<JsonValue> {
    let array = get_value_at_path(current_root, parent_path)?.as_array()?;
    if insert_index > array.len() {
        return None;
    }
    let value = if let Some(root_schema) = root_schema {
        let Some(parent_schema) =
            schema_for_path(root_schema, root_schema, current_root, parent_path)
        else {
            return Some(JsonValue::Null);
        };
        let max_items =
            schema_max_u64_constraint(&parent_schema, "maxItems").map(|value| value as usize);
        if max_items.is_some_and(|max_items| array.len() >= max_items) {
            return None;
        }
        let item_schema = array_item_schema(root_schema, &parent_schema, insert_index)?;
        if matches!(item_schema, JsonValue::Bool(false)) {
            return None;
        }
        let value = default_value_for_schema(&item_schema, root_schema);
        let mut candidate_items = array.clone();
        candidate_items.insert(insert_index, value.clone());
        if validate_container_candidate(
            root_schema,
            current_root,
            parent_path,
            JsonValue::Array(candidate_items),
        )
        .is_some()
        {
            return None;
        }
        value
    } else {
        JsonValue::Null
    };
    Some(value)
}

fn can_duplicate_array_item(
    root_schema: Option<&JsonValue>,
    current_root: &JsonValue,
    path: &[PathSegment],
) -> bool {
    let Some((parent_path, index, len)) = array_item_path_info(current_root, path) else {
        return false;
    };
    let Some(array) = get_value_at_path(current_root, &parent_path).and_then(JsonValue::as_array)
    else {
        return false;
    };
    if index >= len {
        return false;
    }
    if let Some(root_schema) = root_schema {
        let mut candidate_items = array.clone();
        let Some(clone) = candidate_items.get(index).cloned() else {
            return false;
        };
        candidate_items.insert(index + 1, clone);
        validate_container_candidate(
            root_schema,
            current_root,
            &parent_path,
            JsonValue::Array(candidate_items),
        )
        .is_none()
    } else {
        true
    }
}

fn can_remove_array_item(
    root_schema: Option<&JsonValue>,
    current_root: &JsonValue,
    path: &[PathSegment],
) -> bool {
    let Some((parent_path, index, len)) = array_item_path_info(current_root, path) else {
        return false;
    };
    let Some(array) = get_value_at_path(current_root, &parent_path).and_then(JsonValue::as_array)
    else {
        return false;
    };
    if index >= len {
        return false;
    }
    if let Some(root_schema) = root_schema {
        let mut candidate_items = array.clone();
        candidate_items.remove(index);
        validate_container_candidate(
            root_schema,
            current_root,
            &parent_path,
            JsonValue::Array(candidate_items),
        )
        .is_none()
    } else {
        true
    }
}

fn validate_schema_value_for_path(
    root_schema: &JsonValue,
    schema: &JsonValue,
    value: &JsonValue,
    path: &ConfigPath,
    title: &str,
) -> UiResult<()> {
    let mut diagnostics = Vec::new();
    validate_schema_at(&mut diagnostics, root_schema, schema, value, path, title);
    if let Some(error) = diagnostics
        .into_iter()
        .find(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error)
    {
        return Err(error.message);
    }
    Ok(())
}

fn reset_path_block_reason(
    root_schema: Option<&JsonValue>,
    default_root: &JsonValue,
    current_root: &JsonValue,
    path: &ConfigPath,
) -> Option<String> {
    if get_value_at_path(default_root, path).is_some() {
        return None;
    }
    if get_value_at_path(current_root, path).is_none() {
        return None;
    }
    if let Some((parent_path, key)) = path_key_info(path.as_slice()) {
        let object = get_value_at_path(current_root, &parent_path)?.as_object()?;
        let Some(root_schema) = root_schema else {
            return None;
        };
        let Some(parent_schema) =
            schema_for_path(root_schema, root_schema, current_root, &parent_path)
        else {
            return None;
        };
        if schema_required_fields(&parent_schema).contains(key.as_str()) {
            return Some("field is required and has no default".to_owned());
        }
        let min_properties =
            schema_min_u64_constraint(&parent_schema, "minProperties").unwrap_or(0) as usize;
        if object.len() <= min_properties {
            return Some(format!(
                "object requires at least {min_properties} field(s)"
            ));
        }
        return validate_reset_candidate(Some(root_schema), current_root, path);
    }
    let (parent_path, index, len) = array_item_path_info(current_root, path.as_slice())?;
    let Some(root_schema) = root_schema else {
        return None;
    };
    let Some(parent_schema) = schema_for_path(root_schema, root_schema, current_root, &parent_path)
    else {
        return None;
    };
    let tuple_prefix_len = schema_prefix_item_count(&parent_schema);
    if index < tuple_prefix_len {
        return Some("tuple item has no removable default slot".to_owned());
    }
    let min_items = schema_min_u64_constraint(&parent_schema, "minItems").unwrap_or(0) as usize;
    if len <= min_items {
        return Some(format!("array requires at least {min_items} item(s)"));
    }
    validate_reset_candidate(Some(root_schema), current_root, path)
}

fn compare_reset_paths(left: &ConfigPath, right: &ConfigPath) -> std::cmp::Ordering {
    use std::cmp::Ordering;

    for (left_segment, right_segment) in left.iter().zip(right.iter()) {
        match (left_segment, right_segment) {
            (PathSegment::Index(left_index), PathSegment::Index(right_index))
                if left_index != right_index =>
            {
                return right_index.cmp(left_index);
            }
            (PathSegment::Key(left_key), PathSegment::Key(right_key)) if left_key != right_key => {
                return left_key.cmp(right_key);
            }
            (PathSegment::Key(_), PathSegment::Index(_)) => return Ordering::Less,
            (PathSegment::Index(_), PathSegment::Key(_)) => return Ordering::Greater,
            _ => {}
        }
    }
    right.len().cmp(&left.len())
}

fn normalized_reset_paths(paths: &[ConfigPath]) -> Vec<ConfigPath> {
    let mut paths = paths.to_vec();
    paths.sort_by(compare_reset_paths);
    paths.dedup();
    paths
}

fn apply_reset_paths(
    value: &mut JsonValue,
    default_root: &JsonValue,
    root_schema: Option<&JsonValue>,
    paths: &[ConfigPath],
) -> ResetPathsOutcome {
    let mut outcome = ResetPathsOutcome::default();
    for path in normalized_reset_paths(paths) {
        if let Some(reason) = reset_path_block_reason(root_schema, default_root, value, &path) {
            outcome
                .blocked
                .push(format!("{}: {reason}", path_display(&path)));
            continue;
        }
        outcome.changed |= reset_effective_value_at_path(value, default_root, path.as_slice());
    }
    outcome
}

fn reset_paths_warning_message(blocked: &[String]) -> Option<String> {
    let first = blocked.first()?;
    Some(if blocked.len() == 1 {
        format!("cannot reset {first}")
    } else {
        format!("some fields could not be reset (showing first): {first}")
    })
}

fn generic_array_item_action_info(index: usize, len: usize) -> ArrayItemActionInfo {
    ArrayItemActionInfo {
        can_insert_before: true,
        can_insert_after: true,
        can_duplicate: true,
        can_move_up: index > 0,
        can_move_down: index + 1 < len,
        can_remove: len > 0,
    }
}

fn duplicate_array_item_at_path(root: &mut JsonValue, path: &[PathSegment]) -> Option<ConfigPath> {
    let (parent_path, index, len) = array_item_path_info(root, path)?;
    let array = get_value_mut_at_path(root, &parent_path)?.as_array_mut()?;
    if index >= len {
        return None;
    }
    let clone = array.get(index)?.clone();
    let next_index = index + 1;
    array.insert(next_index, clone);
    Some(replace_last_index(path, next_index))
}

fn move_array_item_at_path(
    root: &mut JsonValue,
    path: &[PathSegment],
    direction: isize,
) -> Option<ConfigPath> {
    let (parent_path, index, len) = array_item_path_info(root, path)?;
    let target_index =
        (index as isize + direction).clamp(0, len.saturating_sub(1) as isize) as usize;
    if target_index == index {
        return Some(path.to_vec());
    }
    let array = get_value_mut_at_path(root, &parent_path)?.as_array_mut()?;
    array.swap(index, target_index);
    Some(replace_last_index(path, target_index))
}

fn remove_array_item_at_path(root: &mut JsonValue, path: &[PathSegment]) -> Option<ConfigPath> {
    let (parent_path, index, len) = array_item_path_info(root, path)?;
    let array = get_value_mut_at_path(root, &parent_path)?.as_array_mut()?;
    if index >= len {
        return None;
    }
    array.remove(index);
    if array.is_empty() {
        Some(parent_path)
    } else if index >= array.len() {
        Some(replace_last_index(path, array.len().saturating_sub(1)))
    } else {
        Some(replace_last_index(path, index))
    }
}

fn rename_object_field_at_path(
    root: &mut JsonValue,
    path: &[PathSegment],
    new_key: &str,
) -> Option<ConfigPath> {
    let (parent_path, current_key) = path_key_info(path)?;
    let object = get_value_mut_at_path(root, &parent_path)?.as_object_mut()?;
    let value = object.remove(current_key.as_str())?;
    object.insert(new_key.to_owned(), value);
    let mut next_path = parent_path;
    next_path.push(PathSegment::Key(new_key.to_owned()));
    Some(next_path)
}

fn validate_new_object_field_key(
    root_schema: Option<&JsonValue>,
    config: &JsonValue,
    object_path: &ConfigPath,
    key: &str,
) -> UiResult<Option<JsonValue>> {
    let Some(root_schema) = root_schema else {
        return Ok(None);
    };
    let Some(parent_schema) = schema_for_path(root_schema, root_schema, config, object_path) else {
        return Ok(None);
    };
    for property_names_schema in schema_property_name_schemas(&parent_schema) {
        let mut diagnostics = Vec::new();
        let mut key_path = object_path.clone();
        key_path.push(PathSegment::Key(key.to_owned()));
        validate_schema_at(
            &mut diagnostics,
            root_schema,
            &property_names_schema,
            &JsonValue::String(key.to_owned()),
            &key_path,
            format!("{key} name").as_str(),
        );
        if let Some(error) = diagnostics
            .into_iter()
            .find(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error)
        {
            return Err(error.message);
        }
    }
    let child_schema = object_property_schema(root_schema, &parent_schema, key);
    if matches!(child_schema, Some(JsonValue::Bool(false)))
        || (child_schema.is_none() && schema_prohibits_additional_properties(&parent_schema))
    {
        return Err(format!("field `{key}` is not allowed by this schema"));
    }
    Ok(child_schema)
}

fn array_item_action_info(
    plugin: &PluginWorkbenchPlugin,
    path: &[PathSegment],
) -> Option<ArrayItemActionInfo> {
    let (parent_path, index, len) = array_item_path_info(&plugin.draft_config, path)?;
    let Some(root_schema) = plugin.schema.as_ref() else {
        return Some(generic_array_item_action_info(index, len));
    };
    let Some(parent_schema) =
        schema_for_path(root_schema, root_schema, &plugin.draft_config, &parent_path)
    else {
        return Some(generic_array_item_action_info(index, len));
    };
    let tuple_prefix_len = schema_prefix_item_count(&parent_schema);
    let tuple_slot = index < tuple_prefix_len;
    let min_items = schema_min_u64_constraint(&parent_schema, "minItems").unwrap_or(0) as usize;
    let can_insert_before = !tuple_slot
        && prepared_default_array_item_value(
            &plugin.draft_config,
            plugin.schema.as_ref(),
            &parent_path,
            index,
        )
        .is_some();
    let can_insert_after = !tuple_slot
        && prepared_default_array_item_value(
            &plugin.draft_config,
            plugin.schema.as_ref(),
            &parent_path,
            index + 1,
        )
        .is_some();
    Some(ArrayItemActionInfo {
        can_insert_before,
        can_insert_after,
        can_duplicate: can_insert_after
            && !schema_unique_items(&parent_schema)
            && can_duplicate_array_item(plugin.schema.as_ref(), &plugin.draft_config, path),
        can_move_up: !tuple_slot && index > tuple_prefix_len,
        can_move_down: !tuple_slot && index + 1 < len,
        can_remove: !tuple_slot
            && len > min_items
            && can_remove_array_item(plugin.schema.as_ref(), &plugin.draft_config, path),
    })
}

fn can_append_array_item(plugin: &PluginWorkbenchPlugin, path: &[PathSegment]) -> bool {
    let path = path.to_vec();
    let Some(len) = get_value_at_path(&plugin.draft_config, &path)
        .and_then(JsonValue::as_array)
        .map(|items| items.len())
    else {
        return false;
    };
    prepared_default_array_item_value(&plugin.draft_config, plugin.schema.as_ref(), &path, len)
        .is_some()
}

fn append_default_array_item_at_path(
    root: &mut JsonValue,
    root_schema: Option<&JsonValue>,
    path: &[PathSegment],
) -> Option<ConfigPath> {
    let path = path.to_vec();
    let len = get_value_at_path(root, &path)?.as_array()?.len();
    let value = prepared_default_array_item_value(root, root_schema, &path, len)?;
    let array = get_value_mut_at_path(root, &path)?.as_array_mut()?;
    array.push(value);
    let mut focus_path = path;
    focus_path.push(PathSegment::Index(len));
    Some(focus_path)
}

fn insert_default_array_item_at_path(
    root: &mut JsonValue,
    root_schema: Option<&JsonValue>,
    path: &[PathSegment],
    after: bool,
) -> Option<ConfigPath> {
    let (parent_path, index, _len) = array_item_path_info(root, path)?;
    let insert_index = if after { index + 1 } else { index };
    if let Some(root_schema) = root_schema
        && let Some(parent_schema) = schema_for_path(root_schema, root_schema, root, &parent_path)
        && index < schema_prefix_item_count(&parent_schema)
    {
        return None;
    }
    let value = prepared_default_array_item_value(root, root_schema, &parent_path, insert_index)?;
    let array = get_value_mut_at_path(root, &parent_path)?.as_array_mut()?;
    array.insert(insert_index, value);
    let mut focus_path = parent_path;
    focus_path.push(PathSegment::Index(insert_index));
    Some(focus_path)
}

fn path_display(path: &ConfigPath) -> String {
    if path.is_empty() {
        return "/".to_owned();
    }
    let mut out = String::new();
    for segment in path {
        match segment {
            PathSegment::Key(key) => {
                out.push('/');
                out.push_str(key);
            }
            PathSegment::Index(index) => {
                out.push('[');
                out.push_str(index.to_string().as_str());
                out.push(']');
            }
        }
    }
    out
}

fn title_for_property(root: &JsonValue, schema: &JsonValue, key: &str) -> String {
    object_property_schema(root, schema, key)
        .map(|schema| title_for_schema_or_key(&schema, key))
        .unwrap_or_else(|| title_from_key(key))
}

fn title_for_schema_or_key(schema: &JsonValue, key: &str) -> String {
    schema
        .get("title")
        .and_then(JsonValue::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| title_from_key(key))
}

fn title_from_key(key: &str) -> String {
    let mut out = String::new();
    for (index, part) in key
        .split(['_', '-', '.'])
        .filter(|part| !part.is_empty())
        .enumerate()
    {
        if index > 0 {
            out.push(' ');
        }
        let mut chars = part.chars();
        if let Some(first) = chars.next() {
            out.push(first.to_ascii_uppercase());
            out.push_str(chars.as_str());
        }
    }
    if out.is_empty() { key.to_owned() } else { out }
}

fn title_from_path(path: &[PathSegment]) -> String {
    path.iter()
        .rev()
        .find_map(path_segment_key_name)
        .map(title_from_key)
        .unwrap_or_else(|| "Value".to_owned())
}

fn preview_value(value: &JsonValue) -> String {
    match value {
        JsonValue::Null => "null".to_owned(),
        JsonValue::Bool(value) => {
            if *value {
                "yes".to_owned()
            } else {
                "no".to_owned()
            }
        }
        JsonValue::Number(number) => number.to_string(),
        JsonValue::String(text) => truncate_text(text, 40),
        JsonValue::Array(items) => format!("{} item(s)", items.len()),
        JsonValue::Object(object) => format!("{} field(s)", object.len()),
    }
}

fn diff_preview(value: &JsonValue) -> String {
    if value.is_null() {
        "missing".to_owned()
    } else {
        preview_value(value)
    }
}

fn diff_summary(before: &JsonValue, after: &JsonValue) -> String {
    match (before, after) {
        (JsonValue::Null, _) => "added".to_owned(),
        (_, JsonValue::Null) => "removed".to_owned(),
        (JsonValue::Object(_), JsonValue::Object(_)) => "modified object".to_owned(),
        (JsonValue::Array(_), JsonValue::Array(_)) => "modified array".to_owned(),
        _ => "changed".to_owned(),
    }
}

fn parse_scalar_editor_value(kind: ScalarEditKind, input: &str) -> UiResult<JsonValue> {
    match kind {
        ScalarEditKind::String => Ok(JsonValue::String(input.to_owned())),
        ScalarEditKind::Number => {
            let parsed = input
                .trim()
                .parse::<f64>()
                .map_err(|error| format!("invalid number: {error}"))?;
            let Some(number) = JsonNumber::from_f64(parsed) else {
                return Err("number cannot be NaN or infinite".to_owned());
            };
            Ok(JsonValue::Number(number))
        }
        ScalarEditKind::Integer => {
            let parsed = input
                .trim()
                .parse::<i64>()
                .map_err(|error| format!("invalid integer: {error}"))?;
            Ok(JsonValue::Number(JsonNumber::from(parsed)))
        }
    }
}

fn parse_pair_integer_editor_values(input: &str) -> UiResult<(i64, i64)> {
    let parts = input
        .lines()
        .flat_map(|line| line.split(','))
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    if parts.len() != 2 {
        return Err("expected two integer values".to_owned());
    }
    let left = parts[0]
        .parse::<i64>()
        .map_err(|error| format!("invalid first integer: {error}"))?;
    let right = parts[1]
        .parse::<i64>()
        .map_err(|error| format!("invalid second integer: {error}"))?;
    Ok((left, right))
}

fn field_prompt_for_row(schema: Option<&JsonValue>, row: &ConfigRowView) -> String {
    let mut parts = vec![format!("Path: {}", path_display(&row.primary_path))];
    if let Some(description) = row.description.as_deref() {
        parts.push(description.to_owned());
    } else if let Some(schema) = schema {
        if let Some(description) = schema_description_text(schema) {
            parts.push(description.to_owned());
        }
    }
    if let Some(schema) = schema {
        if let Some(format) = schema_first_string_keyword(schema, "format") {
            parts.push(format!("format: {format}"));
        }
    }
    parts.extend(row.constraints.iter().cloned());
    parts.join("\n")
}

fn field_prompt_for_path(plugin: &PluginWorkbenchPlugin, path: &ConfigPath) -> String {
    let schema = plugin
        .schema
        .as_ref()
        .and_then(|root| declared_schema_for_path(root, root, &plugin.draft_config, path));
    let mut parts = vec![format!("Path: {}", path_display(path))];
    if let Some(description) = path_description(plugin, path) {
        parts.push(description);
    } else if let Some(schema) = schema.as_ref()
        && let Some(description) = schema_description_text(schema)
    {
        parts.push(description);
    }
    if let Some(schema) = schema.as_ref()
        && let Some(format) = schema_first_string_keyword(schema, "format")
    {
        parts.push(format!("format: {format}"));
    }
    parts.extend(path_constraints(plugin, path));
    parts.join("\n")
}

fn schema_string_is_multiline(schema: &JsonValue) -> bool {
    schema_first_string_keyword(schema, "format")
        .is_some_and(|format| matches!(format, "markdown" | "multiline" | "textarea"))
        || schema
            .get("maxLength")
            .and_then(JsonValue::as_u64)
            .is_some_and(|max| max > 240)
}

fn pattern_key_matches(pattern: &str, key: &str) -> bool {
    pattern_matches(pattern, key).unwrap_or(false)
}

fn plugin_header_text(plugin: &PluginWorkbenchPlugin) -> Text<'static> {
    Text::from(vec![
        Line::from(format!(
            "{}        {}        v{}        {}",
            clean(plugin.plugin_id.as_str()),
            clean(plugin.visible_tool.as_str()),
            clean(plugin.version.as_str()),
            clean(plugin.transport.as_str())
        )),
        Line::from(format!(
            "Tools: {}        Commands: {}        Config: {}",
            plugin.tools.len(),
            plugin.commands.len(),
            clean(plugin.config_status.label.as_str())
        )),
        Line::from(format!(
            "Package: {}",
            plugin
                .configured_plugin_value
                .as_ref()
                .and_then(|configured_plugin| configured_plugin.get("package"))
                .map(plugin_package_preview)
                .unwrap_or_else(|| "unavailable".to_owned())
        )),
    ])
}

fn plugin_tools_text(plugin: &PluginWorkbenchPlugin) -> Text<'static> {
    if plugin.tools.is_empty() {
        return Text::from("No tools.");
    }
    let mut lines = vec![Line::from(Span::styled(
        fixed_columns(&[("Tool", 30), ("Description", 68), ("Inputs", 8)], 112),
        Style::default().add_modifier(Modifier::BOLD),
    ))];
    for tool in &plugin.tools {
        let inputs = schema_property_count(&tool.input_schema);
        lines.push(Line::from(fixed_columns(
            &[
                (tool.name.as_str(), 30),
                (tool.description.as_deref().unwrap_or(""), 68),
                (inputs.to_string().as_str(), 8),
            ],
            112,
        )));
    }
    if let Some(tool) = plugin.tools.first() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!("Input preview: {}", tool.name),
            Style::default().add_modifier(Modifier::BOLD),
        )));
        let default = default_value_for_schema(&tool.input_schema, &tool.input_schema);
        append_schema_editor_lines(
            &mut lines,
            Some(&tool.input_schema),
            Some(&tool.input_schema),
            &default,
            "Arguments",
            0,
            112,
            18,
        );
    }
    Text::from(lines)
}

fn plugin_commands_text(plugin: &PluginWorkbenchPlugin) -> Text<'static> {
    if plugin.commands.is_empty() {
        return Text::from("No commands.");
    }
    let mut lines = vec![Line::from(Span::styled(
        fixed_columns(
            &[
                ("Command", 30),
                ("Description", 64),
                ("Args", 8),
                ("Category", 18),
            ],
            124,
        ),
        Style::default().add_modifier(Modifier::BOLD),
    ))];
    for command in &plugin.commands {
        let args = command_argument_count(plugin, command);
        lines.push(Line::from(fixed_columns(
            &[
                (command.title.as_str(), 30),
                (command.description.as_str(), 64),
                (args.to_string().as_str(), 8),
                (command.category.as_str(), 18),
            ],
            124,
        )));
    }
    if let Some(command) = plugin.commands.first() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!("Arguments: {}", command.title),
            Style::default().add_modifier(Modifier::BOLD),
        )));
        match command_schema_and_value(plugin, command) {
            Some((schema, value)) => append_schema_editor_lines(
                &mut lines,
                Some(&schema),
                Some(&schema),
                &value,
                "Arguments",
                0,
                124,
                18,
            ),
            None => lines.push(Line::from("No structured arguments.")),
        }
    }
    Text::from(lines)
}

fn plugin_capabilities_text(plugin: &PluginWorkbenchPlugin) -> Text<'static> {
    let mut lines = Vec::new();
    if let Some(authority) = plugin
        .inspect
        .as_ref()
        .and_then(|inspect| inspect.authority.as_ref())
    {
        lines.push(Line::from(format!(
            "Trust level: {}",
            authority.trust_level
        )));
        if !authority.provenance.is_empty() {
            lines.push(Line::from(format!(
                "Provenance: {}",
                clean(authority.provenance.join(", "))
            )));
        }
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "Plugin capabilities",
            Style::default().add_modifier(Modifier::BOLD),
        )));
        if authority.plugin_capabilities.is_empty() {
            lines.push(Line::from("  none"));
        } else {
            for capability in &authority.plugin_capabilities {
                lines.push(Line::from(format!("  {}", clean(capability))));
            }
        }
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "Tool capabilities",
            Style::default().add_modifier(Modifier::BOLD),
        )));
        if authority.tool_capabilities.is_empty() {
            lines.push(Line::from("  none"));
        } else {
            for (tool_name, capabilities) in &authority.tool_capabilities {
                lines.push(Line::from(format!(
                    "  {}: {}",
                    clean(tool_name),
                    clean(capabilities.join(", "))
                )));
            }
        }
    } else {
        lines.push(Line::from("Authority data unavailable."));
    }
    Text::from(lines)
}

fn plugin_logs_text(plugin: &PluginWorkbenchPlugin) -> Text<'static> {
    if plugin.logs.is_empty() {
        return Text::from("No logs.");
    }
    Text::from(
        plugin
            .logs
            .iter()
            .map(|log_record| {
                Line::from(format!(
                    "#{} {} {}",
                    log_record.seq,
                    log_record.level,
                    clean(log_record.message.as_str())
                ))
            })
            .collect::<Vec<_>>(),
    )
}

fn plugin_diagnostics_text(plugin: &PluginWorkbenchPlugin) -> Text<'static> {
    let mut diagnostics = plugin.diagnostics.clone();
    diagnostics.extend(plugin.runtime_diagnostics.clone());
    diagnostics_text(diagnostics.as_slice(), false, 0)
}

fn config_editor_text(
    dialog: &PluginWorkbenchOverlay,
    plugin: &PluginWorkbenchPlugin,
) -> Text<'static> {
    let mut lines = Vec::new();
    if let Some(section) = dialog.selected_section() {
        let highlight_selection = !plugin_uses_compact_config_layout(plugin)
            || dialog.config_focus == PluginConfigFocus::Editor;
        append_section_lines(&mut lines, dialog, plugin, section, 98, highlight_selection);
    } else {
        lines.push(Line::from("No config section."));
    }
    Text::from(lines)
}

fn append_section_lines(
    lines: &mut Vec<Line<'static>>,
    dialog: &PluginWorkbenchOverlay,
    plugin: &PluginWorkbenchPlugin,
    section: &ConfigSectionView,
    width: u16,
    highlight_selection: bool,
) {
    match &section.body {
        ConfigSectionBody::Overview {
            cards,
            lines: summary,
        } => {
            append_overview_section_lines(lines, cards.as_slice(), summary.as_slice(), width);
        }
        ConfigSectionBody::Form { notice, groups } => {
            lines.push(Line::from(Span::styled(
                section.title.clone(),
                Style::default().add_modifier(Modifier::BOLD),
            )));
            if let Some(notice) = notice.as_deref() {
                lines.push(Line::from(""));
                lines.push(Line::from(clean(notice)));
            }
            for group in groups {
                if !group
                    .rows
                    .iter()
                    .any(|row| row_visible(row, dialog.config_view))
                {
                    continue;
                }
                lines.push(Line::from(""));
                append_group_lines(
                    lines,
                    dialog,
                    plugin,
                    section,
                    group,
                    width,
                    highlight_selection,
                );
            }
        }
    }
}

fn append_overview_section_lines(
    lines: &mut Vec<Line<'static>>,
    cards: &[ConfigOverviewCard],
    summary: &[String],
    width: u16,
) {
    lines.push(Line::from(Span::styled(
        "Overview".to_owned(),
        Style::default().add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        fixed_columns(&[("Section", 12), ("Summary", 68), ("State", 12)], width),
        Style::default().add_modifier(Modifier::BOLD),
    )));
    for card in cards {
        lines.push(Line::from(fixed_columns(
            &[
                (card.title.as_str(), 12),
                (card.summary.as_str(), 68),
                (card.issue_label.as_deref().unwrap_or(""), 12),
            ],
            width,
        )));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Config".to_owned(),
        Style::default().add_modifier(Modifier::BOLD),
    )));
    for line in summary {
        lines.push(Line::from(clean(line)));
    }
}

fn standard_config_row_line(
    setting: &str,
    type_display: &str,
    value: &str,
    default: &str,
    state: &str,
    width: u16,
) -> String {
    fixed_columns(
        &[
            (setting, 22),
            (type_display, 16),
            (value, 22),
            (default, 18),
            (state, 10),
        ],
        width,
    )
}

fn standard_config_row_line_with_action(
    setting: &str,
    type_display: &str,
    value: &str,
    default: &str,
    action: &str,
    state: &str,
    width: u16,
) -> String {
    fixed_columns(
        &[
            (setting, 18),
            (type_display, 14),
            (value, 18),
            (default, 14),
            (action, 14),
            (state, 8),
        ],
        width,
    )
}

fn pair_config_row_line_with_action(
    setting: &str,
    value: &str,
    secondary_value: &str,
    action: &str,
    state: &str,
    width: u16,
) -> String {
    fixed_columns(
        &[
            (setting, 20),
            (value, 18),
            (secondary_value, 18),
            (action, 14),
            (state, 8),
        ],
        width,
    )
}

fn styled_fixed_columns(columns: &[(String, usize, Style)], width: u16) -> Line<'static> {
    let mut spans = Vec::new();
    let mut used = 0usize;
    for (index, (text, size, style)) in columns.iter().enumerate() {
        if index > 0 {
            if used >= width as usize {
                break;
            }
            spans.push(Span::raw("  "));
            used += 2;
        }
        let remaining = width.saturating_sub(used as u16) as usize;
        if remaining == 0 {
            break;
        }
        let size = (*size).min(remaining);
        let cell = pad_to_width(clean(text).as_str(), size);
        spans.push(Span::styled(cell, *style));
        used += size;
    }
    Line::from(spans)
}

fn config_row_title_style(selected_cell: Option<ConfigRowCell>) -> Style {
    if selected_cell.is_some() {
        Style::default().add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    }
}

fn config_row_cell_style(selected_cell: Option<ConfigRowCell>, cell: ConfigRowCell) -> Style {
    if selected_cell == Some(cell) {
        plugin_workbench_selection_highlight_style()
    } else {
        Style::default()
    }
}

fn standard_config_row_line_with_focus(
    row: &ConfigRowView,
    width: u16,
    selected_cell: Option<ConfigRowCell>,
    include_action: bool,
) -> Line<'static> {
    let mut columns = if include_action {
        vec![
            (row.title.clone(), 18, config_row_title_style(selected_cell)),
            (
                row.type_display.clone(),
                14,
                config_row_cell_style(selected_cell, ConfigRowCell::Type),
            ),
            (
                row.value_display.clone(),
                18,
                config_row_cell_style(selected_cell, ConfigRowCell::Value),
            ),
            (
                row.default_display.clone(),
                14,
                config_row_cell_style(selected_cell, ConfigRowCell::Default),
            ),
            (
                row.action_display.clone().unwrap_or_default(),
                14,
                config_row_cell_style(selected_cell, ConfigRowCell::Action),
            ),
            (
                row.state.label().to_owned(),
                8,
                config_row_cell_style(selected_cell, ConfigRowCell::State),
            ),
        ]
    } else {
        vec![
            (row.title.clone(), 22, config_row_title_style(selected_cell)),
            (
                row.type_display.clone(),
                16,
                config_row_cell_style(selected_cell, ConfigRowCell::Type),
            ),
            (
                row.value_display.clone(),
                22,
                config_row_cell_style(selected_cell, ConfigRowCell::Value),
            ),
            (
                row.default_display.clone(),
                18,
                config_row_cell_style(selected_cell, ConfigRowCell::Default),
            ),
            (
                row.state.label().to_owned(),
                10,
                config_row_cell_style(selected_cell, ConfigRowCell::State),
            ),
        ]
    };
    styled_fixed_columns(columns.as_mut_slice(), width)
}

fn pair_config_row_line_with_focus(
    row: &ConfigRowView,
    width: u16,
    selected_cell: Option<ConfigRowCell>,
    include_action: bool,
) -> Line<'static> {
    let mut columns = if include_action {
        vec![
            (row.title.clone(), 20, config_row_title_style(selected_cell)),
            (
                row.value_display.clone(),
                18,
                config_row_cell_style(selected_cell, ConfigRowCell::Value),
            ),
            (
                row.secondary_value_display.clone().unwrap_or_default(),
                18,
                config_row_cell_style(selected_cell, ConfigRowCell::SecondaryValue),
            ),
            (
                row.action_display.clone().unwrap_or_default(),
                14,
                config_row_cell_style(selected_cell, ConfigRowCell::Action),
            ),
            (
                row.state.label().to_owned(),
                8,
                config_row_cell_style(selected_cell, ConfigRowCell::State),
            ),
        ]
    } else {
        vec![
            (row.title.clone(), 24, config_row_title_style(selected_cell)),
            (
                row.value_display.clone(),
                20,
                config_row_cell_style(selected_cell, ConfigRowCell::Value),
            ),
            (
                row.secondary_value_display.clone().unwrap_or_default(),
                20,
                config_row_cell_style(selected_cell, ConfigRowCell::SecondaryValue),
            ),
            (
                row.state.label().to_owned(),
                10,
                config_row_cell_style(selected_cell, ConfigRowCell::State),
            ),
        ]
    };
    styled_fixed_columns(columns.as_mut_slice(), width)
}

fn append_group_lines(
    lines: &mut Vec<Line<'static>>,
    dialog: &PluginWorkbenchOverlay,
    _plugin: &PluginWorkbenchPlugin,
    section: &ConfigSectionView,
    group: &ConfigGroupView,
    width: u16,
    highlight_selection: bool,
) {
    lines.push(Line::from(Span::styled(
        group.title.clone(),
        Style::default().add_modifier(Modifier::BOLD),
    )));
    let include_action = group_has_action_column(group, dialog.config_view);
    match group.layout {
        ConfigGroupLayout::Standard => {
            lines.push(Line::from(Span::styled(
                if include_action {
                    standard_config_row_line_with_action(
                        "Setting", "Type", "Value", "Default", "Action", "State", width,
                    )
                } else {
                    standard_config_row_line("Setting", "Type", "Value", "Default", "State", width)
                },
                Style::default().add_modifier(Modifier::BOLD),
            )));
        }
        ConfigGroupLayout::Pair {
            left_label,
            right_label,
        } => {
            lines.push(Line::from(Span::styled(
                if include_action {
                    pair_config_row_line_with_action(
                        "Setting",
                        left_label,
                        right_label,
                        "Action",
                        "State",
                        width,
                    )
                } else {
                    fixed_columns(
                        &[
                            ("Setting", 24),
                            (left_label, 20),
                            (right_label, 20),
                            ("State", 10),
                        ],
                        width,
                    )
                },
                Style::default().add_modifier(Modifier::BOLD),
            )));
        }
    }
    let mut visible_row_index = 0usize;
    for group_cursor in section_form_groups(section) {
        for row in &group_cursor.rows {
            if !row_visible(row, dialog.config_view) {
                continue;
            }
            let is_selected = dialog.selected_section == section_index_for_row(dialog, section)
                && dialog.selected_node == visible_row_index;
            if std::ptr::eq(group_cursor, group) {
                let focused_cell = if is_selected && highlight_selection {
                    Some(section_selected_row_cell(
                        section,
                        dialog.config_view,
                        visible_row_index,
                        dialog.selected_cell,
                    ))
                } else {
                    None
                };
                let line = match group.layout {
                    ConfigGroupLayout::Standard => standard_config_row_line_with_focus(
                        row,
                        width,
                        focused_cell,
                        include_action,
                    ),
                    ConfigGroupLayout::Pair { .. } => {
                        pair_config_row_line_with_focus(row, width, focused_cell, include_action)
                    }
                };
                lines.push(line);
            }
            visible_row_index += 1;
        }
    }
}

fn section_form_groups(section: &ConfigSectionView) -> &[ConfigGroupView] {
    match &section.body {
        ConfigSectionBody::Form { groups, .. } => groups.as_slice(),
        ConfigSectionBody::Overview { .. } => &[],
    }
}

fn section_index_for_row(dialog: &PluginWorkbenchOverlay, section: &ConfigSectionView) -> usize {
    dialog
        .selected_plugin()
        .and_then(|plugin| {
            plugin
                .sections
                .iter()
                .position(|candidate| candidate.key == section.key)
        })
        .unwrap_or_default()
}

fn pair_editor_labels(
    left_path: &[PathSegment],
    right_path: &[PathSegment],
) -> (&'static str, &'static str) {
    let left_last = left_path.last().and_then(path_segment_key_name);
    let right_last = right_path.last().and_then(path_segment_key_name);
    let left_has_defaults = left_path
        .iter()
        .filter_map(path_segment_key_name)
        .any(|segment| segment == "defaults");
    let right_has_limits = right_path
        .iter()
        .filter_map(path_segment_key_name)
        .any(|segment| segment == "limits");
    if left_has_defaults && right_has_limits {
        return ("Value", "Limit");
    }
    if left_last.is_some_and(|name| name.starts_with("default"))
        && right_last.is_some_and(|name| name.starts_with("max"))
    {
        return ("Value", "Max");
    }
    ("Value 1", "Value 2")
}

fn path_segment_key_name(segment: &PathSegment) -> Option<&str> {
    match segment {
        PathSegment::Key(key) => Some(key.as_str()),
        PathSegment::Index(_) => None,
    }
}

fn plugin_all_diagnostics(plugin: &PluginWorkbenchPlugin) -> Vec<ConfigDiagnostic> {
    let mut diagnostics = plugin.diagnostics.clone();
    diagnostics.extend(plugin.runtime_diagnostics.clone());
    diagnostics
}

fn diagnostics_text(
    diagnostics: &[ConfigDiagnostic],
    highlight_selection: bool,
    selected_row: usize,
) -> Text<'static> {
    let table_width = 112;
    let mut lines = Vec::new();
    if diagnostics.is_empty() {
        lines.push(Line::from("No issues"));
    } else {
        lines.push(Line::from(Span::styled(
            fixed_columns(
                &[
                    ("Severity", 10),
                    ("Source", 10),
                    ("Field", 22),
                    ("Message", 80),
                ],
                table_width,
            ),
            Style::default().add_modifier(Modifier::BOLD),
        )));
        for (index, diagnostic) in diagnostics.iter().enumerate() {
            let line = fixed_columns(
                &[
                    (diagnostic_severity_label(diagnostic.severity), 10),
                    (diagnostic.source.as_str(), 10),
                    (diagnostic.field.as_str(), 22),
                    (diagnostic.message.as_str(), 80),
                ],
                table_width,
            );
            let style = if highlight_selection && index == selected_row {
                plugin_workbench_selection_highlight_style()
            } else {
                Style::default()
            };
            lines.push(Line::from(Span::styled(clean(line), style)));
        }
    }
    Text::from(lines)
}

fn config_diff_text(
    dialog: &PluginWorkbenchOverlay,
    plugin: &PluginWorkbenchPlugin,
) -> Text<'static> {
    let table_width = 116;
    let mut lines = Vec::new();
    if plugin.diff.is_empty() {
        lines.push(Line::from("No changes"));
    } else {
        lines.push(Line::from(Span::styled(
            fixed_columns(
                &[("Field", 28), ("Before", 28), ("After", 28), ("Change", 28)],
                table_width,
            ),
            Style::default().add_modifier(Modifier::BOLD),
        )));
        for (index, row) in plugin.diff.iter().enumerate() {
            let line = fixed_columns(
                &[
                    (path_display(&row.path).as_str(), 28),
                    (row.before.as_str(), 28),
                    (row.after.as_str(), 28),
                    (row.summary.as_str(), 28),
                ],
                table_width,
            );
            let style = if dialog.config_focus == PluginConfigFocus::Diagnostics
                && dialog.show_diff
                && index == dialog.selected_diff_row
            {
                plugin_workbench_selection_highlight_style()
            } else {
                Style::default()
            };
            lines.push(Line::from(Span::styled(clean(line), style)));
        }
    }
    Text::from(lines)
}

#[allow(clippy::too_many_arguments)]
fn append_schema_editor_lines(
    lines: &mut Vec<Line<'static>>,
    root_schema: Option<&JsonValue>,
    schema: Option<&JsonValue>,
    value: &JsonValue,
    title: &str,
    depth: usize,
    width: u16,
    remaining: usize,
) {
    if remaining == 0 {
        lines.push(Line::from(format!(
            "{}[ Configure... ]",
            "  ".repeat(depth)
        )));
        return;
    }
    let active_schema = schema.map(|schema| {
        root_schema
            .map(|root| active_schema_for_value(root, schema, value))
            .unwrap_or_else(|| schema.clone())
    });
    let declared_schema = schema;
    let render_schema = active_schema.as_ref().or(declared_schema);
    let title = clean(title);
    let indent = "  ".repeat(depth);
    if depth == 0 {
        lines.push(Line::from(Span::styled(
            title.clone(),
            Style::default().add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(format!(
            "{}Type: {}        Path editor: structured GUI",
            indent,
            render_schema
                .map(schema_kind_label)
                .unwrap_or_else(|| json_kind_label(value).to_owned())
        )));
        lines.push(Line::from(""));
    }

    if let Some(schema) = declared_schema {
        append_branch_selector_lines(
            lines,
            root_schema.unwrap_or(schema),
            schema,
            value,
            depth,
            width,
        );
        append_type_selector_line(lines, schema, value, depth);
    }
    if let Some(schema) = render_schema {
        if let Some(constant) = schema_const_value(schema) {
            lines.push(Line::from(format!(
                "{}{}        [ {} ] readonly",
                indent,
                title,
                preview_value(&constant)
            )));
            return;
        }
        if let Some(variants) = schema_enum_values(schema)
            && !variants.is_empty()
        {
            lines.push(Line::from(format!(
                "{}{}        [ {} v ]",
                indent,
                title,
                preview_value(value)
            )));
            return;
        }
    } else if depth == 0 {
        lines.push(Line::from("Schema missing        Basic structured editor"));
    }

    match value {
        JsonValue::Object(object) => append_object_editor_lines(
            lines,
            root_schema,
            render_schema,
            object,
            depth,
            width,
            remaining,
        ),
        JsonValue::Array(items) => append_array_editor_lines(
            lines,
            root_schema,
            render_schema,
            items,
            depth,
            width,
            remaining,
        ),
        JsonValue::String(text) => {
            append_string_editor_lines(lines, render_schema, title.as_str(), text, depth)
        }
        JsonValue::Number(number) => {
            append_number_editor_lines(lines, render_schema, title.as_str(), number, depth)
        }
        JsonValue::Bool(value) => lines.push(Line::from(format!(
            "{}{}        [{}]",
            indent,
            title,
            if *value { "x" } else { " " }
        ))),
        JsonValue::Null => append_null_editor_lines(
            lines,
            declared_schema.or(render_schema),
            title.as_str(),
            depth,
        ),
    }
}

fn append_branch_selector_lines(
    lines: &mut Vec<Line<'static>>,
    root_schema: &JsonValue,
    schema: &JsonValue,
    value: &JsonValue,
    depth: usize,
    width: u16,
) {
    let Some(branches) = branch_choices(root_schema, schema) else {
        return;
    };
    let active = active_branch_label(branches.as_slice(), value);
    let active_id = active_branch_id(branches.as_slice(), value);
    let also_matches = branches
        .iter()
        .filter(|branch| {
            branch.id != active_id && schema_matches(root_schema, &branch.schema, value)
        })
        .map(|branch| branch.label.as_str())
        .collect::<Vec<_>>();
    let suffix = if also_matches.is_empty() {
        String::new()
    } else {
        format!("   also matches: {}", clean(also_matches.join(", ")))
    };
    lines.push(Line::from(fixed_columns(
        &[
            (format!("{}Shape", "  ".repeat(depth)).as_str(), 18),
            (format!("[ {active} v ]{suffix}").as_str(), 72),
        ],
        width,
    )));
}

fn append_type_selector_line(
    lines: &mut Vec<Line<'static>>,
    schema: &JsonValue,
    value: &JsonValue,
    depth: usize,
) {
    let choices = schema_type_choices(schema);
    if choices.len() <= 1 {
        return;
    }
    let active = json_kind_label(value);
    lines.push(Line::from(format!(
        "{}Type        [ {} v ]",
        "  ".repeat(depth),
        active
    )));
}

fn append_object_editor_lines(
    lines: &mut Vec<Line<'static>>,
    root_schema: Option<&JsonValue>,
    schema: Option<&JsonValue>,
    object: &JsonMap<String, JsonValue>,
    depth: usize,
    width: u16,
    remaining: usize,
) {
    let indent = "  ".repeat(depth);
    if schema.is_none() {
        lines.push(Line::from(format!("{indent}Generic object editor")));
    } else if schema_is_map_like(
        root_schema.unwrap_or_else(|| schema.expect("checked")),
        schema.expect("checked"),
    ) {
        lines.push(Line::from(format!("{indent}Map editor")));
    } else {
        lines.push(Line::from(format!("{indent}Object editor")));
    }
    lines.push(Line::from(Span::styled(
        fixed_columns(
            &[
                (format!("{indent}Field").as_str(), 28),
                ("Type", 14),
                ("Value", 46),
                ("State", 14),
            ],
            width,
        ),
        Style::default().add_modifier(Modifier::BOLD),
    )));
    let keys = ordered_object_keys(schema, object);
    if keys.is_empty() {
        lines.push(Line::from(format!("{indent}No fields.")));
    }
    for key in keys {
        let child = object.get(key.as_str()).unwrap_or(&JsonValue::Null);
        let child_schema = schema.and_then(|schema| {
            root_schema.and_then(|root| object_property_schema(root, schema, key.as_str()))
        });
        let kind = child_schema
            .as_ref()
            .map(schema_kind_label)
            .unwrap_or_else(|| json_kind_label(child).to_owned());
        let state = object_field_state(
            root_schema,
            schema,
            key.as_str(),
            object.contains_key(key.as_str()),
        );
        lines.push(Line::from(fixed_columns(
            &[
                (
                    format!("{indent}{}", title_from_key(key.as_str())).as_str(),
                    28,
                ),
                (kind.as_str(), 14),
                (structured_preview(child).as_str(), 46),
                (state.as_str(), 14),
            ],
            width,
        )));
        if depth < 2 && matches!(child, JsonValue::Object(_) | JsonValue::Array(_)) && remaining > 1
        {
            append_schema_editor_lines(
                lines,
                root_schema,
                child_schema.as_ref(),
                child,
                title_from_key(key.as_str()).as_str(),
                depth + 1,
                width,
                remaining.saturating_sub(1),
            );
        }
    }
    lines.push(Line::from(format!("{indent}[ Add field ]")));
}

fn append_array_editor_lines(
    lines: &mut Vec<Line<'static>>,
    root_schema: Option<&JsonValue>,
    schema: Option<&JsonValue>,
    items: &[JsonValue],
    depth: usize,
    width: u16,
    remaining: usize,
) {
    let indent = "  ".repeat(depth);
    let tuple = schema.is_some_and(|schema| schema_prefix_item_count(schema) > 0);
    let object_items = items.iter().any(JsonValue::is_object);
    let title = if tuple {
        "Tuple editor"
    } else if object_items {
        "Object array table editor"
    } else {
        "Primitive array editor"
    };
    lines.push(Line::from(format!("{indent}{title}")));
    if object_items {
        append_object_array_table(lines, root_schema, schema, items, depth, width);
    } else {
        lines.push(Line::from(Span::styled(
            fixed_columns(
                &[
                    (format!("{indent}Index").as_str(), 10),
                    ("Type", 14),
                    ("Preview", 56),
                ],
                width,
            ),
            Style::default().add_modifier(Modifier::BOLD),
        )));
        for (index, item) in items.iter().enumerate() {
            let item_schema = schema.and_then(|schema| {
                root_schema.and_then(|root| array_item_schema(root, schema, index))
            });
            lines.push(Line::from(fixed_columns(
                &[
                    (format!("{indent}{index}").as_str(), 10),
                    (
                        item_schema
                            .as_ref()
                            .map(schema_kind_label)
                            .unwrap_or_else(|| json_kind_label(item).to_owned())
                            .as_str(),
                        14,
                    ),
                    (structured_preview(item).as_str(), 56),
                ],
                width,
            )));
            if depth < 2
                && matches!(item, JsonValue::Object(_) | JsonValue::Array(_))
                && remaining > 1
            {
                append_schema_editor_lines(
                    lines,
                    root_schema,
                    item_schema.as_ref(),
                    item,
                    format!("Item {index}").as_str(),
                    depth + 1,
                    width,
                    remaining.saturating_sub(1),
                );
            }
        }
    }
    if items.is_empty() {
        lines.push(Line::from(format!("{indent}No items.")));
    }
    lines.push(Line::from(format!(
        "{indent}[ Add ] [ Edit ] [ Duplicate ] [ Delete ] [ Move Up ] [ Move Down ]"
    )));
}

fn append_object_array_table(
    lines: &mut Vec<Line<'static>>,
    root_schema: Option<&JsonValue>,
    schema: Option<&JsonValue>,
    items: &[JsonValue],
    depth: usize,
    width: u16,
) {
    let indent = "  ".repeat(depth);
    let item_schema =
        schema.and_then(|schema| root_schema.and_then(|root| array_item_schema(root, schema, 0)));
    let columns = object_array_columns(item_schema.as_ref(), items);
    let mut header = vec![(format!("{indent}Index"), 8)];
    for column in &columns {
        header.push((title_from_key(column), 18));
    }
    let header_refs = header
        .iter()
        .map(|(label, size)| (label.as_str(), *size))
        .collect::<Vec<_>>();
    lines.push(Line::from(Span::styled(
        fixed_columns(header_refs.as_slice(), width),
        Style::default().add_modifier(Modifier::BOLD),
    )));
    for (index, item) in items.iter().enumerate() {
        let mut row = vec![(format!("{indent}{index}"), 8)];
        if let Some(object) = item.as_object() {
            for column in &columns {
                row.push((
                    object
                        .get(column)
                        .map(structured_preview)
                        .unwrap_or_else(|| "missing".to_owned()),
                    18,
                ));
            }
        } else {
            row.push((structured_preview(item), 18));
        }
        let row_refs = row
            .iter()
            .map(|(label, size)| (label.as_str(), *size))
            .collect::<Vec<_>>();
        lines.push(Line::from(fixed_columns(row_refs.as_slice(), width)));
    }
    if root_schema.is_some() && item_schema.is_some() {
        lines.push(Line::from(format!(
            "{indent}Edit opens the selected item with the same structured editor."
        )));
    }
}

fn append_string_editor_lines(
    lines: &mut Vec<Line<'static>>,
    schema: Option<&JsonValue>,
    title: &str,
    text: &str,
    depth: usize,
) {
    let indent = "  ".repeat(depth);
    let format_suffix = schema
        .and_then(|schema| schema_first_string_keyword(schema, "format"))
        .map(|format| format!("   format: {format}"))
        .unwrap_or_default();
    if schema.is_some_and(schema_string_is_multiline) || text.contains('\n') {
        lines.push(Line::from(format!("{indent}{title}")));
        lines.push(Line::from(format!("{indent}+{}", "-".repeat(44))));
        for line in text.lines().take(6) {
            lines.push(Line::from(format!("{indent}| {}", clean(line))));
        }
        if text.is_empty() {
            lines.push(Line::from(format!("{indent}| ")));
        }
        lines.push(Line::from(format!("{indent}+{}", "-".repeat(44))));
    } else {
        lines.push(Line::from(format!(
            "{indent}{title}        [ {} ]{}",
            clean(truncate_text(text, 48)),
            format_suffix
        )));
    }
    if let Some(examples) = schema.and_then(schema_examples) {
        let suggestions = examples
            .iter()
            .take(3)
            .map(preview_value)
            .collect::<Vec<_>>()
            .join(", ");
        lines.push(Line::from(format!(
            "{indent}Suggestions        [ {} v ]",
            clean(suggestions)
        )));
    }
}

fn append_number_editor_lines(
    lines: &mut Vec<Line<'static>>,
    schema: Option<&JsonValue>,
    title: &str,
    number: &JsonNumber,
    depth: usize,
) {
    let indent = "  ".repeat(depth);
    let constraints = schema
        .map(number_constraint_summary)
        .filter(|summary| !summary.is_empty())
        .unwrap_or_default();
    lines.push(Line::from(format!(
        "{indent}{title}        [ {} ]{}",
        number, constraints
    )));
}

fn append_null_editor_lines(
    lines: &mut Vec<Line<'static>>,
    schema: Option<&JsonValue>,
    title: &str,
    depth: usize,
) {
    let indent = "  ".repeat(depth);
    let choices = schema.map(schema_type_choices).unwrap_or_default();
    if choices.len() > 1 {
        lines.push(Line::from(format!("{indent}{title}")));
        lines.push(Line::from(format!("{indent}Type        [ null v ]")));
    } else {
        lines.push(Line::from(format!("{indent}{title}        [ null ]")));
    }
}

fn schema_property_count(schema: &JsonValue) -> usize {
    let property_count = schema_declared_property_keys(schema).len();
    if property_count > 0 {
        return property_count;
    }
    let prefix_count = schema_prefix_item_count(schema);
    if prefix_count > 0 {
        return prefix_count;
    }
    if schema_has_array_shape(schema) || schema_has_map_keywords(schema) {
        1
    } else {
        0
    }
}

fn command_argument_count(
    plugin: &PluginWorkbenchPlugin,
    command: &agena::plugin::PluginStudioCommand,
) -> usize {
    match command_schema_and_value(plugin, command) {
        Some((schema, _)) => schema_property_count(&schema),
        None => 0,
    }
}

fn command_schema_and_value(
    plugin: &PluginWorkbenchPlugin,
    command: &agena::plugin::PluginStudioCommand,
) -> Option<(JsonValue, JsonValue)> {
    let agena::plugin::PluginUiAction::InvokeTool { tool, input, .. } = &command.action else {
        return None;
    };
    let tool = plugin
        .tools
        .iter()
        .find(|candidate| candidate.name == *tool)?;
    let schema = tool.input_schema.clone();
    let mut value = default_value_for_schema(&schema, &schema);
    if let Some(input) = input {
        merge_config_override(&mut value, input);
    }
    Some((schema, value))
}

fn schema_is_map_like(root: &JsonValue, schema: &JsonValue) -> bool {
    let schema = active_schema_for_value(root, schema, &JsonValue::Object(JsonMap::new()));
    schema_has_object_shape(&schema) && schema_has_map_keywords(&schema)
}

fn schema_has_map_keywords(schema: &JsonValue) -> bool {
    if schema.as_object().is_some_and(|object| {
        object.contains_key("additionalProperties")
            || object.contains_key("patternProperties")
            || object.contains_key("propertyNames")
    }) {
        return true;
    }
    schema
        .get("allOf")
        .and_then(JsonValue::as_array)
        .is_some_and(|branches| branches.iter().any(schema_has_map_keywords))
}

fn schema_prohibits_additional_properties(schema: &JsonValue) -> bool {
    if schema.get("additionalProperties") == Some(&JsonValue::Bool(false)) {
        return true;
    }
    schema
        .get("allOf")
        .and_then(JsonValue::as_array)
        .is_some_and(|branches| branches.iter().any(schema_prohibits_additional_properties))
}

fn schema_property_name_schemas(schema: &JsonValue) -> Vec<JsonValue> {
    let mut schemas = schema
        .get("propertyNames")
        .cloned()
        .into_iter()
        .collect::<Vec<_>>();
    if let Some(all_of) = schema.get("allOf").and_then(JsonValue::as_array) {
        for branch in all_of {
            schemas.extend(schema_property_name_schemas(branch));
        }
    }
    schemas
}

fn schema_prefix_item_count(schema: &JsonValue) -> usize {
    let direct = schema
        .get("prefixItems")
        .and_then(JsonValue::as_array)
        .map(Vec::len)
        .unwrap_or_default();
    let nested = schema
        .get("allOf")
        .and_then(JsonValue::as_array)
        .map(|branches| {
            branches
                .iter()
                .map(schema_prefix_item_count)
                .max()
                .unwrap_or_default()
        })
        .unwrap_or_default();
    direct.max(nested)
}

fn schema_min_u64_constraint(schema: &JsonValue, key: &str) -> Option<u64> {
    let direct = schema.get(key).and_then(JsonValue::as_u64);
    let nested = schema
        .get("allOf")
        .and_then(JsonValue::as_array)
        .and_then(|branches| {
            branches
                .iter()
                .filter_map(|branch| schema_min_u64_constraint(branch, key))
                .max()
        });
    match (direct, nested) {
        (Some(left), Some(right)) => Some(left.max(right)),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    }
}

fn schema_max_u64_constraint(schema: &JsonValue, key: &str) -> Option<u64> {
    let direct = schema.get(key).and_then(JsonValue::as_u64);
    let nested = schema
        .get("allOf")
        .and_then(JsonValue::as_array)
        .and_then(|branches| {
            branches
                .iter()
                .filter_map(|branch| schema_max_u64_constraint(branch, key))
                .min()
        });
    match (direct, nested) {
        (Some(left), Some(right)) => Some(left.min(right)),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    }
}

fn schema_declared_property_keys(schema: &JsonValue) -> BTreeSet<String> {
    let mut keys = schema
        .get("properties")
        .and_then(JsonValue::as_object)
        .into_iter()
        .flatten()
        .map(|(key, _)| key.clone())
        .collect::<BTreeSet<_>>();
    if let Some(all_of) = schema.get("allOf").and_then(JsonValue::as_array) {
        for branch in all_of {
            keys.extend(schema_declared_property_keys(branch));
        }
    }
    keys
}

fn schema_matches_pattern_property(schema: &JsonValue, key: &str) -> bool {
    if schema
        .get("patternProperties")
        .and_then(JsonValue::as_object)
        .is_some_and(|patterns| {
            patterns
                .keys()
                .any(|pattern| pattern_key_matches(pattern, key))
        })
    {
        return true;
    }
    schema
        .get("allOf")
        .and_then(JsonValue::as_array)
        .is_some_and(|branches| {
            branches
                .iter()
                .any(|branch| schema_matches_pattern_property(branch, key))
        })
}

fn ordered_object_keys(
    schema: Option<&JsonValue>,
    object: &JsonMap<String, JsonValue>,
) -> Vec<String> {
    let mut keys = Vec::new();
    let mut seen = BTreeSet::new();
    if let Some(schema) = schema {
        let required = schema_required_fields(schema);
        for key in &required {
            if seen.insert(key.clone()) {
                keys.push(key.clone());
            }
        }
        for key in schema_declared_property_keys(schema) {
            if seen.insert(key.clone()) {
                keys.push(key);
            }
        }
    }
    for key in object.keys() {
        if seen.insert(key.clone()) {
            keys.push(key.clone());
        }
    }
    keys
}

fn schema_required_fields(schema: &JsonValue) -> BTreeSet<String> {
    let mut required = schema
        .get("required")
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
        .filter_map(JsonValue::as_str)
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    if let Some(all_of) = schema.get("allOf").and_then(JsonValue::as_array) {
        for branch in all_of {
            required.extend(schema_required_fields(branch));
        }
    }
    required
}

fn object_field_state(
    root: Option<&JsonValue>,
    schema: Option<&JsonValue>,
    key: &str,
    present: bool,
) -> String {
    let Some(schema) = schema else {
        return "custom".to_owned();
    };
    if schema_required_fields(schema).contains(key) {
        if present {
            "required".to_owned()
        } else {
            "missing".to_owned()
        }
    } else if schema_declared_property_keys(schema).contains(key)
        || (present
            && root.is_some_and(|root| {
                object_property_schema(root, schema, key).is_some()
                    || schema_matches_pattern_property(schema, key)
            }))
    {
        if present {
            "optional".to_owned()
        } else {
            "available".to_owned()
        }
    } else {
        "map key".to_owned()
    }
}

fn object_array_columns(schema: Option<&JsonValue>, items: &[JsonValue]) -> Vec<String> {
    let mut keys = Vec::new();
    let mut seen = BTreeSet::new();
    if let Some(schema) = schema {
        for key in schema_declared_property_keys(schema).into_iter().take(4) {
            if seen.insert(key.clone()) {
                keys.push(key);
            }
        }
    }
    for item in items {
        if let Some(object) = item.as_object() {
            for key in object.keys().take(4) {
                if seen.insert(key.clone()) {
                    keys.push(key.clone());
                }
                if keys.len() >= 4 {
                    return keys;
                }
            }
        }
    }
    if keys.is_empty() {
        keys.push("value".to_owned());
    }
    keys
}

fn structured_preview(value: &JsonValue) -> String {
    match value {
        JsonValue::Object(object) => format!("Configure... ({} field(s))", object.len()),
        JsonValue::Array(items) => format!("Configure... ({} item(s))", items.len()),
        _ => preview_value(value),
    }
}

fn number_constraint_summary(schema: &JsonValue) -> String {
    let mut parts = BTreeSet::new();
    for key in [
        "minimum",
        "maximum",
        "exclusiveMinimum",
        "exclusiveMaximum",
        "multipleOf",
    ] {
        if let Some(value) = schema.get(key) {
            parts.insert(format!("{key}: {}", preview_value(value)));
        }
    }
    if let Some(all_of) = schema.get("allOf").and_then(JsonValue::as_array) {
        for branch in all_of {
            for part in schema_constraints(branch).into_iter().filter(|part| {
                part.starts_with("minimum:")
                    || part.starts_with("maximum:")
                    || part.starts_with("exclusiveMinimum:")
                    || part.starts_with("exclusiveMaximum:")
                    || part.starts_with("multipleOf:")
            }) {
                parts.insert(part);
            }
        }
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!("     {}", parts.into_iter().collect::<Vec<_>>().join("   "))
    }
}

fn schema_constraints(schema: &JsonValue) -> Vec<String> {
    let mut constraints = BTreeSet::new();
    for key in [
        "format",
        "minimum",
        "maximum",
        "exclusiveMinimum",
        "exclusiveMaximum",
        "multipleOf",
        "minLength",
        "maxLength",
        "pattern",
        "minItems",
        "maxItems",
        "uniqueItems",
    ] {
        if let Some(value) = schema.get(key) {
            constraints.insert(format!("{key}: {}", preview_value(value)));
        }
    }
    if let Some(all_of) = schema.get("allOf").and_then(JsonValue::as_array) {
        for branch in all_of {
            constraints.extend(schema_constraints(branch));
        }
    }
    constraints.into_iter().collect()
}

fn plugin_workbench_summary(dialog: &PluginWorkbenchOverlay) -> String {
    format!(
        "Search: {} · Transport: {} · Config: {} · {}/{} shown",
        if dialog.query.text().is_empty() {
            "all"
        } else {
            dialog.query.text()
        },
        dialog.transport_filter.label(),
        dialog.config_filter.label(),
        dialog.visible_plugins.len(),
        dialog.plugins.len()
    )
}

fn fixed_columns(columns: &[(&str, usize)], width: u16) -> String {
    let mut out = String::new();
    for (index, (text, size)) in columns.iter().enumerate() {
        if index > 0 {
            out.push_str("  ");
        }
        let remaining = width.saturating_sub(out.width() as u16) as usize;
        if remaining == 0 {
            break;
        }
        let size = (*size).min(remaining);
        let clipped = truncate_text(text, size);
        out.push_str(clipped.as_str());
        let padding = size.saturating_sub(clipped.width());
        out.push_str(" ".repeat(padding).as_str());
    }
    out
}

fn pad_to_width(text: &str, width: usize) -> String {
    let clipped = truncate_text(text, width);
    let padding = width.saturating_sub(clipped.width());
    format!("{clipped}{}", " ".repeat(padding))
}

fn wrap_prefixed_text(
    text: &str,
    first_prefix: &str,
    rest_prefix: &str,
    width: usize,
) -> Vec<String> {
    let available_first = width.saturating_sub(first_prefix.width()).max(1);
    let available_rest = width.saturating_sub(rest_prefix.width()).max(1);
    let mut lines = Vec::new();
    let mut prefix = first_prefix;
    let mut available = available_first;
    let mut current = String::new();
    let mut current_width = 0usize;

    for word in text.split_whitespace() {
        let mut remaining = word.to_owned();
        loop {
            let room = if current.is_empty() {
                available
            } else {
                available.saturating_sub(current_width + 1)
            };
            if room == 0 {
                lines.push(format!("{prefix}{current}"));
                prefix = rest_prefix;
                available = available_rest;
                current.clear();
                current_width = 0;
                continue;
            }
            if remaining.width() <= room {
                if !current.is_empty() {
                    current.push(' ');
                    current_width += 1;
                }
                current.push_str(remaining.as_str());
                current_width += remaining.width();
                break;
            }

            let chunk = take_width_prefix(remaining.as_str(), room);
            if chunk.is_empty() {
                break;
            }
            if !current.is_empty() {
                lines.push(format!("{prefix}{current}"));
                prefix = rest_prefix;
                current.clear();
                current_width = 0;
            }
            lines.push(format!("{prefix}{chunk}"));
            let consumed = chunk.len();
            remaining = remaining[consumed..].to_owned();
            prefix = rest_prefix;
            available = available_rest;
        }
    }

    if !current.is_empty() || lines.is_empty() {
        lines.push(format!("{prefix}{current}"));
    }

    lines
}

fn take_width_prefix(text: &str, max_width: usize) -> String {
    let mut out = String::new();
    let mut width = 0usize;
    for ch in text.chars() {
        let ch_width = ch.width().unwrap_or_default();
        if width + ch_width > max_width {
            break;
        }
        out.push(ch);
        width += ch_width;
    }
    out
}

fn plugin_package_preview(value: &JsonValue) -> String {
    value
        .get("kind")
        .and_then(JsonValue::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| preview_value(value))
}

fn diagnostic_severity_label(severity: DiagnosticSeverity) -> &'static str {
    match severity {
        DiagnosticSeverity::Error => "error",
        DiagnosticSeverity::Warning => "warning",
    }
}

fn plugin_workbench_selection_highlight_style() -> Style {
    Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::REVERSED | Modifier::BOLD)
}

fn quote_settings_segment(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

fn get_json_path(value: &JsonValue, path: Option<&str>) -> Result<JsonValue, String> {
    agena::config::get_json_path(value, path).map_err(|error| error.to_string())
}

fn plugin_config_record_value(plugin: &PluginWorkbenchPlugin) -> JsonValue {
    plugin.configured_plugin_value.clone().unwrap_or_else(|| {
        json!({
            "enabled": true,
            "package": {
                "kind": "static"
            },
            "config": JsonValue::Null
        })
    })
}

fn move_selected_config_node(dialog: &mut PluginWorkbenchOverlay, delta: isize) {
    let item_count = dialog
        .selected_section()
        .map(|section| section_row_count(section, dialog.config_view))
        .unwrap_or_default();
    move_index(&mut dialog.selected_node, item_count, delta);
    dialog.clamp_selection();
}

fn move_detail_scroll(dialog: &mut PluginWorkbenchOverlay, delta: isize) {
    if dialog.detail_tab == PluginDetailTab::Diagnostics {
        move_index(&mut dialog.diagnostics_scroll, usize::MAX / 2, delta);
    } else {
        move_index(&mut dialog.config_scroll, usize::MAX / 2, delta);
    }
}

fn move_index(index: &mut usize, item_count: usize, delta: isize) {
    if item_count == 0 {
        *index = 0;
        return;
    }
    let last = item_count.saturating_sub(1) as isize;
    *index = (*index as isize + delta).clamp(0, last) as usize;
}

fn move_index_page(index: &mut usize, item_count: usize, delta: isize, page_size: usize) {
    move_index(
        index,
        item_count,
        delta.saturating_mul(page_size.max(1) as isize),
    );
}

fn truncate_text(text: &str, max_width: usize) -> String {
    if text.width() <= max_width {
        return text.to_owned();
    }
    if max_width <= 3 {
        return ".".repeat(max_width);
    }
    let mut out = String::new();
    let mut width = 0;
    let suffix_width = 3;
    for ch in text.chars() {
        let ch_width = ch.width().unwrap_or_default();
        if width + ch_width + suffix_width > max_width {
            break;
        }
        out.push(ch);
        width += ch_width;
    }
    out.push_str("...");
    out
}

fn clean(text: impl AsRef<str>) -> String {
    text.as_ref()
        .chars()
        .map(|ch| {
            if ch.is_control() && ch != '\n' && ch != '\t' {
                ' '
            } else {
                ch
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_sources() -> crate::backend::ConfigJsonSources {
        crate::backend::ConfigJsonSources {
            config_path: std::path::PathBuf::from("config.json"),
            config_found: false,
            file: json!({}),
            effective: json!({}),
        }
    }

    fn build_schema_fixture_plugin(
        schema: JsonValue,
        raw_config: JsonValue,
    ) -> PluginWorkbenchPlugin {
        let manifest = agena::plugin::PluginManifest::builder("fixture", "0.1.0")
            .config_schema(schema)
            .build();
        let status = agena::plugin::status::PluginStatus::initial("fixture.plugin", "static");
        let inspect = agena::plugin::PluginInspect {
            status: status.clone(),
            manifest: Some(manifest),
            authority: None,
            configured_plugin: Some(agena::plugin::ConfiguredPlugin::static_config(raw_config)),
        };
        build_plugin_workbench_plugin(&test_sources(), "en-US", status, Some(inspect), Vec::new())
    }

    fn build_test_dialog(plugin: PluginWorkbenchPlugin) -> PluginWorkbenchOverlay {
        PluginWorkbenchOverlay {
            title: "Plugins".to_owned(),
            query: Editor::default(),
            mode: PluginWorkbenchMode::Detail,
            transport_filter: PluginTransportFilter::All,
            config_filter: PluginConfigFilter::All,
            plugins: vec![plugin],
            visible_plugins: vec![0],
            selected_plugin: 0,
            detail_tab: PluginDetailTab::Config,
            config_view: PluginConfigView::Effective,
            config_focus: PluginConfigFocus::Editor,
            selected_toolbar_action: 0,
            selected_section: 0,
            selected_node: 0,
            selected_cell: ConfigRowCell::Value,
            selected_diagnostic: 0,
            selected_diff_row: 0,
            config_scroll: 0,
            diagnostics_scroll: 0,
            show_diff: false,
            drilldown_stack: Vec::new(),
            actions: None,
            selection: None,
            editor: None,
        }
    }

    fn line_to_string(line: &Line<'static>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>()
    }

    #[test]
    fn plugin_without_effective_config_does_not_synthesize_static_plugin_config() {
        let sources = test_sources();
        let status = agena::plugin::status::PluginStatus::initial("agena.fs", "static");

        let plugin = build_plugin_workbench_plugin(&sources, "en-US", status, None, Vec::new());
        assert!(plugin.configured_plugin_value.is_none());
        assert_eq!(plugin.saved_config, JsonValue::Null);
    }

    #[test]
    fn null_config_without_schema_is_valid_omitted_config() {
        let diagnostics = validate_config_value(None, &JsonValue::Null, true);
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn null_config_with_object_schema_is_valid_omitted_config() {
        let schema = json!({
            "type": "object",
            "required": ["endpoint"],
            "properties": {
                "endpoint": {
                    "type": "string",
                    "format": "uri"
                }
            }
        });

        let diagnostics = validate_config_value(Some(&schema), &JsonValue::Null, false);
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn validate_config_value_enforces_all_of_required_fields_from_every_branch() {
        let schema = json!({
            "type": "object",
            "allOf": [
                {
                    "type": "object",
                    "properties": {
                        "mode": { "type": "string" }
                    },
                    "required": ["mode"]
                },
                {
                    "type": "object",
                    "properties": {
                        "cron": { "type": "string", "minLength": 1 }
                    },
                    "required": ["cron"]
                }
            ]
        });
        let diagnostics =
            validate_config_value(Some(&schema), &json!({"mode": "scheduled"}), false);

        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.severity == DiagnosticSeverity::Error
                && diagnostic.path == config_path(["cron"])
        }));
    }

    #[test]
    fn validate_config_value_applies_property_and_pattern_constraints_together() {
        let schema = json!({
            "type": "object",
            "properties": {
                "foo": { "type": "string" }
            },
            "patternProperties": {
                "^f": { "minLength": 3 }
            },
            "additionalProperties": false
        });
        let diagnostics = validate_config_value(Some(&schema), &json!({"foo": "ab"}), false);

        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.severity == DiagnosticSeverity::Error
                && diagnostic.path == config_path(["foo"])
        }));
    }

    #[test]
    fn validate_config_value_reports_unique_items_as_error() {
        let schema = json!({
            "type": "object",
            "properties": {
                "tags": {
                    "type": "array",
                    "items": { "type": "string" },
                    "uniqueItems": true
                }
            }
        });
        let diagnostics = validate_config_value(Some(&schema), &json!({"tags": ["a", "a"]}), false);

        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.severity == DiagnosticSeverity::Error
                && diagnostic.path == config_path(["tags"])
        }));
    }

    #[test]
    fn non_null_config_without_schema_reports_schema_missing() {
        let diagnostics =
            validate_config_value(None, &json!({"endpoint": "https://docs.local"}), true);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].severity, DiagnosticSeverity::Warning);
        assert_eq!(
            diagnostics[0].message,
            "schema missing; using generic structured editor"
        );
    }

    #[test]
    fn effective_null_plugin_config_is_valid_for_fresh_static_plugin() {
        let sources = crate::backend::ConfigJsonSources {
            config_path: std::path::PathBuf::from("config.json"),
            config_found: false,
            file: json!({}),
            effective: json!({
                "plugins": {
                    "list": {
                        "agena.fs": {
                            "enabled": true,
                            "package": {
                                "kind": "static"
                            },
                            "config": null
                        }
                    }
                }
            }),
        };
        let status = agena::plugin::status::PluginStatus::initial("agena.fs", "static");

        let plugin = build_plugin_workbench_plugin(&sources, "en-US", status, None, Vec::new());
        assert!(plugin.configured_plugin_value.is_some());
        assert_eq!(plugin.saved_config, JsonValue::Null);
        assert!(
            plugin.config_status.kind == PluginConfigStatusKind::Valid,
            "{:?}",
            plugin.diagnostics
        );
        assert_eq!(plugin.config_status.label, "Valid");
        assert!(plugin.diagnostics.is_empty());
    }

    #[test]
    fn null_config_with_schema_materializes_full_edit_structure_without_dirty() {
        let schema = json!({
            "type": "object",
            "default": {
                "endpoint": "https://docs.local",
                "limits": {
                    "timeoutMs": 5000
                }
            },
            "properties": {
                "endpoint": {
                    "type": "string",
                    "format": "uri"
                },
                "limits": {
                    "type": "object",
                    "properties": {
                        "timeoutMs": {
                            "type": "integer",
                            "default": 5000
                        },
                        "enabled": {
                            "type": "boolean",
                            "default": true
                        }
                    }
                }
            }
        });

        let value = materialized_config_value(Some(&schema), &JsonValue::Null);

        assert_eq!(value["endpoint"], json!("https://docs.local"));
        assert_eq!(value["limits"]["timeoutMs"], json!(5000));
        assert_eq!(value["limits"]["enabled"], json!(true));
        let mut plugin = PluginWorkbenchPlugin {
            plugin_id: "fixture".to_owned(),
            visible_tool: "fixture".to_owned(),
            version: "0.1.0".to_owned(),
            transport: "static".to_owned(),
            tools: Vec::new(),
            commands: Vec::new(),
            config_status: PluginConfigStatus {
                kind: PluginConfigStatusKind::Valid,
                label: "Valid".to_owned(),
            },
            status: agena::plugin::status::PluginStatus::initial("fixture", "static"),
            inspect: None,
            configured_plugin_value: None,
            saved_override: JsonValue::Null,
            draft_override: JsonValue::Null,
            default_config: value.clone(),
            saved_config: value.clone(),
            draft_config: value.clone(),
            schema: Some(schema.clone()),
            schema_missing: false,
            diagnostics: Vec::new(),
            runtime_diagnostics: Vec::new(),
            diff: Vec::new(),
            sections: Vec::new(),
            logs: Vec::new(),
            dirty: false,
            branch_drafts: BTreeMap::new(),
        };
        plugin.sections = build_generic_config_sections(&plugin);
        assert!(
            plugin
                .sections
                .iter()
                .any(|section| section.title == "Limits")
        );
    }

    #[test]
    fn partial_config_with_schema_overrides_defaults_but_keeps_missing_fields() {
        let schema = json!({
            "type": "object",
            "default": {
                "endpoint": "https://docs.local",
                "limits": {
                    "timeoutMs": 5000,
                    "enabled": true
                }
            },
            "properties": {
                "endpoint": {
                    "type": "string"
                },
                "limits": {
                    "type": "object",
                    "properties": {
                        "timeoutMs": {
                            "type": "integer"
                        },
                        "enabled": {
                            "type": "boolean"
                        }
                    }
                }
            }
        });
        let local = json!({
            "limits": {
                "timeoutMs": 8000
            }
        });

        let value = materialized_config_value(Some(&schema), &local);

        assert_eq!(value["endpoint"], json!("https://docs.local"));
        assert_eq!(value["limits"]["timeoutMs"], json!(8000));
        assert_eq!(value["limits"]["enabled"], json!(true));
    }

    #[test]
    fn plugin_builder_uses_manifest_schema_for_effective_null_config() {
        let schema = json!({
            "type": "object",
            "default": {
                "enabled": true,
                "limit": 3
            },
            "properties": {
                "enabled": {
                    "type": "boolean"
                },
                "limit": {
                    "type": "integer",
                    "minimum": 1
                }
            }
        });
        let manifest = agena::plugin::PluginManifest::builder("fixture", "0.1.0")
            .config_schema(schema)
            .build();
        let status = agena::plugin::status::PluginStatus::initial("fixture.plugin", "static");
        let inspect = agena::plugin::PluginInspect {
            status: status.clone(),
            manifest: Some(manifest),
            authority: None,
            configured_plugin: Some(agena::plugin::ConfiguredPlugin::static_config(
                JsonValue::Null,
            )),
        };
        let sources = crate::backend::ConfigJsonSources {
            config_path: std::path::PathBuf::from("config.json"),
            config_found: false,
            file: json!({}),
            effective: json!({}),
        };

        let plugin =
            build_plugin_workbench_plugin(&sources, "en-US", status, Some(inspect), Vec::new());

        assert_eq!(plugin.saved_config["enabled"], json!(true));
        assert_eq!(plugin.saved_config["limit"], json!(3));
        assert_eq!(plugin.draft_config, plugin.saved_config);
        assert!(!plugin.dirty);
        assert!(
            plugin.config_status.kind == PluginConfigStatusKind::Valid,
            "status={:?} diagnostics={:?}",
            plugin.config_status.kind,
            plugin.diagnostics
        );
        assert!(
            plugin
                .sections
                .iter()
                .any(|section| section.title == "Enabled" || section.title == "Limit")
        );
    }

    #[test]
    fn schema_lab_default_config_is_valid_without_local_overrides() {
        let plugin_impl = agena::tool::new_schema_lab_plugin();
        let manifest = agena::plugin::sdk::Plugin::manifest(&plugin_impl);
        let status = agena::plugin::status::PluginStatus::initial(
            agena::tool::schema_lab_plugin_id(),
            "static",
        );
        let inspect = agena::plugin::PluginInspect {
            status: status.clone(),
            manifest: Some(manifest),
            authority: None,
            configured_plugin: Some(agena::plugin::ConfiguredPlugin::static_config(
                JsonValue::Null,
            )),
        };
        let sources = crate::backend::ConfigJsonSources {
            config_path: std::path::PathBuf::from("config.json"),
            config_found: false,
            file: json!({}),
            effective: json!({}),
        };

        let plugin =
            build_plugin_workbench_plugin(&sources, "en-US", status, Some(inspect), Vec::new());

        assert!(
            plugin.config_status.kind == PluginConfigStatusKind::Valid,
            "status={:?} diagnostics={:?}",
            plugin.config_status.kind,
            plugin.diagnostics
        );
        assert!(plugin.diagnostics.is_empty(), "{:?}", plugin.diagnostics);
        assert!(plugin.runtime_diagnostics.is_empty());
    }

    #[test]
    fn schema_lab_saved_full_default_keeps_maps_section_shape() {
        let plugin_impl = agena::tool::new_schema_lab_plugin();
        let manifest = agena::plugin::sdk::Plugin::manifest(&plugin_impl);
        let saved_full_config = manifest
            .config_schema
            .as_ref()
            .and_then(|schema| schema.get("default"))
            .cloned()
            .expect("schema_lab default config");
        let status = agena::plugin::status::PluginStatus::initial(
            agena::tool::schema_lab_plugin_id(),
            "static",
        );
        let sources = crate::backend::ConfigJsonSources {
            config_path: std::path::PathBuf::from("config.json"),
            config_found: false,
            file: json!({}),
            effective: json!({}),
        };
        let plugin_without_local = build_plugin_workbench_plugin(
            &sources,
            "en-US",
            status.clone(),
            Some(agena::plugin::PluginInspect {
                status: status.clone(),
                manifest: Some(manifest.clone()),
                authority: None,
                configured_plugin: Some(agena::plugin::ConfiguredPlugin::static_config(
                    JsonValue::Null,
                )),
            }),
            Vec::new(),
        );
        let plugin_with_saved_full = build_plugin_workbench_plugin(
            &sources,
            "en-US",
            status.clone(),
            Some(agena::plugin::PluginInspect {
                status,
                manifest: Some(manifest),
                authority: None,
                configured_plugin: Some(agena::plugin::ConfiguredPlugin::static_config(
                    saved_full_config,
                )),
            }),
            Vec::new(),
        );

        let section_without_local = plugin_without_local
            .sections
            .iter()
            .find(|section| section.title == "Maps")
            .expect("maps section without local config");
        let section_with_saved_full = plugin_with_saved_full
            .sections
            .iter()
            .find(|section| section.title == "Maps")
            .expect("maps section with saved config");

        let groups_without_local = section_form_groups(section_without_local);
        let groups_with_saved_full = section_form_groups(section_with_saved_full);
        assert_eq!(groups_without_local.len(), 1);
        assert_eq!(groups_with_saved_full.len(), 1);
        let rows_without_local = &groups_without_local[0].rows;
        let rows_with_saved_full = &groups_with_saved_full[0].rows;
        assert_eq!(
            rows_without_local
                .iter()
                .map(|row| row.title.as_str())
                .collect::<Vec<_>>(),
            rows_with_saved_full
                .iter()
                .map(|row| row.title.as_str())
                .collect::<Vec<_>>()
        );
        assert!(
            rows_without_local
                .iter()
                .all(|row| matches!(row.editor, ConfigRowEditor::Structured { .. }))
        );
        assert!(
            rows_with_saved_full
                .iter()
                .all(|row| matches!(row.editor, ConfigRowEditor::Structured { .. }))
        );
    }

    #[test]
    fn schema_lab_saved_full_default_keeps_collection_mesh_shape() {
        let plugin_impl = agena::tool::new_schema_lab_plugin();
        let manifest = agena::plugin::sdk::Plugin::manifest(&plugin_impl);
        let saved_full_config = manifest
            .config_schema
            .as_ref()
            .and_then(|schema| schema.get("default"))
            .cloned()
            .expect("schema_lab default config");
        let status = agena::plugin::status::PluginStatus::initial(
            agena::tool::schema_lab_plugin_id(),
            "static",
        );
        let sources = crate::backend::ConfigJsonSources {
            config_path: std::path::PathBuf::from("config.json"),
            config_found: false,
            file: json!({}),
            effective: json!({}),
        };
        let plugin_without_local = build_plugin_workbench_plugin(
            &sources,
            "en-US",
            status.clone(),
            Some(agena::plugin::PluginInspect {
                status: status.clone(),
                manifest: Some(manifest.clone()),
                authority: None,
                configured_plugin: Some(agena::plugin::ConfiguredPlugin::static_config(
                    JsonValue::Null,
                )),
            }),
            Vec::new(),
        );
        let plugin_with_saved_full = build_plugin_workbench_plugin(
            &sources,
            "en-US",
            status.clone(),
            Some(agena::plugin::PluginInspect {
                status,
                manifest: Some(manifest),
                authority: None,
                configured_plugin: Some(agena::plugin::ConfiguredPlugin::static_config(
                    saved_full_config,
                )),
            }),
            Vec::new(),
        );

        let section_without_local = plugin_without_local
            .sections
            .iter()
            .find(|section| section.title == "Collection Mesh")
            .expect("collection mesh section without local config");
        let section_with_saved_full = plugin_with_saved_full
            .sections
            .iter()
            .find(|section| section.title == "Collection Mesh")
            .expect("collection mesh section with saved config");

        let groups_without_local = section_form_groups(section_without_local);
        let groups_with_saved_full = section_form_groups(section_with_saved_full);
        assert_eq!(groups_without_local.len(), 1);
        assert_eq!(groups_with_saved_full.len(), 1);
        let rows_without_local = &groups_without_local[0].rows;
        let rows_with_saved_full = &groups_with_saved_full[0].rows;
        assert_eq!(
            rows_without_local
                .iter()
                .map(|row| row.title.as_str())
                .collect::<Vec<_>>(),
            rows_with_saved_full
                .iter()
                .map(|row| row.title.as_str())
                .collect::<Vec<_>>()
        );
        assert!(
            rows_without_local
                .iter()
                .all(|row| matches!(row.editor, ConfigRowEditor::Structured { .. }))
        );
        assert!(
            rows_with_saved_full
                .iter()
                .all(|row| matches!(row.editor, ConfigRowEditor::Structured { .. }))
        );
    }

    #[test]
    fn schema_lab_builds_multi_select_rows_for_enum_arrays() {
        let plugin_impl = agena::tool::new_schema_lab_plugin();
        let manifest = agena::plugin::sdk::Plugin::manifest(&plugin_impl);
        let status = agena::plugin::status::PluginStatus::initial(
            agena::tool::schema_lab_plugin_id(),
            "static",
        );
        let inspect = agena::plugin::PluginInspect {
            status: status.clone(),
            manifest: Some(manifest),
            authority: None,
            configured_plugin: Some(agena::plugin::ConfiguredPlugin::static_config(
                JsonValue::Null,
            )),
        };
        let sources = crate::backend::ConfigJsonSources {
            config_path: std::path::PathBuf::from("config.json"),
            config_found: false,
            file: json!({}),
            effective: json!({}),
        };
        let plugin =
            build_plugin_workbench_plugin(&sources, "en-US", status, Some(inspect), Vec::new());

        let enabled_regions = build_row_for_path(
            &plugin,
            config_path(["experiments", "enabled_regions"]),
            "Enabled Regions",
            None,
        );
        assert!(matches!(
            enabled_regions.editor,
            ConfigRowEditor::MultiEnum { .. }
        ));

        let nested_labels = build_row_for_path(
            &plugin,
            config_path(["maps", "region_policies", "apac", "labels"]),
            "Labels",
            None,
        );
        assert!(matches!(
            nested_labels.editor,
            ConfigRowEditor::MultiEnum { .. }
        ));
    }

    #[test]
    fn schema_lab_nested_drilldown_groups_preserve_map_and_list_layers() {
        let plugin_impl = agena::tool::new_schema_lab_plugin();
        let manifest = agena::plugin::sdk::Plugin::manifest(&plugin_impl);
        let status = agena::plugin::status::PluginStatus::initial(
            agena::tool::schema_lab_plugin_id(),
            "static",
        );
        let inspect = agena::plugin::PluginInspect {
            status: status.clone(),
            manifest: Some(manifest),
            authority: None,
            configured_plugin: Some(agena::plugin::ConfiguredPlugin::static_config(
                JsonValue::Null,
            )),
        };
        let sources = crate::backend::ConfigJsonSources {
            config_path: std::path::PathBuf::from("config.json"),
            config_found: false,
            file: json!({}),
            effective: json!({}),
        };
        let plugin =
            build_plugin_workbench_plugin(&sources, "en-US", status, Some(inspect), Vec::new());

        let maps_groups = build_drilldown_groups(&plugin, &config_path(["maps"]), "Maps");
        let region_row = maps_groups
            .iter()
            .flat_map(|group| group.rows.iter())
            .find(|row| row.primary_path == config_path(["maps", "region_policies"]))
            .expect("region policies row");
        assert!(matches!(
            region_row.editor,
            ConfigRowEditor::Structured { .. }
        ));

        let region_groups = build_drilldown_groups(
            &plugin,
            &config_path(["maps", "region_policies", "apac"]),
            "APAC",
        );
        let labels_row = region_groups
            .iter()
            .flat_map(|group| group.rows.iter())
            .find(|row| {
                row.primary_path == config_path(["maps", "region_policies", "apac", "labels"])
            })
            .expect("labels row");
        assert!(matches!(
            labels_row.editor,
            ConfigRowEditor::MultiEnum { .. }
        ));
    }

    #[test]
    fn schema_lab_collection_mesh_preserves_cross_nested_collection_layers() {
        let plugin_impl = agena::tool::new_schema_lab_plugin();
        let manifest = agena::plugin::sdk::Plugin::manifest(&plugin_impl);
        let status = agena::plugin::status::PluginStatus::initial(
            agena::tool::schema_lab_plugin_id(),
            "static",
        );
        let inspect = agena::plugin::PluginInspect {
            status: status.clone(),
            manifest: Some(manifest),
            authority: None,
            configured_plugin: Some(agena::plugin::ConfiguredPlugin::static_config(
                JsonValue::Null,
            )),
        };
        let sources = crate::backend::ConfigJsonSources {
            config_path: std::path::PathBuf::from("config.json"),
            config_found: false,
            file: json!({}),
            effective: json!({}),
        };
        let plugin =
            build_plugin_workbench_plugin(&sources, "en-US", status, Some(inspect), Vec::new());
        let edge_bucket_path = vec![
            PathSegment::Key("collection_mesh".to_owned()),
            PathSegment::Key("list_routes".to_owned()),
            PathSegment::Index(0),
            PathSegment::Key("buckets".to_owned()),
            PathSegment::Key("edge".to_owned()),
        ];
        let edge_labels_path = vec![
            PathSegment::Key("collection_mesh".to_owned()),
            PathSegment::Key("list_routes".to_owned()),
            PathSegment::Index(0),
            PathSegment::Key("buckets".to_owned()),
            PathSegment::Key("edge".to_owned()),
            PathSegment::Key("labels".to_owned()),
        ];
        let edge_weights_path = vec![
            PathSegment::Key("collection_mesh".to_owned()),
            PathSegment::Key("list_routes".to_owned()),
            PathSegment::Index(0),
            PathSegment::Key("buckets".to_owned()),
            PathSegment::Key("edge".to_owned()),
            PathSegment::Key("weights".to_owned()),
        ];
        let priority_steps_path = vec![
            PathSegment::Key("collection_mesh".to_owned()),
            PathSegment::Key("bucket_steps".to_owned()),
            PathSegment::Key("priority".to_owned()),
        ];
        let first_priority_step_path = vec![
            PathSegment::Key("collection_mesh".to_owned()),
            PathSegment::Key("bucket_steps".to_owned()),
            PathSegment::Key("priority".to_owned()),
            PathSegment::Index(0),
        ];
        let matrix_row_path = vec![
            PathSegment::Key("collection_mesh".to_owned()),
            PathSegment::Key("matrix_rows".to_owned()),
            PathSegment::Index(0),
        ];
        let first_matrix_cell_path = vec![
            PathSegment::Key("collection_mesh".to_owned()),
            PathSegment::Key("matrix_rows".to_owned()),
            PathSegment::Index(0),
            PathSegment::Index(0),
        ];

        let mesh_groups = build_drilldown_groups(
            &plugin,
            &config_path(["collection_mesh"]),
            "Collection Mesh",
        );
        let list_routes_row = mesh_groups
            .iter()
            .flat_map(|group| group.rows.iter())
            .find(|row| row.primary_path == config_path(["collection_mesh", "list_routes"]))
            .expect("list routes row");
        let bucket_steps_row = mesh_groups
            .iter()
            .flat_map(|group| group.rows.iter())
            .find(|row| row.primary_path == config_path(["collection_mesh", "bucket_steps"]))
            .expect("bucket steps row");
        let matrix_rows_row = mesh_groups
            .iter()
            .flat_map(|group| group.rows.iter())
            .find(|row| row.primary_path == config_path(["collection_mesh", "matrix_rows"]))
            .expect("matrix rows row");
        assert!(matches!(
            list_routes_row.editor,
            ConfigRowEditor::Structured { .. }
        ));
        assert!(matches!(
            bucket_steps_row.editor,
            ConfigRowEditor::Structured { .. }
        ));
        assert!(matches!(
            matrix_rows_row.editor,
            ConfigRowEditor::Structured { .. }
        ));

        let bucket_groups = build_drilldown_groups(&plugin, &edge_bucket_path, "Edge");
        let labels_row = bucket_groups
            .iter()
            .flat_map(|group| group.rows.iter())
            .find(|row| row.primary_path == edge_labels_path)
            .expect("edge labels row");
        let weights_row = bucket_groups
            .iter()
            .flat_map(|group| group.rows.iter())
            .find(|row| row.primary_path == edge_weights_path)
            .expect("edge weights row");
        assert!(matches!(
            labels_row.editor,
            ConfigRowEditor::MultiEnum { .. }
        ));
        assert!(matches!(
            weights_row.editor,
            ConfigRowEditor::Structured { .. }
        ));

        let step_groups = build_drilldown_groups(&plugin, &priority_steps_path, "Priority");
        let first_step_row = step_groups
            .iter()
            .flat_map(|group| group.rows.iter())
            .find(|row| row.primary_path == first_priority_step_path)
            .expect("first step row");
        assert!(matches!(
            first_step_row.editor,
            ConfigRowEditor::Structured { .. }
        ));

        let matrix_item_groups = build_drilldown_groups(&plugin, &matrix_row_path, "Row 0");
        let first_cell_row = matrix_item_groups
            .iter()
            .flat_map(|group| group.rows.iter())
            .find(|row| row.primary_path == first_matrix_cell_path)
            .expect("first cell row");
        assert!(matches!(
            first_cell_row.editor,
            ConfigRowEditor::Structured { .. }
        ));
    }

    #[test]
    fn rebuild_drilldown_stack_keeps_nested_overlays() {
        let plugin_impl = agena::tool::new_schema_lab_plugin();
        let manifest = agena::plugin::sdk::Plugin::manifest(&plugin_impl);
        let status = agena::plugin::status::PluginStatus::initial(
            agena::tool::schema_lab_plugin_id(),
            "static",
        );
        let inspect = agena::plugin::PluginInspect {
            status: status.clone(),
            manifest: Some(manifest),
            authority: None,
            configured_plugin: Some(agena::plugin::ConfiguredPlugin::static_config(
                JsonValue::Null,
            )),
        };
        let sources = crate::backend::ConfigJsonSources {
            config_path: std::path::PathBuf::from("config.json"),
            config_found: false,
            file: json!({}),
            effective: json!({}),
        };
        let plugin =
            build_plugin_workbench_plugin(&sources, "en-US", status, Some(inspect), Vec::new());
        let dialog = PluginWorkbenchOverlay {
            title: "Plugins".to_owned(),
            query: Editor::default(),
            mode: PluginWorkbenchMode::Detail,
            transport_filter: PluginTransportFilter::All,
            config_filter: PluginConfigFilter::All,
            plugins: vec![plugin.clone()],
            visible_plugins: vec![0],
            selected_plugin: 0,
            detail_tab: PluginDetailTab::Config,
            config_view: PluginConfigView::Effective,
            config_focus: PluginConfigFocus::Structure,
            selected_toolbar_action: 0,
            selected_section: 0,
            selected_node: 0,
            selected_cell: ConfigRowCell::Value,
            selected_diagnostic: 0,
            selected_diff_row: 0,
            config_scroll: 0,
            diagnostics_scroll: 0,
            show_diff: false,
            drilldown_stack: vec![
                PluginConfigDrilldownOverlay {
                    plugin_id: plugin.plugin_id.clone(),
                    path: config_path(["maps"]),
                    title: "Maps".to_owned(),
                    groups: build_drilldown_groups(&plugin, &config_path(["maps"]), "Maps"),
                    selected_row: 0,
                    selected_cell: ConfigRowCell::Value,
                },
                PluginConfigDrilldownOverlay {
                    plugin_id: plugin.plugin_id.clone(),
                    path: config_path(["maps", "region_policies", "apac"]),
                    title: "APAC".to_owned(),
                    groups: build_drilldown_groups(
                        &plugin,
                        &config_path(["maps", "region_policies", "apac"]),
                        "APAC",
                    ),
                    selected_row: 0,
                    selected_cell: ConfigRowCell::Value,
                },
            ],
            actions: None,
            selection: None,
            editor: None,
        };

        let rebuilt = rebuild_drilldown_stack(&dialog, dialog.drilldown_stack.as_slice());
        assert_eq!(rebuilt.len(), 2);
        assert_eq!(rebuilt[0].path, config_path(["maps"]));
        assert_eq!(
            rebuilt[1].path,
            config_path(["maps", "region_policies", "apac"])
        );
    }

    #[test]
    fn array_item_helpers_duplicate_move_and_remove_nested_items() {
        let mut value = json!({
            "items": [
                { "name": "alpha" },
                { "name": "beta" },
                { "name": "gamma" }
            ]
        });
        let item_path = vec![PathSegment::Key("items".to_owned()), PathSegment::Index(1)];

        let duplicated =
            duplicate_array_item_at_path(&mut value, item_path.as_slice()).expect("duplicate");
        assert_eq!(
            duplicated,
            vec![PathSegment::Key("items".to_owned()), PathSegment::Index(2)]
        );
        assert_eq!(
            value["items"]
                .as_array()
                .unwrap()
                .iter()
                .map(|item| item["name"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["alpha", "beta", "beta", "gamma"]
        );

        let moved = move_array_item_at_path(&mut value, duplicated.as_slice(), 1).expect("move");
        assert_eq!(
            moved,
            vec![PathSegment::Key("items".to_owned()), PathSegment::Index(3)]
        );
        assert_eq!(
            value["items"]
                .as_array()
                .unwrap()
                .iter()
                .map(|item| item["name"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["alpha", "beta", "gamma", "beta"]
        );

        let focus_after_remove =
            remove_array_item_at_path(&mut value, moved.as_slice()).expect("remove");
        assert_eq!(
            focus_after_remove,
            vec![PathSegment::Key("items".to_owned()), PathSegment::Index(2)]
        );
        assert_eq!(
            value["items"]
                .as_array()
                .unwrap()
                .iter()
                .map(|item| item["name"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["alpha", "beta", "gamma"]
        );
    }

    #[test]
    fn schema_lab_append_default_array_item_supports_empty_nested_union_arrays() {
        let plugin_impl = agena::tool::new_schema_lab_plugin();
        let manifest = agena::plugin::sdk::Plugin::manifest(&plugin_impl);
        let status = agena::plugin::status::PluginStatus::initial(
            agena::tool::schema_lab_plugin_id(),
            "static",
        );
        let inspect = agena::plugin::PluginInspect {
            status: status.clone(),
            manifest: Some(manifest),
            authority: None,
            configured_plugin: Some(agena::plugin::ConfiguredPlugin::static_config(
                JsonValue::Null,
            )),
        };
        let sources = crate::backend::ConfigJsonSources {
            config_path: std::path::PathBuf::from("config.json"),
            config_found: false,
            file: json!({}),
            effective: json!({}),
        };
        let plugin =
            build_plugin_workbench_plugin(&sources, "en-US", status, Some(inspect), Vec::new());
        let mut value = plugin.draft_config.clone();
        value["collection_mesh"]["bucket_steps"]["priority"] = json!([]);
        let target_path = vec![
            PathSegment::Key("collection_mesh".to_owned()),
            PathSegment::Key("bucket_steps".to_owned()),
            PathSegment::Key("priority".to_owned()),
        ];

        let focus = append_default_array_item_at_path(
            &mut value,
            plugin.schema.as_ref(),
            target_path.as_slice(),
        )
        .expect("appended item");

        assert_eq!(
            focus,
            vec![
                PathSegment::Key("collection_mesh".to_owned()),
                PathSegment::Key("bucket_steps".to_owned()),
                PathSegment::Key("priority".to_owned()),
                PathSegment::Index(0)
            ]
        );
        assert_eq!(
            value["collection_mesh"]["bucket_steps"]["priority"][0]["kind"],
            json!("delay")
        );
        assert_eq!(
            value["collection_mesh"]["bucket_steps"]["priority"][0]["ms"],
            json!(1)
        );
    }

    #[test]
    fn schema_lab_insert_default_array_item_uses_nested_union_item_schema() {
        let plugin_impl = agena::tool::new_schema_lab_plugin();
        let manifest = agena::plugin::sdk::Plugin::manifest(&plugin_impl);
        let status = agena::plugin::status::PluginStatus::initial(
            agena::tool::schema_lab_plugin_id(),
            "static",
        );
        let inspect = agena::plugin::PluginInspect {
            status: status.clone(),
            manifest: Some(manifest),
            authority: None,
            configured_plugin: Some(agena::plugin::ConfiguredPlugin::static_config(
                JsonValue::Null,
            )),
        };
        let sources = crate::backend::ConfigJsonSources {
            config_path: std::path::PathBuf::from("config.json"),
            config_found: false,
            file: json!({}),
            effective: json!({}),
        };
        let plugin =
            build_plugin_workbench_plugin(&sources, "en-US", status, Some(inspect), Vec::new());
        let mut value = plugin.draft_config.clone();
        let target_path = vec![
            PathSegment::Key("collection_mesh".to_owned()),
            PathSegment::Key("bucket_steps".to_owned()),
            PathSegment::Key("priority".to_owned()),
            PathSegment::Index(0),
        ];

        let focus = insert_default_array_item_at_path(
            &mut value,
            plugin.schema.as_ref(),
            target_path.as_slice(),
            true,
        )
        .expect("inserted item");

        assert_eq!(
            focus,
            vec![
                PathSegment::Key("collection_mesh".to_owned()),
                PathSegment::Key("bucket_steps".to_owned()),
                PathSegment::Key("priority".to_owned()),
                PathSegment::Index(1)
            ]
        );
        assert_eq!(
            value["collection_mesh"]["bucket_steps"]["priority"]
                .as_array()
                .unwrap()
                .len(),
            3
        );
        assert_eq!(
            value["collection_mesh"]["bucket_steps"]["priority"][1]["kind"],
            json!("delay")
        );
        assert_eq!(
            value["collection_mesh"]["bucket_steps"]["priority"][1]["ms"],
            json!(1)
        );
    }

    #[test]
    fn tuple_arrays_do_not_offer_structural_actions() {
        let plugin_impl = agena::tool::new_schema_lab_plugin();
        let manifest = agena::plugin::sdk::Plugin::manifest(&plugin_impl);
        let status = agena::plugin::status::PluginStatus::initial(
            agena::tool::schema_lab_plugin_id(),
            "static",
        );
        let inspect = agena::plugin::PluginInspect {
            status: status.clone(),
            manifest: Some(manifest),
            authority: None,
            configured_plugin: Some(agena::plugin::ConfiguredPlugin::static_config(
                JsonValue::Null,
            )),
        };
        let sources = crate::backend::ConfigJsonSources {
            config_path: std::path::PathBuf::from("config.json"),
            config_found: false,
            file: json!({}),
            effective: json!({}),
        };
        let plugin =
            build_plugin_workbench_plugin(&sources, "en-US", status, Some(inspect), Vec::new());
        let info = array_item_action_info(
            &plugin,
            &[
                PathSegment::Key("tuples".to_owned()),
                PathSegment::Key("command".to_owned()),
                PathSegment::Index(0),
            ],
        )
        .expect("tuple item info");
        assert!(!info.can_insert_before);
        assert!(!info.can_insert_after);
        assert!(!info.can_duplicate);
        assert!(!info.can_move_up);
        assert!(!info.can_move_down);
        assert!(!info.can_remove);
        assert!(!can_append_array_item(
            &plugin,
            &[
                PathSegment::Key("tuples".to_owned()),
                PathSegment::Key("command".to_owned()),
            ],
        ));
    }

    #[test]
    fn items_false_prevents_array_growth_actions() {
        let plugin = build_schema_fixture_plugin(
            json!({
                "type": "object",
                "default": {
                    "tuple": ["cmd"]
                },
                "properties": {
                    "tuple": {
                        "type": "array",
                        "prefixItems": [
                            { "type": "string" }
                        ],
                        "items": false
                    }
                }
            }),
            JsonValue::Null,
        );
        let path = config_path(["tuple"]);

        assert!(!can_append_array_item(&plugin, &path));

        let mut value = plugin.draft_config.clone();
        assert!(
            append_default_array_item_at_path(&mut value, plugin.schema.as_ref(), &path).is_none()
        );
    }

    #[test]
    fn arrays_without_items_keyword_allow_tail_growth() {
        let plugin = build_schema_fixture_plugin(
            json!({
                "type": "object",
                "default": {
                    "events": []
                },
                "properties": {
                    "events": {
                        "type": "array"
                    }
                }
            }),
            JsonValue::Null,
        );
        let path = config_path(["events"]);

        assert!(can_append_array_item(&plugin, &path));

        let mut value = plugin.draft_config.clone();
        let first = append_default_array_item_at_path(&mut value, plugin.schema.as_ref(), &path)
            .expect("append unconstrained item");
        assert_eq!(
            first,
            vec![PathSegment::Key("events".to_owned()), PathSegment::Index(0)]
        );
        assert_eq!(value["events"][0], JsonValue::Null);

        let second = insert_default_array_item_at_path(
            &mut value,
            plugin.schema.as_ref(),
            first.as_slice(),
            true,
        )
        .expect("insert unconstrained item");
        assert_eq!(
            second,
            vec![PathSegment::Key("events".to_owned()), PathSegment::Index(1)]
        );
        assert_eq!(value["events"], json!([null, null]));
    }

    #[test]
    fn schema_missing_arrays_allow_generic_tail_growth() {
        let sources = test_sources();
        let status = agena::plugin::status::PluginStatus::initial("fixture.plugin", "static");
        let mut plugin = build_plugin_workbench_plugin(&sources, "en-US", status, None, Vec::new());
        plugin.draft_config = json!({
            "items": []
        });

        let path = config_path(["items"]);
        assert!(can_append_array_item(&plugin, &path));

        let mut value = plugin.draft_config.clone();
        let focus =
            append_default_array_item_at_path(&mut value, None, path.as_slice()).expect("append");
        assert_eq!(
            focus,
            vec![PathSegment::Key("items".to_owned()), PathSegment::Index(0)]
        );
        assert_eq!(value["items"], json!([null]));
    }

    #[test]
    fn hybrid_prefix_item_arrays_allow_tail_appends_without_mutating_tuple_slots() {
        let plugin_impl = agena::tool::new_schema_lab_plugin();
        let manifest = agena::plugin::sdk::Plugin::manifest(&plugin_impl);
        let status = agena::plugin::status::PluginStatus::initial(
            agena::tool::schema_lab_plugin_id(),
            "static",
        );
        let inspect = agena::plugin::PluginInspect {
            status: status.clone(),
            manifest: Some(manifest),
            authority: None,
            configured_plugin: Some(agena::plugin::ConfiguredPlugin::static_config(
                JsonValue::Null,
            )),
        };
        let sources = crate::backend::ConfigJsonSources {
            config_path: std::path::PathBuf::from("config.json"),
            config_found: false,
            file: json!({}),
            effective: json!({}),
        };
        let plugin =
            build_plugin_workbench_plugin(&sources, "en-US", status, Some(inspect), Vec::new());
        let path = vec![
            PathSegment::Key("tuples".to_owned()),
            PathSegment::Key("command_with_tail".to_owned()),
        ];
        assert!(can_append_array_item(&plugin, path.as_slice()));

        let mut value = plugin.draft_config.clone();
        let focus =
            append_default_array_item_at_path(&mut value, plugin.schema.as_ref(), path.as_slice())
                .expect("append tail item");
        assert_eq!(
            focus,
            vec![
                PathSegment::Key("tuples".to_owned()),
                PathSegment::Key("command_with_tail".to_owned()),
                PathSegment::Index(4)
            ]
        );
        assert_eq!(
            value["tuples"]["command_with_tail"],
            json!(["node", "worker.mjs", "--watch", "--json", ""])
        );

        let tuple_slot = array_item_action_info(
            &plugin,
            &[
                PathSegment::Key("tuples".to_owned()),
                PathSegment::Key("command_with_tail".to_owned()),
                PathSegment::Index(0),
            ],
        )
        .expect("tuple slot actions");
        assert!(!tuple_slot.can_insert_before);
        assert!(!tuple_slot.can_insert_after);
        assert!(!tuple_slot.can_duplicate);
        assert!(!tuple_slot.can_remove);

        let tail_item = array_item_action_info(
            &plugin,
            &[
                PathSegment::Key("tuples".to_owned()),
                PathSegment::Key("command_with_tail".to_owned()),
                PathSegment::Index(2),
            ],
        )
        .expect("tail item actions");
        assert!(tail_item.can_insert_before);
        assert!(tail_item.can_insert_after);
        assert!(tail_item.can_duplicate);
        assert!(tail_item.can_remove);
    }

    #[test]
    fn schema_lab_new_map_keys_follow_schema_rules() {
        let plugin_impl = agena::tool::new_schema_lab_plugin();
        let manifest = agena::plugin::sdk::Plugin::manifest(&plugin_impl);
        let status = agena::plugin::status::PluginStatus::initial(
            agena::tool::schema_lab_plugin_id(),
            "static",
        );
        let inspect = agena::plugin::PluginInspect {
            status: status.clone(),
            manifest: Some(manifest),
            authority: None,
            configured_plugin: Some(agena::plugin::ConfiguredPlugin::static_config(
                JsonValue::Null,
            )),
        };
        let sources = crate::backend::ConfigJsonSources {
            config_path: std::path::PathBuf::from("config.json"),
            config_found: false,
            file: json!({}),
            effective: json!({}),
        };
        let plugin =
            build_plugin_workbench_plugin(&sources, "en-US", status, Some(inspect), Vec::new());

        let invalid = validate_new_object_field_key(
            plugin.schema.as_ref(),
            &plugin.draft_config,
            &config_path(["maps", "headers"]),
            "demo-header",
        );
        assert!(invalid.is_err());

        let valid = validate_new_object_field_key(
            plugin.schema.as_ref(),
            &plugin.draft_config,
            &config_path(["maps", "headers"]),
            "x-extra-header",
        );
        assert!(valid.is_ok());

        let disallowed = validate_new_object_field_key(
            plugin.schema.as_ref(),
            &plugin.draft_config,
            &config_path(["identity"]),
            "not_in_schema",
        );
        assert!(disallowed.is_err());
    }

    #[test]
    fn nullable_integer_fields_do_not_use_nullable_string_editor() {
        let plugin = build_schema_fixture_plugin(
            json!({
                "type": "object",
                "properties": {
                    "limit": {
                        "type": ["null", "integer"],
                        "title": "Limit"
                    }
                }
            }),
            JsonValue::Null,
        );
        let path = config_path(["limit"]);
        let title = title_for_config_path(&plugin, &path, "limit");
        let row = build_row_for_path(&plugin, path, title.as_str(), None);

        assert!(matches!(row.editor, ConfigRowEditor::Null { .. }));
    }

    #[test]
    fn union_rows_render_an_explicit_type_selector_column() {
        let plugin = build_schema_fixture_plugin(
            json!({
                "type": "object",
                "properties": {
                    "value": {
                        "type": ["integer", "string"],
                        "title": "Value"
                    }
                }
            }),
            JsonValue::Null,
        );
        let path = config_path(["value"]);
        let title = title_for_config_path(&plugin, &path, "value");
        let row = build_row_for_path(&plugin, path, title.as_str(), None);

        assert_eq!(row.type_mode, ConfigRowTypeMode::SelectType);
        assert_eq!(row.type_display, "[ null ▾ ]");
    }

    #[test]
    fn switchable_standard_rows_expose_all_interactive_cells() {
        let plugin = build_schema_fixture_plugin(
            json!({
                "type": "object",
                "properties": {
                    "value": {
                        "type": ["integer", "string"],
                        "title": "Value"
                    }
                }
            }),
            JsonValue::Null,
        );
        let path = config_path(["value"]);
        let title = title_for_config_path(&plugin, &path, "value");
        let row = build_row_for_path(&plugin, path, title.as_str(), None);

        assert_eq!(
            row_cells(&row, ConfigGroupLayout::Standard),
            vec![
                ConfigRowCell::Type,
                ConfigRowCell::Value,
                ConfigRowCell::Default,
                ConfigRowCell::State
            ]
        );
        assert_eq!(
            move_config_row_cell(&row, ConfigGroupLayout::Standard, ConfigRowCell::Value, -1),
            Some(ConfigRowCell::Type)
        );
        assert_eq!(
            move_config_row_cell(&row, ConfigGroupLayout::Standard, ConfigRowCell::Type, 1),
            Some(ConfigRowCell::Value)
        );
        assert_eq!(
            move_config_row_cell(&row, ConfigGroupLayout::Standard, ConfigRowCell::Value, 1),
            Some(ConfigRowCell::Default)
        );
        assert_eq!(
            move_config_row_cell(&row, ConfigGroupLayout::Standard, ConfigRowCell::Default, 1),
            Some(ConfigRowCell::State)
        );
        assert!(
            move_config_row_cell(&row, ConfigGroupLayout::Standard, ConfigRowCell::Type, -1)
                .is_none()
        );
    }

    #[test]
    fn one_of_rows_render_an_explicit_shape_selector_column() {
        let plugin = build_schema_fixture_plugin(
            json!({
                "type": "object",
                "properties": {
                    "auth": {
                        "title": "Auth",
                        "oneOf": [
                            {
                                "title": "Token",
                                "type": "object",
                                "properties": {
                                    "token": { "type": "string", "default": "" }
                                }
                            },
                            {
                                "title": "Password",
                                "type": "object",
                                "properties": {
                                    "username": { "type": "string", "default": "" },
                                    "password": { "type": "string", "default": "" }
                                }
                            }
                        ]
                    }
                }
            }),
            JsonValue::Null,
        );
        let path = config_path(["auth"]);
        let title = title_for_config_path(&plugin, &path, "auth");
        let row = build_row_for_path(&plugin, path, title.as_str(), None);

        assert_eq!(row.type_mode, ConfigRowTypeMode::SelectShape);
        assert_eq!(row.type_display, "[ Token ▾ ]");
    }

    #[test]
    fn pair_rows_expose_independent_left_and_right_cells() {
        let plugin = build_schema_fixture_plugin(
            json!({
                "type": "object",
                "default": {
                    "value": 5,
                    "max": 10
                },
                "properties": {
                    "value": { "type": "integer" },
                    "max": { "type": "integer" }
                }
            }),
            JsonValue::Null,
        );
        let row = build_pair_integer_row(
            &plugin,
            "Limit",
            config_path(["value"]),
            config_path(["max"]),
            None,
        );
        let layout = ConfigGroupLayout::Pair {
            left_label: "Value",
            right_label: "Max",
        };

        assert_eq!(
            row_cells(&row, layout),
            vec![
                ConfigRowCell::Value,
                ConfigRowCell::SecondaryValue,
                ConfigRowCell::State
            ]
        );
        assert_eq!(
            config_row_cell_label(&row, layout, ConfigRowCell::Value),
            "Value"
        );
        assert_eq!(
            config_row_cell_label(&row, layout, ConfigRowCell::SecondaryValue),
            "Max"
        );
        assert_eq!(
            config_row_cell_label(&row, layout, ConfigRowCell::State),
            "State"
        );

        let line =
            pair_config_row_line_with_focus(&row, 98, Some(ConfigRowCell::SecondaryValue), false);
        let highlighted = line
            .spans
            .iter()
            .find(|span| span.style == plugin_workbench_selection_highlight_style())
            .expect("highlighted secondary value cell");
        assert!(highlighted.content.contains("10"));
    }

    #[test]
    fn nullable_string_rows_use_plain_value_cells_when_type_is_split_out() {
        let plugin = build_schema_fixture_plugin(
            json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": ["string", "null"],
                        "title": "Path"
                    }
                }
            }),
            JsonValue::Null,
        );
        let path = config_path(["path"]);
        let title = title_for_config_path(&plugin, &path, "path");
        let row = build_row_for_path(&plugin, path, title.as_str(), None);

        assert_eq!(row.type_mode, ConfigRowTypeMode::SelectType);
        assert_eq!(row.value_display, "[ Not set ]");
    }

    #[test]
    fn structured_container_rows_expose_primary_action_cells() {
        let plugin = build_schema_fixture_plugin(
            json!({
                "type": "object",
                "default": {
                    "settings": {},
                    "items": []
                },
                "properties": {
                    "settings": {
                        "type": "object",
                        "additionalProperties": { "type": "string" }
                    },
                    "items": {
                        "type": "array",
                        "items": { "type": "string", "default": "" }
                    }
                }
            }),
            JsonValue::Null,
        );

        let object_row = build_row_for_path(&plugin, config_path(["settings"]), "Settings", None);
        assert_eq!(object_row.action_display.as_deref(), Some("[ Add field ]"));
        assert_eq!(
            row_cells(&object_row, ConfigGroupLayout::Standard),
            vec![
                ConfigRowCell::Value,
                ConfigRowCell::Default,
                ConfigRowCell::Action,
                ConfigRowCell::State
            ]
        );

        let array_row = build_row_for_path(&plugin, config_path(["items"]), "Items", None);
        assert_eq!(array_row.action_display.as_deref(), Some("[ Add item ]"));
    }

    #[test]
    fn array_item_rows_expose_direct_primary_actions() {
        let plugin = build_schema_fixture_plugin(
            json!({
                "type": "object",
                "default": {
                    "items": ["a", "b"]
                },
                "properties": {
                    "items": {
                        "type": "array",
                        "items": { "type": "string", "default": "" }
                    }
                }
            }),
            JsonValue::Null,
        );
        let row = build_row_for_path(
            &plugin,
            vec![PathSegment::Key("items".to_owned()), PathSegment::Index(0)],
            "Item 0",
            None,
        );

        assert_eq!(row.action_display.as_deref(), Some("[ Insert ]"));
    }

    #[test]
    fn dynamic_map_keys_expose_rename_as_primary_action() {
        let plugin = build_schema_fixture_plugin(
            json!({
                "type": "object",
                "default": {
                    "headers": {
                        "x-demo": "alpha"
                    }
                },
                "properties": {
                    "headers": {
                        "type": "object",
                        "additionalProperties": { "type": "string" }
                    }
                }
            }),
            JsonValue::Null,
        );
        let row = build_row_for_path(&plugin, config_path(["headers", "x-demo"]), "X Demo", None);
        assert_eq!(row.action_display.as_deref(), Some("[ Rename ]"));
    }

    #[test]
    fn config_editor_renders_action_column_for_structured_groups() {
        let plugin = build_schema_fixture_plugin(
            json!({
                "type": "object",
                "default": {
                    "settings": {}
                },
                "properties": {
                    "settings": {
                        "type": "object",
                        "additionalProperties": { "type": "string" }
                    }
                }
            }),
            JsonValue::Null,
        );
        let mut dialog = build_test_dialog(plugin);
        dialog.selected_section = 1;
        dialog.clamp_selection();

        let editor = text_to_string(config_editor_text(
            &dialog,
            dialog.selected_plugin().expect("selected plugin"),
        ));
        assert!(editor.contains("Action"));
        assert!(editor.contains("[ Add field ]"));
    }

    #[test]
    fn rows_without_action_columns_keep_focus_near_the_right_edge() {
        let plugin = build_schema_fixture_plugin(
            json!({
                "type": "object",
                "properties": {
                    "value": {
                        "type": ["integer", "string"],
                        "title": "Value"
                    }
                }
            }),
            JsonValue::Null,
        );
        let row = build_row_for_path(&plugin, config_path(["value"]), "Value", None);

        assert_eq!(
            normalize_config_row_cell(&row, ConfigGroupLayout::Standard, ConfigRowCell::Action),
            ConfigRowCell::State
        );
    }

    #[test]
    fn compact_config_view_line_uses_the_selected_cell_hint() {
        let plugin = build_schema_fixture_plugin(
            json!({
                "type": "object",
                "default": {
                    "settings": {}
                },
                "properties": {
                    "settings": {
                        "type": "object",
                        "additionalProperties": { "type": "string" }
                    }
                }
            }),
            JsonValue::Null,
        );
        let mut dialog = build_test_dialog(plugin);
        let path = config_path(["settings"]);
        let (section, row, _) = {
            let selected_plugin = dialog.selected_plugin().expect("selected plugin");
            find_best_section_row_for_path(selected_plugin, dialog.config_view, &path)
                .expect("settings row")
        };
        dialog.selected_section = section;
        dialog.selected_node = row;
        dialog.selected_cell = ConfigRowCell::Action;
        dialog.clamp_selection();

        let line =
            compact_config_view_line(dialog.selected_plugin().expect("selected plugin"), &dialog);
        assert!(line.contains("Cell: Action"));
        assert!(line.contains("Enter add field"));
    }

    #[test]
    fn drilldown_footer_tracks_the_selected_cell_action() {
        let plugin = build_schema_fixture_plugin(
            json!({
                "type": "object",
                "default": {
                    "settings": {
                        "x-demo": "alpha"
                    }
                },
                "properties": {
                    "settings": {
                        "type": "object",
                        "additionalProperties": { "type": "string" }
                    }
                }
            }),
            JsonValue::Null,
        );
        let mut dialog = build_test_dialog(plugin);
        let (plugin_id, groups) = {
            let selected_plugin = dialog.selected_plugin().expect("selected plugin");
            (
                selected_plugin.plugin_id.clone(),
                build_drilldown_groups(selected_plugin, &config_path(["settings"]), "Settings"),
            )
        };
        dialog.drilldown_stack.push(PluginConfigDrilldownOverlay {
            plugin_id,
            path: config_path(["settings"]),
            title: "Settings".to_owned(),
            groups,
            selected_row: 0,
            selected_cell: ConfigRowCell::Default,
        });

        let overlay = dialog.current_drilldown().cloned().expect("drilldown");
        let footer = drilldown_footer_text(&dialog, &overlay);
        assert!(footer.contains("Enter reset field"));
    }

    #[test]
    fn state_menus_prioritize_the_row_primary_action() {
        let item_path = vec![PathSegment::Key("items".to_owned()), PathSegment::Index(0)];
        let mut actions = vec![
            PluginConfigActionItem {
                label: "Reset Field".to_owned(),
                description: String::new(),
                action: PluginConfigAction::ResetField {
                    plugin_id: "fixture.plugin".to_owned(),
                    paths: vec![item_path.clone()],
                    focus_path: item_path.clone(),
                },
            },
            PluginConfigActionItem {
                label: "Choose Type".to_owned(),
                description: String::new(),
                action: PluginConfigAction::SelectType {
                    plugin_id: "fixture.plugin".to_owned(),
                    path: item_path,
                },
            },
            PluginConfigActionItem {
                label: "Add Item".to_owned(),
                description: String::new(),
                action: PluginConfigAction::AppendArrayItem {
                    plugin_id: "fixture.plugin".to_owned(),
                    path: config_path(["items"]),
                },
            },
        ];

        let selected = prioritize_config_actions(
            actions.as_mut_slice(),
            ConfigRowCell::State,
            Some(ConfigRowPrimaryAction::AddItem),
        );
        assert_eq!(selected, 0);
        assert!(matches!(
            actions[0].action,
            PluginConfigAction::AppendArrayItem { .. }
        ));
    }

    #[test]
    fn config_editor_highlights_only_the_selected_cell() {
        let plugin = build_schema_fixture_plugin(
            json!({
                "type": "object",
                "properties": {
                    "value": {
                        "type": ["integer", "string"],
                        "title": "Value"
                    }
                }
            }),
            JsonValue::Null,
        );
        let mut dialog = build_test_dialog(plugin);
        dialog.selected_section = 1;
        dialog.selected_node = 0;
        dialog.selected_cell = ConfigRowCell::Type;
        dialog.clamp_selection();

        let text = config_editor_text(&dialog, dialog.selected_plugin().expect("selected plugin"));
        let line = text
            .lines
            .iter()
            .find(|line| {
                let rendered = line_to_string(line);
                rendered.contains("[ null ▾ ]") && rendered.contains("[ null ]")
            })
            .expect("selected config row");

        assert!(line.spans.len() > 3);
        let highlighted = line
            .spans
            .iter()
            .filter(|span| span.style == plugin_workbench_selection_highlight_style())
            .collect::<Vec<_>>();
        assert_eq!(highlighted.len(), 1);
        assert!(highlighted[0].content.contains("[ null ▾ ]"));
        assert!(line.spans.iter().any(|span| {
            span.style != plugin_workbench_selection_highlight_style()
                && span.content.contains("[ null ]")
        }));
    }

    #[test]
    fn implicit_object_shapes_do_not_offer_generic_type_switching() {
        let plugin = build_schema_fixture_plugin(
            json!({
                "type": "object",
                "properties": {
                    "filters": {
                        "title": "Filters",
                        "properties": {
                            "enabled": { "type": "boolean" }
                        }
                    }
                }
            }),
            JsonValue::Null,
        );
        let path = config_path(["filters"]);
        let title = title_for_config_path(&plugin, &path, "filters");
        let row = build_row_for_path(&plugin, path, title.as_str(), None);

        assert_eq!(row.type_mode, ConfigRowTypeMode::Fixed);
        assert_eq!(row.type_display, "object");
    }

    #[test]
    fn enum_arrays_without_unique_items_do_not_use_multi_select_editor() {
        let plugin = build_schema_fixture_plugin(
            json!({
                "type": "object",
                "default": {
                    "tags": ["alpha"]
                },
                "properties": {
                    "tags": {
                        "type": "array",
                        "items": {
                            "type": "string",
                            "enum": ["alpha", "beta", "gamma"]
                        }
                    }
                }
            }),
            JsonValue::Null,
        );
        let path = config_path(["tags"]);
        let title = title_for_config_path(&plugin, &path, "tags");
        let row = build_row_for_path(&plugin, path, title.as_str(), None);

        assert!(!matches!(row.editor, ConfigRowEditor::MultiEnum { .. }));
    }

    #[test]
    fn schema_for_path_applies_if_then_else_branch_constraints() {
        let schema = json!({
            "type": "object",
            "properties": {
                "mode": {
                    "type": "string",
                    "enum": ["basic", "advanced"]
                },
                "setting": true
            },
            "if": {
                "properties": {
                    "mode": { "const": "advanced" }
                },
                "required": ["mode"]
            },
            "then": {
                "properties": {
                    "setting": { "type": "integer", "minimum": 1 }
                }
            },
            "else": {
                "properties": {
                    "setting": { "type": "string", "minLength": 1 }
                }
            }
        });

        let advanced = json!({
            "mode": "advanced",
            "setting": 3
        });
        let advanced_schema =
            schema_for_path(&schema, &schema, &advanced, &config_path(["setting"]))
                .expect("advanced setting schema");
        assert_eq!(
            effective_schema_kind(&advanced_schema).as_deref(),
            Some("integer")
        );
        assert!(
            schema_constraints(&advanced_schema)
                .iter()
                .any(|constraint| constraint == "minimum: 1")
        );

        let basic = json!({
            "mode": "basic",
            "setting": "on"
        });
        let basic_schema = schema_for_path(&schema, &schema, &basic, &config_path(["setting"]))
            .expect("basic setting schema");
        assert_eq!(
            effective_schema_kind(&basic_schema).as_deref(),
            Some("string")
        );
        assert!(
            schema_constraints(&basic_schema)
                .iter()
                .any(|constraint| constraint == "minLength: 1")
        );
    }

    #[test]
    fn ordered_object_keys_include_all_of_declared_and_required_fields() {
        let schema = json!({
            "type": "object",
            "allOf": [
                {
                    "properties": {
                        "mode": { "type": "string" }
                    },
                    "required": ["mode"]
                },
                {
                    "properties": {
                        "cron": { "type": "string", "minLength": 1 }
                    },
                    "required": ["cron"]
                }
            ]
        });
        let value = json!({
            "mode": "scheduled"
        });

        let active_root =
            schema_for_path(&schema, &schema, &value, &Vec::new()).expect("active root schema");
        let keys =
            ordered_object_keys(Some(&active_root), value.as_object().expect("object value"));
        assert!(keys.iter().any(|key| key == "mode"));
        assert!(keys.iter().any(|key| key == "cron"));

        let cron_schema =
            schema_for_path(&schema, &schema, &value, &config_path(["cron"])).expect("cron schema");
        assert!(
            schema_constraints(&cron_schema)
                .iter()
                .any(|constraint| constraint == "minLength: 1")
        );
    }

    #[test]
    fn persisted_plugin_config_value_keeps_sparse_override_shape() {
        let schema = json!({
            "type": "object",
            "default": {
                "endpoint": "https://docs.local",
                "limits": {
                    "timeoutMs": 5000,
                    "enabled": true
                }
            },
            "properties": {
                "endpoint": { "type": "string" },
                "limits": {
                    "type": "object",
                    "properties": {
                        "timeoutMs": { "type": "integer" },
                        "enabled": { "type": "boolean" }
                    }
                }
            }
        });
        let mut plugin = build_schema_fixture_plugin(schema, JsonValue::Null);
        assert_eq!(persisted_plugin_config_value(&plugin), JsonValue::Null);

        set_value_at_path(
            &mut plugin.draft_config,
            &config_path(["limits", "timeoutMs"]),
            json!(9000),
        );
        recompute_plugin_config_state(&mut plugin);

        assert_eq!(
            persisted_plugin_config_value(&plugin),
            json!({
                "limits": {
                    "timeoutMs": 9000
                }
            })
        );
    }

    #[test]
    fn persisted_plugin_config_value_preserves_explicit_empty_objects() {
        let schema = json!({
            "type": "object",
            "properties": {
                "settings": {
                    "type": ["null", "object"],
                    "additionalProperties": false
                }
            }
        });
        let mut plugin = build_schema_fixture_plugin(schema, JsonValue::Null);
        plugin.draft_config = json!({
            "settings": {}
        });
        recompute_plugin_config_state(&mut plugin);

        assert_eq!(
            persisted_plugin_config_value(&plugin),
            json!({
                "settings": {}
            })
        );
        assert!(plugin.dirty);
    }

    #[test]
    fn normalize_override_value_preserves_empty_objects_inside_arrays() {
        assert_eq!(normalize_override_value(json!([{}])), json!([{}]));
    }

    #[test]
    fn merge_multi_enum_selection_preserves_existing_order() {
        let current = vec![json!("beta"), json!("alpha")];
        let selected = vec![json!("alpha"), json!("beta"), json!("gamma")];

        let merged = merge_multi_enum_selection(current.as_slice(), selected.as_slice());

        assert_eq!(merged, vec![json!("beta"), json!("alpha"), json!("gamma")]);
    }

    #[test]
    fn branch_draft_keys_use_branch_ids_even_when_labels_match() {
        let path = config_path(["mode"]);
        let first = plugin_branch_draft_key("fixture", &path, "branch-0");
        let second = plugin_branch_draft_key("fixture", &path, "branch-1");

        assert_ne!(first, second);
    }

    #[test]
    fn structural_changes_clear_branch_draft_cache() {
        let mut plugin = build_schema_fixture_plugin(
            json!({
                "type": "object",
                "properties": {
                    "mode": {
                        "oneOf": [
                            {
                                "title": "A",
                                "type": "object",
                                "properties": {
                                    "value": { "type": "string" }
                                }
                            },
                            {
                                "title": "B",
                                "type": "object",
                                "properties": {
                                    "enabled": { "type": "boolean" }
                                }
                            }
                        ]
                    }
                }
            }),
            JsonValue::Null,
        );
        let path = config_path(["mode"]);
        plugin.branch_drafts.insert(
            plugin_branch_draft_key("fixture.plugin", &path, "branch-0"),
            json!({"value": "demo"}),
        );

        clear_branch_drafts_for_structural_change(&mut plugin);

        assert!(plugin.branch_drafts.is_empty());
    }

    #[test]
    fn plugin_save_block_reason_reports_invalid_configs() {
        let mut plugin = build_schema_fixture_plugin(
            json!({
                "type": "object",
                "properties": {
                    "endpoint": { "type": "string", "format": "uri" }
                }
            }),
            json!({
                "endpoint": 42
            }),
        );
        recompute_plugin_config_state(&mut plugin);

        let reason = plugin_save_block_reason(&plugin).expect("save should be blocked");
        assert!(reason.contains("cannot save"));
        assert!(reason.contains("1 error"));
    }

    #[test]
    fn validate_config_value_reports_invalid_regex_patterns() {
        let string_schema = json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "pattern": "["
                }
            }
        });
        let string_diagnostics =
            validate_config_value(Some(&string_schema), &json!({"name": "demo"}), false);
        assert!(string_diagnostics.iter().any(|diagnostic| {
            diagnostic.severity == DiagnosticSeverity::Error
                && diagnostic.message.contains("invalid regex pattern")
        }));

        let object_schema = json!({
            "type": "object",
            "patternProperties": {
                "[": { "type": "string" }
            }
        });
        let object_diagnostics =
            validate_config_value(Some(&object_schema), &json!({"demo": "value"}), false);
        assert!(object_diagnostics.iter().any(|diagnostic| {
            diagnostic.severity == DiagnosticSeverity::Error
                && diagnostic
                    .message
                    .contains("invalid patternProperties regex")
        }));
    }

    #[test]
    fn validate_config_value_rejects_invalid_hostname_format() {
        let schema = json!({
            "type": "object",
            "properties": {
                "host": {
                    "type": "string",
                    "format": "hostname"
                }
            }
        });
        let diagnostics = validate_config_value(Some(&schema), &json!({"host": "bad/host"}), false);

        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.severity == DiagnosticSeverity::Error
                && diagnostic.path == config_path(["host"])
        }));
    }

    #[test]
    fn rename_rejects_values_that_do_not_match_the_new_key_schema() {
        let schema = json!({
            "type": "object",
            "properties": {
                "headers": {
                    "type": "object",
                    "patternProperties": {
                        "^x-int-": { "type": "integer" },
                        "^x-str-": { "type": "string" }
                    }
                }
            }
        });
        let value = json!({
            "headers": {
                "x-str-name": "demo"
            }
        });

        let target_schema = validate_new_object_field_key(
            Some(&schema),
            &value,
            &config_path(["headers"]),
            "x-int-name",
        )
        .expect("new key allowed")
        .expect("pattern schema");

        let error = validate_schema_value_for_path(
            &schema,
            &target_schema,
            &json!("demo"),
            &config_path(["headers", "x-int-name"]),
            "X Int Name",
        )
        .expect_err("rename should reject incompatible value");

        assert!(error.contains("declared type"));
    }

    #[test]
    fn apply_reset_paths_resets_array_groups_from_the_tail_first() {
        let mut value = json!({
            "items": ["alpha", "beta", "x", "y"]
        });
        let default_root = json!({
            "items": ["alpha", "beta"]
        });
        let paths = vec![
            vec![PathSegment::Key("items".to_owned()), PathSegment::Index(0)],
            vec![PathSegment::Key("items".to_owned()), PathSegment::Index(1)],
            vec![PathSegment::Key("items".to_owned()), PathSegment::Index(2)],
            vec![PathSegment::Key("items".to_owned()), PathSegment::Index(3)],
        ];

        let outcome = apply_reset_paths(&mut value, &default_root, None, paths.as_slice());

        assert!(outcome.changed);
        assert!(outcome.blocked.is_empty());
        assert_eq!(value, default_root);
    }

    #[test]
    fn unique_items_arrays_do_not_offer_duplicate_actions() {
        let plugin = build_schema_fixture_plugin(
            json!({
                "type": "object",
                "default": {
                    "tags": ["alpha", "beta"]
                },
                "properties": {
                    "tags": {
                        "type": "array",
                        "uniqueItems": true,
                        "items": {
                            "type": "string"
                        }
                    }
                }
            }),
            JsonValue::Null,
        );

        let info = array_item_action_info(
            &plugin,
            &[PathSegment::Key("tags".to_owned()), PathSegment::Index(0)],
        )
        .expect("item info");
        assert!(!info.can_duplicate);
        assert!(info.can_insert_before);
        assert!(info.can_insert_after);
    }

    #[test]
    fn max_properties_blocks_add_field_prompts() {
        let schema = json!({
            "type": "object",
            "properties": {
                "settings": {
                    "type": "object",
                    "maxProperties": 1,
                    "properties": {
                        "mode": { "type": "string" }
                    }
                }
            }
        });
        let value = json!({
            "settings": {
                "mode": "on"
            }
        });

        let reason =
            object_add_field_block_reason(Some(&schema), &value, &config_path(["settings"]))
                .expect("maxProperties reason");
        assert!(reason.contains("maximum of 1 field"));
    }

    #[test]
    fn write_path_block_reason_blocks_materializing_optional_fields_past_max_properties() {
        let schema = json!({
            "type": "object",
            "maxProperties": 1,
            "properties": {
                "mode": { "type": "string" },
                "cron": { "type": "string" }
            }
        });
        let value = json!({
            "mode": "scheduled"
        });

        let reason = write_path_block_reason(Some(&schema), &value, &config_path(["cron"]))
            .expect("materialization blocked");
        assert!(reason.contains("maximum of 1 field"));
    }

    #[test]
    fn staged_config_value_updates_are_atomic_when_a_later_write_fails() {
        let schema = json!({
            "type": "object",
            "maxProperties": 1,
            "properties": {
                "mode": { "type": "string" },
                "cron": { "type": "string" }
            }
        });
        let current = json!({});

        let result = apply_staged_config_value_updates(
            Some(&schema),
            &current,
            &[
                (config_path(["mode"]), json!("scheduled")),
                (config_path(["cron"]), json!("* * * * *")),
            ],
        );

        assert!(result.is_err());
        assert_eq!(current, json!({}));
    }

    #[test]
    fn closed_objects_hide_add_field_even_when_declared_rows_are_missing() {
        let schema = json!({
            "type": "object",
            "properties": {
                "settings": {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "mode": { "type": "string" }
                    }
                }
            }
        });
        let value = json!({
            "settings": {}
        });

        let reason =
            object_add_field_block_reason(Some(&schema), &value, &config_path(["settings"]))
                .expect("closed object reason");
        assert!(reason.contains("custom field names"));
    }

    #[test]
    fn array_structural_actions_respect_contains_and_max_contains() {
        let plugin = build_schema_fixture_plugin(
            json!({
                "type": "object",
                "default": {
                    "tags": ["edge"]
                },
                "properties": {
                    "tags": {
                        "type": "array",
                        "items": { "const": "edge" },
                        "contains": { "const": "edge" },
                        "maxContains": 1
                    }
                }
            }),
            JsonValue::Null,
        );
        let path = config_path(["tags"]);
        let item_path = vec![PathSegment::Key("tags".to_owned()), PathSegment::Index(0)];

        assert!(!can_append_array_item(&plugin, &path));

        let info = array_item_action_info(&plugin, &item_path).expect("item info");
        assert!(!info.can_insert_before);
        assert!(!info.can_insert_after);
        assert!(!info.can_duplicate);
        assert!(!info.can_remove);

        let mut current = json!({
            "tags": ["edge"]
        });
        let defaults = json!({
            "tags": []
        });
        let outcome = apply_reset_paths(
            &mut current,
            &defaults,
            plugin.schema.as_ref(),
            &[item_path],
        );
        assert!(!outcome.changed);
        assert!(
            outcome
                .blocked
                .iter()
                .any(|message| message.contains("matching item"))
        );
    }

    #[test]
    fn insert_schema_defaults_skips_nullable_objects_without_defaults() {
        let schema = json!({
            "type": "object",
            "properties": {
                "settings": {
                    "type": ["null", "object"],
                    "properties": {
                        "mode": { "type": "string" }
                    }
                }
            }
        });
        let mut value = json!({
            "settings": null
        });

        insert_schema_defaults(&mut value, &schema, &schema);

        assert_eq!(value["settings"], JsonValue::Null);
    }

    #[test]
    fn insert_schema_defaults_materializes_nullable_objects_with_direct_child_defaults() {
        let schema = json!({
            "type": "object",
            "properties": {
                "settings": {
                    "type": ["null", "object"],
                    "properties": {
                        "mode": {
                            "type": "string",
                            "default": "auto"
                        }
                    }
                }
            }
        });
        let mut value = json!({
            "settings": null
        });

        insert_schema_defaults(&mut value, &schema, &schema);

        assert_eq!(
            value,
            json!({
                "settings": {
                    "mode": "auto"
                }
            })
        );
    }

    #[test]
    fn reset_paths_block_object_removals_that_break_dependent_required() {
        let schema = json!({
            "type": "object",
            "properties": {
                "settings": {
                    "type": "object",
                    "properties": {
                        "mode": { "type": "string" },
                        "cron": { "type": "string" }
                    },
                    "dependentRequired": {
                        "mode": ["cron"]
                    }
                }
            }
        });
        let mut value = json!({
            "settings": {
                "mode": "scheduled",
                "cron": "* * * * *"
            }
        });
        let defaults = json!({
            "settings": {
                "mode": "scheduled"
            }
        });

        let outcome = apply_reset_paths(
            &mut value,
            &defaults,
            Some(&schema),
            &[config_path(["settings", "cron"])],
        );
        assert!(!outcome.changed);
        assert_eq!(value["settings"]["cron"], json!("* * * * *"));
        assert!(
            outcome
                .blocked
                .iter()
                .any(|message| message.contains("required because `mode` is set"))
        );
    }

    #[test]
    fn min_constraints_block_defaultless_reset_removals() {
        let schema = json!({
            "type": "object",
            "properties": {
                "settings": {
                    "type": "object",
                    "minProperties": 1,
                    "properties": {
                        "mode": { "type": "string" }
                    }
                },
                "items": {
                    "type": "array",
                    "minItems": 1,
                    "items": { "type": "string" }
                }
            }
        });

        let mut object_value = json!({
            "settings": {
                "mode": "on"
            }
        });
        let object_defaults = json!({
            "settings": {}
        });
        let object_outcome = apply_reset_paths(
            &mut object_value,
            &object_defaults,
            Some(&schema),
            &[config_path(["settings", "mode"])],
        );
        assert!(!object_outcome.changed);
        assert_eq!(object_value["settings"]["mode"], json!("on"));
        assert!(
            object_outcome
                .blocked
                .iter()
                .any(|message| message.contains("at least 1 field"))
        );

        let mut array_value = json!({
            "items": ["alpha"]
        });
        let array_defaults = json!({
            "items": []
        });
        let array_outcome = apply_reset_paths(
            &mut array_value,
            &array_defaults,
            Some(&schema),
            &[vec![
                PathSegment::Key("items".to_owned()),
                PathSegment::Index(0),
            ]],
        );
        assert!(!array_outcome.changed);
        assert_eq!(array_value["items"], json!(["alpha"]));
        assert!(
            array_outcome
                .blocked
                .iter()
                .any(|message| message.contains("at least 1 item"))
        );
    }

    #[test]
    fn rename_object_field_moves_nested_map_entries() {
        let mut value = json!({
            "headers": {
                "x-demo": "true",
                "x-region": "apac"
            }
        });
        let source_path = vec![
            PathSegment::Key("headers".to_owned()),
            PathSegment::Key("x-demo".to_owned()),
        ];

        let new_path = rename_object_field_at_path(&mut value, source_path.as_slice(), "x-lab")
            .expect("rename");

        assert_eq!(
            new_path,
            vec![
                PathSegment::Key("headers".to_owned()),
                PathSegment::Key("x-lab".to_owned())
            ]
        );
        assert_eq!(value["headers"]["x-lab"], json!("true"));
        assert!(value["headers"].get("x-demo").is_none());
    }

    #[test]
    fn missing_plugin_config_saves_as_standard_static_plugin_record() {
        let sources = crate::backend::ConfigJsonSources {
            config_path: std::path::PathBuf::from("config.json"),
            config_found: false,
            file: json!({}),
            effective: json!({}),
        };
        let status = agena::plugin::status::PluginStatus::initial("agena.web", "static");
        let mut plugin = build_plugin_workbench_plugin(&sources, "en-US", status, None, Vec::new());
        plugin.draft_config = json!({
            "endpoint": "https://docs.local",
            "timeoutMs": 8000
        });

        let mut plugin_record = plugin_config_record_value(&plugin);
        plugin_record
            .as_object_mut()
            .unwrap()
            .insert("config".to_owned(), plugin.draft_config.clone());

        assert_eq!(plugin_record["enabled"], json!(true));
        assert_eq!(plugin_record["package"]["kind"], json!("static"));
        assert_eq!(
            plugin_record["config"]["endpoint"],
            json!("https://docs.local")
        );
        assert_eq!(plugin_record["config"]["timeoutMs"], json!(8000));
    }

    #[test]
    fn config_focus_navigation_stays_inside_config_tab() {
        assert_eq!(
            next_config_focus(PluginConfigFocus::Toolbar, true),
            PluginConfigFocus::Structure
        );
        assert_eq!(
            next_config_focus(PluginConfigFocus::Structure, true),
            PluginConfigFocus::Editor
        );
        assert_eq!(
            previous_config_focus(PluginConfigFocus::Structure, true),
            PluginConfigFocus::Toolbar
        );
        assert_eq!(
            next_config_focus(PluginConfigFocus::Structure, false),
            PluginConfigFocus::Editor
        );
        assert_eq!(
            next_config_focus(PluginConfigFocus::Editor, false),
            PluginConfigFocus::Toolbar
        );
        assert_eq!(
            previous_config_focus(PluginConfigFocus::Editor, false),
            PluginConfigFocus::Structure
        );
        assert_eq!(PluginDetailTab::Config.move_by(1), PluginDetailTab::Tools);
    }

    #[test]
    fn config_tab_is_the_structured_editor_layout() {
        let schema = json!({
            "type": "object",
            "required": ["endpoint"],
            "default": {
                "endpoint": "https://docs.local",
                "limits": {
                    "timeoutMs": 5000,
                    "enabled": true
                }
            },
            "properties": {
                "endpoint": {
                    "type": "string",
                    "title": "Endpoint",
                    "format": "uri"
                },
                "limits": {
                    "type": "object",
                    "title": "Limits",
                    "properties": {
                        "timeoutMs": {
                            "type": "integer",
                            "title": "Timeout",
                            "minimum": 1
                        },
                        "enabled": {
                            "type": "boolean",
                            "title": "Enabled"
                        }
                    }
                }
            }
        });
        let manifest = agena::plugin::PluginManifest::builder("fixture", "0.1.0")
            .config_schema(schema)
            .build();
        let status = agena::plugin::status::PluginStatus::initial("fixture.plugin", "static");
        let inspect = agena::plugin::PluginInspect {
            status: status.clone(),
            manifest: Some(manifest),
            authority: None,
            configured_plugin: Some(agena::plugin::ConfiguredPlugin::static_config(
                JsonValue::Null,
            )),
        };
        let sources = crate::backend::ConfigJsonSources {
            config_path: std::path::PathBuf::from("config.json"),
            config_found: false,
            file: json!({}),
            effective: json!({}),
        };
        let plugin =
            build_plugin_workbench_plugin(&sources, "en-US", status, Some(inspect), Vec::new());
        let dialog = PluginWorkbenchOverlay {
            title: "Plugins".to_owned(),
            query: Editor::default(),
            mode: PluginWorkbenchMode::Detail,
            transport_filter: PluginTransportFilter::All,
            config_filter: PluginConfigFilter::All,
            plugins: vec![plugin],
            visible_plugins: vec![0],
            selected_plugin: 0,
            detail_tab: PluginDetailTab::Config,
            config_view: PluginConfigView::Effective,
            config_focus: PluginConfigFocus::Structure,
            selected_toolbar_action: 0,
            selected_section: 2,
            selected_node: 0,
            selected_cell: ConfigRowCell::Value,
            selected_diagnostic: 0,
            selected_diff_row: 0,
            config_scroll: 0,
            diagnostics_scroll: 0,
            show_diff: false,
            drilldown_stack: Vec::new(),
            actions: None,
            selection: None,
            editor: None,
        };

        assert_eq!(transport_display("static"), "native");

        let toolbar = text_to_string(compact_config_toolbar_text(&dialog));
        assert!(toolbar.contains("[ Validate (V) ]"));
        assert!(toolbar.contains("[ Reset All (R) ]"));
        assert!(!toolbar.contains("Format"));

        let structure = text_to_string(compact_config_sections_text(
            &dialog,
            dialog.selected_plugin().unwrap(),
            20,
        ));
        assert!(structure.contains("Overview"));
        assert!(structure.contains("Endpoint"));
        assert!(structure.contains("Limits"));

        let editor = text_to_string(config_editor_text(
            &dialog,
            dialog.selected_plugin().unwrap(),
        ));
        assert!(editor.contains("Limits"));
        assert!(editor.contains("Type"));
        assert!(editor.contains("Timeout"));
        assert!(editor.contains("Enabled"));
    }

    #[test]
    fn agena_web_schema_compiles_to_partitioned_section_editor() {
        let schema = json!({
            "type": "object",
            "default": {
                "fetch": {
                    "enabled": true,
                    "request": {
                        "delay_ms": 400,
                        "timeout_secs": 30,
                        "max_body_bytes": 5242880,
                        "respect_robots_txt": true
                    },
                    "cache": {
                        "ttl_secs": 900,
                        "capacity": 128
                    }
                },
                "crawl": {
                    "defaults": {
                        "max_pages": 10,
                        "max_depth": 1,
                        "same_host_only": true
                    },
                    "limits": {
                        "max_pages": 100,
                        "max_depth": 4
                    },
                    "indexing": {
                        "document_cache_ttl_secs": 86400,
                        "chunk_chars": 1800,
                        "near_duplicate_hamming_distance": 3
                    }
                },
                "search": {
                    "default_limit": 5,
                    "max_limit": 20
                },
                "store": {
                    "retention": {
                        "max_documents": 200,
                        "max_bytes": 104857600
                    },
                    "listing": {
                        "default_limit": 20,
                        "max_limit": 100
                    }
                },
                "browser": {
                    "enabled": false,
                    "executable_path": null,
                    "wait": {
                        "for_network_idle": true,
                        "timeout_secs": 10,
                        "for_selector": null,
                        "delay_ms": 0
                    }
                }
            },
            "properties": {
                "fetch": {
                    "type": "object",
                    "title": "Fetch",
                    "properties": {
                        "enabled": { "type": "boolean", "title": "Enabled" },
                        "request": {
                            "type": "object",
                            "title": "Request",
                            "properties": {
                                "delay_ms": { "type": "integer", "title": "Delay" },
                                "timeout_secs": { "type": "integer", "title": "Timeout" },
                                "max_body_bytes": { "type": "integer", "title": "Max Body Size" },
                                "respect_robots_txt": { "type": "boolean", "title": "Respect robots.txt" }
                            }
                        },
                        "cache": {
                            "type": "object",
                            "title": "Cache",
                            "properties": {
                                "ttl_secs": { "type": "integer", "title": "TTL" },
                                "capacity": { "type": "integer", "title": "Capacity" }
                            }
                        }
                    }
                },
                "crawl": {
                    "type": "object",
                    "title": "Crawl",
                    "properties": {
                        "defaults": {
                            "type": "object",
                            "properties": {
                                "max_pages": { "type": "integer", "title": "Max pages" },
                                "max_depth": { "type": "integer", "title": "Max depth" },
                                "same_host_only": { "type": "boolean", "title": "Same host only" }
                            }
                        },
                        "limits": {
                            "type": "object",
                            "properties": {
                                "max_pages": { "type": "integer", "title": "Max pages limit" },
                                "max_depth": { "type": "integer", "title": "Max depth limit" }
                            }
                        },
                        "indexing": {
                            "type": "object",
                            "title": "Indexing",
                            "properties": {
                                "document_cache_ttl_secs": { "type": "integer", "title": "Document cache TTL" },
                                "chunk_chars": { "type": "integer", "title": "Chunk size" },
                                "near_duplicate_hamming_distance": { "type": "integer", "title": "Near-duplicate distance" }
                            }
                        }
                    }
                },
                "search": {
                    "type": "object",
                    "title": "Search",
                    "properties": {
                        "default_limit": { "type": "integer", "title": "Default limit" },
                        "max_limit": { "type": "integer", "title": "Max limit" }
                    }
                },
                "store": {
                    "type": "object",
                    "title": "Store",
                    "properties": {
                        "retention": {
                            "type": "object",
                            "title": "Retention",
                            "properties": {
                                "max_documents": { "type": "integer", "title": "Max documents" },
                                "max_bytes": { "type": "integer", "title": "Max bytes" }
                            }
                        },
                        "listing": {
                            "type": "object",
                            "title": "Listing",
                            "properties": {
                                "default_limit": { "type": "integer", "title": "Default limit" },
                                "max_limit": { "type": "integer", "title": "Max limit" }
                            }
                        }
                    }
                },
                "browser": {
                    "type": "object",
                    "title": "Browser",
                    "properties": {
                        "enabled": { "type": "boolean", "title": "Enabled" },
                        "executable_path": { "type": ["string", "null"], "title": "Executable path" },
                        "wait": {
                            "type": "object",
                            "title": "Wait",
                            "properties": {
                                "for_network_idle": { "type": "boolean", "title": "Network idle" },
                                "timeout_secs": { "type": "integer", "title": "Timeout" },
                                "for_selector": { "type": ["string", "null"], "title": "Selector" },
                                "delay_ms": { "type": "integer", "title": "Extra delay" }
                            }
                        }
                    }
                }
            }
        });
        let manifest = agena::plugin::PluginManifest::builder("agena.web", "1.0.0")
            .config_schema(schema)
            .build();
        let status = agena::plugin::status::PluginStatus::initial("agena.web", "static");
        let inspect = agena::plugin::PluginInspect {
            status: status.clone(),
            manifest: Some(manifest),
            authority: None,
            configured_plugin: Some(agena::plugin::ConfiguredPlugin::static_config(
                JsonValue::Null,
            )),
        };
        let sources = crate::backend::ConfigJsonSources {
            config_path: std::path::PathBuf::from("config.json"),
            config_found: false,
            file: json!({}),
            effective: json!({}),
        };
        let plugin =
            build_plugin_workbench_plugin(&sources, "en-US", status, Some(inspect), Vec::new());

        let section_titles = plugin
            .sections
            .iter()
            .map(|section| section.title.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            section_titles,
            vec!["Overview", "Fetch", "Crawl", "Search", "Store", "Browser"]
        );

        let fetch_section = &plugin.sections[1];
        let fetch_groups = section_form_groups(fetch_section)
            .iter()
            .map(|group| group.title.as_str())
            .collect::<Vec<_>>();
        assert_eq!(fetch_groups, vec!["Fetch", "Request", "Cache"]);

        let crawl_section = &plugin.sections[2];
        let crawl_groups = section_form_groups(crawl_section);
        assert_eq!(crawl_groups[0].title, "Crawl Range");
        assert_eq!(
            crawl_groups[0].layout,
            ConfigGroupLayout::Pair {
                left_label: "Value",
                right_label: "Limit"
            }
        );
        assert_eq!(crawl_groups[1].title, "Indexing");

        let browser_section = &plugin.sections[5];
        let browser_groups = section_form_groups(browser_section);
        assert_eq!(browser_groups[0].title, "Browser");
        assert_eq!(browser_groups[1].title, "Wait");
        assert!(browser_groups[0].rows[1].value_display.contains("Not set"));
        assert!(browser_groups[1].rows[2].value_display.contains("Not set"));
    }

    fn text_to_string(text: Text<'static>) -> String {
        text.lines
            .into_iter()
            .map(|line| {
                line.spans
                    .into_iter()
                    .map(|span| span.content.into_owned())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}
