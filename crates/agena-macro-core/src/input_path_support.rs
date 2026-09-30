//! Path-type argument support for tool inputs.

use std::collections::BTreeSet;

use quote::quote;
use syn::{Attribute, Data, LitStr, Result, Type};

use super::{
    NestedInputShapeField, flatten_shape_type, nested_input_shape_field,
    serde_rename_all_fields_rule, serde_rename_all_rule,
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

/// Tool-input tag derivation. Input shapes declare no tags, so this is
/// always the empty list; the function survives as the single place the
/// derive's tag surface is produced.
pub fn expand_input_tags_expr() -> proc_macro2::TokenStream {
    quote! { ::std::vec::Vec::new() }
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
