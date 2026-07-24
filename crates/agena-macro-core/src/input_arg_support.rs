use std::collections::{BTreeMap, BTreeSet};

use syn::spanned::Spanned;
use syn::{Attribute, Data, Expr, Field, Fields, LitStr, Result, Type, Variant};

use super::{
    PathPairConstraint, PathStringConstraint, PathStringsConstraint, PathUsizeConstraint,
    PathValueConstraint, PathValuesConstraint, PluginInputNetworkSpec, PluginInputPathSpec,
    PluginNetworkSemantic, PluginPathPermissionKind, PluginPickerKind, SerdeRenameRule,
    ToolInputConfig, ToolInputVariantConfig, append_constraint_path_suffix,
    field_has_serde_default, field_schema_aliases, field_schema_property_name_with_rule,
    input_type_semantic_shape, normalize_array_value_constraints, normalize_array_value_lit_paths,
    normalize_array_value_nested_path_constraints, parse_input_variant_config,
    prefixed_constraint_group, resolve_constraint_expr_paths, resolve_constraint_group_paths,
    resolve_constraint_lit_paths, resolve_constraint_pair_paths, resolve_constraint_string_paths,
    resolve_constraint_strings_paths, resolve_constraint_usize_paths,
    resolve_constraint_values_paths, resolve_known_constraint_path, serde_rename_all_rule,
    validate_input_jsonpath,
};

use super::input_arg_output_support::{
    apply_arg_aliases_to_spec, apply_arg_default_to_spec, apply_arg_metadata_to_spec,
    ensure_unique_field_arg_names, input_jsonpath_for_arg, input_jsonpath_for_field,
};
use super::input_arg_parse_support::{
    apply_input_variant_field_arg_attrs, arg_config_has_constraints, field_arg_has_default,
    parse_input_field_arg_attrs,
};

#[derive(Default)]
pub struct FieldArgConfig {
    pub default: bool,
    pub default_expr: Option<Expr>,
    pub description: Option<LitStr>,
    pub name: Option<LitStr>,
    pub aliases: Vec<LitStr>,
    pub trim: bool,
    pub item_trim: bool,
    pub non_empty: bool,
    pub item_non_empty: bool,
    pub non_empty_if_present: bool,
    pub item_non_empty_if_present: bool,
    pub distinct_trimmed: bool,
    pub trim_suffix: Option<LitStr>,
    pub item_trim_suffix: Option<LitStr>,
    pub minimum: Option<Expr>,
    pub maximum: Option<Expr>,
    pub exclusive_minimum: Option<Expr>,
    pub exclusive_maximum: Option<Expr>,
    pub min_items: Option<usize>,
    pub max_items: Option<usize>,
    pub min_properties: Option<usize>,
    pub max_properties: Option<usize>,
    pub item_minimum: Option<Expr>,
    pub item_maximum: Option<Expr>,
    pub item_exclusive_minimum: Option<Expr>,
    pub item_exclusive_maximum: Option<Expr>,
    pub item_min_properties: Option<usize>,
    pub item_max_properties: Option<usize>,
    pub min_chars: Option<usize>,
    pub max_chars: Option<usize>,
    pub item_min_chars: Option<usize>,
    pub item_max_chars: Option<usize>,
    pub format: Option<LitStr>,
    pub item_format: Option<LitStr>,
    pub pattern: Option<LitStr>,
    pub item_pattern: Option<LitStr>,
    pub choices: Option<Vec<Expr>>,
    pub item_choices: Option<Vec<Expr>>,
    pub exactly_one_of: Vec<LitStr>,
    pub at_least_one_of: Vec<LitStr>,
    pub requires: Vec<LitStr>,
    pub conflicts_with: Vec<LitStr>,
    pub required_unless_present: Vec<LitStr>,
    pub forbid_substrings: Vec<LitStr>,
    pub distinct_trimmed_within: Vec<LitStr>,
    pub path: Option<PluginPathPermissionKind>,
    pub network: Option<PluginNetworkSemantic>,
    pub optional: bool,
    pub jsonpath: Option<LitStr>,
    pub fallback: Option<LitStr>,
    pub example: Option<Expr>,
    pub secret: bool,
    pub picker: Option<PluginPickerKind>,
}

