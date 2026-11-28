//! Local providers and retained promises for direct Cordis child activations.

use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use agena_plugin_sdk::{
    HostClient, PluginError, PluginErrorKind, Result,
    host_api::{
        AgentModelSelection, AgentSpawnRequest, CordisSubagentDelivery, CordisSubagentResult,
        CordisSubagentRun, CordisSubagentStart, CordisSubagentStopReason,
    },
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::events::SubagentEvents;
use super::workflow::{CordisConfig, SubagentProvider};
use crate::schema::CompiledSchema;

pub(super) const SERVICE: &str = include_str!("subagents.js");
pub(super) const EXPORTS: &str = r#"
export const ROOT_AGENT_NAME = 'lead';
export const SubagentRunId = id => id;
export class SubagentError extends Error {
    constructor(message, code) { super(message); this.name = 'SubagentError'; this.code = code; }
}
export function assertSubagentMaxDepth(depth) {
    if (depth !== undefined && (!Number.isSafeInteger(depth) || depth < 0)) {
        throw new SubagentError('maxDepth must be a non-negative safe integer', 'INVALID_MAX_DEPTH');
    }
}
"#;

const MAX_PROMPT_BYTES: usize = 128 * 1024;
const MAX_SCHEMA_BYTES: usize = 64 * 1024;

pub(super) fn providers(config: &CordisConfig) -> Value {
    Value::Array(config.providers().map(|(name, provider)| {
        let fork = provider == SubagentProvider::Fork;
        json!({ "name": name, "inheritsParentContext": fork, "capabilities": {
            "agentOptions": true, "agentPreset": !fork, "outputSchema": true, "depthLimit": true
        }})
    }).collect())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Request {
    run_id: Uuid,
    provider: String,
    name: Option<String>,
    child_id: Option<String>,
    parent_id: Option<String>,
    label: String,
    prompt: Vec<Value>,
    cwd: Option<String>,
    agent_preset: Option<String>,
    agent_options: Option<AgentOptions>,
    output_schema: Option<Value>,
    max_depth: Option<u64>,
    delivery: CordisSubagentDelivery,
}

#[derive(Deserialize)]
#[serde(
    tag = "operation",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub(super) enum ControlRequest {
    Scope,
    List { parent_id: String },
    Resolve { name: String },
    Message {
        target_id: String,
        #[serde(default)]
        text: Option<String>,
        #[serde(default)]
        blocks: Vec<Value>,
    },
    Interrupt { target_id: String },
    Wait { timeout_ms: u64 },
    WaitForTreeWork,
    ModelRoutes,
}

impl ControlRequest {
    pub(super) fn into_host(self) -> Result<agena_plugin_sdk::host_api::CordisSubagentControl> {
        use agena_plugin_sdk::host_api::CordisSubagentControl;
        match self {
            Self::Scope => Ok(CordisSubagentControl::Scope),
            Self::List { parent_id } => Ok(CordisSubagentControl::List { parent_id }),
            Self::Resolve { name } => Ok(CordisSubagentControl::Resolve { name }),
            Self::Message {
                target_id,
                text,
                blocks,
            } => {
                let (text, attachments) = if !blocks.is_empty() {
                    if blocks.len() > 256 {
                        return Err(PluginError::invalid_params(
                            "A Cordis subagent message accepts at most 256 content blocks",
                        ));
                    }
                    let (decoded_text, attachments) =
                        super::tools::decode_content_blocks(&blocks, "Cordis subagent message")?;
                    if decoded_text.trim().is_empty() && attachments.is_empty() {
                        return Err(PluginError::invalid_params(
                            "A Cordis subagent message requires non-empty text or media",
                        ));
                    }
                    (decoded_text, attachments)
                } else {
                    let text = text.unwrap_or_default();
                    if text.trim().is_empty() {
                        return Err(PluginError::invalid_params(
                            "A Cordis subagent message requires non-empty text or media",
                        ));
                    }
                    (text, Vec::new())
                };
                Ok(CordisSubagentControl::Message {
                    target_id,
                    text,
                    attachments,
                })
            }
            Self::Interrupt { target_id } => Ok(CordisSubagentControl::Interrupt { target_id }),
            Self::Wait { timeout_ms } => Ok(CordisSubagentControl::Wait { timeout_ms }),
            Self::WaitForTreeWork => Ok(CordisSubagentControl::WaitForTreeWork),
            Self::ModelRoutes => Ok(CordisSubagentControl::ModelRoutes),
        }
    }
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AgentOptions {
    provider: Option<String>,
    model: Option<String>,
    reasoning_effort: Option<String>,
    max_tokens: Option<u32>,
}

fn valid_child_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id != "lead"
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b':'))
}

