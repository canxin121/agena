use std::collections::BTreeSet;

use syn::{Expr, LitStr, Result, Type};

use super::{
    PluginInputFieldAliasSpec, PluginInputFieldDefaultSpec, PluginInputFieldMetadata,
    PluginNetworkSemantic, PluginPathPermissionKind, PluginPickerKind,
};

pub(crate) fn apply_arg_default_to_spec(
    target: &mut Vec<PluginInputFieldDefaultSpec>,
    schema_path: &LitStr,
    parse_path: &LitStr,
    aliases: &[LitStr],
    ty: &Type,
    default: bool,
    default_expr: Option<Expr>,
) {
    if !default && default_expr.is_none() {
        return;
    }
    target.push(PluginInputFieldDefaultSpec {
        schema_path: schema_path.clone(),
        parse_path: parse_path.clone(),
        aliases: aliases.to_vec(),
        ty: ty.clone(),
        default_expr,
    });
}

pub(crate) fn apply_arg_aliases_to_spec(
    target: &mut Vec<PluginInputFieldAliasSpec>,
    field_name: &LitStr,
    aliases: &[LitStr],
) {
    if aliases.is_empty() {
        return;
    }
    target.push(PluginInputFieldAliasSpec {
        path: field_name.clone(),
        aliases: aliases.to_vec(),
    });
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn apply_arg_metadata_to_spec(
    target: &mut Vec<PluginInputFieldMetadata>,
    schema_path: &LitStr,
    parse_path: &LitStr,
    aliases: &[LitStr],
    description: Option<LitStr>,
    path_kind: Option<PluginPathPermissionKind>,
    network: Option<PluginNetworkSemantic>,
    non_empty: bool,
    item_non_empty: bool,
    item_non_empty_if_present: bool,
    minimum: Option<Expr>,
    maximum: Option<Expr>,
    exclusive_minimum: Option<Expr>,
    exclusive_maximum: Option<Expr>,
    min_items: Option<usize>,
    max_items: Option<usize>,
    min_properties: Option<usize>,
    max_properties: Option<usize>,
    item_minimum: Option<Expr>,
    item_maximum: Option<Expr>,
    item_exclusive_minimum: Option<Expr>,
    item_exclusive_maximum: Option<Expr>,
    item_min_properties: Option<usize>,
    item_max_properties: Option<usize>,
    min_chars: Option<usize>,
    max_chars: Option<usize>,
    item_min_chars: Option<usize>,
    item_max_chars: Option<usize>,
    format: Option<LitStr>,
    item_format: Option<LitStr>,
    pattern: Option<LitStr>,
    item_pattern: Option<LitStr>,
    example: Option<Expr>,
    choices: Vec<Expr>,
    item_choices: Vec<Expr>,
    secret: bool,
    picker: Option<PluginPickerKind>,
) {
    if description.is_none()
        && path_kind.is_none()
        && network.is_none()
        && !non_empty
        && !item_non_empty
        && !item_non_empty_if_present
        && minimum.is_none()
        && maximum.is_none()
        && exclusive_minimum.is_none()
        && exclusive_maximum.is_none()
        && min_items.is_none()
        && max_items.is_none()
        && min_properties.is_none()
        && max_properties.is_none()
        && item_minimum.is_none()
        && item_maximum.is_none()
        && item_exclusive_minimum.is_none()
        && item_exclusive_maximum.is_none()
        && item_min_properties.is_none()
        && item_max_properties.is_none()
        && min_chars.is_none()
        && max_chars.is_none()
        && item_min_chars.is_none()
        && item_max_chars.is_none()
        && format.is_none()
        && item_format.is_none()
        && pattern.is_none()
        && item_pattern.is_none()
        && example.is_none()
        && choices.is_empty()
        && item_choices.is_empty()
        && aliases.is_empty()
        && !secret
        && picker.is_none()
    {
        return;
    }
    target.push(PluginInputFieldMetadata {
        path: schema_path.clone(),
        parse_path: parse_path.clone(),
        aliases: aliases.to_vec(),
        description,
        path_kind,
        network,
        non_empty,
        item_non_empty,
        item_non_empty_if_present,
        minimum,
        maximum,
        exclusive_minimum,
        exclusive_maximum,
        min_items,
        max_items,
        min_properties,
        max_properties,
        item_minimum,
        item_maximum,
        item_exclusive_minimum,
        item_exclusive_maximum,
        item_min_properties,
        item_max_properties,
        min_chars,
        max_chars,
        item_min_chars,
        item_max_chars,
        format,
        item_format,
        pattern,
        item_pattern,
        example,
        choices,
        item_choices,
        secret,
        picker,
    });
}

pub(crate) fn set_field_arg_path_kind(
    config: &mut super::input_arg_support::FieldArgConfig,
    kind: PluginPathPermissionKind,
    span: impl quote::ToTokens,
) -> Result<()> {
    if config.path.replace(kind).is_some() {
        return Err(syn::Error::new_spanned(
            span,
            "#[arg] accepts only one path permission semantic",
        ));
    }
    Ok(())
}

pub(crate) fn set_field_arg_network(
    config: &mut super::input_arg_support::FieldArgConfig,
    semantic: PluginNetworkSemantic,
    span: impl quote::ToTokens,
) -> Result<()> {
    if config.network.replace(semantic).is_some() {
        return Err(syn::Error::new_spanned(
            span,
            "#[arg] accepts only one network semantic",
        ));
    }
    Ok(())
}

pub(crate) fn set_field_arg_picker(
    config: &mut super::input_arg_support::FieldArgConfig,
    picker: PluginPickerKind,
    span: impl quote::ToTokens,
) -> Result<()> {
    if config.picker.replace(picker).is_some() {
        return Err(syn::Error::new_spanned(
            span,
            "#[arg] accepts only one picker semantic",
        ));
    }
    Ok(())
}

pub(crate) fn input_jsonpath_for_field(field_name: &LitStr, ty: &Type) -> LitStr {
    let shape = super::input_type_semantic_shape(ty);
    let suffix = shape.array.then_some("[*]").unwrap_or("");
    LitStr::new(
        &format!("$.{}{}", field_name.value(), suffix),
        field_name.span(),
    )
}

pub(crate) fn input_jsonpath_for_arg(
    field_name: &LitStr,
    ty: &Type,
    override_path: Option<&LitStr>,
) -> LitStr {
    override_path
        .cloned()
        .unwrap_or_else(|| input_jsonpath_for_field(field_name, ty))
}

pub(crate) fn ensure_unique_field_arg_names(
    seen: &mut BTreeSet<String>,
    field_name: &LitStr,
    aliases: &[LitStr],
) -> Result<()> {
    for candidate in std::iter::once(field_name).chain(aliases.iter()) {
        if !seen.insert(candidate.value()) {
            return Err(syn::Error::new_spanned(
                candidate,
                format!(
                    "duplicate ToolInput field wire name or alias `{}`",
                    candidate.value()
                ),
            ));
        }
    }
    Ok(())
}
