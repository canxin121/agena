pub(super) fn models_dev_adapter_id(
    provider_key: &str,
    provider: &ModelsDevProvider,
) -> Option<String> {
    let normalized = provider
        .id
        .as_deref()
        .unwrap_or(provider_key)
        .trim()
        .to_ascii_lowercase();
    match normalized.as_str() {
        "openai" => Some("openai".to_owned()),
        "anthropic" => Some("anthropic".to_owned()),
        "google" | "gemini" => Some("gemini".to_owned()),
        _ => None,
    }
}

pub(super) fn parse_models_dev_lifecycle(status: Option<&str>) -> Option<ModelLifecycle> {
    match status
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("active") | Some("stable") | Some("ga") => Some(ModelLifecycle::Active),
        Some("preview") => Some(ModelLifecycle::Preview),
        Some("beta") => Some(ModelLifecycle::Beta),
        Some("alpha") => Some(ModelLifecycle::Alpha),
        Some("experimental") => Some(ModelLifecycle::Experimental),
        Some("deprecated") | Some("sunset") => Some(ModelLifecycle::Deprecated),
        _ => None,
    }
}

pub(super) fn models_dev_speed_modes(
    experimental: Option<&ModelsDevExperimental>,
    adapter_id: Option<&str>,
) -> BTreeMap<String, ConfiguredModelSpeedMode> {
    let mut modes = BTreeMap::new();
    let Some(experimental) = experimental else {
        return modes;
    };
    for (name, mode) in &experimental.modes {
        let normalized = name.trim();
        if normalized.is_empty() {
            continue;
        }
        let request_override = mode
            .provider
            .as_ref()
            .map(models_dev_request_override)
            .unwrap_or_default();
        let (default_request_override, adapter_overrides) = if request_override.is_empty() {
            (ModelSpeedModeRequestOverride::default(), BTreeMap::new())
        } else if let Some(adapter_id) = adapter_id {
            (
                ModelSpeedModeRequestOverride::default(),
                BTreeMap::from([(adapter_id.to_owned(), request_override)]),
            )
        } else {
            (request_override, BTreeMap::new())
        };
        modes.insert(
            normalized.to_owned(),
            ConfiguredModelSpeedMode {
                display_name: Some(title_case_tokenized(normalized)),
                description: None,
                request_override: default_request_override,
                adapter_overrides,
                disabled: false,
            },
        );
    }
    modes
}

pub(super) fn models_dev_assistant_reasoning_field(
    interleaved: Option<&ModelsDevInterleaved>,
) -> Option<String> {
    match interleaved {
        Some(ModelsDevInterleaved::Field(field)) => normalize_optional_string(field.field.clone())
            .and_then(|value| match value.as_str() {
                "reasoning_content" | "reasoning_details" => Some(value),
                _ => None,
            }),
        Some(ModelsDevInterleaved::Enabled(_)) => None,
        _ => None,
    }
}

pub(super) fn models_dev_assistant_reasoning_interleaved(
    interleaved: Option<&ModelsDevInterleaved>,
) -> Option<bool> {
    match interleaved {
        Some(ModelsDevInterleaved::Enabled(enabled)) => Some(*enabled),
        Some(ModelsDevInterleaved::Field(_)) => Some(true),
        None => None,
    }
}

pub(super) fn models_dev_request_override(
    provider: &ModelsDevModeProvider,
) -> ModelSpeedModeRequestOverride {
    let headers = provider.headers.clone().unwrap_or_default();
    let body_patch = provider.body.clone().unwrap_or_default();
    ModelSpeedModeRequestOverride {
        headers,
        body_patch,
    }
}

