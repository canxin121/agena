use super::*;
use agena_domain::{UiKind, UiResolvedNode};
use serde_json::json;

fn prop(node: &UiResolvedNode, key: &str) -> String {
    node.props.get(key).map(display).unwrap_or_default()
}
fn enabled(node: &UiResolvedNode) -> bool {
    node.props.get("disabled").and_then(Value::as_bool) != Some(true)
}
fn action(
    node: &UiResolvedNode,
    ctx: &Context<'_>,
    event: InlineEvent,
    rows: Vec<RenderedLine>,
) -> Vec<RenderedLine> {
    layout::interactive(rows, ctx.control(&node.id, event), enabled(node))
}
fn title_rows(title: &str, width: u16) -> Vec<RenderedLine> {
    if title.is_empty() {
        Vec::new()
    } else {
        layout::text(title, width, Style::default().add_modifier(Modifier::BOLD))
    }
}
fn stack(
    children: &[UiResolvedNode],
    width: u16,
    ctx: &Context<'_>,
    gap: usize,
) -> Vec<RenderedLine> {
    let mut out = Vec::new();
    for child in children {
        let rows = render_node(child, width, ctx);
        if !rows.is_empty() {
            if !out.is_empty() {
                for _ in 0..gap {
                    out.push(RenderedLine::plain("", Style::default()));
                }
            }
            out.extend(rows);
        }
    }
    out
}
fn grid(
    children: &[UiResolvedNode],
    requested: usize,
    width: u16,
    ctx: &Context<'_>,
) -> Vec<RenderedLine> {
    if children.is_empty() {
        return Vec::new();
    }
    let metrics = children.iter().all(|node| node.kind == UiKind::Metric);
    let minimum = if metrics { 20 } else { 36 };
    let columns = requested
        .max(1)
        .min(children.len())
        .min((usize::from(width) + 1) / (minimum + 1))
        .max(1);
    if columns == 1 {
        return stack(children, width, ctx, usize::from(!metrics));
    }
    let gap = 1u16;
    let usable = width.saturating_sub((columns - 1) as u16 * gap);
    let mut result = Vec::new();
    for chunk in children.chunks(columns) {
        if !result.is_empty() {
            result.push(RenderedLine::plain("", Style::default()));
        }
        let row = chunk
            .iter()
            .enumerate()
            .map(|(i, node)| {
                let column_width =
                    usable / columns as u16 + u16::from(i < usize::from(usable % columns as u16));
                (column_width, render_node(node, column_width, ctx))
            })
            .collect();
        result.extend(layout::horizontal(row, gap));
    }
    result
}

/// Rows pack content-sized controls instead of applying card/grid minimums.
/// A short heading, badge and action fit together even on an 80-column terminal.
fn row_layout(children: &[UiResolvedNode], width: u16, ctx: &Context<'_>) -> Vec<RenderedLine> {
    let mut output = Vec::new();
    let mut current = Vec::new();
    let mut occupied = 0u16;
    for child in children {
        let rows = render_node(child, width, ctx);
        if rows.is_empty() {
            continue;
        }
        let used = rows
            .iter()
            .map(|row| row.text.trim_end().width())
            .max()
            .unwrap_or(1)
            .max(1)
            .min(usize::from(width)) as u16;
        // Cells with full-width hit regions (e.g. sliders) keep that width.
        let used = used.max(
            rows.iter()
                .flat_map(|row| &row.controls)
                .map(|control| control.columns.end.min(usize::from(width)) as u16)
                .max()
                .unwrap_or(0),
        );
        if !current.is_empty() && occupied.saturating_add(2).saturating_add(used) > width {
            output.extend(layout::horizontal(std::mem::take(&mut current), 2));
            occupied = 0;
        }
        if !current.is_empty() {
            occupied = occupied.saturating_add(2);
        }
        occupied = occupied.saturating_add(used);
        current.push((used, render_node(child, used, ctx)));
    }
    if !current.is_empty() {
        output.extend(layout::horizontal(current, 2));
    }
    output
}

