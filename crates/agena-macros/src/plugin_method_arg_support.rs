use std::collections::{BTreeMap, BTreeSet};

use quote::format_ident;
use syn::ext::IdentExt;
use syn::parse::{Parse, ParseStream};
use syn::{
    Attribute, Expr, FnArg, Ident, ImplItemFn, LitStr, Meta, Pat, Result, Token, Type, parse_quote,
};

use crate::{
    PluginArgConfig, PluginCallInput, PluginCommandInputPlan, PluginCommandMethodShape,
    PluginContextArg, PluginGeneratedInputField, PluginGeneratedToolInput, PluginNetworkSemantic,
    PluginPathPermissionKind, PluginPickerKind, PluginToolAttrConfig, PluginToolMethodShape,
    apply_arg_config_to_spec, empty_tool_spec_config, expr_array_lit_strs, expr_array_values,
    expr_lit_str, expr_lit_usize, input_type_semantic_shape, normalize_array_value_constraints,
    type_is_plugin_command_context, type_is_reference, type_is_tool_invoke_context,
    type_last_segment_is, validate_format_lit, validate_input_jsonpath_lit, validate_pattern_lit,
};

pub(crate) fn build_plugin_tool_method_shape(
    method: &mut ImplItemFn,
    method_ident: &Ident,
    self_label: &str,
    config: &mut PluginToolAttrConfig,
    docs: Option<String>,
) -> Result<PluginToolMethodShape> {
    let value_args = plugin_method_value_args(method)?;
    let context = plugin_inline_context_arg(&value_args)?;
    let stream_arg_types = value_args
        .iter()
        .map(|arg| arg.ty.clone())
        .collect::<Vec<_>>();
    let input_args = value_args
        .into_iter()
        .filter(|arg| !arg.is_context)
        .collect::<Vec<_>>();
    let input_ident = format_ident!("__AgenaPluginToolInput_{}_{}", self_label, method_ident);

    let (input_ident, input_fields, input_ty, call_input) = match input_args.as_slice() {
        [] => (
            Some(input_ident.clone()),
            Vec::new(),
            parse_quote!(#input_ident),
            PluginCallInput::Fields(Vec::new()),
        ),
        [arg] if !arg.has_arg_config => {
            let input_ty = arg.inner_ty.clone();
            config.spec.input_shape = Some(input_ty.clone());
            (
                None,
                Vec::new(),
                input_ty,
                PluginCallInput::Wrapped { by_ref: arg.by_ref },
            )
        }
        args => {
            let mut fields = Vec::new();
            let mut call_fields = Vec::new();
            let prepared_args = prepare_inline_args(args)?;
            let field_path_lookup = inline_arg_constraint_path_lookup(&prepared_args)?;
            let array_field_paths = prepared_args
                .iter()
                .filter(|prepared| input_type_semantic_shape(&prepared.arg.ty).array)
                .map(|prepared| prepared.field_name.value())
                .collect::<BTreeSet<_>>();
            for prepared in prepared_args {
                let arg = prepared.arg;
                validate_inline_shape_wrapper_arg(arg)?;
                if arg.by_ref {
                    return Err(syn::Error::new_spanned(
                        &arg.ty,
                        "field-style #[tool] arguments must be owned values; use a single input struct argument if the handler wants a reference",
                    ));
                }
                apply_arg_config_to_spec(
                    &mut config.spec,
                    &prepared.field_name,
                    &prepared.aliases,
                    &arg.ty,
                    Some(&field_path_lookup),
                    &arg.config,
                );
                fields.push(PluginGeneratedInputField {
                    ident: arg.ident.clone(),
                    wire_name: prepared.field_name,
                    aliases: prepared.aliases,
                    ty: arg.ty.clone(),
                    default: arg.config.default,
                    default_expr: arg.config.default_expr.clone(),
                    flatten_shape: arg.config.flatten_shape,
                    nested_shape: arg.config.nested_shape,
                });
                call_fields.push(arg.ident.clone());
            }
            normalize_array_value_constraints(
                &mut config.spec.trim,
                &mut config.spec.trim_suffix,
                &mut config.spec.minimums,
                &mut config.spec.maximums,
                &mut config.spec.exclusive_minimums,
                &mut config.spec.exclusive_maximums,
                &mut config.spec.min_properties,
                &mut config.spec.max_properties,
                &mut config.spec.min_chars,
                &mut config.spec.max_chars,
                &mut config.spec.formats,
                &mut config.spec.patterns,
                &mut config.spec.choices,
                &mut config.spec.forbid_substrings,
                &mut config.spec.distinct_trimmed,
                &mut config.spec.input_field_metadata,
                &field_path_lookup,
                &array_field_paths,
            );
            (
                Some(input_ident.clone()),
                fields,
                parse_quote!(#input_ident),
                PluginCallInput::Fields(call_fields),
            )
        }
    };

    let input_model = PluginGeneratedToolInput {
        input_ident,
        input_fields,
        input_ty,
        spec: config.spec.clone(),
        docs,
    };

    Ok(PluginToolMethodShape {
        input_model,
        context,
        call_input,
        stream_arg_types,
        stream_method: config.stream_method.clone(),
    })
}

pub(crate) fn build_plugin_command_input_plan(
    method: &mut ImplItemFn,
    method_ident: &Ident,
    self_label: &str,
    docs: Option<String>,
) -> Result<PluginCommandMethodShape> {
    let input_ident = format_ident!("__AgenaPluginCommandInput_{}_{}", self_label, method_ident);
    let args = plugin_method_value_args(method)?;
    if let Some(context_arg) = args.iter().find(|arg| arg.is_context) {
        return Err(syn::Error::new_spanned(
            &context_arg.ty,
            "#[command] methods do not support ToolInvokeContext; use PluginCommandInvokeInput for raw command context",
        ));
    }
    let context = plugin_command_context_arg(&args)?;
    let input_args = args
        .into_iter()
        .filter(|arg| !type_is_plugin_command_context(&arg.ty))
        .collect::<Vec<_>>();
    let input = match input_args.as_slice() {
        [] => PluginCommandInputPlan::None,
        [arg] if !arg.has_arg_config => {
            let by_ref = arg.by_ref;
            let owned_ty = arg.inner_ty.clone();
            if type_last_segment_is(&owned_ty, "PluginCommandInvokeInput") {
                if context.is_some() {
                    return Err(syn::Error::new_spanned(
                        &arg.ty,
                        "PluginCommandInvokeInput already exposes raw command metadata; do not combine it with PluginCommandContext",
                    ));
                }
                PluginCommandInputPlan::Raw { by_ref }
            } else {
                PluginCommandInputPlan::Typed {
                    ty: owned_ty,
                    by_ref,
                }
            }
        }
        args => {
            let mut spec = empty_tool_spec_config();
            let mut fields = Vec::new();
            let mut call_fields = Vec::new();
            let prepared_args = prepare_inline_args(args)?;
            let field_path_lookup = inline_arg_constraint_path_lookup(&prepared_args)?;
            let array_field_paths = prepared_args
                .iter()
                .filter(|prepared| input_type_semantic_shape(&prepared.arg.ty).array)
                .map(|prepared| prepared.field_name.value())
                .collect::<BTreeSet<_>>();
            for prepared in prepared_args {
                let arg = prepared.arg;
                validate_inline_shape_wrapper_arg(arg)?;
                if type_last_segment_is(&arg.inner_ty, "PluginCommandInvokeInput") {
                    return Err(syn::Error::new_spanned(
                        &arg.ty,
                        "PluginCommandInvokeInput is only supported as the sole #[command] argument; use a typed input struct or inline #[arg(...)] fields for structured command inputs",
                    ));
                }
                if arg.by_ref {
                    return Err(syn::Error::new_spanned(
                        &arg.ty,
                        "field-style #[command] arguments must be owned values; use a single input struct argument if the handler wants a reference",
                    ));
                }
                let field_name = prepared.field_name;
                let aliases = prepared.aliases;
                apply_arg_config_to_spec(
                    &mut spec,
                    &field_name,
                    &aliases,
                    &arg.ty,
                    Some(&field_path_lookup),
                    &arg.config,
                );
                fields.push(PluginGeneratedInputField {
                    ident: arg.ident.clone(),
                    wire_name: field_name,
                    aliases,
                    ty: arg.ty.clone(),
                    default: arg.config.default,
                    default_expr: arg.config.default_expr.clone(),
                    flatten_shape: arg.config.flatten_shape,
                    nested_shape: arg.config.nested_shape,
                });
                call_fields.push(arg.ident.clone());
            }
            normalize_array_value_constraints(
                &mut spec.trim,
                &mut spec.trim_suffix,
                &mut spec.minimums,
                &mut spec.maximums,
                &mut spec.exclusive_minimums,
                &mut spec.exclusive_maximums,
                &mut spec.min_properties,
                &mut spec.max_properties,
                &mut spec.min_chars,
                &mut spec.max_chars,
                &mut spec.formats,
                &mut spec.patterns,
                &mut spec.choices,
                &mut spec.forbid_substrings,
                &mut spec.distinct_trimmed,
                &mut spec.input_field_metadata,
                &field_path_lookup,
                &array_field_paths,
            );
            PluginCommandInputPlan::Generated {
                input_model: PluginGeneratedToolInput {
                    input_ident: Some(input_ident.clone()),
                    input_fields: fields,
                    input_ty: parse_quote!(#input_ident),
                    spec,
                    docs,
                },
                input: PluginCallInput::Fields(call_fields),
            }
        }
    };
    Ok(PluginCommandMethodShape { input, context })
}

struct PluginMethodValueArg {
    ident: Ident,
    ty: Type,
    inner_ty: Type,
    by_ref: bool,
    is_context: bool,
    config: PluginArgConfig,
    has_arg_config: bool,
}

fn plugin_method_value_args(method: &mut ImplItemFn) -> Result<Vec<PluginMethodValueArg>> {
    let mut args = Vec::new();
    for arg in method.sig.inputs.iter_mut() {
        let FnArg::Typed(pat_type) = arg else {
            continue;
        };
        let (config, has_arg_config) = parse_plugin_arg_attrs(&mut pat_type.attrs)?;
        let ident = match pat_type.pat.as_ref() {
            Pat::Ident(pat) if pat.by_ref.is_none() && pat.subpat.is_none() => pat.ident.clone(),
            other => {
                return Err(syn::Error::new_spanned(
                    other,
                    "method-level #[tool] generation requires simple identifier arguments",
                ));
            }
        };
        let ty = (*pat_type.ty).clone();
        let by_ref = type_is_reference(&ty);
        let inner_ty = match &ty {
            Type::Reference(reference) => (*reference.elem).clone(),
            other => other.clone(),
        };
        args.push(PluginMethodValueArg {
            ident,
            is_context: type_is_tool_invoke_context(&ty),
            ty,
            inner_ty,
            by_ref,
            config,
            has_arg_config,
        });
    }
    Ok(args)
}

fn plugin_inline_context_arg(args: &[PluginMethodValueArg]) -> Result<Option<PluginContextArg>> {
    let mut context_positions = args
        .iter()
        .enumerate()
        .filter(|(_, arg)| arg.is_context)
        .collect::<Vec<_>>();
    if context_positions.len() > 1 {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            "method-level #[tool] generation supports at most one ToolInvokeContext argument",
        ));
    }
    let Some((index, context_arg)) = context_positions.pop() else {
        return Ok(None);
    };
    let first_input_index = args
        .iter()
        .enumerate()
        .find_map(|(idx, arg)| (!arg.is_context).then_some(idx));
    let first = first_input_index.is_none_or(|input_index| index < input_index);
    Ok(Some(PluginContextArg {
        first,
        by_ref: context_arg.by_ref,
    }))
}

fn plugin_command_context_arg(args: &[PluginMethodValueArg]) -> Result<Option<PluginContextArg>> {
    let mut context_positions = args
        .iter()
        .enumerate()
        .filter(|(_, arg)| type_is_plugin_command_context(&arg.ty))
        .collect::<Vec<_>>();
    if context_positions.len() > 1 {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            "method-level #[command] generation supports at most one PluginCommandContext argument",
        ));
    }
    let Some((index, context_arg)) = context_positions.pop() else {
        return Ok(None);
    };
    let first_input_index = args
        .iter()
        .enumerate()
        .find_map(|(idx, arg)| (!type_is_plugin_command_context(&arg.ty)).then_some(idx));
    let first = first_input_index.is_none_or(|input_index| index < input_index);
    Ok(Some(PluginContextArg {
        first,
        by_ref: context_arg.by_ref,
    }))
}

