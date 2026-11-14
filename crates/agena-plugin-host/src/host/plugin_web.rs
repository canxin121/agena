//! Native page catalog and generation-bound calls on one host snapshot.

use super::*;
use crate::sdk::inspection::PluginWebPageCatalogItem;
use crate::sdk::{PluginWebPage, PluginWebScope, SessionKind};
use crate::web::{PluginWebAsset, PluginWebBundle, PluginWebError};

impl LoadedPlugin {
    pub(crate) async fn initialize_web_assets(&self) -> Result<(), PluginError> {
        let Some(web) = self.manifest.surface.web.as_ref() else {
            return Ok(());
        };
        if self.web_bundle.get().is_some() {
            return Ok(());
        }
        web.validate(&self.manifest.commands, self.manifest.settings.is_some())
            .map_err(PluginError::invalid_params)?;
        let web = web.clone();
        let root = self.configured_plugin.package_root.clone();
        let bundle =
            tokio::task::spawn_blocking(move || PluginWebBundle::load(root.as_deref(), &web))
                .await
                .map_err(|error| {
                    PluginError::internal(format!("Web resource worker failed: {error}"))
                })?
                .map_err(PluginError::invalid_params)?;
        self.web_bundle
            .set(Arc::new(bundle))
            .map_err(|_| PluginError::internal("Web resources were initialized more than once"))
    }
}

impl PluginHost {
    fn active_web_plugin(
        &self,
        plugin_id: &str,
    ) -> Result<(Arc<LoadedPlugin>, String), PluginWebError> {
        let key: PluginKey = plugin_id.parse().map_err(|_| PluginWebError::NotFound)?;
        let plugin = self
            .plugins_by_id
            .get(&key)
            .ok_or(PluginWebError::NotFound)?;
        if !plugin.effect_scope.is_accepting() || plugin.web_bundle.get().is_none() {
            return Err(PluginWebError::NotFound);
        }
        let status = self.statuses.get(&key).ok_or(PluginWebError::NotFound)?;
        if status.state != crate::status::PluginRunState::Running {
            return Err(PluginWebError::NotFound);
        }
        // The UUID changes on host rebinding and process restart changes the
        // suffix, so a retired page cannot target a fresh backend incarnation.
        let generation = format!("{}:{}", plugin.web_generation, status.restart_count);
        Ok((Arc::clone(plugin), generation))
    }

    fn checked_web_page(
        &self,
        plugin_id: &str,
        page_id: &str,
        generation: &str,
    ) -> Result<(Arc<LoadedPlugin>, PluginWebPage), PluginWebError> {
        let (plugin, current) = self.active_web_plugin(plugin_id)?;
        if generation != current {
            return Err(PluginWebError::StaleGeneration);
        }
        let page = plugin
            .manifest
            .surface
            .web
            .as_ref()
            .and_then(|web| web.pages.iter().find(|page| page.id == page_id))
            .cloned()
            .ok_or(PluginWebError::NotFound)?;
        Ok((plugin, page))
    }

    pub fn web_page_catalog(&self) -> Vec<PluginWebPageCatalogItem> {
        let mut pages = Vec::new();
        for candidate in &self.plugins {
            let plugin_id = candidate.key();
            let Ok((plugin, generation)) = self.active_web_plugin(&plugin_id.to_string()) else {
                continue;
            };
            let Some(web) = &plugin.manifest.surface.web else {
                continue;
            };
            let Some(bundle) = plugin.web_bundle.get() else {
                continue;
            };
            let prefix = format!("/plugin-assets/{plugin_id}/{}/", bundle.revision);
            for page in &web.pages {
                pages.push(PluginWebPageCatalogItem {
                    plugin_id: plugin_id.clone(),
                    page_id: page.id.clone(),
                    title: page.title.clone(),
                    api_version: web.api_version,
                    scope: page.scope,
                    entry_url: format!("{prefix}{}", page.entry),
                    style_urls: page
                        .styles
                        .iter()
                        .map(|path| format!("{prefix}{path}"))
                        .collect(),
                    commands: page.commands.clone(),
                    settings: page.settings,
                    revision: bundle.revision.clone(),
                    generation: generation.clone(),
                });
            }
        }
        pages.sort_by(|left, right| {
            (&left.plugin_id, &left.page_id).cmp(&(&right.plugin_id, &right.page_id))
        });
        pages
    }

