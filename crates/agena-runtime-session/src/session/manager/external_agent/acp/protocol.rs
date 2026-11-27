use std::{collections::HashSet, time::Duration};

use agena_provider::CompletionUsage;
use agena_storage::kernel::{ExternalAgentCursorEntity, ExternalAgentNativeCursor};
use serde_json::{Value, json};
use tokio::time::Instant;

use super::{MAX_CURSOR_BYTES, bounded_string, cursors};

const MAX_PROMPT_CURSORS: usize = 64;
const MAX_PENDING_USER_IDS: usize = 64;
const FOREGROUND_FOLLOWUP_GRACE: Duration = Duration::from_millis(250);

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Version {
    V1,
    V2,
}

impl Version {
    pub(super) fn initialize_params(self, terminal_auth: bool) -> Value {
        let info = json!({"name": "agena", "title": "Agena", "version": env!("CARGO_PKG_VERSION")});
        let elicitation = json!({"form": {}, "url": {}});
        let mut params = match self {
            Self::V1 => json!({
                "protocolVersion": 1,
                "clientCapabilities": {
                    "fs": {"readTextFile": true, "writeTextFile": true},
                    "terminal": true,
                    "elicitation": elicitation,
                    "plan": {},
                    "subagents": {},
                    "session": {"notices": {}, "compaction": {}, "promptSuggestions": {}, "configOptions": {"boolean": {}}, "cursors": {}}
                },
                "clientInfo": info
            }),
            Self::V2 => json!({
                "protocolVersion": 2,
                "capabilities": {
                    "elicitation": elicitation,
                    "session": {"promptSuggestions": {}, "commands": {"skill": {}}}
                },
                "info": info
            }),
        };
        if terminal_auth {
            match self {
                Self::V1 => params["clientCapabilities"]["auth"] = json!({"terminal": true}),
                Self::V2 => params["capabilities"]["auth"] = json!({"terminal": {}}),
            }
        }
        params
    }

    pub(super) fn negotiated(self, response: &Value) -> Option<Self> {
        match response["protocolVersion"].as_u64()? {
            1 => Some(Self::V1),
            2 if self == Self::V2 => Some(Self::V2),
            _ => None,
        }
    }
}

pub(super) fn add_air_capability(params: &mut Value, capability: &str) {
    let air = &mut params["clientCapabilities"]["_meta"]["jetbrains"]["air"];
    air["version"] = json!(1);
    let capabilities = &mut air["capabilities"];
    if let Some(list) = capabilities.as_array_mut() {
        if !list.iter().any(|entry| entry == capability) {
            list.push(json!(capability));
        }
    } else {
        *capabilities = json!([capability]);
    }
}

pub(super) fn advertises_air_capability(response: &Value, capability: &str) -> bool {
    let air = &response["_meta"]["jetbrains"]["air"];
    air["version"].as_u64().is_some_and(|version| version >= 1)
        && air["capabilities"]
            .as_array()
            .is_some_and(|list| list.iter().any(|entry| entry == capability))
}

