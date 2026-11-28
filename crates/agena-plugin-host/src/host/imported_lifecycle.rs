use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use super::{HookSubscription, PluginHost, PluginKey, dispatcher, method};
use crate::sdk::{
    ImportedConfigChangeInput, ImportedDirectoryAddedInput, ImportedMessageDisplayInput,
};

impl PluginHost {
    pub async fn dispatch_imported_directory_added(
        &self,
        input: ImportedDirectoryAddedInput,
        enabled: impl Fn(&PluginKey) -> bool + Send,
        cancel: &CancellationToken,
    ) {
        let session = input.session_id;
        let Ok(payload) = serde_json::to_value(input) else {
            return;
        };
        for plugin in self
            .imported_hook_observers(HookSubscription::IMPORTED_DIRECTORY_ADDED)
            .filter(|plugin| enabled(&plugin.key()))
        {
            let timeout = plugin
                .agent_plugin()
                .expect("imported hook observer")
                .directory_added_timeout();
            let result = tokio::select! {
                biased;
                _ = cancel.cancelled() => break,
                result = self.call_session_hook(
                    plugin,
                    method::HOOK_IMPORTED_DIRECTORY_ADDED,
                    payload.clone(),
                    self.timeouts.fast_or(timeout),
                    Some(session),
                ) => result,
            };
            if let Err(error) = result {
                tracing::debug!(plugin = %plugin.key(), session, %error, "DirectoryAdded hook failed");
            }
        }
    }

    pub async fn dispatch_imported_message_display(
        &self,
        input: ImportedMessageDisplayInput,
        enabled: impl Fn(&PluginKey) -> bool + Send,
        cancel: &CancellationToken,
    ) -> Option<String> {
        self.dispatch_claude_lifecycle(true, input.session_id, json!(input), enabled, cancel)
            .await
    }

    pub async fn dispatch_imported_config_change(
        &self,
        input: ImportedConfigChangeInput,
        enabled: impl Fn(&PluginKey) -> bool + Send,
        cancel: &CancellationToken,
    ) -> Option<String> {
        self.dispatch_claude_lifecycle(false, input.session_id, json!(input), enabled, cancel)
            .await
    }

    async fn dispatch_claude_lifecycle(
        &self,
        display: bool,
        session: i64,
        input: Value,
        enabled: impl Fn(&PluginKey) -> bool + Send,
        cancel: &CancellationToken,
    ) -> Option<String> {
        let (event, rpc, subscription) = if display {
            (
                "message.display",
                method::HOOK_IMPORTED_MESSAGE_DISPLAY,
                HookSubscription::IMPORTED_MESSAGE_DISPLAY,
            )
        } else {
            (
                "config.change",
                method::HOOK_IMPORTED_CONFIG_CHANGE,
                HookSubscription::IMPORTED_CONFIG_CHANGE,
            )
        };
        let mut output = None;
        for plugin in self
            .imported_hook_observers(subscription)
            .filter(|plugin| enabled(&plugin.key()))
        {
            let timeout = plugin
                .agent_plugin()
                .expect("imported hook observer")
                .claude_lifecycle_timeout(display);
            let result = tokio::select! {
                biased;
                _ = cancel.cancelled() => break,
                result = self.call_session_hook(plugin, rpc, input.clone(), self.timeouts.fast_or(timeout), Some(session)) => result,
            };
            match result {
                Ok(value) => {
                    if let Some(value) = value.as_str().filter(|value| value.len() <= 65_536) {
                        output = Some(value.to_owned());
                        if !display {
                            tracing::debug!(plugin = %plugin.key(), session, "configuration change blocked");
                            break;
                        }
                    }
                }
                Err(error) => self.push_hook_runs(vec![dispatcher::transport_failure_record(
                    event,
                    &plugin.key().to_string(),
                    Some(session),
                    &error,
                )]),
            }
        }
        output
    }
}
