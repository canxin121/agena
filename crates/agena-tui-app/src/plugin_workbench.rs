//! Application adapters for the plugin workbench.
//!
//! The schema model, validation, row materialization, and presentation text
//! live in `agena-tui-plugin-workbench`. This module only keeps the concrete
//! `App` methods that connect those neutral values to backend/runtime effects,
//! route changes, flash messages, and editor overlays.

use crossterm::event::KeyEvent;
use ratatui::{Frame, layout::Rect};
use serde_json::{Number as JsonNumber, Value as JsonValue, json};

use agena_tui_components::{
    Editor, EditorDialogKeyResult, EditorDialogState, SurfaceMode, drive_editor_dialog_key,
};

use crate::{App, UiResult, editor_save_footer};

pub(crate) use agena_tui_plugin_workbench::api::*;
pub(crate) use agena_tui_plugin_workbench::*;

mod workbench_config;
mod workbench_editor;
mod workbench_input;
mod workbench_navigation;
mod workbench_render;
