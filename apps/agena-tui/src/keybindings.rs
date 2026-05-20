//! Configurable composer key bindings.
//!
//! Two distinct submit actions:
//!
//! * `submit_key`   — fire the message immediately. While the AI is busy
//!   this routes through `steer_input` (Phase 3); when
//!   idle it submits a normal turn.
//! * `queue_key`    — append to the local pending queue. While the AI is
//!   busy, the queued message is held until the current
//!   turn ends. While idle, behaves like `submit_key`.
//! * `newline_key`  — insert a literal newline.
//! * `edit_queue_key` — pull the queue back into the editor for edit.
//!
//! The defaults follow the user's stated preference (Enter = queue,
//! Ctrl+Enter = submit) but every binding can be overridden via TOML
//! (`[tui.keybindings.composer]`).

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyChord {
    pub code: KeyCode,
    pub modifiers: KeyModifiers,
}

impl KeyChord {
    pub const fn new(code: KeyCode, modifiers: KeyModifiers) -> Self {
        Self { code, modifiers }
    }

    pub fn matches(&self, event: &KeyEvent) -> bool {
        // We only care about the modifiers we've explicitly listed; ignore
        // KEYPAD/REPEAT bits and similar so that terminals that pass extra
        // flags still match.
        let want =
            self.modifiers & (KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SHIFT);
        let got =
            event.modifiers & (KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SHIFT);
        event.code == self.code && want == got
    }
}

#[derive(Debug, Clone)]
pub struct ComposerKeyBindings {
    pub submit: Vec<KeyChord>,
    pub queue: Vec<KeyChord>,
    pub newline: Vec<KeyChord>,
    pub edit_queue: Vec<KeyChord>,
    pub history_search: Vec<KeyChord>,
    pub focus_items: Vec<KeyChord>,
    pub attach_file: Vec<KeyChord>,
    pub external_editor: Vec<KeyChord>,
    pub attach_clipboard_image: Vec<KeyChord>,
    pub open_pending_user_input: Vec<KeyChord>,
    pub open_pending_permission: Vec<KeyChord>,
}

impl Default for ComposerKeyBindings {
    fn default() -> Self {
        Self {
            // Per user request: Ctrl+Enter sends immediately.
            submit: vec![KeyChord::new(KeyCode::Enter, KeyModifiers::CONTROL)],
            // Per user request: Enter queues (or sends immediately when idle).
            queue: vec![KeyChord::new(KeyCode::Enter, KeyModifiers::empty())],
            newline: vec![
                KeyChord::new(KeyCode::Enter, KeyModifiers::SHIFT),
                KeyChord::new(KeyCode::Enter, KeyModifiers::ALT),
                KeyChord::new(KeyCode::Char('j'), KeyModifiers::CONTROL),
            ],
            edit_queue: vec![KeyChord::new(KeyCode::Up, KeyModifiers::empty())],
            history_search: vec![KeyChord::new(KeyCode::Char('r'), KeyModifiers::CONTROL)],
            focus_items: vec![KeyChord::new(KeyCode::Tab, KeyModifiers::empty())],
            attach_file: vec![
                KeyChord::new(KeyCode::F(3), KeyModifiers::empty()),
                KeyChord::new(KeyCode::Char('o'), KeyModifiers::CONTROL),
                KeyChord::new(KeyCode::Char('o'), KeyModifiers::ALT),
            ],
            external_editor: vec![
                KeyChord::new(KeyCode::F(4), KeyModifiers::empty()),
                KeyChord::new(KeyCode::Char('e'), KeyModifiers::ALT),
            ],
            attach_clipboard_image: vec![
                KeyChord::new(KeyCode::F(6), KeyModifiers::empty()),
                KeyChord::new(KeyCode::Char('i'), KeyModifiers::ALT),
            ],
            open_pending_user_input: vec![KeyChord::new(KeyCode::Char('u'), KeyModifiers::ALT)],
            open_pending_permission: vec![KeyChord::new(KeyCode::Char('p'), KeyModifiers::ALT)],
        }
    }
}

