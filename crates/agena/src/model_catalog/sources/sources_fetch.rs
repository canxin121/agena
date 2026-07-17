use futures_util::StreamExt;

pub async fn fetch_documents(
    client: &reqwest::Client,
    sources: &[ModelCatalogRemoteSource],
) -> (Vec<FetchedModelCatalogDocument>, Vec<String>) {
    let mut documents = Vec::new();
    let mut warnings = Vec::new();

    let results = join_all(
        sources
            .iter()
            .map(|source| async move { (source, fetch_source_document(client, source).await) }),
    )
    .await;

    for (source, result) in results {
        match result {
            Ok(document) => documents.push(FetchedModelCatalogDocument {
                name: source.name.clone(),
                kind: source.kind,
                grade: source.grade,
                document: annotate_document_source_priority(document, source.grade),
            }),
            Err(error) => warnings.push(format!("{}: {error}", source.name)),
        }
    }

    (documents, warnings)
}

async fn fetch_source_document(
    client: &reqwest::Client,
    source: &ModelCatalogRemoteSource,
) -> Result<ModelCatalogDocument, AppError> {
    let mut last_error = None;

    for url in &source.urls {
        match fetch_and_parse_source_document(client, source.kind, url).await {
            Ok(document) => return Ok(document),
            Err(error) => last_error = Some(format!("{url}: {error}")),
        }
    }

    Err(AppError::Config(format!(
        "all source URLs failed for {}: {}",
        source.name,
        last_error.unwrap_or_else(|| "no URLs configured".to_owned())
    )))
}

async fn fetch_and_parse_source_document(
    client: &reqwest::Client,
    kind: ModelCatalogRemoteSourceKind,
    url: &str,
) -> Result<ModelCatalogDocument, AppError> {
    let response = client.get(url).send().await?.error_for_status()?;
    let body = response.text().await?;
    match kind {
        ModelCatalogRemoteSourceKind::ModelsDev => parse_models_dev_document(body.as_str()),
        ModelCatalogRemoteSourceKind::OpenAiCodexModels => {
            parse_openai_codex_models_document(body.as_str())
        }
        ModelCatalogRemoteSourceKind::HuggingFaceOfficial => {
            parse_hugging_face_official_document(body.as_str())
        }
        ModelCatalogRemoteSourceKind::OfficialHtmlSignals => {
            parse_official_html_source_document(client, url, body.as_str()).await
        }
        ModelCatalogRemoteSourceKind::RouterForMe => parse_router_document(body.as_str()),
    }
}

async fn parse_official_html_source_document(
    client: &reqwest::Client,
    url: &str,
    body: &str,
) -> Result<ModelCatalogDocument, AppError> {
    let mut document = parse_official_html_signals_document(body)?;

    if !is_nvidia_reference_index_url(url) {
        return Ok(document);
    }

    let pages = parse_official_html_reference_index_document(body)?;
    if pages.is_empty() {
        return Ok(document);
    }

    let results = stream::iter(pages.into_iter().map(|page| async move {
        fetch_official_html_reference_page_document(client, page).await
    }))
    .buffer_unordered(24)
    .collect::<Vec<_>>()
    .await;

    for result in results {
        let Ok(Some((aliases, definition))) = result else {
            continue;
        };
        for alias in aliases {
            merge_document_entry(&mut document.models, alias, definition.clone());
        }
    }

    Ok(document)
}