pub(super) fn normalize_air_subagent_update(update: &mut Value) {
    let state = match update["sessionUpdate"].as_str() {
        Some("subagent_spawned") => json!({"state":"running"}),
        Some("subagent_state_update") => json!({"state":"idle", "stopReason":update["state"]}),
        _ => return,
    };
    let mut normalized = json!({
        "sessionUpdate": "subagent_update",
        "sessionId": update["subagentSessionId"],
        "state": state,
    });
    if update["sessionUpdate"] == "subagent_spawned" {
        normalized["title"] = update["name"].clone();
        normalized["description"] = update["task"].clone();
    }
    *update = normalized;
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum ForegroundState {
    Active,
    #[default]
    Idle,
    Unknown,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum BackgroundState {
    #[default]
    Unreported,
    Empty,
    Active,
    Uncertain,
}

impl BackgroundState {
    fn blocks_completion(self) -> bool {
        matches!(self, Self::Active | Self::Uncertain)
    }
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(in crate::session::manager::external_agent) struct ActivitySnapshot {
    foreground: ForegroundState,
    background: BackgroundState,
}

impl ActivitySnapshot {
    pub(super) fn observe(&mut self, update: &Value) -> bool {
        if update["sessionUpdate"] != "state_update" {
            return false;
        }
        let Some(state) = update["state"].as_str() else {
            return false;
        };
        self.foreground = match state {
            "running" | "requires_action" => ForegroundState::Active,
            "idle" => ForegroundState::Idle,
            _ => ForegroundState::Unknown,
        };
        self.background = match update["background"].as_array() {
            Some(background) if background.is_empty() => BackgroundState::Empty,
            Some(_) => BackgroundState::Active,
            None if self.background.blocks_completion() => BackgroundState::Uncertain,
            None => BackgroundState::Unreported,
        };
        true
    }
}

#[derive(Default)]
struct InheritedForeground {
    inserted_users: HashSet<String>,
    stop: Option<Value>,
}

/// A v2 prompt reply acknowledges insertion. Session state ends the workflow
/// and accounts for work that continues after the foreground becomes idle.
#[derive(Default)]
pub(super) struct PromptLifecycle {
    pub(super) replayed_users: Option<super::ReplayedUsers>,
    message_id: Option<String>,
    cursors: cursors::Book,
    preceding: Option<cursors::Entity>,
    tail: cursors::Tail,
    allow_rewind: bool,
    activity: ActivitySnapshot,
    inherited: Option<InheritedForeground>,
    started: bool,
    stop: Option<Value>,
    quiet_after: Option<Instant>,
}

impl PromptLifecycle {
    pub(super) fn new(
        activity: ActivitySnapshot,
        preceding: Option<ExternalAgentCursorEntity>,
        allow_rewind: bool,
    ) -> Self {
        let preceding = preceding.and_then(cursors::Entity::restore);
        Self {
            activity,
            inherited: (activity.foreground != ForegroundState::Idle)
                .then(InheritedForeground::default),
            tail: cursors::Tail::new(preceding.clone()),
            preceding,
            allow_rewind,
            ..Self::default()
        }
    }

    pub(super) fn acknowledge(&mut self, message_id: String) {
        if let Some(inherited) = &mut self.inherited
            && inherited.inserted_users.contains(&message_id)
            && let Some(stop) = inherited.stop.take()
        {
            self.stop = Some(stop);
        }
        self.message_id = Some(message_id);
    }

    pub(super) fn observe(&mut self, update: &Value) {
        if update["sessionUpdate"] == "cursor_update" {
            self.cursors
                .observe(update, self.allow_rewind, MAX_PROMPT_CURSORS);
            return;
        }
        self.tail.observe(update);
        if let Some(inherited) = &mut self.inherited
            && self.activity.foreground == ForegroundState::Active
            && matches!(
                update["sessionUpdate"].as_str(),
                Some("user_message" | "user_message_chunk")
            )
            && inherited.inserted_users.len() < MAX_PENDING_USER_IDS
            && let Some(id) = bounded_string(&update["messageId"], MAX_CURSOR_BYTES)
        {
            inherited.inserted_users.insert(id);
        }
        let previous = self.activity.foreground;
        let previous_background = self.activity.background;
        if !self.activity.observe(update) {
            return;
        }
        if previous_background.blocks_completion()
            && self.activity.background == BackgroundState::Empty
        {
            self.quiet_after = Some(Instant::now() + FOREGROUND_FOLLOWUP_GRACE);
        }
        match self.activity.foreground {
            ForegroundState::Active if previous != ForegroundState::Active => {
                if previous == ForegroundState::Idle {
                    self.started = true;
                    self.inherited = None;
                }
                self.stop = None;
                self.quiet_after = None;
            }
            ForegroundState::Idle if previous == ForegroundState::Active => {
                let stop = json!({
                    "stopReason": update["stopReason"],
                    "error": update["error"],
                    "usage": update["usage"],
                });
                if self.started {
                    self.stop = Some(stop);
                } else if let Some(inherited) = &mut self.inherited
                    && update["stopReason"] != "cancelled"
                {
                    // A prompt can join resumed foreground work. Correlate
                    // a late reply with a user message observed in that work.
                    // Leave room for a following foreground interval.
                    self.quiet_after = Some(Instant::now() + FOREGROUND_FOLLOWUP_GRACE);
                    if self.message_id.is_some() {
                        self.stop = Some(stop);
                    } else {
                        inherited.stop = Some(stop);
                    }
                }
                self.started = false;
            }
            ForegroundState::Idle if previous == ForegroundState::Unknown => {
                if self.started {
                    // The new snapshot confirms there is no foreground work.
                    // Its stop reason may predate the loss of observability.
                    self.stop = Some(json!({}));
                }
                self.started = false;
                self.inherited = None;
            }
            _ => {}
        }
    }

    fn completion_ready(&self, steering_pending: bool) -> bool {
        self.message_id.is_some()
            && self.activity.foreground == ForegroundState::Idle
            && !self.activity.background.blocks_completion()
            && !steering_pending
            && self.stop.is_some()
    }

    pub(super) fn completion_deadline(&self, steering_pending: bool) -> Option<Instant> {
        self.quiet_after
            .filter(|_| self.completion_ready(steering_pending))
    }

    pub(super) fn waiting_for_background(&self) -> bool {
        self.message_id.is_some()
            && self.activity.foreground == ForegroundState::Idle
            && self.activity.background == BackgroundState::Active
    }

    pub(super) fn take_result(&mut self, steering_pending: bool) -> Option<Value> {
        if self.completion_ready(steering_pending)
            && self
                .quiet_after
                .is_none_or(|deadline| deadline <= Instant::now())
        {
            self.stop.take()
        } else {
            None
        }
    }

    pub(super) fn into_fork_cursor(self) -> Option<ExternalAgentNativeCursor> {
        self.cursors
            .for_user(self.message_id.as_deref(), self.preceding.as_ref())
    }

    pub(super) fn cursor_tail(&self) -> Option<ExternalAgentCursorEntity> {
        self.tail.latest()
    }
}

pub(super) fn normalize_config_options(value: &mut Value) {
    if let Some(options) = value.get_mut("configOptions").and_then(Value::as_array_mut) {
        for option in options {
            rename(option, "configId", "id");
            if let Some(choices) = option.get_mut("options").and_then(Value::as_array_mut) {
                for choice in choices {
                    rename(choice, "groupId", "group");
                }
            }
        }
    }
}

fn rename(value: &mut Value, from: &str, to: &str) {
    if let Some(object) = value.as_object_mut()
        && let Some(value) = object.remove(from)
    {
        object.insert(to.into(), value);
    }
}

pub(super) fn permission_params(params: &Value) -> Value {
    let mut params = params.clone();
    let mut tool = params["subject"]["toolCall"]
        .as_object()
        .cloned()
        .unwrap_or_default();
    if let Some(title) = params.get("title") {
        tool.insert("title".into(), title.clone());
    }
    params["toolCall"] = Value::Object(tool);
    params
}

pub(super) fn permission_tool_update(params: &Value) -> Option<Value> {
    let subject = &params["subject"];
    if subject["type"] != "tool_call" {
        return None;
    }
    let mut tool = subject["toolCall"].as_object()?.clone();
    tool.insert("sessionUpdate".into(), json!("tool_call_update"));
    Some(Value::Object(tool))
}

pub(super) fn foreground_usage(value: &Value) -> Option<CompletionUsage> {
    let total = value["totalTokens"].as_u64()?;
    let mut usage = CompletionUsage {
        input_tokens: value["inputTokens"].as_u64()?,
        output_tokens: value["outputTokens"].as_u64()?,
        reasoning_tokens: value["thoughtTokens"].as_u64().unwrap_or_default(),
        cache_read_tokens: value["cachedReadTokens"].as_u64().unwrap_or_default(),
        cache_write_tokens: value["cachedWriteTokens"].as_u64().unwrap_or_default(),
        ..Default::default()
    };
    usage.other_tokens = total.saturating_sub(usage.own_total_tokens());
    Some(usage)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_insertion_and_foreground_idle_leave_background_running() {
        let mut turn = PromptLifecycle::default();
        turn.observe(&json!({
            "sessionUpdate": "state_update", "state": "idle", "stopReason": "end_turn"
        }));
        turn.acknowledge("prompt".into());
        assert!(turn.take_result(false).is_none());
        turn.observe(&json!({"sessionUpdate": "state_update", "state": "running"}));
        turn.observe(&json!({
            "sessionUpdate": "state_update", "state": "idle", "stopReason": "end_turn",
            "background": [{"type": "tool_call", "toolCallId": "job"}]
        }));
        assert!(turn.waiting_for_background());
        assert!(turn.take_result(false).is_none());
        turn.observe(
            &json!({"sessionUpdate": "state_update", "state": "idle", "background": null}),
        );
        assert!(!turn.waiting_for_background());
        assert!(turn.take_result(false).is_none());
        turn.observe(&json!({"sessionUpdate": "state_update", "state": "idle", "background": []}));
        assert!(turn.completion_deadline(false).is_some());
        assert!(turn.take_result(false).is_none());
        turn.quiet_after = Some(Instant::now());
        assert_eq!(turn.take_result(false).unwrap()["stopReason"], "end_turn");
    }

    #[test]
    fn resumed_work_does_not_settle_a_prompt_held_until_later() {
        let mut snapshot = ActivitySnapshot::default();
        let running = json!({"sessionUpdate": "state_update", "state": "running"});
        snapshot.observe(&running);
        let mut prompt = PromptLifecycle::new(snapshot, None, false);
        prompt.observe(&running);
        prompt.observe(
            &json!({"sessionUpdate": "state_update", "state": "idle", "stopReason": "max_tokens"}),
        );
        prompt.acknowledge("new-user-message".into());
        assert!(prompt.take_result(false).is_none());
        prompt.observe(&running);
        prompt.observe(&json!({"sessionUpdate": "state_update", "state": "idle"}));
        assert!(prompt.take_result(false).unwrap()["stopReason"].is_null());
    }

    #[test]
    fn a_late_acknowledgment_matches_the_user_message_in_resumed_foreground() {
        let mut snapshot = ActivitySnapshot::default();
        snapshot.observe(&json!({"sessionUpdate": "state_update", "state": "running"}));
        let mut prompt = PromptLifecycle::new(snapshot, None, false);
        prompt.observe(&json!({
            "sessionUpdate": "user_message", "messageId": "inserted",
            "content": [{"type": "text", "text": "Join the running work"}],
        }));
        prompt.observe(&json!({
            "sessionUpdate": "state_update", "state": "idle", "stopReason": "end_turn",
        }));
        prompt.acknowledge("inserted".into());
        assert!(prompt.completion_deadline(false).is_some());
        prompt.quiet_after = Some(Instant::now());
        assert_eq!(prompt.take_result(false).unwrap()["stopReason"], "end_turn");
    }

    #[test]
    fn regained_observability_settles_confirmed_work_without_a_stale_stop_reason() {
        let mut prompt = PromptLifecycle::default();
        prompt.acknowledge("accepted".into());
        prompt.observe(&json!({"sessionUpdate": "state_update", "state": "running"}));
        prompt.observe(&json!({"sessionUpdate": "state_update", "state": "unknown"}));
        assert!(prompt.take_result(false).is_none());
        prompt.observe(&json!({
            "sessionUpdate": "state_update", "state": "idle", "stopReason": "cancelled",
        }));
        assert_eq!(prompt.take_result(false).unwrap(), json!({}));
    }

    #[test]
    fn initially_unknown_work_can_recover_and_finish_an_inserted_prompt() {
        let mut snapshot = ActivitySnapshot::default();
        snapshot.observe(&json!({"sessionUpdate": "state_update", "state": "unknown"}));
        let mut prompt = PromptLifecycle::new(snapshot, None, false);
        prompt.observe(&json!({"sessionUpdate": "state_update", "state": "running"}));
        prompt.acknowledge("accepted".into());
        prompt.observe(&json!({
            "sessionUpdate": "state_update", "state": "idle", "stopReason": "end_turn",
        }));
        prompt.quiet_after = Some(Instant::now());
        assert_eq!(prompt.take_result(false).unwrap()["stopReason"], "end_turn");
    }

    #[test]
    fn foreground_usage_preserves_reported_token_categories() {
        let usage = foreground_usage(&json!({
            "inputTokens": 13, "outputTokens": 7, "cachedReadTokens": 5, "totalTokens": 26,
        }))
        .unwrap();
        assert_eq!(usage.own_total_tokens(), 26);
        assert_eq!(usage.cache_read_tokens, 5);
        assert_eq!(usage.other_tokens, 1);
        assert!(!usage.recorded_cost_available);
    }

    #[test]
    fn accepts_the_advertised_fork_cursor_for_the_acknowledged_prompt() {
        let update = |message, cursor, position| {
            json!({
                "sessionUpdate": "cursor_update", "cursorId": cursor, "capabilities": {"fork": {}},
                "anchor": {"type": "user_message", "messageId": message, "position": position}
            })
        };
        let mut turn = PromptLifecycle::default();
        turn.observe(&update("other", "unrelated", "before"));
        turn.observe(&update("input", "opaque-cursor", "before"));
        turn.acknowledge("input".into());
        turn.observe(&update("input", "different-anchor", "after"));
        assert_eq!(
            turn.into_fork_cursor(),
            Some(ExternalAgentNativeCursor {
                message_id: "input".into(),
                cursor_id: Some("opaque-cursor".into()),
                fork: true,
                rewind: false,
            })
        );

        let mut command = PromptLifecycle::default();
        command.observe(&update("other", "unrelated", "before"));
        command.acknowledge("command".into());
        assert_eq!(command.into_fork_cursor().unwrap().cursor_id, None);
    }

    #[test]
    fn accepts_a_capability_withdrawal_without_repeating_the_anchor() {
        let mut turn = PromptLifecycle::default();
        turn.acknowledge("input".into());
        turn.observe(&json!({
            "sessionUpdate":"cursor_update", "cursorId":"opaque", "capabilities":{"fork":{}},
            "anchor":{"type":"user_message", "messageId":"input", "position":"before"},
        }));
        turn.observe(&json!({
            "sessionUpdate":"cursor_update", "cursorId":"opaque", "capabilities":null,
        }));
        let position = turn.into_fork_cursor().unwrap();
        assert_eq!(position.cursor_id.as_deref(), Some("opaque"));
        assert!(!position.fork);
    }
}
