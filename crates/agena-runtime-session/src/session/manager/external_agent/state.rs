use std::{collections::HashSet, path::Path};

use agena_domain::{
    ExternalAgentCommand, ExternalAgentCompaction, ExternalAgentCost, ExternalAgentMessage,
    ExternalAgentNotice, ExternalAgentSubagent, ExternalAgentSuggestion,
    ExternalAgentSuggestionBlock, ExternalAgentTerminal,
};
use agena_storage::kernel::{
    ExternalAgentContextUsage, ExternalAgentPlan, ExternalAgentPlanEntry, ExternalAgentPlanStatus,
    ExternalAgentState,
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::Value;

const MAX_PLANS: usize = 8;
const MAX_ENTRIES: usize = 128;
const MAX_PLAN_BYTES: usize = 16 * 1024;
const MAX_COMMANDS: usize = 256;
const MAX_NOTICES: usize = 8;
const MAX_COMPACTIONS: usize = 12;
const MAX_SUMMARY_BYTES: usize = 6 * 1024;
const MAX_CHILDREN: usize = 24;
const MAX_MESSAGES: usize = 6;
const MAX_MESSAGE_BYTES: usize = 2048;
const MAX_TERMINALS: usize = 8;
const MAX_TERMINAL_BYTES: usize = 16 * 1024;
pub(super) const MAX_STATE_SNAPSHOT_BYTES: usize = 48 * 1024;

/// The ID-less ACP plan occupies its own slot. An empty checklist removes it.
const LEGACY_PLAN: &str = "";

pub(super) fn apply_setup_response(state: &mut ExternalAgentState, response: &Value, v2: bool) {
    if v2 || response.get("configOptions").is_some() || response.get("modes").is_some() {
        state.options = super::options::config_options(response);
    }
    if v2 {
        state.commands = available_commands(&response["availableCommands"]).unwrap_or_default();
    } else {
        state.commands.retain(|command| !command.skill);
    }
}

pub(super) fn apply_update(state: &mut ExternalAgentState, update: &Value) -> bool {
    match update["sessionUpdate"].as_str() {
        Some("plan") => {
            let Some(plan) = checklist(&update["entries"]) else {
                return false;
            };
            set_plan(state, LEGACY_PLAN, plan)
        }
        Some("plan_update") => {
            let plan = &update["plan"];
            let Some(id) = plan_id(plan) else {
                return false;
            };
            let update = match plan["type"].as_str() {
                Some("items") => checklist(&plan["entries"]),
                Some("markdown") => plan["content"].as_str().map(|text| {
                    let content = bounded(text.trim(), MAX_PLAN_BYTES);
                    (!content.is_empty()).then_some(ExternalAgentPlan::Markdown { content })
                }),
                _ => None,
            };
            update.is_some_and(|plan| set_plan(state, id, plan))
        }
        Some("plan_removed") => plan_id(update).is_some_and(|id| state.plans.remove(id).is_some()),
        Some("available_commands_update") => {
            let Some(commands) = available_commands(&update["availableCommands"]) else {
                return false;
            };
            if state.commands == commands {
                return false;
            }
            state.commands = commands;
            true
        }
        Some("usage_update") => {
            let mut changed = false;
            if let Some(context) = context_usage(update)
                && state.context != Some(context)
            {
                state.context = Some(context);
                changed = true;
            }
            if let Some(cost) = update.get("cost") {
                let next = if cost.is_null() {
                    Some(None)
                } else {
                    cost_value(cost).map(Some)
                };
                if let Some(next) = next
                    && state.details.cost != next
                {
                    state.details.cost = next;
                    changed = true;
                }
            }
            changed
        }
        Some("config_option_update" | "current_mode_update") => {
            super::options::apply_options_update(state, update)
        }
        Some("session_info_update") => super::lifecycle::apply_info_update(state, update),
        Some("notice") => apply_notice(state, None, update),
        Some("compaction_update" | "compaction_summary_chunk") => apply_compaction(state, update),
        Some("session_message" | "session_message_chunk") => {
            update_message(&mut state.details.messages, update)
        }
        Some("prompt_suggestions_update") => apply_suggestions(state, update),
        _ => false,
    }
}

pub(super) fn accepts_session(state: &ExternalAgentState, root: &str, source: &str) -> bool {
    source == root || state.details.subagents.contains_key(source)
}

/// Apply an update from the root or from a previously announced descendant.
/// Subagents have their own message IDs and cannot change the root's settings.
pub(super) fn apply_native_update(
    state: &mut ExternalAgentState,
    root: &str,
    source: &str,
    update: &Value,
) -> bool {
    if !accepts_session(state, root, source) {
        return false;
    }
    if update["sessionUpdate"] == "subagent_update" {
        return update_child(state, root, source, update);
    }
    if matches!(
        update["sessionUpdate"].as_str(),
        Some("terminal_update" | "terminal_output_chunk")
    ) {
        return apply_terminal(state, source, update);
    }
    if source == root {
        return apply_update(state, update);
    }
    if update["sessionUpdate"] == "notice" {
        return apply_notice(state, Some(source), update);
    }
    let Some(child) = state.details.subagents.get_mut(source) else {
        return false;
    };
    if update["sessionUpdate"] == "state_update" {
        return apply_child_state(child, update);
    }
    update_message(&mut child.messages, update)
}

/// Show late native messages without a transcript entry in the activity pane.
pub(super) fn record_idle_message(state: &mut ExternalAgentState, update: &Value) -> bool {
    matches!(
        update["sessionUpdate"].as_str(),
        Some("agent_message" | "agent_message_chunk")
    ) && update_message(&mut state.details.messages, update)
}

fn apply_suggestions(state: &mut ExternalAgentState, update: &Value) -> bool {
    let Some(items) = update["suggestions"].as_array() else {
        return false;
    };
    let next: Vec<_> = items.iter().take(8).filter_map(parse_suggestion).collect();
    if state.details.suggestions == next {
        return false;
    }
    state.details.suggestions = next;
    true
}

fn parse_suggestion(item: &Value) -> Option<ExternalAgentSuggestion> {
    let blocks = item["prompt"].as_array()?;
    if blocks.is_empty() || blocks.len() > 24 {
        return None;
    }
    let mut remaining = 4096_usize;
    let mut links = 0;
    let mut meaningful = false;
    let mut prompt = Vec::with_capacity(blocks.len());
    for block in blocks {
        let mut read = |value: &Value| -> Option<String> {
            let text = value.as_str()?;
            remaining = remaining.checked_sub(text.len())?;
            (!text
                .chars()
                .any(|ch| ch.is_control() && !matches!(ch, '\n' | '\r' | '\t')))
            .then(|| text.to_owned())
        };
        let content = match block["type"].as_str()? {
            "text" => {
                let text = read(&block["text"])?;
                meaningful |= !text.trim().is_empty();
                ExternalAgentSuggestionBlock::Text { text }
            }
            "resource_link" => {
                let uri = read(&block["uri"])?;
                let name = read(&block["name"])?;
                if links == 8
                    || name.trim().is_empty()
                    || uri.chars().any(char::is_whitespace)
                    || !url::Url::parse(&uri)
                        .is_ok_and(|url| !matches!(url.scheme(), "data" | "javascript"))
                {
                    return None;
                }
                let mime_type = if block["mimeType"].is_null() {
                    None
                } else {
                    Some(read(&block["mimeType"])?)
                };
                let size = if block["size"].is_null() {
                    None
                } else {
                    Some(block["size"].as_u64()?)
                };
                links += 1;
                meaningful = true;
                ExternalAgentSuggestionBlock::ResourceLink {
                    uri,
                    name,
                    mime_type,
                    size,
                }
            }
            _ => return None,
        };
        prompt.push(content);
    }
    meaningful.then_some(ExternalAgentSuggestion { prompt })
}

fn apply_terminal(state: &mut ExternalAgentState, source: &str, update: &Value) -> bool {
    let Some(id) = identifier(&update["terminalId"]) else {
        return false;
    };
    let terminals = &mut state.details.terminals;
    let current = terminals
        .iter()
        .position(|entry| entry.id == id && entry.session_id == source);
    let evict = if current.is_none() && terminals.len() >= MAX_TERMINALS {
        let Some(index) = terminals.iter().position(|entry| entry.exited) else {
            return false;
        };
        Some(index)
    } else {
        None
    };
    let mut next = current.map_or_else(
        || ExternalAgentTerminal {
            id: id.to_owned(),
            session_id: source.to_owned(),
            ..Default::default()
        },
        |index| terminals[index].clone(),
    );
    let key = (source.to_owned(), id.to_owned());
    let append = update["sessionUpdate"] == "terminal_output_chunk";
    if append {
        let Some(bytes) = terminal_bytes(&update["data"]) else {
            return false;
        };
        let raw = state
            .native_terminal_bytes
            .entry(key.clone())
            .or_insert_with(|| next.output.replace('\n', "\r\n").into_bytes());
        let len = raw.len().saturating_add(bytes.len());
        if len > MAX_TERMINAL_BYTES {
            raw.drain(..len.saturating_sub(MAX_TERMINAL_BYTES).min(raw.len()));
        }
        raw.extend(bytes);
    } else {
        if let Some(Value::String(command)) = update.get("command") {
            next.command = Some(bounded(command, 4096));
        } else if update.get("command") == Some(&Value::Null) {
            next.command = None;
        }
        if let Some(cwd) = update.get("cwd") {
            next.cwd = absolute_path(cwd);
        }
        if let Some(size) = update.get("size") {
            let dimensions = |key| {
                size[key]
                    .as_u64()
                    .and_then(|value| u16::try_from(value).ok())
                    .filter(|n| *n > 0)
            };
            next.rows = dimensions("rows");
            next.cols = dimensions("cols");
        }
        if let Some(output) = update.get("output") {
            if output.is_null() {
                state.native_terminal_bytes.remove(&key);
                next.output.clear();
            } else if let Some(bytes) = terminal_bytes(&output["data"]) {
                state.native_terminal_bytes.insert(key.clone(), bytes);
            }
        }
        if let Some(status) = update.get("exitStatus") {
            next.exited = status.is_object();
            next.exit_code = status["exitCode"]
                .as_i64()
                .and_then(|code| i32::try_from(code).ok());
            next.signal = nonempty_text(&status["signal"], 80);
        }
    }
    if let Some(raw) = state.native_terminal_bytes.get(&key) {
        let output = agena_runtime_tools::display_terminal_contents(
            raw,
            next.rows.unwrap_or(48),
            next.cols.unwrap_or(100),
        );
        next.output = output
            [output.ceil_char_boundary(output.len().saturating_sub(MAX_TERMINAL_BYTES))..]
            .to_owned();
    }
    match current {
        Some(i) if next != terminals[i] => terminals[i] = next,
        Some(_) => return false,
        None => {
            if let Some(i) = evict {
                let removed = terminals.remove(i);
                state
                    .native_terminal_bytes
                    .remove(&(removed.session_id, removed.id));
            }
            terminals.push(next);
        }
    }
    true
}

fn terminal_bytes(value: &Value) -> Option<Vec<u8>> {
    let text = value.as_str()?;
    let start = text
        .len()
        .saturating_sub(MAX_TERMINAL_BYTES / 3 * 4)
        .div_ceil(4)
        * 4;
    STANDARD.decode(text.get(start..)?).ok()
}

fn cost_value(value: &Value) -> Option<ExternalAgentCost> {
    let amount = value["amount"].as_number()?.clone();
    if !amount
        .as_f64()
        .is_some_and(|amount| amount.is_finite() && amount >= 0.0)
    {
        return None;
    }
    let currency = value["currency"].as_str()?;
    if currency.is_empty()
        || currency.len() > 32
        || currency
            .chars()
            .any(|ch| ch.is_control() || ch.is_whitespace())
    {
        return None;
    }
    Some(ExternalAgentCost {
        amount,
        currency: currency.to_owned(),
    })
}

fn apply_notice(state: &mut ExternalAgentState, source: Option<&str>, update: &Value) -> bool {
    let (Some(severity), Some(title)) = (
        nonempty_text(&update["severity"], 80),
        nonempty_text(&update["title"], 512),
    ) else {
        return false;
    };
    let notice = ExternalAgentNotice {
        severity,
        title,
        description: nonempty_text(&update["description"], 2048),
        session_id: source.map(str::to_owned),
    };
    if state.details.notices.last() == Some(&notice) {
        return false;
    }
    append_recent(&mut state.details.notices, notice, MAX_NOTICES);
    true
}

fn apply_compaction(state: &mut ExternalAgentState, update: &Value) -> bool {
    let Some(id) = identifier(&update["compactionId"]) else {
        return false;
    };
    let compactions = &mut state.details.compactions;
    let index = compactions.iter().position(|item| item.id == id);
    let mut next = index.map_or_else(
        || ExternalAgentCompaction {
            id: id.into(),
            status: "in_progress".into(),
            summary: String::new(),
            error: None,
        },
        |index| compactions[index].clone(),
    );
    if update["sessionUpdate"] == "compaction_summary_chunk" {
        append_content(&mut next.summary, &update["content"], MAX_SUMMARY_BYTES);
    } else {
        if let Some(status) = nonempty_text(&update["status"], 80) {
            next.status = status;
        }
        if let Some(summary) = update.get("summary").filter(|value| !value.is_null()) {
            next.summary = content_summary(summary, MAX_SUMMARY_BYTES);
        }
        if let Some(error) = update.get("error") {
            next.error = nonempty_text(error, 1024);
        }
    }
    match index {
        Some(i) if next != compactions[i] => compactions[i] = next,
        Some(_) => return false,
        None => append_recent(compactions, next, MAX_COMPACTIONS),
    }
    true
}

fn update_child(state: &mut ExternalAgentState, root: &str, parent: &str, update: &Value) -> bool {
    let Some(child_id) = identifier(&update["sessionId"]).filter(|id| *id != root && *id != parent)
    else {
        return false;
    };
    let children = &mut state.details.subagents;
    let prior = children.get(child_id);
    if prior.is_some_and(|child| child.parent_id != parent) {
        return false;
    }
    if prior.is_none() && children.len() >= MAX_CHILDREN {
        let Some(evict) = children
            .iter()
            .find(|(_, child)| {
                !matches!(child.state.as_deref(), Some("running" | "requires_action"))
            })
            .map(|(id, _)| id.clone())
        else {
            return false;
        };
        children.remove(&evict);
    }
    let prior = children.get(child_id);
    let mut next = prior.cloned().unwrap_or_else(|| ExternalAgentSubagent {
        parent_id: parent.to_owned(),
        ..Default::default()
    });
    if let Some(value) = update.get("title") {
        next.title = nonempty_text(value, 256);
    }
    if let Some(value) = update.get("description") {
        next.description = nonempty_text(value, 512);
    }
    if let Some(value) = update.get("state") {
        apply_child_state(&mut next, value);
    }
    if let Some(value) = update.get("cwd") {
        next.cwd = absolute_path(value);
    }
    if let Some(value) = update.get("additionalDirectories") {
        next.additional_directories = value
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(absolute_path)
            .take(8)
            .collect();
    }
    if prior == Some(&next) {
        return false;
    }
    children.insert(child_id.to_owned(), next);
    true
}

fn apply_child_state(child: &mut ExternalAgentSubagent, update: &Value) -> bool {
    let state = nonempty_text(&update["state"], 80);
    let stop_reason = nonempty_text(&update["stopReason"], 80);
    let error = nonempty_text(&update["error"]["message"], 512);
    if child.state == state && child.stop_reason == stop_reason && child.error == error {
        return false;
    }
    child.state = state;
    child.stop_reason = stop_reason;
    child.error = error;
    true
}

fn update_message(messages: &mut Vec<ExternalAgentMessage>, update: &Value) -> bool {
    let (role, replace) = match update["sessionUpdate"].as_str() {
        Some("agent_message_chunk") => ("assistant", false),
        Some("agent_thought_chunk") => ("reasoning", false),
        Some("user_message_chunk") => ("user", false),
        Some("agent_message") => ("assistant", true),
        Some("agent_thought") => ("reasoning", true),
        Some("user_message") => ("user", true),
        Some("session_message") => ("inter_session", true),
        Some("session_message_chunk") => ("inter_session", false),
        _ => return false,
    };
    let id = identifier(&update["messageId"]);
    if role == "inter_session" && id.is_none() {
        return false;
    }
    let index = id
        .and_then(|id| {
            messages
                .iter()
                .position(|message| message.id == id && message.role == role)
        })
        .or_else(|| {
            if id.is_none()
                && messages
                    .last()
                    .is_some_and(|item| item.id.is_empty() && item.role == role)
            {
                Some(messages.len() - 1)
            } else {
                None
            }
        });
    let mut next = index.map_or_else(
        || ExternalAgentMessage {
            id: id.unwrap_or_default().to_owned(),
            role: role.to_owned(),
            ..Default::default()
        },
        |i| messages[i].clone(),
    );
    if let Some(sender) = update.get("senderSessionId") {
        next.sender_id = identifier(sender).map(str::to_owned);
    }
    if let Some(recipient) = update.get("recipientSessionId") {
        next.recipient_id = identifier(recipient).map(str::to_owned);
    }
    if replace {
        if let Some(content) = update.get("content") {
            next.text = content_summary(content, MAX_MESSAGE_BYTES);
        }
    } else {
        append_content(&mut next.text, &update["content"], MAX_MESSAGE_BYTES);
    }
    match index {
        Some(i) if next != messages[i] => messages[i] = next,
        Some(_) => return false,
        None => append_recent(messages, next, MAX_MESSAGES),
    }
    true
}

fn content_summary(content: &Value, limit: usize) -> String {
    let mut text = String::new();
    for block in content.as_array().into_iter().flatten() {
        append_content(&mut text, block, limit);
    }
    text
}

fn append_content(text: &mut String, value: &Value, limit: usize) {
    let content = match value["type"].as_str() {
        Some("text") => value["text"].as_str(),
        Some("resource") => value["resource"]["text"]
            .as_str()
            .or_else(|| value["resource"]["uri"].as_str()),
        Some("resource_link") => value["uri"].as_str(),
        Some("image") => Some("[Image]"),
        Some("audio") => Some("[Audio]"),
        _ => None,
    };
    if let Some(content) = content {
        text.push_str(&bounded(content, limit.saturating_sub(text.len())));
    }
}

fn append_recent<T>(items: &mut Vec<T>, item: T, limit: usize) {
    if items.len() >= limit {
        items.remove(0);
    }
    items.push(item);
}

fn nonempty_text(value: &Value, limit: usize) -> Option<String> {
    let text = bounded(value.as_str()?.trim(), limit);
    (!text.is_empty()).then_some(text)
}

fn identifier(value: &Value) -> Option<&str> {
    value
        .as_str()
        .filter(|id| !id.is_empty() && id.len() <= 512 && !id.chars().any(char::is_control))
}

fn absolute_path(value: &Value) -> Option<String> {
    value
        .as_str()
        .filter(|path| {
            path.len() <= 2048
                && !path.chars().any(char::is_control)
                && Path::new(path).is_absolute()
        })
        .map(str::to_owned)
}

pub(super) fn available_commands(value: &Value) -> Option<Vec<ExternalAgentCommand>> {
    let mut names = HashSet::new();
    Some(
        value
            .as_array()?
            .iter()
            .filter_map(|command| {
                let skill = match command.get("type") {
                    Some(kind) if kind == "skill" => true,
                    Some(kind) if kind != "command" => return None,
                    _ => false,
                };
                let name = command_name(&command["name"])?;
                let description = if skill {
                    command["description"].as_str()?
                } else {
                    command["description"].as_str().unwrap_or_default()
                };
                if !names.insert(name.to_lowercase()) {
                    return None;
                }
                Some(ExternalAgentCommand {
                    name: name.to_owned(),
                    description: bounded(description.trim(), 512),
                    argument_hint: command
                        .get("input")
                        .filter(|input| {
                            !skill && input.get("type").is_none_or(|kind| kind == "text")
                        })
                        .and_then(|input| input["hint"].as_str())
                        .map(|hint| bounded(hint.trim(), 128)),
                    skill,
                    skill_inline: skill && command["inline"] == true,
                    skill_path: if skill {
                        absolute_path(&command["path"])
                    } else {
                        None
                    },
                })
            })
            .take(MAX_COMMANDS)
            .collect(),
    )
}

pub(super) fn command_name(value: &Value) -> Option<&str> {
    value
        .as_str()
        .filter(|name| ExternalAgentCommand::accepts_name(name))
}

fn plan_id(value: &Value) -> Option<&str> {
    value["planId"]
        .as_str()
        .filter(|id| !id.is_empty() && id.len() <= 256)
}

fn set_plan(state: &mut ExternalAgentState, id: &str, plan: Option<ExternalAgentPlan>) -> bool {
    match plan {
        Some(plan) if state.plans.get(id) != Some(&plan) => {
            if state.plans.len() >= MAX_PLANS && !state.plans.contains_key(id) {
                return false;
            }
            state.plans.insert(id.into(), plan);
            true
        }
        None => state.plans.remove(id).is_some(),
        _ => false,
    }
}

fn checklist(value: &Value) -> Option<Option<ExternalAgentPlan>> {
    use ExternalAgentPlanStatus::{Completed, InProgress, Pending};
    let values = value.as_array()?;
    if values.is_empty() {
        return Some(None);
    }
    let mut entries = Vec::new();
    let mut remaining = MAX_PLAN_BYTES;
    for item in values {
        if entries.len() == MAX_ENTRIES || remaining == 0 {
            break;
        }
        let Some(content) = item["content"]
            .as_str()
            .map(str::trim)
            .filter(|text| !text.is_empty())
        else {
            continue;
        };
        let status = match item["status"].as_str() {
            Some("pending") => Pending,
            Some("in_progress") => InProgress,
            Some("completed") => Completed,
            _ => continue,
        };
        let content = bounded(content, remaining);
        remaining -= content.len();
        if content.is_empty() {
            continue;
        }
        entries.push(ExternalAgentPlanEntry {
            content,
            status,
            priority: item["priority"]
                .as_str()
                .filter(|priority| matches!(*priority, "high" | "medium" | "low"))
                .map(str::to_owned),
        });
    }
    (!entries.is_empty()).then_some(Some(ExternalAgentPlan::Items { entries }))
}

fn context_usage(update: &Value) -> Option<ExternalAgentContextUsage> {
    let used_tokens = update["used"].as_u64()?;
    let window_tokens = u32::try_from(update["size"].as_u64()?).ok()?;
    (window_tokens > 0).then_some(ExternalAgentContextUsage {
        used_tokens,
        window_tokens,
    })
}

fn bounded(text: &str, max: usize) -> String {
    text[..text.floor_char_boundary(max)]
        .chars()
        .filter(|ch| !ch.is_control() || matches!(ch, '\n' | '\t'))
        .collect()
}
