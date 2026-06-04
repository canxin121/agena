use proc_macro::TokenStream;
use quote::quote;
use syn::parse::Parser;
use syn::punctuated::Punctuated;
use syn::{
    Attribute, Data, DeriveInput, Expr, ExprLit, ExprPath, Fields, Lit, LitBool, LitStr, Meta,
    MetaList, MetaNameValue, Path, Result, Token, Variant, parse_macro_input,
};

#[proc_macro_derive(StaticToolSurface, attributes(tool_surface, tool))]
pub fn derive_static_tool_surface(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match expand_static_tool_surface(input) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

#[derive(Default)]
struct SurfaceConfig {
    tool: Option<LitStr>,
    description: Option<LitStr>,
    summary: Option<LitStr>,
    help: Option<LitStr>,
    description_mode: Option<LitStr>,
    ui_display_mode: Option<LitStr>,
    tags: Vec<Expr>,
    host_capabilities: Vec<Expr>,
    concurrency_safe: Option<bool>,
    strict: Option<bool>,
    streaming: Option<LitStr>,
}

enum VariantMapping {
    Exec(LitStr),
    Map(Path),
}

fn expand_static_tool_surface(input: DeriveInput) -> Result<proc_macro2::TokenStream> {
    let name = input.ident;
    let surface = parse_surface_config(&input.attrs)?;
    let tool = surface
        .tool
        .ok_or_else(|| syn::Error::new_spanned(&name, "missing #[tool_surface(tool = \"...\")]"))?;
    let description = surface.description.ok_or_else(|| {
        syn::Error::new_spanned(&name, "missing #[tool_surface(description = \"...\")]")
    })?;
    let concurrency_safe = surface.concurrency_safe.unwrap_or(false);
    let strict = surface.strict.unwrap_or(false);

    let Data::Enum(data_enum) = input.data else {
        return Err(syn::Error::new_spanned(
            name,
            "StaticToolSurface can only be derived for enums",
        ));
    };

    let match_arms = data_enum
        .variants
        .iter()
        .map(expand_variant_arm)
        .collect::<Result<Vec<_>>>()?;

    let summary_chain = surface
        .summary
        .map(|value| quote! { .summary(#value) })
        .unwrap_or_default();
    let help_chain = surface
        .help
        .map(|value| quote! { .help(#value) })
        .unwrap_or_default();
    let description_mode_chain = match surface
        .description_mode
        .as_ref()
        .map(LitStr::value)
        .as_deref()
    {
        Some("brief") | Some("help") => {
            quote! { .description_mode(crate::plugin::sdk::ToolDescriptionMode::Brief) }
        }
        Some("detailed") => {
            quote! { .description_mode(crate::plugin::sdk::ToolDescriptionMode::Detailed) }
        }
        Some(other) => {
            let invalid = surface
                .description_mode
                .clone()
                .expect("description_mode was matched as Some");
            return Err(syn::Error::new_spanned(
                invalid,
                format!("unsupported tool description mode '{other}'"),
            ));
        }
        None => quote! {},
    };
    let ui_display_mode_chain = match surface
        .ui_display_mode
        .as_ref()
        .map(LitStr::value)
        .as_deref()
    {
        Some("summary") => {
            quote! { .ui_display_mode(crate::plugin::sdk::UiTextDisplayMode::Summary) }
        }
        Some("detailed") => {
            quote! { .ui_display_mode(crate::plugin::sdk::UiTextDisplayMode::Detailed) }
        }
        Some(other) => {
            let invalid = surface
                .ui_display_mode
                .clone()
                .expect("ui_display_mode was matched as Some");
            return Err(syn::Error::new_spanned(
                invalid,
                format!("unsupported tool ui display mode '{other}'"),
            ));
        }
        None => quote! {},
    };
    let tags_chain = if surface.tags.is_empty() {
        quote! {}
    } else {
        let tags = surface.tags;
        quote! { .tags([#(#tags),*]) }
    };
    let capabilities_chain = if surface.host_capabilities.is_empty() {
        quote! {}
    } else {
        let capabilities = surface.host_capabilities;
        quote! { .host_capabilities([#(#capabilities),*]) }
    };
    let streaming_chain = match surface.streaming.as_ref().map(LitStr::value).as_deref() {
        Some("streaming") => {
            quote! { .streaming(crate::plugin::sdk::ToolStreamingMode::Streaming) }
        }
        Some("buffered") | None => quote! {},
        Some(other) => {
            return Err(syn::Error::new_spanned(
                surface.streaming.unwrap(),
                format!("unsupported tool streaming mode '{other}'"),
            ));
        }
    };
    let strict_chain = if strict {
        quote! { .strict(true) }
    } else {
        quote! {}
    };

    Ok(quote! {
        impl #name {
            pub(crate) fn parse_input(
                input: serde_json::Value,
            ) -> crate::plugin::sdk::Result<Self> {
                match serde_json::from_value::<Self>(input) {
                    Ok(parsed) => Ok(parsed),
                    Err(primary) => Err(crate::plugin::PluginError::invalid_params(primary.to_string())),
                }
            }

            pub(crate) fn tool_decl() -> crate::plugin::sdk::PluginToolDecl {
                crate::plugin::sdk::PluginToolDecl::new(
                    #tool,
                    crate::tool::definition::json_schema_for::<Self>(),
                )
                .description(#description)
                #summary_chain
                #help_chain
                #description_mode_chain
                #ui_display_mode_chain
                #tags_chain
                #capabilities_chain
                .concurrency_safe(#concurrency_safe)
                #streaming_chain
                #strict_chain
            }

            pub(crate) fn resolve_tool(
                tool: &str,
                input: serde_json::Value,
            ) -> crate::plugin::sdk::Result<(String, serde_json::Value)> {
                match tool {
                    #tool => {}
                    other => {
                        return Err(crate::plugin::PluginError::invalid_params(format!(
                            "unknown {} tool '{other}'",
                            #tool
                        )));
                    }
                }

                let parsed = match serde_json::from_value::<Self>(input.clone()) {
                    Ok(parsed) => parsed,
                    Err(primary) => return Err(crate::plugin::PluginError::invalid_params(primary.to_string())),
                };

                match parsed {
                    #(#match_arms),*
                }
            }
        }
    })
}

fn expand_variant_arm(variant: &Variant) -> Result<proc_macro2::TokenStream> {
    let mapping = parse_variant_mapping(variant)?;
    let variant_name = &variant.ident;
    let (pattern, value_expr) = single_field_pattern_and_value(variant)?;

    Ok(match mapping {
        VariantMapping::Exec(tool_name) => quote! {
            Self::#variant_name #pattern => Ok((
                #tool_name.to_string(),
                #value_expr,
            ))
        },
        VariantMapping::Map(path) => quote! {
            Self::#variant_name #pattern => #path(value)
        },
    })
}

fn single_field_pattern_and_value(
    variant: &Variant,
) -> Result<(proc_macro2::TokenStream, proc_macro2::TokenStream)> {
    match &variant.fields {
        Fields::Named(fields) => {
            if fields.named.len() != 1 {
                return Err(syn::Error::new_spanned(
                    &variant.ident,
                    "tool variants must have exactly one field",
                ));
            }
            let field = fields.named.first().expect("one field");
            let ident = field.ident.as_ref().expect("named field");
            Ok((
                quote! {{ #ident: value }},
                quote! {
                    serde_json::to_value(value)
                        .map_err(|err| crate::plugin::PluginError::invalid_params(err.to_string()))?
                },
            ))
        }
        Fields::Unnamed(fields) => {
            if fields.unnamed.len() != 1 {
                return Err(syn::Error::new_spanned(
                    &variant.ident,
                    "tool variants must have exactly one field",
                ));
            }
            Ok((
                quote! {(value)},
                quote! {
                    serde_json::to_value(value)
                        .map_err(|err| crate::plugin::PluginError::invalid_params(err.to_string()))?
                },
            ))
        }
        Fields::Unit => Ok((
            quote! {},
            quote! { serde_json::Value::Object(serde_json::Map::new()) },
        )),
    }
}

fn parse_surface_config(attrs: &[Attribute]) -> Result<SurfaceConfig> {
    let mut config = SurfaceConfig::default();
    for attr in attrs {
        if !attr.path().is_ident("tool_surface") {
            continue;
        }
        let metas = attr.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)?;
        for meta in metas {
            match meta {
                Meta::NameValue(value) => apply_surface_name_value(&mut config, value)?,
                Meta::List(list) => apply_surface_list(&mut config, list)?,
                Meta::Path(path) => {
                    return Err(syn::Error::new_spanned(
                        path,
                        "unsupported bare tool_surface argument",
                    ));
                }
            }
        }
    }
    Ok(config)
}

fn apply_surface_name_value(config: &mut SurfaceConfig, value: MetaNameValue) -> Result<()> {
    let Some(ident) = value.path.get_ident() else {
        return Err(syn::Error::new_spanned(value.path, "expected identifier"));
    };
    match ident.to_string().as_str() {
        "tool" => config.tool = Some(expr_lit_str(&value.value, "tool")?),
        "description" => config.description = Some(expr_lit_str(&value.value, "description")?),
        "summary" => config.summary = Some(expr_lit_str(&value.value, "summary")?),
        "help" => config.help = Some(expr_lit_str(&value.value, "help")?),
        "description_mode" => {
            config.description_mode = Some(expr_lit_str(&value.value, "description_mode")?)
        }
        "ui_display_mode" => {
            config.ui_display_mode = Some(expr_lit_str(&value.value, "ui_display_mode")?)
        }
        "concurrency_safe" => {
            config.concurrency_safe = Some(expr_lit_bool(&value.value, "concurrency_safe")?)
        }
        "strict" => config.strict = Some(expr_lit_bool(&value.value, "strict")?),
        "streaming" => config.streaming = Some(expr_lit_str(&value.value, "streaming")?),
        other => {
            return Err(syn::Error::new_spanned(
                ident,
                format!("unsupported tool_surface argument '{other}'"),
            ));
        }
    }
    Ok(())
}

fn apply_surface_list(config: &mut SurfaceConfig, list: MetaList) -> Result<()> {
    let Some(ident) = list.path.get_ident() else {
        return Err(syn::Error::new_spanned(list.path, "expected identifier"));
    };
    match ident.to_string().as_str() {
        "tags" => {
            config.tags = parse_expr_list(list.tokens)?;
        }
        "host_capabilities" => {
            config.host_capabilities = parse_expr_list(list.tokens)?;
        }
        other => {
            return Err(syn::Error::new_spanned(
                ident,
                format!("unsupported tool_surface list '{other}'"),
            ));
        }
    }
    Ok(())
}

fn parse_variant_mapping(variant: &Variant) -> Result<VariantMapping> {
    for attr in &variant.attrs {
        if !attr.path().is_ident("tool") {
            continue;
        }
        let metas = attr.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)?;
        let mut mapping = None;
        for meta in metas {
            let Meta::NameValue(value) = meta else {
                return Err(syn::Error::new_spanned(
                    variant,
                    "tool attributes must use name = value syntax",
                ));
            };
            let Some(ident) = value.path.get_ident() else {
                return Err(syn::Error::new_spanned(value.path, "expected identifier"));
            };
            match ident.to_string().as_str() {
                "exec" => mapping = Some(VariantMapping::Exec(expr_lit_str(&value.value, "exec")?)),
                "map" => mapping = Some(VariantMapping::Map(expr_path(&value.value, "map")?)),
                other => {
                    return Err(syn::Error::new_spanned(
                        ident,
                        format!("unsupported tool attribute '{other}'"),
                    ));
                }
            }
        }
        if let Some(mapping) = mapping {
            return Ok(mapping);
        }
    }
    Err(syn::Error::new_spanned(
        variant,
        "missing #[tool(exec = \"...\")] or #[tool(map = path)] on variant",
    ))
}

fn parse_expr_list(tokens: proc_macro2::TokenStream) -> Result<Vec<Expr>> {
    Punctuated::<Expr, Token![,]>::parse_terminated
        .parse2(tokens)
        .map(|items| items.into_iter().collect())
}

fn expr_lit_str(expr: &Expr, field: &str) -> Result<LitStr> {
    match expr {
        Expr::Lit(ExprLit {
            lit: Lit::Str(value),
            ..
        }) => Ok(value.clone()),
        _ => Err(syn::Error::new_spanned(
            expr,
            format!("{field} must be a string literal"),
        )),
    }
}

fn expr_lit_bool(expr: &Expr, field: &str) -> Result<bool> {
    match expr {
        Expr::Lit(ExprLit {
            lit: Lit::Bool(LitBool { value, .. }),
            ..
        }) => Ok(*value),
        _ => Err(syn::Error::new_spanned(
            expr,
            format!("{field} must be a bool literal"),
        )),
    }
}

fn expr_path(expr: &Expr, field: &str) -> Result<syn::Path> {
    match expr {
        Expr::Path(ExprPath { path, .. }) => Ok(path.clone()),
        _ => Err(syn::Error::new_spanned(
            expr,
            format!("{field} must be a path"),
        )),
    }
}
