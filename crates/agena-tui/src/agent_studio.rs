//! Display state and input reducer for the agent-profile workbench.
//!
//! The application projects concrete agent profiles into rows and supplies an
//! opaque action for each row. It retains profile persistence, source-file
//! access, runtime reload, and editor submission; this module owns the list
//! presentation and its keyboard navigation.

use crossterm::event::KeyEvent;

use agena_tui_components::SelectableListState;

use crate::keymap::{KeyAction, KeyContext, resolve};

#[derive(Debug, Clone)]
pub struct AgentStudioItem<A> {
    pub label: String,
    pub value: String,
    pub detail: String,
    pub action: A,
}

#[derive(Debug, Clone)]
pub struct AgentStudioPresentation<A> {
    pub title: String,
    pub footer: String,
    pub list: SelectableListState<AgentStudioItem<A>>,
}

impl<A> AgentStudioPresentation<A> {
    pub fn new(
        title: impl Into<String>,
        footer: impl Into<String>,
        items: Vec<AgentStudioItem<A>>,
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
pub enum AgentStudioEffect {
    KeepOpen,
    Close,
    Activate,
}

pub fn handle_key<A>(
    presentation: &mut AgentStudioPresentation<A>,
    key: KeyEvent,
) -> AgentStudioEffect {
    match resolve(KeyContext::AgentStudio, key) {
        Some(KeyAction::Close) => AgentStudioEffect::Close,
        _ if presentation.list.handle_structural_navigation_key(key, 10) => {
            AgentStudioEffect::KeepOpen
        }
        Some(KeyAction::Activate) => AgentStudioEffect::Activate,
        _ => AgentStudioEffect::KeepOpen,
    }
}

#[cfg(test)]
mod tests {
    use super::{AgentStudioEffect, AgentStudioItem, AgentStudioPresentation, handle_key};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn presentation() -> AgentStudioPresentation<()> {
        AgentStudioPresentation::new(
            "Agents",
            "footer",
            vec![
                AgentStudioItem {
                    label: "Description".to_owned(),
                    value: "".to_owned(),
                    detail: "Edit the description".to_owned(),
                    action: (),
                },
                AgentStudioItem {
                    label: "Prompt".to_owned(),
                    value: "".to_owned(),
                    detail: "Edit the prompt".to_owned(),
                    action: (),
                },
            ],
            0,
        )
    }

    #[test]
    fn navigation_is_owned_by_the_presentation() {
        let mut presentation = presentation();
        assert_eq!(
            handle_key(
                &mut presentation,
                KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
            ),
            AgentStudioEffect::KeepOpen
        );
        assert_eq!(presentation.list.selected, 1);
    }

    #[test]
    fn activation_is_an_application_effect() {
        let mut presentation = presentation();
        assert_eq!(
            handle_key(
                &mut presentation,
                KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            ),
            AgentStudioEffect::Activate
        );
    }
}
