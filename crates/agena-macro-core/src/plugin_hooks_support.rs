use syn::ext::IdentExt;
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::{
    Attribute, Expr, ExprLit, ExprPath, Ident, ImplItemFn, Lit, LitStr, PathArguments, Result,
    Token, Type,
};

use crate::plugin_hooks::PluginHookKind;

use super::{
    expr_lit_i32, plugin_attr_has_explicit_args, plugin_method_return_type,
    plugin_method_return_value_type, type_display, type_is_unit, type_last_segment_is,
};

#[derive(Clone, Default)]
pub struct PluginHookFilters {
    pub tools: Vec<LitStr>,
    pub commands: Vec<LitStr>,
    pub plugins: Vec<PluginHookPluginFilter>,
    pub tags: Vec<LitStr>,
}

#[derive(Clone)]
pub struct PluginHookPluginFilter {
    pub value: LitStr,
    pub namespace: LitStr,
    pub name: LitStr,
}

#[derive(Clone)]
pub struct PluginHookAttrConfig {
    pub hook: PluginHookKind,
    pub priority: i32,
    pub filters: PluginHookFilters,
}

struct PluginHookAttrArgs {
    key: String,
    priority: i32,
    filters: PluginHookFilters,
}

impl Parse for PluginHookAttrArgs {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let mut key = input.call(Ident::parse_any)?.to_string();
        while input.peek(Token![.]) {
            input.parse::<Token![.]>()?;
            key.push('.');
            key.push_str(&input.call(Ident::parse_any)?.to_string());
        }

        let mut priority = 0;
        let mut filters = PluginHookFilters::default();
        while input.peek(Token![,]) {
            input.parse::<Token![,]>()?;
            if input.is_empty() {
                break;
            }
            let option = input.call(Ident::parse_any)?;
            match option.to_string().as_str() {
                "priority" => {
                    input.parse::<Token![=]>()?;
                    priority = expr_lit_i32(&input.parse::<Expr>()?, "priority")?;
                }
                "tool" => {
                    input.parse::<Token![=]>()?;
                    filters.tools.push(input.parse()?);
                }
                "tools" => {
                    let content;
                    syn::parenthesized!(content in input);
                    filters
                        .tools
                        .extend(Punctuated::<LitStr, Token![,]>::parse_terminated(&content)?);
                }
                "plugin" => {
                    input.parse::<Token![=]>()?;
                    filters
                        .plugins
                        .push(parse_plugin_hook_plugin_filter(input.parse()?)?);
                }
                "plugins" => {
                    let content;
                    syn::parenthesized!(content in input);
                    let plugins = Punctuated::<LitStr, Token![,]>::parse_terminated(&content)?;
                    for plugin in plugins {
                        filters
                            .plugins
                            .push(parse_plugin_hook_plugin_filter(plugin)?);
                    }
                }
                "tag" => {
                    input.parse::<Token![=]>()?;
                    filters
                        .tags
                        .push(parse_plugin_hook_tag_filter(input.parse()?)?);
                }
                "tags" => {
                    let content;
                    syn::parenthesized!(content in input);
                    let tags = Punctuated::<Expr, Token![,]>::parse_terminated(&content)?;
                    for tag in tags {
                        filters.tags.push(parse_plugin_hook_tag_filter(tag)?);
                    }
                }
                "command" => {
                    input.parse::<Token![=]>()?;
                    filters.commands.push(input.parse()?);
                }
                "commands" => {
                    let content;
                    syn::parenthesized!(content in input);
                    filters
                        .commands
                        .extend(Punctuated::<LitStr, Token![,]>::parse_terminated(&content)?);
                }
                other => {
                    return Err(syn::Error::new_spanned(
                        option,
                        format!("unsupported hook option '{other}'"),
                    ));
                }
            }
        }

        Ok(Self {
            key,
            priority,
            filters,
        })
    }
}

