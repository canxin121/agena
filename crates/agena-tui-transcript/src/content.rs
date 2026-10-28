//! Incremental, bounded terminal observations owned entirely by the TUI.
//! Escape sequences are interpreted into cells, never sent to the outer tty.

use crate::RenderedLine;
use crate::layout::{BODY_PREFIX, CONTENT_GUTTER, CONTENT_PREFIX};
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
    inline_ui: Option<agena_domain::InlineUiReferences>,
    pub cursor: Option<ContentCursor>,
    pub frame: Option<Arc<ContentFrame>>,
    parser: vt100::Parser,
    gap: bool,
    windowed: bool,
    recovering: bool,
    retired_epochs: Vec<ContentCursor>,
    text: String,
    terminal_frame: Option<(ContentCursor, u16, u16, bool)>,
    document: Option<agena_domain::ContentDocument>,
    document_cursor: Option<ContentCursor>,
}

impl Default for ContentView {
    fn default() -> Self {
        Self {
            inline_ui: None,
            cursor: None,
            frame: None,
            parser: vt100::Parser::new(16, 240, 2048),
            gap: false,
            windowed: false,
            recovering: false,
            retired_epochs: Vec::new(),
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
        if self
            .retired_epochs
            .iter()
            .any(|cursor| cursor.epoch == page.next_cursor.epoch)
        {
            return true;
        }
        let previous_epoch = self.frame.as_ref().map(|frame| frame.resource.cursor.epoch);
        let reset = self.recovering
            || self
                .frame
                .as_ref()
                .is_some_and(|frame| frame.resource.cursor.epoch != page.next_cursor.epoch);
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
            let start = if page.resource.kind == agena_domain::ContentKind::Text {
                let ContentPayload::Text { text } = &chunk.payload else {
                    return false;
                };
                let Some(start) = chunk.cursor.sequence.checked_sub(text.len() as u64) else {
                    return false;
                };
                if let Some(sequence) = expected.filter(|sequence| *sequence > start)
                    && !text.is_char_boundary((sequence - start) as usize)
                {
                    return false;
                }
                start
            } else {
                chunk.cursor.sequence.saturating_sub(1)
            };
            if expected.is_some_and(|sequence| start > sequence) && !page.gap {
                return false;
            }
            expected = Some(chunk.cursor.sequence);
            match &chunk.payload {
                ContentPayload::DocumentEvent { input, checkpoint } => {
                    if let Some(checkpoint) = checkpoint {
                        document = Some((**checkpoint).clone());
                    } else if let agena_domain::ContentInput::Structured { event } = &**input {
                        let Some(next) = document
                            .as_ref()
                            .filter(|_| {
                                document_cursor.is_some_and(|cursor| {
                                    cursor.epoch == chunk.cursor.epoch
                                        && cursor.sequence + 1 == chunk.cursor.sequence
                                })
                            })
                            .and_then(|document| document.updated(event).ok())
                        else {
                            self.cursor = None;
                            self.recovering = true;
                            return false;
                        };
                        document = Some(next);
                    } else if document.is_none() {
                        document = Some(agena_domain::ContentDocument::default());
                    }
                    document_cursor = Some(chunk.cursor);
                }
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
            let mut retired = std::mem::take(&mut self.retired_epochs);
            if let Some(epoch) = previous_epoch.filter(|epoch| *epoch != page.next_cursor.epoch) {
                retired.push(ContentCursor { epoch, sequence: 0 });
                if retired.len() > 16 {
                    retired.remove(0);
                }
            }
            *self = Self::default();
            self.retired_epochs = retired;
        }
        if let Some(refs) = &page.inline_ui {
            let known = self.inline_ui.get_or_insert_with(Default::default);
            for (reference, document) in refs {
                if known.len() >= agena_domain::INLINE_UI_MAX_REFERENCES
                    && !known.contains_key(reference)
                {
                    break;
                }
                if agena_domain::inline_reference_tool_id(reference).is_some()
                    && document.validate().is_ok()
                {
                    known.insert(reference.clone(), document.clone());
                }
            }
        }
        if self.cursor.is_none()
            && page.chunks.first().is_some_and(|chunk| {
                if let ContentPayload::Text { text } = &chunk.payload
                    && page.resource.kind == agena_domain::ContentKind::Text
                {
                    chunk.cursor.sequence > text.len() as u64
                } else {
                    chunk.cursor.sequence > 1
                }
            })
            && page.resource.kind != agena_domain::ContentKind::Structured
        {
            self.windowed = true;
        }
        let mut applied_sequence = self.cursor.map(|cursor| cursor.sequence);
        for chunk in &page.chunks {
            if applied_sequence.is_some_and(|sequence| chunk.cursor.sequence <= sequence) {
                continue;
            }
            match &chunk.payload {
                ContentPayload::Text { text } => {
                    let text = if page.resource.kind == agena_domain::ContentKind::Text {
                        let start = chunk.cursor.sequence - text.len() as u64;
                        let skip = applied_sequence.unwrap_or(start).saturating_sub(start) as usize;
                        &text[skip..]
                    } else {
                        text.as_str()
                    };
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
                ContentPayload::DocumentEvent { input, .. } => match &**input {
                    agena_domain::ContentInput::Text { text } => {
                        self.text.push_str(text);
                        self.parser.process(text.replace('\n', "\r\n").as_bytes());
                    }
                    agena_domain::ContentInput::Log { text, stream } => {
                        if *stream == agena_domain::CommandOutputStream::Stderr {
                            self.parser.process(b"\x1b[31m");
                        }
                        self.parser.process(text.replace('\n', "\r\n").as_bytes());
                        if *stream == agena_domain::CommandOutputStream::Stderr {
                            self.parser.process(b"\x1b[0m");
                        }
                    }
                    _ => {}
                },
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
            applied_sequence = Some(chunk.cursor.sequence);
        }
        self.gap |=
            page.gap || page.resource.dropped_bytes > 0 || page.resource.dropped_packets > 0;
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
        let mut resource = if let Some(previous) =
            previous.filter(|frame| frame.resource.cursor.sequence > page.resource.cursor.sequence)
        {
            previous.resource.clone()
        } else {
            page.resource.clone()
        };
        if let Some(previous) = previous {
            resource.committed_cursor.sequence = resource
                .committed_cursor
                .sequence
                .max(previous.resource.committed_cursor.sequence)
                .max(page.resource.committed_cursor.sequence);
            resource.dropped_bytes = resource
                .dropped_bytes
                .max(previous.resource.dropped_bytes)
                .max(page.resource.dropped_bytes);
            resource.dropped_packets = resource
                .dropped_packets
                .max(previous.resource.dropped_packets)
                .max(page.resource.dropped_packets);
            resource.committed_dropped_bytes = resource
                .committed_dropped_bytes
                .max(previous.resource.committed_dropped_bytes)
                .max(page.resource.committed_dropped_bytes);
            resource.committed_dropped_packets = resource
                .committed_dropped_packets
                .max(previous.resource.committed_dropped_packets)
                .max(page.resource.committed_dropped_packets);
        }
        if let Some(previous) = previous
            && previous.resource.state != ContentState::Active
            && resource.state == ContentState::Active
            && previous.resource.cursor == resource.cursor
        {
            resource.state = previous.resource.state;
        }
        let terminal = resource.kind == agena_domain::ContentKind::Terminal;
        let (lines, wrapped, omitted) = if resource.kind == agena_domain::ContentKind::Text {
            (Vec::new(), Vec::new(), false)
        } else {
            observation_lines(self.parser.screen_mut(), !terminal)
        };
        self.windowed |= omitted;
        let display_text = self.inline_ui.as_ref().map_or_else(
            || self.text.clone(),
            |refs| {
                agena_domain::expand_inline_references(
                    &self.text,
                    refs,
                    resource.state == ContentState::Active || page.has_more,
                )
            },
        );
        self.frame = Some(Arc::new(ContentFrame {
            resource,
            lines,
            wrapped,
            terminal_size: terminal.then(|| self.parser.screen().size()),
            gap: self.gap,
            windowed: self.windowed,
            text: Arc::from(display_text),
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

/// Log history is a local, bounded observation window. Normal PTY screens
/// omit unused trailing rows in the transcript; alternate screens preserve
/// their full layout. Neither kind reflows terminal cells.
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
    if history || !screen.alternate_screen() {
        while lines.last().is_some_and(|line| line.spans.is_empty()) {
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
            "{CONTENT_PREFIX}{}",
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
        RenderedLine::dim(format!("{BODY_PREFIX}┌ {}{geometry}", i18n.text(state)))
            .with_copy_projection("", 0),
    );
    if frame.gap {
        out.push(
            RenderedLine::dim(format!("{CONTENT_PREFIX}{}", i18n.text("content-gap")))
                .with_copy_projection("", 0),
        );
    }
    if frame.windowed {
        out.push(
            RenderedLine::dim(format!("{CONTENT_PREFIX}{}", i18n.text("content-window")))
                .with_copy_projection("", 0),
        );
    }
    if let Some(document) = &frame.document {
        crate::renderer::render_content_document(document, out, width, i18n);
    }
    if frame.resource.kind == agena_domain::ContentKind::Text {
        let body_width = width.saturating_sub(CONTENT_GUTTER).max(1);
        let rows = match format {
            agena_domain::ContentFormat::Diff => {
                crate::renderer::render_diff_document_with_i18n(&frame.text, body_width, i18n)
            }
            agena_domain::ContentFormat::Markdown => {
                crate::renderer::render_markdown_document_with_i18n(&frame.text, body_width, i18n)
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
                CONTENT_PREFIX,
                Style::default().fg(agena_tui_components::theme::muted_color()),
            )];
            spans.extend(row.rich_line.take().map_or_else(
                || vec![Span::styled(row.text.clone(), row.style)],
                |line| line.spans,
            ));
            row.text = format!("{CONTENT_PREFIX}{}", row.text);
            row.copy_column += usize::from(CONTENT_GUTTER);
            for segment in &mut row.copy_segments {
                segment.display_column += usize::from(CONTENT_GUTTER);
            }
            for placement in &mut row.math {
                placement.column = placement.column.saturating_add(CONTENT_GUTTER);
            }
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
        let available = width.saturating_sub(CONTENT_GUTTER).max(1) as usize;
        // PTY cells describe an actual screen. Observer width clips its rows
        // rather than changing the source's cursor and line geometry.
        let rows = if frame.resource.kind == agena_domain::ContentKind::Terminal {
            vec![clip_cells(line, available)]
        } else {
            crate::wrap_rich_line(&line.spans, available, available)
        };
        for wrapped in rows {
            let mut spans = vec![Span::styled(
                CONTENT_PREFIX,
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
                    .with_copy_projection(copy, usize::from(CONTENT_GUTTER))
                    .with_navigation_unit(navigation_base + group_start, group_text.clone()),
            );
        }
        if !frame.wrapped.get(index).copied().unwrap_or(false) {
            group_start = index + 1;
        }
    }
    if let Some(error) = connection_error.or(frame.resource.capture_error.as_deref()) {
        out.push(RenderedLine::plain(
            format!("{CONTENT_PREFIX}{}", agena_tui::sanitize_picker_text(error)),
            Style::default().fg(agena_tui_components::theme::danger_color()),
        ));
    }
    out.push(RenderedLine::dim(format!("{BODY_PREFIX}└─")).with_copy_projection("", 0));
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
mod tests {
    use super::*;
    use agena_domain::{
        ContentChunk, ContentKind, TerminalAttributes, TerminalCellRun, TerminalSnapshot,
    };

    fn page(kind: ContentKind, sequence: u64, payload: ContentPayload) -> ContentPage {
        let cursor = ContentCursor {
            epoch: agena_domain::ContentEpoch::new().into(),
            sequence,
        };
        ContentPage {
            inline_ui: None,
            resource: ContentResource {
                resource_id: agena_domain::ContentId::new(),
                owner_session_id: 1,
                part_id: 1,
                kind,
                state: ContentState::Active,
                cursor,
                committed_cursor: cursor,
                total_bytes: None,
                dropped_bytes: 0,
                dropped_packets: 0,
                committed_dropped_bytes: 0,
                committed_dropped_packets: 0,
                retained_ranges: Vec::new(),
                capture_error: None,
            },
            chunks: vec![ContentChunk {
                cursor,
                captured_at_ms: 0,
                payload,
            }],
            next_cursor: cursor,
            has_more: false,
            gap: false,
        }
    }

    fn screen() -> TerminalSnapshot {
        TerminalSnapshot {
            rows: 24,
            cols: 80,
            cursor_row: 10,
            cursor_col: 0,
            cursor_visible: true,
            alternate_screen: false,
            bracketed_paste: false,
            application_cursor: false,
            cells: (0..10)
                .map(|row| TerminalCellRun {
                    row,
                    col: 0,
                    text: (row + 1).to_string(),
                    attributes: TerminalAttributes::default(),
                })
                .collect(),
        }
    }

    fn text(view: &ContentView) -> Vec<String> {
        view.frame
            .as_ref()
            .unwrap()
            .lines
            .iter()
            .map(ToString::to_string)
            .collect()
    }

    #[test]
    fn terminal_reconnect_does_not_duplicate_output_or_unused_rows() {
        let mut view = ContentView::default();
        let first = page(
            ContentKind::Terminal,
            1,
            ContentPayload::Terminal { screen: screen() },
        );
        assert!(view.apply(first.clone()));
        assert_eq!(
            text(&view),
            (1..=10).map(|i| i.to_string()).collect::<Vec<_>>()
        );
        assert!(view.apply(first.clone()));
        let mut next = first.clone();
        next.next_cursor.sequence = 2;
        next.resource.cursor.sequence = 2;
        next.resource.committed_cursor.sequence = 2;
        next.chunks[0].cursor.sequence = 2;
        assert!(view.apply(next));
        // An old in-flight page must not roll the observation backwards.
        assert!(view.apply(first));
        assert_eq!(view.cursor.unwrap().sequence, 2);
        assert_eq!(text(&view).join("\n"), "1\n2\n3\n4\n5\n6\n7\n8\n9\n10");
    }

    #[test]
    fn terminal_patch_clears_rows_and_recovers_from_missing_base() {
        let mut view = ContentView::default();
        let first = page(
            ContentKind::Terminal,
            1,
            ContentPayload::Terminal { screen: screen() },
        );
        assert!(view.apply(first.clone()));
        let mut next = first.clone();
        next.next_cursor.sequence = 2;
        next.resource.cursor.sequence = 2;
        next.resource.committed_cursor.sequence = 2;
        next.chunks[0].cursor.sequence = 2;
        let mut patch = screen();
        patch.cells.clear();
        next.chunks[0].payload = ContentPayload::TerminalPatch {
            base_cursor: first.next_cursor,
            screen: patch,
            rows_changed: vec![9],
        };
        assert!(view.apply(next.clone()));
        assert_eq!(text(&view).len(), 9);
        let mut fresh = ContentView::default();
        assert!(!fresh.apply(next));
        assert!(fresh.cursor.is_none());
        assert!(fresh.apply(first));
        assert_eq!(text(&fresh).len(), 10);
    }

    #[test]
    fn alternate_screen_preserves_geometry_and_interior_blank_rows() {
        let mut view = ContentView::default();
        let mut snapshot = screen();
        snapshot.alternate_screen = true;
        snapshot.cells.retain(|run| run.row != 4);
        assert!(view.apply(page(
            ContentKind::Terminal,
            1,
            ContentPayload::Terminal { screen: snapshot }
        )));
        assert_eq!(text(&view).len(), 24);
        assert!(text(&view)[4].is_empty());
    }

    #[test]
    fn text_does_not_render_an_extra_emulator_row() {
        let mut view = ContentView::default();
        assert!(view.apply(page(
            ContentKind::Text,
            5,
            ContentPayload::Text {
                text: "hello".into()
            }
        )));
        let frame = view.frame.as_ref().unwrap();
        assert_eq!(frame.text.as_ref(), "hello");
        assert!(frame.lines.is_empty());
    }

    #[test]
    fn compact_content_gutter_keeps_table_copy_segments_on_their_display_cells() {
        use unicode_width::UnicodeWidthStr;

        let mut view = ContentView::default();
        let source = "| 项 | 值 |\n| --- | --- |\n| 说明 | 内容完整 |";
        assert!(view.apply(page(
            ContentKind::Text,
            source.len() as u64,
            ContentPayload::Text {
                text: source.into(),
            }
        )));
        for width in [24, 48, 80] {
            let mut lines = Vec::new();
            render_content(
                view.frame.as_deref(),
                &mut lines,
                width,
                &agena_tui::i18n::I18n::english(),
                None,
                agena_domain::ContentFormat::Markdown,
            );
            let mut segments = 0;
            for line in &lines {
                assert!(UnicodeWidthStr::width(line.text.as_str()) <= usize::from(width));
                for segment in &line.copy_segments {
                    let byte = line.text.find(&segment.text).unwrap();
                    assert_eq!(
                        UnicodeWidthStr::width(&line.text[..byte]),
                        segment.display_column
                    );
                    segments += 1;
                }
            }
            assert!(segments >= 4);
            assert!(lines[0].text.starts_with("  ┌"));
        }
    }

    #[test]
    fn log_scrollback_preserves_each_line_once_without_empty_screen_padding() {
        let mut view = ContentView::default();
        let output = (1..=60).map(|i| format!("{i}\n")).collect::<String>();
        assert!(view.apply(page(
            ContentKind::Log,
            1,
            ContentPayload::Log {
                stream: agena_domain::CommandOutputStream::Stdout,
                text: output,
            }
        )));
        assert_eq!(
            text(&view),
            (1..=60).map(|i| i.to_string()).collect::<Vec<_>>()
        );
    }
}
