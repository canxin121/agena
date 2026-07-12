use std::collections::{BTreeMap, BTreeSet};

use syn::LitStr;

use super::{
    PathPairConstraint, PathStringConstraint, PathStringsConstraint, PathUsizeConstraint,
    PathValueConstraint, PathValuesConstraint, PluginInputFieldMetadata,
};

pub(crate) fn append_constraint_path_suffix(path: &LitStr, suffix: &str) -> LitStr {
    LitStr::new(format!("{}{}", path.value(), suffix).as_str(), path.span())
}

fn resolve_known_input_constraint_path(
    path: &LitStr,
    field_path_lookup: &BTreeMap<String, LitStr>,
) -> LitStr {
    if let Some(resolved) = field_path_lookup.get(&path.value()) {
        return resolved.clone();
    }
    let value = path.value();
    let head_end = value.find('.').unwrap_or(value.len());
    let (head, tail) = value.split_at(head_end);
    let mut base = head;
    let mut suffix = String::new();
    while let Some(stripped) = base.strip_suffix("[]") {
        base = stripped;
        suffix.push_str("[]");
    }
    if let Some(resolved_head) = field_path_lookup.get(base) {
        return LitStr::new(
            format!("{}{}{}", resolved_head.value(), suffix, tail).as_str(),
            path.span(),
        );
    }
    path.clone()
}

pub(crate) fn resolve_known_constraint_path(
    path: &LitStr,
    field_path_lookup: Option<&BTreeMap<String, LitStr>>,
) -> LitStr {
    field_path_lookup
        .map(|lookup| resolve_known_input_constraint_path(path, lookup))
        .unwrap_or_else(|| path.clone())
}

fn normalize_array_value_constraint_path(
    path: &LitStr,
    field_path_lookup: &BTreeMap<String, LitStr>,
    array_field_paths: &BTreeSet<String>,
) -> LitStr {
    let resolved = resolve_known_input_constraint_path(path, field_path_lookup);
    let value = resolved.value();
    let head_end = value.find('.').unwrap_or(value.len());
    let (head, tail) = value.split_at(head_end);
    if head.ends_with("[]") || !array_field_paths.contains(head) {
        return resolved;
    }
    LitStr::new(format!("{head}[]{tail}").as_str(), path.span())
}

pub(crate) fn normalize_array_value_lit_paths(
    paths: &mut [LitStr],
    field_path_lookup: &BTreeMap<String, LitStr>,
    array_field_paths: &BTreeSet<String>,
) {
    for path in paths {
        *path = normalize_array_value_constraint_path(path, field_path_lookup, array_field_paths);
    }
}

pub(crate) fn resolve_constraint_lit_paths(
    paths: &mut [LitStr],
    field_path_lookup: &BTreeMap<String, LitStr>,
) {
    for path in paths {
        *path = resolve_known_input_constraint_path(path, field_path_lookup);
    }
}

pub(crate) fn resolve_constraint_string_paths(
    constraints: &mut [PathStringConstraint],
    field_path_lookup: &BTreeMap<String, LitStr>,
) {
    for constraint in constraints {
        constraint.path = resolve_known_input_constraint_path(&constraint.path, field_path_lookup);
    }
}

pub(crate) fn resolve_constraint_usize_paths(
    constraints: &mut [PathUsizeConstraint],
    field_path_lookup: &BTreeMap<String, LitStr>,
) {
    for constraint in constraints {
        constraint.path = resolve_known_input_constraint_path(&constraint.path, field_path_lookup);
    }
}

pub(crate) fn resolve_constraint_expr_paths(
    constraints: &mut [PathValueConstraint],
    field_path_lookup: &BTreeMap<String, LitStr>,
) {
    for constraint in constraints {
        constraint.path = resolve_known_input_constraint_path(&constraint.path, field_path_lookup);
    }
}

pub(crate) fn resolve_constraint_values_paths(
    constraints: &mut [PathValuesConstraint],
    field_path_lookup: &BTreeMap<String, LitStr>,
) {
    for constraint in constraints {
        constraint.path = resolve_known_input_constraint_path(&constraint.path, field_path_lookup);
    }
}

pub(crate) fn resolve_constraint_group_paths(
    groups: &mut [Vec<LitStr>],
    field_path_lookup: &BTreeMap<String, LitStr>,
) {
    for group in groups {
        resolve_constraint_lit_paths(group, field_path_lookup);
    }
}

pub(crate) fn resolve_constraint_pair_paths(
    constraints: &mut [PathPairConstraint],
    field_path_lookup: &BTreeMap<String, LitStr>,
) {
    for constraint in constraints {
        constraint.left = resolve_known_input_constraint_path(&constraint.left, field_path_lookup);
        constraint.right =
            resolve_known_input_constraint_path(&constraint.right, field_path_lookup);
    }
}

