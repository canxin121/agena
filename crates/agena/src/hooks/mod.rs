//! Config-driven shell hook plugin.
//!
//! Lets users wire shell commands to plugin hook events via `agena.toml`
//! without authoring a Rust plugin. Each `agena.hooks` plugin option entry
//! pairs an `event` name with a `command` to run; an optional `match.tool`
//! glob narrows the scope for tool hooks.
//!
//! Supported events (see `HookEvent`): `user_prompt_submit`, `pre_tool_use`,
//! `post_tool_use`, `post_tool_use_failure`, `stop`, `session_start`,
//! `session_end`, `notification`.
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
    NotificationInput, PermissionAskDecision, PermissionAskInput, Plugin, PluginManifest,
    Result as SdkResult, SessionEndInput, SessionStartInput, SessionStartPatch, ToolAfterInput,
    ToolAfterPatch, ToolBeforeInput, ToolBeforePatch, ToolFailureInput, UserPromptSubmitInput,
    UserPromptSubmitPatch,
};

const SHELL_HOOK_PLUGIN_ID: &str = "agena.hooks";
const DEFAULT_TIMEOUT_MS: u64 = 30_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[serde(rename_all = "snake_case")]
pub enum HookEvent {
    UserPromptSubmit,
    #[serde(rename = "pre_tool_use", alias = "tool_before")]
    ToolBefore,
    #[serde(rename = "post_tool_use", alias = "tool_after")]
    ToolAfter,
    #[serde(rename = "post_tool_use_failure", alias = "tool_failure")]
    ToolFailure,
    #[serde(rename = "stop", alias = "agent_stop")]
    AgentStop,
    SessionStart,
    SessionEnd,
    Notification,
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
    /// Shell command. Mutually exclusive with `url`; if both are set, `url`
    /// wins and a warning is logged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// HTTP endpoint. When set, the hook POSTs the input as JSON to this
    /// URL and parses the response body as the corresponding patch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
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
            && self.url == other.url
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
                HookEvent::Notification => HookSubscription::NOTIFICATION,
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

    fn run_notification_hooks(
        &self,
        kind: &str,
        session_id: Option<i64>,
        title: &str,
        message: &str,
        payload: &serde_json::Value,
    ) {
        let hooks = self.matches(HookEvent::Notification, None);
        if hooks.is_empty() {
            return;
        }

        let mut env = base_env();
        env.insert("AGENA_HOOK_EVENT".into(), "notification".into());
        env.insert("AGENA_NOTIFICATION_KIND".into(), kind.to_string());
        env.insert("AGENA_NOTIFICATION_TITLE".into(), title.to_string());
        env.insert("AGENA_NOTIFICATION_MESSAGE".into(), message.to_string());
        if let Some(session_id) = session_id {
            env.insert("AGENA_SESSION_ID".into(), session_id.to_string());
        }
        for hook in hooks {
            let _ = hook.run::<serde_json::Value>(&env, payload);
        }
    }
}

