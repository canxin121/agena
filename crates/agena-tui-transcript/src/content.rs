//! Incremental, bounded terminal observations owned entirely by the TUI.
//! Escape sequences are interpreted into cells, never sent to the outer tty.

use crate::RenderedLine;
use agena_domain::{ContentCursor, ContentPage, ContentPayload, ContentResource, ContentState};
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct ContentFrame {
    pub resource: ContentResource,
    pub lines: Vec<Line<'static>>,
    pub wrapped: Vec<bool>,
    pub terminal_size: Option<(u16, u16)>,
    pub gap: bool,
    pub windowed: bool,
    pub text: Arc<str>,
    pub document: Option<Arc<agena_domain::ContentDocument>>,
}

pub struct ContentView {
    pub cursor: Option<ContentCursor>,
    pub frame: Option<Arc<ContentFrame>>,
    parser: vt100::Parser,
    gap: bool,
    windowed: bool,
    recovering: bool,
    text: String,
    terminal_frame: Option<(ContentCursor, u16, u16, bool)>,
    document: Option<agena_domain::ContentDocument>,
    document_cursor: Option<ContentCursor>,
}

impl Default for ContentView {
    fn default() -> Self {
        Self {
            cursor: None,
            frame: None,
            parser: vt100::Parser::new(16, 240, 2048),
            gap: false,
            windowed: false,
            recovering: false,
            text: String::new(),
            terminal_frame: None,
            document: None,
            document_cursor: None,
        }
    }
}