impl ComposerKeyBindings {
    pub fn from_raw(raw: &RawComposerKeyBindings) -> Result<Self, String> {
        let defaults = Self::default();
        Ok(Self {
            submit: parse_list(&raw.submit, &defaults.submit)?,
            queue: parse_list(&raw.queue, &defaults.queue)?,
            newline: parse_list(&raw.newline, &defaults.newline)?,
            edit_queue: parse_list(&raw.edit_queue, &defaults.edit_queue)?,
            history_search: parse_list(&raw.history_search, &defaults.history_search)?,
            focus_items: parse_list(&raw.focus_items, &defaults.focus_items)?,
            attach_file: parse_list(&raw.attach_file, &defaults.attach_file)?,
            external_editor: parse_list(&raw.external_editor, &defaults.external_editor)?,
            attach_clipboard_image: parse_list(
                &raw.attach_clipboard_image,
                &defaults.attach_clipboard_image,
            )?,
            open_pending_user_input: parse_list(
                &raw.open_pending_user_input,
                &defaults.open_pending_user_input,
            )?,
            open_pending_permission: parse_list(
                &raw.open_pending_permission,
                &defaults.open_pending_permission,
            )?,
        })
    }

    pub fn match_action(&self, event: &KeyEvent) -> Option<ComposerAction> {
        // Order matters: more specific (with modifiers) wins. We list submit
        // first so Ctrl+Enter is detected before bare Enter.
        if self.submit.iter().any(|c| c.matches(event)) {
            return Some(ComposerAction::Submit);
        }
        if self.newline.iter().any(|c| c.matches(event)) {
            return Some(ComposerAction::Newline);
        }
        if self.history_search.iter().any(|c| c.matches(event)) {
            return Some(ComposerAction::HistorySearch);
        }
        if self
            .open_pending_user_input
            .iter()
            .any(|c| c.matches(event))
        {
            return Some(ComposerAction::OpenPendingUserInput);
        }
        if self
            .open_pending_permission
            .iter()
            .any(|c| c.matches(event))
        {
            return Some(ComposerAction::OpenPendingPermission);
        }
        if self.attach_file.iter().any(|c| c.matches(event)) {
            return Some(ComposerAction::AttachFile);
        }
        if self.external_editor.iter().any(|c| c.matches(event)) {
            return Some(ComposerAction::ExternalEditor);
        }
        if self.attach_clipboard_image.iter().any(|c| c.matches(event)) {
            return Some(ComposerAction::AttachClipboardImage);
        }
        if self.focus_items.iter().any(|c| c.matches(event)) {
            return Some(ComposerAction::FocusItems);
        }
        if self.queue.iter().any(|c| c.matches(event)) {
            return Some(ComposerAction::Queue);
        }
        if self.edit_queue.iter().any(|c| c.matches(event)) {
            return Some(ComposerAction::EditQueue);
        }
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComposerAction {
    Submit,
    Queue,
    Newline,
    EditQueue,
    HistorySearch,
    FocusItems,
    AttachFile,
    ExternalEditor,
    AttachClipboardImage,
    OpenPendingUserInput,
    OpenPendingPermission,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct RawComposerKeyBindings {
    pub submit: Vec<String>,
    pub queue: Vec<String>,
    pub newline: Vec<String>,
    pub edit_queue: Vec<String>,
    pub history_search: Vec<String>,
    pub focus_items: Vec<String>,
    pub attach_file: Vec<String>,
    pub external_editor: Vec<String>,
    pub attach_clipboard_image: Vec<String>,
    pub open_pending_user_input: Vec<String>,
    pub open_pending_permission: Vec<String>,
}

fn parse_list(raw: &[String], default: &[KeyChord]) -> Result<Vec<KeyChord>, String> {
    if raw.is_empty() {
        return Ok(default.to_vec());
    }
    raw.iter().map(|s| parse_chord(s)).collect()
}

/// Parse strings like `"ctrl+enter"`, `"shift+enter"`, `"alt+i"`, `"up"`,
/// `"f3"`, `"esc"`. Case-insensitive.
pub fn parse_chord(s: &str) -> Result<KeyChord, String> {
    let s = s.trim().to_lowercase();
    if s.is_empty() {
        return Err("empty key chord".into());
    }
    let mut modifiers = KeyModifiers::empty();
    let parts: Vec<&str> = s.split('+').map(str::trim).collect();
    let (mods, key) = parts.split_at(parts.len().saturating_sub(1));
    for m in mods {
        match *m {
            "ctrl" | "control" => modifiers |= KeyModifiers::CONTROL,
            "alt" | "meta" | "option" => modifiers |= KeyModifiers::ALT,
            "shift" => modifiers |= KeyModifiers::SHIFT,
            other => return Err(format!("unknown modifier: {other}")),
        }
    }
    let key = key.first().copied().unwrap_or("");
    let code = match key {
        "enter" | "return" => KeyCode::Enter,
        "tab" => KeyCode::Tab,
        "backtab" => KeyCode::BackTab,
        "esc" | "escape" => KeyCode::Esc,
        "space" => KeyCode::Char(' '),
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "pageup" => KeyCode::PageUp,
        "pagedown" => KeyCode::PageDown,
        "backspace" => KeyCode::Backspace,
        "delete" | "del" => KeyCode::Delete,
        "insert" | "ins" => KeyCode::Insert,
        s if s.starts_with('f') && s.len() > 1 => {
            let n: u8 = s[1..]
                .parse()
                .map_err(|_| format!("bad function key: {s}"))?;
            KeyCode::F(n)
        }
        s if s.chars().count() == 1 => {
            let c = s.chars().next().unwrap();
            KeyCode::Char(c)
        }
        other => return Err(format!("unknown key: {other}")),
    };
    Ok(KeyChord { code, modifiers })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_distinguish_submit_and_queue() {
        let kb = ComposerKeyBindings::default();
        let enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::empty());
        let ctrl_enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::CONTROL);
        let shift_enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT);
        assert_eq!(kb.match_action(&enter), Some(ComposerAction::Queue));
        assert_eq!(kb.match_action(&ctrl_enter), Some(ComposerAction::Submit));
        assert_eq!(kb.match_action(&shift_enter), Some(ComposerAction::Newline));
    }

