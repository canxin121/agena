//! Hook dispatcher. For chained hooks (`tool.before/after`, `chat.*`, etc.)
//! the dispatcher feeds each plugin's patch into the next plugin's input,
//! sequentially, deterministically.

use std::collections::BTreeMap;
use std::time::Duration;

use serde::{Serialize, de::DeserializeOwned};
use tracing::Instrument;

use crate::error::TransportError;
use crate::host::{HookRunRecord, HookRunStatus, LoadedPlugin};
use crate::sdk::HookSubscription;
use crate::sdk::host_api::{self, HostCallbackContext};
use crate::sdk::rpc::method;

/// Sequential `Option<Patch>` chain. Each plugin sees the latest mutated
/// input; if it returns `Some(patch)`, the patch is merged in via `apply`.
/// Every plugin invocation is recorded into `runs` so the caller can surface
/// hook execution as transcript activity (`session_id` attributes the run).
pub async fn chain_patch<I, P, F>(
    plugins: &[std::sync::Arc<LoadedPlugin>],
    method_name: &str,
    subscription: HookSubscription,
    timeout: Duration,
    input: I,
    apply: F,
    session_id: Option<i64>,
    runs: &mut Vec<HookRunRecord>,
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
        session_id,
        runs,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn chain_patch_in_context<I, P, F, C>(
    plugins: &[std::sync::Arc<LoadedPlugin>],
    method_name: &str,
    subscription: HookSubscription,
    timeout: Duration,
    mut input: I,
    mut apply: F,
    context: C,
    session_id: Option<i64>,
    runs: &mut Vec<HookRunRecord>,
) -> Result<I, TransportError>
where
    I: Serialize + Clone,
    P: DeserializeOwned,
    F: FnMut(&mut I, P),
    C: Fn(&LoadedPlugin, &I) -> Option<HostCallbackContext>,
{
    let hook = display_hook_name(method_name);
    for plugin in plugins {
        if !plugin.subscribes(subscription) {
            continue;
        }
        let plugin_id = plugin.key().to_string();
        let params = serde_json::to_value(&input)?;
        let call = call_with_timeout(plugin, method_name, params, timeout);
        let result = if let Some(context) = context(plugin, &input) {
            match host_api::run_in_host_callback_context(context, call).await {
                Ok(v) => v,
                Err(err) => {
                    record_transport_failure(runs, &hook, &plugin_id, session_id, &err);
                    return Err(err);
                }
            }
        } else {
            match call.await {
                Ok(v) => v,
                Err(err) => {
                    record_transport_failure(runs, &hook, &plugin_id, session_id, &err);
                    return Err(err);
                }
            }
        };
        if matches!(&result, serde_json::Value::Null) {
            runs.push(HookRunRecord::new(
                &hook,
                &plugin_id,
                session_id,
                HookRunStatus::Skipped,
                format!("{hook} hook ran (no change)"),
                None,
            ));
            continue;
        }
        let patch: Option<P> = serde_json::from_value(result)?;
        match patch {
            Some(p) => {
                apply(&mut input, p);
                runs.push(HookRunRecord::new(
                    &hook,
                    &plugin_id,
                    session_id,
                    HookRunStatus::Applied,
                    format!("{hook} hook ran"),
                    None,
                ));
            }
            None => {
                runs.push(HookRunRecord::new(
                    &hook,
                    &plugin_id,
                    session_id,
                    HookRunStatus::Skipped,
                    format!("{hook} hook ran (no change)"),
                    None,
                ));
            }
        }
    }
    Ok(input)
}

/// Strip the `hooks/` RPC prefix for human-readable hook names. The command
/// hooks keep their shorter plan names (`command.before` / `command.after`)
/// instead of the transport method's `command.execute.*`.
fn display_hook_name(method_name: &str) -> String {
    if let Some(rest) = method_name.strip_prefix("hooks/command.execute.") {
        return format!("command.{rest}");
    }
    method_name
        .strip_prefix("hooks/")
        .unwrap_or(method_name)
        .to_string()
}

/// Build a failed/timed-out hook run record for a transport error.
pub(crate) fn transport_failure_record(
    hook: &str,
    plugin_id: &str,
    session_id: Option<i64>,
    err: &TransportError,
) -> HookRunRecord {
    let (status, summary) = if matches!(err, TransportError::Timeout) {
        (HookRunStatus::TimedOut, format!("{hook} hook timed out"))
    } else {
        (HookRunStatus::Failed, format!("{hook} hook failed: {err}"))
    };
    HookRunRecord::new(
        hook,
        plugin_id,
        session_id,
        status,
        summary,
        Some(err.to_string()),
    )
}

/// Record a failed/timed-out transport call before propagating the error.
fn record_transport_failure(
    runs: &mut Vec<HookRunRecord>,
    hook: &str,
    plugin_id: &str,
    session_id: Option<i64>,
    err: &TransportError,
) {
    runs.push(transport_failure_record(hook, plugin_id, session_id, err));
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
