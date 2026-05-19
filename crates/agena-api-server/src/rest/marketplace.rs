use super::*;

pub async fn search_marketplace_plugins(
    State(_state): State<AppState>,
    Json(request): Json<MarketplaceSearchRequestBody>,
) -> Result<impl IntoResponse, ServerError> {
    let registry_id = request
        .registry_id
        .clone()
        .unwrap_or_else(|| "default".to_string());
    let registry_url = request.registry_url.trim().to_string();
    if registry_url.is_empty() {
        return Err(ServerError::BadRequest(
            "registry_url cannot be empty".to_string(),
        ));
    }

    let cache = agena_plugin_marketplace::MarketplaceCache::new(
        agena_plugin_marketplace::default_cache_root(),
    );
    let client = agena_plugin_marketplace::MarketplaceClient::with_default_fetcher(
        cache,
        std::collections::BTreeMap::new(),
    );
    let registry = client.registry(agena_plugin_marketplace::RegistrySpec {
        id: registry_id.clone(),
        url: registry_url.clone(),
        require_signature: false,
    });
    let index = registry
        .fetch_index(request.refresh)
        .map_err(|error| ServerError::BadRequest(error.to_string()))?;
    let needle = request
        .query
        .as_deref()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();

    let mut entries = index
        .plugins
        .into_iter()
        .filter(|plugin| {
            if needle.is_empty() {
                return true;
            }
            let blob = format!("{} {} {}", plugin.id, plugin.name, plugin.description)
                .to_ascii_lowercase();
            blob.contains(&needle)
        })
        .map(|plugin| {
            let latest = plugin.versions.iter().max_by(|left, right| {
                let left_semver = semver::Version::parse(&left.version).ok();
                let right_semver = semver::Version::parse(&right.version).ok();
                match (left_semver, right_semver) {
                    (Some(left_version), Some(right_version)) => left_version.cmp(&right_version),
                    _ => left.version.cmp(&right.version),
                }
            });
            MarketplacePluginResource {
                plugin_id: plugin.id,
                name: plugin.name,
                description: plugin.description,
                homepage: plugin.homepage,
                version_count: plugin.versions.len(),
                latest_version: latest.map(|version| version.version.clone()),
                latest_kind: latest.map(|version| version.kind.as_str().to_string()),
                latest_platform: latest.map(|version| version.platform.clone()),
            }
        })
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| left.plugin_id.cmp(&right.plugin_id));

    Ok(Json(MarketplaceSearchResponse {
        registry_id,
        registry_url,
        entries,
    }))
}

pub async fn sync_marketplace_registry(
    State(_state): State<AppState>,
    Json(request): Json<MarketplaceRegistryRequestBody>,
) -> Result<impl IntoResponse, ServerError> {
    let registry_id = request
        .registry_id
        .clone()
        .unwrap_or_else(|| "default".to_string());
    let registry_url = request.registry_url.trim().to_string();
    if registry_url.is_empty() {
        return Err(ServerError::BadRequest(
            "registry_url cannot be empty".to_string(),
        ));
    }

    let cache = agena_plugin_marketplace::MarketplaceCache::new(
        agena_plugin_marketplace::default_cache_root(),
    );
    let client = agena_plugin_marketplace::MarketplaceClient::with_default_fetcher(
        cache,
        std::collections::BTreeMap::new(),
    );
    let registry = client.registry(agena_plugin_marketplace::RegistrySpec {
        id: registry_id.clone(),
        url: registry_url.clone(),
        require_signature: false,
    });
    let index = registry
        .fetch_index(true)
        .map_err(|error| ServerError::BadRequest(error.to_string()))?;

    Ok(Json(MarketplaceSyncResponse {
        registry_id,
        registry_url,
        plugin_count: index.plugins.len(),
    }))
}

pub async fn list_marketplace_installed_plugins(
    State(_state): State<AppState>,
) -> Result<impl IntoResponse, ServerError> {
    let cache = agena_plugin_marketplace::MarketplaceCache::new(
        agena_plugin_marketplace::default_cache_root(),
    );
    let client = agena_plugin_marketplace::MarketplaceClient::with_default_fetcher(
        cache,
        std::collections::BTreeMap::new(),
    );
    let mut entries = client
        .list_installed()
        .map_err(|error| ServerError::BadRequest(error.to_string()))?
        .into_iter()
        .map(|record| MarketplaceInstalledPluginResource {
            plugin_id: record.plugin_id,
            version: record.version,
            kind: record.kind.as_str().to_string(),
            platform: record.platform,
            binary_path: record.binary_path.display().to_string(),
            config_path: record.config_path.display().to_string(),
            sha256: record.sha256,
            installed_at: record.installed_at,
            registry_id: record.registry_id,
            registry_url: record.registry_url,
            archive_extracted: record.archive_extracted,
        })
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| left.plugin_id.cmp(&right.plugin_id));
    Ok(Json(MarketplaceInstalledListResponse { entries }))
}

pub async fn list_marketplace_outdated_plugins(
    State(_state): State<AppState>,
) -> Result<impl IntoResponse, ServerError> {
    let cache = agena_plugin_marketplace::MarketplaceCache::new(
        agena_plugin_marketplace::default_cache_root(),
    );
    let client = agena_plugin_marketplace::MarketplaceClient::with_default_fetcher(
        cache,
        std::collections::BTreeMap::new(),
    );
    let entries = client
        .list_outdated()
        .map_err(|error| ServerError::BadRequest(error.to_string()))?
        .into_iter()
        .map(|record| MarketplaceOutdatedPluginResource {
            plugin_id: record.plugin_id,
            installed_version: record.installed_version,
            latest_version: record.latest_version,
        })
        .collect::<Vec<_>>();
    Ok(Json(MarketplaceOutdatedListResponse { entries }))
}