impl ContentView {
    /// Return false on an unexplained gap so the reader resumes from the
    /// last accepted cursor. Duplicate frames never append output twice.
    pub fn apply(&mut self, page: ContentPage) -> bool {
        let reset = self.recovering
            || self
                .cursor
                .is_some_and(|cursor| cursor.epoch != page.next_cursor.epoch);
        if page.resource.cursor.epoch != page.next_cursor.epoch
            || page.resource.committed_cursor.epoch != page.next_cursor.epoch
            || page.next_cursor.sequence > page.resource.cursor.sequence
            || page.resource.committed_cursor.sequence > page.resource.cursor.sequence
            || self
                .frame
                .as_ref()
                .is_some_and(|frame| frame.resource.resource_id != page.resource.resource_id)
            || page
                .chunks
                .windows(2)
                .any(|chunks| chunks[0].cursor.sequence >= chunks[1].cursor.sequence)
        {
            return false;
        }
        let mut expected = if reset {
            None
        } else {
            self.cursor.map(|cursor| cursor.sequence)
        };
        let mut terminal_frame = if reset { None } else { self.terminal_frame };
        let mut document = if reset { None } else { self.document.clone() };
        let mut document_cursor = if reset { None } else { self.document_cursor };
        for chunk in &page.chunks {
            if chunk.cursor.epoch != page.next_cursor.epoch
                || chunk.cursor.sequence > page.next_cursor.sequence
                || !page.resource.kind.accepts(chunk.payload.kind())
                || chunk.payload.validate().is_err()
            {
                return false;
            }
            if expected.is_some_and(|sequence| chunk.cursor.sequence <= sequence) {
                continue;
            }
            if expected.is_some_and(|sequence| chunk.cursor.sequence != sequence + 1) && !page.gap {
                return false;
            }
            expected = Some(chunk.cursor.sequence);
            match &chunk.payload {
                ContentPayload::StructuredSnapshot { document: snapshot } => {
                    if snapshot.validate().is_err() {
                        return false;
                    }
                    document = Some(snapshot.clone());
                    document_cursor = Some(chunk.cursor);
                }
                ContentPayload::Structured { base_cursor, event } => {
                    let Some(next) = document
                        .as_ref()
                        .filter(|_| document_cursor == Some(*base_cursor))
                        .and_then(|document| document.updated(event).ok())
                    else {
                        self.cursor = None;
                        self.recovering = true;
                        return false;
                    };
                    document = Some(next);
                    document_cursor = Some(chunk.cursor);
                }
                ContentPayload::Terminal { screen } => {
                    terminal_frame = Some((
                        chunk.cursor,
                        screen.rows,
                        screen.cols,
                        screen.alternate_screen,
                    ))
                }
                ContentPayload::TerminalPatch {
                    base_cursor,
                    screen,
                    ..
                } => {
                    if terminal_frame
                        != Some((
                            *base_cursor,
                            screen.rows,
                            screen.cols,
                            screen.alternate_screen,
                        ))
                    {
                        self.cursor = None;
                        self.recovering = true;
                        return false;
                    }
                    terminal_frame = Some((
                        chunk.cursor,
                        screen.rows,
                        screen.cols,
                        screen.alternate_screen,
                    ));
                }
                _ => {}
            }
        }
        if reset {
            *self = Self::default();
        }
        if self.cursor.is_none()
            && page
                .chunks
                .first()
                .is_some_and(|chunk| chunk.cursor.sequence > 1)
            && page.resource.kind != agena_domain::ContentKind::Structured
        {
            self.windowed = true;
        }
        for chunk in &page.chunks {
            if self
                .cursor
                .is_some_and(|cursor| chunk.cursor.sequence <= cursor.sequence)
            {
                continue;
            }
            match &chunk.payload {
                ContentPayload::Text { text } => {
                    self.text.push_str(text);
                    if page.resource.kind != agena_domain::ContentKind::Text {
                        self.parser.process(text.replace('\n', "\r\n").as_bytes());
                    }
                }
                ContentPayload::Log { text, stream } => {
                    let stderr = *stream == agena_domain::CommandOutputStream::Stderr;
                    if stderr {
                        self.parser.process(b"\x1b[31m");
                    }
                    if page.resource.kind == agena_domain::ContentKind::Terminal {
                        self.parser.process(text.as_bytes());
                    } else {
                        self.parser.process(text.replace('\n', "\r\n").as_bytes());
                    }
                    if stderr {
                        self.parser.process(b"\x1b[0m");
                    }
                }
                ContentPayload::Structured { .. } | ContentPayload::StructuredSnapshot { .. } => {}
                ContentPayload::Terminal { screen } => {
                    self.parser
                        .screen_mut()
                        .set_size(screen.rows.max(1), screen.cols.max(1));
                    self.parser.process(screen.formatted().as_bytes());
                }
                ContentPayload::TerminalPatch {
                    screen,
                    rows_changed,
                    ..
                } => {
                    self.parser
                        .process(screen.formatted_patch(rows_changed).as_bytes());
                }
            }
        }
        self.gap |= page.gap || page.resource.dropped_bytes > 0;
        self.terminal_frame = terminal_frame;
        self.document = document;
        self.document_cursor = document_cursor;
        if self.text.len() > 512 * 1024 {
            let mut start = self.text.len() - 512 * 1024;
            while !self.text.is_char_boundary(start) {
                start += 1;
            }
            self.text.drain(..start);
            self.windowed = true;
        }
        if self
            .cursor
            .is_none_or(|cursor| cursor.sequence <= page.next_cursor.sequence)
        {
            self.cursor = Some(page.next_cursor);
        }
        let previous = self.frame.as_ref();
        if previous
            .is_some_and(|frame| frame.resource.cursor.sequence > page.resource.cursor.sequence)
        {
            return true;
        }
        let mut resource = page.resource;
        if let Some(previous) = previous
            && previous.resource.state != ContentState::Active
            && resource.state == ContentState::Active
            && previous.resource.cursor == resource.cursor
        {
            resource.state = previous.resource.state;
        }
        let terminal = resource.kind == agena_domain::ContentKind::Terminal;
        let (lines, wrapped, omitted) = observation_lines(self.parser.screen_mut(), !terminal);
        self.windowed |= omitted;
        self.frame = Some(Arc::new(ContentFrame {
            resource,
            lines,
            wrapped,
            terminal_size: terminal.then(|| self.parser.screen().size()),
            gap: self.gap,
            windowed: self.windowed,
            text: Arc::from(self.text.as_str()),
            document: self
                .document
                .as_ref()
                .map(|document| Arc::new(document.clone())),
        }));
        true
    }
}

