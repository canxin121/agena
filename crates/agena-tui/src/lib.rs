//! Terminal presentation primitives and application library.
//!
//! This crate owns TUI behavior. It deliberately contains no process-wide
//! initialization, CLI parser, HTTP transport, database connection, or
//! concrete provider implementation.

pub mod choice;
pub mod command_palette;
pub mod composer;
pub mod file_attach;
pub mod file_mentions;
pub mod help;
pub mod i18n;
pub mod input;
pub mod keymap;
pub mod link;
pub mod main_focus;
pub mod model_catalog;
pub mod model_chooser;
pub mod notice;
pub mod path_browser;
pub mod permission_prompt;
pub mod presentation_config;
pub mod prompt_history;
pub mod selection_picker;
pub mod session_status;
pub mod slash_commands;
pub mod status_line;
pub mod terminal;
pub mod terminal_capabilities;
pub mod terminal_color;
pub mod terminal_graphics;
pub mod terminal_input;
pub mod terminal_lifecycle;
pub mod terminal_overrides;
pub mod terminal_protocol;
pub mod terminal_transaction;
pub mod timeline;
pub mod usage;
pub mod user_input;

pub fn sanitize_picker_text(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                let _ = chars.next();
                for next in chars.by_ref() {
                    if ('@'..='~').contains(&next) {
                        break;
                    }
                }
            }
            continue;
        }
        match ch {
            '\r' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' => {}
            _ => out.push(ch),
        }
    }
    out
}
