//! Dynamic callbacks retain their admitted owner: a process child scope for
//! stdio, or the logical scope for a scoped host client. The task-local binding
//! also identifies its host so nested work cannot borrow another host's owner.

use super::*;
use crate::scoped_registry::{PluginScopeKey, ScopedRegistryLayer};

tokio::task_local! {
    static CALLBACK_EFFECT_OWNER: CallbackEffectOwner;
}

struct CallbackEffectOwner {
    host: std::sync::Weak<HostHandle>,
    scope: Arc<PluginEffectScope>,
}

#[cfg(test)]
mod ownership_tests;

#[cfg(test)]
mod tests {
    use super::*;

    fn visible(host: &HostHandle) -> serde_json::Value {
        serde_json::json!({
            "tools": host.registered_tool_list_response().unwrap().tools,
            "display": host.display_list_response(),
            "themes": host.theme_list_response().themes,
        })
    }

    async fn rejects_late_registration(
        method: &str,
        before: serde_json::Value,
        late: serde_json::Value,
    ) {
        let key: PluginKey = "test.ownership".parse().unwrap();
        let host = Arc::new(HostHandle::new_with_registry(
            Arc::new(crate::sdk::host_api::NoopHostClient),
            Arc::new(RwLock::new(PluginToolRegistry::new())),
            Arc::new(RwLock::new(HashMap::from([(key.clone(), 0)]))),
        ));
        let logical = host.begin_plugin_instance(key.clone());
        let process = PluginEffectScope::new(key.clone());
        logical.own_child(process.clone()).unwrap();
        host.run_in_callback_effect_scope(
            process.clone(),
            host.handle_call_for_plugin(&key.to_string(), method, before),
        )
        .await
        .unwrap();
        let original = visible(&host);

        // An already accepted callback can still be in a synchronous section
        // when another task closes admission. Cleanup waits for its lease.
        let accepted = process.lease().unwrap();
        let cleanup = tokio::spawn({
            let process = process.clone();
            async move { process.dispose().await }
        });
        while process.is_accepting() {
            tokio::task::yield_now().await;
        }
        let result = host
            .run_in_callback_effect_scope(
                process.clone(),
                host.handle_call_for_plugin(&key.to_string(), method, late),
            )
            .await;
        let after_attempt = visible(&host);
        drop(accepted);
        let report = cleanup.await.unwrap();
        let after_disposal = visible(&host);
        assert!(
            result.is_err(),
            "stopped registration must be reported to the plugin"
        );
        assert_eq!(
            after_attempt, original,
            "rejected registration must not change visible state"
        );
        assert!(report.errors.is_empty());
        assert_eq!(
            after_disposal,
            serde_json::json!({"tools": [], "display": [], "themes": []})
        );
    }

    #[tokio::test]
    async fn tool_registration_during_disposal_keeps_the_previous_owner() {
        let tool = |summary| {
            serde_json::json!({"request": {"tool": {
                "name": "shared", "docs": {"summary": summary},
                "contract": {"input_schema": {"type": "object"}}
            }}})
        };
        rejects_late_registration("host/tool.registry.register", tool("before"), tool("late"))
            .await;
    }

    #[tokio::test]
    async fn display_registration_during_disposal_keeps_the_previous_owner() {
        let display = |text| {
            serde_json::json!({"request": {"contribution": {
                "id": "shared", "kind": "status_line_text", "priority": 0,
                "content": {"kind": "text", "text": text}
            }}})
        };
        rejects_late_registration(
            "host/ui.display.contribute",
            display("before"),
            display("late"),
        )
        .await;
    }

    #[tokio::test]
    async fn theme_registration_during_disposal_keeps_the_previous_owner() {
        let theme = |name| {
            serde_json::json!({"request": {
                "id": "shared", "display_name": name, "colors": {}
            }})
        };
        rejects_late_registration("host/ui.theme.register", theme("before"), theme("late")).await;
    }
}

#[derive(Default)]
pub(crate) struct ProcessContributions {
    tools: Vec<crate::sdk::ToolDefinition>,
    scoped_tools: Vec<(PluginScopeKey, crate::sdk::ToolDefinition)>,
    parents: BTreeMap<PluginScopeKey, PluginScopeKey>,
    display: Vec<HostDisplayContribution>,
    themes: Vec<HostThemePalette>,
}

impl HostHandle {
    pub(super) async fn run_in_callback_effect_scope<T>(
        self: &Arc<Self>,
        scope: Arc<PluginEffectScope>,
        future: impl std::future::Future<Output = T>,
    ) -> T {
        CALLBACK_EFFECT_OWNER
            .scope(
                CallbackEffectOwner {
                    host: Arc::downgrade(self),
                    scope,
                },
                future,
            )
            .await
    }