pub(super) fn router_origin(section: &str, model: &RouterModel) -> Option<String> {
    if let Some(owned_by) = model.owned_by.as_deref() {
        let owned = owned_by.trim().to_ascii_lowercase();
        match owned.as_str() {
            "google" => return Some("Google".to_owned()),
            "anthropic" => return Some("Anthropic".to_owned()),
            "openai" => return Some("OpenAI".to_owned()),
            "xai" => return Some("xAI".to_owned()),
            "moonshot" | "moonshotai" | "kimi" => return Some("Moonshot AI".to_owned()),
            _ => {}
        }
    }

    match section {
        "claude" => Some("Anthropic".to_owned()),
        "gemini" | "gemini-cli" | "aistudio" | "vertex" => Some("Google".to_owned()),
        "xai" => Some("xAI".to_owned()),
        "kimi" => Some("Moonshot AI".to_owned()),
        "codex-free" | "codex-team" | "codex-plus" | "codex-pro" => Some("OpenAI".to_owned()),
        "antigravity" => Some("Antigravity".to_owned()),
        _ => None,
    }
}

pub(super) fn router_input_support(
    model: &RouterModel,
) -> (Vec<ModelInputModality>, Vec<ModelInputModality>) {
    let mut supported = Vec::new();
    let unsupported = Vec::new();
    let Some(modalities) = model.supported_input_modalities.as_ref() else {
        return (supported, unsupported);
    };
    for modality in modalities {
        if let Some(mapped) = map_modality_name(modality)
            && mapped != ModelInputModality::Text
            && !supported.contains(&mapped)
        {
            supported.push(mapped);
        }
    }
    (supported, unsupported)
}

pub(super) fn router_thinking_modes(
    thinking: Option<&RouterThinking>,
) -> BTreeMap<String, ConfiguredModelThinkingMode> {
    let mut modes = BTreeMap::new();
    let Some(thinking) = thinking else {
        return modes;
    };

    if thinking.zero_allowed.unwrap_or(false) {
        modes.insert(
            "no-thinking".to_owned(),
            ConfiguredModelThinkingMode {
                display_name: Some("Off".to_owned()),
                description: None,
                thinking: Some(ThinkingRequest::Disabled),
                request_override: Default::default(),
                adapter_overrides: BTreeMap::new(),
                disabled: false,
            },
        );
    }

    for level in &thinking.levels {
        if let Some(effort) = effort_for_variant_name(level) {
            let normalized = level.trim().to_ascii_lowercase();
            let variant_name = if normalized == "none" {
                "no-thinking".to_owned()
            } else {
                format!("thinking-{normalized}")
            };
            modes
                .entry(variant_name)
                .or_insert_with(|| ConfiguredModelThinkingMode {
                    display_name: Some(format!("Think {}", title_case_tokenized(level))),
                    description: None,
                    thinking: Some(ThinkingRequest::Effort { effort }),
                    request_override: Default::default(),
                    adapter_overrides: BTreeMap::new(),
                    disabled: false,
                });
        }
    }

    if thinking.levels.is_empty() {
        if let Some(high_budget) = router_high_budget(thinking) {
            modes.insert(
                "thinking-high".to_owned(),
                ConfiguredModelThinkingMode {
                    display_name: Some("Think High".to_owned()),
                    description: None,
                    thinking: Some(ThinkingRequest::Budget {
                        budget_tokens: high_budget,
                    }),
                    request_override: Default::default(),
                    adapter_overrides: BTreeMap::new(),
                    disabled: false,
                },
            );
        }

        if let Some(max_budget) = thinking.max.map(clamp_u64_to_u32) {
            modes.insert(
                "thinking-max".to_owned(),
                ConfiguredModelThinkingMode {
                    display_name: Some("Think Max".to_owned()),
                    description: None,
                    thinking: Some(ThinkingRequest::Budget {
                        budget_tokens: max_budget,
                    }),
                    request_override: Default::default(),
                    adapter_overrides: BTreeMap::new(),
                    disabled: false,
                },
            );
        }
    }

    modes
}

