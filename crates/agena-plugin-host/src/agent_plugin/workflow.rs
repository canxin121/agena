//! Run imported Claude and Cordis workflows with Agena delegation.

use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use agena_domain::{ActivityOutputRetention, MAX_ACTIVITY_OUTPUT_BYTES};
use agena_plugin_sdk::{
    ApplicableSessions, HostClient, PluginError, PluginErrorKind, SessionKind, ToolContract,
    ToolDefinition, ToolDocs, ToolInvokeInput, ToolInvokeOutput, ToolTag,
    host_api::{
        CordisWorkflowJobHost, CordisWorkflowResult, CordisWorkflowRunner, CordisWorkflowSettlement,
        CordisWorkflowStart, CordisWorkflowStopReason,
    },
};
use rquickjs::{
    AsyncContext, AsyncRuntime, CatchResultExt, Promise,
    function::{Async, Func},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::{Mutex, Semaphore};
use tokio_util::sync::CancellationToken;

mod checkpoint;
pub(super) use checkpoint::ResumeFrom;
use checkpoint::{Checkpoint, Replayed};

use super::{
    MAX_COMPONENT_ENTRIES,
    bridge::{AGENT_TIMEOUT_MS, AgentBridge, AgentTask},
    deepseek_commands::{
        WorkflowCaller,
        events::{AgentEvents, RunEvents},
    },
    manifest_path, read_package_bytes,
};

pub(super) const TOOL_NAME: &str = "workflow";
const SCRIPT_BYTES: u64 = 256 * 1024;
const WORKFLOW_TIMEOUT: Duration = Duration::from_secs(285);
const MAX_WORKFLOW_ITEMS: usize = 4096;
const PRELUDE: &str = r#"
let __agenaCurrentPhase;
const __agenaIsCordis = __agenaCordis;
const __agenaFatal = message => Object.assign(new Error(message), {workflowFatal: true});
const agent = async (prompt, options = {}) => {
  if (__agenaIsCordis) {
    if (typeof prompt !== "string" || !prompt.length) throw __agenaFatal("agent() requires a non-empty prompt string");
    if (options === null || typeof options !== "object" || Array.isArray(options)) {
      throw __agenaFatal("agent() options must be an object");
    }
    if (__agenaCurrentPhase !== undefined && options.phase === undefined) options = {...options, phase: __agenaCurrentPhase};
  }
  const result = JSON.parse(await __agenaAgent(prompt, JSON.stringify(options)));
  if ("error" in result) throw __agenaFatal(result.error);
  return result.value;
};
const batch = (limit => items => {
  if (!Array.isArray(items) || items.length > limit) {
    throw __agenaFatal(`Expected an array of at most ${limit} items.`);
  }
  return items;
})(__agenaMaxItems);
const phase = title => {
  if (__agenaIsCordis) {
    if (typeof title !== "string" || !title.length) throw __agenaFatal("phase() requires a non-empty title string");
    __agenaCurrentPhase = title;
  }
  __agenaLog("phase", String(title));
};
const log = message => {
  if (__agenaIsCordis && typeof message !== "string") throw __agenaFatal("log() requires a message string");
  __agenaLog("log", String(message));
};
const originalDate = Date;
globalThis.Date = class extends originalDate {
  constructor(...values) {
    if (!values.length) throw new Error("Pass dates through the workflow args.");
    super(...values);
  }
  static now() { throw new Error("Pass timestamps through the workflow args."); }
};
Math.random = () => { throw new Error("Pass random values through the workflow args."); };
"#;
const CLAUDE_COMBINATORS: &str = r#"
const parallel = tasks => Promise.all(batch(tasks).map(task => typeof task === "function" ? task() : task));
const pipeline = (items, ...stages) => parallel(batch(items).map((item, index) => async () => {
  for (const stage of stages) {
    if (item === null) return null;
    item = await stage(item, index);
  }
  return item;
}));
"#;
const CORDIS_COMBINATORS: &str = r#"
const isolated = async task => {
    try { return await task(); }
    catch (error) { if (error?.workflowFatal === true) throw error; return null; }
};
const parallel = tasks => {
    const thunks = batch(tasks);
    if (thunks.some(task => typeof task !== 'function')) throw __agenaFatal('parallel expects functions');
    return Promise.all(thunks.map(isolated));
};
const pipeline = (items, ...stages) => {
    if (!stages.length || stages.some(stage => typeof stage !== 'function')) throw __agenaFatal('pipeline requires stage functions');
    return parallel(batch(items).map((item, index) => async () => {
        let value = item;
        for (const stage of stages) {
            value = await stage(value, item, index);
        }
        return value;
    }));
};
"#;

#[derive(Clone, Deserialize, Serialize)]
pub(super) struct Metadata {
    pub name: String,
    pub description: String,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Clone)]
pub(super) struct Workflow {
    pub meta: Metadata,
    pub source: String,
    script: String,
    active_sessions: Arc<dashmap::DashSet<i64>>,
}

struct WorkflowRun<'a> {
    sessions: &'a dashmap::DashSet<i64>,
    session: i64,
}

