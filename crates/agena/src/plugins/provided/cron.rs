//! `agena.cron` plugin: schedules cron and one-shot wakeup jobs.
//!
//! The model-visible schedule entries execute through the same plugin-entry
//! surface as every other tool.

use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;

use crate::plugin::PluginError;
use crate::plugin::sdk::host_api::HostClient;
use crate::plugin::sdk::{
    HookSubscription, HostCapability, InitContext, InitOutcome, Plugin, PluginManifest,
    PluginToolDecl, Result as SdkResult, ToolInvokeInput, ToolInvokeOutput, ToolTag,
};
use crate::plugins::provided::router;

pub(crate) const CRON_PLUGIN_ID: &str = "agena.cron";

pub(crate) struct CronPlugin {
    host: RwLock<Option<Arc<dyn HostClient>>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(tag = "command", content = "args", rename_all = "snake_case")]
enum ScheduleToolInput {
    List(crate::message::CronListToolInput),
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(tag = "command", content = "args", rename_all = "snake_case")]
enum ScheduleEditToolInput {
    Create(crate::message::CronCreateToolInput),
    Delete(crate::message::CronDeleteToolInput),
    Wakeup(crate::message::ScheduleWakeupToolInput),
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
        PluginManifest::builder("agena-cron", env!("CARGO_PKG_VERSION"))
            .description("Cron-style and one-shot wakeup scheduling tools.")
            .hooks(HookSubscription::TOOL_INVOKE)
            .tool(schedule_decl())
            .tool(schedule_edit_decl())
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
        match input.tool_name.as_str() {
            "schedule" => match serde_json::from_value::<ScheduleToolInput>(input.input)? {
                ScheduleToolInput::List(args) => {
                    invoke("cron_list", args, input.session_id, input.call_id)
                }
            },
            "schedule_edit" => {
                match serde_json::from_value::<ScheduleEditToolInput>(input.input)? {
                    ScheduleEditToolInput::Create(args) => {
                        invoke("cron_create", args, input.session_id, input.call_id)
                    }
                    ScheduleEditToolInput::Delete(args) => {
                        invoke("cron_delete", args, input.session_id, input.call_id)
                    }
                    ScheduleEditToolInput::Wakeup(args) => {
                        invoke("schedule_wakeup", args, input.session_id, input.call_id)
                    }
                }
            }
            other => Err(PluginError::invalid_params(format!(
                "unknown cron plugin tool '{other}'"
            ))),
        }
    }
}

fn invoke<T: serde::Serialize>(
    tool_name: &str,
    args: T,
    session_id: i64,
    call_id: i64,
) -> SdkResult<ToolInvokeOutput> {
    router::invoke_tool(
        tool_name,
        serde_json::to_value(args).map_err(|err| PluginError::invalid_params(err.to_string()))?,
        session_id,
        call_id,
    )
}

fn schedule_decl() -> PluginToolDecl {
    PluginToolDecl::new(
        "schedule",
        crate::entry::definition::json_schema_for::<ScheduleToolInput>(),
    )
    .description("Schedule read command. Set command to list; pass that command's payload in args.")
    .tags([ToolTag::ReadOnly, ToolTag::Scheduler])
    .concurrency_safe(true)
    .deferred_load()
    .host_capability(HostCapability::Scheduler)
}

fn schedule_edit_decl() -> PluginToolDecl {
    PluginToolDecl::new(
        "schedule_edit",
        crate::entry::definition::json_schema_for::<ScheduleEditToolInput>(),
    )
    .description(
        "Schedule edit command. Set command to create, delete, or wakeup; pass that command's payload in args.",
    )
    .tags([ToolTag::Mutating, ToolTag::Scheduler])
    .concurrency_safe(false)
    .deferred_load()
    .host_capability(HostCapability::Scheduler)
}