fn parse_models_dev_document(body: &str) -> Result<ModelCatalogDocument, AppError> {
    let providers: BTreeMap<String, ModelsDevProvider> = serde_json::from_str(body)?;
    let mut providers: Vec<_> = providers.into_iter().collect();
    providers.sort_by(|(left_key, left), (right_key, right)| {
        models_dev_provider_rank(right_key, right)
            .cmp(&models_dev_provider_rank(left_key, left))
            .then_with(|| left_key.cmp(right_key))
    });

    let mut document_models = BTreeMap::new();

    for (provider_key, provider) in providers {
        let origin = models_dev_origin(provider_key.as_str(), &provider);
        let adapter_id = models_dev_adapter_id(provider_key.as_str(), &provider);
        let mut models: Vec<_> = provider.models.into_iter().collect();
        models.sort_by(|(left, _), (right, _)| left.cmp(right));

        for (fallback_model_id, model) in models {
            let model_id = normalize_optional_string(model.id)
                .unwrap_or_else(|| fallback_model_id.trim().to_owned());
            if model_id.is_empty() {
                continue;
            }

            let definition = CatalogModelDefinition {
                lifecycle: parse_models_dev_lifecycle(model.status.as_deref()),
                context_window_tokens: model
                    .limit
                    .as_ref()
                    .and_then(|limits| limits.context.or(limits.input))
                    .map(clamp_u64_to_u32),
                max_input_tokens: model
                    .limit
                    .as_ref()
                    .and_then(|limits| limits.input)
                    .map(clamp_u64_to_u32),
                max_output_tokens: model
                    .limit
                    .as_ref()
                    .and_then(|limits| limits.output)
                    .map(clamp_u64_to_u32),
                description: normalize_optional_string(model.description),
                knowledge_cutoff: normalize_optional_string(model.knowledge),
                release_date: normalize_optional_string(model.release_date),
                last_updated: normalize_optional_string(model.last_updated),
                open_weights: model.open_weights,
                default_thinking_mode: None,
                supports_parallel_tool_calls: None,
                supports_verbosity: None,
                default_verbosity: None,
                default_temperature: None,
                default_top_p: None,
                default_top_k: None,
                assistant_reasoning_interleaved: models_dev_assistant_reasoning_interleaved(
                    model.interleaved.as_ref(),
                ),
                assistant_reasoning_field: models_dev_assistant_reasoning_field(
                    model.interleaved.as_ref(),
                ),
                output_modalities: models_dev_output_modalities(model.modalities.as_ref()),
                pricing: models_dev_pricing(model.cost.as_ref()),
                display_name: normalize_optional_string(model.name),
                origin: origin.clone(),
                thinking_modes: BTreeMap::new(),
                speed_modes: models_dev_speed_modes(
                    model.experimental.as_ref(),
                    adapter_id.as_deref(),
                ),
                capabilities: model_capability_patch(
                    models_dev_input_support(model.modalities.as_ref(), model.attachment),
                    features_from_bool_flags(&[
                        (ModelCapabilityFeature::Reasoning, model.reasoning),
                        (ModelCapabilityFeature::ToolCalling, model.tool_call),
                        (
                            ModelCapabilityFeature::StructuredOutput,
                            model.structured_output,
                        ),
                        (ModelCapabilityFeature::Temperature, model.temperature),
                    ]),
                ),
                source_priority: CatalogDefinitionSourcePriority::default(),
            };

            merge_document_entry(&mut document_models, model_id, definition);
        }
    }

    Ok(ModelCatalogDocument {
        models: document_models,
    })
}

fn parse_router_document(body: &str) -> Result<ModelCatalogDocument, AppError> {
    let sections: BTreeMap<String, Vec<RouterModel>> = serde_json::from_str(body)?;
    let mut document_models = BTreeMap::new();

    for (section, models) in sections {
        for model in models {
            let model_id = normalize_optional_string(model.id.clone())
                .or_else(|| normalize_optional_string(model.name.clone()))
                .unwrap_or_default();
            if model_id.is_empty() {
                continue;
            }

            let origin = router_origin(section.as_str(), &model);
            let input_support = router_input_support(&model);
            let mut supported_features = Vec::new();
            let unsupported_features = Vec::new();
            if model.thinking.is_some() {
                supported_features.push(ModelCapabilityFeature::Reasoning);
            }
            if let Some(parameters) = model.supported_parameters.as_ref() {
                if parameters
                    .iter()
                    .any(|parameter| parameter.eq_ignore_ascii_case("temperature"))
                {
                    supported_features.push(ModelCapabilityFeature::Temperature);
                }
                if parameters
                    .iter()
                    .any(|parameter| parameter.eq_ignore_ascii_case("tools"))
                {
                    supported_features.push(ModelCapabilityFeature::ToolCalling);
                }
                if parameters.iter().any(|parameter| {
                    parameter.eq_ignore_ascii_case("response_format")
                        || parameter.eq_ignore_ascii_case("json_schema")
                }) {
                    supported_features.push(ModelCapabilityFeature::StructuredOutput);
                }
            }

            let definition = CatalogModelDefinition {
                lifecycle: None,
                context_window_tokens: model
                    .context_length
                    .or(model.input_token_limit)
                    .map(clamp_u64_to_u32),
                max_input_tokens: model.input_token_limit.map(clamp_u64_to_u32),
                max_output_tokens: model
                    .max_completion_tokens
                    .or(model.output_token_limit)
                    .map(clamp_u64_to_u32),
                description: normalize_optional_string(model.description),
                knowledge_cutoff: None,
                release_date: None,
                last_updated: None,
                open_weights: None,
                default_thinking_mode: None,
                supports_parallel_tool_calls: None,
                supports_verbosity: None,
                default_verbosity: None,
                default_temperature: None,
                default_top_p: None,
                default_top_k: None,
                assistant_reasoning_interleaved: None,
                assistant_reasoning_field: None,
                output_modalities: Vec::new(),
                pricing: None,
                display_name: normalize_optional_string(model.display_name),
                origin,
                speed_modes: BTreeMap::new(),
                thinking_modes: router_thinking_modes(model.thinking.as_ref()),
                capabilities: model_capability_patch(
                    input_support,
                    (supported_features, unsupported_features),
                ),
                source_priority: CatalogDefinitionSourcePriority::default(),
            };

            merge_document_entry(&mut document_models, model_id, definition);
        }
    }

    Ok(ModelCatalogDocument {
        models: document_models,
    })
}