impl Drop for WorkflowRun<'_> {
    fn drop(&mut self) {
        self.sessions.remove(&self.session);
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RunWorkflow {
    name: String,
    args: Option<Value>,
    #[serde(default)]
    resume: bool,
    #[serde(rename = "resumeFromRunId", alias = "resume_from_run_id")]
    resume_from_run_id: Option<String>,
    #[serde(rename = "title")]
    _title: Option<String>,
    #[serde(rename = "description")]
    _description: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AgentOptions {
    #[serde(
        alias = "agentType",
        alias = "agent_type",
        alias = "subagent_type",
        alias = "type"
    )]
    agent: Option<String>,
    schema: Option<Value>,
    label: Option<String>,
    #[serde(rename = "phase")]
    phase: Option<String>,
    provider: Option<String>,
    model: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct CordisRequest {
    pub id: uuid::Uuid,
    pub meta: Metadata,
    pub script: String,
    pub args: Option<Value>,
    pub subagent_provider: Option<String>,
    pub max_total_agents: Option<usize>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct CordisBackgroundRequest {
    pub workflow: CordisRequest,
    pub max_result_chars: usize,
}

#[derive(Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct CordisJobsConfig {
    pub inline_output_bytes: usize,
    pub completion_delivery: CordisCompletionDelivery,
    pub max_consecutive_wakes: Option<usize>,
    pub wait_timeout_ms: u64,
    pub max_wait_timeout_ms: u64,
    #[serde(skip)]
    pub max_concurrent_per_owner: usize,
    #[serde(skip)]
    pub pump_poll_ms: u64,
    #[serde(skip)]
    pub output_retention: ActivityOutputRetention,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum CordisCompletionDelivery {
    Quiet,
    Wakeup,
}

impl Default for CordisJobsConfig {
    fn default() -> Self {
        Self {
            inline_output_bytes: 2_048,
            completion_delivery: CordisCompletionDelivery::Wakeup,
            max_consecutive_wakes: None,
            wait_timeout_ms: 30_000,
            max_wait_timeout_ms: 600_000,
            max_concurrent_per_owner: 10,
            pump_poll_ms: 150,
            output_retention: ActivityOutputRetention {
                live_bytes: 256 * 1024,
                settled_bytes: 16 * 1024,
            },
        }
    }
}

impl CordisJobsConfig {
    pub(super) fn discover(rows: &[(String, serde_yaml::Value)]) -> Result<Self, String> {
        let mut declared = rows.iter().filter(|(_, row)| {
            super::deepseek::module_name(row) == Some(super::deepseek_commands::JOBS_TOOL)
        });
        let mut config = declared
            .next()
            .and_then(|(_, row)| row.get("config"))
            .filter(|config| !config.is_null())
            .map(|config| serde_yaml::from_value::<Self>(config.clone()))
            .transpose()
            .map_err(|error| format!("invalid Cordis jobs configuration: {error}"))?
            .unwrap_or_default();
        if declared.next().is_some() {
            return Err("only one Cordis job controller can be configured".into());
        }
        if config.wait_timeout_ms == 0
            || config.max_wait_timeout_ms == 0
            || config.max_consecutive_wakes == Some(0)
        {
            return Err("Cordis job wait times and wake limits must be positive".into());
        }
        for (_, row) in rows.iter().filter(|(_, row)| {
            super::deepseek::module_name(row) == Some(super::deepseek_commands::JOBS_LOCAL)
        }) {
            let Some(local) = row.get("config").filter(|value| !value.is_null()) else {
                continue;
            };
            if let Some(value) = local.get("maxConcurrentJobsPerOwner") {
                config.max_concurrent_per_owner = value.as_u64()
                    .and_then(|value| usize::try_from(value).ok())
                    .filter(|value| *value > 0)
                    .ok_or("Cordis maxConcurrentJobsPerOwner must be a positive integer")?;
            }
            if let Some(value) = local.get("pumpPollMs") {
                config.pump_poll_ms = value.as_u64()
                    .filter(|value| *value > 0)
                    .ok_or("Cordis pumpPollMs must be a positive integer")?;
            }
            for (key, limit) in [
                ("retainBytes", &mut config.output_retention.live_bytes),
                ("settledRetainBytes", &mut config.output_retention.settled_bytes),
            ] {
                if let Some(value) = local.get(key) {
                    *limit = value.as_u64()
                        .filter(|value| *value > 0)
                        .map(|value| value.min(MAX_ACTIVITY_OUTPUT_BYTES as u64) as usize)
                        .ok_or_else(|| format!("Cordis {key} must be a positive integer"))?;
                }
            }
        }
        Ok(config)
    }
}

#[derive(Clone, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct CordisConfig {
    provider: String,
    max_concurrent_agents: usize,
    pub max_total_agents: usize,
    max_items_per_call: usize,
    sync_timeout_ms: u64,
    #[serde(skip)]
    pub subagent_max_depth: u64,
    #[serde(skip)]
    pub subagent_max_active: u64,
    #[serde(skip)]
    providers: BTreeMap<String, SubagentProvider>,
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum SubagentProvider {
    Spawn,
    Fork,
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
struct SubagentProviderConfig {
    provider_name: Option<String>,
}

impl Default for CordisConfig {
    fn default() -> Self {
        Self {
            provider: "spawn".into(),
            max_concurrent_agents: 0,
            max_total_agents: checkpoint::MAX_CALLS,
            max_items_per_call: MAX_WORKFLOW_ITEMS,
            sync_timeout_ms: 5_000,
            subagent_max_depth: 1,
            subagent_max_active: 8,
            providers: BTreeMap::new(),
        }
    }
}

impl CordisConfig {
    pub(super) fn discover(rows: &[(String, serde_yaml::Value)]) -> Result<Self, String> {
        let mut declared = None;
        for (_, row) in rows.iter().filter(|(_, row)| {
            super::deepseek::module_name(row) == Some(super::deepseek_commands::WORKFLOW_PTC)
        }) {
            if declared.is_some() {
                return Err("only one Cordis workflow engine can be configured".into());
            }
            let config = match row.get("config").filter(|config| !config.is_null()) {
                Some(config) => {
                    serde_yaml::from_value::<Self>(config.clone()).map_err(|error| {
                        format!("invalid Cordis workflow engine configuration: {error}")
                    })?
                }
                None => Self::default(),
            };
            if !(1..=checkpoint::MAX_CALLS).contains(&config.max_total_agents)
                || config.max_concurrent_agents > checkpoint::MAX_CALLS
                || !(1..=MAX_WORKFLOW_ITEMS).contains(&config.max_items_per_call)
                || config.sync_timeout_ms == 0
                || Duration::from_millis(config.sync_timeout_ms) > WORKFLOW_TIMEOUT
            {
                return Err("Cordis workflow engine limits exceed Agena's workflow limits".into());
            }
            declared = Some(config);
        }
        let mut config = declared.unwrap_or_default();
        for (_, row) in rows.iter().filter(|(_, row)| {
            super::deepseek::module_name(row) == Some(super::deepseek_commands::SUBAGENT)
                && !row.get("disabled").is_some_and(|value| !matches!(value, serde_yaml::Value::Null | serde_yaml::Value::Bool(false)))
        }) {
            if let Some(settings) = row.get("config").filter(|config| !config.is_null()) {
                if let Some(depth) = settings.get("maxDepth") {
                    config.subagent_max_depth = depth.as_u64().filter(|depth| *depth <= 9_007_199_254_740_991)
                        .ok_or("Cordis subagent maxDepth must be a non-negative safe integer")?;
                }
                if let Some(active) = settings.get("maxActiveSubagents") {
                    config.subagent_max_active = active
                        .as_u64()
                        .filter(|active| (1..=9_007_199_254_740_991).contains(active))
                        .ok_or("Cordis maxActiveSubagents must be a positive safe integer")?;
                }
            }
        }
        for (kind, module, base_id, default_name) in [
            (
                SubagentProvider::Spawn,
                super::deepseek_commands::SUBAGENT_SPAWN,
                "subagent-spawn-in-process",
                "spawn",
            ),
            (
                SubagentProvider::Fork,
                super::deepseek_commands::SUBAGENT_FORK,
                "subagent-fork-in-process",
                "fork",
            ),
        ] {
            let mut use_base_provider = true;
            for (id, row) in rows
                .iter()
                .filter(|(_, row)| super::deepseek::module_name(row) == Some(module))
            {
                use_base_provider &=
                    row.get("id").and_then(serde_yaml::Value::as_str) != Some(base_id);
                if row.get("disabled").is_some_and(|value| {
                    !matches!(value, serde_yaml::Value::Null | serde_yaml::Value::Bool(false))
                }) {
                    continue;
                }
                let declaration = row
                    .get("config")
                    .filter(|config| !config.is_null())
                    .map(|config| serde_yaml::from_value::<SubagentProviderConfig>(config.clone()))
                    .transpose()
                    .map_err(|error| {
                        format!("invalid Cordis subagent provider configuration for {id:?}: {error}")
                    })?
                    .unwrap_or_default();
                config.register_provider(
                    declaration.provider_name.as_deref().unwrap_or(default_name),
                    kind,
                )?;
            }
            if use_base_provider && config.providers.get(default_name) != Some(&kind) {
                config.register_provider(default_name, kind)?;
            }
        }
        Ok(config)
    }

    fn register_provider(&mut self, name: &str, kind: SubagentProvider) -> Result<(), String> {
        if name.is_empty() || name.len() > 128 || name != name.trim() {
            return Err(
                "Cordis subagent providerName must be a non-empty normalized string".into(),
            );
        }
        if self.providers.insert(name.to_owned(), kind).is_some() {
            return Err(format!("duplicate Cordis subagent provider {name:?}"));
        }
        Ok(())
    }

    pub(super) fn resolve_provider(&self, override_name: Option<&str>) -> Result<SubagentProvider, String> {
        let name = override_name.unwrap_or(&self.provider);
        self.providers
            .get(name)
            .copied()
            .ok_or_else(|| format!("Cordis workflow subagent provider {name:?} is unavailable"))
    }

    pub(super) fn provider_names(&self) -> Vec<&str> {
        self.providers.keys().map(String::as_str).collect()
    }

    pub(super) fn providers(&self) -> impl Iterator<Item = (&str, SubagentProvider)> {
        self.providers.iter().map(|(name, provider)| (name.as_str(), *provider))
    }

    fn concurrency(&self) -> usize {
        if self.max_concurrent_agents != 0 {
            return self.max_concurrent_agents;
        }
        std::thread::available_parallelism()
            .map_or(1, usize::from)
            .saturating_sub(2)
            .clamp(1, 16)
    }
}

pub(super) async fn execute_cordis(
    request: CordisRequest,
    config: &CordisConfig,
    caller: &WorkflowCaller,
    cancellation: CancellationToken,
    events: RunEvents,
) -> agena_plugin_sdk::Result<Value> {
    let run_id = request.id;
    let (workflow, cordis, args) = prepare_cordis(request, config)?;
    let observation = events.start_foreground();
    let result = async {
        if cancellation.is_cancelled() {
            return Err(PluginError::from_kind(
                PluginErrorKind::Cancelled,
                "workflow cancelled",
            ));
        }
        let scope = ExecutionScope {
            session: caller.session,
            call: Some(caller.call),
            resume: ResumeFrom::FreshWithId(run_id),
            cancellation: cancellation.clone(),
            cordis: Some(cordis),
            events: Some(events),
        };
        let output =
            execute_in_scope(&workflow, &caller.bridge, caller.host.clone(), args, scope).await?;
        let value = serde_json::from_str(&output.output_text)
            .map_err(|error| PluginError::internal_error(&error))?;
        let agents_started = output
            .metadata
            .get("agentsStarted")
            .and_then(|count| count.parse().ok())
            .unwrap_or_default();
        Ok(CordisWorkflowResult { value, agents_started })
    }
    .await;
    let (value, settlement) = match result {
        Ok(result) => (
            result.value,
            CordisWorkflowSettlement {
                reason: CordisWorkflowStopReason::Completed,
                error: None,
                agents_started: Some(result.agents_started),
            },
        ),
        Err(error) => (
            Value::Null,
            CordisWorkflowSettlement {
                reason: if cancellation.is_cancelled() || error.kind == PluginErrorKind::Cancelled {
                    CordisWorkflowStopReason::Cancelled
                } else {
                    CordisWorkflowStopReason::Error
                },
                error: Some(error.failure.user.fallback),
                agents_started: None,
            },
        ),
    };
    let mut result = observation.finish(settlement);
    result["value"] = value;
    Ok(result)
}

pub(super) async fn start_cordis(
    request: CordisBackgroundRequest,
    config: &CordisConfig,
    jobs: &CordisJobsConfig,
    bridge: AgentBridge,
    host: &dyn HostClient,
    call: i64,
    cancellation: CancellationToken,
    events: RunEvents,
    report_completion: bool,
) -> agena_plugin_sdk::Result<Value> {
    let run_id = request.workflow.id;
    let (workflow, cordis, args) = prepare_cordis(request.workflow, config)?;
    let terminal_events = events.clone();
    let start = CordisWorkflowStart {
        run_id,
        name: workflow.meta.name.clone(),
        report_completion,
        wake_on_completion: matches!(jobs.completion_delivery, CordisCompletionDelivery::Wakeup),
        max_consecutive_wakes: jobs.max_consecutive_wakes,
        inline_output_bytes: jobs.inline_output_bytes,
        max_result_chars: request.max_result_chars,
        output_retention: jobs.output_retention,
        on_settled: Box::new(move |settlement| {
            terminal_events.end(settlement);
        }),
    };
    let run: CordisWorkflowRunner = Box::new(move |host, cancel| {
        events.start();
        Box::pin(async move {
            let agents = WorkflowAgents::new(
                bridge,
                Some(call),
                Some(&cordis),
                Delegation::Background(host),
                Some(events),
            );
            let args = args.map(|value| value.to_string());
            let (output, agents_started) =
                run_script(&workflow, agents, args, Some(&cordis), cancel).await?;
            let value = serde_json::from_str(&output)
                .map_err(|error| PluginError::internal_error(&error))?;
            Ok(CordisWorkflowResult {
                value,
                agents_started,
            })
        })
    });
    let started = host.start_cordis_workflow(start, run, cancellation).await?;
    Ok(json!({"kind": "background", "jobId": started.job_id, "runId": started.run_id}))
}

fn prepare_cordis(
    request: CordisRequest,
    config: &CordisConfig,
) -> agena_plugin_sdk::Result<(Workflow, CordisExecution, Option<Value>)> {
    validate_metadata(&request.meta).map_err(PluginError::invalid_params)?;
    if request.script.trim().is_empty() || request.script.len() as u64 > SCRIPT_BYTES {
        return Err(PluginError::invalid_params(
            "Cordis workflow script exceeds its byte budget or is empty",
        ));
    }
    let provider = config
        .resolve_provider(request.subagent_provider.as_deref())
        .map_err(PluginError::invalid_params)?;
    let max_agents = request.max_total_agents.unwrap_or(config.max_total_agents);
    if !(1..=config.max_total_agents).contains(&max_agents) {
        return Err(PluginError::invalid_params(
            "maxTotalAgents must be within the configured workflow limit",
        ));
    }
    let workflow = Workflow {
        meta: request.meta,
        source: "Cordis workflow tool".into(),
        script: request.script,
        active_sessions: Default::default(),
    };
    let execution = CordisExecution {
        config: config.clone(),
        provider,
        max_agents,
    };
    Ok((workflow, execution, request.args))
}

pub(super) fn discover(
    root: &Path,
    declaration: Option<&Value>,
    watch: &mut BTreeSet<PathBuf>,
    diagnostics: &mut Vec<String>,
) -> BTreeMap<String, Workflow> {
    let default = Value::String("./workflows".to_owned());
    let paths = declaration.unwrap_or(&default);
    let paths = paths
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_else(|| std::slice::from_ref(paths));
    let mut files = BTreeSet::new();
    for relative in paths {
        let result = relative
            .as_str()
            .ok_or_else(|| "workflows require local paths".to_owned())
            .and_then(|path| manifest_path(root, path));
        let path = match result {
            Ok(path) => path,
            Err(error) => {
                diagnostics.push(error);
                continue;
            }
        };
        watch.insert(path.clone());
        if path.is_dir() {
            match std::fs::read_dir(&path) {
                Ok(entries) => files.extend(
                    entries
                        .take(MAX_COMPONENT_ENTRIES)
                        .filter_map(Result::ok)
                        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
                        .map(|entry| entry.path())
                        .filter(|path| path.extension().is_some_and(|ext| ext == "js")),
                ),
                Err(error) => {
                    diagnostics.push(format!("workflow directory {}: {error}", path.display()))
                }
            }
        } else if path.extension().is_some_and(|extension| extension == "js") {
            files.insert(path);
        } else if declaration.is_some() {
            diagnostics.push(format!(
                "workflow {} requires a .js file or directory",
                path.display()
            ));
        }
    }
    let mut workflows = BTreeMap::new();
    for file in files.into_iter().take(MAX_COMPONENT_ENTRIES) {
        watch.insert(file.clone());
        let relative = file
            .strip_prefix(root)
            .expect("workflow file is inside its plugin");
        let source = relative.to_string_lossy().into_owned();
        let parsed = (|| {
            let data =
                read_package_bytes(root, relative, SCRIPT_BYTES)?.ok_or("workflow is missing")?;
            let text = std::str::from_utf8(&data).map_err(|_| "workflow is not UTF-8")?;
            let (meta, script) = parse(text)?;
            Ok::<_, String>(Workflow {
                meta,
                script: script.to_owned(),
                source: source.clone(),
                active_sessions: Default::default(),
            })
        })();
        match parsed {
            Ok(workflow) if !workflows.contains_key(&workflow.meta.name) => {
                workflows.insert(workflow.meta.name.clone(), workflow);
            }
            Ok(workflow) => diagnostics.push(format!(
                "workflow {source}: duplicate name {:?}",
                workflow.meta.name
            )),
            Err(error) => diagnostics.push(format!("workflow {source}: {error}")),
        }
    }
    workflows
}

fn parse(text: &str) -> Result<(Metadata, &str), String> {
    static META: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"\A\s*export\s+const\s+meta\s*=\s*\{")
            .expect("valid workflow metadata regex")
    });
    let text = text.trim_start_matches('\u{feff}');
    let start = META
        .find(text)
        .ok_or("workflow must begin with export const meta = {...}")?
        .end()
        - 1;
    let end = metadata_end(&text[start..]).ok_or("workflow metadata is not closed")? + start;
    let meta: Metadata = json5::from_str(&text[start..end])
        .map_err(|error| format!("invalid literal workflow metadata: {error}"))?;
    validate_metadata(&meta)?;
    Ok((meta, text[end..].trim_start().trim_start_matches(';')))
}