#[derive(Clone)]
pub struct PluginInputFieldMetadata {
    pub path: LitStr,
    pub parse_path: LitStr,
    pub aliases: Vec<LitStr>,
    pub description: Option<LitStr>,
    pub path_kind: Option<PluginPathPermissionKind>,
    pub network: Option<PluginNetworkSemantic>,
    pub non_empty: bool,
    pub item_non_empty: bool,
    pub item_non_empty_if_present: bool,
    pub minimum: Option<Expr>,
    pub maximum: Option<Expr>,
    pub exclusive_minimum: Option<Expr>,
    pub exclusive_maximum: Option<Expr>,
    pub min_items: Option<usize>,
    pub max_items: Option<usize>,
    pub min_properties: Option<usize>,
    pub max_properties: Option<usize>,
    pub item_minimum: Option<Expr>,
    pub item_maximum: Option<Expr>,
    pub item_exclusive_minimum: Option<Expr>,
    pub item_exclusive_maximum: Option<Expr>,
    pub item_min_properties: Option<usize>,
    pub item_max_properties: Option<usize>,
    pub min_chars: Option<usize>,
    pub max_chars: Option<usize>,
    pub item_min_chars: Option<usize>,
    pub item_max_chars: Option<usize>,
    pub format: Option<LitStr>,
    pub item_format: Option<LitStr>,
    pub pattern: Option<LitStr>,
    pub item_pattern: Option<LitStr>,
    pub example: Option<Expr>,
    pub choices: Vec<Expr>,
    pub item_choices: Vec<Expr>,
    pub secret: bool,
    pub picker: Option<PluginPickerKind>,
}

#[derive(Clone)]
pub struct PluginInputFieldDefaultSpec {
    pub schema_path: LitStr,
    pub parse_path: LitStr,
    pub aliases: Vec<LitStr>,
    pub ty: Type,
    pub default_expr: Option<Expr>,
}

#[derive(Clone)]
pub struct PluginInputFieldAliasSpec {
    pub path: LitStr,
    pub aliases: Vec<LitStr>,
}

pub struct PreparedInputFieldNames {
    pub schema_path: LitStr,
    pub parse_path: LitStr,
    pub schema_aliases: Vec<LitStr>,
    pub parse_aliases: Vec<LitStr>,
}

pub fn prepare_input_field_names(
    field: &Field,
    rename_rule: Option<SerdeRenameRule>,
    arg_config: &FieldArgConfig,
) -> Result<Option<PreparedInputFieldNames>> {
    let Some(parse_path) = field_schema_property_name_with_rule(field, rename_rule)? else {
        return Ok(None);
    };
    let parse_path = LitStr::new(&parse_path, field.span());
    let schema_path = arg_config
        .name
        .clone()
        .unwrap_or_else(|| parse_path.clone());
    let serde_aliases = field_schema_aliases(field)?;

    let mut schema_side_aliases = serde_aliases.clone();
    if schema_path.value() != parse_path.value() {
        schema_side_aliases.insert(0, parse_path.value());
    }
    let schema_aliases =
        merged_field_arg_aliases(&schema_path, &schema_side_aliases, &arg_config.aliases)?;

    let mut parse_side_aliases = serde_aliases;
    if schema_path.value() != parse_path.value() {
        parse_side_aliases.insert(0, schema_path.value());
    }
    let parse_aliases =
        merged_field_arg_aliases(&parse_path, &parse_side_aliases, &arg_config.aliases)?;

    Ok(Some(PreparedInputFieldNames {
        schema_path,
        parse_path,
        schema_aliases,
        parse_aliases,
    }))
}

