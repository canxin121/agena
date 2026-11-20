//! Presentation vocabulary for the session-model picker.
//!
//! Runtime catalog lookup and model persistence remain application effects.
//! This module owns the API-independent picker rows, identity, current-marker,
//! and selection intent used by the terminal presentation.

use std::borrow::Cow;

use agena_tui_components::{
    Editor, SearchPicker, SearchPickerConfig, SearchPickerDialogSpec, SearchPickerInputResult,
    SearchPickerItem, SearchPickerNoCustom, render_search_picker_dialog,
};
use crossterm::event::KeyEvent;
use ratatui::{Frame, layout::Rect, style::Style};

use crate::{
    i18n::I18n,
    keymap::{KeyAction, KeyContext, resolve},
    sanitize_picker_text,
};

/// Stable display identity selected by the session-model picker.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct SessionModelIdentity {
    pub provider_id: String,
    pub adapter_id: Option<String>,
    pub model_id: String,
}

impl SessionModelIdentity {
    pub fn new(
        provider_id: impl Into<String>,
        adapter_id: Option<String>,
        model_id: impl Into<String>,
    ) -> Self {
        Self {
            provider_id: provider_id.into(),
            adapter_id,
            model_id: model_id.into(),
        }
    }

    pub fn key(&self) -> String {
        format!(
            "{}/{}/{}",
            self.provider_id,
            self.adapter_id.as_deref().unwrap_or_default(),
            self.model_id
        )
    }
}

/// Why the application opened the model picker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionModelChooserPurpose {
    RuntimeOverride,
    GlobalDefault,
    WorkspaceDefault,
    Auxiliary,
    WorkspaceAuxiliary,
    PermissionApproval,
    WorkspacePermissionApproval,
}

/// One display row in the session-model picker.
#[derive(Debug, Clone)]
pub struct SessionModelChoiceItem {
    pub label: String,
    pub detail: String,
    pub search_text: String,
    pub identity: SessionModelIdentity,
    pub current: bool,
    pub disabled: bool,
}

impl SearchPickerItem for SessionModelChoiceItem {
    fn search_picker_key(&self) -> Cow<'_, str> {
        Cow::Owned(self.identity.key())
    }

    fn search_picker_label(&self) -> Cow<'_, str> {
        Cow::Borrowed(&self.label)
    }

    fn search_picker_detail(&self) -> Option<Cow<'_, str>> {
        Some(Cow::Borrowed(&self.detail))
    }

    fn search_picker_search_text(&self) -> Cow<'_, str> {
        Cow::Borrowed(&self.search_text)
    }

    fn search_picker_prefix(&self) -> Option<Cow<'_, str>> {
        self.current.then_some(Cow::Borrowed("✓ "))
    }

    fn search_picker_label_style(&self) -> Style {
        if self.disabled {
            agena_tui_components::theme::muted_style().add_modifier(ratatui::style::Modifier::DIM)
        } else {
            Style::default()
        }
    }
    fn search_picker_detail_style(&self) -> Style {
        agena_tui_components::theme::muted_style()
    }
    fn search_picker_prefix_style(&self) -> Style {
        if self.disabled {
            self.search_picker_label_style()
        } else {
            Style::default().fg(agena_tui_components::theme::accent_color())
        }
    }
}

/// Presentation state attached to the shared picker component.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionModelChooserPresentation {
    pub purpose: SessionModelChooserPurpose,
    pub editable: bool,
    pub explicit_local: bool,
}

impl SessionModelChooserPresentation {
    pub fn new(purpose: SessionModelChooserPurpose) -> Self {
        Self {
            purpose,
            editable: true,
            explicit_local: false,
        }
    }

    pub fn selection_effect(&self, item: &SessionModelChoiceItem) -> SessionModelChooserEffect {
        SessionModelChooserEffect::Select {
            purpose: self.purpose,
            identity: item.identity.clone(),
        }
    }
}

/// A concrete picker selection intent for the App's Runtime/persistence adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionModelChooserEffect {
    Select {
        purpose: SessionModelChooserPurpose,
        identity: SessionModelIdentity,
    },
}

pub type SessionModelChooserOverlay = SearchPicker<
    SessionModelChoiceItem,
    SearchPickerNoCustom,
    SessionModelChooserPresentation,
    Editor,
>;

/// Creates the complete model-picker presentation without exposing picker
/// configuration policy to App code.
pub fn new_presentation(
    title: String,
    prompt: String,
    footer: String,
    empty_message: String,
    purpose: SessionModelChooserPurpose,
) -> SessionModelChooserOverlay {
    let mut dialog = SessionModelChooserOverlay::new(
        title,
        prompt,
        footer,
        empty_message,
        Editor::default(),
        SearchPickerConfig::searchable(),
        None,
        SessionModelChooserPresentation::new(purpose),
    );
    dialog.set_loading(true);
    dialog
}

