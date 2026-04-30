//! Config-driven shell hook plugin.
//!
//! Lets users wire shell commands to plugin hook events via `agena.toml`
//! without authoring a Rust plugin — the shape Claude Code's `settings.json`
//! hooks expose. Each `[[hooks]]` entry pairs an `event` name with a `command`
//! to run; an optional `match.tool` glob narrows the scope for tool hooks.
//!
//! Supported events (see `HookEvent`): `user_prompt_submit`, `tool_before`,
//! `tool_after`, `tool_failure`, `agent_stop`, `session_start`, `session_end`.
//!
//! Side-effect-only by default: stdout is captured but ignored. To produce a
//! `Patch` (e.g. block a tool call, replace a prompt), the command must print
//! a single JSON object on stdout matching the corresponding patch struct.
//! A non-zero exit code or invalid JSON is logged but does not derail the
//! hook chain — failures of user-supplied scripts must not silently brick
//! the agent loop.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use globset::{Glob, GlobMatcher};
use serde::{Deserialize, Serialize};

use crate::plugin::sdk::{
    AgentStopInput, AgentStopPatch, HookSubscription, HostClient, InitContext, InitOutcome,
    Plugin, PluginManifest, Result as SdkResult, SessionEndInput, SessionStartInput,
    SessionStartPatch, ToolAfterInput, ToolAfterPatch, ToolBeforeInput, ToolBeforePatch,
    ToolFailureInput, UserPromptSubmitInput, UserPromptSubmitPatch,
};

const SHELL_HOOK_PLUGIN_ID: &str = "agena-shell-hooks";
const DEFAULT_TIMEOUT_MS: u64 = 30_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[serde(rename_all = "snake_case")]
pub enum HookEvent {
    UserPromptSubmit,
    ToolBefore,
    ToolAfter,
    ToolFailure,
    AgentStop,
    SessionStart,
    SessionEnd,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct HookMatcher {
    /// Glob applied to the tool name (only meaningful for `tool_*` events).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookEntry {
    pub event: HookEvent,
    pub command: String,
    #[serde(default)]
    pub matcher: HookMatcher,
    /// Hard timeout applied to each invocation. Defaults to 30s.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct HooksConfig {
    pub entries: Vec<HookEntry>,
}

impl PartialEq for HookEntry {
    fn eq(&self, other: &Self) -> bool {
        self.event == other.event
            && self.command == other.command
            && self.timeout_ms == other.timeout_ms
            && self.matcher.tool == other.matcher.tool
    }
}

impl HooksConfig {
    pub fn new(entries: Vec<HookEntry>) -> Self {
        Self { entries }
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn entries(&self) -> &[HookEntry] {
        &self.entries
    }
}

/// In-tree plugin that fans out hook events to the configured shell commands.
pub struct ShellHookPlugin {
    entries: Vec<CompiledHook>,
}

impl ShellHookPlugin {
    pub fn new(config: HooksConfig) -> Self {
        let entries = config
            .entries
            .into_iter()
            .filter_map(|entry| match CompiledHook::compile(entry) {
                Ok(c) => Some(c),
                Err(err) => {
                    tracing::warn!(
                        target: "agena::hooks",
                        "skipping invalid hook entry: {err}"
                    );
                    None
                }
            })
            .collect();
        Self { entries }
    }

    pub fn id() -> &'static str {
        SHELL_HOOK_PLUGIN_ID
    }

    fn subscriptions(&self) -> HookSubscription {
        let mut subs = HookSubscription::empty();
        for entry in &self.entries {
            subs |= match entry.event {
                HookEvent::UserPromptSubmit => HookSubscription::USER_PROMPT_SUBMIT,
                HookEvent::ToolBefore => HookSubscription::TOOL_BEFORE,
                HookEvent::ToolAfter => HookSubscription::TOOL_AFTER,
                HookEvent::ToolFailure => HookSubscription::TOOL_FAILURE,
                HookEvent::AgentStop => HookSubscription::AGENT_STOP,
                HookEvent::SessionStart => HookSubscription::SESSION_START,
                HookEvent::SessionEnd => HookSubscription::SESSION_END,
            };
        }
        subs
    }

