use syn::punctuated::Punctuated;
use syn::{Attribute, Meta, Result, Token, Variant};

use super::{
    ToolInputConfig, ToolInputVariantConfig, expr_lit_bool, expr_lit_str, expr_path,
    parse_item_lit_str_list, parse_item_path_expr_constraint, parse_item_path_expr_list_constraint,
    parse_item_path_format_constraint, parse_item_path_lit_str_constraint,
    parse_item_path_pattern_constraint, parse_item_path_usize_constraint, parse_lit_str_list,
    parse_path_expr_constraint, parse_path_expr_list_constraint, parse_path_format_constraint,
    parse_path_lit_str_constraint, parse_path_lit_str_list_constraint, parse_path_pair_constraint,
    parse_path_pattern_constraint, parse_path_usize_constraint,
};

pub fn parse_input_config(attrs: &[Attribute]) -> Result<ToolInputConfig> {
    let mut example = None;
    let mut default = false;
    let mut default_expr = None;
    let mut normalize = None;
    let mut validate = None;
    let mut handler_receiver = None;
    let mut handle = None;
    let mut handle_with_context = None;
    let mut stream_handle = None;
    let mut stream_handle_with_context = None;
    let mut permission_paths_handle = None;
    let mut permission_networks_handle = None;
    let mut handle_field = None;
    let mut handle_by_value = false;
    let mut trim = Vec::new();
    let mut trim_suffix = Vec::new();
    let mut non_empty = Vec::new();
    let mut non_empty_if_present = Vec::new();
    let mut minimums = Vec::new();
    let mut maximums = Vec::new();
    let mut exclusive_minimums = Vec::new();
    let mut exclusive_maximums = Vec::new();
    let mut exactly_one_of = Vec::new();
    let mut at_least_one_of = Vec::new();
    let mut requires = Vec::new();
    let mut conflicts_with = Vec::new();
    let mut required_unless_present = Vec::new();
    let mut forbid_substrings = Vec::new();
    let mut distinct_trimmed = Vec::new();
    let mut distinct_trimmed_within = Vec::new();
    let mut min_items = Vec::new();
    let mut max_items = Vec::new();
    let mut min_properties = Vec::new();
    let mut max_properties = Vec::new();
    let mut min_chars = Vec::new();
    let mut max_chars = Vec::new();
    let mut formats = Vec::new();
    let mut patterns = Vec::new();
    let mut choices = Vec::new();
    let input_paths = Vec::new();
    let input_networks = Vec::new();
    let input_aliases = Vec::new();
    let input_defaults = Vec::new();
    let input_field_metadata = Vec::new();
    for attr in attrs {
        if !attr.path().is_ident("input") {
            continue;
        }
        let metas = attr.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)?;
        for meta in metas {
            match meta {
                Meta::NameValue(value) => {
                    let Some(ident) = value.path.get_ident() else {
                        return Err(syn::Error::new_spanned(value.path, "expected identifier"));
                    };
                    match ident.to_string().as_str() {
                        "example" => {
                            if example.replace(value.value).is_some() {
                                return Err(syn::Error::new_spanned(
                                    ident,
                                    "duplicate input example",
                                ));
                            }
                        }
                        "default" => {
                            if default || default_expr.replace(value.value).is_some() {
                                return Err(syn::Error::new_spanned(
                                    ident,
                                    "duplicate input default",
                                ));
                            }
                        }
                        "normalize" => normalize = Some(expr_path(&value.value, "normalize")?),
                        "validate" => validate = Some(expr_path(&value.value, "validate")?),
                        "handler_receiver" => {
                            handler_receiver = Some(expr_path(&value.value, "handler_receiver")?)
                        }
                        "handle" => handle = Some(expr_path(&value.value, "handle")?),
                        "handle_with_context" => {
                            handle_with_context =
                                Some(expr_path(&value.value, "handle_with_context")?)
                        }
                        "stream_handle" => {
                            stream_handle = Some(expr_path(&value.value, "stream_handle")?)
                        }
                        "stream_handle_with_context" => {
                            stream_handle_with_context =
                                Some(expr_path(&value.value, "stream_handle_with_context")?)
                        }
                        "permission_paths_handle" => {
                            permission_paths_handle =
                                Some(expr_path(&value.value, "permission_paths_handle")?)
                        }
                        "permission_networks_handle" => {
                            permission_networks_handle =
                                Some(expr_path(&value.value, "permission_networks_handle")?)
                        }
                        "handle_field" => {
                            handle_field = Some(expr_path(&value.value, "handle_field")?)
                        }
                        "handle_by_value" => {
                            handle_by_value = expr_lit_bool(&value.value, "handle_by_value")?
                        }
                        other => {
                            return Err(syn::Error::new_spanned(
                                ident,
                                format!("unsupported input attribute '{other}'"),
                            ));
                        }
                    }
                }
                Meta::List(list) => {
                    let Some(ident) = list.path.get_ident() else {
                        return Err(syn::Error::new_spanned(list.path, "expected identifier"));
                    };
                    match ident.to_string().as_str() {
                        "trim" => trim.extend(parse_lit_str_list(list.tokens)?),
                        "item_trim" => trim.extend(parse_item_lit_str_list(list.tokens)?),
                        "trim_suffix" => trim_suffix
                            .push(parse_path_lit_str_constraint(list.tokens, "trim_suffix")?),
                        "item_trim_suffix" => trim_suffix.push(parse_item_path_lit_str_constraint(
                            list.tokens,
                            "item_trim_suffix",
                        )?),
                        "non_empty" => non_empty.extend(parse_lit_str_list(list.tokens)?),
                        "item_non_empty" => non_empty.extend(parse_item_lit_str_list(list.tokens)?),
                        "non_empty_if_present" => {
                            non_empty_if_present.extend(parse_lit_str_list(list.tokens)?)
                        }
                        "item_non_empty_if_present" => {
                            non_empty_if_present.extend(parse_item_lit_str_list(list.tokens)?)
                        }
                        "minimum" => {
                            minimums.push(parse_path_expr_constraint(list.tokens, "minimum")?)
                        }
                        "maximum" => {
                            maximums.push(parse_path_expr_constraint(list.tokens, "maximum")?)
                        }
                        "exclusive_minimum" => exclusive_minimums.push(parse_path_expr_constraint(
                            list.tokens,
                            "exclusive_minimum",
                        )?),
                        "exclusive_maximum" => exclusive_maximums.push(parse_path_expr_constraint(
                            list.tokens,
                            "exclusive_maximum",
                        )?),
                        "exactly_one_of" => exactly_one_of.push(parse_lit_str_list(list.tokens)?),
                        "at_least_one_of" => at_least_one_of.push(parse_lit_str_list(list.tokens)?),
                        "requires" => {
                            requires.push(parse_path_pair_constraint(list.tokens, "requires")?)
                        }
                        "conflicts_with" => conflicts_with
                            .push(parse_path_pair_constraint(list.tokens, "conflicts_with")?),
                        "required_unless_present" => required_unless_present.push(
                            parse_path_pair_constraint(list.tokens, "required_unless_present")?,
                        ),
                        "forbid_substrings" => forbid_substrings.push(
                            parse_path_lit_str_list_constraint(list.tokens, "forbid_substrings")?,
                        ),
                        "distinct_trimmed" => {
                            distinct_trimmed.extend(parse_lit_str_list(list.tokens)?)
                        }
                        "distinct_trimmed_within" => distinct_trimmed_within.push(
                            parse_path_pair_constraint(list.tokens, "distinct_trimmed_within")?,
                        ),
                        "min_items" => {
                            min_items.push(parse_path_usize_constraint(list.tokens, "min_items")?)
                        }
                        "max_items" => {
                            max_items.push(parse_path_usize_constraint(list.tokens, "max_items")?)
                        }
                        "min_properties" => min_properties
                            .push(parse_path_usize_constraint(list.tokens, "min_properties")?),
                        "max_properties" => max_properties
                            .push(parse_path_usize_constraint(list.tokens, "max_properties")?),
                        "item_minimum" => minimums.push(parse_item_path_expr_constraint(
                            list.tokens,
                            "item_minimum",
                        )?),
                        "item_maximum" => maximums.push(parse_item_path_expr_constraint(
                            list.tokens,
                            "item_maximum",
                        )?),
                        "item_exclusive_minimum" => exclusive_minimums.push(
                            parse_item_path_expr_constraint(list.tokens, "item_exclusive_minimum")?,
                        ),
                        "item_exclusive_maximum" => exclusive_maximums.push(
                            parse_item_path_expr_constraint(list.tokens, "item_exclusive_maximum")?,
                        ),
                        "item_min_properties" => min_properties.push(
                            parse_item_path_usize_constraint(list.tokens, "item_min_properties")?,
                        ),
                        "item_max_properties" => max_properties.push(
                            parse_item_path_usize_constraint(list.tokens, "item_max_properties")?,
                        ),
                        "item_min_chars" => min_chars.push(parse_item_path_usize_constraint(
                            list.tokens,
                            "item_min_chars",
                        )?),
                        "item_max_chars" => max_chars.push(parse_item_path_usize_constraint(
                            list.tokens,
                            "item_max_chars",
                        )?),
                        "item_format" => formats.push(parse_item_path_format_constraint(
                            list.tokens,
                            "item_format",
                        )?),
                        "min_chars" => {
                            min_chars.push(parse_path_usize_constraint(list.tokens, "min_chars")?)
                        }
                        "max_chars" => {
                            max_chars.push(parse_path_usize_constraint(list.tokens, "max_chars")?)
                        }
                        "format" => {
                            formats.push(parse_path_format_constraint(list.tokens, "format")?)
                        }
                        "item_pattern" => patterns.push(parse_item_path_pattern_constraint(
                            list.tokens,
                            "item_pattern",
                        )?),
                        "item_choices" => choices.push(parse_item_path_expr_list_constraint(
                            list.tokens,
                            "item_choices",
                        )?),
                        "pattern" => patterns.push(parse_path_pattern_constraint(list.tokens)?),
                        "choices" => {
                            choices.push(parse_path_expr_list_constraint(list.tokens, "choices")?)
                        }
                        other => {
                            return Err(syn::Error::new_spanned(
                                ident,
                                format!("unsupported input list '{other}'"),
                            ));
                        }
                    }
                }
                Meta::Path(path) => {
                    if path.is_ident("default") {
                        if default || default_expr.is_some() {
                            return Err(syn::Error::new_spanned(path, "duplicate input default"));
                        }
                        default = true;
                    } else {
                        return Err(syn::Error::new_spanned(
                            path,
                            "unsupported bare input argument",
                        ));
                    }
                }
            }
        }
    }
    Ok(ToolInputConfig {
        example,
        default,
        default_expr,
        normalize,
        validate,
        handler_receiver,
        handle,
        handle_with_context,
        stream_handle,
        stream_handle_with_context,
        permission_paths_handle,
        permission_networks_handle,
        handle_field,
        handle_by_value,
        trim,
        trim_suffix,
        non_empty,
        non_empty_if_present,
        minimums,
        maximums,
        exclusive_minimums,
        exclusive_maximums,
        exactly_one_of,
        at_least_one_of,
        requires,
        conflicts_with,
        required_unless_present,
        forbid_substrings,
        distinct_trimmed,
        distinct_trimmed_within,
        min_items,
        max_items,
        min_properties,
        max_properties,
        min_chars,
        max_chars,
        formats,
        patterns,
        choices,
        input_paths,
        input_networks,
        input_aliases,
        input_defaults,
        input_field_metadata,
    })
}

