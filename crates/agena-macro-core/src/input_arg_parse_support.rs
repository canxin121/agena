//! Parsing of `#[arg(...)]` attributes on tool input fields.

use std::collections::BTreeSet;

use syn::{Attribute, Field, Fields, Meta, Result, Variant};

use super::input_arg_output_support::{
    ensure_unique_field_arg_names, set_field_arg_network, set_field_arg_path_kind,
    set_field_arg_picker,
};
use super::{
    ArgAttrArgs, PluginArgConfig, PluginNetworkSemantic, PluginPathPermissionKind,
    PluginPickerKind, SerdeRenameRule, ensure_arg_permission_locator_has_semantic,
    expr_array_lit_strs, expr_array_values, expr_lit_str, expr_lit_usize, field_has_serde_default,
    validate_format_lit, validate_input_jsonpath_lit, validate_pattern_lit,
};
use crate::input_arg_support::FieldArgConfig;

pub fn parse_input_field_arg_attrs(field: &Field) -> Result<FieldArgConfig> {
    let mut config = FieldArgConfig::default();
    for attr in &field.attrs {
        if !attr.path().is_ident("arg") {
            continue;
        }
        match &attr.meta {
            Meta::Path(_) => {}
            Meta::NameValue(_) => {
                return Err(syn::Error::new_spanned(
                    attr,
                    "#[arg] supports list syntax, for example #[arg(trim, non_empty)]",
                ));
            }
            Meta::List(_) => parse_input_field_arg_config_attr(attr, &mut config)?,
        }
    }
    Ok(config)
}

