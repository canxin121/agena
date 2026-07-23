//! Canonical provider model-catalog identifier values.
//!
//! These transforms normalize provider-native IDs before catalog lookup. They
//! are protocol/catalog values; Runtime curation decides which normalized IDs
//! are admitted into a concrete catalog document.

use std::sync::LazyLock;

use regex::Regex;

static SOURCE_PREFIX_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?:us|eu|au|jp|global|apac)\.").unwrap());

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

/// Normalizes a raw provider model ID into the catalog's canonical identity.
pub fn normalized_catalog_model_id(model_id: &str) -> String {
    let mut normalized = strip_path_prefixes(model_id.trim().to_ascii_lowercase().as_str());
    for suffix in ["@default", "-maas", ":free", "-free"] {
        if let Some(stripped) = normalized.strip_suffix(suffix) {
            normalized = stripped.to_owned();
        }
    }
    if normalized == "study_gpt-chatgpt-4o-latest" {
        normalized = "gpt-4o".to_owned();
    }
    normalized = SOURCE_PREFIX_RE
        .replace(normalized.as_str(), "")
        .into_owned();
    for prefix in ["anthropic.", "openai.", "azure-", "google."] {
        if let Some(stripped) = normalized.strip_prefix(prefix) {
            normalized = stripped.to_owned();
        }
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
    for prefix in [
        "moonshot.",
        "moonshotai.",
        "zai.",
        "minimax.",
        "qwen.",
        "writer.",
        "nvidia.",
        "mistral.",
    ] {
        if let Some(stripped) = normalized.strip_prefix(prefix) {
            normalized = stripped.to_owned();
        }
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
    for (from, to) in [
        ("v1_5", "v1.5"),
        ("v2_5", "v2.5"),
        ("v3_5", "v3.5"),
        ("v4_5", "v4.5"),
        ("v5_5", "v5.5"),
        (".v", "-v"),
    ] {
        normalized = normalized.replace(from, to);
    }
    while normalized.contains("--") {
        normalized = normalized.replace("--", "-");
    }
    normalized
}

/// Converts a raw provider model ID into a non-empty canonical catalog key.
pub fn catalog_model_id_for_raw(raw_model_id: &str) -> Option<String> {
    let canonical = normalized_catalog_model_id(raw_model_id);
    (!canonical.trim().is_empty()).then_some(canonical)
}

fn strip_path_prefixes(value: &str) -> String {
    value
        .rsplit('/')
        .find(|segment| !segment.trim().is_empty())
        .unwrap_or(value)
        .trim()
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::{catalog_model_id_for_raw, normalized_catalog_model_id};

    #[test]
    fn canonicalizes_provider_and_catalog_aliases() {
        assert_eq!(
            normalized_catalog_model_id(" us.openai.GPT_5@default "),
            "gpt.5"
        );
        assert_eq!(catalog_model_id_for_raw("   "), None);
    }
}
