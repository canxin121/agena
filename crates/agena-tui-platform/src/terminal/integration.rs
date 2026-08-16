//! Terminal-native window titles and attention notifications.
//!
//! This module translates session presentation state into validated OSC
//! frames. It owns sanitization and per-family protocol selection but has no
//! terminal handle and performs no writes; callers emit the returned frames
//! through [`TerminalRuntime::write_protocol`](crate::terminal::TerminalRuntime).
//!
//! Selection rules are deliberately conservative:
//! * Window titles (OSC 0/2) are consumed locally by a multiplexer as the pane
//!   title rather than forwarded, so they are never path-gated and work inside
//!   tmux/screen/Zellij.
//! * Notifications use BEL as the universal baseline. A family that profiles
//!   OSC 9 upgrades to a desktop notification; iTerm2 additionally requests
//!   one-shot Dock attention instead of leaving a persistent badge.

use crate::terminal::TerminalFamily;

/// Hard upper bound on an OSC title/notification payload. Titles are truncated
/// by display width first; notifications are truncated by byte length because
/// they may carry a user summary.
const MAX_TITLE_DISPLAY_WIDTH: usize = 120;
const MAX_NOTIFICATION_TEXT_BYTES: usize = 512;

/// Neutralizes control characters in OSC payload text so a title or
/// notification cannot terminate its own frame with an embedded BEL/ESC or
/// inject a nested terminal sequence. Newlines become spaces because most
/// notification consumers render them inconsistently.
pub fn sanitize_osc_text(text: &str) -> String {
    text.chars()
        .map(|ch| {
            if ch == '\n' || ch == '\r' || ch.is_control() {
                ' '
            } else {
                ch
            }
        })
        .collect()
}

fn title_display_text(text: &str) -> String {
    let sanitized = sanitize_osc_text(text);
    let mut width = 0_usize;
    let mut out = String::new();
    for ch in sanitized.chars() {
        let ch_width = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if width.saturating_add(ch_width) > MAX_TITLE_DISPLAY_WIDTH {
            break;
        }
        out.push(ch);
        width = width.saturating_add(ch_width);
    }
    if out.is_empty() {
        "agena".to_owned()
    } else {
        out
    }
}

/// Builds the OSC frames that set the terminal window/tab title to `title`.
///
/// OSC 2 is the universally consumed title selector. iTerm2 and Apple
/// Terminal additionally honor OSC 0 (window + icon/tab title), so both frames
/// are emitted there; emitting OSC 0 elsewhere would overwrite the icon name
/// that xterm-compatible terminals expose.
pub fn title_frames(family: TerminalFamily, title: &str) -> Vec<Vec<u8>> {
    let title = title_display_text(title);
    match family {
        TerminalFamily::Iterm2 | TerminalFamily::AppleTerminal => vec![
            format!("\x1b]2;{title}\x07").into_bytes(),
            format!("\x1b]0;{title}\x07").into_bytes(),
        ],
        _ => vec![format!("\x1b]2;{title}\x07").into_bytes()],
    }
}

/// The attention/notification method selected for one terminal family.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationMethod {
    /// A single BEL (0x07). The universal attention signal.
    Bell,
    /// OSC 9 desktop notification (Windows Terminal, WezTerm).
    Osc9,
    /// OSC 9 desktop notification plus a one-shot iTerm2 Dock attention
    /// request.
    Osc9AndItermAttention,
}

/// Selects the notification method for `family`, or `None` when the terminal
/// cannot render any attention signal. BEL is the baseline for every supported
/// family; OSC 9 (and the iTerm2 Dock attention upgrade) is used for the
/// families verified to implement it.
///
/// Verified OSC 9 implementations: iTerm2, Windows Terminal, WezTerm, Ghostty,
/// foot, and Warp. Kitty uses its own OSC 99 notification protocol, so it
/// falls back to BEL here; xterm-compatible and unknown terminals are also
/// conservative BEL.
pub fn notification_method(family: TerminalFamily) -> Option<NotificationMethod> {
    match family {
        TerminalFamily::Iterm2 => Some(NotificationMethod::Osc9AndItermAttention),
        TerminalFamily::WindowsTerminal
        | TerminalFamily::WezTerm
        | TerminalFamily::Ghostty
        | TerminalFamily::Foot
        | TerminalFamily::Warp => Some(NotificationMethod::Osc9),
        TerminalFamily::Dumb | TerminalFamily::LinuxConsole => None,
        _ => Some(NotificationMethod::Bell),
    }
}

