//! `agena.cron` plugin: schedules cron and one-shot wakeup jobs.
//!
//! The model-visible schedule tools execute through the same plugin tool
//! surface as every other tool.

use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use schemars::JsonSchema;

use crate::message::{
    CronCreateToolInput, CronDeleteToolInput, CronListToolInput, ScheduleWakeupToolInput,
};
use crate::plugin::PluginError;
use crate::plugin::sdk::host_api::HostClient;
use crate::plugin::sdk::{
    HookSubscription, HostCapability, InitContext, InitOutcome, Plugin, PluginManifest,
    PluginToolDecl, Result as SdkResult, ToolDescriptionMode, ToolInvokeInput, ToolInvokeOutput,
    ToolTag, UiTextDisplayMode,
};
use crate::plugins::provided::router;

pub(crate) const CRON_PLUGIN_ID: &str = "agena.cron";

pub(crate) struct CronPlugin {
    host: RwLock<Option<Arc<dyn HostClient>>>,
}

#[derive(Debug, Default, serde::Deserialize, JsonSchema)]
struct ScheduleListInput {
    #[serde(flatten)]
    args: CronListToolInput,
}

#[derive(Debug, serde::Deserialize, JsonSchema)]
struct ScheduleCreateInput {
    #[serde(flatten)]
    args: CronCreateToolInput,
}

#[derive(Debug, serde::Deserialize, JsonSchema)]
struct ScheduleDeleteInput {
    #[serde(flatten)]
    args: CronDeleteToolInput,
}

#[derive(Debug, serde::Deserialize, JsonSchema)]
struct ScheduleWakeupInput {
    #[serde(flatten)]
    args: ScheduleWakeupToolInput,
}

impl CronPlugin {
    pub(crate) fn new() -> Self {
        Self {
            host: RwLock::new(None),
        }
    }

    fn host(&self) -> SdkResult<Arc<dyn HostClient>> {
        self.host
            .read()
            .map_err(|_| PluginError::new("cron plugin host lock poisoned"))?
            .clone()
            .ok_or_else(|| PluginError::new("cron plugin invoked before init"))
    }
}

#[async_trait]
impl Plugin for CronPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::builder(CRON_PLUGIN_ID, env!("CARGO_PKG_VERSION"))
            .description("Cron-style and one-shot wakeup scheduling tools.")
            .tool_description_mode(ToolDescriptionMode::Brief)
            .ui_display_mode(UiTextDisplayMode::Summary)
            .hooks(HookSubscription::TOOL_INVOKE)
            .config_schema(crate::tool::definition::empty_config_schema())
            .tools(schedule_decls())
            .build()
    }

    async fn init(&self, _ctx: InitContext, host: Arc<dyn HostClient>) -> SdkResult<InitOutcome> {
        *self
            .host
            .write()
            .map_err(|_| PluginError::new("cron plugin host lock poisoned"))? = Some(host);
        Ok(InitOutcome::ack(self.manifest()))
    }

    async fn tool_invoke(&self, input: ToolInvokeInput) -> SdkResult<ToolInvokeOutput> {
        let _ = self.host()?;
        let (tool_name, tool_input) =
            resolve_schedule_tool_input(input.tool_name.as_str(), input.input)?;
        router::invoke_tool(&tool_name, tool_input, input.session_id, input.call_id)
    }
}

fn schedule_decls() -> Vec<PluginToolDecl> {
    vec![
        PluginToolDecl::new(
            "schedule.list",
            crate::tool::definition::json_schema_for::<ScheduleListInput>(),
        )
        .description("List registered cron jobs and one-shot wakeups.")
        .summary("List scheduled jobs and wakeups.")
        .help("Lists the registered cron jobs and one-shot wakeups.")
        .ui_display_mode(UiTextDisplayMode::Summary)
        .tags([ToolTag::ReadOnly, ToolTag::Scheduler])
        .host_capability(HostCapability::Scheduler)
        .concurrency_safe(true),
        PluginToolDecl::new(
            "schedule.create",
            crate::tool::definition::json_schema_for::<ScheduleCreateInput>(),
        )
        .description("Create one cron schedule entry.")
        .summary("Create one cron schedule.")
        .help("Creates one cron schedule entry through the scheduler host bridge.")
        .ui_display_mode(UiTextDisplayMode::Summary)
        .tags([ToolTag::Mutating, ToolTag::Scheduler])
        .host_capability(HostCapability::Scheduler)
        .concurrency_safe(false),
        PluginToolDecl::new(
            "schedule.delete",
            crate::tool::definition::json_schema_for::<ScheduleDeleteInput>(),
        )
        .description("Delete one cron schedule entry by id.")
        .summary("Delete one cron schedule.")
        .help("Deletes one cron schedule entry by id through the scheduler host bridge.")
        .ui_display_mode(UiTextDisplayMode::Summary)
        .tags([ToolTag::Mutating, ToolTag::Scheduler])
        .host_capability(HostCapability::Scheduler)
        .concurrency_safe(false),
        PluginToolDecl::new(
            "schedule.wakeup",
            crate::tool::definition::json_schema_for::<ScheduleWakeupInput>(),
        )
        .description("Create one one-shot wakeup request.")
        .summary("Create one one-shot wakeup.")
        .help("Creates one one-shot wakeup request through the scheduler host bridge.")
        .ui_display_mode(UiTextDisplayMode::Summary)
        .tags([ToolTag::Mutating, ToolTag::Scheduler])
        .host_capability(HostCapability::Scheduler)
        .concurrency_safe(false),
    ]
}

fn parse_input<T>(input: serde_json::Value) -> SdkResult<T>
where
    T: for<'de> serde::Deserialize<'de>,
{
    serde_json::from_value(input).map_err(|err| PluginError::invalid_params(err.to_string()))
}

fn resolve_schedule_tool_input(
    tool_name: &str,
    input: serde_json::Value,
) -> SdkResult<(String, serde_json::Value)> {
    match tool_name {
        "schedule.list" => Ok((
            "cron_list".to_string(),
            serde_json::to_value(parse_input::<ScheduleListInput>(input)?.args)
                .map_err(|err| PluginError::invalid_params(err.to_string()))?,
        )),
        "schedule.create" => Ok((
            "cron_create".to_string(),
            serde_json::to_value(parse_input::<ScheduleCreateInput>(input)?.args)
                .map_err(|err| PluginError::invalid_params(err.to_string()))?,
        )),
        "schedule.delete" => Ok((
            "cron_delete".to_string(),
            serde_json::to_value(parse_input::<ScheduleDeleteInput>(input)?.args)
                .map_err(|err| PluginError::invalid_params(err.to_string()))?,
        )),
        "schedule.wakeup" => Ok((
            "schedule_wakeup".to_string(),
            serde_json::to_value(parse_input::<ScheduleWakeupInput>(input)?.args)
                .map_err(|err| PluginError::invalid_params(err.to_string()))?,
        )),
        other => Err(PluginError::invalid_params(format!(
            "unknown schedule tool '{other}'"
        ))),
    }
}