    #[test]
    fn parse_chord_recognises_common_forms() {
        assert_eq!(
            parse_chord("ctrl+enter").unwrap(),
            KeyChord::new(KeyCode::Enter, KeyModifiers::CONTROL)
        );
        assert_eq!(
            parse_chord("up").unwrap(),
            KeyChord::new(KeyCode::Up, KeyModifiers::empty())
        );
        assert_eq!(
            parse_chord("F3").unwrap(),
            KeyChord::new(KeyCode::F(3), KeyModifiers::empty())
        );
        assert_eq!(
            parse_chord("Alt+I").unwrap(),
            KeyChord::new(KeyCode::Char('i'), KeyModifiers::ALT)
        );
    }

    #[test]
    fn override_replaces_defaults() {
        let raw = RawComposerKeyBindings {
            submit: vec!["enter".into()],
            queue: vec!["alt+enter".into()],
            ..Default::default()
        };
        let kb = ComposerKeyBindings::from_raw(&raw).unwrap();
        let enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::empty());
        assert_eq!(kb.match_action(&enter), Some(ComposerAction::Submit));
    }

    #[test]
    fn defaults_include_editor_workflow_actions() {
        let kb = ComposerKeyBindings::default();
        assert_eq!(
            kb.match_action(&KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL,)),
            Some(ComposerAction::HistorySearch)
        );
        assert_eq!(
            kb.match_action(&KeyEvent::new(KeyCode::Tab, KeyModifiers::empty())),
            Some(ComposerAction::FocusItems)
        );
        assert_eq!(
            kb.match_action(&KeyEvent::new(KeyCode::F(3), KeyModifiers::empty())),
            Some(ComposerAction::AttachFile)
        );
        assert_eq!(
            kb.match_action(&KeyEvent::new(KeyCode::Char('u'), KeyModifiers::ALT,)),
            Some(ComposerAction::OpenPendingUserInput)
        );
    }
}
