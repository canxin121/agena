use syn::parse::Parser;
use syn::punctuated::Punctuated;
use syn::{Expr, ExprLit, Ident, Lit, Meta, Result, Token, Type, parse_quote};

use super::parse_expr_list;

pub struct PluginImplConfig {
    pub namespace: Option<Expr>,
    pub name: Option<Expr>,
    pub version: Option<Expr>,
    pub summary: Option<Expr>,
    pub help: Option<Expr>,
    pub skills: Option<Expr>,
    pub config_schema: Option<Expr>,
    pub config_schema_type: Option<Type>,
    pub config_schema_default: Option<Expr>,
    pub config_schema_store: bool,
    pub config_field: Option<Ident>,
    pub config_store: bool,
    pub display: Option<Ident>,
    pub ui_display: Option<Ident>,
    pub tool_description_mode: Option<Expr>,
    pub ui_display_mode: Option<Expr>,
    pub plugin_capabilities_expr: Option<Expr>,
    pub plugin_capabilities: Vec<Expr>,
    pub export: Option<Ident>,
    pub export_bind: Option<Expr>,
}

pub fn parse_plugin_impl_config(attr: proc_macro2::TokenStream) -> Result<PluginImplConfig> {
    let metas = Punctuated::<Meta, Token![,]>::parse_terminated.parse2(attr)?;
    let mut namespace = None;
    let mut name = None;
    let mut version = None;
    let mut summary = None;
    let mut help = None;
    let mut skills = None;
    let mut config_schema = None;
    let mut config_schema_type = None;
    let mut config_schema_default = None;
    let mut config_schema_store = false;
    let mut config_field = None;
    let mut config_store = false;
    let mut display = None;
    let mut ui_display = None;
    let mut tool_description_mode = None;
    let mut ui_display_mode = None;
    let mut plugin_capabilities_expr = None;
    let mut plugin_capabilities = Vec::new();
    let mut export = None;
    let mut export_bind = None;
    for meta in metas {
        match meta {
            Meta::NameValue(value) => {
                let Some(ident) = value.path.get_ident() else {
                    return Err(syn::Error::new_spanned(value.path, "expected identifier"));
                };
                match ident.to_string().as_str() {
                    "namespace" => namespace = Some(value.value),
                    "name" => name = Some(value.value),
                    "version" => version = Some(value.value),
                    "summary" => summary = Some(value.value),
                    "help" => help = Some(value.value),
                    "skills" => skills = Some(value.value),
                    "config" => {
                        config_schema_type = Some(expr_as_type(value.value)?);
                        config_store = true;
                    }
                    "config_schema" => config_schema = Some(value.value),
                    "config_schema_type" => config_schema_type = Some(expr_as_type(value.value)?),
                    "config_default" => config_schema_default = Some(value.value),
                    "config_schema_default" => config_schema_default = Some(value.value),
                    "config_field" => {
                        config_field = Some(expr_path_ident(value.value, "config_field")?)
                    }
                    "config_store" => config_store = expr_bool(value.value, "config_store")?,
                    "display" => display = Some(expr_path_ident(value.value, "display")?),
                    "ui_display" => ui_display = Some(expr_path_ident(value.value, "ui_display")?),
                    "tool_description_mode" => tool_description_mode = Some(value.value),
                    "ui_display_mode" => ui_display_mode = Some(value.value),
                    "commands" => {
                        return Err(syn::Error::new_spanned(
                            ident,
                            "plugin-level `commands = ...` was removed; define commands with method-level #[command(...)]",
                        ));
                    }
                    "plugin_capabilities" => plugin_capabilities_expr = Some(value.value),
                    "hooks" => {
                        return Err(syn::Error::new_spanned(
                            ident,
                            "plugin-level `hooks = ...` was removed; define hooks with method-level #[hook(...)]",
                        ));
                    }
                    "export" => export = Some(expr_path_ident(value.value, "export")?),
                    "bind" => export_bind = Some(value.value),
                    other => {
                        return Err(syn::Error::new_spanned(
                            ident,
                            format!("unsupported plugin argument '{other}'"),
                        ));
                    }
                }
            }
            Meta::List(list) => {
                let Some(ident) = list.path.get_ident() else {
                    return Err(syn::Error::new_spanned(list.path, "expected identifier"));
                };
                match ident.to_string().as_str() {
                    "plugin_capabilities" => {
                        plugin_capabilities.extend(parse_expr_list(list.tokens)?)
                    }
                    other => {
                        return Err(syn::Error::new_spanned(
                            ident,
                            format!("unsupported plugin list '{other}'"),
                        ));
                    }
                }
            }
            Meta::Path(path) => {
                if path.is_ident("config") {
                    config_store = true;
                    config_schema_store = true;
                    continue;
                }
                if path.is_ident("config_store") {
                    config_store = true;
                    config_schema_store = true;
                    continue;
                }
                return Err(syn::Error::new_spanned(
                    path,
                    "unsupported bare plugin argument",
                ));
            }
        }
    }
    for (label, present) in [
        ("namespace", namespace.is_some()),
        ("name", name.is_some()),
        ("version", version.is_some()),
    ] {
        if !present {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                format!("#[agena_plugin(...)] requires `{label} = ...`"),
            ));
        }
    }
    if config_field.is_some() && config_schema_type.is_none() {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            "#[agena_plugin(..., config_field = field)] requires `config = Type` or `config_schema_type = Type`",
        ));
    }
    if config_store && config_schema_type.is_none() {
        config_schema_store = true;
    }
    if config_schema_default.is_some() && config_schema_type.is_none() && config_schema_store {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            "put derived config defaults on the field, e.g. `#[config(default)]`; `config_default = ...` requires `config = Type` or `config_schema_type = Type`",
        ));
    }
    Ok(PluginImplConfig {
        namespace,
        name,
        version,
        summary,
        help,
        skills,
        config_schema,
        config_schema_type,
        config_schema_default,
        config_schema_store,
        config_field,
        config_store,
        display,
        ui_display,
        tool_description_mode,
        ui_display_mode,
        plugin_capabilities_expr,
        plugin_capabilities,
        export,
        export_bind,
    })
}

