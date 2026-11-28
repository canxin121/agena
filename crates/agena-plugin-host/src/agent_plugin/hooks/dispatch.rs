use std::{borrow::Cow, path::PathBuf, process::Output, sync::Arc};

use agena_plugin_sdk::{
    AgentCancelInput, AgentStopInput, AgentStopPatch, CommandHostEffect, CommandInvokeInput,
    CommandResult, CompactionAfterInput, CompactionBeforeInput, CompactionBeforePatch, HostClient,
    ImportedModelAttemptCharge, ImportedModelAttemptInput, InitContext, InitOutcome,
    PermissionRequestInput, PermissionRequestPatch, Plugin, PluginError, PluginManifest,
    Result as PluginResult, SessionCwdChangedInput, SessionEndInput, SessionEndReason,
    SessionFileChangedInput, SessionFileChangedPatch, SessionStartInput, SessionStartPatch,
    SessionStartSource, SubagentStartInput, SubagentStartPatch, ToolAfterInput, ToolAfterPatch,
    ToolBatchInput, ToolBatchPatch, ToolBeforeInput, ToolBeforePatch, ToolFailureInput,
    ToolInvokeInput, ToolInvokeOutput, ToolKey, UserPromptExpansionInput, UserPromptExpansionPatch,
    UserPromptSubmitInput, UserPromptSubmitPatch,
};
use agena_plugin_sdk::{
    McpElicitation, McpElicitationAction, McpElicitationReply, ProcessShell, ShellLaunchMode,
};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{ClaudeMcpTool, CodexShellHook, Dialect, Effects, Event, HookInput, HookScope};
use crate::agent_plugin::{
    AgentPluginAdapter, FORKED_SKILL_COMMAND, ForkedSkillInvocation, bridge,
    markdown::CommandExecution, workflow,
};

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct Reply {
    #[serde(rename = "continue")]
    continue_run: Option<bool>,
    stop_reason: Option<String>,
    system_message: Option<String>,
    reason: Option<String>,
    decision: Option<Value>,
    hook_specific_output: Option<SpecificReply>,
    watch_paths: Option<Vec<PathBuf>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SpecificReply {
    hook_event_name: String,
    display_content: Option<String>,
    additional_context: Option<String>,
    initial_user_message: Option<String>,
    session_title: Option<String>,
    reload_skills: Option<bool>,
    permission_decision: Option<String>,
    permission_decision_reason: Option<String>,
    decision: Option<Value>,
    updated_input: Option<serde_json::Map<String, Value>>,
    updated_tool_output: Option<Value>,
    updated_mcp_tool_output: Option<Value>,
    watch_paths: Option<Vec<PathBuf>>,
    retry: Option<bool>,
    worktree_path: Option<PathBuf>,
    action: Option<Value>,
    content: Option<Value>,
}

#[derive(Default)]
pub(super) struct HookOutput {
    pub(super) system_message: Option<String>,
    pub(super) effects: Effects,
    pub(super) successful: bool,
}

pub(super) fn background_message(event: Event, stdout: &str) -> Option<String> {
    if event.is_model_attempt() {
        return None;
    }
    let text = stdout.trim();
    let reply = serde_json::from_str::<Reply>(text).ok().or_else(|| {
        text.lines()
            .rev()
            .find_map(|line| serde_json::from_str(line.trim()).ok())
    })?;
    let context = reply
        .hook_specific_output
        .filter(|output| output.hook_event_name == event.name())
        .and_then(|output| output.additional_context);
    joined_context(
        context
            .into_iter()
            .chain(reply.system_message)
            .filter(|text| !text.trim().is_empty())
            .collect(),
    )
}

#[cfg(test)]
#[test]
fn background_output_reads_the_last_json_line_and_ignores_decisions() {
    let output = r#"running check
{"decision":"block","systemMessage":"check finished","hookSpecificOutput":{"hookEventName":"PostToolUse","additionalContext":"fixture context"}}
done"#;
    assert_eq!(
        background_message(Event::PostToolUse, output).as_deref(),
        Some("fixture context\ncheck finished")
    );
    assert_eq!(
        background_message(Event::PreToolUse, r#"{"decision":"block"}"#),
        None
    );
    assert_eq!(
        background_message(
            Event::PostModelRequest,
            r#"{"systemMessage":"observer output"}"#
        ),
        None
    );
}

#[cfg(test)]
#[test]
fn codex_session_start_accepts_successful_stdout_and_can_stop_a_turn() {
    let output = decode_text(
        Dialect::Codex,
        Event::SessionStart,
        Some(0),
        "restore these facts",
        "",
    )
    .unwrap();
    assert_eq!(output.effects.context, ["restore these facts"]);
    let output = decode_text(
        Dialect::Codex,
        Event::SessionStart,
        Some(1),
        "command failed",
        "",
    )
    .unwrap();
    assert!(output.effects.context.is_empty());
    let output = decode_text(
        Dialect::Codex,
        Event::SessionStart,
        Some(0),
        r#"{"continue":false,"stopReason":"check the checkpoint","hookSpecificOutput":{"hookEventName":"SessionStart","additionalContext":"carry this into the next turn"}}"#,
        "",
    )
    .unwrap();
    assert_eq!(
        output.effects.block.as_deref(),
        Some("check the checkpoint")
    );
    assert_eq!(output.effects.context, ["carry this into the next turn"]);
}

#[cfg(test)]
#[test]
fn claude_session_start_reloads_skills_on_a_successful_hook_reply() {
    let reply = r#"{"hookSpecificOutput":{"hookEventName":"SessionStart","reloadSkills":true}}"#;
    for (dialect, code, reload_skills) in [
        (Dialect::Claude, 0, true),
        (Dialect::Claude, 1, false),
        (Dialect::Codex, 0, false),
    ] {
        let output = decode_text(dialect, Event::SessionStart, Some(code), reply, "").unwrap();
        assert_eq!(output.effects.reload_skills, reload_skills);
    }
}

#[cfg(test)]
#[test]
fn claude_task_exit_two_uses_stderr_and_continue_does_not_block_tool_updates() {
    for event in [Event::TaskCreated, Event::TaskCompleted] {
        let output =
            decode_text(Dialect::Claude, event, Some(2), "{invalid", "keep working").unwrap();
        assert_eq!(output.effects.block.as_deref(), Some("keep working"));
        let output =
            decode_text(Dialect::Claude, event, Some(0), r#"{"continue":false}"#, "").unwrap();
        assert!(output.effects.block.is_none());
    }
}

#[cfg(test)]
#[test]
fn claude_precompact_stdout_supplies_instructions_and_discards_system_messages() {
    let output = decode_text(
        Dialect::Claude,
        Event::PreCompact,
        Some(0),
        "[Keep exact filenames]",
        "",
    )
    .unwrap();
    assert_eq!(output.effects.context, ["[Keep exact filenames]"]);
    let output = decode_text(
        Dialect::Claude,
        Event::PreCompact,
        Some(0),
        r#"{"continue":false,"systemMessage":"ignored"}"#,
        "",
    )
    .unwrap();
    assert!(output.effects.block.is_none());
    assert!(output.effects.context.is_empty());
    assert!(output.system_message.is_none());
}

#[cfg(test)]
#[test]
fn claude_elicitation_blocks_override_action_and_discard_display_fields() {
    for event in [Event::Elicitation, Event::ElicitationResult] {
        let accept =
            json!({"hookEventName":event.name(),"action":"accept","content":{"value":"ok"}});
        for (code, output) in [
            (2, "{ malformed".to_owned()),
            (
                0,
                json!({"decision":"block","hookSpecificOutput":accept}).to_string(),
            ),
        ] {
            let decoded =
                decode_text(Dialect::Claude, event, Some(code), &output, "fixture").unwrap();
            assert_eq!(
                decoded.effects.elicitation,
                Some(McpElicitationReply::new(McpElicitationAction::Decline)),
            );
        }
        let output =
            json!({"continue":false,"systemMessage":"ignored","hookSpecificOutput":accept});
        let decoded =
            decode_text(Dialect::Claude, event, Some(0), &output.to_string(), "").unwrap();
        assert!(decoded.system_message.is_none());
        assert!(decoded.effects.block.is_none());
        assert_eq!(
            decoded.effects.elicitation.unwrap().content,
            Some(json!({"value":"ok"}))
        );
    }
}

#[cfg(test)]
#[test]
fn claude_mcp_tool_names_keep_scope_and_trusted_server_provenance() {
    use agena_plugin_sdk::host_api::ImportedMcpServerIdentity;

    let tool = claude_mcp_tool(
        "ignored",
        "remote.ping",
        Some(ImportedMcpServerIdentity {
            package: "my-plugin".into(),
            server: "my.db".into(),
        }),
    );
    assert_eq!(tool.tool_name, "mcp__plugin_my-plugin_my_db__remote_ping");
    assert_eq!(
        tool.provenance,
        json!({"name":"plugin:my-plugin:my.db","source":"plugin"})
    );
    let configured = claude_mcp_tool("agent-plugin-claimed", "ping", None);
    assert_eq!(configured.provenance["source"], "agena");
    assert_eq!(configured.provenance["name"], "agent-plugin-claimed");
}

#[cfg(test)]
#[test]
fn classic_pre_tool_approval_keeps_the_rewritten_input_and_context() {
    let effects = decode_text(
        Dialect::Claude,
        Event::PreToolUse,
        Some(0),
        r#"{"hookSpecificOutput":{"hookEventName":"PreToolUse",
            "permissionDecision":"ask","permissionDecisionReason":"review effective command",
            "updatedInput":{"command":"printf updated"},"additionalContext":"approval context"}}"#,
        "",
    )
    .unwrap()
    .effects;
    assert!(effects.block.is_none());
    assert_eq!(
        effects.permission_ask.as_deref(),
        Some("review effective command")
    );
    assert_eq!(effects.input, Some(json!({"command": "printf updated"})));
    assert_eq!(effects.context, ["approval context"]);
}

#[cfg(test)]
#[test]
fn claude_skill_shell_distinguishes_snippet_veto_from_stopping_the_skill() {
    let decode = |code, reply, stderr| {
        decode_text(
            Dialect::Claude,
            Event::PreSkillShellCommand,
            Some(code),
            reply,
            stderr,
        )
        .unwrap()
        .effects
    };
    let skipped = decode(2, "", "snippet veto");
    assert_eq!(skipped.block.as_deref(), Some("snippet veto"));
    assert!(skipped.stop_skill.is_none());

    let rewritten = decode(
        0,
        r#"{"hookSpecificOutput":{"hookEventName":"PreToolUse","updatedInput":{"command":"printf overridden"}}}"#,
        "",
    );
    assert!(rewritten.block.is_some());
    assert!(rewritten.input.is_none());

    let stopped = decode(
        0,
        r#"{"continue":false,"stopReason":"entire skill veto","decision":"block"}"#,
        "",
    );
    assert_eq!(stopped.stop_skill.as_deref(), Some("entire skill veto"));
    assert!(stopped.block.is_none());
}

#[cfg(test)]
#[test]
fn claude_model_switch_denial_precedes_ask_and_post_switch_is_context_only() {
    use agena_plugin_sdk::{ImportedModelSwitchDecision, ImportedModelSwitchPatch};

    let reply = |event, code, text, stderr| {
        decode_text(Dialect::Claude, event, Some(code), text, stderr).unwrap()
    };
    let ask = reply(
        Event::PreModelSwitch,
        0,
        r#"{"hookSpecificOutput":{"hookEventName":"PreModelSwitch","permissionDecision":"ask","permissionDecisionReason":"Ask first"}}"#,
        "",
    );
    let deny = reply(Event::PreModelSwitch, 2, "", "Model unavailable");
    let mut patch = ImportedModelSwitchPatch::default();
    patch.apply_decision(ask.effects.model_switch.decision.unwrap());
    patch.apply_decision(deny.effects.model_switch.decision.unwrap());
    patch.apply_decision(ImportedModelSwitchDecision::Allow);
    assert!(
        matches!(patch.decision, Some(ImportedModelSwitchDecision::Deny(reason)) if reason == "Model unavailable")
    );

    let post = reply(
        Event::PostModelSwitch,
        0,
        r#"{"decision":"block","hookSpecificOutput":{"hookEventName":"PostModelSwitch","additionalContext":"New route instructions"}}"#,
        "",
    );
    assert_eq!(post.effects.context, ["New route instructions"]);
    assert!(post.effects.model_switch.decision.is_none());
    assert!(post.effects.block.is_none());
}

pub(super) fn decode_output(
    dialect: Dialect,
    event: Event,
    output: Output,
) -> Result<HookOutput, String> {
    let code = output.status.code();
    if !matches!(code, Some(0 | 2)) {
        return Err(format!("hook exited with {}", output.status));
    }
    if event.is_elicitation() && code == Some(2) {
        return Ok(declined_elicitation(false));
    }
    if matches!(event, Event::PreSkillShellCommand | Event::PreModelSwitch | Event::ConfigChange)
        && code == Some(2) {
        return decode_text(
            dialect,
            event,
            code,
            "",
            &String::from_utf8_lossy(&output.stderr),
        );
    }
    let text = std::str::from_utf8(&output.stdout).map_err(|_| "hook stdout is not UTF-8")?;
    decode_text(
        dialect,
        event,
        code,
        text,
        &String::from_utf8_lossy(&output.stderr),
    )
}

pub(super) fn decode_worktree(
    event: Event,
    output: Output,
    http: bool,
) -> Result<HookOutput, String> {
    if !output.status.success() {
        return Err(format!("hook exited with {}", output.status));
    }
    let result = if event == Event::WorktreeCreate {
        let stdout = std::str::from_utf8(&output.stdout).map_err(|_| "hook stdout is not UTF-8")?;
        agena_plugin_sdk::WorktreeHookOutput::Created {
            path: worktree_output_path(stdout, http)?,
        }
    } else {
        agena_plugin_sdk::WorktreeHookOutput::Removed
    };
    Ok(HookOutput {
        successful: true,
        effects: Effects {
            worktree: Some(result),
            ..Default::default()
        },
        ..Default::default()
    })
}

fn worktree_output_path(stdout: &str, http: bool) -> Result<PathBuf, String> {
    if http {
        return serde_json::from_str::<Reply>(stdout)
            .map_err(|_| "invalid worktree hook JSON output".to_owned())?
            .hook_specific_output
            .filter(|output| output.hook_event_name == Event::WorktreeCreate.name())
            .and_then(|output| output.worktree_path)
            .filter(|path| !path.as_os_str().is_empty())
            .ok_or_else(|| "WorktreeCreate HTTP hook returned no worktreePath".into());
    }
    // Strip common terminal sequences before selecting the path line.
    static ANSI: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"\x1b(?:\[[0-?]*[ -/]*[@-~]|\][^\x07\x1b]*(?:\x07|\x1b\\)|[@-Z\\-_])")
            .expect("valid ANSI escape regex")
    });
    let text = ANSI.replace_all(stdout, "");
    text.lines()
        .rev()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| "WorktreeCreate command returned no worktree path".into())
}

