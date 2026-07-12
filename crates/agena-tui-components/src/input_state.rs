use crossterm::event::KeyEvent;

use crate::{Editor, InputDialogAction, input_dialog_action};

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
    match input_dialog_action(key, false) {
        Some(InputDialogAction::Close) => InputDialogKeyResult::Close,
        Some(InputDialogAction::Submit) => {
            InputDialogKeyResult::Submit(dialog.action.clone(), dialog.input.text().to_string())
        }
        _ => {
            dialog.input.handle_line_input_key(key);
            InputDialogKeyResult::Continue
        }
    }
}
