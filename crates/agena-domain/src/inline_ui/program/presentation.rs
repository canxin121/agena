use super::expression::display;
use super::*;

pub(super) fn validate_props(kind: UiKind, props: &BTreeMap<String, Value>) -> Result<(), String> {
    if kind.is_control() && kind != UiKind::Tasks {
        let label = props
            .get("label")
            .and_then(Value::as_str)
            .ok_or("controls require a text label")?;
        super::super::text(label, "control label", 160, true)?;
    }
    for (k, v) in props {
        if let Some(text) = v.as_str() {
            super::super::text(text, "resolved UI text", 6000, false)?;
        }
        match k.as_str() {
            "disabled" => {
                if !v.is_boolean() {
                    return Err("disabled must be boolean".into());
                }
            }
            "columns" if kind == UiKind::Grid => {
                if !v.as_u64().is_some_and(|n| (1..=4).contains(&n)) {
                    return Err("grid columns must be 1..4".into());
                }
            }
            "tone" => {
                if !v.as_str().is_some_and(|s| {
                    ["neutral", "info", "success", "warning", "danger", "primary"].contains(&s)
                }) {
                    return Err("invalid semantic tone".into());
                }
            }
            "gap" => {
                if !v.as_str().is_some_and(|s| ["sm", "md", "lg"].contains(&s)) {
                    return Err("invalid spacing token".into());
                }
            }
            "size" => {
                if !v.as_str().is_some_and(|s| ["sm", "md", "lg"].contains(&s)) {
                    return Err("invalid text size".into());
                }
            }
            "value" | "max" | "trend" if matches!(kind, UiKind::Progress) => {
                if !v.as_f64().is_some_and(|n| n.is_finite() && n >= 0.0) {
                    return Err("progress value/max must be nonnegative numbers".into());
                }
            }
            "columns" if kind == UiKind::Table => {
                let list = v.as_array().ok_or("columns must be an array")?;
                if list.is_empty()
                    || list.len() > 8
                    || list.iter().any(|c| {
                        c.get("key")
                            .and_then(Value::as_str)
                            .is_none_or(|s| !ui_identifier(s))
                            || c.get("label").and_then(Value::as_str).is_none()
                    })
                {
                    return Err("table needs 1..8 named columns".into());
                }
            }
            "rows" => {
                if !v
                    .as_array()
                    .is_some_and(|rows| rows.len() <= 256 && rows.iter().all(Value::is_object))
                {
                    return Err("rows must be up to 256 objects".into());
                }
            }
            "items" => {
                let rows = v.as_array().ok_or("items must be an array")?;
                if rows.len() > 128 {
                    return Err("items exceed 128".into());
                }
                let mut ids = BTreeSet::new();
                for item in rows {
                    let id = item
                        .get("id")
                        .and_then(Value::as_str)
                        .ok_or("item requires id")?;
                    if !ui_identifier(id) || !ids.insert(id) {
                        return Err("item id invalid or duplicate".into());
                    }
                    if item.get("title").and_then(Value::as_str).is_none() {
                        return Err("item requires title".into());
                    }
                }
            }
            "labels" | "series" | "chart_type" => {}
            _ => {
                if !v.is_null() && !v.is_string() && !v.is_number() {
                    return Err(format!("{k} requires text or number"));
                }
            }
        }
    }
    if kind == UiKind::Chart {
        if props
            .get("labels")
            .and_then(Value::as_array)
            .is_some_and(Vec::is_empty)
        {
            if props
                .get("series")
                .and_then(Value::as_array)
                .is_some_and(|series| {
                    series.iter().all(|s| {
                        s.get("values")
                            .and_then(Value::as_array)
                            .is_some_and(Vec::is_empty)
                    })
                })
            {
                return Ok(());
            }
            return Err("empty chart labels require empty values".into());
        }
        let chart = json!({"version":1,"blocks":[{"id":"chart","type":"chart","title":props.get("title").cloned().unwrap_or(json!("Chart")),"chart_type":props.get("chart_type").cloned().unwrap_or(json!("line")),"labels":props.get("labels").cloned().unwrap_or(json!([])),"series":props.get("series").cloned().unwrap_or(json!([])),"x_label":props.get("x_label").cloned().unwrap_or(json!("")),"y_label":props.get("y_label").cloned().unwrap_or(json!(""))}]});
        super::super::InlineDocument::from_value(chart)?;
    }
    Ok(())
}
fn md(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if "\\`*_{}[]<>()#+-.!|>".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}
fn text(node: &UiResolvedNode, key: &str) -> String {
    node.props.get(key).map(display).unwrap_or_default()
}
impl UiView {
    /// Exact, copyable terminal fallback for the currently selected state.
    pub fn to_markdown(&self) -> String {
        self.root.to_markdown(0)
    }
}
impl UiResolvedNode {
    fn to_markdown(&self, depth: usize) -> String {
        let p = |key| text(self, key);
        let label = p("label");
        let mut parts = Vec::new();
        let title = p("title");
        if !title.is_empty() {
            parts.push(format!("{} {}", "#".repeat((depth + 3).min(6)), md(&title)));
        }
        let body = match self.kind {
            UiKind::Stack | UiKind::Row | UiKind::Grid | UiKind::Card => md(&p("description")),
            UiKind::Text => md(&p("text")),
            UiKind::Heading => format!("#### {}", md(&p("text"))),
            UiKind::Metric => format!(
                "**{}:** {}{}{}{}",
                md(&label),
                md(&p("prefix")),
                md(&p("value")),
                md(&p("suffix")),
                if p("detail").is_empty() {
                    String::new()
                } else {
                    format!(" — {}", md(&p("detail")))
                }
            ),
            UiKind::Badge => format!("**{}**", md(&p("text"))),
            UiKind::Divider => "---".into(),
            UiKind::Tabs | UiKind::Select => {
                let value = self.value.as_str().unwrap_or("");
                let name = self
                    .field
                    .as_ref()
                    .and_then(|f| f.options.iter().find(|o| o.value == value))
                    .map(|o| o.label.as_str())
                    .unwrap_or(value);
                format!("**{}:** {}", md(&label), md(name))
            }
            UiKind::Checkbox | UiKind::Switch => format!(
                "[{}] {}",
                if self.value.as_bool() == Some(true) {
                    "x"
                } else {
                    " "
                },
                md(&label)
            ),
            UiKind::Slider | UiKind::Number | UiKind::Input => format!(
                "**{}:** {}{}{}",
                md(&label),
                md(&p("prefix")),
                md(&display(&self.value)),
                md(&p("suffix"))
            ),
            UiKind::Button => format!("[ {} ]", md(&label)),
            UiKind::Progress => {
                let value = self
                    .props
                    .get("value")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0);
                let max = self
                    .props
                    .get("max")
                    .and_then(Value::as_f64)
                    .unwrap_or(100.0);
                let ratio = if max > 0.0 {
                    (value / max).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let n = (ratio * 16.0).round() as usize;
                format!(
                    "**{}:** {}{} {:.0}%{}",
                    md(&label),
                    "■".repeat(n),
                    "□".repeat(16 - n),
                    ratio * 100.0,
                    if p("detail").is_empty() {
                        String::new()
                    } else {
                        format!(" — {}", md(&p("detail")))
                    }
                )
            }
            UiKind::Table => {
                let columns = self
                    .props
                    .get("columns")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                let rows = self
                    .props
                    .get("rows")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                let mut lines = vec![
                    format!(
                        "| {} |",
                        columns
                            .iter()
                            .map(|c| md(c["label"].as_str().unwrap_or("")))
                            .collect::<Vec<_>>()
                            .join(" | ")
                    ),
                    format!("| {} |", vec!["---"; columns.len()].join(" | ")),
                ];
                for row in rows {
                    lines.push(format!(
                        "| {} |",
                        columns
                            .iter()
                            .map(|c| md(&display(
                                row.get(c["key"].as_str().unwrap_or(""))
                                    .unwrap_or(&Value::Null)
                            ))
                            .replace('\n', " / "))
                            .collect::<Vec<_>>()
                            .join(" | ")
                    ));
                }
                lines.join("\n")
            }
            UiKind::Tasks | UiKind::Timeline => {
                let rows = self
                    .props
                    .get("items")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                let selected = self.value.as_array().cloned().unwrap_or_default();
                rows.iter()
                    .map(|row| {
                        format!(
                            "- {}**{}**{}",
                            if self.kind == UiKind::Tasks {
                                format!(
                                    "[{}] ",
                                    if selected.contains(&row["id"]) {
                                        "x"
                                    } else {
                                        " "
                                    }
                                )
                            } else {
                                String::new()
                            },
                            md(row["title"].as_str().unwrap_or("")),
                            row.get("detail")
                                .map(|d| format!(" — {}", md(&display(d))))
                                .unwrap_or_default()
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            }
            UiKind::Callout => format!("> {}", md(&p("text")).replace('\n', "\n> ")),
            UiKind::Details => String::new(),
            UiKind::Chart => {
                let labels = self
                    .props
                    .get("labels")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                let series = self
                    .props
                    .get("series")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                let columns = std::iter::once(md(&p("x_label")))
                    .chain(series.iter().map(|s| md(s["name"].as_str().unwrap_or(""))))
                    .collect::<Vec<_>>();
                let mut lines = vec![
                    format!("| {} |", columns.join(" | ")),
                    format!("| {} |", vec!["---"; columns.len()].join(" | ")),
                ];
                for (i, label) in labels.iter().enumerate() {
                    let cells = std::iter::once(md(&display(label)))
                        .chain(series.iter().map(|s| md(&display(&s["values"][i]))))
                        .collect::<Vec<_>>();
                    lines.push(format!("| {} |", cells.join(" | ")));
                }
                if !p("y_label").is_empty() {
                    lines.insert(0, md(&p("y_label")));
                }
                lines.join("\n")
            }
        };
        if !body.is_empty() {
            parts.push(body);
        }
        for child in &self.children {
            parts.push(child.to_markdown(
                depth + usize::from(matches!(self.kind, UiKind::Card | UiKind::Details)),
            ));
        }
        parts.join("\n\n")
    }
}
