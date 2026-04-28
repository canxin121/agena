//! Process-global slot for the active [`PluginHost`]. Provider request
//! builders pull from this to apply the `chat.headers` hook without
//! threading the host through every constructor.
//!
//! The slot is filled by `RuntimeSnapshot::build` and replaced on every
//! reload; readers always see the latest snapshot via `arc_swap::ArcSwap`.

use std::sync::OnceLock;

use arc_swap::ArcSwap;
use std::sync::Arc;

use crate::plugin::PluginHost;

static SLOT: OnceLock<ArcSwap<Option<Arc<PluginHost>>>> = OnceLock::new();

fn slot() -> &'static ArcSwap<Option<Arc<PluginHost>>> {
    SLOT.get_or_init(|| ArcSwap::from_pointee(None))
}

/// Install the live plugin host. Call once per snapshot build.
pub fn install(host: Arc<PluginHost>) {
    slot().store(Arc::new(Some(host)));
}

/// Snapshot the current plugin host, if installed.
pub fn current() -> Option<Arc<PluginHost>> {
    slot().load().as_ref().clone()
}
