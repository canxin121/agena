use std::{future::Future, path::PathBuf};

use agena_plugin_sdk::{
    HookHandlerDefinition, HookSubscription, HostClient, ImportedModelSwitchDecision,
    PluginManifest,
    host_api::{HookTranscriptFormat, LogLevel, current_host_callback_context},
};
use agena_process::sandbox;
use serde_json::{Map, Value, json};

use super::{Dialect, Effects, Event, HookInput};
use crate::agent_plugin::claude_mods::{Bundle, Mods};

const NATIVE_EVENTS: &[(&str, HookSubscription)] = &[
    ("session.start", HookSubscription::SESSION_START),
    ("session.end", HookSubscription::SESSION_END),
    ("command.run", HookSubscription::empty()),
    ("tool.call", HookSubscription::empty()),
    ("tool.check", HookSubscription::empty()),
    ("tool.run", HookSubscription::empty()),
    ("tool.describe", HookSubscription::TOOL_DEFINITION),
    ("tool.offer", HookSubscription::TOOL_DEFINITION),
    ("prompt.submit", HookSubscription::USER_PROMPT_SUBMIT),
];

impl Event {
    pub(super) fn classic_mod_name(self) -> String {
        format!("classic.{}", self.name())
    }
}

pub(in crate::agent_plugin) fn register_mod_events(bundle: &Bundle, manifest: &mut PluginManifest) {
    manifest.hooks.insert(HookSubscription::SESSION_END);
    for (event, subscription) in Event::ALL
        .iter()
        .filter(|event| !event.is_worktree())
        .map(|event| (event.classic_mod_name(), event.subscription()))
        .chain(
            NATIVE_EVENTS
                .iter()
                .map(|(event, subscription)| ((*event).to_owned(), *subscription)),
        )
    {
        if !bundle.matches(&event) || subscription.is_empty() {
            continue;
        }
        manifest.hooks.insert(subscription);
        manifest.hook_handlers.push(HookHandlerDefinition {
            id: format!("claude-mod-{event}"),
            event: subscription.names()[0].to_owned(),
            summary: format!("Claude mod {event}"),
            help: "Dispatches a function hook in the package's session runtime.".into(),
            translations: Default::default(),
            priority: 0,
            filters: Default::default(),
            is_async: false,
        });
    }
    if bundle.matches("classic.FileChanged") {
        manifest.hooks.insert(HookSubscription::SESSION_START);
    }
}

pub(in crate::agent_plugin) fn diagnose_mod_events(bundle: &Bundle, diagnostics: &mut Vec<String>) {
    for event in bundle.events() {
        let supported = matches!(event.as_str(), "*" | "classic.*")
            || event
                .strip_prefix("classic.")
                .and_then(Event::parse)
                .is_some_and(|event| !event.is_worktree())
            || NATIVE_EVENTS.iter().any(|(name, _)| {
                event == name
                    || event.strip_suffix(".*").is_some_and(|prefix| {
                        name.strip_prefix(prefix)
                            .is_some_and(|suffix| suffix.starts_with('.'))
                    })
            });
        if !supported {
            diagnostics.push(format!(
                "Claude mod event {event} has no Agena lifecycle mapping"
            ));
        }
    }
}

