//! Plan document rendering shared with the plan viewer route.
//!
//! The composer carries only a plan *progress* chip; the document itself is a
//! full-screen surface (`Route::PlanViewer`).

use agena_tui_transcript::render_markdown_document;
use ratatui::text::{Line, Span};

/// Project the canonical plan payload into the same progress label as WebUI.
pub(crate) fn plan_progress(payload: Option<&serde_json::Value>) -> Option<String> {
    let plan = payload?
        .get("plan")?
        .as_object()
        .filter(|plan| !plan.is_empty())?;
    let symbol = match plan.get("phase").and_then(serde_json::Value::as_str) {
        Some("completed") => "✓",
        Some("blocked") => "⚠",
        Some("cancelled") => "✕",
        Some("planning") => "⏳",
        _ => "▶",
    };
    let mut parts = vec![symbol.to_owned()];
    if let Some(steps) = plan.get("steps").and_then(serde_json::Value::as_array)
        && !steps.is_empty()
    {
        let completed = steps
            .iter()
            .filter(|step| {
                matches!(
                    step.get("status").and_then(serde_json::Value::as_str),
                    Some("completed" | "skipped")
                )
            })
            .count();
        parts.push(format!("{completed}/{}", steps.len()));
    }
    if plan.get("autorun").and_then(serde_json::Value::as_bool) == Some(true) {
        parts.push("↻".into());
    }
    Some(parts.join(" "))
}

/// Strip the transport prefix (`Revision: …`) from a `plan.get` tool result.
pub(crate) fn plan_document(output: &str) -> String {
    if output.starts_with("Revision:") {
        output
            .split_once('\n')
            .map_or("", |(_, body)| body)
            .trim()
            .to_owned()
    } else {
        output.trim().to_owned()
    }
}

/// Render plan markdown with the transcript markdown renderer.
pub(crate) fn render_plan_document(markdown: &str, width: u16) -> Vec<Line<'static>> {
    render_markdown_document(markdown, width)
        .into_iter()
        .map(|line| {
            line.rich_line
                .unwrap_or_else(|| Line::from(Span::styled(line.text, line.style)))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn progress_counts_completed_and_skipped_steps_and_autorun() {
        let payload = json!({"plan": {
            "phase": "active", "autorun": true,
            "steps": [{"status": "completed"}, {"status": "skipped"},
                      {"status": "in_progress"}, {"status": "pending"}]
        }});
        assert_eq!(plan_progress(Some(&payload)).as_deref(), Some("▶ 2/4 ↻"));
    }

    #[test]
    fn progress_handles_each_phase_without_steps() {
        for (phase, symbol) in [
            ("planning", "⏳"),
            ("active", "▶"),
            ("blocked", "⚠"),
            ("completed", "✓"),
            ("cancelled", "✕"),
        ] {
            let payload = json!({"plan": {"phase": phase, "steps": [], "autorun": false}});
            assert_eq!(plan_progress(Some(&payload)).as_deref(), Some(symbol));
        }
    }

    #[test]
    fn absent_or_cleared_plan_has_no_progress() {
        assert_eq!(plan_progress(None), None);
        for payload in [json!({}), json!({"plan": null}), json!({"plan": {}})] {
            assert_eq!(plan_progress(Some(&payload)), None);
        }
    }
}