#[cfg(test)]
#[test]
fn imported_claude_worktree_paths_follow_the_handler_transport() {
    assert_eq!(
        worktree_output_path("banner\n\x1b[32mcheckout with spaces\x1b[0m\n\n", false).unwrap(),
        PathBuf::from("checkout with spaces"),
    );
    let reply = r#"{"hookSpecificOutput":{"hookEventName":"WorktreeCreate","worktreePath":"/fixture/tree"}}"#;
    assert_eq!(
        worktree_output_path(reply, true).unwrap(),
        PathBuf::from("/fixture/tree")
    );
    assert!(worktree_output_path("/fixture/tree", true).is_err());
    assert!(worktree_output_path(" \n\x1b[0m\n", false).is_err());
}

pub(super) fn decode_text(
    dialect: Dialect,
    event: Event,
    code: Option<i32>,
    stdout: &str,
    stderr: &str,
) -> Result<HookOutput, String> {
    if event == Event::DirectoryAdded && code != Some(0) {
        let status = code.map_or_else(
            || "terminated".into(),
            |code| format!("exited with status {code}"),
        );
        let stderr = stderr.trim();
        let end = stderr.floor_char_boundary(stderr.len().min(8192));
        return Err(format!("DirectoryAdded hook {status}\n{}", &stderr[..end]).trim_end().to_owned());
    }
    if matches!(event, Event::Setup | Event::InstructionsLoaded) || event.is_model_attempt() {
        return Ok(HookOutput {
            successful: code == Some(0),
            ..Default::default()
        });
    }
    if event == Event::ConfigChange && code == Some(2) {
        return Ok(HookOutput {
            effects: Effects {
                block: Some(if stderr.trim().is_empty() {
                    "ConfigChange hook blocked the change".into()
                } else {
                    stderr.trim().to_owned()
                }),
                ..Default::default()
            },
            ..Default::default()
        });
    }
    if event == Event::MessageDisplay && code != Some(0) {
        return Ok(HookOutput::default());
    }
    if event.is_elicitation() && code == Some(2) {
        return Ok(declined_elicitation(false));
    }
    let text = stdout.trim();
    let task_event = matches!(event, Event::TaskCreated | Event::TaskCompleted);
    let claude_compaction = dialect == Dialect::Claude && event == Event::PreCompact;
    let parse_reply = text.starts_with(['{', '['])
        && (code == Some(0) || (dialect == Dialect::Claude && !task_event && !claude_compaction));
    let parsed = parse_reply.then(|| serde_json::from_str::<Reply>(text));
    let has_reply = matches!(parsed, Some(Ok(_)));
    if dialect == Dialect::Codex
        && code == Some(0)
        && !text.is_empty()
        && !has_reply
        && matches!(event, Event::Stop | Event::SubagentStop | Event::Interrupt)
    {
        return Err("Codex hook output must be a JSON object".into());
    }
    let mut reply = match parsed {
        Some(Ok(reply)) => reply,
        Some(Err(_)) if !claude_compaction => return Err("invalid hook JSON output".into()),
        _ => Reply::default(),
    };
    if event.is_elicitation() && reply.decision.as_ref().and_then(Value::as_str) == Some("block") {
        return Ok(declined_elicitation(true));
    }
    if reply.hook_specific_output.as_ref().is_some_and(|specific| {
        specific.hook_event_name != event.name()
            && !(event == Event::PreSkillShellCommand && specific.hook_event_name == "PreToolUse")
    }) {
        return Err("hook output specifies a different event".into());
    }
    if event.is_elicitation() {
        let elicitation = reply
            .hook_specific_output
            .and_then(|specific| specific.action.map(|action| (action, specific.content)))
            .map(|(action, content)| {
                serde_json::from_value(action)
                    .map(|action| McpElicitationReply { action, content })
                    .map_err(|_| "invalid elicitation action".to_owned())
            })
            .transpose()?;
        return Ok(HookOutput {
            successful: code == Some(0),
            effects: Effects {
                elicitation,
                ..Default::default()
            },
            ..Default::default()
        });
    }

    let mut effects = Effects::default();
    let specific = reply.hook_specific_output.take();
    if dialect == Dialect::Claude
        && code == Some(0)
        && matches!(
            event,
            Event::SessionStart | Event::FileChanged | Event::CwdChanged
        )
    {
        effects.watch_paths = reply.watch_paths.or_else(|| {
            specific
                .as_ref()
                .and_then(|specific| specific.watch_paths.clone())
        });
    }
    let decision = reply.decision.as_ref().and_then(Value::as_str);
    let exit_blocks = code == Some(2);
    let reason = |specific: Option<&str>| {
        specific
            .filter(|text| !text.trim().is_empty())
            .or(reply
                .reason
                .as_deref()
                .filter(|text| !text.trim().is_empty()))
            .or(reply
                .stop_reason
                .as_deref()
                .filter(|text| !text.trim().is_empty()))
            .or_else(|| (exit_blocks && !stderr.trim().is_empty()).then(|| stderr.trim()))
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| format!("{} hook blocked the action", event.name()))
    };

    match event {
        Event::MessageDisplay => {
            effects.display_content = specific.and_then(|reply| reply.display_content);
        }
        Event::ConfigChange => {
            if decision == Some("block") {
                effects.block = Some(reason(None));
            }
        }
        Event::PreModelSwitch => {
            use agena_plugin_sdk::ImportedModelSwitchDecision;
            let permission = specific
                .as_ref()
                .and_then(|reply| reply.permission_decision.as_deref());
            let explanation = || {
                reason(
                    specific
                        .as_ref()
                        .and_then(|reply| reply.permission_decision_reason.as_deref()),
                )
            };
            effects.model_switch.decision =
                if exit_blocks || decision == Some("block") || permission == Some("deny") {
                    Some(ImportedModelSwitchDecision::Deny(explanation()))
                } else if permission == Some("ask") {
                    Some(ImportedModelSwitchDecision::Ask(explanation()))
                } else if permission == Some("allow") {
                    Some(ImportedModelSwitchDecision::Allow)
                } else {
                    None
                };
        }
        Event::PostModelSwitch if code == Some(0) => {
            if let Some(specific) = specific {
                effects.context.extend(specific.additional_context);
            } else if !has_reply && !text.is_empty() {
                effects.context.push(text.to_owned());
            }
        }
        Event::PreSkillShellCommand => {
            if reply.continue_run == Some(false) {
                effects.stop_skill = Some(reason(reply.stop_reason.as_deref()));
            } else {
                let permission = specific
                    .as_ref()
                    .and_then(|output| output.permission_decision.as_deref());
                if exit_blocks
                    || decision == Some("block")
                    || matches!(permission, Some("deny" | "ask" | "defer"))
                    || specific
                        .as_ref()
                        .is_some_and(|output| output.updated_input.is_some())
                {
                    effects.block =
                        Some(reason(specific.as_ref().and_then(|output| {
                            output.permission_decision_reason.as_deref()
                        })));
                }
            }
        }
        Event::PreToolUse => {
            let permission = specific
                .as_ref()
                .and_then(|specific| specific.permission_decision.as_deref());
            if exit_blocks || decision == Some("block") || permission == Some("deny") {
                effects.block =
                    Some(reason(specific.as_ref().and_then(|specific| {
                        specific.permission_decision_reason.as_deref()
                    })));
            } else if permission == Some("defer") {
                effects.block = Some(reason(Some("Hook deferred the tool call")));
            } else if let Some(specific) = specific {
                if specific.permission_decision.as_deref() == Some("ask") {
                    effects.permission_ask = Some(
                        specific
                            .permission_decision_reason
                            .as_deref()
                            .or(reply.reason.as_deref())
                            .filter(|text| !text.trim().is_empty())
                            .unwrap_or("PreToolUse hook requested confirmation for this tool")
                            .to_owned(),
                    );
                }
                effects.input = specific.updated_input.map(Value::Object);
                effects.context.extend(specific.additional_context);
            }
        }
        Event::PermissionRequest => {
            let permission = specific
                .as_ref()
                .and_then(|specific| specific.decision.as_ref())
                .or(reply.decision.as_ref());
            let deny = permission
                .and_then(|permission| permission.get("behavior"))
                .and_then(Value::as_str)
                == Some("deny");
            if deny || (exit_blocks && dialect == Dialect::Codex) {
                let message = permission
                    .and_then(|permission| permission.get("message"))
                    .and_then(Value::as_str);
                effects.block = Some(reason(message));
            }
        }
        Event::PermissionDenied => {
            if code == Some(0) && specific.is_some_and(|output| output.retry == Some(true)) {
                effects.context.push(
                    "A PermissionDenied hook suggested retrying the tool. Any new call goes through the current permission policy."
                        .to_owned(),
                );
            }
        }
        Event::UserPromptSubmit | Event::UserPromptExpansion => {
            if exit_blocks || decision == Some("block") || reply.continue_run == Some(false) {
                effects.block = Some(reason(None));
            } else if let Some(specific) = specific {
                effects.context.extend(specific.additional_context);
            } else if event == Event::UserPromptSubmit
                && dialect == Dialect::Claude
                && !text.is_empty()
                && !text.starts_with('{')
            {
                effects.context.push(text.to_owned());
            }
        }
        Event::SessionStart => {
            if code == Some(0) {
                if dialect == Dialect::Codex && reply.continue_run == Some(false) {
                    effects.block = Some(reason(None));
                }
                if let Some(specific) = specific {
                    effects.context.extend(specific.additional_context);
                    if dialect == Dialect::Claude {
                        effects.initial_message = specific.initial_user_message;
                        effects.reload_skills = specific.reload_skills == Some(true);
                        effects.session_title = specific.session_title.filter(|title| {
                            !title.trim().is_empty()
                                && title.len() <= 4096
                                && !title.chars().any(char::is_control)
                        });
                    }
                } else if !has_reply && !text.is_empty() {
                    effects.context.push(text.to_owned());
                }
            }
        }
        Event::SubagentStart => {
            if !exit_blocks && let Some(specific) = specific {
                effects.context.extend(specific.additional_context);
            }
        }
        Event::PostToolUse => {
            let blocked = exit_blocks || decision == Some("block");
            if blocked && dialect == Dialect::Codex {
                effects.block = Some(reason(None));
            } else if blocked {
                effects.context.push(reason(None));
            }
            if let Some(specific) = specific {
                effects.context.extend(specific.additional_context);
                if effects.block.is_none() {
                    effects.output = specific
                        .updated_tool_output
                        .or(specific.updated_mcp_tool_output);
                }
            }
        }
        Event::PostToolUseFailure => {
            if exit_blocks || decision == Some("block") {
                effects.context.push(reason(None));
            }
            if let Some(specific) = specific {
                effects.context.extend(specific.additional_context);
            }
        }
        Event::PostToolBatch => {
            if exit_blocks || decision == Some("block") || reply.continue_run == Some(false) {
                effects.block = Some(reason(None));
            } else if let Some(specific) = specific {
                effects.context.extend(specific.additional_context);
            }
        }
        Event::Stop | Event::SubagentStop | Event::TeammateIdle => {
            if reply.continue_run != Some(false) && (exit_blocks || decision == Some("block")) {
                effects.block = Some(reason(None));
            } else if dialect == Dialect::Claude
                && reply.continue_run != Some(false)
                && let Some(specific) = specific
            {
                effects.context.extend(specific.additional_context);
            }
        }
        Event::PreCompact => {
            if exit_blocks
                || decision == Some("block")
                || (!claude_compaction && reply.continue_run == Some(false))
            {
                effects.block = Some(reason(None));
            } else if claude_compaction && code == Some(0) && !has_reply && !text.is_empty() {
                let instructions = match text.char_indices().nth(10_000) {
                    Some((end, _)) => format!("{}\n[Hook instructions truncated]", &text[..end]),
                    None => text.to_owned(),
                };
                effects.context.push(instructions);
            }
        }
        Event::PostCompact if dialect == Dialect::Codex && reply.continue_run == Some(false) => {
            effects.block = Some(reason(None));
        }
        Event::TaskCreated | Event::TaskCompleted => {
            if exit_blocks || decision == Some("block") {
                effects.block = Some(reason(None));
            }
        }
        _ => {}
    }
    Ok(HookOutput {
        system_message: if matches!(event, Event::Notification | Event::MessageDisplay | Event::ConfigChange)
            || (dialect == Dialect::Claude
                && matches!(event, Event::PreCompact | Event::PostCompact))
        {
            None
        } else {
            reply.system_message
        },
        successful: code == Some(0) && effects.block.is_none(),
        effects,
    })
}

