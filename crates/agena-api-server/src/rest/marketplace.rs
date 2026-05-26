use super::*;

pub async fn search_marketplace_plugins(
    State(_): State<AppState>,
    Json(request): Json<MarketplaceSearchRequest>,
) -> Result<impl IntoResponse, ServerError> {
    let registry_id = registry_id_or_default(request.registry.registry_id.as_deref());
    let registry_url = normalize_registry_url(request.registry.registry_url.as_str())?;

    let client = marketplace_client();
    let registry = client.registry(agena_plugin_marketplace::RegistrySpec {
        id: registry_id.clone(),
        url: registry_url.clone(),
        require_signature: false,
    });
    let index = registry
        .fetch_index(request.refresh)
        .map_err(marketplace_bad_request)?;
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
    State(state): State<AppState>,
    Json(request): Json<MarketplaceRegistryRequest>,
) -> Result<impl IntoResponse, ServerError> {
    let registry_id = registry_id_or_default(request.registry_id.as_deref());
    let registry_url = normalize_registry_url(request.registry_url.as_str())?;

    spawn_marketplace_background_task(
        &state,
        agena::runtime::RuntimeBackgroundTaskKind::MarketplaceRegistrySync,
        format!("Sync marketplace registry {registry_id}"),
        Some(format!(
            "marketplace_registry_sync:{registry_id}:{registry_url}"
        )),
        "sync marketplace registry task failed",
        move || {
            let client = marketplace_client();
            let registry = client.registry(agena_plugin_marketplace::RegistrySpec {
                id: registry_id.clone(),
                url: registry_url.clone(),
                require_signature: false,
            });
            let index = registry.fetch_index(true)?;
            Ok(agena::runtime::RuntimeBackgroundTaskOutcome::succeeded(
                format!(
                    "Synced registry {registry_id} ({} plugins).",
                    index.plugins.len()
                ),
            ))
        },
    )
    .await
}

pub async fn list_marketplace_installed_plugins(
    State(_): State<AppState>,
) -> Result<impl IntoResponse, ServerError> {
    let client = marketplace_client();
    let mut entries = client
        .list_installed()
        .map_err(marketplace_bad_request)?
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
    Ok(items_json(entries))
}

pub async fn list_marketplace_outdated_plugins(
    State(_): State<AppState>,
) -> Result<impl IntoResponse, ServerError> {
    let client = marketplace_client();
    let entries = client
        .list_outdated()
        .map_err(marketplace_bad_request)?
        .into_iter()
        .map(|record| MarketplaceOutdatedPluginResource {
            plugin_id: record.plugin_id,
            installed_version: record.installed_version,
            latest_version: record.latest_version,
        })
        .collect::<Vec<_>>();
    Ok(items_json(entries))
}

pub async fn install_marketplace_plugin(
    State(state): State<AppState>,
    Json(request): Json<MarketplaceInstallRequest>,
) -> Result<impl IntoResponse, ServerError> {
    let registry_url = normalize_registry_url(request.registry.registry_url.as_str())?;
    let spec = request.spec.trim().to_string();
    if spec.is_empty() {
        return Err(ServerError::BadRequest("spec cannot be empty".to_string()));
    }
    let (plugin_id, version) = match spec.split_once('@') {
        Some((id, ver)) => (id.to_string(), Some(ver.to_string())),
        None => (spec, None),
    };
    let registry_id = registry_id_or_default(request.registry.registry_id.as_deref());
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
    let task_title = if request.dry_run {
        format!("Dry-run install marketplace plugin {plugin_id}")
    } else {
        format!("Install marketplace plugin {plugin_id}")
    };
    let dedupe_key = format!(
        "marketplace_plugin_install:{plugin_id}:{registry_id}:{registry_url}:{}",
        version.clone().unwrap_or_else(|| "latest".to_owned())
    );
    let require_signature = request.require_signature;
    let force = request.force;
    let dry_run = request.dry_run;
    let allow_unverified = request.allow_unverified;
    let refresh_index = request.refresh;

    spawn_marketplace_background_task(
        &state,
        agena::runtime::RuntimeBackgroundTaskKind::MarketplacePluginInstall,
        task_title,
        Some(dedupe_key),
        "install marketplace plugin task failed",
        move || {
            let client = marketplace_client();
            let outcome = client.install(agena_plugin_marketplace::InstallRequest {
                registry: agena_plugin_marketplace::RegistrySpec {
                    id: registry_id.clone(),
                    url: registry_url.clone(),
                    require_signature,
                },
                plugin_id: plugin_id.clone(),
                version: version.clone(),
                config_path,
                force,
                dry_run,
                allow_unverified,
                refresh_index,
            })?;
            let message = if outcome.dry_run {
                format!(
                    "Dry-run resolved {} v{}.",
                    outcome.plugin_id, outcome.version
                )
            } else {
                format!("Installed {} v{}.", outcome.plugin_id, outcome.version)
            };
            Ok(agena::runtime::RuntimeBackgroundTaskOutcome::succeeded(
                message,
            ))
        },
    )
    .await
}

