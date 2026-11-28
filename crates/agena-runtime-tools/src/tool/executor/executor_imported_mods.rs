use agena_domain::{PermissionDecision, SessionKind, StructuredObject};
use agena_plugin_host::{
    current_plugin_call_completion,
    host::{ImportedPromptScope, ImportedToolCall, ImportedToolDecision},
    sdk::{
        ToolTag, current_cancellation_token,
        host_api::{current_host_callback_context, run_in_isolated_host_callback_context},
    },
};
use agena_tool::shell_analysis::{CommandClassification, analyze_command};
use futures_util::future::BoxFuture;
use serde_json::{Value, json};

use super::{
    RegisteredTool, ToolError, ToolExecutionView, ToolExecutor, ToolInvocation,
    ToolInvocationExecution, ToolOutput,
};

type Execution = Result<ToolInvocationExecution, ToolError>;
type Execute<'a> = Box<dyn FnOnce(ToolInvocation) -> BoxFuture<'a, Execution> + Send + 'a>;

#[derive(Clone, Copy, PartialEq, Eq)]
enum ToolHook {
    Call,
    Run,
}

/// Nested mod events and native execution can each poll through several host
/// scopes. Run a phase on its own task stack and retain the active authority.
pub async fn run_tool_task<T: Send + 'static>(
    effect: BoxFuture<'static, T>,
) -> Result<T, ToolError> {
    let callback = current_host_callback_context().unwrap_or_default();
    let completion = current_plugin_call_completion();
    let prompt_scope = ImportedPromptScope::current();
    let cancellation = current_cancellation_token().map(|token| token.child_token());
    let sandbox_enabled = agena_process::sandbox::enabled();
    let task = tokio::spawn(Box::pin(async move {
        let effect = agena_process::sandbox::scope_enabled(
            sandbox_enabled,
            run_in_isolated_host_callback_context(callback, effect),
        );
        let run = prompt_scope.run(async move {
            match completion {
                Some(completion) => completion.scope(effect).await,
                None => effect.await,
            }
        });
        match cancellation {
            Some(token) => {
                agena_plugin_host::sdk::cancellation::run_with_cancellation(token, async {
                    Ok(run.await)
                })
                .await
                .map_err(ToolError::from_plugin_error)
            }
            None => Ok(run.await),
        }
    }));
    let _abort = agena_async::AbortOnDrop::new(&task);
    task.await.map_err(|error| {
        if error.is_cancelled() {
            ToolError::Cancelled
        } else {
            ToolError::plugin(format!("tool task failed before completion: {error}"))
        }
    })?
}

impl ToolExecutor {
    pub fn is_claude_mod_tool(&self, invocation: &ToolInvocation) -> bool {
        self.invocation_definition(invocation)
            .is_some_and(|tool| tool.claude_mod_tool_name().is_some())
    }

    pub fn has_imported_tool_check_hooks(&self) -> bool {
        !self.is_global()
            && self.session_kind == SessionKind::Project
            && self
                .plugins
                .has_imported_tool_check_hooks(|plugin| self.plugin_is_enabled(plugin))
    }

    pub async fn run_imported_tool_check(
        &self,
        invocation: &ToolInvocation,
        session_id: i64,
        call_id: i64,
        baseline: PermissionDecision,
    ) -> Option<ImportedToolDecision> {
        if !self.has_imported_tool_check_hooks() {
            return None;
        }
        let executor = self.clone();
        let invocation = invocation.clone();
        match run_tool_task(Box::pin(async move {
            Box::pin(executor.check_imported_tool(&invocation, session_id, call_id, baseline)).await
        }))
        .await
        {
            Ok(decision) => decision,
            Err(error) => {
                tracing::warn!(%error, "Claude mod tool permission hook stopped");
                None
            }
        }
    }

