pub async fn search_marketplace_plugins(
    State(_): State<AppState>,
    Json(request): Json<MarketplaceSearchRequest>,
) -> Result<impl IntoResponse, ServerError> {
    let registry_spec = marketplace_registry_spec(
        request.registry.registry_id.as_deref(),
        request.registry.registry_url.as_deref(),
        false,
    )?;
    let registry_id = registry_spec.id.clone();
    let registry_url = registry_spec.url.clone();

    let client = marketplace_client();
    let registry = client.registry(registry_spec);
    let index = registry
        .fetch_index(request.refresh)
        .map_err(ServerError::service_unavailable)?;
    let needle = request
        .query
        .as_deref()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();

    let marketplace = MarketplaceIdentityResource {
        name: index.marketplace.name.clone(),
        description: index.marketplace.description.clone(),
        homepage: index.marketplace.homepage.clone(),
        repository: index.marketplace.repository.clone(),
        owner_name: index
            .marketplace
            .owner
            .as_ref()
            .map(|owner| owner.name.clone()),
        owner_url: index
            .marketplace
            .owner
            .as_ref()
            .and_then(|owner| owner.url.clone()),
    };
    let mut entries = index
        .plugins
        .into_iter()
        .filter(|plugin| {
            if needle.is_empty() {
                return true;
            }
            let blob = format!(
                "{} {} {} {} {} {}",
                plugin.id,
                plugin.name,
                plugin.description,
                plugin.category.as_deref().unwrap_or_default(),
                plugin.repository.as_deref().unwrap_or_default(),
                plugin.tags.join(" ")
            )
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
                repository: plugin.repository,
                license: plugin.license,
                category: plugin.category,
                tags: plugin.tags,
                version_count: plugin.versions.len(),
                latest_version: latest.map(|version| version.version.clone()),
                latest_kind: latest.map(|version| version.kind.to_string()),
                latest_platform: latest.map(|version| version.platform.clone()),
                latest_source_repository: latest
                    .and_then(|version| version.source.as_ref())
                    .map(|source| source.repository.clone()),
                latest_source_tag: latest
                    .and_then(|version| version.source.as_ref())
                    .map(|source| source.tag.clone()),
                latest_source_commit: latest
                    .and_then(|version| version.source.as_ref())
                    .map(|source| source.commit.clone()),
                review_tier: plugin.review_tier.as_str().to_string(),
                featured: plugin.featured,
            }
        })
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| left.plugin_id.cmp(&right.plugin_id));

    Ok(Json(MarketplaceSearchResponse {
        registry_id,
        registry_url,
        marketplace,
        entries,
    }))
}