pub(super) fn codex_thinking_modes(
    supported_reasoning_levels: Option<&[OpenAiCodexReasoningLevel]>,
) -> BTreeMap<String, ConfiguredModelThinkingMode> {
    let mut modes = BTreeMap::new();
    let Some(supported_reasoning_levels) = supported_reasoning_levels else {
        return modes;
    };

    for level in supported_reasoning_levels {
        let Some(effort) = effort_for_variant_name(level.effort.as_str()) else {
            continue;
        };
        let effort_name = effort.as_ref();
        modes.insert(
            format!("thinking-{effort_name}"),
            ConfiguredModelThinkingMode {
                display_name: Some(format!("Think {}", title_case_tokenized(effort_name))),
                description: normalize_optional_string(level.description.clone()),
                thinking: Some(ThinkingRequest::Effort { effort }),
                request_override: Default::default(),
                adapter_overrides: BTreeMap::new(),
                disabled: false,
            },
        );
    }

    modes
}

pub(super) fn codex_speed_modes(
    service_tiers: Option<&[OpenAiCodexServiceTier]>,
    additional_speed_tiers: Option<&[String]>,
) -> BTreeMap<String, ConfiguredModelSpeedMode> {
    let mut modes = BTreeMap::new();
    let Some(service_tiers) = service_tiers else {
        return modes;
    };

    let aliases = additional_speed_tiers.unwrap_or(&[]);
    for (index, tier) in service_tiers.iter().enumerate() {
        let Some(tier_id) = normalize_optional_string(tier.id.clone()) else {
            continue;
        };
        let alias = aliases
            .get(index)
            .and_then(|value| normalize_optional_string(Some(value.clone())));
        let name = alias.unwrap_or_else(|| {
            normalize_optional_string(tier.name.clone())
                .map(|value| value.to_ascii_lowercase().replace(' ', "-"))
                .unwrap_or_else(|| tier_id.clone())
        });
        if name.is_empty() {
            continue;
        }

        modes.insert(
            name,
            ConfiguredModelSpeedMode {
                display_name: normalize_optional_string(tier.name.clone())
                    .or_else(|| Some(title_case_tokenized(tier_id.as_str()))),
                description: normalize_optional_string(tier.description.clone()),
                request_override: ModelSpeedModeRequestOverride {
                    headers: BTreeMap::new(),
                    body_patch: BTreeMap::from([(
                        "service_tier".to_owned(),
                        serde_json::Value::String(tier_id),
                    )]),
                },
                adapter_overrides: BTreeMap::new(),
                disabled: false,
            },
        );
    }

    modes
}

pub(super) fn codex_default_thinking_mode(default_reasoning_level: Option<&str>) -> Option<String> {
    let effort = default_reasoning_level.and_then(effort_for_variant_name)?;
    Some(format!("thinking-{effort}"))
}

pub(super) fn router_high_budget(thinking: &RouterThinking) -> Option<u32> {
    let max_budget = thinking.max?;
    let min_budget = thinking.min.unwrap_or(0);
    let target = max_budget.min(16_384).max(min_budget);
    Some(clamp_u64_to_u32(target))
}

pub(super) fn codex_input_support(input_modalities: Option<&[String]>) -> Vec<ModelInputModality> {
    let mut supported = Vec::new();
    let Some(input_modalities) = input_modalities else {
        return supported;
    };
    for modality in input_modalities {
        if let Some(mapped) = map_modality_name(modality)
            && mapped != ModelInputModality::Text
            && !supported.contains(&mapped)
        {
            supported.push(mapped);
        }
    }
    supported
}

pub(super) fn models_dev_input_support(
    modalities: Option<&ModelsDevModalities>,
    attachment: Option<bool>,
) -> (Vec<ModelInputModality>, Vec<ModelInputModality>) {
    let mut supported = Vec::new();
    let unsupported = Vec::new();
    if let Some(modalities) = modalities {
        for modality in &modalities.input {
            if let Some(mapped) = map_modality_name(modality)
                && mapped != ModelInputModality::Text
                && !supported.contains(&mapped)
            {
                supported.push(mapped);
            }
        }
    }
    if attachment == Some(true) && !supported.contains(&ModelInputModality::File) {
        supported.push(ModelInputModality::File);
    }
    (supported, unsupported)
}

