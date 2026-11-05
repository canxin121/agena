//! Effect-owned typed interception of agent lifecycle boundaries.
//! A hook has one owner, one event contract, and an explicit composition mode.
//! Unloading that owner disposes its registrations with the plugin generation.

use std::sync::Arc;
use std::time::Duration;

use serde::{Serialize, de::DeserializeOwned};

use super::{HostHandle, LoadedPlugin};
use crate::dispatcher::call_with_timeout;
use crate::effect_scope::PluginEffectScope;
use crate::error::HostError;
use crate::event_pipeline::{
    PluginBailPipeline, PluginGuardDecision, PluginGuardErrorPolicy, PluginGuardPipeline,
    PluginObservePipeline,
};
use crate::sdk::host_api::HostCallbackContext;
use crate::sdk::rpc::method;
use crate::sdk::{
    CompactionAfterInput, CompactionBeforeInput, CompactionBeforePatch, HookSubscription,
    ModelRequestAfterInput, ModelRequestBeforeInput, ModelRequestBeforePatch,
    ModelRequestErrorInput, ModelRequestErrorPatch, PermissionRequestInput, PermissionRequestPatch,
    SubagentStartInput, SubagentStartPatch, SubagentStopInput,
};

trait SessionEvent: Serialize + Clone + Send + 'static {
    fn session_id(&self) -> i64;
}

macro_rules! scoped_event {
    ($($event:ty),+ $(,)?) => {
        $(impl SessionEvent for $event {
            fn session_id(&self) -> i64 { self.session_id }
        })+
    };
}
scoped_event!(
    ModelRequestBeforeInput,
    ModelRequestAfterInput,
    ModelRequestErrorInput,
    CompactionBeforeInput,
    CompactionAfterInput,
    SubagentStartInput,
    SubagentStopInput,
    PermissionRequestInput,
);

fn scope_for(
    plugin: &Arc<LoadedPlugin>,
    handle: &Arc<HostHandle>,
) -> Result<Arc<PluginEffectScope>, HostError> {
    handle
        .effect_scope(&plugin.key())
        .ok_or_else(|| HostError::Config(format!("missing hook effect scope for {}", plugin.key())))
}

async fn invoke<I: SessionEvent>(
    plugin: Arc<LoadedPlugin>,
    handle: Arc<HostHandle>,
    method: &'static str,
    input: I,
    timeout: Duration,
) -> Result<serde_json::Value, String> {
    let context = HostCallbackContext {
        plugin_id: Some(plugin.key().to_string()),
        session_id: Some(input.session_id()),
        ..Default::default()
    };
    let params = serde_json::to_value(input).map_err(|error| error.to_string())?;
    handle
        .run_in_authorized_callback_context(
            &plugin.key(),
            context,
            call_with_timeout(&plugin, method, params, timeout),
        )
        .await
        .map_err(|error| error.to_string())
}

fn install_guard<I, P>(
    pipeline: &PluginGuardPipeline<I>,
    plugins: &[Arc<LoadedPlugin>],
    handle: &Arc<HostHandle>,
    subscription: HookSubscription,
    method: &'static str,
    reason: fn(P) -> Option<String>,
    timeout: Duration,
) -> Result<(), HostError>
where
    I: SessionEvent,
    P: DeserializeOwned + Send + 'static,
{
    for plugin in plugins {
        if !plugin.subscribes(subscription) {
            continue;
        }
        let scope = scope_for(plugin, handle)?;
        let plugin = Arc::clone(plugin);
        let authority = Arc::clone(handle);
        pipeline
            .register(
                &scope,
                0,
                format!("{} {method}", plugin.key()),
                move |input| {
                    let plugin = Arc::clone(&plugin);
                    let authority = Arc::clone(&authority);
                    async move {
                        let raw = invoke(plugin, authority, method, input, timeout).await?;
                        let patch: Option<P> =
                            serde_json::from_value(raw).map_err(|err| err.to_string())?;
                        Ok(match patch.and_then(reason) {
                            Some(reason) => PluginGuardDecision::Deny {
                                code: "plugin_policy".into(),
                                reason,
                            },
                            None => PluginGuardDecision::Abstain,
                        })
                    }
                },
            )
            .map_err(|err| HostError::Config(err.to_string()))?;
    }
    Ok(())
}

fn install_observer<I: SessionEvent>(
    pipeline: &PluginObservePipeline<I>,
    plugins: &[Arc<LoadedPlugin>],
    handle: &Arc<HostHandle>,
    subscription: HookSubscription,
    method: &'static str,
    timeout: Duration,
) -> Result<(), HostError> {
    for plugin in plugins {
        if !plugin.subscribes(subscription) {
            continue;
        }
        let scope = scope_for(plugin, handle)?;
        let plugin = Arc::clone(plugin);
        let authority = Arc::clone(handle);
        pipeline
            .register(
                &scope,
                0,
                format!("{} {method}", plugin.key()),
                move |input| {
                    let plugin = Arc::clone(&plugin);
                    let authority = Arc::clone(&authority);
                    async move {
                        invoke(plugin, authority, method, input, timeout)
                            .await
                            .map(|_| ())
                    }
                },
            )
            .map_err(|err| HostError::Config(err.to_string()))?;
    }
    Ok(())
}