#[cfg(test)]
#[test]
fn imported_claude_lifecycle_uses_only_the_documented_decisions() {
    let display = decode_text(
        Dialect::Claude,
        Event::MessageDisplay,
        Some(0),
        r#"{"continue":false,"systemMessage":"ignored","hookSpecificOutput":{"hookEventName":"MessageDisplay","displayContent":""}}"#,
        "",
    )
    .unwrap();
    assert_eq!(display.effects.display_content.as_deref(), Some(""));
    assert!(display.effects.block.is_none() && display.system_message.is_none());
    let failed = decode_text(Dialect::Claude, Event::MessageDisplay, Some(2), "", "failed").unwrap();
    assert!(failed.effects.display_content.is_none() && failed.effects.block.is_none());

    let veto = decode_text(Dialect::Claude, Event::ConfigChange, Some(2), "", "veto").unwrap();
    assert_eq!(veto.effects.block.as_deref(), Some("veto"));
    let ignored = decode_text(
        Dialect::Claude,
        Event::ConfigChange,
        Some(0),
        r#"{"continue":false,"systemMessage":"ignored"}"#,
        "",
    )
    .unwrap();
    assert!(ignored.effects.block.is_none() && ignored.system_message.is_none());

    let added = decode_text(
        Dialect::Claude,
        Event::DirectoryAdded,
        Some(0),
        r#"{"continue":false,"decision":"block","systemMessage":"diagnostic","watchPaths":["/fixture/config"],"hookSpecificOutput":{"hookEventName":"DirectoryAdded","additionalContext":"ignored"}}"#,
        "",
    )
    .unwrap();
    assert!(added.successful);
    assert_eq!(added.system_message.as_deref(), Some("diagnostic"));
    assert!(added.effects.block.is_none() && added.effects.context.is_empty());
    assert!(added.effects.watch_paths.is_none());
    assert!(matches!(
        decode_text(Dialect::Claude, Event::DirectoryAdded, Some(2), "", "failed setup"),
        Err(diagnostic) if diagnostic.contains("failed setup")
    ));
}

fn declined_elicitation(successful: bool) -> HookOutput {
    HookOutput {
        successful,
        effects: Effects {
            elicitation: Some(McpElicitationReply::new(McpElicitationAction::Decline)),
            ..Default::default()
        },
        ..Default::default()
    }
}

impl AgentPluginAdapter {
    async fn fork_markdown(
        &self,
        session: i64,
        call: Option<i64>,
        prompt: &str,
        agent: Option<&str>,
        scope: &HookScope,
    ) -> PluginResult<CommandResult> {
        let task = self.bridge.prepare_task(call, prompt, agent)?;
        let _fork_scope = self
            .hooks
            .as_ref()
            .and_then(|hooks| hooks.prepare_fork(session, &task.name, scope));
        let host = self.host()?;
        let child = task.spawn(host).await?;
        let result = host
            .wait_imported_plugin_agent(child, bridge::AGENT_TIMEOUT_MS)
            .await?;
        let output = json!({"agent_id": child, "result": result.output_text});
        Ok(delegated_command_result(&result.output_text, output))
    }

    async fn prepare_markdown_prompt(
        &self,
        command: &super::super::markdown::ImportedCommand,
        input: &CommandInvokeInput,
    ) -> PluginResult<String> {
        let id = format!("{}.{}", self.manifest.namespace, self.manifest.name);
        let mut prompt = command.prepare_prompt(&self.root, &self.package_name, &id, &self.claude_prompt_options, input)?;
        let output = match prompt.take_shell() {
            Some(request) => self.host()?.run_imported_skill_shell(request).await?,
            None => Default::default(),
        };
        prompt.finish(output)
    }

    async fn invoke_markdown_command(
        &self,
        command: &super::super::markdown::ImportedCommand,
        input: &CommandInvokeInput,
    ) -> PluginResult<CommandResult> {
        let prompt = match self.prepare_markdown_prompt(command, input).await {
            Ok(prompt) => prompt,
            Err(error) if error.kind == agena_plugin_sdk::PluginErrorKind::Cancelled => {
                return Err(error);
            }
            Err(error) => {
                return Ok(
                    CommandResult::failed("Command prompt could not be prepared")
                        .with_detail(error.failure.user.fallback),
                );
            }
        };
        if let CommandExecution::Fork { agent } = &command.execution {
            return self
                .fork_markdown(
                    command_session(input)?,
                    input.call_id,
                    &prompt,
                    agent.as_deref(),
                    &command.scope,
                )
                .await;
        }
        if let (Some(hooks), Some(session)) = (&self.hooks, input.session_id) {
            hooks.activate(session, command.scope.clone());
            if matches!(command.scope, HookScope::Skill(_)) {
                hooks
                    .start_monitors(self.host()?, session, Some(&command.name))
                    .await;
            }
        }
        Ok(CommandResult::succeeded("Command prompt prepared")
            .with_effect(CommandHostEffect::InsertPrompt { prompt }))
    }