pub(super) fn models_dev_output_modalities(
    modalities: Option<&ModelsDevModalities>,
) -> Vec<String> {
    let mut output = Vec::new();
    let Some(modalities) = modalities else {
        return output;
    };
    for modality in &modalities.output {
        let normalized = normalize_modality_label(modality);
        if !normalized.is_empty() && !output.contains(&normalized) {
            output.push(normalized);
        }
    }
    output
}

pub(super) fn models_dev_pricing(cost: Option<&ModelsDevCost>) -> Option<ModelPricing> {
    let cost = cost?;
    let mut tiers = cost
        .tiers
        .iter()
        .filter_map(models_dev_pricing_tier)
        .collect::<Vec<_>>();

    let context_over_200k_tier = cost
        .context_over_200k
        .as_ref()
        .and_then(|context_over_200k| {
            let tier = ModelPricingTier {
                tier_type: Some("context".to_owned()),
                size_tokens: Some(200_000),
                input_usd_per_million_tokens: pricing_value(context_over_200k.input.as_ref()),
                output_usd_per_million_tokens: pricing_value(context_over_200k.output.as_ref()),
                cache_read_usd_per_million_tokens: pricing_value(
                    context_over_200k.cache_read.as_ref(),
                ),
                cache_write_usd_per_million_tokens: pricing_value(
                    context_over_200k.cache_write.as_ref(),
                ),
            };
            (!tier.is_empty()).then_some(tier)
        });
    if let Some(tier) = context_over_200k_tier
        && !tiers.iter().any(|existing| {
            existing.tier_type.as_deref() == Some("context")
                && existing.size_tokens == Some(200_000)
        })
    {
        tiers.push(tier);
    }

    let pricing = ModelPricing {
        input_usd_per_million_tokens: pricing_value(cost.input.as_ref()),
        output_usd_per_million_tokens: pricing_value(cost.output.as_ref()),
        cache_read_usd_per_million_tokens: pricing_value(cost.cache_read.as_ref()),
        cache_write_usd_per_million_tokens: pricing_value(cost.cache_write.as_ref()),
        tiers,
    };
    (!pricing.is_empty()).then_some(pricing)
}

pub(super) fn models_dev_pricing_tier(tier: &ModelsDevCostTier) -> Option<ModelPricingTier> {
    let tier = ModelPricingTier {
        tier_type: normalize_optional_string(tier.tier_type.clone()),
        size_tokens: tier.size.map(clamp_u64_to_u32),
        input_usd_per_million_tokens: pricing_value(tier.input.as_ref()),
        output_usd_per_million_tokens: pricing_value(tier.output.as_ref()),
        cache_read_usd_per_million_tokens: pricing_value(tier.cache_read.as_ref()),
        cache_write_usd_per_million_tokens: pricing_value(tier.cache_write.as_ref()),
    };
    (!tier.is_empty()).then_some(tier)
}

pub(super) fn pricing_value(value: Option<&serde_json::Value>) -> Option<String> {
    match value {
        Some(serde_json::Value::Number(value)) => Some(value.to_string()),
        Some(serde_json::Value::String(value)) => normalize_optional_string(Some(value.clone())),
        _ => None,
    }
}

pub(super) fn features_from_bool_flags(
    flags: &[(ModelCapabilityFeature, Option<bool>)],
) -> (Vec<ModelCapabilityFeature>, Vec<ModelCapabilityFeature>) {
    let mut supported = Vec::new();
    let mut unsupported = Vec::new();
    for (feature, value) in flags {
        match value {
            Some(true) => supported.push(*feature),
            Some(false) => unsupported.push(*feature),
            None => {}
        }
    }
    (supported, unsupported)
}

