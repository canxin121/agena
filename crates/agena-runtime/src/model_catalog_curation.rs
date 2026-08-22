use std::{collections::BTreeMap, sync::LazyLock};

use regex::Regex;
use serde_json::Value;

use agena_provider::{
    CatalogDefinitionSourcePriority, CatalogModelDefinition, ModelCatalogDocument,
    normalized_catalog_model_id,
};

/// Failure while normalizing or merging a provider catalog document.
#[derive(Debug, thiserror::Error)]
pub enum ModelCatalogCurationError {
    #[error(transparent)]
    Serde(#[from] serde_json::Error),
}

const BANNED_SOURCE_PREFIXES: &[&str] = &[
    "openai.",
    "azure-",
    "google.",
    "cohere-",
    "ai21-",
    "amazon.",
    "anthropic.",
    "duo-chat-",
    "study_gpt-",
    "meta.",
    "mistral.",
    "moonshot.",
    "moonshotai.",
    "qwen.",
    "writer.",
    "zai.",
    "nvidia.",
    "minimax.",
];

const OFFICIAL_ROOTS: &[&str] = &[
    "agnes",
    "aion",
    "all",
    "allam",
    "aura",
    "autoglm",
    "bge",
    "brave",
    "c4ai",
    "chatgpt",
    "claude",
    "codegemma",
    "codestral",
    "codex",
    "codellama",
    "command",
    "deepseek",
    "deplot",
    "devstral",
    "doubao",
    "e5",
    "elevenlabs",
    "embed",
    "ernie",
    "exa",
    "flux",
    "gemini",
    "gemma",
    "glm",
    "gpt",
    "granite",
    "grok",
    "gte",
    "hunyuan",
    "hy3",
    "ideogram",
    "imagen",
    "inflection",
    "internvl",
    "jais",
    "jamba",
    "kimi",
    "kling",
    "learnlm",
    "lfm",
    "ling",
    "llama",
    "llama3",
    "luma",
    "lyria",
    "magistral",
    "manta",
    "mercury",
    "minimax",
    "mimo",
    "ministral",
    "mistral",
    "mixtral",
    "moonshot",
    "morph",
    "nova",
    "o1",
    "o3",
    "o4",
    "olmo",
    "open",
    "orpheus",
    "palmyra",
    "phi",
    "pixtral",
    "qianfan",
    "qvq",
    "qwen",
    "qwen2",
    "qwen3",
    "qwq",
    "recraft",
    "recurrentgemma",
    "rerank",
    "ring",
    "riverflow",
    "runway",
    "seed",
    "solar",
    "sonar",
    "step",
    "text",
    "trinity",
    "v0",
    "veo",
    "venice",
    "voxtral",
    "voyage",
    "whisper",
    "yi",
];

static SOURCE_PREFIX_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?:us|eu|au|jp|global|apac)\.").unwrap());

static BLOCKED_TOKENS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
        r"(^|[-:.])(free|optimized)(?:$|[-:.])",
        r"(^|[-:.])tee(?:$|[-:.])",
        r"(^|[-:.])fp\d+(?:$|[-:.])",
        r"(^|[-:.])abliterated(?:$|[-:.])",
        r"(^|[-:.])uncensored(?:$|[-:.])",
        r"(^|[-:.])derestricted(?:$|[-:.])",
        r"(^|[-:.])chimera(?:$|[-:.])",
        r"(^|[-:.])iceblink(?:$|[-:.])",
        r"(^|[-:.])reextract(?:$|[-:.])",
        r"(^|[-:.])steam(?:$|[-:.])",
        r"(^|[-:.])omega(?:$|[-:.])",
        r"(^|[-:.])story(?:$|[-:.])",
        r"(^|[-:.])rp(?:$|[-:.])",
        r"(^|[-:.])rpmax(?:$|[-:.])",
        r"(^|[-:.])slop(?:$|[-:.])",
        r"(^|[-:.])slerp(?:$|[-:.])",
        r"(^|[-:.])malevolence(?:$|[-:.])",
        r"(^|[-:.])safeword(?:$|[-:.])",
        r"(^|[-:.])euryale(?:$|[-:.])",
        r"(^|[-:.])hanami(?:$|[-:.])",
        r"(^|[-:.])lumimaid(?:$|[-:.])",
        r"(^|[-:.])anubis(?:$|[-:.])",
        r"(^|[-:.])cydonia(?:$|[-:.])",
        r"(^|[-:.])forgotten(?:$|[-:.])",
        r"(^|[-:.])abomination(?:$|[-:.])",
        r"(^|[-:.])magnum(?:$|[-:.])",
        r"(^|[-:.])magidonia(?:$|[-:.])",
        r"(^|[-:.])laguna(?:$|[-:.])",
        r"(^|[-:.])dolphin(?:$|[-:.])",
        r"(^|[-:.])longcat(?:$|[-:.])",
        r"(^|[-:.])arliai(?:$|[-:.])",
        r"(^|[-:.])cheaper(?:$|[-:.])",
        r"(^|[-:.])raw(?:$|[-:.])",
        r"(^|[-:.])cs(?:$|[-:.])",
        r"(^|[-:.])exacto(?:$|[-:.])",
        r"(^|[-:.])high-throughput(?:$|[-:.])",
        r"(^|[-:.])tput(?:$|[-:.])",
        r"(^|[-:.])int4(?:$|[-:.])",
        r"(^|[-:.])mixed-ar(?:$|[-:.])",
        r"(^|[-:.])captioner(?:$|[-:.])",
        r"(^|[-:.])original(?:$|[-:.])",
        r"(^|[-:.])sambanova(?:$|[-:.])",
        r"(^|[-:.])terminus(?:$|[-:.])",
        r"(^|[-:.])speciale(?:$|[-:.])",
        r"(^|[-:.])nex(?:$|[-:.])",
        r"(^|[-:.])eva(?:$|[-:.])",
    ]
    .into_iter()
    .map(|pattern| Regex::new(pattern).unwrap())
    .collect()
});

