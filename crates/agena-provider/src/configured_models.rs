use std::collections::BTreeMap;

use crate::{
    ConfiguredModeDefault, ConfiguredModelModeMap, ConfiguredModelThinkingMode,
    ConfiguredThinkingStrategy,
};
use agena_domain::{ModelThinkingMode, ThinkingRequest};

pub fn apply_configured_modes<'a, Mode, ConfiguredMode: 'a, F>(
    mut modes: BTreeMap<String, Mode>,
    configured_modes: impl Iterator<Item = (&'a String, &'a ConfiguredMode)>,
    apply_to_mode: F,
) -> BTreeMap<String, Mode>
where
    F: Fn(&ConfiguredMode, Option<&Mode>) -> Option<Mode>,
{
    for (name, configured) in configured_modes {
        match apply_to_mode(configured, modes.get(name)) {
            Some(mode) => {
                modes.insert(name.clone(), mode);
            }
            None => {
                modes.remove(name);
            }
        }
    }
    modes
}

pub fn apply_configured_thinking_modes(
    modes: Vec<ModelThinkingMode>,
    configured_modes: &ConfiguredModelModeMap<ConfiguredModelThinkingMode>,
) -> Vec<ModelThinkingMode> {
    let configured_default = configured_modes.default.mode().map(ToOwned::to_owned);
    let mut modes = modes
        .into_iter()
        .filter_map(|mode| {
            let selector = mode.selector().map(|selector| selector.into_owned());
            selector.map(|selector| (selector, mode))
        })
        .collect::<BTreeMap<_, _>>();

    for (selector, configured) in configured_modes.iter() {
        match configured.apply_to_mode(modes.get(selector.as_str())) {
            Some(mut mode) => {
                apply_configured_thinking_payload(selector, configured, &mut mode);
                mode.preset = Some(selector.clone());
                modes.insert(selector.clone(), mode);
            }
            None => {
                modes.remove(selector.as_str());
            }
        }
    }

    if let Some(default_selector) = configured_default {
        for (selector, mode) in &mut modes {
            mode.is_default = selector == &default_selector;
        }
    } else if !matches!(configured_modes.default, ConfiguredModeDefault::Clear) {
        retain_first_default(modes.values_mut());
    }

    modes.into_values().collect()
}

fn retain_first_default<'a, Mode>(modes: impl Iterator<Item = &'a mut Mode>)
where
    Mode: 'a + ModeDefault,
{
    let mut found = false;
    for mode in modes {
        if mode.is_default() {
            if found {
                mode.set_default(false);
            } else {
                found = true;
            }
        }
    }
}

trait ModeDefault {
    fn is_default(&self) -> bool;
    fn set_default(&mut self, is_default: bool);
}

macro_rules! impl_mode_default {
    ($($mode:ty),+ $(,)?) => {
        $(
            impl ModeDefault for $mode {
                fn is_default(&self) -> bool {
                    self.is_default
                }

                fn set_default(&mut self, is_default: bool) {
                    self.is_default = is_default;
                }
            }
        )+
    };
}

impl_mode_default!(ModelThinkingMode);

pub fn configured_thinking_mode_selector(
    name: &str,
    _mode: &ConfiguredModelThinkingMode,
) -> Option<String> {
    let name = name.trim();
    (!name.is_empty()).then(|| name.to_owned())
}

pub fn configured_thinking_mode_to_model(
    name: &str,
    mode: &ConfiguredModelThinkingMode,
) -> ModelThinkingMode {
    let mut model = ModelThinkingMode {
        is_default: mode.is_default.unwrap_or(false),
        preset: Some(name.to_owned()),
        display_name: mode.display_name.clone(),
        description: mode.description.clone(),
        thinking: mode.thinking.clone(),
        request_override: mode.request_override.clone(),
        adapter_overrides: mode.adapter_overrides.clone(),
    };
    apply_configured_thinking_payload(name, mode, &mut model);
    model
}

fn apply_configured_thinking_payload(
    _name: &str,
    configured: &ConfiguredModelThinkingMode,
    mode: &mut ModelThinkingMode,
) {
    if configured.thinking.is_some() {
        return;
    }
    mode.thinking = match configured.strategy {
        None => None,
        Some(ConfiguredThinkingStrategy::Disabled) => Some(ThinkingRequest::Disabled),
        Some(ConfiguredThinkingStrategy::Effort) => configured
            .effort
            .map(|effort| ThinkingRequest::Effort { effort }),
        Some(ConfiguredThinkingStrategy::Budget) => configured
            .budget_tokens
            .map(|budget_tokens| ThinkingRequest::Budget { budget_tokens }),
        Some(ConfiguredThinkingStrategy::Adaptive) => Some(ThinkingRequest::Adaptive {
            effort: configured.effort,
            display: configured.display,
        }),
        Some(ConfiguredThinkingStrategy::RequestOnly) => None,
    };
}

