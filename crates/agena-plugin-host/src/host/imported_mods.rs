use std::future::Future;

use agena_plugin_sdk::{
    PluginKey, SessionStartInput, SessionStartSource, ToolKey,
    host_api::{
        HostCallbackContext, current_host_callback_context, run_in_isolated_host_callback_context,
    },
};
use futures_util::future::BoxFuture;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{LoadedPlugin, PluginHost};
use crate::agent_plugin::{ToolProjection, claude_mods::Mods};

pub struct ImportedPromptSubmit<'a> {
    pub session_id: i64,
    pub workspace_root: &'a str,
    pub model: &'a str,
    pub sandbox_enabled: bool,
    pub input: Value,
    pub media: Option<&'a ImportedPromptMediaReader<'a>>,
}

pub type ImportedPromptMediaReader<'a> =
    dyn Fn(String, u64) -> BoxFuture<'a, Value> + Send + Sync + 'a;

pub struct ImportedToolCall<'a> {
    pub tool: &'a ToolKey,
    pub session_id: i64,
    pub call_id: i64,
    pub workspace_root: &'a str,
    pub subagent: bool,
    pub sandbox_enabled: bool,
    pub input: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum ImportedToolDecision {
    Allow,
    Ask {
        #[serde(skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    Deny {
        #[serde(skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
}

#[derive(Clone, Copy)]
enum ToolModHook {
    Call,
    Run,
    Check { read_only: bool },
}

impl ToolModHook {
    fn event(self) -> &'static str {
        match self {
            Self::Call => "tool.call",
            Self::Run => "tool.run",
            Self::Check { .. } => "tool.check",
        }
    }
}

type Continuation<'a> = Box<dyn FnOnce(Value) -> BoxFuture<'a, Value> + Send + 'a>;

impl PluginHost {
    fn mod_plugins(&self, event: &str) -> impl DoubleEndedIterator<Item = (&LoadedPlugin, &Mods)> {
        self.plugins.iter().filter_map(move |plugin| {
            let mods = plugin.agent_plugin()?.mod_runtime()?;
            mods.matches(event).then_some((plugin.as_ref(), mods))
        })
    }

    pub fn has_imported_tool_call_hooks(&self, enabled: impl Fn(&PluginKey) -> bool) -> bool {
        self.mod_plugins("tool.call")
            .any(|(plugin, _)| enabled(&plugin.key()))
    }

    pub fn has_imported_tool_check_hooks(&self, enabled: impl Fn(&PluginKey) -> bool) -> bool {
        self.mod_plugins("tool.check")
            .any(|(plugin, _)| enabled(&plugin.key()))
    }

    pub fn has_imported_tool_run_hooks(&self, enabled: impl Fn(&PluginKey) -> bool) -> bool {
        self.mod_plugins("tool.run")
            .any(|(plugin, _)| enabled(&plugin.key()))
    }

    pub fn has_imported_mod_session_start(&self) -> bool {
        self.mod_plugins("session.start").next().is_some()
    }

    pub fn has_imported_prompt_submit_hooks(&self) -> bool {
        self.mod_plugins("prompt.submit").next().is_some()
    }

