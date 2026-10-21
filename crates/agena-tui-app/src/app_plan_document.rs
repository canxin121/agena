//! Plan document rendering shared with the plan viewer route.
//!
//! The composer carries only a plan *progress* chip; the document itself is a
//! full-screen surface (`Route::PlanViewer`).

use agena_tui_transcript::render_markdown_document;
use ratatui::text::{Line, Span};

/// Strip the transport prefix (`Revision: …`) from a `plan.get` tool result.
pub(crate) fn plan_document(output: &str) -> String {
    if output.starts_with("Revision:") {
        output
            .split_once('\n')
            .map_or("", |(_, body)| body)
            .trim()
            .to_owned()
    } else {
        output.trim().to_owned()
    }
}

/// Render plan markdown with the transcript markdown renderer.
pub(crate) fn render_plan_document(markdown: &str, width: u16) -> Vec<Line<'static>> {
    render_markdown_document(markdown, width)
        .into_iter()
        .map(|line| {
            line.rich_line
                .unwrap_or_else(|| Line::from(Span::styled(line.text, line.style)))
        })
        .collect()
}
