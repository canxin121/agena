//! Planning support for plugin tool methods.

use quote::quote;
use syn::{Attribute, Ident, ImplItem, ImplItemFn, ItemImpl, LitStr, Meta, Result, Type};

use crate::{
    PluginGeneratedToolInput, PluginInherentMethodAttrs, PluginMethodInfo, PluginOperationAttrArgs,
    PluginOperationHandlerPlan, PluginOperationInputPlan, PluginOperationPlan,
    PluginServiceAttrArgs, PluginServiceAttrTarget, PluginServiceInputPlan, PluginServicePlan,
    PluginServiceTargetPlan, PluginToolAttrConfig, PluginToolInvokeHandler,
    PluginToolPermissionHandlers, PluginToolPlan, PluginToolStreamHandler,
    PluginToolStreamSignature, build_plugin_hook_plan, build_plugin_operation_input_plan,
    build_plugin_tool_method_shape, default_operation_id, doc_summary, doc_text,
    ensure_plugin_method_shared_receiver, expand_plugin_tool_input_schema, expr_lit_str,
    expr_lit_usize, lit_str_from_text, operation_title_from_id, parse_lit_str_list,
    parse_plugin_tool_method_attr, plugin_attr_has_explicit_args,
    plugin_method_has_shared_receiver, plugin_method_return_value_type, plugin_method_tool_output,
    stream_sink_is_edge_info, type_display, type_is_reference, type_is_unit,
    type_without_reference, typed_arg_types_from_inputs, types_equivalent,
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
    let mut operations = Vec::new();
    let mut services = Vec::new();
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
        } else if attr.path().is_ident("operation") {
            let method_docs = doc_text(&kept_attrs);
            operations.push(build_plugin_operation_plan(
                method,
                &method_ident,
                self_label,
                is_async,
                method_docs,
                &attr,
            )?);
        } else if attr.path().is_ident("service") {
            services.push(build_plugin_service_plan(
                method,
                &method_ident,
                is_async,
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
        operations,
        services,
    })
}

pub fn build_plugin_service_plan(
    method: &mut ImplItemFn,
    method_ident: &Ident,
    is_async: bool,
    attr: &Attribute,
) -> Result<PluginServicePlan> {
    ensure_plugin_method_shared_receiver(method, "#[service] methods")?;
    let args = attr.parse_args::<PluginServiceAttrArgs>()?;
    let target = match args.target {
        PluginServiceAttrTarget::Inline(service) => {
            let mut api_version = None;
            let mut method_id = LitStr::new(&method_ident.to_string(), method_ident.span());
            for meta in args.metas {
                let Meta::NameValue(value) = meta else {
                    return Err(syn::Error::new_spanned(
                        meta,
                        "#[service] accepts only `version = N` and `method = \"id\"` arguments",
                    ));
                };
                let Some(ident) = value.path.get_ident() else {
                    return Err(syn::Error::new_spanned(value.path, "expected identifier"));
                };
                match ident.to_string().as_str() {
                    "version" => {
                        let parsed = expr_lit_usize(&value.value, "version")?;
                        let parsed = u32::try_from(parsed).map_err(|_| {
                            syn::Error::new_spanned(
                                &value.value,
                                "service version exceeds u32::MAX",
                            )
                        })?;
                        if parsed == 0 {
                            return Err(syn::Error::new_spanned(
                                &value.value,
                                "service version must be positive",
                            ));
                        }
                        if api_version.replace(parsed).is_some() {
                            return Err(syn::Error::new_spanned(
                                ident,
                                "duplicate service version",
                            ));
                        }
                    }
                    "method" => method_id = expr_lit_str(&value.value, "method")?,
                    other => {
                        return Err(syn::Error::new_spanned(
                            ident,
                            format!("unsupported service argument '{other}'"),
                        ));
                    }
                }
            }
            let api_version = api_version.ok_or_else(|| {
                syn::Error::new_spanned(attr, "#[service] requires an explicit `version = N`")
            })?;
            PluginServiceTargetPlan::Inline {
                service,
                api_version,
                method: method_id,
            }
        }
        PluginServiceAttrTarget::Endpoint(endpoint) => {
            if let Some(meta) = args.metas.first() {
                return Err(syn::Error::new_spanned(
                    meta,
                    "typed endpoint form `#[service(Endpoint)]` does not accept version/method overrides; define them once on the endpoint",
                ));
            }
            PluginServiceTargetPlan::Endpoint { endpoint }
        }
    };

    let typed_args = typed_arg_types_from_inputs(&method.sig.inputs);
    let input = match (&target, typed_args.as_slice()) {
        (PluginServiceTargetPlan::Inline { .. }, []) => PluginServiceInputPlan::None,
        (_, [ty]) => PluginServiceInputPlan::Typed {
            ty: Box::new(type_without_reference(ty)),
            by_ref: type_is_reference(ty),
        },
        (PluginServiceTargetPlan::Endpoint { .. }, []) => {
            return Err(syn::Error::new_spanned(
                &method.sig.inputs,
                "typed endpoint #[service(Endpoint)] methods require exactly one request argument matching Endpoint::Input",
            ));
        }
        _ => {
            return Err(syn::Error::new_spanned(
                &method.sig.inputs,
                "#[service] methods support zero or one typed input argument; use a request struct to group fields",
            ));
        }
    };
    for arg in method.sig.inputs.iter() {
        let syn::FnArg::Typed(arg) = arg else {
            continue;
        };
        if let Some(attr) = arg.attrs.first() {
            return Err(syn::Error::new_spanned(
                attr,
                "#[service] input arguments do not support parameter attributes; put validation metadata on the request type",
            ));
        }
    }

    let Some((output, returns_result)) = plugin_method_return_value_type(method) else {
        return Err(syn::Error::new_spanned(
            &method.sig.output,
            "#[service] methods must return a typed value or Result<T>",
        ));
    };
    if type_is_unit(&output) {
        return Err(syn::Error::new_spanned(
            &method.sig.output,
            "#[service] methods must return a serializable typed value; use an explicit empty response struct instead of ()",
        ));
    }

    Ok(PluginServicePlan {
        target,
        handler: method_ident.clone(),
        input,
        output,
        returns_result,
        is_async,
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
        operation: spec.operation,
    })
}