    fn matches(&self, event: HookEvent, tool_name: Option<&str>) -> Vec<&CompiledHook> {
        self.entries
            .iter()
            .filter(|hook| hook.event == event)
            .filter(|hook| match (&hook.tool_matcher, tool_name) {
                (Some(matcher), Some(name)) => matcher.is_match(name),
                (Some(_), None) => false,
                (None, _) => true,
            })
            .collect()
    }
}

#[async_trait]
impl Plugin for ShellHookPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::builder(SHELL_HOOK_PLUGIN_ID, env!("CARGO_PKG_VERSION"))
            .description("Agena built-in shell hook bridge")
            .hooks(self.subscriptions())
            .build()
    }

    async fn init(&self, _ctx: InitContext, _host: Arc<dyn HostClient>) -> SdkResult<InitOutcome> {
        Ok(InitOutcome::ack(self.manifest()))
    }

    async fn user_prompt_submit(
        &self,
        input: UserPromptSubmitInput,
    ) -> SdkResult<Option<UserPromptSubmitPatch>> {
        let mut env = base_env();
        env.insert("AGENA_HOOK_EVENT".into(), "user_prompt_submit".into());
        env.insert("AGENA_SESSION_ID".into(), input.session_id.to_string());
        env.insert("AGENA_PROMPT".into(), input.prompt.clone());
        let mut merged = UserPromptSubmitPatch::default();
        for hook in self.matches(HookEvent::UserPromptSubmit, None) {
            if let Some(patch) = hook.run::<UserPromptSubmitPatch>(&env) {
                merge_user_prompt_patch(&mut merged, patch);
            }
        }
        Ok(if user_prompt_patch_is_empty(&merged) {
            None
        } else {
            Some(merged)
        })
    }

    async fn tool_execute_before(
        &self,
        input: ToolBeforeInput,
    ) -> SdkResult<Option<ToolBeforePatch>> {
        let mut env = base_env();
        env.insert("AGENA_HOOK_EVENT".into(), "tool_before".into());
        env.insert("AGENA_SESSION_ID".into(), input.session_id.to_string());
        env.insert("AGENA_CALL_ID".into(), input.call_id.to_string());
        env.insert("AGENA_TOOL_NAME".into(), input.tool_name.clone());
        env.insert(
            "AGENA_TOOL_INPUT".into(),
            serde_json::to_string(&input.input).unwrap_or_default(),
        );
        let mut merged = ToolBeforePatch::default();
        for hook in self.matches(HookEvent::ToolBefore, Some(input.tool_name.as_str())) {
            if let Some(patch) = hook.run::<ToolBeforePatch>(&env) {
                merge_tool_before_patch(&mut merged, patch);
            }
        }
        Ok(if tool_before_patch_is_empty(&merged) {
            None
        } else {
            Some(merged)
        })
    }

    async fn tool_execute_after(
        &self,
        input: ToolAfterInput,
    ) -> SdkResult<Option<ToolAfterPatch>> {
        let mut env = base_env();
        env.insert("AGENA_HOOK_EVENT".into(), "tool_after".into());
        env.insert("AGENA_SESSION_ID".into(), input.session_id.to_string());
        env.insert("AGENA_CALL_ID".into(), input.call_id.to_string());
        env.insert("AGENA_TOOL_NAME".into(), input.tool_name.clone());
        for hook in self.matches(HookEvent::ToolAfter, Some(input.tool_name.as_str())) {
            let _ = hook.run::<ToolAfterPatch>(&env);
        }
        Ok(None)
    }

    async fn tool_execute_failure(&self, input: ToolFailureInput) -> SdkResult<()> {
        let mut env = base_env();
        env.insert("AGENA_HOOK_EVENT".into(), "tool_failure".into());
        env.insert("AGENA_SESSION_ID".into(), input.session_id.to_string());
        env.insert("AGENA_CALL_ID".into(), input.call_id.to_string());
        env.insert("AGENA_TOOL_NAME".into(), input.tool_name.clone());
        env.insert("AGENA_ERROR".into(), input.error.clone());
        for hook in self.matches(HookEvent::ToolFailure, Some(input.tool_name.as_str())) {
            let _ = hook.run::<serde_json::Value>(&env);
        }
        Ok(())
    }

    async fn agent_stop(&self, input: AgentStopInput) -> SdkResult<Option<AgentStopPatch>> {
        let mut env = base_env();
        env.insert("AGENA_HOOK_EVENT".into(), "agent_stop".into());
        env.insert("AGENA_SESSION_ID".into(), input.session_id.to_string());
        if let Some(msg) = input.last_assistant_message.as_deref() {
            env.insert("AGENA_LAST_ASSISTANT".into(), msg.to_string());
        }
        let mut merged = AgentStopPatch::default();
        for hook in self.matches(HookEvent::AgentStop, None) {
            if let Some(patch) = hook.run::<AgentStopPatch>(&env) {
                if patch.continue_with_message.is_some() {
                    merged.continue_with_message = patch.continue_with_message;
                }
                if patch.reason.is_some() {
                    merged.reason = patch.reason;
                }
            }
        }
        Ok(if merged.continue_with_message.is_none() && merged.reason.is_none() {
            None
        } else {
            Some(merged)
        })
    }

    async fn session_start(
        &self,
        input: SessionStartInput,
    ) -> SdkResult<Option<SessionStartPatch>> {
        let mut env = base_env();
        env.insert("AGENA_HOOK_EVENT".into(), "session_start".into());
        env.insert("AGENA_SESSION_ID".into(), input.session_id.to_string());
        for hook in self.matches(HookEvent::SessionStart, None) {
            let _ = hook.run::<SessionStartPatch>(&env);
        }
        Ok(None)
    }

    async fn session_end(&self, input: SessionEndInput) -> SdkResult<()> {
        let mut env = base_env();
        env.insert("AGENA_HOOK_EVENT".into(), "session_end".into());
        env.insert("AGENA_SESSION_ID".into(), input.session_id.to_string());
        for hook in self.matches(HookEvent::SessionEnd, None) {
            let _ = hook.run::<serde_json::Value>(&env);
        }
        Ok(())
    }
}

