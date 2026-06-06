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
use crate::plugin::sdk::{HostCapability, Result as SdkResult, ToolInvokeOutput, ToolTag};
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

#[crate::plugin::sdk::plugin(
    id = CRON_PLUGIN_ID,
    version = env!("CARGO_PKG_VERSION"),
    description = "Cron-style and one-shot wakeup scheduling tools.",
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

    #[tool]
    async fn tool_invoke(
        &self,
        input: ScheduleToolInput,
        context: &crate::plugin::sdk::ToolInvokeContext<'_>,
    ) -> SdkResult<ToolInvokeOutput> {
        let _ = self.host()?;
        let (tool_name, payload) = match input {
            ScheduleToolInput::List { args } => ("cron_list", serde_json::to_value(args)),
            ScheduleToolInput::Create { args } => ("cron_create", serde_json::to_value(args)),
            ScheduleToolInput::Delete { args } => ("cron_delete", serde_json::to_value(args)),
            ScheduleToolInput::Wakeup { args } => ("schedule_wakeup", serde_json::to_value(args)),
        };
        router::invoke_tool(
            tool_name,
            payload.map_err(|err| PluginError::invalid_params(err.to_string()))?,
            context.session_id,
            context.call_id,
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