fn color(color: vt100::Color) -> Color {
    match color {
        vt100::Color::Default => Color::Reset,
        vt100::Color::Idx(index) => Color::Indexed(index),
        vt100::Color::Rgb(red, green, blue) => Color::Rgb(red, green, blue),
    }
}

fn screen_lines(screen: &vt100::Screen) -> Vec<Line<'static>> {
    let (rows, cols) = screen.size();
    let mut lines = Vec::new();
    for row in 0..rows {
        let end = (0..cols)
            .rev()
            .find(|col| {
                screen.cell(row, *col).is_some_and(|cell| {
                    cell.has_contents() || cell.bgcolor() != vt100::Color::Default
                })
            })
            .map_or(0, |col| col + 1);
        let mut spans: Vec<Span<'static>> = Vec::new();
        for col in 0..end {
            let Some(cell) = screen.cell(row, col) else {
                continue;
            };
            if cell.is_wide_continuation() {
                continue;
            }
            let mut style = Style::default()
                .fg(color(cell.fgcolor()))
                .bg(color(cell.bgcolor()));
            for (enabled, modifier) in [
                (cell.bold(), Modifier::BOLD),
                (cell.dim(), Modifier::DIM),
                (cell.italic(), Modifier::ITALIC),
                (cell.underline(), Modifier::UNDERLINED),
                (cell.inverse(), Modifier::REVERSED),
            ] {
                if enabled {
                    style = style.add_modifier(modifier);
                }
            }
            let text = if cell.has_contents() {
                cell.contents()
            } else {
                " "
            };
            if let Some(last) = spans.last_mut()
                && last.style == style
            {
                last.content.to_mut().push_str(text);
            } else {
                spans.push(Span::styled(text.to_owned(), style));
            }
        }
        lines.push(Line::from(spans));
    }
    lines
}

/// Log history is a local, bounded observation window. PTY screens preserve
/// every row, including empty rows, and never turn into a wrapped transcript.
fn observation_lines(
    screen: &mut vt100::Screen,
    history: bool,
) -> (Vec<Line<'static>>, Vec<bool>, bool) {
    let rows = usize::from(screen.size().0);
    let mut lines = Vec::new();
    let mut wrapped = Vec::new();
    let mut omitted = false;
    if history {
        screen.set_scrollback(257);
        omitted = screen.scrollback() > 256;
        screen.set_scrollback(256);
        let mut offset = screen.scrollback();
        while offset > 0 {
            let take = rows.min(offset);
            lines.extend(screen_lines(screen).into_iter().take(take));
            wrapped.extend((0..take).map(|row| screen.row_wrapped(row as u16)));
            offset -= take;
            screen.set_scrollback(offset);
        }
    }
    lines.extend(screen_lines(screen));
    wrapped.extend((0..rows).map(|row| screen.row_wrapped(row as u16)));
    if history {
        let observed_rows = lines.len() - rows + usize::from(screen.cursor_position().0) + 1;
        while lines.len() > observed_rows && lines.last().is_some_and(|line| line.spans.is_empty())
        {
            lines.pop();
            wrapped.pop();
        }
    }
    (lines, wrapped, omitted)
}

