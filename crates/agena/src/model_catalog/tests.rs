use super::*;
use crate::{
    model::{CapabilitySupport, ModelId, ModelMetadata, ModelSpeedModeRequestOverride},
    provider::{
        ConfiguredModelSpeedMode, ConfiguredModelThinkingMode, ModelCapabilityFeature,
        ReasoningEffort, ThinkingRequest,
    },
};
use tempfile::tempdir;

fn normalized_catalog_model_id(model_id: &str) -> String {
    curate::normalized_catalog_model_id(model_id)
}

struct StaticListProvider {
    provider_id: &'static str,
    default_model: ModelId,
    models: Vec<Model>,
}

impl StaticListProvider {
    fn new(provider_id: &'static str, default_model: &'static str, models: Vec<Model>) -> Self {
        Self {
            provider_id,
            default_model: ModelId::new(default_model),
            models,
        }
    }
}

#[async_trait::async_trait]
impl ModelRuntime for StaticListProvider {
    fn id(&self) -> &str {
        self.provider_id
    }

    fn default_model(&self) -> &ModelId {
        &self.default_model
    }

    async fn list_models(&self) -> Result<Vec<Model>, AppError> {
        Ok(self.models.clone())
    }

    async fn complete(
        &self,
        _request: crate::provider::CompletionRequest,
    ) -> Result<crate::provider::CompletionResponse, AppError> {
        Err(AppError::Provider("not implemented".to_owned()))
    }
}

#[test]
fn merged_models_prefers_custom_models() {
    let snapshot = ModelCatalogSnapshot {
        official: ModelCatalogDocument {
            models: BTreeMap::from([(
                "gpt-5".to_owned(),
                CatalogModelDefinition {
                    display_name: Some("GPT-5".to_owned()),
                    ..CatalogModelDefinition::default()
                },
            )]),
        },
        custom: ModelCatalogDocument {
            models: BTreeMap::from([(
                "gpt-5-custom".to_owned(),
                CatalogModelDefinition {
                    display_name: Some("GPT-5 Custom".to_owned()),
                    ..CatalogModelDefinition::default()
                },
            )]),
        },
        ..ModelCatalogSnapshot::default()
    };

    let merged = snapshot.merged_models();
    assert!(merged.models.contains_key("gpt-5"));
    assert!(merged.models.contains_key("gpt-5-custom"));
}

#[test]
fn entries_keep_official_and_custom_records_separate() {
    let snapshot = ModelCatalogSnapshot {
        last_successful_source: Some(ModelCatalogEntrySourceKind::Generated),
        official: ModelCatalogDocument {
            models: BTreeMap::from([(
                "claude-sonnet".to_owned(),
                CatalogModelDefinition {
                    display_name: Some("Claude Sonnet".to_owned()),
                    capabilities: ModelCapabilityPatch {
                        reasoning: Some(CapabilitySupport::Supported),
                        ..ModelCapabilityPatch::default()
                    },
                    ..CatalogModelDefinition::default()
                },
            )]),
        },
        custom: ModelCatalogDocument {
            models: BTreeMap::from([(
                "claude-sonnet".to_owned(),
                CatalogModelDefinition {
                    display_name: Some("Claude Sonnet Local".to_owned()),
                    thinking_modes: BTreeMap::from([(
                        "deep".to_owned(),
                        ConfiguredModelThinkingMode {
                            display_name: Some("Deep".to_owned()),
                            description: None,
                            thinking: None,
                            request_override: Default::default(),
                            adapter_overrides: BTreeMap::new(),
                            disabled: false,
                        },
                    )]),
                    ..CatalogModelDefinition::default()
                },
            )]),
        },
        ..ModelCatalogSnapshot::default()
    };

    let entries = snapshot.entries();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].model_id, "claude-sonnet");
    assert_eq!(entries[0].display_name.as_deref(), Some("Claude Sonnet"));
    assert!(!entries[0].has_local_override);
    assert_eq!(entries[1].model_id, "claude-sonnet");
    assert_eq!(
        entries[1].display_name.as_deref(),
        Some("Claude Sonnet Local")
    );
    assert!(entries[1].has_local_override);
    assert!(entries[1].thinking_modes.contains_key("deep"));
}