    pub async fn run_imported_prompt_submit<'a>(
        &'a self,
        submission: ImportedPromptSubmit<'a>,
        enabled: impl Fn(&PluginKey) -> bool,
        terminal: impl FnOnce(Value) -> BoxFuture<'a, Value> + Send + 'a,
    ) -> Value {
        let ImportedPromptSubmit {
            session_id,
            workspace_root,
            model,
            sandbox_enabled,
            input,
            media,
        } = submission;
        let outer = current_host_callback_context().unwrap_or_default();
        let mut next: Continuation<'_> = Box::new(terminal);
        for (plugin, mods) in self.mod_plugins("prompt.submit").rev() {
            let key = plugin.key();
            if !enabled(&key) {
                continue;
            }
            let Some(host) = mods.host() else {
                continue;
            };
            let outer = outer.clone();
            next = Box::new(move |input| {
                Box::pin(async move {
                    let context = HostCallbackContext {
                        session_id: Some(session_id),
                        workspace_root: Some(workspace_root.to_owned()),
                        caller: outer.caller.clone(),
                        sandbox_enabled: Some(sandbox_enabled),
                        ..Default::default()
                    };
                    self._host_handle
                        .run_in_authorized_callback_context(&key, context, async {
                            mods.start(
                                host,
                                &SessionStartInput {
                                    session_id,
                                    workspace_root: workspace_root.to_owned(),
                                    source: SessionStartSource::Resume,
                                    model: Some(model.to_owned()),
                                    session_title: None,
                                    agent_type: None,
                                    resumed_cache: None,
                                },
                            )
                            .await;
                            let mut forwarded = input.clone();
                            mods.run_with_prompt_media(
                                host,
                                session_id,
                                "prompt.submit",
                                input,
                                media,
                                |mut event| async move {
                                    // The VM can only set the two editable fields, even
                                    // if a module changes its own JS runtime globals.
                                    if forwarded.get("command").is_none() {
                                        forwarded["text"] = event["text"].take();
                                    }
                                    if let Some(context) = event
                                        .as_object_mut()
                                        .and_then(|event| event.remove("context"))
                                    {
                                        forwarded["context"] = context;
                                    } else if let Some(event) = forwarded.as_object_mut() {
                                        event.remove("context");
                                    }
                                    run_in_isolated_host_callback_context(outer, next(forwarded))
                                        .await
                                },
                            )
                            .await
                        })
                        .await
                })
            });
        }
        next(input).await
    }

    pub async fn prepare_imported_mod_session(
        &self,
        input: &SessionStartInput,
        enabled: impl Fn(&PluginKey) -> bool,
    ) {
        for (plugin, mods) in self.mod_plugins("session.start") {
            let key = plugin.key();
            if !enabled(&key) {
                continue;
            }
            let Some(host) = mods.host() else {
                continue;
            };
            let context = HostCallbackContext {
                session_id: Some(input.session_id),
                workspace_root: Some(input.workspace_root.clone()),
                ..Default::default()
            };
            self._host_handle
                .run_in_authorized_callback_context(&key, context, mods.start(host, input))
                .await;
        }
    }

    pub async fn run_imported_tool_call<F, Fut>(
        &self,
        call: ImportedToolCall<'_>,
        enabled: impl Fn(&PluginKey) -> bool,
        terminal: F,
    ) -> Value
    where
        F: FnOnce(Value) -> Fut + Send,
        Fut: Future<Output = Value> + Send,
    {
        self.run_imported_tool_hook(call, ToolModHook::Call, enabled, terminal)
            .await
    }

    pub async fn run_imported_tool_run<F, Fut>(
        &self,
        call: ImportedToolCall<'_>,
        enabled: impl Fn(&PluginKey) -> bool,
        terminal: F,
    ) -> Value
    where
        F: FnOnce(Value) -> Fut + Send,
        Fut: Future<Output = Value> + Send,
    {
        self.run_imported_tool_hook(call, ToolModHook::Run, enabled, terminal)
            .await
    }

    pub async fn run_imported_tool_check(
        &self,
        call: ImportedToolCall<'_>,
        read_only: bool,
        baseline: &ImportedToolDecision,
        enabled: impl Fn(&PluginKey) -> bool,
    ) -> Option<ImportedToolDecision> {
        let answer = self
            .run_imported_tool_hook(call, ToolModHook::Check { read_only }, enabled, |_| async {
                json!(baseline)
            })
            .await;
        serde_json::from_value(answer)
            .ok()
            .filter(|decision| decision != baseline)
    }

    async fn run_imported_tool_hook<F, Fut>(
        &self,
        mut call: ImportedToolCall<'_>,
        hook: ToolModHook,
        enabled: impl Fn(&PluginKey) -> bool,
        terminal: F,
    ) -> Value
    where
        F: FnOnce(Value) -> Fut + Send,
        Fut: Future<Output = Value> + Send,
    {
        let args = call.input.take();
        let outer = current_host_callback_context().unwrap_or_default();
        let mut next: Continuation<'_> = Box::new(|args| Box::pin(terminal(args)));
        for (plugin, mods) in self.mod_plugins(hook.event()).rev() {
            let key = plugin.key();
            if !enabled(&key) {
                continue;
            }
            let Some(host) = mods.host() else {
                continue;
            };
            let call = &call;
            let outer = &outer;
            next = Box::new(move |args| {
                Box::pin(async move {
                    let context = HostCallbackContext {
                        session_id: Some(call.session_id),
                        call_id: Some(call.call_id),
                        workspace_root: Some(call.workspace_root.to_owned()),
                        caller: outer.caller.clone(),
                        sandbox_enabled: Some(call.sandbox_enabled),
                        ..Default::default()
                    };
                    self._host_handle.run_in_authorized_callback_context(&key, context, async {
                        if !call.subagent {
                            mods.start(host, &SessionStartInput {
                                session_id: call.session_id,
                                workspace_root: call.workspace_root.to_owned(),
                                source: SessionStartSource::Resume,
                                model: None,
                                session_title: None,
                                agent_type: None,
                                resumed_cache: None,
                            }).await;
                        }
                        let mut tool = ToolProjection::new(call.tool, &args);
                        tool.resolve_mcp(Some(host)).await;
                        match hook {
                            ToolModHook::Call | ToolModHook::Run => {
                                let input = tool.mod_event(
                                    call.call_id, call.subagent.then_some(call.session_id), &args,
                                );
                                mods.run(host, call.session_id, hook.event(), input.clone(), |event| async {
                                    let Some(args) = tool.mod_arguments(&input, &args, event) else {
                                        return json!({"deny": "Claude mod cannot change the tool or call identity"});
                                    };
                                    run_in_isolated_host_callback_context(outer.clone(), next(args)).await
                                }).await
                            }
                            ToolModHook::Check { read_only } => {
                                let input = tool.mod_check_event(
                                    call.call_id, call.subagent.then_some(call.session_id), &args, read_only,
                                );
                                mods.run(host, call.session_id, hook.event(), input, |_| async {
                                    run_in_isolated_host_callback_context(outer.clone(), next(args)).await
                                }).await
                            }
                        }
                    }).await
                })
            });
        }
        next(args).await
    }
}
