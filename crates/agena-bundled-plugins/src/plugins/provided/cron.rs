//! `agena.cron` plugin: schedules cron jobs.
//!
//! The model-visible schedule tools execute through the same plugin tool
//! surface as every other tool.

use std::sync::{Arc, RwLock};

use crate::part::{
    CronCreateToolInput, CronDeleteToolInput, CronHistoryToolInput, CronJobControlToolInput,
    CronListToolInput, CronUpdateToolInput,
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
        help = "List every scheduled job registered in this session. Jobs are session-only — they exist for this session's lifetime and are gone when it ends — and recurring jobs auto-expire after seven days. Use this to review schedules you created; never poll it waiting for a job to fire.",
        read_only,
        scheduler,
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
        help = "Schedule a one-shot or recurring job with a standard 5-field cron expression (minute hour day-of-month month day-of-week). The job fires while the session is idle and delivers its prompt to you as a system_notification appended onto the current run, waking you again; never use it to poll. Jobs are session-only and recurring jobs auto-expire after seven days. When the exact time does not matter, pick a minute that is not :00 or :30 to avoid clumping.",
        mutating,
        scheduler
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
        help = "Permanently remove a scheduled job from this session. Deleting stops future firings immediately.",
        mutating,
        scheduler
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
        help = "Change the prompt or cron parameters of an existing job. The updated schedule takes effect for subsequent firings.",
        mutating,
        scheduler
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
        help = "Temporarily suspend a job's future firings while keeping its definition. Use resume to start it again.",
        mutating,
        scheduler
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
        help = "Re-enable a job that was paused so its future firings happen again.",
        mutating,
        scheduler
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
        help = "Read the bounded delivery history (fire times, outcome, last error) for scheduled jobs. Never poll this waiting for a job to fire — the firing itself appends its prompt to the session and wakes you.",
        read_only,
        scheduler,
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
}
