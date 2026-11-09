//! Plugin slots: typed handles for configured plugin instances.

use std::sync::{Arc, OnceLock, RwLock, Weak};

use agena_plugin_host::PluginHost;

// The process-visible slot is an index, not an owner. Runtime snapshots and
// turn-level ToolExecutor generations own PluginHost lifetimes; keeping a
// strong global Arc here would prevent the final generation from ever retiring.
static SLOT: OnceLock<RwLock<Weak<PluginHost>>> = OnceLock::new();
static GLOBAL_SLOT: OnceLock<RwLock<Weak<PluginHost>>> = OnceLock::new();

fn slot() -> &'static RwLock<Weak<PluginHost>> {
    SLOT.get_or_init(|| RwLock::new(Weak::new()))
}

/// Install the live plugin host for provider/plugin callbacks.
pub fn install_plugin_host(host: Arc<PluginHost>) {
    *slot().write().expect("plugin slot lock poisoned") = Arc::downgrade(&host);
}

/// Install the live instance-scoped plugin host.
pub fn install_global_plugin_host(host: Arc<PluginHost>) {
    *GLOBAL_SLOT
        .get_or_init(|| RwLock::new(Weak::new()))
        .write()
        .expect("plugin slot lock poisoned") = Arc::downgrade(&host);
}

/// Return the currently installed plugin host, if its Runtime generation is
/// still alive. The slot deliberately does not extend that generation.
pub fn current_plugin_host() -> Option<Arc<PluginHost>> {
    let context = agena_plugin_host::sdk::host_api::current_host_callback_context();
    let global = context.as_ref().is_some_and(|context| {
        context.session_id.is_some() && context.workspace_root.as_deref().is_none_or(str::is_empty)
    });
    let selected = if global {
        GLOBAL_SLOT.get_or_init(|| RwLock::new(Weak::new()))
    } else {
        slot()
    };
    selected
        .read()
        .expect("plugin slot lock poisoned")
        .upgrade()
}

#[cfg(test)]
mod tests {
    use super::*;
    use agena_plugin_host::sdk::host_api::{
        HostCallbackContext, run_in_isolated_host_callback_context,
    };

    #[tokio::test]
    async fn session_scope_selects_only_its_host() {
        let project = PluginHost::new_empty();
        let global = PluginHost::new_empty();
        install_plugin_host(project.clone());
        install_global_plugin_host(global.clone());
        assert!(Arc::ptr_eq(&current_plugin_host().unwrap(), &project));
        let context = |root: Option<&str>| HostCallbackContext {
            session_id: Some(42),
            workspace_root: root.map(str::to_owned),
            ..Default::default()
        };
        for root in [None, Some("")] {
            run_in_isolated_host_callback_context(context(root), async {
                assert!(Arc::ptr_eq(&current_plugin_host().unwrap(), &global));
            })
            .await;
        }
        run_in_isolated_host_callback_context(context(Some("/project")), async {
            assert!(Arc::ptr_eq(&current_plugin_host().unwrap(), &project));
        })
        .await;
        drop(global);
        run_in_isolated_host_callback_context(context(None), async {
            assert!(current_plugin_host().is_none());
        })
        .await;
        let global = PluginHost::new_empty();
        install_global_plugin_host(global.clone());
        drop(project);
        run_in_isolated_host_callback_context(context(Some("/project")), async {
            assert!(current_plugin_host().is_none());
        })
        .await;
        drop(global);
        run_in_isolated_host_callback_context(context(None), async {
            assert!(current_plugin_host().is_none());
        })
        .await;
    }
}