pub(super) async fn run_classic<'a, F, Fut>(
    mods: &Mods,
    host: &dyn HostClient,
    module_event: &str,
    event: Event,
    session: i64,
    mut input: HookInput<'a>,
    terminal: F,
) -> Effects
where
    F: FnOnce(HookInput<'a>) -> Fut + Send,
    Fut: Future<Output = Effects> + Send,
{
    let mut envelope = input.fields.clone();
    if let Some(tool) = input.claude_mcp {
        tool.apply(&mut envelope);
    }
    let mut transcripts = None;
    if event == Event::PreToolUse {
        envelope = tool_envelope(envelope);
    } else {
        let context = current_host_callback_context().unwrap_or_default();
        envelope["hook_event_name"] = json!(event.name());
        if envelope.get("session_id").is_none() {
            envelope["session_id"] = json!(session.to_string());
        }
        envelope["cwd"] = json!(
            envelope["new_cwd"]
                .as_str()
                .or(envelope["cwd"].as_str())
                .or(context.workspace_root.as_deref())
                .unwrap_or("")
        );
        envelope["transcript_path"] = json!("");
        if mods.matches(module_event) && sandbox::enabled() {
            match tokio::time::timeout(
                std::time::Duration::from_secs(5),
                host.imported_hook_transcripts(
                    HookTranscriptFormat::Claude,
                    super::process::transcript_scope(event, Dialect::Claude),
                    None,
                ),
            )
            .await
            {
                Ok(Ok(files)) => {
                    if let Some(file) = files.transcript.as_ref() {
                        envelope["transcript_path"] = json!(file.path());
                    }
                    if let Some(file) = files.agent.as_ref() {
                        envelope["agent_transcript_path"] = json!(file.path());
                    }
                    transcripts = Some(files);
                }
                Ok(Err(error)) => {
                    host.log(
                        LogLevel::Warn,
                        error.diagnostic_message().to_owned(),
                        json!({"hook": event.name(), "component": "transcript_path"}),
                    )
                    .await;
                }
                Err(_) => {
                    host.log(
                        LogLevel::Warn,
                        "hook transcript preparation timed out".into(),
                        json!({"hook": event.name(), "component": "transcript_path"}),
                    )
                    .await;
                }
            }
        }
        if envelope.get("permission_mode").is_none() {
            envelope["permission_mode"] = json!("default");
        }
    }

    let mut core = None;
    let core_result = &mut core;
    let original = &envelope;
    let answer = mods
        .run_with_transcripts(
            host,
            session,
            module_event,
            envelope.clone(),
            transcripts.as_ref(),
            |mut changed| async move {
                let rewritten = if event == Event::PreToolUse {
                    if changed["tool"] != original["tool"]
                        || changed
                            .get("tool_use_id")
                            .is_some_and(|id| id != &original["tool_use_id"])
                    {
                        return json!({"deny":"Claude mod cannot change the tool or call identity"});
                    }
                    let fields = changed
                        .as_object_mut()
                        .expect("mod continuation receives an object");
                    for key in ["tool", "tool_use_id"] {
                        if let Some(value) = input.fields["tool_input"].get(key) {
                            fields.insert(key.into(), value.clone());
                        } else {
                            fields.remove(key);
                        }
                    }
                    let rewritten =
                        (input.fields["tool_input"] != changed).then(|| changed.clone());
                    input.fields["tool_input"] = changed;
                    rewritten
                } else {
                    // Keep identity and paths bound to the Agena callback context.
                    let identity_fields = [
                        "hook_event_name",
                        "session_id",
                        "agent_id",
                        "cwd",
                        "transcript_path",
                        "agent_transcript_path",
                        "permission_mode",
                        "tool_name",
                        "tool_use_id",
                        "mcp_server",
                    ];
                    let event_fields: &[&str] = match event {
                        Event::MessageDisplay => &["turn_id", "message_id", "index", "final"],
                        Event::ConfigChange => &["source", "file_path"],
                        Event::DirectoryAdded => &["source", "directory"],
                        Event::Stop | Event::SubagentStop => &["background_tasks", "session_crons"],
                        Event::TeammateIdle => &[
                            "teammate_name",
                            "team_name",
                            "background_tasks",
                            "session_crons",
                        ],
                        _ => &[],
                    };
                    for name in identity_fields.into_iter().chain(event_fields.iter().copied()) {
                        if let Some(value) = input.fields.get(name) {
                            changed[name] = value.clone();
                        } else if let Some(fields) = changed.as_object_mut() {
                            fields.remove(name);
                        }
                    }
                    input.fields = changed;
                    None
                };
                let mut effects = terminal(input).await;
                effects.input = effects.input.or(rewritten);
                let projected = project(event, &effects);
                *core_result = Some((projected.clone(), effects));
                projected
            },
        )
        .await;
    if event == Event::DirectoryAdded {
        if let Some(message) = answer["systemMessage"].as_str() {
            let end = message.floor_char_boundary(message.len().min(8192));
            host.log(
                agena_plugin_sdk::host_api::LogLevel::Debug,
                message[..end].to_owned(),
                json!({"hook": event.name(), "source": "register_repo_root"}),
            ).await;
        }
        return Effects::default();
    }
    match core {
        Some((projected, effects)) if answer == projected => effects,
        core => apply(
            event,
            answer,
            core.map_or_else(Effects::default, |(_, effects)| effects),
        ),
    }
}

fn tool_envelope(fields: Value) -> Value {
    let mut output = fields["tool_input"]
        .as_object()
        .cloned()
        .unwrap_or_default();
    output.insert("tool".into(), fields["tool_name"].clone());
    output.insert("tool_use_id".into(), fields["tool_use_id"].clone());
    Value::Object(output)
}

fn project(event: Event, effects: &Effects) -> Value {
    let mut answer = Map::new();
    let mut insert = |name: &str, value: Option<Value>| {
        if let Some(value) = value {
            answer.insert(name.to_owned(), value);
        }
    };
    if event == Event::PreToolUse {
        insert("deny", effects.block.as_ref().map(|reason| json!(reason)));
        insert(
            "ask",
            effects.permission_ask.as_ref().map(|reason| json!(reason)),
        );
        insert("updatedInput", effects.input.clone());
    } else {
        if event == Event::PermissionRequest {
            insert(
                "decision",
                effects.block.as_ref().map(|reason| {
                    json!({
                        "behavior":"deny", "message":reason,
                    })
                }),
            );
        } else {
            insert("block", effects.block.as_ref().map(|reason| json!(reason)));
        }
        insert("updatedToolOutput", effects.output.clone());
        if event == Event::MessageDisplay {
            insert("displayContent", effects.display_content.as_ref().map(|text| json!(text)));
        }
        insert(
            "initialUserMessage",
            effects.initial_message.as_ref().map(|text| json!(text)),
        );
        insert(
            "sessionTitle",
            effects.session_title.as_ref().map(|text| json!(text)),
        );
        insert(
            "reloadSkills",
            effects.reload_skills.then_some(Value::Bool(true)),
        );
        insert(
            "watchPaths",
            effects.watch_paths.as_ref().map(|paths| json!(paths)),
        );
        if let Some(decision) = &effects.model_switch.decision {
            let (kind, reason) = match decision {
                ImportedModelSwitchDecision::Allow => ("allow", None),
                ImportedModelSwitchDecision::Ask(reason) => ("ask", Some(reason)),
                ImportedModelSwitchDecision::Deny(reason) => ("deny", Some(reason)),
            };
            insert("permissionDecision", Some(json!(kind)));
            insert("permissionDecisionReason", reason.map(|text| json!(text)));
        }
        if let Some(reason) = &effects.stop_skill {
            insert("preventContinuation", Some(json!(true)));
            insert("stopReason", Some(json!(reason)));
        }
    }
    insert(
        "additionalContext",
        (!effects.context.is_empty()).then(|| json!(effects.context)),
    );
    Value::Object(answer)
}

fn apply(event: Event, mut answer: Value, mut effects: Effects) -> Effects {
    let Some(fields) = answer.as_object_mut() else {
        return effects;
    };
    let text = |value: Option<Value>, limit| {
        let mut text = value?.as_str()?.to_owned();
        text.truncate(text.floor_char_boundary(text.len().min(limit)));
        (!text.trim().is_empty()).then_some(text)
    };
    if event == Event::PreToolUse {
        effects.block = text(fields.remove("deny"), 8192);
        effects.permission_ask = text(fields.remove("ask"), 8192);
        effects.input = fields.remove("updatedInput").filter(Value::is_object);
    } else if event.can_block_on_failure(Dialect::Claude)
        || matches!(event, Event::Stop | Event::SubagentStop | Event::TeammateIdle)
    {
        effects.block = text(fields.remove("block"), 8192);
        if fields.remove("preventContinuation") == Some(json!(true)) {
            let reason = text(fields.remove("stopReason"), 8192)
                .unwrap_or_else(|| format!("{} hook stopped the action", event.name()));
            if event == Event::PreSkillShellCommand {
                effects.stop_skill = Some(reason);
            } else if !matches!(event, Event::Stop | Event::SubagentStop | Event::TeammateIdle) {
                effects.block.get_or_insert(reason);
            }
        }
    }
    if let Some(context) = fields
        .remove("additionalContext")
        .and_then(|value| serde_json::from_value::<Vec<String>>(value).ok())
    {
        let mut budget: usize = 16 * 1024;
        effects.context = context
            .into_iter()
            .map_while(|mut message| {
                if budget == 0 {
                    return None;
                }
                message.truncate(message.floor_char_boundary(message.len().min(budget)));
                budget = budget.saturating_sub(message.len() + 1);
                Some(message)
            })
            .filter(|message| !message.is_empty())
            .collect();
    }
    match event {
        Event::MessageDisplay => {
            effects.display_content = fields
                .remove("displayContent")
                .and_then(|value| value.as_str().filter(|text| text.len() <= 65_536).map(ToOwned::to_owned));
        }
        Event::SessionStart => {
            effects.initial_message = text(fields.remove("initialUserMessage"), 16 * 1024);
            effects.session_title = text(fields.remove("sessionTitle"), 4096)
                .filter(|title| !title.chars().any(char::is_control));
            effects.reload_skills = fields.remove("reloadSkills") == Some(Value::Bool(true));
        }
        Event::PostToolUse => {
            effects.output = fields
                .remove("updatedToolOutput")
                .or_else(|| fields.remove("updatedMCPToolOutput"));
        }
        Event::PermissionDenied if fields.get("retry") == Some(&Value::Bool(true)) => {
            effects
                .context
                .push("A PermissionDenied hook suggested retrying the tool.".into());
        }
        Event::PermissionRequest => {
            if let Some(decision) = fields.get("decision")
                && decision["behavior"] == "deny"
            {
                effects.block = Some(
                    text(decision.get("message").cloned(), 8192)
                        .unwrap_or_else(|| "PermissionRequest hook blocked the action".into()),
                );
            }
        }
        Event::PreModelSwitch => {
            let reason = || {
                text(fields.get("permissionDecisionReason").cloned(), 8192)
                    .unwrap_or_else(|| "PreModelSwitch hook requested a permission decision".into())
            };
            effects.model_switch.decision =
                match fields.get("permissionDecision").and_then(Value::as_str) {
                    Some("allow") => Some(ImportedModelSwitchDecision::Allow),
                    Some("ask") => Some(ImportedModelSwitchDecision::Ask(reason())),
                    Some("deny") => Some(ImportedModelSwitchDecision::Deny(reason())),
                    _ => None,
                };
        }
        _ => {}
    }
    if matches!(
        event,
        Event::SessionStart | Event::FileChanged | Event::CwdChanged
    ) && let Some(paths) = fields
        .remove("watchPaths")
        .and_then(|value| serde_json::from_value::<Vec<PathBuf>>(value).ok())
    {
        effects.watch_paths = Some(paths);
    }
    effects
}
