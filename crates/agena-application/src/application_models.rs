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
                Some(adapter_id) => agena_domain::ModelRef::try_new_with_adapter(
                    &model.provider_id,
                    adapter_id,
                    &model.model_id,
                ),
                None => agena_domain::ModelRef::try_new(&model.provider_id, &model.model_id),
            }
            .map_err(|error| {
                ApplicationError::internal(format!(
                    "run option contains an invalid model reference: {error}"
                ))
            });
        }

        Err(ApplicationError::bad_request(
            "model is required; select a model before running",
        ))
    }

    /// Resolve the effective think/speed mode selectors for the model implied
    /// by `request`, matching what a fresh session would apply. When the
    /// request does not pin a model, the request cannot be resolved without a
    /// session-level model selection; otherwise the resolved model's own
    /// defaults are used.
    /// Thinking falls back to the first listed mode for compatibility with
    /// catalogs that omit a thinking default. Speed only uses an explicitly
    /// marked default; `None` means the provider/model native speed default
    /// and therefore no speed override. Returns `(None, None)` when the model
    /// cannot be resolved or exposes no applicable modes.
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
        (
            default_thinking_mode_selector(&options.thinking_modes),
            default_speed_mode_name(&options.speed_modes),
        )
    }
}