pub fn input_constraint_field_lookup(
    fields: &Fields,
    rename_rule: Option<SerdeRenameRule>,
) -> Result<(BTreeMap<String, LitStr>, BTreeSet<String>)> {
    let Fields::Named(named) = fields else {
        return Ok((BTreeMap::new(), BTreeSet::new()));
    };

    let mut array_field_paths = BTreeSet::new();
    let mut field_path_lookup = BTreeMap::new();
    for field in &named.named {
        let arg_config = parse_input_field_arg_attrs(field)?;
        let Some(names) = prepare_input_field_names(field, rename_rule, &arg_config)? else {
            continue;
        };
        let raw_field_name = field
            .ident
            .as_ref()
            .map(|ident| LitStr::new(ident.to_string().as_str(), ident.span()));
        if input_type_semantic_shape(&field.ty).array {
            array_field_paths.insert(names.parse_path.value());
        }
        for candidate in raw_field_name
            .iter()
            .chain(std::iter::once(&names.schema_path))
            .chain(std::iter::once(&names.parse_path))
            .chain(names.schema_aliases.iter())
            .chain(names.parse_aliases.iter())
        {
            if let Some(existing) =
                field_path_lookup.insert(candidate.value(), names.parse_path.clone())
                && existing.value() != names.parse_path.value()
            {
                return Err(syn::Error::new_spanned(
                    candidate,
                    format!(
                        "duplicate ToolInput field wire name or alias `{}`",
                        candidate.value()
                    ),
                ));
            }
        }
    }

    Ok((field_path_lookup, array_field_paths))
}

pub fn validate_input_jsonpath_lit(jsonpath: &LitStr) -> Result<()> {
    validate_input_jsonpath(jsonpath.value().as_str())
        .map_err(|message| syn::Error::new_spanned(jsonpath, message))
}

pub fn apply_input_field_arg_attrs(
    config: &mut ToolInputConfig,
    attrs: &[Attribute],
    data: &Data,
) -> Result<()> {
    let Data::Struct(data_struct) = data else {
        return Ok(());
    };
    let Fields::Named(fields) = &data_struct.fields else {
        return Ok(());
    };
    let rename_rule = serde_rename_all_rule(attrs)?;
    let mut all_field_names = Vec::new();
    let (field_path_lookup, array_field_paths) =
        input_constraint_field_lookup(&Fields::Named(fields.clone()), rename_rule)?;
    let mut prepared_fields = Vec::new();
    for (index, field) in fields.named.iter().enumerate() {
        let arg_config = parse_input_field_arg_attrs(field)?;
        if let Some(names) = prepare_input_field_names(field, rename_rule, &arg_config)? {
            let mut accepted_names = BTreeSet::new();
            ensure_unique_field_arg_names(
                &mut accepted_names,
                &names.schema_path,
                &names.schema_aliases,
            )?;
            all_field_names.push((index, accepted_names));
        }
        if !arg_config_has_constraints(&arg_config) {
            continue;
        }
        let Some(names) = prepare_input_field_names(field, rename_rule, &arg_config)? else {
            return Err(syn::Error::new_spanned(
                field,
                "field-level #[arg(...)] cannot be used on flattened or skipped fields; put the constraint on the flattened input shape or remove the serde skip",
            ));
        };
        let serde_default = field_has_serde_default(field)?;
        if field_arg_has_default(&arg_config) && serde_default {
            return Err(syn::Error::new_spanned(
                field,
                "field-level #[arg(default)] and #[serde(default)] cannot be combined; keep one default source",
            ));
        }
        prepared_fields.push((index, field, names, serde_default, arg_config));
    }
    let mut seen_field_names = BTreeSet::new();
    for (index, field, names, serde_default, arg_config) in prepared_fields {
        for candidate in std::iter::once(&names.schema_path).chain(names.schema_aliases.iter()) {
            for (other_index, other_names) in &all_field_names {
                if *other_index != index && other_names.contains(&candidate.value()) {
                    return Err(syn::Error::new_spanned(
                        candidate,
                        format!(
                            "duplicate ToolInput field wire name or alias `{}`",
                            candidate.value()
                        ),
                    ));
                }
            }
        }
        ensure_unique_field_arg_names(
            &mut seen_field_names,
            &names.schema_path,
            &names.schema_aliases,
        )?;
        apply_field_arg_config_to_input(
            config,
            &field_path_lookup,
            &names.schema_path,
            &names.parse_path,
            &names.schema_aliases,
            &names.parse_aliases,
            &field.ty,
            serde_default,
            &arg_config,
        );
    }
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
    Ok(())
}

