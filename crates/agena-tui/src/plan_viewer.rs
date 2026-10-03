//! A compact plan reader with one scroll owner and frame-local pointer targets.
use crate::{i18n::I18n, user_input::markdown_lines};
use agena_tui_components::theme::{danger_color, muted_style};
use agena_tui_components::{FramedSurfaceSpec, SurfaceMode, render_framed_surface};
use crossterm::event::KeyCode;
use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
};
use std::cell::{Cell, RefCell};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanViewerPresentation {
    scroll: Cell<u16>,
    max_scroll: Cell<u16>,
    content: RefCell<Option<(String, Vec<Line<'static>>)>>,
}

impl Default for PlanViewerPresentation {
    fn default() -> Self {
        Self {
            scroll: Cell::new(0),
            max_scroll: Cell::new(u16::MAX),
            content: RefCell::new(None),
        }
    }
}

impl PlanViewerPresentation {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn scroll(&self) -> u16 {
        self.scroll.get()
    }
    pub fn scroll_to(&mut self, scroll: u16) {
        self.scroll.set(scroll.min(self.max_scroll.get()));
    }
    pub fn scroll_by(&mut self, delta: i64) {
        self.scroll.set(
            (i64::from(self.scroll.get()) + delta).clamp(0, i64::from(self.max_scroll.get()))
                as u16,
        );
    }
}

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
    let title = format!(
        "{}{}",
        i18n.text("plan-viewer-title"),
        summary
            .filter(|s| !s.trim().is_empty())
            .map(|s| format!(" · {s}"))
            .unwrap_or_default()
    );
    let surface = render_framed_surface(
        frame,
        area,
        SurfaceMode::Route,
        &FramedSurfaceSpec {
            title: title.into(),
            target_width: area.width,
            target_height: area.height,
        },
    );
    let inner = surface.inner;
    if inner.is_empty() {
        return;
    }
    let autorun_label = match autorun {
        Some(true) => i18n.text("plan-viewer-autorun-on"),
        _ => i18n.text("plan-viewer-autorun-off"),
    };
    let refresh_label = i18n.text(if loading {
        "plan-viewer-loading"
    } else {
        "plan-viewer-refresh"
    });
    let mut buttons = vec![(
        refresh_label.as_str(),
        agena_tui_components::pointer::key(KeyCode::Char('r')),
    )];
    if autorun.is_some() {
        buttons.push((
            autorun_label.as_str(),
            agena_tui_components::pointer::key(KeyCode::Char('a')),
        ));
    }
    agena_tui_components::pointer::render_buttons(frame, Rect { height: 1, ..inner }, &buttons);
    let body = Rect {
        y: inner.y.saturating_add(1),
        height: inner.height.saturating_sub(2),
        ..inner
    };
    let footer = Rect::new(inner.x, inner.bottom().saturating_sub(1), inner.width, 1);
    agena_tui_components::render_shortcut_footer(frame, footer, &i18n.text("plan-viewer-footer"));
    if body.is_empty() {
        return;
    }
    let mut content = presentation.content.borrow_mut();
    if let Some(markdown) = markdown.filter(|s| !s.trim().is_empty()) {
        if content
            .as_ref()
            .is_none_or(|(previous, _)| previous != markdown)
        {
            *content = Some((markdown.to_owned(), markdown_lines(markdown)));
        }
    } else {
        *content = None;
    }
    let lines = if let Some(error) = error.filter(|s| !s.trim().is_empty()) {
        vec![Line::from(Span::styled(
            format!("✗ {error}"),
            Style::default().fg(danger_color()),
        ))]
    } else if let Some((_, lines)) = content.as_ref() {
        lines.clone()
    } else {
        vec![Line::from(Span::styled(
            i18n.text(if loading {
                "plan-viewer-loading"
            } else {
                "plan-viewer-empty"
            }),
            muted_style(),
        ))]
    };
    let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
    let max_scroll = paragraph
        .line_count(body.width)
        .saturating_sub(usize::from(body.height))
        .min(usize::from(u16::MAX)) as u16;
    presentation.max_scroll.set(max_scroll);
    presentation
        .scroll
        .set(presentation.scroll.get().min(max_scroll));
    frame.render_widget(paragraph.scroll((presentation.scroll.get(), 0)), body);
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

#[cfg(test)]
mod scrolling_regressions {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};
    #[test]
    fn long_wrapped_paragraph_reaches_its_tail_and_scroll_clamps_after_resize() {
        let mut terminal = Terminal::new(TestBackend::new(28, 10)).unwrap();
        let mut state = PlanViewerPresentation::new();
        let text = format!("{} END_OF_PLAN", "中文步骤 abc ".repeat(60));
        let draw = |terminal: &mut Terminal<TestBackend>, state: &PlanViewerPresentation| {
            terminal
                .draw(|frame| {
                    render_plan_viewer(
                        frame,
                        frame.area(),
                        state,
                        None,
                        Some(&text),
                        Some(false),
                        false,
                        None,
                        &I18n::english(),
                    )
                })
                .unwrap();
        };
        draw(&mut terminal, &state);
        assert!(state.max_scroll.get() > 20);
        state.scroll_by(i64::from(u16::MAX));
        draw(&mut terminal, &state);
        let rendered: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(rendered.contains("END_OF_PLAN"), "{rendered}");
        let end = state.scroll();
        state.scroll_by(1);
        assert_eq!(state.scroll(), end);
        state.scroll_by(-1);
        assert_eq!(state.scroll(), end - 1);
        terminal.backend_mut().resize(160, 40);
        terminal.resize(Rect::new(0, 0, 160, 40)).unwrap();
        draw(&mut terminal, &state);
        assert_eq!(state.scroll(), 0);
    }
}
