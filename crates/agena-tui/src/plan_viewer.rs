//! Presentation and rendering for the plan viewer overlay.
//!
//! Fetching plan content (`plan.get` full view) and toggling autorun remain
//! application responsibilities. This module owns the terminal projection:
//! scroll state and rendering the plan markdown through the shared
//! `markdown_lines` helper.

use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};

use agena_tui_components::theme::{danger_color, muted_style};

use crate::i18n::I18n;
use crate::user_input::markdown_lines;

/// Pure presentation state for the plan viewer overlay.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlanViewerPresentation {
    scroll: u16,
}

impl PlanViewerPresentation {
    pub fn new() -> Self {
        Self::default()
    }

    /// The current scroll offset in rendered lines.
    pub fn scroll(&self) -> u16 {
        self.scroll
    }

    pub fn scroll_to(&mut self, scroll: u16) {
        self.scroll = scroll;
    }

    /// Move the scroll offset by `delta` lines, clamped at zero.
    pub fn scroll_by(&mut self, delta: i64) {
        let next = self.scroll as i64 + delta;
        self.scroll = next.clamp(0, u16::MAX as i64) as u16;
    }
}

/// Render the plan viewer overlay into `area`.
///
/// `summary` is the compact display text (for example `▶ 2/5 ↻`), `markdown`
/// is the full plan document from `plan.get`, and `autorun` drives the
/// title badge. Loading and error states replace the body while fetching.
#[allow(clippy::too_many_arguments)]
pub fn render_plan_viewer(
    frame: &mut Frame,
    area: Rect,
    presentation: &PlanViewerPresentation,
    summary: Option<&str>,
    markdown: Option<&str>,
    autorun: Option<bool>,
    loading: bool,
    error: Option<&str>,
    i18n: &I18n,
) {
    let autorun_text = match autorun {
        Some(true) => i18n.text("plan-viewer-autorun-on"),
        Some(false) => i18n.text("plan-viewer-autorun-off"),
        None => String::new(),
    };
    let title = match summary {
        Some(summary) if !summary.trim().is_empty() => {
            if autorun_text.is_empty() {
                format!(" {summary} ")
            } else {
                format!(" {summary} · {autorun_text} ")
            }
        }
        _ => {
            let base = i18n.text("plan-viewer-title");
            if autorun_text.is_empty() {
                format!(" {base} ")
            } else {
                format!(" {base} · {autorun_text} ")
            }
        }
    };
    let footer = i18n.text("plan-viewer-footer");
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(muted_style())
        .title(Line::from(Span::styled(
            title,
            Style::default().add_modifier(Modifier::BOLD),
        )))
        .title_bottom(Line::from(Span::styled(
            footer,
            Style::default().fg(agena_tui_components::theme::muted_color()),
        )));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut lines: Vec<Line<'static>> = Vec::new();
    if let Some(error) = error {
        if !error.trim().is_empty() {
            lines.push(Line::from(Span::styled(
                format!(" ✗ {error}"),
                Style::default().fg(danger_color()),
            )));
        }
    } else if loading {
        lines.push(Line::from(Span::styled(
            format!(" {}\u{2026}", i18n.text("plan-viewer-loading")),
            muted_style(),
        )));
    } else if let Some(markdown) = markdown {
        if markdown.trim().is_empty() {
            lines.push(Line::from(Span::styled(
                format!(" {}", i18n.text("plan-viewer-empty")),
                muted_style(),
            )));
        } else {
            lines = markdown_lines(markdown);
        }
    } else {
        lines.push(Line::from(Span::styled(
            format!(" {}", i18n.text("plan-viewer-empty")),
            muted_style(),
        )));
    }

    let max_scroll = lines.len().saturating_sub(inner.height as usize);
    let scroll = usize::from(presentation.scroll).min(max_scroll);
    let visible: Vec<Line<'static>> = lines
        .iter()
        .skip(scroll)
        .take(inner.height as usize)
        .cloned()
        .collect();
    frame.render_widget(Paragraph::new(visible).wrap(Wrap { trim: false }), inner);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::I18n;
    use ratatui::{Terminal, backend::TestBackend};

    #[test]
    fn presentation_scroll_clamps_at_zero() {
        let mut presentation = PlanViewerPresentation::new();
        assert_eq!(presentation.scroll(), 0);
        presentation.scroll_by(-4);
        assert_eq!(presentation.scroll(), 0);
        presentation.scroll_by(3);
        assert_eq!(presentation.scroll(), 3);
        presentation.scroll_to(9);
        assert_eq!(presentation.scroll(), 9);
    }

    fn render_to_string(
        presentation: &PlanViewerPresentation,
        summary: Option<&str>,
        markdown: Option<&str>,
        autorun: Option<bool>,
        loading: bool,
        error: Option<&str>,
        i18n: &I18n,
    ) -> String {
        let backend = TestBackend::new(100, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                render_plan_viewer(
                    frame,
                    frame.area(),
                    presentation,
                    summary,
                    markdown,
                    autorun,
                    loading,
                    error,
                    i18n,
                )
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        (0..buffer.area.height)
            .flat_map(|y| (0..buffer.area.width).map(move |x| buffer[(x, y)].symbol()))
            .collect::<String>()
    }

    #[test]
    fn render_combines_summary_autorun_and_markdown_body() {
        let i18n = I18n::english();
        let rendered = render_to_string(
            &PlanViewerPresentation::new(),
            Some("▶ 2/5 ↻"),
            Some("# Plan title\n\n- step one\n- step two\n"),
            Some(true),
            false,
            None,
            &i18n,
        );
        assert!(rendered.contains("2/5"), "{rendered}");
        assert!(rendered.contains("autorun: on"), "{rendered}");
        assert!(rendered.contains("Plan title"), "{rendered}");
        assert!(rendered.contains("step one"), "{rendered}");
        assert!(rendered.contains("r refresh"), "{rendered}");
    }

    #[test]
    fn render_shows_loading_error_and_empty_states() {
        let i18n = I18n::english();
        let loading = render_to_string(
            &PlanViewerPresentation::new(),
            None,
            None,
            None,
            true,
            None,
            &i18n,
        );
        assert!(loading.contains("Loading plan"), "{loading}");

        let error = render_to_string(
            &PlanViewerPresentation::new(),
            None,
            None,
            None,
            false,
            Some("boom"),
            &i18n,
        );
        assert!(error.contains("✗ boom"), "{error}");

        let missing = render_to_string(
            &PlanViewerPresentation::new(),
            None,
            None,
            None,
            false,
            None,
            &i18n,
        );
        assert!(missing.contains("No plan yet"), "{missing}");

        let blank = render_to_string(
            &PlanViewerPresentation::new(),
            None,
            Some("   \n  "),
            Some(false),
            false,
            None,
            &i18n,
        );
        assert!(blank.contains("No plan yet"), "{blank}");
        assert!(blank.contains("autorun: off"), "{blank}");
    }
}
