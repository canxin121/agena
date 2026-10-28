use super::{InlineBlock, InlineDocument};

/// Escape semantic plain text so labels cannot inject Markdown/HTML or links.
fn escape(value: &str) -> String {
    let mut out = String::new();
    for c in value.chars() {
        if "\\`*_{}[]<>()#+-.!|>".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}
fn cell(value: &str) -> String {
    escape(value).replace('\n', " / ").replace('\t', " ")
}
fn table(columns: &[String], rows: &[Vec<String>]) -> String {
    let row = |values: &[String]| {
        format!(
            "| {} |",
            values
                .iter()
                .map(|s| cell(s))
                .collect::<Vec<_>>()
                .join(" | ")
        )
    };
    let mut lines = vec![
        row(columns),
        format!("| {} |", vec!["---"; columns.len()].join(" | ")),
    ];
    lines.extend(rows.iter().map(|r| row(r)));
    lines.join("\n")
}

impl InlineDocument {
    /// Lossless readable content for copy/export and terminal Markdown rendering.
    /// Disclosures are expanded; every chart includes its exact source table.
    pub fn to_markdown(&self) -> String {
        self.to_markdown_with_states(&Default::default())
    }

    pub fn to_markdown_with_states(
        &self,
        states: &std::collections::BTreeMap<String, super::UiState>,
    ) -> String {
        let mut sections = Vec::new();
        if !self.title.is_empty() {
            sections.push(format!("### {}", escape(&self.title)));
        }
        for block in &self.blocks {
            let (title, body) = match block {
                InlineBlock::Panel { id, program } => (
                    "",
                    match program.render(states.get(id).unwrap_or(&Default::default())) {
                        Ok(view) => view.to_markdown(),
                        Err(error) => format!("> UI unavailable: {}", escape(&error)),
                    },
                ),
                InlineBlock::Paragraph { text, .. } => ("", escape(text)),
                InlineBlock::Callout { title, text, .. } => {
                    let body = text
                        .lines()
                        .map(|s| format!("> {}", escape(s)))
                        .collect::<Vec<_>>()
                        .join("\n");
                    (title.as_str(), body)
                }
                InlineBlock::Steps { title, items, .. } => (
                    title.as_str(),
                    items
                        .iter()
                        .enumerate()
                        .map(|(i, item)| {
                            let label = if item.label.is_empty() {
                                String::new()
                            } else {
                                format!("{} · ", escape(&item.label))
                            };
                            format!(
                                "{}. **{}{}**\n\n{}",
                                i + 1,
                                label,
                                escape(&item.title),
                                item.text
                                    .lines()
                                    .map(|s| format!("   {}", escape(s)))
                                    .collect::<Vec<_>>()
                                    .join("\n")
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n\n"),
                ),
                InlineBlock::Comparison {
                    title,
                    columns,
                    rows,
                    ..
                } => {
                    let mut headings = vec![String::new()];
                    headings.extend(columns.iter().cloned());
                    let rows = rows
                        .iter()
                        .map(|r| {
                            let mut cells = vec![r.label.clone()];
                            cells.extend(r.values.iter().cloned());
                            cells
                        })
                        .collect::<Vec<_>>();
                    (title.as_str(), table(&headings, &rows))
                }
                InlineBlock::Metrics { title, items, .. } => (
                    title.as_str(),
                    items
                        .iter()
                        .map(|item| {
                            let detail = if item.detail.is_empty() {
                                String::new()
                            } else {
                                format!(" — {}", escape(&item.detail))
                            };
                            format!(
                                "- **{}:** {}{}",
                                escape(&item.label),
                                escape(&item.value),
                                detail
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n"),
                ),
                InlineBlock::Table {
                    title,
                    columns,
                    rows,
                    ..
                } => (title.as_str(), table(columns, rows)),
                InlineBlock::Details { title, text, .. } => (title.as_str(), escape(text)),
                InlineBlock::Chart {
                    title,
                    labels,
                    series,
                    x_label,
                    y_label,
                    ..
                } => {
                    let mut columns = vec![x_label.clone()];
                    columns.extend(series.iter().map(|s| {
                        if y_label.is_empty() {
                            s.name.clone()
                        } else {
                            format!("{} ({y_label})", s.name)
                        }
                    }));
                    let rows = labels
                        .iter()
                        .enumerate()
                        .map(|(i, label)| {
                            let mut row = vec![label.clone()];
                            row.extend(series.iter().map(|s| {
                                s.values
                                    .get(i)
                                    .and_then(Option::as_ref)
                                    .map_or_else(|| "—".into(), ToString::to_string)
                            }));
                            row
                        })
                        .collect::<Vec<_>>();
                    (title.as_str(), table(&columns, &rows))
                }
            };
            sections.push(if title.is_empty() {
                body
            } else {
                format!("#### {}\n\n{body}", escape(title))
            });
        }
        sections.join("\n\n")
    }
}
