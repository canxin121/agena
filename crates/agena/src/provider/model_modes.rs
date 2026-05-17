use std::{collections::BTreeMap, sync::LazyLock};

use regex::Regex;

use crate::model::{AdapterId, ModelThinkingMode};

use super::{CapabilityFamily, ReasoningEffort, ThinkingRequest};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ModelModeRegistry;

impl ModelModeRegistry {
    pub fn thinking_modes_for_family(
        &self,
        family: CapabilityFamily,
        adapter_id: Option<&AdapterId>,
        model: &str,
    ) -> BTreeMap<String, ModelThinkingMode> {
        let normalized = normalize_model(model);
        if normalized.is_empty() {
            return BTreeMap::new();
        }

        let protocol = protocol_for_model(family, adapter_id, normalized.as_str());
        match protocol {
            ThinkingProtocol::None => BTreeMap::new(),
            ThinkingProtocol::OpenAi => openai_reasoning_modes(normalized.as_str()),
            ThinkingProtocol::OpenAiCompatible => {
                openai_compatible_reasoning_modes(normalized.as_str())
            }
            ThinkingProtocol::Anthropic => anthropic_reasoning_modes(normalized.as_str()),
            ThinkingProtocol::Gemini => gemini_reasoning_modes(normalized.as_str()),
            ThinkingProtocol::Bedrock => bedrock_reasoning_modes(normalized.as_str()),
        }
    }
}

pub fn default_model_mode_registry() -> &'static ModelModeRegistry {
    static REGISTRY: LazyLock<ModelModeRegistry> = LazyLock::new(ModelModeRegistry::default);
    &REGISTRY
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ThinkingProtocol {
    None,
    OpenAi,
    OpenAiCompatible,
    Anthropic,
    Gemini,
    Bedrock,
}

fn protocol_for_model(
    family: CapabilityFamily,
    adapter_id: Option<&AdapterId>,
    model: &str,
) -> ThinkingProtocol {
    match adapter_id.map(AdapterId::as_str) {
        Some("anthropic") => return ThinkingProtocol::Anthropic,
        Some("gemini") => return ThinkingProtocol::Gemini,
        Some("openai") => {
            return match family {
                CapabilityFamily::OpenAi => ThinkingProtocol::OpenAi,
                _ => ThinkingProtocol::OpenAiCompatible,
            };
        }
        _ => {}
    }

    match family {
        CapabilityFamily::OpenAi => ThinkingProtocol::OpenAi,
        CapabilityFamily::OpenAiCompatible | CapabilityFamily::Gitlab => {
            if looks_like_openai_compatible_reasoning_model(model) {
                ThinkingProtocol::OpenAiCompatible
            } else {
                ThinkingProtocol::None
            }
        }
        CapabilityFamily::Anthropic => {
            if model.contains("claude") {
                ThinkingProtocol::Anthropic
            } else {
                ThinkingProtocol::None
            }
        }
        CapabilityFamily::Gemini => {
            if model.contains("gemini") {
                ThinkingProtocol::Gemini
            } else {
                ThinkingProtocol::None
            }
        }
        CapabilityFamily::Bedrock => {
            if model.contains("claude") || model.contains("anthropic") || model.contains("nova") {
                ThinkingProtocol::Bedrock
            } else {
                ThinkingProtocol::None
            }
        }
    }
}

fn looks_like_openai_compatible_reasoning_model(model: &str) -> bool {
    model.contains("gpt")
        || model.starts_with("o1")
        || model.starts_with("o3")
        || model.starts_with("o4")
        || model.contains("claude")
        || model.contains("gemini-3")
        || model.contains("deepseek-v4")
        || model.contains("deep-research")
}

fn openai_reasoning_modes(model: &str) -> BTreeMap<String, ModelThinkingMode> {
    if model.contains("deep-research") {
        return effort_modes(&[ReasoningEffort::Medium], false);
    }
    if let Some(modes) = gpt5_chat_reasoning_modes(model) {
        return modes;
    }
    if GPT5_PRO_RE.is_match(model) {
        return effort_modes(&[ReasoningEffort::High], false);
    }
    if let Some(modes) = gpt5_codex_reasoning_modes(model, false) {
        return modes;
    }
    if let Some(modes) = versioned_gpt5_reasoning_modes(model, false) {
        return modes;
    }

    if GPT5_FAMILY_RE.is_match(model) {
        return effort_modes(
            &[
                ReasoningEffort::Minimal,
                ReasoningEffort::Low,
                ReasoningEffort::Medium,
                ReasoningEffort::High,
            ],
            false,
        );
    }

    if model.starts_with("o1") || model.starts_with("o3") || model.starts_with("o4") {
        return effort_modes(
            &[
                ReasoningEffort::Low,
                ReasoningEffort::Medium,
                ReasoningEffort::High,
            ],
            false,
        );
    }

    BTreeMap::new()
}

