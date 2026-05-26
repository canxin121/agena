//! `agena.cron` plugin: schedules cron and one-shot wakeup jobs.
//!
//! The model-visible schedule tools execute through the same plugin tool
//! surface as every other tool.

use std::sync::{Arc, RwLock};

use agena_macros::StaticToolSurface;
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

#[derive(Debug, Deserialize, JsonSchema, StaticToolSurface)]
#[tool_surface(
    tool = "schedule",
    description = "Schedule command. Set action to list, create, delete, or wakeup.",
    summary = "List, create, delete, or trigger scheduled work.",
    help = "Use action `list` to inspect registered cron jobs and one-shot wakeups, `create` for cron jobs, `delete` to remove a scheduled job, and `wakeup` for a one-shot wakeup.",
    tags(ToolTag::ReadOnly, ToolTag::Mutating, ToolTag::Scheduler),
    host_capabilities(HostCapability::Scheduler),
    concurrency_safe = false
)]
#[serde(tag = "action", rename_all = "snake_case")]
enum ScheduleToolInput {
    #[tool(exec = "cron_list")]
    List {
        #[serde(flatten)]
        args: crate::message::CronListToolInput,
    },
    #[tool(exec = "cron_create")]
    Create {
        #[serde(flatten)]
        args: crate::message::CronCreateToolInput,
    },
    #[tool(exec = "cron_delete")]
    Delete {
        #[serde(flatten)]
        args: crate::message::CronDeleteToolInput,
    },
    #[tool(exec = "schedule_wakeup")]
    Wakeup {
        #[serde(flatten)]
        args: crate::message::ScheduleWakeupToolInput,
    },
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
            .hooks(HookSubscription::TOOL_INVOKE)
            .config_schema(crate::tool::definition::empty_config_schema())
            .tool(schedule_decl())
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
            ScheduleToolInput::resolve_tool(input.tool_name.as_str(), input.input)?;
        router::invoke_tool(&tool_name, tool_input, input.session_id, input.call_id)
    }
}

fn schedule_decl() -> PluginToolDecl {
    ScheduleToolInput::tool_decl()
}