pub fn normalized_input_variant_config(
    variant: &Variant,
    enum_field_rule: Option<SerdeRenameRule>,
) -> Result<ToolInputVariantConfig> {
    let mut config = parse_input_variant_config(variant)?;
    let variant_field_rule = serde_rename_all_rule(&variant.attrs)?.or(enum_field_rule);
    apply_input_variant_field_arg_attrs(&mut config, variant, variant_field_rule)?;
    let (field_path_lookup, array_field_paths) =
        input_constraint_field_lookup(&variant.fields, variant_field_rule)?;
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
    resolve_constraint_lit_paths(&mut config.infer_when_present, &field_path_lookup);
    resolve_constraint_lit_paths(&mut config.drop_keys, &field_path_lookup);
    normalize_array_value_lit_paths(
        &mut config.infer_when_present,
        &field_path_lookup,
        &array_field_paths,
    );
    normalize_array_value_lit_paths(
        &mut config.drop_keys,
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
    Ok(config)
}

fn merged_field_arg_aliases(
    field_name: &LitStr,
    serde_aliases: &[String],
    arg_aliases: &[LitStr],
) -> Result<Vec<LitStr>> {
    let mut seen = BTreeSet::from([field_name.value()]);
    let mut aliases = Vec::new();
    for alias in serde_aliases {
        if !seen.insert(alias.clone()) {
            return Err(syn::Error::new(
                field_name.span(),
                format!("duplicate ToolInput field wire name or alias `{alias}`"),
            ));
        }
        aliases.push(LitStr::new(alias, field_name.span()));
    }
    for alias in arg_aliases {
        if !seen.insert(alias.value()) {
            return Err(syn::Error::new_spanned(
                alias,
                format!(
                    "duplicate ToolInput field wire name or alias `{}`",
                    alias.value()
                ),
            ));
        }
        aliases.push(alias.clone());
    }
    Ok(aliases)
}
#[allow(clippy::too_many_arguments)]
fn apply_field_arg_config_to_input(
    target: &mut ToolInputConfig,
    field_path_lookup: &BTreeMap<String, LitStr>,
    schema_path: &LitStr,
    parse_path: &LitStr,
    schema_aliases: &[LitStr],
    parse_aliases: &[LitStr],
    ty: &Type,
    serde_default: bool,
    config: &FieldArgConfig,
) {
    if config.trim {
        target.trim.push(parse_path.clone());
    }
    if config.item_trim {
        target
            .trim
            .push(append_constraint_path_suffix(parse_path, "[]"));
    }
    if config.non_empty {
        target.non_empty.push(parse_path.clone());
    }
    if config.item_non_empty {
        target
            .non_empty
            .push(append_constraint_path_suffix(parse_path, "[]"));
    }
    if config.non_empty_if_present {
        target.non_empty_if_present.push(parse_path.clone());
    }
    if config.item_non_empty_if_present {
        target
            .non_empty_if_present
            .push(append_constraint_path_suffix(parse_path, "[]"));
    }
    if let Some(value) = config.trim_suffix.as_ref() {
        target.trim_suffix.push(PathStringConstraint {
            path: parse_path.clone(),
            value: value.clone(),
        });
    }
    if let Some(value) = config.item_trim_suffix.as_ref() {
        target.trim_suffix.push(PathStringConstraint {
            path: append_constraint_path_suffix(parse_path, "[]"),
            value: value.clone(),
        });
    }
    if let Some(value) = config.minimum.as_ref() {
        target.minimums.push(PathValueConstraint {
            path: parse_path.clone(),
            value: value.clone(),
        });
    }
    if let Some(value) = config.maximum.as_ref() {
        target.maximums.push(PathValueConstraint {
            path: parse_path.clone(),
            value: value.clone(),
        });
    }
    if let Some(value) = config.exclusive_minimum.as_ref() {
        target.exclusive_minimums.push(PathValueConstraint {
            path: parse_path.clone(),
            value: value.clone(),
        });
    }
    if let Some(value) = config.exclusive_maximum.as_ref() {
        target.exclusive_maximums.push(PathValueConstraint {
            path: parse_path.clone(),
            value: value.clone(),
        });
    }
    if let Some(value) = config.min_items {
        target.min_items.push(PathUsizeConstraint {
            path: parse_path.clone(),
            value,
        });
    }
    if let Some(value) = config.max_items {
        target.max_items.push(PathUsizeConstraint {
            path: parse_path.clone(),
            value,
        });
    }
    if let Some(value) = config.min_properties {
        target.min_properties.push(PathUsizeConstraint {
            path: parse_path.clone(),
            value,
        });
    }
    if let Some(value) = config.max_properties {
        target.max_properties.push(PathUsizeConstraint {
            path: parse_path.clone(),
            value,
        });
    }
    if let Some(value) = config.item_minimum.as_ref() {
        target.minimums.push(PathValueConstraint {
            path: append_constraint_path_suffix(parse_path, "[]"),
            value: value.clone(),
        });
    }
    if let Some(value) = config.item_maximum.as_ref() {
        target.maximums.push(PathValueConstraint {
            path: append_constraint_path_suffix(parse_path, "[]"),
            value: value.clone(),
        });
    }
    if let Some(value) = config.item_exclusive_minimum.as_ref() {
        target.exclusive_minimums.push(PathValueConstraint {
            path: append_constraint_path_suffix(parse_path, "[]"),
            value: value.clone(),
        });
    }
    if let Some(value) = config.item_exclusive_maximum.as_ref() {
        target.exclusive_maximums.push(PathValueConstraint {
            path: append_constraint_path_suffix(parse_path, "[]"),
            value: value.clone(),
        });
    }
    if let Some(value) = config.item_min_properties {
        target.min_properties.push(PathUsizeConstraint {
            path: append_constraint_path_suffix(parse_path, "[]"),
            value,
        });
    }
    if let Some(value) = config.item_max_properties {
        target.max_properties.push(PathUsizeConstraint {
            path: append_constraint_path_suffix(parse_path, "[]"),
            value,
        });
    }
    if let Some(value) = config.min_chars {
        target.min_chars.push(PathUsizeConstraint {
            path: parse_path.clone(),
            value,
        });
    }
    if let Some(value) = config.max_chars {
        target.max_chars.push(PathUsizeConstraint {
            path: parse_path.clone(),
            value,
        });
    }
    if let Some(value) = config.item_min_chars {
        target.min_chars.push(PathUsizeConstraint {
            path: append_constraint_path_suffix(parse_path, "[]"),
            value,
        });
    }
    if let Some(value) = config.item_max_chars {
        target.max_chars.push(PathUsizeConstraint {
            path: append_constraint_path_suffix(parse_path, "[]"),
            value,
        });
    }
    if let Some(value) = config.format.as_ref() {
        target.formats.push(PathStringConstraint {
            path: parse_path.clone(),
            value: value.clone(),
        });
    }
    if let Some(value) = config.item_format.as_ref() {
        target.formats.push(PathStringConstraint {
            path: append_constraint_path_suffix(parse_path, "[]"),
            value: value.clone(),
        });
    }
    if let Some(value) = config.pattern.as_ref() {
        target.patterns.push(PathStringConstraint {
            path: parse_path.clone(),
            value: value.clone(),
        });
    }
    if let Some(value) = config.item_pattern.as_ref() {
        target.patterns.push(PathStringConstraint {
            path: append_constraint_path_suffix(parse_path, "[]"),
            value: value.clone(),
        });
    }
    if let Some(values) = config.choices.as_ref() {
        target.choices.push(PathValuesConstraint {
            path: parse_path.clone(),
            values: values.clone(),
        });
    }
    if let Some(values) = config.item_choices.as_ref() {
        target.choices.push(PathValuesConstraint {
            path: append_constraint_path_suffix(parse_path, "[]"),
            values: values.clone(),
        });
    }
    let type_shape = input_type_semantic_shape(ty);
    let field_value_path = if type_shape.array {
        append_constraint_path_suffix(parse_path, "[]")
    } else {
        parse_path.clone()
    };
    target
        .requires
        .extend(config.requires.iter().map(|right| PathPairConstraint {
            left: parse_path.clone(),
            right: resolve_known_constraint_path(right, Some(field_path_lookup)),
        }));
    target.conflicts_with.extend(
        config
            .conflicts_with
            .iter()
            .map(|right| PathPairConstraint {
                left: parse_path.clone(),
                right: resolve_known_constraint_path(right, Some(field_path_lookup)),
            }),
    );
    target
        .required_unless_present
        .extend(
            config
                .required_unless_present
                .iter()
                .map(|right| PathPairConstraint {
                    left: parse_path.clone(),
                    right: resolve_known_constraint_path(right, Some(field_path_lookup)),
                }),
        );
    if !config.forbid_substrings.is_empty() {
        target.forbid_substrings.push(PathStringsConstraint {
            path: field_value_path.clone(),
            values: config.forbid_substrings.clone(),
        });
    }
    if config.distinct_trimmed {
        target.distinct_trimmed.push(field_value_path.clone());
    }
    if !config.exactly_one_of.is_empty() {
        target.exactly_one_of.push(prefixed_constraint_group(
            parse_path,
            &config.exactly_one_of,
            Some(field_path_lookup),
        ));
    }
    if !config.at_least_one_of.is_empty() {
        target.at_least_one_of.push(prefixed_constraint_group(
            parse_path,
            &config.at_least_one_of,
            Some(field_path_lookup),
        ));
    }
    target
        .distinct_trimmed_within
        .extend(
            config
                .distinct_trimmed_within
                .iter()
                .map(|right| PathPairConstraint {
                    left: parse_path.clone(),
                    right: resolve_known_constraint_path(right, Some(field_path_lookup)),
                }),
        );
    let optional = config.optional
        || serde_default
        || field_arg_has_default(config)
        || type_shape.optional
        || !schema_aliases.is_empty();
    let jsonpath = input_jsonpath_for_arg(schema_path, ty, config.jsonpath.as_ref());
    if let Some(kind) = config.path {
        target.input_paths.push(PluginInputPathSpec {
            jsonpath: jsonpath.clone(),
            kind,
            fallback: config.fallback.clone(),
            optional,
        });
        if config.jsonpath.is_none() {
            target
                .input_paths
                .extend(schema_aliases.iter().map(|alias| PluginInputPathSpec {
                    jsonpath: input_jsonpath_for_field(alias, ty),
                    kind,
                    fallback: config.fallback.clone(),
                    optional,
                }));
        }
    }
    if let Some(semantic) = config.network {
        target.input_networks.push(PluginInputNetworkSpec {
            jsonpath,
            fallback: config.fallback.clone(),
            optional,
            semantic,
        });
        if config.jsonpath.is_none() {
            target
                .input_networks
                .extend(schema_aliases.iter().map(|alias| PluginInputNetworkSpec {
                    jsonpath: input_jsonpath_for_field(alias, ty),
                    fallback: config.fallback.clone(),
                    optional,
                    semantic,
                }));
        }
    }
    apply_arg_metadata_to_spec(
        &mut target.input_field_metadata,
        schema_path,
        parse_path,
        schema_aliases,
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
    apply_arg_default_to_spec(
        &mut target.input_defaults,
        schema_path,
        parse_path,
        parse_aliases,
        ty,
        config.default,
        config.default_expr.clone(),
    );
    apply_arg_aliases_to_spec(&mut target.input_aliases, parse_path, parse_aliases);
}

#[allow(clippy::too_many_arguments)]
pub fn apply_field_arg_config_to_input_variant(
    target: &mut ToolInputVariantConfig,
    field_path_lookup: &BTreeMap<String, LitStr>,
    schema_path: &LitStr,
    parse_path: &LitStr,
    schema_aliases: &[LitStr],
    parse_aliases: &[LitStr],
    ty: &Type,
    serde_default: bool,
    config: &FieldArgConfig,
) {
    if config.trim {
        target.trim.push(parse_path.clone());
    }
    if config.item_trim {
        target
            .trim
            .push(append_constraint_path_suffix(parse_path, "[]"));
    }
    if config.non_empty {
        target.non_empty.push(parse_path.clone());
    }
    if config.item_non_empty {
        target
            .non_empty
            .push(append_constraint_path_suffix(parse_path, "[]"));
    }
    if config.non_empty_if_present {
        target.non_empty_if_present.push(parse_path.clone());
    }
    if config.item_non_empty_if_present {
        target
            .non_empty_if_present
            .push(append_constraint_path_suffix(parse_path, "[]"));
    }
    if let Some(value) = config.trim_suffix.as_ref() {
        target.trim_suffix.push(PathStringConstraint {
            path: parse_path.clone(),
            value: value.clone(),
        });
    }
    if let Some(value) = config.item_trim_suffix.as_ref() {
        target.trim_suffix.push(PathStringConstraint {
            path: append_constraint_path_suffix(parse_path, "[]"),
            value: value.clone(),
        });
    }
    if let Some(value) = config.minimum.as_ref() {
        target.minimums.push(PathValueConstraint {
            path: parse_path.clone(),
            value: value.clone(),
        });
    }
    if let Some(value) = config.maximum.as_ref() {
        target.maximums.push(PathValueConstraint {
            path: parse_path.clone(),
            value: value.clone(),
        });
    }
    if let Some(value) = config.exclusive_minimum.as_ref() {
        target.exclusive_minimums.push(PathValueConstraint {
            path: parse_path.clone(),
            value: value.clone(),
        });
    }
    if let Some(value) = config.exclusive_maximum.as_ref() {
        target.exclusive_maximums.push(PathValueConstraint {
            path: parse_path.clone(),
            value: value.clone(),
        });
    }
    if let Some(value) = config.min_items {
        target.min_items.push(PathUsizeConstraint {
            path: parse_path.clone(),
            value,
        });
    }
    if let Some(value) = config.max_items {
        target.max_items.push(PathUsizeConstraint {
            path: parse_path.clone(),
            value,
        });
    }
    if let Some(value) = config.min_properties {
        target.min_properties.push(PathUsizeConstraint {
            path: parse_path.clone(),
            value,
        });
    }
    if let Some(value) = config.max_properties {
        target.max_properties.push(PathUsizeConstraint {
            path: parse_path.clone(),
            value,
        });
    }
    if let Some(value) = config.item_minimum.as_ref() {
        target.minimums.push(PathValueConstraint {
            path: append_constraint_path_suffix(parse_path, "[]"),
            value: value.clone(),
        });
    }
    if let Some(value) = config.item_maximum.as_ref() {
        target.maximums.push(PathValueConstraint {
            path: append_constraint_path_suffix(parse_path, "[]"),
            value: value.clone(),
        });
    }
    if let Some(value) = config.item_exclusive_minimum.as_ref() {
        target.exclusive_minimums.push(PathValueConstraint {
            path: append_constraint_path_suffix(parse_path, "[]"),
            value: value.clone(),
        });
    }
    if let Some(value) = config.item_exclusive_maximum.as_ref() {
        target.exclusive_maximums.push(PathValueConstraint {
            path: append_constraint_path_suffix(parse_path, "[]"),
            value: value.clone(),
        });
    }
    if let Some(value) = config.item_min_properties {
        target.min_properties.push(PathUsizeConstraint {
            path: append_constraint_path_suffix(parse_path, "[]"),
            value,
        });
    }
    if let Some(value) = config.item_max_properties {
        target.max_properties.push(PathUsizeConstraint {
            path: append_constraint_path_suffix(parse_path, "[]"),
            value,
        });
    }
    if let Some(value) = config.min_chars {
        target.min_chars.push(PathUsizeConstraint {
            path: parse_path.clone(),
            value,
        });
    }
    if let Some(value) = config.max_chars {
        target.max_chars.push(PathUsizeConstraint {
            path: parse_path.clone(),
            value,
        });
    }
    if let Some(value) = config.item_min_chars {
        target.min_chars.push(PathUsizeConstraint {
            path: append_constraint_path_suffix(parse_path, "[]"),
            value,
        });
    }
    if let Some(value) = config.item_max_chars {
        target.max_chars.push(PathUsizeConstraint {
            path: append_constraint_path_suffix(parse_path, "[]"),
            value,
        });
    }
    if let Some(value) = config.format.as_ref() {
        target.formats.push(PathStringConstraint {
            path: parse_path.clone(),
            value: value.clone(),
        });
    }
    if let Some(value) = config.item_format.as_ref() {
        target.formats.push(PathStringConstraint {
            path: append_constraint_path_suffix(parse_path, "[]"),
            value: value.clone(),
        });
    }
    if let Some(value) = config.pattern.as_ref() {
        target.patterns.push(PathStringConstraint {
            path: parse_path.clone(),
            value: value.clone(),
        });
    }
    if let Some(value) = config.item_pattern.as_ref() {
        target.patterns.push(PathStringConstraint {
            path: append_constraint_path_suffix(parse_path, "[]"),
            value: value.clone(),
        });
    }
    if let Some(values) = config.choices.as_ref() {
        target.choices.push(PathValuesConstraint {
            path: parse_path.clone(),
            values: values.clone(),
        });
    }
    if let Some(values) = config.item_choices.as_ref() {
        target.choices.push(PathValuesConstraint {
            path: append_constraint_path_suffix(parse_path, "[]"),
            values: values.clone(),
        });
    }
    let type_shape = input_type_semantic_shape(ty);
    let field_value_path = if type_shape.array {
        append_constraint_path_suffix(parse_path, "[]")
    } else {
        parse_path.clone()
    };
    target
        .requires
        .extend(config.requires.iter().map(|right| PathPairConstraint {
            left: parse_path.clone(),
            right: resolve_known_constraint_path(right, Some(field_path_lookup)),
        }));
    target.conflicts_with.extend(
        config
            .conflicts_with
            .iter()
            .map(|right| PathPairConstraint {
                left: parse_path.clone(),
                right: resolve_known_constraint_path(right, Some(field_path_lookup)),
            }),
    );
    target
        .required_unless_present
        .extend(
            config
                .required_unless_present
                .iter()
                .map(|right| PathPairConstraint {
                    left: parse_path.clone(),
                    right: resolve_known_constraint_path(right, Some(field_path_lookup)),
                }),
        );
    if !config.forbid_substrings.is_empty() {
        target.forbid_substrings.push(PathStringsConstraint {
            path: field_value_path.clone(),
            values: config.forbid_substrings.clone(),
        });
    }
    if config.distinct_trimmed {
        target.distinct_trimmed.push(field_value_path.clone());
    }
    if !config.exactly_one_of.is_empty() {
        target.exactly_one_of.push(prefixed_constraint_group(
            parse_path,
            &config.exactly_one_of,
            Some(field_path_lookup),
        ));
    }
    if !config.at_least_one_of.is_empty() {
        target.at_least_one_of.push(prefixed_constraint_group(
            parse_path,
            &config.at_least_one_of,
            Some(field_path_lookup),
        ));
    }
    target
        .distinct_trimmed_within
        .extend(
            config
                .distinct_trimmed_within
                .iter()
                .map(|right| PathPairConstraint {
                    left: parse_path.clone(),
                    right: resolve_known_constraint_path(right, Some(field_path_lookup)),
                }),
        );
    let optional = config.optional
        || serde_default
        || field_arg_has_default(config)
        || type_shape.optional
        || !schema_aliases.is_empty();
    let jsonpath = input_jsonpath_for_arg(schema_path, ty, config.jsonpath.as_ref());
    if let Some(kind) = config.path {
        target.input_paths.push(PluginInputPathSpec {
            jsonpath: jsonpath.clone(),
            kind,
            fallback: config.fallback.clone(),
            optional,
        });
        if config.jsonpath.is_none() {
            target
                .input_paths
                .extend(schema_aliases.iter().map(|alias| PluginInputPathSpec {
                    jsonpath: input_jsonpath_for_field(alias, ty),
                    kind,
                    fallback: config.fallback.clone(),
                    optional,
                }));
        }
    }
    if let Some(semantic) = config.network {
        target.input_networks.push(PluginInputNetworkSpec {
            jsonpath,
            fallback: config.fallback.clone(),
            optional,
            semantic,
        });
        if config.jsonpath.is_none() {
            target
                .input_networks
                .extend(schema_aliases.iter().map(|alias| PluginInputNetworkSpec {
                    jsonpath: input_jsonpath_for_field(alias, ty),
                    fallback: config.fallback.clone(),
                    optional,
                    semantic,
                }));
        }
    }
    apply_arg_metadata_to_spec(
        &mut target.input_field_metadata,
        schema_path,
        parse_path,
        schema_aliases,
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
    apply_arg_default_to_spec(
        &mut target.input_defaults,
        schema_path,
        parse_path,
        parse_aliases,
        ty,
        config.default,
        config.default_expr.clone(),
    );
    apply_arg_aliases_to_spec(&mut target.input_aliases, parse_path, parse_aliases);
}