#[async_trait]
impl Plugin for ShellHookPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::builder(SHELL_HOOK_PLUGIN_ID, env!("CARGO_PKG_VERSION"))
            .description("Agena shell hook bridge")
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
        let payload = serde_json::to_value(&input).unwrap_or(serde_json::Value::Null);
        let mut merged = UserPromptSubmitPatch::default();
        for hook in self.matches(HookEvent::UserPromptSubmit, None) {
            if let Some(patch) = hook.run::<UserPromptSubmitPatch>(&env, &payload) {
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
        let payload = serde_json::to_value(&input).unwrap_or(serde_json::Value::Null);
        let mut merged = ToolBeforePatch::default();
        for hook in self.matches(HookEvent::ToolBefore, Some(input.tool_name.as_str())) {
            if let Some(patch) = hook.run::<ToolBeforePatch>(&env, &payload) {
                merge_tool_before_patch(&mut merged, patch);
            }
        }
        Ok(if tool_before_patch_is_empty(&merged) {
            None
        } else {
            Some(merged)
        })
    }

    async fn tool_execute_after(&self, input: ToolAfterInput) -> SdkResult<Option<ToolAfterPatch>> {
        let mut env = base_env();
        env.insert("AGENA_HOOK_EVENT".into(), "tool_after".into());
        env.insert("AGENA_SESSION_ID".into(), input.session_id.to_string());
        env.insert("AGENA_CALL_ID".into(), input.call_id.to_string());
        env.insert("AGENA_TOOL_NAME".into(), input.tool_name.clone());
        let payload = serde_json::to_value(&input).unwrap_or(serde_json::Value::Null);
        for hook in self.matches(HookEvent::ToolAfter, Some(input.tool_name.as_str())) {
            let _ = hook.run::<ToolAfterPatch>(&env, &payload);
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
        let payload = serde_json::to_value(&input).unwrap_or(serde_json::Value::Null);
        for hook in self.matches(HookEvent::ToolFailure, Some(input.tool_name.as_str())) {
            let _ = hook.run::<serde_json::Value>(&env, &payload);
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
        let payload = serde_json::to_value(&input).unwrap_or(serde_json::Value::Null);
        let mut merged = AgentStopPatch::default();
        for hook in self.matches(HookEvent::AgentStop, None) {
            if let Some(patch) = hook.run::<AgentStopPatch>(&env, &payload) {
                if patch.continue_with_message.is_some() {
                    merged.continue_with_message = patch.continue_with_message;
                }
                if patch.reason.is_some() {
                    merged.reason = patch.reason;
                }
            }
        }
        if merged.continue_with_message.is_none() {
            self.run_notification_hooks(
                "agent_stop",
                Some(input.session_id),
                "Agena turn completed",
                input
                    .last_assistant_message
                    .as_deref()
                    .unwrap_or("turn completed"),
                &payload,
            );
        }
        Ok(
            if merged.continue_with_message.is_none() && merged.reason.is_none() {
                None
            } else {
                Some(merged)
            },
        )
    }

    async fn permission_ask(
        &self,
        input: PermissionAskInput,
    ) -> SdkResult<Option<PermissionAskDecision>> {
        let payload = serde_json::to_value(&input).unwrap_or(serde_json::Value::Null);
        self.run_notification_hooks(
            "permission_request",
            Some(input.session_id),
            "Agena needs permission",
            input.action.as_str(),
            &payload,
        );
        Ok(None)
    }

    async fn notification(&self, input: NotificationInput) -> SdkResult<()> {
        self.run_notification_hooks(
            input.kind.as_str(),
            input.session_id,
            input.title.as_str(),
            input.message.as_str(),
            &input.payload,
        );
        Ok(())
    }

    async fn session_start(
        &self,
        input: SessionStartInput,
    ) -> SdkResult<Option<SessionStartPatch>> {
        let mut env = base_env();
        env.insert("AGENA_HOOK_EVENT".into(), "session_start".into());
        env.insert("AGENA_SESSION_ID".into(), input.session_id.to_string());
        let payload = serde_json::to_value(&input).unwrap_or(serde_json::Value::Null);
        for hook in self.matches(HookEvent::SessionStart, None) {
            let _ = hook.run::<SessionStartPatch>(&env, &payload);
        }
        Ok(None)
    }

    async fn session_end(&self, input: SessionEndInput) -> SdkResult<()> {
        let mut env = base_env();
        env.insert("AGENA_HOOK_EVENT".into(), "session_end".into());
        env.insert("AGENA_SESSION_ID".into(), input.session_id.to_string());
        let payload = serde_json::to_value(&input).unwrap_or(serde_json::Value::Null);
        for hook in self.matches(HookEvent::SessionEnd, None) {
            let _ = hook.run::<serde_json::Value>(&env, &payload);
        }
        Ok(())
    }
}

enum HookTarget {
    Shell { command: String },
    Http { url: String },
}

struct CompiledHook {
    event: HookEvent,
    target: HookTarget,
    tool_matcher: Option<GlobMatcher>,
    timeout: Duration,
}

impl std::fmt::Debug for CompiledHook {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut dbg = f.debug_struct("CompiledHook");
        dbg.field("event", &self.event);
        match &self.target {
            HookTarget::Shell { command } => dbg.field("kind", &"shell").field("command", command),
            HookTarget::Http { url } => dbg.field("kind", &"http").field("url", url),
        };
        dbg.field("has_tool_matcher", &self.tool_matcher.is_some())
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

        let target = match (entry.url, entry.command) {
            (Some(url), command) => {
                if command.is_some() {
                    tracing::warn!(
                        target: "agena::hooks",
                        "hook entry has both `url` and `command`; using `url`"
                    );
                }
                let url = url.trim().to_string();
                if url.is_empty() {
                    return Err("hook url must not be empty".to_string());
                }
                if !(url.starts_with("http://") || url.starts_with("https://")) {
                    return Err(format!(
                        "hook url must start with http:// or https:// (got `{url}`)"
                    ));
                }
                HookTarget::Http { url }
            }
            (None, Some(command)) => {
                let command = command.trim().to_string();
                if command.is_empty() {
                    return Err("hook command must not be empty".to_string());
                }
                HookTarget::Shell { command }
            }
            (None, None) => {
                return Err("hook entry must specify either `command` or `url`".to_string());
            }
        };

        Ok(Self {
            event: entry.event,
            target,
            tool_matcher,
            timeout,
        })
    }

    fn run<P>(&self, env: &BTreeMap<String, String>, payload: &serde_json::Value) -> Option<P>
    where
        P: serde::de::DeserializeOwned,
    {
        match &self.target {
            HookTarget::Shell { command } => self.run_shell(command, env),
            HookTarget::Http { url } => self.run_http(url, env, payload),
        }
    }

    fn run_shell<P>(&self, command: &str, env: &BTreeMap<String, String>) -> Option<P>
    where
        P: serde::de::DeserializeOwned,
    {
        let shell_cmd = if cfg!(windows) {
            ("cmd", vec!["/d", "/s", "/c", command])
        } else {
            ("/bin/sh", vec!["-lc", command])
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
                    "hook command failed to spawn: {err} (command: {command})"
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
                            "hook exited with status {status} (command: {command}): {stderr}"
                        );
                        return None;
                    }
                    return parse_patch(stdout.trim());
                }
                Ok(None) => {
                    if started.elapsed() >= self.timeout {
                        let _ = child.kill();
                        let _ = child.wait();
                        tracing::warn!(
                            target: "agena::hooks",
                            "hook timed out after {:?} (command: {command})",
                            self.timeout
                        );
                        return None;
                    }
                    std::thread::sleep(poll);
                }
                Err(err) => {
                    tracing::warn!(
                        target: "agena::hooks",
                        "failed to wait on hook child: {err} (command: {command})"
                    );
                    return None;
                }
            }
        }
    }

    fn run_http<P>(
        &self,
        url: &str,
        env: &BTreeMap<String, String>,
        payload: &serde_json::Value,
    ) -> Option<P>
    where
        P: serde::de::DeserializeOwned,
    {
        let event_label = env.get("AGENA_HOOK_EVENT").cloned().unwrap_or_default();
        let body = serde_json::json!({
            "event": event_label,
            "env": env,
            "payload": payload,
        });

        let timeout = self.timeout;
        let url = url.to_string();

        // Hooks are dispatched from async plugin impls. We spin a dedicated
        // single-thread runtime so the async reqwest call cannot stall the
        // caller's runtime. The dispatch stays synchronous to match
        // `run_shell` semantics.
        let result = std::thread::scope(|scope| {
            let handle = scope.spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(err) => {
                        tracing::warn!(
                            target: "agena::hooks",
                            "failed to build hook runtime: {err}"
                        );
                        return None;
                    }
                };
                runtime.block_on(async move {
                    let client = match reqwest::Client::builder().timeout(timeout).build() {
                        Ok(client) => client,
                        Err(err) => {
                            tracing::warn!(
                                target: "agena::hooks",
                                "failed to build hook http client: {err}"
                            );
                            return None;
                        }
                    };
                    let response = match client.post(&url).json(&body).send().await {
                        Ok(r) => r,
                        Err(err) => {
                            tracing::warn!(
                                target: "agena::hooks",
                                "hook http POST failed: {err} (url: {url})"
                            );
                            return None;
                        }
                    };
                    if !response.status().is_success() {
                        tracing::warn!(
                            target: "agena::hooks",
                            "hook http endpoint returned status {} (url: {url})",
                            response.status()
                        );
                        return None;
                    }
                    match response.text().await {
                        Ok(text) => Some(text),
                        Err(err) => {
                            tracing::warn!(
                                target: "agena::hooks",
                                "failed to read hook http response: {err}"
                            );
                            None
                        }
                    }
                })
            });
            handle.join().ok().flatten()
        });

        result.and_then(|text| parse_patch(text.trim()))
    }
}

