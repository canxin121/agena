//! Internal activity observations for notifications and runtime invalidation.

pub mod registry;
pub(crate) mod state;

pub(crate) use registry::ActivityRegistry;
pub(crate) use state::{
    ActivityRuntimeState, BackgroundCompletionBridge, MonitorActivityBridge,
    RuntimeTaskActivityBridge, upsert_agent_activity_from_meta,
};