fn openai_compatible_reasoning_modes(model: &str) -> BTreeMap<String, ModelThinkingMode> {
    if model.contains("deep-research") {
        return effort_modes(&[ReasoningEffort::Medium], false);
    }
    if let Some(modes) = gpt5_chat_reasoning_modes(model) {
        return modes;
    }
    if GPT5_PRO_RE.is_match(model) {
        return effort_modes(&[ReasoningEffort::High], false);
    }
    if let Some(modes) = gpt5_codex_reasoning_modes(model, true) {
        return modes;
    }
    if let Some(modes) = versioned_gpt5_reasoning_modes(model, true) {
        return modes;
    }
    if model.contains("deepseek-v4") {
        return effort_modes(
            &[
                ReasoningEffort::Low,
                ReasoningEffort::Medium,
                ReasoningEffort::High,
                ReasoningEffort::Max,
            ],
            false,
        );
    }
    if looks_like_openai_compatible_reasoning_model(model) {
        return effort_modes(
            &[
                ReasoningEffort::Minimal,
                ReasoningEffort::Low,
                ReasoningEffort::Medium,
                ReasoningEffort::High,
                ReasoningEffort::Xhigh,
            ],
            true,
        );
    }
    BTreeMap::new()
}

fn anthropic_reasoning_modes(model: &str) -> BTreeMap<String, ModelThinkingMode> {
    if !model.contains("claude") {
        return BTreeMap::new();
    }
    if model.contains("opus-4-7") || model.contains("opus-4.7") {
        return effort_modes(
            &[
                ReasoningEffort::Low,
                ReasoningEffort::Medium,
                ReasoningEffort::High,
                ReasoningEffort::Xhigh,
                ReasoningEffort::Max,
            ],
            false,
        );
    }
    if model.contains("opus-4-6")
        || model.contains("opus-4.6")
        || model.contains("sonnet-4-6")
        || model.contains("sonnet-4.6")
    {
        return effort_modes(
            &[
                ReasoningEffort::Low,
                ReasoningEffort::Medium,
                ReasoningEffort::High,
                ReasoningEffort::Max,
            ],
            false,
        );
    }
    if model.contains("opus-4-5") || model.contains("opus-4.5") {
        return effort_modes(
            &[
                ReasoningEffort::Low,
                ReasoningEffort::Medium,
                ReasoningEffort::High,
            ],
            false,
        );
    }
    effort_modes(&[ReasoningEffort::High, ReasoningEffort::Max], false)
}

fn gemini_reasoning_modes(model: &str) -> BTreeMap<String, ModelThinkingMode> {
    if model.contains("gemini-2.5") {
        return effort_modes(&[ReasoningEffort::High, ReasoningEffort::Max], false);
    }
    if model.contains("gemini-3") {
        if model.contains("flash-image") {
            return effort_modes(&[ReasoningEffort::Minimal, ReasoningEffort::High], false);
        }
        if model.contains("pro-image") {
            return effort_modes(&[ReasoningEffort::High], false);
        }
        if model.contains("flash") {
            return effort_modes(
                &[
                    ReasoningEffort::Minimal,
                    ReasoningEffort::Low,
                    ReasoningEffort::Medium,
                    ReasoningEffort::High,
                ],
                false,
            );
        }
        return effort_modes(
            &[
                ReasoningEffort::Low,
                ReasoningEffort::Medium,
                ReasoningEffort::High,
            ],
            false,
        );
    }
    BTreeMap::new()
}

