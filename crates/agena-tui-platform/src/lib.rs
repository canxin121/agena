//! # agena-tui-platform
//!
//! Platform integration services for the Agena terminal UI.
//!
//! Thin wrappers around OS/terminal capabilities used by the TUI: clipboard
//! access ([`clipboard`]), external editor/pager launch ([`external_editor`],
//! [`external_pager`]), terminal graphics protocols ([`kitty`], [`iterm2`]),
//! terminal transfer ([`terminal_transfer`]), attachment sources
//! ([`attachment_source`]), and terminal queries ([`terminal`]). Failures are
//! reported through [`ProviderError`].

pub mod attachment_source;
pub mod clipboard;
pub mod external_editor;
pub mod external_pager;
mod helper_runner;
pub mod iterm2;
pub mod kitty;
mod provider_error;
pub mod terminal;
pub mod terminal_transfer;

pub use provider_error::ProviderError;