#[test]
fn merged_models_keep_primary_capability_when_sources_conflict() {
    let snapshot = ModelCatalogSnapshot {
        official: ModelCatalogDocument {
            models: BTreeMap::from([(
                "glm-5".to_owned(),
                CatalogModelDefinition {
                    capabilities: ModelCapabilityPatch {
                        features: Some(FeatureCapabilityPatch::Patch(FeatureCapabilityPatchBody {
                            supported: vec![ModelCapabilityFeature::StructuredOutput],
                            unsupported: vec![ModelCapabilityFeature::ToolCalling],
                        })),
                        ..ModelCapabilityPatch::default()
                    },
                    ..CatalogModelDefinition::default()
                },
            )]),
        },
        custom: ModelCatalogDocument {
            models: BTreeMap::from([(
                "glm-5".to_owned(),
                CatalogModelDefinition {
                    capabilities: ModelCapabilityPatch {
                        features: Some(FeatureCapabilityPatch::Patch(FeatureCapabilityPatchBody {
                            supported: vec![ModelCapabilityFeature::ToolCalling],
                            unsupported: vec![ModelCapabilityFeature::StructuredOutput],
                        })),
                        ..ModelCapabilityPatch::default()
                    },
                    ..CatalogModelDefinition::default()
                },
            )]),
        },
        ..ModelCatalogSnapshot::default()
    };

    let merged = snapshot.merged_models();
    let capabilities = &merged
        .models
        .get("glm-5")
        .expect("glm-5 should exist after merge")
        .capabilities;

    capabilities
        .validate()
        .expect("merged capability patch should stay valid");
    assert_eq!(
        capabilities.feature_support(ModelCapabilityFeature::StructuredOutput),
        Some(CapabilitySupport::Unsupported)
    );
    assert_eq!(
        capabilities.feature_support(ModelCapabilityFeature::ToolCalling),
        Some(CapabilitySupport::Supported)
    );
}

#[test]
fn live_provider_catalog_document_corrects_public_catalog_baseline() {
    let mut current = ModelCatalogDocument {
        models: BTreeMap::from([(
            "glm-5".to_owned(),
            CatalogModelDefinition {
                display_name: Some("GLM-5".to_owned()),
                max_output_tokens: Some(32_768),
                capabilities: ModelCapabilityPatch {
                    features: Some(FeatureCapabilityPatch::Supported(vec![
                        ModelCapabilityFeature::StructuredOutput,
                    ])),
                    ..ModelCapabilityPatch::default()
                },
                ..CatalogModelDefinition::default()
            },
        )]),
    };
    let provider_document = ModelCatalogDocument {
        models: BTreeMap::from([(
            "glm-5".to_owned(),
            CatalogModelDefinition {
                max_output_tokens: Some(8_192),
                capabilities: ModelCapabilityPatch {
                    features: Some(FeatureCapabilityPatch::Patch(FeatureCapabilityPatchBody {
                        supported: vec![ModelCapabilityFeature::Reasoning],
                        unsupported: vec![ModelCapabilityFeature::StructuredOutput],
                    })),
                    ..ModelCapabilityPatch::default()
                },
                ..CatalogModelDefinition::default()
            },
        )]),
    };

    merge_live_provider_catalog_document(&mut current, provider_document);

    let corrected = current
        .models
        .get("glm-5")
        .expect("glm-5 should still exist");
    assert_eq!(corrected.display_name.as_deref(), Some("GLM-5"));
    assert_eq!(corrected.max_output_tokens, Some(8_192));
    assert_eq!(
        corrected
            .capabilities
            .feature_support(ModelCapabilityFeature::StructuredOutput),
        Some(CapabilitySupport::Unsupported)
    );
    assert_eq!(
        corrected
            .capabilities
            .feature_support(ModelCapabilityFeature::Reasoning),
        Some(CapabilitySupport::Supported)
    );
}

#[test]
fn decorate_provider_models_preserves_listed_model_id_exactly() {
    let provider = StaticListProvider::new(
        "openrouter",
        "openai/gpt-5.4",
        vec![
            Model::new("openrouter", "openai/gpt-5.4").with_display_name("OpenRouter Raw GPT-5.4"),
        ],
    );
    let provider_record = ModelCatalogProviderRecord {
        models: BTreeMap::from([(
            "gpt-5.4".to_owned(),
            CatalogModelDefinition {
                display_name: Some("GPT-5.4 Catalog".to_owned()),
                ..CatalogModelDefinition::default()
            },
        )]),
        appendable_model_ids: BTreeSet::new(),
    };

    let decorated = decorate_provider_models(
        &provider,
        &provider_record,
        vec![
            Model::new("openrouter", "openai/gpt-5.4").with_display_name("OpenRouter Raw GPT-5.4"),
        ],
    );

    assert_eq!(decorated.len(), 1);
    assert_eq!(decorated[0].id.as_str(), "openai/gpt-5.4");
    assert_eq!(
        decorated[0].catalog_model_id.as_ref().map(ModelId::as_str),
        Some("gpt-5.4")
    );
    assert_eq!(
        decorated[0].display_name.as_deref(),
        Some("GPT-5.4 Catalog")
    );
}

