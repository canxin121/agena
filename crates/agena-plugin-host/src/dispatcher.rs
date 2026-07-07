//! Hook dispatcher. For chained hooks (`tool.before/after`, `chat.*`, etc.)
//! the dispatcher feeds each plugin's patch into the next plugin's input,
//! sequentially, deterministically.

use std::collections::BTreeMap;
use std::time::Duration;

use serde::{Serialize, de::DeserializeOwned};
use tracing::Instrument;

use crate::error::TransportError;
use crate::host::LoadedPlugin;
use crate::sdk::HookSubscription;
use crate::sdk::host_api::{self, HostCallbackContext};
use crate::sdk::rpc::method;

/// Sequential `Option<Patch>` chain. Each plugin sees the latest mutated
/// input; if it returns `Some(patch)`, the patch is merged in via `apply`.
pub async fn chain_patch<I, P, F>(
    plugins: &[std::sync::Arc<LoadedPlugin>],
    method_name: &str,
    subscription: HookSubscription,
    timeout: Duration,
    input: I,
    apply: F,
) -> Result<I, TransportError>
where
    I: Serialize + Clone,
    P: DeserializeOwned,
    F: FnMut(&mut I, P),
{
    chain_patch_in_context(
        plugins,
        method_name,
        subscription,
        timeout,
        input,
        apply,
        |_, _| None,
    )
    .await
}

pub async fn chain_patch_in_context<I, P, F, C>(
    plugins: &[std::sync::Arc<LoadedPlugin>],
    method_name: &str,
    subscription: HookSubscription,
    timeout: Duration,
    mut input: I,
    mut apply: F,
    context: C,
) -> Result<I, TransportError>
where
    I: Serialize + Clone,
    P: DeserializeOwned,
    F: FnMut(&mut I, P),
    C: Fn(&LoadedPlugin, &I) -> Option<HostCallbackContext>,
{
    for plugin in plugins {
        if !plugin.subscribes(subscription) {
            continue;
        }
        let params = serde_json::to_value(&input)?;
        let call = call_with_timeout(plugin, method_name, params, timeout);
        let result = if let Some(context) = context(plugin, &input) {
            host_api::run_in_host_callback_context(context, call).await?
        } else {
            call.await?
        };
        if matches!(&result, serde_json::Value::Null) {
            continue;
        }
        let patch: Option<P> = serde_json::from_value(result)?;
        if let Some(p) = patch {
            apply(&mut input, p);
        }
    }
    Ok(input)
}

pub async fn call_with_timeout(
    plugin: &LoadedPlugin,
    method_name: &str,
    params: serde_json::Value,
    timeout: Duration,
) -> Result<serde_json::Value, TransportError> {
    let hook_span = tracing::info_span!(
        "hook.call",
        plugin = %plugin.manifest.name,
        method = method_name,
    );
    let fut = plugin.transport.dispatch(method_name, params);
    match tokio::time::timeout(timeout, fut)
        .instrument(hook_span)
        .await
    {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(e)) => {
            tracing::warn!(
                target: "agena_plugin_host::dispatch",
                plugin = %plugin.manifest.name,
                method = method_name,
                "plugin call failed: {e}"
            );
            Err(e)
        }
        Err(_) => {
            tracing::warn!(
                target: "agena_plugin_host::dispatch",
                plugin = %plugin.manifest.name,
                method = method_name,
                "plugin call timed out"
            );
            Err(TransportError::Timeout)
        }
    }
}

/// Helper: merge BTreeMap into accumulator.
pub fn merge_string_map(into: &mut BTreeMap<String, String>, from: BTreeMap<String, String>) {
    for (k, v) in from {
        into.insert(k, v);
    }
}

pub mod methods {
    pub use super::method::*;
}
