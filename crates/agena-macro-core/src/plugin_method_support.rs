//! Method dispatch glue for plugin tool/hook methods.

use std::collections::BTreeSet;

use syn::punctuated::Punctuated;
use syn::{Field, FnArg, ImplItemFn, LitStr, Meta, PathArguments, Result, Token, Type};

use crate::{
    PluginCommandPlan, PluginGeneratedInputField, PluginInputFieldAliasSpec, PluginMethodInfo,
    PluginToolOutputPlan, PluginToolPlan, SerdeRenameRule, append_constraint_path_suffix,
    input_type_semantic_shape, parse_input_field_arg_attrs, prepare_input_field_names,
    type_first_generic_arg, type_is_unit, type_last_segment_is, type_without_reference,
};

#[derive(Clone)]
pub struct NestedInputShapeSpec {
    pub inner_ty: Type,
    pub optional: bool,
    pub array: bool,
}

#[derive(Clone)]
pub struct NestedInputShapeField {
    pub spec: NestedInputShapeSpec,
    pub normalize_path: LitStr,
    pub schema_path: LitStr,
    pub schema_aliases: Vec<LitStr>,
}

pub fn plugin_method_tool_output(
    method: &ImplItemFn,
    explicit: Option<Type>,
) -> PluginToolOutputPlan {
    let returns_result = plugin_method_result_ok_type(method).is_some();
    if let Some(explicit) = explicit {
        return PluginToolOutputPlan {
            ty: Some(explicit),
            returns_result,
        };
    }
    let Some((candidate, is_result)) = plugin_method_return_value_type(method) else {
        return PluginToolOutputPlan {
            ty: None,
            returns_result: false,
        };
    };
    if type_is_unit(&candidate)
        || type_last_segment_is(&candidate, "ToolInvokeOutput")
        || type_last_segment_is(&candidate, "ToolStreamEnd")
    {
        return PluginToolOutputPlan {
            ty: None,
            returns_result: false,
        };
    }
    PluginToolOutputPlan {
        ty: Some(candidate),
        returns_result: is_result,
    }
}

pub fn plugin_method_return_value_type(method: &ImplItemFn) -> Option<(Type, bool)> {
    let ty = plugin_method_return_type(method)?;
    if let Some(ok_ty) = result_ok_type(ty) {
        return Some((ok_ty.clone(), true));
    }
    Some((ty.clone(), false))
}

fn plugin_method_result_ok_type(method: &ImplItemFn) -> Option<&Type> {
    result_ok_type(plugin_method_return_type(method)?)
}

pub fn plugin_method_return_type(method: &ImplItemFn) -> Option<&Type> {
    let syn::ReturnType::Type(_, ty) = &method.sig.output else {
        return None;
    };
    Some(ty.as_ref())
}

fn result_ok_type(ty: &Type) -> Option<&Type> {
    let Type::Path(path) = ty else {
        return None;
    };
    let segment = path.path.segments.last()?;
    if !matches!(segment.ident.to_string().as_str(), "Result" | "SdkResult") {
        return None;
    }
    let PathArguments::AngleBracketed(args) = &segment.arguments else {
        return None;
    };
    args.args.iter().find_map(|arg| match arg {
        syn::GenericArgument::Type(ty) => Some(ty),
        _ => None,
    })
}

pub fn nested_input_shape_spec(field: &Field) -> Result<Option<NestedInputShapeSpec>> {
    if crate::field_is_flatten(field)? {
        return Ok(None);
    }
    let mut enabled = false;
    for attr in &field.attrs {
        if !attr.path().is_ident("input") {
            continue;
        }
        let metas = attr.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)?;
        for meta in metas {
            if let Meta::Path(path) = meta
                && path.is_ident("nested_shape")
            {
                enabled = true;
            }
        }
    }
    if !enabled {
        return Ok(None);
    }

    Ok(nested_input_shape_spec_from_type(&field.ty))
}

pub fn nested_input_shape_spec_from_type(ty: &Type) -> Option<NestedInputShapeSpec> {
    let ty = type_without_reference(ty).clone();
    let (optional, inner_ty) = if type_last_segment_is(&ty, "Option") {
        let inner = type_first_generic_arg(&ty)?;
        (true, inner.clone())
    } else {
        (false, ty)
    };
    let (array, inner_ty) = if type_last_segment_is(&inner_ty, "Vec") {
        let inner = type_first_generic_arg(&inner_ty)?;
        (true, inner.clone())
    } else {
        (false, inner_ty)
    };

    Some(NestedInputShapeSpec {
        inner_ty,
        optional,
        array,
    })
}

pub fn nested_input_shape_field(
    field: &Field,
    rename_rule: Option<SerdeRenameRule>,
) -> Result<Option<NestedInputShapeField>> {
    let Some(spec) = nested_input_shape_spec(field)? else {
        return Ok(None);
    };
    let arg_config = parse_input_field_arg_attrs(field)?;
    let Some(names) = prepare_input_field_names(field, rename_rule, &arg_config)? else {
        return Ok(None);
    };
    let normalize_path = if spec.array {
        append_constraint_path_suffix(&names.parse_path, "[]")
    } else {
        names.parse_path.clone()
    };
    Ok(Some(NestedInputShapeField {
        spec,
        normalize_path,
        schema_path: names.schema_path,
        schema_aliases: names.schema_aliases,
    }))
}

