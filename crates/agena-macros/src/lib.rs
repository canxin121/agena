//! # agena-macros
//!
//! Procedural macros for the Agena plugin SDK.
//!
//! This crate is the compiler-facing entry point; all expansion logic lives in
//! [`agena_macro_core`]. It exports:
//!
//! - [`agena_plugin`] — attribute macro that turns a plugin `impl` block into
//!   a complete plugin registration (metadata, tool handlers, hooks, config).
//! - [`ToolInput`] — derive macro that generates the hidden JSON input schema
//!   and dispatch glue for a tool input struct.
//! - [`PluginConfigStore`] — derive macro that aggregates plugin configuration
//!   fields and generates the config schema.
//!
//! Plugin authors normally use these through the `agena-plugin-sdk` prelude
//! rather than depending on this crate directly.

use proc_macro::TokenStream;
use syn::{DeriveInput, ItemImpl, parse_macro_input};

use agena_macro_core::{
    input_expand_support::expand_input, plugin_config_store::expand_plugin_config_store,
    plugin_expand_support::expand_plugin_impl_attr,
};

#[proc_macro_attribute]
pub fn agena_plugin(attr: TokenStream, item: TokenStream) -> TokenStream {
    let attr = parse_macro_input!(attr as proc_macro2::TokenStream);
    let item = parse_macro_input!(item as ItemImpl);
    match expand_plugin_impl_attr(attr, item) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

#[proc_macro_derive(ToolInput, attributes(input, arg))]
pub fn derive_input(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match expand_input(input) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

#[proc_macro_derive(PluginConfigStore, attributes(config, plugin_config))]
pub fn derive_plugin_config_store(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match expand_plugin_config_store(input) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}