    async fn check_imported_tool(
        &self,
        invocation: &ToolInvocation,
        session_id: i64,
        call_id: i64,
        baseline: PermissionDecision,
    ) -> Option<ImportedToolDecision> {
        let definition = self.invocation_definition(invocation)?;
        let baseline = match baseline {
            PermissionDecision::Allow => ImportedToolDecision::Allow,
            PermissionDecision::Ask { reason } | PermissionDecision::Auto { reason } => {
                ImportedToolDecision::Ask {
                    reason: Some(reason),
                }
            }
            PermissionDecision::Deny { reason } => ImportedToolDecision::Deny {
                reason: Some(reason),
            },
        };
        let workspace = self.workspace_root.to_string_lossy();
        let call =
            self.imported_tool_call(&definition, invocation, session_id, call_id, &workspace);
        let read_only = ToolTag::is_read_only(&definition.effective_tags())
            || (matches!(
                definition.canonical_name().as_str(),
                "agena.shell.exec" | "agena.shell.spawn"
            ) && call.input["shell"] != "powershell"
                && call.input["command"].as_str().is_some_and(|command| {
                    analyze_command(command).classification == CommandClassification::ReadOnly
                }));
        self.plugins
            .run_imported_tool_check(call, read_only, &baseline, |plugin| {
                self.plugin_is_enabled(plugin)
            })
            .await
    }

    fn imported_tool_call<'a>(
        &self,
        definition: &'a RegisteredTool,
        invocation: &ToolInvocation,
        session_id: i64,
        call_id: i64,
        workspace: &'a str,
    ) -> ImportedToolCall<'a> {
        ImportedToolCall {
            tool: definition.tool_key(),
            session_id,
            call_id,
            workspace_root: workspace,
            subagent: self.session_relation_kind.is_subagent(),
            sandbox_enabled: self.sandbox_enabled(),
            input: Value::from(invocation.input.clone()),
        }
    }

    pub fn run_imported_tool_call<'a>(
        &'a self,
        invocation: &'a ToolInvocation,
        session_id: i64,
        call_id: i64,
        terminal: impl FnOnce(ToolInvocation) -> BoxFuture<'a, Execution> + Send + 'a,
    ) -> BoxFuture<'a, Execution> {
        Box::pin(self.execute_imported_tool_hook(
            invocation,
            session_id,
            call_id,
            ToolHook::Call,
            Box::new(terminal),
        ))
    }

    pub fn run_authorized_tool<'a>(
        &'a self,
        invocation: &'a ToolInvocation,
        context: crate::tool::ToolRuntimeContext,
    ) -> BoxFuture<'a, Execution> {
        if !self
            .plugins
            .has_imported_tool_run_hooks(|plugin| self.plugin_is_enabled(plugin))
        {
            return Box::pin(self.execute_invocation(invocation, context));
        }
        let executor = self.clone();
        let invocation = invocation.clone();
        Box::pin(async move {
            run_tool_task(Box::pin(async move {
                let action = &executor;
                Box::pin(executor.execute_imported_tool_hook(
                    &invocation,
                    context.session_id.unwrap_or(-1),
                    context.call_id.unwrap_or(-1),
                    ToolHook::Run,
                    Box::new(move |invocation| {
                        Box::pin(
                            async move { action.execute_invocation(&invocation, context).await },
                        )
                    }),
                ))
                .await
            }))
            .await?
        })
    }

    async fn execute_imported_tool_hook(
        &self,
        invocation: &ToolInvocation,
        session_id: i64,
        call_id: i64,
        hook: ToolHook,
        terminal: Execute<'_>,
    ) -> Execution {
        if session_id <= 0
            || self.is_global()
            || self.session_kind != SessionKind::Project
            || !match hook {
                ToolHook::Call => self
                    .plugins
                    .has_imported_tool_call_hooks(|plugin| self.plugin_is_enabled(plugin)),
                ToolHook::Run => self
                    .plugins
                    .has_imported_tool_run_hooks(|plugin| self.plugin_is_enabled(plugin)),
            }
        {
            return terminal(invocation.clone()).await;
        }
        let Some(definition) = self.invocation_definition(invocation) else {
            return terminal(invocation.clone()).await;
        };
        // The registering mod implements this tool through the authorized
        // plugin invocation. Other mods can still wrap that invocation.
        let owner = definition
            .claude_mod_tool_name()
            .map(|_| definition.plugin_key());
        self.ensure_not_cancelled()?;
        self.verify_channel_access().await?;
        self.require_plugin_enabled(&definition)?;
        self.check_cloud_tool_adapter(&definition.canonical_name())?;
        if self.channel_authority.is_some()
            || self
                .conversation
                .as_ref()
                .is_some_and(|context| context.is_read_only())
        {
            self.authorize_invocation(invocation)?;
        }

        let workspace = self.workspace_root.to_string_lossy();
        let call =
            self.imported_tool_call(&definition, invocation, session_id, call_id, &workspace);
        let mut core = None;
        let receipt = &mut core;
        let forward = |input| async move {
            let execution = match StructuredObject::try_from(input) {
                Ok(input) => {
                    terminal(ToolInvocation {
                        input,
                        ..invocation.clone()
                    })
                    .await
                }
                Err(error) => Err(ToolError::invalid_input(error)),
            };
            let answer = project_result(&execution);
            *receipt = Some((answer.clone(), execution));
            answer
        };
        let enabled = |plugin: &_| Some(plugin) != owner && self.plugin_is_enabled(plugin);
        let answer = match hook {
            ToolHook::Call => {
                Box::pin(self.plugins.run_imported_tool_call(call, enabled, forward)).await
            }
            ToolHook::Run => {
                Box::pin(self.plugins.run_imported_tool_run(call, enabled, forward)).await
            }
        };
        apply_answer(&definition, answer, core)
    }
}