pub(super) fn model_capability_patch(
    (supported_inputs, unsupported_inputs): (Vec<ModelInputModality>, Vec<ModelInputModality>),
    (supported_features, unsupported_features): (
        Vec<ModelCapabilityFeature>,
        Vec<ModelCapabilityFeature>,
    ),
) -> ModelCapabilityPatch {
    ModelCapabilityPatch {
        input: CapabilitySelectionPatch::optional_from_supported_unsupported(
            supported_inputs,
            unsupported_inputs,
        ),
        features: CapabilitySelectionPatch::optional_from_supported_unsupported(
            supported_features,
            unsupported_features,
        ),
        ..ModelCapabilityPatch::default()
    }
}

pub(super) fn hugging_face_capability_patch(
    pipeline_tag: Option<&str>,
    repo_name: &str,
    tags: Option<&[String]>,
) -> ModelCapabilityPatch {
    let normalized_repo = repo_name.trim().to_ascii_lowercase();
    let normalized_pipeline = pipeline_tag
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    let mut supported_inputs = Vec::new();
    let tag_set = normalized_tag_set(tags);

    if matches!(
        normalized_pipeline.as_str(),
        "image-text-to-text"
            | "image-to-text"
            | "visual-question-answering"
            | "image-classification"
            | "object-detection"
            | "image-segmentation"
            | "document-question-answering"
            | "any-to-any"
    ) || normalized_repo.contains("vision")
        || normalized_repo.contains("kosmos")
        || tag_set.iter().any(|tag| {
            matches!(
                tag.as_str(),
                "vision"
                    | "image-text-to-text"
                    | "image-to-text"
                    | "visual-question-answering"
                    | "image-classification"
                    | "object-detection"
                    | "image-segmentation"
                    | "document-question-answering"
            )
        })
    {
        supported_inputs.push(ModelInputModality::Image);
    }
    if matches!(
        normalized_pipeline.as_str(),
        "automatic-speech-recognition"
            | "audio-to-audio"
            | "audio-classification"
            | "voice-activity-detection"
    ) || tag_set.iter().any(|tag| {
        matches!(
            tag.as_str(),
            "audio" | "audio-to-audio" | "automatic-speech-recognition" | "audio-classification"
        )
    }) {
        supported_inputs.push(ModelInputModality::Audio);
    }
    if matches!(
        normalized_pipeline.as_str(),
        "video-text-to-text" | "video-classification" | "video-text-retrieval"
    ) || tag_set.iter().any(|tag| {
        matches!(
            tag.as_str(),
            "video" | "video-text-to-text" | "video-classification"
        )
    }) {
        supported_inputs.push(ModelInputModality::Video);
    }
    if matches!(normalized_pipeline.as_str(), "document-question-answering")
        || tag_set
            .iter()
            .any(|tag| matches!(tag.as_str(), "document-question-answering" | "document"))
    {
        supported_inputs.push(ModelInputModality::Document);
    }
    supported_inputs.dedup();

    model_capability_patch((supported_inputs, Vec::new()), (Vec::new(), Vec::new()))
}

pub(super) fn hugging_face_output_modalities(
    pipeline_tag: Option<&str>,
    repo_name: &str,
    tags: Option<&[String]>,
) -> Vec<String> {
    let normalized_repo = repo_name.trim().to_ascii_lowercase();
    let normalized_pipeline = pipeline_tag
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    let tag_set = normalized_tag_set(tags);

    if normalized_pipeline == "any-to-any" {
        return vec!["text".to_owned(), "image".to_owned()];
    }
    if matches!(
        normalized_pipeline.as_str(),
        "image-text-to-text"
            | "image-to-text"
            | "visual-question-answering"
            | "text-generation"
            | "text2text-generation"
            | "translation"
            | "summarization"
            | "automatic-speech-recognition"
            | "document-question-answering"
            | "video-text-to-text"
    ) {
        return vec!["text".to_owned()];
    }
    if matches!(
        normalized_pipeline.as_str(),
        "text-to-image" | "image-to-image"
    ) {
        return vec!["image".to_owned()];
    }
    if matches!(
        normalized_pipeline.as_str(),
        "text-to-audio" | "text-to-speech" | "audio-to-audio"
    ) {
        return vec!["audio".to_owned()];
    }
    if matches!(
        normalized_pipeline.as_str(),
        "text-to-video" | "image-to-video"
    ) {
        return vec!["video".to_owned()];
    }
    if normalized_repo.contains("vision") || normalized_repo.contains("kosmos") {
        return vec!["text".to_owned()];
    }
    if tag_set.iter().any(|tag| tag == "vision") {
        return vec!["text".to_owned()];
    }
    Vec::new()
}