fn validate_metadata(meta: &Metadata) -> Result<(), String> {
    if meta.name.is_empty()
        || meta.name.len() > 64
        || !meta
            .name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"-_".contains(&byte))
        || meta.description.trim().is_empty()
        || meta.description.len() > 1024
    {
        return Err("workflow requires a name and description".into());
    }
    Ok(())
}

fn metadata_end(text: &str) -> Option<usize> {
    let mut depth = 0;
    let mut quote = None;
    let mut bytes = text.bytes().enumerate().peekable();
    while let Some((index, byte)) = bytes.next() {
        if let Some(end) = quote {
            if byte == b'\\' {
                bytes.next();
            } else if byte == end {
                quote = None;
            }
            continue;
        }
        match byte {
            b'"' | b'\'' => quote = Some(byte),
            b'/' if bytes.peek().is_some_and(|(_, next)| *next == b'/') => {
                while bytes.next().is_some_and(|(_, byte)| byte != b'\n') {}
            }
            b'/' if bytes.peek().is_some_and(|(_, next)| *next == b'*') => {
                bytes.next();
                while let Some((_, next)) = bytes.next() {
                    if next == b'*' && bytes.next_if(|(_, next)| *next == b'/').is_some() {
                        break;
                    }
                }
            }
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(index + 1);
                }
            }
            _ => {}
        }
    }
    None
}