fn notification_text(text: &str) -> String {
    let mut out = sanitize_osc_text(text);
    // String::truncate panics when the byte index falls inside a multi-byte
    // char, so back off to the previous char boundary when the hard cap cuts
    // through one. At most four bytes are dropped (a UTF-8 char is ≤4 bytes).
    if out.len() > MAX_NOTIFICATION_TEXT_BYTES {
        let mut end = MAX_NOTIFICATION_TEXT_BYTES;
        while !out.is_char_boundary(end) {
            end -= 1;
        }
        out.truncate(end);
    }
    out
}

/// Builds the OSC frames that raise `text` as a terminal-native notification
/// using `method`. The returned frames are ordered so the one-shot attention
/// request follows the desktop notification.
pub fn notification_frames(method: NotificationMethod, text: &str) -> Vec<Vec<u8>> {
    let text = notification_text(text);
    match method {
        NotificationMethod::Bell => vec![b"\x07".to_vec()],
        NotificationMethod::Osc9 => vec![format!("\x1b]9;{text}\x07").into_bytes()],
        NotificationMethod::Osc9AndItermAttention => vec![
            format!("\x1b]9;{text}\x07").into_bytes(),
            b"\x1b]1337;RequestAttention=yes\x07".to_vec(),
        ],
    }
}

/// The OSC 9;4 progress-bar state projected from session activity.
///
/// The ConEmu numbering is shared by iTerm2, Windows Terminal, WezTerm,
/// Ghostty, VS Code/xterm.js, Konsole and Warp. State 3 is indeterminate:
/// the terminal animates a pulsing/indeterminate indicator itself, so a
/// single frame is enough to signal "working" without a client-side loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressState {
    /// State 0 - remove/hide the progress indicator.
    Clear,
    /// State 3 - indeterminate (pulsing/back-and-forth animation).
    Working,
    /// State 4 - paused/warning: awaiting permission or user input.
    Awaiting,
    /// State 2 - error: the run ended and needs recovery.
    Error,
}

impl ProgressState {
    const fn osc_state(self) -> u8 {
        match self {
            Self::Clear => 0,
            Self::Working => 3,
            Self::Awaiting => 4,
            Self::Error => 2,
        }
    }
}