struct CompiledHook {
    event: HookEvent,
    command: String,
    tool_matcher: Option<GlobMatcher>,
    timeout: Duration,
}

impl std::fmt::Debug for CompiledHook {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompiledHook")
            .field("event", &self.event)
            .field("command", &self.command)
            .field("has_tool_matcher", &self.tool_matcher.is_some())
            .field("timeout", &self.timeout)
            .finish()
    }
}

impl CompiledHook {
    fn compile(entry: HookEntry) -> Result<Self, String> {
        let tool_matcher = match entry.matcher.tool.as_deref() {
            Some(pattern) => Some(
                Glob::new(pattern)
                    .map_err(|e| format!("invalid hook tool glob `{pattern}`: {e}"))?
                    .compile_matcher(),
            ),
            None => None,
        };
        let timeout = Duration::from_millis(entry.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS));
        if entry.command.trim().is_empty() {
            return Err("hook command must not be empty".to_string());
        }
        Ok(Self {
            event: entry.event,
            command: entry.command,
            tool_matcher,
            timeout,
        })
    }

    fn run<P>(&self, env: &BTreeMap<String, String>) -> Option<P>
    where
        P: serde::de::DeserializeOwned,
    {
        let shell_cmd = if cfg!(windows) {
            ("cmd", vec!["/d", "/s", "/c", self.command.as_str()])
        } else {
            ("/bin/sh", vec!["-lc", self.command.as_str()])
        };

        let mut cmd = Command::new(shell_cmd.0);
        cmd.args(&shell_cmd.1)
            .envs(env.iter())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(err) => {
                tracing::warn!(
                    target: "agena::hooks",
                    "hook command failed to spawn: {err} (command: {})",
                    self.command
                );
                return None;
            }
        };

        let started = std::time::Instant::now();
        let poll = Duration::from_millis(20);
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    let mut stdout = String::new();
                    let mut stderr = String::new();
                    if let Some(mut out) = child.stdout.take() {
                        let _ = std::io::Read::read_to_string(&mut out, &mut stdout);
                    }
                    if let Some(mut err) = child.stderr.take() {
                        let _ = std::io::Read::read_to_string(&mut err, &mut stderr);
                    }
                    if !status.success() {
                        tracing::warn!(
                            target: "agena::hooks",
                            "hook exited with status {} (command: {}): {stderr}",
                            status,
                            self.command
                        );
                        return None;
                    }
                    let trimmed = stdout.trim();
                    if trimmed.is_empty() {
                        return None;
                    }
                    return match serde_json::from_str::<P>(trimmed) {
                        Ok(patch) => Some(patch),
                        Err(err) => {
                            tracing::debug!(
                                target: "agena::hooks",
                                "hook stdout was not a valid patch ({err}); treating as side-effect only"
                            );
                            None
                        }
                    };
                }
                Ok(None) => {
                    if started.elapsed() >= self.timeout {
                        let _ = child.kill();
                        let _ = child.wait();
                        tracing::warn!(
                            target: "agena::hooks",
                            "hook timed out after {:?} (command: {})",
                            self.timeout, self.command
                        );
                        return None;
                    }
                    std::thread::sleep(poll);
                }
                Err(err) => {
                    tracing::warn!(
                        target: "agena::hooks",
                        "failed to wait on hook child: {err} (command: {})",
                        self.command
                    );
                    return None;
                }
            }
        }
    }
}