fn parse_openai_codex_models_document(body: &str) -> Result<ModelCatalogDocument, AppError> {
    let payload: OpenAiCodexModelsDocument = serde_json::from_str(body)?;
    let mut document_models = BTreeMap::new();

    for model in payload.models {
        let model_id = normalize_optional_string(model.slug).unwrap_or_default();
        if model_id.is_empty() {
            continue;
        }

        let description = normalize_optional_string(model.description);
        let display_name = normalize_optional_string(model.display_name);
        let thinking_modes = codex_thinking_modes(model.supported_reasoning_levels.as_deref());
        let speed_modes = codex_speed_modes(
            model.service_tiers.as_deref(),
            model.additional_speed_tiers.as_deref(),
        );
        let definition = CatalogModelDefinition {
            lifecycle: None,
            context_window_tokens: model
                .context_window
                .max(model.max_context_window)
                .map(clamp_u64_to_u32),
            max_input_tokens: None,
            max_output_tokens: None,
            description,
            knowledge_cutoff: None,
            release_date: None,
            last_updated: None,
            open_weights: None,
            default_thinking_mode: codex_default_thinking_mode(
                model.default_reasoning_level.as_deref(),
            ),
            supports_parallel_tool_calls: model.supports_parallel_tool_calls,
            supports_verbosity: model.support_verbosity,
            default_verbosity: normalize_optional_string(model.default_verbosity),
            default_temperature: None,
            default_top_p: None,
            default_top_k: None,
            assistant_reasoning_interleaved: None,
            assistant_reasoning_field: None,
            output_modalities: Vec::new(),
            pricing: None,
            display_name,
            origin: Some("OpenAI".to_owned()),
            thinking_modes,
            speed_modes,
            capabilities: model_capability_patch(
                (
                    codex_input_support(model.input_modalities.as_deref()),
                    Vec::new(),
                ),
                (
                    [
                        (!model
                            .supported_reasoning_levels
                            .as_deref()
                            .unwrap_or(&[])
                            .is_empty())
                        .then_some(ModelCapabilityFeature::Reasoning),
                        (model.supports_parallel_tool_calls == Some(true))
                            .then_some(ModelCapabilityFeature::ToolCalling),
                    ]
                    .into_iter()
                    .flatten()
                    .collect(),
                    Vec::new(),
                ),
            ),
            source_priority: CatalogDefinitionSourcePriority::default(),
        };

        merge_document_entry(&mut document_models, model_id, definition);
    }

    Ok(ModelCatalogDocument {
        models: document_models,
    })
}