pub(super) fn tool(workflows: &BTreeMap<String, Workflow>) -> ToolDefinition {
    let help = workflows
        .values()
        .map(|workflow| format!("{} — {}", workflow.meta.name, workflow.meta.description))
        .collect::<Vec<_>>()
        .join("\n");
    ToolDefinition {
        name: TOOL_NAME.into(),
        contract: ToolContract {
            input_schema: json!({
                "type": "object",
                "required": ["name"],
                "additionalProperties": false,
                "properties": {
                    "name": {"type": "string", "enum": workflows.keys().collect::<Vec<_>>()},
                    "args": {"description": "Structured workflow input."},
                    "resume": {"type": "boolean", "description": "Replay the latest run of this workflow in the current session."},
                    "resumeFromRunId": {"type": "string", "description": "Replay the given workflow run in the current session."},
                    "title": {"type": "string", "description": "The workflow's metadata supplies its title."},
                    "description": {"type": "string", "description": "The workflow's metadata supplies its description."}
                }
            }),
            output_schema: Value::Null,
        },
        docs: ToolDocs {
            summary: Some("Run an imported Claude workflow with Agena agents.".into()),
            help: Some(help),
            ..Default::default()
        },
        runtime: Default::default(),
        tags: vec![ToolTag::Agent, ToolTag::Execute, ToolTag::Mutate],
        applies_to: ApplicableSessions::only(SessionKind::Project),
    }
}