pub fn build_plugin_operation_plan(
    method: &mut ImplItemFn,
    method_ident: &Ident,
    self_label: &str,
    is_async: bool,
    docs: Option<String>,
    attr: &Attribute,
) -> Result<PluginOperationPlan> {
    ensure_plugin_method_shared_receiver(method, "#[operation] methods")?;
    let mut id = LitStr::new(&default_operation_id(method_ident), method_ident.span());
    let mut title = None;
    let mut description = lit_str_from_text(docs.as_deref());
    let mut category = LitStr::new("Plugin", method_ident.span());
    let mut slash = None;
    let mut aliases = Vec::new();
    let mut usage = None;
    let mut group = LitStr::new("command_palette", method_ident.span());

    if plugin_attr_has_explicit_args(attr) {
        let args = attr.parse_args::<PluginOperationAttrArgs>()?;
        slash = args.slash;
        for meta in args.metas {
            match meta {
                Meta::NameValue(value) => {
                    let Some(ident) = value.path.get_ident() else {
                        return Err(syn::Error::new_spanned(value.path, "expected identifier"));
                    };
                    match ident.to_string().as_str() {
                        "id" => id = expr_lit_str(&value.value, "id")?,
                        "title" => title = Some(expr_lit_str(&value.value, "title")?),
                        "description" => {
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
                        "group" => group = expr_lit_str(&value.value, "group")?,
                        other => {
                            return Err(syn::Error::new_spanned(
                                ident,
                                format!("unsupported operation argument '{other}'"),
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
                                format!("unsupported operation list '{other}'"),
                            ));
                        }
                    }
                }
                Meta::Path(path) => {
                    return Err(syn::Error::new_spanned(
                        path,
                        "unsupported bare operation flag",
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
            "operation slash value must start with `/`",
        ));
    }

    let method_shape =
        build_plugin_operation_input_plan(method, method_ident, self_label, docs.clone())?;

    Ok(PluginOperationPlan {
        title: title.unwrap_or_else(|| {
            LitStr::new(&operation_title_from_id(&id.value()), method_ident.span())
        }),
        description: description.unwrap_or_else(|| LitStr::new("", method_ident.span())),
        group,
        category,
        slash,
        aliases,
        usage,
        handler: PluginOperationHandlerPlan::Method {
            method: method_ident.clone(),
            input: method_shape.input,
            context: method_shape.context,
            is_async,
        },
        id,
    })
}

pub fn build_tool_operation_plan(tool: &PluginToolPlan) -> Option<Result<PluginOperationPlan>> {
    let config = tool.operation.as_ref()?;
    let id = config.id.clone().unwrap_or_else(|| tool.tool.clone());
    let title = config
        .title
        .clone()
        .unwrap_or_else(|| LitStr::new(&operation_title_from_id(&id.value()), id.span()));
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
            "tool operation slash value must start with `/`",
        )));
    }
    Some(Ok(PluginOperationPlan {
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
        group: config
            .group
            .clone()
            .unwrap_or_else(|| LitStr::new("command_palette", tool.tool.span())),
        handler: PluginOperationHandlerPlan::InvokeTool {
            tool: tool.tool.clone(),
            input_model: Box::new(tool.input_model.clone()),
        },
    }))
}

pub fn operation_generated_input_model(
    operation: &PluginOperationPlan,
) -> Option<&PluginGeneratedToolInput> {
    match &operation.handler {
        PluginOperationHandlerPlan::Method {
            input: PluginOperationInputPlan::Generated { input_model, .. },
            ..
        } => Some(input_model),
        PluginOperationHandlerPlan::Method { .. }
        | PluginOperationHandlerPlan::InvokeTool { .. } => None,
    }
}

pub fn expand_plugin_operation_usage_expr(
    operation: &PluginOperationPlan,
) -> Result<proc_macro2::TokenStream> {
    if let Some(usage) = operation.usage.as_ref() {
        return Ok(quote! { Some(#usage.to_string()) });
    }
    let Some(slash) = operation.slash.as_ref() else {
        return Ok(quote! { None });
    };
    let input_usage = match &operation.handler {
        PluginOperationHandlerPlan::Method { input, .. } => match input {
            PluginOperationInputPlan::Typed { ty, .. } => quote! {
                <#ty as ::agena_plugin_sdk::ToolInput>::input_usage()
            },
            PluginOperationInputPlan::Generated { input_model, .. } => {
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
            PluginOperationInputPlan::None | PluginOperationInputPlan::Raw { .. } => {
                quote! { None }
            }
        },
        PluginOperationHandlerPlan::InvokeTool { input_model, .. } => {
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
