//! Adapters from a `Plugin` impl to a transport entrypoint.
//!
//! A plugin author implements `Plugin` and then uses ONE of the `export_*!`
//! macros to ship it as a particular transport.

pub mod dispatch;

#[cfg(feature = "cdylib")]
pub mod cdylib;

#[cfg(feature = "stdio")]
pub mod stdio;

#[cfg(feature = "http")]
pub mod http;
