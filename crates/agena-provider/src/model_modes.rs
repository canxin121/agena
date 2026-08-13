use agena_domain::*;

use std::{collections::BTreeMap, sync::LazyLock};

use regex::Regex;

use agena_domain::{AdapterId, ModelThinkingMode};
use agena_domain::{
    ModelSpeedModeRequestOverride, ReasoningEffort, ThinkingDisplay, ThinkingRequest,
};

use crate::{CapabilityFamily, ModelModeResolver};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
/// Registry of known model modes.
pub struct ModelModeRegistry;

impl ModelModeRegistry {
    pub fn thinking_modes_for_family(
        &self,
        family: CapabilityFamily,
        adapter_id: Option<&AdapterId>,
        model: &str,
        metadata: &ModelMetadata,
    ) -> Vec<ModelThinkingMode> {
        let normalized = normalize_model(model);
        if normalized.is_empty() {
            return Vec::new();
        }

        let protocol = protocol_for_model(family, adapter_id, normalized.as_str());
        match protocol {
            ThinkingProtocol::None => Vec::new(),
            ThinkingProtocol::OpenAi => openai_reasoning_modes(normalized.as_str(), metadata),
            ThinkingProtocol::OpenAiCompatible => {
                openai_compatible_reasoning_modes(normalized.as_str())
            }
            ThinkingProtocol::Anthropic => anthropic_reasoning_modes(normalized.as_str()),
            ThinkingProtocol::Gemini => gemini_reasoning_modes(normalized.as_str()),
            ThinkingProtocol::Bedrock => bedrock_reasoning_modes(normalized.as_str()),
        }
    }
}