fn install_error_recovery(
    pipeline: &PluginBailPipeline<ModelRequestErrorInput, bool>,
    plugins: &[Arc<LoadedPlugin>],
    handle: &Arc<HostHandle>,
    timeout: Duration,
) -> Result<(), HostError> {
    for plugin in plugins {
        if !plugin.subscribes(HookSubscription::MODEL_REQUEST_ERROR) {
            continue;
        }
        let scope = scope_for(plugin, handle)?;
        let plugin = Arc::clone(plugin);
        let authority = Arc::clone(handle);
        pipeline
            .register(
                &scope,
                0,
                format!("{} model.request.error", plugin.key()),
                move |input| {
                    let plugin = Arc::clone(&plugin);
                    let authority = Arc::clone(&authority);
                    async move {
                        let raw = invoke(
                            plugin,
                            authority,
                            method::HOOK_MODEL_REQUEST_ERROR,
                            input,
                            timeout,
                        )
                        .await?;
                        let patch: Option<ModelRequestErrorPatch> =
                            serde_json::from_value(raw).map_err(|error| error.to_string())?;
                        Ok(patch.and_then(|patch| patch.retry.then_some(true)))
                    }
                },
            )
            .map_err(|err| HostError::Config(err.to_string()))?;
    }
    Ok(())
}

/// The current plugin host's owned runtime lifecycle hooks. All handlers live
/// inside plugin effect scopes and are removed on generation disposal.
pub(super) struct LifecycleHooks {
    pub model_before: PluginGuardPipeline<ModelRequestBeforeInput>,
    pub model_after: PluginObservePipeline<ModelRequestAfterInput>,
    pub model_error: PluginBailPipeline<ModelRequestErrorInput, bool>,
    pub compaction_before: PluginGuardPipeline<CompactionBeforeInput>,
    pub compaction_after: PluginObservePipeline<CompactionAfterInput>,
    pub subagent_start: PluginGuardPipeline<SubagentStartInput>,
    pub subagent_stop: PluginObservePipeline<SubagentStopInput>,
    pub permission_request: PluginGuardPipeline<PermissionRequestInput>,
}

impl LifecycleHooks {
    pub fn empty() -> Self {
        Self {
            model_before: PluginGuardPipeline::new(PluginGuardErrorPolicy::Deny),
            model_after: PluginObservePipeline::new(true),
            model_error: PluginBailPipeline::new(),
            compaction_before: PluginGuardPipeline::new(PluginGuardErrorPolicy::Deny),
            compaction_after: PluginObservePipeline::new(true),
            subagent_start: PluginGuardPipeline::new(PluginGuardErrorPolicy::Deny),
            subagent_stop: PluginObservePipeline::new(true),
            permission_request: PluginGuardPipeline::new(PluginGuardErrorPolicy::Deny),
        }
    }

    pub fn build(
        plugins: &[Arc<LoadedPlugin>],
        handle: &Arc<HostHandle>,
        timeout: Duration,
    ) -> Result<Self, HostError> {
        let hooks = Self::empty();
        install_guard::<_, ModelRequestBeforePatch>(
            &hooks.model_before,
            plugins,
            handle,
            HookSubscription::MODEL_REQUEST_BEFORE,
            method::HOOK_MODEL_REQUEST_BEFORE,
            |patch| patch.deny_reason,
            timeout,
        )?;
        install_observer(
            &hooks.model_after,
            plugins,
            handle,
            HookSubscription::MODEL_REQUEST_AFTER,
            method::HOOK_MODEL_REQUEST_AFTER,
            timeout,
        )?;
        install_error_recovery(&hooks.model_error, plugins, handle, timeout)?;
        install_guard::<_, CompactionBeforePatch>(
            &hooks.compaction_before,
            plugins,
            handle,
            HookSubscription::COMPACTION_BEFORE,
            method::HOOK_COMPACTION_BEFORE,
            |patch| patch.deny_reason,
            timeout,
        )?;
        install_observer(
            &hooks.compaction_after,
            plugins,
            handle,
            HookSubscription::COMPACTION_AFTER,
            method::HOOK_COMPACTION_AFTER,
            timeout,
        )?;
        install_guard::<_, SubagentStartPatch>(
            &hooks.subagent_start,
            plugins,
            handle,
            HookSubscription::SUBAGENT_START,
            method::HOOK_SUBAGENT_START,
            |patch| patch.deny_reason,
            timeout,
        )?;
        install_observer(
            &hooks.subagent_stop,
            plugins,
            handle,
            HookSubscription::SUBAGENT_STOP,
            method::HOOK_SUBAGENT_STOP,
            timeout,
        )?;
        install_guard::<_, PermissionRequestPatch>(
            &hooks.permission_request,
            plugins,
            handle,
            HookSubscription::PERMISSION_REQUEST,
            method::HOOK_PERMISSION_REQUEST,
            |patch| patch.deny_reason,
            timeout,
        )?;
        Ok(hooks)
    }
}