pub(super) fn render_node(
    node: &UiResolvedNode,
    width: u16,
    ctx: &Context<'_>,
) -> Vec<RenderedLine> {
    let width = width.max(1);
    let title = prop(node, "title");
    let label = prop(node, "label");
    let color = tone(&prop(node, "tone"));
    match node.kind {
        UiKind::Stack | UiKind::Row | UiKind::Grid => {
            let mut rows = title_rows(&title, width);
            let description = prop(node, "description");
            if !description.is_empty() {
                rows.extend(layout::text(&description, width, theme::muted_style()));
            }
            let children = match node.kind {
                UiKind::Grid => grid(
                    &node.children,
                    node.props
                        .get("columns")
                        .and_then(Value::as_u64)
                        .unwrap_or(2) as usize,
                    width,
                    ctx,
                ),
                UiKind::Row => row_layout(&node.children, width, ctx),
                _ => stack(
                    &node.children,
                    width,
                    ctx,
                    usize::from(prop(node, "gap") != "sm"),
                ),
            };
            rows.extend(children);
            rows
        }
        UiKind::Card => {
            let inner = width.saturating_sub(4).max(1);
            let mut rows = Vec::new();
            let description = prop(node, "description");
            if !description.is_empty() {
                rows.extend(layout::text(&description, inner, theme::muted_style()));
            }
            rows.extend(stack(&node.children, inner, ctx, 1));
            layout::frame(rows, &title, width, theme::muted_style())
        }
        UiKind::Heading => layout::text(
            &format!("◆ {}", prop(node, "text")),
            width,
            Style::default().add_modifier(Modifier::BOLD),
        ),
        UiKind::Text => layout::text(
            &prop(node, "text"),
            width,
            if prop(node, "size") == "sm" {
                theme::muted_style()
            } else {
                Style::default()
            },
        ),
        UiKind::Divider => vec![layout::rule(width)],
        UiKind::Badge => layout::text(
            &format!("● {}", prop(node, "text")),
            width,
            color.add_modifier(Modifier::BOLD),
        ),
        UiKind::Metric => {
            let inner = width.saturating_sub(4).max(1);
            let value = format!(
                "{}{}{}",
                prop(node, "prefix"),
                prop(node, "value"),
                prop(node, "suffix")
            );
            let mut rows = layout::text(
                &value,
                inner,
                if prop(node, "tone").is_empty() || prop(node, "tone") == "neutral" {
                    Style::default().add_modifier(Modifier::BOLD)
                } else {
                    color.add_modifier(Modifier::BOLD)
                },
            );
            let detail = prop(node, "detail");
            if !detail.is_empty() {
                rows.extend(layout::text(&detail, inner, theme::muted_style()));
            }
            let trend = prop(node, "trend");
            if !trend.is_empty() {
                rows.extend(layout::text(&format!("↗ {trend}"), inner, color));
            }
            layout::frame(rows, &label, width, theme::muted_style())
        }
        UiKind::Progress => {
            let value = node
                .props
                .get("value")
                .and_then(Value::as_f64)
                .unwrap_or(0.0);
            let max = node
                .props
                .get("max")
                .and_then(Value::as_f64)
                .unwrap_or(100.0)
                .max(f64::EPSILON);
            let ratio = (value / max).clamp(0.0, 1.0);
            let mut rows = vec![
                layout::split_label(
                    &label,
                    &format!("{:.0}%", ratio * 100.0),
                    width,
                    Style::default(),
                    color.add_modifier(Modifier::BOLD),
                ),
                RenderedLine::rich(layout::gauge(ratio, usize::from(width), color, false)),
            ];
            let detail = prop(node, "detail");
            if !detail.is_empty() {
                rows.extend(layout::text(&detail, width, theme::muted_style()));
            }
            rows
        }
        UiKind::Tabs => {
            let mut rows = title_rows(&label, width);
            let mut spans = Vec::new();
            let mut controls = Vec::new();
            let mut col = 0;
            if let Some(field) = &node.field {
                for option in &field.options {
                    let selected = node.value.as_str() == Some(option.value.as_str());
                    let text = format!(" {}{} ", if selected { "● " } else { "" }, option.label);
                    let text = agena_tui_components::truncate_display_text(
                        &layout::single_line(&text),
                        usize::from(width),
                    );
                    let size = text.width();
                    if col > 0 && col + size + 1 > usize::from(width) {
                        let mut line = RenderedLine::rich(Line::from(std::mem::take(&mut spans)));
                        line.controls = std::mem::take(&mut controls);
                        rows.push(line);
                        col = 0;
                    }
                    if col > 0 {
                        spans.push(Span::styled("│", theme::muted_style()));
                        col += 1;
                    }
                    controls.push(RenderedTranscriptControl {
                        columns: col..col + size,
                        action: ctx.control(&node.id, InlineEvent::Choose(option.value.clone())),
                        enabled: enabled(node),
                    });
                    spans.push(Span::styled(
                        text,
                        if selected {
                            theme::selection_style().add_modifier(Modifier::BOLD)
                        } else {
                            theme::muted_style()
                        },
                    ));
                    col += size;
                }
            }
            if !spans.is_empty() {
                let mut line = RenderedLine::rich(Line::from(spans));
                line.controls = controls;
                rows.push(line);
            }
            rows
        }
        UiKind::Select => {
            let current = node
                .field
                .as_ref()
                .and_then(|field| {
                    field
                        .options
                        .iter()
                        .find(|o| Some(o.value.as_str()) == node.value.as_str())
                })
                .map(|o| o.label.clone())
                .unwrap_or_else(|| display(&node.value));
            let mut rows = title_rows(&label, width);
            rows.extend(action(
                node,
                ctx,
                InlineEvent::Activate,
                layout::text(&format!("[ {current} ▾ ]"), width, color),
            ));
            let desc = prop(node, "description");
            if !desc.is_empty() {
                rows.extend(layout::text(&desc, width, theme::muted_style()));
            }
            rows
        }
        UiKind::Slider => {
            let min = node
                .field
                .as_ref()
                .and_then(|f| f.min.as_ref())
                .and_then(|n| n.as_f64())
                .unwrap_or(0.0);
            let max = node
                .field
                .as_ref()
                .and_then(|f| f.max.as_ref())
                .and_then(|n| n.as_f64())
                .unwrap_or(100.0);
            let value = node.value.as_f64().unwrap_or(min);
            let ratio = if max > min {
                (value - min) / (max - min)
            } else {
                0.0
            };
            let mut rows = vec![layout::split_label(
                &label,
                &format!(
                    "{}{}{}",
                    prop(node, "prefix"),
                    display(&node.value),
                    prop(node, "suffix")
                ),
                width,
                Style::default(),
                color.add_modifier(Modifier::BOLD),
            )];
            rows.extend(action(
                node,
                ctx,
                InlineEvent::Slider,
                vec![RenderedLine::rich(layout::gauge(
                    ratio,
                    usize::from(width),
                    color,
                    true,
                ))],
            ));
            let desc = prop(node, "description");
            if !desc.is_empty() {
                rows.extend(layout::text(&desc, width, theme::muted_style()));
            }
            rows
        }
        UiKind::Number | UiKind::Input => {
            let mut rows = title_rows(&label, width);
            let value = display(&node.value);
            let value = if value.is_empty() {
                prop(node, "placeholder")
            } else {
                value
            };
            rows.extend(action(
                node,
                ctx,
                InlineEvent::Activate,
                layout::text(
                    &format!(
                        "[ {}{}{} ▏ ]",
                        prop(node, "prefix"),
                        value,
                        prop(node, "suffix")
                    ),
                    width,
                    color,
                ),
            ));
            let desc = prop(node, "description");
            if !desc.is_empty() {
                rows.extend(layout::text(&desc, width, theme::muted_style()));
            }
            rows
        }
        UiKind::Checkbox | UiKind::Switch => {
            let checked = node.value.as_bool().unwrap_or(false);
            let mark = if node.kind == UiKind::Switch {
                if checked { "[● ON ]" } else { "[ OFF ○]" }
            } else if checked {
                "[✓]"
            } else {
                "[ ]"
            };
            let mut rows = action(
                node,
                ctx,
                InlineEvent::Activate,
                layout::text(
                    &format!("{mark} {label}"),
                    width,
                    if checked { color } else { Style::default() },
                ),
            );
            let desc = prop(node, "description");
            if !desc.is_empty() {
                rows.extend(layout::text(&desc, width, theme::muted_style()));
            }
            rows
        }
        UiKind::Button => action(
            node,
            ctx,
            InlineEvent::Activate,
            layout::text(
                &format!("[ {label} ]"),
                width,
                color.add_modifier(Modifier::BOLD),
            ),
        ),
        UiKind::Tasks => {
            let mut rows = title_rows(&title, width);
            if let Some(items) = node.props.get("items").and_then(Value::as_array) {
                for item in items {
                    let id = item.get("id").map(display).unwrap_or_default();
                    let checked = node
                        .value
                        .as_array()
                        .is_some_and(|values| values.contains(&Value::String(id.clone())));
                    let title = item.get("title").map(display).unwrap_or_default();
                    let mut item_rows = layout::text(
                        &format!("{} {title}", if checked { "[✓]" } else { "[ ]" }),
                        width,
                        if checked {
                            Style::default().fg(theme::success_color())
                        } else {
                            Style::default()
                        },
                    );
                    if let Some(detail) = item.get("detail").and_then(Value::as_str) {
                        item_rows.extend(prefix_rows(
                            layout::text(
                                detail,
                                width.saturating_sub(4).max(1),
                                theme::muted_style(),
                            ),
                            "    ",
                        ));
                    }
                    rows.extend(action(node, ctx, InlineEvent::Task(id), item_rows));
                }
            }
            rows
        }
        UiKind::Details => {
            let open = ctx.presentation.open.contains(&node.id);
            let mut rows = action(
                node,
                ctx,
                InlineEvent::Details,
                layout::text(
                    &format!("{} {title}", if open { "▾" } else { "▸" }),
                    width,
                    Style::default().add_modifier(Modifier::BOLD),
                ),
            );
            if open {
                rows.extend(prefix_rows(
                    stack(&node.children, width.saturating_sub(2).max(1), ctx, 1),
                    "  ",
                ));
            }
            rows
        }
        UiKind::Timeline => {
            let mut rows = title_rows(&title, width);
            if let Some(items) = node.props.get("items").and_then(Value::as_array) {
                for (index, item) in items.iter().enumerate() {
                    rows.extend(layout::text(
                        &format!(
                            "{} {}",
                            if index == 0 { "●" } else { "○" },
                            item.get("title").map(display).unwrap_or_default()
                        ),
                        width,
                        Style::default(),
                    ));
                    if let Some(detail) = item.get("detail").and_then(Value::as_str) {
                        rows.extend(prefix_rows(
                            layout::text(
                                detail,
                                width.saturating_sub(2).max(1),
                                theme::muted_style(),
                            ),
                            "│ ",
                        ));
                    }
                }
            }
            rows
        }
        UiKind::Callout => {
            let mut rows = title_rows(
                &format!(
                    "{} {title}",
                    match prop(node, "tone").as_str() {
                        "warning" | "danger" => "!",
                        "success" => "✓",
                        _ => "i",
                    }
                ),
                width.saturating_sub(2).max(1),
            );
            rows.extend(layout::text(
                &prop(node, "text"),
                width.saturating_sub(2).max(1),
                Style::default(),
            ));
            let mut rows = prefix_rows(rows, "▌ ");
            for row in &mut rows {
                if let Some(line) = &mut row.rich_line {
                    line.spans[0].style = color;
                }
            }
            rows
        }
        UiKind::Table => {
            let mut out = title_rows(&title, width);
            let columns = node
                .props
                .get("columns")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let headings = columns
                .iter()
                .map(|c| c.get("label").map(display).unwrap_or_default())
                .collect::<Vec<_>>();
            let rows = node
                .props
                .get("rows")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(|row| {
                    columns
                        .iter()
                        .map(|c| {
                            row.get(c.get("key").and_then(Value::as_str).unwrap_or(""))
                                .map(display)
                                .unwrap_or_default()
                        })
                        .collect()
                })
                .collect::<Vec<Vec<String>>>();
            out.extend(layout::table(&headings, &rows, width));
            out
        }
        UiKind::Chart => charts::render_chart(&node.id, &node.props, width, ctx),
    }
}

