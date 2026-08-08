//! Post-processing of parsed tool input values.

use std::collections::BTreeSet;

use quote::quote;
use syn::punctuated::Punctuated;
use syn::{Attribute, Data, Expr, Fields, Ident, LitStr, Result, Token, Variant};

use super::input_schema_path_support::escape_json_pointer_segment;
use super::{
    PluginInputFieldAliasSpec, PluginInputFieldDefaultSpec, PluginInputFieldMetadata,
    dispatch_variant_pattern_and_args, input_variant_action_name, normalized_input_variant_config,
    parse_input_field_arg_attrs, prepare_input_field_names, serde_rename_all_rule,
};

pub fn expand_input_example_expr(
    explicit_example: Option<&Expr>,
    metadata: &[PluginInputFieldMetadata],
) -> proc_macro2::TokenStream {
    if let Some(example) = explicit_example {
        return quote! { Some(::agena_plugin_sdk::serde_json::json!(#example)) };
    }
    let entries = metadata
        .iter()
        .filter_map(|field| {
            let example = field.example.as_ref()?;
            let path = &field.path;
            Some(quote! {
                __object.insert(
                    #path.to_string(),
                    ::agena_plugin_sdk::serde_json::json!(#example),
                );
            })
        })
        .collect::<Vec<_>>();
    if entries.is_empty() {
        quote! { ::agena_plugin_sdk::macro_support::example_value_from_schema(&Self::input_schema()) }
    } else {
        quote! {{
            let mut __object = ::agena_plugin_sdk::serde_json::Map::new();
            #(#entries)*
            Some(::agena_plugin_sdk::serde_json::Value::Object(__object))
        }}
    }
}

pub fn expand_input_root_example_schema_metadata_tokens(
    example: Option<&Expr>,
) -> proc_macro2::TokenStream {
    let Some(example) = example else {
        return quote! {};
    };
    quote! {
        ::agena_plugin_sdk::macro_support::set_schema_value_list_metadata(
            schema,
            "",
            "examples",
            &[::agena_plugin_sdk::serde_json::json!(#example)],
        );
    }
}

pub fn input_error_path_mappings(
    attrs: &[Attribute],
    data: &Data,
) -> Result<Vec<(LitStr, LitStr)>> {
    let Data::Struct(data_struct) = data else {
        return Ok(Vec::new());
    };
    let Fields::Named(fields) = &data_struct.fields else {
        return Ok(Vec::new());
    };
    let rename_rule = serde_rename_all_rule(attrs)?;
    field_error_path_mappings(&Fields::Named(fields.clone()), rename_rule)
}

pub fn expand_input_shape_enum_parse_error_remap_expr(
    variants: &Punctuated<Variant, Token![,]>,
    enum_field_rule: Option<super::SerdeRenameRule>,
) -> Result<proc_macro2::TokenStream> {
    let mut arms = Vec::new();
    for variant in variants {
        let mappings = field_error_path_mappings(
            &variant.fields,
            serde_rename_all_rule(&variant.attrs)?.or(enum_field_rule),
        )?;
        if mappings.is_empty() {
            continue;
        }
        let config = normalized_input_variant_config(variant, enum_field_rule)?;
        let action = input_variant_action_name(variant, &config);
        let mapping_tokens = mappings
            .iter()
            .map(|(from, to)| quote! { (#from, #to) })
            .collect::<Vec<_>>();
        arms.push(quote! {
            Some(#action) => {
                ::agena_plugin_sdk::macro_support::remap_invalid_params_paths(
                    __macro_parse_result,
                    &[#(#mapping_tokens),*],
                )
            }
        });
    }
    if arms.is_empty() {
        Ok(quote! { __macro_parse_result })
    } else {
        Ok(quote! {
            match &input {
                ::agena_plugin_sdk::serde_json::Value::Object(object) => {
                    match object
                        .get("action")
                        .and_then(::agena_plugin_sdk::serde_json::Value::as_str)
                    {
                        #(#arms,)*
                        _ => __macro_parse_result,
                    }
                }
                _ => __macro_parse_result,
            }
        })
    }
}

pub fn expand_input_shape_enum_validate_error_remap_expr(
    variants: &Punctuated<Variant, Token![,]>,
    enum_field_rule: Option<super::SerdeRenameRule>,
) -> Result<proc_macro2::TokenStream> {
    let mut arms = Vec::new();
    for variant in variants {
        let mappings = field_error_path_mappings(
            &variant.fields,
            serde_rename_all_rule(&variant.attrs)?.or(enum_field_rule),
        )?;
        if mappings.is_empty() {
            continue;
        }
        let (_, ignore_pattern, _) = dispatch_variant_pattern_and_args(variant, false)?;
        let mapping_tokens = mappings
            .iter()
            .map(|(from, to)| quote! { (#from, #to) })
            .collect::<Vec<_>>();
        arms.push(quote! {
            #ignore_pattern => {
                ::agena_plugin_sdk::macro_support::remap_invalid_params_paths(
                    __macro_validate_result,
                    &[#(#mapping_tokens),*],
                )
            }
        });
    }
    if arms.is_empty() {
        Ok(quote! { __macro_validate_result })
    } else {
        Ok(quote! {
            match &parsed {
                #(#arms,)*
                _ => __macro_validate_result,
            }
        })
    }
}

pub fn expand_result_path_remap_expr(
    result_ident: &Ident,
    mappings: &[(LitStr, LitStr)],
) -> proc_macro2::TokenStream {
    if mappings.is_empty() {
        return quote! { #result_ident? };
    }
    let from = mappings.iter().map(|(from, _)| from).collect::<Vec<_>>();
    let to = mappings.iter().map(|(_, to)| to).collect::<Vec<_>>();
    quote! {
        ::agena_plugin_sdk::macro_support::remap_invalid_params_paths(
            #result_ident,
            &[#((#from, #to)),*],
        )?
    }
}

pub fn expand_input_root_default_insert_tokens(
    default: bool,
    default_expr: Option<&Expr>,
) -> proc_macro2::TokenStream {
    if let Some(default_expr) = default_expr {
        return quote! {
            if matches!(input, ::agena_plugin_sdk::serde_json::Value::Null) {
                input = ::agena_plugin_sdk::serde_json::to_value(#default_expr)
                    .map_err(|err| ::agena_plugin_sdk::PluginError::invalid_params(err.to_string()))?;
            }
        };
    }
    if default {
        return quote! {
            if matches!(input, ::agena_plugin_sdk::serde_json::Value::Null) {
                input = ::agena_plugin_sdk::serde_json::to_value(<Self as ::core::default::Default>::default())
                    .map_err(|err| ::agena_plugin_sdk::PluginError::invalid_params(err.to_string()))?;
            }
        };
    }
    quote! {}
}

pub fn expand_input_root_default_schema_metadata_tokens(
    default: bool,
    default_expr: Option<&Expr>,
) -> proc_macro2::TokenStream {
    if let Some(default_expr) = default_expr {
        return quote! {
            if let Ok(__default) = ::agena_plugin_sdk::serde_json::to_value(#default_expr) {
                ::agena_plugin_sdk::macro_support::set_schema_value_metadata(
                    schema,
                    "",
                    "default",
                    __default,
                );
            }
        };
    }
    if default {
        return quote! {
            if let Ok(__default) = ::agena_plugin_sdk::serde_json::to_value(
                <Self as ::core::default::Default>::default(),
            ) {
                ::agena_plugin_sdk::macro_support::set_schema_value_metadata(
                    schema,
                    "",
                    "default",
                    __default,
                );
            }
        };
    }
    quote! {}
}

pub fn expand_input_default_insert_tokens(
    defaults: &[PluginInputFieldDefaultSpec],
) -> proc_macro2::TokenStream {
    if defaults.is_empty() {
        return quote! {};
    }
    let inserts = defaults.iter().map(|default| {
        let path = &default.parse_path;
        let ty = &default.ty;
        let missing_expr = if default.aliases.is_empty() {
            quote! { !object.contains_key::<str>(#path) }
        } else {
            let aliases = &default.aliases;
            quote! {
                !object.contains_key::<str>(#path)
                    && ![#(#aliases),*]
                        .iter()
                        .any(|alias| object.contains_key::<str>(*alias))
            }
        };
        let default_value = if let Some(expr) = default.default_expr.as_ref() {
            quote! { ::agena_plugin_sdk::serde_json::json!(#expr) }
        } else {
            quote! {
                ::agena_plugin_sdk::serde_json::to_value(
                    <#ty as ::core::default::Default>::default(),
                )
                .map_err(|err| ::agena_plugin_sdk::PluginError::invalid_params(err.to_string()))?
            }
        };
        quote! {
            if #missing_expr {
                object.insert(#path.to_string(), #default_value);
            }
        }
    });
    quote! {
        match &mut input {
            ::agena_plugin_sdk::serde_json::Value::Object(object) => {
                #(#inserts)*
            }
            _ => {}
        }
    }
}

pub fn expand_input_alias_normalize_tokens(
    aliases: &[PluginInputFieldAliasSpec],
) -> proc_macro2::TokenStream {
    if aliases.is_empty() {
        return quote! {};
    }
    let moves = aliases.iter().map(|alias_spec| {
        let path = &alias_spec.path;
        let aliases = &alias_spec.aliases;
        quote! {
            if !object.contains_key::<str>(#path) {
                let mut __alias_key = None;
                for __candidate in [#(#aliases),*] {
                    if object.contains_key::<str>(__candidate) {
                        __alias_key = Some(__candidate);
                        break;
                    }
                }
                if let Some(__alias_key) = __alias_key {
                    if let Some(__alias_value) = object.remove::<str>(__alias_key) {
                        object.insert(#path.to_string(), __alias_value);
                    }
                }
            } else {
                for __alias_key in [#(#aliases),*] {
                    object.remove::<str>(__alias_key);
                }
            }
        }
    });
    quote! {
        match &mut input {
            ::agena_plugin_sdk::serde_json::Value::Object(object) => {
                #(#moves)*
            }
            _ => {}
        }
    }
}

pub fn expand_input_default_schema_metadata_tokens(
    defaults: &[PluginInputFieldDefaultSpec],
) -> proc_macro2::TokenStream {
    let calls = input_default_schema_metadata_calls("", defaults);
    quote! { #(#calls)* }
}

pub fn input_default_schema_metadata_calls(
    prefix: &str,
    defaults: &[PluginInputFieldDefaultSpec],
) -> Vec<proc_macro2::TokenStream> {
    defaults
        .iter()
        .map(|default| {
            let pointer = if prefix.is_empty() {
                format!(
                    "/properties/{}",
                    escape_json_pointer_segment(default.schema_path.value().as_str())
                )
            } else {
                format!(
                    "{prefix}/properties/{}",
                    escape_json_pointer_segment(default.schema_path.value().as_str())
                )
            };
            let pointer = LitStr::new(pointer.as_str(), default.schema_path.span());
            let ty = &default.ty;
            if let Some(expr) = default.default_expr.as_ref() {
                quote! {
                    ::agena_plugin_sdk::macro_support::set_schema_value_metadata(
                        schema,
                        #pointer,
                        "default",
                        ::agena_plugin_sdk::serde_json::json!(#expr),
                    );
                }
            } else {
                quote! {
                    if let Ok(__default) = ::agena_plugin_sdk::serde_json::to_value(
                        <#ty as ::core::default::Default>::default(),
                    ) {
                        ::agena_plugin_sdk::macro_support::set_schema_value_metadata(
                            schema,
                            #pointer,
                            "default",
                            __default,
                        );
                    }
                }
            }
        })
        .collect()
}

fn field_error_path_mappings(
    fields: &Fields,
    rename_rule: Option<super::SerdeRenameRule>,
) -> Result<Vec<(LitStr, LitStr)>> {
    let Fields::Named(named) = fields else {
        return Ok(Vec::new());
    };
    let mut seen = BTreeSet::new();
    let mut mappings = Vec::new();
    for field in &named.named {
        let arg_config = parse_input_field_arg_attrs(field)?;
        let Some(names) = prepare_input_field_names(field, rename_rule, &arg_config)? else {
            continue;
        };
        if names.schema_path.value() == names.parse_path.value() {
            continue;
        }
        let key = (names.parse_path.value(), names.schema_path.value());
        if seen.insert(key.clone()) {
            mappings.push((
                LitStr::new(key.0.as_str(), names.parse_path.span()),
                LitStr::new(key.1.as_str(), names.schema_path.span()),
            ));
        }
    }
    Ok(mappings)
}