    async fn hook<'a>(
        &self,
        event: Event,
        session: i64,
        fields: impl Into<HookInput<'a>>,
        aliases: &[&str],
    ) -> PluginResult<Effects> {
        let Some(hooks) = &self.hooks else {
            return Ok(Effects::default());
        };
        Ok(hooks
            .run(self.host()?, event, session, fields, aliases, None)
            .await)
    }

    async fn run_agent_stop(
        &self,
        input: AgentStopInput,
        dialect: Option<Dialect>,
    ) -> PluginResult<Option<AgentStopPatch>> {
        let Some(hooks) = &self.hooks else {
            return Ok(None);
        };
        let host = self.host()?;
        let dialect = if let Some(failure) = input.provider_failure {
            hooks
                .run(
                    host,
                    Event::StopFailure,
                    input.session_id,
                    json!({
                        "turn_id": input.execution_id,
                        "model": input.model,
                        "error": provider_error_type(failure),
                        "error_details": input.run_error,
                        "last_assistant_message": input.run_error,
                    }),
                    &[],
                    Some(Dialect::Claude),
                )
                .await;
            if dialect == Some(Dialect::Claude) {
                return Ok(None);
            }
            Some(Dialect::Codex)
        } else {
            dialect
        };
        if let Some(child) = &input.subagent {
            for target in [Dialect::Claude, Dialect::Codex] {
                if dialect.is_some_and(|dialect| dialect != target) {
                    continue;
                }
                let session = if target == Dialect::Claude {
                    child.parent_session_id
                } else {
                    input.session_id
                };
                let effects = hooks
                    .run(
                        host,
                        Event::SubagentStop,
                        input.session_id,
                        json!({
                            "session_id": session.to_string(),
                            "turn_id": input.execution_id.as_deref().unwrap_or_default(),
                            "model": input.model.as_deref().unwrap_or_default(),
                            "agent_id": input.session_id.to_string(),
                            "agent_type": subagent_type(target, &child.agent_type),
                            "agent_transcript_path": if target == Dialect::Claude { json!("") } else { Value::Null },
                            "stop_hook_active": input.stop_hook_active,
                            "last_assistant_message": input.last_assistant_message,
                        }),
                        &[],
                        Some(target),
                    )
                    .await;
                if let Some(patch) = stop_patch(effects) {
                    return Ok(Some(patch));
                }
            }
            if dialect != Some(Dialect::Codex) && input.run_error.is_none() {
                let teammate_name = child
                    .name
                    .as_deref()
                    .filter(|name| !name.trim().is_empty())
                    .unwrap_or_else(|| subagent_type(Dialect::Claude, &child.agent_type));
                let effects = hooks
                    .run(
                        host,
                        Event::TeammateIdle,
                        input.session_id,
                        json!({
                            "session_id": child.parent_session_id.to_string(),
                            "turn_id": input.execution_id.as_deref().unwrap_or_default(),
                            "model": input.model.as_deref().unwrap_or_default(),
                            "agent_id": input.session_id.to_string(),
                            "agent_type": subagent_type(Dialect::Claude, &child.agent_type),
                            "teammate_name": teammate_name,
                            "team_name": self.package_name,
                            "agent_transcript_path": "",
                            "stop_hook_active": input.stop_hook_active,
                            "last_assistant_message": input.last_assistant_message,
                        }),
                        &[],
                        Some(Dialect::Claude),
                    )
                    .await;
                if let Some(patch) = stop_patch(effects) {
                    return Ok(Some(patch));
                }
            }
        }
        let mut fields = json!({
            "turn_id": input.execution_id.unwrap_or_default(),
            "model": input.model.unwrap_or_default(),
            "stop_hook_active": input.stop_hook_active,
            "last_assistant_message": input.last_assistant_message,
        });
        if let Some(child) = input.subagent {
            fields["agent_id"] = json!(input.session_id.to_string());
            fields["agent_type"] = json!(subagent_type(Dialect::Claude, &child.agent_type));
        }
        let effects = hooks
            .run(host, Event::Stop, input.session_id, fields, &[], dialect)
            .await;
        Ok(stop_patch(effects))
    }

    async fn project_tool<'a>(&self, key: &'a ToolKey, args: &Value) -> ToolProjection<'a> {
        let mut projection = ToolProjection::new(key, args);
        projection.resolve_mcp(self.host().ok()).await;
        projection
    }
}

#[async_trait]
impl Plugin for AgentPluginAdapter {
    fn manifest(&self) -> PluginManifest {
        self.manifest.clone()
    }

    async fn chat_system_transform(
        &self,
        input: agena_plugin_sdk::ChatSystemTransformInput,
    ) -> PluginResult<Option<agena_plugin_sdk::ChatSystemTransformPatch>> {
        let mut sections = Vec::new();
        let mut complete_system = None;
        let host = self.host()?;
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(3);
        let mut remaining = 32 * 1024usize;
        if self.start_deepseek_observers && input.session_id > 0 {
            let assembled = tokio::time::timeout_at(
                deadline,
                super::super::deepseek_commands::assemble_system_prompt(
                    &self.deepseek_programs,
                    input.session_id,
                    host,
                    remaining,
                ),
            )
            .await
            .map_err(|_| {
                PluginError::from_kind(
                    agena_plugin_sdk::PluginErrorKind::Timeout,
                    "Cordis system prompt did not assemble in time",
                )
            })??;
            if assembled.complete {
                complete_system = Some(assembled.text);
                remaining = 0;
            } else if !assembled.text.is_empty() {
                remaining = remaining.saturating_sub(assembled.text.len() + 1);
                sections.push(assembled.text);
            }
        }
        for memory in &self.memvault {
            if remaining == 0 {
                break;
            }
            match tokio::time::timeout_at(deadline, memory.prompt_section(host)).await {
                Ok(Some(mut text)) => {
                    text.truncate(text.floor_char_boundary(text.len().min(remaining)));
                    remaining = remaining.saturating_sub(text.len() + 1);
                    sections.push(text);
                }
                Ok(None) => {}
                Err(_) => break,
            }
        }
        if let Some(system) = complete_system {
            return Ok(Some(agena_plugin_sdk::ChatSystemTransformPatch {
                system: Some(system),
                ..Default::default()
            }));
        }
        Ok(
            joined_context(sections).map(|append| agena_plugin_sdk::ChatSystemTransformPatch {
                append: Some(append),
                ..Default::default()
            }),
        )
    }

    async fn imported_turn_completed(
        &self,
        input: agena_plugin_sdk::ImportedTurnCompletedInput,
    ) -> PluginResult<()> {
        let host = self.host()?;
        for memory in &self.memvault {
            memory.completed_turn(host, &input).await;
        }
        Ok(())
    }

    async fn init(
        &self,
        _ctx: InitContext,
        host: Arc<dyn HostClient>,
    ) -> PluginResult<InitOutcome> {
        self.host
            .set(host.clone())
            .map_err(|_| PluginError::internal("Agent Plugin is already initialized"))?;
        if let Some(mods) = self.hooks.as_ref().and_then(|hooks| hooks.mods()) {
            mods.bind_host(host);
        }
        Ok(InitOutcome::ack(self.manifest()))
    }

    async fn shutdown(&self) -> PluginResult<()> {
        if let Some(hooks) = &self.hooks {
            hooks.stop_background();
        }
        Ok(())
    }

    async fn tool_invoke(&self, input: ToolInvokeInput) -> PluginResult<ToolInvokeOutput> {
        if let Some(program) = self.deepseek_tool_programs.get(&input.tool_name) {
            let host = self.host.get().cloned().ok_or_else(|| PluginError::internal("Agent Plugin host is unavailable"))?;
            return program.invoke_tool(&self.bridge, host, input).await;
        }
        match input.tool_name.as_str() {
            bridge::TOOL_NAME => self.bridge.run(self.host()?, input).await,
            workflow::TOOL_NAME => {
                let host = self
                    .host
                    .get()
                    .cloned()
                    .ok_or_else(|| PluginError::internal("Agent Plugin host is unavailable"))?;
                workflow::invoke(&self.workflows, &self.bridge, host, input).await
            }
            name if name.starts_with("mcp__")
                && let Some(mods) = self.hooks.as_ref().and_then(|hooks| hooks.mods()) =>
            {
                mods.invoke_registered_tool(self.host()?, input).await
            }
            _ => Err(PluginError::not_implemented("imported plugin task")),
        }
    }

    async fn command_invoke(&self, input: CommandInvokeInput) -> PluginResult<CommandResult> {
        if input.command_id == FORKED_SKILL_COMMAND {
            let session = command_session(&input)?;
            let call = input.call_id.filter(|call| *call >= 0).ok_or_else(|| {
                PluginError::invalid_params("a model-invoked Claude fork requires a live tool call")
            })?;
            let invocation: ForkedSkillInvocation = serde_json::from_value(input.input)
                .map_err(|error| PluginError::invalid_params_error(&error))?;
            return self
                .fork_markdown(
                    session,
                    Some(call),
                    &invocation.prompt,
                    invocation.scope.agent.as_deref(),
                    &HookScope::Skill(invocation.scope.directory),
                )
                .await;
        }
        let Some(command) = self.commands.get(&input.command_id) else {
            if let Some(mods) = self.hooks.as_ref().and_then(|hooks| hooks.mods()) {
                return Ok(mods.command(self.host()?, &input).await);
            }
            return Err(PluginError::not_implemented("imported command"));
        };
        match &command.execution {
            CommandExecution::Prompt | CommandExecution::Fork { .. } => {
                self.invoke_markdown_command(command, &input).await
            }
            CommandExecution::DeepSeek(command) => {
                if command_session(&input).is_err() {
                    return Ok(CommandResult::unavailable(
                        "This command requires a project session",
                    ));
                }
                command
                    .invoke(input, self.host.get().map(Arc::as_ref))
                    .await
            }
            CommandExecution::Workflow { name } => {
                let session = command_session(&input)?;
                let workflow = self
                    .workflows
                    .get(name)
                    .ok_or_else(|| PluginError::invalid_params("unknown imported workflow"))?;
                let raw = input
                    .input
                    .get("args")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let (resume, args) = workflow::command_args(raw)?;
                let host = self
                    .host
                    .get()
                    .cloned()
                    .ok_or_else(|| PluginError::internal("Agent Plugin host is unavailable"))?;
                let result = workflow::execute(
                    workflow,
                    &self.bridge,
                    host,
                    session,
                    input.call_id,
                    args,
                    resume,
                )
                .await?;
                let output = serde_json::from_str(&result.output_text)
                    .map_err(|error| PluginError::internal_error(&error))?;
                let mut response = delegated_command_result(&result.output_text, output);
                if let Some(run_id) = result.metadata.get("runId")
                    && let Some(detail) = &mut response.detail
                {
                    detail.push_str(&format!("\nWorkflow run ID: {run_id}"));
                }
                Ok(response)
            }
        }
    }

