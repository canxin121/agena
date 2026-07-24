use quote::{format_ident, quote};
use syn::{Data, DeriveInput, Result};

use crate::{
    apply_input_field_arg_attrs, built_in_normalization_tokens,
    built_in_post_parse_normalization_tokens, built_in_validation_tokens,
    constraint_relation_metadata_calls, constraint_schema_metadata_calls,
    expand_flatten_shape_post_parse_tokens, expand_flatten_shape_schema_normalize_expr,
    expand_input_alias_normalize_tokens, expand_input_default_insert_tokens,
    expand_input_default_schema_metadata_tokens, expand_input_dispatch_fn,
    expand_input_example_expr, expand_input_networks_expr, expand_input_paths_expr,
    expand_input_root_default_insert_tokens, expand_input_root_default_schema_metadata_tokens,
    expand_input_root_example_schema_metadata_tokens, expand_input_shape_enum_normalize_fn,
    expand_input_shape_enum_parse_error_remap_expr,
    expand_input_shape_enum_post_parse_normalize_expr,
    expand_input_shape_enum_validate_error_remap_expr, expand_input_shape_variant_validation_arm,
    expand_input_tags_expr, expand_nested_shape_schema_normalize_expr,
    expand_result_path_remap_expr, expand_schema_metadata_fn, input_constraint_field_lookup,
    input_default_schema_metadata_calls, input_error_path_mappings,
    normalize_array_value_constraints, normalize_array_value_nested_path_constraints,
    normalized_input_variant_config, parse_input_config, resolve_constraint_expr_paths,
    resolve_constraint_group_paths, resolve_constraint_lit_paths, resolve_constraint_pair_paths,
    resolve_constraint_string_paths, resolve_constraint_strings_paths,
    resolve_constraint_usize_paths, resolve_constraint_values_paths, serde_rename_all_fields_rule,
    serde_rename_all_rule, struct_flatten_shape_types, struct_nested_shape_fields,
};