fn parse_hugging_face_official_document(body: &str) -> Result<ModelCatalogDocument, AppError> {
    let payload: Vec<HuggingFaceHubModel> = serde_json::from_str(body)?;
    let mut document_models = BTreeMap::new();

    for model in payload {
        let repo_id = normalize_optional_string(model.id.or(model.model_id)).unwrap_or_default();
        if repo_id.is_empty() || model.private || !hugging_face_model_is_supported(repo_id.as_str())
        {
            continue;
        }

        let Some((owner, repo_name)) = repo_id.split_once('/') else {
            continue;
        };
        let Some(origin) = hugging_face_owner_origin(owner) else {
            continue;
        };

        let definition = CatalogModelDefinition {
            lifecycle: None,
            context_window_tokens: None,
            max_input_tokens: None,
            max_output_tokens: None,
            description: None,
            knowledge_cutoff: None,
            release_date: hugging_face_release_date(model.created_at.as_deref()),
            last_updated: None,
            open_weights: Some(true),
            default_thinking_mode: None,
            supports_parallel_tool_calls: None,
            supports_verbosity: None,
            default_verbosity: None,
            default_temperature: None,
            default_top_p: None,
            default_top_k: None,
            assistant_reasoning_interleaved: None,
            assistant_reasoning_field: None,
            output_modalities: hugging_face_output_modalities(
                model.pipeline_tag.as_deref(),
                repo_name,
                model.tags.as_deref(),
            ),
            pricing: None,
            display_name: Some(repo_name.to_owned()),
            origin: Some(origin.to_owned()),
            thinking_modes: BTreeMap::new(),
            speed_modes: BTreeMap::new(),
            capabilities: hugging_face_capability_patch(
                model.pipeline_tag.as_deref(),
                repo_name,
                model.tags.as_deref(),
            ),
            source_priority: CatalogDefinitionSourcePriority::default(),
        };

        for alias in hugging_face_model_aliases(repo_name) {
            merge_document_entry(&mut document_models, alias, definition.clone());
        }
    }

    Ok(ModelCatalogDocument {
        models: document_models,
    })
}

#[derive(Debug, Clone, Copy)]
struct OfficialHtmlModelSignal {
    model_ids: &'static [&'static str],
    display_name: &'static str,
    origin: &'static str,
    markers: &'static [&'static str],
    input_modalities: &'static [ModelInputModality],
    output_modalities: &'static [&'static str],
}