    async fn tool_definition(
        &self,
        input: agena_plugin_sdk::ToolDefinitionInput,
    ) -> PluginResult<Option<agena_plugin_sdk::ToolDefinitionPatch>> {
        Box::pin(async move {
            let context =
                agena_plugin_sdk::host_api::current_host_callback_context().unwrap_or_default();
            let Some(session_id) = context.session_id.filter(|id| *id > 0) else {
                return Ok(None);
            };
            if input.tool.plugin().namespace() == self.manifest.namespace
                && input.tool.plugin().name() == self.manifest.name
                && let Some(program) = self.deepseek_tool_programs.get(input.tool.name())
                && program.tool_has_when(input.tool.name())
            {
                let host = self.host()?;
                let active = program
                    .is_tool_active(session_id, host, input.tool.name())
                    .await;
                let next_hidden = !active;
                if next_hidden != input.hidden {
                    return Ok(Some(agena_plugin_sdk::ToolDefinitionPatch {
                        hidden: Some(next_hidden),
                        ..Default::default()
                    }));
                }
                return Ok(None);
            }
            let Some(mods) = self.hooks.as_ref().and_then(|hooks| hooks.mods()) else {
                return Ok(None);
            };
            if !mods.matches("tool.offer") && !mods.matches("tool.describe") {
                return Ok(None);
            }
            let host = self.host()?;
            let mut projection = ToolProjection::new(&input.tool, &Value::Null);
            projection.resolve_mcp(Some(host)).await;
            let tool_name = projection.mod_tool_name().to_owned();
            let wants_offer = mods.matches_tool("tool.offer", &tool_name);
            let wants_describe = mods.matches_tool("tool.describe", &tool_name);
            if !wants_offer && !wants_describe {
                return Ok(None);
            }
            if let Some(workspace) = context
                .workspace_root
                .as_deref()
                .filter(|root| !root.is_empty())
            {
                Box::pin(mods.start(
                    host,
                    &SessionStartInput {
                        session_id,
                        workspace_root: workspace.to_owned(),
                        source: SessionStartSource::Resume,
                        model: None,
                        session_title: None,
                        agent_type: None,
                        resumed_cache: None,
                    },
                ))
                .await;
            }
            let mut patch = agena_plugin_sdk::ToolDefinitionPatch::default();
            let mut changed = false;
            let mut hidden = input.hidden;
            if wants_offer {
                let offer_event = json!({
                    "tool": tool_name,
                    "canonicalTool": input.tool.to_string(),
                    "description": input.summary,
                    "inputSchema": input.input_schema,
                    "offered": !hidden,
                });
                let answer = mods
                    .run(host, session_id, "tool.offer", offer_event, |_| async move {
                        json!({"offer": !hidden})
                    })
                    .await;
                if let Some(offer) = answer["offer"].as_bool() {
                    let next_hidden = !offer;
                    if next_hidden != hidden {
                        hidden = next_hidden;
                        patch.hidden = Some(next_hidden);
                        changed = true;
                    }
                }
            }
            if wants_describe && !hidden {
                let describe_event = json!({
                    "tool": tool_name,
                    "canonicalTool": input.tool.to_string(),
                    "description": input.summary,
                    "summary": input.summary,
                    "help": input.help,
                    "inputSchema": input.input_schema,
                });
                let default_desc = input.summary.clone();
                let default_help = input.help.clone();
                let answer = mods
                    .run(
                        host,
                        session_id,
                        "tool.describe",
                        describe_event,
                        move |_| async move {
                            let mut reply = serde_json::Map::new();
                            reply.insert("description".into(), json!(default_desc));
                            if let Some(help) = default_help {
                                reply.insert("help".into(), json!(help));
                            }
                            Value::Object(reply)
                        },
                    )
                    .await;
                let bound = |value: &str, max: usize| {
                    value[..value.floor_char_boundary(value.len().min(max))].to_owned()
                };
                if let Some(desc) = answer
                    .get("summary")
                    .or_else(|| answer.get("description"))
                    .and_then(Value::as_str)
                {
                    let trimmed = bound(desc.trim(), 4096);
                    if !trimmed.is_empty() && trimmed != input.summary {
                        patch.summary = Some(trimmed);
                        changed = true;
                    }
                }
                let explicit_help = answer.get("help").and_then(Value::as_str);
                let search_hint = answer.get("searchHint").and_then(Value::as_str);
                let when_to_use = answer.get("whenToUse").and_then(Value::as_str);
                if explicit_help.is_some() || search_hint.is_some() || when_to_use.is_some() {
                    let help_text = [
                        explicit_help.or_else(|| patch.summary.as_deref()),
                        search_hint,
                        when_to_use,
                    ]
                    .into_iter()
                    .flatten()
                    .map(|part| bound(part.trim(), 16 * 1024))
                    .filter(|part| !part.is_empty())
                    .collect::<Vec<_>>()
                    .join("\n\n");
                    let bounded_help = bound(&help_text, 16 * 1024);
                    if !bounded_help.is_empty()
                        && input.help.as_deref() != Some(bounded_help.as_str())
                    {
                        patch.help = Some(bounded_help);
                        changed = true;
                    }
                }
            }
            Ok(changed.then_some(patch))
        })
        .await
    }

    async fn tool_execute_before(
        &self,
        input: ToolBeforeInput,
    ) -> PluginResult<Option<ToolBeforePatch>> {
        let tool = self.project_tool(&input.tool, &input.input).await;
        let codex_shell_hook = tool.codex_shell_hook(&input.input, input.tool_cwd.as_deref());
        let effects = self
            .hook(
                Event::PreToolUse,
                input.session_id,
                tool.hook_input(tool.fields(input.call_id, input.input.clone()))
                    .with_codex_tool_cwd(input.tool_cwd.as_deref())
                    .with_codex_shell_hook(codex_shell_hook),
                &tool.aliases(),
            )
            .await?;
        let updated = effects.input.map(|value| tool.to_agena_input(value));
        if updated.is_none()
            && effects.block.is_none()
            && effects.permission_ask.is_none()
            && effects.context.is_empty()
        {
            return Ok(None);
        }
        Ok(Some(ToolBeforePatch {
            input: updated,
            abort_reason: effects.block,
            permission_ask: effects.permission_ask,
            additional_context: effects.context,
            ..Default::default()
        }))
    }

    async fn tool_execute_after(
        &self,
        input: ToolAfterInput,
    ) -> PluginResult<Option<ToolAfterPatch>> {
        if let Some(hooks) = &self.hooks
            && input.tool.plugin().namespace() == "agena"
            && input.tool.plugin().name() == "skills"
            && input.tool.name() == "get"
            && !input.metadata.contains_key("skip_skill_activation")
            && let Some(skill) = &input.payload
            && skill["enabled"] == true
            && skill["valid"] == true
            && let Some(path) = skill["path"].as_str()
            && hooks.activate_skill(input.session_id, std::path::Path::new(path))
            && let Some(name) = skill["name"].as_str()
        {
            hooks
                .start_monitors(self.host()?, input.session_id, Some(name))
                .await;
        }
        let arguments = input.input.unwrap_or_else(|| json!({}));
        let tool = self.project_tool(&input.tool, &arguments).await;
        let codex_shell_hook = match input
            .shell_completion
            .as_ref()
            .filter(|result| result.invocation.call.shell == ProcessShell::Bash)
        {
            Some(result) => CodexShellHook::Completed(result),
            None => match tool.codex_shell_hook(&arguments, input.tool_cwd.as_deref()) {
                CodexShellHook::Default => CodexShellHook::Default,
                _ => CodexShellHook::Skip,
            },
        };
        let mut fields = tool.fields(input.call_id, arguments);
        let shell_exit = matches!(tool.name.as_ref(), "Bash" | "PowerShell")
            .then(|| input.metadata.get("exit_code")?.parse::<i32>().ok())
            .flatten()
            .filter(|code| *code != 0);
        fields["tool_response"] = input
            .model_output
            .map(Value::String)
            .or(input.payload)
            .unwrap_or_else(|| json!(&input.output_text));
        let effects = if let Some(code) = shell_exit {
            let Some(hooks) = &self.hooks else {
                return Ok(None);
            };
            let host = self.host()?;
            let aliases = tool.aliases();
            let mut codex = hooks
                .run(
                    host,
                    Event::PostToolUse,
                    input.session_id,
                    tool.hook_input(fields.clone())
                        .with_codex_tool_cwd(input.tool_cwd.as_deref())
                        .with_codex_shell_hook(codex_shell_hook),
                    &aliases,
                    Some(Dialect::Codex),
                )
                .await;
            fields
                .as_object_mut()
                .expect("tool hook fields")
                .remove("tool_response");
            fields["error"] = json!(format!("Exit code {code}\n{}", input.output_text));
            fields["is_interrupt"] = json!(false);
            let claude = hooks
                .run(
                    host,
                    Event::PostToolUseFailure,
                    input.session_id,
                    fields,
                    &aliases,
                    Some(Dialect::Claude),
                )
                .await;
            codex.context.extend(claude.context);
            codex
        } else {
            self.hook(
                Event::PostToolUse,
                input.session_id,
                tool.hook_input(fields)
                    .with_codex_tool_cwd(input.tool_cwd.as_deref())
                    .with_codex_shell_hook(codex_shell_hook),
                &tool.aliases(),
            )
            .await?
        };
        if effects.context.is_empty() && effects.output.is_none() && effects.block.is_none() {
            return Ok(None);
        }
        if let Some(reason) = effects.block {
            return Ok(Some(ToolAfterPatch {
                model_output: Some(format!(
                    "{reason} The tool already ran. Its effects may be committed."
                )),
                metadata: [(
                    agena_plugin_sdk::TOOL_MODEL_OUTPUT_WITHHELD.into(),
                    "true".into(),
                )]
                .into(),
                ..Default::default()
            }));
        }
        let model_output = effects.output.map(|output| match output {
            Value::String(text) => text,
            value => value.to_string(),
        });
        Ok(Some(ToolAfterPatch {
            model_output,
            additional_context: effects.context,
            ..Default::default()
        }))
    }

    async fn tool_execute_failure(&self, input: ToolFailureInput) -> PluginResult<()> {
        let event = match (&input.automatic_denial, input.duration_ms) {
            (Some(_), _) => Event::PermissionDenied,
            (None, Some(_))
                if input.failure.category != agena_failure::FailureCategory::PermissionDenied =>
            {
                Event::PostToolUseFailure
            }
            _ => return Ok(()),
        };
        let tool = self.project_tool(&input.tool, &input.input).await;
        let mut fields = tool.fields(input.call_id, input.input);
        if let Some(reason) = input.automatic_denial {
            fields["reason"] = json!(reason);
            fields["permission_mode"] = json!("auto");
        } else {
            fields["error"] = json!(input.failure.user.fallback);
            fields["is_interrupt"] = json!(false);
            fields["duration_ms"] = json!(input.duration_ms);
        }
        let effects = self
            .hook(
                event,
                input.session_id,
                tool.hook_input(fields),
                &tool.aliases(),
            )
            .await?;
        if let Some(context) = joined_context(effects.context) {
            let binding = self.host()?.bind_imported_plugin_background().await?;
            binding.notices.deliver(context, false).await?;
        }
        Ok(())
    }

    async fn tool_batch_after(
        &self,
        input: ToolBatchInput,
    ) -> PluginResult<Option<ToolBatchPatch>> {
        let mut calls = Vec::with_capacity(input.calls.len());
        for call in input.calls {
            let mut fields = if let Some(key) = &call.tool {
                let tool = self.project_tool(key, &call.input).await;
                let mut fields = tool.fields(call.call_id, call.input);
                if let Some(claude) = tool.claude_mcp {
                    claude.apply(&mut fields);
                }
                fields
            } else {
                json!({
                    "tool_name": call.name,
                    "tool_input": call.input,
                    "tool_use_id": call.call_id.to_string(),
                })
            };
            fields["tool_response"] = Value::String(call.model_output);
            calls.push(fields);
        }
        let effects = self
            .hook(
                Event::PostToolBatch,
                input.session_id,
                json!({"tool_calls": calls}),
                &[],
            )
            .await?;
        if effects.block.is_none() && effects.context.is_empty() {
            return Ok(None);
        }
        Ok(Some(ToolBatchPatch {
            additional_context: joined_context(effects.context),
            block_reason: effects.block,
        }))
    }

    async fn permission_request(
        &self,
        input: PermissionRequestInput,
    ) -> PluginResult<Option<PermissionRequestPatch>> {
        let key = input
            .tool_name
            .parse::<ToolKey>()
            .or_else(|_| format!("agena.{}", input.tool_name).parse())
            .ok();
        let arguments = input
            .tool_input
            .unwrap_or_else(|| json!({"action": input.action, "description": input.description}));
        let tool = match key.as_ref() {
            Some(key) => Some(self.project_tool(key, &arguments).await),
            None => None,
        };
        let name = tool
            .as_ref()
            .map(|tool| tool.name.as_ref())
            .unwrap_or(&input.tool_name);
        let aliases = tool
            .as_ref()
            .map(ToolProjection::aliases)
            .unwrap_or_default();
        let codex_shell_hook = tool.as_ref().map_or(CodexShellHook::Default, |tool| {
            tool.codex_shell_hook(&arguments, input.tool_cwd.as_deref())
        });
        let arguments = match &tool {
            Some(tool) => tool.foreign_input(arguments),
            None => arguments,
        };
        let fields = json!({"tool_name": name,"tool_input": arguments});
        let input_fields = HookInput {
            fields,
            claude_mcp: tool.as_ref().and_then(|tool| tool.claude_mcp.as_ref()),
            codex_tool_cwd: input.tool_cwd.as_deref(),
            codex_shell_hook,
            skip_scope: None,
            canonical_model: None,
            process_cwd: None,
        };
        let effects = self
            .hook(
                Event::PermissionRequest,
                input.session_id,
                input_fields,
                &aliases,
            )
            .await?;
        Ok(effects.block.map(|deny_reason| PermissionRequestPatch {
            deny_reason: Some(deny_reason),
        }))
    }

