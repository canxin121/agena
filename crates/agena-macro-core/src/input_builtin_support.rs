use quote::quote;
use syn::{LitStr, Type};

use super::{
    NestedInputShapeField, PathPairConstraint, PathStringConstraint, PathStringsConstraint,
    PathUsizeConstraint, PathValueConstraint, PathValuesConstraint,
    expand_input_shape_resolved_path_expr,
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
    let minimum_exprs = minimums.iter().map(|constraint| {
        let path = &constraint.path;
        let value = &constraint.value;
        if flatten_shapes.is_empty() && nested_shapes.is_empty() {
            quote! {
                ::agena_plugin_sdk::macro_support::validate_minimum_path(
                    &#target,
                    #path,
                    &::agena_plugin_sdk::serde_json::json!(#value),
                )?;
            }
        } else {
            let resolved_path =
                expand_input_shape_resolved_path_expr(flatten_shapes, nested_shapes, path);
            quote! {
                let __path = #resolved_path;
                ::agena_plugin_sdk::macro_support::validate_minimum_path(
                    &#target,
                    __path.as_str(),
                    &::agena_plugin_sdk::serde_json::json!(#value),
                )?;
            }
        }
    });
    let maximum_exprs = maximums.iter().map(|constraint| {
        let path = &constraint.path;
        let value = &constraint.value;
        if flatten_shapes.is_empty() && nested_shapes.is_empty() {
            quote! {
                ::agena_plugin_sdk::macro_support::validate_maximum_path(
                    &#target,
                    #path,
                    &::agena_plugin_sdk::serde_json::json!(#value),
                )?;
            }
        } else {
            let resolved_path =
                expand_input_shape_resolved_path_expr(flatten_shapes, nested_shapes, path);
            quote! {
                let __path = #resolved_path;
                ::agena_plugin_sdk::macro_support::validate_maximum_path(
                    &#target,
                    __path.as_str(),
                    &::agena_plugin_sdk::serde_json::json!(#value),
                )?;
            }
        }
    });
    let exclusive_minimum_exprs = exclusive_minimums.iter().map(|constraint| {
        let path = &constraint.path;
        let value = &constraint.value;
        if flatten_shapes.is_empty() && nested_shapes.is_empty() {
            quote! {
                ::agena_plugin_sdk::macro_support::validate_exclusive_minimum_path(
                    &#target,
                    #path,
                    &::agena_plugin_sdk::serde_json::json!(#value),
                )?;
            }
        } else {
            let resolved_path =
                expand_input_shape_resolved_path_expr(flatten_shapes, nested_shapes, path);
            quote! {
                let __path = #resolved_path;
                ::agena_plugin_sdk::macro_support::validate_exclusive_minimum_path(
                    &#target,
                    __path.as_str(),
                    &::agena_plugin_sdk::serde_json::json!(#value),
                )?;
            }
        }
    });
    let exclusive_maximum_exprs = exclusive_maximums.iter().map(|constraint| {
        let path = &constraint.path;
        let value = &constraint.value;
        if flatten_shapes.is_empty() && nested_shapes.is_empty() {
            quote! {
                ::agena_plugin_sdk::macro_support::validate_exclusive_maximum_path(
                    &#target,
                    #path,
                    &::agena_plugin_sdk::serde_json::json!(#value),
                )?;
            }
        } else {
            let resolved_path =
                expand_input_shape_resolved_path_expr(flatten_shapes, nested_shapes, path);
            quote! {
                let __path = #resolved_path;
                ::agena_plugin_sdk::macro_support::validate_exclusive_maximum_path(
                    &#target,
                    __path.as_str(),
                    &::agena_plugin_sdk::serde_json::json!(#value),
                )?;
            }
        }
    });
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
        #(#minimum_exprs)*
        #(#maximum_exprs)*
        #(#exclusive_minimum_exprs)*
        #(#exclusive_maximum_exprs)*
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
