//! Session-level cost / token aggregation.
//!
//! Walks a session's assistant messages, sums up their `MessageUsage`
//! totals, and produces a [`SessionCostSummary`] suitable for `/cost`-style
//! UI panels. Per-model breakdowns are preserved so a session that switched
//! providers mid-flight surfaces both contributions.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::message::{Message, MessageUsage};
use crate::role::Role;

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct ModelCostBreakdown {
    pub provider_id: String,
    pub model_id: String,
    pub turns: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_tokens: u64,
    pub cache_write_tokens: u64,
    pub cache_read_tokens: u64,
    pub total_cost_usd: f64,
}

impl ModelCostBreakdown {
    pub fn total_tokens(&self) -> u64 {
        self.input_tokens
            .saturating_add(self.output_tokens)
            .saturating_add(self.reasoning_tokens)
    }

    fn fold(&mut self, usage: &MessageUsage) {
        self.turns = self.turns.saturating_add(1);
        self.input_tokens = self.input_tokens.saturating_add(usage.input_tokens);
        self.output_tokens = self.output_tokens.saturating_add(usage.output_tokens);
        self.reasoning_tokens = self.reasoning_tokens.saturating_add(usage.reasoning_tokens);
        self.cache_write_tokens = self
            .cache_write_tokens
            .saturating_add(usage.cache_write_tokens);
        self.cache_read_tokens = self
            .cache_read_tokens
            .saturating_add(usage.cache_read_tokens);
        self.total_cost_usd += usage.total_cost;
    }
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct SessionCostSummary {
    pub turns: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_tokens: u64,
    pub cache_write_tokens: u64,
    pub cache_read_tokens: u64,
    pub total_cost_usd: f64,
    pub by_model: Vec<ModelCostBreakdown>,
}

impl SessionCostSummary {
    pub fn total_tokens(&self) -> u64 {
        self.input_tokens
            .saturating_add(self.output_tokens)
            .saturating_add(self.reasoning_tokens)
    }

    pub fn is_empty(&self) -> bool {
        self.turns == 0
    }