    async fn notification(&self, input: agena_plugin_sdk::NotificationInput) -> PluginResult<()> {
        if let Some(session) = input.session_id {
            self.hook(
                Event::Notification,
                session,
                json!({
                    "notification_type": input.kind,
                    "title": input.title,
                    "message": input.message,
                }),
                &[],
            )
            .await?;
        }
        Ok(())
    }

    async fn user_prompt_submit(
        &self,
        input: UserPromptSubmitInput,
    ) -> PluginResult<Option<UserPromptSubmitPatch>> {
        if let Some(hooks) = &self.hooks {
            hooks
                .start_monitors(self.host()?, input.session_id, None)
                .await;
            hooks
                .watch_files(self.host()?, input.session_id, None)
                .await;
        }
        let effects = self
            .hook(
                Event::UserPromptSubmit,
                input.session_id,
                json!({"prompt": input.prompt}),
                &[],
            )
            .await?;
        if effects.block.is_none() && effects.context.is_empty() {
            return Ok(None);
        }
        Ok(Some(UserPromptSubmitPatch {
            block_reason: effects.block,
            additional_context: joined_context(effects.context),
            ..Default::default()
        }))
    }

    async fn user_prompt_expansion(
        &self,
        input: UserPromptExpansionInput,
    ) -> PluginResult<Option<UserPromptExpansionPatch>> {
        let effects = self
            .hook(
                Event::UserPromptExpansion,
                input.session_id,
                json!({
                    "expansion_type": "slash_command",
                    "command_name": input.command_name,
                    "command_args": input.command_args,
                    "command_source": "plugin",
                    "prompt": input.prompt,
                }),
                &[],
            )
            .await?;
        if effects.block.is_none() && effects.context.is_empty() {
            return Ok(None);
        }
        Ok(Some(UserPromptExpansionPatch {
            block_reason: effects.block,
            additional_context: joined_context(effects.context),
        }))
    }

    async fn session_start(
        &self,
        input: SessionStartInput,
    ) -> PluginResult<Option<SessionStartPatch>> {
        if self.start_deepseek_observers && input.source != SessionStartSource::Compact {
            for program in &self.deepseek_programs {
                let _ = program.start_session(input.session_id, self.host()?).await;
            }
        }
        if input.source != SessionStartSource::Compact
            && let Some(mods) = self.hooks.as_ref().and_then(|hooks| hooks.mods())
        {
            mods.start(self.host()?, &input).await;
        }
        if input.source != SessionStartSource::Compact
            && let Some(hooks) = &self.hooks
        {
            hooks
                .start_monitors(self.host()?, input.session_id, None)
                .await;
        }
        let mut fields = json!({"source": input.source.as_str(), "cwd":input.workspace_root});
        if let Some(model) = input.model {
            fields["model"] = json!(model);
        }
        if let Some(title) = input.session_title {
            fields["session_title"] = json!(title);
        }
        if let Some(agent) = input.agent_type {
            fields["agent_type"] = json!(agent);
        }
        if let Some(cache) = input.resumed_cache {
            fields["seconds_since_last_response"] = json!(cache.seconds_since_last_response);
            fields["prompt_cache_likely_expired"] = json!(cache.prompt_cache_likely_expired);
            if let Some(tokens) = cache.context_tokens {
                fields["context_tokens"] = json!(tokens);
            }
            if let Some(cost) = cache.estimated_cache_write_usd {
                fields["estimated_cache_write_usd"] = json!(cost);
            }
        }
        let effects = self
            .hook(Event::SessionStart, input.session_id, fields, &[])
            .await?;
        if let Some(hooks) = &self.hooks {
            hooks
                .watch_files(self.host()?, input.session_id, effects.watch_paths)
                .await;
        }
        let initial_user_message = match input.source {
            SessionStartSource::Startup | SessionStartSource::Clear => effects.initial_message,
            _ => None,
        };
        let session_title = match input.source {
            SessionStartSource::Startup | SessionStartSource::Resume | SessionStartSource::Fork => {
                effects.session_title
            }
            _ => None,
        };
        if effects.context.is_empty()
            && initial_user_message.is_none()
            && session_title.is_none()
            && effects.block.is_none()
            && !effects.reload_skills
        {
            return Ok(None);
        }
        Ok(Some(SessionStartPatch {
            additional_context: joined_context(effects.context),
            initial_user_message,
            session_title,
            stop_reason: effects.block,
            reload_skills: effects.reload_skills,
        }))
    }

    async fn session_file_changed(
        &self,
        input: SessionFileChangedInput,
    ) -> PluginResult<Option<SessionFileChangedPatch>> {
        let effects = self
            .hook(
                Event::FileChanged,
                input.session_id,
                json!({"file_path": input.path, "event": input.change}),
                &[],
            )
            .await?;
        Ok(effects
            .watch_paths
            .map(|watch_paths| SessionFileChangedPatch { watch_paths }))
    }

    async fn imported_session_setup(
        &self,
        input: agena_plugin_sdk::ImportedSessionSetupInput,
    ) -> PluginResult<()> {
        self.hook(
            Event::Setup,
            input.session_id,
            json!({"trigger":input.trigger}),
            &[],
        )
        .await?;
        Ok(())
    }

    async fn imported_instructions_loaded(
        &self,
        input: agena_plugin_sdk::ImportedInstructionsLoadedInput,
    ) -> PluginResult<()> {
        if let Some(hooks) = &self.hooks {
            hooks.instructions_loaded(self.host()?, input).await;
        }
        Ok(())
    }

    async fn imported_message_display(
        &self,
        input: agena_plugin_sdk::ImportedMessageDisplayInput,
    ) -> PluginResult<Option<String>> {
        let session = input.session_id;
        let mut fields = serde_json::to_value(input)?;
        fields["session_id"] = json!(session.to_string());
        let effects = self.hook(Event::MessageDisplay, session, fields, &[]).await?;
        Ok(effects.display_content)
    }

    async fn imported_config_change(
        &self,
        input: agena_plugin_sdk::ImportedConfigChangeInput,
    ) -> PluginResult<Option<String>> {
        let fields = json!({"source": input.source, "file_path": input.file_path});
        let effects = self.hook(Event::ConfigChange, input.session_id, fields, &[]).await?;
        if input.source == agena_plugin_sdk::ImportedConfigChangeSource::PolicySettings {
            return Ok(None);
        }
        Ok(effects.block)
    }

    async fn imported_directory_added(
        &self,
        input: agena_plugin_sdk::ImportedDirectoryAddedInput,
    ) -> PluginResult<()> {
        if let Some(hooks) = &self.hooks {
            let mut fields = HookInput::from(json!({
                "source": "register_repo_root",
                "directory": input.directory,
                "cwd": input.cwd,
            }));
            fields.process_cwd = Some(&input.cwd);
            hooks.run(
                self.host()?,
                Event::DirectoryAdded,
                input.session_id,
                fields,
                &[],
                Some(Dialect::Claude),
            ).await;
        }
        Ok(())
    }

    async fn imported_mcp_elicitation(
        &self,
        input: agena_plugin_sdk::ImportedMcpElicitationInput,
    ) -> PluginResult<Option<McpElicitationReply>> {
        let Some(hooks) = &self.hooks else {
            return Ok(None);
        };
        let (event, mut fields) = if let Some(reply) = input.reply {
            let mut fields = json!({});
            fields["action"] = json!(reply.action);
            if let Some(content) = reply.content {
                fields["content"] = content;
            }
            match input.request {
                McpElicitation::Form { .. } => fields["mode"] = json!("form"),
                McpElicitation::Url { elicitation_id, .. } => {
                    fields["mode"] = json!("url");
                    fields["elicitation_id"] = json!(elicitation_id);
                }
            }
            (Event::ElicitationResult, fields)
        } else {
            (Event::Elicitation, serde_json::to_value(input.request)?)
        };
        fields["mcp_server_name"] = json!(input.server);
        let effects = hooks
            .run(
                self.host()?,
                event,
                input.session_id,
                fields,
                &[],
                Some(Dialect::Claude),
            )
            .await;
        if effects.block.is_some() {
            return Ok(Some(McpElicitationReply::new(
                McpElicitationAction::Decline,
            )));
        }
        Ok(effects.elicitation)
    }

    async fn imported_skill_shell_before(
        &self,
        input: agena_plugin_sdk::ImportedSkillShellHookInput,
    ) -> PluginResult<Option<agena_plugin_sdk::ImportedSkillShellDecision>> {
        use agena_plugin_sdk::{ImportedSkillShellDecision, ProcessShell};
        let Some(hooks) = &self.hooks else {
            return Ok(None);
        };
        let tool = match input.shell {
            ProcessShell::Bash => "Bash",
            ProcessShell::Powershell => "PowerShell",
        };
        let skip_scope = ((input.forked || input.subagent)
            && input.source_plugin.namespace() == self.manifest.namespace
            && input.source_plugin.name() == self.manifest.name)
            .then(|| {
                input.skill_dir.map(HookScope::Skill).unwrap_or_else(|| {
                    let (_, name) = input
                        .command_name
                        .rsplit_once(':')
                        .unwrap_or(("", &input.command_name));
                    HookScope::Command(name.to_owned())
                })
            });
        let effects = hooks
            .run(
                self.host()?,
                Event::PreSkillShellCommand,
                input.session_id,
                HookInput {
                    fields: json!({
                        "tool_name": tool,
                        "tool_input": {"command": input.command},
                        "tool_use_id": input.tool_use_id,
                        "command_name": input.command_name,
                        "command_source": "plugin",
                        "invoked_by": input.invoked_by,
                    }),
                    claude_mcp: None,
                    codex_tool_cwd: None,
                    codex_shell_hook: CodexShellHook::Default,
                    skip_scope,
                    canonical_model: None,
                    process_cwd: None,
                },
                &[],
                Some(Dialect::Claude),
            )
            .await;
        Ok(effects
            .stop_skill
            .map(ImportedSkillShellDecision::Stop)
            .or_else(|| effects.block.map(ImportedSkillShellDecision::Skip)))
    }

    async fn worktree_hook(
        &self,
        input: agena_plugin_sdk::WorktreeHookInput,
    ) -> PluginResult<Option<agena_plugin_sdk::WorktreeHookOutput>> {
        use agena_plugin_sdk::WorktreeOperation;

        let Some(hooks) = &self.hooks else {
            return Ok(None);
        };
        let (event, fields) = match input.operation {
            WorktreeOperation::Create { name } => (Event::WorktreeCreate, json!({"name": name})),
            WorktreeOperation::Remove { path } => {
                (Event::WorktreeRemove, json!({"worktree_path": path}))
            }
        };
        let mut fields = HookInput::from(fields);
        fields.process_cwd = Some(&input.cwd);
        let effects = hooks
            .run(
                self.host()?,
                event,
                input.session_id,
                fields,
                &[],
                Some(Dialect::Claude),
            )
            .await;
        if let Some(reason) = effects.block {
            return Err(PluginError::invalid_params(reason));
        }
        Ok(effects.worktree)
    }