    /// Resource paths are exact keys in the frozen package snapshot. No HTTP
    /// request can select a host filesystem path or a different revision.
    pub fn web_asset(
        &self,
        plugin_id: &str,
        revision: &str,
        path: &str,
    ) -> Result<PluginWebAsset, PluginWebError> {
        if !crate::sdk::web::valid_web_asset_path(path) {
            return Err(PluginWebError::NotFound);
        }
        let (plugin, _) = self.active_web_plugin(plugin_id)?;
        let bundle = plugin.web_bundle.get().ok_or(PluginWebError::NotFound)?;
        if revision != bundle.revision {
            return Err(PluginWebError::NotFound);
        }
        bundle
            .assets
            .get(path)
            .cloned()
            .ok_or(PluginWebError::NotFound)
    }

    /// Return settings inspection from the same generation that authorized
    /// the page; callers must not re-resolve an unguarded runtime snapshot.
    pub fn web_settings_inspect(
        &self,
        plugin_id: &str,
        page_id: &str,
        generation: &str,
    ) -> Result<PluginInspect, PluginWebError> {
        let (_, page) = self.checked_web_page(plugin_id, page_id, generation)?;
        if !page.settings {
            return Err(PluginWebError::Forbidden);
        }
        self.plugin_inspect(plugin_id)
            .ok_or(PluginWebError::NotFound)
    }

    /// Validate and invoke a page's Method command while retaining the same
    /// plugin host, effect lease and cancellation token throughout the call.
    pub async fn invoke_web_command(
        &self,
        plugin_id: &str,
        page_id: &str,
        generation: &str,
        input: CommandInvokeInput,
        session_kind: Option<SessionKind>,
    ) -> Result<CommandResult, PluginWebError> {
        let (plugin, page) = self.checked_web_page(plugin_id, page_id, generation)?;
        if !page.commands.contains(&input.command_id) {
            return Err(PluginWebError::Forbidden);
        }
        match page.scope {
            PluginWebScope::Instance
                if input.session_id.is_some() || input.workspace_root.is_some() =>
            {
                return Err(PluginWebError::InvalidInput(
                    "This page uses plugin-instance data and does not accept a session.".into(),
                ));
            }
            PluginWebScope::Session
                if input.session_id.is_none() || input.workspace_root.is_none() =>
            {
                return Err(PluginWebError::InvalidInput(
                    "Select a session in a project before using this page.".into(),
                ));
            }
            _ => {}
        }
        let definition = self
            .resolve_command(plugin_id, &input.command_id)
            .ok_or(PluginWebError::NotFound)?;
        if !matches!(definition.target, crate::sdk::CommandTarget::Method { .. }) {
            return Err(PluginWebError::Forbidden);
        }
        if let Some(kind) = session_kind
            && !definition.applies_to.contains(kind)
        {
            return Err(PluginWebError::InvalidInput(
                "This command is not available in the selected conversation.".into(),
            ));
        }
        let _lease = plugin
            .effect_scope
            .lease()
            .map_err(|_| PluginWebError::StaleGeneration)?;
        let cancellation = plugin.effect_scope.cancellation_token();
        let mut result = tokio::select! {
            biased;
            _ = cancellation.cancelled() => return Err(PluginWebError::StaleGeneration),
            result = self.invoke_plugin_command_async(plugin_id, input) => result.map_err(|error| PluginWebError::Plugin(Box::new(error)))?,
        };
        if result.title.trim().is_empty() {
            result.title = definition.title;
        }
        Ok(result)
    }
}
