use std::{collections::BTreeMap, path::PathBuf, sync::Arc};

use agena::{
    config::{
        ConfigEnvironment, ConfigLoader, ConfigSettingsEditOptions, ConfigSettingsPatchInput,
        ConfigSettingsPathInput, LoadConfigRequest, ProcessEnvironment,
        ProviderAdapterModelsResult, list_provider_adapter_models_with_config,
        patch_file_settings_with_env, provider_model_overlay_from_catalog_definition,
        saved_provider_adapter_models_target,
    },
    db::init_schema,
    model_catalog::{
        CatalogModelDefinition, ModelCatalogConfig, ModelCatalogService, ModelCatalogStore,
    },
    provider::ProviderRegistry,
    tracing as tracing_config,
};

#[derive(Debug, Clone)]
struct TestEnvironment {
    home: PathBuf,
}

impl ConfigEnvironment for TestEnvironment {
    fn var(&self, key: &str) -> Option<String> {
        match key {
            "HOME" => Some(self.home.display().to_string()),
            _ => std::env::var(key).ok(),
        }
    }

    fn vars(&self) -> Vec<(String, String)> {
        let mut vars = std::env::vars().collect::<Vec<_>>();
        vars.retain(|(key, _)| key != "HOME");
        vars.push(("HOME".to_string(), self.home.display().to_string()));
        vars
    }
}
use serde_json::{Map as JsonMap, Value as JsonValue, json};
use tempfile::tempdir;
use tokio::sync::OnceCell;

static PUBLIC_CATALOG: OnceCell<Arc<CatalogDefinitions>> = OnceCell::const_new();

#[derive(Debug, Clone)]
struct CatalogDefinitions {
    definitions: BTreeMap<String, CatalogModelDefinition>,
}

#[derive(Debug, Clone)]
struct MatchedModelSample {
    adapter_id: String,
    model_id: String,
    catalog_model_id: String,
}

#[derive(Debug, Clone)]
struct AdapterMatchStats {
    adapter_id: String,
    total: usize,
    matched: usize,
    matched_samples: Vec<MatchedModelSample>,
    unmatched_samples: Vec<(String, String)>,
}

#[derive(Debug, Clone)]
struct MatchResult {
    scope: &'static str,
    total: usize,
    matched: usize,
    adapter_stats: Vec<AdapterMatchStats>,
}

#[derive(Debug, Clone)]
struct CaseResult {
    public_catalog: Option<MatchResult>,
    effective_catalog: MatchResult,
    saved_routes: Vec<(String, String)>,
}

#[derive(Debug, Clone, Copy)]
struct MatchExpectations {
    public_minimum_match_rate: Option<f64>,
    effective_minimum_match_rate: f64,
    save_sample_count_per_adapter: usize,
}

struct CollectedMatch {
    result: MatchResult,
    provider_models_patch: JsonMap<String, JsonValue>,
    saved_routes: Vec<(String, String)>,
}

#[tokio::test]
#[ignore = "uses external provider APIs and remote public model catalog sources"]
async fn cxits_saved_provider_models_match_public_catalog_and_save_samples() {
    require_env("CX_API_KEY");

    let result = run_live_provider_case(
        "provider_gateway_live",
        cxits_provider_patch(),
        &["openai", "anthropic", "gemini"],
        MatchExpectations {
            public_minimum_match_rate: Some(0.70),
            effective_minimum_match_rate: 0.98,
            save_sample_count_per_adapter: 3,
        },
    )
    .await;

    assert!(
        !result.saved_routes.is_empty(),
        "{}",
        result.detailed_report()
    );
}

#[tokio::test]
#[ignore = "uses external provider APIs and remote public model catalog sources"]
async fn opencode_public_saved_provider_models_match_public_catalog_and_save_samples() {
    let result = run_live_provider_case(
        "opencode_public_live",
        opencode_public_provider_patch(),
        &["openai"],
        MatchExpectations {
            public_minimum_match_rate: Some(0.95),
            effective_minimum_match_rate: 0.98,
            save_sample_count_per_adapter: 3,
        },
    )
    .await;

    assert!(
        !result.saved_routes.is_empty(),
        "{}",
        result.detailed_report()
    );
}

