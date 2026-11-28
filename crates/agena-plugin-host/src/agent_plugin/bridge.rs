//! Runs imported plugin tasks through the owning Agena project session.

use std::collections::BTreeSet;

use agena_plugin_sdk::{
    ApplicableSessions, HostClient, PluginError, PluginKey, Result, SessionKind, ToolContract,
    ToolDefinition, ToolDocs, ToolInvokeInput, ToolInvokeOutput, ToolTag,
};
use serde::Deserialize;
use serde_json::{Value, json};

use super::AgentPluginDescriptor;

pub(super) const TOOL_NAME: &str = "run";
const MAX_PROMPT_BYTES: usize = 64 * 1024;
pub(super) const AGENT_TIMEOUT_MS: u64 = 180_000;

#[derive(Clone)]
pub(super) struct AgentBridge {
    id: PluginKey,
    package_name: String,
    agents: Vec<String>,
    background_agents: BTreeSet<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RunInput {
    prompt: String,
    agent: Option<String>,
    #[serde(default)]
    background: Option<bool>,
}

pub(super) struct AgentTask {
    pub name: String,
    input: Value,
}

impl AgentTask {
    pub(super) fn with_parent_context(mut self, inherit: bool) -> Self {
        if inherit {
            self.input["inherit_parent_context"] = json!(true);
        }
        self
    }

    pub(super) fn with_model(mut self, provider: Option<String>, model: Option<String>) -> Self {
        if provider.is_some() || model.is_some() {
            self.input["selection"] = json!({"provider": provider, "model": model});
        }
        self
    }

    pub(super) fn into_spawn_request(self) -> Result<agena_plugin_sdk::host_api::AgentSpawnRequest> {
        serde_json::from_value(self.input).map_err(|error| PluginError::internal_error(&error))
    }

    pub(super) async fn spawn(self, host: &dyn HostClient) -> Result<i64> {
        let result = host
            .invoke_tool("agena.agent.spawn".into(), self.input)
            .await?;
        result
            .payload
            .as_ref()
            .and_then(|payload| payload.get("agent_id"))
            .and_then(Value::as_i64)
            .ok_or_else(|| PluginError::internal("Agena agent launch returned no child ID"))
    }
}

impl AgentBridge {
    pub(super) fn new(id: &PluginKey, package: &AgentPluginDescriptor) -> Self {
        Self {
            id: id.clone(),
            package_name: package.name.clone(),
            agents: package
                .roles
                .iter()
                .filter(|role| !role.output_style)
                .map(|role| role.name.clone())
                .collect(),
            background_agents: package
                .roles
                .iter()
                .filter(|role| !role.output_style && role.claude_options.background)
                .map(|role| role.name.clone())
                .collect(),
        }
    }

    pub(super) fn tool(&self) -> ToolDefinition {
        let mut schema = json!({
            "type": "object",
            "properties": {
                "prompt": {"type": "string", "minLength": 1, "maxLength": MAX_PROMPT_BYTES},
                "background": {
                    "type": "boolean",
                    "description": "Run the delegated agent in the background and return its handle immediately instead of waiting for completion."
                }
            },
            "required": ["prompt"],
            "additionalProperties": false
        });
        if !self.agents.is_empty() {
            schema["properties"]["agent"] = json!({
                "type": "string",
                "enum": self.agents,
                "description": "The name of an agent defined by the imported plugin."
            });
        }
        ToolDefinition {
            name: TOOL_NAME.into(),
            contract: ToolContract { input_schema: schema, output_schema: Value::Null },
            docs: ToolDocs {
                summary: Some(format!("Run an Agena task with the {} plugin.", self.package_name)),
                help: Some("Delegates in this project with the current model and permissions. You can select an imported role when the plugin defines agents.".into()),
                ..Default::default()
            },
            runtime: Default::default(),
            tags: vec![ToolTag::Agent, ToolTag::Execute, ToolTag::Mutate],
            applies_to: ApplicableSessions::only(SessionKind::Project),
        }
    }

    pub(super) async fn run(
        &self,
        host: &dyn HostClient,
        input: ToolInvokeInput,
    ) -> Result<ToolInvokeOutput> {
        let context = input.context();
        if context.is_global() || context.session_id <= 0 {
            return Err(PluginError::invalid_params(
                "an imported plugin task requires a project session",
            ));
        }
        let call = input.call_id;
        let request: RunInput = serde_json::from_value(input.input).map_err(|_| {
            PluginError::invalid_params("expected a prompt and an optional plugin agent name")
        })?;
        let background = request
            .background
            .unwrap_or_else(|| self.role_is_background(request.agent.as_deref()));
        let task = self.prepare_task(Some(call), &request.prompt, request.agent.as_deref())?;
        let task_name = task.name.clone();
        let child = Box::pin(task.spawn(host)).await?;
        if background {
            let payload = json!({
                "agent_id": child,
                "name": task_name,
                "status": "running",
                "background": true,
            });
            let mut output = ToolInvokeOutput::text(format!(
                "Started background agent {task_name} (session {child}). Completion will be delivered to this session when it finishes."
            ));
            output.summary = format!("Started background agent {task_name}");
            output.payload = Some(payload);
            return Ok(output);
        }
        host.wait_imported_plugin_agent(child, AGENT_TIMEOUT_MS)
            .await
    }

    fn role_is_background(&self, agent: Option<&str>) -> bool {
        let Some(agent) = agent else { return false };
        let local = agent
            .strip_prefix(&format!("{}:", self.package_name))
            .unwrap_or(agent);
        let name = local.replace(':', ".").to_ascii_lowercase();
        self.background_agents.contains(&name)
    }

    pub(super) fn prepare_task(
        &self,
        call: Option<i64>,
        prompt: &str,
        agent: Option<&str>,
    ) -> Result<AgentTask> {
        let prompt = prompt.trim();
        if prompt.is_empty() || prompt.len() > MAX_PROMPT_BYTES {
            return Err(PluginError::invalid_params(
                "prompt must contain 1–65536 bytes",
            ));
        }
        let profile = self.resolve_profile(agent)?;
        let name = format!(
            "plugin_{}_{}",
            call.unwrap_or_default().unsigned_abs(),
            uuid::Uuid::new_v4().simple(),
        );
        let task = format!(
            "Task for the imported Agena plugin {} ({}).\n\n{prompt}",
            self.id, self.package_name
        );
        let input = json!({
            "name": name,
            "prompt": task,
            "profile": profile,
            "timeout_ms": AGENT_TIMEOUT_MS,
        });
        Ok(AgentTask { name, input })
    }

    fn resolve_profile(&self, agent: Option<&str>) -> Result<Option<String>> {
        let Some(agent) = agent else { return Ok(None) };
        let local = agent
            .strip_prefix(&format!("{}:", self.package_name))
            .unwrap_or(agent);
        let name = local.replace(':', ".").to_ascii_lowercase();
        if self.agents.contains(&name) {
            return Ok(Some(format!("{}.{name}", self.id)));
        }
        match agent {
            "general-purpose" => Ok(None),
            "Explore" | "Plan" => Ok(Some("explore".into())),
            _ => Err(PluginError::invalid_params(format!(
                "unknown imported agent {agent:?}"
            ))),
        }
    }
}