    async fn imported_model_switch(
        &self,
        input: agena_plugin_sdk::ImportedModelSwitchInput,
        before: bool,
    ) -> PluginResult<agena_plugin_sdk::ImportedModelSwitchPatch> {
        let Some(hooks) = &self.hooks else {
            return Ok(Default::default());
        };
        let effects = hooks
            .run(
                self.host()?,
                if before {
                    Event::PreModelSwitch
                } else {
                    Event::PostModelSwitch
                },
                input.session_id,
                HookInput {
                    fields: json!({
                        "from_model": input.from_model,
                        "to_model": input.to_model,
                        "requested_model": input.requested_model,
                        "source": input.source,
                        "context_tokens": input.context_tokens,
                        "prompt_cache_warm": input.prompt_cache_warm,
                        "cache_ttl": input.cache_ttl,
                        "estimated_cache_write_usd": input.estimated_cache_write_usd,
                        "pricing": input.pricing,
                    }),
                    claude_mcp: None,
                    codex_tool_cwd: None,
                    codex_shell_hook: CodexShellHook::Default,
                    skip_scope: None,
                    canonical_model: input.canonical_model.as_deref(),
                    process_cwd: None,
                },
                &[],
                Some(Dialect::Claude),
            )
            .await;
        Ok(agena_plugin_sdk::ImportedModelSwitchPatch {
            context: effects.context,
            ..effects.model_switch
        })
    }

    async fn session_cwd_changed(&self, input: SessionCwdChangedInput) -> PluginResult<()> {
        let Some(hooks) = &self.hooks else {
            return Ok(());
        };
        let host = self.host()?;
        let effects = hooks
            .run(
                host,
                Event::CwdChanged,
                input.session_id,
                json!({
                    "old_cwd": input.old_cwd,
                    "new_cwd": input.new_cwd,
                }),
                &[],
                Some(Dialect::Claude),
            )
            .await;
        if effects.watch_paths.is_some() {
            hooks
                .watch_files(host, input.session_id, effects.watch_paths)
                .await;
        }
        Ok(())
    }

    async fn imported_task_before(
        &self,
        input: agena_plugin_sdk::ImportedTaskBeforeInput,
    ) -> PluginResult<Option<String>> {
        use agena_plugin_sdk::ImportedTaskChange;

        let Some(hooks) = &self.hooks else {
            return Ok(None);
        };
        let event = match input.change {
            ImportedTaskChange::Create => Event::TaskCreated,
            ImportedTaskChange::Complete => Event::TaskCompleted,
        };
        let mut fields = json!({
            "task_id": input.task_id,
            "task_subject": input.subject,
            "task_description": input.description,
        });
        if let Some(agent_id) = input.agent_id {
            fields["agent_id"] = json!(agent_id);
        }
        let effects = hooks
            .run(
                self.host()?,
                event,
                input.session_id,
                fields,
                &[],
                Some(Dialect::Claude),
            )
            .await;
        Ok(effects.block)
    }

    async fn session_end(&self, input: SessionEndInput) -> PluginResult<()> {
        for program in &self.deepseek_programs {
            program.end_session(input.session_id);
        }
        if input.reason == SessionEndReason::UserExit && !self.memvault.is_empty() {
            super::super::memvault::infer_session_outcome(&self.memvault, self.host()?).await;
        }
        let reason = match input.reason {
            SessionEndReason::Clear => "clear",
            SessionEndReason::Resume => "resume",
            SessionEndReason::Logout => "logout",
            SessionEndReason::UserExit => "prompt_input_exit",
            SessionEndReason::Other => "other",
        };
        let result = self
            .hook(
                Event::SessionEnd,
                input.session_id,
                json!({"reason": reason}),
                &[],
            )
            .await;
        if let Some(hooks) = &self.hooks {
            if let Some(mods) = hooks.mods() {
                mods.end(self.host()?, input.session_id, reason).await;
            }
            hooks.end_session(input.session_id);
        }
        result?;
        Ok(())
    }

    async fn agent_stop(&self, input: AgentStopInput) -> PluginResult<Option<AgentStopPatch>> {
        self.run_agent_stop(input, None).await
    }

    async fn imported_claude_stop(&self, input: AgentStopInput) -> PluginResult<Option<AgentStopPatch>> {
        self.run_agent_stop(input, Some(Dialect::Claude)).await
    }

    async fn agent_cancel(&self, input: AgentCancelInput) -> PluginResult<()> {
        self.hook(
            Event::Interrupt,
            input.session_id,
            json!({"turn_id": input.execution_id}),
            &[],
        )
        .await?;
        Ok(())
    }

    async fn compaction_before(
        &self,
        input: CompactionBeforeInput,
    ) -> PluginResult<Option<CompactionBeforePatch>> {
        let Some(hooks) = &self.hooks else {
            return Ok(None);
        };
        let trigger = compaction_trigger(input.automatic);
        let effects = hooks.run(self.host()?, Event::PreCompact, input.session_id, json!({
            "trigger": trigger, "model": input.model, "custom_instructions": input.custom_instructions,
        }), &[], Some(Dialect::Claude)).await;
        let additional_instructions = joined_context(effects.context);
        let mut deny_reason = effects.block;
        if deny_reason.is_none() {
            deny_reason = hooks
                .run(
                    self.host()?,
                    Event::PreCompact,
                    input.session_id,
                    json!({
                        "trigger": trigger, "model": input.model,
                    }),
                    &[],
                    Some(Dialect::Codex),
                )
                .await
                .block;
        }
        Ok(
            (deny_reason.is_some() || additional_instructions.is_some()).then_some(
                CompactionBeforePatch {
                    deny_reason,
                    additional_instructions,
                },
            ),
        )
    }

    async fn compaction_after_control(
        &self,
        input: CompactionAfterInput,
    ) -> PluginResult<Option<String>> {
        let Some(hooks) = &self.hooks else {
            return Ok(None);
        };
        let Some(summary) = input.compact_summary else {
            return Ok(None);
        };
        let trigger = compaction_trigger(input.automatic);
        let mut stop_reason = None;
        for dialect in [Dialect::Claude, Dialect::Codex] {
            let mut fields = json!({"trigger": trigger, "model": input.model});
            if dialect == Dialect::Claude {
                fields["compact_summary"] = json!(summary);
            }
            let effects = hooks
                .run(
                    self.host()?,
                    Event::PostCompact,
                    input.session_id,
                    fields,
                    &[],
                    Some(dialect),
                )
                .await;
            stop_reason = stop_reason.or(effects.block);
        }
        Ok(stop_reason)
    }

    async fn subagent_start(
        &self,
        input: SubagentStartInput,
    ) -> PluginResult<Option<SubagentStartPatch>> {
        let Some(hooks) = &self.hooks else {
            return Ok(None);
        };
        hooks.activate_fork(input.session_id, input.child_session_id, &input.name);
        let host = self.host()?;
        let agent_type = input.agent_type.as_deref().unwrap_or(&input.name);
        let mut context = Vec::new();
        for (dialect, session) in [
            (Dialect::Claude, input.session_id),
            (Dialect::Codex, input.child_session_id),
        ] {
            let effects = hooks
                .run(
                    host,
                    Event::SubagentStart,
                    input.child_session_id,
                    json!({
                        "session_id": session.to_string(),
                        "agent_id": input.child_session_id.to_string(),
                        "agent_type": subagent_type(dialect, agent_type),
                        "model": input.model.as_deref().unwrap_or_default(),
                    }),
                    &[],
                    Some(dialect),
                )
                .await;
            if let Some(reason) = effects.block {
                return Ok(Some(SubagentStartPatch {
                    deny_reason: Some(reason),
                    additional_context: None,
                }));
            }
            context.extend(effects.context);
        }
        Ok(joined_context(context).map(|context| SubagentStartPatch {
            additional_context: Some(context),
            ..Default::default()
        }))
    }

    async fn imported_model_attempt(
        &self,
        input: ImportedModelAttemptInput,
    ) -> PluginResult<ImportedModelAttemptCharge> {
        let Some(hooks) = &self.hooks else {
            return Ok(ImportedModelAttemptCharge::default());
        };
        let mut fields = json!({
            "session_id": input.root_session_id.to_string(),
            "thread_id": input.session_id.to_string(),
            "turn_id": input.turn_id,
            "request_id": input.request_id,
            "attempt_number": input.attempt_number,
            "kind": input.kind,
            "model": input.model,
        });
        if let Some(agent_type) = input.agent_type {
            fields["agent_type"] = json!(agent_type);
            fields["agent_id"] = json!(input.session_id.to_string());
        }
        let event = if let Some(outcome) = input.outcome {
            fields
                .as_object_mut()
                .expect("sampling hook object")
                .extend(
                    serde_json::to_value(outcome)?
                        .as_object()
                        .expect("sampling result object")
                        .clone(),
                );
            Event::PostModelRequest
        } else {
            Event::PreModelRequest
        };
        let effects = hooks
            .run(
                self.host()?,
                event,
                input.session_id,
                fields,
                &[],
                Some(Dialect::Codex),
            )
            .await;
        Ok(ImportedModelAttemptCharge {
            extra_codex_rollout_budget_units: effects.extra_codex_rollout_budget_units,
        })
    }
}

fn command_session(input: &CommandInvokeInput) -> PluginResult<i64> {
    input
        .session_id
        .filter(|session| *session > 0)
        .ok_or_else(|| PluginError::invalid_params("this command requires a project session"))
}

fn delegated_command_result(text: &str, output: Value) -> CommandResult {
    let summary = text
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("Completed")
        .trim()
        .chars()
        .take(160)
        .collect::<String>();
    CommandResult::succeeded(summary)
        .with_detail(text.chars().take(8192).collect::<String>())
        .with_output(output)
}

fn joined_context(context: Vec<String>) -> Option<String> {
    (!context.is_empty()).then(|| context.join("\n"))
}

fn stop_patch(effects: Effects) -> Option<AgentStopPatch> {
    let message = effects
        .block
        .clone()
        .or_else(|| joined_context(effects.context))?;
    Some(AgentStopPatch {
        continue_with_message: Some(message),
        reason: effects.block,
    })
}

fn subagent_type(dialect: Dialect, role: &str) -> &str {
    match (dialect, role) {
        (Dialect::Claude, "worker") => "general-purpose",
        (Dialect::Claude, "explore") => "Explore",
        (Dialect::Codex, "worker") => "default",
        _ => role,
    }
}

fn compaction_trigger(automatic: bool) -> &'static str {
    if automatic { "auto" } else { "manual" }
}

fn provider_error_type(category: agena_failure::FailureCategory) -> &'static str {
    use agena_failure::FailureCategory;
    match category {
        FailureCategory::RateLimited => "rate_limit",
        FailureCategory::AuthenticationRequired => "authentication_failed",
        FailureCategory::QuotaExceeded => "billing_error",
        FailureCategory::InvalidInput => "invalid_request",
        FailureCategory::NotFound => "model_not_found",
        FailureCategory::Timeout
        | FailureCategory::DependencyUnavailable
        | FailureCategory::ProtocolFailure => "server_error",
        _ => "unknown",
    }
}

