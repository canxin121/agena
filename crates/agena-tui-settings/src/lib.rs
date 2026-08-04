//! Presentation state and input reducer for the settings workbench.
//!
//! The application supplies display strings and an opaque action for each row.
//! It remains responsible for configuration reads, persistence, process launch,
//! and opening concrete workbenches. This module owns the display-only section
//! hierarchy plus its selection, query, and keyboard-navigation policy.

use crossterm::event::KeyEvent;

use agena_tui_components::{SectionedListFocus, SectionedListSection, SectionedListState};

use agena_tui::i18n::I18n;
use agena_tui::keymap::{KeyAction, KeyContext, resolve};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsAction {
    SelectField { path: String },
    EditField { path: String, value: String },
    Save,
    Reload,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsEffect {
    LoadSnapshot,
    SaveField { path: String, value: String },
    ReloadRuntime,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
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
pub enum SettingsStudioSectionId {
    ModelsProviders,
    Agents,
    Permissions,
    PluginsTools,
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
        | SettingsStudioSectionId::PluginsTools => "overlay-settings-group-core",
        SettingsStudioSectionId::RuntimeSession
        | SettingsStudioSectionId::ProviderClientVersions
        | SettingsStudioSectionId::Interface => "overlay-settings-group-application",
        SettingsStudioSectionId::Diagnostics => "overlay-settings-group-system",
    };
    i18n.text(key)
}

#[derive(Debug, Clone, PartialEq, Eq)]
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
pub struct SettingsStudioPresentation<A> {
    state: SectionedListState<SettingsStudioSection<A>>,
}

impl<A> SettingsStudioPresentation<A> {
    pub fn new(
        sections: Vec<SettingsStudioSection<A>>,
        selected_section: usize,
        selected_item: usize,
        focus: SettingsStudioFocus,
    ) -> Self {
        Self {
            state: SectionedListState::new(sections, selected_section, selected_item, focus),
        }
    }

    pub fn sections(&self) -> &[SettingsStudioSection<A>] {
        self.state.sections()
    }

    pub fn selected_section(&self) -> Option<&SettingsStudioSection<A>> {
        self.state.selected_section()
    }

    pub fn selected_item(&self) -> Option<&SettingsStudioItem<A>> {
        self.state.selected_item()
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
        self.state.set_focus(focus);
    }

    pub fn set_indices(&mut self, section: usize, item: usize) {
        self.state.set_indices(section, item);
    }

    pub fn move_selection(&mut self, delta: isize) {
        self.state.move_selection(delta);
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    match resolve(KeyContext::SettingsStudio, key) {
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
        Some(KeyAction::Activate) => SettingsStudioEffect::Activate,
        _ => SettingsStudioEffect::KeepOpen,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        SettingsStudioEffect, SettingsStudioFocus, SettingsStudioItem, SettingsStudioPresentation,
        SettingsStudioSection, SettingsStudioSectionId, handle_key,
    };
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn presentation() -> SettingsStudioPresentation<()> {
        SettingsStudioPresentation::new(
            vec![
                SettingsStudioSection {
                    id: SettingsStudioSectionId::Agents,
                    label: "Agents".to_owned(),
                    summary: "Profiles".to_owned(),
                    description: "Agent configuration".to_owned(),
                    items: vec![SettingsStudioItem::new("Default agent", "review", "", ())],
                },
                SettingsStudioSection {
                    id: SettingsStudioSectionId::Diagnostics,
                    label: "Diagnostics".to_owned(),
                    summary: "Tracing".to_owned(),
                    description: "Terminal diagnostics".to_owned(),
                    items: vec![SettingsStudioItem::new("Terminal", "ready", "", ())],
                },
            ],
            0,
            0,
            SettingsStudioFocus::Navigation,
        )
    }

    #[test]
    fn query_selects_visible_row_and_moves_focus_to_items() {
        let mut presentation = presentation();

        assert!(presentation.select_query("terminal"));
        assert_eq!(presentation.selected_section_index(), 1);
        assert_eq!(presentation.selected_item_index(), 0);
        assert_eq!(presentation.focus(), SettingsStudioFocus::Items);
    }

    #[test]
    fn empty_query_keeps_existing_selection() {
        let mut presentation = presentation();

        assert!(!presentation.select_query("  "));
        assert_eq!(presentation.selected_section_index(), 0);
        assert_eq!(presentation.focus(), SettingsStudioFocus::Navigation);
    }

    #[test]
    fn reducer_owns_pane_focus_and_refresh_intent() {
        let mut presentation = presentation();

        assert_eq!(
            handle_key(
                &mut presentation,
                KeyEvent::new(KeyCode::Right, KeyModifiers::NONE),
            ),
            SettingsStudioEffect::KeepOpen
        );
        assert_eq!(presentation.focus(), SettingsStudioFocus::Items);
        assert_eq!(
            handle_key(
                &mut presentation,
                KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL),
            ),
            SettingsStudioEffect::Refresh
        );
    }
}