pub fn expand_input(input: DeriveInput) -> Result<proc_macro2::TokenStream> {
    let name = input.ident;
    let mut config = parse_input_config(&input.attrs)?;
    apply_input_field_arg_attrs(&mut config, &input.attrs, &input.data)?;
    if let Data::Struct(data_struct) = &input.data {
        let rename_rule = serde_rename_all_rule(&input.attrs)?;
        let (field_path_lookup, array_field_paths) =
            input_constraint_field_lookup(&data_struct.fields, rename_rule)?;
        resolve_constraint_lit_paths(&mut config.trim, &field_path_lookup);
        resolve_constraint_string_paths(&mut config.trim_suffix, &field_path_lookup);
        resolve_constraint_lit_paths(&mut config.non_empty, &field_path_lookup);
        resolve_constraint_lit_paths(&mut config.non_empty_if_present, &field_path_lookup);
        resolve_constraint_expr_paths(&mut config.minimums, &field_path_lookup);
        resolve_constraint_expr_paths(&mut config.maximums, &field_path_lookup);
        resolve_constraint_expr_paths(&mut config.exclusive_minimums, &field_path_lookup);
        resolve_constraint_expr_paths(&mut config.exclusive_maximums, &field_path_lookup);
        resolve_constraint_group_paths(&mut config.exactly_one_of, &field_path_lookup);
        resolve_constraint_group_paths(&mut config.at_least_one_of, &field_path_lookup);
        resolve_constraint_pair_paths(&mut config.requires, &field_path_lookup);
        resolve_constraint_pair_paths(&mut config.conflicts_with, &field_path_lookup);
        resolve_constraint_pair_paths(&mut config.required_unless_present, &field_path_lookup);
        resolve_constraint_strings_paths(&mut config.forbid_substrings, &field_path_lookup);
        resolve_constraint_lit_paths(&mut config.distinct_trimmed, &field_path_lookup);
        resolve_constraint_pair_paths(&mut config.distinct_trimmed_within, &field_path_lookup);
        resolve_constraint_usize_paths(&mut config.min_items, &field_path_lookup);
        resolve_constraint_usize_paths(&mut config.max_items, &field_path_lookup);
        resolve_constraint_usize_paths(&mut config.min_properties, &field_path_lookup);
        resolve_constraint_usize_paths(&mut config.max_properties, &field_path_lookup);
        resolve_constraint_usize_paths(&mut config.min_chars, &field_path_lookup);
        resolve_constraint_usize_paths(&mut config.max_chars, &field_path_lookup);
        resolve_constraint_string_paths(&mut config.formats, &field_path_lookup);
        resolve_constraint_string_paths(&mut config.patterns, &field_path_lookup);
        resolve_constraint_values_paths(&mut config.choices, &field_path_lookup);
        normalize_array_value_nested_path_constraints(
            &mut config.non_empty,
            &mut config.non_empty_if_present,
            &mut config.exactly_one_of,
            &mut config.at_least_one_of,
            &mut config.requires,
            &mut config.conflicts_with,
            &mut config.required_unless_present,
            &mut config.distinct_trimmed_within,
            &field_path_lookup,
            &array_field_paths,
        );
        normalize_array_value_constraints(
            &mut config.trim,
            &mut config.trim_suffix,
            &mut config.minimums,
            &mut config.maximums,
            &mut config.exclusive_minimums,
            &mut config.exclusive_maximums,
            &mut config.min_properties,
            &mut config.max_properties,
            &mut config.min_chars,
            &mut config.max_chars,
            &mut config.formats,
            &mut config.patterns,
            &mut config.choices,
            &mut config.forbid_substrings,
            &mut config.distinct_trimmed,
            &mut config.input_field_metadata,
            &field_path_lookup,
            &array_field_paths,
        );
    }
    let enum_field_rule = serde_rename_all_fields_rule(&input.attrs)?;
    if let Data::Enum(data_enum) = &input.data {
        for variant in &data_enum.variants {
            let mut variant_config = normalized_input_variant_config(variant, enum_field_rule)?;
            for spec in &mut variant_config.input_paths {
                spec.optional = true;
            }
            for spec in &mut variant_config.input_networks {
                spec.optional = true;
            }
            config.input_paths.extend(variant_config.input_paths);
            config.input_networks.extend(variant_config.input_networks);
        }
    }
    let schema_metadata_fn =
        expand_schema_metadata_fn(&input.attrs, &input.data, &config, |variant, prefix| {
            let config = normalized_input_variant_config(variant, enum_field_rule)?;
            let mut calls = constraint_schema_metadata_calls(prefix, &config)?;
            calls.extend(constraint_relation_metadata_calls(prefix, &config)?);
            calls.extend(input_default_schema_metadata_calls(
                prefix,
                &config.input_defaults,
            ));
            Ok(calls)
        })?;
    let flatten_shape_post_parse_expr =
        expand_flatten_shape_post_parse_tokens(&input.attrs, &input.data)?;
    let (
        enum_helper_fn,
        variant_parse_error_remap_expr,
        variant_post_parse_normalize_expr,
        variant_validate_arms,
        variant_validate_error_remap_expr,
    ) = match &input.data {
        Data::Enum(data_enum) => (
            expand_input_shape_enum_normalize_fn(&data_enum.variants, enum_field_rule)?,
            expand_input_shape_enum_parse_error_remap_expr(&data_enum.variants, enum_field_rule)?,
            expand_input_shape_enum_post_parse_normalize_expr(
                &data_enum.variants,
                enum_field_rule,
            )?,
            data_enum
                .variants
                .iter()
                .filter_map(|variant| {
                    expand_input_shape_variant_validation_arm(variant, enum_field_rule).transpose()
                })
                .collect::<Result<Vec<_>>>()?,
            expand_input_shape_enum_validate_error_remap_expr(
                &data_enum.variants,
                enum_field_rule,
            )?,
        ),
        Data::Struct(_) => (
            quote! {
                fn __macro_normalize_enum_input(
                    input: serde_json::Value,
                ) -> ::agena_plugin_sdk::Result<serde_json::Value> {
                    Ok(input)
                }
            },
            quote! { __macro_parse_result },
            quote! { parsed },
            Vec::new(),
            quote! { __macro_validate_result },
        ),
        _ => {
            return Err(syn::Error::new_spanned(
                name,
                "ToolInput can only be derived for enums or structs",
            ));
        }
    };

    let struct_flatten_shapes = struct_flatten_shape_types(&input.data)?;
    let struct_nested_shapes = struct_nested_shape_fields(&input.attrs, &input.data)?;
    let built_in_normalize_expr = built_in_normalization_tokens(
        quote! { &mut input },
        &config.trim,
        &config.trim_suffix,
        &struct_flatten_shapes,
        &struct_nested_shapes,
    );
    let built_in_post_parse_normalize_expr = built_in_post_parse_normalization_tokens(
        &config.trim,
        &config.trim_suffix,
        &struct_flatten_shapes,
        &struct_nested_shapes,
    );
    let normalize_expr = config
        .normalize
        .as_ref()
        .map(|path| quote! { #path(input)? })
        .unwrap_or_else(|| quote! { input });
    let validate_expr = config
        .validate
        .as_ref()
        .map(|path| quote! { #path(&parsed)?; })
        .unwrap_or_default();
    let built_in_validate_expr = built_in_validation_tokens(
        quote! { parsed },
        &config.non_empty,
        &config.non_empty_if_present,
        &config.minimums,
        &config.maximums,
        &config.exclusive_minimums,
        &config.exclusive_maximums,
        &config.exactly_one_of,
        &config.at_least_one_of,
        &config.requires,
        &config.conflicts_with,
        &config.required_unless_present,
        &config.forbid_substrings,
        &config.distinct_trimmed,
        &config.distinct_trimmed_within,
        &config.min_items,
        &config.max_items,
        &config.min_properties,
        &config.max_properties,
        &config.min_chars,
        &config.max_chars,
        &config.formats,
        &config.patterns,
        &config.choices,
        &struct_flatten_shapes,
        &struct_nested_shapes,
    );
    let dispatch_tool_invoke_fn = expand_input_dispatch_fn(&input.data, &config)?;
    let input_paths_expr = expand_input_paths_expr(&input.attrs, &input.data, &config.input_paths)?;
    let input_networks_expr =
        expand_input_networks_expr(&input.attrs, &input.data, &config.input_networks)?;
    let input_tags_expr = expand_input_tags_expr(
        &input.attrs,
        &input.data,
        &config.input_paths,
        &config.input_networks,
    )?;
    let input_example_expr =
        expand_input_example_expr(config.example.as_ref(), &config.input_field_metadata);
    let input_root_default_insert_expr =
        expand_input_root_default_insert_tokens(config.default, config.default_expr.as_ref());
    let input_alias_normalize_expr = expand_input_alias_normalize_tokens(&config.input_aliases);
    let input_default_insert_expr = expand_input_default_insert_tokens(&config.input_defaults);
    let nested_shape_input_normalize_expr =
        expand_nested_shape_schema_normalize_expr(&struct_nested_shapes);
    let flatten_shape_input_normalize_expr =
        expand_flatten_shape_schema_normalize_expr(&struct_flatten_shapes);
    let input_root_default_schema_metadata_expr = expand_input_root_default_schema_metadata_tokens(
        config.default,
        config.default_expr.as_ref(),
    );
    let input_default_schema_metadata_expr =
        expand_input_default_schema_metadata_tokens(&config.input_defaults);
    let input_root_example_schema_metadata_expr =
        expand_input_root_example_schema_metadata_tokens(config.example.as_ref());
    let input_error_path_mappings = input_error_path_mappings(&input.attrs, &input.data)?;
    let parse_error_remap_expr = expand_result_path_remap_expr(
        &format_ident!("__macro_parse_result"),
        &input_error_path_mappings,
    );
    let validate_error_remap_expr = expand_result_path_remap_expr(
        &format_ident!("__macro_validate_result"),
        &input_error_path_mappings,
    );

    Ok(quote! {
        impl #name {
            #enum_helper_fn
            #schema_metadata_fn
            #dispatch_tool_invoke_fn

            pub fn input_schema() -> serde_json::Value {
                static __AGENA_TOOL_INPUT_SCHEMA: ::std::sync::OnceLock<serde_json::Value> =
                    ::std::sync::OnceLock::new();
                __AGENA_TOOL_INPUT_SCHEMA.get_or_init(|| {
                    let mut schema = ::agena_plugin_sdk::macro_support::json_schema_for::<Self>();
                    Self::__macro_apply_schema_metadata(&mut schema);
                    {
                        let schema = &mut schema;
                        #input_root_default_schema_metadata_expr
                        #input_root_example_schema_metadata_expr
                        #input_default_schema_metadata_expr
                    }
                    schema
                }).clone()
            }

            pub fn input_paths() -> Vec<::agena_plugin_sdk::InputPathSpec> {
                #input_paths_expr
            }

            pub fn input_networks() -> Vec<::agena_plugin_sdk::InputNetworkSpec> {
                #input_networks_expr
            }

            pub fn input_tags() -> Vec<::agena_plugin_sdk::ToolTag> {
                #input_tags_expr
            }

            pub fn input_example() -> Option<::agena_plugin_sdk::serde_json::Value> {
                #input_example_expr
            }

            pub fn parse_input(
                input: serde_json::Value,
            ) -> ::agena_plugin_sdk::Result<Self> {
                let mut input = input;
                #input_root_default_insert_expr
                #input_alias_normalize_expr
                #input_default_insert_expr
                #nested_shape_input_normalize_expr
                #flatten_shape_input_normalize_expr
                #built_in_normalize_expr
                let input = #normalize_expr;
                let input = Self::__macro_normalize_enum_input(input)?;
                let schema = Self::input_schema();
                let __macro_parse_result = ::agena_plugin_sdk::macro_support::parse_typed_json_value_with_field_suggestions::<Self>(
                    input.clone(),
                    &schema,
                    "field",
                );
                let __macro_parse_result = #variant_parse_error_remap_expr;
                let parsed = #parse_error_remap_expr;
                let parsed = #built_in_post_parse_normalize_expr;
                let parsed = #variant_post_parse_normalize_expr;
                let parsed = #flatten_shape_post_parse_expr;
                let __macro_validate_result: ::agena_plugin_sdk::Result<()> = (|| {
                    match &parsed {
                        #(#variant_validate_arms)*
                        _ => {}
                    }
                    #built_in_validate_expr
                    #validate_expr
                    Ok(())
                })();
                let __macro_validate_result = #variant_validate_error_remap_expr;
                let () = #validate_error_remap_expr;
                Ok(parsed)
            }

            pub fn parse_json_str(
                input: &str,
            ) -> ::agena_plugin_sdk::Result<Self> {
                let input = ::agena_plugin_sdk::macro_support::parse_json_value_str(input)?;
                Self::parse_input(input)
            }
        }

        impl ::agena_plugin_sdk::ToolInput for #name {
            fn input_schema() -> serde_json::Value {
                Self::input_schema()
            }

            fn parse_input(input: serde_json::Value) -> ::agena_plugin_sdk::Result<Self> {
                Self::parse_input(input)
            }

            fn input_paths() -> Vec<::agena_plugin_sdk::InputPathSpec> {
                Self::input_paths()
            }

            fn input_networks() -> Vec<::agena_plugin_sdk::InputNetworkSpec> {
                Self::input_networks()
            }

            fn input_tags() -> Vec<::agena_plugin_sdk::ToolTag> {
                Self::input_tags()
            }

            fn input_example() -> Option<::agena_plugin_sdk::serde_json::Value> {
                Self::input_example()
            }

            fn parse_json_str(input: &str) -> ::agena_plugin_sdk::Result<Self> {
                Self::parse_json_str(input)
            }
        }
    })
}