fn parse_input_field_arg_config_attr(attr: &Attribute, config: &mut FieldArgConfig) -> Result<()> {
    let args = attr.parse_args::<ArgAttrArgs>()?;
    for item in args.items {
        match (item.key.as_str(), item.value) {
            ("default", None) => config.default = true,
            ("default", Some(value)) => {
                if config.default || config.default_expr.replace(value).is_some() {
                    return Err(syn::Error::new_spanned(
                        item.first_ident,
                        "duplicate field #[arg(default)] or #[arg(default = ...)]",
                    ));
                }
            }
            ("trim", None) => config.trim = true,
            ("item_trim", None) => config.item_trim = true,
            ("non_empty", None) => config.non_empty = true,
            ("item_non_empty", None) => config.item_non_empty = true,
            ("non_empty_if_present", None) => config.non_empty_if_present = true,
            ("item_non_empty_if_present", None) => config.item_non_empty_if_present = true,
            ("distinct_trimmed", None) => config.distinct_trimmed = true,
            ("description", Some(value)) => {
                if config
                    .description
                    .replace(expr_lit_str(&value, "description")?)
                    .is_some()
                {
                    return Err(syn::Error::new_spanned(
                        item.first_ident,
                        "duplicate field #[arg(description = ...)]",
                    ));
                }
            }
            ("name", Some(value)) => {
                if config.name.replace(expr_lit_str(&value, "name")?).is_some() {
                    return Err(syn::Error::new_spanned(
                        item.first_ident,
                        "duplicate field #[arg(name = ...)]",
                    ));
                }
            }
            ("alias", Some(value)) => config.aliases.push(expr_lit_str(&value, "alias")?),
            ("path.read", None) => {
                set_field_arg_path_kind(config, PluginPathPermissionKind::Read, &item.first_ident)?
            }
            ("path.write", None) => {
                set_field_arg_path_kind(config, PluginPathPermissionKind::Write, &item.first_ident)?
            }
            ("network", None) => {
                set_field_arg_network(config, PluginNetworkSemantic::Network, &item.first_ident)?
            }
            ("network.url", None) => {
                set_field_arg_network(config, PluginNetworkSemantic::Url, &item.first_ident)?
            }
            ("network.host", None) => {
                set_field_arg_network(config, PluginNetworkSemantic::Host, &item.first_ident)?
            }
            ("network.internet", None) => {
                set_field_arg_network(config, PluginNetworkSemantic::Internet, &item.first_ident)?
            }
            ("network.private", None) => {
                set_field_arg_network(config, PluginNetworkSemantic::Private, &item.first_ident)?
            }
            ("optional", None) => config.optional = true,
            ("secret", None) => config.secret = true,
            ("file", None) => {
                set_field_arg_picker(config, PluginPickerKind::File, &item.first_ident)?
            }
            ("dir", None) => {
                set_field_arg_picker(config, PluginPickerKind::Dir, &item.first_ident)?
            }
            ("jsonpath", Some(value)) => {
                let jsonpath = expr_lit_str(&value, "jsonpath")?;
                validate_input_jsonpath_lit(&jsonpath)?;
                config.jsonpath = Some(jsonpath);
            }
            ("fallback", Some(value)) => config.fallback = Some(expr_lit_str(&value, "fallback")?),
            ("example", Some(value)) => config.example = Some(value),
            ("trim_suffix", Some(value)) => {
                config.trim_suffix = Some(expr_lit_str(&value, "trim_suffix")?)
            }
            ("item_trim_suffix", Some(value)) => {
                let suffix = expr_lit_str(&value, "item_trim_suffix")?;
                if config.item_trim_suffix.replace(suffix).is_some() {
                    return Err(syn::Error::new_spanned(
                        item.first_ident,
                        "duplicate field #[arg(item_trim_suffix = ...)]",
                    ));
                }
            }
            ("minimum", Some(value)) => {
                if config.minimum.replace(value).is_some() {
                    return Err(syn::Error::new_spanned(
                        item.first_ident,
                        "duplicate field #[arg(minimum = ...)]",
                    ));
                }
            }
            ("maximum", Some(value)) => {
                if config.maximum.replace(value).is_some() {
                    return Err(syn::Error::new_spanned(
                        item.first_ident,
                        "duplicate field #[arg(maximum = ...)]",
                    ));
                }
            }
            ("exclusive_minimum", Some(value)) => {
                if config.exclusive_minimum.replace(value).is_some() {
                    return Err(syn::Error::new_spanned(
                        item.first_ident,
                        "duplicate field #[arg(exclusive_minimum = ...)]",
                    ));
                }
            }
            ("exclusive_maximum", Some(value)) => {
                if config.exclusive_maximum.replace(value).is_some() {
                    return Err(syn::Error::new_spanned(
                        item.first_ident,
                        "duplicate field #[arg(exclusive_maximum = ...)]",
                    ));
                }
            }
            ("min_items", Some(value)) => {
                config.min_items = Some(expr_lit_usize(&value, "min_items")?)
            }
            ("max_items", Some(value)) => {
                config.max_items = Some(expr_lit_usize(&value, "max_items")?)
            }
            ("min_properties", Some(value)) => {
                config.min_properties = Some(expr_lit_usize(&value, "min_properties")?)
            }
            ("max_properties", Some(value)) => {
                config.max_properties = Some(expr_lit_usize(&value, "max_properties")?)
            }
            ("item_minimum", Some(value)) => {
                if config.item_minimum.replace(value).is_some() {
                    return Err(syn::Error::new_spanned(
                        item.first_ident,
                        "duplicate field #[arg(item_minimum = ...)]",
                    ));
                }
            }
            ("item_maximum", Some(value)) => {
                if config.item_maximum.replace(value).is_some() {
                    return Err(syn::Error::new_spanned(
                        item.first_ident,
                        "duplicate field #[arg(item_maximum = ...)]",
                    ));
                }
            }
            ("item_exclusive_minimum", Some(value)) => {
                if config.item_exclusive_minimum.replace(value).is_some() {
                    return Err(syn::Error::new_spanned(
                        item.first_ident,
                        "duplicate field #[arg(item_exclusive_minimum = ...)]",
                    ));
                }
            }
            ("item_exclusive_maximum", Some(value)) => {
                if config.item_exclusive_maximum.replace(value).is_some() {
                    return Err(syn::Error::new_spanned(
                        item.first_ident,
                        "duplicate field #[arg(item_exclusive_maximum = ...)]",
                    ));
                }
            }
            ("item_min_properties", Some(value)) => {
                config.item_min_properties = Some(expr_lit_usize(&value, "item_min_properties")?)
            }
            ("item_max_properties", Some(value)) => {
                config.item_max_properties = Some(expr_lit_usize(&value, "item_max_properties")?)
            }
            ("item_min_chars", Some(value)) => {
                config.item_min_chars = Some(expr_lit_usize(&value, "item_min_chars")?)
            }
            ("item_max_chars", Some(value)) => {
                config.item_max_chars = Some(expr_lit_usize(&value, "item_max_chars")?)
            }
            ("min_chars", Some(value)) => {
                config.min_chars = Some(expr_lit_usize(&value, "min_chars")?)
            }
            ("max_chars", Some(value)) => {
                config.max_chars = Some(expr_lit_usize(&value, "max_chars")?)
            }
            ("item_format", Some(value)) => {
                let format = validate_format_lit(&expr_lit_str(&value, "item_format")?)?;
                if config.item_format.replace(format).is_some() {
                    return Err(syn::Error::new_spanned(
                        item.first_ident,
                        "duplicate field #[arg(item_format = ...)]",
                    ));
                }
            }
            ("item_pattern", Some(value)) => {
                let pattern = expr_lit_str(&value, "item_pattern")?;
                validate_pattern_lit(&pattern)?;
                if config.item_pattern.replace(pattern).is_some() {
                    return Err(syn::Error::new_spanned(
                        item.first_ident,
                        "duplicate field #[arg(item_pattern = ...)]",
                    ));
                }
            }
            ("format", Some(value)) => {
                let format = validate_format_lit(&expr_lit_str(&value, "format")?)?;
                if config.format.replace(format).is_some() {
                    return Err(syn::Error::new_spanned(
                        item.first_ident,
                        "duplicate field #[arg(format = ...)]",
                    ));
                }
            }
            ("item_choices", Some(value)) => {
                if config
                    .item_choices
                    .replace(expr_array_values(&value, "item_choices")?)
                    .is_some()
                {
                    return Err(syn::Error::new_spanned(
                        item.first_ident,
                        "duplicate field #[arg(item_choices = [...])]",
                    ));
                }
            }
            ("exactly_one_of", Some(value)) => config
                .exactly_one_of
                .extend(expr_array_lit_strs(&value, "exactly_one_of")?),
            ("at_least_one_of", Some(value)) => config
                .at_least_one_of
                .extend(expr_array_lit_strs(&value, "at_least_one_of")?),
            ("requires", Some(value)) => config.requires.push(expr_lit_str(&value, "requires")?),
            ("conflicts_with", Some(value)) => config
                .conflicts_with
                .push(expr_lit_str(&value, "conflicts_with")?),
            ("required_unless_present", Some(value)) => config
                .required_unless_present
                .push(expr_lit_str(&value, "required_unless_present")?),
            ("forbid_substrings", Some(value)) => config
                .forbid_substrings
                .extend(expr_array_lit_strs(&value, "forbid_substrings")?),
            ("distinct_trimmed_within", Some(value)) => config
                .distinct_trimmed_within
                .push(expr_lit_str(&value, "distinct_trimmed_within")?),
            ("pattern", Some(value)) => {
                let pattern = expr_lit_str(&value, "pattern")?;
                validate_pattern_lit(&pattern)?;
                if config.pattern.replace(pattern).is_some() {
                    return Err(syn::Error::new_spanned(
                        item.first_ident,
                        "duplicate field #[arg(pattern = ...)]",
                    ));
                }
            }
            ("choices", Some(value)) => {
                if config
                    .choices
                    .replace(expr_array_values(&value, "choices")?)
                    .is_some()
                {
                    return Err(syn::Error::new_spanned(
                        item.first_ident,
                        "duplicate field #[arg(choices = [...])]",
                    ));
                }
            }
            (key, Some(_)) => {
                return Err(syn::Error::new_spanned(
                    item.first_ident,
                    format!("unsupported field #[arg] option '{key}'"),
                ));
            }
            (key, None) => {
                return Err(syn::Error::new_spanned(
                    item.first_ident,
                    format!("unsupported field #[arg] flag '{key}'"),
                ));
            }
        }
    }
    ensure_arg_permission_locator_has_semantic(
        config.jsonpath.as_ref(),
        config.fallback.as_ref(),
        config.path.is_some() || config.network.is_some(),
    )?;
    Ok(())
}

