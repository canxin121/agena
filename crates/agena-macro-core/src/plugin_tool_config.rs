use quote::quote;
use syn::parse::Parser;
use syn::punctuated::Punctuated;
use syn::{Attribute, Expr, Ident, LitStr, Meta, Result, Token, parse_quote};

use crate::{
    PluginToolAttrConfig, PluginToolCommandConfig, PluginToolNetworkPermissionRule,
    PluginToolPathPermissionRule, default_tool_name, empty_tool_spec_config, expr_lit_bool,
    expr_lit_str, expr_path, expr_path_ident, expr_string_like, parse_expr_list,
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
    let mut permission_path_rules = Vec::new();
    let mut permission_network_rules = Vec::new();
    let mut command = None;

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
                    "display" => spec.display = Some(expr_string_like(&value.value, "display")?),
                    "ui_display" => {
                        spec.ui_display = Some(expr_string_like(&value.value, "ui_display")?)
                    }
                    "output" => {
                        return Err(syn::Error::new_spanned(
                            ident,
                            "use `output(Type)` instead of `output = Type`",
                        ));
                    }
                    "description_mode" => {
                        spec.description_mode =
                            Some(expr_string_like(&value.value, "description_mode")?)
                    }
                    "ui_display_mode" => {
                        spec.ui_display_mode =
                            Some(expr_string_like(&value.value, "ui_display_mode")?)
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
                    "command" => {
                        let config = PluginToolCommandConfig {
                            slash: Some(expr_lit_str(&value.value, "command")?),
                            ..PluginToolCommandConfig::default()
                        };
                        if command.replace(config).is_some() {
                            return Err(syn::Error::new_spanned(ident, "duplicate command config"));
                        }
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
                                Expr::Path(path) => path
                                    .path
                                    .get_ident()
                                    .map(Ident::to_string),
                                _ => None,
                            };
                            if let Some(ident) = ident {
                                if let Some(tag) = inline_tool_tag_expr(ident.as_str()) {
                                    spec.tags.push(tag);
                                    continue;
                                }
                            }
                            spec.tags.push(expr.clone());
                        }
                    }
                    "capabilities" => spec.capabilities = parse_expr_list(list.tokens)?,
                    "output" => spec.output_ty = Some(parse_type_list(list.tokens, "output")?),
                    "permission" => {
                        return Err(syn::Error::new_spanned(
                            ident,
                            "permission(...) has been removed; use path(...) or network(...)",
                        ));
                    }
                    "path" => {
                        let rules = parse_inline_path_permission_rules(list.tokens)?;
                        permission_path_rules.extend(rules);
                    }
                    "network" => {
                        let rules = parse_inline_network_permission_rules(list.tokens)?;
                        permission_network_rules.extend(rules);
                    }
                    "command" => {
                        if command
                            .replace(parse_inline_tool_command_config(list.tokens)?)
                            .is_some()
                        {
                            return Err(syn::Error::new_spanned(ident, "duplicate command config"));
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
                    "command" => {
                        if command
                            .replace(PluginToolCommandConfig::default())
                            .is_some()
                        {
                            return Err(syn::Error::new_spanned(ident, "duplicate command config"));
                        }
                    }
                    // Authority-bearing capability flags. These live on the
                    // permission contract, never as tags: tags are metadata.
                    "mutating" => spec.mutating = true,
                    "read_only" => spec.read_only = true,
                    "shell" => spec.shell = true,
                    "interactive" => spec.interactive = true,
                    "task" => spec.task = true,
                    // Function/category metadata tags: describe what the tool
                    // does, never what it is allowed to do.
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
        permission_path_rules,
        permission_network_rules,
        command,
    })
}