#[derive(Debug, Clone)]
/// Action of the session model chooser.
pub enum SessionModelChooserAction {
    Input(KeyEvent),
    Paste(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Effect produced by the session model chooser reducer.
pub enum SessionModelChooserReducerEffect {
    Close,
    KeepOpen,
    Source,
    Select {
        purpose: SessionModelChooserPurpose,
        identity: SessionModelIdentity,
    },
}

/// Reduces model-picker input into an opaque model identity intent. Runtime
/// model lookup and applying the persisted/runtime selection remain App
/// effects.
pub fn reduce(
    dialog: &mut SessionModelChooserOverlay,
    action: SessionModelChooserAction,
) -> SessionModelChooserReducerEffect {
    match action {
        SessionModelChooserAction::Input(key)
            if resolve(KeyContext::SessionModel, key) == Some(KeyAction::SettingsSource) =>
        {
            SessionModelChooserReducerEffect::Source
        }
        SessionModelChooserAction::Input(key)
            if resolve(KeyContext::SessionModel, key) == Some(KeyAction::Accept) =>
        {
            if !dialog.meta.editable {
                return SessionModelChooserReducerEffect::KeepOpen;
            }
            dialog
                .selected_item()
                .map(|item| match dialog.meta.selection_effect(item) {
                    SessionModelChooserEffect::Select { purpose, identity } => {
                        SessionModelChooserReducerEffect::Select { purpose, identity }
                    }
                })
                .unwrap_or(SessionModelChooserReducerEffect::KeepOpen)
        }
        SessionModelChooserAction::Input(key) => match dialog.handle_input_key(key) {
            SearchPickerInputResult::Close => SessionModelChooserReducerEffect::Close,
            SearchPickerInputResult::Navigated | SearchPickerInputResult::Edited { .. } => {
                SessionModelChooserReducerEffect::KeepOpen
            }
        },
        SessionModelChooserAction::Paste(text) => {
            dialog.input.insert_str(text.as_str());
            refresh(dialog, false);
            SessionModelChooserReducerEffect::KeepOpen
        }
    }
}

/// Re-applies query filtering while preserving the active model whenever it
/// remains visible. This is display selection policy, not a Runtime query.
pub fn refresh(dialog: &mut SessionModelChooserOverlay, prefer_current_model: bool) {
    let previous_model = if dialog.query_changed_since_results() {
        None
    } else {
        dialog.selected_item().map(|item| item.identity.clone())
    };
    dialog.refresh_results();
    if dialog.result_count() == 0 {
        dialog.selected = 0;
        return;
    }
    if prefer_current_model && dialog.select_item_where(|item| item.current) {
        return;
    }
    if let Some(previous_model) = previous_model
        && dialog.select_item_where(|item| item.identity == previous_model)
    {
        return;
    }
    dialog.clamp_selection();
}

/// Renders the full model-picker dialog from TUI-owned display state.
pub fn render_overlay(
    frame: &mut Frame<'_>,
    area: Rect,
    dialog: &SessionModelChooserOverlay,
    i18n: &I18n,
) {
    let spec = SearchPickerDialogSpec::new(
        i18n.text("overlay-picker-loading").into(),
        i18n.text("overlay-session-model-list-title").into(),
    )
    .with_text(i18n);
    render_search_picker_dialog(frame, area, dialog, &spec, sanitize_picker_text);
}

#[cfg(test)]
mod inheritance_tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyModifiers};
    use ratatui::{Terminal, backend::TestBackend, style::Modifier};
    #[test]
    fn inherited_model_picker_renders_disabled_values_and_keeps_source_shortcut() {
        let mut picker = new_presentation(
            "Model".into(),
            "inherit · Parent configuration · parent/model".into(),
            "Ctrl+O source".into(),
            "empty".into(),
            SessionModelChooserPurpose::WorkspaceDefault,
        );
        picker.meta.editable = false;
        picker.set_loading(false);
        picker.replace_items(vec![SessionModelChoiceItem {
            label: "λInheritedModel".into(),
            detail: "parent/model".into(),
            search_text: "model".into(),
            identity: SessionModelIdentity::new("parent", None, "model"),
            current: true,
            disabled: true,
        }]);
        assert_eq!(
            reduce(
                &mut picker,
                SessionModelChooserAction::Input(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            ),
            SessionModelChooserReducerEffect::KeepOpen
        );
        assert_eq!(
            reduce(
                &mut picker,
                SessionModelChooserAction::Input(KeyEvent::new(
                    KeyCode::Char('o'),
                    KeyModifiers::CONTROL
                ))
            ),
            SessionModelChooserReducerEffect::Source
        );
        let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
        terminal
            .draw(|frame| render_overlay(frame, frame.area(), &picker, &I18n::english()))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let text = buffer
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("Parent configuration"));
        assert!(text.contains("parent/model"));
        let value = buffer
            .content
            .iter()
            .find(|cell| cell.symbol() == "λ")
            .unwrap();
        assert_eq!(value.fg, agena_tui_components::theme::muted_color());
        assert!(value.modifier.contains(Modifier::DIM));
        picker.meta.editable = true;
        assert!(matches!(
            reduce(
                &mut picker,
                SessionModelChooserAction::Input(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            ),
            SessionModelChooserReducerEffect::Select { .. }
        ));
    }
}