pub async fn sync_marketplace_registry(
    State(state): State<AppState>,
    Json(request): Json<MarketplaceRegistryRequest>,
) -> Result<impl IntoResponse, ServerError> {
    let registry_spec = marketplace_registry_spec(
        request.registry_id.as_deref(),
        request.registry_url.as_deref(),
        false,
    )?;
    let registry_id = registry_spec.id.clone();
    let registry_url = registry_spec.url.clone();

    spawn_marketplace_background_task(
        &state,
        agena_runtime::RuntimeBackgroundTaskKind::MarketplaceRegistrySync,
        format!("Sync marketplace registry {registry_id}"),
        Some(format!(
            "marketplace_registry_sync:{registry_id}:{registry_url}"
        )),
        "sync marketplace registry task failed",
        move || {
            let client = marketplace_client();
            let registry = client.registry(registry_spec);
            let index = registry.fetch_index(true)?;
            Ok(agena_runtime::RuntimeBackgroundTaskOutcome::succeeded(
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
        .map_err(ServerError::internal)?
        .into_iter()
        .map(|record| MarketplaceInstalledPluginResource {
            plugin_id: record.plugin_id,
            version: record.version,
            kind: record.kind.to_string(),
            platform: record.platform,
            binary_path: record.binary_path.display().to_string(),
            config_path: record.config_path.display().to_string(),
            sha256: record.sha256,
            installed_at: record.installed_at,
            registry_id: record.registry_id,
            registry_url: record.registry_url,
            require_signature: record.require_signature,
            require_github_distribution: record.require_github_distribution,
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
        .map_err(ServerError::internal)?
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
    let locator = agena_plugin_marketplace::parse_plugin_install_locator(request.spec.as_str())
        .map_err(|error| {
            ServerError::bad_request_with_diagnostic(
                "The plugin install specification is invalid.",
                error,
            )
        })?;
    let require_signature = request.require_signature;
    let plan = match (request.registry.registry_url.as_deref(), locator) {
        (
            source,
            agena_plugin_marketplace::PluginInstallLocator::Marketplace { plugin_id, version },
        ) => MarketplaceInstallPlan::Marketplace {
            registry: marketplace_registry_spec(
                request.registry.registry_id.as_deref(),
                source,
                require_signature,
            )?,
            plugin_id,
            version,
        },
        (Some(_), agena_plugin_marketplace::PluginInstallLocator::GitHubRelease { .. }) => {
            return Err(ServerError::bad_request(
                "Do not combine a direct GitHub repository install with registry_url.",
            ));
        }
        (
            None,
            agena_plugin_marketplace::PluginInstallLocator::GitHubRelease { repository, tag },
        ) => MarketplaceInstallPlan::GitHubRelease {
            registry: agena_plugin_marketplace::RegistrySpec::github_release(
                repository,
                tag.as_deref(),
                require_signature,
            )
            .map_err(|error| {
                ServerError::bad_request_with_diagnostic(
                    "The GitHub plugin source is invalid.",
                    error,
                )
            })?,
        },
    };
    let config_path = state.config_path().map_err(ServerError::from)?;
    let target = plan.display_target();
    let registry_url = plan.registry().url.clone();
    let task_title = if request.dry_run {
        format!("Dry-run install marketplace plugin {target}")
    } else {
        format!("Install marketplace plugin {target}")
    };
    let dedupe_key = format!(
        "marketplace_plugin_install:{target}:{registry_url}:{}",
        plan.requested_version().unwrap_or("latest")
    );
    let force = request.force;
    let dry_run = request.dry_run;
    let allow_unverified = request.allow_unverified;
    let refresh_index = request.refresh;

    spawn_marketplace_background_task(
        &state,
        agena_runtime::RuntimeBackgroundTaskKind::MarketplacePluginInstall,
        task_title,
        Some(dedupe_key),
        "install marketplace plugin task failed",
        move || {
            let client = marketplace_client();
            let (registry, plugin_id, version, install_refresh) = match plan {
                MarketplaceInstallPlan::Marketplace {
                    registry,
                    plugin_id,
                    version,
                } => (registry, plugin_id, version, refresh_index),
                MarketplaceInstallPlan::GitHubRelease { registry } => {
                    let index = client
                        .registry(registry.clone())
                        .fetch_index(refresh_index)?;
                    let plugin_id = single_release_plugin_id(&index)?;
                    (registry, plugin_id, None, false)
                }
            };
            let outcome = client.install(agena_plugin_marketplace::InstallRequest {
                registry,
                plugin_id: plugin_id.clone(),
                version: version.clone(),
                config_path,
                force,
                dry_run,
                allow_unverified,
                refresh_index: install_refresh,
            })?;
            let message = if outcome.dry_run {
                format!(
                    "Dry-run resolved {} v{}.",
                    outcome.plugin_id, outcome.version
                )
            } else {
                format!("Installed {} v{}.", outcome.plugin_id, outcome.version)
            };
            Ok(agena_runtime::RuntimeBackgroundTaskOutcome::succeeded(
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
        return Err(ServerError::bad_request("The plugin ID cannot be empty."));
    }
    let cascade = request.cascade;
    let title = format!("Uninstall marketplace plugin {plugin_id}");
    let dedupe_key = format!("marketplace_plugin_uninstall:{plugin_id}:{cascade}");
    spawn_marketplace_background_task(
        &state,
        agena_runtime::RuntimeBackgroundTaskKind::MarketplacePluginUninstall,
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
            Ok(agena_runtime::RuntimeBackgroundTaskOutcome::succeeded(
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
        return Err(ServerError::bad_request(
            "A plugin ID is required unless all plugins are selected.",
        ));
    }
    let override_spec = match request.registry.registry_url.as_deref() {
        Some(source) => Some(marketplace_registry_spec(
            request.registry.registry_id.as_deref(),
            Some(source),
            false,
        )?),
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
        agena_runtime::RuntimeBackgroundTaskKind::MarketplacePluginUpgrade,
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
            Ok(agena_runtime::RuntimeBackgroundTaskOutcome::succeeded(
                message,
            ))
        },
    )
    .await
}

#[derive(Debug, Clone)]
enum MarketplaceInstallPlan {
    Marketplace {
        registry: agena_plugin_marketplace::RegistrySpec,
        plugin_id: String,
        version: Option<String>,
    },
    GitHubRelease {
        registry: agena_plugin_marketplace::RegistrySpec,
    },
}

impl MarketplaceInstallPlan {
    fn registry(&self) -> &agena_plugin_marketplace::RegistrySpec {
        match self {
            Self::Marketplace { registry, .. } | Self::GitHubRelease { registry } => registry,
        }
    }

    fn display_target(&self) -> String {
        match self {
            Self::Marketplace { plugin_id, .. } => plugin_id.clone(),
            Self::GitHubRelease { registry } => registry.id.clone(),
        }
    }

    fn requested_version(&self) -> Option<&str> {
        match self {
            Self::Marketplace { version, .. } => version.as_deref(),
            Self::GitHubRelease { .. } => None,
        }
    }
}

fn marketplace_registry_spec(
    registry_id: Option<&str>,
    source: Option<&str>,
    require_signature: bool,
) -> Result<agena_plugin_marketplace::RegistrySpec, ServerError> {
    let source = source
        .map(str::trim)
        .filter(|source| !source.is_empty())
        .unwrap_or(agena_plugin_marketplace::DEFAULT_MARKETPLACE_SOURCE);
    let default_id = if source == agena_plugin_marketplace::DEFAULT_MARKETPLACE_SOURCE {
        "official"
    } else {
        "default"
    };
    agena_plugin_marketplace::RegistrySpec::from_source(
        registry_id.unwrap_or(default_id),
        source,
        require_signature,
    )
    .map_err(|error| {
        ServerError::bad_request_with_diagnostic("The marketplace source is invalid.", error)
    })
}

fn single_release_plugin_id(
    index: &agena_plugin_marketplace::RegistryIndex,
) -> Result<String, agena_plugin_marketplace::MarketplaceError> {
    match index.plugins.as_slice() {
        [plugin] => Ok(plugin.id.clone()),
        [] => Err(agena_plugin_marketplace::MarketplaceError::Index(
            "GitHub release manifest contains no plugin".to_string(),
        )),
        plugins => Err(agena_plugin_marketplace::MarketplaceError::Index(format!(
            "GitHub release manifest contains {} plugins; direct repository installs require exactly one",
            plugins.len()
        ))),
    }
}

fn marketplace_client()
-> agena_plugin_marketplace::MarketplaceClient<agena_plugin_marketplace::ReqwestFetcher> {
    agena_plugin_marketplace::MarketplaceClient::new(
        agena_plugin_marketplace::MarketplaceCache::new(
            agena_plugin_marketplace::default_cache_root(),
        ),
        std::collections::BTreeMap::new(),
    )
}

async fn spawn_marketplace_background_task<F>(
    state: &AppState,
    kind: agena_runtime::RuntimeBackgroundTaskKind,
    title: String,
    dedupe_key: Option<String>,
    task_error_context: &'static str,
    task: F,
) -> Result<Json<agena_application::dto::RuntimeBackgroundTaskStartResponse>, ServerError>
where
    F: FnOnce() -> Result<
            agena_runtime::RuntimeBackgroundTaskOutcome,
            agena_plugin_marketplace::MarketplaceError,
        > + Send
        + 'static,
{
    let work: agena_runtime::RuntimeBackgroundTaskWork = Box::new(move |_| {
        Box::pin(async move {
            tokio::task::spawn_blocking(move || task().map_err(|error| error.to_string()))
                .await
                .map_err(|error| {
                    agena_runtime::RuntimeControlServiceError::new(format!(
                        "{task_error_context}: {error}"
                    ))
                })?
                .map_err(agena_runtime::RuntimeControlServiceError::new)
        })
    });
    let start = state
        .runtime_control()
        .start_background_task(
            kind,
            agena_runtime::RuntimeBackgroundTaskOrigin::User,
            title,
            dedupe_key,
            false,
            work,
        )
        .map_err(super::server_error_from_runtime_background_task)?;
    Ok(Json(runtime_background_task_start_response(start)))
}
use super::{
    AppState, IntoResponse, Json, MarketplaceIdentityResource, MarketplaceInstallRequest,
    MarketplaceInstalledPluginResource, MarketplaceOutdatedPluginResource,
    MarketplacePluginResource, MarketplaceRegistryRequest, MarketplaceSearchRequest,
    MarketplaceSearchResponse, MarketplaceUninstallRequestBody, MarketplaceUpgradeRequest,
    ServerError, State, items_json, runtime_background_task_start_response,
};