pub(crate) fn resolve_constraint_strings_paths(
    constraints: &mut [PathStringsConstraint],
    field_path_lookup: &BTreeMap<String, LitStr>,
) {
    for constraint in constraints {
        constraint.path = resolve_known_input_constraint_path(&constraint.path, field_path_lookup);
    }
}

fn normalize_array_value_string_constraints(
    constraints: &mut [PathStringConstraint],
    field_path_lookup: &BTreeMap<String, LitStr>,
    array_field_paths: &BTreeSet<String>,
) {
    for constraint in constraints {
        constraint.path = normalize_array_value_constraint_path(
            &constraint.path,
            field_path_lookup,
            array_field_paths,
        );
    }
}

fn normalize_array_value_usize_constraints(
    constraints: &mut [PathUsizeConstraint],
    field_path_lookup: &BTreeMap<String, LitStr>,
    array_field_paths: &BTreeSet<String>,
) {
    for constraint in constraints {
        constraint.path = normalize_array_value_constraint_path(
            &constraint.path,
            field_path_lookup,
            array_field_paths,
        );
    }
}

fn normalize_array_value_expr_constraints(
    constraints: &mut [PathValueConstraint],
    field_path_lookup: &BTreeMap<String, LitStr>,
    array_field_paths: &BTreeSet<String>,
) {
    for constraint in constraints {
        constraint.path = normalize_array_value_constraint_path(
            &constraint.path,
            field_path_lookup,
            array_field_paths,
        );
    }
}

fn normalize_array_value_group_constraints(
    groups: &mut [Vec<LitStr>],
    field_path_lookup: &BTreeMap<String, LitStr>,
    array_field_paths: &BTreeSet<String>,
) {
    for group in groups {
        normalize_array_value_lit_paths(group, field_path_lookup, array_field_paths);
    }
}

fn normalize_array_value_pair_constraints(
    constraints: &mut [PathPairConstraint],
    field_path_lookup: &BTreeMap<String, LitStr>,
    array_field_paths: &BTreeSet<String>,
) {
    for constraint in constraints {
        constraint.left = normalize_array_value_constraint_path(
            &constraint.left,
            field_path_lookup,
            array_field_paths,
        );
        constraint.right = normalize_array_value_constraint_path(
            &constraint.right,
            field_path_lookup,
            array_field_paths,
        );
    }
}

// Keeping each constraint family explicit prevents accidental cross-family mutation.
#[allow(clippy::too_many_arguments)]
pub(crate) fn normalize_array_value_nested_path_constraints(
    non_empty: &mut [LitStr],
    non_empty_if_present: &mut [LitStr],
    exactly_one_of: &mut [Vec<LitStr>],
    at_least_one_of: &mut [Vec<LitStr>],
    requires: &mut [PathPairConstraint],
    conflicts_with: &mut [PathPairConstraint],
    required_unless_present: &mut [PathPairConstraint],
    distinct_trimmed_within: &mut [PathPairConstraint],
    field_path_lookup: &BTreeMap<String, LitStr>,
    array_field_paths: &BTreeSet<String>,
) {
    normalize_array_value_lit_paths(non_empty, field_path_lookup, array_field_paths);
    normalize_array_value_lit_paths(non_empty_if_present, field_path_lookup, array_field_paths);
    normalize_array_value_group_constraints(exactly_one_of, field_path_lookup, array_field_paths);
    normalize_array_value_group_constraints(at_least_one_of, field_path_lookup, array_field_paths);
    normalize_array_value_pair_constraints(requires, field_path_lookup, array_field_paths);
    normalize_array_value_pair_constraints(conflicts_with, field_path_lookup, array_field_paths);
    normalize_array_value_pair_constraints(
        required_unless_present,
        field_path_lookup,
        array_field_paths,
    );
    normalize_array_value_pair_constraints(
        distinct_trimmed_within,
        field_path_lookup,
        array_field_paths,
    );
}

fn normalize_array_value_values_constraints(
    constraints: &mut [PathValuesConstraint],
    field_path_lookup: &BTreeMap<String, LitStr>,
    array_field_paths: &BTreeSet<String>,
) {
    for constraint in constraints {
        constraint.path = normalize_array_value_constraint_path(
            &constraint.path,
            field_path_lookup,
            array_field_paths,
        );
    }
}

fn normalize_array_value_relation_constraints(
    forbid_substrings: &mut [PathStringsConstraint],
    distinct_trimmed: &mut [LitStr],
    field_path_lookup: &BTreeMap<String, LitStr>,
    array_field_paths: &BTreeSet<String>,
) {
    for constraint in forbid_substrings {
        constraint.path = normalize_array_value_constraint_path(
            &constraint.path,
            field_path_lookup,
            array_field_paths,
        );
    }
    normalize_array_value_lit_paths(distinct_trimmed, field_path_lookup, array_field_paths);
}

