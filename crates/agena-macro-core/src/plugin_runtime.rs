//! Runtime-side types generated for plugins.

use quote::quote;
use syn::{Ident, Result, Type};

use crate::plugin_hooks::PluginHookPlan;
use crate::plugin_impl_config::{PluginImplConfig, plugin_id_label};
use crate::plugin_tooling::expand_plugin_tool_parse_input;

use super::{
    PluginCallInput, PluginContextArg, PluginOperationHandlerPlan, PluginOperationInputPlan,
    PluginOperationPlan, PluginServiceInputPlan, PluginServicePlan, PluginServiceTargetPlan,
    PluginToolNetworkPermissionRule, PluginToolOutputPlan, PluginToolPathPermissionRule,
    PluginToolPlan,
};

pub fn expand_plugin_layer_tool_invoke(
    _self_ty: &Type,
    tools: &[PluginToolPlan],
) -> Result<proc_macro2::TokenStream> {
    let branches = tools
        .iter()
        .map(expand_plugin_layer_tool_invoke_branch)
        .collect::<Result<Vec<_>>>()?;

    Ok(quote! {
        async fn tool_invoke(
            &self,
            input: ::agena_plugin_sdk::ToolInvokeInput,
        ) -> ::agena_plugin_sdk::Result<::agena_plugin_sdk::ToolInvokeOutput> {
            let ::agena_plugin_sdk::ToolInvokeInput {
                tool_name: __tool_name,
                session_id: __session_id,
                call_id: __call_id,
                workspace_root: __workspace_root,
                input: __input,
            } = input;
            let __context = ::agena_plugin_sdk::ToolInvokeContext {
                tool_name: __tool_name.as_str(),
                session_id: __session_id,
                call_id: __call_id,
                workspace_root: __workspace_root.as_str(),
            };
            match __tool_name.as_str() {
                #(#branches,)*
                _ => Err(::agena_plugin_sdk::PluginError::not_implemented(format!(
                    "tool_invoke({})",
                    __tool_name
                ))),
            }
        }
    })
}

pub fn expand_plugin_layer_service_invoke(
    _self_ty: &Type,
    services: &[PluginServicePlan],
) -> Result<proc_macro2::TokenStream> {
    let branches = services
        .iter()
        .map(expand_plugin_layer_service_invoke_branch)
        .collect::<Vec<_>>();
    Ok(quote! {
        async fn service_invoke(
            &self,
            input: ::agena_plugin_sdk::PluginServiceInvokeInput,
        ) -> ::agena_plugin_sdk::Result<::agena_plugin_sdk::serde_json::Value> {
            #(#branches)*
            Err(::agena_plugin_sdk::PluginError::not_implemented(format!(
                "service_invoke({}@v{}::{})",
                input.service, input.api_version, input.method
            )))
        }
    })
}

