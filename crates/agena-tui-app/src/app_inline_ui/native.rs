//! Live controls in the actual transcript, using the same bounded program and
//! state as the focused UI view. No alternate business logic or mock side effects.
use super::*;
use agena_tui_transcript::inline_ui::{InlineControl, InlineEvent, InlineReply};
use agena_tui_transcript::{
    RenderedLine, RenderedTranscriptControl, TranscriptControlAction, TranscriptTextPosition,
};
use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

#[derive(Debug, Clone)]
pub(crate) struct InlinePointer {
    target: InlineControl,
    columns: std::ops::Range<usize>,
}
#[derive(Debug, Clone, Default)]
pub(crate) struct NativeLayout {
    pub body: Rect,
    pub rows: Vec<RenderedLine>,
    pub scroll: u16,
}
#[derive(Default)]
struct Effect {
    changed: bool,
    copy: Option<String>,
    edit: Option<String>,
    picker: bool,
    error: Option<String>,
}

fn contains(area: Rect, column: u16, row: u16) -> bool {
    column >= area.x && column < area.right() && row >= area.y && row < area.bottom()
}

mod actions;
mod editor;
mod pointer;
mod surface;
mod transcript;
use actions::{activate, resolve, updated_target};
use editor::apply_modal_effect;
pub(super) use editor::handle_modal_key;
pub(crate) use editor::paste_editor;
pub(super) use surface::render_modal;
use surface::{focus_target, rendered_controls};
