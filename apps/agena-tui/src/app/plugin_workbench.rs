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
    config_focus: PluginConfigFocus,
    selected_node: usize,
    config_scroll: usize,
    diagnostics_scroll: usize,
    show_diff: bool,
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
    Overview,
    Config,
    Tools,
    Commands,
    Capabilities,
    Logs,
    Diagnostics,
}

impl PluginDetailTab {
    const ALL: [Self; 7] = [
        Self::Overview,
        Self::Config,
        Self::Tools,
        Self::Commands,
        Self::Capabilities,
        Self::Logs,
        Self::Diagnostics,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
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
    Structure,
    Editor,
    FieldInfo,
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
    saved_config: JsonValue,
    draft_config: JsonValue,
    schema: Option<JsonValue>,
    schema_missing: bool,
    diagnostics: Vec<ConfigDiagnostic>,
    runtime_diagnostics: Vec<ConfigDiagnostic>,
    diff: Vec<ConfigDiffRow>,
    nodes: Vec<ConfigNodeSummary>,
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

#[derive(Debug, Clone)]
struct ConfigNodeSummary {
    path: ConfigPath,
    title: String,
    kind: String,
    preview: String,
    depth: usize,
    error_count: usize,
    warning_count: usize,
    dirty: bool,
}

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

type PluginConfigEditOverlay = EditorDialogState<PluginConfigEditAction>;

#[derive(Debug, Clone)]
enum PluginConfigEditAction {
    SetScalar {
        plugin_id: String,
        path: ConfigPath,
        kind: ScalarEditKind,
    },
    AddObjectField {
        plugin_id: String,
        path: ConfigPath,
    },
    SelectType {
        plugin_id: String,
        path: ConfigPath,
        choices: Vec<String>,
    },
    SelectBranch {
        plugin_id: String,
        path: ConfigPath,
        branches: Vec<BranchChoice>,
    },
    SelectEnum {
        plugin_id: String,
        path: ConfigPath,
        variants: Vec<JsonValue>,
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
    index: usize,
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

    fn selected_node(&self) -> Option<&ConfigNodeSummary> {
        self.selected_plugin()
            .and_then(|plugin| plugin.nodes.get(self.selected_node))
    }

    fn clamp_selection(&mut self) {
        if self.visible_plugins.is_empty() {
            self.selected_plugin = 0;
        } else {
            self.selected_plugin = self
                .selected_plugin
                .min(self.visible_plugins.len().saturating_sub(1));
        }
        let node_count = self
            .selected_plugin()
            .map(|plugin| plugin.nodes.len())
            .unwrap_or_default();
        if node_count == 0 {
            self.selected_node = 0;
        } else {
            self.selected_node = self.selected_node.min(node_count.saturating_sub(1));
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
            detail_tab: PluginDetailTab::Overview,
            config_focus: PluginConfigFocus::Structure,
            selected_node: 0,
            config_scroll: 0,
            diagnostics_scroll: 0,
            show_diff: false,
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
        let selected_path = dialog.selected_node().map(|node| node.path.clone());
        match self.build_plugin_workbench(query.as_str()) {
            Ok(mut refreshed) => {
                refreshed.mode = dialog.mode;
                refreshed.transport_filter = dialog.transport_filter;
                refreshed.config_filter = dialog.config_filter;
                refreshed.detail_tab = dialog.detail_tab;
                refreshed.config_focus = dialog.config_focus;
                refreshed.show_diff = dialog.show_diff;
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
                if let Some(path) = selected_path {
                    refreshed.selected_node = refreshed
                        .selected_plugin()
                        .and_then(|plugin| plugin.nodes.iter().position(|node| node.path == path))
                        .unwrap_or_default();
                }
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
        let selected_path = dialog.selected_node().map(|node| node.path.clone());
        let Ok(mut refreshed) = self.build_plugin_workbench(query.as_str()) else {
            return dialog;
        };
        refreshed.mode = dialog.mode;
        refreshed.transport_filter = dialog.transport_filter;
        refreshed.config_filter = dialog.config_filter;
        refreshed.detail_tab = dialog.detail_tab;
        refreshed.config_focus = dialog.config_focus;
        refreshed.show_diff = dialog.show_diff;
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
        if let Some(path) = selected_path {
            refreshed.selected_node = refreshed
                .selected_plugin()
                .and_then(|plugin| plugin.nodes.iter().position(|node| node.path == path))
                .unwrap_or_default();
        }
        refreshed
    }

    pub(super) fn handle_plugin_workbench_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut PluginWorkbenchOverlay,
    ) -> bool {
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
                dialog.detail_tab = PluginDetailTab::Overview;
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
        match key.code {
            KeyCode::Tab if key.modifiers.is_empty() => {
                dialog.config_focus = next_config_focus(dialog.config_focus);
                false
            }
            KeyCode::BackTab => {
                dialog.config_focus = previous_config_focus(dialog.config_focus);
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
            KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.delete_selected_config_node(dialog);
                false
            }
            KeyCode::Char('D') => {
                dialog.show_diff = !dialog.show_diff;
                false
            }
            KeyCode::Char('r') => {
                self.reset_selected_plugin_config_to_saved(dialog);
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
                self.open_selected_config_value_editor(dialog);
                false
            }
            KeyCode::PageUp => {
                move_selected_config_node(dialog, -(CONFIG_EDITOR_PAGE_SIZE as isize));
                false
            }
            KeyCode::PageDown => {
                move_selected_config_node(dialog, CONFIG_EDITOR_PAGE_SIZE as isize);
                false
            }
            KeyCode::Home => {
                dialog.selected_node = 0;
                false
            }
            KeyCode::End => {
                dialog.selected_node = dialog
                    .selected_plugin()
                    .map(|plugin| plugin.nodes.len().saturating_sub(1))
                    .unwrap_or_default();
                false
            }
            KeyCode::Left | KeyCode::Char('h') => {
                dialog.config_focus = previous_config_focus(dialog.config_focus);
                false
            }
            KeyCode::Right | KeyCode::Char('l') => {
                dialog.config_focus = next_config_focus(dialog.config_focus);
                false
            }
            KeyCode::Up | KeyCode::Char('k') => {
                move_selected_config_node(dialog, -1);
                false
            }
            KeyCode::Down | KeyCode::Char('j') => {
                move_selected_config_node(dialog, 1);
                false
            }
            _ => false,
        }
    }

    fn save_selected_plugin_config(&mut self, dialog: &mut PluginWorkbenchOverlay) {
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
        plugin_object.insert("config".to_owned(), plugin.draft_config.clone());
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
            recompute_plugin_config_state(plugin);
            self.flash_success(format!("inserted defaults for {}", plugin.plugin_id));
        }
    }

    fn reset_selected_plugin_config_to_saved(&mut self, dialog: &mut PluginWorkbenchOverlay) {
        let Some(plugin) = dialog.selected_plugin_mut() else {
            return;
        };
        plugin.draft_config = plugin.saved_config.clone();
        plugin.branch_drafts.clear();
        recompute_plugin_config_state(plugin);
        self.flash_success(format!("reset {} config to saved value", plugin.plugin_id));
    }

    fn delete_selected_config_node(&mut self, dialog: &mut PluginWorkbenchOverlay) {
        let Some(path) = dialog.selected_node().map(|node| node.path.clone()) else {
            return;
        };
        if path.is_empty() {
            self.flash_warning("root config cannot be deleted".to_owned());
            return;
        }
        let Some(plugin) = dialog.selected_plugin_mut() else {
            return;
        };
        if remove_value_at_path(&mut plugin.draft_config, &path).is_some() {
            recompute_plugin_config_state(plugin);
            dialog.selected_node = dialog.selected_node.saturating_sub(1);
            dialog.clamp_selection();
        }
    }

    fn open_add_config_value_editor(&mut self, dialog: &mut PluginWorkbenchOverlay) {
        let Some(node) = dialog.selected_node().cloned() else {
            return;
        };
        let Some(plugin) = dialog.selected_plugin() else {
            return;
        };
        let value = get_value_at_path(&plugin.draft_config, &node.path).unwrap_or(&JsonValue::Null);
        if value.is_object() {
            dialog.editor = Some(EditorDialogState::new(
                "Add Field".to_owned(),
                format!(
                    "Enter a field name for {}. The new value starts as a structured null; use `t` to choose another JSON type.",
                    path_display(&node.path)
                ),
                "Enter create  Esc cancel".to_owned(),
                false,
                Editor::default(),
                PluginConfigEditAction::AddObjectField {
                    plugin_id: plugin.plugin_id.clone(),
                    path: node.path,
                },
            ));
        } else if value.is_array() {
            self.append_config_array_item(dialog, node.path);
        } else {
            self.flash_warning("add is available for object and array nodes".to_owned());
        }
    }

    fn append_config_array_item(&mut self, dialog: &mut PluginWorkbenchOverlay, path: ConfigPath) {
        let Some(plugin) = dialog.selected_plugin_mut() else {
            return;
        };
        let item_schema = plugin
            .schema
            .as_ref()
            .and_then(|schema| schema_for_path(schema, schema, &plugin.draft_config, &path))
            .and_then(|schema| array_item_schema(&schema, 0));
        let value = item_schema
            .as_ref()
            .map(|schema| {
                default_value_for_schema(schema, plugin.schema.as_ref().unwrap_or(schema))
            })
            .unwrap_or(JsonValue::Null);
        if let Some(array) =
            get_value_mut_at_path(&mut plugin.draft_config, &path).and_then(JsonValue::as_array_mut)
        {
            array.push(value);
            recompute_plugin_config_state(plugin);
        }
    }

    fn open_config_type_selector(&mut self, dialog: &mut PluginWorkbenchOverlay) {
        let Some(node) = dialog.selected_node().cloned() else {
            return;
        };
        let Some(plugin) = dialog.selected_plugin() else {
            return;
        };
        let schema = plugin.schema.as_ref().and_then(|schema| {
            declared_schema_for_path(schema, schema, &plugin.draft_config, &node.path)
        });
        if let Some(branches) = schema.as_ref().and_then(branch_choices) {
            dialog.editor = Some(EditorDialogState::new(
                "Select Branch".to_owned(),
                format_branch_prompt("Choose branch", &branches),
                "Enter branch number/title  Esc cancel".to_owned(),
                false,
                Editor::default(),
                PluginConfigEditAction::SelectBranch {
                    plugin_id: plugin.plugin_id.clone(),
                    path: node.path,
                    branches,
                },
            ));
            return;
        }
        let choices = schema
            .as_ref()
            .map(schema_type_choices)
            .filter(|choices| !choices.is_empty())
            .unwrap_or_else(|| {
                vec![
                    "string".to_owned(),
                    "number".to_owned(),
                    "integer".to_owned(),
                    "boolean".to_owned(),
                    "object".to_owned(),
                    "array".to_owned(),
                    "null".to_owned(),
                ]
            });
        dialog.editor = Some(EditorDialogState::new(
            "Select Type".to_owned(),
            format!(
                "Choose JSON type for {}: {}",
                path_display(&node.path),
                choices.join(", ")
            ),
            "Enter type  Esc cancel".to_owned(),
            false,
            Editor::default(),
            PluginConfigEditAction::SelectType {
                plugin_id: plugin.plugin_id.clone(),
                path: node.path,
                choices,
            },
        ));
    }

    fn open_selected_config_value_editor(&mut self, dialog: &mut PluginWorkbenchOverlay) {
        let Some(node) = dialog.selected_node().cloned() else {
            return;
        };
        let Some(plugin) = dialog.selected_plugin() else {
            return;
        };
        let value = get_value_at_path(&plugin.draft_config, &node.path).unwrap_or(&JsonValue::Null);
        let schema = plugin.schema.as_ref().and_then(|schema| {
            declared_schema_for_path(schema, schema, &plugin.draft_config, &node.path)
        });
        if let Some(variants) = schema
            .as_ref()
            .and_then(|schema| schema.get("enum"))
            .and_then(JsonValue::as_array)
            .filter(|variants| !variants.is_empty())
        {
            dialog.editor = Some(EditorDialogState::new(
                "Select Value".to_owned(),
                format_enum_prompt(&node.title, variants),
                "Enter value number/text  Esc cancel".to_owned(),
                false,
                Editor::from_text(preview_value(value)),
                PluginConfigEditAction::SelectEnum {
                    plugin_id: plugin.plugin_id.clone(),
                    path: node.path,
                    variants: variants.clone(),
                },
            ));
            return;
        }
        if let Some(branches) = schema.as_ref().and_then(branch_choices) {
            dialog.editor = Some(EditorDialogState::new(
                "Select Branch".to_owned(),
                format_branch_prompt(node.title.as_str(), &branches),
                "Enter branch number/title  Esc cancel".to_owned(),
                false,
                Editor::default(),
                PluginConfigEditAction::SelectBranch {
                    plugin_id: plugin.plugin_id.clone(),
                    path: node.path,
                    branches,
                },
            ));
            return;
        }
        match value {
            JsonValue::Bool(current) => {
                self.set_config_value_at(
                    dialog,
                    plugin.plugin_id.clone(),
                    node.path,
                    json!(!current),
                );
            }
            JsonValue::String(text) => {
                let multiline = schema
                    .as_ref()
                    .is_some_and(|schema| schema_string_is_multiline(schema));
                dialog.editor = Some(EditorDialogState::new(
                    format!("Edit {}", node.title),
                    field_prompt(schema.as_ref(), &node),
                    editor_save_footer(&self.i18n, multiline),
                    multiline,
                    Editor::from_text(text.clone()),
                    PluginConfigEditAction::SetScalar {
                        plugin_id: plugin.plugin_id.clone(),
                        path: node.path,
                        kind: ScalarEditKind::String,
                    },
                ));
            }
            JsonValue::Number(number) => {
                dialog.editor = Some(EditorDialogState::new(
                    format!("Edit {}", node.title),
                    field_prompt(schema.as_ref(), &node),
                    "Enter save  Esc cancel".to_owned(),
                    false,
                    Editor::from_text(number.to_string()),
                    PluginConfigEditAction::SetScalar {
                        plugin_id: plugin.plugin_id.clone(),
                        path: node.path,
                        kind: if number.as_i64().is_some() || number.as_u64().is_some() {
                            ScalarEditKind::Integer
                        } else {
                            ScalarEditKind::Number
                        },
                    },
                ));
            }
            JsonValue::Null => self.open_config_type_selector(dialog),
            JsonValue::Object(_) | JsonValue::Array(_) => {
                self.flash_info(
                    "selected structured value is edited through its child rows".to_owned(),
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
    ) {
        let Some(plugin) = dialog
            .plugins
            .iter_mut()
            .find(|plugin| plugin.plugin_id == plugin_id)
        else {
            return;
        };
        set_value_at_path(&mut plugin.draft_config, &path, value);
        recompute_plugin_config_state(plugin);
        dialog.selected_node = plugin
            .nodes
            .iter()
            .position(|node| node.path == path)
            .unwrap_or(dialog.selected_node);
        dialog.clamp_selection();
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
                self.set_config_value_at(dialog, plugin_id, path, value);
            }
            PluginConfigEditAction::AddObjectField { plugin_id, path } => {
                let key = input.trim();
                if key.is_empty() {
                    return Err("field name cannot be empty".to_owned());
                }
                let Some(plugin) = dialog
                    .plugins
                    .iter_mut()
                    .find(|plugin| plugin.plugin_id == plugin_id)
                else {
                    return Ok(());
                };
                let mut child_path = path.clone();
                child_path.push(PathSegment::Key(key.to_owned()));
                let child_schema = plugin.schema.as_ref().and_then(|root| {
                    schema_for_path(root, root, &plugin.draft_config, &path)
                        .and_then(|schema| object_property_schema(&schema, key))
                });
                let default = child_schema
                    .as_ref()
                    .map(|schema| {
                        default_value_for_schema(schema, plugin.schema.as_ref().unwrap_or(schema))
                    })
                    .unwrap_or(JsonValue::Null);
                set_value_at_path(&mut plugin.draft_config, &child_path, default);
                recompute_plugin_config_state(plugin);
                dialog.selected_node = plugin
                    .nodes
                    .iter()
                    .position(|node| node.path == child_path)
                    .unwrap_or(dialog.selected_node);
            }
            PluginConfigEditAction::SelectType {
                plugin_id,
                path,
                choices,
            } => {
                let selected = select_named_choice(input, choices.as_slice())?;
                let Some(plugin) = dialog
                    .plugins
                    .iter_mut()
                    .find(|plugin| plugin.plugin_id == plugin_id)
                else {
                    return Ok(());
                };
                let schema = plugin
                    .schema
                    .as_ref()
                    .and_then(|root| schema_for_path(root, root, &plugin.draft_config, &path));
                let value = default_value_for_type(selected.as_str(), schema.as_ref());
                set_value_at_path(&mut plugin.draft_config, &path, value);
                recompute_plugin_config_state(plugin);
            }
            PluginConfigEditAction::SelectBranch {
                plugin_id,
                path,
                branches,
            } => {
                let branch = select_branch_choice(input, branches.as_slice())?;
                let Some(plugin) = dialog
                    .plugins
                    .iter_mut()
                    .find(|plugin| plugin.plugin_id == plugin_id)
                else {
                    return Ok(());
                };
                if let Some(current) = get_value_at_path(&plugin.draft_config, &path).cloned() {
                    let active_key = plugin_branch_draft_key(
                        plugin.plugin_id.as_str(),
                        &path,
                        active_branch_label(branches.as_slice(), &current),
                    );
                    plugin.branch_drafts.insert(active_key, current);
                }
                let target_key = plugin_branch_draft_key(
                    plugin.plugin_id.as_str(),
                    &path,
                    branch.label.as_str(),
                );
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
                set_value_at_path(&mut plugin.draft_config, &path, value);
                recompute_plugin_config_state(plugin);
            }
            PluginConfigEditAction::SelectEnum {
                plugin_id,
                path,
                variants,
            } => {
                let selected = select_enum_variant(input, variants.as_slice())?;
                self.set_config_value_at(dialog, plugin_id, path, selected);
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
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(4),
                Constraint::Length(3),
                Constraint::Min(20),
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
        render_plugin_config_page(frame, rows[2], dialog);
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
        PluginDetailTab::Overview => plugin_overview_text(plugin),
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
        "Esc plugins  Left/Right tabs  Tab next  Shift+Tab previous  PageUp/PageDown scroll  r refresh",
    );
}

fn render_plugin_config_page(frame: &mut Frame, area: Rect, dialog: &PluginWorkbenchOverlay) {
    let Some(plugin) = dialog.selected_plugin() else {
        render_plugin_panel(
            frame,
            area,
            "Config",
            Text::from("No plugin selected."),
            None,
        );
        return;
    };
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),
            Constraint::Min(12),
            Constraint::Length(8),
            Constraint::Length(1),
        ])
        .split(area);
    render_plugin_panel(
        frame,
        rows[0],
        "Config",
        config_toolbar_text(plugin, dialog),
        None,
    );

    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(30),
            Constraint::Min(46),
            Constraint::Length(34),
        ])
        .split(rows[1]);
    render_plugin_panel(
        frame,
        columns[0],
        focus_title(
            "Structure",
            dialog.config_focus == PluginConfigFocus::Structure,
        ),
        config_structure_text(dialog, plugin),
        None,
    );
    render_plugin_panel(
        frame,
        columns[1],
        focus_title("Editor", dialog.config_focus == PluginConfigFocus::Editor),
        config_editor_text(dialog, plugin),
        None,
    );
    render_plugin_panel(
        frame,
        columns[2],
        focus_title(
            "Field Info",
            dialog.config_focus == PluginConfigFocus::FieldInfo,
        ),
        field_info_text(dialog, plugin),
        None,
    );

    let bottom_title = if dialog.show_diff {
        "Config Diff"
    } else {
        "Diagnostics"
    };
    let bottom_text = if dialog.show_diff {
        config_diff_text(plugin)
    } else {
        config_diagnostics_text(plugin)
    };
    render_plugin_panel(
        frame,
        rows[2],
        focus_title(
            bottom_title,
            dialog.config_focus == PluginConfigFocus::Diagnostics,
        ),
        bottom_text,
        None,
    );
    render_plugin_footer(
        frame,
        rows[3],
        "Esc plugins  Left/Right focus  Ctrl+Left/Right tabs  Enter/e edit  a add  t type/branch  s save  v validate  i defaults  D diff",
    );
}

fn render_plugin_workbench_editor_overlay(
    frame: &mut Frame,
    area: Rect,
    _workbench_area: Rect,
    dialog: &PluginWorkbenchOverlay,
) {
    let Some(editor) = dialog.editor.as_ref() else {
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

fn focus_title(title: &str, focused: bool) -> String {
    if focused {
        format!("> {title}")
    } else {
        title.to_owned()
    }
}

fn transport_display(transport: &str) -> &str {
    match transport {
        "static" => "native",
        other => other,
    }
}

fn config_structure_text(
    dialog: &PluginWorkbenchOverlay,
    plugin: &PluginWorkbenchPlugin,
) -> Text<'static> {
    if plugin.nodes.is_empty() {
        return Text::from("No structure nodes.");
    }
    let mut lines = Vec::new();
    for (index, node) in plugin.nodes.iter().enumerate() {
        let selected = index == dialog.selected_node;
        let marker = if selected { "> " } else { "  " };
        let connector = if node.depth == 0 {
            ""
        } else if node.depth == 1 {
            "|- "
        } else {
            "`- "
        };
        let mut suffix = if node.error_count > 0 {
            format!("error {}", node.error_count)
        } else if node.warning_count > 0 {
            format!("warning {}", node.warning_count)
        } else if node.dirty {
            "dirty".to_owned()
        } else if matches!(node.kind.as_str(), "object" | "array") {
            node.preview.clone()
        } else {
            node.kind.clone()
        };
        if suffix.is_empty() {
            suffix = node.kind.clone();
        }
        let title = format!(
            "{}{}{}{}",
            marker,
            "  ".repeat(node.depth),
            connector,
            node.title
        );
        let line = fixed_columns(&[(title.as_str(), 24), (suffix.as_str(), 18)], 44);
        let style = if selected {
            plugin_workbench_selection_highlight_style()
        } else {
            Style::default()
        };
        lines.push(Line::from(Span::styled(clean(line), style)));
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
    let saved_config = materialized_config_value(schema.as_ref(), &raw_config);
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
        saved_config: saved_config.clone(),
        draft_config: saved_config,
        schema,
        schema_missing,
        diagnostics: Vec::new(),
        runtime_diagnostics: Vec::new(),
        diff: Vec::new(),
        nodes: Vec::new(),
        logs,
        dirty: false,
        branch_drafts: BTreeMap::new(),
    };
    recompute_plugin_config_state(&mut plugin);
    plugin
}

fn recompute_plugin_config_state(plugin: &mut PluginWorkbenchPlugin) {
    plugin.dirty = plugin.draft_config != plugin.saved_config;
    plugin.diagnostics = validate_config_value(
        plugin.schema.as_ref(),
        &plugin.draft_config,
        plugin.schema_missing,
    );
    plugin.runtime_diagnostics = runtime_diagnostics(&plugin.status);
    plugin.diff = diff_config_values(&plugin.saved_config, &plugin.draft_config);
    plugin.nodes = build_config_nodes(
        plugin.schema.as_ref(),
        &plugin.draft_config,
        &plugin.saved_config,
        plugin.diagnostics.as_slice(),
    );
    plugin.config_status = config_status_for_plugin(plugin);
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
    let errors = plugin
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error)
        .count();
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

fn next_config_focus(focus: PluginConfigFocus) -> PluginConfigFocus {
    match focus {
        PluginConfigFocus::Structure => PluginConfigFocus::Editor,
        PluginConfigFocus::Editor => PluginConfigFocus::FieldInfo,
        PluginConfigFocus::FieldInfo => PluginConfigFocus::Diagnostics,
        PluginConfigFocus::Diagnostics => PluginConfigFocus::Structure,
    }
}

fn previous_config_focus(focus: PluginConfigFocus) -> PluginConfigFocus {
    match focus {
        PluginConfigFocus::Structure => PluginConfigFocus::Diagnostics,
        PluginConfigFocus::Editor => PluginConfigFocus::Structure,
        PluginConfigFocus::FieldInfo => PluginConfigFocus::Editor,
        PluginConfigFocus::Diagnostics => PluginConfigFocus::FieldInfo,
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

fn materialized_value_for_schema(schema: &JsonValue, root: &JsonValue) -> JsonValue {
    let schema = resolve_schema(root, schema);
    if let Some(default) = schema.get("default") {
        return default.clone();
    }
    if schema_type_choices(schema)
        .iter()
        .any(|kind| kind == "null")
    {
        return JsonValue::Null;
    }
    if let Some(variants) = schema.get("enum").and_then(JsonValue::as_array)
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
            if let Some(properties) = schema.get("properties").and_then(JsonValue::as_object) {
                for (key, child_schema) in properties {
                    object.insert(
                        key.clone(),
                        materialized_value_for_schema(child_schema, root),
                    );
                }
            }
            JsonValue::Object(object)
        }
        Some("array") => JsonValue::Array(Vec::new()),
        Some("string") => JsonValue::String(String::new()),
        Some("integer") => JsonValue::Number(JsonNumber::from(0)),
        Some("number") => JsonValue::Number(JsonNumber::from(0)),
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
            if let Some(properties) = schema.get("properties").and_then(JsonValue::as_object) {
                for (key, child_schema) in properties {
                    let child = object
                        .entry(key.clone())
                        .or_insert_with(|| materialized_value_for_schema(child_schema, root));
                    materialize_schema_fields(child, child_schema, root);
                }
            }
        }
        Some("array") => {
            let JsonValue::Array(items) = value else {
                return;
            };
            for (index, item) in items.iter_mut().enumerate() {
                if let Some(item_schema) = array_item_schema(&schema, index) {
                    materialize_schema_fields(item, &item_schema, root);
                }
            }
        }
        _ => {}
    }
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
        for item in all_of {
            validate_schema_at(diagnostics, root, item, value, path, title);
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
                &title_for_property(schema, field),
                "required field is missing",
            );
        }
    }
    let properties = schema_object
        .get("properties")
        .and_then(JsonValue::as_object);
    for (key, child_value) in value {
        let mut child_path = path.clone();
        child_path.push(PathSegment::Key(key.clone()));
        if let Some(child_schema) = object_property_schema(schema, key) {
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
                    DiagnosticSeverity::Warning,
                    path,
                    &title_for_schema_or_key(schema, "Array"),
                    "array contains duplicate items",
                );
                break;
            }
        }
    }
    for (index, item) in value.iter().enumerate() {
        if let Some(item_schema) = array_item_schema(schema, index) {
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

fn format_is_valid(format: &str, text: &str) -> bool {
    match format {
        "uri" | "url" => url::Url::parse(text).is_ok(),
        "email" => text.contains('@') && text.split('@').all(|part| !part.is_empty()),
        "hostname" => !text.trim().is_empty() && !text.contains('/'),
        "ipv4" => text.parse::<std::net::Ipv4Addr>().is_ok(),
        "ipv6" => text.parse::<std::net::Ipv6Addr>().is_ok(),
        "uuid" => uuid::Uuid::parse_str(text).is_ok(),
        _ => true,
    }
}

fn build_config_nodes(
    schema: Option<&JsonValue>,
    value: &JsonValue,
    saved: &JsonValue,
    diagnostics: &[ConfigDiagnostic],
) -> Vec<ConfigNodeSummary> {
    let mut nodes = Vec::new();
    collect_config_nodes(
        &mut nodes,
        schema,
        schema,
        value,
        saved,
        diagnostics,
        &Vec::new(),
        0,
        "Config".to_owned(),
    );
    nodes
}

#[allow(clippy::too_many_arguments)]
fn collect_config_nodes(
    nodes: &mut Vec<ConfigNodeSummary>,
    root_schema: Option<&JsonValue>,
    schema: Option<&JsonValue>,
    value: &JsonValue,
    saved: &JsonValue,
    diagnostics: &[ConfigDiagnostic],
    path: &ConfigPath,
    depth: usize,
    title: String,
) {
    let node_diags = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.path == *path)
        .collect::<Vec<_>>();
    nodes.push(ConfigNodeSummary {
        path: path.clone(),
        title,
        kind: schema
            .map(schema_kind_label)
            .unwrap_or_else(|| json_kind_label(value).to_owned()),
        preview: preview_value(value),
        depth,
        error_count: node_diags
            .iter()
            .filter(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error)
            .count(),
        warning_count: node_diags
            .iter()
            .filter(|diagnostic| diagnostic.severity == DiagnosticSeverity::Warning)
            .count(),
        dirty: saved != value,
    });

    match value {
        JsonValue::Object(object) => {
            let mut keys = object.keys().cloned().collect::<BTreeSet<_>>();
            if let Some(schema) = schema
                && let Some(required) = resolve_schema(root_schema.unwrap_or(schema), schema)
                    .get("required")
                    .and_then(JsonValue::as_array)
            {
                for field in required.iter().filter_map(JsonValue::as_str) {
                    keys.insert(field.to_owned());
                }
            }
            for key in keys {
                let mut child_path = path.clone();
                child_path.push(PathSegment::Key(key.clone()));
                let child_value = object.get(key.as_str()).unwrap_or(&JsonValue::Null);
                let child_saved = saved
                    .as_object()
                    .and_then(|object| object.get(key.as_str()))
                    .unwrap_or(&JsonValue::Null);
                let child_schema = match (root_schema, schema) {
                    (Some(root), Some(schema)) => object_property_schema(schema, key.as_str())
                        .or_else(|| {
                            schema_for_path(root, root, value, path)
                                .and_then(|schema| object_property_schema(&schema, key.as_str()))
                        }),
                    _ => None,
                };
                let title = child_schema
                    .as_ref()
                    .map(|schema| title_for_schema_or_key(schema, key.as_str()))
                    .unwrap_or_else(|| title_from_key(key.as_str()));
                collect_config_nodes(
                    nodes,
                    root_schema,
                    child_schema.as_ref(),
                    child_value,
                    child_saved,
                    diagnostics,
                    &child_path,
                    depth + 1,
                    title,
                );
            }
        }
        JsonValue::Array(items) => {
            for (index, child_value) in items.iter().enumerate() {
                let mut child_path = path.clone();
                child_path.push(PathSegment::Index(index));
                let child_saved = saved
                    .as_array()
                    .and_then(|items| items.get(index))
                    .unwrap_or(&JsonValue::Null);
                let child_schema = match (root_schema, schema) {
                    (Some(_), Some(schema)) => array_item_schema(schema, index),
                    _ => None,
                };
                let title = child_schema
                    .as_ref()
                    .map(|schema| title_for_schema_or_key(schema, format!("Item {index}").as_str()))
                    .unwrap_or_else(|| format!("Item {index}"));
                collect_config_nodes(
                    nodes,
                    root_schema,
                    child_schema.as_ref(),
                    child_value,
                    child_saved,
                    diagnostics,
                    &child_path,
                    depth + 1,
                    title,
                );
            }
        }
        _ => {}
    }
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

fn insert_schema_defaults(value: &mut JsonValue, schema: &JsonValue, root: &JsonValue) {
    let schema = resolve_schema(root, schema);
    if value.is_null()
        && let Some(default) = schema.get("default")
    {
        *value = default.clone();
    }
    let kind = effective_schema_kind(schema);
    if kind.as_deref() == Some("object") {
        if !value.is_object() {
            *value = JsonValue::Object(JsonMap::new());
        }
        let Some(object) = value.as_object_mut() else {
            return;
        };
        if let Some(properties) = schema.get("properties").and_then(JsonValue::as_object) {
            for (key, child_schema) in properties {
                if let Some(default) = child_schema.get("default")
                    && !object.contains_key(key)
                {
                    object.insert(key.clone(), default.clone());
                }
                if let Some(child) = object.get_mut(key) {
                    insert_schema_defaults(child, child_schema, root);
                }
            }
        }
    } else if kind.as_deref() == Some("array")
        && let Some(array) = value.as_array_mut()
        && let Some(items_schema) = schema.get("items")
    {
        for item in array {
            insert_schema_defaults(item, items_schema, root);
        }
    }
}

fn default_value_for_schema(schema: &JsonValue, root: &JsonValue) -> JsonValue {
    let schema = resolve_schema(root, schema);
    if let Some(default) = schema.get("default") {
        return default.clone();
    }
    if let Some(variants) = schema.get("enum").and_then(JsonValue::as_array)
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
        return default_value_for_schema(first, root);
    }
    if let Some(all_of) = schema.get("allOf").and_then(JsonValue::as_array) {
        let mut object = JsonValue::Object(JsonMap::new());
        for branch in all_of {
            let branch_default = default_value_for_schema(branch, root);
            merge_default_value(&mut object, branch_default);
        }
        return object;
    }
    if let Some(kind) = effective_schema_kind(schema) {
        return default_value_for_type(kind.as_str(), Some(schema));
    }
    JsonValue::Null
}

fn merge_default_value(target: &mut JsonValue, patch: JsonValue) {
    match (target, patch) {
        (JsonValue::Object(target), JsonValue::Object(patch)) => {
            for (key, value) in patch {
                target.entry(key).or_insert(value);
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
                current_schema = object_property_schema(&current_schema, key)?;
                current_value = current_value.get(key).unwrap_or(&JsonValue::Null);
            }
            PathSegment::Index(index) => {
                current_schema = array_item_schema(&current_schema, *index)?;
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
                current_schema = object_property_schema(&parent_schema, key)?;
                current_value = current_value.get(key).unwrap_or(&JsonValue::Null);
            }
            PathSegment::Index(index) => {
                current_schema = array_item_schema(&parent_schema, *index)?;
                current_value = current_value.get(*index).unwrap_or(&JsonValue::Null);
            }
        }
    }
    Some(resolve_schema(root, &current_schema).clone())
}

fn active_schema_for_value(root: &JsonValue, schema: &JsonValue, value: &JsonValue) -> JsonValue {
    let schema = resolve_schema(root, schema);
    if let Some(all_of) = schema.get("allOf").and_then(JsonValue::as_array) {
        return merge_all_of(all_of);
    }
    for key in ["oneOf", "anyOf"] {
        if let Some(branches) = schema.get(key).and_then(JsonValue::as_array) {
            if let Some(branch) = branches
                .iter()
                .find(|branch| schema_matches(root, branch, value))
            {
                return branch.clone();
            }
            if let Some(first) = branches.first() {
                return first.clone();
            }
        }
    }
    schema.clone()
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

fn merge_all_of(items: &[JsonValue]) -> JsonValue {
    let mut merged = JsonValue::Object(JsonMap::new());
    for item in items {
        merge_schema_overlay(&mut merged, item);
    }
    merged
}

fn object_property_schema(schema: &JsonValue, key: &str) -> Option<JsonValue> {
    let schema = resolve_schema(schema, schema);
    if let Some(properties) = schema.get("properties").and_then(JsonValue::as_object)
        && let Some(child) = properties.get(key)
    {
        return Some(child.clone());
    }
    if let Some(patterns) = schema
        .get("patternProperties")
        .and_then(JsonValue::as_object)
    {
        for (pattern, child) in patterns {
            if pattern_key_matches(pattern, key) {
                return Some(child.clone());
            }
        }
    }
    match schema.get("additionalProperties") {
        Some(JsonValue::Object(object)) => Some(JsonValue::Object(object.clone())),
        _ => None,
    }
}

fn array_item_schema(schema: &JsonValue, index: usize) -> Option<JsonValue> {
    let schema = resolve_schema(schema, schema);
    if let Some(prefix) = schema.get("prefixItems").and_then(JsonValue::as_array)
        && let Some(item) = prefix.get(index)
    {
        return Some(item.clone());
    }
    schema.get("items").cloned()
}

fn branch_choices(schema: &JsonValue) -> Option<Vec<BranchChoice>> {
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
            index,
            label: branch_label(index, schema),
            schema: schema.clone(),
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

fn active_branch_label<'a>(branches: &'a [BranchChoice], value: &JsonValue) -> &'a str {
    branches
        .iter()
        .find(|branch| schema_matches(&branch.schema, &branch.schema, value))
        .or_else(|| branches.first())
        .map(|branch| branch.label.as_str())
        .unwrap_or("branch")
}

fn plugin_branch_draft_key(plugin_id: &str, path: &ConfigPath, branch: &str) -> String {
    format!("{plugin_id}:{}:{branch}", path_display(path))
}

fn schema_type_choices(schema: &JsonValue) -> Vec<String> {
    match schema.get("type") {
        Some(JsonValue::String(kind)) => vec![kind.clone()],
        Some(JsonValue::Array(items)) => items
            .iter()
            .filter_map(JsonValue::as_str)
            .map(str::to_owned)
            .collect(),
        _ => Vec::new(),
    }
}

fn effective_schema_kind(schema: &JsonValue) -> Option<String> {
    match schema.get("type") {
        Some(JsonValue::String(kind)) => Some(kind.clone()),
        Some(JsonValue::Array(items)) => items
            .iter()
            .filter_map(JsonValue::as_str)
            .find(|kind| *kind != "null")
            .map(str::to_owned),
        _ if schema.get("properties").is_some() => Some("object".to_owned()),
        _ if schema.get("items").is_some() || schema.get("prefixItems").is_some() => {
            Some("array".to_owned())
        }
        _ => None,
    }
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

fn title_for_property(schema: &JsonValue, key: &str) -> String {
    object_property_schema(schema, key)
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

fn select_named_choice(input: &str, choices: &[String]) -> UiResult<String> {
    let trimmed = input.trim();
    if let Ok(index) = trimmed.parse::<usize>()
        && let Some(choice) = choices.get(index.saturating_sub(1))
    {
        return Ok(choice.clone());
    }
    choices
        .iter()
        .find(|choice| choice.eq_ignore_ascii_case(trimmed))
        .cloned()
        .ok_or_else(|| format!("unknown choice `{trimmed}`"))
}

fn select_branch_choice(input: &str, branches: &[BranchChoice]) -> UiResult<BranchChoice> {
    let trimmed = input.trim();
    if let Ok(index) = trimmed.parse::<usize>()
        && let Some(branch) = branches.get(index.saturating_sub(1))
    {
        return Ok(branch.clone());
    }
    branches
        .iter()
        .find(|branch| branch.label.eq_ignore_ascii_case(trimmed))
        .cloned()
        .ok_or_else(|| format!("unknown branch `{trimmed}`"))
}

fn select_enum_variant(input: &str, variants: &[JsonValue]) -> UiResult<JsonValue> {
    let trimmed = input.trim();
    if let Ok(index) = trimmed.parse::<usize>()
        && let Some(variant) = variants.get(index.saturating_sub(1))
    {
        return Ok(variant.clone());
    }
    variants
        .iter()
        .find(|variant| preview_value(variant).eq_ignore_ascii_case(trimmed))
        .cloned()
        .ok_or_else(|| format!("unknown enum value `{trimmed}`"))
}

fn format_enum_prompt(title: &str, variants: &[JsonValue]) -> String {
    let mut lines = vec![format!("Choose {title}:")];
    for (index, variant) in variants.iter().enumerate() {
        lines.push(format!("{}. {}", index + 1, preview_value(variant)));
    }
    lines.join("\n")
}

fn format_branch_prompt(title: &str, branches: &[BranchChoice]) -> String {
    let mut lines = vec![format!("{title}:")];
    for branch in branches {
        lines.push(format!("{}. {}", branch.index + 1, branch.label));
    }
    lines.join("\n")
}

fn field_prompt(schema: Option<&JsonValue>, node: &ConfigNodeSummary) -> String {
    let mut parts = vec![format!("Path: {}", path_display(&node.path))];
    if let Some(schema) = schema {
        if let Some(description) = schema.get("description").and_then(JsonValue::as_str) {
            parts.push(description.to_owned());
        }
        if let Some(format) = schema.get("format").and_then(JsonValue::as_str) {
            parts.push(format!("format: {format}"));
        }
        parts.extend(schema_constraints(schema));
    }
    parts.join("\n")
}

fn schema_string_is_multiline(schema: &JsonValue) -> bool {
    schema
        .get("format")
        .and_then(JsonValue::as_str)
        .is_some_and(|format| matches!(format, "markdown" | "multiline" | "textarea"))
        || schema
            .get("maxLength")
            .and_then(JsonValue::as_u64)
            .is_some_and(|max| max > 240)
}

fn pattern_key_matches(pattern: &str, key: &str) -> bool {
    if let Some(prefix) = pattern.strip_prefix('^') {
        let prefix = prefix.trim_end_matches(".*").trim_end_matches('$');
        return key.starts_with(prefix);
    }
    if let Some(suffix) = pattern.strip_suffix('$') {
        return key.ends_with(suffix);
    }
    key.contains(pattern.trim_matches('*'))
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

fn plugin_overview_text(plugin: &PluginWorkbenchPlugin) -> Text<'static> {
    let manifest = plugin
        .inspect
        .as_ref()
        .and_then(|inspect| inspect.manifest.as_ref());
    let mut lines = plugin_header_text(plugin).lines;
    if let Some(manifest) = manifest {
        if let Some(description) = manifest.description.as_deref() {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "Description",
                Style::default().add_modifier(Modifier::BOLD),
            )));
            lines.push(Line::from(clean(description)));
        }
        if !manifest.authors.is_empty() {
            lines.push(Line::from(format!(
                "Authors: {}",
                clean(manifest.authors.join(", "))
            )));
        }
        if !manifest.transports.is_empty() {
            let transports = manifest
                .transports
                .iter()
                .map(|transport| format!("{transport:?}").to_ascii_lowercase())
                .collect::<Vec<_>>();
            lines.push(Line::from(format!(
                "Manifest transports: {}",
                transports.join(", ")
            )));
        }
    }
    Text::from(lines)
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
    diagnostics_text(diagnostics.as_slice())
}

