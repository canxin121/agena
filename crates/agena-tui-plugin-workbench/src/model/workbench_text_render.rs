//! Text rendering helpers for plugin workbench and schema editors.

mod config;
mod editor;
mod plugin;

pub(crate) use self::{config::*, editor::*, plugin::*};

pub mod api {
    pub use super::config::{pair_editor_labels, plugin_all_diagnostics};
}
