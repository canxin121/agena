//! Presentation state and input reducer for the settings workbench.
//!
//! The application supplies display strings and an opaque action for each row.
//! It remains responsible for configuration reads, persistence, process launch,
//! and opening concrete workbenches. This module owns the display-only section
//! hierarchy plus its selection, query, and keyboard-navigation policy.

use std::cell::Cell;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use agena_tui_components::{Editor, SectionedListFocus, SectionedListSection, SectionedListState};

mod render;
pub use render::{SettingsViewSpec, render_settings};

use agena_tui::i18n::I18n;
use agena_tui::keymap::{KeyAction, KeyContext, resolve};

#[derive(Debug, Clone, PartialEq, Eq)]
/// Action of the settings studio.
pub enum SettingsAction {
    SelectField { path: String },
    EditField { path: String, value: String },
    Save,
    Reload,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Effect produced by the settings studio.
pub enum SettingsEffect {
    LoadSnapshot,
    SaveField { path: String, value: String },
    ReloadRuntime,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
/// State of the settings studio.
pub struct SettingsState {
    pub selected_path: Option<String>,
    pub pending_value: Option<String>,
}

impl SettingsState {
    pub fn reduce(&mut self, action: SettingsAction) -> Option<SettingsEffect> {
        match action {
            SettingsAction::SelectField { path } => {
                self.selected_path = Some(path);
                None
            }
            SettingsAction::EditField { path, value } => {
                self.selected_path = Some(path.clone());
                self.pending_value = Some(value);
                None
            }
            SettingsAction::Save => self
                .selected_path
                .clone()
                .zip(self.pending_value.clone())
                .map(|(path, value)| SettingsEffect::SaveField { path, value }),
            SettingsAction::Reload => Some(SettingsEffect::ReloadRuntime),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Section of the settings studio.
pub enum SettingsStudioSectionId {
    ModelsProviders,
    Agents,
    Permissions,
    PluginsTools,
    McpServer,
    RuntimeSession,
    ProviderClientVersions,
    Interface,
    Diagnostics,
}

pub fn section_group_label(i18n: &I18n, section: SettingsStudioSectionId) -> String {
    let key = match section {
        SettingsStudioSectionId::ModelsProviders
        | SettingsStudioSectionId::Agents
        | SettingsStudioSectionId::Permissions
        | SettingsStudioSectionId::PluginsTools
        | SettingsStudioSectionId::McpServer => "overlay-settings-group-core",
        SettingsStudioSectionId::RuntimeSession
        | SettingsStudioSectionId::ProviderClientVersions
        | SettingsStudioSectionId::Interface => "overlay-settings-group-application",
        SettingsStudioSectionId::Diagnostics => "overlay-settings-group-system",
    };
    i18n.text(key)
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// A source row of the settings studio.
pub struct SettingsStudioSourceRow {
    pub label: String,
    pub value: String,
}

impl SettingsStudioSourceRow {
    pub fn new(label: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            value: value.into(),
        }
    }
}

#[derive(Debug, Clone)]
/// An item of the settings studio.
pub struct SettingsStudioItem<A> {
    pub label: String,
    pub value: String,
    pub detail: String,
    pub path: Option<String>,
    pub current_value: Option<String>,
    pub effective_value: Option<String>,
    pub source_rows: Vec<SettingsStudioSourceRow>,
    pub action: A,
}

impl<A> SettingsStudioItem<A> {
    pub fn new(
        label: impl Into<String>,
        value: impl Into<String>,
        detail: impl Into<String>,
        action: A,
    ) -> Self {
        let value = value.into();
        let current_value = (!value.trim().is_empty()).then(|| value.clone());
        Self::from_parts(
            label,
            value,
            detail,
            None,
            current_value,
            None,
            Vec::new(),
            action,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn from_parts(
        label: impl Into<String>,
        value: impl Into<String>,
        detail: impl Into<String>,
        path: Option<String>,
        current_value: Option<String>,
        effective_value: Option<String>,
        source_rows: Vec<SettingsStudioSourceRow>,
        action: A,
    ) -> Self {
        Self {
            label: label.into(),
            value: value.into(),
            detail: detail.into(),
            path,
            current_value,
            effective_value,
            source_rows,
            action,
        }
    }
}

#[derive(Debug, Clone)]
/// A section of the settings studio.
pub struct SettingsStudioSection<A> {
    pub id: SettingsStudioSectionId,
    pub label: String,
    pub summary: String,
    pub description: String,
    pub items: Vec<SettingsStudioItem<A>>,
}

impl<A> SectionedListSection for SettingsStudioSection<A> {
    type Item = SettingsStudioItem<A>;

    fn items(&self) -> &[Self::Item] {
        self.items.as_slice()
    }
}

pub type SettingsStudioFocus = SectionedListFocus;

#[derive(Debug, Clone)]
/// Presentation of the settings studio.
pub struct SettingsStudioPresentation<A> {
    state: SectionedListState<SettingsStudioSection<A>>,
    search: Editor,
    searching: bool,
    details: bool,
    detail_scroll: Cell<u16>,
    detail_max_scroll: Cell<u16>,
    page_size: Cell<usize>,
    remembered_items: Vec<usize>,
}

impl<A> SettingsStudioPresentation<A> {
    pub fn new(
        sections: Vec<SettingsStudioSection<A>>,
        selected_section: usize,
        selected_item: usize,
        focus: SettingsStudioFocus,
    ) -> Self {
        let count = sections.len();
        let mut presentation = Self {
            state: SectionedListState::new(sections, selected_section, selected_item, focus),
            search: Editor::default(),
            searching: false,
            details: false,
            detail_scroll: Cell::new(0),
            detail_max_scroll: Cell::new(0),
            page_size: Cell::new(5),
            remembered_items: vec![0; count],
        };
        presentation.remember_selection();
        presentation
    }

    pub fn sections(&self) -> &[SettingsStudioSection<A>] {
        self.state.sections()
    }

    pub fn selected_section(&self) -> Option<&SettingsStudioSection<A>> {
        self.state.selected_section()
    }

    pub fn selected_item(&self) -> Option<&SettingsStudioItem<A>> {
        self.visible_item_indices(self.selected_section_index())
            .contains(&self.selected_item_index())
            .then(|| self.state.selected_item())
            .flatten()
    }

    pub fn selected_section_index(&self) -> usize {
        self.state.selected_section_index()
    }

    pub fn selected_item_index(&self) -> usize {
        self.state.selected_item_index()
    }

    pub fn focus(&self) -> SettingsStudioFocus {
        self.state.focus()
    }

    pub fn set_focus(&mut self, focus: SettingsStudioFocus) {
        self.searching = false;
        self.state.set_focus(focus);
    }

    pub fn set_indices(&mut self, section: usize, item: usize) {
        self.state.set_indices(section, item);
        self.remember_selection();
    }

    pub fn move_selection(&mut self, delta: isize) {
        match self.focus() {
            SettingsStudioFocus::Navigation => {
                let indices = self.visible_section_indices();
                if let Some(index) = stepped_index(&indices, self.selected_section_index(), delta) {
                    let item = self.remembered_items.get(index).copied().unwrap_or(0);
                    self.state.set_indices(index, item);
                    self.clamp_filtered_selection();
                }
            }
            SettingsStudioFocus::Items => {
                let section = self.selected_section_index();
                let indices = self.visible_item_indices(section);
                if let Some(index) = stepped_index(&indices, self.selected_item_index(), delta) {
                    self.state.set_indices(section, index);
                }
            }
        }
        self.remember_selection();
    }

    pub fn query(&self) -> &str {
        self.search.text()
    }

    pub fn is_searching(&self) -> bool {
        self.searching
    }

    pub fn details_open(&self) -> bool {
        self.details
    }

    pub fn set_query(&mut self, query: &str) {
        self.search.set_text(query.replace(['\n', '\r', '\t'], " "));
        self.clamp_filtered_selection();
    }

    pub fn paste(&mut self, text: &str) {
        if self.searching {
            self.search
                .insert_str(&text.replace(['\n', '\r', '\t'], " "));
            self.clamp_filtered_selection();
        }
    }

    pub fn visible_section_indices(&self) -> Vec<usize> {
        (0..self.sections().len())
            .filter(|&index| {
                self.query().trim().is_empty() || !self.visible_item_indices(index).is_empty()
            })
            .collect()
    }

    pub fn visible_item_indices(&self, section: usize) -> Vec<usize> {
        let Some(section) = self.sections().get(section) else {
            return Vec::new();
        };
        let query = self.query().to_lowercase();
        let terms = query.split_whitespace().collect::<Vec<_>>();
        section
            .items
            .iter()
            .enumerate()
            .filter_map(|(index, item)| {
                if terms.is_empty() {
                    return Some(index);
                }
                let text = format!(
                    "{} {} {} {} {}",
                    section.label,
                    item.label,
                    item.value,
                    item.detail,
                    item.path.as_deref().unwrap_or_default()
                )
                .to_lowercase();
                terms
                    .iter()
                    .all(|term| text.contains(term))
                    .then_some(index)
            })
            .collect()
    }

    /// Preserve the user's search and per-section position across asynchronous
    /// snapshot refreshes and edits. The new snapshot owns all displayed values.
    pub fn restore_view(&mut self, previous: &Self) {
        for (index, section) in self.state.sections().iter().enumerate() {
            if let Some(old_index) = previous
                .sections()
                .iter()
                .position(|old| old.id == section.id)
            {
                self.remembered_items[index] = previous.remembered_items[old_index];
            }
        }
        self.search = previous.search.clone();
        self.searching = previous.searching;
        self.details = previous.details;
        self.detail_scroll.set(previous.detail_scroll.get());
        self.clamp_filtered_selection();
        self.remember_selection();
    }

    /// Pointer indices address the filtered lists, exactly like keyboard steps.
    pub fn select_visible(&mut self, panel: usize, position: usize) -> bool {
        self.searching = false;
        if panel == 0 {
            if let Some(&section) = self.visible_section_indices().get(position) {
                self.state
                    .set_indices(section, self.remembered_items[section]);
                self.clamp_filtered_selection();
                self.set_focus(SettingsStudioFocus::Items);
            }
            false
        } else if let Some(&item) = self
            .visible_item_indices(self.selected_section_index())
            .get(position)
        {
            self.set_indices(self.selected_section_index(), item);
            self.set_focus(SettingsStudioFocus::Items);
            true
        } else {
            false
        }
    }

    fn remember_selection(&mut self) {
        let section = self.selected_section_index();
        let item = self.selected_item_index();
        if let Some(remembered) = self.remembered_items.get_mut(section) {
            *remembered = item;
        }
    }

    fn clamp_filtered_selection(&mut self) {
        let sections = self.visible_section_indices();
        let section = self.selected_section_index();
        let section = if sections.contains(&section) {
            section
        } else {
            sections.first().copied().unwrap_or(0)
        };
        let items = self.visible_item_indices(section);
        let item = self.selected_item_index();
        let item = if items.contains(&item) {
            item
        } else {
            items.first().copied().unwrap_or(0)
        };
        self.state.set_indices(section, item);
    }

    /// Selects the first presentation row whose visible strings contain the
    /// case-insensitive query. Empty queries intentionally preserve selection.
    pub fn select_query(&mut self, query: &str) -> bool {
        let query = query.trim().to_ascii_lowercase();
        if query.is_empty() {
            return false;
        }
        let selection =
            self.state
                .sections()
                .iter()
                .enumerate()
                .find_map(|(section_index, section)| {
                    section
                        .items
                        .iter()
                        .enumerate()
                        .find_map(|(item_index, item)| {
                            (section.label.to_ascii_lowercase().contains(query.as_str())
                                || section
                                    .summary
                                    .to_ascii_lowercase()
                                    .contains(query.as_str())
                                || section
                                    .description
                                    .to_ascii_lowercase()
                                    .contains(query.as_str())
                                || item.label.to_ascii_lowercase().contains(query.as_str())
                                || item.value.to_ascii_lowercase().contains(query.as_str())
                                || item.detail.to_ascii_lowercase().contains(query.as_str()))
                            .then_some((section_index, item_index))
                        })
                });
        if let Some((section_index, item_index)) = selection {
            self.state.set_indices(section_index, item_index);
            self.state.set_focus(SettingsStudioFocus::Items);
            true
        } else {
            false
        }
    }
}

fn stepped_index(indices: &[usize], selected: usize, delta: isize) -> Option<usize> {
    let position = indices
        .iter()
        .position(|&index| index == selected)
        .unwrap_or(0);
    indices
        .get(
            position
                .saturating_add_signed(delta)
                .min(indices.len().saturating_sub(1)),
        )
        .copied()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Effect produced by the settings studio.
pub enum SettingsStudioEffect {
    KeepOpen,
    Close,
    Refresh,
    Activate,
}

pub fn handle_key<A>(
    presentation: &mut SettingsStudioPresentation<A>,
    key: KeyEvent,
) -> SettingsStudioEffect {
    let action = resolve(KeyContext::SettingsStudio, key);
    if presentation.details {
        if matches!(action, Some(KeyAction::Close | KeyAction::SettingsDetails)) {
            presentation.details = false;
        } else if matches!(
            action,
            Some(
                KeyAction::MoveUp
                    | KeyAction::MoveDown
                    | KeyAction::PageUp
                    | KeyAction::PageDown
                    | KeyAction::Home
                    | KeyAction::End
            )
        ) {
            let mut scroll =
                agena_tui_components::ScrollState::new(presentation.detail_scroll.get());
            scroll.handle_navigation_key(
                key,
                presentation.detail_max_scroll.get(),
                presentation.page_size.get().min(u16::MAX as usize) as u16,
            );
            presentation.detail_scroll.set(scroll.scroll);
        }
        return SettingsStudioEffect::KeepOpen;
    }
    let plain = key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT;
    if presentation.searching {
        match key.code {
            KeyCode::Esc if plain => {
                presentation.set_query("");
                presentation.searching = false;
            }
            KeyCode::Enter | KeyCode::Down | KeyCode::Up if plain => {
                presentation.set_focus(SettingsStudioFocus::Items);
            }
            KeyCode::Tab | KeyCode::BackTab if plain => {
                presentation.set_focus(SettingsStudioFocus::Navigation)
            }
            _ => {
                presentation.search.handle_line_input_key(key);
                presentation.clamp_filtered_selection();
            }
        }
        return SettingsStudioEffect::KeepOpen;
    }
    match action {
        Some(KeyAction::SettingsSearch) => {
            presentation.searching = true;
            SettingsStudioEffect::KeepOpen
        }
        Some(KeyAction::SettingsDetails) => {
            if presentation.selected_item().is_some() {
                presentation.details = true;
                presentation.detail_scroll.set(0);
            }
            SettingsStudioEffect::KeepOpen
        }
        Some(KeyAction::Close) if !presentation.query().is_empty() => {
            presentation.set_query("");
            SettingsStudioEffect::KeepOpen
        }
        Some(KeyAction::PageUp | KeyAction::PageDown | KeyAction::Home | KeyAction::End) => {
            let step = match action {
                Some(KeyAction::PageUp) => -(presentation.page_size.get() as isize),
                Some(KeyAction::PageDown) => presentation.page_size.get() as isize,
                Some(KeyAction::Home) => isize::MIN,
                _ => isize::MAX,
            };
            presentation.move_selection(step);
            SettingsStudioEffect::KeepOpen
        }
        Some(KeyAction::Close) => SettingsStudioEffect::Close,
        Some(KeyAction::MoveLeft) => {
            presentation.set_focus(SettingsStudioFocus::Navigation);
            SettingsStudioEffect::KeepOpen
        }
        Some(KeyAction::MoveRight) => {
            presentation.set_focus(SettingsStudioFocus::Items);
            SettingsStudioEffect::KeepOpen
        }
        Some(KeyAction::NextTab | KeyAction::PreviousTab) => {
            presentation.set_focus(match presentation.focus() {
                SettingsStudioFocus::Navigation => SettingsStudioFocus::Items,
                SettingsStudioFocus::Items => SettingsStudioFocus::Navigation,
            });
            SettingsStudioEffect::KeepOpen
        }
        Some(KeyAction::MoveUp) => {
            presentation.move_selection(-1);
            SettingsStudioEffect::KeepOpen
        }
        Some(KeyAction::MoveDown) => {
            presentation.move_selection(1);
            SettingsStudioEffect::KeepOpen
        }
        Some(KeyAction::Refresh) => SettingsStudioEffect::Refresh,
        Some(KeyAction::Activate)
            if presentation.focus() == SettingsStudioFocus::Navigation
                || presentation.selected_item().is_some() =>
        {
            SettingsStudioEffect::Activate
        }
        _ => SettingsStudioEffect::KeepOpen,
    }
}