pub(super) async fn invoke(
    workflows: &BTreeMap<String, Workflow>,
    bridge: &AgentBridge,
    host: Arc<dyn HostClient>,
    input: ToolInvokeInput,
) -> agena_plugin_sdk::Result<ToolInvokeOutput> {
    if input.context().is_global() || input.session_id <= 0 {
        return Err(PluginError::invalid_params(
            "workflows require a project session",
        ));
    }
    let request: RunWorkflow = serde_json::from_value(input.input.clone())
        .map_err(|error| PluginError::invalid_params_error(&error))?;
    let workflow = workflows
        .get(&request.name)
        .ok_or_else(|| PluginError::invalid_params("unknown imported workflow"))?;
    let resume = ResumeFrom::from_inputs(request.resume, request.resume_from_run_id.as_deref())?;
    execute(
        workflow,
        bridge,
        host,
        input.session_id,
        Some(input.call_id),
        request.args,
        resume,
    )
    .await
}

pub(super) fn command_args(raw: &str) -> agena_plugin_sdk::Result<(ResumeFrom, Option<Value>)> {
    let raw = raw.trim();
    let (flag, remaining) = raw.split_once(char::is_whitespace).unwrap_or((raw, ""));
    let (resume, args) = if flag == "--resume" {
        (ResumeFrom::Latest, remaining.trim())
    } else if let Some(id) = flag.strip_prefix("--resume=") {
        (ResumeFrom::from_inputs(false, Some(id))?, remaining.trim())
    } else {
        (ResumeFrom::Fresh, raw)
    };
    let args =
        (!args.is_empty()).then(|| serde_json::from_str(args).unwrap_or_else(|_| json!(args)));
    Ok((resume, args))
}

