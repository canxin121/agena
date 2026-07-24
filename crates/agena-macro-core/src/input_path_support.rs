use std::collections::BTreeSet;

use quote::quote;
use syn::{Attribute, Data, LitStr, Result, Type};

use crate::plugin_tooling::{
    expand_input_network_specs, expand_input_path_specs, expand_input_tags,
};

use super::{
    NestedInputShapeField, PluginInputNetworkSpec, PluginInputPathSpec, flatten_shape_type,
    nested_input_shape_field, serde_rename_all_fields_rule, serde_rename_all_rule,
};

pub fn struct_flatten_shape_types(data: &Data) -> Result<Vec<Type>> {
    let Data::Struct(data_struct) = data else {
        return Ok(Vec::new());
    };
    data_struct
        .fields
        .iter()
        .filter_map(|field| flatten_shape_type(field).transpose())
        .collect()
}

pub fn enum_flatten_shape_types(data: &Data) -> Result<Vec<Type>> {
    let Data::Enum(data_enum) = data else {
        return Ok(Vec::new());
    };
    data_enum
        .variants
        .iter()
        .flat_map(|variant| {
            variant
                .fields
                .iter()
                .filter_map(|field| flatten_shape_type(field).transpose())
        })
        .collect()
}

pub fn struct_nested_shape_fields(
    attrs: &[Attribute],
    data: &Data,
) -> Result<Vec<NestedInputShapeField>> {
    let Data::Struct(data_struct) = data else {
        return Ok(Vec::new());
    };
    let rename_rule = serde_rename_all_rule(attrs)?;
    data_struct
        .fields
        .iter()
        .filter_map(|field| nested_input_shape_field(field, rename_rule).transpose())
        .collect()
}

pub fn enum_nested_shape_fields(
    attrs: &[Attribute],
    data: &Data,
) -> Result<Vec<NestedInputShapeField>> {
    let Data::Enum(data_enum) = data else {
        return Ok(Vec::new());
    };
    let enum_field_rule = serde_rename_all_fields_rule(attrs)?;
    let mut fields = Vec::new();
    for variant in &data_enum.variants {
        let variant_field_rule = serde_rename_all_rule(&variant.attrs)?.or(enum_field_rule);
        for field in &variant.fields {
            if let Some(field) = nested_input_shape_field(field, variant_field_rule)? {
                fields.push(field);
            }
        }
    }
    Ok(fields)
}

pub fn expand_input_paths_expr(
    attrs: &[Attribute],
    data: &Data,
    paths: &[PluginInputPathSpec],
) -> Result<proc_macro2::TokenStream> {
    let own = expand_input_path_specs(paths);
    let struct_flatten_shapes = struct_flatten_shape_types(data)?;
    let enum_flatten_shapes = enum_flatten_shape_types(data)?;
    let struct_nested_shapes = struct_nested_shape_fields(attrs, data)?;
    let enum_nested_shapes = enum_nested_shape_fields(attrs, data)?;
    if struct_flatten_shapes.is_empty()
        && enum_flatten_shapes.is_empty()
        && struct_nested_shapes.is_empty()
        && enum_nested_shapes.is_empty()
    {
        return Ok(own);
    }
    let struct_nested_path_expr = expand_nested_shape_path_specs_expr(&struct_nested_shapes, false);
    let enum_nested_path_expr = expand_nested_shape_path_specs_expr(&enum_nested_shapes, true);
    Ok(quote! {{
        let mut __items = #own;
        #(
            __items.extend(<#struct_flatten_shapes as ::agena_plugin_sdk::ToolInput>::input_paths());
        )*
        #(
            __items.extend(
                <#enum_flatten_shapes as ::agena_plugin_sdk::ToolInput>::input_paths()
                    .into_iter()
                    .map(|mut __spec| {
                        __spec.optional = true;
                        __spec
                    })
            );
        )*
        #struct_nested_path_expr
        #enum_nested_path_expr
        __items
    }})
}

pub fn expand_input_networks_expr(
    attrs: &[Attribute],
    data: &Data,
    networks: &[PluginInputNetworkSpec],
) -> Result<proc_macro2::TokenStream> {
    let own = expand_input_network_specs(networks);
    let struct_flatten_shapes = struct_flatten_shape_types(data)?;
    let enum_flatten_shapes = enum_flatten_shape_types(data)?;
    let struct_nested_shapes = struct_nested_shape_fields(attrs, data)?;
    let enum_nested_shapes = enum_nested_shape_fields(attrs, data)?;
    if struct_flatten_shapes.is_empty()
        && enum_flatten_shapes.is_empty()
        && struct_nested_shapes.is_empty()
        && enum_nested_shapes.is_empty()
    {
        return Ok(own);
    }
    let struct_nested_network_expr =
        expand_nested_shape_network_specs_expr(&struct_nested_shapes, false);
    let enum_nested_network_expr =
        expand_nested_shape_network_specs_expr(&enum_nested_shapes, true);
    Ok(quote! {{
        let mut __items = #own;
        #(
            __items.extend(<#struct_flatten_shapes as ::agena_plugin_sdk::ToolInput>::input_networks());
        )*
        #(
            __items.extend(
                <#enum_flatten_shapes as ::agena_plugin_sdk::ToolInput>::input_networks()
                    .into_iter()
                    .map(|mut __spec| {
                        __spec.optional = true;
                        __spec
                    })
            );
        )*
        #struct_nested_network_expr
        #enum_nested_network_expr
        __items
    }})
}

