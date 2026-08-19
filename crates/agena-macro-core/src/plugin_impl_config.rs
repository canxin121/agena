//! Plugin `impl`-level configuration parsing.

use syn::parse::Parser;
use syn::punctuated::Punctuated;
use syn::{Expr, ExprLit, Ident, Lit, Meta, Result, Token, Type, parse_quote};

use super::parse_expr_list;

/// Configuration of a plugin impl.
pub struct PluginImplConfig {
    pub namespace: Option<Expr>,
    pub name: Option<Expr>,
    pub version: Option<Expr>,
    pub summary: Option<Expr>,
    pub help: Option<Expr>,
    pub skills: Option<Expr>,
    pub activity_kinds: Option<Expr>,
    /// Typed settings are compiled internally to the constrained contract.
    pub settings: Option<Type>,
    pub settings_default: Option<Expr>,
    /// Presentation-only decoration applied after compiling `settings = Type`.
    pub settings_metadata: Option<Expr>,
    pub settings_field: Option<Ident>,
    pub settings_store: bool,
    pub plugin_tags: Vec<Expr>,
    /// Declarative consumer dependencies. Provider exports are generated only
    /// from method-level `#[service]` handlers so manifest and dispatch cannot drift.
    pub service_imports: Vec<Expr>,
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
    let mut activity_kinds = None;
    let mut settings = None;
    let mut settings_default = None;
    let mut settings_metadata = None;
    let mut settings_field = None;
    let mut settings_store = false;
    let mut plugin_tags = Vec::new();
    let mut service_imports = Vec::new();
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
                    "activity_kinds" => activity_kinds = Some(value.value),
                    "settings" => {
                        if settings.is_some() {
                            return Err(syn::Error::new_spanned(ident, "duplicate settings type"));
                        }
                        if settings_metadata.is_some() && settings.is_none() {
                            return Err(syn::Error::new(
                                proc_macro2::Span::call_site(),
                                "`settings_metadata = ...` requires `settings = Type`",
                            ));
                        }
                        settings = Some(expr_as_type(value.value)?);
                    }
                    "settings_default" => settings_default = Some(value.value),
                    "settings_metadata" => settings_metadata = Some(value.value),
                    "settings_builder" => {
                        return Err(syn::Error::new_spanned(
                            ident,
                            "plugin-level `settings_builder = ...` was removed; use `settings = Type`, optional `settings_default`, and presentation-only `settings_metadata`",
                        ));
                    }
                    "settings_field" => {
                        settings_field = Some(expr_path_ident(value.value, "settings_field")?)
                    }
                    "settings_store" => settings_store = expr_bool(value.value, "settings_store")?,
                    "commands" => {
                        return Err(syn::Error::new_spanned(
                            ident,
                            "plugin-level `commands = ...` was removed; define commands with method-level #[command(...)]",
                        ));
                    }
                    "tags" => {
                        return Err(syn::Error::new_spanned(
                            ident,
                            "plugin-level `tags(...)` is not supported; declare plugin metadata tags with `tags(ToolTag::...)` on the plugin attribute list",
                        ));
                    }
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
                    "tags" => {
                        plugin_tags.extend(parse_expr_list(list.tokens)?);
                    }
                    "exports" => {
                        return Err(syn::Error::new_spanned(
                            ident,
                            "plugin-level `exports(...)` was removed; define provider methods with method-level `#[service(...)]` or `#[service(Endpoint)]`",
                        ));
                    }
                    "imports" => {
                        service_imports.extend(parse_expr_list(list.tokens)?);
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
                if path.is_ident("settings") {
                    settings_store = true;
                    continue;
                }
                if path.is_ident("settings_store") {
                    settings_store = true;
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
    if settings_field.is_some() && settings.is_none() {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            "#[agena_plugin(..., settings_field = field)] requires `settings = Type`",
        ));
    }
    if settings_default.is_some() && settings.is_none() {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            "`settings_default = ...` requires `settings = Type`",
        ));
    }
    Ok(PluginImplConfig {
        namespace,
        name,
        version,
        summary,
        help,
        skills,
        activity_kinds,
        settings,
        settings_default,
        settings_metadata,
        settings_field,
        settings_store,
        plugin_tags,
        service_imports,
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