pub(super) async fn execute(
    workflow: &Workflow,
    bridge: &AgentBridge,
    host: Arc<dyn HostClient>,
    session: i64,
    call: Option<i64>,
    args: Option<Value>,
    resume: ResumeFrom,
) -> agena_plugin_sdk::Result<ToolInvokeOutput> {
    let scope = ExecutionScope {
        session,
        call,
        resume,
        cancellation: agena_plugin_sdk::current_cancellation_token().unwrap_or_default(),
        cordis: None,
        events: None,
    };
    execute_in_scope(workflow, bridge, host, args, scope).await
}

struct ExecutionScope {
    session: i64,
    call: Option<i64>,
    resume: ResumeFrom,
    cancellation: CancellationToken,
    cordis: Option<CordisExecution>,
    events: Option<RunEvents>,
}

struct CordisExecution {
    config: CordisConfig,
    provider: SubagentProvider,
    max_agents: usize,
}

async fn execute_in_scope(
    workflow: &Workflow,
    bridge: &AgentBridge,
    host: Arc<dyn HostClient>,
    args: Option<Value>,
    scope: ExecutionScope,
) -> agena_plugin_sdk::Result<ToolInvokeOutput> {
    let ExecutionScope {
        session,
        call,
        resume,
        cancellation,
        cordis,
        events,
    } = scope;
    if !workflow.active_sessions.insert(session) {
        return Err(PluginError::invalid_params(
            "this workflow is already running in this session",
        ));
    }
    let _run = WorkflowRun {
        sessions: &workflow.active_sessions,
        session,
    };
    let key = if let ResumeFrom::FreshWithId(id) = resume {
        format!("cordis-{}", id.simple())
    } else {
        workflow.meta.name.clone()
    };
    let checkpoint = Checkpoint::load(host.as_ref(), &key, args, resume).await?;
    checkpoint.register(host.as_ref()).await?;
    let run_id = checkpoint.run_id();
    let args = checkpoint.args();
    let checkpoint = Arc::new(Mutex::new(checkpoint));
    let launched = Arc::new(std::sync::Mutex::new(BTreeSet::new()));
    let agents = WorkflowAgents::new(
        bridge.clone(),
        call,
        cordis.as_ref(),
        Delegation::Live {
            host: host.clone(),
            checkpoint: checkpoint.clone(),
            launched: launched.clone(),
        },
        events,
    );
    let output = run_script(workflow, agents, args, cordis.as_ref(), cancellation).await;
    if cordis.is_some() {
        let children =
            std::mem::take(&mut *launched.lock().unwrap_or_else(|error| error.into_inner()));
        for child in children {
            let _ = host
                .invoke_tool(
                    "agena.agent.cancel".into(),
                    json!({"target": child.to_string()}),
                )
                .await;
        }
    }
    let (output, started) = output?;
    checkpoint.lock().await.finish(host.as_ref()).await?;
    let mut result = ToolInvokeOutput::text(output);
    result.metadata.insert("runId".into(), run_id.to_string());
    if cordis.is_some() {
        result
            .metadata
            .insert("agentsStarted".into(), started.to_string());
    }
    Ok(result)
}

enum Delegation {
    Live {
        host: Arc<dyn HostClient>,
        checkpoint: Arc<Mutex<Checkpoint>>,
        launched: Arc<std::sync::Mutex<BTreeSet<i64>>>,
    },
    Background(Arc<dyn CordisWorkflowJobHost>),
}

struct AgentProgress {
    seq: usize,
    label: String,
    phase: Option<String>,
}

struct WorkflowAgents {
    bridge: AgentBridge,
    call: Option<i64>,
    delegation: Delegation,
    events: Option<RunEvents>,
    max_agents: Option<usize>,
    inherit_parent_context: bool,
    concurrency: Option<Semaphore>,
    started: AtomicUsize,
    calls: AtomicUsize,
}

impl WorkflowAgents {
    fn new(
        bridge: AgentBridge,
        call: Option<i64>,
        cordis: Option<&CordisExecution>,
        delegation: Delegation,
        events: Option<RunEvents>,
    ) -> Self {
        Self {
            bridge,
            call,
            delegation,
            events,
            max_agents: cordis.map(|run| run.max_agents),
            inherit_parent_context: cordis
                .is_some_and(|run| run.provider == SubagentProvider::Fork),
            concurrency: cordis.map(|run| Semaphore::new(run.config.concurrency())),
            started: AtomicUsize::new(0),
            calls: AtomicUsize::new(0),
        }
    }

    fn progress(&self, kind: &str, message: &str) {
        if let Delegation::Background(host) = &self.delegation {
            host.progress(&format!("{kind}: {message}"));
        }
        if let Some(events) = &self.events {
            events.narration(kind, message);
        }
    }

    fn observe_agent(&self, progress: Option<&AgentProgress>, child: i64) -> Option<AgentEvents> {
        let (events, progress) = self.events.as_ref().zip(progress)?;
        Some(events.agent(progress.seq, &progress.label, progress.phase.as_deref(), child))
    }

