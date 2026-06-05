//! `agena.cron` plugin: schedules cron and one-shot wakeup jobs.
//!
//! The model-visible schedule tools execute through the same plugin tool
//! surface as every other tool.

use std::sync::{Arc, RwLock};

use agena_macros::StaticToolSurface;
use schemars::JsonSchema;

use crate::message::{
    CronCreateToolInput, CronDeleteToolInput, CronListToolInput, ScheduleWakeupToolInput,
};
use crate::plugin::PluginError;
use crate::plugin::sdk::host_api::HostClient;
use crate::plugin::sdk::{
    HookSubscription, HostCapability, Result as SdkResult, ToolInvokeOutput, ToolTag,
};
use crate::plugins::provided::router;

pub(crate) const CRON_PLUGIN_ID: &str = "agena.cron";

pub(crate) struct CronPlugin {
    host: RwLock<Option<Arc<dyn HostClient>>>,
}

#[derive(Debug, serde::Deserialize, JsonSchema, StaticToolSurface)]
#[tool_surface(
    tool = "schedule",
    description = "Scheduler command. Use action `list`, `create`, `delete`, or `wakeup` to manage cron schedules and one-shot wakeups.",
    summary = "Manage cron schedules and one-shot wakeups.",
    help = "Use action `list` to inspect registered jobs, `create` to add one cron schedule, `delete` to remove one schedule by id, and `wakeup` to create one one-shot wakeup request.",
    display = brief,
    tags(ToolTag::ReadOnly, ToolTag::Mutating, ToolTag::Scheduler),
    host_capabilities(HostCapability::Scheduler),
    concurrency_safe = false
)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum ScheduleToolInput {
    #[tool(exec = "list", route = "cron_list", shape = CronListToolInput)]
    List {
        #[serde(flatten)]
        args: CronListToolInput,
    },
    #[tool(exec = "create", route = "cron_create", shape = CronCreateToolInput)]
    Create {
        #[serde(flatten)]
        args: CronCreateToolInput,
    },
    #[tool(exec = "delete", route = "cron_delete", shape = CronDeleteToolInput)]
    Delete {
        #[serde(flatten)]
        args: CronDeleteToolInput,
    },
    #[tool(exec = "wakeup", route = "schedule_wakeup", shape = ScheduleWakeupToolInput)]
    Wakeup {
        #[serde(flatten)]
        args: ScheduleWakeupToolInput,
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

#[crate::plugin::sdk::plugin]
impl crate::plugin::sdk::Plugin for CronPlugin {
    #[agena_plugin_sdk::plugin_manifest_method(
        id = CRON_PLUGIN_ID,
        version = env!("CARGO_PKG_VERSION"),
        description = "Cron-style and one-shot wakeup scheduling tools.",
        hooks = HookSubscription::TOOL_INVOKE,
        display = brief,
        tool_surface = ScheduleToolInput,
    )]
    fn manifest(&self) -> crate::plugin::sdk::PluginManifest {}

    #[agena_plugin_sdk::plugin_init_method(
        host_cell = {
            field = self.host,
            value = host,
            poisoned = "cron plugin host lock poisoned"
        },
    )]
    async fn init(
        &self,
        _ctx: crate::plugin::sdk::InitContext,
        host: Arc<dyn HostClient>,
    ) -> SdkResult<crate::plugin::sdk::InitOutcome> {
    }

    async fn tool_invoke(
        &self,
        input: crate::plugin::sdk::ToolInvokeInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let _ = self.host()?;
        router::invoke_tool_surface::<ScheduleToolInput>(
            input.tool_name.as_str(),
            input.input,
            input.session_id,
            input.call_id,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn schedule_tool_routes_actions_into_internal_scheduler_payloads() {
        let (list_tool, list_input) =
            ScheduleToolInput::resolve_tool("schedule", json!({ "action": "list" }))
                .expect("list action should resolve");
        assert_eq!(list_tool, "cron_list");
        assert_eq!(list_input, json!({}));

        let (wakeup_tool, wakeup_input) = ScheduleToolInput::resolve_tool(
            "schedule",
            json!({
                "action": "wakeup",
                "prompt": "Check status",
                "delay_seconds": 30
            }),
        )
        .expect("wakeup action should resolve");
        assert_eq!(wakeup_tool, "schedule_wakeup");
        assert_eq!(
            wakeup_input,
            json!({
                "prompt": "Check status",
                "delay_seconds": 30
            })
        );
    }
}