fn config_editor_text(
    dialog: &PluginWorkbenchOverlay,
    plugin: &PluginWorkbenchPlugin,
) -> Text<'static> {
    let selected = dialog.selected_node().or_else(|| plugin.nodes.first());
    let mut lines = Vec::new();
    if let Some(node) = selected {
        let value = get_value_at_path(&plugin.draft_config, &node.path).unwrap_or(&JsonValue::Null);
        let schema = plugin.schema.as_ref().and_then(|schema| {
            declared_schema_for_path(schema, schema, &plugin.draft_config, &node.path)
        });
        append_schema_editor_lines(
            &mut lines,
            plugin.schema.as_ref(),
            schema.as_ref(),
            value,
            node.title.as_str(),
            0,
            96,
            28,
        );
    } else {
        lines.push(Line::from("No config nodes."));
    }
    Text::from(lines)
}

fn field_info_text(
    dialog: &PluginWorkbenchOverlay,
    plugin: &PluginWorkbenchPlugin,
) -> Text<'static> {
    let mut lines = Vec::new();
    if let Some(node) = dialog.selected_node() {
        let schema = plugin
            .schema
            .as_ref()
            .and_then(|schema| schema_for_path(schema, schema, &plugin.draft_config, &node.path));
        lines.push(Line::from(Span::styled(
            node.title.clone(),
            Style::default().add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(""));
        if let Some(schema) = schema.as_ref()
            && let Some(description) = schema.get("description").and_then(JsonValue::as_str)
        {
            lines.push(Line::from(clean(description)));
            lines.push(Line::from(""));
        }
        lines.push(Line::from("Path"));
        lines.push(Line::from(path_display(&node.path)));
        lines.push(Line::from(""));
        lines.push(Line::from("Type"));
        lines.push(Line::from(node.kind.clone()));
        if let Some(schema) = schema.as_ref() {
            if let Some(default) = schema.get("default") {
                lines.push(Line::from(""));
                lines.push(Line::from("Default"));
                lines.push(Line::from(preview_value(default)));
            }
            let constraints = schema_constraints(schema);
            if !constraints.is_empty() {
                lines.push(Line::from(""));
                lines.push(Line::from("Constraints"));
                lines.extend(constraints.into_iter().map(Line::from));
            }
        }
        let diagnostics = plugin
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.path == node.path)
            .collect::<Vec<_>>();
        lines.push(Line::from(""));
        lines.push(Line::from("Errors"));
        if diagnostics.is_empty() {
            lines.push(Line::from("none"));
        } else {
            for diagnostic in diagnostics {
                lines.push(Line::from(format!(
                    "{}: {}",
                    diagnostic_severity_label(diagnostic.severity),
                    diagnostic.message
                )));
            }
        }
    }
    Text::from(lines)
}