const OFFICIAL_HTML_MODEL_SIGNALS: &[OfficialHtmlModelSignal] = &[
    OfficialHtmlModelSignal {
        model_ids: &["dbrx-instruct"],
        display_name: "dbrx-instruct",
        origin: "Databricks",
        markers: &["filename\":\"nvidia-nim-api-for-databricksdbrx-instruct.json\""],
        input_modalities: &[],
        output_modalities: &[],
    },
    OfficialHtmlModelSignal {
        model_ids: &["embed-qa-4"],
        display_name: "embed-qa-4",
        origin: "NVIDIA",
        markers: &["href=\"/nim/reference/nvidia-embed-qa-4\""],
        input_modalities: &[],
        output_modalities: &[],
    },
    OfficialHtmlModelSignal {
        model_ids: &["ising-calibration-1-35b-a3b"],
        display_name: "ising-calibration-1-35b-a3b",
        origin: "NVIDIA",
        markers: &["href=\"/nim/reference/nvidia-ising-calibration-1-35b-a3b\""],
        input_modalities: &[],
        output_modalities: &[],
    },
    OfficialHtmlModelSignal {
        model_ids: &["llama-3.2-nemoretriever-1b-vlm-embed-v1"],
        display_name: "llama-3.2-nemoretriever-1b-vlm-embed-v1",
        origin: "NVIDIA",
        markers: &["href=\"/nim/reference/nvidia-llama-3_2-nemoretriever-1b-vlm-embed-v1\""],
        input_modalities: &[],
        output_modalities: &[],
    },
    OfficialHtmlModelSignal {
        model_ids: &["llama-3.2-nv-embedqa-1b-v1"],
        display_name: "llama-3.2-nv-embedqa-1b-v1",
        origin: "NVIDIA",
        markers: &["href=\"/nim/reference/nvidia-llama-3_2-nv-embedqa-1b-v1\""],
        input_modalities: &[],
        output_modalities: &[],
    },
    OfficialHtmlModelSignal {
        model_ids: &["llama-3.2-nv-embedqa-1b-v2"],
        display_name: "llama-3.2-nv-embedqa-1b-v2",
        origin: "NVIDIA",
        markers: &["href=\"/nim/reference/nvidia-llama-3_2-nv-embedqa-1b-v2\""],
        input_modalities: &[],
        output_modalities: &[],
    },
    OfficialHtmlModelSignal {
        model_ids: &["nemoretriever-parse"],
        display_name: "nemoretriever-parse",
        origin: "NVIDIA",
        markers: &["href=\"/nim/reference/nvidia-nemoretriever-parse\""],
        input_modalities: &[],
        output_modalities: &[],
    },
    OfficialHtmlModelSignal {
        model_ids: &["nemotron-4-340b-reward"],
        display_name: "nemotron-4-340b-reward",
        origin: "NVIDIA",
        markers: &["filename\":\"nvidia-nim-api-for-nvidianemotron-4-340b-reward.json\""],
        input_modalities: &[],
        output_modalities: &[],
    },
    OfficialHtmlModelSignal {
        model_ids: &["neva-22b"],
        display_name: "neva-22b",
        origin: "NVIDIA",
        markers: &["filename\":\"nvidia-nim-api-for-nvidianeva-22b.json\""],
        input_modalities: &[],
        output_modalities: &[],
    },
    OfficialHtmlModelSignal {
        model_ids: &["nv-embedqa-e5-v5"],
        display_name: "nv-embedqa-e5-v5",
        origin: "NVIDIA",
        markers: &["href=\"/nim/reference/nvidia-nv-embedqa-e5-v5\""],
        input_modalities: &[],
        output_modalities: &[],
    },
    OfficialHtmlModelSignal {
        model_ids: &["nv-embedqa-mistral-7b-v2"],
        display_name: "nv-embedqa-mistral-7b-v2",
        origin: "NVIDIA",
        markers: &["filename\":\"nvidia-nim-api-for-nvidianv-embedqa-mistral-7b-v2.json\""],
        input_modalities: &[],
        output_modalities: &[],
    },
    OfficialHtmlModelSignal {
        model_ids: &["nvclip"],
        display_name: "nvclip",
        origin: "NVIDIA",
        markers: &["href=\"/nim/reference/nvidia-nvclip\""],
        input_modalities: &[],
        output_modalities: &[],
    },
    OfficialHtmlModelSignal {
        model_ids: &["vila"],
        display_name: "vila",
        origin: "NVIDIA",
        markers: &["href=\"/nim/reference/nvidia-vila\""],
        input_modalities: &[],
        output_modalities: &[],
    },
    OfficialHtmlModelSignal {
        model_ids: &["ai-synthetic-video-detector", "synthetic-video-detector"],
        display_name: "synthetic-video-detector",
        origin: "NVIDIA",
        markers: &["synthetic-video-detector"],
        input_modalities: &[ModelInputModality::Video],
        output_modalities: &["text"],
    },
];

fn parse_official_html_signals_document(body: &str) -> Result<ModelCatalogDocument, AppError> {
    let normalized = body.trim().to_ascii_lowercase();
    let mut document_models = BTreeMap::new();

    for signal in OFFICIAL_HTML_MODEL_SIGNALS {
        if !signal
            .markers
            .iter()
            .any(|marker| normalized.contains(marker))
        {
            continue;
        }

        let definition = CatalogModelDefinition {
            lifecycle: None,
            context_window_tokens: None,
            max_input_tokens: None,
            max_output_tokens: None,
            description: None,
            knowledge_cutoff: None,
            release_date: None,
            last_updated: None,
            open_weights: None,
            default_thinking_mode: None,
            supports_parallel_tool_calls: None,
            supports_verbosity: None,
            default_verbosity: None,
            default_temperature: None,
            default_top_p: None,
            default_top_k: None,
            assistant_reasoning_interleaved: None,
            assistant_reasoning_field: None,
            output_modalities: signal
                .output_modalities
                .iter()
                .map(|modality| (*modality).to_owned())
                .collect(),
            pricing: None,
            display_name: Some(signal.display_name.to_owned()),
            origin: Some(signal.origin.to_owned()),
            thinking_modes: BTreeMap::new(),
            speed_modes: BTreeMap::new(),
            capabilities: model_capability_patch(
                (signal.input_modalities.to_vec(), Vec::new()),
                (Vec::new(), Vec::new()),
            ),
            source_priority: CatalogDefinitionSourcePriority::default(),
        };

        for model_id in signal.model_ids {
            merge_document_entry(
                &mut document_models,
                (*model_id).to_owned(),
                definition.clone(),
            );
        }
    }

    Ok(ModelCatalogDocument {
        models: document_models,
    })
}

