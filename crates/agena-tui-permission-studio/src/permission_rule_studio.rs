//! Presentation state and navigation reducer for the permission-rule workbench.

use agena_tui::keymap::{KeyAction, KeyContext, resolve};
use agena_tui_components::SelectableListState;
use crossterm::event::KeyEvent;

#[derive(Debug, Clone)]
pub struct PermissionRuleStudioItem<A> {
    pub label: String,
    pub value: String,
    pub detail: String,
    pub action: A,
}

#[derive(Debug, Clone)]
pub struct PermissionRuleStudioPresentation<A> {
    pub title: String,
    pub footer: String,
    pub list: SelectableListState<PermissionRuleStudioItem<A>>,
}

impl<A> PermissionRuleStudioPresentation<A> {
    pub fn new(
        title: impl Into<String>,
        footer: impl Into<String>,
        items: Vec<PermissionRuleStudioItem<A>>,
        selected: usize,
    ) -> Self {
        Self {
            title: title.into(),
            footer: footer.into(),
            list: SelectableListState::new(items, selected),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionRuleStudioEffect {
    KeepOpen,
    Close,
    Activate,
    Browse,
    Save,
    Delete,
}

pub fn handle_key<A>(
    presentation: &mut PermissionRuleStudioPresentation<A>,
    key: KeyEvent,
    deletable: bool,
) -> PermissionRuleStudioEffect {
    match resolve(KeyContext::PermissionRuleStudio, key) {
        Some(KeyAction::Close) => PermissionRuleStudioEffect::Close,
        _ if presentation.list.handle_structural_navigation_key(key, 8) => {
            PermissionRuleStudioEffect::KeepOpen
        }
        Some(KeyAction::PermissionBrowse) => PermissionRuleStudioEffect::Browse,
        Some(KeyAction::PermissionSave) => PermissionRuleStudioEffect::Save,
        Some(KeyAction::Delete) if deletable => PermissionRuleStudioEffect::Delete,
        Some(KeyAction::Activate) => PermissionRuleStudioEffect::Activate,
        _ => PermissionRuleStudioEffect::KeepOpen,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        PermissionRuleStudioEffect, PermissionRuleStudioItem, PermissionRuleStudioPresentation,
        handle_key,
    };
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn presentation() -> PermissionRuleStudioPresentation<()> {
        PermissionRuleStudioPresentation::new(
            "Rule",
            "footer",
            vec![
                PermissionRuleStudioItem {
                    label: "Subject".into(),
                    value: "tool".into(),
                    detail: String::new(),
                    action: (),
                },
                PermissionRuleStudioItem {
                    label: "Mode".into(),
                    value: "ask".into(),
                    detail: String::new(),
                    action: (),
                },
            ],
            0,
        )
    }

    #[test]
    fn navigation_and_activation_are_presentation_effects() {
        let mut presentation = presentation();
        assert_eq!(
            handle_key(
                &mut presentation,
                KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
                false
            ),
            PermissionRuleStudioEffect::KeepOpen
        );
        assert_eq!(presentation.list.selected, 1);
        assert_eq!(
            handle_key(
                &mut presentation,
                KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
                false
            ),
            PermissionRuleStudioEffect::Activate
        );
    }
}
