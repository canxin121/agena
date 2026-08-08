//! Hook descriptors and hook dispatch wiring for plugins.

use quote::{format_ident, quote};
use syn::{Attribute, Ident, ImplItemFn, Result, Type};

use crate::plugin_runtime::plugin_layer_method_call;

use super::plugin_hooks_support::{
    PluginHookExpectedInput, PluginHookFilters, parse_plugin_hook_attr, plugin_hook_input_segment,
    plugin_hook_name, validate_plugin_hook_filters, validate_plugin_hook_output,
};
use super::{
    PluginToolPlan, type_display, type_last_segment_is, type_mentions_segment, typed_arg_types,
};

#[derive(Clone)]
pub struct PluginHookPlan {
    pub method: Ident,
    pub hook: PluginHookKind,
    pub is_async: bool,
    pub priority: i32,
    pub filters: PluginHookFilters,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PluginHookKind {
    Init,
    Shutdown,
    ToolExecuteBefore,
    ToolExecuteAfter,
    ToolExecuteFailure,
    ToolDefinition,
    ChatMessage,
    ChatParams,
    ChatHeaders,
    ChatSystemTransform,
    ChatMessagesTransform,
    Event,
    Auth,
    ProviderList,
    Notification,
    CommandExecuteBefore,
    CommandExecuteAfter,
    ShellEnv,
    PreRun,
    PostRun,
    SessionStart,
    SessionEnd,
    UserPromptSubmit,
    AgentStop,
    ConfigResolved,
}

pub fn build_plugin_hook_plan(
    method: &ImplItemFn,
    method_ident: &Ident,
    is_async: bool,
    attr: &Attribute,
) -> Result<PluginHookPlan> {
    let config = parse_plugin_hook_attr(attr, method_ident)?;
    let hook = config.hook;
    validate_plugin_hook_filters(hook, &config.filters)?;
    let args = typed_arg_types(method);
    match plugin_hook_input_segment(hook) {
        PluginHookExpectedInput::None => {
            if !args.is_empty() {
                return Err(syn::Error::new_spanned(
                    &method.sig,
                    format!(
                        "#[hook({})] must not take typed arguments after `&self`",
                        plugin_hook_name(hook)
                    ),
                ));
            }
        }
        PluginHookExpectedInput::Single(expected) => {
            let [ty] = args.as_slice() else {
                return Err(syn::Error::new_spanned(
                    &method.sig,
                    format!(
                        "#[hook({})] must take exactly one `{expected}` argument after `&self`",
                        plugin_hook_name(hook)
                    ),
                ));
            };
            if !type_last_segment_is(ty, expected) {
                return Err(syn::Error::new_spanned(
                    ty,
                    format!(
                        "#[hook({})] input must be `{expected}`, got `{}`",
                        plugin_hook_name(hook),
                        type_display(ty)
                    ),
                ));
            }
        }
        PluginHookExpectedInput::Init => {
            let [ctx, host] = args.as_slice() else {
                return Err(syn::Error::new_spanned(
                    &method.sig,
                    "#[hook(init)] must take `InitContext` and `Arc<dyn HostClient>` after `&self`",
                ));
            };
            if !type_last_segment_is(ctx, "InitContext") {
                return Err(syn::Error::new_spanned(
                    ctx,
                    format!(
                        "#[hook(init)] first input must be `InitContext`, got `{}`",
                        type_display(ctx)
                    ),
                ));
            }
            if !type_mentions_segment(host, "HostClient") {
                return Err(syn::Error::new_spanned(
                    host,
                    format!(
                        "#[hook(init)] second input must be `Arc<dyn HostClient>`, got `{}`",
                        type_display(host)
                    ),
                ));
            }
        }
    }
    validate_plugin_hook_output(method, hook)?;
    Ok(PluginHookPlan {
        method: method_ident.clone(),
        hook,
        is_async,
        priority: config.priority,
        filters: config.filters,
    })
}

pub fn reject_duplicate_init_hooks(hooks: &[PluginHookPlan]) -> Result<()> {
    let init_count = hooks
        .iter()
        .filter(|hook| hook.hook == PluginHookKind::Init)
        .count();
    if init_count > 1 {
        let hook = hooks
            .iter()
            .find(|hook| hook.hook == PluginHookKind::Init)
            .expect("init count checked as non-zero");
        return Err(syn::Error::new_spanned(
            &hook.method,
            "duplicate #[hook(init)] binding; init remains a single plugin lifecycle hook",
        ));
    }
    Ok(())
}

pub fn plugin_layer_hooks_expr(
    tools: &[PluginToolPlan],
    hooks: &[PluginHookPlan],
) -> proc_macro2::TokenStream {
    let mut terms = Vec::new();
    if !tools.is_empty() {
        terms.push(quote! { ::agena_plugin_sdk::HookSubscription::TOOL_INVOKE });
    }
    if tools.iter().any(|tool| tool.stream.is_some()) {
        terms.push(quote! { ::agena_plugin_sdk::HookSubscription::TOOL_INVOKE_STREAM });
    }
    for hook in hooks {
        terms.push(plugin_hook_subscription_expr(hook.hook));
    }
    if terms.is_empty() {
        quote! { ::agena_plugin_sdk::HookSubscription::empty() }
    } else {
        quote! { #(#terms)|* }
    }
}

pub fn expand_plugin_layer_hook_methods(
    _self_ty: &Type,
    hooks: &[PluginHookPlan],
) -> Result<Vec<proc_macro2::TokenStream>> {
    let mut methods = Vec::new();
    for hook in plugin_hook_order() {
        if *hook == PluginHookKind::Init {
            continue;
        }
        let mut bindings = hooks
            .iter()
            .enumerate()
            .filter(|(_, binding)| binding.hook == *hook)
            .collect::<Vec<_>>();
        if bindings.is_empty() {
            continue;
        }
        bindings.sort_by(|(left_index, left), (right_index, right)| {
            right
                .priority
                .cmp(&left.priority)
                .then_with(|| left_index.cmp(right_index))
        });
        let sorted = bindings
            .into_iter()
            .map(|(_, binding)| binding)
            .collect::<Vec<_>>();
        methods.push(expand_plugin_layer_hook_method_group(*hook, &sorted)?);
    }
    Ok(methods)
}

fn plugin_hook_subscription_expr(hook: PluginHookKind) -> proc_macro2::TokenStream {
    match hook {
        PluginHookKind::Init => quote! { ::agena_plugin_sdk::HookSubscription::INIT },
        PluginHookKind::Shutdown => quote! { ::agena_plugin_sdk::HookSubscription::SHUTDOWN },
        PluginHookKind::ToolExecuteBefore => {
            quote! { ::agena_plugin_sdk::HookSubscription::TOOL_BEFORE }
        }
        PluginHookKind::ToolExecuteAfter => {
            quote! { ::agena_plugin_sdk::HookSubscription::TOOL_AFTER }
        }
        PluginHookKind::ToolExecuteFailure => {
            quote! { ::agena_plugin_sdk::HookSubscription::TOOL_FAILURE }
        }
        PluginHookKind::ToolDefinition => {
            quote! { ::agena_plugin_sdk::HookSubscription::TOOL_DEFINITION }
        }
        PluginHookKind::ChatMessage => {
            quote! { ::agena_plugin_sdk::HookSubscription::CHAT_MESSAGE }
        }
        PluginHookKind::ChatParams => quote! { ::agena_plugin_sdk::HookSubscription::CHAT_PARAMS },
        PluginHookKind::ChatHeaders => {
            quote! { ::agena_plugin_sdk::HookSubscription::CHAT_HEADERS }
        }
        PluginHookKind::ChatSystemTransform => {
            quote! { ::agena_plugin_sdk::HookSubscription::CHAT_SYSTEM_TRANSFORM }
        }
        PluginHookKind::ChatMessagesTransform => {
            quote! { ::agena_plugin_sdk::HookSubscription::CHAT_MESSAGES_TRANSFORM }
        }
        PluginHookKind::Event => quote! { ::agena_plugin_sdk::HookSubscription::EVENT },
        PluginHookKind::Auth => quote! { ::agena_plugin_sdk::HookSubscription::AUTH },
        PluginHookKind::ProviderList => {
            quote! { ::agena_plugin_sdk::HookSubscription::PROVIDER_LIST }
        }
        PluginHookKind::Notification => {
            quote! { ::agena_plugin_sdk::HookSubscription::NOTIFICATION }
        }
        PluginHookKind::CommandExecuteBefore => {
            quote! { ::agena_plugin_sdk::HookSubscription::COMMAND_BEFORE }
        }
        PluginHookKind::CommandExecuteAfter => {
            quote! { ::agena_plugin_sdk::HookSubscription::COMMAND_AFTER }
        }
        PluginHookKind::ShellEnv => quote! { ::agena_plugin_sdk::HookSubscription::SHELL_ENV },
        PluginHookKind::PreRun => quote! { ::agena_plugin_sdk::HookSubscription::PRE_RUN },
        PluginHookKind::PostRun => quote! { ::agena_plugin_sdk::HookSubscription::POST_RUN },
        PluginHookKind::SessionStart => {
            quote! { ::agena_plugin_sdk::HookSubscription::SESSION_START }
        }
        PluginHookKind::SessionEnd => quote! { ::agena_plugin_sdk::HookSubscription::SESSION_END },
        PluginHookKind::UserPromptSubmit => {
            quote! { ::agena_plugin_sdk::HookSubscription::USER_PROMPT_SUBMIT }
        }
        PluginHookKind::AgentStop => quote! { ::agena_plugin_sdk::HookSubscription::AGENT_STOP },
        PluginHookKind::ConfigResolved => quote! { ::agena_plugin_sdk::HookSubscription::CONFIG },
    }
}

fn plugin_hook_order() -> &'static [PluginHookKind] {
    &[
        PluginHookKind::Init,
        PluginHookKind::Shutdown,
        PluginHookKind::ToolExecuteBefore,
        PluginHookKind::ToolExecuteAfter,
        PluginHookKind::ToolExecuteFailure,
        PluginHookKind::ToolDefinition,
        PluginHookKind::ChatMessage,
        PluginHookKind::ChatParams,
        PluginHookKind::ChatHeaders,
        PluginHookKind::ChatSystemTransform,
        PluginHookKind::ChatMessagesTransform,
        PluginHookKind::Event,
        PluginHookKind::Auth,
        PluginHookKind::ProviderList,
        PluginHookKind::Notification,
        PluginHookKind::CommandExecuteBefore,
        PluginHookKind::CommandExecuteAfter,
        PluginHookKind::ShellEnv,
        PluginHookKind::PreRun,
        PluginHookKind::PostRun,
        PluginHookKind::SessionStart,
        PluginHookKind::SessionEnd,
        PluginHookKind::UserPromptSubmit,
        PluginHookKind::AgentStop,
        PluginHookKind::ConfigResolved,
    ]
}

fn expand_plugin_layer_hook_method_group(
    hook: PluginHookKind,
    bindings: &[&PluginHookPlan],
) -> Result<proc_macro2::TokenStream> {
    let tokens = match hook {
        PluginHookKind::Init => unreachable!("init is expanded separately"),
        PluginHookKind::Shutdown => expand_plugin_layer_no_arg_unit_hook("shutdown", bindings),
        PluginHookKind::ToolExecuteBefore => expand_plugin_layer_single_arg_hook_group(
            "tool_execute_before",
            quote! { ::agena_plugin_sdk::ToolBeforeInput },
            quote! { Option<::agena_plugin_sdk::ToolBeforePatch> },
            bindings,
        ),
        PluginHookKind::ToolExecuteAfter => expand_plugin_layer_single_arg_hook_group(
            "tool_execute_after",
            quote! { ::agena_plugin_sdk::ToolAfterInput },
            quote! { Option<::agena_plugin_sdk::ToolAfterPatch> },
            bindings,
        ),
        PluginHookKind::ToolExecuteFailure => expand_plugin_layer_single_arg_hook_group(
            "tool_execute_failure",
            quote! { ::agena_plugin_sdk::ToolFailureInput },
            quote! { () },
            bindings,
        ),
        PluginHookKind::ToolDefinition => expand_plugin_layer_single_arg_hook_group(
            "tool_definition",
            quote! { ::agena_plugin_sdk::ToolDefinitionInput },
            quote! { Option<::agena_plugin_sdk::ToolDefinitionPatch> },
            bindings,
        ),
        PluginHookKind::ChatMessage => expand_plugin_layer_single_arg_hook_group(
            "chat_message",
            quote! { ::agena_plugin_sdk::ChatMessageInput },
            quote! { Option<::agena_plugin_sdk::ChatMessagePatch> },
            bindings,
        ),
        PluginHookKind::ChatParams => expand_plugin_layer_single_arg_hook_group(
            "chat_params",
            quote! { ::agena_plugin_sdk::ChatParamsInput },
            quote! { Option<::agena_plugin_sdk::ChatParamsPatch> },
            bindings,
        ),
        PluginHookKind::ChatHeaders => expand_plugin_layer_single_arg_hook_group(
            "chat_headers",
            quote! { ::agena_plugin_sdk::ChatHeadersInput },
            quote! { Option<::agena_plugin_sdk::ChatHeadersPatch> },
            bindings,
        ),
        PluginHookKind::ChatSystemTransform => expand_plugin_layer_single_arg_hook_group(
            "chat_system_transform",
            quote! { ::agena_plugin_sdk::ChatSystemTransformInput },
            quote! { Option<::agena_plugin_sdk::ChatSystemTransformPatch> },
            bindings,
        ),
        PluginHookKind::ChatMessagesTransform => expand_plugin_layer_single_arg_hook_group(
            "chat_messages_transform",
            quote! { ::agena_plugin_sdk::ChatMessagesTransformInput },
            quote! { Option<::agena_plugin_sdk::ChatMessagesTransformPatch> },
            bindings,
        ),
        PluginHookKind::Event => expand_plugin_layer_single_arg_hook_group(
            "event",
            quote! { ::agena_plugin_sdk::EventEnvelope },
            quote! { () },
            bindings,
        ),
        PluginHookKind::Auth => expand_plugin_layer_single_arg_hook_group(
            "auth",
            quote! { ::agena_plugin_sdk::AuthInput },
            quote! { Option<::agena_plugin_sdk::AuthOutput> },
            bindings,
        ),
        PluginHookKind::ProviderList => expand_plugin_layer_single_arg_hook_group(
            "provider_list",
            quote! { ::agena_plugin_sdk::ProviderListInput },
            quote! { Option<::agena_plugin_sdk::ProviderListPatch> },
            bindings,
        ),
        PluginHookKind::Notification => expand_plugin_layer_single_arg_hook_group(
            "notification",
            quote! { ::agena_plugin_sdk::NotificationInput },
            quote! { () },
            bindings,
        ),
        PluginHookKind::CommandExecuteBefore => expand_plugin_layer_single_arg_hook_group(
            "command_execute_before",
            quote! { ::agena_plugin_sdk::CommandBeforeInput },
            quote! { Option<::agena_plugin_sdk::CommandBeforeResponse> },
            bindings,
        ),
        PluginHookKind::CommandExecuteAfter => expand_plugin_layer_single_arg_hook_group(
            "command_execute_after",
            quote! { ::agena_plugin_sdk::CommandAfterInput },
            quote! { Option<::agena_plugin_sdk::CommandAfterPatch> },
            bindings,
        ),
        PluginHookKind::ShellEnv => expand_plugin_layer_single_arg_hook_group(
            "shell_env",
            quote! { ::agena_plugin_sdk::ShellEnvInput },
            quote! { Option<::agena_plugin_sdk::ShellEnvPatch> },
            bindings,
        ),
        PluginHookKind::PreRun => expand_plugin_layer_single_arg_hook_group(
            "pre_run",
            quote! { ::agena_plugin_sdk::PreRunInput },
            quote! { () },
            bindings,
        ),
        PluginHookKind::PostRun => expand_plugin_layer_single_arg_hook_group(
            "post_run",
            quote! { ::agena_plugin_sdk::PostRunInput },
            quote! { () },
            bindings,
        ),
        PluginHookKind::SessionStart => expand_plugin_layer_single_arg_hook_group(
            "session_start",
            quote! { ::agena_plugin_sdk::SessionStartInput },
            quote! { Option<::agena_plugin_sdk::SessionStartPatch> },
            bindings,
        ),
        PluginHookKind::SessionEnd => expand_plugin_layer_single_arg_hook_group(
            "session_end",
            quote! { ::agena_plugin_sdk::SessionEndInput },
            quote! { () },
            bindings,
        ),
        PluginHookKind::UserPromptSubmit => expand_plugin_layer_single_arg_hook_group(
            "user_prompt_submit",
            quote! { ::agena_plugin_sdk::UserPromptSubmitInput },
            quote! { Option<::agena_plugin_sdk::UserPromptSubmitPatch> },
            bindings,
        ),
        PluginHookKind::AgentStop => expand_plugin_layer_single_arg_hook_group(
            "agent_stop",
            quote! { ::agena_plugin_sdk::AgentStopInput },
            quote! { Option<::agena_plugin_sdk::AgentStopPatch> },
            bindings,
        ),
        PluginHookKind::ConfigResolved => expand_plugin_layer_single_arg_hook_group(
            "config_resolved",
            quote! { ::agena_plugin_sdk::ConfigInput },
            quote! { Option<::agena_plugin_sdk::ConfigPatch> },
            bindings,
        ),
    };
    Ok(tokens)
}

fn expand_plugin_layer_no_arg_unit_hook(
    trait_method: &str,
    bindings: &[&PluginHookPlan],
) -> proc_macro2::TokenStream {
    let trait_method = format_ident!("{trait_method}");
    let calls = bindings.iter().map(|binding| {
        let call = plugin_layer_method_call(&binding.method, binding.is_async, &[]);
        quote! {
            ::agena_plugin_sdk::IntoHookOutput::<()>::into_hook_output(#call)?;
        }
    });
    quote! {
        async fn #trait_method(&self) -> ::agena_plugin_sdk::Result<()> {
            #(#calls)*
            Ok(())
        }
    }
}

fn expand_plugin_layer_single_arg_hook_group(
    trait_method: &str,
    input_ty: proc_macro2::TokenStream,
    output_ty: proc_macro2::TokenStream,
    bindings: &[&PluginHookPlan],
) -> proc_macro2::TokenStream {
    let trait_method = format_ident!("{trait_method}");
    let is_unit = output_ty.to_string() == "()";
    let calls = bindings.iter().map(|binding| {
        let guard = expand_plugin_hook_filter_guard(&binding.filters);
        let input_arg = if bindings.len() == 1 {
            quote! { input }
        } else {
            quote! { input.clone() }
        };
        let call = plugin_layer_method_call(&binding.method, binding.is_async, &[input_arg]);
        if is_unit {
            quote! {
                if #guard {
                    ::agena_plugin_sdk::IntoHookOutput::<#output_ty>::into_hook_output(#call)?;
                }
            }
        } else {
            quote! {
                if #guard {
                    let __hook_output =
                        ::agena_plugin_sdk::IntoHookOutput::<#output_ty>::into_hook_output(#call)?;
                    if __hook_output.is_some() {
                        return Ok(__hook_output);
                    }
                }
            }
        }
    });
    let fallback = if is_unit {
        quote! { Ok(()) }
    } else {
        quote! { Ok(None) }
    };
    quote! {
        async fn #trait_method(
            &self,
            input: #input_ty,
        ) -> ::agena_plugin_sdk::Result<#output_ty> {
            #(#calls)*
            #fallback
        }
    }
}

fn expand_plugin_hook_filter_guard(filters: &PluginHookFilters) -> proc_macro2::TokenStream {
    let mut terms = Vec::new();
    if !filters.tools.is_empty() {
        let tools = &filters.tools;
        terms.push(quote! { (#(input.tool_name() == #tools)||*) });
    }
    if !filters.plugins.is_empty() {
        let plugin_matches = filters.plugins.iter().map(|plugin| {
            let namespace = &plugin.namespace;
            let name = &plugin.name;
            quote! { (__plugin.namespace() == #namespace && __plugin.name() == #name) }
        });
        terms.push(quote! {{
            let __plugin = input.plugin_key();
            (#(#plugin_matches)||*)
        }});
    }
    if !filters.tags.is_empty() {
        let tags = &filters.tags;
        terms.push(quote! {
            input.tags.iter().any(|__tag| #(__tag.as_ref() == #tags)||*)
        });
    }
    if !filters.commands.is_empty() {
        let commands = &filters.commands;
        terms.push(quote! { (#(input.command.as_str() == #commands)||*) });
    }
    if terms.is_empty() {
        quote! { true }
    } else {
        quote! { (#(#terms)&&*) }
    }
}
