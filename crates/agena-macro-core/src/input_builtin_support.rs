//! Builtin argument kinds (path, network, stream, ...) and their handling.

use std::collections::BTreeSet;

use quote::quote;
use syn::{Expr, LitStr, Type};

use super::{
    NestedInputShapeField, PathPairConstraint, PathStringConstraint, PathStringsConstraint,
    PathUsizeConstraint, PathValueConstraint, PathValuesConstraint, PluginInputFieldMetadata,
    SchemaConstraintSource, expand_input_shape_resolved_path_expr,
};

pub fn built_in_normalization_tokens(
    target: proc_macro2::TokenStream,
    trim: &[LitStr],
    trim_suffix: &[PathStringConstraint],
    flatten_shapes: &[Type],
    nested_shapes: &[NestedInputShapeField],
) -> proc_macro2::TokenStream {
    let trim_expr = if trim.is_empty() {
        quote! {}
    } else if flatten_shapes.is_empty() && nested_shapes.is_empty() {
        quote! {
            ::agena_plugin_sdk::macro_support::normalize_trim_paths(
                #target,
                &[#(#trim),*],
            );
        }
    } else {
        let resolved_paths = trim
            .iter()
            .map(|path| expand_input_shape_resolved_path_expr(flatten_shapes, nested_shapes, path));
        quote! {
            let __paths = vec![#(#resolved_paths),*];
            let __path_refs = __paths.iter().map(::std::string::String::as_str).collect::<Vec<_>>();
            ::agena_plugin_sdk::macro_support::normalize_trim_paths(#target, __path_refs.as_slice());
        }
    };
    let trim_suffix_exprs = trim_suffix.iter().map(|constraint| {
        let path = &constraint.path;
        let value = &constraint.value;
        if flatten_shapes.is_empty() && nested_shapes.is_empty() {
            quote! {
                ::agena_plugin_sdk::macro_support::normalize_trim_suffix_path(
                    #target,
                    #path,
                    #value,
                );
            }
        } else {
            let resolved_path =
                expand_input_shape_resolved_path_expr(flatten_shapes, nested_shapes, path);
            quote! {
                let __path = #resolved_path;
                ::agena_plugin_sdk::macro_support::normalize_trim_suffix_path(
                    #target,
                    __path.as_str(),
                    #value,
                );
            }
        }
    });
    quote! {
        #trim_expr
        #(#trim_suffix_exprs)*
    }
}

pub fn built_in_post_parse_normalization_tokens(
    trim: &[LitStr],
    trim_suffix: &[PathStringConstraint],
    flatten_shapes: &[Type],
    nested_shapes: &[NestedInputShapeField],
) -> proc_macro2::TokenStream {
    if trim.is_empty() && trim_suffix.is_empty() {
        quote! { parsed }
    } else {
        let normalize_expr = built_in_normalization_tokens(
            quote! { input },
            trim,
            trim_suffix,
            flatten_shapes,
            nested_shapes,
        );
        quote! {
            ::agena_plugin_sdk::macro_support::normalize_typed_json_value(&parsed, |input| {
                #normalize_expr
            })?
        }
    }
}

// These slices are independent validation dimensions assembled by the derive parser.
#[allow(clippy::too_many_arguments)]
pub fn built_in_validation_tokens(
    target: proc_macro2::TokenStream,
    non_empty: &[LitStr],
    non_empty_if_present: &[LitStr],
    minimums: &[PathValueConstraint],
    maximums: &[PathValueConstraint],
    exclusive_minimums: &[PathValueConstraint],
    exclusive_maximums: &[PathValueConstraint],
    exactly_one_of: &[Vec<LitStr>],
    at_least_one_of: &[Vec<LitStr>],
    requires: &[PathPairConstraint],
    conflicts_with: &[PathPairConstraint],
    required_unless_present: &[PathPairConstraint],
    forbid_substrings: &[PathStringsConstraint],
    distinct_trimmed: &[LitStr],
    distinct_trimmed_within: &[PathPairConstraint],
    min_items: &[PathUsizeConstraint],
    max_items: &[PathUsizeConstraint],
    min_properties: &[PathUsizeConstraint],
    max_properties: &[PathUsizeConstraint],
    min_chars: &[PathUsizeConstraint],
    max_chars: &[PathUsizeConstraint],
    formats: &[PathStringConstraint],
    patterns: &[PathStringConstraint],
    choices: &[PathValuesConstraint],
    flatten_shapes: &[Type],
    nested_shapes: &[NestedInputShapeField],
) -> proc_macro2::TokenStream {
    let non_empty_expr = if non_empty.is_empty() {
        quote! {}
    } else if flatten_shapes.is_empty() && nested_shapes.is_empty() {
        quote! {
            ::agena_plugin_sdk::macro_support::validate_non_empty_paths(
                &#target,
                &[#(#non_empty),*],
            )?;
        }
    } else {
        let resolved_paths = non_empty
            .iter()
            .map(|path| expand_input_shape_resolved_path_expr(flatten_shapes, nested_shapes, path));
        quote! {
            let __paths = vec![#(#resolved_paths),*];
            let __path_refs = __paths.iter().map(::std::string::String::as_str).collect::<Vec<_>>();
            ::agena_plugin_sdk::macro_support::validate_non_empty_paths(
                &#target,
                __path_refs.as_slice(),
            )?;
        }
    };
    let non_empty_if_present_expr = if non_empty_if_present.is_empty() {
        quote! {}
    } else if flatten_shapes.is_empty() && nested_shapes.is_empty() {
        quote! {
            ::agena_plugin_sdk::macro_support::validate_non_empty_if_present_paths(
                &#target,
                &[#(#non_empty_if_present),*],
            )?;
        }
    } else {
        let resolved_paths = non_empty_if_present
            .iter()
            .map(|path| expand_input_shape_resolved_path_expr(flatten_shapes, nested_shapes, path));
        quote! {
            let __paths = vec![#(#resolved_paths),*];
            let __path_refs = __paths.iter().map(::std::string::String::as_str).collect::<Vec<_>>();
            ::agena_plugin_sdk::macro_support::validate_non_empty_if_present_paths(
                &#target,
                __path_refs.as_slice(),
            )?;
        }
    };
    let mut numeric_exprs = Vec::new();
    for (rule, constraints) in [
        (&NUMERIC_RULES[0], minimums),
        (&NUMERIC_RULES[1], maximums),
        (&NUMERIC_RULES[2], exclusive_minimums),
        (&NUMERIC_RULES[3], exclusive_maximums),
    ] {
        numeric_exprs.extend(constraints.iter().map(|constraint| {
            rule.validation_call(&target, constraint, flatten_shapes, nested_shapes)
        }));
    }
    let exactly_one_of_exprs = exactly_one_of.iter().map(|group| {
        if flatten_shapes.is_empty() && nested_shapes.is_empty() {
            quote! {
                ::agena_plugin_sdk::macro_support::validate_exactly_one_of_paths(
                    &#target,
                    &[#(#group),*],
                )?;
            }
        } else {
            let resolved_paths = group
                .iter()
                .map(|path| expand_input_shape_resolved_path_expr(flatten_shapes, nested_shapes, path));
            quote! {
                let __paths = vec![#(#resolved_paths),*];
                let __path_refs = __paths.iter().map(::std::string::String::as_str).collect::<Vec<_>>();
                ::agena_plugin_sdk::macro_support::validate_exactly_one_of_paths(
                    &#target,
                    __path_refs.as_slice(),
                )?;
            }
        }
    });
    let at_least_one_of_exprs = at_least_one_of.iter().map(|group| {
        if flatten_shapes.is_empty() && nested_shapes.is_empty() {
            quote! {
                ::agena_plugin_sdk::macro_support::validate_at_least_one_of_paths(
                    &#target,
                    &[#(#group),*],
                )?;
            }
        } else {
            let resolved_paths = group
                .iter()
                .map(|path| expand_input_shape_resolved_path_expr(flatten_shapes, nested_shapes, path));
            quote! {
                let __paths = vec![#(#resolved_paths),*];
                let __path_refs = __paths.iter().map(::std::string::String::as_str).collect::<Vec<_>>();
                ::agena_plugin_sdk::macro_support::validate_at_least_one_of_paths(
                    &#target,
                    __path_refs.as_slice(),
                )?;
            }
        }
    });
    let requires_exprs = requires.iter().map(|constraint| {
        let left = &constraint.left;
        let right = &constraint.right;
        if flatten_shapes.is_empty() && nested_shapes.is_empty() {
            quote! {
                ::agena_plugin_sdk::macro_support::validate_requires_path(
                    &#target,
                    #left,
                    #right,
                )?;
            }
        } else {
            let resolved_left =
                expand_input_shape_resolved_path_expr(flatten_shapes, nested_shapes, left);
            let resolved_right =
                expand_input_shape_resolved_path_expr(flatten_shapes, nested_shapes, right);
            quote! {
                let __left = #resolved_left;
                let __right = #resolved_right;
                ::agena_plugin_sdk::macro_support::validate_requires_path(
                    &#target,
                    __left.as_str(),
                    __right.as_str(),
                )?;
            }
        }
    });
    let conflicts_exprs = conflicts_with.iter().map(|constraint| {
        let left = &constraint.left;
        let right = &constraint.right;
        if flatten_shapes.is_empty() && nested_shapes.is_empty() {
            quote! {
                ::agena_plugin_sdk::macro_support::validate_conflicts_with_path(
                    &#target,
                    #left,
                    #right,
                )?;
            }
        } else {
            let resolved_left =
                expand_input_shape_resolved_path_expr(flatten_shapes, nested_shapes, left);
            let resolved_right =
                expand_input_shape_resolved_path_expr(flatten_shapes, nested_shapes, right);
            quote! {
                let __left = #resolved_left;
                let __right = #resolved_right;
                ::agena_plugin_sdk::macro_support::validate_conflicts_with_path(
                    &#target,
                    __left.as_str(),
                    __right.as_str(),
                )?;
            }
        }
    });
    let required_unless_exprs = required_unless_present.iter().map(|constraint| {
        let left = &constraint.left;
        let right = &constraint.right;
        if flatten_shapes.is_empty() && nested_shapes.is_empty() {
            quote! {
                ::agena_plugin_sdk::macro_support::validate_required_unless_present_path(
                    &#target,
                    #left,
                    #right,
                )?;
            }
        } else {
            let resolved_left =
                expand_input_shape_resolved_path_expr(flatten_shapes, nested_shapes, left);
            let resolved_right =
                expand_input_shape_resolved_path_expr(flatten_shapes, nested_shapes, right);
            quote! {
                let __left = #resolved_left;
                let __right = #resolved_right;
                ::agena_plugin_sdk::macro_support::validate_required_unless_present_path(
                    &#target,
                    __left.as_str(),
                    __right.as_str(),
                )?;
            }
        }
    });
    let forbid_substrings_exprs = forbid_substrings.iter().map(|constraint| {
        let path = &constraint.path;
        let values = &constraint.values;
        if flatten_shapes.is_empty() && nested_shapes.is_empty() {
            quote! {
                ::agena_plugin_sdk::macro_support::validate_forbid_substrings_path(
                    &#target,
                    #path,
                    &[#(#values),*],
                )?;
            }
        } else {
            let resolved_path =
                expand_input_shape_resolved_path_expr(flatten_shapes, nested_shapes, path);
            quote! {
                let __path = #resolved_path;
                ::agena_plugin_sdk::macro_support::validate_forbid_substrings_path(
                    &#target,
                    __path.as_str(),
                    &[#(#values),*],
                )?;
            }
        }
    });
    let distinct_trimmed_exprs = distinct_trimmed.iter().map(|path| {
        if flatten_shapes.is_empty() && nested_shapes.is_empty() {
            quote! {
                ::agena_plugin_sdk::macro_support::validate_distinct_trimmed_path(
                    &#target,
                    #path,
                )?;
            }
        } else {
            let resolved_path =
                expand_input_shape_resolved_path_expr(flatten_shapes, nested_shapes, path);
            quote! {
                let __path = #resolved_path;
                ::agena_plugin_sdk::macro_support::validate_distinct_trimmed_path(
                    &#target,
                    __path.as_str(),
                )?;
            }
        }
    });
    let distinct_trimmed_within_exprs = distinct_trimmed_within.iter().map(|constraint| {
        let path = &constraint.left;
        let scope = &constraint.right;
        if flatten_shapes.is_empty() && nested_shapes.is_empty() {
            quote! {
                ::agena_plugin_sdk::macro_support::validate_distinct_trimmed_within_path(
                    &#target,
                    #path,
                    #scope,
                )?;
            }
        } else {
            let resolved_path =
                expand_input_shape_resolved_path_expr(flatten_shapes, nested_shapes, path);
            let resolved_scope =
                expand_input_shape_resolved_path_expr(flatten_shapes, nested_shapes, scope);
            quote! {
                let __path = #resolved_path;
                let __scope = #resolved_scope;
                ::agena_plugin_sdk::macro_support::validate_distinct_trimmed_within_path(
                    &#target,
                    __path.as_str(),
                    __scope.as_str(),
                )?;
            }
        }
    });
    let min_items_exprs = min_items.iter().map(|constraint| {
        let path = &constraint.path;
        let value = constraint.value;
        if flatten_shapes.is_empty() && nested_shapes.is_empty() {
            quote! {
                ::agena_plugin_sdk::macro_support::validate_min_items_path(
                    &#target,
                    #path,
                    #value,
                )?;
            }
        } else {
            let resolved_path =
                expand_input_shape_resolved_path_expr(flatten_shapes, nested_shapes, path);
            quote! {
                let __path = #resolved_path;
                ::agena_plugin_sdk::macro_support::validate_min_items_path(
                    &#target,
                    __path.as_str(),
                    #value,
                )?;
            }
        }
    });
    let max_items_exprs = max_items.iter().map(|constraint| {
        let path = &constraint.path;
        let value = constraint.value;
        if flatten_shapes.is_empty() && nested_shapes.is_empty() {
            quote! {
                ::agena_plugin_sdk::macro_support::validate_max_items_path(
                    &#target,
                    #path,
                    #value,
                )?;
            }
        } else {
            let resolved_path =
                expand_input_shape_resolved_path_expr(flatten_shapes, nested_shapes, path);
            quote! {
                let __path = #resolved_path;
                ::agena_plugin_sdk::macro_support::validate_max_items_path(
                    &#target,
                    __path.as_str(),
                    #value,
                )?;
            }
        }
    });
    let min_properties_exprs = min_properties.iter().map(|constraint| {
        let path = &constraint.path;
        let value = constraint.value;
        if flatten_shapes.is_empty() && nested_shapes.is_empty() {
            quote! {
                ::agena_plugin_sdk::macro_support::validate_min_properties_path(
                    &#target,
                    #path,
                    #value,
                )?;
            }
        } else {
            let resolved_path =
                expand_input_shape_resolved_path_expr(flatten_shapes, nested_shapes, path);
            quote! {
                let __path = #resolved_path;
                ::agena_plugin_sdk::macro_support::validate_min_properties_path(
                    &#target,
                    __path.as_str(),
                    #value,
                )?;
            }
        }
    });
    let max_properties_exprs = max_properties.iter().map(|constraint| {
        let path = &constraint.path;
        let value = constraint.value;
        if flatten_shapes.is_empty() && nested_shapes.is_empty() {
            quote! {
                ::agena_plugin_sdk::macro_support::validate_max_properties_path(
                    &#target,
                    #path,
                    #value,
                )?;
            }
        } else {
            let resolved_path =
                expand_input_shape_resolved_path_expr(flatten_shapes, nested_shapes, path);
            quote! {
                let __path = #resolved_path;
                ::agena_plugin_sdk::macro_support::validate_max_properties_path(
                    &#target,
                    __path.as_str(),
                    #value,
                )?;
            }
        }
    });
    let min_chars_exprs = min_chars.iter().map(|constraint| {
        let path = &constraint.path;
        let value = constraint.value;
        if flatten_shapes.is_empty() && nested_shapes.is_empty() {
            quote! {
                ::agena_plugin_sdk::macro_support::validate_min_chars_path(
                    &#target,
                    #path,
                    #value,
                )?;
            }
        } else {
            let resolved_path =
                expand_input_shape_resolved_path_expr(flatten_shapes, nested_shapes, path);
            quote! {
                let __path = #resolved_path;
                ::agena_plugin_sdk::macro_support::validate_min_chars_path(
                    &#target,
                    __path.as_str(),
                    #value,
                )?;
            }
        }
    });
    let max_chars_exprs = max_chars.iter().map(|constraint| {
        let path = &constraint.path;
        let value = constraint.value;
        if flatten_shapes.is_empty() && nested_shapes.is_empty() {
            quote! {
                ::agena_plugin_sdk::macro_support::validate_max_chars_path(
                    &#target,
                    #path,
                    #value,
                )?;
            }
        } else {
            let resolved_path =
                expand_input_shape_resolved_path_expr(flatten_shapes, nested_shapes, path);
            quote! {
                let __path = #resolved_path;
                ::agena_plugin_sdk::macro_support::validate_max_chars_path(
                    &#target,
                    __path.as_str(),
                    #value,
                )?;
            }
        }
    });
    let format_exprs = formats.iter().map(|constraint| {
        let path = &constraint.path;
        let value = &constraint.value;
        if flatten_shapes.is_empty() && nested_shapes.is_empty() {
            quote! {
                ::agena_plugin_sdk::macro_support::validate_format_path(
                    &#target,
                    #path,
                    #value,
                )?;
            }
        } else {
            let resolved_path =
                expand_input_shape_resolved_path_expr(flatten_shapes, nested_shapes, path);
            quote! {
                let __path = #resolved_path;
                ::agena_plugin_sdk::macro_support::validate_format_path(
                    &#target,
                    __path.as_str(),
                    #value,
                )?;
            }
        }
    });
    let pattern_exprs = patterns.iter().map(|constraint| {
        let path = &constraint.path;
        let value = &constraint.value;
        if flatten_shapes.is_empty() && nested_shapes.is_empty() {
            quote! {
                ::agena_plugin_sdk::macro_support::validate_pattern_path(
                    &#target,
                    #path,
                    #value,
                )?;
            }
        } else {
            let resolved_path =
                expand_input_shape_resolved_path_expr(flatten_shapes, nested_shapes, path);
            quote! {
                let __path = #resolved_path;
                ::agena_plugin_sdk::macro_support::validate_pattern_path(
                    &#target,
                    __path.as_str(),
                    #value,
                )?;
            }
        }
    });
    let choices_exprs = choices.iter().map(|constraint| {
        let path = &constraint.path;
        let values = &constraint.values;
        if flatten_shapes.is_empty() && nested_shapes.is_empty() {
            quote! {
                ::agena_plugin_sdk::macro_support::validate_allowed_values_path(
                    &#target,
                    #path,
                    &[#(::agena_plugin_sdk::serde_json::json!(#values)),*],
                )?;
            }
        } else {
            let resolved_path =
                expand_input_shape_resolved_path_expr(flatten_shapes, nested_shapes, path);
            quote! {
                let __path = #resolved_path;
                ::agena_plugin_sdk::macro_support::validate_allowed_values_path(
                    &#target,
                    __path.as_str(),
                    &[#(::agena_plugin_sdk::serde_json::json!(#values)),*],
                )?;
            }
        }
    });
    quote! {
        #(#min_items_exprs)*
        #(#max_items_exprs)*
        #(#min_properties_exprs)*
        #(#max_properties_exprs)*
        #(#min_chars_exprs)*
        #(#max_chars_exprs)*
        #(#format_exprs)*
        #(#pattern_exprs)*
        #(#choices_exprs)*
        #non_empty_expr
        #non_empty_if_present_expr
        #(#numeric_exprs)*
        #(#forbid_substrings_exprs)*
        #(#distinct_trimmed_exprs)*
        #(#distinct_trimmed_within_exprs)*
        #(#exactly_one_of_exprs)*
        #(#at_least_one_of_exprs)*
        #(#requires_exprs)*
        #(#conflicts_exprs)*
        #(#required_unless_exprs)*
    }
}

/// One numeric rule connects its source, field override, schema keyword and
/// runtime validator. Both generation paths encode the original expression as
/// JSON, preserving integer precision and the author's source spans.
struct NumericRule {
    schema_key: &'static str,
    validator: &'static str,
    source: fn(&dyn SchemaConstraintSource) -> &[PathValueConstraint],
    field: fn(&PluginInputFieldMetadata) -> Option<&Expr>,
    item: fn(&PluginInputFieldMetadata) -> Option<&Expr>,
}

const NUMERIC_RULES: [NumericRule; 4] = [
    NumericRule {
        schema_key: "minimum",
        validator: "validate_minimum_path",
        source: |source| source.minimums(),
        field: |field| field.minimum.as_ref(),
        item: |field| field.item_minimum.as_ref(),
    },
    NumericRule {
        schema_key: "maximum",
        validator: "validate_maximum_path",
        source: |source| source.maximums(),
        field: |field| field.maximum.as_ref(),
        item: |field| field.item_maximum.as_ref(),
    },
    NumericRule {
        schema_key: "exclusiveMinimum",
        validator: "validate_exclusive_minimum_path",
        source: |source| source.exclusive_minimums(),
        field: |field| field.exclusive_minimum.as_ref(),
        item: |field| field.item_exclusive_minimum.as_ref(),
    },
    NumericRule {
        schema_key: "exclusiveMaximum",
        validator: "validate_exclusive_maximum_path",
        source: |source| source.exclusive_maximums(),
        field: |field| field.exclusive_maximum.as_ref(),
        item: |field| field.item_exclusive_maximum.as_ref(),
    },
];

fn schema_number_expression(value: &Expr) -> proc_macro2::TokenStream {
    quote!(::agena_plugin_sdk::serde_json::json!(#value))
}

impl NumericRule {
    fn schema_call(&self, pointer: &LitStr, value: &Expr) -> proc_macro2::TokenStream {
        let key = self.schema_key;
        let value = schema_number_expression(value);
        quote! {
            ::agena_plugin_sdk::macro_support::set_schema_number_metadata(
                schema, #pointer, #key, #value,
            );
        }
    }

    fn validation_call(
        &self,
        target: &proc_macro2::TokenStream,
        constraint: &PathValueConstraint,
        flatten_shapes: &[Type],
        nested_shapes: &[NestedInputShapeField],
    ) -> proc_macro2::TokenStream {
        let path = &constraint.path;
        let validator = syn::Ident::new(self.validator, path.span());
        let value = schema_number_expression(&constraint.value);
        if flatten_shapes.is_empty() && nested_shapes.is_empty() {
            quote! {
                ::agena_plugin_sdk::macro_support::#validator(&#target, #path, &#value)?;
            }
        } else {
            let resolved_path =
                expand_input_shape_resolved_path_expr(flatten_shapes, nested_shapes, path);
            quote! {
                let __path = #resolved_path;
                ::agena_plugin_sdk::macro_support::#validator(
                    &#target, __path.as_str(), &#value,
                )?;
            }
        }
    }
}

pub(crate) fn numeric_field_schema_calls(
    pointer: &LitStr,
    metadata: &PluginInputFieldMetadata,
) -> Vec<proc_macro2::TokenStream> {
    let mut calls = Vec::new();
    for rule in &NUMERIC_RULES {
        if let Some(value) = (rule.field)(metadata) {
            calls.push(rule.schema_call(pointer, value));
        }
    }
    let item_pointer = LitStr::new(&format!("{}/items", pointer.value()), metadata.path.span());
    // Preserve the established generation order for item constraints.
    for index in [0, 2, 3, 1] {
        let rule = &NUMERIC_RULES[index];
        if let Some(value) = (rule.item)(metadata) {
            calls.push(rule.schema_call(&item_pointer, value));
        }
    }
    calls
}

pub(crate) fn numeric_constraint_schema_calls<C: SchemaConstraintSource>(
    prefix: &str,
    constraints: &C,
) -> syn::Result<Vec<proc_macro2::TokenStream>> {
    let mut calls = Vec::new();
    for rule in &NUMERIC_RULES {
        let mut overridden = BTreeSet::new();
        for metadata in constraints.input_field_metadata() {
            if (rule.field)(metadata).is_some() {
                overridden.insert(metadata.parse_path.value());
            }
            if (rule.item)(metadata).is_some() {
                overridden.insert(format!("{}[]", metadata.parse_path.value()));
            }
        }
        for constraint in (rule.source)(constraints) {
            if overridden.contains(&constraint.path.value()) {
                continue;
            }
            let pointer = super::input_schema_path_support::schema_pointer_from_logical_path(
                prefix,
                &constraint.path.value(),
            )?;
            let pointer = LitStr::new(&pointer, constraint.path.span());
            calls.push(rule.schema_call(&pointer, &constraint.value));
        }
    }
    Ok(calls)
}
