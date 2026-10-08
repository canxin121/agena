//! Transcript block, operation, and request rendering helpers.

mod message_render;
mod operation_render;

pub(crate) fn render_content_document(
    document: &agena_domain::ContentDocument,
    out: &mut Vec<crate::RenderedLine>,
    width: u16,
    i18n: &agena_tui::i18n::I18n,
) {
    operation_render::render_operation_blocks(
        &document.blocks,
        out,
        width,
        i18n,
        true,
        None,
        &Default::default(),
        operation_render::ContentObservation {
            frames: &Default::default(),
            errors: &Default::default(),
        },
    );
}
mod request_render;

pub use self::message_render::*;
#[cfg(test)]
pub(super) use self::operation_render::render_tool_execution;
