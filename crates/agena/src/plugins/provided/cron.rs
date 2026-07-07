//! `agena.cron` plugin: schedules cron and one-shot wakeup jobs.
//!
//! The model-visible schedule tools execute through the same plugin tool
//! surface as every other tool.

use std::sync::{Arc, RwLock};

use agena_macros::{StaticToolSurface, ToolSuite};
use schemars::JsonSchema;

use crate::message::{
    CronCreateToolInput, CronDeleteToolInput, CronListToolInput, ScheduleWakeupToolInput,
};
use crate::plugin::PluginError;
use crate::plugin::sdk::host_api::HostClient;
use crate::plugin::sdk::{HostCapability, Result as SdkResult, ToolInvokeOutput, ToolTag};
use crate::plugins::provided::router;

pub(crate) const CRON_PLUGIN_ID: &str = "agena.cron";

pub(crate) struct CronPlugin {
    host: RwLock<Option<Arc<dyn HostClient>>>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize, JsonSchema, StaticToolSurface)]
#[tool_surface(
    tool = "list",
    summary = "List registered cron jobs and wakeups.",
    handler_receiver = CronPlugin,
    handle_with_context = CronPlugin::invoke_list,
    handle_field = args,
    handle_by_value = true,
    display = brief,
    tags(ToolTag::ReadOnly, ToolTag::Scheduler),
    capabilities(HostCapability::Scheduler),
    concurrency_safe = true
)]
#[serde(deny_unknown_fields)]
struct CronListSurfaceInput {
    #[serde(flatten)]
    args: CronListToolInput,
}

#[derive(Debug, serde::Serialize, serde::Deserialize, JsonSchema, StaticToolSurface)]
#[tool_surface(
    tool = "create",
    summary = "Create one cron schedule.",
    handler_receiver = CronPlugin,
    handle_with_context = CronPlugin::invoke_create,
    handle_field = args,
    handle_by_value = true,
    display = brief,
    tags(ToolTag::Mutating, ToolTag::Scheduler),
    capabilities(HostCapability::Scheduler),
    concurrency_safe = false
)]
#[serde(deny_unknown_fields)]
struct CronCreateSurfaceInput {
    #[serde(flatten)]
    args: CronCreateToolInput,
}

#[derive(Debug, serde::Serialize, serde::Deserialize, JsonSchema, StaticToolSurface)]
#[tool_surface(
    tool = "delete",
    summary = "Delete one cron schedule.",
    handler_receiver = CronPlugin,
    handle_with_context = CronPlugin::invoke_delete,
    handle_field = args,
    handle_by_value = true,
    display = brief,
    tags(ToolTag::Mutating, ToolTag::Scheduler),
    capabilities(HostCapability::Scheduler),
    concurrency_safe = false
)]
#[serde(deny_unknown_fields)]
struct CronDeleteSurfaceInput {
    #[serde(flatten)]
    args: CronDeleteToolInput,
}

#[derive(Debug, serde::Serialize, serde::Deserialize, JsonSchema, StaticToolSurface)]
#[tool_surface(
    tool = "wakeup",
    summary = "Create one one-shot wakeup.",
    handler_receiver = CronPlugin,
    handle_with_context = CronPlugin::invoke_wakeup,
    handle_field = args,
    handle_by_value = true,
    display = brief,
    tags(ToolTag::Mutating, ToolTag::Scheduler),
    capabilities(HostCapability::Scheduler),
    concurrency_safe = false
)]
#[serde(deny_unknown_fields)]
struct CronWakeupSurfaceInput {
    #[serde(flatten)]
    args: ScheduleWakeupToolInput,
}

#[allow(dead_code)]
#[derive(Debug, ToolSuite)]
#[tool_suite(handler_receiver = CronPlugin)]
enum CronToolSuite {
    List(CronListSurfaceInput),
    Create(CronCreateSurfaceInput),
    Delete(CronDeleteSurfaceInput),
    Wakeup(CronWakeupSurfaceInput),
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

    async fn invoke_list(
        &self,
        context: &crate::plugin::sdk::ToolInvokeContext<'_>,
        args: CronListToolInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let _ = self.host()?;
        router::invoke_tool(
            "cron_list",
            serde_json::to_value(args)
                .map_err(|err| PluginError::invalid_params(err.to_string()))?,
            context.session_id,
            context.call_id,
        )
    }

    async fn invoke_create(
        &self,
        context: &crate::plugin::sdk::ToolInvokeContext<'_>,
        args: CronCreateToolInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let _ = self.host()?;
        router::invoke_tool(
            "cron_create",
            serde_json::to_value(args)
                .map_err(|err| PluginError::invalid_params(err.to_string()))?,
            context.session_id,
            context.call_id,
        )
    }

    async fn invoke_delete(
        &self,
        context: &crate::plugin::sdk::ToolInvokeContext<'_>,
        args: CronDeleteToolInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let _ = self.host()?;
        router::invoke_tool(
            "cron_delete",
            serde_json::to_value(args)
                .map_err(|err| PluginError::invalid_params(err.to_string()))?,
            context.session_id,
            context.call_id,
        )
    }

    async fn invoke_wakeup(
        &self,
        context: &crate::plugin::sdk::ToolInvokeContext<'_>,
        args: ScheduleWakeupToolInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let _ = self.host()?;
        router::invoke_tool(
            "schedule_wakeup",
            serde_json::to_value(args)
                .map_err(|err| PluginError::invalid_params(err.to_string()))?,
            context.session_id,
            context.call_id,
        )
    }
}

#[crate::plugin::sdk::plugin(
    namespace = "agena",
    name = "cron",
    version = env!("CARGO_PKG_VERSION"),
    summary = "Cron-style and one-shot wakeup scheduling tools.",
    display = brief
)]
impl CronPlugin {
    #[hook]
    async fn init(
        &self,
        _ctx: crate::plugin::sdk::InitContext,
        host: Arc<dyn HostClient>,
    ) -> SdkResult<crate::plugin::sdk::InitOutcome> {
        *self
            .host
            .write()
            .map_err(|_| PluginError::new("cron plugin host lock poisoned"))? = Some(host);
        Ok(crate::plugin::sdk::InitOutcome::ack(
            crate::plugin::sdk::Plugin::manifest(self),
        ))
    }

    #[tool_suite]
    async fn tool_invoke(
        &self,
        input: CronToolSuite,
        context: &crate::plugin::sdk::ToolInvokeContext<'_>,
    ) -> SdkResult<ToolInvokeOutput> {
        input.dispatch_tool_invoke_with_context(self, context).await
    }
}