fn node(kind: UiKind, id: &str, props: Value) -> UiResolvedNode {
    UiResolvedNode {
        id: id.into(),
        kind,
        props: serde_json::from_value(props).unwrap_or_default(),
        children: Vec::new(),
        bind: None,
        value: Value::Null,
        field: None,
        action: None,
    }
}
pub(super) fn render_static(
    block: &InlineBlock,
    width: u16,
    ctx: &Context<'_>,
) -> Vec<RenderedLine> {
    match block {
        InlineBlock::Paragraph { text, .. } => layout::text(text, width, Style::default()),
        InlineBlock::Callout {
            id,
            title,
            tone,
            text,
        } => render_node(
            &node(
                UiKind::Callout,
                id,
                json!({"title":title,"text":text,"tone":match tone{agena_domain::InlineTone::Warning=>"warning",agena_domain::InlineTone::Tip=>"success",_=>"info"}}),
            ),
            width,
            ctx,
        ),
        InlineBlock::Steps { title, items, .. } => {
            let mut out = title_rows(title, width);
            for (index, item) in items.iter().enumerate() {
                let icon = match item.icon {
                    agena_domain::InlineIcon::Shield => "◇",
                    agena_domain::InlineIcon::Layers => "▣",
                    agena_domain::InlineIcon::Sparkles => "✧",
                    agena_domain::InlineIcon::Link => "↔",
                    agena_domain::InlineIcon::Check => "✓",
                    agena_domain::InlineIcon::Search => "⌕",
                    agena_domain::InlineIcon::Code => "‹›",
                    _ => "•",
                };
                let label = if item.label.is_empty() {
                    (index + 1).to_string()
                } else {
                    item.label.clone()
                };
                if index > 0 {
                    out.push(layout::rule(width));
                }
                out.extend(layout::text(
                    &format!("{icon} {label} · {}", item.title),
                    width,
                    Style::default().add_modifier(Modifier::BOLD),
                ));
                out.extend(layout::text(&item.text, width, Style::default()));
            }
            out
        }
        InlineBlock::Comparison {
            title,
            columns,
            rows,
            ..
        } => {
            let mut headings = vec![String::new()];
            headings.extend(columns.clone());
            let values = rows
                .iter()
                .map(|row| {
                    let mut cells = vec![row.label.clone()];
                    cells.extend(row.values.clone());
                    cells
                })
                .collect::<Vec<_>>();
            let mut out = title_rows(title, width);
            out.extend(layout::table(&headings, &values, width));
            out
        }
        InlineBlock::Metrics { title, items, .. } => {
            let children = items
                .iter()
                .enumerate()
                .map(|(i, item)| {
                    node(
                        UiKind::Metric,
                        &format!("metric{i}"),
                        json!({"label":item.label,"value":item.value,"detail":item.detail}),
                    )
                })
                .collect::<Vec<_>>();
            let mut out = title_rows(title, width);
            out.extend(grid(&children, children.len().min(4), width, ctx));
            out
        }
        InlineBlock::Table {
            title,
            columns,
            rows,
            ..
        } => {
            let mut out = title_rows(title, width);
            out.extend(layout::table(columns, rows, width));
            out
        }
        InlineBlock::Details { id, title, text } => {
            let mut n = node(UiKind::Details, id, json!({"title":title}));
            n.children
                .push(node(UiKind::Text, "text", json!({"text":text})));
            render_node(&n, width, ctx)
        }
        InlineBlock::Chart {
            id,
            title,
            chart_type,
            labels,
            series,
            x_label,
            y_label,
        } => render_node(
            &node(
                UiKind::Chart,
                id,
                json!({"title":title,"chart_type":chart_type,"labels":labels,"series":series,"x_label":x_label,"y_label":y_label}),
            ),
            width,
            ctx,
        ),
        InlineBlock::Panel { .. } => Vec::new(),
    }
}