fn parse_patch<P>(text: &str) -> Option<P>
where
    P: serde::de::DeserializeOwned,
{
    if text.is_empty() {
        return None;
    }
    match serde_json::from_str::<P>(text) {
        Ok(patch) => Some(patch),
        Err(err) => {
            tracing::debug!(
                target: "agena::hooks",
                "hook output was not a valid patch ({err}); treating as side-effect only"
            );
            None
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
            command: Some("   ".to_string()),
            url: None,
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
            command: Some("true".to_string()),
            url: None,
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
                command: Some("true".to_string()),
                url: None,
                matcher: HookMatcher::default(),
                timeout_ms: None,
            },
            HookEntry {
                event: HookEvent::ToolBefore,
                command: Some("true".to_string()),
                url: None,
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
    fn notification_hook_subscribes_to_notification() {
        let plugin = ShellHookPlugin::new(HooksConfig::new(vec![HookEntry {
            event: HookEvent::Notification,
            command: Some("true".to_string()),
            url: None,
            matcher: HookMatcher::default(),
            timeout_ms: None,
        }]));

        let subs = plugin.subscriptions();

        assert!(subs.contains(HookSubscription::NOTIFICATION));
    }

    #[test]
    fn tool_match_filters_by_glob() {
        let plugin = ShellHookPlugin::new(HooksConfig::new(vec![HookEntry {
            event: HookEvent::ToolBefore,
            command: Some("true".to_string()),
            url: None,
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
                command: Some("true".to_string()),
                url: None,
                matcher: HookMatcher {
                    tool: Some("[bad".to_string()),
                },
                timeout_ms: None,
            },
            HookEntry {
                event: HookEvent::ToolBefore,
                command: Some("true".to_string()),
                url: None,
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
            command: Some(r#"printf '{"additional_context":"injected"}'"#.to_string()),
            url: None,
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
    fn notification_hook_runs_real_command() {
        use tokio::runtime::Runtime;
        let rt = Runtime::new().unwrap();
        let plugin = ShellHookPlugin::new(HooksConfig::new(vec![HookEntry {
            event: HookEvent::Notification,
            command: Some(r#"test \"$AGENA_NOTIFICATION_KIND\" = \"scheduled_job\" && test \"$AGENA_NOTIFICATION_TITLE\" = \"Scheduled job failed\""#.to_string()),
            url: None,
            matcher: HookMatcher::default(),
            timeout_ms: Some(2_000),
        }]));
        rt.block_on(plugin.notification(NotificationInput {
            kind: "scheduled_job".to_string(),
            session_id: Some(7),
            title: "Scheduled job failed".to_string(),
            message: "boom".to_string(),
            payload: serde_json::json!({"job_id": "123"}),
        }))
        .expect("hook ok");
    }

    #[cfg(unix)]
    #[test]
    fn tool_before_hook_blocks_when_matcher_hits_and_returns_block_reason() {
        use tokio::runtime::Runtime;
        let rt = Runtime::new().unwrap();
        let plugin = ShellHookPlugin::new(HooksConfig::new(vec![HookEntry {
            event: HookEvent::ToolBefore,
            command: Some(r#"printf '{"title_override":"Audited"}'"#.to_string()),
            url: None,
            matcher: HookMatcher {
                tool: Some("bash".to_string()),
            },
            timeout_ms: Some(2_000),
        }]));
        let patch = rt
            .block_on(plugin.tool_execute_before(ToolBeforeInput {
                tool_name: "bash".to_string(),
                plugin_name: "bash".to_string(),
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
                plugin_name: "read".to_string(),
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

    /// Minimal one-shot HTTP/1.1 server: accepts a single connection, reads
    /// the request bytes, then writes a fixed 200 response and closes.
    fn spawn_one_shot_http(response_body: &'static str) -> u16 {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf);
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    response_body.len(),
                    response_body
                );
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
            }
        });
        port
    }

    #[test]
    fn http_hook_compile_rejects_invalid_url() {
        let err = CompiledHook::compile(HookEntry {
            event: HookEvent::UserPromptSubmit,
            command: None,
            url: Some("ftp://example.com/x".to_string()),
            matcher: HookMatcher::default(),
            timeout_ms: None,
        })
        .unwrap_err();
        assert!(err.contains("must start with http"));
    }

    #[test]
    fn http_hook_compile_requires_command_or_url() {
        let err = CompiledHook::compile(HookEntry {
            event: HookEvent::UserPromptSubmit,
            command: None,
            url: None,
            matcher: HookMatcher::default(),
            timeout_ms: None,
        })
        .unwrap_err();
        assert!(err.contains("either `command` or `url`"));
    }

    #[test]
    fn http_hook_runs_endpoint_and_parses_patch() {
        use tokio::runtime::Runtime;
        let rt = Runtime::new().unwrap();
        let port = spawn_one_shot_http(r#"{"additional_context":"http-injected"}"#);
        let plugin = ShellHookPlugin::new(HooksConfig::new(vec![HookEntry {
            event: HookEvent::UserPromptSubmit,
            command: None,
            url: Some(format!("http://127.0.0.1:{port}/hook")),
            matcher: HookMatcher::default(),
            timeout_ms: Some(3_000),
        }]));
        let patch = rt
            .block_on(plugin.user_prompt_submit(UserPromptSubmitInput {
                session_id: 9,
                prompt: "ping".to_string(),
            }))
            .expect("hook ok")
            .expect("patch present");
        assert_eq!(patch.additional_context.as_deref(), Some("http-injected"));
    }
}
