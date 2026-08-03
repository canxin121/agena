//! `agena.cron` plugin: schedules cron and one-shot wakeup jobs.
//!
//! The model-visible schedule tools execute through the same plugin tool
//! surface as every other tool.

use std::sync::{Arc, RwLock};

use crate::message::{
    CronCreateToolInput, CronDeleteToolInput, CronHistoryToolInput, CronJobControlToolInput,
    CronListToolInput, CronUpdateToolInput, ScheduleWakeupToolInput,
};
use crate::plugins::provided::router;
use agena_plugin_host::PluginError;
use agena_plugin_host::sdk::host_api::HostClient;
use agena_plugin_host::sdk::{Result as SdkResult, ToolInvokeOutput};

pub(crate) const CRON_PLUGIN_ID: &str = "agena.cron";

pub(crate) struct CronPlugin {
    host: RwLock<Option<Arc<dyn HostClient>>>,
}

#[agena_plugin_host::sdk::agena_plugin(
    namespace = "agena",
    name = "cron",
    version = env!("CARGO_PKG_VERSION"),
    summary = "Cron-style and one-shot wakeup scheduling tools.",
    display = brief
)]
impl CronPlugin {
    pub(crate) fn new() -> Self {
        Self {
            host: RwLock::new(None),
        }
    }

    #[hook(init)]
    async fn init(
        &self,
        _ctx: agena_plugin_host::sdk::InitContext,
        host: Arc<dyn HostClient>,
    ) -> SdkResult<agena_plugin_host::sdk::InitOutcome> {
        *self
            .host
            .write()
            .map_err(|_| PluginError::internal("cron plugin host lock poisoned"))? = Some(host);
        Ok(agena_plugin_host::sdk::InitOutcome::ack(
            agena_plugin_host::sdk::Plugin::manifest(self),
        ))
    }

    fn host(&self) -> SdkResult<Arc<dyn HostClient>> {
        self.host
            .read()
            .map_err(|_| PluginError::internal("cron plugin host lock poisoned"))?
            .clone()
            .ok_or_else(|| PluginError::internal("cron plugin invoked before init"))
    }

    #[tool(
        tags(query, scheduler, discovery),
        summary = "List registered cron jobs and wakeups.",
        read_only,
        scheduler,

        display = brief,
        concurrency_safe
    )]
    async fn invoke_list(
        &self,
        context: &agena_plugin_host::sdk::ToolInvokeContext<'_>,
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

    #[tool(
        tags(mutate, scheduler),
        summary = "Create one cron schedule.",
        mutating,
        scheduler,

        display = brief
    )]
    async fn invoke_create(
        &self,
        context: &agena_plugin_host::sdk::ToolInvokeContext<'_>,
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

    #[tool(
        tags(mutate, scheduler),
        summary = "Delete one cron schedule.",
        mutating,
        scheduler,

        display = brief
    )]
    async fn invoke_delete(
        &self,
        context: &agena_plugin_host::sdk::ToolInvokeContext<'_>,
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

    #[tool(
        tags(mutate, scheduler),
        summary = "Update the prompt or schedule parameters of one retained job.",
        mutating,
        scheduler,

        display = brief
    )]
    async fn invoke_update(
        &self,
        context: &agena_plugin_host::sdk::ToolInvokeContext<'_>,
        args: CronUpdateToolInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let _ = self.host()?;
        router::invoke_tool(
            "cron_update",
            serde_json::to_value(args)
                .map_err(|err| PluginError::invalid_params(err.to_string()))?,
            context.session_id,
            context.call_id,
        )
    }

    #[tool(
        tags(mutate, scheduler),
        summary = "Pause one scheduled job without deleting it.",
        mutating,
        scheduler,

        display = brief
    )]
    async fn invoke_pause(
        &self,
        context: &agena_plugin_host::sdk::ToolInvokeContext<'_>,
        args: CronJobControlToolInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let _ = self.host()?;
        router::invoke_tool(
            "cron_pause",
            serde_json::to_value(args)
                .map_err(|err| PluginError::invalid_params(err.to_string()))?,
            context.session_id,
            context.call_id,
        )
    }

    #[tool(
        tags(mutate, scheduler),
        summary = "Resume one paused scheduled job.",
        mutating,
        scheduler,

        display = brief
    )]
    async fn invoke_resume(
        &self,
        context: &agena_plugin_host::sdk::ToolInvokeContext<'_>,
        args: CronJobControlToolInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let _ = self.host()?;
        router::invoke_tool(
            "cron_resume",
            serde_json::to_value(args)
                .map_err(|err| PluginError::invalid_params(err.to_string()))?,
            context.session_id,
            context.call_id,
        )
    }

    #[tool(
        tags(query, scheduler),
        summary = "Inspect bounded persisted delivery history for scheduled jobs.",
        read_only,
        scheduler,

        display = detailed,
        concurrency_safe
    )]
    async fn invoke_history(
        &self,
        context: &agena_plugin_host::sdk::ToolInvokeContext<'_>,
        args: CronHistoryToolInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let _ = self.host()?;
        router::invoke_tool(
            "cron_history",
            serde_json::to_value(args)
                .map_err(|err| PluginError::invalid_params(err.to_string()))?,
            context.session_id,
            context.call_id,
        )
    }

    #[tool(
        tags(mutate, scheduler),
        summary = "Create one one-shot wakeup.",
        mutating,
        scheduler,

        display = brief
    )]
    async fn invoke_wakeup(
        &self,
        context: &agena_plugin_host::sdk::ToolInvokeContext<'_>,
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
