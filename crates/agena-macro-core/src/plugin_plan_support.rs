//! Planning support for plugin tool methods.

use quote::quote;
use syn::{Attribute, Ident, ImplItem, ImplItemFn, ItemImpl, LitStr, Meta, Result, Type};

use crate::{
    PluginCommandAttrArgs, PluginCommandHandlerPlan, PluginCommandInputPlan, PluginCommandPlan,
    PluginGeneratedToolInput, PluginInherentMethodAttrs, PluginMethodInfo, PluginToolAttrConfig,
    PluginToolInvokeHandler, PluginToolPermissionHandlers, PluginToolPlan, PluginToolStreamHandler,
    PluginToolStreamSignature, build_plugin_command_input_plan, build_plugin_hook_plan,
    build_plugin_tool_method_shape, command_title_from_id, default_command_id, doc_summary,
    doc_text, ensure_plugin_method_shared_receiver, expand_plugin_tool_input_schema, expr_lit_str,
    lit_str_from_text, parse_lit_str_list, parse_plugin_tool_method_attr,
    plugin_attr_has_explicit_args, plugin_method_has_shared_receiver, plugin_method_tool_output,
    stream_sink_is_edge_info, type_display, typed_arg_types_from_inputs, types_equivalent,
};

pub fn plugin_impl_method_infos(item: &ItemImpl) -> Vec<PluginMethodInfo> {
    item.items
        .iter()
        .filter_map(|item| {
            let ImplItem::Fn(method) = item else {
                return None;
            };
            Some(PluginMethodInfo {
                ident: method.sig.ident.clone(),
                is_async: method.sig.asyncness.is_some(),
                typed_args: typed_arg_types_from_inputs(&method.sig.inputs),
                shared_receiver: plugin_method_has_shared_receiver(method),
            })
        })
        .collect()
}

fn plugin_method_info<'a>(
    methods: &'a [PluginMethodInfo],
    target: &Ident,
) -> Result<&'a PluginMethodInfo> {
    methods
        .iter()
        .find(|method| method.ident == *target)
        .ok_or_else(|| {
            syn::Error::new_spanned(
                target,
                "referenced plugin method does not exist in this impl block",
            )
        })
}

fn ensure_plugin_method_info_shared_receiver(info: &PluginMethodInfo, label: &str) -> Result<()> {
    if info.shared_receiver {
        return Ok(());
    }
    Err(syn::Error::new_spanned(
        &info.ident,
        format!("{label} must be inherent methods with `&self` receiver"),
    ))
}

fn validate_tool_stream_handler(
    methods: &[PluginMethodInfo],
    target: &Ident,
    expected_args: &[Type],
) -> Result<PluginToolStreamSignature> {
    let info = plugin_method_info(methods, target)?;
    let label = "#[tool(stream = ...)] handlers";
    ensure_plugin_method_info_shared_receiver(info, label)?;
    let sink_first = stream_sink_is_edge_info(info, label)?;
    let handler_args = stream_handler_value_arg_types(info, sink_first);
    ensure_plugin_method_info_typed_args_for_slice(
        &info.ident,
        handler_args.as_slice(),
        expected_args,
        label,
    )?;
    Ok(PluginToolStreamSignature {
        method: target.clone(),
        is_async: info.is_async,
        sink_first,
    })
}

fn stream_handler_value_arg_types(info: &PluginMethodInfo, sink_first: bool) -> Vec<Type> {
    let mut args = info.typed_args.clone();
    if sink_first {
        let _ = args.remove(0);
    } else {
        let _ = args.pop();
    }
    args
}

fn ensure_plugin_method_info_typed_args_for_slice(
    ident: &Ident,
    actual: &[Type],
    expected: &[Type],
    label: &str,
) -> Result<()> {
    if actual.len() != expected.len() {
        return Err(syn::Error::new_spanned(
            ident,
            format!(
                "{label} must take exactly {} plugin argument(s) plus ToolStreamSink",
                expected.len(),
            ),
        ));
    }
    for (index, (actual, expected)) in actual.iter().zip(expected).enumerate() {
        if !types_equivalent(actual, expected) {
            return Err(syn::Error::new_spanned(
                ident,
                format!(
                    "{label} argument {} must have type `{}`; found `{}`",
                    index + 1,
                    type_display(expected),
                    type_display(actual),
                ),
            ));
        }
    }
    Ok(())
}