fn is_nvidia_reference_index_url(url: &str) -> bool {
    url.trim()
        .to_ascii_lowercase()
        .contains("docs.api.nvidia.com/nim/reference/models-1")
}

fn parse_official_html_reference_index_document(
    body: &str,
) -> Result<Vec<OfficialHtmlReferencePage>, AppError> {
    let Some(json) = extract_official_html_ssr_props_json(body) else {
        return Ok(Vec::new());
    };

    let props: OfficialHtmlSsrProps = serde_json::from_str(json)?;
    let mut pages = Vec::new();
    for reference in props.sidebars.refs {
        collect_official_html_reference_pages(&reference.pages, &mut pages);
    }
    Ok(pages)
}

fn collect_official_html_reference_pages(
    candidates: &[OfficialHtmlSsrPage],
    pages: &mut Vec<OfficialHtmlReferencePage>,
) {
    for page in candidates {
        if let Some(reference_page) = page.as_reference_page()
            && !pages
                .iter()
                .any(|existing| existing.slug == reference_page.slug)
        {
            pages.push(reference_page);
        }
        collect_official_html_reference_pages(&page.children, pages);
    }
}

async fn fetch_official_html_reference_page_document(
    client: &reqwest::Client,
    page: OfficialHtmlReferencePage,
) -> Result<Option<(Vec<String>, CatalogModelDefinition)>, AppError> {
    let url = format!("https://docs.api.nvidia.com/nim/reference/{}", page.slug);
    let mut last_error = None;

    for _ in 0..2 {
        match client.get(&url).send().await {
            Ok(response) => match response.error_for_status() {
                Ok(response) => {
                    let body = response.text().await?;
                    return Ok(parse_official_html_reference_page_document(
                        page.title.as_str(),
                        page.slug.as_str(),
                        body.as_str(),
                    ));
                }
                Err(error) => last_error = Some(error),
            },
            Err(error) => last_error = Some(error),
        }
    }

    Err(last_error
        .map(AppError::from)
        .unwrap_or_else(|| AppError::Config(format!("fetch official html page failed: {url}"))))
}

fn parse_official_html_reference_page_document(
    title: &str,
    slug: &str,
    body: &str,
) -> Option<(Vec<String>, CatalogModelDefinition)> {
    let limits = parse_official_html_token_limits(body)?;
    let aliases = official_html_reference_page_aliases(title, slug);
    if aliases.is_empty() {
        return None;
    }

    let (display_name, origin) = official_html_display_name_and_origin(title);
    Some((
        aliases,
        CatalogModelDefinition {
            lifecycle: None,
            context_window_tokens: Some(limits.context_window_tokens),
            max_input_tokens: Some(
                limits
                    .max_input_tokens
                    .unwrap_or(limits.context_window_tokens),
            ),
            max_output_tokens: None,
            description: None,
            knowledge_cutoff: None,
            release_date: None,
            last_updated: None,
            open_weights: None,
            default_thinking_mode: None,
            supports_parallel_tool_calls: None,
            supports_verbosity: None,
            default_verbosity: None,
            default_temperature: None,
            default_top_p: None,
            default_top_k: None,
            assistant_reasoning_interleaved: None,
            assistant_reasoning_field: None,
            output_modalities: Vec::new(),
            pricing: None,
            display_name,
            origin,
            thinking_modes: BTreeMap::new(),
            speed_modes: BTreeMap::new(),
            capabilities: ModelCapabilityPatch::default(),
            source_priority: CatalogDefinitionSourcePriority::default(),
        },
    ))
}

fn official_html_reference_page_aliases(title: &str, slug: &str) -> Vec<String> {
    let mut aliases = Vec::new();

    if let Some((owner, model_name)) = split_official_html_reference_title(title) {
        push_official_html_alias(&mut aliases, model_name);
        push_official_html_alias(&mut aliases, format!("{owner}/{model_name}"));
        push_official_html_alias(&mut aliases, format!("{owner}-{model_name}"));
    }

    push_official_html_alias(&mut aliases, slug);
    aliases
}

fn push_official_html_alias(aliases: &mut Vec<String>, value: impl AsRef<str>) {
    let canonical = canonical_model_catalog_id(value.as_ref());
    if !canonical.is_empty() && !aliases.contains(&canonical) {
        aliases.push(canonical);
    }
}

