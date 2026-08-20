//! Plugin slots: typed handles for configured plugin instances.

use std::sync::{Arc, OnceLock, RwLock, Weak};

use agena_plugin_host::PluginHost;

// The process-visible slot is an index, not an owner. Runtime snapshots and
// turn-level ToolExecutor generations own PluginHost lifetimes; keeping a
// strong global Arc here would prevent the final generation from ever retiring.
static SLOT: OnceLock<RwLock<Weak<PluginHost>>> = OnceLock::new();

fn slot() -> &'static RwLock<Weak<PluginHost>> {
    SLOT.get_or_init(|| RwLock::new(Weak::new()))
}

/// Install the live plugin host for provider/plugin callbacks.
pub fn install_plugin_host(host: Arc<PluginHost>) {
    *slot().write().expect("plugin slot lock poisoned") = Arc::downgrade(&host);
}

/// Return the currently installed plugin host, if its Runtime generation is
/// still alive. The slot deliberately does not extend that generation.
pub fn current_plugin_host() -> Option<Arc<PluginHost>> {
    slot().read().expect("plugin slot lock poisoned").upgrade()
}
