use std::time::Duration;

use serde_json::Value;
use tokio::time::{self, Instant};

use super::{AppError, mcp, protocol::ActivitySnapshot};

const DEFAULT_SECONDS: u64 = 180;

pub(super) struct PromptIdle {
    interval: Duration,
    last_progress: Instant,
    state: ActivitySnapshot,
    mcp: Option<mcp::ToolProgress>,
}

impl PromptIdle {
    pub(super) fn after_send(mcp: Option<mcp::ToolProgress>) -> Self {
        let seconds = std::env::var("AGENA_ACP_PROMPT_IDLE_TIMEOUT_SECS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|seconds| (1..=86_400).contains(seconds))
            .unwrap_or(DEFAULT_SECONDS);
        Self::new(Duration::from_secs(seconds), mcp)
    }

    pub(super) fn new(interval: Duration, mcp: Option<mcp::ToolProgress>) -> Self {
        Self {
            interval,
            last_progress: Instant::now(),
            state: ActivitySnapshot::default(),
            mcp,
        }
    }

    pub(super) fn record(&mut self) {
        self.last_progress = Instant::now();
    }

    pub(super) fn observe_update(&mut self, root: bool, update: &Value) {
        let progress = match update["sessionUpdate"].as_str() {
            Some("state_update") if root => {
                let previous = self.state;
                self.state.observe(update) && self.state != previous
            }
            Some(
                "agent_message"
                | "agent_message_chunk"
                | "agent_thought"
                | "agent_thought_chunk"
                | "user_message"
                | "user_message_chunk"
                | "session_message"
                | "session_message_chunk"
                | "compaction_summary_chunk",
            ) => update.get("content").is_some_and(|content| match content {
                Value::Array(items) => !items.is_empty(),
                Value::Object(block) => block.get("type").is_some_and(|kind| {
                    kind != "text"
                        || block
                            .get("text")
                            .and_then(Value::as_str)
                            .is_some_and(|text| !text.is_empty())
                }),
                _ => false,
            }),
            Some(
                "tool_call"
                | "tool_call_update"
                | "tool_call_content_chunk"
                | "plan"
                | "plan_update"
                | "plan_removed"
                | "subagent_update"
                | "compaction_update"
                | "terminal_update"
                | "terminal_output_chunk",
            ) => true,
            _ => false,
        };
        if progress {
            self.record();
        }
    }

    pub(super) async fn expired(&mut self, authorizing_until: Option<Instant>) {
        loop {
            let mut since = self.last_progress;
            if let Some(until) = authorizing_until {
                since = since.max(until);
            }
            let Some(mcp) = &mut self.mcp else {
                time::sleep_until(since + self.interval).await;
                return;
            };
            let activity = *mcp.activity.borrow_and_update();
            if let Some(finished_at) = activity.finished_at {
                since = since.max(finished_at);
            }
            tokio::select! {
                biased;
                update = mcp.activity.changed() => {
                    if update.is_err() {
                        self.last_progress = self.last_progress.max(activity.finished_at.unwrap_or(self.last_progress));
                        self.mcp = None;
                    }
                }
                _ = time::sleep_until(since + self.interval), if activity.active == 0 => {
                    // Synchronize expiration with tool admission on the bridge.
                    // An accepted effect cannot race a timeout decision.
                    if mcp.revoke_if_idle() {
                        return;
                    }
                    self.record();
                }
            }
        }
    }

    pub(super) fn error(&self, agent: &'static str) -> AppError {
        AppError::ExternalAgentPromptInactive {
            agent,
            seconds: self.interval.as_secs(),
        }
    }
}