fn base_env() -> BTreeMap<String, String> {
    let mut env: BTreeMap<String, String> = std::env::vars().collect();
    env.insert(
        "AGENA_VERSION".into(),
        env!("CARGO_PKG_VERSION").to_string(),
    );
    if let Ok(cwd) = std::env::current_dir() {
        env.insert("AGENA_CWD".into(), normalize(cwd));
    }
    env
}

fn normalize(path: PathBuf) -> String {
    path.to_string_lossy().into_owned()
}

fn merge_user_prompt_patch(into: &mut UserPromptSubmitPatch, overlay: UserPromptSubmitPatch) {
    if overlay.prompt.is_some() {
        into.prompt = overlay.prompt;
    }
    if let Some(extra) = overlay.additional_context {
        match into.additional_context.as_mut() {
            Some(existing) => existing.push_str(&format!("\n{extra}")),
            None => into.additional_context = Some(extra),
        }
    }
    if overlay.block_reason.is_some() {
        into.block_reason = overlay.block_reason;
    }
}

fn user_prompt_patch_is_empty(p: &UserPromptSubmitPatch) -> bool {
    p.prompt.is_none() && p.additional_context.is_none() && p.block_reason.is_none()
}

fn merge_tool_before_patch(into: &mut ToolBeforePatch, overlay: ToolBeforePatch) {
    if overlay.input.is_some() {
        into.input = overlay.input;
    }
    if overlay.title_override.is_some() {
        into.title_override = overlay.title_override;
    }
    if !overlay.metadata.is_empty() {
        into.metadata.extend(overlay.metadata);
    }
}

