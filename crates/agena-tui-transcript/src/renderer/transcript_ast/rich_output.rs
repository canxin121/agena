//! Bounded numeric charts. Chart data is the same `agena-chart` schema used by
//! the web renderer. Ratatui draws themed braille cells without terminal images.
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    symbols::Marker,
    text::{Line, Span},
    widgets::{Axis, Chart, Dataset, GraphType, Widget},
};
use serde_json::Value;
use unicode_width::UnicodeWidthStr;

fn palette() -> [Color; 8] {
    use agena_tui_components::theme::*;
    [
        accent_color(),
        success_color(),
        warning_color(),
        info_color(),
        special_color(),
        danger_color(),
        Color::Cyan,
        Color::Yellow,
    ]
}

struct Series {
    name: String,
    values: Vec<Option<f64>>,
}
struct Spec {
    kind: String,
    title: String,
    x_label: String,
    y_label: String,
    labels: Vec<String>,
    series: Vec<Series>,
}

fn field(value: &Value, key: &str, max: usize) -> Result<String, String> {
    match value.get(key) {
        None => Ok(String::new()),
        Some(Value::String(s)) if s.len() <= max && !s.chars().any(char::is_control) => {
            Ok(s.clone())
        }
        _ => Err(format!("invalid chart {key}")),
    }
}

