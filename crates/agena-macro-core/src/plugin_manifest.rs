//! Plugin manifest generation (metadata, tools, hooks, config).

use quote::quote;
use syn::{Result, Type};

use crate::plugin_hooks::plugin_layer_hooks_expr;
use crate::plugin_impl_config::expr_is_ident;
use crate::plugin_tooling::{expand_plugin_operation_definition, expand_plugin_tool_definition};

use super::{
    PluginHookPlan, PluginImplConfig, PluginOperationPlan, PluginServiceInputPlan,
    PluginServicePlan, PluginServiceTargetPlan, PluginToolPlan, doc_summary, lit_str_from_text,
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
    operations: &[PluginOperationPlan],
    services: &[PluginServicePlan],
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

    let settings_assignment = expand_plugin_layer_settings_assignment(config, self_ty)?;
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
    let activity_kinds_assignment = config
        .activity_kinds
        .as_ref()
        .map(|kinds| quote! { manifest.activity_kinds.extend(#kinds); })
        .unwrap_or_default();
    let plugin_tag_assignments = config
        .plugin_tags
        .iter()
        .map(|tag| quote! { manifest.tags.push(#tag); })
        .collect::<Vec<_>>();
    let service_import_assignments = config
        .service_imports
        .iter()
        .map(|service| quote! { manifest.services.imports.push(#service); })
        .collect::<Vec<_>>();
    let typed_service_assignments = services
        .iter()
        .map(|service| {
            let (service_id, api_version, method_contract) = match &service.target {
                PluginServiceTargetPlan::Inline {
                    service: service_id,
                    api_version,
                    method: method_id,
                } => {
                    let output = &service.output;
                    let method_contract = match &service.input {
                        PluginServiceInputPlan::None => quote! {
                            ::agena_plugin_sdk::PluginServiceMethod::new(
                                #method_id,
                                ::agena_plugin_sdk::SettingsContract::empty_object("No input", ""),
                                ::agena_plugin_sdk::macro_support::settings_contract_for::<#output>(),
                            )
                        },
                        PluginServiceInputPlan::Typed { ty, .. } => quote! {
                            ::agena_plugin_sdk::service_method_for::<#ty, #output>(#method_id)
                        },
                    };
                    (
                        quote! { #service_id },
                        quote! { #api_version },
                        method_contract,
                    )
                }
                PluginServiceTargetPlan::Endpoint { endpoint } => (
                    quote! { <#endpoint as ::agena_plugin_sdk::PluginServiceEndpoint>::SERVICE },
                    quote! { <#endpoint as ::agena_plugin_sdk::PluginServiceEndpoint>::API_VERSION },
                    quote! { <#endpoint as ::agena_plugin_sdk::PluginServiceEndpoint>::method_contract() },
                ),
            };
            quote! {
                {
                    let __agena_service_method = #method_contract;
                    if let Some(__agena_service_export) = manifest
                        .services
                        .exports
                        .iter_mut()
                        .find(|export| export.id == #service_id && export.api_version == #api_version)
                    {
                        __agena_service_export.methods.push(__agena_service_method);
                    } else {
                        manifest.services.exports.push(
                            ::agena_plugin_sdk::PluginServiceExport::new(#service_id, #api_version)
                                .with_method(__agena_service_method),
                        );
                    }
                }
            }
        })
        .collect::<Vec<_>>();
    let tool_definition_assignments = tools
        .iter()
        .map(|binding| {
            let definition = expand_plugin_tool_definition(&binding.input_model)?;
            Ok(quote! { manifest.tools.push(#definition); })
        })
        .collect::<Result<Vec<_>>>()?;
    let operation_definition_assignments = operations
        .iter()
        .map(expand_plugin_operation_definition)
        .collect::<Result<Vec<_>>>()?;

    let build_manifest = quote! {{
            let mut manifest = ::agena_plugin_sdk::PluginManifest::new(#namespace, #name, #version);
            manifest.summary = Some(#summary.to_string());
            manifest.hooks = #hooks_expr;
            #settings_assignment
            #help_assignment
            #skills_assignment
            #activity_kinds_assignment
            #(#plugin_tag_assignments)*
            #(#service_import_assignments)*
            #(#typed_service_assignments)*
            #(#tool_definition_assignments)*
            #(#operation_definition_assignments)*
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

fn expand_plugin_layer_settings_assignment(
    config: &PluginImplConfig,
    self_ty: &Type,
) -> Result<proc_macro2::TokenStream> {
    let Some(ty) = config.settings.as_ref() else {
        if config.settings_store {
            return Ok(quote! {
                manifest.settings = Some(
                    <#self_ty as ::agena_plugin_sdk::plugin::PluginSettingsStoreAccess>::plugin_settings_contract(),
                );
            });
        }
        return Ok(quote! {});
    };
    let contract = if let Some(default) = config.settings_default.as_ref() {
        if expr_is_ident(default, "default") {
            quote! {
            ::agena_plugin_sdk::settings_contract_for_default(
                    <#ty as ::core::default::Default>::default(),
                )
                .expect("typed plugin settings must compile to the constrained settings contract")
            }
        } else {
            quote! {
                ::agena_plugin_sdk::settings_contract_for_default(#default)
                    .expect("typed plugin settings must compile to the constrained settings contract")
            }
        }
    } else {
        quote! {
            ::agena_plugin_sdk::settings_contract_for::<#ty>()
                .expect("typed plugin settings must compile to the constrained settings contract")
        }
    };
    let contract = if let Some(metadata) = config.settings_metadata.as_ref() {
        quote! {
            ::agena_plugin_sdk::decorate_settings_contract(#contract, #metadata)
                .expect("typed plugin settings metadata must reference existing contract paths")
        }
    } else {
        contract
    };
    Ok(quote! {
        manifest.settings = Some(#contract);
    })
}
