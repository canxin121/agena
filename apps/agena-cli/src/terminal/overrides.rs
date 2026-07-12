use super::identity::TerminalEnvironment;

pub(super) fn boolean(
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

pub(super) fn keyboard_protocol(
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