#[test]
fn curated_catalog_document_canonicalizes_aliases_and_seeds_origin_labels() {
    let document = curate::curate_catalog_document(ModelCatalogDocument {
        models: BTreeMap::from([
            (
                "openai.gpt-5.4".to_owned(),
                CatalogModelDefinition {
                    display_name: Some("OpenAI GPT-5.4".to_owned()),
                    ..CatalogModelDefinition::default()
                },
            ),
            (
                "study_gpt-chatgpt-4o-latest".to_owned(),
                CatalogModelDefinition {
                    display_name: Some("Study GPT ChatGPT-4o Latest".to_owned()),
                    ..CatalogModelDefinition::default()
                },
            ),
            (
                "amazon.nova-pro-v1:0".to_owned(),
                CatalogModelDefinition {
                    display_name: Some("Amazon Nova Pro".to_owned()),
                    ..CatalogModelDefinition::default()
                },
            ),
            (
                "gpt-oss-120b:free".to_owned(),
                CatalogModelDefinition {
                    display_name: Some("GPT OSS 120B".to_owned()),
                    ..CatalogModelDefinition::default()
                },
            ),
            (
                "claude-opus-4-7".to_owned(),
                CatalogModelDefinition {
                    display_name: Some("Claude Opus 4.7".to_owned()),
                    ..CatalogModelDefinition::default()
                },
            ),
        ]),
    })
    .expect("seed document should curate");

    let catalog = document.model_record();
    assert!(catalog.models.contains_key("claude-opus-4-7"));
    assert!(catalog.models.contains_key("nova-pro-v1"));
    assert!(catalog.models.contains_key("gpt-5.4"));
    assert!(catalog.models.contains_key("gpt-4o"));
    assert!(catalog.models.contains_key("gpt-oss-120b"));
    assert_eq!(
        catalog
            .models
            .get("gpt-5.4")
            .and_then(|definition| definition.origin.as_deref()),
        Some("OpenAI")
    );
    assert_eq!(
        catalog
            .models
            .get("claude-opus-4-7")
            .and_then(|definition| definition.origin.as_deref()),
        Some("Anthropic")
    );
    assert!(!catalog.models.contains_key("openai.gpt-5.4"));
    assert!(!catalog.models.contains_key("study_gpt-chatgpt-4o-latest"));
    assert!(!catalog.models.contains_key("gpt-oss-120b:free"));
    assert!(!catalog.models.contains_key("amazon.nova-pro-v1:0"));

    let mut lowered = BTreeSet::new();
    let mut normalized = BTreeSet::new();
    for model_id in catalog.models.keys() {
        assert_eq!(
            model_id,
            &model_id.to_ascii_lowercase(),
            "curated model id should be lowercase canonical text: {model_id}"
        );
        assert!(
            !model_id.contains('/'),
            "curated model id should not contain '/': {model_id}"
        );
        assert!(
            !model_id.contains("@default"),
            "curated model id should not contain '@default': {model_id}"
        );
        assert!(
            !model_id.ends_with("-maas"),
            "curated model id should not contain provider route suffix '-maas': {model_id}"
        );
        assert!(
            !model_id.ends_with(":free"),
            "curated model id should not contain free-tier suffix ':free': {model_id}"
        );
        assert!(
            catalog
                .models
                .get(model_id)
                .and_then(|definition| definition.origin.as_ref())
                .is_some_and(|origin| !origin.trim().is_empty()),
            "curated model id should include a non-empty origin label: {model_id}"
        );
        assert!(
            lowered.insert(model_id.to_ascii_lowercase()),
            "curated catalog should not contain case-insensitive duplicate model ids: {model_id}"
        );
        let normalized_model_id = normalized_catalog_model_id(model_id);
        assert!(
            normalized.insert(normalized_model_id.clone()),
            "curated catalog should not contain normalized duplicate model ids: {model_id} -> {normalized_model_id}"
        );
    }
}

#[tokio::test]
async fn startup_refresh_reuses_fresh_cached_catalog() {
    let dir = tempdir().expect("tempdir should create");
    let store = ModelCatalogStore::new(ModelCatalogConfig {
        cache_path: dir.path().join("model-catalog-cache.json"),
        custom_path: dir.path().join("model-catalog-custom.json"),
        cache_max_age_secs: 60,
    });
    let cached_document = model_catalog_document("cached-model");
    store
        .write_cached_official(&CachedOfficialCatalog {
            fetched_at_unix_ms: now_unix_ms(),
            source: ModelCatalogEntrySourceKind::Cache,
            document: cached_document.clone(),
        })
        .await
        .expect("cache should be written");

    let service = ModelCatalogService::with_remote_sources(store, Vec::new())
        .await
        .expect("service should load");
    let providers = ProviderRegistry::new();

    let snapshot = service
        .refresh_if_stale_on_startup(&providers, None)
        .await
        .expect("fresh startup snapshot should succeed");

    assert_eq!(snapshot.official, cached_document);
    assert_eq!(
        snapshot.last_successful_source,
        Some(ModelCatalogEntrySourceKind::Cache)
    );
}