#[tokio::test]
#[ignore = "uses remote public model catalog sources"]
async fn public_catalog_remote_sources_capture_context_limits() {
    let catalog = public_catalog().await;

    let deepseek = catalog
        .definitions
        .get("deepseek-v4-flash")
        .expect("deepseek-v4-flash should exist in public catalog");
    assert!(
        deepseek
            .context_window_tokens
            .is_some_and(|value| value > 0),
        "expected deepseek-v4-flash to expose a positive context window, got {:?}",
        deepseek.context_window_tokens
    );

    let nemotron = catalog
        .definitions
        .get("nemotron-3-super-120b-a12b")
        .expect("nemotron-3-super-120b-a12b should exist in public catalog");
    assert!(
        nemotron
            .context_window_tokens
            .is_some_and(|value| value > 0),
        "expected nemotron-3-super-120b-a12b to expose a positive context window, got {:?}",
        nemotron.context_window_tokens
    );

    let gpt = catalog
        .definitions
        .get("gpt-5.5")
        .expect("gpt-5.5 should exist in public catalog");
    assert!(
        gpt.context_window_tokens.is_some_and(|value| value > 0),
        "expected gpt-5.5 to expose positive context metadata, got {:?}",
        gpt.context_window_tokens
    );
}

impl MatchResult {
    fn match_rate(&self) -> f64 {
        if self.total == 0 {
            return 0.0;
        }
        self.matched as f64 / self.total as f64
    }

    fn summary(&self) -> String {
        format!(
            "{} matched={} total={} rate={:.2}%",
            self.scope,
            self.matched,
            self.total,
            self.match_rate() * 100.0
        )
    }

    fn detailed_report(&self) -> String {
        let mut lines = Vec::new();
        lines.push(self.summary());
        for adapter in &self.adapter_stats {
            lines.push(stats_report(adapter));
        }
        lines.join("\n")
    }
}

impl CaseResult {
    fn detailed_report(&self) -> String {
        let mut lines = Vec::new();
        if let Some(public_catalog) = &self.public_catalog {
            lines.push(public_catalog.detailed_report());
        }
        lines.push(self.effective_catalog.detailed_report());
        lines.push(format!("saved_routes={:?}", self.saved_routes));
        lines.join("\n")
    }
}

fn require_env(key: &str) {
    let value = std::env::var(key)
        .unwrap_or_else(|_| panic!("set {key} before running this ignored live test"));
    assert!(
        !value.trim().is_empty(),
        "set {key} before running this ignored live test"
    );
}

async fn public_catalog() -> Arc<CatalogDefinitions> {
    PUBLIC_CATALOG
        .get_or_init(|| async {
            let store = build_test_catalog_store().await;
            let service = ModelCatalogService::new(store)
                .await
                .expect("build public catalog service");
            let snapshot = service
                .refresh_from_registry(&ProviderRegistry::new(), None)
                .await
                .expect("refresh public model catalog");
            Arc::new(CatalogDefinitions {
                definitions: snapshot.official.models,
            })
        })
        .await
        .clone()
}

async fn effective_catalog(
    registry: &ProviderRegistry,
    resolution: &agena::config::ConfigResolution,
) -> CatalogDefinitions {
    let store = build_test_catalog_store().await;
    let service = ModelCatalogService::new(store)
        .await
        .expect("build effective catalog service");
    let snapshot = service
        .refresh_from_registry(registry, Some(resolution))
        .await
        .expect("refresh effective model catalog");
    CatalogDefinitions {
        definitions: snapshot.official.models,
    }
}

async fn build_test_catalog_store() -> ModelCatalogStore {
    let db = Arc::new(
        tracing_config::connect_database("sqlite::memory:", &Default::default())
            .await
            .expect("connect sqlite catalog test database"),
    );
    init_schema(db.as_ref())
        .await
        .expect("migrate sqlite catalog test database");
    ModelCatalogStore::new(
        ModelCatalogConfig {
            cache_max_age_secs: 0,
        },
        db,
    )
}