fn bedrock_reasoning_modes(model: &str) -> BTreeMap<String, ModelThinkingMode> {
    if model.contains("claude-opus-4-7") || model.contains("claude-opus-4.7") {
        return adaptive_modes(&[
            ReasoningEffort::Low,
            ReasoningEffort::Medium,
            ReasoningEffort::High,
            ReasoningEffort::Xhigh,
            ReasoningEffort::Max,
        ]);
    }
    if model.contains("claude-opus-4-6") || model.contains("claude-opus-4.6") {
        return adaptive_modes(&[
            ReasoningEffort::Low,
            ReasoningEffort::Medium,
            ReasoningEffort::High,
            ReasoningEffort::Max,
        ]);
    }
    if model.contains("claude-sonnet-4-6") || model.contains("claude-sonnet-4.6") {
        return adaptive_modes(&[
            ReasoningEffort::Low,
            ReasoningEffort::Medium,
            ReasoningEffort::High,
        ]);
    }
    if model.contains("claude") || model.contains("anthropic") {
        return effort_modes(&[ReasoningEffort::High, ReasoningEffort::Max], false);
    }
    if model.contains("nova") {
        return effort_modes(
            &[
                ReasoningEffort::Low,
                ReasoningEffort::Medium,
                ReasoningEffort::High,
            ],
            false,
        );
    }
    BTreeMap::new()
}

fn gpt5_chat_reasoning_modes(model: &str) -> Option<BTreeMap<String, ModelThinkingMode>> {
    if !GPT5_FAMILY_RE.is_match(model) || !model.contains("-chat") {
        return None;
    }
    if gpt5_version(model).is_none() {
        return Some(BTreeMap::new());
    }
    Some(effort_modes(&[ReasoningEffort::Medium], false))
}

fn gpt5_codex_reasoning_modes(
    model: &str,
    compatible: bool,
) -> Option<BTreeMap<String, ModelThinkingMode>> {
    if !GPT5_FAMILY_RE.is_match(model) || !model.contains("codex") {
        return None;
    }
    let version = gpt5_version(model);
    let efforts = if version.is_some_and(|version| version >= 3) {
        vec![
            ReasoningEffort::Low,
            ReasoningEffort::Medium,
            ReasoningEffort::High,
            ReasoningEffort::Xhigh,
        ]
    } else if model.contains("codex-max") || version.is_some_and(|version| version >= 2) {
        vec![
            ReasoningEffort::Low,
            ReasoningEffort::Medium,
            ReasoningEffort::High,
            ReasoningEffort::Xhigh,
        ]
    } else {
        vec![
            ReasoningEffort::Low,
            ReasoningEffort::Medium,
            ReasoningEffort::High,
        ]
    };
    Some(effort_modes(
        efforts.as_slice(),
        compatible && version.is_some_and(|v| v >= 3),
    ))
}

fn versioned_gpt5_reasoning_modes(
    model: &str,
    compatible: bool,
) -> Option<BTreeMap<String, ModelThinkingMode>> {
    let version = gpt5_version(model)?;
    if GPT5_VERSIONED_PRO_RE.is_match(model) {
        return Some(effort_modes(
            &[
                ReasoningEffort::Medium,
                ReasoningEffort::High,
                ReasoningEffort::Xhigh,
            ],
            false,
        ));
    }
    if version == 1 {
        return Some(effort_modes(
            &[
                ReasoningEffort::Low,
                ReasoningEffort::Medium,
                ReasoningEffort::High,
            ],
            true,
        ));
    }
    Some(effort_modes(
        &[
            ReasoningEffort::Low,
            ReasoningEffort::Medium,
            ReasoningEffort::High,
            ReasoningEffort::Xhigh,
        ],
        compatible || version >= 2,
    ))
}

fn adaptive_modes(efforts: &[ReasoningEffort]) -> BTreeMap<String, ModelThinkingMode> {
    let mut modes = BTreeMap::new();
    for effort in efforts {
        modes.insert(
            format!("thinking-{}", effort.as_str()),
            ModelThinkingMode::new()
                .with_display_name(format!("Thinking {}", title_case(effort.as_str())))
                .with_thinking(ThinkingRequest::Adaptive {
                    effort: Some(*effort),
                }),
        );
    }
    modes
}

fn effort_modes(
    efforts: &[ReasoningEffort],
    include_disabled: bool,
) -> BTreeMap<String, ModelThinkingMode> {
    let mut modes = BTreeMap::new();
    if include_disabled {
        modes.insert(
            "no-thinking".to_owned(),
            ModelThinkingMode::new()
                .with_display_name("No Thinking")
                .with_thinking(ThinkingRequest::Disabled),
        );
    }
    for effort in efforts {
        modes.insert(
            format!("thinking-{}", effort.as_str()),
            ModelThinkingMode::new()
                .with_display_name(format!("Thinking {}", title_case(effort.as_str())))
                .with_thinking(ThinkingRequest::Effort { effort: *effort }),
        );
    }
    modes
}

