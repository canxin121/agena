use std::collections::BTreeMap;

use crate::model::ModelSpeedModeRequestOverride;

pub(crate) trait CatalogModeFields {
    fn display_name(&self) -> &Option<String>;
    fn display_name_mut(&mut self) -> &mut Option<String>;
    fn description(&self) -> &Option<String>;
    fn description_mut(&mut self) -> &mut Option<String>;
    fn request_override(&self) -> &ModelSpeedModeRequestOverride;
    fn request_override_mut(&mut self) -> &mut ModelSpeedModeRequestOverride;
    fn adapter_overrides(&self) -> &BTreeMap<String, ModelSpeedModeRequestOverride>;
    fn adapter_overrides_mut(&mut self) -> &mut BTreeMap<String, ModelSpeedModeRequestOverride>;
}

macro_rules! impl_catalog_mode_fields {
    ($ty:path) => {
        impl $crate::model_catalog::CatalogModeFields for $ty {
            fn display_name(&self) -> &Option<String> {
                &self.display_name
            }

            fn display_name_mut(&mut self) -> &mut Option<String> {
                &mut self.display_name
            }

            fn description(&self) -> &Option<String> {
                &self.description
            }

            fn description_mut(&mut self) -> &mut Option<String> {
                &mut self.description
            }

            fn request_override(&self) -> &$crate::model::ModelSpeedModeRequestOverride {
                &self.request_override
            }

            fn request_override_mut(
                &mut self,
            ) -> &mut $crate::model::ModelSpeedModeRequestOverride {
                &mut self.request_override
            }

            fn adapter_overrides(
                &self,
            ) -> &std::collections::BTreeMap<
                String,
                $crate::model::ModelSpeedModeRequestOverride,
            > {
                &self.adapter_overrides
            }

            fn adapter_overrides_mut(
                &mut self,
            ) -> &mut std::collections::BTreeMap<
                String,
                $crate::model::ModelSpeedModeRequestOverride,
            > {
                &mut self.adapter_overrides
            }
        }
    };
}

pub(crate) use impl_catalog_mode_fields;