    pub(super) fn callback_effect_owner(
        &self,
        plugin_id: &PluginKey,
    ) -> Option<Arc<PluginEffectScope>> {
        CALLBACK_EFFECT_OWNER
            .try_with(|owner| {
                (std::ptr::eq(owner.host.as_ptr(), self) && owner.scope.plugin_id() == plugin_id)
                    .then(|| Arc::clone(&owner.scope))
            })
            .ok()
            .flatten()
    }

    pub(crate) fn host_handler_for_process(
        self: &Arc<Self>,
        logical: Arc<PluginEffectScope>,
        process: Arc<PluginEffectScope>,
    ) -> crate::transport::stdio::HostHandler {
        let weak = Arc::downgrade(self);
        let logical = Arc::downgrade(&logical);
        Arc::new(move |method, params| {
            let weak = weak.clone();
            let logical = logical.clone();
            let process = Arc::clone(&process);
            Box::pin(async move {
                let host = weak
                    .upgrade()
                    .ok_or_else(|| PluginError::internal("plugin host has been dropped"))?;
                let logical = logical.upgrade().ok_or_else(|| {
                    PluginError::internal("plugin logical owner has been dropped")
                })?;
                if !logical.is_accepting()
                    || !host.is_current_effect_scope(logical.plugin_id(), &logical)
                {
                    return Err(PluginError::internal(
                        "plugin callback belongs to a stale host generation",
                    ));
                }
                let _lease = process
                    .lease()
                    .map_err(|error| PluginError::internal_error(&error))?;
                let stop = process.cancellation_token();
                let plugin_id = logical.plugin_id().to_string();
                let callback = host.run_in_callback_effect_scope(
                    Arc::clone(&process),
                    host.handle_call_for_plugin(&plugin_id, &method, params),
                );
                tokio::select! {
                    biased;
                    _ = stop.cancelled() => Err(PluginError::internal("plugin process callbacks have stopped")),
                    result = callback => result,
                }
            })
        })
    }

    pub(crate) fn process_contributions(&self, owner: &PluginEffectScope) -> ProcessContributions {
        let tools = self
            .tool_registry
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .tools_owned_by(owner);
        let mut snapshot = ProcessContributions {
            tools,
            ..Default::default()
        };
        for (_, value) in self.scoped_tools.owned_entries(owner) {
            if let ScopedRegistryLayer::Scope { scope } = value.layer {
                let mut child = scope.clone();
                while let Some(parent) = self.scoped_tools.parent(&child) {
                    snapshot.parents.insert(child, parent.clone());
                    child = parent;
                }
                snapshot.scoped_tools.push((scope, value.value.definition));
            }
        }
        snapshot.display = self.display.owned_values(owner);
        snapshot.themes = self.themes.owned_values(owner);
        snapshot
    }

    pub(crate) async fn import_process_contributions(
        self: &Arc<Self>,
        owner: Arc<PluginEffectScope>,
        snapshot: ProcessContributions,
    ) -> Result<(), PluginError> {
        let plugin_id = owner.plugin_id().to_string();
        self.run_in_callback_effect_scope(owner, async {
            for (scope, parent) in snapshot.parents {
                self.scoped_tools
                    .set_parent(scope, parent)
                    .map_err(|error| PluginError::internal_error(&error))?;
            }
            for tool in snapshot.tools {
                self.tool_upsert_for_plugin(&plugin_id, tool)?;
            }
            for (scope, tool) in snapshot.scoped_tools {
                let owner = self
                    .callback_effect_owner(&plugin_id.parse()?)
                    .expect("process scope installed");
                let registered = RegisteredTool::new(owner.plugin_id().clone(), tool)
                    .map_err(PluginError::invalid_params)?;
                let key = registered.tool_key().clone();
                let label = format!("{scope}:{}", registered.tool_name());
                self.scoped_tools
                    .replace_owned(&owner, Some(scope), key, registered, "host.tool", label)
                    .map_err(|error| PluginError::internal_error(&error))?;
            }
            for value in snapshot.display {
                self.display_contribute(
                    &plugin_id,
                    HostDisplayContributeRequest {
                        contribution: value.contribution,
                    },
                )?;
            }
            for value in snapshot.themes {
                self.theme_register(
                    &plugin_id,
                    HostThemeRegisterRequest {
                        id: value.id,
                        display_name: value.display_name,
                        colors: value.colors,
                    },
                )?;
            }
            Ok(())
        })
        .await
    }

    pub(crate) fn restore_manifest_tools(
        &self,
        plugin_id: &PluginKey,
        manifest: &PluginManifest,
    ) -> Result<(), PluginError> {
        let owner = self
            .effect_scope(plugin_id)
            .ok_or_else(|| PluginError::internal("manifest tools have no logical owner"))?;
        let _lease = owner
            .lease()
            .map_err(|error| PluginError::internal_error(&error))?;
        let mut registry = self
            .tool_registry
            .write()
            .unwrap_or_else(|error| error.into_inner());
        registry
            .extend_from_plugin(plugin_id, &manifest.tools)
            .map_err(PluginError::invalid_params)?;
        self.own_manifest_tools(&owner, manifest, &mut registry)
    }
}
