mod settings;
mod utils;

pub use settings::{config_settings_get, config_settings_put};

// Internal helper for SSE snapshots / structured responses.
pub(crate) use settings::format_settings_response;
