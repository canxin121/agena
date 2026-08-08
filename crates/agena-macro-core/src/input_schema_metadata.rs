//! JSON Schema metadata generation for tool inputs.

use std::collections::BTreeSet;

use quote::quote;
use syn::{Attribute, Data, LitStr, Result, Variant};

use super::input_schema_metadata_fields::tool_input_struct_field_schema_metadata_calls;
use super::input_schema_path_support::{
    schema_pointer_from_logical_path, schema_relation_display_path,
};
use super::{
    SchemaConstraintSource, SchemaRelationSource, ToolSpecConfig, doc_text, network_semantic_label,
    path_permission_kind_label, picker_kind_label, serde_rename_all_fields_rule,
    serde_rename_all_rule,
};

pub fn expand_schema_metadata_fn<C, F>(
    attrs: &[Attribute],
    data: &Data,
    constraints: &C,
    mut variant_constraint_metadata: F,
) -> Result<proc_macro2::TokenStream>
where
    C: SchemaConstraintSource + SchemaRelationSource,
    F: FnMut(&Variant, &str) -> Result<Vec<proc_macro2::TokenStream>>,
{
    let mut metadata_calls = Vec::new();
    match data {
        Data::Struct(data_struct) => {
            let rename_rule = serde_rename_all_rule(attrs)?;
            metadata_calls.extend(tool_input_struct_field_schema_metadata_calls(
                "",
                &data_struct.fields,
                rename_rule,
            )?);
            metadata_calls.extend(constraint_schema_metadata_calls("", constraints)?);
            metadata_calls.extend(constraint_relation_metadata_calls("", constraints)?);
        }
        Data::Enum(data_enum) => {
            let enum_field_rule = serde_rename_all_fields_rule(attrs)?;
            for (index, variant) in data_enum.variants.iter().enumerate() {
                if let Some(description) = doc_text(&variant.attrs).and_then(|text| {
                    let trimmed = text.trim().to_string();
                    (!trimmed.is_empty()).then_some(trimmed)
                }) {
                    let description = LitStr::new(&description, variant.ident.span());
                    for group in ["oneOf", "anyOf", "allOf"] {
                        let pointer =
                            LitStr::new(format!("/{group}/{index}").as_str(), variant.ident.span());
                        metadata_calls.push(quote! {
                            ::agena_plugin_sdk::macro_support::set_schema_metadata(
                                schema,
                                #pointer,
                                None,
                                Some(#description),
                            );
                        });
                    }
                }
                for group in ["oneOf", "anyOf", "allOf"] {
                    let prefix = format!("/{group}/{index}");
                    let variant_field_rule =
                        serde_rename_all_rule(&variant.attrs)?.or(enum_field_rule);
                    metadata_calls.extend(tool_input_struct_field_schema_metadata_calls(
                        prefix.as_str(),
                        &variant.fields,
                        variant_field_rule,
                    )?);
                    metadata_calls.extend(constraint_schema_metadata_calls(
                        prefix.as_str(),
                        constraints,
                    )?);
                    metadata_calls.extend(constraint_relation_metadata_calls(
                        prefix.as_str(),
                        constraints,
                    )?);
                    metadata_calls.extend(variant_constraint_metadata(variant, prefix.as_str())?);
                }
            }
        }
        Data::Union(_) => {}
    }

    Ok(quote! {
        fn __macro_apply_schema_metadata(schema: &mut serde_json::Value) {
            #(#metadata_calls)*
        }
    })
}

pub fn tool_spec_schema_metadata_calls(
    spec: &ToolSpecConfig,
) -> Result<Vec<proc_macro2::TokenStream>> {
    let mut metadata_calls = constraint_schema_metadata_calls("", spec)?;
    metadata_calls.extend(constraint_relation_metadata_calls("", spec)?);
    Ok(metadata_calls)
}

pub fn constraint_relation_metadata_calls<C: SchemaRelationSource + SchemaConstraintSource>(
    prefix: &str,
    constraints: &C,
) -> Result<Vec<proc_macro2::TokenStream>> {
    let mut labels = Vec::new();
    let display_path = |path: &LitStr| {
        schema_relation_display_path(path.value().as_str(), constraints.input_field_metadata())
    };
    for group in constraints.exactly_one_of() {
        if !group.is_empty() {
            let joined = group
                .iter()
                .map(|path| format!("`{}`", display_path(path)))
                .collect::<Vec<_>>()
                .join(", ");
            labels.push(format!("exactly_one_of: {joined}"));
        }
    }
    for group in constraints.at_least_one_of() {
        if !group.is_empty() {
            let joined = group
                .iter()
                .map(|path| format!("`{}`", display_path(path)))
                .collect::<Vec<_>>()
                .join(", ");
            labels.push(format!("at_least_one_of: {joined}"));
        }
    }
    for constraint in constraints.requires() {
        labels.push(format!(
            "requires `{}` -> `{}`",
            display_path(&constraint.left),
            display_path(&constraint.right)
        ));
    }
    for constraint in constraints.conflicts_with() {
        labels.push(format!(
            "conflicts_with `{}` x `{}`",
            display_path(&constraint.left),
            display_path(&constraint.right)
        ));
    }
    for constraint in constraints.required_unless_present() {
        labels.push(format!(
            "required_unless_present `{}` unless `{}` present",
            display_path(&constraint.left),
            display_path(&constraint.right)
        ));
    }
    for constraint in constraints.forbid_substrings() {
        let joined = constraint
            .values
            .iter()
            .map(|value| format!("\"{}\"", value.value()))
            .collect::<Vec<_>>()
            .join(", ");
        labels.push(format!(
            "forbid_substrings `{}`: {joined}",
            display_path(&constraint.path)
        ));
    }
    for path in constraints.distinct_trimmed() {
        labels.push(format!("distinct_trimmed `{}`", display_path(path)));
    }
    for constraint in constraints.distinct_trimmed_within() {
        labels.push(format!(
            "distinct_trimmed_within `{}` within `{}`",
            display_path(&constraint.left),
            display_path(&constraint.right)
        ));
    }
    if labels.is_empty() {
        return Ok(Vec::new());
    }
    let labels = labels
        .into_iter()
        .map(|label| LitStr::new(label.as_str(), proc_macro2::Span::call_site()))
        .collect::<Vec<_>>();
    let pointer = LitStr::new(prefix, proc_macro2::Span::call_site());
    Ok(vec![quote! {
        ::agena_plugin_sdk::macro_support::set_schema_string_list_metadata(
            schema,
            #pointer,
            "x-agena-relations",
            &[#(#labels),*],
        );
    }])
}

pub fn constraint_schema_metadata_calls<C: SchemaConstraintSource>(
    prefix: &str,
    constraints: &C,
) -> Result<Vec<proc_macro2::TokenStream>> {
    let mut calls = Vec::new();
    let non_empty_metadata_parse_paths = constraints
        .input_field_metadata()
        .iter()
        .filter(|metadata| metadata.non_empty)
        .map(|metadata| metadata.parse_path.value())
        .collect::<BTreeSet<_>>();
    let minimum_metadata_parse_paths = constraints
        .input_field_metadata()
        .iter()
        .filter(|metadata| metadata.minimum.is_some())
        .map(|metadata| metadata.parse_path.value())
        .collect::<BTreeSet<_>>();
    let maximum_metadata_parse_paths = constraints
        .input_field_metadata()
        .iter()
        .filter(|metadata| metadata.maximum.is_some())
        .map(|metadata| metadata.parse_path.value())
        .collect::<BTreeSet<_>>();
    let exclusive_minimum_metadata_parse_paths = constraints
        .input_field_metadata()
        .iter()
        .filter(|metadata| metadata.exclusive_minimum.is_some())
        .map(|metadata| metadata.parse_path.value())
        .collect::<BTreeSet<_>>();
    let exclusive_maximum_metadata_parse_paths = constraints
        .input_field_metadata()
        .iter()
        .filter(|metadata| metadata.exclusive_maximum.is_some())
        .map(|metadata| metadata.parse_path.value())
        .collect::<BTreeSet<_>>();
    let item_minimum_metadata_parse_paths = constraints
        .input_field_metadata()
        .iter()
        .filter_map(|metadata| {
            metadata
                .item_minimum
                .as_ref()
                .map(|_| format!("{}[]", metadata.parse_path.value()))
        })
        .collect::<BTreeSet<_>>();
    let item_maximum_metadata_parse_paths = constraints
        .input_field_metadata()
        .iter()
        .filter_map(|metadata| {
            metadata
                .item_maximum
                .as_ref()
                .map(|_| format!("{}[]", metadata.parse_path.value()))
        })
        .collect::<BTreeSet<_>>();
    let item_exclusive_minimum_metadata_parse_paths = constraints
        .input_field_metadata()
        .iter()
        .filter_map(|metadata| {
            metadata
                .item_exclusive_minimum
                .as_ref()
                .map(|_| format!("{}[]", metadata.parse_path.value()))
        })
        .collect::<BTreeSet<_>>();
    let item_exclusive_maximum_metadata_parse_paths = constraints
        .input_field_metadata()
        .iter()
        .filter_map(|metadata| {
            metadata
                .item_exclusive_maximum
                .as_ref()
                .map(|_| format!("{}[]", metadata.parse_path.value()))
        })
        .collect::<BTreeSet<_>>();
    let min_items_metadata_parse_paths = constraints
        .input_field_metadata()
        .iter()
        .filter(|metadata| metadata.min_items.is_some())
        .map(|metadata| metadata.parse_path.value())
        .collect::<BTreeSet<_>>();
    let max_items_metadata_parse_paths = constraints
        .input_field_metadata()
        .iter()
        .filter(|metadata| metadata.max_items.is_some())
        .map(|metadata| metadata.parse_path.value())
        .collect::<BTreeSet<_>>();
    let min_properties_metadata_parse_paths = constraints
        .input_field_metadata()
        .iter()
        .filter(|metadata| metadata.min_properties.is_some())
        .map(|metadata| metadata.parse_path.value())
        .collect::<BTreeSet<_>>();
    let max_properties_metadata_parse_paths = constraints
        .input_field_metadata()
        .iter()
        .filter(|metadata| metadata.max_properties.is_some())
        .map(|metadata| metadata.parse_path.value())
        .collect::<BTreeSet<_>>();
    let item_min_properties_metadata_parse_paths = constraints
        .input_field_metadata()
        .iter()
        .filter_map(|metadata| {
            metadata
                .item_min_properties
                .map(|_| format!("{}[]", metadata.parse_path.value()))
        })
        .collect::<BTreeSet<_>>();
    let item_max_properties_metadata_parse_paths = constraints
        .input_field_metadata()
        .iter()
        .filter_map(|metadata| {
            metadata
                .item_max_properties
                .map(|_| format!("{}[]", metadata.parse_path.value()))
        })
        .collect::<BTreeSet<_>>();
    let min_chars_metadata_parse_paths = constraints
        .input_field_metadata()
        .iter()
        .filter(|metadata| metadata.min_chars.is_some())
        .map(|metadata| metadata.parse_path.value())
        .collect::<BTreeSet<_>>();
    let max_chars_metadata_parse_paths = constraints
        .input_field_metadata()
        .iter()
        .filter(|metadata| metadata.max_chars.is_some())
        .map(|metadata| metadata.parse_path.value())
        .collect::<BTreeSet<_>>();
    let item_min_chars_metadata_parse_paths = constraints
        .input_field_metadata()
        .iter()
        .filter_map(|metadata| {
            metadata
                .item_min_chars
                .map(|_| format!("{}[]", metadata.parse_path.value()))
        })
        .collect::<BTreeSet<_>>();
    let item_max_chars_metadata_parse_paths = constraints
        .input_field_metadata()
        .iter()
        .filter_map(|metadata| {
            metadata
                .item_max_chars
                .map(|_| format!("{}[]", metadata.parse_path.value()))
        })
        .collect::<BTreeSet<_>>();
    let format_metadata_parse_paths = constraints
        .input_field_metadata()
        .iter()
        .filter(|metadata| metadata.format.is_some())
        .map(|metadata| metadata.parse_path.value())
        .collect::<BTreeSet<_>>();
    let item_format_metadata_parse_paths = constraints
        .input_field_metadata()
        .iter()
        .filter_map(|metadata| {
            metadata
                .item_format
                .as_ref()
                .map(|_| format!("{}[]", metadata.parse_path.value()))
        })
        .collect::<BTreeSet<_>>();
    let pattern_metadata_parse_paths = constraints
        .input_field_metadata()
        .iter()
        .filter(|metadata| metadata.pattern.is_some())
        .map(|metadata| metadata.parse_path.value())
        .collect::<BTreeSet<_>>();
    let item_pattern_metadata_parse_paths = constraints
        .input_field_metadata()
        .iter()
        .filter_map(|metadata| {
            metadata
                .item_pattern
                .as_ref()
                .map(|_| format!("{}[]", metadata.parse_path.value()))
        })
        .collect::<BTreeSet<_>>();
    let item_choice_metadata_parse_paths = constraints
        .input_field_metadata()
        .iter()
        .filter(|metadata| !metadata.item_choices.is_empty())
        .map(|metadata| format!("{}[]", metadata.parse_path.value()))
        .collect::<BTreeSet<_>>();
    let choice_metadata_parse_paths = constraints
        .input_field_metadata()
        .iter()
        .filter(|metadata| !metadata.choices.is_empty())
        .map(|metadata| metadata.parse_path.value())
        .collect::<BTreeSet<_>>();
    calls.extend(
        constraints
            .input_field_metadata()
            .iter()
            .enumerate()
            .map(|(index, metadata)| {
                let pointer = schema_pointer_from_logical_path(prefix, &metadata.path.value())?;
                let pointer = LitStr::new(pointer.as_str(), metadata.path.span());
                let order = LitStr::new(format!("{index:06}").as_str(), metadata.path.span());
                let mut calls = Vec::new();
                calls.push(quote! {
                    ::agena_plugin_sdk::macro_support::set_schema_string_metadata(
                        schema,
                        #pointer,
                        "x-agena-order",
                        #order,
                    );
                });
                if let Some(description) = metadata.description.as_ref() {
                    calls.push(quote! {
                        ::agena_plugin_sdk::macro_support::set_schema_metadata(
                            schema,
                            #pointer,
                            None,
                            Some(#description),
                        );
                    });
                }
                if let Some(kind) = metadata.path_kind {
                    let label = LitStr::new(path_permission_kind_label(kind), metadata.path.span());
                    calls.push(quote! {
                        ::agena_plugin_sdk::macro_support::set_schema_string_metadata(
                            schema,
                            #pointer,
                            "x-agena-path",
                            #label,
                        );
                    });
                }
                if let Some(network) = metadata.network {
                    let label = LitStr::new(network_semantic_label(network), metadata.path.span());
                    calls.push(quote! {
                        ::agena_plugin_sdk::macro_support::set_schema_string_metadata(
                            schema,
                            #pointer,
                            "x-agena-network",
                            #label,
                        );
                    });
                }
                if metadata.non_empty {
                    calls.push(quote! {
                        ::agena_plugin_sdk::macro_support::set_schema_non_empty_metadata(
                            schema,
                            #pointer,
                        );
                    });
                }
                if metadata.item_non_empty || metadata.item_non_empty_if_present {
                    let item_pointer = LitStr::new(
                        format!("{}/items", pointer.value()).as_str(),
                        metadata.path.span(),
                    );
                    calls.push(quote! {
                        ::agena_plugin_sdk::macro_support::set_schema_non_empty_metadata(
                            schema,
                            #item_pointer,
                        );
                    });
                }
                if let Some(minimum) = metadata.minimum.as_ref() {
                    calls.push(quote! {
                        ::agena_plugin_sdk::macro_support::set_schema_number_metadata(
                            schema,
                            #pointer,
                            "minimum",
                            ::agena_plugin_sdk::serde_json::json!(#minimum),
                        );
                    });
                }
                if let Some(maximum) = metadata.maximum.as_ref() {
                    calls.push(quote! {
                        ::agena_plugin_sdk::macro_support::set_schema_number_metadata(
                            schema,
                            #pointer,
                            "maximum",
                            ::agena_plugin_sdk::serde_json::json!(#maximum),
                        );
                    });
                }
                if let Some(minimum) = metadata.exclusive_minimum.as_ref() {
                    calls.push(quote! {
                        ::agena_plugin_sdk::macro_support::set_schema_number_metadata(
                            schema,
                            #pointer,
                            "exclusiveMinimum",
                            ::agena_plugin_sdk::serde_json::json!(#minimum),
                        );
                    });
                }
                if let Some(maximum) = metadata.exclusive_maximum.as_ref() {
                    calls.push(quote! {
                        ::agena_plugin_sdk::macro_support::set_schema_number_metadata(
                            schema,
                            #pointer,
                            "exclusiveMaximum",
                            ::agena_plugin_sdk::serde_json::json!(#maximum),
                        );
                    });
                }
                if let Some(minimum) = metadata.item_minimum.as_ref() {
                    let item_pointer = LitStr::new(
                        format!("{}/items", pointer.value()).as_str(),
                        metadata.path.span(),
                    );
                    calls.push(quote! {
                        ::agena_plugin_sdk::macro_support::set_schema_number_metadata(
                            schema,
                            #item_pointer,
                            "minimum",
                            ::agena_plugin_sdk::serde_json::json!(#minimum),
                        );
                    });
                }
                if let Some(minimum) = metadata.item_exclusive_minimum.as_ref() {
                    let item_pointer = LitStr::new(
                        format!("{}/items", pointer.value()).as_str(),
                        metadata.path.span(),
                    );
                    calls.push(quote! {
                        ::agena_plugin_sdk::macro_support::set_schema_number_metadata(
                            schema,
                            #item_pointer,
                            "exclusiveMinimum",
                            ::agena_plugin_sdk::serde_json::json!(#minimum),
                        );
                    });
                }
                if let Some(maximum) = metadata.item_exclusive_maximum.as_ref() {
                    let item_pointer = LitStr::new(
                        format!("{}/items", pointer.value()).as_str(),
                        metadata.path.span(),
                    );
                    calls.push(quote! {
                        ::agena_plugin_sdk::macro_support::set_schema_number_metadata(
                            schema,
                            #item_pointer,
                            "exclusiveMaximum",
                            ::agena_plugin_sdk::serde_json::json!(#maximum),
                        );
                    });
                }
                if let Some(maximum) = metadata.item_maximum.as_ref() {
                    let item_pointer = LitStr::new(
                        format!("{}/items", pointer.value()).as_str(),
                        metadata.path.span(),
                    );
                    calls.push(quote! {
                        ::agena_plugin_sdk::macro_support::set_schema_number_metadata(
                            schema,
                            #item_pointer,
                            "maximum",
                            ::agena_plugin_sdk::serde_json::json!(#maximum),
                        );
                    });
                }
                if let Some(value) = metadata.min_items {
                    let value = value as u64;
                    calls.push(quote! {
                        ::agena_plugin_sdk::macro_support::set_schema_u64_metadata(
                            schema,
                            #pointer,
                            "minItems",
                            #value,
                        );
                    });
                }
                if let Some(value) = metadata.max_items {
                    let value = value as u64;
                    calls.push(quote! {
                        ::agena_plugin_sdk::macro_support::set_schema_u64_metadata(
                            schema,
                            #pointer,
                            "maxItems",
                            #value,
                        );
                    });
                }
                if let Some(value) = metadata.min_properties {
                    let value = value as u64;
                    calls.push(quote! {
                        ::agena_plugin_sdk::macro_support::set_schema_minimum_u64_metadata(
                            schema,
                            #pointer,
                            "minProperties",
                            #value,
                        );
                    });
                }
                if let Some(value) = metadata.max_properties {
                    let value = value as u64;
                    calls.push(quote! {
                        ::agena_plugin_sdk::macro_support::set_schema_u64_metadata(
                            schema,
                            #pointer,
                            "maxProperties",
                            #value,
                        );
                    });
                }
                if let Some(value) = metadata.item_min_properties {
                    let item_pointer = LitStr::new(
                        format!("{}/items", pointer.value()).as_str(),
                        metadata.path.span(),
                    );
                    let value = value as u64;
                    calls.push(quote! {
                        ::agena_plugin_sdk::macro_support::set_schema_minimum_u64_metadata(
                            schema,
                            #item_pointer,
                            "minProperties",
                            #value,
                        );
                    });
                }
                if let Some(value) = metadata.item_max_properties {
                    let item_pointer = LitStr::new(
                        format!("{}/items", pointer.value()).as_str(),
                        metadata.path.span(),
                    );
                    let value = value as u64;
                    calls.push(quote! {
                        ::agena_plugin_sdk::macro_support::set_schema_u64_metadata(
                            schema,
                            #item_pointer,
                            "maxProperties",
                            #value,
                        );
                    });
                }
                if let Some(value) = metadata.min_chars {
                    let value = value as u64;
                    calls.push(quote! {
                        ::agena_plugin_sdk::macro_support::set_schema_minimum_u64_metadata(
                            schema,
                            #pointer,
                            "minLength",
                            #value,
                        );
                    });
                }
                if let Some(value) = metadata.max_chars {
                    let value = value as u64;
                    calls.push(quote! {
                        ::agena_plugin_sdk::macro_support::set_schema_u64_metadata(
                            schema,
                            #pointer,
                            "maxLength",
                            #value,
                        );
                    });
                }
                if let Some(value) = metadata.item_min_chars {
                    let item_pointer = LitStr::new(
                        format!("{}/items", pointer.value()).as_str(),
                        metadata.path.span(),
                    );
                    let value = value as u64;
                    calls.push(quote! {
                        ::agena_plugin_sdk::macro_support::set_schema_minimum_u64_metadata(
                            schema,
                            #item_pointer,
                            "minLength",
                            #value,
                        );
                    });
                }
                if let Some(value) = metadata.item_max_chars {
                    let item_pointer = LitStr::new(
                        format!("{}/items", pointer.value()).as_str(),
                        metadata.path.span(),
                    );
                    let value = value as u64;
                    calls.push(quote! {
                        ::agena_plugin_sdk::macro_support::set_schema_u64_metadata(
                            schema,
                            #item_pointer,
                            "maxLength",
                            #value,
                        );
                    });
                }
                if let Some(format) = metadata.format.as_ref() {
                    calls.push(quote! {
                        ::agena_plugin_sdk::macro_support::set_schema_string_metadata(
                            schema,
                            #pointer,
                            "format",
                            #format,
                        );
                    });
                }
                if let Some(format) = metadata.item_format.as_ref() {
                    let item_pointer = LitStr::new(
                        format!("{}/items", pointer.value()).as_str(),
                        metadata.path.span(),
                    );
                    calls.push(quote! {
                        ::agena_plugin_sdk::macro_support::set_schema_string_metadata(
                            schema,
                            #item_pointer,
                            "format",
                            #format,
                        );
                    });
                }
                if let Some(pattern) = metadata.pattern.as_ref() {
                    calls.push(quote! {
                        ::agena_plugin_sdk::macro_support::set_schema_string_metadata(
                            schema,
                            #pointer,
                            "pattern",
                            #pattern,
                        );
                    });
                }
                if let Some(pattern) = metadata.item_pattern.as_ref() {
                    let item_pointer = LitStr::new(
                        format!("{}/items", pointer.value()).as_str(),
                        metadata.path.span(),
                    );
                    calls.push(quote! {
                        ::agena_plugin_sdk::macro_support::set_schema_string_metadata(
                            schema,
                            #item_pointer,
                            "pattern",
                            #pattern,
                        );
                    });
                }
                if !metadata.item_choices.is_empty() {
                    let item_pointer = LitStr::new(
                        format!("{}/items", pointer.value()).as_str(),
                        metadata.path.span(),
                    );
                    let values = metadata.item_choices.iter().collect::<Vec<_>>();
                    calls.push(quote! {
                        ::agena_plugin_sdk::macro_support::set_schema_value_list_metadata(
                            schema,
                            #item_pointer,
                            "enum",
                            &[#(::agena_plugin_sdk::serde_json::json!(#values)),*],
                        );
                    });
                }
                if let Some(picker) = metadata.picker {
                    let label = LitStr::new(picker_kind_label(picker), metadata.path.span());
                    calls.push(quote! {
                        ::agena_plugin_sdk::macro_support::set_schema_string_metadata(
                            schema,
                            #pointer,
                            "x-agena-picker",
                            #label,
                        );
                    });
                }
                if metadata.secret {
                    calls.push(quote! {
                        ::agena_plugin_sdk::macro_support::set_schema_bool_metadata(
                            schema,
                            #pointer,
                            "x-agena-secret",
                            true,
                        );
                    });
                }
                if let Some(example) = metadata.example.as_ref() {
                    calls.push(quote! {
                        ::agena_plugin_sdk::macro_support::set_schema_value_list_metadata(
                            schema,
                            #pointer,
                            "examples",
                            &[::agena_plugin_sdk::serde_json::json!(#example)],
                        );
                    });
                }
                if !metadata.choices.is_empty() {
                    let values = metadata.choices.iter().collect::<Vec<_>>();
                    calls.push(quote! {
                        ::agena_plugin_sdk::macro_support::set_schema_value_list_metadata(
                            schema,
                            #pointer,
                            "enum",
                            &[#(::agena_plugin_sdk::serde_json::json!(#values)),*],
                        );
                    });
                }
                if !metadata.aliases.is_empty() {
                    let aliases = metadata.aliases.iter().collect::<Vec<_>>();
                    calls.push(quote! {
                        ::agena_plugin_sdk::macro_support::set_schema_string_list_metadata(
                            schema,
                            #pointer,
                            "x-agena-aliases",
                            &[#(#aliases),*],
                        );
                    });
                }
                if metadata.path.value() != metadata.parse_path.value() {
                    let parse_path = &metadata.parse_path;
                    calls.push(quote! {
                        ::agena_plugin_sdk::macro_support::set_schema_string_metadata(
                            schema,
                            #pointer,
                            "x-agena-parse-name",
                            #parse_path,
                        );
                    });
                }
                Ok(quote! { #(#calls)* })
            })
            .collect::<Result<Vec<_>>>()?,
    );
    calls.extend(
        constraints
            .choices()
            .iter()
            .filter(|constraint| {
                !choice_metadata_parse_paths.contains(&constraint.path.value())
                    && !item_choice_metadata_parse_paths.contains(&constraint.path.value())
            })
            .map(|constraint| {
                let pointer = schema_pointer_from_logical_path(prefix, &constraint.path.value())?;
                let pointer = LitStr::new(pointer.as_str(), constraint.path.span());
                let values = &constraint.values;
                Ok(quote! {
                    ::agena_plugin_sdk::macro_support::set_schema_value_list_metadata(
                        schema,
                        #pointer,
                        "enum",
                        &[#(::agena_plugin_sdk::serde_json::json!(#values)),*],
                    );
                })
            })
            .collect::<Result<Vec<_>>>()?,
    );
    calls.extend(
        constraints
            .non_empty()
            .iter()
            .chain(constraints.non_empty_if_present().iter())
            .filter(|path| !non_empty_metadata_parse_paths.contains(&path.value()))
            .map(|path| {
                let pointer = schema_pointer_from_logical_path(prefix, &path.value())?;
                let pointer = LitStr::new(pointer.as_str(), path.span());
                Ok(quote! {
                    ::agena_plugin_sdk::macro_support::set_schema_non_empty_metadata(
                        schema,
                        #pointer,
                    );
                })
            })
            .collect::<Result<Vec<_>>>()?,
    );
    calls.extend(
        constraints
            .minimums()
            .iter()
            .filter(|constraint| {
                !minimum_metadata_parse_paths.contains(&constraint.path.value())
                    && !item_minimum_metadata_parse_paths.contains(&constraint.path.value())
            })
            .map(|constraint| {
                let pointer = schema_pointer_from_logical_path(prefix, &constraint.path.value())?;
                let pointer = LitStr::new(pointer.as_str(), constraint.path.span());
                let value = &constraint.value;
                Ok(quote! {
                    ::agena_plugin_sdk::macro_support::set_schema_number_metadata(
                        schema,
                        #pointer,
                        "minimum",
                        ::agena_plugin_sdk::serde_json::json!(#value),
                    );
                })
            })
            .collect::<Result<Vec<_>>>()?,
    );
    calls.extend(
        constraints
            .maximums()
            .iter()
            .filter(|constraint| {
                !maximum_metadata_parse_paths.contains(&constraint.path.value())
                    && !item_maximum_metadata_parse_paths.contains(&constraint.path.value())
            })
            .map(|constraint| {
                let pointer = schema_pointer_from_logical_path(prefix, &constraint.path.value())?;
                let pointer = LitStr::new(pointer.as_str(), constraint.path.span());
                let value = &constraint.value;
                Ok(quote! {
                    ::agena_plugin_sdk::macro_support::set_schema_number_metadata(
                        schema,
                        #pointer,
                        "maximum",
                        ::agena_plugin_sdk::serde_json::json!(#value),
                    );
                })
            })
            .collect::<Result<Vec<_>>>()?,
    );
    calls.extend(
        constraints
            .exclusive_minimums()
            .iter()
            .filter(|constraint| {
                !exclusive_minimum_metadata_parse_paths.contains(&constraint.path.value())
                    && !item_exclusive_minimum_metadata_parse_paths
                        .contains(&constraint.path.value())
            })
            .map(|constraint| {
                let pointer = schema_pointer_from_logical_path(prefix, &constraint.path.value())?;
                let pointer = LitStr::new(pointer.as_str(), constraint.path.span());
                let value = &constraint.value;
                Ok(quote! {
                    ::agena_plugin_sdk::macro_support::set_schema_number_metadata(
                        schema,
                        #pointer,
                        "exclusiveMinimum",
                        ::agena_plugin_sdk::serde_json::json!(#value),
                    );
                })
            })
            .collect::<Result<Vec<_>>>()?,
    );
    calls.extend(
        constraints
            .exclusive_maximums()
            .iter()
            .filter(|constraint| {
                !exclusive_maximum_metadata_parse_paths.contains(&constraint.path.value())
                    && !item_exclusive_maximum_metadata_parse_paths
                        .contains(&constraint.path.value())
            })
            .map(|constraint| {
                let pointer = schema_pointer_from_logical_path(prefix, &constraint.path.value())?;
                let pointer = LitStr::new(pointer.as_str(), constraint.path.span());
                let value = &constraint.value;
                Ok(quote! {
                    ::agena_plugin_sdk::macro_support::set_schema_number_metadata(
                        schema,
                        #pointer,
                        "exclusiveMaximum",
                        ::agena_plugin_sdk::serde_json::json!(#value),
                    );
                })
            })
            .collect::<Result<Vec<_>>>()?,
    );
    calls.extend(
        constraints
            .min_items()
            .iter()
            .filter(|constraint| !min_items_metadata_parse_paths.contains(&constraint.path.value()))
            .map(|constraint| {
                let pointer = schema_pointer_from_logical_path(prefix, &constraint.path.value())?;
                let pointer = LitStr::new(pointer.as_str(), constraint.path.span());
                let value = constraint.value as u64;
                Ok(quote! {
                    ::agena_plugin_sdk::macro_support::set_schema_u64_metadata(
                        schema,
                        #pointer,
                        "minItems",
                        #value,
                    );
                })
            })
            .collect::<Result<Vec<_>>>()?,
    );
    calls.extend(
        constraints
            .max_items()
            .iter()
            .filter(|constraint| !max_items_metadata_parse_paths.contains(&constraint.path.value()))
            .map(|constraint| {
                let pointer = schema_pointer_from_logical_path(prefix, &constraint.path.value())?;
                let pointer = LitStr::new(pointer.as_str(), constraint.path.span());
                let value = constraint.value as u64;
                Ok(quote! {
                    ::agena_plugin_sdk::macro_support::set_schema_u64_metadata(
                        schema,
                        #pointer,
                        "maxItems",
                        #value,
                    );
                })
            })
            .collect::<Result<Vec<_>>>()?,
    );
    calls.extend(
        constraints
            .min_properties()
            .iter()
            .filter(|constraint| {
                !min_properties_metadata_parse_paths.contains(&constraint.path.value())
                    && !item_min_properties_metadata_parse_paths.contains(&constraint.path.value())
            })
            .map(|constraint| {
                let pointer = schema_pointer_from_logical_path(prefix, &constraint.path.value())?;
                let pointer = LitStr::new(pointer.as_str(), constraint.path.span());
                let value = constraint.value as u64;
                Ok(quote! {
                    ::agena_plugin_sdk::macro_support::set_schema_minimum_u64_metadata(
                        schema,
                        #pointer,
                        "minProperties",
                        #value,
                    );
                })
            })
            .collect::<Result<Vec<_>>>()?,
    );
    calls.extend(
        constraints
            .max_properties()
            .iter()
            .filter(|constraint| {
                !max_properties_metadata_parse_paths.contains(&constraint.path.value())
                    && !item_max_properties_metadata_parse_paths.contains(&constraint.path.value())
            })
            .map(|constraint| {
                let pointer = schema_pointer_from_logical_path(prefix, &constraint.path.value())?;
                let pointer = LitStr::new(pointer.as_str(), constraint.path.span());
                let value = constraint.value as u64;
                Ok(quote! {
                    ::agena_plugin_sdk::macro_support::set_schema_u64_metadata(
                        schema,
                        #pointer,
                        "maxProperties",
                        #value,
                    );
                })
            })
            .collect::<Result<Vec<_>>>()?,
    );
    calls.extend(
        constraints
            .min_chars()
            .iter()
            .filter(|constraint| {
                !min_chars_metadata_parse_paths.contains(&constraint.path.value())
                    && !item_min_chars_metadata_parse_paths.contains(&constraint.path.value())
            })
            .map(|constraint| {
                let pointer = schema_pointer_from_logical_path(prefix, &constraint.path.value())?;
                let pointer = LitStr::new(pointer.as_str(), constraint.path.span());
                let value = constraint.value as u64;
                Ok(quote! {
                    ::agena_plugin_sdk::macro_support::set_schema_minimum_u64_metadata(
                        schema,
                        #pointer,
                        "minLength",
                        #value,
                    );
                })
            })
            .collect::<Result<Vec<_>>>()?,
    );
    calls.extend(
        constraints
            .max_chars()
            .iter()
            .filter(|constraint| {
                !max_chars_metadata_parse_paths.contains(&constraint.path.value())
                    && !item_max_chars_metadata_parse_paths.contains(&constraint.path.value())
            })
            .map(|constraint| {
                let pointer = schema_pointer_from_logical_path(prefix, &constraint.path.value())?;
                let pointer = LitStr::new(pointer.as_str(), constraint.path.span());
                let value = constraint.value as u64;
                Ok(quote! {
                    ::agena_plugin_sdk::macro_support::set_schema_u64_metadata(
                        schema,
                        #pointer,
                        "maxLength",
                        #value,
                    );
                })
            })
            .collect::<Result<Vec<_>>>()?,
    );
    calls.extend(
        constraints
            .formats()
            .iter()
            .filter(|constraint| {
                !format_metadata_parse_paths.contains(&constraint.path.value())
                    && !item_format_metadata_parse_paths.contains(&constraint.path.value())
            })
            .map(|constraint| {
                let pointer = schema_pointer_from_logical_path(prefix, &constraint.path.value())?;
                let pointer = LitStr::new(pointer.as_str(), constraint.path.span());
                let value = &constraint.value;
                Ok(quote! {
                    ::agena_plugin_sdk::macro_support::set_schema_string_metadata(
                        schema,
                        #pointer,
                        "format",
                        #value,
                    );
                })
            })
            .collect::<Result<Vec<_>>>()?,
    );
    calls.extend(
        constraints
            .patterns()
            .iter()
            .filter(|constraint| {
                !pattern_metadata_parse_paths.contains(&constraint.path.value())
                    && !item_pattern_metadata_parse_paths.contains(&constraint.path.value())
            })
            .map(|constraint| {
                let pointer = schema_pointer_from_logical_path(prefix, &constraint.path.value())?;
                let pointer = LitStr::new(pointer.as_str(), constraint.path.span());
                let value = &constraint.value;
                Ok(quote! {
                    ::agena_plugin_sdk::macro_support::set_schema_string_metadata(
                        schema,
                        #pointer,
                        "pattern",
                        #value,
                    );
                })
            })
            .collect::<Result<Vec<_>>>()?,
    );
    Ok(calls)
}