pub fn generated_input_nested_shape_fields(
    fields: &[PluginGeneratedInputField],
) -> Vec<NestedInputShapeField> {
    fields
        .iter()
        .filter_map(|field| {
            let spec = field
                .nested_shape
                .then(|| nested_input_shape_spec_from_type(&field.ty))
                .flatten()?;
            let normalize_path = if spec.array {
                append_constraint_path_suffix(&field.wire_name, "[]")
            } else {
                field.wire_name.clone()
            };
            Some(NestedInputShapeField {
                spec,
                normalize_path,
                schema_path: field.wire_name.clone(),
                schema_aliases: field.aliases.clone(),
            })
        })
        .collect()
}

pub fn generated_input_alias_specs(
    fields: &[PluginGeneratedInputField],
) -> Vec<PluginInputFieldAliasSpec> {
    fields
        .iter()
        .filter(|field| !field.aliases.is_empty())
        .map(|field| PluginInputFieldAliasSpec {
            path: field.wire_name.clone(),
            aliases: field.aliases.clone(),
        })
        .collect()
}

pub fn generated_input_flatten_shape_types(
    fields: &[PluginGeneratedInputField],
) -> Result<Vec<Type>> {
    fields
        .iter()
        .filter(|field| field.flatten_shape)
        .map(|field| {
            let shape = input_type_semantic_shape(&field.ty);
            if shape.optional || shape.array {
                return Err(syn::Error::new_spanned(
                    &field.ty,
                    "inline #[arg(flatten_shape)] only supports plain ToolInput object types; use a non-Option, non-Vec ToolInput",
                ));
            }
            Ok(field.ty.clone())
        })
        .collect()
}

pub fn input_keys_for_parse_path(
    path: &LitStr,
    aliases: &[PluginInputFieldAliasSpec],
) -> Vec<LitStr> {
    let mut seen = BTreeSet::new();
    let mut keys = Vec::new();
    if seen.insert(path.value()) {
        keys.push(path.clone());
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
    for alias_spec in aliases {
        if alias_spec.path.value() != base {
            continue;
        }
        for alias in &alias_spec.aliases {
            let candidate = if tail.is_empty() && suffix.is_empty() {
                alias.clone()
            } else {
                LitStr::new(
                    format!("{}{}{}", alias.value(), suffix, tail).as_str(),
                    alias.span(),
                )
            };
            if seen.insert(candidate.value()) {
                keys.push(candidate);
            }
        }
    }
    keys
}

pub fn ensure_plugin_method_shared_receiver(method: &ImplItemFn, label: &str) -> Result<()> {
    if plugin_method_has_shared_receiver(method) {
        return Ok(());
    }
    Err(syn::Error::new_spanned(
        &method.sig,
        format!("{label} must be inherent methods with `&self` receiver"),
    ))
}

pub fn plugin_method_has_shared_receiver(method: &ImplItemFn) -> bool {
    matches!(
        method.sig.inputs.first(),
        Some(FnArg::Receiver(receiver))
            if receiver.reference.is_some() && receiver.mutability.is_none()
    )
}

pub fn stream_sink_is_edge_info(info: &PluginMethodInfo, label: &str) -> Result<bool> {
    let sink_positions = info
        .typed_args
        .iter()
        .enumerate()
        .filter_map(|(index, ty)| type_last_segment_is(ty, "ToolStreamSink").then_some(index))
        .collect::<Vec<_>>();
    let [sink_index] = sink_positions.as_slice() else {
        return Err(syn::Error::new_spanned(
            &info.ident,
            format!("{label} must include exactly one ToolStreamSink argument"),
        ));
    };
    if *sink_index == 0 {
        return Ok(true);
    }
    if *sink_index + 1 == info.typed_args.len() {
        return Ok(false);
    }
    Err(syn::Error::new_spanned(
        &info.ident,
        format!("{label} must put ToolStreamSink either first or last"),
    ))
}

pub fn typed_arg_types(method: &ImplItemFn) -> Vec<Type> {
    typed_arg_types_from_inputs(&method.sig.inputs)
}

pub fn typed_arg_types_from_inputs(inputs: &Punctuated<FnArg, Token![,]>) -> Vec<Type> {
    inputs
        .iter()
        .filter_map(|arg| match arg {
            FnArg::Receiver(_) => None,
            FnArg::Typed(pat_type) => Some((*pat_type.ty).clone()),
        })
        .collect()
}

pub fn reject_duplicate_tool_plans(tools: &[PluginToolPlan]) -> Result<()> {
    for (index, tool) in tools.iter().enumerate() {
        let name = &tool.tool;
        if tools
            .iter()
            .skip(index + 1)
            .any(|other| other.tool.value() == name.value())
        {
            return Err(syn::Error::new_spanned(
                name,
                format!("duplicate inline tool name '{}'", name.value()),
            ));
        }
    }
    Ok(())
}

pub fn reject_duplicate_command_plans(commands: &[PluginCommandPlan]) -> Result<()> {
    for (index, command) in commands.iter().enumerate() {
        let id = &command.id;
        if commands
            .iter()
            .skip(index + 1)
            .any(|other| other.id.value() == id.value())
        {
            return Err(syn::Error::new_spanned(
                &command.id,
                format!("duplicate #[command] id '{}'", id.value()),
            ));
        }
    }
    Ok(())
}
