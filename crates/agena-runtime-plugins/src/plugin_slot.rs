//! Plugin slots: typed handles for configured plugin instances.

use std::sync::{Arc, OnceLock};

use agena_plugin_host::PluginHost;
use arc_swap::ArcSwap;

static SLOT: OnceLock<ArcSwap<Option<Arc<PluginHost>>>> = OnceLock::new();

fn slot() -> &'static ArcSwap<Option<Arc<PluginHost>>> {
    SLOT.get_or_init(|| ArcSwap::from_pointee(None))
}

/// Install the live plugin host for provider/plugin callbacks.
pub fn install_plugin_host(host: Arc<PluginHost>) {
    slot().store(Arc::new(Some(host)));
}

/// Return the currently installed plugin host, if any.
pub fn current_plugin_host() -> Option<Arc<PluginHost>> {
    slot().load().as_ref().clone()
}