static LLAMA_PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
        r"^llama-2-(7b|13b|70b)$",
        r"^llama-2-(7b|13b|70b)-chat$",
        r"^llama2-(7b|13b|70b)$",
        r"^llama2-(7b|13b|70b)-chat$",
        r"^llama-3-(8b|70b)-instruct$",
        r"^llama-3\.1-nemoguard-8b-(content-safety|topic-control)$",
        r"^llama-3\.1-nemotron-[a-z0-9.-]+$",
        r"^llama-3\.1-(8b|70b|405b)-instruct$",
        r"^llama-3\.2-(nemoretriever|nv-embedqa)-[a-z0-9.-]+$",
        r"^llama-3\.2-(1b|3b)-instruct$",
        r"^llama-3\.2-(11b|90b)-vision-instruct$",
        r"^llama-3\.3-nemotron-super-49b-v1(\.5)?$",
        r"^llama-3\.3-70b-instruct$",
        r"^llama-4-(maverick|scout)$",
        r"^llama-4-(maverick|scout)-17b(-128e|-16e)?-instruct$",
        r"^llama-guard-",
        r"^llama-nemotron-(embed|embed-vl|rerank|rerank-vl)-[a-z0-9.-]+$",
    ]
    .into_iter()
    .map(|pattern| Regex::new(pattern).unwrap())
    .collect()
});

static GEMMA_PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
        r"^gemma-(2b|7b)$",
        r"^gemma-2-(2b|9b|27b)-it$",
        r"^gemma-3$",
        r"^gemma-3-(1b|4b|12b|27b)-it$",
        r"^gemma-3n-(e2b|e4b)-it$",
        r"^gemma-4-(26b-a4b|31b)-it$",
    ]
    .into_iter()
    .map(|pattern| Regex::new(pattern).unwrap())
    .collect()
});

static CODEGEMMA_PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [r"^codegemma-(1\.1-7b|2b|7b|7b-it)$"]
        .into_iter()
        .map(|pattern| Regex::new(pattern).unwrap())
        .collect()
});

static LLAMA3_CANONICAL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^llama3((\.1-8b|\.3-70b-instruct)|-chatqa-(1\.5|2)-(8b|70b))$").unwrap()
});