fn config_diagnostics_text(plugin: &PluginWorkbenchPlugin) -> Text<'static> {
    let mut diagnostics = plugin.diagnostics.clone();
    diagnostics.extend(plugin.runtime_diagnostics.clone());
    diagnostics_text(diagnostics.as_slice())
}

fn diagnostics_text(diagnostics: &[ConfigDiagnostic]) -> Text<'static> {
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
        for diagnostic in diagnostics {
            lines.push(Line::from(fixed_columns(
                &[
                    (diagnostic_severity_label(diagnostic.severity), 10),
                    (diagnostic.source.as_str(), 10),
                    (diagnostic.field.as_str(), 22),
                    (diagnostic.message.as_str(), 80),
                ],
                table_width,
            )));
        }
    }
    Text::from(lines)
}

fn config_diff_text(plugin: &PluginWorkbenchPlugin) -> Text<'static> {
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
        for row in &plugin.diff {
            lines.push(Line::from(fixed_columns(
                &[
                    (path_display(&row.path).as_str(), 28),
                    (row.before.as_str(), 28),
                    (row.after.as_str(), 28),
                    (row.summary.as_str(), 28),
                ],
                table_width,
            )));
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
        if let Some(constant) = schema.get("const") {
            lines.push(Line::from(format!(
                "{}{}        [ {} ] readonly",
                indent,
                title,
                preview_value(constant)
            )));
            return;
        }
        if let Some(variants) = schema.get("enum").and_then(JsonValue::as_array)
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
    let Some(branches) = branch_choices(schema) else {
        return;
    };
    let active = active_branch_label(branches.as_slice(), value);
    let also_matches = branches
        .iter()
        .filter(|branch| {
            branch.label != active && schema_matches(root_schema, &branch.schema, value)
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
    } else if schema_is_map_like(schema.expect("checked")) {
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
        let child_schema = schema.and_then(|schema| object_property_schema(schema, key.as_str()));
        let kind = child_schema
            .as_ref()
            .map(schema_kind_label)
            .unwrap_or_else(|| json_kind_label(child).to_owned());
        let state = object_field_state(schema, key.as_str(), object.contains_key(key.as_str()));
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
    let tuple = schema
        .and_then(|schema| schema.get("prefixItems"))
        .and_then(JsonValue::as_array)
        .is_some();
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
            let item_schema = schema.and_then(|schema| array_item_schema(schema, index));
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
    let item_schema = schema.and_then(|schema| array_item_schema(schema, 0));
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
        .and_then(|schema| schema.get("format"))
        .and_then(JsonValue::as_str)
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
    if let Some(examples) = schema
        .and_then(|schema| schema.get("examples"))
        .and_then(JsonValue::as_array)
        .filter(|examples| !examples.is_empty())
    {
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
    let schema = resolve_schema(schema, schema);
    schema
        .get("properties")
        .and_then(JsonValue::as_object)
        .map(|object| object.len())
        .or_else(|| {
            schema
                .get("prefixItems")
                .and_then(JsonValue::as_array)
                .map(Vec::len)
        })
        .unwrap_or_else(|| {
            if schema.get("items").is_some() || schema.get("additionalProperties").is_some() {
                1
            } else {
                0
            }
        })
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

fn schema_is_map_like(schema: &JsonValue) -> bool {
    schema.get("additionalProperties").is_some()
        || schema.get("patternProperties").is_some()
        || schema.get("propertyNames").is_some()
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
        if let Some(properties) = schema.get("properties").and_then(JsonValue::as_object) {
            for key in properties.keys() {
                if seen.insert(key.clone()) {
                    keys.push(key.clone());
                }
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
    schema
        .get("required")
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
        .filter_map(JsonValue::as_str)
        .map(str::to_owned)
        .collect()
}

fn object_field_state(schema: Option<&JsonValue>, key: &str, present: bool) -> String {
    let Some(schema) = schema else {
        return "custom".to_owned();
    };
    if schema_required_fields(schema).contains(key) {
        if present {
            "required".to_owned()
        } else {
            "missing".to_owned()
        }
    } else if schema
        .get("properties")
        .and_then(JsonValue::as_object)
        .is_some_and(|properties| properties.contains_key(key))
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
    if let Some(schema) = schema
        && let Some(properties) = schema.get("properties").and_then(JsonValue::as_object)
    {
        for key in properties.keys().take(4) {
            if seen.insert(key.clone()) {
                keys.push(key.clone());
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
    let mut parts = Vec::new();
    for key in [
        "minimum",
        "maximum",
        "exclusiveMinimum",
        "exclusiveMaximum",
        "multipleOf",
    ] {
        if let Some(value) = schema.get(key) {
            parts.push(format!("{key}: {}", preview_value(value)));
        }
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!("     {}", parts.join("   "))
    }
}

fn schema_constraints(schema: &JsonValue) -> Vec<String> {
    let mut constraints = Vec::new();
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
            constraints.push(format!("{key}: {}", preview_value(value)));
        }
    }
    constraints
}

fn config_toolbar_text(
    plugin: &PluginWorkbenchPlugin,
    dialog: &PluginWorkbenchOverlay,
) -> Text<'static> {
    let save_state = if plugin.dirty { "Unsaved" } else { "Saved" };
    let bottom = if dialog.show_diff {
        "Diff"
    } else {
        "Diagnostics"
    };
    Text::from(vec![
        Line::from(format!(
            "Config: {}        {}",
            clean(plugin.config_status.label.as_str()),
            save_state
        )),
        Line::from(format!(
            "[ Validate ] [ Insert Defaults ] [ Reset ] [ {bottom} ] [ Save ] [ Restart/Reload ]"
        )),
    ])
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
        .selected_plugin()
        .map(|plugin| plugin.nodes.len())
        .unwrap_or_default();
    move_index(&mut dialog.selected_node, item_count, delta);
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

    #[test]
    fn plugin_without_effective_config_does_not_synthesize_static_plugin_config() {
        let sources = crate::backend::ConfigJsonSources {
            config_path: std::path::PathBuf::from("config.json"),
            config_found: false,
            file: json!({}),
            effective: json!({}),
        };
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
        assert_eq!(plugin.config_status.kind, PluginConfigStatusKind::Valid);
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
        let nodes = build_config_nodes(Some(&schema), &value, &value, &[]);
        assert!(
            nodes
                .iter()
                .any(|node| node.path == vec![PathSegment::Key("endpoint".to_owned())])
        );
        assert!(nodes.iter().any(|node| node.path
            == vec![
                PathSegment::Key("limits".to_owned()),
                PathSegment::Key("timeoutMs".to_owned())
            ]));
        assert!(nodes.iter().all(|node| !node.dirty));
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
        assert_eq!(plugin.config_status.kind, PluginConfigStatusKind::Valid);
        assert!(
            plugin
                .nodes
                .iter()
                .any(|node| node.path == vec![PathSegment::Key("enabled".to_owned())])
        );
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
            next_config_focus(PluginConfigFocus::Structure),
            PluginConfigFocus::Editor
        );
        assert_eq!(
            next_config_focus(PluginConfigFocus::Editor),
            PluginConfigFocus::FieldInfo
        );
        assert_eq!(
            previous_config_focus(PluginConfigFocus::Editor),
            PluginConfigFocus::Structure
        );
        assert_eq!(
            PluginDetailTab::Overview.move_by(1),
            PluginDetailTab::Config
        );
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
            config_focus: PluginConfigFocus::Structure,
            selected_node: 0,
            config_scroll: 0,
            diagnostics_scroll: 0,
            show_diff: false,
            editor: None,
        };

        assert_eq!(focus_title("Structure", true), "> Structure");
        assert_eq!(transport_display("static"), "native");

        let toolbar = text_to_string(config_toolbar_text(
            dialog.selected_plugin().unwrap(),
            &dialog,
        ));
        assert!(toolbar.contains("[ Validate ]"));
        assert!(toolbar.contains("[ Insert Defaults ]"));
        assert!(!toolbar.contains("Format"));

        let structure = text_to_string(config_structure_text(
            &dialog,
            dialog.selected_plugin().unwrap(),
        ));
        assert!(structure.contains("Config"));
        assert!(structure.contains("Endpoint"));
        assert!(structure.contains("Limits"));

        let editor = text_to_string(config_editor_text(
            &dialog,
            dialog.selected_plugin().unwrap(),
        ));
        assert!(editor.contains("Object editor"));
        assert!(editor.contains("Endpoint"));
        assert!(editor.contains("Limits"));
        assert!(editor.contains("[ Add field ]"));

        let info = text_to_string(field_info_text(&dialog, dialog.selected_plugin().unwrap()));
        assert!(info.contains("Path"));
        assert!(info.contains("/"));
        assert!(info.contains("Type"));
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