pub fn parse_plugin_hook_attr(
    attr: &Attribute,
    _method_ident: &Ident,
) -> Result<PluginHookAttrConfig> {
    if plugin_attr_has_explicit_args(attr) {
        let args = attr.parse_args::<PluginHookAttrArgs>()?;
        let key = args.key;
        if key.contains('_') {
            return Err(syn::Error::new_spanned(
                attr,
                "explicit hook names use dotted DSL, e.g. #[hook(tool.before)] or #[hook(shell.after)]",
            ));
        }
        Ok(PluginHookAttrConfig {
            hook: plugin_hook_kind_from_key(&key, attr)?,
            priority: args.priority,
            filters: args.filters,
        })
    } else {
        Err(syn::Error::new_spanned(
            attr,
            "#[hook] now requires an explicit hook name, e.g. #[hook(init)], #[hook(tool.before)], or #[hook(shell.after)]",
        ))
    }
}

fn plugin_hook_kind_from_key(key: &str, span: impl quote::ToTokens) -> Result<PluginHookKind> {
    match key {
        "init" => Ok(PluginHookKind::Init),
        "shutdown" => Ok(PluginHookKind::Shutdown),
        "tool.before" => Ok(PluginHookKind::ToolExecuteBefore),
        "tool.after" => Ok(PluginHookKind::ToolExecuteAfter),
        "tool.failure" => Ok(PluginHookKind::ToolExecuteFailure),
        "tool.definition" => Ok(PluginHookKind::ToolDefinition),
        "chat.message" => Ok(PluginHookKind::ChatMessage),
        "chat.params" => Ok(PluginHookKind::ChatParams),
        "chat.headers" => Ok(PluginHookKind::ChatHeaders),
        "chat.system" => Ok(PluginHookKind::ChatSystemTransform),
        "chat.messages" => Ok(PluginHookKind::ChatMessagesTransform),
        "event" => Ok(PluginHookKind::Event),
        "auth" => Ok(PluginHookKind::Auth),
        "provider.list" => Ok(PluginHookKind::ProviderList),
        "permission.ask" => Ok(PluginHookKind::PermissionAsk),
        "notification" => Ok(PluginHookKind::Notification),
        "shell.before" => Ok(PluginHookKind::CommandExecuteBefore),
        "shell.after" => Ok(PluginHookKind::CommandExecuteAfter),
        "shell.env" => Ok(PluginHookKind::ShellEnv),
        "run.pre" => Ok(PluginHookKind::PreRun),
        "run.post" => Ok(PluginHookKind::PostRun),
        "session.start" => Ok(PluginHookKind::SessionStart),
        "session.end" => Ok(PluginHookKind::SessionEnd),
        "prompt.submit" => Ok(PluginHookKind::UserPromptSubmit),
        "agent.stop" => Ok(PluginHookKind::AgentStop),
        "config.resolved" => Ok(PluginHookKind::ConfigResolved),
        other => Err(syn::Error::new_spanned(
            span,
            format!("unsupported plugin hook '{other}'"),
        )),
    }
}

fn parse_plugin_hook_plugin_filter(value: LitStr) -> Result<PluginHookPluginFilter> {
    let raw = value.value();
    let Some((namespace, name)) = raw.split_once('.') else {
        return Err(syn::Error::new_spanned(
            value,
            "plugin filters must use `namespace.name`, e.g. plugin = \"agena.fs\"",
        ));
    };
    if namespace.is_empty() || name.is_empty() || name.contains('.') {
        return Err(syn::Error::new_spanned(
            value,
            "plugin filters must use exactly `namespace.name`",
        ));
    }
    Ok(PluginHookPluginFilter {
        namespace: LitStr::new(namespace, value.span()),
        name: LitStr::new(name, value.span()),
        value,
    })
}

fn parse_plugin_hook_tag_filter(expr: Expr) -> Result<LitStr> {
    let (tag, span) = match expr {
        Expr::Lit(ExprLit {
            lit: Lit::Str(value),
            ..
        }) => (value.value(), value.span()),
        Expr::Path(ExprPath { path, .. }) => {
            let Some(ident) = path.get_ident() else {
                return Err(syn::Error::new_spanned(
                    path,
                    "tag filters expect a tag identifier or string literal",
                ));
            };
            (ident.to_string(), ident.span())
        }
        other => {
            return Err(syn::Error::new_spanned(
                other,
                "tag filters expect a tag identifier or string literal",
            ));
        }
    };
    let Some(normalized) = normalize_plugin_hook_tag_filter(&tag) else {
        return Err(syn::Error::new(
            span,
            "tag filters cannot be empty after normalization",
        ));
    };
    Ok(LitStr::new(&normalized, span))
}

