use std::{collections::BTreeMap, sync::LazyLock};

use regex::Regex;
use serde_json::Value;

use crate::AppError;

use super::{CatalogModelDefinition, ModelCatalogDocument};

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
        r"^llama-2-(7b|13b|70b)-chat$",
        r"^llama-3-(8b|70b)-instruct$",
        r"^llama-3\.1-(8b|70b|405b)-instruct$",
        r"^llama-3\.2-(1b|3b)-instruct$",
        r"^llama-3\.2-(11b|90b)-vision-instruct$",
        r"^llama-3\.3-70b-instruct$",
        r"^llama-4-(maverick|scout)$",
        r"^llama-4-(maverick|scout)-17b(-128e|-16e)?-instruct$",
        r"^llama-guard-",
    ]
    .into_iter()
    .map(|pattern| Regex::new(pattern).unwrap())
    .collect()
});

static GEMMA_PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
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
    [r"^codegemma-(2b|7b|7b-it)$"]
        .into_iter()
        .map(|pattern| Regex::new(pattern).unwrap())
        .collect()
});

static LLAMA3_CANONICAL_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^llama3(\.1-8b|\.3-70b-instruct)$").unwrap());

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

static CANONICAL_RULES: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
    vec![
        (
            Regex::new(r"^amazon\.nova-([a-z0-9-]+)-v1:0$").unwrap(),
            "nova-$1-v1",
        ),
        (
            Regex::new(r"^amazon\.nova-([a-z0-9-]+):0$").unwrap(),
            "nova-$1",
        ),
        (
            Regex::new(r"^meta\.llama3-1-(\d+b)-instruct-v1:0$").unwrap(),
            "llama-3.1-$1-instruct",
        ),
        (
            Regex::new(r"^meta\.llama3-3-(70b)-instruct-v1:0$").unwrap(),
            "llama-3.3-$1-instruct",
        ),
        (
            Regex::new(r"^meta\.llama3-(70b)-instruct$").unwrap(),
            "llama-3-$1-instruct",
        ),
        (
            Regex::new(r"^meta\.llama4-maverick-17b-instruct-v1:0$").unwrap(),
            "llama-4-maverick-17b-128e-instruct",
        ),
        (
            Regex::new(r"^meta\.llama4-scout-17b-instruct-v1:0$").unwrap(),
            "llama-4-scout-17b-16e-instruct",
        ),
        (
            Regex::new(r"^openai\.gpt-oss-(120b|20b)-1:0$").unwrap(),
            "gpt-oss-$1",
        ),
        (
            Regex::new(r"^gpt-oss-(120b|20b)-1:0$").unwrap(),
            "gpt-oss-$1",
        ),
        (
            Regex::new(r"^(claude-(?:haiku|opus|sonnet)-[^:]+)-v\d+:0$").unwrap(),
            "$1",
        ),
        (
            Regex::new(r"^claude-3-5-haiku-(\d{8})-v1:0$").unwrap(),
            "claude-haiku-3-5-$1",
        ),
        (
            Regex::new(r"^claude-3-5-sonnet-(\d{8})-v2:0$").unwrap(),
            "claude-sonnet-3-5-$1",
        ),
        (
            Regex::new(r"^claude-3-7-sonnet-(\d{8})-v1:0$").unwrap(),
            "claude-sonnet-3-7-$1",
        ),
        (
            Regex::new(r"^claude-opus-4-6-v1$").unwrap(),
            "claude-opus-4-6",
        ),
        (Regex::new(r"^deepseek\.r1-v1:0$").unwrap(), "deepseek-r1"),
        (Regex::new(r"^deepseek\.v3-v1:0$").unwrap(), "deepseek-v3"),
        (
            Regex::new(r"^qwen3-235b-a22b-2507-v1:0$").unwrap(),
            "qwen3-235b-a22b-2507",
        ),
        (
            Regex::new(r"^qwen3\.235b-a22b-instruct-2507$").unwrap(),
            "qwen3-235b-a22b-instruct-2507",
        ),
        (Regex::new(r"^qwen3\.5:397b$").unwrap(), "qwen3.5-397b-a17b"),
        (Regex::new(r"^qwen3-32b-v1:0$").unwrap(), "qwen3-32b"),
        (
            Regex::new(r"^qwen3-coder-30b-a3b-v1:0$").unwrap(),
            "qwen3-coder-30b-a3b",
        ),
        (
            Regex::new(r"^qwen3-coder-480b-a35b-v1:0$").unwrap(),
            "qwen3-coder-480b-a35b-instruct",
        ),
        (
            Regex::new(r"^qwen3-coder:480b$").unwrap(),
            "qwen3-coder-480b-a35b-instruct",
        ),
        (
            Regex::new(r"^qwen3-next-80b-a3b$").unwrap(),
            "qwen3-next-80b-a3b-instruct",
        ),
        (Regex::new(r"^qwen3-next:80b$").unwrap(), "qwen3-next-80b"),
        (
            Regex::new(r"^qwen3-vl:235b$").unwrap(),
            "qwen3-vl-235b-a22b",
        ),
        (
            Regex::new(r"^qwen3-vl:235b-instruct$").unwrap(),
            "qwen3-vl-235b-a22b-instruct",
        ),
        (
            Regex::new(r"^palmyra-x([45])-v1:0$").unwrap(),
            "palmyra-x$1",
        ),
        (Regex::new(r"^gpt-oss:(120b|20b)$").unwrap(), "gpt-oss-$1"),
        (Regex::new(r"^glm4\.7$").unwrap(), "glm-4.7"),
        (Regex::new(r"^glm5$").unwrap(), "glm-5"),
        (Regex::new(r"^llama3\.1-8b$").unwrap(), "llama-3.1-8b"),
        (
            Regex::new(r"^llama3\.3-70b-instruct$").unwrap(),
            "llama-3.3-70b-instruct",
        ),
        (Regex::new(r"^devstral-2:123b$").unwrap(), "devstral-2-123b"),
        (
            Regex::new(r"^devstral-small-2:24b$").unwrap(),
            "devstral-small-2-24b",
        ),
        (
            Regex::new(r"^ministral-3:(14b|8b|3b)$").unwrap(),
            "ministral-3-$1",
        ),
        (
            Regex::new(r"^pixtral-large-2502-v1:0$").unwrap(),
            "pixtral-large-2502",
        ),
        (
            Regex::new(r"^(aion)-(\d+)[-.](\d+)(.*)$").unwrap(),
            "$1-$2.$3$4",
        ),
        (
            Regex::new(r"^(claude-(?:haiku|opus|sonnet)-)(\d)(\d)(.*)$").unwrap(),
            "$1$2-$3$4",
        ),
        (
            Regex::new(r"^(deepseek-v)(\d+)[-.](\d+)(.*)$").unwrap(),
            "$1$2.$3$4",
        ),
        (
            Regex::new(r"^(gemini)-(\d+)[-.](\d+)(.*)$").unwrap(),
            "$1-$2.$3$4",
        ),
        (
            Regex::new(r"^(gpt)-(\d+)[-.](\d+)(.*)$").unwrap(),
            "$1-$2.$3$4",
        ),
        (Regex::new(r"^(grok)-(\d+)(\d)(.*)$").unwrap(), "$1-$2.$3$4"),
        (
            Regex::new(r"^(grok)-(\d+)[-.](\d+)(.*)$").unwrap(),
            "$1-$2.$3$4",
        ),
        (
            Regex::new(r"^(kimi-k\d+)[-.](\d+)(.*)$").unwrap(),
            "$1.$2$3",
        ),
        (
            Regex::new(r"^(llama)-(\d+)[-.](\d+)(.*)$").unwrap(),
            "$1-$2.$3$4",
        ),
        (
            Regex::new(r"^(minimax-m\d+)[-.](\d+)(.*)$").unwrap(),
            "$1.$2$3",
        ),
        (
            Regex::new(r"^(mistral-small)-(\d+)[-.](\d+)(.*)$").unwrap(),
            "$1-$2.$3$4",
        ),
        (Regex::new(r"^(nvidia)\.(.*)$").unwrap(), "$1-$2"),
        (Regex::new(r"^(qwen\d+)[-.](\d+)(.*)$").unwrap(), "$1.$2$3"),
    ]
});