static DISPLAY_ORIGIN_RULES: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
    [
        (r"\bopenai\b", "OpenAI"),
        (r"\banthropic\b", "Anthropic"),
        (r"\bgoogle\b", "Google"),
        (r"\bamazon\b", "Amazon"),
        (r"\bdeepgram\b", "Deepgram"),
        (r"\bdeepseek\b", "DeepSeek"),
        (r"\bcohere\b", "Cohere"),
        (r"\bmistral\b", "Mistral AI"),
        (r"\bmoonshot\b", "Moonshot AI"),
        (r"\bz(?:\.?ai|hipu)\b", "Zhipu AI"),
        (r"\bbaidu\b", "Baidu"),
        (r"\btencent\b", "Tencent"),
        (r"\bwriter\b", "Writer"),
        (r"\bperplexity\b", "Perplexity"),
        (r"\bblack forest\b", "Black Forest Labs"),
        (r"\bvoyage\b", "Voyage AI"),
        (r"\bliquid ?ai\b", "Liquid AI"),
        (r"\bai21\b", "AI21 Labs"),
        (r"\bxiaomi\b", "Xiaomi"),
        (r"\bupstage\b", "Upstage"),
        (r"\bsdaia\b", "SDAIA"),
        (r"\bg42\b", "G42"),
        (r"\binclusionai\b", "Inclusion AI"),
        (r"\bibm\b", "IBM"),
        (r"\ballenai\b", "Allen AI"),
        (r"\bdeepgram\b", "Deepgram"),
    ]
    .into_iter()
    .map(|(pattern, origin)| (Regex::new(pattern).unwrap(), origin))
    .collect()
});

const ROOT_ORIGINS: &[(&str, &str)] = &[
    ("aion", "AionLabs"),
    ("all", "Sentence Transformers"),
    ("allam", "SDAIA"),
    ("aura", "Deepgram"),
    ("autoglm", "Zhipu AI"),
    ("bge", "BAAI"),
    ("brave", "Brave"),
    ("c4ai", "Cohere"),
    ("chatgpt", "OpenAI"),
    ("claude", "Anthropic"),
    ("codegemma", "Google"),
    ("codestral", "Mistral AI"),
    ("codex", "OpenAI"),
    ("codellama", "Meta"),
    ("command", "Cohere"),
    ("deepseek", "DeepSeek"),
    ("deplot", "Google"),
    ("devstral", "Mistral AI"),
    ("doubao", "ByteDance"),
    ("e5", "Microsoft"),
    ("elevenlabs", "ElevenLabs"),
    ("embed", "Cohere"),
    ("ernie", "Baidu"),
    ("exa", "Exa"),
    ("flux", "Black Forest Labs"),
    ("gemini", "Google"),
    ("gemma", "Google"),
    ("glm", "Zhipu AI"),
    ("gpt", "OpenAI"),
    ("granite", "IBM"),
    ("grok", "xAI"),
    ("gte", "Alibaba"),
    ("hunyuan", "Tencent"),
    ("ideogram", "Ideogram"),
    ("imagen", "Google"),
    ("inflection", "Inflection AI"),
    ("internvl", "OpenGVLab"),
    ("jais", "G42"),
    ("jamba", "AI21 Labs"),
    ("kimi", "Moonshot AI"),
    ("kling", "Kuaishou"),
    ("learnlm", "Google"),
    ("lfm", "Liquid AI"),
    ("ling", "Inclusion AI"),
    ("llama", "Meta"),
    ("llama3", "Meta"),
    ("luma", "Luma AI"),
    ("lyria", "Google"),
    ("magistral", "Mistral AI"),
    ("minimax", "MiniMax"),
    ("ministral", "Mistral AI"),
    ("mistral", "Mistral AI"),
    ("mixtral", "Mistral AI"),
    ("moonshot", "Moonshot AI"),
    ("mimo", "Xiaomi"),
    ("o1", "OpenAI"),
    ("o3", "OpenAI"),
    ("o4", "OpenAI"),
    ("olmo", "Allen AI"),
    ("open", "Mistral AI"),
    ("palmyra", "Writer"),
    ("phi", "Microsoft"),
    ("pixtral", "Mistral AI"),
    ("qianfan", "Baidu"),
    ("qvq", "Alibaba"),
    ("qwen", "Alibaba"),
    ("qwen2", "Alibaba"),
    ("qwen3", "Alibaba"),
    ("qwq", "Alibaba"),
    ("recraft", "Recraft"),
    ("recurrentgemma", "Google"),
    ("rerank", "Cohere"),
    ("runway", "Runway"),
    ("seed", "ByteDance"),
    ("solar", "Upstage"),
    ("sonar", "Perplexity"),
    ("step", "StepFun"),
    ("v0", "Vercel"),
    ("veo", "Google"),
    ("venice", "Venice"),
    ("voxtral", "Mistral AI"),
    ("voyage", "Voyage AI"),
    ("whisper", "OpenAI"),
    ("yi", "01.AI"),
];

#[derive(Clone)]
struct CatalogCandidate {
    raw_id: String,
    canonical_id: String,
    definition: CatalogModelDefinition,
}

