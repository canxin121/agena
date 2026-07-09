use quote::{format_ident, quote};
use syn::punctuated::Punctuated;
use syn::{Fields, LitStr, Result, Token, Type, Variant};

use super::{
    NestedInputShapeField, PathStringConstraint, SerdeRenameRule, built_in_normalization_tokens,
    built_in_post_parse_normalization_tokens, built_in_validation_tokens,
    expand_flatten_shape_input_keys_expr, expand_flatten_shape_schema_normalize_expr,
    expand_input_alias_normalize_tokens, expand_input_default_insert_tokens,
    expand_nested_shape_input_keys_expr, expand_nested_shape_schema_normalize_expr,
    flatten_shape_type, input_keys_for_parse_path, input_variant_action_name,
    named_field_object_insert_tokens, nested_input_shape_field, normalized_input_variant_config,
    serde_rename_all_rule,
};

pub(crate) fn expand_input_shape_enum_normalize_fn(
    variants: &Punctuated<Variant, Token![,]>,
    enum_field_rule: Option<SerdeRenameRule>,
) -> Result<proc_macro2::TokenStream> {
    struct EnumNormalizeVariant {
        action: LitStr,
        default_when_empty: bool,
        infer_when_present: Vec<LitStr>,
        drop_keys: Vec<LitStr>,
        trim: Vec<LitStr>,
        trim_suffix: Vec<PathStringConstraint>,
        input_aliases: Vec<super::PluginInputFieldAliasSpec>,
        input_defaults: Vec<super::PluginInputFieldDefaultSpec>,
        nested_shapes: Vec<NestedInputShapeField>,
        flatten_shapes: Vec<Type>,
    }

    let mut normalize_variants = Vec::new();
    let mut action_candidates = Vec::new();
    for variant in variants {
        let config = normalized_input_variant_config(variant, enum_field_rule)?;
        let variant_field_rule = serde_rename_all_rule(&variant.attrs)?.or(enum_field_rule);
        let nested_shapes = variant
            .fields
            .iter()
            .filter_map(|field| nested_input_shape_field(field, variant_field_rule).transpose())
            .collect::<Result<Vec<_>>>()?;
        let flatten_shapes = variant
            .fields
            .iter()
            .filter_map(|field| flatten_shape_type(field).transpose())
            .collect::<Result<Vec<_>>>()?;
        let action = input_variant_action_name(variant, &config);
        action_candidates.push(action.clone());
        if config.default_when_empty
            || !config.infer_when_present.is_empty()
            || !config.drop_keys.is_empty()
            || !config.trim.is_empty()
            || !config.trim_suffix.is_empty()
            || !config.input_aliases.is_empty()
            || !config.input_defaults.is_empty()
            || !nested_shapes.is_empty()
            || !flatten_shapes.is_empty()
        {
            normalize_variants.push(EnumNormalizeVariant {
                action: input_variant_action_name(variant, &config),
                default_when_empty: config.default_when_empty,
                infer_when_present: config.infer_when_present,
                drop_keys: config.drop_keys,
                trim: config.trim,
                trim_suffix: config.trim_suffix,
                input_aliases: config.input_aliases,
                input_defaults: config.input_defaults,
                nested_shapes,
                flatten_shapes,
            });
        }
    }

    if action_candidates.is_empty() {
        return Ok(quote! {
            fn __macro_normalize_enum_input(
                input: serde_json::Value,
            ) -> ::agena_plugin_sdk::Result<serde_json::Value> {
                Ok(input)
            }
        });
    }

    let default_actions = normalize_variants
        .iter()
        .filter(|variant| variant.default_when_empty)
        .map(|variant| variant.action.value())
        .collect::<Vec<_>>();
    if default_actions.len() > 1 {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            format!(
                "only one #[input(default_when_empty = true)] variant is allowed, found {}",
                default_actions.join(", ")
            ),
        ));
    }

    let default_empty_expr = normalize_variants
        .iter()
        .find(|variant| variant.default_when_empty)
        .map(|variant| {
            let action = &variant.action;
            quote! {
                if object.is_empty() {
                    object.insert(
                        "action".to_string(),
                        serde_json::Value::String(#action.to_string()),
                    );
                }
            }
        })
        .unwrap_or_default();

    let infer_match_exprs = normalize_variants
        .iter()
        .filter(|variant| !variant.infer_when_present.is_empty())
        .map(|variant| {
            let action = &variant.action;
            let static_keys = variant
                .infer_when_present
                .iter()
                .flat_map(|path| input_keys_for_parse_path(path, &variant.input_aliases))
                .collect::<Vec<_>>();
            let flattened_key_exprs = variant
                .infer_when_present
                .iter()
                .map(|path| expand_flatten_shape_input_keys_expr(&variant.flatten_shapes, path))
                .collect::<Vec<_>>();
            let nested_key_exprs = variant
                .infer_when_present
                .iter()
                .map(|path| expand_nested_shape_input_keys_expr(&variant.nested_shapes, path))
                .collect::<Vec<_>>();
            quote! {
                let mut __paths = vec![#(#static_keys.to_string()),*];
                #(
                    __paths.extend(#flattened_key_exprs);
                )*
                #(
                    __paths.extend(#nested_key_exprs);
                )*
                __paths.sort();
                __paths.dedup();
                let __input = serde_json::Value::Object(object.clone());
                if inferred_action.is_none()
                    && __paths.iter().any(|path| {
                        ::agena_plugin_sdk::macro_support::json_path_present(
                            &__input,
                            path.as_str(),
                        )
                    })
                {
                    inferred_action = Some(#action);
                }
            }
        });

    let drop_match_arms = normalize_variants
        .iter()
        .filter(|variant| !variant.drop_keys.is_empty())
        .map(|variant| {
            let action = &variant.action;
            let static_keys = variant
                .drop_keys
                .iter()
                .flat_map(|path| input_keys_for_parse_path(path, &variant.input_aliases))
                .collect::<Vec<_>>();
            let flattened_key_exprs = variant
                .drop_keys
                .iter()
                .map(|path| expand_flatten_shape_input_keys_expr(&variant.flatten_shapes, path))
                .collect::<Vec<_>>();
            let nested_key_exprs = variant
                .drop_keys
                .iter()
                .map(|path| expand_nested_shape_input_keys_expr(&variant.nested_shapes, path))
                .collect::<Vec<_>>();
            quote! {
                #action => {
                    let mut __paths = vec![#(#static_keys.to_string()),*];
                    #(
                        __paths.extend(#flattened_key_exprs);
                    )*
                    #(
                        __paths.extend(#nested_key_exprs);
                    )*
                    __paths.sort();
                    __paths.dedup();
                    let mut input = serde_json::Value::Object(object);
                    for path in __paths {
                        ::agena_plugin_sdk::macro_support::remove_json_path(
                            &mut input,
                            path.as_str(),
                        );
                    }
                    object = match input {
                        serde_json::Value::Object(object) => object,
                        other => {
                            return Err(::agena_plugin_sdk::PluginError::invalid_params(
                                format!(
                                    "enum input normalization expected object after drop_keys, found {}",
                                    other
                                ),
                            ));
                        }
                    };
                }
            }
        });

    let normalize_match_arms = normalize_variants
        .iter()
        .filter(|variant| {
            !variant.trim.is_empty()
                || !variant.trim_suffix.is_empty()
                || !variant.input_aliases.is_empty()
                || !variant.input_defaults.is_empty()
                || !variant.nested_shapes.is_empty()
                || !variant.flatten_shapes.is_empty()
        })
        .map(|variant| {
            let action = &variant.action;
            let alias_normalize_expr = expand_input_alias_normalize_tokens(&variant.input_aliases);
            let default_insert_expr = expand_input_default_insert_tokens(&variant.input_defaults);
            let nested_normalize_expr =
                expand_nested_shape_schema_normalize_expr(&variant.nested_shapes);
            let flatten_normalize_expr =
                expand_flatten_shape_schema_normalize_expr(&variant.flatten_shapes);
            let normalize_expr = built_in_normalization_tokens(
                quote! { &mut input },
                &variant.trim,
                &variant.trim_suffix,
                &variant.flatten_shapes,
                &variant.nested_shapes,
            );
            quote! {
                #action => {
                    let mut input = serde_json::Value::Object(object);
                    #alias_normalize_expr
                    #default_insert_expr
                    #nested_normalize_expr
                    #flatten_normalize_expr
                    #normalize_expr
                    return Ok(input);
                }
            }
        });

    Ok(quote! {
        fn __macro_normalize_enum_input(
            input: serde_json::Value,
        ) -> ::agena_plugin_sdk::Result<serde_json::Value> {
            let mut object = match input {
                serde_json::Value::Object(object) => object,
                other => return Ok(other),
            };
            let action_candidates = [#(#action_candidates),*];

            #default_empty_expr

            let action = match object.get("action").and_then(serde_json::Value::as_str) {
                Some(action) => match action {
                    other if action_candidates.iter().any(|candidate| *candidate == other) => other.to_string(),
                    other => {
                        let suggestions = ::agena_plugin_sdk::macro_support::suggest_name_candidates(
                            other,
                            action_candidates,
                            1,
                        );
                        let message = ::agena_plugin_sdk::macro_support::unknown_name_message(
                            "action",
                            other,
                            &suggestions,
                        );
                        return Err(::agena_plugin_sdk::PluginError::invalid_params(message));
                    }
                },
                None => {
                    let mut inferred_action: Option<&str> = None;
                    #(#infer_match_exprs)*
                    let Some(action) = inferred_action else {
                        return Ok(serde_json::Value::Object(object));
                    };
                    let action = action.to_string();
                    object.insert(
                        "action".to_string(),
                        serde_json::Value::String(action.clone()),
                    );
                    action
                }
            };

            match action.as_str() {
                #(#drop_match_arms)*
                _ => {}
            }

            match action.as_str() {
                #(#normalize_match_arms)*
                _ => {}
            }

            Ok(serde_json::Value::Object(object))
        }
    })
}

pub(crate) fn expand_input_shape_enum_post_parse_normalize_expr(
    variants: &Punctuated<Variant, Token![,]>,
    enum_field_rule: Option<SerdeRenameRule>,
) -> Result<proc_macro2::TokenStream> {
    let arms = variants
        .iter()
        .map(|variant| {
            expand_input_shape_variant_post_parse_normalize_arm(variant, enum_field_rule)
        })
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    if arms.is_empty() {
        Ok(quote! { parsed })
    } else {
        Ok(quote! {
            match parsed {
                #(#arms,)*
                other => other,
            }
        })
    }
}

fn expand_input_shape_variant_post_parse_normalize_arm(
    variant: &Variant,
    enum_field_rule: Option<SerdeRenameRule>,
) -> Result<Option<proc_macro2::TokenStream>> {
    let config = normalized_input_variant_config(variant, enum_field_rule)?;
    if config.trim.is_empty() && config.trim_suffix.is_empty() {
        return Ok(None);
    }
    let variant_name = &variant.ident;
    let variant_field_rule = serde_rename_all_rule(&variant.attrs)?.or(enum_field_rule);
    let nested_shapes = variant
        .fields
        .iter()
        .filter_map(|field| nested_input_shape_field(field, variant_field_rule).transpose())
        .collect::<Result<Vec<_>>>()?;
    let flatten_shapes = variant
        .fields
        .iter()
        .filter_map(|field| flatten_shape_type(field).transpose())
        .collect::<Result<Vec<_>>>()?;
    let normalize_expr = built_in_post_parse_normalization_tokens(
        &config.trim,
        &config.trim_suffix,
        &flatten_shapes,
        &nested_shapes,
    );
    match &variant.fields {
        Fields::Named(fields) => {
            let bindings = fields
                .named
                .iter()
                .map(|field| {
                    field.ident.clone().ok_or_else(|| {
                        syn::Error::new_spanned(
                            field,
                            "named tool input variant field is missing identifier",
                        )
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            Ok(Some(quote! {
                Self::#variant_name { #(#bindings),* } => {
                    let parsed = Self::#variant_name { #(#bindings),* };
                    #normalize_expr
                }
            }))
        }
        Fields::Unnamed(fields) => {
            let bindings = fields
                .unnamed
                .iter()
                .enumerate()
                .map(|(index, _)| format_ident!("value_{index}"))
                .collect::<Vec<_>>();
            Ok(Some(quote! {
                Self::#variant_name(#(#bindings),*) => {
                    let parsed = Self::#variant_name(#(#bindings),*);
                    #normalize_expr
                }
            }))
        }
        Fields::Unit => Ok(None),
    }
}

pub(crate) fn expand_input_shape_variant_validation_arm(
    variant: &Variant,
    enum_field_rule: Option<SerdeRenameRule>,
) -> Result<Option<proc_macro2::TokenStream>> {
    let config = normalized_input_variant_config(variant, enum_field_rule)?;
    let variant_field_rule = serde_rename_all_rule(&variant.attrs)?.or(enum_field_rule);
    let nested_shapes = variant
        .fields
        .iter()
        .filter_map(|field| nested_input_shape_field(field, variant_field_rule).transpose())
        .collect::<Result<Vec<_>>>()?;
    let flatten_shapes = variant
        .fields
        .iter()
        .filter_map(|field| flatten_shape_type(field).transpose())
        .collect::<Result<Vec<_>>>()?;
    let has_built_in_validation = !config.non_empty.is_empty()
        || !config.non_empty_if_present.is_empty()
        || !config.minimums.is_empty()
        || !config.maximums.is_empty()
        || !config.exclusive_minimums.is_empty()
        || !config.exclusive_maximums.is_empty()
        || !config.exactly_one_of.is_empty()
        || !config.at_least_one_of.is_empty()
        || !config.requires.is_empty()
        || !config.conflicts_with.is_empty()
        || !config.required_unless_present.is_empty()
        || !config.forbid_substrings.is_empty()
        || !config.distinct_trimmed.is_empty()
        || !config.distinct_trimmed_within.is_empty()
        || !config.min_items.is_empty()
        || !config.max_items.is_empty()
        || !config.min_properties.is_empty()
        || !config.max_properties.is_empty()
        || !config.min_chars.is_empty()
        || !config.max_chars.is_empty()
        || !config.formats.is_empty()
        || !config.patterns.is_empty()
        || !config.choices.is_empty();
    if config.validate.is_none() && !has_built_in_validation {
        return Ok(None);
    }

    let variant_name = &variant.ident;
    let built_in_validate_expr = built_in_validation_tokens(
        quote! { value },
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
        &flatten_shapes,
        &nested_shapes,
    );
    let arm = match &variant.fields {
        Fields::Named(fields) => {
            let bindings = fields
                .named
                .iter()
                .map(|field| {
                    field.ident.clone().ok_or_else(|| {
                        syn::Error::new_spanned(
                            field,
                            "named tool input variant field is missing identifier",
                        )
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            let field_inserts = named_field_object_insert_tokens(
                fields.named.iter().zip(bindings.iter()),
                "flattened tool input variant fields must serialize to objects",
                serde_rename_all_rule(&variant.attrs)?.or(enum_field_rule),
            )?;
            let validate_expr = config
                .validate
                .as_ref()
                .map(|path| quote! { #path(&value)?; })
                .unwrap_or_default();
            quote! {
                Self::#variant_name { #(#bindings),* } => {
                    let value = {
                        let mut object = serde_json::Map::new();
                        #(#field_inserts)*
                        serde_json::Value::Object(object)
                    };
                    #built_in_validate_expr
                    #validate_expr
                }
            }
        }
        Fields::Unnamed(fields) => {
            if fields.unnamed.len() == 1 {
                let validate_expr = config
                    .validate
                    .as_ref()
                    .map(|path| quote! { #path(value)?; })
                    .unwrap_or_default();
                quote! {
                    Self::#variant_name(value) => {
                        #built_in_validate_expr
                        #validate_expr
                    }
                }
            } else {
                let bindings = fields
                    .unnamed
                    .iter()
                    .enumerate()
                    .map(|(index, _)| format_ident!("value_{index}"))
                    .collect::<Vec<_>>();
                let validate_expr = config
                    .validate
                    .as_ref()
                    .map(|path| quote! { #path(&value)?; })
                    .unwrap_or_default();
                quote! {
                    Self::#variant_name(#(#bindings),*) => {
                        let value = serde_json::to_value((#(#bindings,)*))
                            .map_err(|err| ::agena_plugin_sdk::PluginError::invalid_params(err.to_string()))?;
                        #built_in_validate_expr
                        #validate_expr
                    }
                }
            }
        }
        Fields::Unit => {
            return Err(syn::Error::new_spanned(
                &variant.ident,
                "tool input variant validate hooks are not supported on unit variants",
            ));
        }
    };

    Ok(Some(arm))
}