fn normalize_array_value_metadata(
    metadata: &mut [PluginInputFieldMetadata],
    field_path_lookup: &BTreeMap<String, LitStr>,
    array_field_paths: &BTreeSet<String>,
) {
    for field in metadata {
        let normalized = normalize_array_value_constraint_path(
            &field.parse_path,
            field_path_lookup,
            array_field_paths,
        );
        if normalized.value() == field.parse_path.value() {
            continue;
        }
        if let Some(value) = field.minimum.take()
            && field.item_minimum.is_none()
        {
            field.item_minimum = Some(value);
        }
        if let Some(value) = field.maximum.take()
            && field.item_maximum.is_none()
        {
            field.item_maximum = Some(value);
        }
        if let Some(value) = field.exclusive_minimum.take()
            && field.item_exclusive_minimum.is_none()
        {
            field.item_exclusive_minimum = Some(value);
        }
        if let Some(value) = field.exclusive_maximum.take()
            && field.item_exclusive_maximum.is_none()
        {
            field.item_exclusive_maximum = Some(value);
        }
        if let Some(value) = field.min_properties.take()
            && field.item_min_properties.is_none()
        {
            field.item_min_properties = Some(value);
        }
        if let Some(value) = field.max_properties.take()
            && field.item_max_properties.is_none()
        {
            field.item_max_properties = Some(value);
        }
        if let Some(value) = field.min_chars.take()
            && field.item_min_chars.is_none()
        {
            field.item_min_chars = Some(value);
        }
        if let Some(value) = field.max_chars.take()
            && field.item_max_chars.is_none()
        {
            field.item_max_chars = Some(value);
        }
        if let Some(value) = field.format.take()
            && field.item_format.is_none()
        {
            field.item_format = Some(value);
        }
        if let Some(value) = field.pattern.take()
            && field.item_pattern.is_none()
        {
            field.item_pattern = Some(value);
        }
        if !field.choices.is_empty() && field.item_choices.is_empty() {
            field.item_choices = std::mem::take(&mut field.choices);
        }
    }
}

// Keeping each constraint family explicit prevents accidental cross-family mutation.
#[allow(clippy::too_many_arguments)]
pub(crate) fn normalize_array_value_constraints(
    trim: &mut [LitStr],
    trim_suffix: &mut [PathStringConstraint],
    minimums: &mut [PathValueConstraint],
    maximums: &mut [PathValueConstraint],
    exclusive_minimums: &mut [PathValueConstraint],
    exclusive_maximums: &mut [PathValueConstraint],
    min_properties: &mut [PathUsizeConstraint],
    max_properties: &mut [PathUsizeConstraint],
    min_chars: &mut [PathUsizeConstraint],
    max_chars: &mut [PathUsizeConstraint],
    formats: &mut [PathStringConstraint],
    patterns: &mut [PathStringConstraint],
    choices: &mut [PathValuesConstraint],
    forbid_substrings: &mut [PathStringsConstraint],
    distinct_trimmed: &mut [LitStr],
    input_field_metadata: &mut [PluginInputFieldMetadata],
    field_path_lookup: &BTreeMap<String, LitStr>,
    array_field_paths: &BTreeSet<String>,
) {
    normalize_array_value_lit_paths(trim, field_path_lookup, array_field_paths);
    normalize_array_value_string_constraints(trim_suffix, field_path_lookup, array_field_paths);
    normalize_array_value_expr_constraints(minimums, field_path_lookup, array_field_paths);
    normalize_array_value_expr_constraints(maximums, field_path_lookup, array_field_paths);
    normalize_array_value_expr_constraints(
        exclusive_minimums,
        field_path_lookup,
        array_field_paths,
    );
    normalize_array_value_expr_constraints(
        exclusive_maximums,
        field_path_lookup,
        array_field_paths,
    );
    normalize_array_value_usize_constraints(min_properties, field_path_lookup, array_field_paths);
    normalize_array_value_usize_constraints(max_properties, field_path_lookup, array_field_paths);
    normalize_array_value_usize_constraints(min_chars, field_path_lookup, array_field_paths);
    normalize_array_value_usize_constraints(max_chars, field_path_lookup, array_field_paths);
    normalize_array_value_string_constraints(formats, field_path_lookup, array_field_paths);
    normalize_array_value_string_constraints(patterns, field_path_lookup, array_field_paths);
    normalize_array_value_values_constraints(choices, field_path_lookup, array_field_paths);
    normalize_array_value_relation_constraints(
        forbid_substrings,
        distinct_trimmed,
        field_path_lookup,
        array_field_paths,
    );
    normalize_array_value_metadata(input_field_metadata, field_path_lookup, array_field_paths);
}

pub(crate) fn prefixed_constraint_group(
    current: &LitStr,
    peers: &[LitStr],
    field_path_lookup: Option<&BTreeMap<String, LitStr>>,
) -> Vec<LitStr> {
    let mut seen = BTreeSet::new();
    let mut group = Vec::new();
    if seen.insert(current.value()) {
        group.push(current.clone());
    }
    for peer in peers {
        let resolved = resolve_known_constraint_path(peer, field_path_lookup);
        if seen.insert(resolved.value()) {
            group.push(resolved);
        }
    }
    group
}
