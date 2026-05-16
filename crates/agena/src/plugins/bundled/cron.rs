//! First-party `agena.cron` plugin: schedules cron and one-shot wakeup jobs.
//!
//! The model-visible `cron_create / cron_list / cron_delete / schedule_wakeup`
//! entries now belong to this plugin. Their execution currently reuses the
//! shared in-process router bridge while the runtime keeps a single plugin-entry
//! surface.

use std::sync::{Arc, RwLock};

use async_trait::async_trait;

use crate::plugin::PluginError;
use crate::plugin::sdk::host_api::HostClient;
use crate::plugin::sdk::{
    HookSubscription, HostCapability, InitContext, InitOutcome, Plugin, PluginToolDecl,
    PluginManifest, Result as SdkResult, ToolInvokeInput, ToolInvokeOutput, ToolTag,
};
use crate::plugins::bundled::router;

pub(crate) const CRON_PLUGIN_ID: &str = "agena.cron";

pub(crate) struct CronPlugin {
    host: RwLock<Option<Arc<dyn HostClient>>>,
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
            .description(
                "Cron-style and one-shot wakeup scheduling exposed as a bundled plugin.",
            )
            .hooks(HookSubscription::TOOL_INVOKE)
            .tool(cron_create_decl())
            .tool(cron_list_decl())
            .tool(cron_delete_decl())
            .tool(schedule_wakeup_decl())
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
        match input.tool_name.as_str() {
            "cron_create" | "cron_list" | "cron_delete" | "schedule_wakeup" => {
                let _ = self.host()?;
                router::invoke_bundled_tool(
                    &input.tool_name,
                    input.input,
                    input.session_id,
                    input.call_id,
                )
            }
            other => Err(PluginError::invalid_params(format!(
                "unknown cron plugin tool '{other}'"
            ))),
        }
    }
}

fn deferred_decl<T: schemars::JsonSchema>(
    name: &str,
    description: &str,
    tags: &[ToolTag],
    concurrency_safe: bool,
) -> PluginToolDecl {
    PluginToolDecl::new(name, crate::entry::definition::json_schema_for::<T>())
        .description(description)
        .tags(tags.iter().cloned())
        .concurrency_safe(concurrency_safe)
        .deferred_load()
        .host_capability(HostCapability::Scheduler)
}

pub(crate) fn cron_create_decl() -> PluginToolDecl {
    deferred_decl::<crate::message::CronCreateToolInput>(
        "cron_create",
        "Schedule a recurring prompt with a 6-field cron expression.",
        &[ToolTag::Mutating, ToolTag::Scheduler],
        false,
    )
}

pub(crate) fn cron_list_decl() -> PluginToolDecl {
    deferred_decl::<crate::message::CronListToolInput>(
        "cron_list",
        "List all currently scheduled cron jobs and one-shot wakeups.",
        &[ToolTag::ReadOnly, ToolTag::Scheduler],
        true,
    )
}

pub(crate) fn cron_delete_decl() -> PluginToolDecl {
    deferred_decl::<crate::message::CronDeleteToolInput>(
        "cron_delete",
        "Delete a scheduled job by id.",
        &[ToolTag::Mutating, ToolTag::Scheduler],
        false,
    )
}

pub(crate) fn schedule_wakeup_decl() -> PluginToolDecl {
    deferred_decl::<crate::message::ScheduleWakeupToolInput>(
        "schedule_wakeup",
        "Schedule a one-shot prompt to fire after `delay_seconds`.",
        &[ToolTag::Mutating, ToolTag::Scheduler],
        false,
    )
}