impl Request {
    fn prepare(
        self,
        config: &CordisConfig,
    ) -> Result<(CordisSubagentStart, Option<CompiledSchema>)> {
        let provider = config
            .resolve_provider(Some(&self.provider))
            .map_err(PluginError::invalid_params)?;
        if self.max_depth == Some(0) {
            return Err(PluginError::invalid_params("Cordis child exceeds maxDepth"));
        }
        if provider == SubagentProvider::Fork && self.agent_preset.is_some() {
            return Err(PluginError::invalid_params(
                "The Cordis fork provider inherits its parent's context and cannot select agentPreset",
            ));
        }
        if let Some(child_id) = self.child_id.as_deref()
            && !valid_child_id(child_id)
        {
            return Err(PluginError::invalid_params(
                "Cordis childId must be 1–128 ASCII identifier characters and differ from lead",
            ));
        }
        if self.prompt.is_empty() || self.prompt.len() > 256 {
            return Err(PluginError::invalid_params(
                "A Cordis child requires 1–256 prompt blocks",
            ));
        }
        let (mut prompt, attachments) =
            super::tools::decode_content_blocks(&self.prompt, "Cordis subagent prompt")?;
        if prompt.len() > MAX_PROMPT_BYTES {
            return Err(PluginError::invalid_params(
                "Cordis child prompt exceeds 128 KiB",
            ));
        }
        if prompt.trim().is_empty() && attachments.is_empty() {
            return Err(PluginError::invalid_params(
                "A Cordis child requires a non-empty prompt or media attachment",
            ));
        }
        let validator = self
            .output_schema
            .as_ref()
            .map(|schema| {
                let source = schema.to_string();
                if schema.get("type").and_then(Value::as_str) != Some("object")
                    || source.len() > MAX_SCHEMA_BYTES
                {
                    return Err(PluginError::invalid_params(
                        "Cordis outputSchema must be an object-rooted JSON Schema within 64 KiB",
                    ));
                }
                let validator = CompiledSchema::new(schema).map_err(PluginError::invalid_params)?;
                prompt.push_str("\n\nReturn one JSON object matching this schema:\n");
                prompt.push_str(&source);
                Ok(validator)
            })
            .transpose()?;
        let options = self.agent_options.unwrap_or_default();
        let start = CordisSubagentStart {
            provider: self.provider,
            request: AgentSpawnRequest {
                name: self.name.unwrap_or_default(),
                prompt,
                profile: self.agent_preset,
                commands: Vec::new(),
                selection: Some(AgentModelSelection {
                    provider: options.provider,
                    model: options.model,
                    thinking_mode: options.reasoning_effort,
                    ..Default::default()
                }),
                timeout_ms: None,
                max_tokens: None,
                max_cost_microusd: None,
                inherit_parent_context: provider == SubagentProvider::Fork,
            },
            label: self.label,
            reserved_id: self.child_id,
            attachments,
            cwd: self.cwd,
            max_output_tokens: options.max_tokens,
            max_active_children: config.subagent_max_active,
            max_depth: self.max_depth,
            delivery: self.delivery,
        };
        Ok((start, validator))
    }
}

#[derive(Default)]
struct Status {
    active: BTreeMap<Uuid, Arc<dyn CordisSubagentRun>>,
    completed: Vec<Completion>,
}

pub(super) struct Runs {
    status: Mutex<Status>,
    changed: Notify,
    lifetime: CancellationToken,
}

impl Runs {
    pub(super) fn new(lifetime: CancellationToken) -> Self {
        Self {
            status: Mutex::new(Status::default()),
            changed: Notify::new(),
            lifetime,
        }
    }

    pub(super) fn take_completed(&self) -> Vec<Completion> {
        std::mem::take(
            &mut self
                .status
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .completed,
        )
    }

    pub(super) fn has_pending(&self) -> bool {
        let state = self
            .status
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        !state.active.is_empty() || !state.completed.is_empty()
    }

    pub(super) async fn ready(&self) {
        loop {
            let mut changed = std::pin::pin!(self.changed.notified());
            changed.as_mut().enable();
            if self.lifetime.is_cancelled()
                || !self
                    .status
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .completed
                    .is_empty()
            {
                return;
            }
            tokio::select! {
                _ = self.lifetime.cancelled() => return,
                _ = changed => {},
            }
        }
    }

