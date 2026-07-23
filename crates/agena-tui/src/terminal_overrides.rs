use crate::terminal::TerminalEnvironment;

pub fn boolean(
    environment: &TerminalEnvironment,
    key: &'static str,
    diagnostics: &mut Vec<String>,
) -> Option<bool> {
    let value = environment.get(key)?.trim();
    match value.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" | "enable" | "enabled" => Some(true),
        "0" | "false" | "no" | "off" | "disable" | "disabled" => Some(false),
        _ => {
            diagnostics.push(format!(
                "invalid {key} value `{value}`; expected true/on/1 or false/off/0"
            ));
            None
        }
    }
}

pub fn keyboard_protocol(
    environment: &TerminalEnvironment,
    diagnostics: &mut Vec<String>,
) -> Option<bool> {
    let key = "AGENA_TUI_KEYBOARD_PROTOCOL";
    let value = environment.get(key)?.trim();
    match value.to_ascii_lowercase().as_str() {
        "kitty" | "csi-u" | "csiu" => Some(true),
        "legacy" | "off" | "0" | "false" => Some(false),
        "auto" | "" => None,
        _ => {
            diagnostics.push(format!(
                "invalid {key} value `{value}`; expected kitty/csi-u or legacy/off"
            ));
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{boolean, keyboard_protocol};
    use crate::terminal::TerminalEnvironment;

    #[test]
    fn boolean_accepts_documented_forms_and_records_invalid_values() {
        let environment = TerminalEnvironment::from_pairs(&[
            ("TRUE", "enable"),
            ("FALSE", "off"),
            ("INVALID", "perhaps"),
        ]);
        let mut diagnostics = Vec::new();
        assert_eq!(boolean(&environment, "TRUE", &mut diagnostics), Some(true));
        assert_eq!(
            boolean(&environment, "FALSE", &mut diagnostics),
            Some(false)
        );
        assert_eq!(boolean(&environment, "INVALID", &mut diagnostics), None);
        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0].contains("INVALID"));
    }

    #[test]
    fn keyboard_protocol_accepts_explicit_modes_and_leaves_auto_unset() {
        let environment =
            TerminalEnvironment::from_pairs(&[("AGENA_TUI_KEYBOARD_PROTOCOL", "kitty")]);
        assert_eq!(keyboard_protocol(&environment, &mut Vec::new()), Some(true));

        let environment =
            TerminalEnvironment::from_pairs(&[("AGENA_TUI_KEYBOARD_PROTOCOL", "legacy")]);
        assert_eq!(
            keyboard_protocol(&environment, &mut Vec::new()),
            Some(false)
        );

        let environment =
            TerminalEnvironment::from_pairs(&[("AGENA_TUI_KEYBOARD_PROTOCOL", "auto")]);
        assert_eq!(keyboard_protocol(&environment, &mut Vec::new()), None);
    }
}