async fn run_live_provider_case(
    provider_id: &str,
    provider_patch: JsonValue,
    adapter_ids: &[&str],
    expectations: MatchExpectations,
) -> CaseResult {
    let public_catalog = public_catalog().await;
    let dir = tempdir().expect("tempdir for live provider test");
    let env = TestEnvironment {
        home: dir.path().to_path_buf(),
    };
    let config_path = dir.path().join("agena").join("agena.json");

    let save_provider_response = patch_file_settings_with_env(
        &config_path,
        ConfigSettingsPatchInput {
            target: ConfigSettingsPathInput {
                path: Some("providers".to_owned()),
            },
            changes: json!({ provider_id: provider_patch }),
            options: ConfigSettingsEditOptions {
                dry_run: false,
                validate: true,
                reload: false,
            },
        },
        &env,
    )
    .expect("save provider patch");

    assert!(
        save_provider_response.changed,
        "provider patch should change config"
    );
    assert!(
        save_provider_response.validated,
        "provider patch should validate successfully"
    );

    let initial_resolution = load_config(&env);
    assert!(
        initial_resolution
            .config
            .providers
            .contains_key(provider_id),
        "saved provider should be present after reload"
    );

    let registry = initial_resolution
        .config
        .build_provider_registry_with_env(&ProcessEnvironment)
        .expect("provider registry should build from saved provider");
    assert!(
        registry
            .provider_ids()
            .iter()
            .any(|value| value == provider_id),
        "saved provider should register into provider registry"
    );

    let resolved_provider = initial_resolution
        .config
        .providers
        .get(provider_id)
        .expect("saved provider config should exist");
    let adapter_ids_vec = adapter_ids
        .iter()
        .map(|value| value.to_string())
        .collect::<Vec<_>>();
    let target =
        saved_provider_adapter_models_target(provider_id, resolved_provider, &adapter_ids_vec)
            .expect("saved provider adapter target should resolve");

    let adapter_results = list_provider_adapter_models_with_config(
        &initial_resolution.config,
        &target,
        &ProcessEnvironment,
    )
    .await
    .expect("provider adapter model listing should run")
    .adapters;
    assert_adapter_results_ready(&adapter_results);

    let public_match = collect_match_result(
        "public_catalog",
        &adapter_results,
        &public_catalog.definitions,
        0,
    );
    if let Some(minimum_match_rate) = expectations.public_minimum_match_rate {
        assert!(
            public_match.result.match_rate() >= minimum_match_rate,
            "{}\n{}",
            format_match_failure(
                "public catalog match rate",
                minimum_match_rate,
                &public_match.result,
            ),
            public_match.result.detailed_report(),
        );
    }

    let effective_catalog = effective_catalog(&registry, &initial_resolution).await;
    let effective_match = collect_match_result(
        "effective_catalog",
        &adapter_results,
        &effective_catalog.definitions,
        expectations.save_sample_count_per_adapter,
    );
    assert!(
        effective_match.result.match_rate() >= expectations.effective_minimum_match_rate,
        "{}\n{}",
        format_match_failure(
            "effective catalog match rate",
            expectations.effective_minimum_match_rate,
            &effective_match.result,
        ),
        effective_match.result.detailed_report(),
    );
    assert!(
        !effective_match.provider_models_patch.is_empty(),
        "expected matched provider models to save back into config"
    );
    let CollectedMatch {
        result: effective_result,
        provider_models_patch,
        saved_routes,
    } = effective_match;

    let save_models_response = patch_file_settings_with_env(
        &config_path,
        ConfigSettingsPatchInput {
            target: ConfigSettingsPathInput {
                path: Some("providers".to_owned()),
            },
            changes: json!({
                provider_id: {
                    "adapters": provider_models_patch,
                }
            }),
            options: ConfigSettingsEditOptions {
                dry_run: false,
                validate: true,
                reload: false,
            },
        },
        &ProcessEnvironment,
    )
    .expect("save matched provider models patch");

    assert!(
        save_models_response.changed,
        "saving matched models should change config"
    );
    assert!(
        save_models_response.validated,
        "saving matched models should validate successfully"
    );

    let saved_resolution = load_config(&env);
    let saved_provider = saved_resolution
        .config
        .providers
        .get(provider_id)
        .expect("saved provider should still exist after matched model save");

    for (adapter_id, model_id) in &saved_routes {
        let route_id = format!("{adapter_id}/{model_id}");
        assert!(
            saved_provider.models.contains_key(route_id.as_str()),
            "saved provider should contain configured route {route_id}"
        );
    }

    saved_resolution
        .config
        .build_provider_registry_with_env(&ProcessEnvironment)
        .expect("provider registry should still build after matched model save");

    CaseResult {
        public_catalog: Some(public_match.result),
        effective_catalog: effective_result,
        saved_routes,
    }
}

fn assert_adapter_results_ready(adapter_results: &[ProviderAdapterModelsResult]) {
    for adapter_result in adapter_results {
        assert!(
            adapter_result.error.is_none(),
            "adapter {} listing failed: {:?}",
            adapter_result.adapter_id,
            adapter_result.error
        );
        assert!(
            !adapter_result.models.is_empty(),
            "adapter {} should return at least one model",
            adapter_result.adapter_id
        );
    }
}

