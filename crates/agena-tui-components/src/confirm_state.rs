#[derive(Debug, Clone)]
pub struct ConfirmDialogState<TAction> {
    pub title: String,
    pub body_lines: Vec<String>,
    pub footer: String,
    pub action: TAction,
}

impl<TAction> ConfirmDialogState<TAction> {
    pub fn new(title: String, body_lines: Vec<String>, footer: String, action: TAction) -> Self {
        Self {
            title,
            body_lines,
            footer,
            action,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ConfirmDialogState;

    #[test]
    fn new_preserves_dialog_fields() {
        let state =
            ConfirmDialogState::new("Confirm".into(), vec!["body".into()], "footer".into(), 7_u8);

        assert_eq!(state.title, "Confirm");
        assert_eq!(state.body_lines, vec!["body"]);
        assert_eq!(state.footer, "footer");
        assert_eq!(state.action, 7);
    }
}