#[tokio::test]
async fn startup_refresh_updates_stale_cached_catalog_from_provider_registry() {
    let dir = tempdir().expect("tempdir should create");
    let store = ModelCatalogStore::new(ModelCatalogConfig {
        cache_path: dir.path().join("model-catalog-cache.json"),
        custom_path: dir.path().join("model-catalog-custom.json"),
        cache_max_age_secs: 1,
    });
    store
        .write_cached_official(&CachedOfficialCatalog {
            fetched_at_unix_ms: now_unix_ms() - 5_000,
            source: ModelCatalogEntrySourceKind::Cache,
            document: model_catalog_document("gpt-4o"),
        })
        .await
        .expect("stale cache should be written");

    let service = ModelCatalogService::with_remote_sources(store, Vec::new())
        .await
        .expect("service should load");
    let mut providers = ProviderRegistry::new();
    providers.register(StaticListProvider::new(
        "openai",
        "gpt-5.4",
        vec![
            Model::new("openai", "openai.gpt-5.4")
                .with_display_name("GPT-5.4")
                .with_metadata(ModelMetadata {
                    description: Some("Official OpenAI model".to_owned()),
                    ..ModelMetadata::default()
                }),
        ],
    ));

    let snapshot = service
        .refresh_if_stale_on_startup(&providers, None)
        .await
        .expect("stale startup refresh should succeed");

    assert!(snapshot.official.models.contains_key("gpt-5.4"));
    assert_eq!(
        snapshot.last_successful_source,
        Some(ModelCatalogEntrySourceKind::Generated)
    );
    assert_eq!(
        snapshot
            .official
            .models
            .get("gpt-5.4")
            .and_then(|definition| definition.origin.as_deref()),
        Some("OpenAI")
    );
}