fn normalize_plugin_hook_tag_filter(tag: &str) -> Option<String> {
    let normalized = tag
        .trim()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect::<String>()
        .split('_')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("_");
    (!normalized.is_empty()).then_some(normalized)
}

pub fn validate_plugin_hook_filters(
    hook: PluginHookKind,
    filters: &PluginHookFilters,
) -> Result<()> {
    if !filters.tools.is_empty()
        && !matches!(
            hook,
            PluginHookKind::ToolExecuteBefore
                | PluginHookKind::ToolExecuteAfter
                | PluginHookKind::ToolExecuteFailure
                | PluginHookKind::ToolDefinition
        )
    {
        return Err(syn::Error::new_spanned(
            &filters.tools[0],
            "`tool`/`tools(...)` filters are only valid for tool hooks",
        ));
    }
    if !filters.commands.is_empty()
        && !matches!(
            hook,
            PluginHookKind::CommandExecuteBefore | PluginHookKind::CommandExecuteAfter
        )
    {
        return Err(syn::Error::new_spanned(
            &filters.commands[0],
            "`command`/`commands(...)` filters are only valid for shell.before/shell.after hooks",
        ));
    }
    if !filters.plugins.is_empty()
        && !matches!(
            hook,
            PluginHookKind::ToolExecuteBefore
                | PluginHookKind::ToolExecuteAfter
                | PluginHookKind::ToolExecuteFailure
                | PluginHookKind::ToolDefinition
        )
    {
        return Err(syn::Error::new_spanned(
            &filters.plugins[0].value,
            "`plugin`/`plugins(...)` filters are only valid for tool hooks",
        ));
    }
    if !filters.tags.is_empty() && !matches!(hook, PluginHookKind::ToolExecuteBefore) {
        return Err(syn::Error::new_spanned(
            &filters.tags[0],
            "`tag`/`tags(...)` filters are only valid for tool.before hooks",
        ));
    }
    Ok(())
}

