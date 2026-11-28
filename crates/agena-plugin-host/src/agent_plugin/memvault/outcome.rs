//! The MemVault outcome contract, projected from Agena session history.

use std::collections::HashSet;

use agena_plugin_sdk::host_api::SessionHistoryEvent;
use serde_json::{Value, json};

// These signals match the public dsh-memvault outcome-evidence tracker.
const REJECTIONS: &[&str] = &[
    "doesn't want to proceed",
    "don't want to proceed",
    "tool call was rejected",
    "tool use was rejected",
    "user rejected this",
    "user denied",
];
const CORRECTIONS: &[&str] = &[
    "no, ",
    "not what i",
    "that's wrong",
    "that's not right",
    "stop, ",
    "stop.",
    "wait, ",
    "actually, ",
    "revert",
    "undo that",
    "roll that back",
    "不对",
    "不是这样",
    "不是这个意思",
    "停一下",
    "等一下",
    "撤销",
    "改回去",
    "别这样",
];

#[derive(Default)]
pub(super) struct OutcomeEvidence {
    task: Option<String>,
    last_user: String,
    last_assistant: String,
    human_turns: u64,
    retries: u64,
    rejections: u64,
    corrections: u64,
    failing_tools: HashSet<String>,
    terminal: Terminal,
}

#[derive(Default)]
enum Terminal {
    #[default]
    Resolved,
    ToolError(String),
    Correction(String),
}

impl OutcomeEvidence {
    pub(super) fn observe(&mut self, event: SessionHistoryEvent) {
        match event {
            SessionHistoryEvent::ContextCleared => *self = Self::default(),
            SessionHistoryEvent::User(text) => {
                self.human_turns += 1;
                if self.task.is_none() && !text.trim().is_empty() {
                    self.task = Some(text.trim().to_owned());
                }
                self.terminal = if self.human_turns > 1 && matches_phrase(&text, CORRECTIONS) {
                    self.corrections += 1;
                    Terminal::Correction(detail(&text))
                } else {
                    Terminal::Resolved
                };
                self.last_user = text;
            }
            SessionHistoryEvent::Assistant(text) => {
                self.last_assistant = text;
                self.terminal = Terminal::Resolved;
            }
            SessionHistoryEvent::ToolCompleted {
                name,
                output,
                failed,
                user_declined,
            } => {
                if failed {
                    if !self.failing_tools.insert(name.clone()) {
                        self.retries += 1;
                    }
                } else {
                    self.failing_tools.remove(&name);
                }
                let rejected = user_declined || matches_phrase(&output, REJECTIONS);
                if rejected {
                    self.rejections += 1;
                }
                self.terminal = if failed || rejected {
                    Terminal::ToolError(format!("{name}: {}", detail(&output)))
                } else {
                    Terminal::Resolved
                };
            }
        }
    }

    pub(super) fn arguments(&self, agent_id: &str) -> Option<Value> {
        let task = self.task.as_deref()?;
        let (terminal, terminal_detail) = match &self.terminal {
            Terminal::Resolved => ("resolved", None),
            Terminal::ToolError(detail) => ("unresolved_tool_error", Some(detail)),
            Terminal::Correction(detail) => ("unresolved_correction", Some(detail)),
        };
        let mut judge_context = format!("user: {task}");
        if !self.last_user.is_empty() && self.last_user != task {
            judge_context.push_str("\nuser: ");
            judge_context.push_str(&self.last_user);
        }
        if !self.last_assistant.is_empty() {
            judge_context.push_str("\nassistant: ");
            judge_context.push_str(&self.last_assistant);
        }
        let mut evidence = json!({
            "task": task,
            "retry_hits": self.retries,
            "rejection_hits": self.rejections,
            "correction_hits": self.corrections,
            "terminal": terminal,
        });
        if let Some(detail) = terminal_detail {
            evidence["terminal_detail"] = json!(detail);
        }
        Some(json!({
            "agent_id": agent_id,
            "evidence": evidence,
            "judge_context": judge_context,
        }))
    }
}

fn matches_phrase(text: &str, phrases: &[&str]) -> bool {
    let text = text.to_lowercase();
    phrases.iter().any(|phrase| text.contains(phrase))
}

fn detail(text: &str) -> String {
    match text.char_indices().nth(160) {
        Some((end, _)) => format!("{}…", &text[..end]),
        None => text.to_owned(),
    }
}