pub fn apply_input_variant_field_arg_attrs(
    config: &mut super::ToolInputVariantConfig,
    variant: &Variant,
    rename_rule: Option<SerdeRenameRule>,
) -> Result<()> {
    let Fields::Named(fields) = &variant.fields else {
        for field in variant.fields.iter() {
            let arg_config = parse_input_field_arg_attrs(field)?;
            if arg_config_has_constraints(&arg_config) {
                return Err(syn::Error::new_spanned(
                    field,
                    "field-level #[arg(...)] on ToolInput enum variants is only supported on named fields",
                ));
            }
        }
        return Ok(());
    };
    let mut all_field_names = Vec::new();
    let (field_path_lookup, array_field_paths) =
        super::input_constraint_field_lookup(&variant.fields, rename_rule)?;
    let mut prepared_fields = Vec::new();
    for (index, field) in fields.named.iter().enumerate() {
        let arg_config = parse_input_field_arg_attrs(field)?;
        if let Some(names) = super::prepare_input_field_names(field, rename_rule, &arg_config)? {
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
        let Some(names) = super::prepare_input_field_names(field, rename_rule, &arg_config)? else {
            return Err(syn::Error::new_spanned(
                field,
                "field-level #[arg(...)] cannot be used on flattened or skipped variant fields; put the constraint on the flattened input shape or remove the serde skip",
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
        crate::input_arg_support::apply_field_arg_config_to_input_variant(
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
    super::normalize_array_value_constraints(
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

pub fn arg_config_has_constraints(config: &FieldArgConfig) -> bool {
    config.default
        || config.default_expr.is_some()
        || config.description.is_some()
        || config.name.is_some()
        || !config.aliases.is_empty()
        || config.trim
        || config.item_trim
        || config.non_empty
        || config.item_non_empty
        || config.non_empty_if_present
        || config.item_non_empty_if_present
        || config.trim_suffix.is_some()
        || config.item_trim_suffix.is_some()
        || config.minimum.is_some()
        || config.maximum.is_some()
        || config.exclusive_minimum.is_some()
        || config.exclusive_maximum.is_some()
        || config.min_items.is_some()
        || config.max_items.is_some()
        || config.min_properties.is_some()
        || config.max_properties.is_some()
        || config.item_minimum.is_some()
        || config.item_maximum.is_some()
        || config.item_exclusive_minimum.is_some()
        || config.item_exclusive_maximum.is_some()
        || config.item_min_properties.is_some()
        || config.item_max_properties.is_some()
        || config.min_chars.is_some()
        || config.max_chars.is_some()
        || config.item_min_chars.is_some()
        || config.item_max_chars.is_some()
        || config.format.is_some()
        || config.item_format.is_some()
        || config.item_pattern.is_some()
        || config.pattern.is_some()
        || config.choices.is_some()
        || config.item_choices.is_some()
        || !config.requires.is_empty()
        || !config.conflicts_with.is_empty()
        || !config.required_unless_present.is_empty()
        || !config.forbid_substrings.is_empty()
        || config.distinct_trimmed
        || !config.exactly_one_of.is_empty()
        || !config.at_least_one_of.is_empty()
        || !config.distinct_trimmed_within.is_empty()
        || config.path.is_some()
        || config.network.is_some()
        || config.optional
        || config.jsonpath.is_some()
        || config.fallback.is_some()
        || config.example.is_some()
        || config.secret
        || config.picker.is_some()
}

pub fn inline_arg_has_default(config: &PluginArgConfig) -> bool {
    config.default || config.default_expr.is_some()
}

pub fn field_arg_has_default(config: &FieldArgConfig) -> bool {
    config.default || config.default_expr.is_some()
}
