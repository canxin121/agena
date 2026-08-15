//! Transport boundary and presentation adapters used by the TUI.
//!
//! After the R7 refactor, the TUI app holds [`agena_application::Application`]
//! directly. The methods in this module are the former `Backend` methods that
//! do **not** carry shared runtime logic (those down-moved onto
//! `impl Application`); they
//! translate runtime/API resources into UI-friendly presentation values and
//! are implemented as free functions taking `&Application`.
//!
//! Hot paths that the UI calls every frame (workspace root, plugin display
//! contributions, model display names, permission tool catalog, theme
//! palettes, file search) stay synchronous — never `async` — so they can run
//! directly inside synchronous TUI handlers.

pub(crate) mod activities;
pub(crate) mod aws;
pub(crate) mod config;
pub(crate) mod file_index;
pub(crate) mod inspector;
pub(crate) mod live_events;
pub(crate) mod operations;
pub(crate) mod permission_catalog;
pub(crate) mod permission_studio;
pub(crate) mod plugin_effects;
pub(crate) mod provider_mappings;
pub(crate) mod session_refresh;
pub(crate) mod timeline;
mod transport;

pub use self::transport::{BackendMode, TuiBackend};

pub(crate) use self::inspector::InspectorRow;
pub(crate) use self::live_events::LiveEvent;
pub(crate) use self::permission_studio::SessionPermissionStudioState;
pub(crate) use self::plugin_effects::PluginCommandEffect;
pub(crate) use self::session_refresh::SessionRefresh;
pub(crate) use self::timeline::SessionTimelineEntry;