    async fn agent(&self, prompt: String, options: String) -> agena_plugin_sdk::Result<Value> {
        let seq = if let Some(limit) = self.max_agents {
            let seq = self.calls.fetch_add(1, Ordering::Relaxed) + 1;
            if seq > limit {
                return Err(PluginError::invalid_params(
                    "Cordis workflow exceeded maxTotalAgents",
                ));
            }
            seq
        } else {
            0
        };
        if options.len() > 64 * 1024 {
            return Err(PluginError::invalid_params(
                "workflow agent options exceed their byte budget",
            ));
        }
        let options: AgentOptions = serde_json::from_str(&options)
            .map_err(|error| PluginError::invalid_params_error(&error))?;
        let is_cordis = self.max_agents.is_some();
        if !is_cordis
            && (options.phase.is_some() || options.provider.is_some() || options.model.is_some())
        {
            return Err(PluginError::invalid_params(
                "unsupported imported workflow agent options",
            ));
        }
        let validator = options
            .schema
            .as_ref()
            .map(crate::schema::CompiledSchema::new)
            .transpose()
            .map_err(PluginError::invalid_params)?;
        let label = options.label.or_else(|| {
            is_cordis.then(|| {
                let first_line = prompt.lines().next().unwrap_or_default();
                match first_line.char_indices().nth(48) {
                    Some(_) => format!("{}…", first_line.chars().take(47).collect::<String>()),
                    None => first_line.to_owned(),
                }
            })
        });
        let prompt = match &options.schema {
            Some(schema) => {
                format!("{prompt}\n\nReturn one JSON value matching this schema:\n{schema}")
            }
            None => prompt,
        };
        let signature = if is_cordis {
            serde_json::to_vec(&(
                prompt.trim(),
                &options.agent,
                &options.schema,
                &options.provider,
                &options.model,
                self.inherit_parent_context,
            ))
        } else {
            serde_json::to_vec(&(prompt.trim(), &options.agent, &options.schema))
        }
        .map_err(|error| PluginError::internal_error(&error))?;
        let task = self
            .bridge
            .prepare_task(self.call, &prompt, options.agent.as_deref())?
            .with_model(options.provider, options.model)
            .with_parent_context(self.inherit_parent_context);
        let progress = label.map(|label| AgentProgress {
            seq,
            label,
            phase: options.phase,
        });
        let _permit = match &self.concurrency {
            Some(concurrency) => Some(concurrency.acquire().await.map_err(|_| {
                PluginError::internal("Cordis workflow agent concurrency is closed")
            })?),
            None => None,
        };
        match &self.delegation {
            Delegation::Live {
                host,
                checkpoint,
                launched,
            } => {
                self.live_agent(
                    host.as_ref(),
                    checkpoint,
                    launched,
                    task,
                    &signature,
                    validator.as_ref(),
                    progress.as_ref(),
                )
                .await
            }
            Delegation::Background(host) => {
                let request = task.into_spawn_request()?;
                let progress = progress.as_ref().expect("Cordis child has a label");
                let description = match &progress.phase {
                    Some(phase) => format!("{} ({phase})", progress.label),
                    None => progress.label.clone(),
                };
                let child = host.spawn_child(request).await?;
                self.started.fetch_add(1, Ordering::Relaxed);
                let agent_events = self.observe_agent(Some(progress), child);
                self.progress("agent/start", &format!("{description} ({child})"));
                let answer = host.wait_child(child, AGENT_TIMEOUT_MS).await;
                let value = answer.and_then(|answer| agent_value(answer, validator.as_ref()));
                let state = agent_outcome(&value);
                if let Some(agent_events) = agent_events {
                    agent_events.finish(state);
                }
                self.progress("agent/end", &format!("{description} ({child}): {state}"));
                Ok(value.unwrap_or(Value::Null))
            }
        }
    }

    async fn live_agent(
        &self,
        host: &dyn HostClient,
        checkpoint: &Mutex<Checkpoint>,
        launched: &std::sync::Mutex<BTreeSet<i64>>,
        task: AgentTask,
        signature: &[u8],
        validator: Option<&crate::schema::CompiledSchema>,
        progress: Option<&AgentProgress>,
    ) -> agena_plugin_sdk::Result<Value> {
        let is_cordis = self.max_agents.is_some();
        let fingerprint = blake3::hash(signature).to_hex().to_string();
        let (index, child) = {
            let mut journal = checkpoint.lock().await;
            let (index, replay) = journal.next(&fingerprint)?;
            match replay {
                Some(Replayed::Value(value)) => return Ok(value),
                Some(Replayed::CompletedAgent(child)) => {
                    let answer = host
                        .wait_imported_plugin_agent(child, AGENT_TIMEOUT_MS)
                        .await;
                    if let Ok(answer) = answer
                        && let Ok(value) = agent_value(answer, validator)
                    {
                        journal.complete(host, index, value.clone()).await?;
                        return Ok(value);
                    }
                }
                None => {}
            }
            journal
                .start(host, index, fingerprint, task.name.clone())
                .await?;
            let child = task.spawn(host).await?;
            if is_cordis {
                self.started.fetch_add(1, Ordering::Relaxed);
                launched
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .insert(child);
            }
            (index, child)
        };
        let agent_events = self.observe_agent(progress, child);
        let answer = host
            .wait_imported_plugin_agent(child, AGENT_TIMEOUT_MS)
            .await;
        if is_cordis && answer.is_ok() {
            launched
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .remove(&child);
        }
        let value = answer.and_then(|answer| agent_value(answer, validator));
        if let Some(agent_events) = agent_events {
            agent_events.finish(agent_outcome(&value));
        }
        let value = match value {
            Ok(value) => value,
            Err(_) if is_cordis => Value::Null,
            Err(error) => return Err(error),
        };
        checkpoint
            .lock()
            .await
            .complete(host, index, value.clone())
            .await?;
        Ok(value)
    }
}