pub fn curate_catalog_document(
    document: ModelCatalogDocument,
) -> Result<ModelCatalogDocument, ModelCatalogCurationError> {
    curate_catalog_document_with_mode(document, CatalogCurationMode::Public)
}

pub fn curate_live_catalog_document(
    document: ModelCatalogDocument,
) -> Result<ModelCatalogDocument, ModelCatalogCurationError> {
    curate_catalog_document_with_mode(document, CatalogCurationMode::Live)
}

#[derive(Clone, Copy)]
enum CatalogCurationMode {
    Public,
    Live,
}

fn curate_catalog_document_with_mode(
    document: ModelCatalogDocument,
    mode: CatalogCurationMode,
) -> Result<ModelCatalogDocument, ModelCatalogCurationError> {
    let mut curated_models = BTreeMap::<String, CatalogCandidate>::new();

    for (raw_id, definition) in document.models {
        let canonical_id = normalized_catalog_model_id(raw_id.as_str());
        let allowed = match mode {
            CatalogCurationMode::Public => is_allowed_canonical_model_id(canonical_id.as_str()),
            CatalogCurationMode::Live => is_allowed_live_canonical_model_id(canonical_id.as_str()),
        };
        if !allowed {
            continue;
        }

        let candidate = CatalogCandidate {
            raw_id,
            canonical_id: canonical_id.clone(),
            definition,
        };
        if let Some(current) = curated_models.get(canonical_id.as_str()) {
            let (primary, secondary) = if compare_candidates(&candidate, current) > 0 {
                (&candidate, current)
            } else {
                (current, &candidate)
            };
            curated_models.insert(
                canonical_id,
                CatalogCandidate {
                    raw_id: primary.raw_id.clone(),
                    canonical_id: primary.canonical_id.clone(),
                    definition: merge_definitions(&primary.definition, &secondary.definition)?,
                },
            );
            continue;
        }
        curated_models.insert(canonical_id, candidate);
    }

    let mut models = BTreeMap::new();
    for (model_id, candidate) in curated_models {
        let mut definition = candidate.definition;
        let existing_origin = normalize_optional_text(definition.origin.clone());
        definition.origin =
            existing_origin.or_else(|| origin_for_model(model_id.as_str(), &definition));
        models.insert(model_id, definition);
    }

    Ok(ModelCatalogDocument { models })
}

fn is_allowed_canonical_model_id(id: &str) -> bool {
    if id.is_empty() || id.contains('/') || is_canonical_source_alias(id) {
        return false;
    }
    if matches!(id, "deepseek-v3.1-terminus" | "vila") {
        return true;
    }
    if BLOCKED_TOKENS.iter().any(|pattern| pattern.is_match(id)) {
        return false;
    }
    if id.starts_with("llama-") && !LLAMA_PATTERNS.iter().any(|pattern| pattern.is_match(id)) {
        return false;
    }
    if id.starts_with("llama3") && !LLAMA3_CANONICAL_RE.is_match(id) {
        return false;
    }
    if id.starts_with("gemma-") && !GEMMA_PATTERNS.iter().any(|pattern| pattern.is_match(id)) {
        return false;
    }
    if id.starts_with("codegemma-")
        && !CODEGEMMA_PATTERNS
            .iter()
            .any(|pattern| pattern.is_match(id))
    {
        return false;
    }

    let root = extract_root(id);
    OFFICIAL_ROOTS.contains(&root)
        || id.starts_with("all-mini-lm-l6-v2")
        || id.starts_with("text-embedding-")
        || id.starts_with("gpt-image-")
        || id.starts_with("omni-moderation-")
        || id.starts_with("tts-")
        || id.starts_with("whisper-")
        || id.starts_with("dall-e-")
        || looks_like_generic_model_id(id)
}

fn is_allowed_live_canonical_model_id(id: &str) -> bool {
    !id.is_empty() && !id.contains('/') && !is_canonical_source_alias(id)
}

fn looks_like_generic_model_id(id: &str) -> bool {
    let has_alpha = id.chars().any(|ch| ch.is_ascii_alphabetic());
    let has_signal = id.contains('-')
        || id.contains('.')
        || id.chars().any(|ch| ch.is_ascii_digit())
        || id.len() >= 5;
    has_alpha && has_signal
}

fn is_canonical_source_alias(id: &str) -> bool {
    SOURCE_PREFIX_RE.is_match(id)
        || BANNED_SOURCE_PREFIXES
            .iter()
            .any(|prefix| id.starts_with(prefix))
}

