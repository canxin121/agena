use std::collections::BTreeMap;

use syn::{LitStr, Type};

use super::{
    PathPairConstraint, PathStringConstraint, PathStringsConstraint, PathUsizeConstraint,
    PathValueConstraint, PathValuesConstraint, PluginArgConfig, PluginInputNetworkSpec,
    PluginInputPathSpec, ToolSpecConfig, append_constraint_path_suffix,
    input_type_semantic_shape, prefixed_constraint_group, resolve_known_constraint_path,
};
use super::input_arg_output_support::{
    apply_arg_metadata_to_spec, input_jsonpath_for_arg, input_jsonpath_for_field,
};
use super::input_arg_parse_support::inline_arg_has_default;

pub(crate) fn apply_arg_config_to_spec(
    spec: &mut ToolSpecConfig,
    field_name: &LitStr,
    aliases: &[LitStr],
    ty: &Type,
    field_path_lookup: Option<&BTreeMap<String, LitStr>>,
    config: &PluginArgConfig,
) {
    if config.trim {
        spec.trim.push(field_name.clone());
    }
    if config.item_trim {
        spec.trim
            .push(append_constraint_path_suffix(field_name, "[]"));
    }
    if config.non_empty {
        spec.non_empty.push(field_name.clone());
    }
    if config.item_non_empty {
        spec.non_empty
            .push(append_constraint_path_suffix(field_name, "[]"));
    }
    if config.non_empty_if_present {
        spec.non_empty_if_present.push(field_name.clone());
    }
    if config.item_non_empty_if_present {
        spec.non_empty_if_present
            .push(append_constraint_path_suffix(field_name, "[]"));
    }
    if let Some(value) = config.trim_suffix.as_ref() {
        spec.trim_suffix.push(PathStringConstraint {
            path: field_name.clone(),
            value: value.clone(),
        });
    }
    if let Some(value) = config.item_trim_suffix.as_ref() {
        spec.trim_suffix.push(PathStringConstraint {
            path: append_constraint_path_suffix(field_name, "[]"),
            value: value.clone(),
        });
    }
    if let Some(value) = config.minimum.as_ref() {
        spec.minimums.push(PathValueConstraint {
            path: field_name.clone(),
            value: value.clone(),
        });
    }
    if let Some(value) = config.maximum.as_ref() {
        spec.maximums.push(PathValueConstraint {
            path: field_name.clone(),
            value: value.clone(),
        });
    }
    if let Some(value) = config.exclusive_minimum.as_ref() {
        spec.exclusive_minimums.push(PathValueConstraint {
            path: field_name.clone(),
            value: value.clone(),
        });
    }
    if let Some(value) = config.exclusive_maximum.as_ref() {
        spec.exclusive_maximums.push(PathValueConstraint {
            path: field_name.clone(),
            value: value.clone(),
        });
    }
    if let Some(value) = config.min_items {
        spec.min_items.push(PathUsizeConstraint {
            path: field_name.clone(),
            value,
        });
    }
    if let Some(value) = config.max_items {
        spec.max_items.push(PathUsizeConstraint {
            path: field_name.clone(),
            value,
        });
    }
    if let Some(value) = config.min_properties {
        spec.min_properties.push(PathUsizeConstraint {
            path: field_name.clone(),
            value,
        });
    }
    if let Some(value) = config.max_properties {
        spec.max_properties.push(PathUsizeConstraint {
            path: field_name.clone(),
            value,
        });
    }
    if let Some(value) = config.item_minimum.as_ref() {
        spec.minimums.push(PathValueConstraint {
            path: append_constraint_path_suffix(field_name, "[]"),
            value: value.clone(),
        });
    }
    if let Some(value) = config.item_maximum.as_ref() {
        spec.maximums.push(PathValueConstraint {
            path: append_constraint_path_suffix(field_name, "[]"),
            value: value.clone(),
        });
    }
    if let Some(value) = config.item_exclusive_minimum.as_ref() {
        spec.exclusive_minimums.push(PathValueConstraint {
            path: append_constraint_path_suffix(field_name, "[]"),
            value: value.clone(),
        });
    }
    if let Some(value) = config.item_exclusive_maximum.as_ref() {
        spec.exclusive_maximums.push(PathValueConstraint {
            path: append_constraint_path_suffix(field_name, "[]"),
            value: value.clone(),
        });
    }
    if let Some(value) = config.item_min_properties {
        spec.min_properties.push(PathUsizeConstraint {
            path: append_constraint_path_suffix(field_name, "[]"),
            value,
        });
    }
    if let Some(value) = config.item_max_properties {
        spec.max_properties.push(PathUsizeConstraint {
            path: append_constraint_path_suffix(field_name, "[]"),
            value,
        });
    }
    if let Some(value) = config.min_chars {
        spec.min_chars.push(PathUsizeConstraint {
            path: field_name.clone(),
            value,
        });
    }
    if let Some(value) = config.max_chars {
        spec.max_chars.push(PathUsizeConstraint {
            path: field_name.clone(),
            value,
        });
    }
    if let Some(value) = config.item_min_chars {
        spec.min_chars.push(PathUsizeConstraint {
            path: append_constraint_path_suffix(field_name, "[]"),
            value,
        });
    }
    if let Some(value) = config.item_max_chars {
        spec.max_chars.push(PathUsizeConstraint {
            path: append_constraint_path_suffix(field_name, "[]"),
            value,
        });
    }
    if let Some(value) = config.format.as_ref() {
        spec.formats.push(PathStringConstraint {
            path: field_name.clone(),
            value: value.clone(),
        });
    }
    if let Some(value) = config.item_format.as_ref() {
        spec.formats.push(PathStringConstraint {
            path: append_constraint_path_suffix(field_name, "[]"),
            value: value.clone(),
        });
    }
    if let Some(value) = config.pattern.as_ref() {
        spec.patterns.push(PathStringConstraint {
            path: field_name.clone(),
            value: value.clone(),
        });
    }
    if let Some(value) = config.item_pattern.as_ref() {
        spec.patterns.push(PathStringConstraint {
            path: append_constraint_path_suffix(field_name, "[]"),
            value: value.clone(),
        });
    }
    if let Some(values) = config.choices.as_ref() {
        spec.choices.push(PathValuesConstraint {
            path: field_name.clone(),
            values: values.clone(),
        });
    }
    if let Some(values) = config.item_choices.as_ref() {
        spec.choices.push(PathValuesConstraint {
            path: append_constraint_path_suffix(field_name, "[]"),
            values: values.clone(),
        });
    }
    let type_shape = input_type_semantic_shape(ty);
    let field_value_path = if type_shape.array {
        append_constraint_path_suffix(field_name, "[]")
    } else {
        field_name.clone()
    };
    spec.requires
        .extend(config.requires.iter().map(|right| PathPairConstraint {
            left: field_name.clone(),
            right: resolve_known_constraint_path(right, field_path_lookup),
        }));
    spec.conflicts_with.extend(
        config
            .conflicts_with
            .iter()
            .map(|right| PathPairConstraint {
                left: field_name.clone(),
                right: resolve_known_constraint_path(right, field_path_lookup),
            }),
    );
    spec.required_unless_present
        .extend(
            config
                .required_unless_present
                .iter()
                .map(|right| PathPairConstraint {
                    left: field_name.clone(),
                    right: resolve_known_constraint_path(right, field_path_lookup),
                }),
        );
    if !config.forbid_substrings.is_empty() {
        spec.forbid_substrings.push(PathStringsConstraint {
            path: field_value_path.clone(),
            values: config.forbid_substrings.clone(),
        });
    }
    if config.distinct_trimmed {
        spec.distinct_trimmed.push(field_value_path.clone());
    }
    if !config.exactly_one_of.is_empty() {
        spec.exactly_one_of.push(prefixed_constraint_group(
            field_name,
            &config.exactly_one_of,
            field_path_lookup,
        ));
    }
    if !config.at_least_one_of.is_empty() {
        spec.at_least_one_of.push(prefixed_constraint_group(
            field_name,
            &config.at_least_one_of,
            field_path_lookup,
        ));
    }
    spec.distinct_trimmed_within
        .extend(
            config
                .distinct_trimmed_within
                .iter()
                .map(|right| PathPairConstraint {
                    left: field_name.clone(),
                    right: resolve_known_constraint_path(right, field_path_lookup),
                }),
        );
    let optional = config.optional
        || inline_arg_has_default(config)
        || type_shape.optional
        || !aliases.is_empty();
    let jsonpath = input_jsonpath_for_arg(field_name, ty, config.jsonpath.as_ref());
    if let Some(kind) = config.path {
        spec.input_paths.push(PluginInputPathSpec {
            jsonpath: jsonpath.clone(),
            kind,
            fallback: config.fallback.clone(),
            optional,
        });
        if config.jsonpath.is_none() {
            spec.input_paths
                .extend(aliases.iter().map(|alias| PluginInputPathSpec {
                    jsonpath: input_jsonpath_for_field(alias, ty),
                    kind,
                    fallback: config.fallback.clone(),
                    optional,
                }));
        }
    }
    if let Some(semantic) = config.network {
        spec.input_networks.push(PluginInputNetworkSpec {
            jsonpath,
            fallback: config.fallback.clone(),
            optional,
            semantic,
        });
        if config.jsonpath.is_none() {
            spec.input_networks
                .extend(aliases.iter().map(|alias| PluginInputNetworkSpec {
                    jsonpath: input_jsonpath_for_field(alias, ty),
                    fallback: config.fallback.clone(),
                    optional,
                    semantic,
                }));
        }
    }
    apply_arg_metadata_to_spec(
        &mut spec.input_field_metadata,
        field_name,
        field_name,
        aliases,
        config.description.clone(),
        config.path,
        config.network,
        config.non_empty || config.non_empty_if_present,
        config.item_non_empty,
        config.item_non_empty_if_present,
        config.minimum.clone(),
        config.maximum.clone(),
        config.exclusive_minimum.clone(),
        config.exclusive_maximum.clone(),
        config.min_items,
        config.max_items,
        config.min_properties,
        config.max_properties,
        config.item_minimum.clone(),
        config.item_maximum.clone(),
        config.item_exclusive_minimum.clone(),
        config.item_exclusive_maximum.clone(),
        config.item_min_properties,
        config.item_max_properties,
        config.min_chars,
        config.max_chars,
        config.item_min_chars,
        config.item_max_chars,
        config.format.clone(),
        config.item_format.clone(),
        config.pattern.clone(),
        config.item_pattern.clone(),
        config.example.clone(),
        config.choices.clone().unwrap_or_default(),
        config.item_choices.clone().unwrap_or_default(),
        config.secret,
        config.picker,
    );
}
