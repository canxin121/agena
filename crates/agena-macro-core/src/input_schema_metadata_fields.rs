//! Field-level JSON Schema metadata for tool inputs.

use quote::quote;
use syn::spanned::Spanned;
use syn::{Fields, LitStr, Result};

use super::{
    doc_text, flatten_shape_type, nested_input_shape_spec, parse_input_field_arg_attrs,
    prepare_input_field_names,
};

pub fn tool_input_struct_field_schema_metadata_calls(
    prefix: &str,
    fields: &Fields,
    rename_rule: Option<super::SerdeRenameRule>,
) -> Result<Vec<proc_macro2::TokenStream>> {
    let Fields::Named(named) = fields else {
        return Ok(Vec::new());
    };
    let mut calls = Vec::new();
    for (index, field) in named.named.iter().enumerate() {
        let order = LitStr::new(format!("{index:06}").as_str(), field.span());
        if let Some(flatten_shape_ty) = flatten_shape_type(field)? {
            let pointer = LitStr::new(prefix, field.span());
            calls.push(quote! {
                let mut overlay = <#flatten_shape_ty as ::agena_plugin_sdk::ToolInput>::input_schema();
                ::agena_plugin_sdk::macro_support::prefix_schema_order_metadata(
                    &mut overlay,
                    #order,
                );
                ::agena_plugin_sdk::macro_support::merge_flattened_schema_at_pointer(
                    schema,
                    #pointer,
                    &overlay,
                );
            });
            continue;
        }
        let arg_config = parse_input_field_arg_attrs(field)?;
        let Some(names) = prepare_input_field_names(field, rename_rule, &arg_config)? else {
            continue;
        };
        let nested_shape = nested_input_shape_spec(field)?;
        if names.schema_path.value() != names.parse_path.value() {
            let pointer = LitStr::new(prefix, field.span());
            let from = &names.parse_path;
            let to = &names.schema_path;
            calls.push(quote! {
                ::agena_plugin_sdk::macro_support::rename_schema_property(
                    schema,
                    #pointer,
                    #from,
                    #to,
                );
            });
        }
        if let Some(nested_shape) = nested_shape {
            let pointer = if prefix.is_empty() {
                if nested_shape.array {
                    format!("/properties/{}/items", names.schema_path.value())
                } else {
                    format!("/properties/{}", names.schema_path.value())
                }
            } else if nested_shape.array {
                format!("{prefix}/properties/{}/items", names.schema_path.value())
            } else {
                format!("{prefix}/properties/{}", names.schema_path.value())
            };
            let pointer = LitStr::new(pointer.as_str(), field.span());
            let inner_ty = &nested_shape.inner_ty;
            calls.push(quote! {
                let mut overlay = <#inner_ty as ::agena_plugin_sdk::ToolInput>::input_schema();
                ::agena_plugin_sdk::macro_support::prefix_schema_order_metadata(
                    &mut overlay,
                    #order,
                );
                ::agena_plugin_sdk::macro_support::merge_schema_overlay_at_pointer(
                    schema,
                    #pointer,
                    &overlay,
                );
            });
        }
        let description = doc_text(&field.attrs).and_then(|text| {
            let trimmed = text.trim().to_string();
            (!trimmed.is_empty()).then_some(trimmed)
        });
        let pointer = if prefix.is_empty() {
            format!("/properties/{}", names.schema_path.value())
        } else {
            format!("{prefix}/properties/{}", names.schema_path.value())
        };
        let pointer = LitStr::new(pointer.as_str(), field.span());
        calls.push(quote! {
            ::agena_plugin_sdk::macro_support::set_schema_string_metadata(
                schema,
                #pointer,
                "x-agena-order",
                #order,
            );
        });
        if let Some(description) = description {
            let description = LitStr::new(&description, field.span());
            calls.push(quote! {
                ::agena_plugin_sdk::macro_support::set_schema_metadata(
                    schema,
                    #pointer,
                    None,
                    Some(#description),
                );
            });
        }
        if !names.schema_aliases.is_empty() {
            let alias_values = names.schema_aliases.iter().collect::<Vec<_>>();
            calls.push(quote! {
                ::agena_plugin_sdk::macro_support::set_schema_string_list_metadata(
                    schema,
                    #pointer,
                    "x-agena-aliases",
                    &[#(#alias_values),*],
                );
            });
        }
    }
    Ok(calls)
}