pub async fn install_marketplace_plugin(
    State(state): State<AppState>,
    Json(request): Json<MarketplaceInstallRequestBody>,
) -> Result<impl IntoResponse, ServerError> {
    let registry_url = request.registry_url.trim().to_string();
    if registry_url.is_empty() {
        return Err(ServerError::BadRequest(
            "registry_url cannot be empty".to_string(),
        ));
    }
    let spec = request.spec.trim().to_string();
    if spec.is_empty() {
        return Err(ServerError::BadRequest("spec cannot be empty".to_string()));
    }
    let (plugin_id, version) = match spec.split_once('@') {
        Some((id, ver)) => (id.to_string(), Some(ver.to_string())),
        None => (spec, None),
    };
    let registry_id = request
        .registry_id
        .clone()
        .unwrap_or_else(|| "default".to_string());
    let config_path = request
        .config_path
        .clone()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            state
                .runtime()
                .current_snapshot()
                .config_resolution()
                .meta
                .config_path
                .clone()
        });

    let cache = agena_plugin_marketplace::MarketplaceCache::new(
        agena_plugin_marketplace::default_cache_root(),
    );
    let client = agena_plugin_marketplace::MarketplaceClient::with_default_fetcher(
        cache,
        std::collections::BTreeMap::new(),
    );
    let outcome = client
        .install(agena_plugin_marketplace::InstallRequest {
            registry: agena_plugin_marketplace::RegistrySpec {
                id: registry_id,
                url: registry_url,
                require_signature: request.require_signature,
            },
            plugin_id,
            version,
            config_path,
            force: request.force,
            dry_run: request.dry_run,
            allow_unverified: request.allow_unverified,
            refresh_index: request.refresh,
        })
        .map_err(|error| ServerError::BadRequest(error.to_string()))?;

    Ok(Json(MarketplaceInstallOutcomeResource {
        plugin_id: outcome.plugin_id,
        version: outcome.version,
        kind: outcome.kind.as_str().to_string(),
        artifact_path: outcome.artifact_path.display().to_string(),
        config_path: outcome.config_path.display().to_string(),
        dry_run: outcome.dry_run,
    }))
}

pub async fn uninstall_marketplace_plugin(
    State(_state): State<AppState>,
    Json(request): Json<MarketplaceUninstallRequestBody>,
) -> Result<impl IntoResponse, ServerError> {
    let plugin_id = request.plugin_id.trim().to_string();
    if plugin_id.is_empty() {
        return Err(ServerError::BadRequest(
            "plugin_id cannot be empty".to_string(),
        ));
    }
    let cache = agena_plugin_marketplace::MarketplaceCache::new(
        agena_plugin_marketplace::default_cache_root(),
    );
    let client = agena_plugin_marketplace::MarketplaceClient::with_default_fetcher(
        cache,
        std::collections::BTreeMap::new(),
    );
    let entries = client
        .uninstall_with(plugin_id.as_str(), request.cascade)
        .map_err(|error| ServerError::BadRequest(error.to_string()))?
        .into_iter()
        .map(|outcome| MarketplaceUninstallOutcomeResource {
            plugin_id: outcome.plugin_id,
            version: outcome.version,
            config_path: outcome.config_path.display().to_string(),
        })
        .collect::<Vec<_>>();
    Ok(Json(MarketplaceUninstallResponse { entries }))
}

pub async fn upgrade_marketplace_plugins(
    State(_state): State<AppState>,
    Json(request): Json<MarketplaceUpgradeRequestBody>,
) -> Result<impl IntoResponse, ServerError> {
    if !request.all && request.plugin_id.as_deref().unwrap_or("").trim().is_empty() {
        return Err(ServerError::BadRequest(
            "plugin_id is required unless all=true".to_string(),
        ));
    }
    let cache = agena_plugin_marketplace::MarketplaceCache::new(
        agena_plugin_marketplace::default_cache_root(),
    );
    let client = agena_plugin_marketplace::MarketplaceClient::with_default_fetcher(
        cache,
        std::collections::BTreeMap::new(),
    );
    let override_spec =
        request
            .registry_url
            .as_ref()
            .map(|registry_url| agena_plugin_marketplace::RegistrySpec {
                id: request
                    .registry_id
                    .clone()
                    .unwrap_or_else(|| "default".to_string()),
                url: registry_url.trim().to_string(),
                require_signature: false,
            });

    let targets = if request.all {
        client
            .list_installed()
            .map_err(|error| ServerError::BadRequest(error.to_string()))?
            .into_iter()
            .map(|record| record.plugin_id)
            .collect::<Vec<_>>()
    } else {
        vec![
            request
                .plugin_id
                .clone()
                .unwrap_or_default()
                .trim()
                .to_string(),
        ]
    };

    let mut entries = Vec::new();
    for plugin_id in targets.into_iter().filter(|item| !item.is_empty()) {
        let outcome = client
            .upgrade(plugin_id.as_str(), override_spec.clone())
            .map_err(|error| ServerError::BadRequest(error.to_string()))?;
        entries.push(MarketplaceUpgradeOutcomeResource {
            plugin_id: outcome.plugin_id,
            previous_version: outcome.previous_version,
            installed_version: outcome.installed_version,
            upgraded: outcome.upgraded,
            outcome: outcome
                .outcome
                .map(|inner| MarketplaceInstallOutcomeResource {
                    plugin_id: inner.plugin_id,
                    version: inner.version,
                    kind: inner.kind.as_str().to_string(),
                    artifact_path: inner.artifact_path.display().to_string(),
                    config_path: inner.config_path.display().to_string(),
                    dry_run: inner.dry_run,
                }),
        });
    }

    Ok(Json(MarketplaceUpgradeResponse { entries }))
}
