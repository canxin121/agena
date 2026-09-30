//! Tool configuration parsing for `#[tool(...)]`.

use quote::quote;
use syn::punctuated::Punctuated;
use syn::{Attribute, Expr, Ident, LitStr, Meta, Result, Token, parse_quote};

use crate::{
    PluginToolAttrConfig, PluginToolOperationConfig, default_tool_name, empty_tool_spec_config,
    expr_lit_bool, expr_lit_str, expr_path, expr_path_ident, parse_expr_list,
    parse_item_lit_str_list, parse_item_path_expr_constraint, parse_item_path_expr_list_constraint,
    parse_item_path_format_constraint, parse_item_path_lit_str_constraint,
    parse_item_path_pattern_constraint, parse_item_path_usize_constraint, parse_lit_str_list,
    parse_path_expr_constraint, parse_path_expr_list_constraint, parse_path_format_constraint,
    parse_path_lit_str_constraint, parse_path_lit_str_list_constraint, parse_path_pair_constraint,
    parse_path_pattern_constraint, parse_path_usize_constraint, parse_type_list,
    plugin_attr_has_explicit_args,
};

pub fn parse_plugin_tool_method_attr(
    attr: &Attribute,
    method_ident: &Ident,
) -> Result<PluginToolAttrConfig> {
    if !plugin_attr_has_explicit_args(attr) {
        return parse_plugin_inline_tool_config(Vec::new(), method_ident);
    }

    let metas = attr.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)?;
    parse_plugin_inline_tool_config(metas.into_iter().collect(), method_ident)
}
fn parse_plugin_inline_tool_config(
    metas: Vec<Meta>,
    method_ident: &Ident,
) -> Result<PluginToolAttrConfig> {
    let mut spec = empty_tool_spec_config();
    spec.tool = Some(LitStr::new(
        &default_tool_name(method_ident),
        method_ident.span(),
    ));
    let mut stream_method = None;
    let mut operation = None;

    for meta in metas {
        match meta {
            Meta::NameValue(value) => {
                let Some(ident) = value.path.get_ident() else {
                    return Err(syn::Error::new_spanned(value.path, "expected identifier"));
                };
                match ident.to_string().as_str() {
                    "name" => spec.tool = Some(expr_lit_str(&value.value, "name")?),
                    "summary" => spec.summary = Some(expr_lit_str(&value.value, "summary")?),
                    "help" => spec.help = Some(expr_lit_str(&value.value, "help")?),
                    "after_help" => {
                        spec.after_help = Some(expr_lit_str(&value.value, "after_help")?)
                    }
                    "before_help" => {
                        spec.before_help = Some(expr_lit_str(&value.value, "before_help")?)
                    }
                    "normalize" => spec.normalize = Some(expr_path(&value.value, "normalize")?),
                    "validate" => spec.validate = Some(expr_path(&value.value, "validate")?),
                    "output" => {
                        return Err(syn::Error::new_spanned(
                            ident,
                            "use `output(Type)` instead of `output = Type`",
                        ));
                    }
                    "stream" => {
                        if stream_method
                            .replace(expr_path_ident(value.value, "stream")?)
                            .is_some()
                        {
                            return Err(syn::Error::new_spanned(ident, "duplicate stream handler"));
                        }
                    }
                    "concurrency_safe" => {
                        spec.concurrency_safe = expr_lit_bool(&value.value, "concurrency_safe")?
                    }
                    "strict" => spec.strict = expr_lit_bool(&value.value, "strict")?,
                    "operation" => {
                        return Err(syn::Error::new_spanned(
                            ident,
                            "use `operation(...)` for operation metadata",
                        ));
                    }
                    other => {
                        return Err(syn::Error::new_spanned(
                            ident,
                            format!("unsupported inline tool argument '{other}'"),
                        ));
                    }
                }
            }
            Meta::List(list) => {
                let Some(ident) = list.path.get_ident() else {
                    return Err(syn::Error::new_spanned(list.path, "expected identifier"));
                };
                match ident.to_string().as_str() {
                    "trim" => spec.trim.extend(parse_lit_str_list(list.tokens)?),
                    "item_trim" => spec.trim.extend(parse_item_lit_str_list(list.tokens)?),
                    "trim_suffix" => spec
                        .trim_suffix
                        .push(parse_path_lit_str_constraint(list.tokens, "trim_suffix")?),
                    "item_trim_suffix" => spec.trim_suffix.push(
                        parse_item_path_lit_str_constraint(list.tokens, "item_trim_suffix")?,
                    ),
                    "non_empty" => spec.non_empty.extend(parse_lit_str_list(list.tokens)?),
                    "item_non_empty" => {
                        spec.non_empty.extend(parse_item_lit_str_list(list.tokens)?)
                    }
                    "non_empty_if_present" => spec
                        .non_empty_if_present
                        .extend(parse_lit_str_list(list.tokens)?),
                    "item_non_empty_if_present" => spec
                        .non_empty_if_present
                        .extend(parse_item_lit_str_list(list.tokens)?),
                    "minimum" => spec
                        .minimums
                        .push(parse_path_expr_constraint(list.tokens, "minimum")?),
                    "maximum" => spec
                        .maximums
                        .push(parse_path_expr_constraint(list.tokens, "maximum")?),
                    "exclusive_minimum" => spec.exclusive_minimums.push(
                        parse_path_expr_constraint(list.tokens, "exclusive_minimum")?,
                    ),
                    "exclusive_maximum" => spec.exclusive_maximums.push(
                        parse_path_expr_constraint(list.tokens, "exclusive_maximum")?,
                    ),
                    "exactly_one_of" => spec.exactly_one_of.push(parse_lit_str_list(list.tokens)?),
                    "at_least_one_of" => {
                        spec.at_least_one_of.push(parse_lit_str_list(list.tokens)?)
                    }
                    "examples" => spec.examples.extend(parse_lit_str_list(list.tokens)?),
                    "requires" => spec
                        .requires
                        .push(parse_path_pair_constraint(list.tokens, "requires")?),
                    "conflicts_with" => spec
                        .conflicts_with
                        .push(parse_path_pair_constraint(list.tokens, "conflicts_with")?),
                    "required_unless_present" => {
                        spec.required_unless_present
                            .push(parse_path_pair_constraint(
                                list.tokens,
                                "required_unless_present",
                            )?)
                    }
                    "forbid_substrings" => {
                        spec.forbid_substrings
                            .push(parse_path_lit_str_list_constraint(
                                list.tokens,
                                "forbid_substrings",
                            )?)
                    }
                    "distinct_trimmed" => spec
                        .distinct_trimmed
                        .extend(parse_lit_str_list(list.tokens)?),
                    "distinct_trimmed_within" => {
                        spec.distinct_trimmed_within
                            .push(parse_path_pair_constraint(
                                list.tokens,
                                "distinct_trimmed_within",
                            )?)
                    }
                    "min_items" => spec
                        .min_items
                        .push(parse_path_usize_constraint(list.tokens, "min_items")?),
                    "max_items" => spec
                        .max_items
                        .push(parse_path_usize_constraint(list.tokens, "max_items")?),
                    "min_properties" => spec
                        .min_properties
                        .push(parse_path_usize_constraint(list.tokens, "min_properties")?),
                    "max_properties" => spec
                        .max_properties
                        .push(parse_path_usize_constraint(list.tokens, "max_properties")?),
                    "item_minimum" => spec.minimums.push(parse_item_path_expr_constraint(
                        list.tokens,
                        "item_minimum",
                    )?),
                    "item_maximum" => spec.maximums.push(parse_item_path_expr_constraint(
                        list.tokens,
                        "item_maximum",
                    )?),
                    "item_exclusive_minimum" => {
                        spec.exclusive_minimums
                            .push(parse_item_path_expr_constraint(
                                list.tokens,
                                "item_exclusive_minimum",
                            )?)
                    }
                    "item_exclusive_maximum" => {
                        spec.exclusive_maximums
                            .push(parse_item_path_expr_constraint(
                                list.tokens,
                                "item_exclusive_maximum",
                            )?)
                    }
                    "item_min_properties" => spec.min_properties.push(
                        parse_item_path_usize_constraint(list.tokens, "item_min_properties")?,
                    ),
                    "item_max_properties" => spec.max_properties.push(
                        parse_item_path_usize_constraint(list.tokens, "item_max_properties")?,
                    ),
                    "item_min_chars" => spec.min_chars.push(parse_item_path_usize_constraint(
                        list.tokens,
                        "item_min_chars",
                    )?),
                    "item_max_chars" => spec.max_chars.push(parse_item_path_usize_constraint(
                        list.tokens,
                        "item_max_chars",
                    )?),
                    "item_format" => spec.formats.push(parse_item_path_format_constraint(
                        list.tokens,
                        "item_format",
                    )?),
                    "min_chars" => spec
                        .min_chars
                        .push(parse_path_usize_constraint(list.tokens, "min_chars")?),
                    "max_chars" => spec
                        .max_chars
                        .push(parse_path_usize_constraint(list.tokens, "max_chars")?),
                    "format" => spec
                        .formats
                        .push(parse_path_format_constraint(list.tokens, "format")?),
                    "item_pattern" => spec.patterns.push(parse_item_path_pattern_constraint(
                        list.tokens,
                        "item_pattern",
                    )?),
                    "item_choices" => spec.choices.push(parse_item_path_expr_list_constraint(
                        list.tokens,
                        "item_choices",
                    )?),
                    "pattern" => spec
                        .patterns
                        .push(parse_path_pattern_constraint(list.tokens)?),
                    "choices" => spec
                        .choices
                        .push(parse_path_expr_list_constraint(list.tokens, "choices")?),
                    "tags" => {
                        let exprs = parse_expr_list(list.tokens)?;
                        for expr in &exprs {
                            let ident = match expr {
                                Expr::Path(path) => path.path.get_ident().map(Ident::to_string),
                                _ => None,
                            };
                            if let Some(ident) = ident
                                && let Some(tag) = inline_tool_tag_expr(ident.as_str())
                            {
                                spec.tags.push(tag);
                                continue;
                            }
                            spec.tags.push(expr.clone());
                        }
                    }
                    "capabilities" => spec.capabilities = parse_expr_list(list.tokens)?,
                    "output" => spec.output_ty = Some(parse_type_list(list.tokens, "output")?),
                    "operation" => {
                        if operation
                            .replace(parse_inline_tool_operation_config(list.tokens)?)
                            .is_some()
                        {
                            return Err(syn::Error::new_spanned(
                                ident,
                                "duplicate operation config",
                            ));
                        }
                    }
                    other => {
                        return Err(syn::Error::new_spanned(
                            ident,
                            format!("unsupported inline tool list '{other}'"),
                        ));
                    }
                }
            }
            Meta::Path(path) => {
                let Some(ident) = path.get_ident() else {
                    return Err(syn::Error::new_spanned(path, "expected identifier"));
                };
                match ident.to_string().as_str() {
                    "concurrency_safe" => spec.concurrency_safe = true,
                    "strict" => spec.strict = true,
                    "operation" => {
                        if operation
                            .replace(PluginToolOperationConfig::default())
                            .is_some()
                        {
                            return Err(syn::Error::new_spanned(
                                ident,
                                "duplicate operation config",
                            ));
                        }
                    }
                    // The tool's tag vocabulary: what the tool is and does,
                    // never what it is allowed to do. A tag may be written as a
                    // bare flag (`shell`, `read_only`) or inside `tags(...)`;
                    // both spellings land in the same list.
                    tag if inline_tool_tag_expr(tag).is_some() => {
                        spec.tags
                            .push(inline_tool_tag_expr(tag).expect("tag checked as present"));
                    }
                    other => {
                        return Err(syn::Error::new_spanned(
                            ident,
                            format!("unsupported inline tool flag '{other}'"),
                        ));
                    }
                }
            }
        }
    }

    Ok(PluginToolAttrConfig {
        spec,
        stream_method,
        operation,
    })
}