#[tokio::test]
async fn refresh_from_registry_merges_public_sources_and_keeps_custom_appendable_only() {
    let mut server = mockito::Server::new_async().await;
    let _models_dev = server
        .mock("GET", "/models-dev.json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            serde_json::json!({
                "openai": {
                    "id": "openai",
                    "name": "OpenAI",
                    "models": {
                        "gpt-5": {
                            "id": "gpt-5",
                            "name": "GPT-5",
                            "description": "Models.dev GPT-5 description",
                            "knowledge": "2025-04",
                            "release_date": "2026-04-22",
                            "last_updated": "2026-04-24",
                            "open_weights": false,
                            "interleaved": {
                                "field": "reasoning_content"
                            },
                            "reasoning": true,
                            "tool_call": true,
                            "structured_output": true,
                            "temperature": false,
                            "modalities": {
                                "input": ["text", "image"],
                                "output": ["text", "image"]
                            },
                            "cost": {
                                "input": 1.25,
                                "output": 10,
                                "cache_read": 0.125,
                                "tiers": [{
                                    "type": "context",
                                    "size": 200000,
                                    "input": 2.5,
                                    "output": 15
                                }]
                            },
                            "limit": {
                                "context": 400000,
                                "input": 300000,
                                "output": 128000
                            },
                            "experimental": {
                                "modes": {
                                    "fast": {
                                        "provider": {
                                            "headers": {
                                                "openai-beta": "fast-mode-2026-02-01"
                                            },
                                            "body": {
                                                "service_tier": "priority"
                                            }
                                        }
                                    }
                                }
                            }
                        },
                        "claude-sonnet-4-6": {
                            "id": "claude-sonnet-4-6",
                            "name": "Claude Sonnet 4.6",
                            "description": "Interleaved boolean source",
                            "reasoning": true,
                            "interleaved": true
                        }
                    }
                }
            })
            .to_string(),
        )
        .create_async()
        .await;
    let _codex_models = server
        .mock("GET", "/openai-codex-models.json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            serde_json::json!({
                "models": [{
                    "slug": "gpt-5",
                    "display_name": "GPT-5",
                    "description": "Frontier coding model",
                    "default_reasoning_level": "medium",
                    "supports_parallel_tool_calls": true,
                    "support_verbosity": true,
                    "default_verbosity": "low",
                    "input_modalities": ["text", "image"],
                    "context_window": 400000,
                    "supported_reasoning_levels": [{
                        "effort": "low",
                        "description": "Fast responses with lighter reasoning"
                    }, {
                        "effort": "medium",
                        "description": "Balanced reasoning"
                    }, {
                        "effort": "high",
                        "description": "Deep reasoning for complex work"
                    }, {
                        "effort": "xhigh",
                        "description": "Maximum reasoning depth"
                    }],
                    "service_tiers": [{
                        "id": "turbo",
                        "name": "Fast",
                        "description": "Priority route"
                    }],
                    "additional_speed_tiers": ["fast"]
                }]
            })
            .to_string(),
        )
        .create_async()
        .await;
    let _router = server
        .mock("GET", "/router.json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            serde_json::json!({
                "claude": [{
                    "id": "claude-opus-4-7",
                    "display_name": "Claude Opus 4.7",
                    "owned_by": "anthropic",
                    "description": "Anthropic route",
                    "context_length": 200000,
                    "max_completion_tokens": 64000,
                    "thinking": {
                        "zero_allowed": true,
                        "levels": ["low", "high", "max"]
                    }
                }, {
                    "id": "gemini-2.5-pro",
                    "display_name": "Gemini 2.5 Pro",
                    "owned_by": "google",
                    "description": "Google route",
                    "inputTokenLimit": 1048576,
                    "outputTokenLimit": 65536,
                    "thinking": {
                        "min": 1024,
                        "max": 32768,
                        "dynamic_allowed": true
                    }
                }]
            })
            .to_string(),
        )
        .create_async()
        .await;

    let dir = tempdir().expect("tempdir should create");
    let store = ModelCatalogStore::new(ModelCatalogConfig {
        cache_path: dir.path().join("model-catalog-cache.json"),
        custom_path: dir.path().join("model-catalog-custom.json"),
        cache_max_age_secs: 60,
    });
    let service = ModelCatalogService::with_remote_sources(
        store,
        vec![
            sources::ModelCatalogRemoteSource::new(
                "models.dev",
                sources::ModelCatalogRemoteSourceKind::ModelsDev,
                [format!("{}/models-dev.json", server.url())],
            ),
            sources::ModelCatalogRemoteSource::new(
                "router-for-me",
                sources::ModelCatalogRemoteSourceKind::RouterForMe,
                [format!("{}/router.json", server.url())],
            ),
            sources::ModelCatalogRemoteSource::new(
                "openai-codex-models",
                sources::ModelCatalogRemoteSourceKind::OpenAiCodexModels,
                [format!("{}/openai-codex-models.json", server.url())],
            ),
        ],
    )
    .await
    .expect("service should load");
    let mut providers = ProviderRegistry::new();
    providers.register(StaticListProvider::new(
        "gateway",
        "openai/gpt-5",
        vec![Model::new("gateway", "openai/gpt-5")],
    ));

    let snapshot = service
        .refresh_from_registry(&providers, None)
        .await
        .expect("refresh should succeed");

    let gpt5 = snapshot
        .official
        .models
        .get("gpt-5")
        .expect("gpt-5 should exist");
    assert_eq!(gpt5.display_name.as_deref(), Some("GPT-5"));
    assert_eq!(gpt5.origin.as_deref(), Some("OpenAI"));
    assert_eq!(gpt5.context_window_tokens, Some(400_000));
    assert_eq!(gpt5.max_input_tokens, Some(300_000));
    assert_eq!(gpt5.max_output_tokens, Some(128_000));
    assert_eq!(
        gpt5.description.as_deref(),
        Some("Models.dev GPT-5 description")
    );
    assert_eq!(gpt5.knowledge_cutoff.as_deref(), Some("2025-04"));
    assert_eq!(gpt5.release_date.as_deref(), Some("2026-04-22"));
    assert_eq!(gpt5.last_updated.as_deref(), Some("2026-04-24"));
    assert_eq!(gpt5.open_weights, Some(false));
    assert_eq!(
        gpt5.default_thinking_mode.as_deref(),
        Some("thinking-medium")
    );
    assert_eq!(gpt5.supports_parallel_tool_calls, Some(true));
    assert_eq!(gpt5.supports_verbosity, Some(true));
    assert_eq!(gpt5.default_verbosity.as_deref(), Some("low"));
    assert_eq!(gpt5.assistant_reasoning_interleaved, Some(true));
    assert_eq!(
        gpt5.assistant_reasoning_field.as_deref(),
        Some("reasoning_content")
    );
    assert_eq!(gpt5.output_modalities, vec!["text", "image"]);
    assert_eq!(
        gpt5.pricing
            .as_ref()
            .and_then(|pricing| pricing.input_usd_per_million_tokens.as_deref()),
        Some("1.25")
    );
    assert_eq!(
        gpt5.pricing
            .as_ref()
            .and_then(|pricing| pricing.output_usd_per_million_tokens.as_deref()),
        Some("10")
    );
    assert_eq!(
        gpt5.pricing.as_ref().map(|pricing| pricing.tiers.len()),
        Some(1)
    );
    assert_eq!(
        gpt5.capabilities
            .feature_support(ModelCapabilityFeature::Reasoning),
        Some(CapabilitySupport::Supported)
    );
    assert_eq!(
        gpt5.capabilities
            .feature_support(ModelCapabilityFeature::StructuredOutput),
        Some(CapabilitySupport::Supported)
    );

    let claude_sonnet = snapshot
        .official
        .models
        .get("claude-sonnet-4-6")
        .expect("claude-sonnet-4-6 should exist");
    assert_eq!(claude_sonnet.assistant_reasoning_interleaved, Some(true));
    assert_eq!(claude_sonnet.assistant_reasoning_field, None);
    assert_eq!(
        gpt5.capabilities
            .feature_support(ModelCapabilityFeature::Temperature),
        Some(CapabilitySupport::Unsupported)
    );
    assert!(gpt5.speed_modes.contains_key("fast"));
    assert_eq!(
        gpt5.speed_modes
            .get("fast")
            .and_then(|mode| mode.adapter_overrides.get("openai")),
        Some(&ModelSpeedModeRequestOverride {
            headers: BTreeMap::from([(
                "openai-beta".to_owned(),
                "fast-mode-2026-02-01".to_owned(),
            )]),
            body_patch: BTreeMap::from([(
                "service_tier".to_owned(),
                serde_json::json!("priority"),
            )]),
        })
    );
    assert_eq!(
        gpt5.speed_modes
            .get("fast")
            .and_then(|mode| mode.description.as_deref()),
        Some("Priority route")
    );
    assert_eq!(
        gpt5.thinking_modes
            .get("thinking-high")
            .and_then(|mode| mode.description.as_deref()),
        Some("Deep reasoning for complex work")
    );
    assert_eq!(
        gpt5.thinking_modes
            .get("thinking-xhigh")
            .and_then(|mode| mode.thinking.as_ref()),
        Some(&ThinkingRequest::Effort {
            effort: crate::provider::ReasoningEffort::Xhigh,
        })
    );

    let claude = snapshot
        .official
        .models
        .get("claude-opus-4-7")
        .expect("claude-opus-4-7 should exist");
    assert_eq!(claude.origin.as_deref(), Some("Anthropic"));
    assert_eq!(claude.description.as_deref(), Some("Anthropic route"));
    assert_eq!(claude.context_window_tokens, Some(200_000));
    assert_eq!(claude.max_input_tokens, None);
    assert_eq!(claude.max_output_tokens, Some(64_000));
    assert_eq!(
        claude
            .capabilities
            .feature_support(ModelCapabilityFeature::Reasoning),
        Some(CapabilitySupport::Supported)
    );
    assert!(claude.thinking_modes.contains_key("no-thinking"));
    assert!(claude.thinking_modes.contains_key("thinking-low"));
    assert!(claude.thinking_modes.contains_key("thinking-high"));
    assert!(claude.thinking_modes.contains_key("thinking-max"));

    let gemini = snapshot
        .official
        .models
        .get("gemini-2.5-pro")
        .expect("gemini-2.5-pro should exist");
    assert_eq!(gemini.origin.as_deref(), Some("Google"));
    assert_eq!(gemini.context_window_tokens, Some(1_048_576));
    assert_eq!(gemini.max_input_tokens, Some(1_048_576));
    assert_eq!(gemini.max_output_tokens, Some(65_536));
    assert_eq!(
        gemini
            .capabilities
            .feature_support(ModelCapabilityFeature::Reasoning),
        Some(CapabilitySupport::Supported)
    );
    assert_eq!(
        gemini
            .thinking_modes
            .get("thinking-high")
            .and_then(|mode| mode.thinking.as_ref()),
        Some(&ThinkingRequest::Budget {
            budget_tokens: 16_384,
        })
    );
    assert_eq!(
        gemini
            .thinking_modes
            .get("thinking-max")
            .and_then(|mode| mode.thinking.as_ref()),
        Some(&ThinkingRequest::Budget {
            budget_tokens: 32_768,
        })
    );

    let merged = snapshot.merged_models();
    assert!(
        merged.appendable_model_ids.is_empty(),
        "official public sources should not append every catalog model into provider /models"
    );
}