const DUO_MODEL_MAP: &[(&str, &str)] = &[
    ("duo-chat-sonnet-4-5", "claude-sonnet-4-5"),
    ("duo-chat-sonnet-4-6", "claude-sonnet-4-6"),
    ("duo-chat-opus-4-5", "claude-opus-4-5"),
    ("duo-chat-opus-4-6", "claude-opus-4-6"),
    ("duo-chat-opus-4-7", "claude-opus-4-7"),
    ("duo-chat-haiku-4-5", "claude-haiku-4-5"),
    ("duo-chat-gpt-5", "gpt-5"),
    ("duo-chat-gpt-5-mini", "gpt-5-mini"),
    ("duo-chat-gpt-5-codex", "gpt-5-codex"),
    ("duo-chat-gpt-5-1", "gpt-5.1"),
    ("duo-chat-gpt-5-2", "gpt-5.2"),
    ("duo-chat-gpt-5-2-codex", "gpt-5.2-codex"),
    ("duo-chat-gpt-5-3-codex", "gpt-5.3-codex"),
    ("duo-chat-gpt-5-4", "gpt-5.4"),
    ("duo-chat-gpt-5-4-mini", "gpt-5.4-mini"),
    ("duo-chat-gpt-5-4-nano", "gpt-5.4-nano"),
];

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