pub fn parse_plugin_inherent_method_attrs(
    method: &mut ImplItemFn,
    self_label: &str,
    method_infos: &[PluginMethodInfo],
) -> Result<PluginInherentMethodAttrs> {
    let mut tools = Vec::new();
    let mut hooks = Vec::new();
    let mut commands = Vec::new();
    let mut kept_attrs = Vec::new();
    let method_ident = method.sig.ident.clone();
    let is_async = method.sig.asyncness.is_some();
    let attrs = std::mem::take(&mut method.attrs);
    for attr in attrs {
        if attr.path().is_ident("tool") {
            let method_docs = doc_text(&kept_attrs);
            tools.push(build_plugin_tool_plan(
                method,
                &method_ident,
                is_async,
                self_label,
                method_infos,
                method_docs,
                &attr,
            )?);
        } else if attr.path().is_ident("hook") {
            ensure_plugin_method_shared_receiver(method, "#[hook] methods")?;
            hooks.push(build_plugin_hook_plan(
                method,
                &method_ident,
                is_async,
                &attr,
            )?);
        } else if attr.path().is_ident("command") {
            let method_docs = doc_text(&kept_attrs);
            commands.push(build_plugin_command_plan(
                method,
                &method_ident,
                self_label,
                is_async,
                method_docs,
                &attr,
            )?);
        } else {
            kept_attrs.push(attr);
        }
    }
    method.attrs = kept_attrs;
    Ok(PluginInherentMethodAttrs {
        tools,
        hooks,
        commands,
    })
}

pub fn build_plugin_tool_plan(
    method: &mut ImplItemFn,
    method_ident: &Ident,
    is_async: bool,
    self_label: &str,
    method_infos: &[PluginMethodInfo],
    docs: Option<String>,
    attr: &Attribute,
) -> Result<PluginToolPlan> {
    ensure_plugin_method_shared_receiver(method, "#[tool] methods")?;
    let mut spec = parse_plugin_tool_method_attr(attr, method_ident)?;
    let shape = build_plugin_tool_method_shape(method, method_ident, self_label, &mut spec, docs)?;
    let stream = build_plugin_tool_stream_handler(method_infos, &shape)?;
    let permissions = build_plugin_tool_permission_handlers(&spec);
    let mut input_model = shape.input_model;
    let output = plugin_method_tool_output(method, input_model.spec.output_ty.clone());
    input_model.spec.output_ty = output.ty.clone();
    let tool = input_model
        .spec
        .tool
        .clone()
        .expect("inline tool config has a default tool name");
    if stream.is_some() {
        input_model.spec.streaming = true;
    }

    Ok(PluginToolPlan {
        tool,
        input_model,
        invoke: PluginToolInvokeHandler {
            method: method_ident.clone(),
            output,
            is_async,
            context: shape.context,
            input: shape.call_input.clone(),
        },
        stream,
        permissions,
        command: spec.command,
    })
}