fn parse_inline_tool_operation_config(
    tokens: proc_macro2::TokenStream,
) -> Result<PluginToolOperationConfig> {
    let args = syn::parse2::<crate::PluginOperationAttrArgs>(tokens)?;
    let mut config = PluginToolOperationConfig {
        slash: args.slash,
        ..PluginToolOperationConfig::default()
    };
    for meta in args.metas {
        match meta {
            Meta::NameValue(value) => {
                let Some(ident) = value.path.get_ident() else {
                    return Err(syn::Error::new_spanned(value.path, "expected identifier"));
                };
                match ident.to_string().as_str() {
                    "id" => config.id = Some(expr_lit_str(&value.value, "id")?),
                    "title" => config.title = Some(expr_lit_str(&value.value, "title")?),
                    "description" => {
                        config.description = Some(expr_lit_str(&value.value, "description")?)
                    }
                    "category" => config.category = Some(expr_lit_str(&value.value, "category")?),
                    "slash" => {
                        if config
                            .slash
                            .replace(expr_lit_str(&value.value, "slash")?)
                            .is_some()
                        {
                            return Err(syn::Error::new_spanned(ident, "duplicate slash"));
                        }
                    }
                    "usage" => config.usage = Some(expr_lit_str(&value.value, "usage")?),
                    "group" => config.group = Some(expr_lit_str(&value.value, "group")?),
                    other => {
                        return Err(syn::Error::new_spanned(
                            ident,
                            format!("unsupported tool operation argument '{other}'"),
                        ));
                    }
                }
            }
            Meta::List(list) => {
                let Some(ident) = list.path.get_ident() else {
                    return Err(syn::Error::new_spanned(list.path, "expected identifier"));
                };
                match ident.to_string().as_str() {
                    "aliases" => config.aliases.extend(parse_lit_str_list(list.tokens)?),
                    other => {
                        return Err(syn::Error::new_spanned(
                            ident,
                            format!("unsupported tool operation list '{other}'"),
                        ));
                    }
                }
            }
            Meta::Path(path) => {
                let Some(ident) = path.get_ident() else {
                    return Err(syn::Error::new_spanned(path, "expected identifier"));
                };
                let other = ident.to_string();
                return Err(syn::Error::new_spanned(
                    ident,
                    format!("unsupported tool operation flag '{other}'"),
                ));
            }
        }
    }
    if let Some(slash) = config.slash.as_ref()
        && !slash.value().starts_with('/')
    {
        return Err(syn::Error::new_spanned(
            slash,
            "tool operation slash value must start with `/`",
        ));
    }
    Ok(config)
}