fn compare_candidates(next: &CatalogCandidate, current: &CatalogCandidate) -> i32 {
    let source_priority_delta = next.definition.source_priority.sort_priority
        - current.definition.source_priority.sort_priority;
    if source_priority_delta != 0 {
        return source_priority_delta;
    }
    let source_delta = source_preference_score(
        next.raw_id.as_str(),
        next.canonical_id.as_str(),
        &next.definition,
    ) - source_preference_score(
        current.raw_id.as_str(),
        current.canonical_id.as_str(),
        &current.definition,
    );
    if source_delta != 0 {
        return source_delta;
    }
    model_richness_score(&next.definition) - model_richness_score(&current.definition)
}

fn source_preference_score(
    raw_id: &str,
    canonical_id: &str,
    definition: &CatalogModelDefinition,
) -> i32 {
    let mut score = 0;
    let raw_id_lower = raw_id.to_ascii_lowercase();
    if raw_id_lower == canonical_id {
        score += 500;
    }
    if raw_id == raw_id_lower {
        score += 25;
    }
    if SOURCE_PREFIX_RE.is_match(raw_id_lower.as_str()) {
        score -= 200;
    }
    for prefix in BANNED_SOURCE_PREFIXES {
        if raw_id_lower.starts_with(prefix) {
            score -= 150;
        }
    }
    if raw_id_lower.ends_with("@default")
        || raw_id_lower.ends_with("-maas")
        || raw_id_lower.ends_with(":free")
        || raw_id_lower.ends_with("-free")
    {
        score -= 200;
    }
    if let Some(display_name) = definition.display_name.as_deref() {
        let display = display_name.to_ascii_lowercase();
        if display.contains("bedrock") || display.contains("gitlab") || display.contains("free") {
            score -= 50;
        }
    }
    score
}

fn model_richness_score(definition: &CatalogModelDefinition) -> i32 {
    let mut score = 0;
    if definition.display_name.is_some() {
        score += 5;
    }
    if definition.description.is_some() {
        score += 5;
    }
    if definition.context_window_tokens.is_some() {
        score += 2;
    }
    if definition.max_output_tokens.is_some() {
        score += 2;
    }
    if let Ok(value) = serde_json::to_value(definition) {
        score += supported_list_count(&value, "input") as i32;
        score += supported_list_count(&value, "features") as i32;
    }
    score
}

fn supported_list_count(value: &Value, key: &str) -> usize {
    let Some(field) = value.get(key) else {
        return 0;
    };
    match field {
        Value::Array(values) => values.len(),
        Value::Object(map) => map
            .get("supported")
            .and_then(Value::as_array)
            .map(Vec::len)
            .unwrap_or(0),
        _ => 0,
    }
}

fn merge_definitions(
    primary: &CatalogModelDefinition,
    fallback: &CatalogModelDefinition,
) -> Result<CatalogModelDefinition, ModelCatalogCurationError> {
    let mut primary_value =
        serde_json::to_value(primary).map_err(ModelCatalogCurationError::from)?;
    let fallback_value = serde_json::to_value(fallback).map_err(ModelCatalogCurationError::from)?;
    merge_json(&mut primary_value, &fallback_value);
    let mut merged: CatalogModelDefinition = serde_json::from_value(primary_value)?;
    apply_priority_limit_overrides(&mut merged, primary, fallback);
    merged.source_priority = CatalogDefinitionSourcePriority {
        sort_priority: primary
            .source_priority
            .sort_priority
            .max(fallback.source_priority.sort_priority),
        descriptive_priority: primary
            .source_priority
            .descriptive_priority
            .max(fallback.source_priority.descriptive_priority),
        limits_priority: primary
            .source_priority
            .limits_priority
            .max(fallback.source_priority.limits_priority),
        capability_priority: primary
            .source_priority
            .capability_priority
            .max(fallback.source_priority.capability_priority),
        semantics_priority: primary
            .source_priority
            .semantics_priority
            .max(fallback.source_priority.semantics_priority),
        pricing_priority: primary
            .source_priority
            .pricing_priority
            .max(fallback.source_priority.pricing_priority),
        mode_priority: primary
            .source_priority
            .mode_priority
            .max(fallback.source_priority.mode_priority),
    };
    Ok(merged)
}