pub(super) fn curate_catalog_document(
    document: ModelCatalogDocument,
) -> Result<ModelCatalogDocument, AppError> {
    let mut curated_models = BTreeMap::<String, CatalogCandidate>::new();

    for (raw_id, definition) in document.models {
        let canonical_id = normalized_catalog_model_id(raw_id.as_str());
        if !is_allowed_canonical_model_id(canonical_id.as_str()) {
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

pub(super) fn normalized_catalog_model_id(model_id: &str) -> String {
    let mut normalized = model_id.trim().to_ascii_lowercase();
    normalized = strip_path_prefixes(normalized.as_str());
    if let Some(stripped) = normalized.strip_suffix("@default") {
        normalized = stripped.to_owned();
    }
    if let Some(stripped) = normalized.strip_suffix("-maas") {
        normalized = stripped.to_owned();
    }
    if let Some(stripped) = normalized.strip_suffix(":free") {
        normalized = stripped.to_owned();
    }
    if normalized == "study_gpt-chatgpt-4o-latest" {
        normalized = "gpt-4o".to_owned();
    }
    normalized = SOURCE_PREFIX_RE
        .replace(normalized.as_str(), "")
        .into_owned();

    if let Some(stripped) = normalized.strip_prefix("anthropic.") {
        normalized = stripped.to_owned();
    }
    if let Some(stripped) = normalized.strip_prefix("openai.") {
        normalized = stripped.to_owned();
    }
    if let Some(stripped) = normalized.strip_prefix("azure-") {
        normalized = stripped.to_owned();
    }
    if let Some(stripped) = normalized.strip_prefix("google.") {
        normalized = stripped.to_owned();
    }
    if let Some(stripped) = normalized.strip_prefix("cohere-command-") {
        normalized = format!("command-{stripped}");
    } else if normalized == "cohere-embed-v-4-0" {
        normalized = "embed-v4.0".to_owned();
    } else if let Some(stripped) = normalized.strip_prefix("cohere-embed-v3-") {
        normalized = format!("embed-v3-{stripped}");
    }
    if let Some(stripped) = normalized.strip_prefix("ai21-jamba-") {
        normalized = format!("jamba-{stripped}");
    }
    if let Some(stripped) = normalized.strip_prefix("moonshot-kimi-") {
        normalized = format!("kimi-{stripped}");
    }
    if let Some(stripped) = normalized.strip_prefix("moonshot.") {
        normalized = stripped.to_owned();
    }
    if let Some(stripped) = normalized.strip_prefix("moonshotai.") {
        normalized = stripped.to_owned();
    }
    if let Some(stripped) = normalized.strip_prefix("zai.") {
        normalized = stripped.to_owned();
    }
    if let Some(stripped) = normalized.strip_prefix("minimax.") {
        normalized = stripped.to_owned();
    }
    if let Some(stripped) = normalized.strip_prefix("qwen.") {
        normalized = stripped.to_owned();
    }
    if let Some(stripped) = normalized.strip_prefix("writer.") {
        normalized = stripped.to_owned();
    }
    if let Some(stripped) = normalized.strip_prefix("nvidia.") {
        normalized = stripped.to_owned();
    }
    if let Some(stripped) = normalized.strip_prefix("mistral.") {
        normalized = stripped.to_owned();
    }

    if let Some((_, canonical)) = DUO_MODEL_MAP
        .iter()
        .find(|(raw_id, _)| *raw_id == normalized.as_str())
    {
        normalized = (*canonical).to_owned();
    }

    normalized = normalized.replace('_', ".");

    for (pattern, replacement) in CANONICAL_RULES.iter() {
        normalized = pattern
            .replace(normalized.as_str(), *replacement)
            .into_owned();
    }

    normalized = normalized.replace("v1_5", "v1.5");
    normalized = normalized.replace("v2_5", "v2.5");
    normalized = normalized.replace("v3_5", "v3.5");
    normalized = normalized.replace("v4_5", "v4.5");
    normalized = normalized.replace("v5_5", "v5.5");
    normalized = normalized.replace(".v", "-v");
    while normalized.contains("--") {
        normalized = normalized.replace("--", "-");
    }

    normalized
}

fn strip_path_prefixes(value: &str) -> String {
    value
        .rsplit('/')
        .find(|segment| !segment.trim().is_empty())
        .unwrap_or(value)
        .trim()
        .to_owned()
}

fn is_allowed_canonical_model_id(id: &str) -> bool {
    if id.is_empty() || id.contains('/') || is_canonical_source_alias(id) {
        return false;
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
}

fn is_canonical_source_alias(id: &str) -> bool {
    SOURCE_PREFIX_RE.is_match(id)
        || BANNED_SOURCE_PREFIXES
            .iter()
            .any(|prefix| id.starts_with(prefix))
}

fn compare_candidates(next: &CatalogCandidate, current: &CatalogCandidate) -> i32 {
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
) -> Result<CatalogModelDefinition, AppError> {
    let mut primary_value = serde_json::to_value(primary).map_err(AppError::from)?;
    let fallback_value = serde_json::to_value(fallback).map_err(AppError::from)?;
    merge_json(&mut primary_value, &fallback_value);
    serde_json::from_value(primary_value)
        .map_err(|err| AppError::Config(format!("merge model catalog definitions: {err}")))
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
    use super::*;

    #[test]
    fn curate_catalog_collapses_aliases_and_keeps_official_entry() {
        let document = ModelCatalogDocument {
            models: BTreeMap::from([
                (
                    "openai.gpt-5.4".to_owned(),
                    CatalogModelDefinition {
                        display_name: Some("GPT-5.4 via Bedrock".to_owned()),
                        description: Some("fallback".to_owned()),
                        ..CatalogModelDefinition::default()
                    },
                ),
                (
                    "gpt-5.4".to_owned(),
                    CatalogModelDefinition {
                        display_name: Some("GPT-5.4".to_owned()),
                        ..CatalogModelDefinition::default()
                    },
                ),
                (
                    "study_gpt-chatgpt-4o-latest".to_owned(),
                    CatalogModelDefinition {
                        display_name: Some("ChatGPT-4o Latest".to_owned()),
                        ..CatalogModelDefinition::default()
                    },
                ),
            ]),
        };

        let curated = curate_catalog_document(document).expect("catalog should curate");
        assert!(curated.models.contains_key("gpt-5.4"));
        assert!(curated.models.contains_key("gpt-4o"));
        assert!(!curated.models.contains_key("openai.gpt-5.4"));
        assert!(!curated.models.contains_key("study_gpt-chatgpt-4o-latest"));
        assert_eq!(
            curated
                .models
                .get("gpt-5.4")
                .and_then(|definition| definition.origin.as_deref()),
            Some("OpenAI")
        );
        assert_eq!(
            curated
                .models
                .get("gpt-5.4")
                .and_then(|definition| definition.description.as_deref()),
            Some("fallback")
        );
        assert_eq!(
            curated
                .models
                .get("gpt-5.4")
                .and_then(|definition| definition.display_name.as_deref()),
            Some("GPT-5.4")
        );
    }

    #[test]
    fn canonical_model_id_matches_expected_aliases() {
        assert_eq!(normalized_catalog_model_id("openai.gpt-5.4"), "gpt-5.4");
        assert_eq!(
            normalized_catalog_model_id("openai/gpt-oss-120b"),
            "gpt-oss-120b"
        );
        assert_eq!(
            normalized_catalog_model_id("models/qwen/qwen3-next-80b-a3b-thinking"),
            "qwen3-next-80b-a3b-thinking"
        );
        assert_eq!(
            normalized_catalog_model_id("deepseek-ai/deepseek-v4-pro"),
            "deepseek-v4-pro"
        );
        assert_eq!(
            normalized_catalog_model_id("amazon.nova-pro-v1:0"),
            "nova-pro-v1"
        );
        assert_eq!(
            normalized_catalog_model_id("meta.llama4-scout-17b-instruct-v1:0"),
            "llama-4-scout-17b-16e-instruct"
        );
        assert_eq!(normalized_catalog_model_id("Kimi-K2_6"), "kimi-k2.6");
        assert_eq!(normalized_catalog_model_id("gpt-oss:120b"), "gpt-oss-120b");
    }
}
