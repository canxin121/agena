//! Runtime composition facade for configuration and provider construction.
//!
//! Pure configuration values live in `agena-runtime-config`; provider adapter
//! construction lives in `agena-runtime-provider`. This module owns the
//! parent-runtime hook that lets configured plugins amend the provider list.

pub use agena_runtime_config::*;
pub use agena_runtime_provider::ProviderRegistry;
pub use agena_runtime_provider::config_support::*;

pub use agena_provider::ModelCatalogSnapshot;
pub use agena_runtime_provider::config_support::registry as provider_registry;

pub mod registry;
pub use registry::build_provider_registry_from_inputs;