fn official_html_display_name_and_origin(title: &str) -> (Option<String>, Option<String>) {
    let Some((owner, model_name)) = split_official_html_reference_title(title) else {
        return (None, None);
    };
    let origin = official_html_owner_origin(owner)
        .map(str::to_owned)
        .or_else(|| Some(title_case_tokenized(owner)));
    (Some(model_name.to_owned()), origin)
}

fn split_official_html_reference_title(title: &str) -> Option<(&str, &str)> {
    let (owner, model_name) = title.split_once('/')?;
    let owner = owner.trim();
    let model_name = model_name.trim();
    (!owner.is_empty() && !model_name.is_empty()).then_some((owner, model_name))
}

fn official_html_owner_origin(owner: &str) -> Option<&'static str> {
    match owner.trim().to_ascii_lowercase().as_str() {
        "abacusai" => Some("Abacus.AI"),
        "bytedance" => Some("ByteDance"),
        "meta" => Some("Meta"),
        "minimaxai" => Some("MiniMax"),
        "moonshotai" => Some("Moonshot AI"),
        "openai" => Some("OpenAI"),
        "z-ai" | "zai" => Some("Z.AI"),
        other => hugging_face_owner_origin(other),
    }
}

fn parse_official_html_token_limits(body: &str) -> Option<OfficialHtmlTokenLimits> {
    let plain_text = official_html_plain_text(body);

    let mut context_values =
        capture_token_values(plain_text.as_str(), official_html_input_context_length_re());
    context_values.extend(capture_token_values(
        plain_text.as_str(),
        official_html_context_length_re(),
    ));
    context_values.extend(capture_token_values(
        plain_text.as_str(),
        official_html_context_window_re(),
    ));
    context_values.extend(capture_token_values(
        plain_text.as_str(),
        official_html_leading_context_size_re(),
    ));
    context_values.extend(capture_token_values(
        plain_text.as_str(),
        official_html_supports_context_size_re(),
    ));

    let context_window_tokens = context_values.into_iter().max()?;
    let max_input_tokens =
        capture_token_values(plain_text.as_str(), official_html_input_context_length_re())
            .into_iter()
            .max()
            .or(Some(context_window_tokens));

    Some(OfficialHtmlTokenLimits {
        context_window_tokens,
        max_input_tokens,
    })
}

fn capture_token_values(text: &str, pattern: &Regex) -> Vec<u32> {
    pattern
        .captures_iter(text)
        .filter_map(|capture| capture.name("value"))
        .filter_map(|value| parse_token_quantity(value.as_str()))
        .collect()
}

fn parse_token_quantity(raw: &str) -> Option<u32> {
    let normalized = raw.trim().to_ascii_lowercase().replace(',', "");
    let (numeric, multiplier) = if let Some(value) = normalized.strip_suffix("thousand") {
        (value.trim(), 1_000.0)
    } else if let Some(value) = normalized.strip_suffix("million") {
        (value.trim(), 1_000_000.0)
    } else if let Some(value) = normalized.strip_suffix("billion") {
        (value.trim(), 1_000_000_000.0)
    } else if let Some(value) = normalized.strip_suffix('k') {
        (value.trim(), 1_000.0)
    } else if let Some(value) = normalized.strip_suffix('m') {
        (value.trim(), 1_000_000.0)
    } else if let Some(value) = normalized.strip_suffix('b') {
        (value.trim(), 1_000_000_000.0)
    } else {
        (normalized.trim(), 1.0)
    };

    let value = numeric.parse::<f64>().ok()?;
    (value.is_finite() && value > 0.0)
        .then(|| clamp_u64_to_u32((value * multiplier).round() as u64))
}

fn official_html_plain_text(body: &str) -> String {
    let meta_content = official_html_meta_content_re()
        .captures_iter(body)
        .filter_map(|capture| capture.name("content"))
        .map(|content| content.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    let without_scripts = official_html_script_re().replace_all(body, " ");
    let without_styles = official_html_style_re().replace_all(without_scripts.as_ref(), " ");
    let without_tags = official_html_tag_re().replace_all(without_styles.as_ref(), " ");
    let combined = format!("{meta_content} {}", without_tags.as_ref());
    let decoded = html_escape::decode_html_entities(combined.as_str());
    official_html_whitespace_re()
        .replace_all(decoded.as_ref(), " ")
        .trim()
        .to_owned()
}

fn extract_official_html_ssr_props_json(body: &str) -> Option<&str> {
    official_html_ssr_props_re()
        .captures(body)
        .and_then(|capture| capture.name("json"))
        .map(|json| json.as_str())
}

fn official_html_ssr_props_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r#"(?s)<script id="ssr-props" type="application/json">(?P<json>.*?)</script>"#)
            .expect("valid ssr props regex")
    })
}