fn apply_priority_limit_overrides(
    merged: &mut CatalogModelDefinition,
    primary: &CatalogModelDefinition,
    fallback: &CatalogModelDefinition,
) {
    if fallback.source_priority.limits_priority <= primary.source_priority.limits_priority {
        return;
    }

    override_option_field(
        &mut merged.context_window_tokens,
        fallback.context_window_tokens,
    );
    override_option_field(&mut merged.max_input_tokens, fallback.max_input_tokens);
    override_option_field(&mut merged.max_output_tokens, fallback.max_output_tokens);
}

fn override_option_field<T: Copy>(current: &mut Option<T>, preferred: Option<T>) {
    if preferred.is_some() {
        *current = preferred;
    }
}

fn merge_json(primary: &mut Value, fallback: &Value) {
    match fallback {
        Value::Array(fallback_values) => {
            let Some(primary_values) = primary.as_array_mut() else {
                if primary.is_null() {
                    *primary = fallback.clone();
                }
                return;
            };
            for value in fallback_values {
                if !primary_values.contains(value) {
                    primary_values.push(value.clone());
                }
            }
        }
        Value::Object(fallback_values) => {
            let Some(primary_values) = primary.as_object_mut() else {
                if primary.is_null() {
                    *primary = fallback.clone();
                }
                return;
            };
            for (key, fallback_value) in fallback_values {
                if let Some(primary_value) = primary_values.get_mut(key) {
                    merge_json(primary_value, fallback_value);
                } else {
                    primary_values.insert(key.clone(), fallback_value.clone());
                }
            }
        }
        _ if primary.is_null() => *primary = fallback.clone(),
        _ => {}
    }
}

fn origin_for_model(model_id: &str, definition: &CatalogModelDefinition) -> Option<String> {
    let root = extract_root(model_id);
    let display_name = normalize_optional_text(definition.display_name.clone())
        .unwrap_or_default()
        .to_ascii_lowercase();

    if model_id.starts_with("text-embedding-005") {
        return Some("Google".to_owned());
    }
    if model_id.starts_with("text-embedding-") {
        return Some("OpenAI".to_owned());
    }
    if model_id.starts_with("gpt-image-") || model_id.starts_with("dall-e-") {
        return Some("OpenAI".to_owned());
    }
    if model_id.starts_with("omni-moderation-") || model_id.starts_with("tts-") {
        return Some("OpenAI".to_owned());
    }
    if model_id.starts_with("nova-3") || model_id.starts_with("nova-2-") {
        return Some("Deepgram".to_owned());
    }
    if model_id.starts_with("nova-lite")
        || model_id.starts_with("nova-micro")
        || model_id.starts_with("nova-pro")
        || model_id.starts_with("nova-premier")
        || display_name.contains("amazon")
    {
        return Some("Amazon".to_owned());
    }

    if let Some((_, origin)) = ROOT_ORIGINS
        .iter()
        .find(|(candidate, _)| *candidate == root)
    {
        return Some((*origin).to_owned());
    }

    for (pattern, origin) in DISPLAY_ORIGIN_RULES.iter() {
        if pattern.is_match(display_name.as_str()) {
            return Some((*origin).to_owned());
        }
    }

    (!root.is_empty()).then(|| title_case_origin(root))
}

fn extract_root(model_id: &str) -> &str {
    let end = model_id
        .char_indices()
        .find(|(_, ch)| !ch.is_ascii_alphanumeric())
        .map(|(index, _)| index)
        .unwrap_or(model_id.len());
    &model_id[..end]
}

fn title_case_origin(value: &str) -> String {
    value
        .split([' ', '.', '_', '-'])
        .filter(|part| !part.is_empty())
        .map(|part| {
            if part.chars().next().is_some_and(|ch| ch.is_ascii_digit()) {
                part.to_ascii_uppercase()
            } else {
                let mut chars = part.chars();
                let Some(first) = chars.next() else {
                    return String::new();
                };
                let mut titled = first.to_uppercase().collect::<String>();
                titled.push_str(chars.as_str());
                titled
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn normalize_optional_text(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let normalized = value.trim();
        (!normalized.is_empty()).then(|| normalized.to_owned())
    })
}

#[cfg(test)]
mod tests {
    use agena_provider::catalog_model_id_for_raw;

    #[test]
    fn raw_catalog_id_projection_normalizes_and_rejects_blank_values() {
        assert_eq!(
            catalog_model_id_for_raw(" us.openai.GPT_5@default "),
            Some("gpt.5".to_owned())
        );
        assert_eq!(catalog_model_id_for_raw("   "), None);
    }
}