#[cfg(test)]
mod tests {
    use super::apply_configured_thinking_modes;
    use crate::{
        ConfiguredModeDefault, ConfiguredModelDefinition, ConfiguredModelModeMap,
        ConfiguredModelThinkingMode,
    };

    #[test]
    fn named_mode_maps_round_trip() {
        let definition: ConfiguredModelDefinition = serde_json::from_value(serde_json::json!({
            "thinking_modes": {
                "default": "medium",
                "low": { "strategy": "effort", "effort": "low" },
                "medium": { "strategy": "effort", "effort": "medium" },
                "high": { "strategy": "effort", "effort": "high" }
            },
            "speed_modes": {
                "default": "fast", "standard": {}, "fast": {}
            }
        }))
        .unwrap();
        assert_eq!(definition.thinking_modes.default.mode(), Some("medium"));
        assert_eq!(definition.speed_modes.default.mode(), Some("fast"));
        let serialized = serde_json::to_value(definition).expect("definition should serialize");
        assert_eq!(serialized["thinking_modes"]["default"], "medium");
        assert_eq!(serialized["speed_modes"]["default"], "fast");

        assert!(
            serde_json::from_value::<ConfiguredModelDefinition>(serde_json::json!({
                "thinking_modes": [{ "thinking": { "type": "effort", "effort": "high" } }]
            }))
            .is_err()
        );

        let cleared: ConfiguredModelDefinition = serde_json::from_value(serde_json::json!({
            "thinking_modes": {
                "default": null,
                "low": { "strategy": "effort", "effort": "low" }
            }
        }))
        .unwrap();
        assert!(matches!(
            cleared.thinking_modes.default,
            ConfiguredModeDefault::Clear
        ));
        assert_eq!(
            serde_json::to_value(cleared).unwrap()["thinking_modes"]["default"],
            serde_json::Value::Null
        );
    }

    #[test]
    fn named_modes_use_explicit_payload_and_default() {
        let definition: ConfiguredModelDefinition = serde_json::from_value(serde_json::json!({
            "thinking_modes": {
                "default": "high",
                "off": { "strategy": "disabled" },
                "high": { "strategy": "effort", "effort": "high" }
            }
        }))
        .unwrap();
        let modes = apply_configured_thinking_modes(Vec::new(), &definition.thinking_modes);
        assert_eq!(
            modes
                .iter()
                .find(|mode| mode.is_default)
                .unwrap()
                .selector()
                .as_deref(),
            Some("high")
        );
        assert!(
            modes
                .iter()
                .any(|mode| mode.selector().as_deref() == Some("off"))
        );
    }

    #[test]
    fn named_budget_mode_uses_flat_strategy_fields() {
        let definition: ConfiguredModelDefinition = serde_json::from_value(serde_json::json!({
            "thinking_modes": {
                "default": "deep",
                "deep": { "strategy": "budget", "budget_tokens": 16000 }
            }
        }))
        .unwrap();
        let modes = apply_configured_thinking_modes(Vec::new(), &definition.thinking_modes);
        let mode = modes.first().unwrap();
        assert_eq!(mode.selector().as_deref(), Some("deep"));
        assert_eq!(
            mode.thinking,
            Some(agena_domain::ThinkingRequest::Budget {
                budget_tokens: 16000
            })
        );
        assert!(mode.is_default);
    }

    #[test]
    fn runtime_modes_serialize_with_explicit_strategies() {
        let modes: ConfiguredModelModeMap<ConfiguredModelThinkingMode> = vec![
            ConfiguredModelThinkingMode {
                thinking: Some(agena_domain::ThinkingRequest::Disabled),
                ..Default::default()
            },
            ConfiguredModelThinkingMode {
                thinking: Some(agena_domain::ThinkingRequest::Effort {
                    effort: agena_domain::ReasoningEffort::High,
                }),
                ..Default::default()
            },
        ]
        .into();

        let value = serde_json::to_value(modes).unwrap();
        assert_eq!(value["off"]["strategy"], "disabled");
        assert_eq!(value["high"]["strategy"], "effort");
        assert_eq!(value["high"]["effort"], "high");
    }
}
