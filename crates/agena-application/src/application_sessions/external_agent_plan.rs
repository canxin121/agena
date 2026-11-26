use std::{collections::BTreeMap, fmt::Write as _};

use agena_domain::ExternalAgent;
use agena_plugin_host::{PluginToolInvokeResponse, PluginToolInvokeStatus};
use agena_storage::kernel::{ExternalAgentPlan, ExternalAgentPlanStatus};
use serde_json::json;

pub(super) fn response(
    agent: ExternalAgent,
    plans: &BTreeMap<String, ExternalAgentPlan>,
) -> PluginToolInvokeResponse {
    use ExternalAgentPlanStatus::{Completed, InProgress, Pending};

    let title = format!("{} plan", agent.label());
    let entries = plans
        .values()
        .filter_map(|plan| match plan {
            ExternalAgentPlan::Items { entries } => Some(entries.as_slice()),
            _ => None,
        })
        .flatten()
        .collect::<Vec<_>>();
    let phase = if entries.is_empty() {
        "planning"
    } else if entries.iter().all(|entry| entry.status == Completed) {
        "completed"
    } else if entries.iter().all(|entry| entry.status == Pending) {
        "planning"
    } else {
        "active"
    };
    let steps = entries
        .iter()
        .map(|entry| json!({"title": entry.content, "status": entry.status, "priority": entry.priority}))
        .collect::<Vec<_>>();
    let current = [InProgress, Pending]
        .into_iter()
        .find_map(|status| entries.iter().find(|entry| entry.status == status))
        .map(|entry| json!({"title": entry.content}));

    let mut document = String::new();
    for (id, plan) in plans {
        if plans.len() > 1 {
            let label = if id.is_empty() { "Checklist" } else { id };
            let _ = writeln!(document, "### {}\n", markdown_label(label));
        }
        match plan {
            ExternalAgentPlan::Items { entries } => {
                for entry in entries {
                    let marker = match entry.status {
                        Pending => "[ ]",
                        InProgress => "▶",
                        Completed => "[x]",
                    };
                    let label = entry
                        .content
                        .lines()
                        .map(str::trim)
                        .collect::<Vec<_>>()
                        .join(" ");
                    let _ = writeln!(document, "- {marker} {label}");
                }
            }
            ExternalAgentPlan::Markdown { content } => {
                let _ = writeln!(document, "{content}");
            }
        }
        document.push('\n');
    }

    PluginToolInvokeResponse {
        plugin_id: "agena.plan".parse().expect("static plan tool identity"),
        tool: "plan.get".into(),
        status: PluginToolInvokeStatus::Completed,
        title: title.clone(),
        output_text: document.trim_end().to_owned(),
        payload: Some(json!({
            "plan": {"title": title, "phase": phase, "steps": steps},
            "current_step": current,
        })),
        metadata: BTreeMap::from([("external_agent".into(), agent.id().into())]),
    }
}

fn markdown_label(label: &str) -> String {
    let mut result = String::new();
    for ch in label.chars() {
        if ch.is_ascii_punctuation() {
            result.push('\\');
        }
        if ch.is_control() {
            result.push(' ');
        } else {
            result.push(ch);
        }
    }
    result
}