fn project_result(execution: &Execution) -> Value {
    match execution {
        Err(error) => {
            let reason = error
                .actionable_message()
                .unwrap_or_else(|| error.to_string());
            if matches!(
                error,
                ToolError::PolicyDenied(_) | ToolError::UserDeclined(_)
            ) {
                json!({"deny":reason})
            } else {
                json!({"result":reason, "text":reason, "isError":true})
            }
        }
        Ok(execution) => {
            let text = execution
                .view
                .model_output
                .as_ref()
                .unwrap_or(&execution.view.output_text);
            let result = execution
                .output
                .to_json_payload()
                .unwrap_or_else(|| json!(text));
            let mut response = json!({"ref":1,"result":result,"text":text});
            if !execution.view.additional_context.is_empty() {
                response["context"] = json!(execution.view.additional_context);
            }
            if execution
                .view
                .metadata
                .get("exit_code")
                .is_some_and(|code| code != "0")
            {
                response["isError"] = json!(true);
            }
            response
        }
    }
}

fn apply_answer(
    definition: &RegisteredTool,
    mut answer: Value,
    core: Option<(Value, Execution)>,
) -> Execution {
    let end_turn = answer["isEnd"] == Value::Bool(true);
    if let Some((projected, execution)) = core {
        if answer == projected {
            return execution;
        }
        // The executor owns the receipt and permission failures, including
        // any interaction that was opened during the original invocation.
        let mut execution = execution?;
        if let Some(reason) = answer["deny"].as_str() {
            execution.view.model_output = Some(format!(
                "{} ran and a Claude mod withheld its output. {}",
                definition.tool_name(),
                bounded(reason, 8192),
            ));
            execution
                .view
                .metadata
                .insert("imported_mod_result".into(), "withheld".into());
            if end_turn {
                execution
                    .view
                    .metadata
                    .insert(agena_plugin_host::sdk::TOOL_END_TURN.into(), "true".into());
            }
            return Ok(execution);
        }
        if let Some(Value::Array(blocks)) = answer.get_mut("content").map(Value::take) {
            let (text, attachments) =
                agena_plugin_host::agent_plugin::decode_content_blocks(&blocks, "Claude mod tool")
                    .map_err(ToolError::from_plugin_error)?;
            if !text.is_empty() || !attachments.is_empty() {
                execution.view.model_output = Some(text);
                execution.view.attachments = attachments;
                execution
                    .view
                    .metadata
                    .insert("imported_mod_result".into(), "replaced".into());
            }
        } else if let Some(mut result) = answer.get_mut("result").map(Value::take)
            && result != projected["result"]
            && definition.validate_output(Some(&result)).is_ok()
        {
            execution.view.model_output = Some(result_text(&mut result));
            execution
                .view
                .metadata
                .insert("imported_mod_result".into(), "replaced".into());
        }
        if let Some(context) = result_context(&mut answer) {
            execution.view.additional_context = context;
        }
        if end_turn {
            execution
                .view
                .metadata
                .insert(agena_plugin_host::sdk::TOOL_END_TURN.into(), "true".into());
        }
        return Ok(execution);
    }
    if let Some(reason) = answer["deny"].as_str() {
        return Err(ToolError::plugin(bounded(reason, 8192)));
    }
    let content_blocks = answer.get_mut("content").map(Value::take);
    let result = answer.get_mut("result").map(Value::take);
    let (output, output_text, attachments) = if let Some(Value::Array(blocks)) = content_blocks {
        let (text, attachments) =
            agena_plugin_host::agent_plugin::decode_content_blocks(&blocks, "Claude mod tool")
                .map_err(ToolError::from_plugin_error)?;
        let (output, fallback_text) = if let Some(mut result) = result {
            definition
                .validate_output(Some(&result))
                .map_err(ToolError::plugin)?;
            let output = ToolOutput::from_json_payload(result.as_object().map(|_| &result))
                .map_err(ToolError::plugin)?;
            (output, result_text(&mut result))
        } else {
            (ToolOutput::default(), String::new())
        };
        let final_text = if text.is_empty() { fallback_text } else { text };
        (output, final_text, attachments)
    } else {
        let Some(mut result) = result else {
            return Err(ToolError::plugin("Claude mod returned no tool result"));
        };
        definition
            .validate_output(Some(&result))
            .map_err(ToolError::plugin)?;
        let output = ToolOutput::from_json_payload(result.as_object().map(|_| &result))
            .map_err(ToolError::plugin)?;
        (output, result_text(&mut result), Vec::new())
    };
    let mut view = ToolExecutionView::simple(
        definition.tool_name(),
        "Claude mod supplied the tool result",
        output_text,
    );
    view.attachments = attachments;
    view.additional_context = result_context(&mut answer).unwrap_or_default();
    view.metadata
        .insert("imported_mod_result".into(), "supplied".into());
    if end_turn {
        view.metadata
            .insert(agena_plugin_host::sdk::TOOL_END_TURN.into(), "true".into());
    }
    Ok(ToolInvocationExecution::new(output, view))
}

fn result_text(value: &mut Value) -> String {
    match value.take() {
        Value::String(mut text) => {
            text.truncate(text.floor_char_boundary(text.len().min(128 * 1024)));
            text
        }
        value => bounded(&value.to_string(), 128 * 1024),
    }
}

fn result_context(answer: &mut Value) -> Option<Vec<String>> {
    let mut remaining = 16 * 1024;
    Some(
        answer
            .get_mut("context")?
            .as_array_mut()?
            .iter_mut()
            .map_while(|item| {
                if remaining == 0 {
                    return None;
                }
                let text = bounded(item.as_str().unwrap_or_default(), remaining);
                remaining = remaining.saturating_sub(text.len() + 1);
                Some(text)
            })
            .filter(|text| !text.is_empty())
            .collect(),
    )
}

fn bounded(text: &str, limit: usize) -> String {
    text[..text.floor_char_boundary(text.len().min(limit))].to_owned()
}
