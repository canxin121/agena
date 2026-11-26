use std::collections::BTreeMap;

use agena_provider::CompletionUsage;
use agena_storage::kernel::ExternalAgentNativeCursor;
use serde_json::{Value, json};

use super::{MAX_CURSOR_BYTES, bounded_string};

const MAX_PROMPT_CURSORS: usize = 64;

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
                    "session": {"notices": {}, "compaction": {}, "promptSuggestions": {}, "configOptions": {"boolean": {}}}
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

#[derive(Clone, Copy, Default)]
pub(in crate::session::manager::external_agent) struct ActivitySnapshot {
    foreground: ForegroundState,
    background: bool,
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
        if let Some(background) = update["background"].as_array() {
            self.background = !background.is_empty();
        }
        true
    }
}

/// A v2 prompt reply acknowledges insertion. Session state ends the workflow
/// and accounts for work that continues after the foreground becomes idle.
#[derive(Default)]
pub(super) struct PromptLifecycle {
    accepted: bool,
    message_id: Option<String>,
    cursors: BTreeMap<String, String>,
    activity: ActivitySnapshot,
    started: bool,
    stop: Option<Value>,
}

pub(in crate::session::manager::external_agent) fn fork_cursor(
    update: &Value,
) -> Option<ExternalAgentNativeCursor> {
    let grant = &update["capabilities"]["fork"];
    let anchor = &update["anchor"];
    if update["sessionUpdate"] != "cursor_update"
        || !(grant.is_object() || grant == true)
        || anchor["type"] != "user_message"
        || anchor["position"] != "before"
    {
        return None;
    }
    Some(ExternalAgentNativeCursor {
        message_id: bounded_string(&anchor["messageId"], MAX_CURSOR_BYTES)?,
        cursor_id: bounded_string(&update["cursorId"], MAX_CURSOR_BYTES)?,
    })
}

impl PromptLifecycle {
    pub(super) fn new(activity: ActivitySnapshot) -> Self {
        Self {
            activity,
            ..Self::default()
        }
    }

    pub(super) fn acknowledge(&mut self, result: &Value) {
        if self.accepted {
            return;
        }
        self.accepted = true;
        self.message_id = bounded_string(&result["messageId"], MAX_CURSOR_BYTES);
        self.cursors
            .retain(|message, _| self.message_id.as_ref() == Some(message));
    }

    pub(super) fn observe(&mut self, update: &Value) {
        if let Some(cursor) = fork_cursor(update) {
            if (!self.accepted || self.message_id.as_ref() == Some(&cursor.message_id))
                && (self.cursors.len() < MAX_PROMPT_CURSORS
                    || self.cursors.contains_key(&cursor.message_id))
            {
                self.cursors.insert(cursor.message_id, cursor.cursor_id);
            }
            return;
        }
        let previous = self.activity.foreground;
        if !self.activity.observe(update) {
            return;
        }
        match self.activity.foreground {
            ForegroundState::Active if previous != ForegroundState::Active => {
                self.started |= previous == ForegroundState::Idle;
                self.stop = None;
            }
            ForegroundState::Idle if previous != ForegroundState::Idle => {
                if self.started || (self.accepted && previous == ForegroundState::Active) {
                    self.stop = Some(json!({
                        "stopReason": update["stopReason"],
                        "error": update["error"],
                        "usage": update["usage"],
                    }));
                }
                self.started = false;
            }
            _ => {}
        }
    }

    pub(super) fn take_result(&mut self, steering_pending: bool) -> Option<Value> {
        if self.accepted
            && self.activity.foreground == ForegroundState::Idle
            && !self.activity.background
            && !steering_pending
        {
            self.stop.take()
        } else {
            None
        }
    }

    pub(super) fn into_fork_cursor(mut self) -> Option<ExternalAgentNativeCursor> {
        let message_id = self.message_id?;
        let cursor_id = self.cursors.remove(&message_id)?;
        Some(ExternalAgentNativeCursor {
            message_id,
            cursor_id,
        })
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
        turn.acknowledge(&json!({"messageId": "prompt"}));
        assert!(turn.take_result(false).is_none());
        turn.observe(&json!({"sessionUpdate": "state_update", "state": "running"}));
        turn.observe(&json!({
            "sessionUpdate": "state_update", "state": "idle", "stopReason": "end_turn",
            "background": [{"type": "tool_call", "toolCallId": "job"}]
        }));
        assert!(turn.take_result(false).is_none());
        turn.observe(
            &json!({"sessionUpdate": "state_update", "state": "idle", "background": null}),
        );
        assert!(turn.take_result(false).is_none());
        turn.observe(&json!({"sessionUpdate": "state_update", "state": "idle", "background": []}));
        assert_eq!(turn.take_result(false).unwrap()["stopReason"], "end_turn");
    }

    #[test]
    fn resumed_work_does_not_settle_a_prompt_held_until_later() {
        let mut snapshot = ActivitySnapshot::default();
        let running = json!({"sessionUpdate": "state_update", "state": "running"});
        snapshot.observe(&running);
        let mut prompt = PromptLifecycle::new(snapshot);
        prompt.observe(&running);
        prompt.observe(
            &json!({"sessionUpdate": "state_update", "state": "idle", "stopReason": "max_tokens"}),
        );
        prompt.acknowledge(&json!({"messageId": "new-user-message"}));
        assert!(prompt.take_result(false).is_none());
        prompt.observe(&running);
        prompt.observe(&json!({"sessionUpdate": "state_update", "state": "idle"}));
        assert!(prompt.take_result(false).unwrap()["stopReason"].is_null());
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
        turn.acknowledge(&json!({"messageId": "input"}));
        turn.observe(&update("input", "different-anchor", "after"));
        assert_eq!(
            turn.into_fork_cursor(),
            Some(ExternalAgentNativeCursor {
                message_id: "input".into(),
                cursor_id: "opaque-cursor".into(),
            })
        );

        let mut command = PromptLifecycle::default();
        command.observe(&update("other", "unrelated", "before"));
        command.acknowledge(&json!({"messageId": "command"}));
        assert_eq!(command.into_fork_cursor(), None);
    }
}