/// Builds the OSC 9;4 frame that reports `state` to the terminal chrome.
/// Unlike titles, this sequence is only safe when the endpoint is verified
/// to support progress; unsupported terminals may interpret `OSC 9;4;*` as
/// an OSC 9 notification.
pub fn progress_frames(state: ProgressState) -> Vec<Vec<u8>> {
    vec![format!("\x1b]9;4;{}\x07", state.osc_state()).into_bytes()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_frames_use_osc2_everywhere_and_osc0_for_apple_like_terminals() {
        assert_eq!(
            title_frames(TerminalFamily::Iterm2, "fix login"),
            vec![
                b"\x1b]2;fix login\x07".to_vec(),
                b"\x1b]0;fix login\x07".to_vec(),
            ]
        );
        assert_eq!(
            title_frames(TerminalFamily::WezTerm, "build"),
            vec![b"\x1b]2;build\x07".to_vec()]
        );
        assert_eq!(
            title_frames(TerminalFamily::Unknown, "build"),
            vec![b"\x1b]2;build\x07".to_vec()]
        );
    }

    #[test]
    fn sanitize_neutralizes_control_characters_that_would_terminate_the_frame() {
        // BEL and ESC become spaces so no embedded sequence can end or start
        // a frame; their trailing payload text stays as inert literal text.
        assert_eq!(sanitize_osc_text("a\x07b\x1b]2;c"), "a b ]2;c");
        assert_eq!(sanitize_osc_text("line1\nline2"), "line1 line2");
    }

    #[test]
    fn empty_title_falls_back_to_the_product_name() {
        assert_eq!(
            title_frames(TerminalFamily::Kitty, ""),
            vec![b"\x1b]2;agena\x07".to_vec()]
        );
    }

    #[test]
    fn titles_are_truncated_by_display_width() {
        let wide = "中".repeat(200);
        let frames = title_frames(TerminalFamily::Kitty, &wide);
        let frame = String::from_utf8(frames[0].clone()).unwrap();
        let body = frame
            .strip_prefix("\x1b]2;")
            .and_then(|value| value.strip_suffix('\x07'))
            .unwrap();
        let width: usize = body
            .chars()
            .map(|ch| unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0))
            .sum();
        assert!(width <= MAX_TITLE_DISPLAY_WIDTH);
    }

    #[test]
    fn notification_method_selection_is_conservative() {
        assert_eq!(
            notification_method(TerminalFamily::Iterm2),
            Some(NotificationMethod::Osc9AndItermAttention)
        );
        for osc9_family in [
            TerminalFamily::WindowsTerminal,
            TerminalFamily::WezTerm,
            TerminalFamily::Ghostty,
            TerminalFamily::Foot,
            TerminalFamily::Warp,
        ] {
            assert_eq!(
                notification_method(osc9_family),
                Some(NotificationMethod::Osc9)
            );
        }
        for bel_family in [
            TerminalFamily::Kitty,
            TerminalFamily::VsCode,
            TerminalFamily::AppleTerminal,
            TerminalFamily::Alacritty,
            TerminalFamily::Vte,
            TerminalFamily::Konsole,
            TerminalFamily::Rio,
            TerminalFamily::Contour,
            TerminalFamily::XtermCompatible,
            TerminalFamily::Unknown,
        ] {
            assert_eq!(
                notification_method(bel_family),
                Some(NotificationMethod::Bell)
            );
        }
        assert_eq!(notification_method(TerminalFamily::Dumb), None);
        assert_eq!(notification_method(TerminalFamily::LinuxConsole), None);
    }

    #[test]
    fn notification_frames_emit_the_selected_method() {
        assert_eq!(
            notification_frames(NotificationMethod::Bell, "done"),
            vec![b"\x07".to_vec()]
        );
        assert_eq!(
            notification_frames(NotificationMethod::Osc9, "done"),
            vec![b"\x1b]9;done\x07".to_vec()]
        );
        assert_eq!(
            notification_frames(NotificationMethod::Osc9AndItermAttention, "done"),
            vec![
                b"\x1b]9;done\x07".to_vec(),
                b"\x1b]1337;RequestAttention=yes\x07".to_vec(),
            ]
        );
    }

    #[test]
    fn notification_text_capped_at_byte_limit_never_splits_a_multibyte_char() {
        // A cap landing inside a multi-byte char used to make String::truncate
        // panic; the text must be cut at the nearest char boundary instead.
        let wide = "中".repeat(300);
        let out = notification_text(&wide);
        assert!(out.len() <= MAX_NOTIFICATION_TEXT_BYTES);
        assert!(out.is_char_boundary(out.len()));
        assert_eq!(out.chars().next(), Some('中'));

        // Byte-length cap may still exceed the char boundary requirement.
        let mixed = format!("{}a{}b", "中".repeat(200), "文".repeat(200));
        let out = notification_text(&mixed);
        assert!(out.len() <= MAX_NOTIFICATION_TEXT_BYTES);
        assert!(out.is_char_boundary(out.len()));
    }

    #[test]
    fn short_notification_text_is_untouched() {
        assert_eq!(notification_text("done"), "done");
        assert_eq!(notification_text("中"), "中");
    }

    #[test]
    fn progress_frames_use_the_conemu_state_numbering() {
        assert_eq!(
            progress_frames(ProgressState::Clear),
            vec![b"\x1b]9;4;0\x07".to_vec()]
        );
        assert_eq!(
            progress_frames(ProgressState::Working),
            vec![b"\x1b]9;4;3\x07".to_vec()]
        );
        assert_eq!(
            progress_frames(ProgressState::Awaiting),
            vec![b"\x1b]9;4;4\x07".to_vec()]
        );
        assert_eq!(
            progress_frames(ProgressState::Error),
            vec![b"\x1b]9;4;2\x07".to_vec()]
        );
    }
}