pub(super) fn normalized_tag_set(tags: Option<&[String]>) -> Vec<String> {
    let mut values = Vec::new();
    for tag in tags.unwrap_or(&[]) {
        let normalized = tag.trim().to_ascii_lowercase();
        if !normalized.is_empty() && !values.contains(&normalized) {
            values.push(normalized);
        }
    }
    values
}

pub(super) fn hugging_face_owner_origin(owner: &str) -> Option<&'static str> {
    match owner.trim().to_ascii_lowercase().as_str() {
        "google" => Some("Google"),
        "meta-llama" | "codellama" => Some("Meta"),
        "mistralai" => Some("Mistral AI"),
        "ibm-granite" => Some("IBM"),
        "microsoft" => Some("Microsoft"),
        "bigcode" => Some("BigCode"),
        "snowflake" => Some("Snowflake"),
        "adept" => Some("Adept"),
        "aisingapore" => Some("AI Singapore"),
        "stockmark" => Some("Stockmark"),
        "zyphra" => Some("Zyphra"),
        "deepseek-ai" => Some("DeepSeek"),
        "ai21labs" => Some("AI21 Labs"),
        "nvidia" => Some("NVIDIA"),
        "writer" => Some("Writer"),
        _ => None,
    }
}

pub(super) fn hugging_face_release_date(created_at: Option<&str>) -> Option<String> {
    let created_at = created_at?.trim();
    let date = created_at.split('T').next().unwrap_or_default().trim();
    (!date.is_empty()).then(|| date.to_owned())
}

pub(super) fn hugging_face_model_is_supported(repo_id: &str) -> bool {
    let Some((owner, repo_name)) = repo_id.split_once('/') else {
        return false;
    };
    if hugging_face_owner_origin(owner).is_none() {
        return false;
    }

    let normalized = repo_name.trim().to_ascii_lowercase();
    if normalized == "nvidia-nemotron-3-nano-30b-a3b-bf16" {
        return true;
    }
    !matches!(
        normalized.as_str(),
        value
            if value.ends_with("-gguf")
                || value.ends_with("-onnx")
                || value.contains("-onnx-")
                || value.ends_with("-tflite")
                || value.ends_with("-keras")
                || value.ends_with("-pytorch")
                || value.ends_with("-ov")
                || value.ends_with("-accelerator")
                || value.ends_with("-assistant")
                || value.ends_with("-fp8")
                || value.ends_with("-bf16")
                || value.ends_with("-nvfp4")
                || value.ends_with("-awq")
                || value.ends_with("-gptq")
                || value.ends_with("-exl2")
                || value.ends_with("-flax")
                || value.ends_with("-vllm")
                || value.ends_with("-dummy-weights")
                || value.contains("tiny-random")
                || value.contains("-mlx-")
                || value.contains("-sfp-cpp")
                || value.contains("-reward")
                || value.contains("-base")
    )
}

