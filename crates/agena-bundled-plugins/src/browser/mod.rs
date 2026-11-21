//! Managed Chrome runtime and interactive browser tools.

pub(crate) mod driver;
mod plugin;
pub(crate) mod runtime;

pub use plugin::BrowserSessionPool;
pub(crate) use plugin::{BROWSER_PLUGIN_ID, BrowserPlugin};