    /// One-line human summary, e.g.
    /// `1,234 in + 567 out + 12 reasoning = 1,813 tokens · $0.0420 over 4 turns`.
    pub fn one_line(&self) -> String {
        if self.is_empty() {
            return "no usage recorded yet".to_string();
        }
        format!(
            "{} in + {} out{} = {} tokens · ${:.4} over {} turn{}",
            format_count(self.input_tokens),
            format_count(self.output_tokens),
            if self.reasoning_tokens > 0 {
                format!(" + {} reasoning", format_count(self.reasoning_tokens))
            } else {
                String::new()
            },
            format_count(self.total_tokens()),
            self.total_cost_usd,
            self.turns,
            if self.turns == 1 { "" } else { "s" },
        )
    }
}

/// Build a per-session summary from the session's message history.
pub fn summarize(messages: &[Message]) -> SessionCostSummary {
    let mut totals = SessionCostSummary::default();
    let mut by_key: BTreeMap<(String, String), ModelCostBreakdown> = BTreeMap::new();

    for message in messages {
        if message.role != Role::Assistant {
            continue;
        }
        let Some(usage) = message.usage.as_ref() else {
            continue;
        };
        let provider = message.metadata.model_provider_id.clone();
        let model = message.metadata.model_id.clone();

        totals.turns = totals.turns.saturating_add(1);
        totals.input_tokens = totals.input_tokens.saturating_add(usage.input_tokens);
        totals.output_tokens = totals.output_tokens.saturating_add(usage.output_tokens);
        totals.reasoning_tokens = totals
            .reasoning_tokens
            .saturating_add(usage.reasoning_tokens);
        totals.cache_write_tokens = totals
            .cache_write_tokens
            .saturating_add(usage.cache_write_tokens);
        totals.cache_read_tokens = totals
            .cache_read_tokens
            .saturating_add(usage.cache_read_tokens);
        totals.total_cost_usd += usage.total_cost;

        let entry = by_key
            .entry((provider.clone(), model.clone()))
            .or_insert_with(|| ModelCostBreakdown {
                provider_id: provider,
                model_id: model,
                ..ModelCostBreakdown::default()
            });
        entry.fold(usage);
    }

    totals.by_model = by_key.into_values().collect();
    totals
}

fn format_count(n: u64) -> String {
    let s = n.to_string();
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 && (bytes.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(*b as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{MessageMetadata, MessageSource, MessageStatus, PartContent};
    use crate::role::Role;
    use chrono::Utc;

    fn assistant(provider: &str, model: &str, usage: MessageUsage) -> Message {
        let mut msg = Message::prompt_parts(Role::Assistant, vec![PartContent::text("hi")]);
        msg.id = 0;
        msg.state = MessageStatus::Completed;
        msg.usage = Some(usage);
        msg.metadata = MessageMetadata {
            source: MessageSource::Assistant,
            parent_message_id: None,
            generated_by_call_id: None,
            model_provider_id: provider.to_string(),
            model_id: model.to_string(),
            provider_metadata: None,
            tags: Vec::new(),
        };
        for part in msg.parts.iter_mut() {
            part.created_at = Utc::now();
        }
        msg
    }

    fn user_msg() -> Message {
        let mut msg = Message::prompt_parts(Role::User, vec![PartContent::text("hi")]);
        msg.id = 0;
        msg.state = MessageStatus::Completed;
        msg
    }

    #[test]
    fn empty_session_has_no_usage() {
        let summary = summarize(&[]);
        assert!(summary.is_empty());
        assert_eq!(summary.one_line(), "no usage recorded yet");
    }

    #[test]
    fn user_messages_are_ignored() {
        let summary = summarize(&[user_msg()]);
        assert!(summary.is_empty());
    }

    #[test]
    fn aggregates_one_assistant_turn() {
        let usage = MessageUsage {
            input_tokens: 100,
            output_tokens: 200,
            reasoning_tokens: 0,
            cache_write_tokens: 0,
            cache_read_tokens: 0,
            total_cost: 0.005,
        };
        let summary = summarize(&[assistant("anthropic", "claude-haiku-4-5", usage)]);
        assert_eq!(summary.turns, 1);
        assert_eq!(summary.total_tokens(), 300);
        assert!((summary.total_cost_usd - 0.005).abs() < 1e-9);
        assert_eq!(summary.by_model.len(), 1);
        assert_eq!(summary.by_model[0].model_id, "claude-haiku-4-5");
    }

    #[test]
    fn breaks_down_per_provider_model_pair() {
        let messages = vec![
            assistant(
                "anthropic",
                "claude-haiku-4-5",
                MessageUsage {
                    input_tokens: 100,
                    output_tokens: 200,
                    total_cost: 0.001,
                    ..Default::default()
                },
            ),
            assistant(
                "anthropic",
                "claude-sonnet-4-5",
                MessageUsage {
                    input_tokens: 50,
                    output_tokens: 80,
                    total_cost: 0.010,
                    ..Default::default()
                },
            ),
            assistant(
                "anthropic",
                "claude-haiku-4-5",
                MessageUsage {
                    input_tokens: 30,
                    output_tokens: 40,
                    total_cost: 0.0005,
                    ..Default::default()
                },
            ),
        ];
        let summary = summarize(&messages);
        assert_eq!(summary.turns, 3);
        assert_eq!(summary.input_tokens, 180);
        assert_eq!(summary.output_tokens, 320);
        assert_eq!(summary.by_model.len(), 2);

        let haiku = summary
            .by_model
            .iter()
            .find(|m| m.model_id == "claude-haiku-4-5")
            .unwrap();
        assert_eq!(haiku.turns, 2);
        assert_eq!(haiku.input_tokens, 130);
        assert!((haiku.total_cost_usd - 0.0015).abs() < 1e-9);

        let sonnet = summary
            .by_model
            .iter()
            .find(|m| m.model_id == "claude-sonnet-4-5")
            .unwrap();
        assert_eq!(sonnet.turns, 1);
        assert!((sonnet.total_cost_usd - 0.010).abs() < 1e-9);
    }

    #[test]
    fn one_line_reports_compact_summary() {
        let usage = MessageUsage {
            input_tokens: 1234,
            output_tokens: 567,
            reasoning_tokens: 12,
            total_cost: 0.042,
            ..Default::default()
        };
        let summary = summarize(&[assistant("anthropic", "claude-haiku-4-5", usage)]);
        let line = summary.one_line();
        assert!(line.contains("1,234 in"));
        assert!(line.contains("567 out"));
        assert!(line.contains("12 reasoning"));
        assert!(line.contains("$0.0420"));
        assert!(line.contains("1 turn"));
    }

    #[test]
    fn one_line_omits_reasoning_when_zero() {
        let usage = MessageUsage {
            input_tokens: 100,
            output_tokens: 200,
            ..Default::default()
        };
        let summary = summarize(&[assistant("openai", "gpt-5", usage)]);
        let line = summary.one_line();
        assert!(!line.contains("reasoning"));
    }
}