pub(super) fn hugging_face_model_aliases(repo_name: &str) -> Vec<String> {
    let normalized = repo_name.trim().to_ascii_lowercase();
    let mut aliases = vec![repo_name.trim().to_owned()];

    if let Some(stripped) = normalized.strip_suffix("-hf") {
        aliases.push(stripped.to_owned());
    }
    if normalized.starts_with("codegemma-") && normalized.ends_with("-it") {
        aliases.push(normalized.trim_end_matches("-it").to_owned());
    }
    if normalized.starts_with("llama-2-") {
        aliases.push(normalized.replacen("llama-2-", "llama2-", 1));
    }
    if normalized == "codestral-22b-v0.1" {
        aliases.push("codestral-22b-instruct-v0.1".to_owned());
    }
    if normalized == "mistral-large-instruct-2407" {
        aliases.push("mistral-large-2-instruct".to_owned());
    }
    if normalized == "kosmos-2-patch14-224" {
        aliases.push("kosmos-2".to_owned());
    }
    if normalized == "snowflake-arctic-embed-l" {
        aliases.push("arctic-embed-l".to_owned());
    }
    if normalized == "sea-lion-v1-7b-it" {
        aliases.push("sea-lion-7b-instruct".to_owned());
    }
    if normalized == "granite-34b-code-instruct-8k" {
        aliases.push("granite-34b-code-instruct".to_owned());
    }
    if matches!(
        normalized.as_str(),
        "granite-8b-code-instruct-4k" | "granite-8b-code-instruct-128k"
    ) {
        aliases.push("granite-8b-code-instruct".to_owned());
    }
    if normalized == "palmyra-creative" {
        aliases.push("palmyra-creative-122b".to_owned());
    }
    if normalized == "mistral-nemo-minitron-8b-instruct" {
        aliases.push("mistral-nemo-minitron-8b-8k-instruct".to_owned());
    }
    if normalized == "nvidia-nemotron-3-nano-30b-a3b-bf16" {
        aliases.push("nemotron-nano-3-30b-a3b".to_owned());
    }
    if normalized.starts_with("nvidia-nemotron-parse-") {
        aliases.push("nemotron-parse".to_owned());
    }
    if let Some((size, version)) = normalized
        .strip_prefix("ai21-jamba-")
        .and_then(|value| value.rsplit_once('-'))
    {
        aliases.push(format!("jamba-{size}-{version}"));
        aliases.push(format!("jamba-{version}-{size}-instruct"));
    }

    aliases.sort();
    aliases.dedup();
    aliases
}

pub(super) fn map_modality_name(value: &str) -> Option<ModelInputModality> {
    match value.trim().to_ascii_lowercase().as_str() {
        "text" => Some(ModelInputModality::Text),
        "image" => Some(ModelInputModality::Image),
        "audio" => Some(ModelInputModality::Audio),
        "video" => Some(ModelInputModality::Video),
        "document" | "pdf" => Some(ModelInputModality::Document),
        "file" => Some(ModelInputModality::File),
        _ => None,
    }
}

pub(super) fn normalize_modality_label(value: &str) -> String {
    match value.trim().to_ascii_lowercase().as_str() {
        "pdf" => "document".to_owned(),
        other => other.to_owned(),
    }
}

pub(super) fn effort_for_variant_name(name: &str) -> Option<ReasoningEffort> {
    match name.trim().to_ascii_lowercase().as_str() {
        "minimal" => Some(ReasoningEffort::Minimal),
        "low" => Some(ReasoningEffort::Low),
        "medium" => Some(ReasoningEffort::Medium),
        "high" => Some(ReasoningEffort::High),
        "xhigh" => Some(ReasoningEffort::Xhigh),
        "max" => Some(ReasoningEffort::Max),
        _ => None,
    }
}

pub fn enrich_catalog_document_thinking_modes(document: &mut ModelCatalogDocument) {
    for (model_id, definition) in &mut document.models {
        let inferred = inferred_thinking_modes(model_id.as_str(), definition);
        for (name, mode) in inferred {
            definition.thinking_modes.entry(name).or_insert(mode);
        }
    }
}