fn agent_outcome(result: &agena_plugin_sdk::Result<Value>) -> &'static str {
    match result {
        Ok(_) => "completed",
        Err(error) if error.kind == PluginErrorKind::Cancelled => "cancelled",
        Err(_) => "failed",
    }
}

async fn run_script(
    workflow: &Workflow,
    agents: WorkflowAgents,
    args: Option<String>,
    cordis: Option<&CordisExecution>,
    cancellation: CancellationToken,
) -> agena_plugin_sdk::Result<(String, usize)> {
    let label = workflow.meta.name.clone();
    let is_cordis = cordis.is_some();
    let agents = Arc::new(agents);
    let script_agents = agents.clone();
    let run_agent = move |prompt: String, options: String| {
        let agents = script_agents.clone();
        async move {
            let value = match agents.agent(prompt, options).await {
                Ok(value) => json!({"value": value}),
                Err(error) => json!({"error": error.diagnostic_message()}),
            };
            Ok::<_, rquickjs::Error>(value.to_string())
        }
    };
    let cancel = cancellation.clone();
    let runtime = AsyncRuntime::new().map_err(|error| PluginError::internal_error(&error))?;
    runtime.set_memory_limit(32 * 1024 * 1024).await;
    runtime.set_max_stack_size(512 * 1024).await;
    let deadline = Instant::now() + WORKFLOW_TIMEOUT;
    let initial_deadline = Arc::new(std::sync::Mutex::new(None));
    let interrupt_deadline = initial_deadline.clone();
    let mut interrupts = 0_u32;
    runtime
        .set_interrupt_handler(Some(Box::new(move || {
            interrupts = interrupts.saturating_add(1);
            let now = Instant::now();
            interrupts >= 50_000
                || cancel.is_cancelled()
                || now >= deadline
                || interrupt_deadline
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .is_some_and(|initial| now >= initial)
        })))
        .await;
    let context = AsyncContext::full(&runtime)
        .await
        .map_err(|error| PluginError::internal_error(&error))?;
    let script = format!(
        "(async function () {{ 'use strict';\n{PRELUDE}\n{}\n{}\n}})()",
        if is_cordis {
            CORDIS_COMBINATORS
        } else {
            CLAUDE_COMBINATORS
        },
        workflow.script
    );
    let batch_limit = cordis
        .as_ref()
        .map_or(MAX_WORKFLOW_ITEMS, |run| run.config.max_items_per_call);
    let sync_timeout = cordis
        .as_ref()
        .map(|run| Duration::from_millis(run.config.sync_timeout_ms));
    let log_agents = agents.clone();
    let script_result = context.async_with(async move |ctx| {
        let globals = ctx.globals();
        globals
            .set("__agenaAgent", Func::from(Async(run_agent)))
            .catch(&ctx)
            .map_err(|error| error.to_string())?;
        globals
            .set(
                "__agenaLog",
                Func::from(move |kind: String, message: String| {
                    let end = message.floor_char_boundary(message.len().min(4096));
                    let message = &message[..end];
                    log_agents.progress(&kind, message);
                    tracing::info!(workflow = %label, %kind, %message, "imported workflow");
                }),
            )
            .catch(&ctx)
            .map_err(|error| error.to_string())?;
        let args = match args {
            Some(args) => ctx
                .json_parse(args)
                .catch(&ctx)
                .map_err(|error| error.to_string())?,
            None => rquickjs::Value::new_undefined(ctx.clone()),
        };
        globals
            .set("args", args)
            .catch(&ctx)
            .map_err(|error| error.to_string())?;
        globals
            .set("__agenaMaxItems", batch_limit)
            .catch(&ctx)
            .map_err(|error| error.to_string())?;
        globals
            .set("__agenaCordis", is_cordis)
            .catch(&ctx)
            .map_err(|error| error.to_string())?;
        *initial_deadline
            .lock()
            .unwrap_or_else(|error| error.into_inner()) =
            sync_timeout.map(|timeout| Instant::now() + timeout);
        let evaluation = ctx.eval::<Promise, _>(script);
        *initial_deadline
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = None;
        let promise = evaluation.catch(&ctx).map_err(|error| error.to_string())?;
        let result = promise
            .into_future::<rquickjs::Value>()
            .await
            .catch(&ctx)
            .map_err(|error| error.to_string())?;
        ctx.json_stringify(result)
            .catch(&ctx)
            .map_err(|error| error.to_string())?
            .map(|text| text.to_string().map_err(|error| error.to_string()))
            .transpose()
    });
    let output = tokio::select! {
        biased;
        _ = cancellation.cancelled() => Err(PluginError::from_kind(PluginErrorKind::Cancelled, "workflow cancelled")),
        result = tokio::time::timeout(WORKFLOW_TIMEOUT, script_result) => match result {
            Ok(result) => result.map_err(PluginError::internal),
            Err(_) => Err(PluginError::internal("workflow timed out")),
        },
    };
    drop(context);
    drop(runtime);
    let output = output?.unwrap_or_else(|| "null".into());
    if output.len() > 64 * 1024 {
        return Err(PluginError::internal("workflow output exceeds 64 KiB"));
    }
    Ok((output, agents.started.load(Ordering::Relaxed)))
}

fn agent_value(
    answer: ToolInvokeOutput,
    validator: Option<&crate::schema::CompiledSchema>,
) -> agena_plugin_sdk::Result<Value> {
    let Some(validator) = validator else {
        return Ok(Value::String(answer.output_text));
    };
    let decoded: Value = serde_json::from_str(&answer.output_text)
        .map_err(|error| PluginError::invalid_params_error(&error))?;
    validator
        .validate(&decoded)
        .map_err(PluginError::invalid_params)?;
    Ok(decoded)
}
