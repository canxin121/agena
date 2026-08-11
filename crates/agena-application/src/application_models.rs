//! Model resolution for run options, migrated from
//! `agena-tui-backend/src/backend_provider/selection.rs`
//! (`resolved_model_for_run_options`, `resolved_model_default_modes`).

use crate::provider_studio::save::{default_speed_mode_name, default_thinking_mode_selector};
use crate::{Application, ApplicationError};

impl Application {
    pub fn resolved_model_for_run_options(
        &self,
        request: &agena_api::resource::RunOptions,
    ) -> Result<agena_domain::ModelRef, ApplicationError> {
        if let Some(model) = request.model.as_ref() {
            return match model.adapter_id.as_deref() {
                Some(adapter_id) => {
                    agena_domain::ModelRef::try_new_with_adapter(
                        &model.provider_id,
                        adapter_id,
                        &model.model_id,
                    )
                }
                None => agena_domain::ModelRef::try_new(&model.provider_id, &model.model_id),
            }
            .map_err(|error| {
                ApplicationError::internal(format!(
                    "run option contains an invalid model reference: {error}"
                ))
            });
        }

        self.provider_catalog()
            .default_model()
            .map_err(|error| ApplicationError::internal(error.to_string()))?
            .ok_or_else(|| ApplicationError::internal("no providers configured"))
    }

    /// Resolve the effective think/speed mode selectors for the model implied
    /// by `request`, matching what a fresh session would apply. When the
    /// request does not pin a model, the configured default selection's
    /// thinking/speed modes take precedence (the runtime applies them to new
    /// sessions); otherwise the resolved model's own default (marked default,
    /// then first listed mode) is used. Returns `(None, None)` when the model
    /// cannot be resolved or exposes no modes.
    pub fn resolved_model_default_modes(
        &self,
        request: &agena_api::resource::RunOptions,
    ) -> (Option<String>, Option<String>) {
        let Ok(model) = self.resolved_model_for_run_options(request) else {
            return (None, None);
        };
        let Ok(options) = self.provider_catalog().model_execution_options(&model) else {
            return (None, None);
        };
        let selection = self.provider_catalog().default_selection();
        let configured_thinking = selection
            .thinking_mode
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);
        let configured_speed = selection
            .speed_mode
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);
        if request.model.is_none() {
            (
                configured_thinking
                    .or_else(|| default_thinking_mode_selector(&options.thinking_modes)),
                configured_speed.or_else(|| default_speed_mode_name(&options.speed_modes)),
            )
        } else {
            (
                default_thinking_mode_selector(&options.thinking_modes),
                default_speed_mode_name(&options.speed_modes),
            )
        }
    }
}