pub fn expand_input_tags_expr(
    attrs: &[Attribute],
    data: &Data,
    paths: &[PluginInputPathSpec],
    networks: &[PluginInputNetworkSpec],
) -> Result<proc_macro2::TokenStream> {
    let own = expand_input_tags(paths, networks);
    let struct_flatten_shapes = struct_flatten_shape_types(data)?;
    let enum_flatten_shapes = enum_flatten_shape_types(data)?;
    let struct_nested_shapes = struct_nested_shape_fields(attrs, data)?;
    let enum_nested_shapes = enum_nested_shape_fields(attrs, data)?;
    if struct_flatten_shapes.is_empty()
        && enum_flatten_shapes.is_empty()
        && struct_nested_shapes.is_empty()
        && enum_nested_shapes.is_empty()
    {
        return Ok(own);
    }
    let struct_nested_tag_exprs = struct_nested_shapes.iter().map(|field| {
        let ty = &field.spec.inner_ty;
        quote! {
            __items.extend(<#ty as ::agena_plugin_sdk::ToolInput>::input_tags());
        }
    });
    let enum_nested_tag_exprs = enum_nested_shapes.iter().map(|field| {
        let ty = &field.spec.inner_ty;
        quote! {
            __items.extend(<#ty as ::agena_plugin_sdk::ToolInput>::input_tags());
        }
    });
    Ok(quote! {{
        let mut __items = #own;
        #(
            __items.extend(<#struct_flatten_shapes as ::agena_plugin_sdk::ToolInput>::input_tags());
        )*
        #(
            __items.extend(<#enum_flatten_shapes as ::agena_plugin_sdk::ToolInput>::input_tags());
        )*
        #(#struct_nested_tag_exprs)*
        #(#enum_nested_tag_exprs)*
        __items
    }})
}

pub fn expand_nested_shape_schema_normalize_expr(
    nested_shapes: &[NestedInputShapeField],
) -> proc_macro2::TokenStream {
    if nested_shapes.is_empty() {
        return quote! {};
    }
    let exprs = nested_shapes.iter().map(|field| {
        let path = &field.normalize_path;
        let ty = &field.spec.inner_ty;
        quote! {
            ::agena_plugin_sdk::macro_support::normalize_nested_input_path(
                &mut input,
                #path,
                &<#ty as ::agena_plugin_sdk::ToolInput>::input_schema(),
            );
        }
    });
    quote! { #(#exprs)* }
}

pub fn expand_nested_shape_path_specs_expr(
    nested_shapes: &[NestedInputShapeField],
    variant_optional: bool,
) -> proc_macro2::TokenStream {
    if nested_shapes.is_empty() {
        return quote! {};
    }
    let field_exprs = nested_shapes.iter().map(|field| {
        let ty = &field.spec.inner_ty;
        let force_optional = field.spec.optional || variant_optional;
        let mut seen = BTreeSet::new();
        let prefixes = std::iter::once(&field.schema_path)
            .chain(field.schema_aliases.iter())
            .filter_map(|candidate| {
                let prefix = if field.spec.array {
                    format!("$.{}[*]", candidate.value())
                } else {
                    format!("$.{}", candidate.value())
                };
                if seen.insert(prefix.clone()) {
                    Some(LitStr::new(prefix.as_str(), candidate.span()))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        quote! {
            #(
                __items.extend(
                    <#ty as ::agena_plugin_sdk::ToolInput>::input_paths()
                        .into_iter()
                        .map(|mut __spec| {
                            if let Some(__jsonpath) = ::agena_plugin_sdk::macro_support::prefix_input_jsonpath(
                                #prefixes,
                                __spec.jsonpath.as_str(),
                            ) {
                                __spec.jsonpath = __jsonpath;
                            }
                            if #force_optional {
                                __spec.optional = true;
                            }
                            __spec
                        })
                );
            )*
        }
    });
    quote! { #(#field_exprs)* }
}

pub fn expand_nested_shape_network_specs_expr(
    nested_shapes: &[NestedInputShapeField],
    variant_optional: bool,
) -> proc_macro2::TokenStream {
    if nested_shapes.is_empty() {
        return quote! {};
    }
    let field_exprs = nested_shapes.iter().map(|field| {
        let ty = &field.spec.inner_ty;
        let force_optional = field.spec.optional || variant_optional;
        let mut seen = BTreeSet::new();
        let prefixes = std::iter::once(&field.schema_path)
            .chain(field.schema_aliases.iter())
            .filter_map(|candidate| {
                let prefix = if field.spec.array {
                    format!("$.{}[*]", candidate.value())
                } else {
                    format!("$.{}", candidate.value())
                };
                if seen.insert(prefix.clone()) {
                    Some(LitStr::new(prefix.as_str(), candidate.span()))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        quote! {
            #(
                __items.extend(
                    <#ty as ::agena_plugin_sdk::ToolInput>::input_networks()
                        .into_iter()
                        .map(|mut __spec| {
                            if let Some(__jsonpath) = ::agena_plugin_sdk::macro_support::prefix_input_jsonpath(
                                #prefixes,
                                __spec.jsonpath.as_str(),
                            ) {
                                __spec.jsonpath = __jsonpath;
                            }
                            if #force_optional {
                                __spec.optional = true;
                            }
                            __spec
                        })
                );
            )*
        }
    });
    quote! { #(#field_exprs)* }
}

pub fn expand_nested_shape_input_keys_expr(
    nested_shapes: &[NestedInputShapeField],
    path: &LitStr,
) -> proc_macro2::TokenStream {
    if nested_shapes.is_empty() {
        return quote! { ::std::vec::Vec::new() };
    }
    let field_exprs = nested_shapes.iter().map(|field| {
        let ty = &field.spec.inner_ty;
        let prefix = &field.normalize_path;
        let prefix_dot = LitStr::new(format!("{}.", prefix.value()).as_str(), prefix.span());
        let mut seen = BTreeSet::new();
        let prefixes = std::iter::once(&field.schema_path)
            .chain(field.schema_aliases.iter())
            .filter_map(|candidate| {
                let value = if field.spec.array {
                    format!("{}[]", candidate.value())
                } else {
                    candidate.value()
                };
                if seen.insert(value.clone()) {
                    Some(LitStr::new(value.as_str(), candidate.span()))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        quote! {
            if let Some(__tail) = #path.strip_prefix(#prefix_dot) {
                let __inner_keys =
                    ::agena_plugin_sdk::macro_support::flattened_input_keys_for_parse_path(
                        &<#ty as ::agena_plugin_sdk::ToolInput>::input_schema(),
                        __tail,
                    );
                #(
                    __keys.extend(
                        __inner_keys
                            .iter()
                            .map(|__inner| format!("{}.{__inner}", #prefixes)),
                    );
                )*
            }
        }
    });
    quote! {{
        let mut __keys = ::std::vec::Vec::new();
        #(#field_exprs)*
        __keys
    }}
}

pub fn expand_input_shape_resolved_path_expr(
    flatten_shapes: &[Type],
    nested_shapes: &[NestedInputShapeField],
    path: &LitStr,
) -> proc_macro2::TokenStream {
    if flatten_shapes.is_empty() && nested_shapes.is_empty() {
        return quote! { #path.to_string() };
    }
    let nested_expr = if nested_shapes.is_empty() {
        quote! {}
    } else {
        let exprs = nested_shapes.iter().map(|field| {
            let ty = &field.spec.inner_ty;
            let prefix = &field.normalize_path;
            let prefix_dot = LitStr::new(format!("{}.", prefix.value()).as_str(), prefix.span());
            quote! {
                if let Some(__tail) = __path.strip_prefix(#prefix_dot) {
                    let __resolved = ::agena_plugin_sdk::macro_support::resolve_input_constraint_path(
                        &<#ty as ::agena_plugin_sdk::ToolInput>::input_schema(),
                        __tail,
                    );
                    __path = format!("{}.{__resolved}", #prefix);
                }
            }
        });
        quote! { #(#exprs)* }
    };
    quote! {{
        let mut __path = #path.to_string();
        #nested_expr
        #(
            __path = ::agena_plugin_sdk::macro_support::resolve_input_constraint_path(
                &<#flatten_shapes as ::agena_plugin_sdk::ToolInput>::input_schema(),
                __path.as_str(),
            );
        )*
        __path
    }}
}

pub fn expand_flatten_shape_schema_normalize_expr(
    flatten_shapes: &[Type],
) -> proc_macro2::TokenStream {
    if flatten_shapes.is_empty() {
        return quote! {};
    }
    quote! {
        #(
            ::agena_plugin_sdk::macro_support::normalize_flattened_input_object(
                &mut input,
                &<#flatten_shapes as ::agena_plugin_sdk::ToolInput>::input_schema(),
            );
        )*
    }
}

pub fn expand_flatten_shape_input_keys_expr(
    flatten_shapes: &[Type],
    path: &LitStr,
) -> proc_macro2::TokenStream {
    if flatten_shapes.is_empty() {
        return quote! { ::std::vec::Vec::new() };
    }
    quote! {{
        let mut __keys = ::std::vec::Vec::new();
        #(
            __keys.extend(
                ::agena_plugin_sdk::macro_support::flattened_input_keys_for_parse_path(
                    &<#flatten_shapes as ::agena_plugin_sdk::ToolInput>::input_schema(),
                    #path,
                ),
            );
        )*
        __keys
    }}
}
