use std::collections::BTreeMap;

use agena_domain::{CapabilitySupport, ReasoningEffort, ThinkingRequest};

use crate::{
    CatalogModelDefinition, ConfiguredModelModeMap, ConfiguredModelThinkingMode,
    ModelCapabilityFeature, ModelCatalogDocument,
};

/// Enriches catalog definitions with the well-known reasoning modes implied by
/// their model IDs and declared reasoning capability.
pub fn enrich_catalog_document_thinking_modes(document: &mut ModelCatalogDocument) {
    for (model_id, definition) in &mut document.models {
        for mode in inferred_catalog_thinking_modes(model_id, definition) {
            let normalized: ConfiguredModelModeMap<_> = vec![mode].into();
            for (selector, mode) in normalized.modes {
                definition.thinking_modes.entry(selector).or_insert(mode);
            }
        }
    }
}

pub fn inferred_catalog_thinking_modes(
    model_id: &str,
    definition: &CatalogModelDefinition,
) -> Vec<ConfiguredModelThinkingMode> {
    let mut modes = Vec::new();
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
        for effort in openai_catalog_reasoning_efforts(normalized.as_str()) {
            insert_catalog_thinking_effort(&mut modes, effort);
        }
        return modes;
    }

    if normalized.contains("gemini-3") {
        insert_catalog_thinking_effort(&mut modes, ReasoningEffort::Low);
        insert_catalog_thinking_effort(&mut modes, ReasoningEffort::High);
        return modes;
    }

    if normalized.contains("gemini-2.5") {
        insert_catalog_thinking_effort(&mut modes, ReasoningEffort::High);
        insert_catalog_thinking_effort(&mut modes, ReasoningEffort::Max);
        return modes;
    }

    if normalized.contains("claude") && definition.thinking_modes.is_empty() {
        insert_catalog_thinking_effort(&mut modes, ReasoningEffort::High);
        insert_catalog_thinking_effort(&mut modes, ReasoningEffort::Max);
    }

    if normalized.contains("deepseek-v4") {
        for effort in [
            ReasoningEffort::Low,
            ReasoningEffort::Medium,
            ReasoningEffort::High,
            ReasoningEffort::Max,
        ] {
            insert_catalog_thinking_effort(&mut modes, effort);
        }
    }

    modes
}

pub fn openai_catalog_reasoning_efforts(model_id: &str) -> Vec<ReasoningEffort> {
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
        if model_id
            .split(['.', '-'])
            .skip_while(|segment| *segment != "5")
            .nth(1)
            .and_then(|segment| segment.parse::<u32>().ok())
            .is_some_and(|version| version >= 6)
        {
            efforts.push(ReasoningEffort::Max);
        }
    }
    efforts
}

pub fn insert_catalog_thinking_effort(
    modes: &mut Vec<ConfiguredModelThinkingMode>,
    effort: ReasoningEffort,
) {
    if !modes.iter().any(|mode| {
        matches!(
            mode.thinking,
            Some(ThinkingRequest::Effort { effort: existing }) if existing == effort
        )
    }) {
        modes.push(catalog_thinking_mode_for_effort(effort));
    }
}

pub fn catalog_thinking_mode_for_effort(effort: ReasoningEffort) -> ConfiguredModelThinkingMode {
    let effort_name = effort.as_ref();
    ConfiguredModelThinkingMode {
        is_default: None,
        preset: None,
        display_name: Some(format!("Think {}", title_case_tokenized(effort_name))),
        description: None,
        thinking: Some(ThinkingRequest::Effort { effort }),
        request_override: Default::default(),
        adapter_overrides: BTreeMap::new(),
        disabled: false,
        strategy: None,
        effort: None,
        budget_tokens: None,
        display: None,
    }
}

fn title_case_tokenized(value: &str) -> String {
    value
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|segment| !segment.is_empty())
        .map(|segment| {
            let mut chars = segment.chars();
            let Some(first) = chars.next() else {
                return String::new();
            };
            let mut titled = first.to_uppercase().collect::<String>();
            titled.push_str(chars.as_str());
            titled
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::enrich_catalog_document_thinking_modes;
    use crate::{
        CapabilitySelectionPatch, CatalogModelDefinition, ModelCapabilityFeature,
        ModelCapabilityPatch, ModelCatalogDocument,
    };
    use std::collections::BTreeMap;

    #[test]
    fn enriches_a_reasoning_deepseek_v4_catalog_entry() {
        let mut document = ModelCatalogDocument {
            models: BTreeMap::from([(
                "deepseek-v4-pro".to_owned(),
                CatalogModelDefinition {
                    capabilities: ModelCapabilityPatch {
                        features: Some(CapabilitySelectionPatch::Supported(vec![
                            ModelCapabilityFeature::Reasoning,
                        ])),
                        ..Default::default()
                    },
                    ..Default::default()
                },
            )]),
        };
        enrich_catalog_document_thinking_modes(&mut document);
        let modes = &document.models["deepseek-v4-pro"].thinking_modes;
        for selector in ["low", "medium", "high", "max"] {
            assert!(modes.contains_key(selector), "missing {selector}");
        }
    }

    #[test]
    fn enriches_a_reasoning_gpt_five_catalog_entry() {
        let mut document = ModelCatalogDocument {
            models: BTreeMap::from([(
                "gpt-5.6".to_owned(),
                CatalogModelDefinition {
                    capabilities: ModelCapabilityPatch {
                        features: Some(CapabilitySelectionPatch::Supported(vec![
                            ModelCapabilityFeature::Reasoning,
                        ])),
                        ..Default::default()
                    },
                    ..Default::default()
                },
            )]),
        };
        enrich_catalog_document_thinking_modes(&mut document);
        assert!(
            document.models["gpt-5.6"]
                .thinking_modes
                .contains_key("max")
        );
    }
}
