use sea_orm::FromJsonQueryResult;
use serde::{Deserialize, Serialize};

use crate::agent::PermissionConfig;
use crate::model::ModelRef;

fn normalize_optional_string(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, FromJsonQueryResult)]
#[serde(default)]
pub struct ExecutionSelection {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adapter: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verbosity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parallel_tool_calls: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    #[serde(default, skip_serializing_if = "PermissionConfig::is_empty")]
    pub permission: PermissionConfig,
}

impl ExecutionSelection {
    pub fn default_agent(agent: impl Into<String>) -> Self {
        Self {
            agent: normalize_optional_string(Some(agent.into())),
            ..Self::default()
        }
    }

    pub fn is_empty(&self) -> bool {
        self.provider.is_none()
            && self.adapter.is_none()
            && self.model.is_none()
            && self.thinking_mode.is_none()
            && self.speed_mode.is_none()
            && self.verbosity.is_none()
            && self.parallel_tool_calls.is_none()
            && self.agent.is_none()
            && self.permission.is_empty()
    }

    pub fn normalize(&mut self) {
        self.provider = normalize_optional_string(self.provider.take());
        self.adapter = normalize_optional_string(self.adapter.take());
        self.model = normalize_optional_string(self.model.take());
        self.thinking_mode = normalize_optional_string(self.thinking_mode.take());
        self.speed_mode = normalize_optional_string(self.speed_mode.take());
        self.verbosity = normalize_optional_string(self.verbosity.take());
        self.agent = normalize_optional_string(self.agent.take());
    }

    pub fn model_ref(&self) -> Result<Option<ModelRef>, crate::model::IdentifierError> {
        let Some(provider_id) = self.provider.as_deref() else {
            return Ok(None);
        };
        let Some(model_id) = self.model.as_deref() else {
            return Ok(None);
        };
        let model = match self.adapter.as_deref() {
            Some(adapter_id) => ModelRef::try_new_with_adapter(provider_id, adapter_id, model_id)?,
            None => ModelRef::try_new(provider_id, model_id)?,
        };
        Ok(Some(model))
    }

    pub fn set_model_override(
        &mut self,
        provider: Option<String>,
        adapter: Option<String>,
        model: Option<String>,
    ) {
        self.provider = normalize_optional_string(provider);
        self.adapter = normalize_optional_string(adapter);
        self.model = normalize_optional_string(model);
        self.thinking_mode = None;
        self.speed_mode = None;
        self.verbosity = None;
        self.parallel_tool_calls = None;
    }

    pub fn set_model_mode_overrides(
        &mut self,
        thinking_mode: Option<String>,
        speed_mode: Option<String>,
        verbosity: Option<String>,
        parallel_tool_calls: Option<bool>,
    ) {
        self.thinking_mode = normalize_optional_string(thinking_mode);
        self.speed_mode = normalize_optional_string(speed_mode);
        self.verbosity = normalize_optional_string(verbosity);
        self.parallel_tool_calls = parallel_tool_calls;
    }

    pub fn overlay_with_cascade(&self, overlay: &Self) -> Self {
        let mut effective = self.clone();

        if overlay.provider.is_some() {
            effective.provider = overlay.provider.clone();
            effective.adapter = overlay.adapter.clone();
            effective.model = overlay.model.clone();
            effective.thinking_mode = overlay.thinking_mode.clone();
            effective.speed_mode = overlay.speed_mode.clone();
            effective.verbosity = overlay.verbosity.clone();
            effective.parallel_tool_calls = overlay.parallel_tool_calls;
        } else if overlay.adapter.is_some() {
            effective.adapter = overlay.adapter.clone();
            effective.model = overlay.model.clone();
            effective.thinking_mode = overlay.thinking_mode.clone();
            effective.speed_mode = overlay.speed_mode.clone();
            effective.verbosity = overlay.verbosity.clone();
            effective.parallel_tool_calls = overlay.parallel_tool_calls;
        } else if overlay.model.is_some() {
            effective.model = overlay.model.clone();
            effective.thinking_mode = overlay.thinking_mode.clone();
            effective.speed_mode = overlay.speed_mode.clone();
            effective.verbosity = overlay.verbosity.clone();
            effective.parallel_tool_calls = overlay.parallel_tool_calls;
        } else {
            if overlay.thinking_mode.is_some() {
                effective.thinking_mode = overlay.thinking_mode.clone();
            }
            if overlay.speed_mode.is_some() {
                effective.speed_mode = overlay.speed_mode.clone();
            }
            if overlay.verbosity.is_some() {
                effective.verbosity = overlay.verbosity.clone();
            }
            if overlay.parallel_tool_calls.is_some() {
                effective.parallel_tool_calls = overlay.parallel_tool_calls;
            }
        }

        if overlay.agent.is_some() {
            effective.agent = overlay.agent.clone();
        }
        effective.permission.merge_from(overlay.permission.clone());
        effective
    }
}
