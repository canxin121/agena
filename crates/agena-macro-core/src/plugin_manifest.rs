//! Plugin manifest generation (metadata, tools, hooks, config).

use quote::quote;
use syn::{Result, Type};

use crate::plugin_hooks::plugin_layer_hooks_expr;
use crate::plugin_impl_config::expr_is_ident;
use crate::plugin_tooling::{expand_plugin_command_definition, expand_plugin_tool_definition};

use super::{
    PluginCommandPlan, PluginHookPlan, PluginImplConfig, PluginToolPlan, doc_summary,
    lit_str_from_text,
};

pub fn expand_plugin_layer_export(
    config: &PluginImplConfig,
    self_ty: &Type,
    generics: &syn::Generics,
) -> Result<proc_macro2::TokenStream> {
    let Some(export) = config.export.as_ref() else {
        return Ok(quote! {});
    };
    match export.to_string().as_str() {
        "cdylib" => {
            if !generics.params.is_empty() {
                return Err(syn::Error::new_spanned(
                    self_ty,
                    "export = cdylib does not support generic plugin types",
                ));
            }
            Ok(quote! {
                ::agena_plugin_sdk::export_cdylib!(#self_ty);
            })
        }
        "stdio" => {
            if !generics.params.is_empty() {
                return Err(syn::Error::new_spanned(
                    self_ty,
                    "export = stdio does not support generic plugin types",
                ));
            }
            Ok(quote! {
                ::agena_plugin_sdk::export_stdio!(<#self_ty as ::core::default::Default>::default());
            })
        }
        "http" => {
            if !generics.params.is_empty() {
                return Err(syn::Error::new_spanned(
                    self_ty,
                    "export = http does not support generic plugin types",
                ));
            }
            let bind = config.export_bind.as_ref().ok_or_else(|| {
                syn::Error::new_spanned(export, "export = http requires `bind = ...`")
            })?;
            Ok(quote! {
                ::agena_plugin_sdk::export_http!(<#self_ty as ::core::default::Default>::default(), #bind);
            })
        }
        other => Err(syn::Error::new_spanned(
            export,
            format!("unsupported plugin export '{other}'; expected cdylib, stdio, or http"),
        )),
    }
}

pub fn expand_plugin_layer_manifest(
    config: &PluginImplConfig,
    self_ty: &Type,
    cacheable: bool,
    docs: Option<&str>,
    tools: &[PluginToolPlan],
    hooks: &[PluginHookPlan],
    commands: &[PluginCommandPlan],
) -> Result<proc_macro2::TokenStream> {
    let namespace = config
        .namespace
        .as_ref()
        .expect("plugin namespace validated");
    let name = config.name.as_ref().expect("plugin name validated");
    let version = config.version.as_ref().expect("plugin version validated");
    let summary = if let Some(summary) = config.summary.as_ref() {
        quote! { #summary }
    } else if let Some(summary) = lit_str_from_text(doc_summary(docs).as_deref()) {
        quote! { #summary }
    } else {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            "#[agena_plugin(...)] requires `summary = ...` or doc comments on the impl block",
        ));
    };
    let hooks_expr = plugin_layer_hooks_expr(tools, hooks);

    let config_schema_assignment = expand_plugin_layer_config_schema_assignment(
        config.config_schema_type.as_ref(),
        config,
        self_ty,
    )?;
    let config_schema_value_assignment = config
        .config_schema
        .as_ref()
        .map(|schema| quote! { manifest.config_schema = Some(#schema); })
        .unwrap_or_default();
    let display_assignment = config
        .display
        .as_ref()
        .map(|display| match display.to_string().as_str() {
            "brief" | "compact" => {
                quote! { manifest.set_display(::agena_plugin_sdk::manifest::ToolDisplayPreset::Compact); }
            }
            "brief_detailed" => {
                quote! { manifest.set_display(::agena_plugin_sdk::manifest::ToolDisplayPreset::BriefDetailed); }
            }
            "detailed" => {
                quote! { manifest.set_display(::agena_plugin_sdk::manifest::ToolDisplayPreset::Detailed); }
            }
            _ => quote! { compile_error!("unsupported plugin display mode"); },
        })
        .unwrap_or_default();
    let ui_display_assignment = config
        .ui_display
        .as_ref()
        .map(|display| match display.to_string().as_str() {
            "brief" | "summary" => {
                quote! { manifest.ui_display_mode = Some(::agena_plugin_sdk::UiTextDisplayMode::Summary); }
            }
            "detailed" => {
                quote! { manifest.ui_display_mode = Some(::agena_plugin_sdk::UiTextDisplayMode::Detailed); }
            }
            _ => quote! { compile_error!("unsupported plugin UI display mode"); },
        })
        .unwrap_or_default();
    let help_assignment = if let Some(help) = config.help.as_ref() {
        quote! { manifest.help = Some(#help.to_string()); }
    } else if let Some(help) = lit_str_from_text(docs) {
        quote! { manifest.help = Some(#help.to_string()); }
    } else {
        quote! {}
    };
    let skills_assignment = config
        .skills
        .as_ref()
        .map(|skills| quote! { manifest.skills.extend(#skills); })
        .unwrap_or_default();
    let tool_description_mode_assignment = config
        .tool_description_mode
        .as_ref()
        .map(|mode| quote! { manifest.tool_description_mode = Some(#mode); })
        .unwrap_or_default();
    let ui_display_mode_assignment = config
        .ui_display_mode
        .as_ref()
        .map(|mode| quote! { manifest.ui_display_mode = Some(#mode); })
        .unwrap_or_default();
    let plugin_tag_assignments = config
        .plugin_tags
        .iter()
        .map(|tag| quote! { manifest.tags.push(#tag); })
        .collect::<Vec<_>>();
    let tool_definition_assignments = tools
        .iter()
        .map(|binding| {
            let definition = expand_plugin_tool_definition(&binding.input_model)?;
            Ok(quote! { manifest.tools.push(#definition); })
        })
        .collect::<Result<Vec<_>>>()?;
    let command_definition_assignments = commands
        .iter()
        .map(expand_plugin_command_definition)
        .collect::<Result<Vec<_>>>()?;

    let build_manifest = quote! {{
            let mut manifest = ::agena_plugin_sdk::PluginManifest::new(#namespace, #name, #version);
            manifest.summary = Some(#summary.to_string());
            manifest.hooks = #hooks_expr;
            manifest.config_schema = Some(::agena_plugin_sdk::macro_support::empty_config_schema());
            #config_schema_assignment
            #config_schema_value_assignment
            #display_assignment
            #ui_display_assignment
            #help_assignment
            #skills_assignment
            #tool_description_mode_assignment
            #ui_display_mode_assignment
            #(#plugin_tag_assignments)*
            #(#tool_definition_assignments)*
            #(#command_definition_assignments)*
            manifest
    }};
    let body = if cacheable {
        quote! {
            static __AGENA_PLUGIN_MANIFEST: ::std::sync::OnceLock<::agena_plugin_sdk::PluginManifest> =
                ::std::sync::OnceLock::new();
            __AGENA_PLUGIN_MANIFEST.get_or_init(|| { #build_manifest }).clone()
        }
    } else {
        build_manifest
    };

    Ok(quote! {
        fn manifest(&self) -> ::agena_plugin_sdk::PluginManifest {
            #body
        }
    })
}

fn expand_plugin_layer_config_schema_assignment(
    config_schema_type: Option<&Type>,
    config: &PluginImplConfig,
    self_ty: &Type,
) -> Result<proc_macro2::TokenStream> {
    let Some(ty) = config_schema_type else {
        if config.config_schema_store {
            return Ok(quote! {
                manifest.config_schema = Some(
                    <#self_ty as ::agena_plugin_sdk::plugin::PluginConfigStoreAccess>::plugin_config_schema(),
                );
            });
        }
        return Ok(quote! {});
    };
    let Some(default) = config.config_schema_default.as_ref() else {
        return Ok(quote! {
            manifest.config_schema = Some(::agena_plugin_sdk::macro_support::json_schema_for::<#ty>());
        });
    };
    if expr_is_ident(default, "default") {
        Ok(quote! {
            manifest.config_schema = Some(
                ::agena_plugin_sdk::macro_support::json_schema_for_default(
                    <#ty as ::core::default::Default>::default(),
                ),
            );
        })
    } else {
        Ok(quote! {
            manifest.config_schema = Some(
                ::agena_plugin_sdk::macro_support::json_schema_for_default(#default),
            );
        })
    }
}
