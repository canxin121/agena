use super::*;
use agena_tui_components::truncate_display_text;

pub(super) fn single_line(value: &str) -> String {
    crate::sanitize_terminal_text(value).replace(['\n', '\t'], " ")
}

pub(super) fn text(value: &str, width: u16, style: Style) -> Vec<RenderedLine> {
    crate::sanitize_terminal_text(value)
        .split('\n')
        .flat_map(|line| {
            crate::wrap_rich_line(
                &[Span::styled(line.to_owned(), style)],
                usize::from(width.max(1)),
                usize::from(width.max(1)),
            )
            .into_iter()
            .map(RenderedLine::rich)
        })
        .collect()
}
pub(super) fn rule(width: u16) -> RenderedLine {
    RenderedLine::plain("─".repeat(usize::from(width)), theme::muted_style())
        .with_copy_projection("", 0)
}
pub(super) fn split_label(
    label: &str,
    value: &str,
    width: u16,
    left: Style,
    right: Style,
) -> RenderedLine {
    let width = usize::from(width);
    let value = truncate_display_text(&single_line(value), width.saturating_sub(2).min(width / 2));
    let label = truncate_display_text(
        &single_line(label),
        width.saturating_sub(value.width() + usize::from(!value.is_empty())),
    );
    let gap = width.saturating_sub(label.width() + value.width());
    RenderedLine::rich(Line::from(vec![
        Span::styled(label, left),
        Span::raw(" ".repeat(gap)),
        Span::styled(value, right),
    ]))
}
pub(super) fn interactive(
    mut rows: Vec<RenderedLine>,
    action: TranscriptControlAction,
    enabled: bool,
) -> Vec<RenderedLine> {
    for row in &mut rows {
        let width = row.text.width();
        if width > 0 {
            row.controls.push(RenderedTranscriptControl {
                columns: 0..width,
                action: action.clone(),
                enabled,
            });
        }
        if !enabled {
            row.style = theme::muted_style();
            if let Some(line) = &mut row.rich_line {
                for span in &mut line.spans {
                    span.style = theme::muted_style();
                }
            }
        }
    }
    rows
}
pub(super) fn frame(
    content: Vec<RenderedLine>,
    title: &str,
    width: u16,
    style: Style,
) -> Vec<RenderedLine> {
    if width < 6 {
        return content;
    }
    let inner = width - 2;
    let title = truncate_display_text(&single_line(title), usize::from(width - 4));
    let label = if title.is_empty() {
        String::new()
    } else {
        format!(" {title} ")
    };
    let mut result = vec![
        RenderedLine::rich(Line::from(vec![
            Span::styled("┌", style),
            Span::styled(label.clone(), Style::default().add_modifier(Modifier::BOLD)),
            Span::styled(
                format!(
                    "{}┐",
                    "─".repeat(usize::from(inner).saturating_sub(label.width()))
                ),
                style,
            ),
        ]))
        .with_copy_projection("", 0),
    ];
    for row in content {
        let used = row.text.width();
        let mut row = prefix_rows(vec![row], "│ ").remove(0);
        let padding = usize::from(inner.saturating_sub(2)).saturating_sub(used);
        row.text.push_str(&format!("{} │", " ".repeat(padding)));
        if let Some(line) = &mut row.rich_line {
            line.spans[0].style = style;
            line.spans.push(Span::raw(" ".repeat(padding + 1)));
            line.spans.push(Span::styled("│", style));
        }
        result.push(row);
    }
    result.push(
        RenderedLine::plain(format!("└{}┘", "─".repeat(usize::from(inner))), style)
            .with_copy_projection("", 0),
    );
    result
}
pub(super) fn horizontal(columns: Vec<(u16, Vec<RenderedLine>)>, gap: u16) -> Vec<RenderedLine> {
    let height = columns
        .iter()
        .map(|(_, rows)| rows.len())
        .max()
        .unwrap_or(0);
    let mut output = Vec::new();
    for index in 0..height {
        let mut spans = Vec::new();
        let mut controls = Vec::new();
        let mut offset = 0;
        let mut copies = Vec::new();
        let mut copy_segments = Vec::new();
        for (i, (width, rows)) in columns.iter().enumerate() {
            if i > 0 {
                spans.push(Span::raw(" ".repeat(usize::from(gap))));
                offset += usize::from(gap);
            }
            let row = rows.get(index);
            let used = row.map_or(0, |row| row.text.width());
            if let Some(row) = row {
                spans.extend(
                    row.rich_line
                        .clone()
                        .unwrap_or_else(|| Line::styled(row.text.clone(), row.style))
                        .spans,
                );
                copies.push(row.copy_text.clone());
                if row.copy_segments.is_empty() && !row.copy_text.is_empty() {
                    copy_segments.push(crate::RenderedCopySegment {
                        display_column: offset + row.copy_column,
                        text: row.copy_text.clone(),
                        separator_before: if copy_segments.is_empty() {
                            String::new()
                        } else {
                            "\t".into()
                        },
                    });
                } else {
                    for segment in &row.copy_segments {
                        let mut segment = segment.clone();
                        segment.display_column += offset;
                        if segment.separator_before.is_empty() && !copy_segments.is_empty() {
                            segment.separator_before = "\t".into();
                        }
                        copy_segments.push(segment);
                    }
                }
                controls.extend(row.controls.iter().cloned().map(|mut ctrl| {
                    ctrl.columns = (ctrl.columns.start + offset)..(ctrl.columns.end + offset);
                    ctrl
                }));
            }
            spans.push(Span::raw(
                " ".repeat(usize::from(*width).saturating_sub(used)),
            ));
            offset += usize::from(*width);
        }
        let mut row = RenderedLine::rich(Line::from(spans));
        row.controls = controls;
        row.copy_text = copies.join("\t");
        row.copy_segments = copy_segments;
        output.push(row);
    }
    output
}
pub(super) fn gauge(ratio: f64, width: usize, style: Style, knob: bool) -> Line<'static> {
    let ratio = if ratio.is_finite() {
        ratio.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let width = width.max(1);
    let units = (ratio * width as f64 * 8.0).round() as usize;
    let full = (units / 8).min(width);
    let fraction = units % 8;
    let mut spans = vec![Span::styled(
        "━".repeat(if knob {
            full.saturating_sub(usize::from(full == width))
        } else {
            full
        }),
        style,
    )];
    if knob {
        spans.push(Span::styled("●", style));
    } else if full < width && fraction > 0 {
        spans.push(Span::styled(
            ["", "▏", "▎", "▍", "▌", "▋", "▊", "▉"][fraction].to_owned(),
            style,
        ));
    }
    let consumed = if knob {
        full.min(width - 1) + 1
    } else {
        full + usize::from(full < width && fraction > 0)
    };
    spans.push(Span::styled(
        "─".repeat(width.saturating_sub(consumed)),
        theme::muted_style(),
    ));
    Line::from(spans)
}
pub(super) fn table(columns: &[String], rows: &[Vec<String>], width: u16) -> Vec<RenderedLine> {
    if columns.is_empty() {
        return Vec::new();
    }
    let n = columns.len();
    let available = usize::from(width).saturating_sub(n.saturating_sub(1));
    if available / n < 6 && n > 1 {
        let mut out = Vec::new();
        for row in rows {
            for (index, label) in columns.iter().enumerate() {
                out.extend(text(
                    &format!("{label}: {}", row.get(index).cloned().unwrap_or_default()),
                    width,
                    Style::default(),
                ));
            }
            out.push(rule(width));
        }
        return out;
    }
    let sizes = (0..n)
        .map(|index| available / n + usize::from(index < available % n))
        .collect::<Vec<_>>();
    let render = |row: &[String], header: bool| {
        horizontal(
            row.iter()
                .take(n)
                .enumerate()
                .map(|(index, value)| {
                    (
                        sizes[index] as u16,
                        text(
                            value,
                            sizes[index].max(1) as u16,
                            if header {
                                Style::default()
                                    .fg(theme::accent_color())
                                    .add_modifier(Modifier::BOLD)
                            } else {
                                Style::default()
                            },
                        ),
                    )
                })
                .collect(),
            1,
        )
    };
    let mut out = render(columns, true);
    out.push(rule(width));
    for row in rows {
        out.extend(render(row, false));
    }
    out
}
