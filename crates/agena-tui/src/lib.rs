//! # agena-tui
//!
//! Terminal presentation primitives and application library.
//!
//! This crate owns TUI behavior. It deliberately contains no process-wide
//! initialization, CLI parser, HTTP transport, database connection, or
//! concrete provider implementation.
//!
//! ## Areas
//!
//! - **Composer & input** — [`composer`], [`input`], [`file_attach`],
//!   [`file_mentions`], [`command_palette`], [`prompt_history`].
//! - **Transcript & timeline** — [`timeline`], [`plan_viewer`],
//!   [`activities`], [`session_status`].
//! - **Permissions & choices** — [`permission_prompt`], [`choice`],
//!   [`selection_picker`], [`user_input`], [`model_chooser`].
//! - **Terminal** — [`terminal`], [`terminal_capabilities`],
//!   [`terminal_color`], [`terminal_graphics`], [`terminal_input`],
//!   [`terminal_lifecycle`], [`terminal_protocol`], [`terminal_transaction`].
//! - **Chrome** — [`status_line`], [`help`], [`keymap`], [`slash_commands`],
//!   [`usage`], [`i18n`], [`main_focus`].

pub mod activities;
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
pub mod path_browser;
pub mod permission_prompt;
pub mod plan_viewer;
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

pub use agena_tui_components::text::sanitize_terminal_text as sanitize_picker_text;
