//! Inline Generative UI: bounded, inert semantic content inside an assistant reply.
//!
//! Documents are immutable tool results, not applications. They contain no HTML,
//! executable code, URLs, commands, remote data bindings, or runtime state. Both
//! clients render the same facts; layout and disclosure remain client-local.
mod markdown;
mod references;
pub use references::*;
#[cfg(test)]
mod tests;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Number, Value};
use std::collections::BTreeSet;

pub const INLINE_UI_VERSION: u8 = 1;
// Reserve envelope/metadata space below the runtime’s 64 KiB inline payload limit.
pub const INLINE_UI_MAX_BYTES: usize = 60 * 1024;
pub const INLINE_UI_MAX_BLOCKS: usize = 24;
pub const INLINE_UI_TOOL: &str = "agena.inline_ui.present";

/// Recognize only the owning tool's canonical or normal execution name.
pub fn is_inline_ui_tool(name: &str) -> bool {
    matches!(name, "agena.inline_ui.present" | "inline_ui.present")
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InlineDocument {
    /// Protocol version. Currently exactly 1; unknown versions are rejected.
    pub version: u8,
    /// Optional heading for the whole passage, not a separate app/window title.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    /// Ordered semantic content. Each id is unique within this document.
    pub blocks: Vec<InlineBlock>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum InlineBlock {
    Paragraph {
        id: String,
        text: String,
    },
    Callout {
        id: String,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        title: String,
        #[serde(default)]
        tone: InlineTone,
        text: String,
    },
    Steps {
        id: String,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        title: String,
        items: Vec<InlineStep>,
    },
    Comparison {
        id: String,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        title: String,
        columns: Vec<String>,
        rows: Vec<InlineComparisonRow>,
    },
    Metrics {
        id: String,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        title: String,
        items: Vec<InlineMetric>,
    },
    Table {
        id: String,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        title: String,
        columns: Vec<String>,
        rows: Vec<Vec<String>>,
    },
    Details {
        id: String,
        title: String,
        text: String,
    },
    Chart {
        id: String,
        title: String,
        chart_type: InlineChartType,
        labels: Vec<String>,
        series: Vec<InlineSeries>,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        x_label: String,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        y_label: String,
    },
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum InlineTone {
    #[default]
    Note,
    Tip,
    Warning,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum InlineIcon {
    #[default]
    Number,
    Shield,
    Layers,
    Sparkles,
    Link,
    Check,
    Search,
    Code,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InlineStep {
    /// Optional short stage label, such as "P0". Does not imply execution status.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
    pub title: String,
    pub text: String,
    #[serde(default)]
    pub icon: InlineIcon,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InlineComparisonRow {
    pub label: String,
    pub values: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InlineMetric {
    pub label: String,
    /// A supplied fact including its unit. Never an inferred or invented value.
    pub value: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub detail: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum InlineChartType {
    Line,
    Bar,
    Area,
    Pie,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InlineSeries {
    pub name: String,
    pub values: Vec<Option<Number>>,
}

impl InlineBlock {
    pub fn id(&self) -> &str {
        match self {
            Self::Paragraph { id, .. }
            | Self::Callout { id, .. }
            | Self::Steps { id, .. }
            | Self::Comparison { id, .. }
            | Self::Metrics { id, .. }
            | Self::Table { id, .. }
            | Self::Details { id, .. }
            | Self::Chart { id, .. } => id,
        }
    }

    pub fn kind(&self) -> &'static str {
        match self {
            Self::Paragraph { .. } => "paragraph",
            Self::Callout { .. } => "callout",
            Self::Steps { .. } => "steps",
            Self::Comparison { .. } => "comparison",
            Self::Metrics { .. } => "metrics",
            Self::Table { .. } => "table",
            Self::Details { .. } => "details",
            Self::Chart { .. } => "chart",
        }
    }
}

fn text(value: &str, path: &str, max: usize, required: bool) -> Result<(), String> {
    if required && value.trim().is_empty() {
        return Err(format!("{path}: must not be empty"));
    }
    if value.chars().count() > max {
        return Err(format!("{path}: exceeds {max} characters"));
    }
    // Do not emit terminal escape/control sequences in either client or exports.
    if value
        .chars()
        .any(|c| c.is_control() && !matches!(c, '\n' | '\t'))
    {
        return Err(format!("{path}: control characters are not allowed"));
    }
    Ok(())
}
fn count(len: usize, path: &str, min: usize, max: usize) -> Result<(), String> {
    if !(min..=max).contains(&len) {
        return Err(format!("{path}: expected {min}..={max} items"));
    }
    Ok(())
}
fn strings(
    values: &[String],
    path: &str,
    min: usize,
    max: usize,
    max_text: usize,
) -> Result<(), String> {
    count(values.len(), path, min, max)?;
    for (i, value) in values.iter().enumerate() {
        text(value, &format!("{path}[{i}]"), max_text, true)?;
    }
    Ok(())
}

impl InlineDocument {
    /// Validate before publishing and again when projecting persisted content.
    pub fn from_value(value: Value) -> Result<Self, String> {
        let bytes = serde_json::to_vec(&value).map_err(|e| e.to_string())?;
        if bytes.len() > INLINE_UI_MAX_BYTES {
            return Err("inline document exceeds 60 KiB".into());
        }
        let document: Self =
            serde_json::from_value(value).map_err(|e| format!("invalid inline document: {e}"))?;
        document.validate()?;
        Ok(document)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.version != INLINE_UI_VERSION {
            return Err("version: expected 1".into());
        }
        if serde_json::to_vec(self).map_err(|e| e.to_string())?.len() > INLINE_UI_MAX_BYTES {
            return Err("inline document exceeds 60 KiB".into());
        }
        text(&self.title, "title", 160, false)?;
        count(self.blocks.len(), "blocks", 1, INLINE_UI_MAX_BLOCKS)?;
        let mut ids = BTreeSet::new();
        for (i, block) in self.blocks.iter().enumerate() {
            let path = format!("blocks[{i}]");
            let id = block.id();
            if id.is_empty()
                || id.len() > 80
                || !id
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-'))
            {
                return Err(format!(
                    "{path}.id: use 1..80 ASCII letters, numbers, underscores or hyphens"
                ));
            }
            if !ids.insert(id) {
                return Err(format!("{path}.id: duplicate id '{id}'"));
            }
            match block {
                InlineBlock::Paragraph { text: body, .. } => {
                    text(body, &format!("{path}.text"), 6000, true)?
                }
                InlineBlock::Callout {
                    title, text: body, ..
                } => {
                    text(title, &format!("{path}.title"), 160, false)?;
                    text(body, &format!("{path}.text"), 3000, true)?;
                }
                InlineBlock::Details {
                    title, text: body, ..
                } => {
                    text(title, &format!("{path}.title"), 160, true)?;
                    text(body, &format!("{path}.text"), 6000, true)?;
                }
                InlineBlock::Steps { title, items, .. } => {
                    text(title, &format!("{path}.title"), 160, false)?;
                    count(items.len(), &format!("{path}.items"), 1, 12)?;
                    for (j, item) in items.iter().enumerate() {
                        let p = format!("{path}.items[{j}]");
                        text(&item.label, &format!("{p}.label"), 24, false)?;
                        text(&item.title, &format!("{p}.title"), 160, true)?;
                        text(&item.text, &format!("{p}.text"), 3000, true)?;
                    }
                }
                InlineBlock::Comparison {
                    title,
                    columns,
                    rows,
                    ..
                } => {
                    text(title, &format!("{path}.title"), 160, false)?;
                    strings(columns, &format!("{path}.columns"), 2, 4, 120)?;
                    count(rows.len(), &format!("{path}.rows"), 1, 16)?;
                    for (j, row) in rows.iter().enumerate() {
                        let p = format!("{path}.rows[{j}]");
                        text(&row.label, &format!("{p}.label"), 160, true)?;
                        strings(
                            &row.values,
                            &format!("{p}.values"),
                            columns.len(),
                            columns.len(),
                            1500,
                        )?;
                    }
                }
                InlineBlock::Metrics { title, items, .. } => {
                    text(title, &format!("{path}.title"), 160, false)?;
                    count(items.len(), &format!("{path}.items"), 1, 6)?;
                    for (j, item) in items.iter().enumerate() {
                        let p = format!("{path}.items[{j}]");
                        text(&item.label, &format!("{p}.label"), 120, true)?;
                        text(&item.value, &format!("{p}.value"), 120, true)?;
                        text(&item.detail, &format!("{p}.detail"), 500, false)?;
                    }
                }
                InlineBlock::Table {
                    title,
                    columns,
                    rows,
                    ..
                } => {
                    text(title, &format!("{path}.title"), 160, false)?;
                    strings(columns, &format!("{path}.columns"), 1, 8, 120)?;
                    count(rows.len(), &format!("{path}.rows"), 1, 40)?;
                    for (j, row) in rows.iter().enumerate() {
                        count(
                            row.len(),
                            &format!("{path}.rows[{j}]"),
                            columns.len(),
                            columns.len(),
                        )?;
                        for (k, cell) in row.iter().enumerate() {
                            text(cell, &format!("{path}.rows[{j}][{k}]"), 1500, false)?;
                        }
                    }
                }
                InlineBlock::Chart {
                    title,
                    chart_type,
                    labels,
                    series,
                    x_label,
                    y_label,
                    ..
                } => {
                    text(title, &format!("{path}.title"), 160, true)?;
                    text(x_label, &format!("{path}.x_label"), 120, false)?;
                    text(y_label, &format!("{path}.y_label"), 120, false)?;
                    strings(labels, &format!("{path}.labels"), 1, 128, 100)?;
                    count(series.len(), &format!("{path}.series"), 1, 6)?;
                    let mut has_number = false;
                    for (j, serie) in series.iter().enumerate() {
                        let p = format!("{path}.series[{j}]");
                        text(&serie.name, &format!("{p}.name"), 120, true)?;
                        count(
                            serie.values.len(),
                            &format!("{p}.values"),
                            labels.len(),
                            labels.len(),
                        )?;
                        for value in serie.values.iter().flatten() {
                            let n = value
                                .as_f64()
                                .filter(|n| n.is_finite() && n.abs() <= 1e100)
                                .ok_or_else(|| {
                                    format!(
                                        "{p}.values: require finite numbers with magnitude <= 1e100"
                                    )
                                })?;
                            has_number = true;
                            if *chart_type == InlineChartType::Pie && n < 0.0 {
                                return Err(format!("{p}.values: pie values must be nonnegative"));
                            }
                        }
                    }
                    if !has_number {
                        return Err(format!("{path}.series: chart has no numeric data"));
                    }
                    if *chart_type == InlineChartType::Pie
                        && (series.len() != 1
                            || !series[0]
                                .values
                                .iter()
                                .flatten()
                                .any(|n| n.as_f64().is_some_and(|n| n > 0.0)))
                    {
                        return Err(format!(
                            "{path}.series: pie needs exactly one series with a positive total"
                        ));
                    }
                }
            }
        }
        Ok(())
    }
}
