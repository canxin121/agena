use crossterm::event::{KeyCode, KeyEvent};

use crate::Editor;

#[derive(Debug, Clone)]
pub struct InputDialogState<TAction> {
    pub title: String,
    pub prompt: String,
    pub input: Editor,
    pub action: TAction,
}

impl<TAction> InputDialogState<TAction> {
    pub fn new(title: String, prompt: String, input: Editor, action: TAction) -> Self {
        Self {
            title,
            prompt,
            input,
            action,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputDialogKeyResult<TAction> {
    Continue,
    Close,
    Submit(TAction, String),
}

pub fn drive_input_dialog_key<TAction: Clone>(
    dialog: &mut InputDialogState<TAction>,
    key: KeyEvent,
) -> InputDialogKeyResult<TAction> {
    match key.code {
        KeyCode::Esc => InputDialogKeyResult::Close,
        KeyCode::Enter => {
            dialog.input.flush_all_pending_input();
            InputDialogKeyResult::Submit(dialog.action.clone(), dialog.input.text().to_string())
        }
        _ => {
            dialog.input.handle_line_input_key(key);
            InputDialogKeyResult::Continue
        }
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    use super::{InputDialogKeyResult, InputDialogState, drive_input_dialog_key};
    use crate::Editor;

    #[test]
    fn constructor_preserves_input_dialog_fields() {
        let state = InputDialogState::new(
            "Title".to_string(),
            "Prompt".to_string(),
            Editor::from_text("value".to_string()),
            3_u8,
        );

        assert_eq!(state.title, "Title");
        assert_eq!(state.prompt, "Prompt");
        assert_eq!(state.input.text(), "value");
        assert_eq!(state.action, 3);
    }

    #[test]
    fn input_dialog_submits_on_enter() {
        let mut state = InputDialogState::new(
            "Title".to_string(),
            "Prompt".to_string(),
            Editor::from_text("value".to_string()),
            3_u8,
        );

        let result = drive_input_dialog_key(
            &mut state,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        );

        assert_eq!(result, InputDialogKeyResult::Submit(3, "value".to_string()));
    }

    #[test]
    fn input_dialog_closes_on_escape() {
        let mut state = InputDialogState::new(
            "Title".to_string(),
            "Prompt".to_string(),
            Editor::from_text("value".to_string()),
            3_u8,
        );

        let result =
            drive_input_dialog_key(&mut state, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

        assert_eq!(result, InputDialogKeyResult::Close);
    }

    #[test]
    fn input_dialog_continues_on_text_input() {
        let mut state = InputDialogState::new(
            "Title".to_string(),
            "Prompt".to_string(),
            Editor::from_text(String::new()),
            3_u8,
        );

        let result = drive_input_dialog_key(
            &mut state,
            KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE),
        );

        assert_eq!(result, InputDialogKeyResult::Continue);
        state.input.flush_all_pending_input();
        assert_eq!(state.input.text(), "a");
    }
}