pub(crate) struct ToolProjection<'a> {
    key: &'a ToolKey,
    name: Cow<'a, str>,
    fields: &'static [(&'static str, &'static str)],
    canonical: String,
    mcp: Option<(String, String)>,
    claude_mcp: Option<ClaudeMcpTool>,
}

impl<'a> ToolProjection<'a> {
    pub(crate) fn new(key: &'a ToolKey, args: &Value) -> Self {
        let (name, fields): (&str, &[_]) =
            match (key.plugin().namespace(), key.plugin().name(), key.name()) {
                ("agena", "shell", "exec" | "spawn") => {
                    let name = if args.get("shell").and_then(Value::as_str) == Some("powershell") {
                        "PowerShell"
                    } else {
                        "Bash"
                    };
                    (name, &[])
                }
                ("agena", "fs", "read") => ("Read", &[]),
                ("agena", "fs", "write") => ("Write", &[("path", "file_path")]),
                ("agena", "fs", "replace") => (
                    "Edit",
                    &[
                        ("path", "file_path"),
                        ("old", "old_string"),
                        ("new", "new_string"),
                    ],
                ),
                ("agena", "fs", "apply_patch") => ("apply_patch", &[]),
                ("agena", "fs", "glob") => ("Glob", &[]),
                ("agena", "fs", "grep") => ("Grep", &[]),
                ("agena", "notebook", "read") => ("NotebookRead", &[]),
                ("agena", "notebook", "edit") => ("NotebookEdit", &[]),
                ("agena", "notebook", "edit_cell") => (
                    "NotebookEdit",
                    &[
                        ("path", "notebook_path"),
                        ("cell_index", "cell_number"),
                        ("source", "new_source"),
                    ],
                ),
                ("agena", "web", "fetch") => ("WebFetch", &[]),
                ("agena", "web", "search") => ("WebSearch", &[]),
                ("agena", "agent", "spawn") => ("Agent", &[]),
                ("agena", "skills", "get") => ("Skill", &[("id", "skill"), ("arguments", "args")]),
                ("agena", "interaction", "ask") => ("AskUserQuestion", &[]),
                ("agena", "task", "create") => ("TaskCreate", &[]),
                ("agena", "task", "get") => ("TaskGet", &[]),
                ("agena", "task", "list") => ("TaskList", &[]),
                ("agena", "task", "update") => ("TaskUpdate", &[]),
                ("agena", "task", "todo_write") => ("TodoWrite", &[]),
                _ => (key.name(), &[]),
            };
        let canonical = key.to_string();
        let mcp = (canonical == "agena.mcp.tools.call")
            .then(|| {
                Some((
                    args.get("server")?.as_str()?.to_owned(),
                    args.get("name")?.as_str()?.to_owned(),
                ))
            })
            .flatten();
        let name = match &mcp {
            Some((server, tool)) => {
                // Package-contributed server IDs carry a namespace in Agena.
                // Match the upstream hook name against the original server name.
                let source = server
                    .strip_prefix("agent-plugin-")
                    .and_then(|value| value.split_once('-'))
                    .filter(|(hash, _)| {
                        hash.len() == 24 && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
                    })
                    .map_or(server.as_str(), |(_, source)| source);
                Cow::Owned(format!("mcp__{source}__{tool}"))
            }
            None => Cow::Borrowed(name),
        };
        Self {
            key,
            name,
            fields,
            canonical,
            mcp,
            claude_mcp: None,
        }
    }

    pub(crate) async fn resolve_mcp(&mut self, host: Option<&dyn HostClient>) {
        let Some((server, tool)) = &self.mcp else {
            return;
        };
        let identity = match host.filter(|_| server.starts_with("agent-plugin-")) {
            Some(host) => host
                .imported_mcp_server_identity(server)
                .await
                .unwrap_or_default(),
            None => None,
        };
        self.claude_mcp = Some(claude_mcp_tool(server, tool, identity));
    }

    pub(crate) fn mod_event(&self, call_id: i64, agent_id: Option<i64>, input: &Value) -> Value {
        let arguments = self.foreign_input(input.clone());
        let mut fields = arguments.as_object().cloned().unwrap_or_default();
        fields.insert("tool".into(), json!(self.mod_tool_name()));
        fields.insert("tool_use_id".into(), json!(call_id.to_string()));
        if let Some(agent_id) = agent_id {
            fields.insert("agentId".into(), json!(agent_id.to_string()));
        }
        Value::Object(fields)
    }

    pub(crate) fn mod_check_event(
        &self,
        call_id: i64,
        agent_id: Option<i64>,
        input: &Value,
        read_only: bool,
    ) -> Value {
        let mut event = json!({
            "tool": self.mod_tool_name(),
            "input": self.foreign_input(input.clone()),
        });
        if call_id >= 0 {
            event["tool_use_id"] = json!(call_id.to_string());
        }
        if let Some(agent_id) = agent_id {
            event["agentId"] = json!(agent_id.to_string());
        } else {
            event["approver"] = json!("person");
        }
        if read_only {
            event["isReadOnly"] = json!(true);
        }
        event
    }

    fn mod_tool_name(&self) -> &str {
        self.claude_mcp
            .as_ref()
            .map_or(self.name.as_ref(), |mcp| &mcp.tool_name)
    }

    pub(crate) fn mod_arguments(
        &self,
        event: &Value,
        input: &Value,
        changed: Value,
    ) -> Option<Value> {
        let mut changed = match changed {
            Value::Object(changed) => changed,
            _ => return None,
        };
        for field in ["tool", "tool_use_id", "agentId"] {
            if changed
                .get(field)
                .is_some_and(|value| event.get(field) != Some(value))
            {
                return None;
            }
        }
        let original = self.foreign_input(input.clone());
        for field in ["tool", "tool_use_id", "agentId", "requestMeta"] {
            match original.get(field) {
                Some(value) => changed.insert(field.into(), value.clone()),
                None => changed.remove(field),
            };
        }
        let arguments = Value::Object(changed);
        if arguments == original {
            return Some(input.clone());
        }
        let mut projected = self.to_agena_input(arguments);
        if self.mcp.is_some() {
            // MCP operation options belong to the host invocation envelope.
            let mut original = input.clone();
            original["arguments"] = projected["arguments"].take();
            projected = original;
        }
        Some(projected)
    }

    fn hook_input(&self, fields: Value) -> HookInput<'_> {
        HookInput {
            fields,
            claude_mcp: self.claude_mcp.as_ref(),
            codex_tool_cwd: None,
            codex_shell_hook: CodexShellHook::Default,
            skip_scope: None,
            canonical_model: None,
            process_cwd: None,
        }
    }

    fn codex_shell_hook(&self, args: &Value, cwd: Option<&str>) -> CodexShellHook<'static> {
        if cwd.is_none() {
            return CodexShellHook::Default;
        }
        match (self.canonical.as_str(), args["shell"] != "powershell") {
            ("agena.shell.open", true) => CodexShellHook::Launch(ShellLaunchMode::Open),
            ("agena.shell.spawn", true) => CodexShellHook::Launch(ShellLaunchMode::Spawn),
            ("agena.shell.watch", true) if args["command"].is_string() => {
                CodexShellHook::Launch(ShellLaunchMode::Watch)
            }
            ("agena.shell.read" | "agena.shell.write" | "agena.shell.logs", _) => {
                CodexShellHook::Skip
            }
            _ => CodexShellHook::Default,
        }
    }

    fn aliases(&self) -> Vec<&str> {
        let mut aliases = vec![self.key.name(), &self.canonical];
        if self.name == "Bash" {
            aliases.extend(["bash", "shell", "shell_command", "exec_command"]);
        }
        if self.name == "Agent" {
            aliases.push("Task");
        }
        if self.name == "Edit" {
            aliases.push("MultiEdit");
        }
        if self.name == "NotebookRead" {
            aliases.extend(["Read", "ReadNotebook"]);
        }
        if self.name == "NotebookEdit" {
            aliases.extend(["Edit", "NotebookEditCell"]);
        }
        aliases
    }

    fn fields(&self, call: i64, input: Value) -> Value {
        json!({
            "tool_name": self.name,
            "tool_input": self.foreign_input(input),
            "tool_use_id": call.to_string(),
        })
    }

    fn foreign_input(&self, mut input: Value) -> Value {
        if self.mcp.is_some() {
            return input
                .as_object_mut()
                .and_then(|input| input.remove("arguments"))
                .filter(|input| input.is_object())
                .unwrap_or_else(|| json!({}));
        }
        if let Some(object) = input.as_object_mut() {
            for (agena, foreign) in self.fields {
                if let Some(value) = object.remove(*agena) {
                    object.insert((*foreign).into(), value);
                }
            }
        }
        input
    }

    fn to_agena_input(&self, mut input: Value) -> Value {
        if let Some((server, tool)) = &self.mcp {
            return json!({"server": server, "name": tool, "arguments": input});
        }
        if let Some(object) = input.as_object_mut() {
            for (agena, foreign) in self.fields {
                if let Some(value) = object.remove(*foreign) {
                    object.insert((*agena).into(), value);
                }
            }
        }
        input
    }
}

fn claude_mcp_tool(
    server: &str,
    tool: &str,
    identity: Option<agena_plugin_sdk::host_api::ImportedMcpServerIdentity>,
) -> ClaudeMcpTool {
    fn component(value: &str) -> String {
        value
            .chars()
            .map(|ch| {
                if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
                    ch
                } else {
                    '_'
                }
            })
            .collect()
    }
    let (segment, provenance) = match identity {
        Some(identity) => (
            format!(
                "plugin_{}_{}",
                component(&identity.package),
                component(&identity.server)
            ),
            json!({"name":format!("plugin:{}:{}", identity.package, identity.server), "source":"plugin"}),
        ),
        None => (component(server), json!({"name":server,"source":"agena"})),
    };
    ClaudeMcpTool {
        tool_name: format!("mcp__{segment}__{}", component(tool)),
        provenance,
    }
}

pub fn project_transcript_tool(
    name: &str,
    input: Value,
    format: agena_plugin_sdk::host_api::HookTranscriptFormat,
) -> (String, Value) {
    use agena_plugin_sdk::host_api::HookTranscriptFormat;

    if format == HookTranscriptFormat::Claude {
        let key = if [
            "shell.",
            "fs.",
            "mcp.",
            "web.",
            "agent.",
            "skills.",
            "task.",
            "interaction.",
        ]
        .iter()
        .any(|prefix| name.starts_with(*prefix))
        {
            format!("agena.{name}").parse::<ToolKey>()
        } else {
            name.parse::<ToolKey>()
        };
        if let Ok(key) = key {
            let tool = ToolProjection::new(&key, &input);
            return (tool.name.to_string(), tool.foreign_input(input));
        }
    }
    (name.to_owned(), input)
}

pub fn project_claude_mod_tool_input(key: &ToolKey, mut input: Value) -> Value {
    if matches!(
        (key.plugin().namespace(), key.plugin().name()),
        ("agena", "shell")
    ) && let Some(args) = input.as_object_mut()
        && let Some(timeout) = args.remove("timeout")
    {
        args.entry("timeout_ms").or_insert(timeout);
    }
    ToolProjection::new(key, &input).to_agena_input(input)
}