fn collect_match_result(
    scope: &'static str,
    adapter_results: &[ProviderAdapterModelsResult],
    definitions: &BTreeMap<String, CatalogModelDefinition>,
    save_sample_count_per_adapter: usize,
) -> CollectedMatch {
    let mut total = 0usize;
    let mut matched = 0usize;
    let mut adapter_stats = Vec::new();
    let mut provider_models_patch = JsonMap::new();
    let mut saved_routes = Vec::new();

    for adapter_result in adapter_results {
        let mut stats = AdapterMatchStats {
            adapter_id: adapter_result.adapter_id.clone(),
            total: 0,
            matched: 0,
            matched_samples: Vec::new(),
            unmatched_samples: Vec::new(),
        };
        let mut adapter_models_patch = JsonMap::new();

        for model in &adapter_result.models {
            stats.total += 1;
            total += 1;

            let Some(catalog_model_id) = model.catalog_model_id.as_ref().map(ToString::to_string)
            else {
                if stats.unmatched_samples.len() < 10 {
                    stats
                        .unmatched_samples
                        .push((model.id.to_string(), String::new()));
                }
                continue;
            };

            let Some(definition) = definitions.get(catalog_model_id.as_str()) else {
                if stats.unmatched_samples.len() < 10 {
                    stats
                        .unmatched_samples
                        .push((model.id.to_string(), catalog_model_id));
                }
                continue;
            };

            stats.matched += 1;
            matched += 1;

            if stats.matched_samples.len() < save_sample_count_per_adapter {
                stats.matched_samples.push(MatchedModelSample {
                    adapter_id: adapter_result.adapter_id.clone(),
                    model_id: model.id.to_string(),
                    catalog_model_id: catalog_model_id.clone(),
                });
                adapter_models_patch.insert(
                    model.id.to_string(),
                    serde_json::to_value(provider_model_overlay_from_catalog_definition(
                        definition,
                    ))
                    .expect("serialize provider model overlay"),
                );
                saved_routes.push((adapter_result.adapter_id.clone(), model.id.to_string()));
            }
        }

        assert!(
            stats.matched > 0,
            "adapter {} should have at least one {} match\n{}",
            stats.adapter_id,
            scope,
            stats_report(&stats)
        );

        if !adapter_models_patch.is_empty() {
            provider_models_patch.insert(
                adapter_result.adapter_id.clone(),
                json!({
                    "enabled": true,
                    "models": adapter_models_patch,
                }),
            );
        }

        adapter_stats.push(stats);
    }

    CollectedMatch {
        result: MatchResult {
            scope,
            total,
            matched,
            adapter_stats,
        },
        provider_models_patch,
        saved_routes,
    }
}

fn format_match_failure(label: &str, minimum_match_rate: f64, result: &MatchResult) -> String {
    format!(
        "{label} below threshold {:.2}%: {}",
        minimum_match_rate * 100.0,
        result.summary()
    )
}

fn load_config(env: &TestEnvironment) -> agena::config::ConfigResolution {
    let loader = ConfigLoader::new(env.clone());
    loader
        .load(&LoadConfigRequest::default())
        .expect("load saved config")
}

fn stats_report(stats: &AdapterMatchStats) -> String {
    let rate = if stats.total == 0 {
        0.0
    } else {
        stats.matched as f64 / stats.total as f64
    };
    format!(
        "adapter={} matched={} total={} rate={:.2}% matched_samples={:?} unmatched_samples={:?}",
        stats.adapter_id,
        stats.matched,
        stats.total,
        rate * 100.0,
        stats
            .matched_samples
            .iter()
            .map(|sample| (
                &sample.adapter_id,
                &sample.model_id,
                &sample.catalog_model_id
            ))
            .collect::<Vec<_>>(),
        stats.unmatched_samples,
    )
}

fn cxits_provider_patch() -> JsonValue {
    json!({
        "defaults": {
            "adapter": "openai",
            "model": "gpt-4.1-mini"
        },
        "auth": {
            "mode": "api",
            "subtype": "custom",
            "base_url": "https://api.cxits.cn",
            "api_key": {
              "kind": "env",
              "value": "CX_API_KEY"
            },
            "protocol_paths": {
                "openai": "/api/provider/openai/v1",
                "anthropic": "/api/provider/anthropic/v1",
                "gemini": "/api/provider/google/v1beta"
            }
        },
        "adapters": {
            "openai": {
                "enabled": true
            },
            "anthropic": {
                "enabled": true
            },
            "gemini": {
                "enabled": true
            }
        }
    })
}

fn opencode_public_provider_patch() -> JsonValue {
    json!({
        "defaults": {
            "adapter": "openai",
            "model": "deepseek-v4-flash-free"
        },
        "auth": {
            "mode": "api",
            "subtype": "custom",
            "base_url": "https://opencode.ai/zen",
            "api_key": {
              "kind": "inline",
              "value": "public"
            },
            "protocol_paths": {
                "gemini": "/v1"
            }
        },
        "adapters": {
            "openai": {
                "enabled": true
            }
        }
    })
}