pub async fn uninstall_marketplace_plugin(
    State(state): State<AppState>,
    Json(request): Json<MarketplaceUninstallRequestBody>,
) -> Result<impl IntoResponse, ServerError> {
    let plugin_id = request.plugin_id.trim().to_string();
    if plugin_id.is_empty() {
        return Err(ServerError::BadRequest(
            "plugin_id cannot be empty".to_string(),
        ));
    }
    let cascade = request.cascade;
    let title = format!("Uninstall marketplace plugin {plugin_id}");
    let dedupe_key = format!("marketplace_plugin_uninstall:{plugin_id}:{cascade}");
    spawn_marketplace_background_task(
        &state,
        agena::runtime::RuntimeBackgroundTaskKind::MarketplacePluginUninstall,
        title,
        Some(dedupe_key),
        "uninstall marketplace plugin task failed",
        move || {
            let client = marketplace_client();
            let entries = client.uninstall_with(plugin_id.as_str(), cascade)?;
            let message = format!(
                "Uninstalled {}.",
                entries
                    .iter()
                    .map(|entry| entry.plugin_id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            Ok(agena::runtime::RuntimeBackgroundTaskOutcome::succeeded(
                message,
            ))
        },
    )
    .await
}

pub async fn upgrade_marketplace_plugins(
    State(state): State<AppState>,
    Json(request): Json<MarketplaceUpgradeRequest>,
) -> Result<impl IntoResponse, ServerError> {
    if !request.all && request.plugin_id.as_deref().unwrap_or("").trim().is_empty() {
        return Err(ServerError::BadRequest(
            "plugin_id is required unless all=true".to_string(),
        ));
    }
    let override_spec = match request.registry.registry_url.as_deref() {
        Some(registry_url) => Some(agena_plugin_marketplace::RegistrySpec {
            id: registry_id_or_default(request.registry.registry_id.as_deref()),
            url: normalize_registry_url(registry_url)?,
            require_signature: false,
        }),
        None => None,
    };
    let plugin_id = request
        .plugin_id
        .clone()
        .unwrap_or_default()
        .trim()
        .to_string();
    let title = if request.all {
        "Upgrade marketplace plugins".to_owned()
    } else {
        format!("Upgrade marketplace plugin {plugin_id}")
    };
    let dedupe_key = if request.all {
        "marketplace_plugin_upgrade:all".to_owned()
    } else {
        format!("marketplace_plugin_upgrade:{plugin_id}")
    };
    let all = request.all;
    spawn_marketplace_background_task(
        &state,
        agena::runtime::RuntimeBackgroundTaskKind::MarketplacePluginUpgrade,
        title,
        Some(dedupe_key),
        "upgrade marketplace plugin task failed",
        move || {
            let client = marketplace_client();
            let targets = if all {
                client
                    .list_installed()?
                    .into_iter()
                    .map(|record| record.plugin_id)
                    .collect::<Vec<_>>()
            } else {
                vec![plugin_id.clone()]
            };

            let mut upgraded = Vec::new();
            for plugin_id in targets.into_iter().filter(|item| !item.is_empty()) {
                let outcome = client.upgrade(plugin_id.as_str(), override_spec.clone())?;
                if outcome.upgraded {
                    upgraded.push(outcome.plugin_id);
                }
            }

            let message = if upgraded.is_empty() {
                "Marketplace plugins are already up to date.".to_owned()
            } else {
                format!("Upgraded {}.", upgraded.join(", "))
            };
            Ok(agena::runtime::RuntimeBackgroundTaskOutcome::succeeded(
                message,
            ))
        },
    )
    .await
}

fn registry_id_or_default(registry_id: Option<&str>) -> String {
    registry_id.unwrap_or("default").to_owned()
}

fn normalize_registry_url(registry_url: &str) -> Result<String, ServerError> {
    let registry_url = registry_url.trim().to_string();
    if registry_url.is_empty() {
        return Err(ServerError::BadRequest(
            "registry_url cannot be empty".to_string(),
        ));
    }
    Ok(registry_url)
}

fn marketplace_client()
-> agena_plugin_marketplace::MarketplaceClient<agena_plugin_marketplace::ReqwestFetcher> {
    agena_plugin_marketplace::MarketplaceClient::with_default_fetcher(
        agena_plugin_marketplace::MarketplaceCache::new(
            agena_plugin_marketplace::default_cache_root(),
        ),
        std::collections::BTreeMap::new(),
    )
}

fn marketplace_bad_request(error: impl ToString) -> ServerError {
    ServerError::BadRequest(error.to_string())
}

async fn spawn_marketplace_background_task<F>(
    state: &AppState,
    kind: agena::runtime::RuntimeBackgroundTaskKind,
    title: String,
    dedupe_key: Option<String>,
    task_error_context: &'static str,
    task: F,
) -> Result<Json<crate::local_api::RuntimeBackgroundTaskStartResponse>, ServerError>
where
    F: FnOnce() -> Result<
            agena::runtime::RuntimeBackgroundTaskOutcome,
            agena_plugin_marketplace::MarketplaceError,
        > + Send
        + 'static,
{
    let start = state
        .runtime()
        .spawn_background_task(
            kind,
            agena::runtime::RuntimeBackgroundTaskOrigin::User,
            title,
            dedupe_key,
            false,
            move |_| async move {
                tokio::task::spawn_blocking(move || {
                    task().map_err(|error| agena::AppError::Config(error.to_string()))
                })
                .await
                .map_err(|error| {
                    agena::AppError::Internal(format!("{task_error_context}: {error}"))
                })?
            },
        )
        .map_err(super::server_error_from_runtime_background_task)?;
    Ok(Json(runtime_background_task_start_response(start)))
}
