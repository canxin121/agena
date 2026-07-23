//! Presentation navigation for the plugin workbench.
//!
//! The application projects plugin manifests, configuration state, and runtime
//! diagnostics into the workbench, but this module owns the display-only page
//! and tab navigation policy. Configuration editing, schema validation,
//! persistence, reload, and process control deliberately remain application
//! effects.

use std::borrow::Cow;

use crossterm::event::KeyEvent;

use agena_tui_components::{
    Editor, SearchPicker, SearchPickerConfig, SearchPickerItem, SearchPickerNoCustom,
    SearchPickerSelectionMode,
};

use crate::keymap::{KeyAction, KeyContext, resolve};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginWorkbenchMode {
    List,
    Detail,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginDetailTab {
    Config,
    Tools,
    Commands,
    Capabilities,
    Logs,
    Diagnostics,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginTransportFilter {
    All,
    Static,
    Stdio,
    Cdylib,
    Http,
    Wasm,
    Other,
}

impl PluginTransportFilter {
    pub fn label(self) -> &'static str {
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

    pub fn next(self) -> Self {
        match self {
            Self::All => Self::Static,
            Self::Static => Self::Stdio,
            Self::Stdio => Self::Cdylib,
            Self::Cdylib => Self::Http,
            Self::Http => Self::Wasm,
            Self::Wasm => Self::Other,
            Self::Other => Self::All,
        }
    }

    pub fn matches(self, transport: &str) -> bool {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginConfigFilter {
    All,
    Valid,
    Missing,
    SchemaMissing,
    Issues,
    NeedsRestart,
    RuntimeIssue,
}

impl PluginConfigFilter {
    pub fn label(self) -> &'static str {
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

    pub fn next(self) -> Self {
        match self {
            Self::All => Self::Valid,
            Self::Valid => Self::Missing,
            Self::Missing => Self::SchemaMissing,
            Self::SchemaMissing => Self::Issues,
            Self::Issues => Self::NeedsRestart,
            Self::NeedsRestart => Self::RuntimeIssue,
            Self::RuntimeIssue => Self::All,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginConfigFilterValue {
    Valid,
    Missing,
    SchemaMissing,
    Issues,
    NeedsRestart,
    RuntimeIssue,
}

#[derive(Debug, Clone)]
pub struct PluginWorkbenchListItem {
    pub key: String,
    pub search_text: Vec<String>,
    pub transport: String,
    pub config_filter_value: PluginConfigFilterValue,
}

#[derive(Debug, Clone)]
pub struct PluginWorkbenchListPresentation {
    pub query: Editor,
    pub transport_filter: PluginTransportFilter,
    pub config_filter: PluginConfigFilter,
    items: Vec<PluginWorkbenchListItem>,
    visible_indices: Vec<usize>,
    selected_visible_index: usize,
}

/// Opaque display row for a schema-aware configuration action or value picker.
/// The App keeps schema paths, JSON values, branch records, and persistence
/// commands in a key-to-concrete-effect map.
#[derive(Debug, Clone)]
pub struct PluginConfigPickerItem {
    pub key: String,
    pub label: String,
    pub detail: Option<String>,
    pub initially_selected: bool,
}

impl SearchPickerItem for PluginConfigPickerItem {
    fn search_picker_key(&self) -> Cow<'_, str> {
        Cow::Borrowed(self.key.as_str())
    }

    fn search_picker_label(&self) -> Cow<'_, str> {
        Cow::Borrowed(self.label.as_str())
    }

    fn search_picker_detail(&self) -> Option<Cow<'_, str>> {
        self.detail.as_deref().map(Cow::Borrowed)
    }
}

pub type PluginConfigPickerPresentation =
    SearchPicker<PluginConfigPickerItem, SearchPickerNoCustom, (), Editor>;

pub fn new_plugin_config_picker(
    title: String,
    prompt: String,
    footer: String,
    empty_message: String,
    multi_select: bool,
    items: Vec<PluginConfigPickerItem>,
) -> PluginConfigPickerPresentation {
    let selected = items
        .iter()
        .position(|item| item.initially_selected)
        .unwrap_or_default();
    let checked_keys = items
        .iter()
        .filter(|item| item.initially_selected)
        .map(|item| item.key.clone())
        .collect::<Vec<_>>();
    let mut presentation = PluginConfigPickerPresentation::new(
        title,
        prompt,
        footer,
        empty_message,
        Editor::default(),
        SearchPickerConfig {
            selection_mode: if multi_select {
                SearchPickerSelectionMode::Multiple
            } else {
                SearchPickerSelectionMode::Single
            },
            ..SearchPickerConfig::select_only()
        },
        None,
        (),
    );
    presentation.replace_items(items);
    presentation.selected = selected;
    presentation.checked_keys = checked_keys;
    presentation
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginConfigPickerAction {
    Close,
    MoveUp,
    MoveDown,
    PageUp,
    PageDown,
    Toggle,
    Accept,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginConfigPickerEffect {
    Close,
    Activate { key: String },
    KeepOpen,
}

pub fn reduce_plugin_config_picker(
    presentation: &mut PluginConfigPickerPresentation,
    action: PluginConfigPickerAction,
) -> PluginConfigPickerEffect {
    match action {
        PluginConfigPickerAction::Close => PluginConfigPickerEffect::Close,
        PluginConfigPickerAction::MoveUp => {
            presentation.move_selection(-1);
            PluginConfigPickerEffect::KeepOpen
        }
        PluginConfigPickerAction::MoveDown => {
            presentation.move_selection(1);
            PluginConfigPickerEffect::KeepOpen
        }
        PluginConfigPickerAction::PageUp => {
            presentation.move_selection_page(-1);
            PluginConfigPickerEffect::KeepOpen
        }
        PluginConfigPickerAction::PageDown => {
            presentation.move_selection_page(1);
            PluginConfigPickerEffect::KeepOpen
        }
        PluginConfigPickerAction::Toggle => {
            presentation.toggle_selected();
            PluginConfigPickerEffect::KeepOpen
        }
        PluginConfigPickerAction::Accept => presentation
            .selected_item()
            .map(|item| PluginConfigPickerEffect::Activate {
                key: item.key.clone(),
            })
            .unwrap_or(PluginConfigPickerEffect::KeepOpen),
    }
}

impl PluginWorkbenchListPresentation {
    pub fn new(items: Vec<PluginWorkbenchListItem>, query: impl Into<String>) -> Self {
        let mut presentation = Self {
            query: Editor::from_text(query.into()),
            transport_filter: PluginTransportFilter::All,
            config_filter: PluginConfigFilter::All,
            items,
            visible_indices: Vec::new(),
            selected_visible_index: 0,
        };
        presentation.rebuild_visible_indices();
        presentation
    }

    pub fn replace_items(&mut self, items: Vec<PluginWorkbenchListItem>) {
        let selected_key = self.selected_key().map(str::to_owned);
        self.items = items;
        self.rebuild_visible_indices();
        if let Some(selected_key) = selected_key {
            self.select_key(selected_key.as_str());
        }
    }

    pub fn visible_len(&self) -> usize {
        self.visible_indices.len()
    }

    pub fn visible_key(&self, visible_index: usize) -> Option<&str> {
        self.visible_indices
            .get(visible_index)
            .and_then(|index| self.items.get(*index))
            .map(|item| item.key.as_str())
    }

    pub fn selected_key(&self) -> Option<&str> {
        self.visible_key(self.selected_visible_index)
    }

    pub fn selected_visible_index(&self) -> usize {
        self.selected_visible_index
    }

    pub fn select_key(&mut self, key: &str) {
        if let Some(index) =
            (0..self.visible_indices.len()).find(|index| self.visible_key(*index) == Some(key))
        {
            self.selected_visible_index = index;
        }
        self.clamp_selection();
    }

    pub fn append_query_text(&mut self, text: &str) {
        self.query.insert_str(text);
        self.rebuild_visible_indices();
    }

    fn rebuild_visible_indices(&mut self) {
        let query = self.query.text().trim().to_ascii_lowercase();
        self.visible_indices = self
            .items
            .iter()
            .enumerate()
            .filter_map(|(index, item)| {
                let matches_query = query.is_empty()
                    || item
                        .search_text
                        .iter()
                        .any(|text| text.to_ascii_lowercase().contains(query.as_str()));
                let matches_transport = self.transport_filter.matches(item.transport.as_str());
                let matches_config = match self.config_filter {
                    PluginConfigFilter::All => true,
                    PluginConfigFilter::Valid => {
                        item.config_filter_value == PluginConfigFilterValue::Valid
                    }
                    PluginConfigFilter::Missing => {
                        item.config_filter_value == PluginConfigFilterValue::Missing
                    }
                    PluginConfigFilter::SchemaMissing => {
                        item.config_filter_value == PluginConfigFilterValue::SchemaMissing
                    }
                    PluginConfigFilter::Issues => {
                        item.config_filter_value == PluginConfigFilterValue::Issues
                    }
                    PluginConfigFilter::NeedsRestart => {
                        item.config_filter_value == PluginConfigFilterValue::NeedsRestart
                    }
                    PluginConfigFilter::RuntimeIssue => {
                        item.config_filter_value == PluginConfigFilterValue::RuntimeIssue
                    }
                };
                (matches_query && matches_transport && matches_config).then_some(index)
            })
            .collect();
        self.clamp_selection();
    }

    fn clamp_selection(&mut self) {
        self.selected_visible_index = self
            .selected_visible_index
            .min(self.visible_indices.len().saturating_sub(1));
    }

    fn move_selection(&mut self, delta: isize) {
        if self.visible_indices.is_empty() {
            self.selected_visible_index = 0;
            return;
        }
        self.selected_visible_index = (self.selected_visible_index as isize + delta)
            .clamp(0, self.visible_indices.len().saturating_sub(1) as isize)
            as usize;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginWorkbenchListEffect {
    KeepOpen,
    Refresh,
}

pub fn handle_list_key(
    presentation: &mut PluginWorkbenchListPresentation,
    key: KeyEvent,
) -> PluginWorkbenchListEffect {
    match resolve(KeyContext::PluginList, key) {
        Some(KeyAction::PluginCycleTransport) => {
            presentation.transport_filter = presentation.transport_filter.next();
            presentation.rebuild_visible_indices();
        }
        Some(KeyAction::PluginCycleConfig) => {
            presentation.config_filter = presentation.config_filter.next();
            presentation.rebuild_visible_indices();
        }
        Some(KeyAction::Refresh) => return PluginWorkbenchListEffect::Refresh,
        Some(KeyAction::MoveUp) => presentation.move_selection(-1),
        Some(KeyAction::MoveDown) => presentation.move_selection(1),
        _ => {
            let before = presentation.query.text().to_owned();
            presentation.query.handle_line_input_key(key);
            if presentation.query.text() != before {
                presentation.rebuild_visible_indices();
            }
        }
    }
    PluginWorkbenchListEffect::KeepOpen
}

impl PluginDetailTab {
    pub const ALL: [Self; 6] = [
        Self::Config,
        Self::Tools,
        Self::Commands,
        Self::Capabilities,
        Self::Logs,
        Self::Diagnostics,
    ];

    pub fn label(self) -> &'static str {
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
        let next = (index as isize + delta).rem_euclid(Self::ALL.len() as isize) as usize;
        Self::ALL[next]
    }
}

/// The authoritative display navigation state for a plugin workbench.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PluginWorkbenchNavigation {
    pub mode: PluginWorkbenchMode,
    pub detail_tab: PluginDetailTab,
}

impl Default for PluginWorkbenchNavigation {
    fn default() -> Self {
        Self::new()
    }
}

impl PluginWorkbenchNavigation {
    pub const fn new() -> Self {
        Self {
            mode: PluginWorkbenchMode::List,
            detail_tab: PluginDetailTab::Config,
        }
    }

    pub fn return_to_list(&mut self) {
        self.mode = PluginWorkbenchMode::List;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginWorkbenchNavigationEffect {
    KeepOpen,
    Close,
    OpenSelected,
    ScrollDetail(isize),
}

/// Reduces list/detail navigation into display state and semantic host effects.
///
/// A Config-tab key is intentionally not interpreted here: its focused editor
/// and schema-aware interactions are owned by the application's configuration
/// feature. The caller should pass `true` for that case and delegate it.
pub fn handle_key(
    navigation: &mut PluginWorkbenchNavigation,
    key: KeyEvent,
    config_tab_active: bool,
) -> PluginWorkbenchNavigationEffect {
    match navigation.mode {
        PluginWorkbenchMode::List => match resolve(KeyContext::PluginList, key) {
            Some(KeyAction::Close) => PluginWorkbenchNavigationEffect::Close,
            Some(KeyAction::Open) => {
                navigation.mode = PluginWorkbenchMode::Detail;
                navigation.detail_tab = PluginDetailTab::Config;
                PluginWorkbenchNavigationEffect::OpenSelected
            }
            _ => PluginWorkbenchNavigationEffect::KeepOpen,
        },
        PluginWorkbenchMode::Detail if config_tab_active => {
            PluginWorkbenchNavigationEffect::KeepOpen
        }
        PluginWorkbenchMode::Detail => match resolve(KeyContext::PluginDetail, key) {
            Some(KeyAction::Back) => {
                navigation.return_to_list();
                PluginWorkbenchNavigationEffect::KeepOpen
            }
            Some(KeyAction::NextTab) => {
                navigation.detail_tab = navigation.detail_tab.move_by(1);
                PluginWorkbenchNavigationEffect::KeepOpen
            }
            Some(KeyAction::PreviousTab) => {
                navigation.detail_tab = navigation.detail_tab.move_by(-1);
                PluginWorkbenchNavigationEffect::KeepOpen
            }
            Some(KeyAction::MoveUp) => PluginWorkbenchNavigationEffect::ScrollDetail(-1),
            Some(KeyAction::MoveDown) => PluginWorkbenchNavigationEffect::ScrollDetail(1),
            _ => PluginWorkbenchNavigationEffect::KeepOpen,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{
        PluginConfigPickerAction, PluginConfigPickerEffect, PluginConfigPickerItem,
        PluginDetailTab, PluginWorkbenchMode, PluginWorkbenchNavigation,
        PluginWorkbenchNavigationEffect, handle_key, new_plugin_config_picker,
        reduce_plugin_config_picker,
    };
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    #[test]
    fn config_picker_returns_an_opaque_selected_key() {
        let mut presentation = new_plugin_config_picker(
            "Actions".to_owned(),
            "Choose".to_owned(),
            String::new(),
            "Empty".to_owned(),
            false,
            vec![PluginConfigPickerItem {
                key: "action:reset".to_owned(),
                label: "Reset".to_owned(),
                detail: Some("Restore defaults".to_owned()),
                initially_selected: true,
            }],
        );

        assert_eq!(
            reduce_plugin_config_picker(&mut presentation, PluginConfigPickerAction::Accept),
            PluginConfigPickerEffect::Activate {
                key: "action:reset".to_owned(),
            }
        );
    }

    #[test]
    fn config_picker_owns_multi_select_toggle_state() {
        let mut presentation = new_plugin_config_picker(
            "Choices".to_owned(),
            "Choose".to_owned(),
            "Space toggle".to_owned(),
            "Empty".to_owned(),
            true,
            vec![PluginConfigPickerItem {
                key: "value:alpha".to_owned(),
                label: "Alpha".to_owned(),
                detail: None,
                initially_selected: false,
            }],
        );

        assert_eq!(
            reduce_plugin_config_picker(&mut presentation, PluginConfigPickerAction::Toggle),
            PluginConfigPickerEffect::KeepOpen
        );
        assert_eq!(presentation.checked_keys, vec!["value:alpha".to_owned()]);
    }

    #[test]
    fn list_open_reduces_to_detail_and_requests_host_selection() {
        let mut navigation = PluginWorkbenchNavigation::new();

        assert_eq!(
            handle_key(
                &mut navigation,
                KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
                false,
            ),
            PluginWorkbenchNavigationEffect::OpenSelected,
        );
        assert_eq!(navigation.mode, PluginWorkbenchMode::Detail);
        assert_eq!(navigation.detail_tab, PluginDetailTab::Config);
    }

    #[test]
    fn detail_tab_cycle_and_scroll_are_presentation_effects() {
        let mut navigation = PluginWorkbenchNavigation {
            mode: PluginWorkbenchMode::Detail,
            detail_tab: PluginDetailTab::Tools,
        };

        assert_eq!(
            handle_key(
                &mut navigation,
                KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE),
                false,
            ),
            PluginWorkbenchNavigationEffect::KeepOpen,
        );
        assert_eq!(navigation.detail_tab, PluginDetailTab::Commands);
        assert_eq!(
            handle_key(
                &mut navigation,
                KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
                false,
            ),
            PluginWorkbenchNavigationEffect::ScrollDetail(1),
        );
    }

    #[test]
    fn config_tab_is_left_to_the_schema_aware_application_feature() {
        let mut navigation = PluginWorkbenchNavigation {
            mode: PluginWorkbenchMode::Detail,
            detail_tab: PluginDetailTab::Config,
        };

        assert_eq!(
            handle_key(
                &mut navigation,
                KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE),
                true,
            ),
            PluginWorkbenchNavigationEffect::KeepOpen,
        );
        assert_eq!(navigation.detail_tab, PluginDetailTab::Config);
    }

    #[test]
    fn list_presentation_owns_filtering_and_stable_selection() {
        use super::{
            PluginConfigFilterValue, PluginTransportFilter, PluginWorkbenchListItem,
            PluginWorkbenchListPresentation,
        };

        let mut presentation = PluginWorkbenchListPresentation::new(
            vec![
                PluginWorkbenchListItem {
                    key: "native".to_owned(),
                    search_text: vec!["native plugin".to_owned()],
                    transport: "static".to_owned(),
                    config_filter_value: PluginConfigFilterValue::Valid,
                },
                PluginWorkbenchListItem {
                    key: "remote".to_owned(),
                    search_text: vec!["remote plugin".to_owned()],
                    transport: "http".to_owned(),
                    config_filter_value: PluginConfigFilterValue::Issues,
                },
            ],
            "",
        );

        presentation.transport_filter = PluginTransportFilter::Http;
        presentation.append_query_text("remote");
        assert_eq!(presentation.visible_len(), 1);
        assert_eq!(presentation.selected_key(), Some("remote"));
    }

    #[test]
    fn replacing_rows_reapplies_filters_and_preserves_the_selected_key() {
        use super::{
            PluginConfigFilter, PluginConfigFilterValue, PluginWorkbenchListItem,
            PluginWorkbenchListPresentation,
        };

        let remote = PluginWorkbenchListItem {
            key: "remote".to_owned(),
            search_text: vec!["remote plugin".to_owned()],
            transport: "http".to_owned(),
            config_filter_value: PluginConfigFilterValue::Issues,
        };
        let mut presentation = PluginWorkbenchListPresentation::new(
            vec![
                PluginWorkbenchListItem {
                    key: "native".to_owned(),
                    search_text: vec!["native plugin".to_owned()],
                    transport: "static".to_owned(),
                    config_filter_value: PluginConfigFilterValue::Valid,
                },
                remote.clone(),
            ],
            "",
        );
        presentation.config_filter = PluginConfigFilter::Issues;
        presentation.replace_items(vec![
            PluginWorkbenchListItem {
                key: "native".to_owned(),
                search_text: vec!["native plugin".to_owned()],
                transport: "static".to_owned(),
                config_filter_value: PluginConfigFilterValue::Valid,
            },
            remote,
        ]);

        assert_eq!(presentation.visible_len(), 1);
        assert_eq!(presentation.selected_key(), Some("remote"));
    }
}