pub fn build_plugin_command_plan(
    method: &mut ImplItemFn,
    method_ident: &Ident,
    self_label: &str,
    is_async: bool,
    docs: Option<String>,
    attr: &Attribute,
) -> Result<PluginCommandPlan> {
    ensure_plugin_method_shared_receiver(method, "#[command] methods")?;
    let mut id = LitStr::new(&default_command_id(method_ident), method_ident.span());
    let mut title = None;
    let mut description = lit_str_from_text(docs.as_deref());
    let mut category = LitStr::new("Plugin", method_ident.span());
    let mut slash = None;
    let mut aliases = Vec::new();
    let mut usage = None;
    let mut location = LitStr::new("command_palette", method_ident.span());
    let mut action = None;

    if plugin_attr_has_explicit_args(attr) {
        let args = attr.parse_args::<PluginCommandAttrArgs>()?;
        slash = args.slash;
        for meta in args.metas {
            match meta {
                Meta::NameValue(value) => {
                    let Some(ident) = value.path.get_ident() else {
                        return Err(syn::Error::new_spanned(value.path, "expected identifier"));
                    };
                    match ident.to_string().as_str() {
                        "id" | "name" => id = expr_lit_str(&value.value, "id")?,
                        "title" => title = Some(expr_lit_str(&value.value, "title")?),
                        "description" | "summary" => {
                            description = Some(expr_lit_str(&value.value, "description")?)
                        }
                        "category" => category = expr_lit_str(&value.value, "category")?,
                        "slash" => {
                            if slash
                                .replace(expr_lit_str(&value.value, "slash")?)
                                .is_some()
                            {
                                return Err(syn::Error::new_spanned(ident, "duplicate slash"));
                            }
                        }
                        "usage" => usage = Some(expr_lit_str(&value.value, "usage")?),
                        "location" => location = expr_lit_str(&value.value, "location")?,
                        "action" => {
                            if action.replace(value.value).is_some() {
                                return Err(syn::Error::new_spanned(ident, "duplicate action"));
                            }
                        }
                        other => {
                            return Err(syn::Error::new_spanned(
                                ident,
                                format!("unsupported command argument '{other}'"),
                            ));
                        }
                    }
                }
                Meta::List(list) => {
                    let Some(ident) = list.path.get_ident() else {
                        return Err(syn::Error::new_spanned(list.path, "expected identifier"));
                    };
                    match ident.to_string().as_str() {
                        "aliases" => aliases.extend(parse_lit_str_list(list.tokens)?),
                        other => {
                            return Err(syn::Error::new_spanned(
                                ident,
                                format!("unsupported command list '{other}'"),
                            ));
                        }
                    }
                }
                Meta::Path(path) => {
                    return Err(syn::Error::new_spanned(
                        path,
                        "unsupported bare command flag",
                    ));
                }
            }
        }
    }

    if let Some(slash) = slash.as_ref()
        && !slash.value().starts_with('/')
    {
        return Err(syn::Error::new_spanned(
            slash,
            "command slash value must start with `/`",
        ));
    }

    let method_shape =
        build_plugin_command_input_plan(method, method_ident, self_label, docs.clone())?;

    Ok(PluginCommandPlan {
        title: title.unwrap_or_else(|| {
            LitStr::new(&command_title_from_id(&id.value()), method_ident.span())
        }),
        description: description.unwrap_or_else(|| LitStr::new("", method_ident.span())),
        category,
        slash,
        aliases,
        usage,
        location,
        action,
        handler: PluginCommandHandlerPlan::Method {
            method: method_ident.clone(),
            input: method_shape.input,
            context: method_shape.context,
            is_async,
        },
        id,
    })
}

pub fn build_tool_command_plan(tool: &PluginToolPlan) -> Option<Result<PluginCommandPlan>> {
    let config = tool.command.as_ref()?;
    let id = config.id.clone().unwrap_or_else(|| tool.tool.clone());
    let title = config
        .title
        .clone()
        .unwrap_or_else(|| LitStr::new(&command_title_from_id(&id.value()), id.span()));
    let description = config
        .description
        .clone()
        .or_else(|| tool.input_model.spec.summary.clone())
        .or_else(|| lit_str_from_text(doc_summary(tool.input_model.docs.as_deref()).as_deref()))
        .unwrap_or_else(|| LitStr::new("", id.span()));
    let slash = config.slash.clone();
    if let Some(slash) = slash.as_ref()
        && !slash.value().starts_with('/')
    {
        return Some(Err(syn::Error::new_spanned(
            slash,
            "tool command slash value must start with `/`",
        )));
    }
    Some(Ok(PluginCommandPlan {
        id,
        title,
        description,
        category: config
            .category
            .clone()
            .unwrap_or_else(|| LitStr::new("Plugin", tool.tool.span())),
        slash,
        aliases: config.aliases.clone(),
        usage: config.usage.clone(),
        location: config
            .location
            .clone()
            .unwrap_or_else(|| LitStr::new("command_palette", tool.tool.span())),
        action: None,
        handler: PluginCommandHandlerPlan::InvokeTool {
            tool: tool.tool.clone(),
            input_model: Box::new(tool.input_model.clone()),
            submit_output_as_prompt: config.submit_output_as_prompt,
        },
    }))
}

