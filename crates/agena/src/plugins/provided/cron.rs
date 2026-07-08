//! `agena.cron` plugin: schedules cron and one-shot wakeup jobs.
//!
//! The model-visible schedule tools execute through the same plugin tool
//! surface as every other tool.

use std::sync::{Arc, RwLock};

use crate::message::{
    CronCreateToolInput, CronDeleteToolInput, CronListToolInput, ScheduleWakeupToolInput,
};
use crate::plugin::PluginError;
use crate::plugin::sdk::host_api::HostClient;
use crate::plugin::sdk::{HostCapability, Result as SdkResult, ToolInvokeOutput};
use crate::plugins::provided::router;

pub(crate) const CRON_PLUGIN_ID: &str = "agena.cron";

pub(crate) struct CronPlugin {
    host: RwLock<Option<Arc<dyn HostClient>>>,
}

#[crate::plugin::sdk::agena_plugin(
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

    fn host(&self) -> SdkResult<Arc<dyn HostClient>> {
        self.host
            .read()
            .map_err(|_| PluginError::new("cron plugin host lock poisoned"))?
            .clone()
            .ok_or_else(|| PluginError::new("cron plugin invoked before init"))
    }

    #[tool(
        name = "list",
        summary = "List registered cron jobs and wakeups.",
        read_only,
        scheduler,
        capabilities(HostCapability::Scheduler),
        display = brief,
        concurrency_safe
    )]
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

    #[tool(
        name = "create",
        summary = "Create one cron schedule.",
        mutating,
        scheduler,
        capabilities(HostCapability::Scheduler),
        display = brief
    )]
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

    #[tool(
        name = "delete",
        summary = "Delete one cron schedule.",
        mutating,
        scheduler,
        capabilities(HostCapability::Scheduler),
        display = brief
    )]
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

    #[tool(
        name = "wakeup",
        summary = "Create one one-shot wakeup.",
        mutating,
        scheduler,
        capabilities(HostCapability::Scheduler),
        display = brief
    )]
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