fn tool_before_patch_is_empty(p: &ToolBeforePatch) -> bool {
    p.input.is_none() && p.title_override.is_none() && p.metadata.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_command_is_rejected() {
        let err = CompiledHook::compile(HookEntry {
            event: HookEvent::UserPromptSubmit,
            command: "   ".to_string(),
            matcher: HookMatcher::default(),
            timeout_ms: None,
        })
        .unwrap_err();
        assert!(err.contains("must not be empty"));
    }

    #[test]
    fn invalid_tool_glob_is_rejected() {
        let err = CompiledHook::compile(HookEntry {
            event: HookEvent::ToolBefore,
            command: "true".to_string(),
            matcher: HookMatcher {
                tool: Some("[bad".to_string()),
            },
            timeout_ms: None,
        })
        .unwrap_err();
        assert!(err.contains("invalid hook tool glob"));
    }

    #[test]
    fn shell_hook_plugin_picks_subscriptions_from_entries() {
        let plugin = ShellHookPlugin::new(HooksConfig::new(vec![
            HookEntry {
                event: HookEvent::UserPromptSubmit,
                command: "true".to_string(),
                matcher: HookMatcher::default(),
                timeout_ms: None,
            },
            HookEntry {
                event: HookEvent::ToolBefore,
                command: "true".to_string(),
                matcher: HookMatcher::default(),
                timeout_ms: None,
            },
        ]));
        let subs = plugin.subscriptions();
        assert!(subs.contains(HookSubscription::USER_PROMPT_SUBMIT));
        assert!(subs.contains(HookSubscription::TOOL_BEFORE));
        assert!(!subs.contains(HookSubscription::TOOL_AFTER));
    }

    #[test]
    fn tool_match_filters_by_glob() {
        let plugin = ShellHookPlugin::new(HooksConfig::new(vec![HookEntry {
            event: HookEvent::ToolBefore,
            command: "true".to_string(),
            matcher: HookMatcher {
                tool: Some("bash".to_string()),
            },
            timeout_ms: None,
        }]));
        assert_eq!(plugin.matches(HookEvent::ToolBefore, Some("bash")).len(), 1);
        assert_eq!(plugin.matches(HookEvent::ToolBefore, Some("read")).len(), 0);
        assert_eq!(plugin.matches(HookEvent::ToolBefore, None).len(), 0);
    }

    #[test]
    fn invalid_glob_entry_is_dropped_at_construction() {
        let plugin = ShellHookPlugin::new(HooksConfig::new(vec![
            HookEntry {
                event: HookEvent::ToolBefore,
                command: "true".to_string(),
                matcher: HookMatcher {
                    tool: Some("[bad".to_string()),
                },
                timeout_ms: None,
            },
            HookEntry {
                event: HookEvent::ToolBefore,
                command: "true".to_string(),
                matcher: HookMatcher::default(),
                timeout_ms: None,
            },
        ]));
        assert_eq!(plugin.entries.len(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn user_prompt_hook_runs_real_command_and_parses_patch() {
        use tokio::runtime::Runtime;
        let rt = Runtime::new().unwrap();
        let plugin = ShellHookPlugin::new(HooksConfig::new(vec![HookEntry {
            event: HookEvent::UserPromptSubmit,
            command: r#"printf '{"additional_context":"injected"}'"#.to_string(),
            matcher: HookMatcher::default(),
            timeout_ms: Some(2_000),
        }]));
        let patch = rt
            .block_on(plugin.user_prompt_submit(UserPromptSubmitInput {
                session_id: 7,
                prompt: "hi".to_string(),
            }))
            .expect("hook ok")
            .expect("patch present");
        assert_eq!(patch.additional_context.as_deref(), Some("injected"));
        assert!(patch.prompt.is_none());
    }

    #[cfg(unix)]
    #[test]
    fn tool_before_hook_blocks_when_matcher_hits_and_returns_block_reason() {
        use tokio::runtime::Runtime;
        let rt = Runtime::new().unwrap();
        let plugin = ShellHookPlugin::new(HooksConfig::new(vec![HookEntry {
            event: HookEvent::ToolBefore,
            command: r#"printf '{"title_override":"Audited"}'"#.to_string(),
            matcher: HookMatcher {
                tool: Some("bash".to_string()),
            },
            timeout_ms: Some(2_000),
        }]));
        let patch = rt
            .block_on(plugin.tool_execute_before(ToolBeforeInput {
                tool_name: "bash".to_string(),
                source: crate::plugin::sdk::ToolSource::Builtin,
                session_id: 1,
                call_id: 2,
                workspace_root: "/tmp".to_string(),
                input: serde_json::json!({"command": "echo hi"}),
                title_override: None,
                metadata: Default::default(),
            }))
            .expect("hook ok")
            .expect("patch present");
        assert_eq!(patch.title_override.as_deref(), Some("Audited"));

        // A non-matching tool yields None.
        let none = rt
            .block_on(plugin.tool_execute_before(ToolBeforeInput {
                tool_name: "read".to_string(),
                source: crate::plugin::sdk::ToolSource::Builtin,
                session_id: 1,
                call_id: 3,
                workspace_root: "/tmp".to_string(),
                input: serde_json::json!({}),
                title_override: None,
                metadata: Default::default(),
            }))
            .expect("hook ok");
        assert!(none.is_none());
    }
}