fn parse_plugin_arg_attrs(attrs: &mut Vec<Attribute>) -> Result<(PluginArgConfig, bool)> {
    let mut config = PluginArgConfig::default();
    let mut found = false;
    let mut kept = Vec::new();
    for attr in std::mem::take(attrs) {
        if !attr.path().is_ident("arg") {
            kept.push(attr);
            continue;
        }
        found = true;
        match &attr.meta {
            Meta::Path(_) => {}
            Meta::NameValue(_) => {
                return Err(syn::Error::new_spanned(
                    attr,
                    "#[arg] supports list syntax, for example #[arg(trim, non_empty)]",
                ));
            }
            Meta::List(_) => parse_plugin_arg_config_attr(&attr, &mut config)?,
        }
    }
    *attrs = kept;
    Ok((config, found))
}

fn parse_plugin_arg_config_attr(attr: &Attribute, config: &mut PluginArgConfig) -> Result<()> {
    let args = attr.parse_args::<ArgAttrArgs>()?;
    for item in args.items {
        match (item.key.as_str(), item.value) {
            ("default", None) => config.default = true,
            ("default", Some(value)) => {
                if config.default || config.default_expr.replace(value).is_some() {
                    return Err(syn::Error::new_spanned(
                        item.first_ident,
                        "duplicate #[arg(default)] or #[arg(default = ...)]",
                    ));
                }
            }
            ("description", Some(value)) => {
                if config
                    .description
                    .replace(expr_lit_str(&value, "description")?)
                    .is_some()
                {
                    return Err(syn::Error::new_spanned(
                        item.first_ident,
                        "duplicate #[arg(description = ...)]",
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
            ("path.read", None) => {
                set_plugin_arg_path_kind(config, PluginPathPermissionKind::Read, &item.first_ident)?
            }
            ("path.write", None) => set_plugin_arg_path_kind(
                config,
                PluginPathPermissionKind::Write,
                &item.first_ident,
            )?,
            ("network", None) => {
                set_plugin_arg_network(config, PluginNetworkSemantic::Network, &item.first_ident)?
            }
            ("network.url", None) => {
                set_plugin_arg_network(config, PluginNetworkSemantic::Url, &item.first_ident)?
            }
            ("network.host", None) => {
                set_plugin_arg_network(config, PluginNetworkSemantic::Host, &item.first_ident)?
            }
            ("network.internet", None) => {
                set_plugin_arg_network(config, PluginNetworkSemantic::Internet, &item.first_ident)?
            }
            ("network.private", None) => {
                set_plugin_arg_network(config, PluginNetworkSemantic::Private, &item.first_ident)?
            }
            ("optional", None) => config.optional = true,
            ("flatten_shape", None) => config.flatten_shape = true,
            ("nested_shape", None) => config.nested_shape = true,
            ("secret", None) => config.secret = true,
            ("file", None) => {
                set_plugin_arg_picker(config, PluginPickerKind::File, &item.first_ident)?
            }
            ("dir", None) => {
                set_plugin_arg_picker(config, PluginPickerKind::Dir, &item.first_ident)?
            }
            ("jsonpath", Some(value)) => {
                let jsonpath = expr_lit_str(&value, "jsonpath")?;
                validate_input_jsonpath_lit(&jsonpath)?;
                config.jsonpath = Some(jsonpath);
            }
            ("fallback", Some(value)) => config.fallback = Some(expr_lit_str(&value, "fallback")?),
            ("name", Some(value)) => {
                if config.name.replace(expr_lit_str(&value, "name")?).is_some() {
                    return Err(syn::Error::new_spanned(
                        item.first_ident,
                        "duplicate #[arg(name = ...)]",
                    ));
                }
            }
            ("alias", Some(value)) => config.aliases.push(expr_lit_str(&value, "alias")?),
            ("example", Some(value)) => config.example = Some(value),
            ("trim_suffix", Some(value)) => {
                config.trim_suffix = Some(expr_lit_str(&value, "trim_suffix")?)
            }
            ("item_trim_suffix", Some(value)) => {
                let suffix = expr_lit_str(&value, "item_trim_suffix")?;
                if config.item_trim_suffix.replace(suffix).is_some() {
                    return Err(syn::Error::new_spanned(
                        item.first_ident,
                        "duplicate #[arg(item_trim_suffix = ...)]",
                    ));
                }
            }
            ("minimum", Some(value)) => {
                if config.minimum.replace(value).is_some() {
                    return Err(syn::Error::new_spanned(
                        item.first_ident,
                        "duplicate #[arg(minimum = ...)]",
                    ));
                }
            }
            ("maximum", Some(value)) => {
                if config.maximum.replace(value).is_some() {
                    return Err(syn::Error::new_spanned(
                        item.first_ident,
                        "duplicate #[arg(maximum = ...)]",
                    ));
                }
            }
            ("exclusive_minimum", Some(value)) => {
                if config.exclusive_minimum.replace(value).is_some() {
                    return Err(syn::Error::new_spanned(
                        item.first_ident,
                        "duplicate #[arg(exclusive_minimum = ...)]",
                    ));
                }
            }
            ("exclusive_maximum", Some(value)) => {
                if config.exclusive_maximum.replace(value).is_some() {
                    return Err(syn::Error::new_spanned(
                        item.first_ident,
                        "duplicate #[arg(exclusive_maximum = ...)]",
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
                        "duplicate #[arg(item_minimum = ...)]",
                    ));
                }
            }
            ("item_maximum", Some(value)) => {
                if config.item_maximum.replace(value).is_some() {
                    return Err(syn::Error::new_spanned(
                        item.first_ident,
                        "duplicate #[arg(item_maximum = ...)]",
                    ));
                }
            }
            ("item_exclusive_minimum", Some(value)) => {
                if config.item_exclusive_minimum.replace(value).is_some() {
                    return Err(syn::Error::new_spanned(
                        item.first_ident,
                        "duplicate #[arg(item_exclusive_minimum = ...)]",
                    ));
                }
            }
            ("item_exclusive_maximum", Some(value)) => {
                if config.item_exclusive_maximum.replace(value).is_some() {
                    return Err(syn::Error::new_spanned(
                        item.first_ident,
                        "duplicate #[arg(item_exclusive_maximum = ...)]",
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
                        "duplicate #[arg(item_format = ...)]",
                    ));
                }
            }
            ("item_pattern", Some(value)) => {
                let pattern = expr_lit_str(&value, "item_pattern")?;
                validate_pattern_lit(&pattern)?;
                if config.item_pattern.replace(pattern).is_some() {
                    return Err(syn::Error::new_spanned(
                        item.first_ident,
                        "duplicate #[arg(item_pattern = ...)]",
                    ));
                }
            }
            ("format", Some(value)) => {
                let format = validate_format_lit(&expr_lit_str(&value, "format")?)?;
                if config.format.replace(format).is_some() {
                    return Err(syn::Error::new_spanned(
                        item.first_ident,
                        "duplicate #[arg(format = ...)]",
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
                        "duplicate #[arg(item_choices = [...])]",
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
                        "duplicate #[arg(pattern = ...)]",
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
                        "duplicate #[arg(choices = [...])]",
                    ));
                }
            }
            (key, Some(_)) => {
                return Err(syn::Error::new_spanned(
                    item.first_ident,
                    format!("unsupported #[arg] option '{key}'"),
                ));
            }
            (key, None) => {
                return Err(syn::Error::new_spanned(
                    item.first_ident,
                    format!("unsupported #[arg] flag '{key}'"),
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

pub(crate) fn ensure_arg_permission_locator_has_semantic(
    jsonpath: Option<&LitStr>,
    fallback: Option<&LitStr>,
    has_permission_semantic: bool,
) -> Result<()> {
    if has_permission_semantic {
        return Ok(());
    }
    if let Some(value) = jsonpath.or(fallback) {
        return Err(syn::Error::new_spanned(
            value,
            "`jsonpath` and `fallback` require a path.* or network.* semantic",
        ));
    }
    Ok(())
}

pub(crate) struct ArgAttrArgs {
    pub(crate) items: Vec<ArgAttrItem>,
}

pub(crate) struct ArgAttrItem {
    pub(crate) key: String,
    pub(crate) first_ident: Ident,
    pub(crate) value: Option<Expr>,
}

impl Parse for ArgAttrArgs {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let mut items = Vec::new();
        while !input.is_empty() {
            let first_ident = input.call(Ident::parse_any)?;
            let mut key = first_ident.to_string();
            while input.peek(Token![.]) {
                input.parse::<Token![.]>()?;
                key.push('.');
                key.push_str(&input.call(Ident::parse_any)?.to_string());
            }
            let value = if input.peek(Token![=]) {
                input.parse::<Token![=]>()?;
                Some(input.parse()?)
            } else {
                None
            };
            items.push(ArgAttrItem {
                key,
                first_ident,
                value,
            });
            if input.peek(Token![,]) {
                input.parse::<Token![,]>()?;
            } else if !input.is_empty() {
                return Err(input.error("expected `,` between #[arg] entries"));
            }
        }
        Ok(Self { items })
    }
}

fn set_plugin_arg_path_kind(
    config: &mut PluginArgConfig,
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

fn set_plugin_arg_network(
    config: &mut PluginArgConfig,
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

fn set_plugin_arg_picker(
    config: &mut PluginArgConfig,
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

fn inline_arg_field_names(
    ident: &Ident,
    config: &PluginArgConfig,
) -> Result<(LitStr, Vec<LitStr>)> {
    let field_name = config
        .name
        .clone()
        .unwrap_or_else(|| LitStr::new(&ident.to_string(), ident.span()));
    let mut seen = BTreeSet::from([field_name.value()]);
    let mut aliases = Vec::new();
    for alias in &config.aliases {
        if !seen.insert(alias.value()) {
            return Err(syn::Error::new_spanned(
                alias,
                format!(
                    "duplicate inline #[arg] wire name or alias `{}`",
                    alias.value()
                ),
            ));
        }
        aliases.push(alias.clone());
    }
    Ok((field_name, aliases))
}

struct PreparedInlineArg<'a> {
    arg: &'a PluginMethodValueArg,
    field_name: LitStr,
    aliases: Vec<LitStr>,
}

fn prepare_inline_args<'a>(args: &'a [PluginMethodValueArg]) -> Result<Vec<PreparedInlineArg<'a>>> {
    let mut seen_field_names = BTreeSet::new();
    let mut prepared = Vec::with_capacity(args.len());
    for arg in args {
        let (field_name, aliases) = inline_arg_field_names(&arg.ident, &arg.config)?;
        ensure_unique_inline_arg_field_names(&mut seen_field_names, &field_name, &aliases)?;
        prepared.push(PreparedInlineArg {
            arg,
            field_name,
            aliases,
        });
    }
    Ok(prepared)
}

fn inline_arg_constraint_path_lookup(
    prepared: &[PreparedInlineArg<'_>],
) -> Result<BTreeMap<String, LitStr>> {
    let mut lookup = BTreeMap::new();
    for prepared_arg in prepared {
        let target = prepared_arg.field_name.clone();
        let candidates = std::iter::once((&prepared_arg.arg.ident, None))
            .chain(std::iter::once((&prepared_arg.arg.ident, Some(&target))))
            .chain(
                prepared_arg
                    .aliases
                    .iter()
                    .map(|alias| (&prepared_arg.arg.ident, Some(alias))),
            );
        for (ident, candidate) in candidates {
            let candidate = candidate
                .cloned()
                .unwrap_or_else(|| LitStr::new(&ident.to_string(), ident.span()));
            let candidate_value = candidate.value();
            if let Some(existing) = lookup.insert(candidate.value(), target.clone())
                && existing.value() != target.value()
            {
                return Err(syn::Error::new_spanned(
                    &candidate,
                    format!(
                        "duplicate inline #[arg] wire name or alias `{}`",
                        candidate_value
                    ),
                ));
            }
        }
        if let Some(existing) = lookup.insert(target.value(), target.clone())
            && existing.value() != target.value()
        {
            return Err(syn::Error::new_spanned(
                &prepared_arg.field_name,
                format!(
                    "duplicate inline #[arg] wire name or alias `{}`",
                    prepared_arg.field_name.value()
                ),
            ));
        }
    }
    Ok(lookup)
}

fn inline_flatten_shape_has_extra_config(config: &PluginArgConfig) -> bool {
    config.default
        || config.default_expr.is_some()
        || config.description.is_some()
        || config.trim
        || config.item_trim
        || config.non_empty
        || config.item_non_empty
        || config.non_empty_if_present
        || config.item_non_empty_if_present
        || config.distinct_trimmed
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
        || config.pattern.is_some()
        || config.item_pattern.is_some()
        || config.choices.is_some()
        || config.item_choices.is_some()
        || !config.exactly_one_of.is_empty()
        || !config.at_least_one_of.is_empty()
        || !config.requires.is_empty()
        || !config.conflicts_with.is_empty()
        || !config.required_unless_present.is_empty()
        || !config.forbid_substrings.is_empty()
        || !config.distinct_trimmed_within.is_empty()
        || config.path.is_some()
        || config.network.is_some()
        || config.optional
        || config.nested_shape
        || config.jsonpath.is_some()
        || config.fallback.is_some()
        || config.name.is_some()
        || !config.aliases.is_empty()
        || config.example.is_some()
        || config.secret
        || config.picker.is_some()
}

fn validate_inline_shape_wrapper_arg(arg: &PluginMethodValueArg) -> Result<()> {
    if !arg.config.flatten_shape {
        return Ok(());
    }
    if inline_flatten_shape_has_extra_config(&arg.config) {
        return Err(syn::Error::new_spanned(
            &arg.ty,
            "inline #[arg(flatten_shape)] cannot be combined with name/alias/default/validation/permission metadata; put those rules on the flattened ToolInput type itself",
        ));
    }
    let shape = input_type_semantic_shape(&arg.ty);
    if shape.optional || shape.array {
        return Err(syn::Error::new_spanned(
            &arg.ty,
            "inline #[arg(flatten_shape)] only supports plain ToolInput object types; use a non-Option, non-Vec ToolInput",
        ));
    }
    Ok(())
}

fn ensure_unique_inline_arg_field_names(
    seen: &mut BTreeSet<String>,
    field_name: &LitStr,
    aliases: &[LitStr],
) -> Result<()> {
    for candidate in std::iter::once(field_name).chain(aliases.iter()) {
        if !seen.insert(candidate.value()) {
            return Err(syn::Error::new_spanned(
                candidate,
                format!(
                    "duplicate inline #[arg] wire name or alias `{}`",
                    candidate.value()
                ),
            ));
        }
    }
    Ok(())
}
