//! Categorical charts rendered as terminal cells, not replaced by a table.
//! Exact values remain available through an explicit Data control and export.
use super::*;
use ratatui::style::Color;

fn colors(index: usize) -> Color {
    [
        theme::accent_color(),
        theme::info_color(),
        theme::success_color(),
        theme::special_color(),
        theme::warning_color(),
        theme::danger_color(),
    ][index % 6]
}
fn number(value: f64) -> String {
    if value.abs() >= 1e8 || (value != 0.0 && value.abs() < 0.001) {
        format!("{value:.1e}")
    } else if value.fract().abs() < 1e-8 {
        format!("{value:.0}")
    } else {
        format!("{value:.2}")
    }
}
struct PixelPlot {
    width: usize,
    height: usize,
    pixels: Vec<Option<usize>>,
}
impl PixelPlot {
    fn new(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            pixels: vec![None; width * 2 * height * 4],
        }
    }
    fn dot(&mut self, x: isize, y: isize, series: usize) {
        if x >= 0 && y >= 0 && (x as usize) < self.width * 2 && (y as usize) < self.height * 4 {
            self.pixels[y as usize * self.width * 2 + x as usize] = Some(series);
        }
    }
    fn line(&mut self, from: (isize, isize), to: (isize, isize), series: usize) {
        let (mut x, mut y) = from;
        let dx = (to.0 - x).abs();
        let sx = if x < to.0 { 1 } else { -1 };
        let dy = -(to.1 - y).abs();
        let sy = if y < to.1 { 1 } else { -1 };
        let mut err = dx + dy;
        loop {
            self.dot(x, y, series);
            if (x, y) == to {
                break;
            }
            let twice = 2 * err;
            if twice >= dy {
                err += dy;
                x += sx;
            }
            if twice <= dx {
                err += dx;
                y += sy;
            }
        }
    }
    fn row(&self, y: usize) -> Vec<Span<'static>> {
        let mut spans = Vec::new();
        for x in 0..self.width {
            let mut mask = 0u32;
            let mut selected = 0;
            for py in 0..4 {
                for px in 0..2 {
                    if let Some(series) = self.pixels[(y * 4 + py) * self.width * 2 + x * 2 + px] {
                        mask |= 1 << [[0, 3], [1, 4], [2, 5], [6, 7]][py][px];
                        selected = series;
                    }
                }
            }
            spans.push(if mask == 0 {
                Span::raw(" ")
            } else {
                Span::styled(
                    if mask == 255 {
                        "█".to_owned()
                    } else {
                        char::from_u32(0x2800 + mask).unwrap_or(' ').to_string()
                    },
                    Style::default().fg(colors(selected)),
                )
            });
        }
        spans
    }
}
pub(super) fn render_chart(
    id: &str,
    props: &BTreeMap<String, Value>,
    width: u16,
    ctx: &Context<'_>,
) -> Vec<RenderedLine> {
    let text = |key: &str| props.get(key).map(display).unwrap_or_default();
    let enabled = props.get("disabled").and_then(Value::as_bool) != Some(true);
    let labels = props
        .get("labels")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let series = props
        .get("series")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let hidden = ctx.presentation.hidden_series.get(id);
    let visible = series
        .iter()
        .enumerate()
        .filter(|(index, _)| !hidden.is_some_and(|set| set.contains(index)))
        .collect::<Vec<_>>();
    let mut out = layout::text(
        &format!("▥ {}", text("title")),
        width,
        Style::default().add_modifier(Modifier::BOLD),
    );
    let units = text("y_label");
    if !units.is_empty() {
        out.extend(layout::text(&units, width, theme::muted_style()));
    }
    if width >= 18 && !labels.is_empty() && !visible.is_empty() {
        if text("chart_type") == "pie" {
            out.extend(pie(&labels, &visible, width));
        } else {
            out.extend(cartesian(
                &labels,
                &visible,
                width,
                text("chart_type").as_str(),
            ));
        }
    }
    let mut legend = Vec::new();
    for (index, s) in series.iter().enumerate() {
        let name = s.get("name").map(display).unwrap_or_default();
        let on = !hidden.is_some_and(|h| h.contains(&index));
        let label = format!("[ {} {name} ]", if on { "●" } else { "○" });
        let rows = layout::interactive(
            layout::text(
                &label,
                width,
                if on {
                    Style::default().fg(colors(index))
                } else {
                    theme::muted_style()
                },
            ),
            ctx.control(id, InlineEvent::Series(index)),
            enabled,
        );
        legend.extend(rows);
    }
    out.extend(legend);
    let showing = ctx.presentation.data.contains(id);
    out.extend(layout::interactive(
        layout::text(
            &format!(
                "[ {} {} ]",
                if showing { "▾" } else { "▸" },
                ctx.i18n.text("inline-ui-data")
            ),
            width,
            theme::muted_style(),
        ),
        ctx.control(id, InlineEvent::Data),
        enabled,
    ));
    if showing || width < 18 {
        let mut columns = vec![text("x_label")];
        columns.extend(
            series
                .iter()
                .map(|s| s.get("name").map(display).unwrap_or_default()),
        );
        let rows = labels
            .iter()
            .enumerate()
            .map(|(index, label)| {
                let mut row = vec![display(label)];
                row.extend(series.iter().map(|s| {
                    s.get("values")
                        .and_then(|v| v.get(index))
                        .filter(|v| !v.is_null())
                        .map(display)
                        .unwrap_or_else(|| "—".into())
                }));
                row
            })
            .collect::<Vec<_>>();
        out.extend(layout::table(&columns, &rows, width));
    }
    out
}
fn cartesian(
    labels: &[Value],
    series: &[(usize, &Value)],
    width: u16,
    kind: &str,
) -> Vec<RenderedLine> {
    let values = series
        .iter()
        .flat_map(|(_, s)| {
            s.get("values")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_f64)
        })
        .collect::<Vec<_>>();
    let mut min = values.iter().copied().fold(0.0, f64::min);
    let mut max = values.iter().copied().fold(0.0, f64::max);
    if max <= min {
        max = min + 1.0;
    }
    if kind != "bar" && min == max {
        min = 0.0;
    }
    let maxlabel = number(max);
    let minlabel = number(min);
    let axis = maxlabel.width().max(minlabel.width()).min(12) + 2;
    let plot_width = usize::from(width).saturating_sub(axis).max(1);
    let height = if width < 45 { 7 } else { 10 };
    let mut plot = PixelPlot::new(plot_width, height);
    let y = |v: f64| {
        ((max - v) / (max - min) * (height * 4 - 1) as f64)
            .round()
            .clamp(0.0, (height * 4 - 1) as f64) as isize
    };
    let zero = y(0.0);
    let n = labels.len().max(1);
    for (order, (index, s)) in series.iter().enumerate() {
        let data = s
            .get("values")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut previous = None;
        for (i, value) in data.iter().take(n).enumerate() {
            let Some(value) = value.as_f64() else {
                previous = None;
                continue;
            };
            let x = (((i as f64 + 0.5) / n as f64) * (plot_width * 2 - 1) as f64).round() as isize;
            let yy = y(value);
            if kind == "bar" {
                let group = ((plot_width * 2) / n).max(1);
                let bar = (group.saturating_sub(2) / series.len()).max(1);
                let start = ((i * plot_width * 2) / n + order * bar) as isize;
                for xx in start..start + bar as isize {
                    plot.line((xx, zero), (xx, yy), *index);
                }
            } else {
                plot.dot(x, yy, *index);
                if let Some((px, py)) = previous {
                    plot.line((px, py), (x, yy), *index);
                    if kind == "area" {
                        for xx in px..=x {
                            let sy = if x == px {
                                yy
                            } else {
                                py + (yy - py) * (xx - px) / (x - px)
                            };
                            for fy in sy.min(zero)..=sy.max(zero) {
                                if xx % 2 == 0 {
                                    plot.dot(xx, fy, *index);
                                }
                            }
                        }
                    }
                }
                previous = Some((x, yy));
            }
        }
    }
    let mut rows = Vec::new();
    for yy in 0..height {
        let label = if yy == 0 {
            maxlabel.clone()
        } else if yy == height - 1 {
            minlabel.clone()
        } else if yy == height / 2 {
            number((min + max) / 2.0)
        } else {
            String::new()
        };
        let label = agena_tui_components::truncate_display_text(&label, axis - 2);
        let mut spans = vec![Span::styled(
            format!("{}{label} │", " ".repeat(axis - 2 - label.width())),
            theme::muted_style(),
        )];
        spans.extend(plot.row(yy));
        rows.push(RenderedLine::rich(Line::from(spans)).with_copy_projection("", 0));
    }
    rows.push(
        RenderedLine::plain(
            format!("{}└{}", " ".repeat(axis - 1), "─".repeat(plot_width)),
            theme::muted_style(),
        )
        .with_copy_projection("", 0),
    );
    let tick_width = labels
        .iter()
        .map(|label| display(label).width())
        .max()
        .unwrap_or(4)
        .clamp(4, 12);
    let step = (n * (tick_width + 1)).div_ceil(plot_width.max(1)).max(1);
    // Labels are placed as rich spans below. Unicode width is counted explicitly.
    let mut spans = vec![Span::raw(" ".repeat(axis))];
    let mut column = 0;
    for i in (0..n).step_by(step) {
        let start = (i * plot_width / n).max(column);
        if start >= plot_width {
            break;
        }
        let limit = (plot_width / n * step)
            .saturating_sub(1)
            .max(1)
            .min(plot_width - start);
        let label = agena_tui_components::truncate_display_text(
            &layout::single_line(&display(&labels[i])),
            limit,
        );
        spans.push(Span::raw(" ".repeat(start.saturating_sub(column))));
        spans.push(Span::styled(label.clone(), theme::muted_style()));
        column = start + label.width();
    }
    rows.push(RenderedLine::rich(Line::from(spans)));
    rows
}
fn pie(labels: &[Value], series: &[(usize, &Value)], width: u16) -> Vec<RenderedLine> {
    let Some((_, data)) = series.first() else {
        return Vec::new();
    };
    let values = data
        .get("values")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let total = values.iter().filter_map(Value::as_f64).sum::<f64>();
    if total <= 0.0 {
        return Vec::new();
    }
    let w = usize::from(width).min(24);
    let h = (w / 2).clamp(6, 12);
    let mut plot = PixelPlot::new(w, h);
    let rx = (w as f64 * 2.0 - 2.0) / 2.0;
    let ry = (h as f64 * 4.0 - 2.0) / 2.0;
    for y in 0..h * 4 {
        for x in 0..w * 2 {
            let dx = (x as f64 - rx) / rx;
            let dy = (y as f64 - ry) / ry;
            let radius = dx * dx + dy * dy;
            if !(0.28..=1.0).contains(&radius) {
                continue;
            }
            let fraction = (dy.atan2(dx) + std::f64::consts::PI) / (2.0 * std::f64::consts::PI);
            let mut sum = 0.0;
            for (index, value) in values.iter().enumerate() {
                sum += value.as_f64().unwrap_or(0.0) / total;
                if fraction <= sum {
                    plot.dot(x as isize, y as isize, index);
                    break;
                }
            }
        }
    }
    let mut rows = (0..h)
        .map(|y| RenderedLine::rich(Line::from(plot.row(y))).with_copy_projection("", 0))
        .collect::<Vec<_>>();
    for (index, label) in labels.iter().enumerate() {
        let value = values.get(index).and_then(Value::as_f64).unwrap_or(0.0);
        rows.extend(layout::text(
            &format!(
                "● {}  {} ({:.1}%)",
                display(label),
                number(value),
                value / total * 100.0
            ),
            width,
            Style::default().fg(colors(index)),
        ));
    }
    rows
}