pub fn command_generated_input_model(
    command: &PluginCommandPlan,
) -> Option<&PluginGeneratedToolInput> {
    match &command.handler {
        PluginCommandHandlerPlan::Method {
            input: PluginCommandInputPlan::Generated { input_model, .. },
            ..
        } => Some(input_model),
        PluginCommandHandlerPlan::Method { .. } | PluginCommandHandlerPlan::InvokeTool { .. } => {
            None
        }
    }
}

pub fn expand_plugin_command_usage_expr(
    command: &PluginCommandPlan,
) -> Result<proc_macro2::TokenStream> {
    if let Some(usage) = command.usage.as_ref() {
        return Ok(quote! { Some(#usage.to_string()) });
    }
    let Some(slash) = command.slash.as_ref() else {
        return Ok(quote! { None });
    };
    let input_usage = match &command.handler {
        PluginCommandHandlerPlan::Method { input, .. } => match input {
            PluginCommandInputPlan::Typed { ty, .. } => quote! {
                <#ty as ::agena_plugin_sdk::ToolInput>::input_usage()
            },
            PluginCommandInputPlan::Generated { input_model, .. } => {
                let spec = &input_model.spec;
                if let Some(input_shape_ty) = spec.input_shape.as_ref() {
                    quote! { <#input_shape_ty as ::agena_plugin_sdk::ToolInput>::input_usage() }
                } else {
                    let schema = expand_plugin_tool_input_schema(input_model)?;
                    quote! {
                        ::agena_plugin_sdk::macro_support::command_usage_text_from_schema(
                            &#schema,
                        )
                    }
                }
            }
            PluginCommandInputPlan::None | PluginCommandInputPlan::Raw { .. } => quote! { None },
        },
        PluginCommandHandlerPlan::InvokeTool { input_model, .. } => {
            let spec = &input_model.spec;
            if let Some(input_shape_ty) = spec.input_shape.as_ref() {
                quote! { <#input_shape_ty as ::agena_plugin_sdk::ToolInput>::input_usage() }
            } else {
                let schema = expand_plugin_tool_input_schema(input_model)?;
                quote! {
                    ::agena_plugin_sdk::macro_support::command_usage_text_from_schema(
                        &#schema,
                    )
                }
            }
        }
    };
    Ok(quote! {{
        match #input_usage {
            Some(__usage) if !__usage.trim().is_empty() => {
                Some(format!("{} {}", #slash, __usage))
            }
            _ => Some(#slash.to_string()),
        }
    }})
}

fn build_plugin_tool_stream_handler(
    methods: &[PluginMethodInfo],
    shape: &crate::PluginToolMethodShape,
) -> Result<Option<PluginToolStreamHandler>> {
    let Some(stream_method) = shape.stream_method.as_ref() else {
        return Ok(None);
    };
    let stream = validate_tool_stream_handler(methods, stream_method, &shape.stream_arg_types)?;
    Ok(Some(PluginToolStreamHandler {
        method: stream.method,
        is_async: stream.is_async,
        sink_first: stream.sink_first,
        context: shape.context,
        input: shape.call_input.clone(),
    }))
}

fn build_plugin_tool_permission_handlers(
    config: &PluginToolAttrConfig,
) -> PluginToolPermissionHandlers {
    PluginToolPermissionHandlers {
        path_rules: config.permission_path_rules.clone(),
        network_rules: config.permission_network_rules.clone(),
    }
}