#[test]
fn catalog_definition_from_model_preserves_sampling_defaults() {
    let model = Model::new("openai", "google/gemini-2.5-pro").with_metadata(
        crate::provider::ModelMetadata::default()
            .with_default_temperature("1.0")
            .with_default_top_p("0.95")
            .with_default_top_k(64),
    );

    let definition = catalog_definition_from_model(&model);
    assert_eq!(definition.default_temperature.as_deref(), Some("1.0"));
    assert_eq!(definition.default_top_p.as_deref(), Some("0.95"));
    assert_eq!(definition.default_top_k, Some(64));

    let record = ModelCatalogSnapshot::entry_record("google/gemini-2.5-pro", &definition, false);
    assert_eq!(record.default_temperature.as_deref(), Some("1.0"));
    assert_eq!(record.default_top_p.as_deref(), Some("0.95"));
    assert_eq!(record.default_top_k, Some(64));
}

#[test]
fn merge_catalog_speed_mode_merges_request_overrides_and_adapter_overrides() {
    let mut current = ConfiguredModelSpeedMode {
        display_name: Some("Fast".to_owned()),
        description: None,
        request_override: ModelSpeedModeRequestOverride {
            headers: BTreeMap::from([("x-base".to_owned(), "one".to_owned())]),
            body_patch: BTreeMap::from([(
                "response_format".to_owned(),
                serde_json::json!({
                    "type": "json_object"
                }),
            )]),
        },
        adapter_overrides: BTreeMap::from([(
            "openai".to_owned(),
            ModelSpeedModeRequestOverride {
                headers: BTreeMap::new(),
                body_patch: BTreeMap::from([(
                    "service_tier".to_owned(),
                    serde_json::json!("default"),
                )]),
            },
        )]),
        disabled: false,
    };
    let next = ConfiguredModelSpeedMode {
        display_name: None,
        description: Some("Priority route".to_owned()),
        request_override: ModelSpeedModeRequestOverride {
            headers: BTreeMap::from([("x-extra".to_owned(), "two".to_owned())]),
            body_patch: BTreeMap::from([(
                "response_format".to_owned(),
                serde_json::json!({
                    "strict": true
                }),
            )]),
        },
        adapter_overrides: BTreeMap::from([(
            "openai".to_owned(),
            ModelSpeedModeRequestOverride {
                headers: BTreeMap::from([("openai-beta".to_owned(), "fast".to_owned())]),
                body_patch: BTreeMap::from([(
                    "service_tier".to_owned(),
                    serde_json::json!("priority"),
                )]),
            },
        )]),
        disabled: false,
    };

    merge_catalog_speed_mode(&mut current, &next);

    assert_eq!(current.display_name.as_deref(), Some("Fast"));
    assert_eq!(current.description.as_deref(), Some("Priority route"));
    assert_eq!(
        current
            .request_override
            .headers
            .get("x-base")
            .map(String::as_str),
        Some("one")
    );
    assert_eq!(
        current
            .request_override
            .headers
            .get("x-extra")
            .map(String::as_str),
        Some("two")
    );
    assert_eq!(
        current.request_override.body_patch.get("response_format"),
        Some(&serde_json::json!({
            "type": "json_object",
            "strict": true
        }))
    );
    assert_eq!(
        current
            .adapter_overrides
            .get("openai")
            .and_then(|override_patch| override_patch.headers.get("openai-beta"))
            .map(String::as_str),
        Some("fast")
    );
    assert_eq!(
        current
            .adapter_overrides
            .get("openai")
            .and_then(|override_patch| override_patch.body_patch.get("service_tier")),
        Some(&serde_json::json!("priority"))
    );
}