pub fn parse_type_list(tokens: proc_macro2::TokenStream, label: &str) -> Result<Type> {
    syn::parse2::<Type>(tokens).map_err(|err| {
        syn::Error::new(
            err.span(),
            format!("{label} expects a single type, such as `{label}(Vec<Item>)`"),
        )
    })
}

pub fn plugin_self_type_label(ty: &Type) -> String {
    let raw = match ty {
        Type::Path(path) => path
            .path
            .segments
            .last()
            .map(|segment| segment.ident.to_string())
            .unwrap_or_else(|| "Plugin".to_string()),
        _ => "Plugin".to_string(),
    };
    sanitize_generated_ident_label(&raw)
}

pub fn sanitize_generated_ident_label(value: &str) -> String {
    let mut out = String::new();
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        "Plugin".to_string()
    } else if out.chars().next().is_some_and(|ch| ch.is_ascii_digit()) {
        format!("_{out}")
    } else {
        out
    }
}

pub fn plugin_id_label(config: &PluginImplConfig) -> String {
    let literal = |expr: &Expr| match expr {
        Expr::Lit(ExprLit {
            lit: Lit::Str(value),
            ..
        }) => Some(value.value()),
        _ => None,
    };
    match (
        config.namespace.as_ref().and_then(literal),
        config.name.as_ref().and_then(literal),
    ) {
        (Some(namespace), Some(name)) => format!("{namespace}.{name}"),
        _ => "plugin".to_string(),
    }
}

pub fn expr_is_ident(expr: &Expr, expected: &str) -> bool {
    let Expr::Path(path) = expr else {
        return false;
    };
    path.path.get_ident().is_some_and(|ident| ident == expected)
}

fn expr_as_type(expr: Expr) -> Result<Type> {
    match expr {
        Expr::Path(path) => {
            let path = path.path;
            Ok(parse_quote!(#path))
        }
        other => Err(syn::Error::new_spanned(
            other,
            "expected a type path, such as `MyType`",
        )),
    }
}

pub fn expr_path_ident(expr: Expr, label: &str) -> Result<Ident> {
    match expr {
        Expr::Path(path) => path.path.get_ident().cloned().ok_or_else(|| {
            syn::Error::new_spanned(path, format!("{label} must be a single identifier"))
        }),
        other => Err(syn::Error::new_spanned(
            other,
            format!("{label} must be a single identifier"),
        )),
    }
}

fn expr_bool(expr: Expr, label: &str) -> Result<bool> {
    match expr {
        Expr::Lit(ExprLit {
            lit: Lit::Bool(value),
            ..
        }) => Ok(value.value),
        other => Err(syn::Error::new_spanned(
            other,
            format!("{label} must be a boolean literal"),
        )),
    }
}
