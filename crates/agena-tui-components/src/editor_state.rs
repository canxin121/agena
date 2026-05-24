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

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    use super::{EditorDialogKeyResult, EditorDialogState, drive_editor_dialog_key};
    use crate::Editor;

    #[test]
    fn constructor_preserves_editor_dialog_fields() {
        let state = EditorDialogState::new(
            "Title".to_string(),
            "Prompt".to_string(),
            "Footer".to_string(),
            true,
            Editor::from_text("body".to_string()),
            7_u8,
        );

        assert_eq!(state.title, "Title");
        assert_eq!(state.prompt, "Prompt");
        assert_eq!(state.footer, "Footer");
        assert!(state.multiline);
        assert_eq!(state.input.text(), "body");
        assert_eq!(state.action, 7);
    }

    #[test]
    fn single_line_editor_submits_on_enter() {
        let mut state = EditorDialogState::new(
            "Title".to_string(),
            "Prompt".to_string(),
            "Footer".to_string(),
            false,
            Editor::from_text("body".to_string()),
            7_u8,
        );

        let result = drive_editor_dialog_key(
            &mut state,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        );

        assert_eq!(result, EditorDialogKeyResult::Submit(7, "body".to_string()));
    }

    #[test]
    fn multiline_editor_submits_on_ctrl_s() {
        let mut state = EditorDialogState::new(
            "Title".to_string(),
            "Prompt".to_string(),
            "Footer".to_string(),
            true,
            Editor::from_text("body".to_string()),
            9_u8,
        );

        let result = drive_editor_dialog_key(
            &mut state,
            KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL),
        );

        assert_eq!(result, EditorDialogKeyResult::Submit(9, "body".to_string()));
    }

    #[test]
    fn editor_closes_on_escape() {
        let mut state = EditorDialogState::new(
            "Title".to_string(),
            "Prompt".to_string(),
            "Footer".to_string(),
            false,
            Editor::from_text("body".to_string()),
            1_u8,
        );

        let result =
            drive_editor_dialog_key(&mut state, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

        assert_eq!(result, EditorDialogKeyResult::Close);
    }
}