pub enum PluginHookExpectedInput {
    None,
    Single(&'static str),
    Init,
}

pub enum PluginHookExpectedOutput {
    Unit,
    Init,
    Option(&'static str),
}

pub fn plugin_hook_input_segment(hook: PluginHookKind) -> PluginHookExpectedInput {
    match hook {
        PluginHookKind::Shutdown => PluginHookExpectedInput::None,
        PluginHookKind::Init => PluginHookExpectedInput::Init,
        PluginHookKind::ToolExecuteBefore => PluginHookExpectedInput::Single("ToolBeforeInput"),
        PluginHookKind::ToolExecuteAfter => PluginHookExpectedInput::Single("ToolAfterInput"),
        PluginHookKind::ToolExecuteFailure => PluginHookExpectedInput::Single("ToolFailureInput"),
        PluginHookKind::ToolDefinition => PluginHookExpectedInput::Single("ToolDefinitionInput"),
        PluginHookKind::ChatMessage => PluginHookExpectedInput::Single("ChatMessageInput"),
        PluginHookKind::ChatParams => PluginHookExpectedInput::Single("ChatParamsInput"),
        PluginHookKind::ChatHeaders => PluginHookExpectedInput::Single("ChatHeadersInput"),
        PluginHookKind::ChatSystemTransform => {
            PluginHookExpectedInput::Single("ChatSystemTransformInput")
        }
        PluginHookKind::ChatMessagesTransform => {
            PluginHookExpectedInput::Single("ChatMessagesTransformInput")
        }
        PluginHookKind::Event => PluginHookExpectedInput::Single("EventEnvelope"),
        PluginHookKind::Auth => PluginHookExpectedInput::Single("AuthInput"),
        PluginHookKind::ProviderList => PluginHookExpectedInput::Single("ProviderListInput"),
        PluginHookKind::PermissionAsk => PluginHookExpectedInput::Single("PermissionAskInput"),
        PluginHookKind::Notification => PluginHookExpectedInput::Single("NotificationInput"),
        PluginHookKind::CommandExecuteBefore => {
            PluginHookExpectedInput::Single("CommandBeforeInput")
        }
        PluginHookKind::CommandExecuteAfter => PluginHookExpectedInput::Single("CommandAfterInput"),
        PluginHookKind::ShellEnv => PluginHookExpectedInput::Single("ShellEnvInput"),
        PluginHookKind::PreRun => PluginHookExpectedInput::Single("PreRunInput"),
        PluginHookKind::PostRun => PluginHookExpectedInput::Single("PostRunInput"),
        PluginHookKind::SessionStart => PluginHookExpectedInput::Single("SessionStartInput"),
        PluginHookKind::SessionEnd => PluginHookExpectedInput::Single("SessionEndInput"),
        PluginHookKind::UserPromptSubmit => {
            PluginHookExpectedInput::Single("UserPromptSubmitInput")
        }
        PluginHookKind::AgentStop => PluginHookExpectedInput::Single("AgentStopInput"),
        PluginHookKind::ConfigResolved => PluginHookExpectedInput::Single("ConfigInput"),
    }
}

pub fn plugin_hook_output_segment(hook: PluginHookKind) -> PluginHookExpectedOutput {
    match hook {
        PluginHookKind::Init => PluginHookExpectedOutput::Init,
        PluginHookKind::Shutdown
        | PluginHookKind::ToolExecuteFailure
        | PluginHookKind::Event
        | PluginHookKind::Notification
        | PluginHookKind::PreRun
        | PluginHookKind::PostRun
        | PluginHookKind::SessionEnd => PluginHookExpectedOutput::Unit,
        PluginHookKind::ToolExecuteBefore => PluginHookExpectedOutput::Option("ToolBeforePatch"),
        PluginHookKind::ToolExecuteAfter => PluginHookExpectedOutput::Option("ToolAfterPatch"),
        PluginHookKind::ToolDefinition => PluginHookExpectedOutput::Option("ToolDefinitionPatch"),
        PluginHookKind::ChatMessage => PluginHookExpectedOutput::Option("ChatMessagePatch"),
        PluginHookKind::ChatParams => PluginHookExpectedOutput::Option("ChatParamsPatch"),
        PluginHookKind::ChatHeaders => PluginHookExpectedOutput::Option("ChatHeadersPatch"),
        PluginHookKind::ChatSystemTransform => {
            PluginHookExpectedOutput::Option("ChatSystemTransformPatch")
        }
        PluginHookKind::ChatMessagesTransform => {
            PluginHookExpectedOutput::Option("ChatMessagesTransformPatch")
        }
        PluginHookKind::Auth => PluginHookExpectedOutput::Option("AuthOutput"),
        PluginHookKind::ProviderList => PluginHookExpectedOutput::Option("ProviderListPatch"),
        PluginHookKind::PermissionAsk => PluginHookExpectedOutput::Option("PermissionAskDecision"),
        PluginHookKind::CommandExecuteBefore => {
            PluginHookExpectedOutput::Option("CommandBeforeResponse")
        }
        PluginHookKind::CommandExecuteAfter => {
            PluginHookExpectedOutput::Option("CommandAfterPatch")
        }
        PluginHookKind::ShellEnv => PluginHookExpectedOutput::Option("ShellEnvPatch"),
        PluginHookKind::SessionStart => PluginHookExpectedOutput::Option("SessionStartPatch"),
        PluginHookKind::UserPromptSubmit => {
            PluginHookExpectedOutput::Option("UserPromptSubmitPatch")
        }
        PluginHookKind::AgentStop => PluginHookExpectedOutput::Option("AgentStopPatch"),
        PluginHookKind::ConfigResolved => PluginHookExpectedOutput::Option("ConfigPatch"),
    }
}

pub fn validate_plugin_hook_output(method: &ImplItemFn, hook: PluginHookKind) -> Result<()> {
    let hook_name = plugin_hook_name(hook);
    match plugin_hook_output_segment(hook) {
        PluginHookExpectedOutput::Unit => {
            let Some((ty, _)) = plugin_method_return_value_type(method) else {
                return Ok(());
            };
            if type_is_unit(&ty) {
                return Ok(());
            }
            Err(syn::Error::new_spanned(
                plugin_method_return_type(method).unwrap_or(&ty),
                format!("#[hook({hook_name})] must return `()` or `Result<()>`"),
            ))
        }
        PluginHookExpectedOutput::Init => {
            let Some((ty, returns_result)) = plugin_method_return_value_type(method) else {
                return Err(syn::Error::new_spanned(
                    &method.sig,
                    "#[hook(init)] must return `Result<InitOutcome>`",
                ));
            };
            if returns_result && type_last_segment_is(&ty, "InitOutcome") {
                return Ok(());
            }
            Err(syn::Error::new_spanned(
                plugin_method_return_type(method).unwrap_or(&ty),
                format!(
                    "#[hook({hook_name})] must return `Result<InitOutcome>`, got `{}`",
                    type_display(plugin_method_return_type(method).unwrap_or(&ty))
                ),
            ))
        }
        PluginHookExpectedOutput::Option(expected) => {
            let Some((ty, _)) = plugin_method_return_value_type(method) else {
                return Err(syn::Error::new_spanned(
                    &method.sig,
                    format!(
                        "#[hook({hook_name})] must return `{expected}`, `Option<{expected}>`, or `Result<...>`"
                    ),
                ));
            };
            if type_last_segment_is(&ty, expected)
                || option_inner_type(&ty).is_some_and(|inner| type_last_segment_is(inner, expected))
            {
                return Ok(());
            }
            Err(syn::Error::new_spanned(
                plugin_method_return_type(method).unwrap_or(&ty),
                format!(
                    "#[hook({hook_name})] must return `{expected}`, `Option<{expected}>`, or `Result<...>`, got `{}`",
                    type_display(plugin_method_return_type(method).unwrap_or(&ty))
                ),
            ))
        }
    }
}

fn option_inner_type(ty: &Type) -> Option<&Type> {
    let Type::Path(path) = ty else {
        return None;
    };
    let segment = path.path.segments.last()?;
    if segment.ident != "Option" {
        return None;
    }
    let PathArguments::AngleBracketed(args) = &segment.arguments else {
        return None;
    };
    args.args.iter().find_map(|arg| match arg {
        syn::GenericArgument::Type(ty) => Some(ty),
        _ => None,
    })
}

pub fn plugin_hook_name(hook: PluginHookKind) -> &'static str {
    match hook {
        PluginHookKind::Init => "init",
        PluginHookKind::Shutdown => "shutdown",
        PluginHookKind::ToolExecuteBefore => "tool.before",
        PluginHookKind::ToolExecuteAfter => "tool.after",
        PluginHookKind::ToolExecuteFailure => "tool.failure",
        PluginHookKind::ToolDefinition => "tool.definition",
        PluginHookKind::ChatMessage => "chat.message",
        PluginHookKind::ChatParams => "chat.params",
        PluginHookKind::ChatHeaders => "chat.headers",
        PluginHookKind::ChatSystemTransform => "chat.system",
        PluginHookKind::ChatMessagesTransform => "chat.messages",
        PluginHookKind::Event => "event",
        PluginHookKind::Auth => "auth",
        PluginHookKind::ProviderList => "provider.list",
        PluginHookKind::PermissionAsk => "permission.ask",
        PluginHookKind::Notification => "notification",
        PluginHookKind::CommandExecuteBefore => "shell.before",
        PluginHookKind::CommandExecuteAfter => "shell.after",
        PluginHookKind::ShellEnv => "shell.env",
        PluginHookKind::PreRun => "run.pre",
        PluginHookKind::PostRun => "run.post",
        PluginHookKind::SessionStart => "session.start",
        PluginHookKind::SessionEnd => "session.end",
        PluginHookKind::UserPromptSubmit => "prompt.submit",
        PluginHookKind::AgentStop => "agent.stop",
        PluginHookKind::ConfigResolved => "config.resolved",
    }
}