fn official_html_script_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?is)<script\b.*?</script>").expect("valid script regex"))
}

fn official_html_meta_content_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r#"(?is)<meta\b[^>]*\bcontent="(?P<content>[^"]*)"[^>]*>"#)
            .expect("valid meta content regex")
    })
}

fn official_html_style_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?is)<style\b.*?</style>").expect("valid style regex"))
}

fn official_html_tag_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?is)<[^>]+>").expect("valid tag regex"))
}

fn official_html_whitespace_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\s+").expect("valid whitespace regex"))
}

fn official_html_input_context_length_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?i)\binput\s+context\s+length\s*\(isl\)[^0-9]{0,24}(?P<value>\d+(?:,\d{3})*(?:\.\d+)?(?:\s*(?:k|m|b|thousand|million|billion))?)\b",
        )
        .expect("valid input context regex")
    })
}

fn official_html_context_length_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?i)\b(?:maximum\s+context\s+length|context\s+length)\b[^0-9]{0,24}(?:of\s+|up\s+to\s+|is\s+)?(?P<value>\d+(?:,\d{3})*(?:\.\d+)?(?:\s*(?:k|m|b|thousand|million|billion))?)(?:\s*tokens?)?\b",
        )
        .expect("valid context length regex")
    })
}

fn official_html_context_window_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?i)\bcontext\s+window\b[^0-9]{0,32}(?:of\s+up\s+to\s+|up\s+to\s+|of\s+|is\s+)?(?P<value>\d+(?:,\d{3})*(?:\.\d+)?(?:\s*(?:k|m|b|thousand|million|billion))?)(?:\s*tokens?)?\b",
        )
        .expect("valid context window regex")
    })
}

fn official_html_leading_context_size_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?i)\b(?P<value>\d+(?:,\d{3})*(?:\.\d+)?(?:\s*(?:k|m|b|thousand|million|billion))?)\s*(?:-|\s)?tokens?\s+context\s+(?:window|length|size)\b",
        )
        .expect("valid leading context regex")
    })
}

fn official_html_supports_context_size_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?i)\bsupports\s+up\s+to\s+(?:a|an)?\s*(?P<value>\d+(?:,\d{3})*(?:\.\d+)?(?:\s*(?:k|m|b|thousand|million|billion))?)\s+context\s+(?:window|length|size)\b",
        )
        .expect("valid supports context regex")
    })
}
use super::{
    AppError, BTreeMap, CatalogDefinitionSourcePriority, CatalogModelDefinition,
    FetchedModelCatalogDocument, HuggingFaceHubModel, ModelCapabilityFeature, ModelCapabilityPatch,
    ModelCatalogDocument, ModelCatalogRemoteSource, ModelCatalogRemoteSourceKind,
    ModelInputModality, ModelsDevProvider, OfficialHtmlReferencePage, OfficialHtmlSsrPage,
    OfficialHtmlSsrProps, OfficialHtmlTokenLimits, OnceLock, OpenAiCodexModelsDocument, Regex,
    RouterModel, annotate_document_source_priority, canonical_model_catalog_id,
    codex_default_thinking_mode, codex_input_support, codex_speed_modes, codex_thinking_modes,
    features_from_bool_flags, hugging_face_capability_patch, hugging_face_model_aliases,
    hugging_face_model_is_supported, hugging_face_output_modalities, hugging_face_owner_origin,
    hugging_face_release_date, join_all, merge_document_entry, model_capability_patch,
    models_dev_adapter_id, models_dev_assistant_reasoning_field,
    models_dev_assistant_reasoning_interleaved, models_dev_input_support, models_dev_origin,
    models_dev_output_modalities, models_dev_pricing, models_dev_provider_rank,
    models_dev_speed_modes, normalize_optional_string, parse_models_dev_lifecycle,
    router_input_support, router_origin, router_thinking_modes, stream, title_case_tokenized,
};
use crate::model::clamp_u64_to_u32;
