use std::sync::Arc;

use tokio::sync::{Semaphore, watch};

use super::*;

const STATUS: &str = "agena.mcp.servers.status";
static OBSERVERS: Semaphore = Semaphore::const_new(256);

impl RuntimeHostClient {
    pub(in crate::runtime::host_client) async fn subscribe_deepseek_mcp(
        &self,
        cancellation: CancellationToken,
    ) -> Result<watch::Receiver<Value>> {
        let session = self
            .callback_context()?
            .session_id
            .filter(|id| *id > 0)
            .ok_or_else(|| host_unavailable("Cordis MCP listeners require a project session"))?;
        let plugin = self.callback_plugin_key()?;
        let runtime = self.runtime()?;
        let snapshot = runtime.current_snapshot();
        let generation = snapshot.generation();
        let cleanup = runtime.deepseek_mcp_cleanup.clone();
        let sessions = snapshot
            .session_manager()
            .ok_or_else(|| host_unavailable("the DeepSeek MCP manager requires sessions"))?;
        let mcp = snapshot
            .mcp_manager()
            .ok_or_else(|| host_unavailable("the Agena MCP bridge is disabled"))?;
        let plugins = snapshot.plugin_manager();
        let scope = plugins
            .plugins()
            .iter()
            .find(|loaded| {
                loaded.key() == plugin
                    && loaded
                        .agent_plugin()
                        .is_some_and(|package| package.format == AgentPluginFormat::DeepSeekHarness)
            })
            .map(|loaded| loaded.effect_scope().cancellation_token())
            .ok_or_else(|| host_unavailable("this callback requires an active DeepSeek bundle"))?;
        let admission = OBSERVERS
            .try_acquire()
            .map_err(|_| host_unavailable("Cordis MCP listener limit reached"))?;
        let session_id = session.try_into().map_err(PluginError::invalid_params)?;
        let subscribe = || {
            sessions
                .kernel()
                .reader
                .watch(session_id, None)
                .map_err(|error| PluginError::internal_error(&error))
        };
        let mut session_changes = subscribe()?;
        let mut observations = mcp.observe_plugin_connections();
        let mut retirements = cleanup.watch();
        let revision = observations
            .borrow_and_update()
            .saturating_add(*retirements.borrow_and_update());
        let executor = sessions
            .authorize_deepseek_mcp_manager(session, &plugin, STATUS)
            .await
            .map_err(plugin_error_from_app)?;
        if !Arc::ptr_eq(executor.plugin_manager(), &plugins) || snapshot.replaced.is_cancelled() {
            return Err(host_unavailable(
                "the DeepSeek MCP manager configuration changed",
            ));
        }
        let views = manager_views(
            &mcp,
            &managed_servers(plugins.plugins()),
            &executor,
            session,
            "",
            &cleanup,
            generation,
        )
        .await;
        let (updates, receiver) = watch::channel(json!({ "views": views, "revision": revision }));
        let replaced = snapshot.replaced.clone();
        tokio::spawn(async move {
            let _admission = admission;
            let declarations = managed_servers(plugins.plugins());
            let subscribe = || sessions.kernel().reader.watch(session_id, None);
            let mut previous = views;
            loop {
                tokio::select! {
                    biased;
                    _ = replaced.cancelled() => break,
                    _ = cancellation.cancelled() => break,
                    _ = scope.cancelled() => break,
                    _ = updates.closed() => break,
                    changed = observations.changed() => {
                        if changed.is_err() {
                            break;
                        }
                    }
                    changed = retirements.changed() => {
                        if changed.is_err() {
                            break;
                        }
                    }
                    changed = next_catalog_change(&mut session_changes) => match changed {
                        Ok(change) if session_was_deleted(&change) => break,
                        Ok(_) => {}
                        Err(WatchError::Reset { .. }) => match subscribe() {
                            Ok(watch) => session_changes = watch,
                            Err(_) => break,
                        },
                        Err(WatchError::Closed) => break,
                    },
                }
                let revision = observations
                    .borrow_and_update()
                    .saturating_add(*retirements.borrow_and_update());
                let authorization = tokio::select! {
                    biased;
                    _ = replaced.cancelled() => break,
                    _ = cancellation.cancelled() => break,
                    _ = scope.cancelled() => break,
                    _ = updates.closed() => break,
                    result = async {
                        let _permit = BACKGROUND_POLICY_CHECKS.acquire().await.ok()?;
                        sessions.authorize_deepseek_mcp_manager(session, &plugin, STATUS).await.ok()
                    } => result,
                };
                match authorization {
                    Some(executor) if Arc::ptr_eq(executor.plugin_manager(), &plugins) => {
                        let views = manager_views(
                            &mcp,
                            &declarations,
                            &executor,
                            session,
                            "",
                            &cleanup,
                            generation,
                        )
                        .await;
                        if views != previous {
                            updates.send_replace(json!({ "views": views, "revision": revision }));
                            previous = views;
                        }
                    }
                    _ => break,
                }
            }
        });
        Ok(receiver)
    }
}