fn parse(source: &str) -> Result<Spec, String> {
    if source.len() > 128 * 1024 {
        return Err("chart source exceeds 128 KiB".into());
    }
    let value: Value = serde_json::from_str(source).map_err(|_| "invalid chart JSON")?;
    let kind = field(&value, "type", 20)?;
    if !matches!(kind.as_str(), "line" | "area" | "bar" | "scatter" | "pie") {
        return Err("unsupported chart type".into());
    }
    let labels = value["labels"]
        .as_array()
        .filter(|l| !l.is_empty() && l.len() <= 512)
        .ok_or("chart needs 1–512 labels")?
        .iter()
        .map(|label| {
            label
                .as_str()
                .filter(|s| s.len() <= 400 && !s.chars().any(char::is_control))
                .map(str::to_owned)
                .ok_or_else(|| "invalid chart label".to_owned())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let series = value["series"]
        .as_array()
        .filter(|s| !s.is_empty() && s.len() <= 8)
        .ok_or("chart needs 1–8 series")?
        .iter()
        .map(|s| {
            let name = field(s, "name", 400)?;
            let values = s["values"]
                .as_array()
                .filter(|v| v.len() == labels.len())
                .ok_or("series must match labels")?
                .iter()
                .map(|v| {
                    if v.is_null() {
                        Ok(None)
                    } else {
                        v.as_f64()
                            .filter(|n| n.is_finite() && n.abs() <= 1e100)
                            .map(Some)
                            .ok_or_else(|| "chart requires finite values".to_owned())
                    }
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Series { name, values })
        })
        .collect::<Result<Vec<_>, String>>()?;
    if !series.iter().any(|s| s.values.iter().any(Option::is_some)) {
        return Err("chart has no numeric data".into());
    }
    if kind == "pie"
        && (series.len() != 1
            || series[0].values.iter().flatten().any(|v| *v < 0.0)
            || !series[0].values.iter().flatten().any(|v| *v > 0.0))
    {
        return Err("pie needs one nonnegative series with a positive total".into());
    }
    Ok(Spec {
        kind,
        title: field(&value, "title", 800)?,
        x_label: field(&value, "xLabel", 800)?,
        y_label: field(&value, "yLabel", 800)?,
        labels,
        series,
    })
}

fn clipped(text: &str, width: usize) -> String {
    if UnicodeWidthStr::width(text) <= width {
        return text.to_owned();
    }
    let mut result = String::new();
    let mut used = 0;
    for ch in text.chars() {
        let cells = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + cells + 1 > width {
            break;
        }
        result.push(ch);
        used += cells;
    }
    if width > 0 {
        result.push('…');
    }
    result
}

fn number(value: f64) -> String {
    let abs = value.abs();
    if abs != 0.0 && !(0.001..1e6).contains(&abs) {
        format!("{value:.1e}")
    } else if value.fract() == 0.0 {
        format!("{value:.0}")
    } else {
        format!("{value:.2}")
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_owned()
    }
}

pub(super) fn chart(source: &str, width: usize) -> Result<Vec<Line<'static>>, String> {
    let spec = parse(source)?;
    if width < 28 {
        return Err("chart needs at least 28 columns".into());
    }
    // A huge terminal must not allocate an unbounded cell buffer.
    let width = width.min(200);
    let colors = palette();
    let mut lines = Vec::new();
    if !spec.title.is_empty() {
        lines.push(Line::from(Span::styled(
            clipped(&spec.title, width),
            Style::default().fg(colors[0]).add_modifier(Modifier::BOLD),
        )));
    }
    if matches!(spec.kind.as_str(), "bar" | "pie") {
        let label_width = spec
            .labels
            .iter()
            .map(|s| UnicodeWidthStr::width(s.as_str()))
            .max()
            .unwrap_or(0)
            .min(width / 3)
            .max(1);
        let max = spec
            .series
            .iter()
            .flat_map(|s| s.values.iter().flatten())
            .map(|v| v.abs())
            .fold(0.0_f64, f64::max)
            .max(f64::MIN_POSITIVE);
        let negative = spec
            .series
            .iter()
            .any(|s| s.values.iter().flatten().any(|v| *v < 0.0));
        let room = width.saturating_sub(label_width + 14);
        let half = if negative { room / 2 } else { room };
        let total = spec.series[0].values.iter().flatten().sum::<f64>();
        // Text charts stay useful for large datasets without making one reply
        // thousands of rows tall. Full values remain in source/copy/export.
        for (index, series) in spec.series.iter().enumerate() {
            if spec.series.len() > 1 {
                lines.push(Line::from(Span::styled(
                    clipped(&series.name, width),
                    Style::default().fg(colors[index]),
                )));
            }
            for (row, label) in spec.labels.iter().take(24).enumerate() {
                let label = clipped(label, label_width);
                let padding =
                    " ".repeat(label_width.saturating_sub(UnicodeWidthStr::width(label.as_str())));
                let value = series.values[row];
                let mut spans = vec![Span::styled(
                    format!("{label}{padding} "),
                    Style::default().fg(agena_tui_components::theme::muted_color()),
                )];
                if let Some(value) = value {
                    let cells = (value.abs() / max * half as f64).round() as usize;
                    let bar = "━".repeat(cells);
                    let shape = if negative {
                        if value < 0.0 {
                            format!("{}{}│{}", " ".repeat(half - cells), bar, " ".repeat(half))
                        } else {
                            format!("{}│{}{}", " ".repeat(half), bar, " ".repeat(half - cells))
                        }
                    } else {
                        format!("{bar}{}", " ".repeat(half - cells))
                    };
                    spans.push(Span::styled(shape, Style::default().fg(colors[index])));
                    let label = if spec.kind == "pie" {
                        format!(" {:.1}% {}", value / total * 100.0, number(value))
                    } else {
                        format!(" {}", number(value))
                    };
                    let budget = width.saturating_sub(
                        spans
                            .iter()
                            .map(|s| UnicodeWidthStr::width(s.content.as_ref()))
                            .sum::<usize>(),
                    );
                    spans.push(Span::raw(clipped(&label, budget)));
                } else {
                    spans.push(Span::raw("—"));
                }
                lines.push(Line::from(spans));
            }
        }
        if spec.labels.len() > 24 {
            lines.push(Line::from(Span::styled(
                format!("… +{}", spec.labels.len() - 24),
                Style::default().fg(agena_tui_components::theme::muted_color()),
            )));
        }
    } else {
        let min = spec
            .series
            .iter()
            .flat_map(|s| s.values.iter().flatten())
            .copied()
            .fold(0.0_f64, f64::min);
        let mut max = spec
            .series
            .iter()
            .flat_map(|s| s.values.iter().flatten())
            .copied()
            .fold(0.0_f64, f64::max);
        if min == max {
            max = min + 1.0;
        }
        let mut segments = Vec::<(usize, Vec<(f64, f64)>)>::new();
        for (index, series) in spec.series.iter().enumerate() {
            let mut points = Vec::new();
            for (i, value) in series.values.iter().enumerate() {
                if let Some(value) = value {
                    points.push((i as f64, *value));
                } else if !points.is_empty() {
                    segments.push((index, std::mem::take(&mut points)));
                }
            }
            if !points.is_empty() {
                segments.push((index, points));
            }
        }
        let datasets = segments
            .iter()
            .map(|(i, data)| {
                Dataset::default()
                    .marker(Marker::Braille)
                    .graph_type(if spec.kind == "scatter" || data.len() == 1 {
                        GraphType::Scatter
                    } else {
                        GraphType::Line
                    })
                    .style(Style::default().fg(colors[*i]))
                    .data(data)
            })
            .collect::<Vec<_>>();
        let muted = Style::default().fg(agena_tui_components::theme::muted_color());
        let x_axis = Axis::default()
            .bounds([0.0, (spec.labels.len() - 1).max(1) as f64])
            .style(muted)
            .labels([
                clipped(&spec.labels[0], 12),
                clipped(&spec.labels[spec.labels.len() - 1], 12),
            ]);
        let y_axis = Axis::default()
            .bounds([min, max])
            .style(muted)
            .labels([number(min), number(max)]);
        let area = Rect::new(0, 0, width as u16, 12);
        let mut buffer = Buffer::empty(area);
        Chart::new(datasets)
            .x_axis(x_axis)
            .y_axis(y_axis)
            .render(area, &mut buffer);
        for row in 0..area.height {
            let mut spans = Vec::new();
            let mut column = 0;
            while column < area.width {
                let cell = &buffer[(column, row)];
                spans.push(Span::styled(cell.symbol().to_owned(), cell.style()));
                column += u16::try_from(UnicodeWidthStr::width(cell.symbol()).max(1)).unwrap_or(1);
            }
            lines.push(Line::from(spans));
        }
        if !spec.x_label.is_empty() || !spec.y_label.is_empty() {
            lines.push(Line::from(Span::styled(
                clipped(
                    &[spec.x_label, spec.y_label]
                        .into_iter()
                        .filter(|s| !s.is_empty())
                        .collect::<Vec<_>>()
                        .join(" · "),
                    width,
                ),
                muted,
            )));
        }
        let mut legend = Vec::new();
        for (i, series) in spec.series.iter().enumerate() {
            let last = series
                .values
                .iter()
                .rev()
                .flatten()
                .next()
                .copied()
                .map(number)
                .unwrap_or_else(|| "—".into());
            let item = format!("● {} {last}  ", series.name);
            let used = legend
                .iter()
                .map(|span: &Span<'_>| UnicodeWidthStr::width(span.content.as_ref()))
                .sum::<usize>();
            if used + UnicodeWidthStr::width(item.as_str()) > width && !legend.is_empty() {
                lines.push(Line::from(std::mem::take(&mut legend)));
            }
            legend.push(Span::styled(
                clipped(&item, width),
                Style::default().fg(colors[i]),
            ));
        }
        if !legend.is_empty() {
            lines.push(Line::from(legend));
        }
    }
    Ok(lines)
}

pub(super) fn app(
    source: &str,
    width: usize,
    i18n: &agena_tui::i18n::I18n,
) -> Result<Vec<Line<'static>>, String> {
    if source.len() > 600 * 1024 {
        return Err("app source exceeds limits".into());
    }
    let value: Value = serde_json::from_str(source).map_err(|_| "invalid app JSON")?;
    let title = value["title"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or("missing app title")?;
    let mut lines = vec![Line::from(Span::styled(
        clipped(&format!("◈ {title}"), width),
        Style::default()
            .fg(palette()[0])
            .add_modifier(Modifier::BOLD),
    ))];
    if let Some(description) = value["description"].as_str() {
        lines.push(Line::from(clipped(description, width)));
    }
    if let Some(services) = value["services"].as_array() {
        for service in services.iter().take(4) {
            lines.push(Line::from(Span::styled(
                clipped(
                    &format!(
                        "  {} · {}",
                        service["name"].as_str().unwrap_or("service"),
                        service["command"].as_str().unwrap_or("")
                    ),
                    width,
                ),
                Style::default().fg(agena_tui_components::theme::muted_color()),
            )));
        }
    }
    lines.push(Line::from(Span::styled(
        clipped(&i18n.text("transcript-interactive-app"), width),
        Style::default().fg(agena_tui_components::theme::muted_color()),
    )));
    Ok(lines)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn charts_handle_signed_missing_and_unicode_values_within_width() {
        for kind in ["line", "area", "scatter", "bar"] {
            let source = format!(
                r#"{{"type":"{kind}","title":"中文趋势","labels":["一月","二月","三月"],"series":[{{"name":"数据","values":[-4,null,8]}}]}}"#
            );
            for width in [28, 40, 80, 200] {
                let lines = chart(&source, width).unwrap();
                assert!(lines.iter().all(|line| line.width() <= width));
                assert!(
                    lines
                        .iter()
                        .any(|line| line.to_string().contains("中文趋势"))
                );
            }
        }
    }
    #[test]
    fn malformed_or_unbounded_charts_fall_back() {
        for source in [
            r#"{"type":"bar","labels":["a"],"series":[{"name":"a","values":[1,2]}]}"#,
            r#"{"type":"pie","labels":["a"],"series":[{"name":"a","values":[-1]}]}"#,
            r#"{"type":"line","labels":["a"],"series":[{"name":"a","values":[null]}]}"#,
        ] {
            assert!(chart(source, 80).is_err());
        }
        assert!(
            chart(
                r#"{"type":"line","labels":["a"],"series":[{"name":"a","values":[1]}]}"#,
                20
            )
            .is_err()
        );
    }
}