#[test]
fn merge_catalog_thinking_mode_fill_missing_preserves_existing_override_values() {
    let mut current = ConfiguredModelThinkingMode {
        display_name: Some("Deep".to_owned()),
        description: None,
        thinking: Some(ThinkingRequest::Effort {
            effort: ReasoningEffort::High,
        }),
        request_override: ModelSpeedModeRequestOverride {
            headers: BTreeMap::from([("x-base".to_owned(), "one".to_owned())]),
            body_patch: BTreeMap::from([(
                "reasoning".to_owned(),
                serde_json::json!({ "summary": "auto" }),
            )]),
        },
        adapter_overrides: BTreeMap::from([(
            "openai".to_owned(),
            ModelSpeedModeRequestOverride {
                headers: BTreeMap::from([("x-profile".to_owned(), "deep".to_owned())]),
                body_patch: BTreeMap::new(),
            },
        )]),
        disabled: false,
    };
    let next = ConfiguredModelThinkingMode {
        display_name: None,
        description: Some("More reasoning".to_owned()),
        thinking: Some(ThinkingRequest::Effort {
            effort: ReasoningEffort::Low,
        }),
        request_override: ModelSpeedModeRequestOverride {
            headers: BTreeMap::from([
                ("x-base".to_owned(), "two".to_owned()),
                ("x-extra".to_owned(), "three".to_owned()),
            ]),
            body_patch: BTreeMap::from([(
                "reasoning".to_owned(),
                serde_json::json!({ "summary": "concise" }),
            )]),
        },
        adapter_overrides: BTreeMap::from([(
            "openai".to_owned(),
            ModelSpeedModeRequestOverride {
                headers: BTreeMap::from([
                    ("x-profile".to_owned(), "light".to_owned()),
                    ("x-extra".to_owned(), "adapter".to_owned()),
                ]),
                body_patch: BTreeMap::new(),
            },
        )]),
        disabled: false,
    };

    merge_catalog_thinking_mode(&mut current, &next);

    assert_eq!(current.display_name.as_deref(), Some("Deep"));
    assert_eq!(current.description.as_deref(), Some("More reasoning"));
    assert_eq!(
        current.thinking,
        Some(ThinkingRequest::Effort {
            effort: ReasoningEffort::High,
        })
    );
    assert_eq!(
        current
            .request_override
            .headers
            .get("x-base")
            .map(String::as_str),
        Some("one")
    );
    assert_eq!(
        current
            .request_override
            .headers
            .get("x-extra")
            .map(String::as_str),
        Some("three")
    );
    assert_eq!(
        current.request_override.body_patch.get("reasoning"),
        Some(&serde_json::json!({ "summary": "auto" }))
    );
    assert_eq!(
        current
            .adapter_overrides
            .get("openai")
            .and_then(|override_patch| override_patch.headers.get("x-profile"))
            .map(String::as_str),
        Some("deep")
    );
    assert_eq!(
        current
            .adapter_overrides
            .get("openai")
            .and_then(|override_patch| override_patch.headers.get("x-extra"))
            .map(String::as_str),
        Some("adapter")
    );
}