pub(super) fn inferred_thinking_modes(
    model_id: &str,
    definition: &CatalogModelDefinition,
) -> BTreeMap<String, ConfiguredModelThinkingMode> {
    let mut modes = BTreeMap::new();
    if !matches!(
        definition
            .capabilities
            .feature_support(ModelCapabilityFeature::Reasoning),
        Some(CapabilitySupport::Supported)
    ) {
        return modes;
    }

    let normalized = model_id.trim().to_ascii_lowercase();
    if normalized.contains("gpt-5")
        || normalized.starts_with("o1")
        || normalized.starts_with("o3")
        || normalized.starts_with("o4")
    {
        for effort in openai_reasoning_efforts(normalized.as_str()) {
            insert_effort_mode(&mut modes, effort);
        }
        return modes;
    }

    if normalized.contains("gemini-3") {
        insert_effort_mode(&mut modes, ReasoningEffort::Low);
        insert_effort_mode(&mut modes, ReasoningEffort::High);
        return modes;
    }

    if normalized.contains("gemini-2.5") {
        insert_effort_mode(&mut modes, ReasoningEffort::High);
        insert_effort_mode(&mut modes, ReasoningEffort::Max);
        return modes;
    }

    if normalized.contains("claude") && definition.thinking_modes.is_empty() {
        insert_effort_mode(&mut modes, ReasoningEffort::High);
        insert_effort_mode(&mut modes, ReasoningEffort::Max);
    }

    modes
}

pub(super) fn openai_reasoning_efforts(model_id: &str) -> Vec<ReasoningEffort> {
    let mut efforts = Vec::new();
    if model_id.contains("gpt-5") {
        efforts.push(ReasoningEffort::Minimal);
    }
    efforts.extend([
        ReasoningEffort::Low,
        ReasoningEffort::Medium,
        ReasoningEffort::High,
    ]);
    if model_id.contains("gpt-5") {
        efforts.push(ReasoningEffort::Xhigh);
    }
    efforts
}

pub(super) fn insert_effort_mode(
    modes: &mut BTreeMap<String, ConfiguredModelThinkingMode>,
    effort: ReasoningEffort,
) {
    let effort_name = effort.as_ref();
    modes
        .entry(format!("thinking-{effort_name}"))
        .or_insert_with(|| ConfiguredModelThinkingMode {
            display_name: Some(format!("Think {}", title_case_tokenized(effort_name))),
            description: None,
            thinking: Some(ThinkingRequest::Effort { effort }),
            request_override: Default::default(),
            adapter_overrides: BTreeMap::new(),
            disabled: false,
        });
}

pub(super) fn title_case_tokenized(value: &str) -> String {
    value
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|segment| !segment.is_empty())
        .map(|segment| {
            let mut chars = segment.chars();
            let Some(first) = chars.next() else {
                return String::new();
            };
            format!(
                "{}{}",
                first.to_ascii_uppercase(),
                chars.as_str().to_ascii_lowercase()
            )
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub(super) fn normalize_optional_string(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_owned())
    })
}

pub(super) fn clamp_u64_to_u32(value: u64) -> u32 {
    value.min(u32::MAX as u64) as u32
}
use super::{
    BTreeMap, CapabilitySelectionPatch, CapabilitySupport, CatalogModelDefinition,
    ConfiguredModelSpeedMode, ConfiguredModelThinkingMode, ModelCapabilityFeature,
    ModelCapabilityPatch, ModelCatalogDocument, ModelInputModality, ModelLifecycle, ModelPricing,
    ModelPricingTier, ModelSpeedModeRequestOverride, ModelsDevCost, ModelsDevCostTier,
    ModelsDevExperimental, ModelsDevInterleaved, ModelsDevModalities, ModelsDevModeProvider,
    ModelsDevProvider, OpenAiCodexReasoningLevel, OpenAiCodexServiceTier, ReasoningEffort,
    RouterModel, RouterThinking, ThinkingRequest,
};
