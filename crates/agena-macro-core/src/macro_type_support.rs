//! Type-name and path helpers used by macro expansion.

use quote::quote;
use syn::{LitStr, PathArguments, Result, Type};

use super::{PluginNetworkSemantic, PluginPathPermissionKind, PluginPickerKind};

#[derive(Default)]
/// Semantic shape of a macro input type.
pub struct InputTypeSemanticShape {
    pub optional: bool,
    pub array: bool,
}

pub fn validate_pattern_lit(pattern: &LitStr) -> Result<()> {
    regex::Regex::new(pattern.value().as_str())
        .map(|_| ())
        .map_err(|err| syn::Error::new_spanned(pattern, format!("invalid pattern regex: {err}")))
}

pub fn validate_format_lit(format: &LitStr) -> Result<LitStr> {
    let value = format.value();
    match normalized_format_name(value.as_str()) {
        Some(normalized) => Ok(LitStr::new(normalized, format.span())),
        None => Err(syn::Error::new_spanned(
            format,
            format!(
                "unsupported format `{value}`; supported formats: {}",
                supported_format_names()
            ),
        )),
    }
}

pub fn validate_input_jsonpath(jsonpath: &str) -> std::result::Result<(), String> {
    if jsonpath == "$" {
        return Ok(());
    }
    let Some(mut rest) = jsonpath.strip_prefix("$.") else {
        return Err(format!("unsupported input jsonpath '{jsonpath}'"));
    };
    if rest.is_empty() {
        return Err(format!("unsupported input jsonpath '{jsonpath}'"));
    }
    while !rest.is_empty() {
        let key_end = rest.find(['.', '[']).unwrap_or(rest.len());
        let key = &rest[..key_end];
        if key.is_empty() {
            return Err(format!("unsupported input jsonpath '{jsonpath}'"));
        }
        rest = &rest[key_end..];
        while let Some(tail) = rest.strip_prefix("[*]") {
            rest = tail;
        }
        if rest.is_empty() {
            break;
        }
        let Some(tail) = rest.strip_prefix('.') else {
            return Err(format!("unsupported input jsonpath '{jsonpath}'"));
        };
        rest = tail;
        if rest.is_empty() {
            return Err(format!("unsupported input jsonpath '{jsonpath}'"));
        }
    }
    Ok(())
}

pub fn input_type_semantic_shape(ty: &Type) -> InputTypeSemanticShape {
    let ty = type_without_reference(ty);
    if type_last_segment_is(&ty, "Option")
        && let Some(inner) = type_first_generic_arg(&ty)
    {
        let mut shape = input_type_semantic_shape(inner);
        shape.optional = true;
        return shape;
    }
    InputTypeSemanticShape {
        optional: false,
        array: type_last_segment_is(&ty, "Vec"),
    }
}

pub fn type_first_generic_arg(ty: &Type) -> Option<&Type> {
    let Type::Path(path) = ty else {
        return None;
    };
    let segment = path.path.segments.last()?;
    let PathArguments::AngleBracketed(args) = &segment.arguments else {
        return None;
    };
    args.args.iter().find_map(|arg| match arg {
        syn::GenericArgument::Type(ty) => Some(ty),
        _ => None,
    })
}

pub fn path_permission_kind_label(kind: PluginPathPermissionKind) -> &'static str {
    match kind {
        PluginPathPermissionKind::Read => "read",
        PluginPathPermissionKind::Write => "write",
    }
}

pub fn network_semantic_label(semantic: PluginNetworkSemantic) -> &'static str {
    match semantic {
        PluginNetworkSemantic::Network => "network",
        PluginNetworkSemantic::Url => "url",
        PluginNetworkSemantic::Host => "host",
        PluginNetworkSemantic::Internet => "internet",
        PluginNetworkSemantic::Private => "private",
    }
}

pub fn picker_kind_label(picker: PluginPickerKind) -> &'static str {
    match picker {
        PluginPickerKind::File => "file",
        PluginPickerKind::Dir => "dir",
    }
}

pub fn type_last_segment_is(ty: &Type, expected: &str) -> bool {
    let ty = match ty {
        Type::Reference(reference) => reference.elem.as_ref(),
        other => other,
    };
    let Type::Path(path) = ty else {
        return false;
    };
    path.path
        .segments
        .last()
        .is_some_and(|segment| segment.ident == expected)
}

pub fn type_mentions_segment(ty: &Type, expected: &str) -> bool {
    type_key(ty).contains(expected)
}

pub fn type_is_tool_invoke_context(ty: &Type) -> bool {
    type_last_segment_is(ty, "ToolInvokeContext")
}

pub fn type_is_plugin_command_context(ty: &Type) -> bool {
    type_last_segment_is(ty, "PluginOperationContext")
}

pub fn type_is_reference(ty: &Type) -> bool {
    matches!(ty, Type::Reference(_))
}

pub fn type_without_reference(ty: &Type) -> Type {
    match ty {
        Type::Reference(reference) => (*reference.elem).clone(),
        other => other.clone(),
    }
}

pub fn type_is_unit(ty: &Type) -> bool {
    matches!(ty, Type::Tuple(tuple) if tuple.elems.is_empty())
}

pub fn types_equivalent(left: &Type, right: &Type) -> bool {
    type_key(left) == type_key(right)
}

pub fn type_display(ty: &Type) -> String {
    quote! { #ty }.to_string()
}

pub fn type_key(ty: &Type) -> String {
    type_display(ty)
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect()
}

fn normalized_format_name(value: &str) -> Option<&'static str> {
    match value {
        "uri" => Some("uri"),
        "uuid" => Some("uuid"),
        "email" => Some("email"),
        "hostname" => Some("hostname"),
        "ipv4" => Some("ipv4"),
        "ipv6" => Some("ipv6"),
        _ => None,
    }
}

fn supported_format_names() -> &'static str {
    "uri, uuid, email, hostname, ipv4, ipv6"
}