fn expand_plugin_layer_service_invoke_branch(
    service: &PluginServicePlan,
) -> proc_macro2::TokenStream {
    let handler = &service.handler;
    let (predicate, decode, call_args) = match &service.target {
        PluginServiceTargetPlan::Inline {
            service: service_id,
            api_version,
            method: method_id,
        } => {
            let (decode, call_args) = match &service.input {
                PluginServiceInputPlan::None => (quote! {}, quote! {}),
                PluginServiceInputPlan::Typed { ty, by_ref } => {
                    let call_arg = if *by_ref {
                        quote! { &__agena_service_input }
                    } else {
                        quote! { __agena_service_input }
                    };
                    (
                        quote! {
                            let __agena_service_input: #ty =
                                ::agena_plugin_sdk::PluginServiceInvokeExt::decode(&input)?;
                        },
                        call_arg,
                    )
                }
            };
            (
                quote! {
                    input.service == #service_id
                        && input.api_version == #api_version
                        && input.method == #method_id
                },
                decode,
                call_args,
            )
        }
        PluginServiceTargetPlan::Endpoint { endpoint } => {
            let by_ref = matches!(
                &service.input,
                PluginServiceInputPlan::Typed { by_ref: true, .. }
            );
            let call_arg = if by_ref {
                quote! { &__agena_service_input }
            } else {
                quote! { __agena_service_input }
            };
            (
                quote! {
                    input.service == <#endpoint as ::agena_plugin_sdk::PluginServiceEndpoint>::SERVICE
                        && input.api_version == <#endpoint as ::agena_plugin_sdk::PluginServiceEndpoint>::API_VERSION
                        && input.method == <#endpoint as ::agena_plugin_sdk::PluginServiceEndpoint>::METHOD
                },
                quote! {
                    let __agena_service_input:
                        <#endpoint as ::agena_plugin_sdk::PluginServiceEndpoint>::Input =
                        ::agena_plugin_sdk::PluginServiceInvokeExt::decode(&input)?;
                },
                call_arg,
            )
        }
    };
    let call = if matches!(&service.input, PluginServiceInputPlan::None) {
        quote! { self.#handler() }
    } else {
        quote! { self.#handler(#call_args) }
    };
    let call = match (service.is_async, service.returns_result) {
        (true, true) => quote! { #call.await? },
        (true, false) => quote! { #call.await },
        (false, true) => quote! { #call? },
        (false, false) => call,
    };
    let output_binding = match &service.target {
        PluginServiceTargetPlan::Inline { .. } => quote! {
            let __agena_service_output = #call;
        },
        PluginServiceTargetPlan::Endpoint { endpoint } => quote! {
            let __agena_service_output:
                <#endpoint as ::agena_plugin_sdk::PluginServiceEndpoint>::Output = #call;
        },
    };
    quote! {
        if #predicate {
            #decode
            #output_binding
            return ::agena_plugin_sdk::encode_service_output(__agena_service_output);
        }
    }
}

pub fn expand_plugin_layer_operation_invoke(
    _self_ty: &Type,
    operations: &[PluginOperationPlan],
) -> Result<proc_macro2::TokenStream> {
    let branches = operations
        .iter()
        .map(expand_plugin_layer_operation_invoke_branch)
        .collect::<Result<Vec<_>>>()?;

    Ok(quote! {
        async fn operation_invoke(
            &self,
            input: ::agena_plugin_sdk::PluginOperationInvokeInput,
        ) -> ::agena_plugin_sdk::Result<::agena_plugin_sdk::PluginOperationResult> {
            let __operation_id = input.operation_id.clone();
            let __context = ::agena_plugin_sdk::PluginOperationContext::from_input(&input);
            match __operation_id.as_str() {
                #(#branches,)*
                _ => Err(::agena_plugin_sdk::PluginError::not_implemented(format!(
                    "operation_invoke({})",
                    __operation_id
                ))),
            }
        }
    })
}

pub fn expand_plugin_layer_tool_stream(
    _self_ty: &Type,
    tools: &[PluginToolPlan],
) -> Result<proc_macro2::TokenStream> {
    let branches = tools
        .iter()
        .filter(|tool| tool.stream.is_some())
        .map(expand_plugin_layer_tool_stream_branch)
        .collect::<Result<Vec<_>>>()?;

    Ok(quote! {
        async fn tool_invoke_stream(
            &self,
            input: ::agena_plugin_sdk::ToolInvokeInput,
            sink: ::agena_plugin_sdk::ToolStreamSink,
        ) -> ::agena_plugin_sdk::Result<::agena_plugin_sdk::ToolStreamEnd> {
            let ::agena_plugin_sdk::ToolInvokeInput {
                tool_name: __tool_name,
                session_id: __session_id,
                call_id: __call_id,
                workspace_root: __workspace_root,
                input: __input,
            } = input;
            let __context = ::agena_plugin_sdk::ToolInvokeContext {
                tool_name: __tool_name.as_str(),
                session_id: __session_id,
                call_id: __call_id,
                workspace_root: __workspace_root.as_str(),
            };
            match __tool_name.as_str() {
                #(#branches,)*
                _ => {}
            }

            let __stream_id = sink.stream_id().to_string();
            let input = ::agena_plugin_sdk::ToolInvokeInput {
                tool_name: __tool_name,
                session_id: __session_id,
                call_id: __call_id,
                workspace_root: __workspace_root,
                input: __input,
            };
            let __result = self.tool_invoke(input).await?;
            sink.chunk(::agena_plugin_sdk::ToolStreamChunk {
                stream_id: __stream_id.clone(),
                text_delta: Some(__result.output_text.clone()),
                                metadata: __result.metadata.clone(),
            })
            .await;
            Ok(::agena_plugin_sdk::ToolStreamEnd::from_output(__stream_id, __result))
        }
    })
}

pub fn expand_plugin_layer_permission_paths(
    _self_ty: &Type,
    tools: &[PluginToolPlan],
) -> Result<proc_macro2::TokenStream> {
    let branches = tools
        .iter()
        .filter(|tool| tool.permissions.has_path_permissions())
        .map(|tool| expand_plugin_layer_permission_branch(tool, true))
        .collect::<Result<Vec<_>>>()?;

    Ok(quote! {
        async fn permission_paths(
            &self,
            tool: &str,
            input: &::agena_plugin_sdk::serde_json::Value,
        ) -> ::agena_plugin_sdk::Result<Vec<::agena_plugin_sdk::PathRequest>> {
            match tool {
                #(#branches,)*
                _ => Ok(Vec::new()),
            }
        }
    })
}

pub fn expand_plugin_layer_permission_networks(
    _self_ty: &Type,
    tools: &[PluginToolPlan],
) -> Result<proc_macro2::TokenStream> {
    let branches = tools
        .iter()
        .filter(|tool| tool.permissions.has_network_permissions())
        .map(|tool| expand_plugin_layer_permission_branch(tool, false))
        .collect::<Result<Vec<_>>>()?;

    Ok(quote! {
        async fn permission_networks(
            &self,
            tool: &str,
            input: &::agena_plugin_sdk::serde_json::Value,
        ) -> ::agena_plugin_sdk::Result<Vec<::agena_plugin_sdk::NetworkRequest>> {
            match tool {
                #(#branches,)*
                _ => Ok(Vec::new()),
            }
        }
    })
}

pub fn expand_plugin_layer_init_method(
    config: &PluginImplConfig,
    self_ty: &Type,
    binding: Option<&PluginHookPlan>,
) -> Result<proc_macro2::TokenStream> {
    let config_store = {
        let invalid = format!("invalid {} config", plugin_id_label(config));
        let already = format!(
            "{} plugin config already initialized",
            plugin_id_label(config)
        );
        if let Some(field) = config.settings_field.as_ref() {
            quote! {
                self.#field.set_from_json(ctx.settings.clone(), #invalid, #already)?;
            }
        } else if config.settings_store {
            quote! {
                <#self_ty as ::agena_plugin_sdk::plugin::PluginSettingsStoreAccess>::set_plugin_settings_from_json(
                    self,
                    ctx.settings.clone(),
                    #invalid,
                    #already.to_string(),
                )?;
            }
        } else {
            quote! {}
        }
    };
    let body = if let Some(binding) = binding {
        let method = &binding.method;
        plugin_layer_method_call(method, binding.is_async, &[quote! { ctx }, quote! { host }])
    } else {
        quote! { Ok(::agena_plugin_sdk::InitOutcome::ack(self.manifest())) }
    };
    Ok(quote! {
        async fn init(
            &self,
            ctx: ::agena_plugin_sdk::InitContext,
            host: ::std::sync::Arc<dyn ::agena_plugin_sdk::HostClient>,
        ) -> ::agena_plugin_sdk::Result<::agena_plugin_sdk::InitOutcome> {
            #config_store
            #body
        }
    })
}

pub fn plugin_layer_method_call(
    method: &Ident,
    is_async: bool,
    args: &[proc_macro2::TokenStream],
) -> proc_macro2::TokenStream {
    let call = quote! { Self::#method(self #(, #args)*) };
    if is_async {
        quote! { #call.await }
    } else {
        call
    }
}

fn expand_plugin_layer_tool_invoke_branch(
    tool: &PluginToolPlan,
) -> Result<proc_macro2::TokenStream> {
    let tool_name = &tool.tool;
    let handler = &tool.invoke;
    let call_args = plugin_layer_tool_call_args(handler.context, &handler.input);
    let call = plugin_layer_tool_method_call(
        &handler.method,
        handler.is_async,
        &call_args,
        &handler.output,
    );
    let parse =
        expand_plugin_tool_parse_input(&tool.input_model, quote! { __input }, &handler.method)?;
    Ok(quote! {
        #tool_name => {
            let __parsed = #parse;
            return #call;
        }
    })
}

fn expand_plugin_layer_operation_invoke_branch(
    operation: &PluginOperationPlan,
) -> Result<proc_macro2::TokenStream> {
    let id = &operation.id;
    let body = match &operation.handler {
        PluginOperationHandlerPlan::Method {
            method,
            input: operation_input,
            context,
            is_async,
        } => match operation_input {
            PluginOperationInputPlan::None => {
                let call_args = plugin_layer_operation_call_args(*context, Vec::new());
                let call = plugin_layer_method_call(method, *is_async, &call_args);
                quote! {
                    ::agena_plugin_sdk::into_plugin_operation_result(#call)
                }
            }
            PluginOperationInputPlan::Raw { by_ref, .. } => {
                let arg = if *by_ref {
                    quote! { &input }
                } else {
                    quote! { input.clone() }
                };
                let call_args = plugin_layer_operation_call_args(*context, vec![arg]);
                let call = plugin_layer_method_call(method, *is_async, &call_args);
                quote! {
                    ::agena_plugin_sdk::into_plugin_operation_result(#call)
                }
            }
            PluginOperationInputPlan::Typed { ty, by_ref } => {
                let parse = expand_plugin_operation_parse_input(ty);
                let arg = if *by_ref {
                    quote! { &__parsed }
                } else {
                    quote! { __parsed }
                };
                let call_args = plugin_layer_operation_call_args(*context, vec![arg]);
                let call = plugin_layer_method_call(method, *is_async, &call_args);
                quote! {
                    let __parsed = #parse;
                    ::agena_plugin_sdk::into_plugin_operation_result(#call)
                }
            }
            PluginOperationInputPlan::Generated { input_model, input } => {
                let parse = expand_plugin_tool_parse_input(
                    input_model,
                    quote! { input.input.clone() },
                    method,
                )?;
                let call_args =
                    plugin_layer_operation_call_args(*context, plugin_call_input_args(input));
                let call = plugin_layer_method_call(method, *is_async, &call_args);
                quote! {
                    let __parsed = #parse;
                    ::agena_plugin_sdk::into_plugin_operation_result(#call)
                }
            }
        },
        PluginOperationHandlerPlan::InvokeTool { .. } => {
            quote! {
                Ok(::agena_plugin_sdk::PluginOperationResult::unavailable(
                    "tool-backed operations are executed by the host runtime",
                ))
            }
        }
    };
    Ok(quote! {
        #id => {
            return { #body };
        }
    })
}

fn expand_plugin_operation_parse_input(ty: &Type) -> proc_macro2::TokenStream {
    quote! {{
        <#ty as ::agena_plugin_sdk::ToolInput>::parse_input(input.input.clone())?
    }}
}

fn expand_plugin_layer_tool_stream_branch(
    tool: &PluginToolPlan,
) -> Result<proc_macro2::TokenStream> {
    let tool_name = &tool.tool;
    let stream = tool.stream.as_ref().expect("stream branch prefiltered");
    let input_args = plugin_layer_tool_call_args(stream.context, &stream.input);
    let args = if stream.sink_first {
        let mut args = vec![quote! { sink }];
        args.extend(input_args);
        args
    } else {
        let mut args = input_args;
        args.push(quote! { sink });
        args
    };
    let call = plugin_layer_stream_method_call(&stream.method, stream.is_async, &args);
    let parse =
        expand_plugin_tool_parse_input(&tool.input_model, quote! { __input }, &stream.method)?;
    Ok(quote! {
        #tool_name => {
            let __parsed = #parse;
            return #call;
        }
    })
}

fn expand_plugin_layer_permission_branch(
    tool: &PluginToolPlan,
    paths: bool,
) -> Result<proc_macro2::TokenStream> {
    let tool_name = &tool.tool;
    let parse = expand_plugin_tool_parse_input(
        &tool.input_model,
        quote! { input.clone() },
        &tool.invoke.method,
    )?;
    if paths {
        let capacity = tool.permissions.path_rules.len();
        let pushes = tool.permissions.path_rules.iter().map(|rule| match rule {
            PluginToolPathPermissionRule::Read(expr) => quote! {
                if let Some(__path) = ::agena_plugin_sdk::IntoPermissionPath::into_permission_path(#expr)? {
                    __requests.push(::agena_plugin_sdk::PathRequest::read(__path));
                }
            },
            PluginToolPathPermissionRule::Reads(expr) => quote! {
                __requests.extend(
                    ::agena_plugin_sdk::IntoPermissionPaths::into_permission_paths(#expr)?
                        .into_iter()
                        .map(::agena_plugin_sdk::PathRequest::read)
                );
            },
            PluginToolPathPermissionRule::Write(expr) => quote! {
                if let Some(__path) = ::agena_plugin_sdk::IntoPermissionPath::into_permission_path(#expr)? {
                    __requests.push(::agena_plugin_sdk::PathRequest::write(__path));
                }
            },
            PluginToolPathPermissionRule::Writes(expr) => quote! {
                __requests.extend(
                    ::agena_plugin_sdk::IntoPermissionPaths::into_permission_paths(#expr)?
                        .into_iter()
                        .map(::agena_plugin_sdk::PathRequest::write)
                );
            },
            PluginToolPathPermissionRule::Requests(expr) => quote! {
                __requests.extend(
                    ::agena_plugin_sdk::IntoPathRequests::into_path_requests(#expr)?
                );
            },
        });
        Ok(quote! {
            #tool_name => {
                let __parsed = #parse;
                let input = &__parsed;
                let mut __requests = ::std::vec::Vec::with_capacity(#capacity);
                #(#pushes)*
                return Ok(__requests);
            }
        })
    } else {
        let capacity = tool.permissions.network_rules.len();
        let pushes = tool.permissions.network_rules.iter().map(|rule| match rule {
            PluginToolNetworkPermissionRule::Connect(expr) => quote! {
                if let Some(__target) = ::agena_plugin_sdk::IntoPermissionTarget::into_permission_target(#expr)? {
                    __requests.push(::agena_plugin_sdk::NetworkRequest::connect(__target));
                }
            },
            PluginToolNetworkPermissionRule::Connects(expr) => quote! {
                __requests.extend(
                    ::agena_plugin_sdk::IntoPermissionTargets::into_permission_targets(#expr)?
                        .into_iter()
                        .map(::agena_plugin_sdk::NetworkRequest::connect)
                );
            },
            PluginToolNetworkPermissionRule::Requests(expr) => quote! {
                __requests.extend(
                    ::agena_plugin_sdk::IntoNetworkRequests::into_network_requests(#expr)?
                );
            },
        });
        Ok(quote! {
            #tool_name => {
                let __parsed = #parse;
                let input = &__parsed;
                let mut __requests = ::std::vec::Vec::with_capacity(#capacity);
                #(#pushes)*
                return Ok(__requests);
            }
        })
    }
}

fn plugin_layer_tool_method_call(
    method: &Ident,
    is_async: bool,
    args: &[proc_macro2::TokenStream],
    output: &PluginToolOutputPlan,
) -> proc_macro2::TokenStream {
    let call = plugin_layer_method_call(method, is_async, args);
    if let Some(output_ty) = output.ty.as_ref() {
        if output.returns_result {
            quote! {
                match #call {
                    Ok(__value) => {
                        ::agena_plugin_sdk::macro_support::typed_tool_output::<#output_ty>(__value)
                    }
                    Err(__err) => Err(::core::convert::Into::into(__err)),
                }
            }
        } else {
            quote! {
                ::agena_plugin_sdk::macro_support::typed_tool_output::<#output_ty>(#call)
            }
        }
    } else {
        quote! {
            ::agena_plugin_sdk::IntoToolInvokeOutput::into_tool_invoke_output(#call)
        }
    }
}

fn plugin_layer_tool_call_args(
    context: Option<PluginContextArg>,
    input: &PluginCallInput,
) -> Vec<proc_macro2::TokenStream> {
    let mut input_args = plugin_call_input_args(input);
    let Some(context) = context else {
        return input_args;
    };
    let context_arg = if context.by_ref {
        quote! { &__context }
    } else {
        quote! { __context }
    };
    if context.first {
        let mut args = vec![context_arg];
        args.extend(input_args);
        args
    } else {
        input_args.push(context_arg);
        input_args
    }
}

fn plugin_layer_operation_call_args(
    context: Option<PluginContextArg>,
    mut input_args: Vec<proc_macro2::TokenStream>,
) -> Vec<proc_macro2::TokenStream> {
    let Some(context) = context else {
        return input_args;
    };
    let context_arg = if context.by_ref {
        quote! { &__context }
    } else {
        quote! { __context }
    };
    if context.first {
        let mut args = vec![context_arg];
        args.extend(input_args);
        args
    } else {
        input_args.push(context_arg);
        input_args
    }
}

fn plugin_call_input_args(input: &PluginCallInput) -> Vec<proc_macro2::TokenStream> {
    match input {
        PluginCallInput::Wrapped { by_ref } => {
            if *by_ref {
                vec![quote! { &__parsed }]
            } else {
                vec![quote! { __parsed }]
            }
        }
        PluginCallInput::Fields(fields) => fields
            .iter()
            .map(|field| quote! { __parsed.#field })
            .collect(),
    }
}

fn plugin_layer_stream_method_call(
    method: &Ident,
    is_async: bool,
    args: &[proc_macro2::TokenStream],
) -> proc_macro2::TokenStream {
    let call = plugin_layer_method_call(method, is_async, args);
    quote! {{
        let __stream_id = sink.stream_id().to_string();
        ::agena_plugin_sdk::IntoToolStreamEnd::into_tool_stream_end(#call, __stream_id)
    }}
}