fn title_case(value: &str) -> String {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };
    format!(
        "{}{}",
        first.to_ascii_uppercase(),
        chars.as_str().to_ascii_lowercase()
    )
}

fn normalize_model(model: &str) -> String {
    model.trim().to_ascii_lowercase()
}

fn gpt5_version(model: &str) -> Option<u32> {
    GPT5_VERSION_RE
        .captures(model)
        .and_then(|captures| captures.get(1))
        .and_then(|capture| capture.as_str().parse::<u32>().ok())
}

static GPT5_FAMILY_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:^|/)gpt-5(?:[.-]|$)").expect("valid gpt-5 regex"));
static GPT5_VERSION_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:^|/)gpt-5[.-](\d+)(?:[.-]|$)").expect("valid version regex"));
static GPT5_PRO_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:^|/)gpt-5[.-]?pro(?:[.-]|$)").expect("valid gpt-5 pro regex"));
static GPT5_VERSIONED_PRO_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:^|/)gpt-5[.-]\d+[.-]pro(?:[.-]|$)").expect("valid versioned gpt-5 pro regex")
});

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openai_gpt5_versioned_modes_match_expected_efforts() {
        let modes = default_model_mode_registry().thinking_modes_for_family(
            CapabilityFamily::OpenAi,
            None,
            "gpt-5.2",
        );
        assert!(modes.contains_key("no-thinking"));
        assert!(modes.contains_key("thinking-low"));
        assert!(modes.contains_key("thinking-medium"));
        assert!(modes.contains_key("thinking-high"));
        assert!(modes.contains_key("thinking-xhigh"));
        assert!(!modes.contains_key("thinking-minimal"));
    }

    #[test]
    fn openai_compatible_claude_modes_use_wide_effort_set() {
        let modes = default_model_mode_registry().thinking_modes_for_family(
            CapabilityFamily::OpenAiCompatible,
            None,
            "claude-sonnet-4-6",
        );
        assert!(modes.contains_key("no-thinking"));
        assert!(modes.contains_key("thinking-minimal"));
        assert!(modes.contains_key("thinking-high"));
        assert!(modes.contains_key("thinking-xhigh"));
    }

    #[test]
    fn anthropic_adaptive_models_get_richer_effort_ladder() {
        let modes = default_model_mode_registry().thinking_modes_for_family(
            CapabilityFamily::Anthropic,
            None,
            "claude-opus-4.7",
        );
        assert!(modes.contains_key("thinking-low"));
        assert!(modes.contains_key("thinking-medium"));
        assert!(modes.contains_key("thinking-high"));
        assert!(modes.contains_key("thinking-xhigh"));
        assert!(modes.contains_key("thinking-max"));
    }

    #[test]
    fn gemini_flash_image_modes_follow_online_provider_profiles() {
        let modes = default_model_mode_registry().thinking_modes_for_family(
            CapabilityFamily::Gemini,
            None,
            "gemini-3-flash-image",
        );
        assert!(modes.contains_key("thinking-minimal"));
        assert!(modes.contains_key("thinking-high"));
        assert_eq!(modes.len(), 2);
    }

    #[test]
    fn bedrock_nova_models_get_reasoning_efforts() {
        let modes = default_model_mode_registry().thinking_modes_for_family(
            CapabilityFamily::Bedrock,
            None,
            "nova-pro-v1",
        );
        assert!(modes.contains_key("thinking-low"));
        assert!(modes.contains_key("thinking-medium"));
        assert!(modes.contains_key("thinking-high"));
    }

    #[test]
    fn bedrock_claude_47_modes_use_adaptive_thinking_only() {
        let modes = default_model_mode_registry().thinking_modes_for_family(
            CapabilityFamily::Bedrock,
            None,
            "anthropic.claude-opus-4-7",
        );
        assert!(modes.contains_key("thinking-low"));
        assert!(modes.contains_key("thinking-medium"));
        assert!(modes.contains_key("thinking-high"));
        assert!(modes.contains_key("thinking-xhigh"));
        assert!(modes.contains_key("thinking-max"));
        assert_eq!(
            modes
                .get("thinking-low")
                .and_then(|mode| mode.thinking.as_ref()),
            Some(&ThinkingRequest::Adaptive {
                effort: Some(ReasoningEffort::Low),
            })
        );
    }
}