fn inline_tool_tag_expr(tag: &str) -> Option<Expr> {
    let variant = match tag {
        // The tag vocabulary, one entry per spelled tag. None of them is a
        // permission.
        "query" => quote! { ::agena_plugin_sdk::ToolTag::Query },
        // `mutating` is the adjective spelling of the `mutate` tag.
        "mutating" => quote! { ::agena_plugin_sdk::ToolTag::Mutate },
        "shell" => quote! { ::agena_plugin_sdk::ToolTag::Shell },
        "task" => quote! { ::agena_plugin_sdk::ToolTag::Task },
        "read_only" => quote! { ::agena_plugin_sdk::ToolTag::ReadOnly },
        "mutate" => quote! { ::agena_plugin_sdk::ToolTag::Mutate },
        "execute" => quote! { ::agena_plugin_sdk::ToolTag::Execute },
        "filesystem" => quote! { ::agena_plugin_sdk::ToolTag::Filesystem },
        "network" => quote! { ::agena_plugin_sdk::ToolTag::Network },
        "fetch" => quote! { ::agena_plugin_sdk::ToolTag::Fetch },
        "discovery" => quote! { ::agena_plugin_sdk::ToolTag::Discovery },
        "interactive" => quote! { ::agena_plugin_sdk::ToolTag::Interactive },
        "planning" => quote! { ::agena_plugin_sdk::ToolTag::Planning },
        "goal" => quote! { ::agena_plugin_sdk::ToolTag::Goal },
        "snapshot" => quote! { ::agena_plugin_sdk::ToolTag::Snapshot },
        "scheduler" => quote! { ::agena_plugin_sdk::ToolTag::Scheduler },
        "lsp" => quote! { ::agena_plugin_sdk::ToolTag::Lsp },
        "mcp" => quote! { ::agena_plugin_sdk::ToolTag::Mcp },
        "subtask" => quote! { ::agena_plugin_sdk::ToolTag::Subtask },
        _ => return None,
    };
    Some(parse_quote!(#variant))
}