pub fn render_content(
    frame: Option<&ContentFrame>,
    out: &mut Vec<RenderedLine>,
    width: u16,
    i18n: &agena_tui::i18n::I18n,
    connection_error: Option<&str>,
    format: agena_domain::ContentFormat,
) {
    let Some(frame) = frame else {
        out.push(RenderedLine::dim(format!(
            "    │ {}",
            connection_error.unwrap_or(&i18n.text("content-loading"))
        )));
        return;
    };
    let state = match frame.resource.state {
        ContentState::Active => "content-active",
        ContentState::Complete => "content-complete",
        ContentState::Interrupted => "content-interrupted",
    };
    let geometry = frame
        .terminal_size
        .map(|(rows, cols)| format!(" · {cols}×{rows}"))
        .unwrap_or_default();
    out.push(
        RenderedLine::dim(format!("    ┌ {}{geometry}", i18n.text(state)))
            .with_copy_projection("", 0),
    );
    if frame.gap {
        out.push(
            RenderedLine::dim(format!("    │ {}", i18n.text("content-gap")))
                .with_copy_projection("", 0),
        );
    }
    if frame.windowed {
        out.push(
            RenderedLine::dim(format!("    │ {}", i18n.text("content-window")))
                .with_copy_projection("", 0),
        );
    }
    if let Some(document) = &frame.document {
        crate::renderer::render_content_document(document, out, width, i18n);
    }
    if frame.resource.kind == agena_domain::ContentKind::Text {
        let body_width = width.saturating_sub(6).max(1);
        let rows = match format {
            agena_domain::ContentFormat::Diff => {
                crate::renderer::render_diff_document(&frame.text, body_width)
            }
            agena_domain::ContentFormat::Markdown => {
                crate::renderer::render_markdown_document(&frame.text, body_width)
            }
            _ => crate::sanitize_terminal_text(&frame.text)
                .split('\n')
                .flat_map(|text| {
                    crate::wrap_rich_line(
                        &[Span::raw(text.to_owned())],
                        usize::from(body_width),
                        usize::from(body_width),
                    )
                    .into_iter()
                    .map(RenderedLine::rich)
                })
                .collect(),
        };
        for mut row in rows {
            let mut spans = vec![Span::styled(
                "    │ ",
                Style::default().fg(agena_tui_components::theme::muted_color()),
            )];
            spans.extend(row.rich_line.take().map_or_else(
                || vec![Span::styled(row.text.clone(), row.style)],
                |line| line.spans,
            ));
            row.text = format!("    │ {}", row.text);
            row.copy_column += 6;
            row.rich_line = Some(Line::from(spans));
            out.push(row);
        }
    }
    let navigation_base = out.len();
    let mut group_start = 0;
    let mut group_text = String::new();
    for (index, line) in frame.lines.iter().enumerate() {
        if index == group_start {
            group_text.clear();
            for next in index..frame.lines.len() {
                for span in &frame.lines[next].spans {
                    group_text.push_str(&span.content);
                }
                if !frame.wrapped.get(next).copied().unwrap_or(false) {
                    break;
                }
            }
        }
        let available = width.saturating_sub(6).max(1) as usize;
        // PTY cells describe an actual screen. Observer width clips its rows
        // rather than changing the source's cursor and line geometry.
        let rows = if frame.resource.kind == agena_domain::ContentKind::Terminal {
            vec![clip_cells(line, available)]
        } else {
            crate::wrap_rich_line(&line.spans, available, available)
        };
        for wrapped in rows {
            let mut spans = vec![Span::styled(
                "    │ ",
                Style::default().fg(agena_tui_components::theme::muted_color()),
            )];
            let copy = wrapped
                .spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>();
            spans.extend(wrapped.spans);
            out.push(
                RenderedLine::rich(Line::from(spans))
                    .with_copy_projection(copy, 6)
                    .with_navigation_unit(navigation_base + group_start, group_text.clone()),
            );
        }
        if !frame.wrapped.get(index).copied().unwrap_or(false) {
            group_start = index + 1;
        }
    }
    if let Some(error) = connection_error.or(frame.resource.capture_error.as_deref()) {
        out.push(RenderedLine::plain(
            format!("    │ {}", agena_tui::sanitize_picker_text(error)),
            Style::default().fg(agena_tui_components::theme::danger_color()),
        ));
    }
    out.push(RenderedLine::dim("    └─").with_copy_projection("", 0));
}

fn clip_cells(line: &Line<'static>, width: usize) -> Line<'static> {
    use unicode_segmentation::UnicodeSegmentation;
    use unicode_width::UnicodeWidthStr;
    let mut used = 0usize;
    let mut spans = Vec::new();
    for span in &line.spans {
        let mut text = String::new();
        for grapheme in span.content.graphemes(true) {
            let cells = UnicodeWidthStr::width(grapheme);
            if used + cells > width {
                if !text.is_empty() {
                    spans.push(Span::styled(text, span.style));
                }
                return Line::from(spans);
            }
            text.push_str(grapheme);
            used += cells;
        }
        if !text.is_empty() {
            spans.push(Span::styled(text, span.style));
        }
    }
    Line::from(spans)
}

#[cfg(test)]
mod tests;