    pub(super) async fn start(
        self: &Arc<Self>,
        host: &dyn HostClient,
        request: Request,
        config: &CordisConfig,
        cancellation: CancellationToken,
        events: Option<SubagentEvents>,
    ) -> Result<Value> {
        let run_id = request.run_id;
        let provider = request.provider.clone();
        let parent_id = request.parent_id.clone();
        let (start, validator) = request.prepare(config)?;
        let launch = host.start_cordis_subagent(start, cancellation.clone());
        tokio::pin!(launch);
        let started = tokio::select! {
            biased;
            _ = self.lifetime.cancelled() => { cancellation.cancel(); launch.await? },
            started = &mut launch => started?,
        };
        let visible_id = started.visible_child_id();
        let ancestor_sessions = started.ancestor_sessions.clone();
        let observation = events.map(|events| {
            let parent_session_id = parent_id.unwrap_or_else(|| events.session.to_string());
            let info = json!({
                "runId": run_id, "provider": provider, "id": visible_id,
                "parentSessionId": parent_session_id, "local": true,
            });
            events.emit_tree(&ancestor_sessions, "subagent/start", info.clone());
            (events, ancestor_sessions, info)
        });
        let run = started.run;
        self.status
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .active
            .insert(run_id, run.clone());
        let runs = self.clone();
        tokio::spawn(async move {
            let outcome = tokio::select! {
                biased;
                // The native session owns an accepted attempt. Reload of the
                // JS module ends observation without retracting the launch.
                _ = runs.lifetime.cancelled() => return,
                outcome = run.result() => outcome,
            };
            let result = outcome.map(|result| Output::capture(result, validator.as_ref()));
            if let Some((events, ancestor_sessions, mut info)) = observation {
                info["stopReason"] = json!(
                    result
                        .as_ref()
                        .map_or(CordisSubagentStopReason::Error, |value| value.stop_reason,)
                );
                if let Ok(value) = &result
                    && !value.output.is_empty()
                {
                    info["lastAssistantMessage"] = json!(value.output);
                }
                events.emit_tree(&ancestor_sessions, "subagent/end", info);
            }
            let mut status = runs
                .status
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            status.active.remove(&run_id);
            status.completed.push(Completion {
                run_id,
                value: result.as_ref().ok().map(|output| json!(output)),
                error: result.err().map(|error| error.failure.user.fallback),
            });
            drop(status);
            runs.changed.notify_waiters();
        });
        Ok(json!({
            "childId": visible_id, "messageId": started.message_id.to_string(),
            "name": started.name, "provider": provider, "runId": run_id,
        }))
    }

    pub(super) async fn dispose(&self, run_id: Uuid) -> Result<Value> {
        let run = self
            .status
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .active
            .get(&run_id)
            .cloned();
        if let Some(run) = run {
            run.dispose().await?;
        }
        Ok(Value::Null)
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Dispose {
    pub run_id: Uuid,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Completion {
    run_id: Uuid,
    #[serde(skip_serializing_if = "Option::is_none")]
    value: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Output {
    stop_reason: CordisSubagentStopReason,
    output: Vec<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    structured: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    diagnostic: Option<&'static str>,
}

impl Output {
    fn capture(result: CordisSubagentResult, validator: Option<&CompiledSchema>) -> Self {
        let mut stop_reason = result.stop_reason;
        let mut diagnostic = (stop_reason == CordisSubagentStopReason::Error)
            .then_some("The child execution did not complete");
        let structured = validator
            .filter(|_| stop_reason == CordisSubagentStopReason::Completed)
            .and_then(|validator| {
                let value = result
                    .text
                    .as_deref()
                    .and_then(|text| serde_json::from_str::<Value>(text).ok());
                match value {
                    Some(value) if validator.validate(&value).is_ok() => Some(value),
                    _ => {
                        stop_reason = CordisSubagentStopReason::Error;
                        diagnostic =
                            Some("The child did not return an object matching outputSchema");
                        None
                    }
                }
            });
        Self {
            stop_reason,
            output: result
                .text
                .into_iter()
                .map(|text| json!({ "type": "text", "text": text }))
                .collect(),
            structured,
            diagnostic,
        }
    }
}

pub(super) fn unavailable() -> PluginError {
    PluginError::from_kind(
        PluginErrorKind::PolicyDenied,
        "Cordis subagents require a live project command or tool",
    )
}