impl ModelModeResolver for ModelModeRegistry {
    fn thinking_modes_for_family(
        &self,
        family: CapabilityFamily,
        adapter_id: Option<&AdapterId>,
        model: &str,
        metadata: &ModelMetadata,
    ) -> Vec<ModelThinkingMode> {
        self.thinking_modes_for_family(family, adapter_id, model, metadata)
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
    match adapter_id.map(AsRef::<str>::as_ref) {
        Some("anthropic") => return ThinkingProtocol::Anthropic,
        Some("gemini") => return ThinkingProtocol::Gemini,
        Some("openai_responses" | "openai_chat_completions" | "openai_realtime") => {
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

fn openai_reasoning_modes(model: &str, metadata: &ModelMetadata) -> Vec<ModelThinkingMode> {
    if model.contains("deep-research") {
        return openai_reasoning_mode_overrides(effort_modes(&[ReasoningEffort::Medium], false));
    }
    if let Some(modes) = gpt5_chat_reasoning_modes(model) {
        return openai_reasoning_mode_overrides(modes);
    }
    if GPT5_PRO_RE.is_match(model) {
        return openai_reasoning_mode_overrides(effort_modes(&[ReasoningEffort::High], false));
    }
    if let Some(modes) = gpt5_codex_reasoning_modes(model, false) {
        return openai_reasoning_mode_overrides(modes);
    }
    if let Some(modes) = versioned_gpt5_reasoning_modes(model, false) {
        return openai_reasoning_mode_overrides(modes);
    }

    if GPT5_FAMILY_RE.is_match(model) {
        let mut efforts = vec![
            ReasoningEffort::Minimal,
            ReasoningEffort::Low,
            ReasoningEffort::Medium,
            ReasoningEffort::High,
        ];
        if openai_release_date_supports_xhigh(metadata) {
            efforts.push(ReasoningEffort::Xhigh);
        }
        return openai_reasoning_mode_overrides(effort_modes(
            efforts.as_slice(),
            openai_release_date_supports_none(metadata),
        ));
    }

    if model.starts_with("o1") || model.starts_with("o3") || model.starts_with("o4") {
        let mut efforts = vec![
            ReasoningEffort::Low,
            ReasoningEffort::Medium,
            ReasoningEffort::High,
        ];
        if openai_release_date_supports_xhigh(metadata) {
            efforts.push(ReasoningEffort::Xhigh);
        }
        return openai_reasoning_mode_overrides(effort_modes(
            efforts.as_slice(),
            openai_release_date_supports_none(metadata),
        ));
    }

    if model.contains("deepseek-v4") {
        return openai_reasoning_mode_overrides(effort_modes(
            &[
                ReasoningEffort::Low,
                ReasoningEffort::Medium,
                ReasoningEffort::High,
                ReasoningEffort::Max,
            ],
            false,
        ));
    }

    Vec::new()
}

fn openai_compatible_reasoning_modes(model: &str) -> Vec<ModelThinkingMode> {
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
    Vec::new()
}

fn anthropic_reasoning_modes(model: &str) -> Vec<ModelThinkingMode> {
    if !model.contains("claude") {
        return Vec::new();
    }
    if model.contains("opus-4-7")
        || model.contains("opus-4.7")
        || model.contains("opus-4-8")
        || model.contains("opus-4.8")
        || model.contains("sonnet-5")
        || model.contains("fable-5")
        || model.contains("mythos-5")
    {
        return adaptive_modes_with_display(
            &[
                ReasoningEffort::Low,
                ReasoningEffort::Medium,
                ReasoningEffort::High,
                ReasoningEffort::Xhigh,
                ReasoningEffort::Max,
            ],
            Some(ThinkingDisplay::Summarized),
        );
    }
    if model.contains("mythos-preview")
        || model.contains("opus-4-6")
        || model.contains("opus-4.6")
        || model.contains("sonnet-4-6")
        || model.contains("sonnet-4.6")
    {
        return adaptive_modes(&[
            ReasoningEffort::Low,
            ReasoningEffort::Medium,
            ReasoningEffort::High,
            ReasoningEffort::Max,
        ]);
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

fn gemini_reasoning_modes(model: &str) -> Vec<ModelThinkingMode> {
    if model.contains("gemini-2.5") {
        return effort_modes(
            &[
                ReasoningEffort::Minimal,
                ReasoningEffort::Low,
                ReasoningEffort::Medium,
                ReasoningEffort::High,
                ReasoningEffort::Max,
            ],
            !(model.contains("pro") && !model.contains("flash")),
        );
    }
    if model.contains("gemini-3") {
        if model.contains("flash-lite-image") || model.contains("flash-image") {
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
        if model.contains("gemini-3-pro") && !model.contains("gemini-3.1-pro") {
            return effort_modes(&[ReasoningEffort::Low, ReasoningEffort::High], false);
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
    Vec::new()
}

fn bedrock_reasoning_modes(model: &str) -> Vec<ModelThinkingMode> {
    if model.contains("claude-opus-4-6") || model.contains("claude-opus-4.6") {
        return adaptive_modes(&[
            ReasoningEffort::Low,
            ReasoningEffort::Medium,
            ReasoningEffort::High,
            ReasoningEffort::Max,
        ]);
    }
    if model.contains("claude-opus-4-7")
        || model.contains("claude-opus-4.7")
        || model.contains("claude-fable-5")
        || model.contains("claude-mythos-5")
        || model.contains("claude-mythos-preview")
    {
        return adaptive_modes_with_display(
            &[
                ReasoningEffort::Low,
                ReasoningEffort::Medium,
                ReasoningEffort::High,
            ],
            Some(ThinkingDisplay::Summarized),
        );
    }
    if model.contains("claude-sonnet-4-6") || model.contains("claude-sonnet-4.6") {
        return adaptive_modes(&[
            ReasoningEffort::Low,
            ReasoningEffort::Medium,
            ReasoningEffort::High,
        ]);
    }
    if model.contains("claude-opus-4-5") || model.contains("claude-opus-4.5") {
        return effort_modes(
            &[
                ReasoningEffort::Low,
                ReasoningEffort::Medium,
                ReasoningEffort::High,
            ],
            false,
        );
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
    Vec::new()
}

fn gpt5_chat_reasoning_modes(model: &str) -> Option<Vec<ModelThinkingMode>> {
    if !GPT5_FAMILY_RE.is_match(model) || !model.contains("-chat") {
        return None;
    }
    if gpt5_version(model).is_none() {
        return Some(Vec::new());
    }
    Some(effort_modes(&[ReasoningEffort::Medium], false))
}

fn gpt5_codex_reasoning_modes(model: &str, compatible: bool) -> Option<Vec<ModelThinkingMode>> {
    if !GPT5_FAMILY_RE.is_match(model) || !model.contains("codex") {
        return None;
    }
    let version = gpt5_version(model);
    let efforts = if model.contains("codex-max") || version.is_some_and(|version| version >= 2) {
        let mut efforts = vec![
            ReasoningEffort::Low,
            ReasoningEffort::Medium,
            ReasoningEffort::High,
            ReasoningEffort::Xhigh,
        ];
        if version.is_some_and(|version| version >= 6) {
            efforts.push(ReasoningEffort::Max);
        }
        efforts
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

fn versioned_gpt5_reasoning_modes(model: &str, compatible: bool) -> Option<Vec<ModelThinkingMode>> {
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
    let mut efforts = vec![
        ReasoningEffort::Low,
        ReasoningEffort::Medium,
        ReasoningEffort::High,
        ReasoningEffort::Xhigh,
    ];
    if version >= 6 {
        efforts.push(ReasoningEffort::Max);
    }
    Some(effort_modes(efforts.as_slice(), compatible || version >= 2))
}

fn adaptive_modes(efforts: &[ReasoningEffort]) -> Vec<ModelThinkingMode> {
    adaptive_modes_with_display(efforts, None)
}

fn adaptive_modes_with_display(
    efforts: &[ReasoningEffort],
    display: Option<ThinkingDisplay>,
) -> Vec<ModelThinkingMode> {
    let mut modes = Vec::new();
    for effort in efforts {
        modes.push(ModelThinkingMode {
            is_default: false,
            preset: None,
            display_name: Some(format!("Think {}", title_case(effort.as_ref()))),
            description: None,
            thinking: Some(ThinkingRequest::Adaptive {
                effort: Some(*effort),
                display,
            }),
            request_override: ModelSpeedModeRequestOverride::default(),
            adapter_overrides: BTreeMap::new(),
        });
    }
    modes
}

fn openai_reasoning_mode_overrides(mut modes: Vec<ModelThinkingMode>) -> Vec<ModelThinkingMode> {
    let request_override = openai_reasoning_request_override();
    for mode in &mut modes {
        if matches!(mode.thinking, Some(ThinkingRequest::Disabled) | None) {
            continue;
        }
        mode.request_override = mode.request_override.merged_with(&request_override);
    }
    modes
}

fn openai_reasoning_request_override() -> ModelSpeedModeRequestOverride {
    let mut body_patch = BTreeMap::new();
    body_patch.insert(
        "reasoning".to_owned(),
        serde_json::json!({
            "summary": "auto",
        }),
    );
    body_patch.insert(
        "include".to_owned(),
        serde_json::json!(["reasoning.encrypted_content"]),
    );
    ModelSpeedModeRequestOverride {
        headers: BTreeMap::new(),
        body_patch,
    }
}

fn effort_modes(efforts: &[ReasoningEffort], include_disabled: bool) -> Vec<ModelThinkingMode> {
    let mut modes = Vec::new();
    if include_disabled {
        modes.push(ModelThinkingMode {
            is_default: false,
            preset: None,
            display_name: Some("Off".to_string()),
            description: None,
            thinking: Some(ThinkingRequest::Disabled),
            request_override: ModelSpeedModeRequestOverride::default(),
            adapter_overrides: BTreeMap::new(),
        });
    }
    for effort in efforts {
        modes.push(ModelThinkingMode {
            is_default: false,
            preset: None,
            display_name: Some(format!("Think {}", title_case(effort.as_ref()))),
            description: None,
            thinking: Some(ThinkingRequest::Effort { effort: *effort }),
            request_override: ModelSpeedModeRequestOverride::default(),
            adapter_overrides: BTreeMap::new(),
        });
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

fn openai_release_date_supports_none(metadata: &ModelMetadata) -> bool {
    metadata
        .release_date
        .as_deref()
        .is_some_and(|date| date >= OPENAI_NONE_EFFORT_RELEASE_DATE)
}

fn openai_release_date_supports_xhigh(metadata: &ModelMetadata) -> bool {
    metadata
        .release_date
        .as_deref()
        .is_some_and(|date| date >= OPENAI_XHIGH_EFFORT_RELEASE_DATE)
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
const OPENAI_NONE_EFFORT_RELEASE_DATE: &str = "2025-11-13";
const OPENAI_XHIGH_EFFORT_RELEASE_DATE: &str = "2025-12-04";

#[cfg(test)]
mod tests {
    use super::{ModelMetadata, gemini_reasoning_modes, openai_reasoning_modes};

    fn has_mode(modes: &[agena_domain::ModelThinkingMode], selector: &str) -> bool {
        modes
            .iter()
            .any(|mode| mode.selector().as_deref() == Some(selector))
    }

    #[test]
    fn deepseek_v4_exposes_effort_ladder_for_openai_protocol() {
        let modes = openai_reasoning_modes("deepseek-v4-pro", &ModelMetadata::default());
        assert!(has_mode(&modes, "low"));
        assert!(has_mode(&modes, "medium"));
        assert!(has_mode(&modes, "high"));
        assert!(has_mode(&modes, "max"));

        let flash = openai_reasoning_modes("deepseek-v4-flash", &ModelMetadata::default());
        assert!(has_mode(&flash, "max"));
    }

    #[test]
    fn gpt_5_6_exposes_max_as_a_reasoning_effort_not_an_orchestration_mode() {
        let modes = openai_reasoning_modes("gpt-5.6", &ModelMetadata::default());
        assert!(has_mode(&modes, "max"));
        assert!(!has_mode(&modes, "ultra"));

        let codex_modes = openai_reasoning_modes("gpt-5.6-codex", &ModelMetadata::default());
        assert!(has_mode(&codex_modes, "max"));
        assert!(!has_mode(&codex_modes, "ultra"));
    }

    #[test]
    fn gemini_modes_match_generate_content_model_restrictions() {
        let pro_25 = gemini_reasoning_modes("gemini-2.5-pro");
        assert!(!has_mode(&pro_25, "off"));
        assert!(has_mode(&pro_25, "minimal"));
        assert!(has_mode(&pro_25, "max"));

        let flash_25 = gemini_reasoning_modes("gemini-2.5-flash");
        assert!(has_mode(&flash_25, "off"));
        assert!(has_mode(&flash_25, "medium"));

        let pro_30 = gemini_reasoning_modes("gemini-3-pro-preview");
        assert!(has_mode(&pro_30, "low"));
        assert!(!has_mode(&pro_30, "medium"));
        assert!(has_mode(&pro_30, "high"));

        let pro_31 = gemini_reasoning_modes("gemini-3.1-pro-preview");
        assert!(has_mode(&pro_31, "medium"));

        let flash_lite_image = gemini_reasoning_modes("gemini-3.1-flash-lite-image-preview");
        assert!(has_mode(&flash_lite_image, "minimal"));
        assert!(!has_mode(&flash_lite_image, "low"));
        assert!(!has_mode(&flash_lite_image, "medium"));
        assert!(has_mode(&flash_lite_image, "high"));
    }
}
