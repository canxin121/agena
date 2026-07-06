use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::Editor;

#[derive(Debug, Clone)]
pub struct EditorDialogState<TAction> {
    pub title: String,
    pub prompt: String,
    pub footer: String,
    pub multiline: bool,
    pub input: Editor,
    pub action: TAction,
}

impl<TAction> EditorDialogState<TAction> {
    pub fn new(
        title: String,
        prompt: String,
        footer: String,
        multiline: bool,
        input: Editor,
        action: TAction,
    ) -> Self {
        Self {
            title,
            prompt,
            footer,
            multiline,
            input,
            action,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditorDialogKeyResult<TAction> {
    Continue,
    Close,
    Submit(TAction, String),
}

pub fn drive_editor_dialog_key<TAction: Clone>(
    editor: &mut EditorDialogState<TAction>,
    key: KeyEvent,
) -> EditorDialogKeyResult<TAction> {
    if matches!(key.code, KeyCode::Esc) {
        return EditorDialogKeyResult::Close;
    }

    if editor.multiline {
        if matches!(key.code, KeyCode::Char('s')) && key.modifiers.contains(KeyModifiers::CONTROL) {
            editor.input.flush_all_pending_input();
            return EditorDialogKeyResult::Submit(
                editor.action.clone(),
                editor.input.text().to_string(),
            );
        }
        editor.input.handle_multiline_input_key(key);
        return EditorDialogKeyResult::Continue;
    }

    match key.code {
        KeyCode::Enter => {
            editor.input.flush_all_pending_input();
            EditorDialogKeyResult::Submit(editor.action.clone(), editor.input.text().to_string())
        }
        _ => {
            editor.input.handle_line_input_key(key);
            EditorDialogKeyResult::Continue
        }
    }
}