pub fn parse_input_variant_config(variant: &Variant) -> Result<ToolInputVariantConfig> {
    let mut action = None;
    let mut validate = None;
    let mut handle = None;
    let mut handle_with_context = None;
    let mut stream_handle = None;
    let mut stream_handle_with_context = None;
    let mut permission_paths_handle = None;
    let mut permission_networks_handle = None;
    let mut handle_by_value = false;
    let mut trim = Vec::new();
    let mut trim_suffix = Vec::new();
    let mut non_empty = Vec::new();
    let mut non_empty_if_present = Vec::new();
    let mut minimums = Vec::new();
    let mut maximums = Vec::new();
    let mut exclusive_minimums = Vec::new();
    let mut exclusive_maximums = Vec::new();
    let mut exactly_one_of = Vec::new();
    let mut at_least_one_of = Vec::new();
    let mut requires = Vec::new();
    let mut conflicts_with = Vec::new();
    let mut required_unless_present = Vec::new();
    let mut forbid_substrings = Vec::new();
    let mut distinct_trimmed = Vec::new();
    let mut distinct_trimmed_within = Vec::new();
    let mut min_items = Vec::new();
    let mut max_items = Vec::new();
    let mut min_properties = Vec::new();
    let mut max_properties = Vec::new();
    let mut min_chars = Vec::new();
    let mut max_chars = Vec::new();
    let mut formats = Vec::new();
    let mut patterns = Vec::new();
    let mut choices = Vec::new();
    let mut default_when_empty = false;
    let mut infer_when_present = Vec::new();
    let mut drop_keys = Vec::new();
    for attr in &variant.attrs {
        if !attr.path().is_ident("input") {
            continue;
        }
        let metas = attr.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)?;
        for meta in metas {
            match meta {
                Meta::NameValue(value) => {
                    let Some(ident) = value.path.get_ident() else {
                        return Err(syn::Error::new_spanned(value.path, "expected identifier"));
                    };
                    match ident.to_string().as_str() {
                        "action" => action = Some(expr_lit_str(&value.value, "action")?),
                        "exec" => {
                            return Err(syn::Error::new_spanned(
                                ident,
                                "ToolInput uses `#[input(action = \"...\")]`; `exec` is only valid for generated tool routing",
                            ));
                        }
                        "validate" => validate = Some(expr_path(&value.value, "validate")?),
                        "handle" => handle = Some(expr_path(&value.value, "handle")?),
                        "handle_with_context" => {
                            handle_with_context =
                                Some(expr_path(&value.value, "handle_with_context")?)
                        }
                        "stream_handle" => {
                            stream_handle = Some(expr_path(&value.value, "stream_handle")?)
                        }
                        "stream_handle_with_context" => {
                            stream_handle_with_context =
                                Some(expr_path(&value.value, "stream_handle_with_context")?)
                        }
                        "permission_paths_handle" => {
                            permission_paths_handle =
                                Some(expr_path(&value.value, "permission_paths_handle")?)
                        }
                        "permission_networks_handle" => {
                            permission_networks_handle =
                                Some(expr_path(&value.value, "permission_networks_handle")?)
                        }
                        "handle_by_value" => {
                            handle_by_value = expr_lit_bool(&value.value, "handle_by_value")?
                        }
                        "default_when_empty" => {
                            default_when_empty = expr_lit_bool(&value.value, "default_when_empty")?
                        }
                        "map" => {
                            return Err(syn::Error::new_spanned(
                                ident,
                                "ToolInput does not support `map`; use `#[input(action = \"...\")]` to override the action name",
                            ));
                        }
                        other => {
                            return Err(syn::Error::new_spanned(
                                ident,
                                format!("unsupported input variant attribute '{other}'"),
                            ));
                        }
                    }
                }
                Meta::List(list) => {
                    let Some(ident) = list.path.get_ident() else {
                        return Err(syn::Error::new_spanned(list.path, "expected identifier"));
                    };
                    match ident.to_string().as_str() {
                        "trim" => trim.extend(parse_lit_str_list(list.tokens)?),
                        "item_trim" => trim.extend(parse_item_lit_str_list(list.tokens)?),
                        "trim_suffix" => trim_suffix
                            .push(parse_path_lit_str_constraint(list.tokens, "trim_suffix")?),
                        "item_trim_suffix" => trim_suffix.push(parse_item_path_lit_str_constraint(
                            list.tokens,
                            "item_trim_suffix",
                        )?),
                        "non_empty" => non_empty.extend(parse_lit_str_list(list.tokens)?),
                        "item_non_empty" => non_empty.extend(parse_item_lit_str_list(list.tokens)?),
                        "non_empty_if_present" => {
                            non_empty_if_present.extend(parse_lit_str_list(list.tokens)?)
                        }
                        "item_non_empty_if_present" => {
                            non_empty_if_present.extend(parse_item_lit_str_list(list.tokens)?)
                        }
                        "minimum" => {
                            minimums.push(parse_path_expr_constraint(list.tokens, "minimum")?)
                        }
                        "maximum" => {
                            maximums.push(parse_path_expr_constraint(list.tokens, "maximum")?)
                        }
                        "exclusive_minimum" => exclusive_minimums.push(parse_path_expr_constraint(
                            list.tokens,
                            "exclusive_minimum",
                        )?),
                        "exclusive_maximum" => exclusive_maximums.push(parse_path_expr_constraint(
                            list.tokens,
                            "exclusive_maximum",
                        )?),
                        "exactly_one_of" => exactly_one_of.push(parse_lit_str_list(list.tokens)?),
                        "at_least_one_of" => at_least_one_of.push(parse_lit_str_list(list.tokens)?),
                        "requires" => {
                            requires.push(parse_path_pair_constraint(list.tokens, "requires")?)
                        }
                        "conflicts_with" => conflicts_with
                            .push(parse_path_pair_constraint(list.tokens, "conflicts_with")?),
                        "required_unless_present" => required_unless_present.push(
                            parse_path_pair_constraint(list.tokens, "required_unless_present")?,
                        ),
                        "forbid_substrings" => forbid_substrings.push(
                            parse_path_lit_str_list_constraint(list.tokens, "forbid_substrings")?,
                        ),
                        "distinct_trimmed" => {
                            distinct_trimmed.extend(parse_lit_str_list(list.tokens)?)
                        }
                        "distinct_trimmed_within" => distinct_trimmed_within.push(
                            parse_path_pair_constraint(list.tokens, "distinct_trimmed_within")?,
                        ),
                        "min_items" => {
                            min_items.push(parse_path_usize_constraint(list.tokens, "min_items")?)
                        }
                        "max_items" => {
                            max_items.push(parse_path_usize_constraint(list.tokens, "max_items")?)
                        }
                        "min_properties" => min_properties
                            .push(parse_path_usize_constraint(list.tokens, "min_properties")?),
                        "max_properties" => max_properties
                            .push(parse_path_usize_constraint(list.tokens, "max_properties")?),
                        "item_minimum" => minimums.push(parse_item_path_expr_constraint(
                            list.tokens,
                            "item_minimum",
                        )?),
                        "item_maximum" => maximums.push(parse_item_path_expr_constraint(
                            list.tokens,
                            "item_maximum",
                        )?),
                        "item_exclusive_minimum" => exclusive_minimums.push(
                            parse_item_path_expr_constraint(list.tokens, "item_exclusive_minimum")?,
                        ),
                        "item_exclusive_maximum" => exclusive_maximums.push(
                            parse_item_path_expr_constraint(list.tokens, "item_exclusive_maximum")?,
                        ),
                        "item_min_properties" => min_properties.push(
                            parse_item_path_usize_constraint(list.tokens, "item_min_properties")?,
                        ),
                        "item_max_properties" => max_properties.push(
                            parse_item_path_usize_constraint(list.tokens, "item_max_properties")?,
                        ),
                        "item_min_chars" => min_chars.push(parse_item_path_usize_constraint(
                            list.tokens,
                            "item_min_chars",
                        )?),
                        "item_max_chars" => max_chars.push(parse_item_path_usize_constraint(
                            list.tokens,
                            "item_max_chars",
                        )?),
                        "item_format" => formats.push(parse_item_path_format_constraint(
                            list.tokens,
                            "item_format",
                        )?),
                        "min_chars" => {
                            min_chars.push(parse_path_usize_constraint(list.tokens, "min_chars")?)
                        }
                        "max_chars" => {
                            max_chars.push(parse_path_usize_constraint(list.tokens, "max_chars")?)
                        }
                        "format" => {
                            formats.push(parse_path_format_constraint(list.tokens, "format")?)
                        }
                        "item_pattern" => patterns.push(parse_item_path_pattern_constraint(
                            list.tokens,
                            "item_pattern",
                        )?),
                        "item_choices" => choices.push(parse_item_path_expr_list_constraint(
                            list.tokens,
                            "item_choices",
                        )?),
                        "pattern" => patterns.push(parse_path_pattern_constraint(list.tokens)?),
                        "choices" => {
                            choices.push(parse_path_expr_list_constraint(list.tokens, "choices")?)
                        }
                        "infer_when_present" => {
                            infer_when_present.extend(parse_lit_str_list(list.tokens)?)
                        }
                        "drop_keys" => drop_keys.extend(parse_lit_str_list(list.tokens)?),
                        other => {
                            return Err(syn::Error::new_spanned(
                                ident,
                                format!("unsupported input variant list '{other}'"),
                            ));
                        }
                    }
                }
                Meta::Path(path) => {
                    return Err(syn::Error::new_spanned(
                        path,
                        "unsupported bare input variant argument",
                    ));
                }
            }
        }
    }
    Ok(ToolInputVariantConfig {
        action,
        validate,
        handle,
        handle_with_context,
        stream_handle,
        stream_handle_with_context,
        permission_paths_handle,
        permission_networks_handle,
        handle_by_value,
        trim,
        trim_suffix,
        non_empty,
        non_empty_if_present,
        minimums,
        maximums,
        exclusive_minimums,
        exclusive_maximums,
        exactly_one_of,
        at_least_one_of,
        requires,
        conflicts_with,
        required_unless_present,
        forbid_substrings,
        distinct_trimmed,
        distinct_trimmed_within,
        min_items,
        max_items,
        min_properties,
        max_properties,
        min_chars,
        max_chars,
        formats,
        patterns,
        choices,
        input_paths: Vec::new(),
        input_networks: Vec::new(),
        input_aliases: Vec::new(),
        input_defaults: Vec::new(),
        input_field_metadata: Vec::new(),
        default_when_empty,
        infer_when_present,
        drop_keys,
    })
}