#[test]
fn merge_catalog_speed_mode_fill_missing_preserves_higher_priority_override_values() {
    let mut current = ConfiguredModelSpeedMode {
        display_name: Some("Fast".to_owned()),
        description: None,
        request_override: ModelSpeedModeRequestOverride {
            headers: BTreeMap::from([("x-base".to_owned(), "one".to_owned())]),
            body_patch: BTreeMap::from([(
                "service_tier".to_owned(),
                serde_json::json!("priority"),
            )]),
        },
        adapter_overrides: BTreeMap::from([(
            "openai".to_owned(),
            ModelSpeedModeRequestOverride {
                headers: BTreeMap::from([("openai-beta".to_owned(), "fast".to_owned())]),
                body_patch: BTreeMap::from([(
                    "service_tier".to_owned(),
                    serde_json::json!("priority"),
                )]),
            },
        )]),
        disabled: false,
    };
    let next = ConfiguredModelSpeedMode {
        display_name: None,
        description: Some("Priority route".to_owned()),
        request_override: ModelSpeedModeRequestOverride {
            headers: BTreeMap::from([
                ("x-base".to_owned(), "two".to_owned()),
                ("x-extra".to_owned(), "three".to_owned()),
            ]),
            body_patch: BTreeMap::from([("service_tier".to_owned(), serde_json::json!("turbo"))]),
        },
        adapter_overrides: BTreeMap::from([(
            "openai".to_owned(),
            ModelSpeedModeRequestOverride {
                headers: BTreeMap::from([
                    ("openai-beta".to_owned(), "slow".to_owned()),
                    ("openai-extra".to_owned(), "tier".to_owned()),
                ]),
                body_patch: BTreeMap::from([(
                    "service_tier".to_owned(),
                    serde_json::json!("turbo"),
                )]),
            },
        )]),
        disabled: false,
    };

    merge_catalog_speed_mode_fill_missing(&mut current, &next);

    assert_eq!(current.description.as_deref(), Some("Priority route"));
    assert_eq!(
        current
            .request_override
            .headers
            .get("x-base")
            .map(String::as_str),
        Some("one")
    );
    assert_eq!(
        current
            .request_override
            .headers
            .get("x-extra")
            .map(String::as_str),
        Some("three")
    );
    assert_eq!(
        current.request_override.body_patch.get("service_tier"),
        Some(&serde_json::json!("priority"))
    );
    assert_eq!(
        current
            .adapter_overrides
            .get("openai")
            .and_then(|override_patch| override_patch.headers.get("openai-beta"))
            .map(String::as_str),
        Some("fast")
    );
    assert_eq!(
        current
            .adapter_overrides
            .get("openai")
            .and_then(|override_patch| override_patch.headers.get("openai-extra"))
            .map(String::as_str),
        Some("tier")
    );
    assert_eq!(
        current
            .adapter_overrides
            .get("openai")
            .and_then(|override_patch| override_patch.body_patch.get("service_tier")),
        Some(&serde_json::json!("priority"))
    );
}

fn model_catalog_document(model_id: &str) -> ModelCatalogDocument {
    ModelCatalogDocument {
        models: BTreeMap::from([(
            model_id.to_owned(),
            CatalogModelDefinition {
                display_name: Some(model_id.to_owned()),
                ..CatalogModelDefinition::default()
            },
        )]),
    }
}
