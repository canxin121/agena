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

    let mut modes = modes.into_values().collect::<Vec<_>>();
    modes.sort_by(agena_domain::compare_thinking_mode_strength);
    modes
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