fn parse_inline_tool_command_config(
    tokens: proc_macro2::TokenStream,
) -> Result<PluginToolCommandConfig> {
    let args = syn::parse2::<crate::PluginCommandAttrArgs>(tokens)?;
    let mut config = PluginToolCommandConfig {
        slash: args.slash,
        ..PluginToolCommandConfig::default()
    };
    for meta in args.metas {
        match meta {
            Meta::NameValue(value) => {
                let Some(ident) = value.path.get_ident() else {
                    return Err(syn::Error::new_spanned(value.path, "expected identifier"));
                };
                match ident.to_string().as_str() {
                    "id" | "name" => config.id = Some(expr_lit_str(&value.value, "id")?),
                    "title" => config.title = Some(expr_lit_str(&value.value, "title")?),
                    "description" | "summary" => {
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
                    "location" => config.location = Some(expr_lit_str(&value.value, "location")?),
                    "submit_output_as_prompt" => {
                        config.submit_output_as_prompt =
                            expr_lit_bool(&value.value, "submit_output_as_prompt")?
                    }
                    other => {
                        return Err(syn::Error::new_spanned(
                            ident,
                            format!("unsupported tool command argument '{other}'"),
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
                            format!("unsupported tool command list '{other}'"),
                        ));
                    }
                }
            }
            Meta::Path(path) => {
                let Some(ident) = path.get_ident() else {
                    return Err(syn::Error::new_spanned(path, "expected identifier"));
                };
                match ident.to_string().as_str() {
                    "submit_output_as_prompt" => config.submit_output_as_prompt = true,
                    other => {
                        return Err(syn::Error::new_spanned(
                            ident,
                            format!("unsupported tool command flag '{other}'"),
                        ));
                    }
                }
            }
        }
    }
    if let Some(slash) = config.slash.as_ref()
        && !slash.value().starts_with('/')
    {
        return Err(syn::Error::new_spanned(
            slash,
            "tool command slash value must start with `/`",
        ));
    }
    Ok(config)
}

fn parse_inline_path_permission_rules(
    tokens: proc_macro2::TokenStream,
) -> Result<Vec<PluginToolPathPermissionRule>> {
    let metas = Punctuated::<Meta, Token![,]>::parse_terminated.parse2(tokens)?;
    let mut rules = Vec::new();
    for meta in metas {
        let Meta::NameValue(value) = meta else {
            return Err(syn::Error::new_spanned(
                meta,
                "path(...) expects read = expr, reads = expr, write = expr, writes = expr, or requests = expr",
            ));
        };
        let Some(ident) = value.path.get_ident() else {
            return Err(syn::Error::new_spanned(value.path, "expected identifier"));
        };
        let rule = match ident.to_string().as_str() {
            "read" => PluginToolPathPermissionRule::Read(value.value),
            "reads" => PluginToolPathPermissionRule::Reads(value.value),
            "write" => PluginToolPathPermissionRule::Write(value.value),
            "writes" => PluginToolPathPermissionRule::Writes(value.value),
            "requests" => PluginToolPathPermissionRule::Requests(value.value),
            other => {
                return Err(syn::Error::new_spanned(
                    ident,
                    format!("unsupported path permission rule '{other}'"),
                ));
            }
        };
        rules.push(rule);
    }
    Ok(rules)
}

fn parse_inline_network_permission_rules(
    tokens: proc_macro2::TokenStream,
) -> Result<Vec<PluginToolNetworkPermissionRule>> {
    let metas = Punctuated::<Meta, Token![,]>::parse_terminated.parse2(tokens)?;
    let mut rules = Vec::new();
    for meta in metas {
        let Meta::NameValue(value) = meta else {
            return Err(syn::Error::new_spanned(
                meta,
                "network(...) expects connect = expr, connects = expr, or requests = expr",
            ));
        };
        let Some(ident) = value.path.get_ident() else {
            return Err(syn::Error::new_spanned(value.path, "expected identifier"));
        };
        match ident.to_string().as_str() {
            "connect" => rules.push(PluginToolNetworkPermissionRule::Connect(value.value)),
            "connects" => rules.push(PluginToolNetworkPermissionRule::Connects(value.value)),
            "requests" => rules.push(PluginToolNetworkPermissionRule::Requests(value.value)),
            other => {
                return Err(syn::Error::new_spanned(
                    ident,
                    format!("unsupported network permission rule '{other}'"),
                ));
            }
        }
    }
    Ok(rules)
}

fn inline_tool_tag_expr(tag: &str) -> Option<Expr> {
    let variant = match tag {
        // Function/category metadata only. Tags describe what a tool does for
        // discovery/UI/workflow hints and carry no authority: permission
        // declarations live on the tool contract, never on a tag.
        "query" => quote! { ::agena_plugin_sdk::ToolTag::Query },
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
