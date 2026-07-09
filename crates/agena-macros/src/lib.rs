use std::collections::{BTreeMap, BTreeSet};

use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::ext::IdentExt;
use syn::parse::Parser;
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::{
    Attribute, Data, DeriveInput, Expr, ExprLit, ExprPath, Field, Fields, FnArg, Ident, ImplItem,
    ImplItemFn, Index, ItemImpl, Lit, LitBool, LitStr, Member, Meta, Pat, Path, PathArguments,
    Result, Token, Type, Variant, parse_macro_input, parse_quote,
};

#[proc_macro_attribute]
pub fn agena_plugin(attr: TokenStream, item: TokenStream) -> TokenStream {
    let attr = parse_macro_input!(attr as proc_macro2::TokenStream);
    let item = parse_macro_input!(item as ItemImpl);
    match expand_plugin_impl_attr(attr, item) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

#[proc_macro_derive(ToolInput, attributes(input, arg))]
pub fn derive_input(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match expand_input(input) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

#[proc_macro_derive(PluginConfigStore, attributes(config, plugin_config))]
pub fn derive_plugin_config_store(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match expand_plugin_config_store(input) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

fn expand_plugin_impl_attr(
    attr: proc_macro2::TokenStream,
    item: ItemImpl,
) -> Result<proc_macro2::TokenStream> {
    if item.trait_.is_some() {
        return Err(syn::Error::new_spanned(
            &item.self_ty,
            "#[agena_plugin(...)] only supports inherent impl blocks; write `#[async_trait] impl Plugin for Type` manually for dynamic plugins",
        ));
    }

    if attr.is_empty() {
        return Err(syn::Error::new_spanned(
            &item.self_ty,
            "#[agena_plugin(...)] inherent impls require id/version/summary metadata",
        ));
    }

    expand_plugin_inherent_impl_attr(attr, item)
}

fn expand_plugin_config_store(input: DeriveInput) -> Result<proc_macro2::TokenStream> {
    let name = input.ident;
    let config_field = find_plugin_config_store_field(&input.data)?;
    let field_member = config_field.member;
    let config_ty = config_field.config_ty;
    let schema_expr = match config_field.default {
        PluginConfigDefault::None => {
            quote! { ::agena_plugin_sdk::macro_support::json_schema_for::<#config_ty>() }
        }
        PluginConfigDefault::Default => {
            quote! {
                ::agena_plugin_sdk::macro_support::json_schema_for_default(
                    <#config_ty as ::core::default::Default>::default(),
                )
            }
        }
        PluginConfigDefault::Expr(default) => {
            quote! { ::agena_plugin_sdk::macro_support::json_schema_for_default(#default) }
        }
    };
    let generics = input.generics;
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

    Ok(quote! {
        impl #impl_generics ::agena_plugin_sdk::plugin::PluginConfigStoreAccess for #name #ty_generics #where_clause {
            fn plugin_config_schema() -> ::agena_plugin_sdk::serde_json::Value {
                #schema_expr
            }

            fn set_plugin_config_from_json(
                &self,
                input: ::agena_plugin_sdk::serde_json::Value,
                invalid: &str,
                already: ::std::string::String,
            ) -> ::agena_plugin_sdk::Result<()> {
                self.#field_member.set_from_json(input, invalid, already)
            }
        }
    })
}

struct PluginConfigStoreField {
    member: Member,
    config_ty: Type,
    default: PluginConfigDefault,
}

enum PluginConfigDefault {
    None,
    Default,
    Expr(Expr),
}

fn find_plugin_config_store_field(data: &Data) -> Result<PluginConfigStoreField> {
    let Data::Struct(data_struct) = data else {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            "PluginConfigStore can only be derived for structs",
        ));
    };

    let mut found = None;
    for (index, field) in data_struct.fields.iter().enumerate() {
        let config_attr = parse_plugin_config_store_field_attrs(field)?;
        if config_attr.is_none() {
            continue;
        }
        if found.is_some() {
            return Err(syn::Error::new_spanned(
                field,
                "PluginConfigStore supports exactly one #[config] field",
            ));
        }
        let member = match &field.ident {
            Some(ident) => Member::Named(ident.clone()),
            None => Member::Unnamed(Index::from(index)),
        };
        let config_ty = plugin_config_inner_type(field)?;
        found = Some(PluginConfigStoreField {
            member,
            config_ty,
            default: config_attr.expect("config attr checked as present").default,
        });
    }

    found.ok_or_else(|| {
        syn::Error::new(
            proc_macro2::Span::call_site(),
            "PluginConfigStore requires one field marked #[config] or #[plugin_config]",
        )
    })
}

fn is_plugin_config_store_attr(attr: &Attribute) -> bool {
    attr.path().is_ident("config") || attr.path().is_ident("plugin_config")
}

struct PluginConfigFieldAttr {
    default: PluginConfigDefault,
}

fn parse_plugin_config_store_field_attrs(field: &Field) -> Result<Option<PluginConfigFieldAttr>> {
    let mut found = false;
    let mut default = PluginConfigDefault::None;
    for attr in &field.attrs {
        if !is_plugin_config_store_attr(attr) {
            continue;
        }
        found = true;
        let attr_default = parse_plugin_config_store_field_attr(attr)?;
        match (&default, attr_default) {
            (PluginConfigDefault::None, next) => default = next,
            (_, PluginConfigDefault::None) => {}
            (_, _) => {
                return Err(syn::Error::new_spanned(
                    attr,
                    "duplicate #[config] default option",
                ));
            }
        }
    }

    Ok(found.then_some(PluginConfigFieldAttr { default }))
}

fn parse_plugin_config_store_field_attr(attr: &Attribute) -> Result<PluginConfigDefault> {
    match &attr.meta {
        Meta::Path(_) => Ok(PluginConfigDefault::None),
        Meta::NameValue(_) => Err(syn::Error::new_spanned(
            attr,
            "#[config] only supports `#[config]`, `#[config(default)]`, or `#[config(default = expr)]`",
        )),
        Meta::List(_) => attr.parse_args::<PluginConfigDefault>(),
    }
}

impl Parse for PluginConfigDefault {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        if input.is_empty() {
            return Ok(Self::None);
        }
        let key = input.call(Ident::parse_any)?;
        if key != "default" {
            return Err(syn::Error::new_spanned(
                key,
                "unsupported #[config] option; expected `default`",
            ));
        }
        if input.peek(Token![=]) {
            let _: Token![=] = input.parse()?;
            let default = input.parse::<Expr>()?;
            if !input.is_empty() {
                return Err(input.error("unexpected trailing tokens in #[config]"));
            }
            return Ok(Self::Expr(default));
        }
        if !input.is_empty() {
            return Err(input.error("unexpected trailing tokens in #[config]"));
        }
        Ok(Self::Default)
    }
}

fn plugin_config_inner_type(field: &Field) -> Result<Type> {
    let Type::Path(path) = &field.ty else {
        return Err(syn::Error::new_spanned(
            &field.ty,
            "#[config] fields must have type PluginConfig<T>",
        ));
    };
    let Some(segment) = path.path.segments.last() else {
        return Err(syn::Error::new_spanned(
            &field.ty,
            "#[config] fields must have type PluginConfig<T>",
        ));
    };
    if segment.ident != "PluginConfig" {
        return Err(syn::Error::new_spanned(
            &field.ty,
            "#[config] fields must have type PluginConfig<T>",
        ));
    }
    let PathArguments::AngleBracketed(args) = &segment.arguments else {
        return Err(syn::Error::new_spanned(
            &field.ty,
            "#[config] fields must specify PluginConfig<T>",
        ));
    };
    let mut types = args.args.iter().filter_map(|arg| match arg {
        syn::GenericArgument::Type(ty) => Some(ty.clone()),
        _ => None,
    });
    let config_ty = types.next().ok_or_else(|| {
        syn::Error::new_spanned(&field.ty, "#[config] fields must specify PluginConfig<T>")
    })?;
    if types.next().is_some() {
        return Err(syn::Error::new_spanned(
            &field.ty,
            "#[config] fields must specify exactly one PluginConfig<T> type",
        ));
    }
    Ok(config_ty)
}

struct PluginImplConfig {
    namespace: Option<Expr>,
    name: Option<Expr>,
    version: Option<Expr>,
    summary: Option<Expr>,
    help: Option<Expr>,
    config_schema: Option<Expr>,
    config_schema_type: Option<Type>,
    config_schema_default: Option<Expr>,
    config_schema_store: bool,
    config_field: Option<Ident>,
    config_store: bool,
    display: Option<Ident>,
    ui_display: Option<Ident>,
    tool_description_mode: Option<Expr>,
    ui_display_mode: Option<Expr>,
    plugin_capabilities_expr: Option<Expr>,
    plugin_capabilities: Vec<Expr>,
    export: Option<Ident>,
    export_bind: Option<Expr>,
}

#[derive(Clone)]
struct PluginToolPlan {
    tool: LitStr,
    input_model: PluginGeneratedToolInput,
    invoke: PluginToolInvokeHandler,
    stream: Option<PluginToolStreamHandler>,
    permissions: PluginToolPermissionHandlers,
    command: Option<PluginToolCommandConfig>,
}

#[derive(Clone)]
struct PluginToolInvokeHandler {
    method: Ident,
    output: PluginToolOutputPlan,
    is_async: bool,
    context: Option<PluginContextArg>,
    input: PluginCallInput,
}

#[derive(Clone)]
struct PluginToolOutputPlan {
    ty: Option<Type>,
    returns_result: bool,
}

#[derive(Clone)]
struct PluginToolStreamHandler {
    method: Ident,
    is_async: bool,
    sink_first: bool,
    context: Option<PluginContextArg>,
    input: PluginCallInput,
}

#[derive(Clone)]
struct PluginToolStreamSignature {
    method: Ident,
    is_async: bool,
    sink_first: bool,
}

#[derive(Clone, Default)]
struct PluginToolPermissionHandlers {
    path_rules: Vec<PluginToolPathPermissionRule>,
    network_rules: Vec<PluginToolNetworkPermissionRule>,
}

#[derive(Clone)]
enum PluginToolPathPermissionRule {
    Read(Expr),
    Reads(Expr),
    Write(Expr),
    Writes(Expr),
    Requests(Expr),
}

#[derive(Clone)]
enum PluginToolNetworkPermissionRule {
    Connect(Expr),
    Connects(Expr),
    Requests(Expr),
}

#[derive(Clone, Copy)]
struct PluginContextArg {
    first: bool,
    by_ref: bool,
}

#[derive(Clone)]
enum PluginCallInput {
    Wrapped { by_ref: bool },
    Fields(Vec<Ident>),
}

#[derive(Clone)]
struct PluginGeneratedToolInput {
    input_ident: Option<Ident>,
    input_fields: Vec<PluginGeneratedInputField>,
    input_ty: Type,
    spec: ToolSpecConfig,
    docs: Option<String>,
}

#[derive(Clone)]
struct PluginGeneratedInputField {
    ident: Ident,
    wire_name: LitStr,
    aliases: Vec<LitStr>,
    ty: Type,
    default: bool,
    default_expr: Option<Expr>,
    flatten_shape: bool,
    nested_shape: bool,
}

#[derive(Clone)]
struct PluginHookPlan {
    method: Ident,
    hook: PluginHookKind,
    is_async: bool,
    priority: i32,
    filters: PluginHookFilters,
}

#[derive(Clone)]
struct PluginCommandPlan {
    id: LitStr,
    title: LitStr,
    description: LitStr,
    category: LitStr,
    slash: Option<LitStr>,
    aliases: Vec<LitStr>,
    usage: Option<LitStr>,
    location: LitStr,
    action: Option<Expr>,
    handler: PluginCommandHandlerPlan,
}

#[derive(Clone)]
enum PluginCommandHandlerPlan {
    Method {
        method: Ident,
        input: PluginCommandInputPlan,
        context: Option<PluginContextArg>,
        is_async: bool,
    },
    InvokeTool {
        tool: LitStr,
        input_model: PluginGeneratedToolInput,
        submit_output_as_prompt: bool,
    },
}

#[derive(Clone)]
enum PluginCommandInputPlan {
    None,
    Raw {
        by_ref: bool,
    },
    Typed {
        ty: Type,
        by_ref: bool,
    },
    Generated {
        input_model: PluginGeneratedToolInput,
        input: PluginCallInput,
    },
}

#[derive(Clone)]
struct PluginCommandMethodShape {
    input: PluginCommandInputPlan,
    context: Option<PluginContextArg>,
}

#[derive(Clone, Default)]
struct PluginToolCommandConfig {
    id: Option<LitStr>,
    title: Option<LitStr>,
    description: Option<LitStr>,
    category: Option<LitStr>,
    slash: Option<LitStr>,
    aliases: Vec<LitStr>,
    usage: Option<LitStr>,
    location: Option<LitStr>,
    submit_output_as_prompt: bool,
}

#[derive(Clone, Default)]
struct PluginHookFilters {
    tools: Vec<LitStr>,
    commands: Vec<LitStr>,
    plugins: Vec<PluginHookPluginFilter>,
    tags: Vec<LitStr>,
}

#[derive(Clone)]
struct PluginHookPluginFilter {
    value: LitStr,
    namespace: LitStr,
    name: LitStr,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PluginHookKind {
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
    PermissionAsk,
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

fn expand_plugin_inherent_impl_attr(
    attr: proc_macro2::TokenStream,
    mut item: ItemImpl,
) -> Result<proc_macro2::TokenStream> {
    let config = parse_plugin_impl_config(attr)?;
    let docs = doc_text(&item.attrs);
    let self_ty = item.self_ty.as_ref().clone();
    let self_label = plugin_self_type_label(&self_ty);
    let method_infos = plugin_impl_method_infos(&item);
    let mut tool_plans = Vec::new();
    let mut hook_bindings = Vec::new();
    let mut command_plans = Vec::new();

    for impl_item in &mut item.items {
        let ImplItem::Fn(method) = impl_item else {
            continue;
        };
        let attrs = parse_plugin_inherent_method_attrs(method, &self_label, &method_infos)?;
        tool_plans.extend(attrs.tools);
        hook_bindings.extend(attrs.hooks);
        command_plans.extend(attrs.commands);
    }

    if (!tool_plans.is_empty() || !command_plans.is_empty()) && !item.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(
            &item.generics,
            "method-level #[tool(...)]/#[command(...)] generation does not support generic plugin impls yet; use a non-generic plugin wrapper type",
        ));
    }
    command_plans.extend(
        tool_plans
            .iter()
            .filter_map(build_tool_command_plan)
            .collect::<Result<Vec<_>>>()?,
    );
    reject_duplicate_tool_plans(&tool_plans)?;
    reject_duplicate_init_hooks(&hook_bindings)?;
    reject_duplicate_command_plans(&command_plans)?;
    let generated_input_items = tool_plans
        .iter()
        .map(|tool| expand_plugin_generated_input(&tool.input_model))
        .chain(
            command_plans
                .iter()
                .filter_map(command_generated_input_model)
                .map(expand_plugin_generated_input),
        )
        .collect::<Result<Vec<_>>>()?;

    let manifest_method = expand_plugin_layer_manifest(
        &config,
        &self_ty,
        item.generics.params.is_empty(),
        docs.as_deref(),
        &tool_plans,
        &hook_bindings,
        &command_plans,
    )?;
    let tool_invoke_method = (!tool_plans.is_empty())
        .then(|| expand_plugin_layer_tool_invoke(&self_ty, &tool_plans))
        .transpose()?;
    let stream_method = tool_plans
        .iter()
        .any(|tool| tool.stream.is_some())
        .then(|| expand_plugin_layer_tool_stream(&self_ty, &tool_plans))
        .transpose()?;
    let permission_paths_method = tool_plans
        .iter()
        .any(|tool| tool.permissions.has_path_permissions())
        .then(|| expand_plugin_layer_permission_paths(&self_ty, &tool_plans))
        .transpose()?;
    let permission_networks_method = tool_plans
        .iter()
        .any(|tool| tool.permissions.has_network_permissions())
        .then(|| expand_plugin_layer_permission_networks(&self_ty, &tool_plans))
        .transpose()?;
    let command_invoke_method = (!command_plans.is_empty())
        .then(|| expand_plugin_layer_command_invoke(&self_ty, &command_plans))
        .transpose()?;
    let init_binding = hook_bindings
        .iter()
        .find(|binding| binding.hook == PluginHookKind::Init);
    let init_method =
        (config.config_field.is_some() || config.config_store || init_binding.is_some())
            .then(|| expand_plugin_layer_init_method(&config, &self_ty, init_binding))
            .transpose()?;
    let hook_methods = expand_plugin_layer_hook_methods(&self_ty, &hook_bindings)?;
    let generics = &item.generics;
    let export = expand_plugin_layer_export(&config, &self_ty, generics)?;
    let (impl_generics, _ty_generics, where_clause) = generics.split_for_impl();
    Ok(quote! {
        #item

        #(#generated_input_items)*

        #[::agena_plugin_sdk::async_trait]
        impl #impl_generics ::agena_plugin_sdk::Plugin for #self_ty #where_clause {
            #manifest_method
            #tool_invoke_method
            #stream_method
            #permission_paths_method
            #permission_networks_method
            #command_invoke_method
            #init_method
            #(#hook_methods)*
        }

        #export
    })
}

fn parse_plugin_impl_config(attr: proc_macro2::TokenStream) -> Result<PluginImplConfig> {
    let metas = Punctuated::<Meta, Token![,]>::parse_terminated.parse2(attr)?;
    let mut namespace = None;
    let mut name = None;
    let mut version = None;
    let mut summary = None;
    let mut help = None;
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

fn parse_type_list(tokens: proc_macro2::TokenStream, label: &str) -> Result<Type> {
    syn::parse2::<Type>(tokens).map_err(|err| {
        syn::Error::new(
            err.span(),
            format!("{label} expects a single type, such as `{label}(Vec<Item>)`"),
        )
    })
}

fn expr_path_ident(expr: Expr, label: &str) -> Result<Ident> {
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

fn plugin_self_type_label(ty: &Type) -> String {
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

fn sanitize_generated_ident_label(value: &str) -> String {
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

fn plugin_impl_method_infos(item: &ItemImpl) -> Vec<PluginMethodInfo> {
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

struct PluginInherentMethodAttrs {
    tools: Vec<PluginToolPlan>,
    hooks: Vec<PluginHookPlan>,
    commands: Vec<PluginCommandPlan>,
}

fn parse_plugin_inherent_method_attrs(
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

fn build_plugin_tool_plan(
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

struct PluginCommandAttrArgs {
    slash: Option<LitStr>,
    metas: Vec<Meta>,
}

impl Parse for PluginCommandAttrArgs {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let mut slash = None;
        if input.peek(LitStr) {
            slash = Some(input.parse()?);
            if input.peek(Token![,]) {
                input.parse::<Token![,]>()?;
            } else if !input.is_empty() {
                return Err(input.error("expected `,` after command slash shorthand"));
            }
        }

        let mut metas = Vec::new();
        while !input.is_empty() {
            metas.push(input.parse()?);
            if input.peek(Token![,]) {
                input.parse::<Token![,]>()?;
            } else if !input.is_empty() {
                return Err(input.error("expected `,` between command arguments"));
            }
        }

        Ok(Self { slash, metas })
    }
}

fn build_plugin_command_plan(
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

fn build_plugin_command_input_plan(
    method: &mut ImplItemFn,
    method_ident: &Ident,
    self_label: &str,
    docs: Option<String>,
) -> Result<PluginCommandMethodShape> {
    let input_ident = format_ident!("__AgenaPluginCommandInput_{}_{}", self_label, method_ident);
    let args = plugin_method_value_args(method)?;
    if let Some(context_arg) = args.iter().find(|arg| arg.is_context) {
        return Err(syn::Error::new_spanned(
            &context_arg.ty,
            "#[command] methods do not support ToolInvokeContext; use PluginCommandInvokeInput for raw command context",
        ));
    }
    let context = plugin_command_context_arg(&args)?;
    let input_args = args
        .into_iter()
        .filter(|arg| !type_is_plugin_command_context(&arg.ty))
        .collect::<Vec<_>>();
    let input = match input_args.as_slice() {
        [] => PluginCommandInputPlan::None,
        [arg] if !arg.has_arg_config => {
            let by_ref = arg.by_ref;
            let owned_ty = arg.inner_ty.clone();
            if type_last_segment_is(&owned_ty, "PluginCommandInvokeInput") {
                if context.is_some() {
                    return Err(syn::Error::new_spanned(
                        &arg.ty,
                        "PluginCommandInvokeInput already exposes raw command metadata; do not combine it with PluginCommandContext",
                    ));
                }
                PluginCommandInputPlan::Raw { by_ref }
            } else {
                PluginCommandInputPlan::Typed {
                    ty: owned_ty,
                    by_ref,
                }
            }
        }
        args => {
            let mut spec = empty_tool_spec_config();
            let mut fields = Vec::new();
            let mut call_fields = Vec::new();
            let prepared_args = prepare_inline_args(args)?;
            let field_path_lookup = inline_arg_constraint_path_lookup(&prepared_args)?;
            let array_field_paths = prepared_args
                .iter()
                .filter(|prepared| input_type_semantic_shape(&prepared.arg.ty).array)
                .map(|prepared| prepared.field_name.value())
                .collect::<BTreeSet<_>>();
            for prepared in prepared_args {
                let arg = prepared.arg;
                validate_inline_shape_wrapper_arg(arg)?;
                if type_last_segment_is(&arg.inner_ty, "PluginCommandInvokeInput") {
                    return Err(syn::Error::new_spanned(
                        &arg.ty,
                        "PluginCommandInvokeInput is only supported as the sole #[command] argument; use a typed input struct or inline #[arg(...)] fields for structured command inputs",
                    ));
                }
                if arg.by_ref {
                    return Err(syn::Error::new_spanned(
                        &arg.ty,
                        "field-style #[command] arguments must be owned values; use a single input struct argument if the handler wants a reference",
                    ));
                }
                let field_name = prepared.field_name;
                let aliases = prepared.aliases;
                apply_arg_config_to_spec(
                    &mut spec,
                    &field_name,
                    &aliases,
                    &arg.ty,
                    Some(&field_path_lookup),
                    &arg.config,
                );
                fields.push(PluginGeneratedInputField {
                    ident: arg.ident.clone(),
                    wire_name: field_name,
                    aliases,
                    ty: arg.ty.clone(),
                    default: arg.config.default,
                    default_expr: arg.config.default_expr.clone(),
                    flatten_shape: arg.config.flatten_shape,
                    nested_shape: arg.config.nested_shape,
                });
                call_fields.push(arg.ident.clone());
            }
            normalize_array_value_constraints(
                &mut spec.trim,
                &mut spec.trim_suffix,
                &mut spec.minimums,
                &mut spec.maximums,
                &mut spec.exclusive_minimums,
                &mut spec.exclusive_maximums,
                &mut spec.min_properties,
                &mut spec.max_properties,
                &mut spec.min_chars,
                &mut spec.max_chars,
                &mut spec.formats,
                &mut spec.patterns,
                &mut spec.choices,
                &mut spec.forbid_substrings,
                &mut spec.distinct_trimmed,
                &mut spec.input_field_metadata,
                &field_path_lookup,
                &array_field_paths,
            );
            PluginCommandInputPlan::Generated {
                input_model: PluginGeneratedToolInput {
                    input_ident: Some(input_ident.clone()),
                    input_fields: fields,
                    input_ty: parse_quote!(#input_ident),
                    spec,
                    docs,
                },
                input: PluginCallInput::Fields(call_fields),
            }
        }
    };
    Ok(PluginCommandMethodShape { input, context })
}

fn build_tool_command_plan(tool: &PluginToolPlan) -> Option<Result<PluginCommandPlan>> {
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
            input_model: tool.input_model.clone(),
            submit_output_as_prompt: config.submit_output_as_prompt,
        },
    }))
}

fn command_generated_input_model(command: &PluginCommandPlan) -> Option<&PluginGeneratedToolInput> {
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

fn expand_plugin_command_usage_expr(
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
    shape: &PluginToolMethodShape,
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

impl PluginToolPermissionHandlers {
    fn has_path_permissions(&self) -> bool {
        !self.path_rules.is_empty()
    }

    fn has_network_permissions(&self) -> bool {
        !self.network_rules.is_empty()
    }
}

fn plugin_attr_has_explicit_args(attr: &Attribute) -> bool {
    match &attr.meta {
        Meta::Path(_) => false,
        Meta::List(list) => !list.tokens.is_empty(),
        Meta::NameValue(_) => true,
    }
}

struct PluginToolAttrConfig {
    spec: ToolSpecConfig,
    stream_method: Option<Ident>,
    permission_path_rules: Vec<PluginToolPathPermissionRule>,
    permission_network_rules: Vec<PluginToolNetworkPermissionRule>,
    command: Option<PluginToolCommandConfig>,
}

struct PluginToolMethodShape {
    input_model: PluginGeneratedToolInput,
    context: Option<PluginContextArg>,
    call_input: PluginCallInput,
    stream_arg_types: Vec<Type>,
    stream_method: Option<Ident>,
}

struct PluginMethodInfo {
    ident: Ident,
    is_async: bool,
    typed_args: Vec<Type>,
    shared_receiver: bool,
}

#[derive(Default)]
struct PluginArgConfig {
    default: bool,
    default_expr: Option<Expr>,
    description: Option<LitStr>,
    trim: bool,
    item_trim: bool,
    non_empty: bool,
    item_non_empty: bool,
    non_empty_if_present: bool,
    item_non_empty_if_present: bool,
    distinct_trimmed: bool,
    trim_suffix: Option<LitStr>,
    item_trim_suffix: Option<LitStr>,
    minimum: Option<Expr>,
    maximum: Option<Expr>,
    exclusive_minimum: Option<Expr>,
    exclusive_maximum: Option<Expr>,
    min_items: Option<usize>,
    max_items: Option<usize>,
    min_properties: Option<usize>,
    max_properties: Option<usize>,
    item_minimum: Option<Expr>,
    item_maximum: Option<Expr>,
    item_exclusive_minimum: Option<Expr>,
    item_exclusive_maximum: Option<Expr>,
    item_min_properties: Option<usize>,
    item_max_properties: Option<usize>,
    min_chars: Option<usize>,
    max_chars: Option<usize>,
    item_min_chars: Option<usize>,
    item_max_chars: Option<usize>,
    format: Option<LitStr>,
    item_format: Option<LitStr>,
    pattern: Option<LitStr>,
    item_pattern: Option<LitStr>,
    choices: Option<Vec<Expr>>,
    item_choices: Option<Vec<Expr>>,
    exactly_one_of: Vec<LitStr>,
    at_least_one_of: Vec<LitStr>,
    requires: Vec<LitStr>,
    conflicts_with: Vec<LitStr>,
    required_unless_present: Vec<LitStr>,
    forbid_substrings: Vec<LitStr>,
    distinct_trimmed_within: Vec<LitStr>,
    path: Option<PluginPathPermissionKind>,
    network: Option<PluginNetworkSemantic>,
    optional: bool,
    flatten_shape: bool,
    nested_shape: bool,
    jsonpath: Option<LitStr>,
    fallback: Option<LitStr>,
    name: Option<LitStr>,
    aliases: Vec<LitStr>,
    example: Option<Expr>,
    secret: bool,
    picker: Option<PluginPickerKind>,
}

#[derive(Default)]
struct FieldArgConfig {
    default: bool,
    default_expr: Option<Expr>,
    description: Option<LitStr>,
    name: Option<LitStr>,
    aliases: Vec<LitStr>,
    trim: bool,
    item_trim: bool,
    non_empty: bool,
    item_non_empty: bool,
    non_empty_if_present: bool,
    item_non_empty_if_present: bool,
    distinct_trimmed: bool,
    trim_suffix: Option<LitStr>,
    item_trim_suffix: Option<LitStr>,
    minimum: Option<Expr>,
    maximum: Option<Expr>,
    exclusive_minimum: Option<Expr>,
    exclusive_maximum: Option<Expr>,
    min_items: Option<usize>,
    max_items: Option<usize>,
    min_properties: Option<usize>,
    max_properties: Option<usize>,
    item_minimum: Option<Expr>,
    item_maximum: Option<Expr>,
    item_exclusive_minimum: Option<Expr>,
    item_exclusive_maximum: Option<Expr>,
    item_min_properties: Option<usize>,
    item_max_properties: Option<usize>,
    min_chars: Option<usize>,
    max_chars: Option<usize>,
    item_min_chars: Option<usize>,
    item_max_chars: Option<usize>,
    format: Option<LitStr>,
    item_format: Option<LitStr>,
    pattern: Option<LitStr>,
    item_pattern: Option<LitStr>,
    choices: Option<Vec<Expr>>,
    item_choices: Option<Vec<Expr>>,
    exactly_one_of: Vec<LitStr>,
    at_least_one_of: Vec<LitStr>,
    requires: Vec<LitStr>,
    conflicts_with: Vec<LitStr>,
    required_unless_present: Vec<LitStr>,
    forbid_substrings: Vec<LitStr>,
    distinct_trimmed_within: Vec<LitStr>,
    path: Option<PluginPathPermissionKind>,
    network: Option<PluginNetworkSemantic>,
    optional: bool,
    jsonpath: Option<LitStr>,
    fallback: Option<LitStr>,
    example: Option<Expr>,
    secret: bool,
    picker: Option<PluginPickerKind>,
}

#[derive(Clone, Copy)]
enum PluginPathPermissionKind {
    Read,
    Write,
}

#[derive(Clone, Copy)]
enum PluginNetworkSemantic {
    Network,
    Url,
    Host,
    Internet,
    Private,
}

#[derive(Clone, Copy)]
enum PluginPickerKind {
    File,
    Dir,
}

#[derive(Clone)]
struct PluginInputPathSpec {
    jsonpath: LitStr,
    kind: PluginPathPermissionKind,
    fallback: Option<LitStr>,
    optional: bool,
}

#[derive(Clone)]
struct PluginInputNetworkSpec {
    jsonpath: LitStr,
    fallback: Option<LitStr>,
    optional: bool,
    semantic: PluginNetworkSemantic,
}

#[derive(Clone)]
struct PluginInputFieldMetadata {
    path: LitStr,
    parse_path: LitStr,
    aliases: Vec<LitStr>,
    description: Option<LitStr>,
    path_kind: Option<PluginPathPermissionKind>,
    network: Option<PluginNetworkSemantic>,
    non_empty: bool,
    item_non_empty: bool,
    item_non_empty_if_present: bool,
    minimum: Option<Expr>,
    maximum: Option<Expr>,
    exclusive_minimum: Option<Expr>,
    exclusive_maximum: Option<Expr>,
    min_items: Option<usize>,
    max_items: Option<usize>,
    min_properties: Option<usize>,
    max_properties: Option<usize>,
    item_minimum: Option<Expr>,
    item_maximum: Option<Expr>,
    item_exclusive_minimum: Option<Expr>,
    item_exclusive_maximum: Option<Expr>,
    item_min_properties: Option<usize>,
    item_max_properties: Option<usize>,
    min_chars: Option<usize>,
    max_chars: Option<usize>,
    item_min_chars: Option<usize>,
    item_max_chars: Option<usize>,
    format: Option<LitStr>,
    item_format: Option<LitStr>,
    pattern: Option<LitStr>,
    item_pattern: Option<LitStr>,
    example: Option<Expr>,
    choices: Vec<Expr>,
    item_choices: Vec<Expr>,
    secret: bool,
    picker: Option<PluginPickerKind>,
}

#[derive(Clone)]
struct PluginInputFieldDefaultSpec {
    schema_path: LitStr,
    parse_path: LitStr,
    aliases: Vec<LitStr>,
    ty: Type,
    default_expr: Option<Expr>,
}

#[derive(Clone)]
struct PluginInputFieldAliasSpec {
    path: LitStr,
    aliases: Vec<LitStr>,
}

struct PreparedInputFieldNames {
    schema_path: LitStr,
    parse_path: LitStr,
    schema_aliases: Vec<LitStr>,
    parse_aliases: Vec<LitStr>,
}

#[derive(Clone)]
struct NestedInputShapeSpec {
    inner_ty: Type,
    optional: bool,
    array: bool,
}

#[derive(Clone)]
struct NestedInputShapeField {
    spec: NestedInputShapeSpec,
    normalize_path: LitStr,
    schema_path: LitStr,
    schema_aliases: Vec<LitStr>,
}

fn parse_plugin_tool_method_attr(
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
                        let mut config = PluginToolCommandConfig::default();
                        config.slash = Some(expr_lit_str(&value.value, "command")?);
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
                    "tags" => spec.tags = parse_expr_list(list.tokens)?,
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
                        for rule in &rules {
                            spec.tags.push(match rule {
                                PluginToolPathPermissionRule::Read(_)
                                | PluginToolPathPermissionRule::Reads(_) => {
                                    parse_quote!(::agena_plugin_sdk::ToolTag::FilesystemRead)
                                }
                                PluginToolPathPermissionRule::Write(_)
                                | PluginToolPathPermissionRule::Writes(_) => {
                                    parse_quote!(::agena_plugin_sdk::ToolTag::FilesystemWrite)
                                }
                                PluginToolPathPermissionRule::Requests(_) => continue,
                            });
                        }
                        permission_path_rules.extend(rules);
                    }
                    "network" => {
                        let rules = parse_inline_network_permission_rules(list.tokens)?;
                        if !rules.is_empty() {
                            spec.tags
                                .push(parse_quote!(::agena_plugin_sdk::ToolTag::Network));
                        }
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
    let args = PluginCommandAttrArgs::parse.parse2(tokens)?;
    let mut config = PluginToolCommandConfig::default();
    config.slash = args.slash;
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
        "read_only" => quote! { ::agena_plugin_sdk::ToolTag::ReadOnly },
        "mutating" => quote! { ::agena_plugin_sdk::ToolTag::Mutating },
        "task" => quote! { ::agena_plugin_sdk::ToolTag::Task },
        "filesystem_read" => quote! { ::agena_plugin_sdk::ToolTag::FilesystemRead },
        "filesystem_write" => quote! { ::agena_plugin_sdk::ToolTag::FilesystemWrite },
        "network" => quote! { ::agena_plugin_sdk::ToolTag::Network },
        "internet" => quote! { ::agena_plugin_sdk::ToolTag::Internet },
        "shell" => quote! { ::agena_plugin_sdk::ToolTag::Shell },
        "interactive" => quote! { ::agena_plugin_sdk::ToolTag::Interactive },
        "discovery" => quote! { ::agena_plugin_sdk::ToolTag::Discovery },
        "planning" => quote! { ::agena_plugin_sdk::ToolTag::Planning },
        "goal" => quote! { ::agena_plugin_sdk::ToolTag::Goal },
        "snapshot" => quote! { ::agena_plugin_sdk::ToolTag::Snapshot },
        "scheduler" => quote! { ::agena_plugin_sdk::ToolTag::Scheduler },
        "lsp" => quote! { ::agena_plugin_sdk::ToolTag::Lsp },
        "mcp" => quote! { ::agena_plugin_sdk::ToolTag::Mcp },
        "subtask" => quote! { ::agena_plugin_sdk::ToolTag::Subtask },
        "private_network" => quote! { ::agena_plugin_sdk::ToolTag::PrivateNetwork },
        _ => return None,
    };
    Some(parse_quote!(#variant))
}

fn plugin_method_tool_output(method: &ImplItemFn, explicit: Option<Type>) -> PluginToolOutputPlan {
    let returns_result = plugin_method_result_ok_type(method).is_some();
    if let Some(explicit) = explicit {
        return PluginToolOutputPlan {
            ty: Some(explicit),
            returns_result,
        };
    }
    let Some((candidate, is_result)) = plugin_method_return_value_type(method) else {
        return PluginToolOutputPlan {
            ty: None,
            returns_result: false,
        };
    };
    if type_is_unit(&candidate)
        || type_last_segment_is(&candidate, "ToolInvokeOutput")
        || type_last_segment_is(&candidate, "ToolStreamEnd")
    {
        return PluginToolOutputPlan {
            ty: None,
            returns_result: false,
        };
    }
    PluginToolOutputPlan {
        ty: Some(candidate),
        returns_result: is_result,
    }
}

fn plugin_method_return_value_type(method: &ImplItemFn) -> Option<(Type, bool)> {
    let ty = plugin_method_return_type(method)?;
    if let Some(ok_ty) = result_ok_type(ty) {
        return Some((ok_ty.clone(), true));
    }
    Some((ty.clone(), false))
}

fn plugin_method_result_ok_type(method: &ImplItemFn) -> Option<&Type> {
    result_ok_type(plugin_method_return_type(method)?)
}

fn plugin_method_return_type(method: &ImplItemFn) -> Option<&Type> {
    let syn::ReturnType::Type(_, ty) = &method.sig.output else {
        return None;
    };
    Some(ty.as_ref())
}

fn result_ok_type(ty: &Type) -> Option<&Type> {
    let Type::Path(path) = ty else {
        return None;
    };
    let segment = path.path.segments.last()?;
    if !matches!(segment.ident.to_string().as_str(), "Result" | "SdkResult") {
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

fn build_plugin_tool_method_shape(
    method: &mut ImplItemFn,
    method_ident: &Ident,
    self_label: &str,
    config: &mut PluginToolAttrConfig,
    docs: Option<String>,
) -> Result<PluginToolMethodShape> {
    let value_args = plugin_method_value_args(method)?;
    let context = plugin_inline_context_arg(&value_args)?;
    let stream_arg_types = value_args
        .iter()
        .map(|arg| arg.ty.clone())
        .collect::<Vec<_>>();
    let input_args = value_args
        .into_iter()
        .filter(|arg| !arg.is_context)
        .collect::<Vec<_>>();
    let input_ident = format_ident!("__AgenaPluginToolInput_{}_{}", self_label, method_ident);

    let (input_ident, input_fields, input_ty, call_input) = match input_args.as_slice() {
        [] => (
            Some(input_ident.clone()),
            Vec::new(),
            parse_quote!(#input_ident),
            PluginCallInput::Fields(Vec::new()),
        ),
        [arg] if !arg.has_arg_config => {
            let input_ty = arg.inner_ty.clone();
            config.spec.input_shape = Some(input_ty.clone());
            (
                None,
                Vec::new(),
                input_ty,
                PluginCallInput::Wrapped { by_ref: arg.by_ref },
            )
        }
        args => {
            let mut fields = Vec::new();
            let mut call_fields = Vec::new();
            let prepared_args = prepare_inline_args(args)?;
            let field_path_lookup = inline_arg_constraint_path_lookup(&prepared_args)?;
            let array_field_paths = prepared_args
                .iter()
                .filter(|prepared| input_type_semantic_shape(&prepared.arg.ty).array)
                .map(|prepared| prepared.field_name.value())
                .collect::<BTreeSet<_>>();
            for prepared in prepared_args {
                let arg = prepared.arg;
                validate_inline_shape_wrapper_arg(arg)?;
                if arg.by_ref {
                    return Err(syn::Error::new_spanned(
                        &arg.ty,
                        "field-style #[tool] arguments must be owned values; use a single input struct argument if the handler wants a reference",
                    ));
                }
                apply_arg_config_to_spec(
                    &mut config.spec,
                    &prepared.field_name,
                    &prepared.aliases,
                    &arg.ty,
                    Some(&field_path_lookup),
                    &arg.config,
                );
                fields.push(PluginGeneratedInputField {
                    ident: arg.ident.clone(),
                    wire_name: prepared.field_name,
                    aliases: prepared.aliases,
                    ty: arg.ty.clone(),
                    default: arg.config.default,
                    default_expr: arg.config.default_expr.clone(),
                    flatten_shape: arg.config.flatten_shape,
                    nested_shape: arg.config.nested_shape,
                });
                call_fields.push(arg.ident.clone());
            }
            normalize_array_value_constraints(
                &mut config.spec.trim,
                &mut config.spec.trim_suffix,
                &mut config.spec.minimums,
                &mut config.spec.maximums,
                &mut config.spec.exclusive_minimums,
                &mut config.spec.exclusive_maximums,
                &mut config.spec.min_properties,
                &mut config.spec.max_properties,
                &mut config.spec.min_chars,
                &mut config.spec.max_chars,
                &mut config.spec.formats,
                &mut config.spec.patterns,
                &mut config.spec.choices,
                &mut config.spec.forbid_substrings,
                &mut config.spec.distinct_trimmed,
                &mut config.spec.input_field_metadata,
                &field_path_lookup,
                &array_field_paths,
            );
            (
                Some(input_ident.clone()),
                fields,
                parse_quote!(#input_ident),
                PluginCallInput::Fields(call_fields),
            )
        }
    };

    let input_model = PluginGeneratedToolInput {
        input_ident,
        input_fields,
        input_ty,
        spec: config.spec.clone(),
        docs,
    };

    Ok(PluginToolMethodShape {
        input_model,
        context,
        call_input,
        stream_arg_types,
        stream_method: config.stream_method.clone(),
    })
}

struct PluginMethodValueArg {
    ident: Ident,
    ty: Type,
    inner_ty: Type,
    by_ref: bool,
    is_context: bool,
    config: PluginArgConfig,
    has_arg_config: bool,
}

fn plugin_method_value_args(method: &mut ImplItemFn) -> Result<Vec<PluginMethodValueArg>> {
    let mut args = Vec::new();
    for arg in method.sig.inputs.iter_mut() {
        let FnArg::Typed(pat_type) = arg else {
            continue;
        };
        let (config, has_arg_config) = parse_plugin_arg_attrs(&mut pat_type.attrs)?;
        let ident = match pat_type.pat.as_ref() {
            Pat::Ident(pat) if pat.by_ref.is_none() && pat.subpat.is_none() => pat.ident.clone(),
            other => {
                return Err(syn::Error::new_spanned(
                    other,
                    "method-level #[tool] generation requires simple identifier arguments",
                ));
            }
        };
        let ty = (*pat_type.ty).clone();
        let by_ref = type_is_reference(&ty);
        let inner_ty = match &ty {
            Type::Reference(reference) => (*reference.elem).clone(),
            other => other.clone(),
        };
        args.push(PluginMethodValueArg {
            ident,
            is_context: type_is_tool_invoke_context(&ty),
            ty,
            inner_ty,
            by_ref,
            config,
            has_arg_config,
        });
    }
    Ok(args)
}

fn plugin_inline_context_arg(args: &[PluginMethodValueArg]) -> Result<Option<PluginContextArg>> {
    let mut context_positions = args
        .iter()
        .enumerate()
        .filter(|(_, arg)| arg.is_context)
        .collect::<Vec<_>>();
    if context_positions.len() > 1 {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            "method-level #[tool] generation supports at most one ToolInvokeContext argument",
        ));
    }
    let Some((index, context_arg)) = context_positions.pop() else {
        return Ok(None);
    };
    let first_input_index = args
        .iter()
        .enumerate()
        .find_map(|(idx, arg)| (!arg.is_context).then_some(idx));
    let first = first_input_index.is_none_or(|input_index| index < input_index);
    Ok(Some(PluginContextArg {
        first,
        by_ref: context_arg.by_ref,
    }))
}

fn plugin_command_context_arg(args: &[PluginMethodValueArg]) -> Result<Option<PluginContextArg>> {
    let mut context_positions = args
        .iter()
        .enumerate()
        .filter(|(_, arg)| type_is_plugin_command_context(&arg.ty))
        .collect::<Vec<_>>();
    if context_positions.len() > 1 {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            "method-level #[command] generation supports at most one PluginCommandContext argument",
        ));
    }
    let Some((index, context_arg)) = context_positions.pop() else {
        return Ok(None);
    };
    let first_input_index = args
        .iter()
        .enumerate()
        .find_map(|(idx, arg)| (!type_is_plugin_command_context(&arg.ty)).then_some(idx));
    let first = first_input_index.is_none_or(|input_index| index < input_index);
    Ok(Some(PluginContextArg {
        first,
        by_ref: context_arg.by_ref,
    }))
}

fn parse_plugin_arg_attrs(attrs: &mut Vec<Attribute>) -> Result<(PluginArgConfig, bool)> {
    let mut config = PluginArgConfig::default();
    let mut found = false;
    let mut kept = Vec::new();
    for attr in std::mem::take(attrs) {
        if !attr.path().is_ident("arg") {
            kept.push(attr);
            continue;
        }
        found = true;
        match &attr.meta {
            Meta::Path(_) => {}
            Meta::NameValue(_) => {
                return Err(syn::Error::new_spanned(
                    attr,
                    "#[arg] supports list syntax, for example #[arg(trim, non_empty)]",
                ));
            }
            Meta::List(_) => parse_plugin_arg_config_attr(&attr, &mut config)?,
        }
    }
    *attrs = kept;
    Ok((config, found))
}

fn parse_plugin_arg_config_attr(attr: &Attribute, config: &mut PluginArgConfig) -> Result<()> {
    let args = attr.parse_args::<ArgAttrArgs>()?;
    for item in args.items {
        match (item.key.as_str(), item.value) {
            ("default", None) => config.default = true,
            ("default", Some(value)) => {
                if config.default || config.default_expr.replace(value).is_some() {
                    return Err(syn::Error::new_spanned(
                        item.first_ident,
                        "duplicate #[arg(default)] or #[arg(default = ...)]",
                    ));
                }
            }
            ("description", Some(value)) => {
                if config
                    .description
                    .replace(expr_lit_str(&value, "description")?)
                    .is_some()
                {
                    return Err(syn::Error::new_spanned(
                        item.first_ident,
                        "duplicate #[arg(description = ...)]",
                    ));
                }
            }
            ("trim", None) => config.trim = true,
            ("item_trim", None) => config.item_trim = true,
            ("non_empty", None) => config.non_empty = true,
            ("item_non_empty", None) => config.item_non_empty = true,
            ("non_empty_if_present", None) => config.non_empty_if_present = true,
            ("item_non_empty_if_present", None) => config.item_non_empty_if_present = true,
            ("distinct_trimmed", None) => config.distinct_trimmed = true,
            ("path.read", None) => {
                set_plugin_arg_path_kind(config, PluginPathPermissionKind::Read, &item.first_ident)?
            }
            ("path.write", None) => set_plugin_arg_path_kind(
                config,
                PluginPathPermissionKind::Write,
                &item.first_ident,
            )?,
            ("network", None) => {
                set_plugin_arg_network(config, PluginNetworkSemantic::Network, &item.first_ident)?
            }
            ("network.url", None) => {
                set_plugin_arg_network(config, PluginNetworkSemantic::Url, &item.first_ident)?
            }
            ("network.host", None) => {
                set_plugin_arg_network(config, PluginNetworkSemantic::Host, &item.first_ident)?
            }
            ("network.internet", None) => {
                set_plugin_arg_network(config, PluginNetworkSemantic::Internet, &item.first_ident)?
            }
            ("network.private", None) => {
                set_plugin_arg_network(config, PluginNetworkSemantic::Private, &item.first_ident)?
            }
            ("optional", None) => config.optional = true,
            ("flatten_shape", None) => config.flatten_shape = true,
            ("nested_shape", None) => config.nested_shape = true,
            ("secret", None) => config.secret = true,
            ("file", None) => {
                set_plugin_arg_picker(config, PluginPickerKind::File, &item.first_ident)?
            }
            ("dir", None) => {
                set_plugin_arg_picker(config, PluginPickerKind::Dir, &item.first_ident)?
            }
            ("jsonpath", Some(value)) => {
                let jsonpath = expr_lit_str(&value, "jsonpath")?;
                validate_input_jsonpath_lit(&jsonpath)?;
                config.jsonpath = Some(jsonpath);
            }
            ("fallback", Some(value)) => config.fallback = Some(expr_lit_str(&value, "fallback")?),
            ("name", Some(value)) => {
                if config.name.replace(expr_lit_str(&value, "name")?).is_some() {
                    return Err(syn::Error::new_spanned(
                        item.first_ident,
                        "duplicate #[arg(name = ...)]",
                    ));
                }
            }
            ("alias", Some(value)) => config.aliases.push(expr_lit_str(&value, "alias")?),
            ("example", Some(value)) => config.example = Some(value),
            ("trim_suffix", Some(value)) => {
                config.trim_suffix = Some(expr_lit_str(&value, "trim_suffix")?)
            }
            ("item_trim_suffix", Some(value)) => {
                let suffix = expr_lit_str(&value, "item_trim_suffix")?;
                if config.item_trim_suffix.replace(suffix).is_some() {
                    return Err(syn::Error::new_spanned(
                        item.first_ident,
                        "duplicate #[arg(item_trim_suffix = ...)]",
                    ));
                }
            }
            ("minimum", Some(value)) => {
                if config.minimum.replace(value).is_some() {
                    return Err(syn::Error::new_spanned(
                        item.first_ident,
                        "duplicate #[arg(minimum = ...)]",
                    ));
                }
            }
            ("maximum", Some(value)) => {
                if config.maximum.replace(value).is_some() {
                    return Err(syn::Error::new_spanned(
                        item.first_ident,
                        "duplicate #[arg(maximum = ...)]",
                    ));
                }
            }
            ("exclusive_minimum", Some(value)) => {
                if config.exclusive_minimum.replace(value).is_some() {
                    return Err(syn::Error::new_spanned(
                        item.first_ident,
                        "duplicate #[arg(exclusive_minimum = ...)]",
                    ));
                }
            }
            ("exclusive_maximum", Some(value)) => {
                if config.exclusive_maximum.replace(value).is_some() {
                    return Err(syn::Error::new_spanned(
                        item.first_ident,
                        "duplicate #[arg(exclusive_maximum = ...)]",
                    ));
                }
            }
            ("min_items", Some(value)) => {
                config.min_items = Some(expr_lit_usize(&value, "min_items")?)
            }
            ("max_items", Some(value)) => {
                config.max_items = Some(expr_lit_usize(&value, "max_items")?)
            }
            ("min_properties", Some(value)) => {
                config.min_properties = Some(expr_lit_usize(&value, "min_properties")?)
            }
            ("max_properties", Some(value)) => {
                config.max_properties = Some(expr_lit_usize(&value, "max_properties")?)
            }
            ("item_minimum", Some(value)) => {
                if config.item_minimum.replace(value).is_some() {
                    return Err(syn::Error::new_spanned(
                        item.first_ident,
                        "duplicate #[arg(item_minimum = ...)]",
                    ));
                }
            }
            ("item_maximum", Some(value)) => {
                if config.item_maximum.replace(value).is_some() {
                    return Err(syn::Error::new_spanned(
                        item.first_ident,
                        "duplicate #[arg(item_maximum = ...)]",
                    ));
                }
            }
            ("item_exclusive_minimum", Some(value)) => {
                if config.item_exclusive_minimum.replace(value).is_some() {
                    return Err(syn::Error::new_spanned(
                        item.first_ident,
                        "duplicate #[arg(item_exclusive_minimum = ...)]",
                    ));
                }
            }
            ("item_exclusive_maximum", Some(value)) => {
                if config.item_exclusive_maximum.replace(value).is_some() {
                    return Err(syn::Error::new_spanned(
                        item.first_ident,
                        "duplicate #[arg(item_exclusive_maximum = ...)]",
                    ));
                }
            }
            ("item_min_properties", Some(value)) => {
                config.item_min_properties = Some(expr_lit_usize(&value, "item_min_properties")?)
            }
            ("item_max_properties", Some(value)) => {
                config.item_max_properties = Some(expr_lit_usize(&value, "item_max_properties")?)
            }
            ("item_min_chars", Some(value)) => {
                config.item_min_chars = Some(expr_lit_usize(&value, "item_min_chars")?)
            }
            ("item_max_chars", Some(value)) => {
                config.item_max_chars = Some(expr_lit_usize(&value, "item_max_chars")?)
            }
            ("min_chars", Some(value)) => {
                config.min_chars = Some(expr_lit_usize(&value, "min_chars")?)
            }
            ("max_chars", Some(value)) => {
                config.max_chars = Some(expr_lit_usize(&value, "max_chars")?)
            }
            ("item_format", Some(value)) => {
                let format = validate_format_lit(&expr_lit_str(&value, "item_format")?)?;
                if config.item_format.replace(format).is_some() {
                    return Err(syn::Error::new_spanned(
                        item.first_ident,
                        "duplicate #[arg(item_format = ...)]",
                    ));
                }
            }
            ("item_pattern", Some(value)) => {
                let pattern = expr_lit_str(&value, "item_pattern")?;
                validate_pattern_lit(&pattern)?;
                if config.item_pattern.replace(pattern).is_some() {
                    return Err(syn::Error::new_spanned(
                        item.first_ident,
                        "duplicate #[arg(item_pattern = ...)]",
                    ));
                }
            }
            ("format", Some(value)) => {
                let format = validate_format_lit(&expr_lit_str(&value, "format")?)?;
                if config.format.replace(format).is_some() {
                    return Err(syn::Error::new_spanned(
                        item.first_ident,
                        "duplicate #[arg(format = ...)]",
                    ));
                }
            }
            ("item_choices", Some(value)) => {
                if config
                    .item_choices
                    .replace(expr_array_values(&value, "item_choices")?)
                    .is_some()
                {
                    return Err(syn::Error::new_spanned(
                        item.first_ident,
                        "duplicate #[arg(item_choices = [...])]",
                    ));
                }
            }
            ("exactly_one_of", Some(value)) => config
                .exactly_one_of
                .extend(expr_array_lit_strs(&value, "exactly_one_of")?),
            ("at_least_one_of", Some(value)) => config
                .at_least_one_of
                .extend(expr_array_lit_strs(&value, "at_least_one_of")?),
            ("requires", Some(value)) => config.requires.push(expr_lit_str(&value, "requires")?),
            ("conflicts_with", Some(value)) => config
                .conflicts_with
                .push(expr_lit_str(&value, "conflicts_with")?),
            ("required_unless_present", Some(value)) => config
                .required_unless_present
                .push(expr_lit_str(&value, "required_unless_present")?),
            ("forbid_substrings", Some(value)) => config
                .forbid_substrings
                .extend(expr_array_lit_strs(&value, "forbid_substrings")?),
            ("distinct_trimmed_within", Some(value)) => config
                .distinct_trimmed_within
                .push(expr_lit_str(&value, "distinct_trimmed_within")?),
            ("pattern", Some(value)) => {
                let pattern = expr_lit_str(&value, "pattern")?;
                validate_pattern_lit(&pattern)?;
                if config.pattern.replace(pattern).is_some() {
                    return Err(syn::Error::new_spanned(
                        item.first_ident,
                        "duplicate #[arg(pattern = ...)]",
                    ));
                }
            }
            ("choices", Some(value)) => {
                if config
                    .choices
                    .replace(expr_array_values(&value, "choices")?)
                    .is_some()
                {
                    return Err(syn::Error::new_spanned(
                        item.first_ident,
                        "duplicate #[arg(choices = [...])]",
                    ));
                }
            }
            (key, Some(_)) => {
                return Err(syn::Error::new_spanned(
                    item.first_ident,
                    format!("unsupported #[arg] option '{key}'"),
                ));
            }
            (key, None) => {
                return Err(syn::Error::new_spanned(
                    item.first_ident,
                    format!("unsupported #[arg] flag '{key}'"),
                ));
            }
        }
    }
    ensure_arg_permission_locator_has_semantic(
        config.jsonpath.as_ref(),
        config.fallback.as_ref(),
        config.path.is_some() || config.network.is_some(),
    )?;
    Ok(())
}

fn ensure_arg_permission_locator_has_semantic(
    jsonpath: Option<&LitStr>,
    fallback: Option<&LitStr>,
    has_permission_semantic: bool,
) -> Result<()> {
    if has_permission_semantic {
        return Ok(());
    }
    if let Some(value) = jsonpath.or(fallback) {
        return Err(syn::Error::new_spanned(
            value,
            "`jsonpath` and `fallback` require a path.* or network.* semantic",
        ));
    }
    Ok(())
}

struct ArgAttrArgs {
    items: Vec<ArgAttrItem>,
}

struct ArgAttrItem {
    key: String,
    first_ident: Ident,
    value: Option<Expr>,
}

impl Parse for ArgAttrArgs {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let mut items = Vec::new();
        while !input.is_empty() {
            let first_ident = input.call(Ident::parse_any)?;
            let mut key = first_ident.to_string();
            while input.peek(Token![.]) {
                input.parse::<Token![.]>()?;
                key.push('.');
                key.push_str(&input.call(Ident::parse_any)?.to_string());
            }
            let value = if input.peek(Token![=]) {
                input.parse::<Token![=]>()?;
                Some(input.parse()?)
            } else {
                None
            };
            items.push(ArgAttrItem {
                key,
                first_ident,
                value,
            });
            if input.peek(Token![,]) {
                input.parse::<Token![,]>()?;
            } else if !input.is_empty() {
                return Err(input.error("expected `,` between #[arg] entries"));
            }
        }
        Ok(Self { items })
    }
}

fn set_plugin_arg_path_kind(
    config: &mut PluginArgConfig,
    kind: PluginPathPermissionKind,
    span: impl quote::ToTokens,
) -> Result<()> {
    if config.path.replace(kind).is_some() {
        return Err(syn::Error::new_spanned(
            span,
            "#[arg] accepts only one path permission semantic",
        ));
    }
    Ok(())
}

fn set_field_arg_path_kind(
    config: &mut FieldArgConfig,
    kind: PluginPathPermissionKind,
    span: impl quote::ToTokens,
) -> Result<()> {
    if config.path.replace(kind).is_some() {
        return Err(syn::Error::new_spanned(
            span,
            "#[arg] accepts only one path permission semantic",
        ));
    }
    Ok(())
}

fn set_plugin_arg_network(
    config: &mut PluginArgConfig,
    semantic: PluginNetworkSemantic,
    span: impl quote::ToTokens,
) -> Result<()> {
    if config.network.replace(semantic).is_some() {
        return Err(syn::Error::new_spanned(
            span,
            "#[arg] accepts only one network semantic",
        ));
    }
    Ok(())
}

fn set_field_arg_network(
    config: &mut FieldArgConfig,
    semantic: PluginNetworkSemantic,
    span: impl quote::ToTokens,
) -> Result<()> {
    if config.network.replace(semantic).is_some() {
        return Err(syn::Error::new_spanned(
            span,
            "#[arg] accepts only one network semantic",
        ));
    }
    Ok(())
}

fn set_plugin_arg_picker(
    config: &mut PluginArgConfig,
    picker: PluginPickerKind,
    span: impl quote::ToTokens,
) -> Result<()> {
    if config.picker.replace(picker).is_some() {
        return Err(syn::Error::new_spanned(
            span,
            "#[arg] accepts only one picker semantic",
        ));
    }
    Ok(())
}

fn set_field_arg_picker(
    config: &mut FieldArgConfig,
    picker: PluginPickerKind,
    span: impl quote::ToTokens,
) -> Result<()> {
    if config.picker.replace(picker).is_some() {
        return Err(syn::Error::new_spanned(
            span,
            "#[arg] accepts only one picker semantic",
        ));
    }
    Ok(())
}

fn input_jsonpath_for_field(field_name: &LitStr, ty: &Type) -> LitStr {
    let shape = input_type_semantic_shape(ty);
    let suffix = shape.array.then_some("[*]").unwrap_or("");
    LitStr::new(
        &format!("$.{}{}", field_name.value(), suffix),
        field_name.span(),
    )
}

fn input_jsonpath_for_arg(
    field_name: &LitStr,
    ty: &Type,
    override_path: Option<&LitStr>,
) -> LitStr {
    override_path
        .cloned()
        .unwrap_or_else(|| input_jsonpath_for_field(field_name, ty))
}

fn inline_arg_field_names(
    ident: &Ident,
    config: &PluginArgConfig,
) -> Result<(LitStr, Vec<LitStr>)> {
    let field_name = config
        .name
        .clone()
        .unwrap_or_else(|| LitStr::new(&ident.to_string(), ident.span()));
    let mut seen = BTreeSet::from([field_name.value()]);
    let mut aliases = Vec::new();
    for alias in &config.aliases {
        if !seen.insert(alias.value()) {
            return Err(syn::Error::new_spanned(
                alias,
                format!(
                    "duplicate inline #[arg] wire name or alias `{}`",
                    alias.value()
                ),
            ));
        }
        aliases.push(alias.clone());
    }
    Ok((field_name, aliases))
}

struct PreparedInlineArg<'a> {
    arg: &'a PluginMethodValueArg,
    field_name: LitStr,
    aliases: Vec<LitStr>,
}

fn prepare_inline_args<'a>(args: &'a [PluginMethodValueArg]) -> Result<Vec<PreparedInlineArg<'a>>> {
    let mut seen_field_names = BTreeSet::new();
    let mut prepared = Vec::with_capacity(args.len());
    for arg in args {
        let (field_name, aliases) = inline_arg_field_names(&arg.ident, &arg.config)?;
        ensure_unique_inline_arg_field_names(&mut seen_field_names, &field_name, &aliases)?;
        prepared.push(PreparedInlineArg {
            arg,
            field_name,
            aliases,
        });
    }
    Ok(prepared)
}

fn inline_arg_constraint_path_lookup(
    prepared: &[PreparedInlineArg<'_>],
) -> Result<BTreeMap<String, LitStr>> {
    let mut lookup = BTreeMap::new();
    for prepared_arg in prepared {
        let target = prepared_arg.field_name.clone();
        let candidates = std::iter::once((&prepared_arg.arg.ident, None))
            .chain(std::iter::once((&prepared_arg.arg.ident, Some(&target))))
            .chain(
                prepared_arg
                    .aliases
                    .iter()
                    .map(|alias| (&prepared_arg.arg.ident, Some(alias))),
            );
        for (ident, candidate) in candidates {
            let candidate = candidate
                .cloned()
                .unwrap_or_else(|| LitStr::new(&ident.to_string(), ident.span()));
            let candidate_value = candidate.value();
            if let Some(existing) = lookup.insert(candidate.value(), target.clone())
                && existing.value() != target.value()
            {
                return Err(syn::Error::new_spanned(
                    &candidate,
                    format!(
                        "duplicate inline #[arg] wire name or alias `{}`",
                        candidate_value
                    ),
                ));
            }
        }
        if let Some(existing) = lookup.insert(target.value(), target.clone())
            && existing.value() != target.value()
        {
            return Err(syn::Error::new_spanned(
                &prepared_arg.field_name,
                format!(
                    "duplicate inline #[arg] wire name or alias `{}`",
                    prepared_arg.field_name.value()
                ),
            ));
        }
    }
    Ok(lookup)
}

fn merged_field_arg_aliases(
    field_name: &LitStr,
    serde_aliases: &[String],
    arg_aliases: &[LitStr],
) -> Result<Vec<LitStr>> {
    let mut seen = BTreeSet::from([field_name.value()]);
    let mut aliases = Vec::new();
    for alias in serde_aliases {
        if !seen.insert(alias.clone()) {
            return Err(syn::Error::new(
                field_name.span(),
                format!("duplicate ToolInput field wire name or alias `{alias}`"),
            ));
        }
        aliases.push(LitStr::new(alias, field_name.span()));
    }
    for alias in arg_aliases {
        if !seen.insert(alias.value()) {
            return Err(syn::Error::new_spanned(
                alias,
                format!(
                    "duplicate ToolInput field wire name or alias `{}`",
                    alias.value()
                ),
            ));
        }
        aliases.push(alias.clone());
    }
    Ok(aliases)
}

fn prepare_input_field_names(
    field: &Field,
    rename_rule: Option<SerdeRenameRule>,
    arg_config: &FieldArgConfig,
) -> Result<Option<PreparedInputFieldNames>> {
    let Some(parse_path) = field_schema_property_name_with_rule(field, rename_rule)? else {
        return Ok(None);
    };
    let parse_path = LitStr::new(&parse_path, field.span());
    let schema_path = arg_config
        .name
        .clone()
        .unwrap_or_else(|| parse_path.clone());
    let serde_aliases = field_schema_aliases(field)?;

    let mut schema_side_aliases = serde_aliases.clone();
    if schema_path.value() != parse_path.value() {
        schema_side_aliases.insert(0, parse_path.value());
    }
    let schema_aliases =
        merged_field_arg_aliases(&schema_path, &schema_side_aliases, &arg_config.aliases)?;

    let mut parse_side_aliases = serde_aliases;
    if schema_path.value() != parse_path.value() {
        parse_side_aliases.insert(0, schema_path.value());
    }
    let parse_aliases =
        merged_field_arg_aliases(&parse_path, &parse_side_aliases, &arg_config.aliases)?;

    Ok(Some(PreparedInputFieldNames {
        schema_path,
        parse_path,
        schema_aliases,
        parse_aliases,
    }))
}

fn nested_input_shape_spec(field: &Field) -> Result<Option<NestedInputShapeSpec>> {
    if field_is_flatten(field)? {
        return Ok(None);
    }
    let mut enabled = false;
    for attr in &field.attrs {
        if !attr.path().is_ident("input") {
            continue;
        }
        let metas = attr.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)?;
        for meta in metas {
            if let Meta::Path(path) = meta
                && path.is_ident("nested_shape")
            {
                enabled = true;
            }
        }
    }
    if !enabled {
        return Ok(None);
    }

    Ok(nested_input_shape_spec_from_type(&field.ty))
}

fn nested_input_shape_spec_from_type(ty: &Type) -> Option<NestedInputShapeSpec> {
    let ty = type_without_reference(ty).clone();
    let (optional, inner_ty) = if type_last_segment_is(&ty, "Option") {
        let inner = type_first_generic_arg(&ty)?;
        (true, inner.clone())
    } else {
        (false, ty)
    };
    let (array, inner_ty) = if type_last_segment_is(&inner_ty, "Vec") {
        let inner = type_first_generic_arg(&inner_ty)?;
        (true, inner.clone())
    } else {
        (false, inner_ty)
    };

    Some(NestedInputShapeSpec {
        inner_ty,
        optional,
        array,
    })
}

fn nested_input_shape_field(
    field: &Field,
    rename_rule: Option<SerdeRenameRule>,
) -> Result<Option<NestedInputShapeField>> {
    let Some(spec) = nested_input_shape_spec(field)? else {
        return Ok(None);
    };
    let arg_config = parse_input_field_arg_attrs(field)?;
    let Some(names) = prepare_input_field_names(field, rename_rule, &arg_config)? else {
        return Ok(None);
    };
    let normalize_path = if spec.array {
        append_constraint_path_suffix(&names.parse_path, "[]")
    } else {
        names.parse_path.clone()
    };
    Ok(Some(NestedInputShapeField {
        spec,
        normalize_path,
        schema_path: names.schema_path,
        schema_aliases: names.schema_aliases,
    }))
}

fn generated_input_nested_shape_fields(
    fields: &[PluginGeneratedInputField],
) -> Vec<NestedInputShapeField> {
    fields
        .iter()
        .filter_map(|field| {
            let spec = field
                .nested_shape
                .then(|| nested_input_shape_spec_from_type(&field.ty))
                .flatten()?;
            let normalize_path = if spec.array {
                append_constraint_path_suffix(&field.wire_name, "[]")
            } else {
                field.wire_name.clone()
            };
            Some(NestedInputShapeField {
                spec,
                normalize_path,
                schema_path: field.wire_name.clone(),
                schema_aliases: field.aliases.clone(),
            })
        })
        .collect()
}

fn generated_input_alias_specs(
    fields: &[PluginGeneratedInputField],
) -> Vec<PluginInputFieldAliasSpec> {
    fields
        .iter()
        .filter(|field| !field.aliases.is_empty())
        .map(|field| PluginInputFieldAliasSpec {
            path: field.wire_name.clone(),
            aliases: field.aliases.clone(),
        })
        .collect()
}

fn generated_input_flatten_shape_types(fields: &[PluginGeneratedInputField]) -> Result<Vec<Type>> {
    fields
        .iter()
        .filter(|field| field.flatten_shape)
        .map(|field| {
            let shape = input_type_semantic_shape(&field.ty);
            if shape.optional || shape.array {
                return Err(syn::Error::new_spanned(
                    &field.ty,
                    "inline #[arg(flatten_shape)] only supports plain ToolInput object types; use a non-Option, non-Vec ToolInput",
                ));
            }
            Ok(field.ty.clone())
        })
        .collect()
}

fn inline_flatten_shape_has_extra_config(config: &PluginArgConfig) -> bool {
    config.default
        || config.default_expr.is_some()
        || config.description.is_some()
        || config.trim
        || config.item_trim
        || config.non_empty
        || config.item_non_empty
        || config.non_empty_if_present
        || config.item_non_empty_if_present
        || config.distinct_trimmed
        || config.trim_suffix.is_some()
        || config.item_trim_suffix.is_some()
        || config.minimum.is_some()
        || config.maximum.is_some()
        || config.exclusive_minimum.is_some()
        || config.exclusive_maximum.is_some()
        || config.min_items.is_some()
        || config.max_items.is_some()
        || config.min_properties.is_some()
        || config.max_properties.is_some()
        || config.item_minimum.is_some()
        || config.item_maximum.is_some()
        || config.item_exclusive_minimum.is_some()
        || config.item_exclusive_maximum.is_some()
        || config.item_min_properties.is_some()
        || config.item_max_properties.is_some()
        || config.min_chars.is_some()
        || config.max_chars.is_some()
        || config.item_min_chars.is_some()
        || config.item_max_chars.is_some()
        || config.format.is_some()
        || config.item_format.is_some()
        || config.pattern.is_some()
        || config.item_pattern.is_some()
        || config.choices.is_some()
        || config.item_choices.is_some()
        || !config.exactly_one_of.is_empty()
        || !config.at_least_one_of.is_empty()
        || !config.requires.is_empty()
        || !config.conflicts_with.is_empty()
        || !config.required_unless_present.is_empty()
        || !config.forbid_substrings.is_empty()
        || !config.distinct_trimmed_within.is_empty()
        || config.path.is_some()
        || config.network.is_some()
        || config.optional
        || config.nested_shape
        || config.jsonpath.is_some()
        || config.fallback.is_some()
        || config.name.is_some()
        || !config.aliases.is_empty()
        || config.example.is_some()
        || config.secret
        || config.picker.is_some()
}

fn validate_inline_shape_wrapper_arg(arg: &PluginMethodValueArg) -> Result<()> {
    if !arg.config.flatten_shape {
        return Ok(());
    }
    if inline_flatten_shape_has_extra_config(&arg.config) {
        return Err(syn::Error::new_spanned(
            &arg.ty,
            "inline #[arg(flatten_shape)] cannot be combined with name/alias/default/validation/permission metadata; put those rules on the flattened ToolInput type itself",
        ));
    }
    let shape = input_type_semantic_shape(&arg.ty);
    if shape.optional || shape.array {
        return Err(syn::Error::new_spanned(
            &arg.ty,
            "inline #[arg(flatten_shape)] only supports plain ToolInput object types; use a non-Option, non-Vec ToolInput",
        ));
    }
    Ok(())
}

fn input_constraint_field_lookup(
    fields: &Fields,
    rename_rule: Option<SerdeRenameRule>,
) -> Result<(BTreeMap<String, LitStr>, BTreeSet<String>)> {
    let Fields::Named(named) = fields else {
        return Ok((BTreeMap::new(), BTreeSet::new()));
    };

    let mut array_field_paths = BTreeSet::new();
    let mut field_path_lookup = BTreeMap::new();
    for field in &named.named {
        let arg_config = parse_input_field_arg_attrs(field)?;
        let Some(names) = prepare_input_field_names(field, rename_rule, &arg_config)? else {
            continue;
        };
        let raw_field_name = field
            .ident
            .as_ref()
            .map(|ident| LitStr::new(ident.to_string().as_str(), ident.span()));
        if input_type_semantic_shape(&field.ty).array {
            array_field_paths.insert(names.parse_path.value());
        }
        for candidate in raw_field_name
            .iter()
            .chain(std::iter::once(&names.schema_path))
            .chain(std::iter::once(&names.parse_path))
            .chain(names.schema_aliases.iter())
            .chain(names.parse_aliases.iter())
        {
            if let Some(existing) =
                field_path_lookup.insert(candidate.value(), names.parse_path.clone())
                && existing.value() != names.parse_path.value()
            {
                return Err(syn::Error::new_spanned(
                    candidate,
                    format!(
                        "duplicate ToolInput field wire name or alias `{}`",
                        candidate.value()
                    ),
                ));
            }
        }
    }

    Ok((field_path_lookup, array_field_paths))
}

fn input_keys_for_parse_path(path: &LitStr, aliases: &[PluginInputFieldAliasSpec]) -> Vec<LitStr> {
    let mut seen = BTreeSet::new();
    let mut keys = Vec::new();
    if seen.insert(path.value()) {
        keys.push(path.clone());
    }
    let value = path.value();
    let head_end = value.find('.').unwrap_or(value.len());
    let (head, tail) = value.split_at(head_end);
    let mut base = head;
    let mut suffix = String::new();
    while let Some(stripped) = base.strip_suffix("[]") {
        base = stripped;
        suffix.push_str("[]");
    }
    for alias_spec in aliases {
        if alias_spec.path.value() != base {
            continue;
        }
        for alias in &alias_spec.aliases {
            let candidate = if tail.is_empty() && suffix.is_empty() {
                alias.clone()
            } else {
                LitStr::new(
                    format!("{}{}{}", alias.value(), suffix, tail).as_str(),
                    alias.span(),
                )
            };
            if seen.insert(candidate.value()) {
                keys.push(candidate);
            }
        }
    }
    keys
}

fn ensure_unique_field_arg_names(
    seen: &mut BTreeSet<String>,
    field_name: &LitStr,
    aliases: &[LitStr],
) -> Result<()> {
    for candidate in std::iter::once(field_name).chain(aliases.iter()) {
        if !seen.insert(candidate.value()) {
            return Err(syn::Error::new_spanned(
                candidate,
                format!(
                    "duplicate ToolInput field wire name or alias `{}`",
                    candidate.value()
                ),
            ));
        }
    }
    Ok(())
}

fn ensure_unique_inline_arg_field_names(
    seen: &mut BTreeSet<String>,
    field_name: &LitStr,
    aliases: &[LitStr],
) -> Result<()> {
    for candidate in std::iter::once(field_name).chain(aliases.iter()) {
        if !seen.insert(candidate.value()) {
            return Err(syn::Error::new_spanned(
                candidate,
                format!(
                    "duplicate inline #[arg] wire name or alias `{}`",
                    candidate.value()
                ),
            ));
        }
    }
    Ok(())
}

fn validate_input_jsonpath_lit(jsonpath: &LitStr) -> Result<()> {
    validate_input_jsonpath(jsonpath.value().as_str())
        .map_err(|message| syn::Error::new_spanned(jsonpath, message))
}

fn validate_pattern_lit(pattern: &LitStr) -> Result<()> {
    regex::Regex::new(pattern.value().as_str())
        .map(|_| ())
        .map_err(|err| syn::Error::new_spanned(pattern, format!("invalid pattern regex: {err}")))
}

fn normalized_format_name(value: &str) -> Option<&'static str> {
    match value {
        "uri" => Some("uri"),
        "uuid" => Some("uuid"),
        "email" => Some("email"),
        "hostname" => Some("hostname"),
        "ipv4" => Some("ipv4"),
        "ipv6" => Some("ipv6"),
        _ => None,
    }
}

fn supported_format_names() -> &'static str {
    "uri, uuid, email, hostname, ipv4, ipv6"
}

fn validate_format_lit(format: &LitStr) -> Result<LitStr> {
    let value = format.value();
    match normalized_format_name(value.as_str()) {
        Some(normalized) => Ok(LitStr::new(normalized, format.span())),
        None => Err(syn::Error::new_spanned(
            format,
            format!(
                "unsupported format `{value}`; supported formats: {}",
                supported_format_names()
            ),
        )),
    }
}

fn validate_input_jsonpath(jsonpath: &str) -> std::result::Result<(), String> {
    if jsonpath == "$" {
        return Ok(());
    }
    let Some(mut rest) = jsonpath.strip_prefix("$.") else {
        return Err(format!("unsupported input jsonpath '{jsonpath}'"));
    };
    if rest.is_empty() {
        return Err(format!("unsupported input jsonpath '{jsonpath}'"));
    }
    while !rest.is_empty() {
        let key_end = rest.find(['.', '[']).unwrap_or(rest.len());
        let key = &rest[..key_end];
        if key.is_empty() {
            return Err(format!("unsupported input jsonpath '{jsonpath}'"));
        }
        rest = &rest[key_end..];
        while let Some(tail) = rest.strip_prefix("[*]") {
            rest = tail;
        }
        if rest.is_empty() {
            break;
        }
        let Some(tail) = rest.strip_prefix('.') else {
            return Err(format!("unsupported input jsonpath '{jsonpath}'"));
        };
        rest = tail;
        if rest.is_empty() {
            return Err(format!("unsupported input jsonpath '{jsonpath}'"));
        }
    }
    Ok(())
}

#[derive(Default)]
struct InputTypeSemanticShape {
    optional: bool,
    array: bool,
}

fn input_type_semantic_shape(ty: &Type) -> InputTypeSemanticShape {
    let ty = type_without_reference(ty);
    if type_last_segment_is(&ty, "Option")
        && let Some(inner) = type_first_generic_arg(&ty)
    {
        let mut shape = input_type_semantic_shape(inner);
        shape.optional = true;
        return shape;
    }
    InputTypeSemanticShape {
        optional: false,
        array: type_last_segment_is(&ty, "Vec"),
    }
}

fn type_first_generic_arg(ty: &Type) -> Option<&Type> {
    let Type::Path(path) = ty else {
        return None;
    };
    let segment = path.path.segments.last()?;
    let PathArguments::AngleBracketed(args) = &segment.arguments else {
        return None;
    };
    args.args.iter().find_map(|arg| match arg {
        syn::GenericArgument::Type(ty) => Some(ty),
        _ => None,
    })
}

fn path_permission_kind_label(kind: PluginPathPermissionKind) -> &'static str {
    match kind {
        PluginPathPermissionKind::Read => "read",
        PluginPathPermissionKind::Write => "write",
    }
}

fn network_semantic_label(semantic: PluginNetworkSemantic) -> &'static str {
    match semantic {
        PluginNetworkSemantic::Network => "network",
        PluginNetworkSemantic::Url => "url",
        PluginNetworkSemantic::Host => "host",
        PluginNetworkSemantic::Internet => "internet",
        PluginNetworkSemantic::Private => "private",
    }
}

fn picker_kind_label(picker: PluginPickerKind) -> &'static str {
    match picker {
        PluginPickerKind::File => "file",
        PluginPickerKind::Dir => "dir",
    }
}

fn apply_arg_config_to_spec(
    spec: &mut ToolSpecConfig,
    field_name: &LitStr,
    aliases: &[LitStr],
    ty: &Type,
    field_path_lookup: Option<&BTreeMap<String, LitStr>>,
    config: &PluginArgConfig,
) {
    if config.trim {
        spec.trim.push(field_name.clone());
    }
    if config.item_trim {
        spec.trim
            .push(append_constraint_path_suffix(field_name, "[]"));
    }
    if config.non_empty {
        spec.non_empty.push(field_name.clone());
    }
    if config.item_non_empty {
        spec.non_empty
            .push(append_constraint_path_suffix(field_name, "[]"));
    }
    if config.non_empty_if_present {
        spec.non_empty_if_present.push(field_name.clone());
    }
    if config.item_non_empty_if_present {
        spec.non_empty_if_present
            .push(append_constraint_path_suffix(field_name, "[]"));
    }
    if let Some(value) = config.trim_suffix.as_ref() {
        spec.trim_suffix.push(PathStringConstraint {
            path: field_name.clone(),
            value: value.clone(),
        });
    }
    if let Some(value) = config.item_trim_suffix.as_ref() {
        spec.trim_suffix.push(PathStringConstraint {
            path: append_constraint_path_suffix(field_name, "[]"),
            value: value.clone(),
        });
    }
    if let Some(value) = config.minimum.as_ref() {
        spec.minimums.push(PathValueConstraint {
            path: field_name.clone(),
            value: value.clone(),
        });
    }
    if let Some(value) = config.maximum.as_ref() {
        spec.maximums.push(PathValueConstraint {
            path: field_name.clone(),
            value: value.clone(),
        });
    }
    if let Some(value) = config.exclusive_minimum.as_ref() {
        spec.exclusive_minimums.push(PathValueConstraint {
            path: field_name.clone(),
            value: value.clone(),
        });
    }
    if let Some(value) = config.exclusive_maximum.as_ref() {
        spec.exclusive_maximums.push(PathValueConstraint {
            path: field_name.clone(),
            value: value.clone(),
        });
    }
    if let Some(value) = config.min_items {
        spec.min_items.push(PathUsizeConstraint {
            path: field_name.clone(),
            value,
        });
    }
    if let Some(value) = config.max_items {
        spec.max_items.push(PathUsizeConstraint {
            path: field_name.clone(),
            value,
        });
    }
    if let Some(value) = config.min_properties {
        spec.min_properties.push(PathUsizeConstraint {
            path: field_name.clone(),
            value,
        });
    }
    if let Some(value) = config.max_properties {
        spec.max_properties.push(PathUsizeConstraint {
            path: field_name.clone(),
            value,
        });
    }
    if let Some(value) = config.item_minimum.as_ref() {
        spec.minimums.push(PathValueConstraint {
            path: append_constraint_path_suffix(field_name, "[]"),
            value: value.clone(),
        });
    }
    if let Some(value) = config.item_maximum.as_ref() {
        spec.maximums.push(PathValueConstraint {
            path: append_constraint_path_suffix(field_name, "[]"),
            value: value.clone(),
        });
    }
    if let Some(value) = config.item_exclusive_minimum.as_ref() {
        spec.exclusive_minimums.push(PathValueConstraint {
            path: append_constraint_path_suffix(field_name, "[]"),
            value: value.clone(),
        });
    }
    if let Some(value) = config.item_exclusive_maximum.as_ref() {
        spec.exclusive_maximums.push(PathValueConstraint {
            path: append_constraint_path_suffix(field_name, "[]"),
            value: value.clone(),
        });
    }
    if let Some(value) = config.item_min_properties {
        spec.min_properties.push(PathUsizeConstraint {
            path: append_constraint_path_suffix(field_name, "[]"),
            value,
        });
    }
    if let Some(value) = config.item_max_properties {
        spec.max_properties.push(PathUsizeConstraint {
            path: append_constraint_path_suffix(field_name, "[]"),
            value,
        });
    }
    if let Some(value) = config.min_chars {
        spec.min_chars.push(PathUsizeConstraint {
            path: field_name.clone(),
            value,
        });
    }
    if let Some(value) = config.max_chars {
        spec.max_chars.push(PathUsizeConstraint {
            path: field_name.clone(),
            value,
        });
    }
    if let Some(value) = config.item_min_chars {
        spec.min_chars.push(PathUsizeConstraint {
            path: append_constraint_path_suffix(field_name, "[]"),
            value,
        });
    }
    if let Some(value) = config.item_max_chars {
        spec.max_chars.push(PathUsizeConstraint {
            path: append_constraint_path_suffix(field_name, "[]"),
            value,
        });
    }
    if let Some(value) = config.format.as_ref() {
        spec.formats.push(PathStringConstraint {
            path: field_name.clone(),
            value: value.clone(),
        });
    }
    if let Some(value) = config.item_format.as_ref() {
        spec.formats.push(PathStringConstraint {
            path: append_constraint_path_suffix(field_name, "[]"),
            value: value.clone(),
        });
    }
    if let Some(value) = config.pattern.as_ref() {
        spec.patterns.push(PathStringConstraint {
            path: field_name.clone(),
            value: value.clone(),
        });
    }
    if let Some(value) = config.item_pattern.as_ref() {
        spec.patterns.push(PathStringConstraint {
            path: append_constraint_path_suffix(field_name, "[]"),
            value: value.clone(),
        });
    }
    if let Some(values) = config.choices.as_ref() {
        spec.choices.push(PathValuesConstraint {
            path: field_name.clone(),
            values: values.clone(),
        });
    }
    if let Some(values) = config.item_choices.as_ref() {
        spec.choices.push(PathValuesConstraint {
            path: append_constraint_path_suffix(field_name, "[]"),
            values: values.clone(),
        });
    }
    let type_shape = input_type_semantic_shape(ty);
    let field_value_path = if type_shape.array {
        append_constraint_path_suffix(field_name, "[]")
    } else {
        field_name.clone()
    };
    spec.requires
        .extend(config.requires.iter().map(|right| PathPairConstraint {
            left: field_name.clone(),
            right: resolve_known_constraint_path(right, field_path_lookup),
        }));
    spec.conflicts_with.extend(
        config
            .conflicts_with
            .iter()
            .map(|right| PathPairConstraint {
                left: field_name.clone(),
                right: resolve_known_constraint_path(right, field_path_lookup),
            }),
    );
    spec.required_unless_present
        .extend(
            config
                .required_unless_present
                .iter()
                .map(|right| PathPairConstraint {
                    left: field_name.clone(),
                    right: resolve_known_constraint_path(right, field_path_lookup),
                }),
        );
    if !config.forbid_substrings.is_empty() {
        spec.forbid_substrings.push(PathStringsConstraint {
            path: field_value_path.clone(),
            values: config.forbid_substrings.clone(),
        });
    }
    if config.distinct_trimmed {
        spec.distinct_trimmed.push(field_value_path.clone());
    }
    if !config.exactly_one_of.is_empty() {
        spec.exactly_one_of.push(prefixed_constraint_group(
            field_name,
            &config.exactly_one_of,
            field_path_lookup,
        ));
    }
    if !config.at_least_one_of.is_empty() {
        spec.at_least_one_of.push(prefixed_constraint_group(
            field_name,
            &config.at_least_one_of,
            field_path_lookup,
        ));
    }
    spec.distinct_trimmed_within
        .extend(
            config
                .distinct_trimmed_within
                .iter()
                .map(|right| PathPairConstraint {
                    left: field_name.clone(),
                    right: resolve_known_constraint_path(right, field_path_lookup),
                }),
        );
    let optional = config.optional
        || inline_arg_has_default(config)
        || type_shape.optional
        || !aliases.is_empty();
    let jsonpath = input_jsonpath_for_arg(field_name, ty, config.jsonpath.as_ref());
    if let Some(kind) = config.path {
        spec.input_paths.push(PluginInputPathSpec {
            jsonpath: jsonpath.clone(),
            kind,
            fallback: config.fallback.clone(),
            optional,
        });
        if config.jsonpath.is_none() {
            spec.input_paths
                .extend(aliases.iter().map(|alias| PluginInputPathSpec {
                    jsonpath: input_jsonpath_for_field(alias, ty),
                    kind,
                    fallback: config.fallback.clone(),
                    optional,
                }));
        }
    }
    if let Some(semantic) = config.network {
        spec.input_networks.push(PluginInputNetworkSpec {
            jsonpath,
            fallback: config.fallback.clone(),
            optional,
            semantic,
        });
        if config.jsonpath.is_none() {
            spec.input_networks
                .extend(aliases.iter().map(|alias| PluginInputNetworkSpec {
                    jsonpath: input_jsonpath_for_field(alias, ty),
                    fallback: config.fallback.clone(),
                    optional,
                    semantic,
                }));
        }
    }
    apply_arg_metadata_to_spec(
        &mut spec.input_field_metadata,
        field_name,
        field_name,
        aliases,
        config.description.clone(),
        config.path,
        config.network,
        config.non_empty || config.non_empty_if_present,
        config.item_non_empty,
        config.item_non_empty_if_present,
        config.minimum.clone(),
        config.maximum.clone(),
        config.exclusive_minimum.clone(),
        config.exclusive_maximum.clone(),
        config.min_items,
        config.max_items,
        config.min_properties,
        config.max_properties,
        config.item_minimum.clone(),
        config.item_maximum.clone(),
        config.item_exclusive_minimum.clone(),
        config.item_exclusive_maximum.clone(),
        config.item_min_properties,
        config.item_max_properties,
        config.min_chars,
        config.max_chars,
        config.item_min_chars,
        config.item_max_chars,
        config.format.clone(),
        config.item_format.clone(),
        config.pattern.clone(),
        config.item_pattern.clone(),
        config.example.clone(),
        config.choices.clone().unwrap_or_default(),
        config.item_choices.clone().unwrap_or_default(),
        config.secret,
        config.picker,
    );
}

fn apply_input_field_arg_attrs(
    config: &mut ToolInputConfig,
    attrs: &[Attribute],
    data: &Data,
) -> Result<()> {
    let Data::Struct(data_struct) = data else {
        return Ok(());
    };
    let Fields::Named(fields) = &data_struct.fields else {
        return Ok(());
    };
    let rename_rule = serde_rename_all_rule(attrs)?;
    let mut all_field_names = Vec::new();
    let (field_path_lookup, array_field_paths) =
        input_constraint_field_lookup(&Fields::Named(fields.clone()), rename_rule)?;
    let mut prepared_fields = Vec::new();
    for (index, field) in fields.named.iter().enumerate() {
        let arg_config = parse_input_field_arg_attrs(field)?;
        if let Some(names) = prepare_input_field_names(field, rename_rule, &arg_config)? {
            let mut accepted_names = BTreeSet::new();
            ensure_unique_field_arg_names(
                &mut accepted_names,
                &names.schema_path,
                &names.schema_aliases,
            )?;
            all_field_names.push((index, accepted_names));
        }
        if !arg_config_has_constraints(&arg_config) {
            continue;
        }
        let Some(names) = prepare_input_field_names(field, rename_rule, &arg_config)? else {
            return Err(syn::Error::new_spanned(
                field,
                "field-level #[arg(...)] cannot be used on flattened or skipped fields; put the constraint on the flattened input shape or remove the serde skip",
            ));
        };
        let serde_default = field_has_serde_default(field)?;
        if field_arg_has_default(&arg_config) && serde_default {
            return Err(syn::Error::new_spanned(
                field,
                "field-level #[arg(default)] and #[serde(default)] cannot be combined; keep one default source",
            ));
        }
        prepared_fields.push((index, field, names, serde_default, arg_config));
    }
    let mut seen_field_names = BTreeSet::new();
    for (index, field, names, serde_default, arg_config) in prepared_fields {
        for candidate in std::iter::once(&names.schema_path).chain(names.schema_aliases.iter()) {
            for (other_index, other_names) in &all_field_names {
                if *other_index != index && other_names.contains(candidate.value().as_str()) {
                    return Err(syn::Error::new_spanned(
                        candidate,
                        format!(
                            "duplicate ToolInput field wire name or alias `{}`",
                            candidate.value()
                        ),
                    ));
                }
            }
        }
        ensure_unique_field_arg_names(
            &mut seen_field_names,
            &names.schema_path,
            &names.schema_aliases,
        )?;
        apply_field_arg_config_to_input(
            config,
            &field_path_lookup,
            &names.schema_path,
            &names.parse_path,
            &names.schema_aliases,
            &names.parse_aliases,
            &field.ty,
            serde_default,
            &arg_config,
        );
    }
    normalize_array_value_constraints(
        &mut config.trim,
        &mut config.trim_suffix,
        &mut config.minimums,
        &mut config.maximums,
        &mut config.exclusive_minimums,
        &mut config.exclusive_maximums,
        &mut config.min_properties,
        &mut config.max_properties,
        &mut config.min_chars,
        &mut config.max_chars,
        &mut config.formats,
        &mut config.patterns,
        &mut config.choices,
        &mut config.forbid_substrings,
        &mut config.distinct_trimmed,
        &mut config.input_field_metadata,
        &field_path_lookup,
        &array_field_paths,
    );
    Ok(())
}

fn parse_input_field_arg_attrs(field: &Field) -> Result<FieldArgConfig> {
    let mut config = FieldArgConfig::default();
    for attr in &field.attrs {
        if !attr.path().is_ident("arg") {
            continue;
        }
        match &attr.meta {
            Meta::Path(_) => {}
            Meta::NameValue(_) => {
                return Err(syn::Error::new_spanned(
                    attr,
                    "#[arg] supports list syntax, for example #[arg(trim, non_empty)]",
                ));
            }
            Meta::List(_) => parse_input_field_arg_config_attr(attr, &mut config)?,
        }
    }
    Ok(config)
}

fn apply_input_variant_field_arg_attrs(
    config: &mut ToolInputVariantConfig,
    variant: &Variant,
    rename_rule: Option<SerdeRenameRule>,
) -> Result<()> {
    let Fields::Named(fields) = &variant.fields else {
        for field in variant.fields.iter() {
            let arg_config = parse_input_field_arg_attrs(field)?;
            if arg_config_has_constraints(&arg_config) {
                return Err(syn::Error::new_spanned(
                    field,
                    "field-level #[arg(...)] on ToolInput enum variants is only supported on named fields",
                ));
            }
        }
        return Ok(());
    };
    let mut all_field_names = Vec::new();
    let (field_path_lookup, array_field_paths) =
        input_constraint_field_lookup(&variant.fields, rename_rule)?;
    let mut prepared_fields = Vec::new();
    for (index, field) in fields.named.iter().enumerate() {
        let arg_config = parse_input_field_arg_attrs(field)?;
        if let Some(names) = prepare_input_field_names(field, rename_rule, &arg_config)? {
            let mut accepted_names = BTreeSet::new();
            ensure_unique_field_arg_names(
                &mut accepted_names,
                &names.schema_path,
                &names.schema_aliases,
            )?;
            all_field_names.push((index, accepted_names));
        }
        if !arg_config_has_constraints(&arg_config) {
            continue;
        }
        let Some(names) = prepare_input_field_names(field, rename_rule, &arg_config)? else {
            return Err(syn::Error::new_spanned(
                field,
                "field-level #[arg(...)] cannot be used on flattened or skipped variant fields; put the constraint on the flattened input shape or remove the serde skip",
            ));
        };
        let serde_default = field_has_serde_default(field)?;
        if field_arg_has_default(&arg_config) && serde_default {
            return Err(syn::Error::new_spanned(
                field,
                "field-level #[arg(default)] and #[serde(default)] cannot be combined; keep one default source",
            ));
        }
        prepared_fields.push((index, field, names, serde_default, arg_config));
    }
    let mut seen_field_names = BTreeSet::new();
    for (index, field, names, serde_default, arg_config) in prepared_fields {
        for candidate in std::iter::once(&names.schema_path).chain(names.schema_aliases.iter()) {
            for (other_index, other_names) in &all_field_names {
                if *other_index != index && other_names.contains(candidate.value().as_str()) {
                    return Err(syn::Error::new_spanned(
                        candidate,
                        format!(
                            "duplicate ToolInput field wire name or alias `{}`",
                            candidate.value()
                        ),
                    ));
                }
            }
        }
        ensure_unique_field_arg_names(
            &mut seen_field_names,
            &names.schema_path,
            &names.schema_aliases,
        )?;
        apply_field_arg_config_to_input_variant(
            config,
            &field_path_lookup,
            &names.schema_path,
            &names.parse_path,
            &names.schema_aliases,
            &names.parse_aliases,
            &field.ty,
            serde_default,
            &arg_config,
        );
    }
    normalize_array_value_constraints(
        &mut config.trim,
        &mut config.trim_suffix,
        &mut config.minimums,
        &mut config.maximums,
        &mut config.exclusive_minimums,
        &mut config.exclusive_maximums,
        &mut config.min_properties,
        &mut config.max_properties,
        &mut config.min_chars,
        &mut config.max_chars,
        &mut config.formats,
        &mut config.patterns,
        &mut config.choices,
        &mut config.forbid_substrings,
        &mut config.distinct_trimmed,
        &mut config.input_field_metadata,
        &field_path_lookup,
        &array_field_paths,
    );
    Ok(())
}

fn normalized_input_variant_config(
    variant: &Variant,
    enum_field_rule: Option<SerdeRenameRule>,
) -> Result<ToolInputVariantConfig> {
    let mut config = parse_input_variant_config(variant)?;
    let variant_field_rule = serde_rename_all_rule(&variant.attrs)?.or(enum_field_rule);
    apply_input_variant_field_arg_attrs(&mut config, variant, variant_field_rule)?;
    let (field_path_lookup, array_field_paths) =
        input_constraint_field_lookup(&variant.fields, variant_field_rule)?;
    resolve_constraint_lit_paths(&mut config.trim, &field_path_lookup);
    resolve_constraint_string_paths(&mut config.trim_suffix, &field_path_lookup);
    resolve_constraint_lit_paths(&mut config.non_empty, &field_path_lookup);
    resolve_constraint_lit_paths(&mut config.non_empty_if_present, &field_path_lookup);
    resolve_constraint_expr_paths(&mut config.minimums, &field_path_lookup);
    resolve_constraint_expr_paths(&mut config.maximums, &field_path_lookup);
    resolve_constraint_expr_paths(&mut config.exclusive_minimums, &field_path_lookup);
    resolve_constraint_expr_paths(&mut config.exclusive_maximums, &field_path_lookup);
    resolve_constraint_group_paths(&mut config.exactly_one_of, &field_path_lookup);
    resolve_constraint_group_paths(&mut config.at_least_one_of, &field_path_lookup);
    resolve_constraint_pair_paths(&mut config.requires, &field_path_lookup);
    resolve_constraint_pair_paths(&mut config.conflicts_with, &field_path_lookup);
    resolve_constraint_pair_paths(&mut config.required_unless_present, &field_path_lookup);
    resolve_constraint_strings_paths(&mut config.forbid_substrings, &field_path_lookup);
    resolve_constraint_lit_paths(&mut config.distinct_trimmed, &field_path_lookup);
    resolve_constraint_pair_paths(&mut config.distinct_trimmed_within, &field_path_lookup);
    resolve_constraint_usize_paths(&mut config.min_items, &field_path_lookup);
    resolve_constraint_usize_paths(&mut config.max_items, &field_path_lookup);
    resolve_constraint_usize_paths(&mut config.min_properties, &field_path_lookup);
    resolve_constraint_usize_paths(&mut config.max_properties, &field_path_lookup);
    resolve_constraint_usize_paths(&mut config.min_chars, &field_path_lookup);
    resolve_constraint_usize_paths(&mut config.max_chars, &field_path_lookup);
    resolve_constraint_string_paths(&mut config.formats, &field_path_lookup);
    resolve_constraint_string_paths(&mut config.patterns, &field_path_lookup);
    resolve_constraint_values_paths(&mut config.choices, &field_path_lookup);
    normalize_array_value_nested_path_constraints(
        &mut config.non_empty,
        &mut config.non_empty_if_present,
        &mut config.exactly_one_of,
        &mut config.at_least_one_of,
        &mut config.requires,
        &mut config.conflicts_with,
        &mut config.required_unless_present,
        &mut config.distinct_trimmed_within,
        &field_path_lookup,
        &array_field_paths,
    );
    resolve_constraint_lit_paths(&mut config.infer_when_present, &field_path_lookup);
    resolve_constraint_lit_paths(&mut config.drop_keys, &field_path_lookup);
    normalize_array_value_lit_paths(
        &mut config.infer_when_present,
        &field_path_lookup,
        &array_field_paths,
    );
    normalize_array_value_lit_paths(
        &mut config.drop_keys,
        &field_path_lookup,
        &array_field_paths,
    );
    normalize_array_value_constraints(
        &mut config.trim,
        &mut config.trim_suffix,
        &mut config.minimums,
        &mut config.maximums,
        &mut config.exclusive_minimums,
        &mut config.exclusive_maximums,
        &mut config.min_properties,
        &mut config.max_properties,
        &mut config.min_chars,
        &mut config.max_chars,
        &mut config.formats,
        &mut config.patterns,
        &mut config.choices,
        &mut config.forbid_substrings,
        &mut config.distinct_trimmed,
        &mut config.input_field_metadata,
        &field_path_lookup,
        &array_field_paths,
    );
    Ok(config)
}

fn parse_input_field_arg_config_attr(attr: &Attribute, config: &mut FieldArgConfig) -> Result<()> {
    let args = attr.parse_args::<ArgAttrArgs>()?;
    for item in args.items {
        match (item.key.as_str(), item.value) {
            ("default", None) => config.default = true,
            ("default", Some(value)) => {
                if config.default || config.default_expr.replace(value).is_some() {
                    return Err(syn::Error::new_spanned(
                        item.first_ident,
                        "duplicate field #[arg(default)] or #[arg(default = ...)]",
                    ));
                }
            }
            ("trim", None) => config.trim = true,
            ("item_trim", None) => config.item_trim = true,
            ("non_empty", None) => config.non_empty = true,
            ("item_non_empty", None) => config.item_non_empty = true,
            ("non_empty_if_present", None) => config.non_empty_if_present = true,
            ("item_non_empty_if_present", None) => config.item_non_empty_if_present = true,
            ("distinct_trimmed", None) => config.distinct_trimmed = true,
            ("description", Some(value)) => {
                if config
                    .description
                    .replace(expr_lit_str(&value, "description")?)
                    .is_some()
                {
                    return Err(syn::Error::new_spanned(
                        item.first_ident,
                        "duplicate field #[arg(description = ...)]",
                    ));
                }
            }
            ("name", Some(value)) => {
                if config.name.replace(expr_lit_str(&value, "name")?).is_some() {
                    return Err(syn::Error::new_spanned(
                        item.first_ident,
                        "duplicate field #[arg(name = ...)]",
                    ));
                }
            }
            ("alias", Some(value)) => config.aliases.push(expr_lit_str(&value, "alias")?),
            ("path.read", None) => {
                set_field_arg_path_kind(config, PluginPathPermissionKind::Read, &item.first_ident)?
            }
            ("path.write", None) => {
                set_field_arg_path_kind(config, PluginPathPermissionKind::Write, &item.first_ident)?
            }
            ("network", None) => {
                set_field_arg_network(config, PluginNetworkSemantic::Network, &item.first_ident)?
            }
            ("network.url", None) => {
                set_field_arg_network(config, PluginNetworkSemantic::Url, &item.first_ident)?
            }
            ("network.host", None) => {
                set_field_arg_network(config, PluginNetworkSemantic::Host, &item.first_ident)?
            }
            ("network.internet", None) => {
                set_field_arg_network(config, PluginNetworkSemantic::Internet, &item.first_ident)?
            }
            ("network.private", None) => {
                set_field_arg_network(config, PluginNetworkSemantic::Private, &item.first_ident)?
            }
            ("optional", None) => config.optional = true,
            ("secret", None) => config.secret = true,
            ("file", None) => {
                set_field_arg_picker(config, PluginPickerKind::File, &item.first_ident)?
            }
            ("dir", None) => {
                set_field_arg_picker(config, PluginPickerKind::Dir, &item.first_ident)?
            }
            ("jsonpath", Some(value)) => {
                let jsonpath = expr_lit_str(&value, "jsonpath")?;
                validate_input_jsonpath_lit(&jsonpath)?;
                config.jsonpath = Some(jsonpath);
            }
            ("fallback", Some(value)) => config.fallback = Some(expr_lit_str(&value, "fallback")?),
            ("example", Some(value)) => config.example = Some(value),
            ("trim_suffix", Some(value)) => {
                config.trim_suffix = Some(expr_lit_str(&value, "trim_suffix")?)
            }
            ("item_trim_suffix", Some(value)) => {
                let suffix = expr_lit_str(&value, "item_trim_suffix")?;
                if config.item_trim_suffix.replace(suffix).is_some() {
                    return Err(syn::Error::new_spanned(
                        item.first_ident,
                        "duplicate field #[arg(item_trim_suffix = ...)]",
                    ));
                }
            }
            ("minimum", Some(value)) => {
                if config.minimum.replace(value).is_some() {
                    return Err(syn::Error::new_spanned(
                        item.first_ident,
                        "duplicate field #[arg(minimum = ...)]",
                    ));
                }
            }
            ("maximum", Some(value)) => {
                if config.maximum.replace(value).is_some() {
                    return Err(syn::Error::new_spanned(
                        item.first_ident,
                        "duplicate field #[arg(maximum = ...)]",
                    ));
                }
            }
            ("exclusive_minimum", Some(value)) => {
                if config.exclusive_minimum.replace(value).is_some() {
                    return Err(syn::Error::new_spanned(
                        item.first_ident,
                        "duplicate field #[arg(exclusive_minimum = ...)]",
                    ));
                }
            }
            ("exclusive_maximum", Some(value)) => {
                if config.exclusive_maximum.replace(value).is_some() {
                    return Err(syn::Error::new_spanned(
                        item.first_ident,
                        "duplicate field #[arg(exclusive_maximum = ...)]",
                    ));
                }
            }
            ("min_items", Some(value)) => {
                config.min_items = Some(expr_lit_usize(&value, "min_items")?)
            }
            ("max_items", Some(value)) => {
                config.max_items = Some(expr_lit_usize(&value, "max_items")?)
            }
            ("min_properties", Some(value)) => {
                config.min_properties = Some(expr_lit_usize(&value, "min_properties")?)
            }
            ("max_properties", Some(value)) => {
                config.max_properties = Some(expr_lit_usize(&value, "max_properties")?)
            }
            ("item_minimum", Some(value)) => {
                if config.item_minimum.replace(value).is_some() {
                    return Err(syn::Error::new_spanned(
                        item.first_ident,
                        "duplicate field #[arg(item_minimum = ...)]",
                    ));
                }
            }
            ("item_maximum", Some(value)) => {
                if config.item_maximum.replace(value).is_some() {
                    return Err(syn::Error::new_spanned(
                        item.first_ident,
                        "duplicate field #[arg(item_maximum = ...)]",
                    ));
                }
            }
            ("item_exclusive_minimum", Some(value)) => {
                if config.item_exclusive_minimum.replace(value).is_some() {
                    return Err(syn::Error::new_spanned(
                        item.first_ident,
                        "duplicate field #[arg(item_exclusive_minimum = ...)]",
                    ));
                }
            }
            ("item_exclusive_maximum", Some(value)) => {
                if config.item_exclusive_maximum.replace(value).is_some() {
                    return Err(syn::Error::new_spanned(
                        item.first_ident,
                        "duplicate field #[arg(item_exclusive_maximum = ...)]",
                    ));
                }
            }
            ("item_min_properties", Some(value)) => {
                config.item_min_properties = Some(expr_lit_usize(&value, "item_min_properties")?)
            }
            ("item_max_properties", Some(value)) => {
                config.item_max_properties = Some(expr_lit_usize(&value, "item_max_properties")?)
            }
            ("item_min_chars", Some(value)) => {
                config.item_min_chars = Some(expr_lit_usize(&value, "item_min_chars")?)
            }
            ("item_max_chars", Some(value)) => {
                config.item_max_chars = Some(expr_lit_usize(&value, "item_max_chars")?)
            }
            ("min_chars", Some(value)) => {
                config.min_chars = Some(expr_lit_usize(&value, "min_chars")?)
            }
            ("max_chars", Some(value)) => {
                config.max_chars = Some(expr_lit_usize(&value, "max_chars")?)
            }
            ("item_format", Some(value)) => {
                let format = validate_format_lit(&expr_lit_str(&value, "item_format")?)?;
                if config.item_format.replace(format).is_some() {
                    return Err(syn::Error::new_spanned(
                        item.first_ident,
                        "duplicate field #[arg(item_format = ...)]",
                    ));
                }
            }
            ("item_pattern", Some(value)) => {
                let pattern = expr_lit_str(&value, "item_pattern")?;
                validate_pattern_lit(&pattern)?;
                if config.item_pattern.replace(pattern).is_some() {
                    return Err(syn::Error::new_spanned(
                        item.first_ident,
                        "duplicate field #[arg(item_pattern = ...)]",
                    ));
                }
            }
            ("format", Some(value)) => {
                let format = validate_format_lit(&expr_lit_str(&value, "format")?)?;
                if config.format.replace(format).is_some() {
                    return Err(syn::Error::new_spanned(
                        item.first_ident,
                        "duplicate field #[arg(format = ...)]",
                    ));
                }
            }
            ("item_choices", Some(value)) => {
                if config
                    .item_choices
                    .replace(expr_array_values(&value, "item_choices")?)
                    .is_some()
                {
                    return Err(syn::Error::new_spanned(
                        item.first_ident,
                        "duplicate field #[arg(item_choices = [...])]",
                    ));
                }
            }
            ("exactly_one_of", Some(value)) => config
                .exactly_one_of
                .extend(expr_array_lit_strs(&value, "exactly_one_of")?),
            ("at_least_one_of", Some(value)) => config
                .at_least_one_of
                .extend(expr_array_lit_strs(&value, "at_least_one_of")?),
            ("requires", Some(value)) => config.requires.push(expr_lit_str(&value, "requires")?),
            ("conflicts_with", Some(value)) => config
                .conflicts_with
                .push(expr_lit_str(&value, "conflicts_with")?),
            ("required_unless_present", Some(value)) => config
                .required_unless_present
                .push(expr_lit_str(&value, "required_unless_present")?),
            ("forbid_substrings", Some(value)) => config
                .forbid_substrings
                .extend(expr_array_lit_strs(&value, "forbid_substrings")?),
            ("distinct_trimmed_within", Some(value)) => config
                .distinct_trimmed_within
                .push(expr_lit_str(&value, "distinct_trimmed_within")?),
            ("pattern", Some(value)) => {
                let pattern = expr_lit_str(&value, "pattern")?;
                validate_pattern_lit(&pattern)?;
                if config.pattern.replace(pattern).is_some() {
                    return Err(syn::Error::new_spanned(
                        item.first_ident,
                        "duplicate field #[arg(pattern = ...)]",
                    ));
                }
            }
            ("choices", Some(value)) => {
                if config
                    .choices
                    .replace(expr_array_values(&value, "choices")?)
                    .is_some()
                {
                    return Err(syn::Error::new_spanned(
                        item.first_ident,
                        "duplicate field #[arg(choices = [...])]",
                    ));
                }
            }
            (key, Some(_)) => {
                return Err(syn::Error::new_spanned(
                    item.first_ident,
                    format!("unsupported field #[arg] option '{key}'"),
                ));
            }
            (key, None) => {
                return Err(syn::Error::new_spanned(
                    item.first_ident,
                    format!("unsupported field #[arg] flag '{key}'"),
                ));
            }
        }
    }
    ensure_arg_permission_locator_has_semantic(
        config.jsonpath.as_ref(),
        config.fallback.as_ref(),
        config.path.is_some() || config.network.is_some(),
    )?;
    Ok(())
}

fn arg_config_has_constraints(config: &FieldArgConfig) -> bool {
    config.default
        || config.default_expr.is_some()
        || config.description.is_some()
        || config.name.is_some()
        || !config.aliases.is_empty()
        || config.trim
        || config.item_trim
        || config.non_empty
        || config.item_non_empty
        || config.non_empty_if_present
        || config.item_non_empty_if_present
        || config.trim_suffix.is_some()
        || config.item_trim_suffix.is_some()
        || config.minimum.is_some()
        || config.maximum.is_some()
        || config.exclusive_minimum.is_some()
        || config.exclusive_maximum.is_some()
        || config.min_items.is_some()
        || config.max_items.is_some()
        || config.min_properties.is_some()
        || config.max_properties.is_some()
        || config.item_minimum.is_some()
        || config.item_maximum.is_some()
        || config.item_exclusive_minimum.is_some()
        || config.item_exclusive_maximum.is_some()
        || config.item_min_properties.is_some()
        || config.item_max_properties.is_some()
        || config.min_chars.is_some()
        || config.max_chars.is_some()
        || config.item_min_chars.is_some()
        || config.item_max_chars.is_some()
        || config.format.is_some()
        || config.item_format.is_some()
        || config.item_pattern.is_some()
        || config.pattern.is_some()
        || config.choices.is_some()
        || config.item_choices.is_some()
        || !config.requires.is_empty()
        || !config.conflicts_with.is_empty()
        || !config.required_unless_present.is_empty()
        || !config.forbid_substrings.is_empty()
        || config.distinct_trimmed
        || !config.exactly_one_of.is_empty()
        || !config.at_least_one_of.is_empty()
        || !config.distinct_trimmed_within.is_empty()
        || config.path.is_some()
        || config.network.is_some()
        || config.optional
        || config.jsonpath.is_some()
        || config.fallback.is_some()
        || config.example.is_some()
        || config.secret
        || config.picker.is_some()
}

fn inline_arg_has_default(config: &PluginArgConfig) -> bool {
    config.default || config.default_expr.is_some()
}

fn field_arg_has_default(config: &FieldArgConfig) -> bool {
    config.default || config.default_expr.is_some()
}

fn apply_field_arg_config_to_input(
    target: &mut ToolInputConfig,
    field_path_lookup: &BTreeMap<String, LitStr>,
    schema_path: &LitStr,
    parse_path: &LitStr,
    schema_aliases: &[LitStr],
    parse_aliases: &[LitStr],
    ty: &Type,
    serde_default: bool,
    config: &FieldArgConfig,
) {
    if config.trim {
        target.trim.push(parse_path.clone());
    }
    if config.item_trim {
        target
            .trim
            .push(append_constraint_path_suffix(parse_path, "[]"));
    }
    if config.non_empty {
        target.non_empty.push(parse_path.clone());
    }
    if config.item_non_empty {
        target
            .non_empty
            .push(append_constraint_path_suffix(parse_path, "[]"));
    }
    if config.non_empty_if_present {
        target.non_empty_if_present.push(parse_path.clone());
    }
    if config.item_non_empty_if_present {
        target
            .non_empty_if_present
            .push(append_constraint_path_suffix(parse_path, "[]"));
    }
    if let Some(value) = config.trim_suffix.as_ref() {
        target.trim_suffix.push(PathStringConstraint {
            path: parse_path.clone(),
            value: value.clone(),
        });
    }
    if let Some(value) = config.item_trim_suffix.as_ref() {
        target.trim_suffix.push(PathStringConstraint {
            path: append_constraint_path_suffix(parse_path, "[]"),
            value: value.clone(),
        });
    }
    if let Some(value) = config.minimum.as_ref() {
        target.minimums.push(PathValueConstraint {
            path: parse_path.clone(),
            value: value.clone(),
        });
    }
    if let Some(value) = config.maximum.as_ref() {
        target.maximums.push(PathValueConstraint {
            path: parse_path.clone(),
            value: value.clone(),
        });
    }
    if let Some(value) = config.exclusive_minimum.as_ref() {
        target.exclusive_minimums.push(PathValueConstraint {
            path: parse_path.clone(),
            value: value.clone(),
        });
    }
    if let Some(value) = config.exclusive_maximum.as_ref() {
        target.exclusive_maximums.push(PathValueConstraint {
            path: parse_path.clone(),
            value: value.clone(),
        });
    }
    if let Some(value) = config.min_items {
        target.min_items.push(PathUsizeConstraint {
            path: parse_path.clone(),
            value,
        });
    }
    if let Some(value) = config.max_items {
        target.max_items.push(PathUsizeConstraint {
            path: parse_path.clone(),
            value,
        });
    }
    if let Some(value) = config.min_properties {
        target.min_properties.push(PathUsizeConstraint {
            path: parse_path.clone(),
            value,
        });
    }
    if let Some(value) = config.max_properties {
        target.max_properties.push(PathUsizeConstraint {
            path: parse_path.clone(),
            value,
        });
    }
    if let Some(value) = config.item_minimum.as_ref() {
        target.minimums.push(PathValueConstraint {
            path: append_constraint_path_suffix(parse_path, "[]"),
            value: value.clone(),
        });
    }
    if let Some(value) = config.item_maximum.as_ref() {
        target.maximums.push(PathValueConstraint {
            path: append_constraint_path_suffix(parse_path, "[]"),
            value: value.clone(),
        });
    }
    if let Some(value) = config.item_exclusive_minimum.as_ref() {
        target.exclusive_minimums.push(PathValueConstraint {
            path: append_constraint_path_suffix(parse_path, "[]"),
            value: value.clone(),
        });
    }
    if let Some(value) = config.item_exclusive_maximum.as_ref() {
        target.exclusive_maximums.push(PathValueConstraint {
            path: append_constraint_path_suffix(parse_path, "[]"),
            value: value.clone(),
        });
    }
    if let Some(value) = config.item_min_properties {
        target.min_properties.push(PathUsizeConstraint {
            path: append_constraint_path_suffix(parse_path, "[]"),
            value,
        });
    }
    if let Some(value) = config.item_max_properties {
        target.max_properties.push(PathUsizeConstraint {
            path: append_constraint_path_suffix(parse_path, "[]"),
            value,
        });
    }
    if let Some(value) = config.min_chars {
        target.min_chars.push(PathUsizeConstraint {
            path: parse_path.clone(),
            value,
        });
    }
    if let Some(value) = config.max_chars {
        target.max_chars.push(PathUsizeConstraint {
            path: parse_path.clone(),
            value,
        });
    }
    if let Some(value) = config.item_min_chars {
        target.min_chars.push(PathUsizeConstraint {
            path: append_constraint_path_suffix(parse_path, "[]"),
            value,
        });
    }
    if let Some(value) = config.item_max_chars {
        target.max_chars.push(PathUsizeConstraint {
            path: append_constraint_path_suffix(parse_path, "[]"),
            value,
        });
    }
    if let Some(value) = config.format.as_ref() {
        target.formats.push(PathStringConstraint {
            path: parse_path.clone(),
            value: value.clone(),
        });
    }
    if let Some(value) = config.item_format.as_ref() {
        target.formats.push(PathStringConstraint {
            path: append_constraint_path_suffix(parse_path, "[]"),
            value: value.clone(),
        });
    }
    if let Some(value) = config.pattern.as_ref() {
        target.patterns.push(PathStringConstraint {
            path: parse_path.clone(),
            value: value.clone(),
        });
    }
    if let Some(value) = config.item_pattern.as_ref() {
        target.patterns.push(PathStringConstraint {
            path: append_constraint_path_suffix(parse_path, "[]"),
            value: value.clone(),
        });
    }
    if let Some(values) = config.choices.as_ref() {
        target.choices.push(PathValuesConstraint {
            path: parse_path.clone(),
            values: values.clone(),
        });
    }
    if let Some(values) = config.item_choices.as_ref() {
        target.choices.push(PathValuesConstraint {
            path: append_constraint_path_suffix(parse_path, "[]"),
            values: values.clone(),
        });
    }
    let type_shape = input_type_semantic_shape(ty);
    let field_value_path = if type_shape.array {
        append_constraint_path_suffix(parse_path, "[]")
    } else {
        parse_path.clone()
    };
    target
        .requires
        .extend(config.requires.iter().map(|right| PathPairConstraint {
            left: parse_path.clone(),
            right: resolve_known_constraint_path(right, Some(field_path_lookup)),
        }));
    target.conflicts_with.extend(
        config
            .conflicts_with
            .iter()
            .map(|right| PathPairConstraint {
                left: parse_path.clone(),
                right: resolve_known_constraint_path(right, Some(field_path_lookup)),
            }),
    );
    target
        .required_unless_present
        .extend(
            config
                .required_unless_present
                .iter()
                .map(|right| PathPairConstraint {
                    left: parse_path.clone(),
                    right: resolve_known_constraint_path(right, Some(field_path_lookup)),
                }),
        );
    if !config.forbid_substrings.is_empty() {
        target.forbid_substrings.push(PathStringsConstraint {
            path: field_value_path.clone(),
            values: config.forbid_substrings.clone(),
        });
    }
    if config.distinct_trimmed {
        target.distinct_trimmed.push(field_value_path.clone());
    }
    if !config.exactly_one_of.is_empty() {
        target.exactly_one_of.push(prefixed_constraint_group(
            parse_path,
            &config.exactly_one_of,
            Some(field_path_lookup),
        ));
    }
    if !config.at_least_one_of.is_empty() {
        target.at_least_one_of.push(prefixed_constraint_group(
            parse_path,
            &config.at_least_one_of,
            Some(field_path_lookup),
        ));
    }
    target
        .distinct_trimmed_within
        .extend(
            config
                .distinct_trimmed_within
                .iter()
                .map(|right| PathPairConstraint {
                    left: parse_path.clone(),
                    right: resolve_known_constraint_path(right, Some(field_path_lookup)),
                }),
        );
    let optional = config.optional
        || serde_default
        || field_arg_has_default(config)
        || type_shape.optional
        || !schema_aliases.is_empty();
    let jsonpath = input_jsonpath_for_arg(schema_path, ty, config.jsonpath.as_ref());
    if let Some(kind) = config.path {
        target.input_paths.push(PluginInputPathSpec {
            jsonpath: jsonpath.clone(),
            kind,
            fallback: config.fallback.clone(),
            optional,
        });
        if config.jsonpath.is_none() {
            target
                .input_paths
                .extend(schema_aliases.iter().map(|alias| PluginInputPathSpec {
                    jsonpath: input_jsonpath_for_field(alias, ty),
                    kind,
                    fallback: config.fallback.clone(),
                    optional,
                }));
        }
    }
    if let Some(semantic) = config.network {
        target.input_networks.push(PluginInputNetworkSpec {
            jsonpath,
            fallback: config.fallback.clone(),
            optional,
            semantic,
        });
        if config.jsonpath.is_none() {
            target
                .input_networks
                .extend(schema_aliases.iter().map(|alias| PluginInputNetworkSpec {
                    jsonpath: input_jsonpath_for_field(alias, ty),
                    fallback: config.fallback.clone(),
                    optional,
                    semantic,
                }));
        }
    }
    apply_arg_metadata_to_spec(
        &mut target.input_field_metadata,
        schema_path,
        parse_path,
        schema_aliases,
        config.description.clone(),
        config.path,
        config.network,
        config.non_empty || config.non_empty_if_present,
        config.item_non_empty,
        config.item_non_empty_if_present,
        config.minimum.clone(),
        config.maximum.clone(),
        config.exclusive_minimum.clone(),
        config.exclusive_maximum.clone(),
        config.min_items,
        config.max_items,
        config.min_properties,
        config.max_properties,
        config.item_minimum.clone(),
        config.item_maximum.clone(),
        config.item_exclusive_minimum.clone(),
        config.item_exclusive_maximum.clone(),
        config.item_min_properties,
        config.item_max_properties,
        config.min_chars,
        config.max_chars,
        config.item_min_chars,
        config.item_max_chars,
        config.format.clone(),
        config.item_format.clone(),
        config.pattern.clone(),
        config.item_pattern.clone(),
        config.example.clone(),
        config.choices.clone().unwrap_or_default(),
        config.item_choices.clone().unwrap_or_default(),
        config.secret,
        config.picker,
    );
    apply_arg_default_to_spec(
        &mut target.input_defaults,
        schema_path,
        parse_path,
        parse_aliases,
        ty,
        config.default,
        config.default_expr.clone(),
    );
    apply_arg_aliases_to_spec(&mut target.input_aliases, parse_path, parse_aliases);
}

fn apply_field_arg_config_to_input_variant(
    target: &mut ToolInputVariantConfig,
    field_path_lookup: &BTreeMap<String, LitStr>,
    schema_path: &LitStr,
    parse_path: &LitStr,
    schema_aliases: &[LitStr],
    parse_aliases: &[LitStr],
    ty: &Type,
    serde_default: bool,
    config: &FieldArgConfig,
) {
    if config.trim {
        target.trim.push(parse_path.clone());
    }
    if config.item_trim {
        target
            .trim
            .push(append_constraint_path_suffix(parse_path, "[]"));
    }
    if config.non_empty {
        target.non_empty.push(parse_path.clone());
    }
    if config.item_non_empty {
        target
            .non_empty
            .push(append_constraint_path_suffix(parse_path, "[]"));
    }
    if config.non_empty_if_present {
        target.non_empty_if_present.push(parse_path.clone());
    }
    if config.item_non_empty_if_present {
        target
            .non_empty_if_present
            .push(append_constraint_path_suffix(parse_path, "[]"));
    }
    if let Some(value) = config.trim_suffix.as_ref() {
        target.trim_suffix.push(PathStringConstraint {
            path: parse_path.clone(),
            value: value.clone(),
        });
    }
    if let Some(value) = config.item_trim_suffix.as_ref() {
        target.trim_suffix.push(PathStringConstraint {
            path: append_constraint_path_suffix(parse_path, "[]"),
            value: value.clone(),
        });
    }
    if let Some(value) = config.minimum.as_ref() {
        target.minimums.push(PathValueConstraint {
            path: parse_path.clone(),
            value: value.clone(),
        });
    }
    if let Some(value) = config.maximum.as_ref() {
        target.maximums.push(PathValueConstraint {
            path: parse_path.clone(),
            value: value.clone(),
        });
    }
    if let Some(value) = config.exclusive_minimum.as_ref() {
        target.exclusive_minimums.push(PathValueConstraint {
            path: parse_path.clone(),
            value: value.clone(),
        });
    }
    if let Some(value) = config.exclusive_maximum.as_ref() {
        target.exclusive_maximums.push(PathValueConstraint {
            path: parse_path.clone(),
            value: value.clone(),
        });
    }
    if let Some(value) = config.min_items {
        target.min_items.push(PathUsizeConstraint {
            path: parse_path.clone(),
            value,
        });
    }
    if let Some(value) = config.max_items {
        target.max_items.push(PathUsizeConstraint {
            path: parse_path.clone(),
            value,
        });
    }
    if let Some(value) = config.min_properties {
        target.min_properties.push(PathUsizeConstraint {
            path: parse_path.clone(),
            value,
        });
    }
    if let Some(value) = config.max_properties {
        target.max_properties.push(PathUsizeConstraint {
            path: parse_path.clone(),
            value,
        });
    }
    if let Some(value) = config.item_minimum.as_ref() {
        target.minimums.push(PathValueConstraint {
            path: append_constraint_path_suffix(parse_path, "[]"),
            value: value.clone(),
        });
    }
    if let Some(value) = config.item_maximum.as_ref() {
        target.maximums.push(PathValueConstraint {
            path: append_constraint_path_suffix(parse_path, "[]"),
            value: value.clone(),
        });
    }
    if let Some(value) = config.item_exclusive_minimum.as_ref() {
        target.exclusive_minimums.push(PathValueConstraint {
            path: append_constraint_path_suffix(parse_path, "[]"),
            value: value.clone(),
        });
    }
    if let Some(value) = config.item_exclusive_maximum.as_ref() {
        target.exclusive_maximums.push(PathValueConstraint {
            path: append_constraint_path_suffix(parse_path, "[]"),
            value: value.clone(),
        });
    }
    if let Some(value) = config.item_min_properties {
        target.min_properties.push(PathUsizeConstraint {
            path: append_constraint_path_suffix(parse_path, "[]"),
            value,
        });
    }
    if let Some(value) = config.item_max_properties {
        target.max_properties.push(PathUsizeConstraint {
            path: append_constraint_path_suffix(parse_path, "[]"),
            value,
        });
    }
    if let Some(value) = config.min_chars {
        target.min_chars.push(PathUsizeConstraint {
            path: parse_path.clone(),
            value,
        });
    }
    if let Some(value) = config.max_chars {
        target.max_chars.push(PathUsizeConstraint {
            path: parse_path.clone(),
            value,
        });
    }
    if let Some(value) = config.item_min_chars {
        target.min_chars.push(PathUsizeConstraint {
            path: append_constraint_path_suffix(parse_path, "[]"),
            value,
        });
    }
    if let Some(value) = config.item_max_chars {
        target.max_chars.push(PathUsizeConstraint {
            path: append_constraint_path_suffix(parse_path, "[]"),
            value,
        });
    }
    if let Some(value) = config.format.as_ref() {
        target.formats.push(PathStringConstraint {
            path: parse_path.clone(),
            value: value.clone(),
        });
    }
    if let Some(value) = config.item_format.as_ref() {
        target.formats.push(PathStringConstraint {
            path: append_constraint_path_suffix(parse_path, "[]"),
            value: value.clone(),
        });
    }
    if let Some(value) = config.pattern.as_ref() {
        target.patterns.push(PathStringConstraint {
            path: parse_path.clone(),
            value: value.clone(),
        });
    }
    if let Some(value) = config.item_pattern.as_ref() {
        target.patterns.push(PathStringConstraint {
            path: append_constraint_path_suffix(parse_path, "[]"),
            value: value.clone(),
        });
    }
    if let Some(values) = config.choices.as_ref() {
        target.choices.push(PathValuesConstraint {
            path: parse_path.clone(),
            values: values.clone(),
        });
    }
    if let Some(values) = config.item_choices.as_ref() {
        target.choices.push(PathValuesConstraint {
            path: append_constraint_path_suffix(parse_path, "[]"),
            values: values.clone(),
        });
    }
    let type_shape = input_type_semantic_shape(ty);
    let field_value_path = if type_shape.array {
        append_constraint_path_suffix(parse_path, "[]")
    } else {
        parse_path.clone()
    };
    target
        .requires
        .extend(config.requires.iter().map(|right| PathPairConstraint {
            left: parse_path.clone(),
            right: resolve_known_constraint_path(right, Some(field_path_lookup)),
        }));
    target.conflicts_with.extend(
        config
            .conflicts_with
            .iter()
            .map(|right| PathPairConstraint {
                left: parse_path.clone(),
                right: resolve_known_constraint_path(right, Some(field_path_lookup)),
            }),
    );
    target
        .required_unless_present
        .extend(
            config
                .required_unless_present
                .iter()
                .map(|right| PathPairConstraint {
                    left: parse_path.clone(),
                    right: resolve_known_constraint_path(right, Some(field_path_lookup)),
                }),
        );
    if !config.forbid_substrings.is_empty() {
        target.forbid_substrings.push(PathStringsConstraint {
            path: field_value_path.clone(),
            values: config.forbid_substrings.clone(),
        });
    }
    if config.distinct_trimmed {
        target.distinct_trimmed.push(field_value_path.clone());
    }
    if !config.exactly_one_of.is_empty() {
        target.exactly_one_of.push(prefixed_constraint_group(
            parse_path,
            &config.exactly_one_of,
            Some(field_path_lookup),
        ));
    }
    if !config.at_least_one_of.is_empty() {
        target.at_least_one_of.push(prefixed_constraint_group(
            parse_path,
            &config.at_least_one_of,
            Some(field_path_lookup),
        ));
    }
    target
        .distinct_trimmed_within
        .extend(
            config
                .distinct_trimmed_within
                .iter()
                .map(|right| PathPairConstraint {
                    left: parse_path.clone(),
                    right: resolve_known_constraint_path(right, Some(field_path_lookup)),
                }),
        );
    let optional = config.optional
        || serde_default
        || field_arg_has_default(config)
        || type_shape.optional
        || !schema_aliases.is_empty();
    let jsonpath = input_jsonpath_for_arg(schema_path, ty, config.jsonpath.as_ref());
    if let Some(kind) = config.path {
        target.input_paths.push(PluginInputPathSpec {
            jsonpath: jsonpath.clone(),
            kind,
            fallback: config.fallback.clone(),
            optional,
        });
        if config.jsonpath.is_none() {
            target
                .input_paths
                .extend(schema_aliases.iter().map(|alias| PluginInputPathSpec {
                    jsonpath: input_jsonpath_for_field(alias, ty),
                    kind,
                    fallback: config.fallback.clone(),
                    optional,
                }));
        }
    }
    if let Some(semantic) = config.network {
        target.input_networks.push(PluginInputNetworkSpec {
            jsonpath,
            fallback: config.fallback.clone(),
            optional,
            semantic,
        });
        if config.jsonpath.is_none() {
            target
                .input_networks
                .extend(schema_aliases.iter().map(|alias| PluginInputNetworkSpec {
                    jsonpath: input_jsonpath_for_field(alias, ty),
                    fallback: config.fallback.clone(),
                    optional,
                    semantic,
                }));
        }
    }
    apply_arg_metadata_to_spec(
        &mut target.input_field_metadata,
        schema_path,
        parse_path,
        schema_aliases,
        config.description.clone(),
        config.path,
        config.network,
        config.non_empty || config.non_empty_if_present,
        config.item_non_empty,
        config.item_non_empty_if_present,
        config.minimum.clone(),
        config.maximum.clone(),
        config.exclusive_minimum.clone(),
        config.exclusive_maximum.clone(),
        config.min_items,
        config.max_items,
        config.min_properties,
        config.max_properties,
        config.item_minimum.clone(),
        config.item_maximum.clone(),
        config.item_exclusive_minimum.clone(),
        config.item_exclusive_maximum.clone(),
        config.item_min_properties,
        config.item_max_properties,
        config.min_chars,
        config.max_chars,
        config.item_min_chars,
        config.item_max_chars,
        config.format.clone(),
        config.item_format.clone(),
        config.pattern.clone(),
        config.item_pattern.clone(),
        config.example.clone(),
        config.choices.clone().unwrap_or_default(),
        config.item_choices.clone().unwrap_or_default(),
        config.secret,
        config.picker,
    );
    apply_arg_default_to_spec(
        &mut target.input_defaults,
        schema_path,
        parse_path,
        parse_aliases,
        ty,
        config.default,
        config.default_expr.clone(),
    );
    apply_arg_aliases_to_spec(&mut target.input_aliases, parse_path, parse_aliases);
}

fn apply_arg_default_to_spec(
    target: &mut Vec<PluginInputFieldDefaultSpec>,
    schema_path: &LitStr,
    parse_path: &LitStr,
    aliases: &[LitStr],
    ty: &Type,
    default: bool,
    default_expr: Option<Expr>,
) {
    if !default && default_expr.is_none() {
        return;
    }
    target.push(PluginInputFieldDefaultSpec {
        schema_path: schema_path.clone(),
        parse_path: parse_path.clone(),
        aliases: aliases.to_vec(),
        ty: ty.clone(),
        default_expr,
    });
}

fn apply_arg_aliases_to_spec(
    target: &mut Vec<PluginInputFieldAliasSpec>,
    field_name: &LitStr,
    aliases: &[LitStr],
) {
    if aliases.is_empty() {
        return;
    }
    target.push(PluginInputFieldAliasSpec {
        path: field_name.clone(),
        aliases: aliases.to_vec(),
    });
}

fn apply_arg_metadata_to_spec(
    target: &mut Vec<PluginInputFieldMetadata>,
    schema_path: &LitStr,
    parse_path: &LitStr,
    aliases: &[LitStr],
    description: Option<LitStr>,
    path_kind: Option<PluginPathPermissionKind>,
    network: Option<PluginNetworkSemantic>,
    non_empty: bool,
    item_non_empty: bool,
    item_non_empty_if_present: bool,
    minimum: Option<Expr>,
    maximum: Option<Expr>,
    exclusive_minimum: Option<Expr>,
    exclusive_maximum: Option<Expr>,
    min_items: Option<usize>,
    max_items: Option<usize>,
    min_properties: Option<usize>,
    max_properties: Option<usize>,
    item_minimum: Option<Expr>,
    item_maximum: Option<Expr>,
    item_exclusive_minimum: Option<Expr>,
    item_exclusive_maximum: Option<Expr>,
    item_min_properties: Option<usize>,
    item_max_properties: Option<usize>,
    min_chars: Option<usize>,
    max_chars: Option<usize>,
    item_min_chars: Option<usize>,
    item_max_chars: Option<usize>,
    format: Option<LitStr>,
    item_format: Option<LitStr>,
    pattern: Option<LitStr>,
    item_pattern: Option<LitStr>,
    example: Option<Expr>,
    choices: Vec<Expr>,
    item_choices: Vec<Expr>,
    secret: bool,
    picker: Option<PluginPickerKind>,
) {
    if description.is_none()
        && path_kind.is_none()
        && network.is_none()
        && !non_empty
        && !item_non_empty
        && !item_non_empty_if_present
        && minimum.is_none()
        && maximum.is_none()
        && exclusive_minimum.is_none()
        && exclusive_maximum.is_none()
        && min_items.is_none()
        && max_items.is_none()
        && min_properties.is_none()
        && max_properties.is_none()
        && item_minimum.is_none()
        && item_maximum.is_none()
        && item_exclusive_minimum.is_none()
        && item_exclusive_maximum.is_none()
        && item_min_properties.is_none()
        && item_max_properties.is_none()
        && min_chars.is_none()
        && max_chars.is_none()
        && item_min_chars.is_none()
        && item_max_chars.is_none()
        && format.is_none()
        && item_format.is_none()
        && pattern.is_none()
        && item_pattern.is_none()
        && example.is_none()
        && choices.is_empty()
        && item_choices.is_empty()
        && aliases.is_empty()
        && !secret
        && picker.is_none()
    {
        return;
    }
    target.push(PluginInputFieldMetadata {
        path: schema_path.clone(),
        parse_path: parse_path.clone(),
        aliases: aliases.to_vec(),
        description,
        path_kind,
        network,
        non_empty,
        item_non_empty,
        item_non_empty_if_present,
        minimum,
        maximum,
        exclusive_minimum,
        exclusive_maximum,
        min_items,
        max_items,
        min_properties,
        max_properties,
        item_minimum,
        item_maximum,
        item_exclusive_minimum,
        item_exclusive_maximum,
        item_min_properties,
        item_max_properties,
        min_chars,
        max_chars,
        item_min_chars,
        item_max_chars,
        format,
        item_format,
        pattern,
        item_pattern,
        example,
        choices,
        item_choices,
        secret,
        picker,
    });
}

fn ensure_plugin_method_shared_receiver(method: &ImplItemFn, label: &str) -> Result<()> {
    if plugin_method_has_shared_receiver(method) {
        return Ok(());
    }
    Err(syn::Error::new_spanned(
        &method.sig,
        format!("{label} must be inherent methods with `&self` receiver"),
    ))
}

fn plugin_method_has_shared_receiver(method: &ImplItemFn) -> bool {
    matches!(
        method.sig.inputs.first(),
        Some(FnArg::Receiver(receiver))
            if receiver.reference.is_some() && receiver.mutability.is_none()
    )
}

fn stream_sink_is_edge_info(info: &PluginMethodInfo, label: &str) -> Result<bool> {
    let sink_positions = info
        .typed_args
        .iter()
        .enumerate()
        .filter_map(|(index, ty)| type_last_segment_is(ty, "ToolStreamSink").then_some(index))
        .collect::<Vec<_>>();
    let [sink_index] = sink_positions.as_slice() else {
        return Err(syn::Error::new_spanned(
            &info.ident,
            format!("{label} must include exactly one ToolStreamSink argument"),
        ));
    };
    if *sink_index == 0 {
        return Ok(true);
    }
    if *sink_index + 1 == info.typed_args.len() {
        return Ok(false);
    }
    Err(syn::Error::new_spanned(
        &info.ident,
        format!("{label} must put ToolStreamSink either first or last"),
    ))
}

fn typed_arg_types(method: &ImplItemFn) -> Vec<Type> {
    typed_arg_types_from_inputs(&method.sig.inputs)
}

fn typed_arg_types_from_inputs(inputs: &Punctuated<FnArg, Token![,]>) -> Vec<Type> {
    inputs
        .iter()
        .filter_map(|arg| match arg {
            FnArg::Receiver(_) => None,
            FnArg::Typed(pat_type) => Some((*pat_type.ty).clone()),
        })
        .collect()
}

fn type_last_segment_is(ty: &Type, expected: &str) -> bool {
    let ty = match ty {
        Type::Reference(reference) => reference.elem.as_ref(),
        other => other,
    };
    let Type::Path(path) = ty else { return false };
    path.path
        .segments
        .last()
        .is_some_and(|segment| segment.ident == expected)
}

fn type_mentions_segment(ty: &Type, expected: &str) -> bool {
    type_key(ty).contains(expected)
}

fn type_is_tool_invoke_context(ty: &Type) -> bool {
    type_last_segment_is(ty, "ToolInvokeContext")
}

fn type_is_plugin_command_context(ty: &Type) -> bool {
    type_last_segment_is(ty, "PluginCommandContext")
}

fn type_is_reference(ty: &Type) -> bool {
    matches!(ty, Type::Reference(_))
}

fn type_without_reference(ty: &Type) -> Type {
    match ty {
        Type::Reference(reference) => (*reference.elem).clone(),
        other => other.clone(),
    }
}

fn type_is_unit(ty: &Type) -> bool {
    matches!(ty, Type::Tuple(tuple) if tuple.elems.is_empty())
}

fn types_equivalent(left: &Type, right: &Type) -> bool {
    type_key(left) == type_key(right)
}

fn type_display(ty: &Type) -> String {
    quote! { #ty }.to_string()
}

fn type_key(ty: &Type) -> String {
    type_display(ty)
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect()
}

fn build_plugin_hook_plan(
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

struct PluginHookAttrConfig {
    hook: PluginHookKind,
    priority: i32,
    filters: PluginHookFilters,
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
                    filters.tools.extend(
                        Punctuated::<LitStr, Token![,]>::parse_terminated(&content)?.into_iter(),
                    );
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
                    filters.commands.extend(
                        Punctuated::<LitStr, Token![,]>::parse_terminated(&content)?.into_iter(),
                    );
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

fn parse_plugin_hook_attr(attr: &Attribute, _method_ident: &Ident) -> Result<PluginHookAttrConfig> {
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

fn validate_plugin_hook_filters(hook: PluginHookKind, filters: &PluginHookFilters) -> Result<()> {
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

enum PluginHookExpectedInput {
    None,
    Single(&'static str),
    Init,
}

enum PluginHookExpectedOutput {
    Unit,
    Init,
    Option(&'static str),
}

fn plugin_hook_input_segment(hook: PluginHookKind) -> PluginHookExpectedInput {
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

fn plugin_hook_output_segment(hook: PluginHookKind) -> PluginHookExpectedOutput {
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

fn validate_plugin_hook_output(method: &ImplItemFn, hook: PluginHookKind) -> Result<()> {
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

fn plugin_hook_name(hook: PluginHookKind) -> &'static str {
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

fn reject_duplicate_init_hooks(hooks: &[PluginHookPlan]) -> Result<()> {
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

fn reject_duplicate_tool_plans(tools: &[PluginToolPlan]) -> Result<()> {
    for (index, tool) in tools.iter().enumerate() {
        let name = &tool.tool;
        if tools
            .iter()
            .skip(index + 1)
            .any(|other| other.tool.value() == name.value())
        {
            return Err(syn::Error::new_spanned(
                name,
                format!("duplicate inline tool name '{}'", name.value()),
            ));
        }
    }
    Ok(())
}

fn reject_duplicate_command_plans(commands: &[PluginCommandPlan]) -> Result<()> {
    for (index, command) in commands.iter().enumerate() {
        let id = &command.id;
        if commands
            .iter()
            .skip(index + 1)
            .any(|other| other.id.value() == id.value())
        {
            return Err(syn::Error::new_spanned(
                &command.id,
                format!("duplicate #[command] id '{}'", id.value()),
            ));
        }
    }
    Ok(())
}

fn expand_plugin_layer_export(
    config: &PluginImplConfig,
    self_ty: &Type,
    generics: &syn::Generics,
) -> Result<proc_macro2::TokenStream> {
    let Some(export) = config.export.as_ref() else {
        return Ok(quote! {});
    };
    match export.to_string().as_str() {
        "cdylib" => {
            if !generics.params.is_empty() {
                return Err(syn::Error::new_spanned(
                    self_ty,
                    "export = cdylib does not support generic plugin types",
                ));
            }
            Ok(quote! {
                ::agena_plugin_sdk::export_cdylib!(#self_ty);
            })
        }
        "stdio" => {
            if !generics.params.is_empty() {
                return Err(syn::Error::new_spanned(
                    self_ty,
                    "export = stdio does not support generic plugin types",
                ));
            }
            Ok(quote! {
                ::agena_plugin_sdk::export_stdio!(<#self_ty as ::core::default::Default>::default());
            })
        }
        "http" => {
            if !generics.params.is_empty() {
                return Err(syn::Error::new_spanned(
                    self_ty,
                    "export = http does not support generic plugin types",
                ));
            }
            let bind = config.export_bind.as_ref().ok_or_else(|| {
                syn::Error::new_spanned(export, "export = http requires `bind = ...`")
            })?;
            Ok(quote! {
                ::agena_plugin_sdk::export_http!(<#self_ty as ::core::default::Default>::default(), #bind);
            })
        }
        other => Err(syn::Error::new_spanned(
            export,
            format!("unsupported plugin export '{other}'; expected cdylib, stdio, or http"),
        )),
    }
}

fn expand_plugin_layer_manifest(
    config: &PluginImplConfig,
    self_ty: &Type,
    cacheable: bool,
    docs: Option<&str>,
    tools: &[PluginToolPlan],
    hooks: &[PluginHookPlan],
    commands: &[PluginCommandPlan],
) -> Result<proc_macro2::TokenStream> {
    let namespace = config
        .namespace
        .as_ref()
        .expect("plugin namespace validated");
    let name = config.name.as_ref().expect("plugin name validated");
    let version = config.version.as_ref().expect("plugin version validated");
    let summary = if let Some(summary) = config.summary.as_ref() {
        quote! { #summary }
    } else if let Some(summary) = lit_str_from_text(doc_summary(docs).as_deref()) {
        quote! { #summary }
    } else {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            "#[agena_plugin(...)] requires `summary = ...` or doc comments on the impl block",
        ));
    };
    let hooks_expr = plugin_layer_hooks_expr(tools, hooks);

    let config_schema_assignment = expand_plugin_layer_config_schema_assignment(
        config.config_schema_type.as_ref(),
        config,
        self_ty,
    )?;
    let config_schema_value_assignment = config
        .config_schema
        .as_ref()
        .map(|schema| quote! { manifest.config_schema = Some(#schema); })
        .unwrap_or_default();
    let display_assignment = config
        .display
        .as_ref()
        .map(|display| {
            match display.to_string().as_str() {
                "brief" | "compact" => {
                    quote! { manifest.set_display(::agena_plugin_sdk::manifest::ToolDisplayPreset::Compact); }
                }
                "brief_detailed" => {
                    quote! { manifest.set_display(::agena_plugin_sdk::manifest::ToolDisplayPreset::BriefDetailed); }
                }
                "detailed" => {
                    quote! { manifest.set_display(::agena_plugin_sdk::manifest::ToolDisplayPreset::Detailed); }
                }
                _ => quote! { compile_error!("unsupported plugin display mode"); },
            }
        })
        .unwrap_or_default();
    let ui_display_assignment = config
        .ui_display
        .as_ref()
        .map(|display| {
            match display.to_string().as_str() {
                "brief" | "summary" => {
                    quote! { manifest.ui_display_mode = Some(::agena_plugin_sdk::UiTextDisplayMode::Summary); }
                }
                "detailed" => {
                    quote! { manifest.ui_display_mode = Some(::agena_plugin_sdk::UiTextDisplayMode::Detailed); }
                }
                _ => quote! { compile_error!("unsupported plugin UI display mode"); },
            }
        })
        .unwrap_or_default();
    let help_assignment = if let Some(help) = config.help.as_ref() {
        quote! { manifest.help = Some(#help.to_string()); }
    } else if let Some(help) = lit_str_from_text(docs) {
        quote! { manifest.help = Some(#help.to_string()); }
    } else {
        quote! {}
    };
    let tool_description_mode_assignment = config
        .tool_description_mode
        .as_ref()
        .map(|mode| quote! { manifest.tool_description_mode = Some(#mode); })
        .unwrap_or_default();
    let ui_display_mode_assignment = config
        .ui_display_mode
        .as_ref()
        .map(|mode| quote! { manifest.ui_display_mode = Some(#mode); })
        .unwrap_or_default();
    let plugin_capabilities_expr_assignment = config
        .plugin_capabilities_expr
        .as_ref()
        .map(|capabilities| quote! { manifest.add_plugin_capabilities(#capabilities); })
        .unwrap_or_default();
    let plugin_capability_assignments = config
        .plugin_capabilities
        .iter()
        .map(|capability| quote! { manifest.add_plugin_capability(#capability); })
        .collect::<Vec<_>>();
    let tool_definition_assignments = tools
        .iter()
        .map(|binding| {
            let definition = expand_plugin_tool_definition(&binding.input_model)?;
            Ok(quote! { manifest.tools.push(#definition); })
        })
        .collect::<Result<Vec<_>>>()?;
    let command_definition_assignments = commands
        .iter()
        .map(expand_plugin_command_definition)
        .collect::<Result<Vec<_>>>()?;

    let build_manifest = quote! {{
            let mut manifest = ::agena_plugin_sdk::PluginManifest::new(#namespace, #name, #version);
            manifest.summary = Some(#summary.to_string());
            manifest.hooks = #hooks_expr;
            manifest.config_schema = Some(::agena_plugin_sdk::macro_support::empty_config_schema());
            #config_schema_assignment
            #config_schema_value_assignment
            #display_assignment
            #ui_display_assignment
            #help_assignment
            #tool_description_mode_assignment
            #ui_display_mode_assignment
            #plugin_capabilities_expr_assignment
            #(#plugin_capability_assignments)*
            #(#tool_definition_assignments)*
            #(#command_definition_assignments)*
            manifest
    }};
    let body = if cacheable {
        quote! {
            static __AGENA_PLUGIN_MANIFEST: ::std::sync::OnceLock<::agena_plugin_sdk::PluginManifest> =
                ::std::sync::OnceLock::new();
            __AGENA_PLUGIN_MANIFEST.get_or_init(|| { #build_manifest }).clone()
        }
    } else {
        build_manifest
    };

    Ok(quote! {
        fn manifest(&self) -> ::agena_plugin_sdk::PluginManifest {
            #body
        }
    })
}

fn expand_plugin_layer_config_schema_assignment(
    config_schema_type: Option<&Type>,
    config: &PluginImplConfig,
    self_ty: &Type,
) -> Result<proc_macro2::TokenStream> {
    let Some(ty) = config_schema_type else {
        if config.config_schema_store {
            return Ok(quote! {
                manifest.config_schema = Some(
                    <#self_ty as ::agena_plugin_sdk::plugin::PluginConfigStoreAccess>::plugin_config_schema(),
                );
            });
        }
        return Ok(quote! {});
    };
    let Some(default) = config.config_schema_default.as_ref() else {
        return Ok(quote! {
            manifest.config_schema = Some(::agena_plugin_sdk::macro_support::json_schema_for::<#ty>());
        });
    };
    if expr_is_ident(default, "default") {
        Ok(quote! {
            manifest.config_schema = Some(
                ::agena_plugin_sdk::macro_support::json_schema_for_default(
                    <#ty as ::core::default::Default>::default(),
                ),
            );
        })
    } else {
        Ok(quote! {
            manifest.config_schema = Some(
                ::agena_plugin_sdk::macro_support::json_schema_for_default(#default),
            );
        })
    }
}

fn expr_is_ident(expr: &Expr, expected: &str) -> bool {
    let Expr::Path(path) = expr else {
        return false;
    };
    path.path.get_ident().is_some_and(|ident| ident == expected)
}

fn plugin_layer_hooks_expr(
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
        PluginHookKind::PermissionAsk => {
            quote! { ::agena_plugin_sdk::HookSubscription::PERMISSION_ASK }
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

fn expand_plugin_layer_tool_invoke(
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

fn expand_plugin_layer_command_invoke(
    _self_ty: &Type,
    commands: &[PluginCommandPlan],
) -> Result<proc_macro2::TokenStream> {
    let branches = commands
        .iter()
        .map(expand_plugin_layer_command_invoke_branch)
        .collect::<Result<Vec<_>>>()?;

    Ok(quote! {
        async fn command_invoke(
            &self,
            input: ::agena_plugin_sdk::PluginCommandInvokeInput,
        ) -> ::agena_plugin_sdk::Result<::agena_plugin_sdk::PluginCommandOutput> {
            let __command_id = input.command_id.clone();
            match __command_id.as_str() {
                #(#branches,)*
                _ => Err(::agena_plugin_sdk::PluginError::not_implemented(format!(
                    "command_invoke({})",
                    __command_id
                ))),
            }
        }
    })
}

fn expand_plugin_layer_command_invoke_branch(
    command: &PluginCommandPlan,
) -> Result<proc_macro2::TokenStream> {
    let id = &command.id;
    let body = match &command.handler {
        PluginCommandHandlerPlan::Method {
            method,
            input: command_input,
            context,
            is_async,
        } => match command_input {
            PluginCommandInputPlan::None => {
                let call_args = plugin_layer_command_call_args(*context, Vec::new());
                let call = plugin_layer_method_call(method, *is_async, &call_args);
                quote! {
                    ::agena_plugin_sdk::into_plugin_command_output(#call)
                }
            }
            PluginCommandInputPlan::Raw { by_ref, .. } => {
                let arg = if *by_ref {
                    quote! { &input }
                } else {
                    quote! { input.clone() }
                };
                let call_args = plugin_layer_command_call_args(*context, vec![arg]);
                let call = plugin_layer_method_call(method, *is_async, &call_args);
                quote! {
                    ::agena_plugin_sdk::into_plugin_command_output(#call)
                }
            }
            PluginCommandInputPlan::Typed { ty, by_ref } => {
                let parse = expand_plugin_command_parse_input(ty);
                let arg = if *by_ref {
                    quote! { &__parsed }
                } else {
                    quote! { __parsed }
                };
                let call_args = plugin_layer_command_call_args(*context, vec![arg]);
                let call = plugin_layer_method_call(method, *is_async, &call_args);
                quote! {
                    let __parsed = #parse;
                    ::agena_plugin_sdk::into_plugin_command_output(#call)
                }
            }
            PluginCommandInputPlan::Generated { input_model, input } => {
                let parse = expand_plugin_tool_parse_input(
                    input_model,
                    quote! { input.input.clone() },
                    method,
                )?;
                let call_args =
                    plugin_layer_command_call_args(*context, plugin_call_input_args(input));
                let call = plugin_layer_method_call(method, *is_async, &call_args);
                quote! {
                    let __parsed = #parse;
                    ::agena_plugin_sdk::into_plugin_command_output(#call)
                }
            }
        },
        PluginCommandHandlerPlan::InvokeTool {
            tool,
            submit_output_as_prompt,
            ..
        } => {
            quote! {
                Ok(::agena_plugin_sdk::PluginCommandOutput::InvokeTool {
                    tool: #tool.to_string(),
                    input: Some(input.input.clone()),
                    submit_output_as_prompt: #submit_output_as_prompt,
                })
            }
        }
    };
    Ok(quote! {
        #id => {
            return { #body };
        }
    })
}

fn expand_plugin_command_parse_input(ty: &Type) -> proc_macro2::TokenStream {
    quote! {{
        <#ty as ::agena_plugin_sdk::ToolInput>::parse_input(input.input.clone())?
    }}
}

fn expand_plugin_layer_tool_stream(
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
                payload_delta: __result.payload.clone(),
                metadata: __result.metadata.clone(),
            })
            .await;
            Ok(::agena_plugin_sdk::ToolStreamEnd::from_output(__stream_id, __result))
        }
    })
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

fn expand_plugin_layer_permission_paths(
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

fn expand_plugin_layer_permission_networks(
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
        let pushes = tool
            .permissions
            .network_rules
            .iter()
            .map(|rule| match rule {
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

fn expand_plugin_layer_init_method(
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
        if let Some(field) = config.config_field.as_ref() {
            quote! {
                self.#field.set_from_json(ctx.config.clone(), #invalid, #already)?;
            }
        } else if config.config_store {
            quote! {
                <#self_ty as ::agena_plugin_sdk::plugin::PluginConfigStoreAccess>::set_plugin_config_from_json(
                    self,
                    ctx.config.clone(),
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

fn plugin_id_label(config: &PluginImplConfig) -> String {
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

fn expand_plugin_layer_hook_methods(
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
        PluginHookKind::PermissionAsk,
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
        PluginHookKind::PermissionAsk => expand_plugin_layer_single_arg_hook_group(
            "permission_ask",
            quote! { ::agena_plugin_sdk::PermissionAskInput },
            quote! { Option<::agena_plugin_sdk::PermissionAskDecision> },
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

fn plugin_layer_method_call(
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

fn plugin_layer_command_call_args(
    context: Option<PluginContextArg>,
    mut input_args: Vec<proc_macro2::TokenStream>,
) -> Vec<proc_macro2::TokenStream> {
    let Some(context) = context else {
        return input_args;
    };
    let context_arg = if context.by_ref {
        quote! { &(input.context()) }
    } else {
        quote! { input.context() }
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

fn expand_plugin_generated_input(
    generated: &PluginGeneratedToolInput,
) -> Result<proc_macro2::TokenStream> {
    let Some(input_ident) = generated.input_ident.as_ref() else {
        return Ok(quote! {});
    };
    let fields = generated.input_fields.iter().map(|field| {
        let ident = &field.ident;
        let wire_name = &field.wire_name;
        let flatten_attrs = field
            .flatten_shape
            .then(|| quote! { #[serde(flatten)] #[schemars(flatten)] });
        let rename_attr = (!field.flatten_shape
            && field.wire_name.value() != field.ident.to_string())
        .then(|| quote! { #[serde(rename = #wire_name)] });
        let alias_attrs = if field.flatten_shape {
            Vec::new()
        } else {
            field
                .aliases
                .iter()
                .map(|alias| quote! { #[serde(alias = #alias)] })
                .collect::<Vec<_>>()
        };
        let ty = &field.ty;
        let default_attr = if field.flatten_shape {
            None
        } else if let Some(_) = field.default_expr {
            let helper = format_ident!("{}_default_{}", input_ident, ident);
            let helper_name = LitStr::new(&helper.to_string(), helper.span());
            Some(quote! { #[serde(default = #helper_name)] })
        } else {
            field.default.then(|| quote! { #[serde(default)] })
        };
        quote! {
            #flatten_attrs
            #default_attr
            #rename_attr
            #(#alias_attrs)*
            #ident: #ty
        }
    });
    let default_helpers = generated
        .input_fields
        .iter()
        .filter_map(|field| {
            let expr = field.default_expr.as_ref()?;
            let helper = format_ident!("{}_default_{}", input_ident, field.ident);
            let ty = &field.ty;
            Some(quote! {
                #[allow(non_snake_case)]
                fn #helper() -> #ty {
                    #expr
                }
            })
        })
        .collect::<Vec<_>>();
    let docs_attr = generated
        .docs
        .as_ref()
        .map(|docs| quote! { #[doc = #docs] })
        .unwrap_or_default();

    Ok(quote! {
        #[allow(non_camel_case_types)]
        #docs_attr
        #[derive(
            ::agena_plugin_sdk::serde::Serialize,
            ::agena_plugin_sdk::serde::Deserialize,
            ::agena_plugin_sdk::JsonSchema
        )]
        #[serde(deny_unknown_fields)]
        struct #input_ident {
            #(#fields),*
        }

        #(#default_helpers)*
    })
}

#[derive(Clone)]
struct ToolSpecConfig {
    tool: Option<LitStr>,
    before_help: Option<LitStr>,
    after_help: Option<LitStr>,
    summary: Option<LitStr>,
    help: Option<LitStr>,
    examples: Vec<LitStr>,
    normalize: Option<Path>,
    validate: Option<Path>,
    trim: Vec<LitStr>,
    trim_suffix: Vec<PathStringConstraint>,
    non_empty: Vec<LitStr>,
    non_empty_if_present: Vec<LitStr>,
    minimums: Vec<PathValueConstraint>,
    maximums: Vec<PathValueConstraint>,
    exclusive_minimums: Vec<PathValueConstraint>,
    exclusive_maximums: Vec<PathValueConstraint>,
    exactly_one_of: Vec<Vec<LitStr>>,
    at_least_one_of: Vec<Vec<LitStr>>,
    requires: Vec<PathPairConstraint>,
    conflicts_with: Vec<PathPairConstraint>,
    required_unless_present: Vec<PathPairConstraint>,
    forbid_substrings: Vec<PathStringsConstraint>,
    distinct_trimmed: Vec<LitStr>,
    distinct_trimmed_within: Vec<PathPairConstraint>,
    min_items: Vec<PathUsizeConstraint>,
    max_items: Vec<PathUsizeConstraint>,
    min_properties: Vec<PathUsizeConstraint>,
    max_properties: Vec<PathUsizeConstraint>,
    min_chars: Vec<PathUsizeConstraint>,
    max_chars: Vec<PathUsizeConstraint>,
    formats: Vec<PathStringConstraint>,
    patterns: Vec<PathStringConstraint>,
    choices: Vec<PathValuesConstraint>,
    input_paths: Vec<PluginInputPathSpec>,
    input_networks: Vec<PluginInputNetworkSpec>,
    input_field_metadata: Vec<PluginInputFieldMetadata>,
    display: Option<LitStr>,
    ui_display: Option<LitStr>,
    description_mode: Option<LitStr>,
    ui_display_mode: Option<LitStr>,
    tags: Vec<Expr>,
    capabilities: Vec<Expr>,
    concurrency_safe: bool,
    strict: bool,
    streaming: bool,
    input_shape: Option<Type>,
    output_ty: Option<Type>,
}

fn empty_tool_spec_config() -> ToolSpecConfig {
    ToolSpecConfig {
        tool: None,
        before_help: None,
        after_help: None,
        summary: None,
        help: None,
        examples: Vec::new(),
        normalize: None,
        validate: None,
        trim: Vec::new(),
        trim_suffix: Vec::new(),
        non_empty: Vec::new(),
        non_empty_if_present: Vec::new(),
        minimums: Vec::new(),
        maximums: Vec::new(),
        exclusive_minimums: Vec::new(),
        exclusive_maximums: Vec::new(),
        exactly_one_of: Vec::new(),
        at_least_one_of: Vec::new(),
        requires: Vec::new(),
        conflicts_with: Vec::new(),
        required_unless_present: Vec::new(),
        forbid_substrings: Vec::new(),
        distinct_trimmed: Vec::new(),
        distinct_trimmed_within: Vec::new(),
        min_items: Vec::new(),
        max_items: Vec::new(),
        min_properties: Vec::new(),
        max_properties: Vec::new(),
        min_chars: Vec::new(),
        max_chars: Vec::new(),
        formats: Vec::new(),
        patterns: Vec::new(),
        choices: Vec::new(),
        input_paths: Vec::new(),
        input_networks: Vec::new(),
        input_field_metadata: Vec::new(),
        display: None,
        ui_display: None,
        description_mode: None,
        ui_display_mode: None,
        tags: Vec::new(),
        capabilities: Vec::new(),
        concurrency_safe: false,
        strict: false,
        streaming: false,
        input_shape: None,
        output_ty: None,
    }
}

#[derive(Clone)]
struct PathUsizeConstraint {
    path: LitStr,
    value: usize,
}

#[derive(Clone)]
struct PathPairConstraint {
    left: LitStr,
    right: LitStr,
}

#[derive(Clone)]
struct PathStringsConstraint {
    path: LitStr,
    values: Vec<LitStr>,
}

#[derive(Clone)]
struct PathValueConstraint {
    path: LitStr,
    value: Expr,
}

#[derive(Clone)]
struct PathValuesConstraint {
    path: LitStr,
    values: Vec<Expr>,
}

#[derive(Clone)]
struct PathStringConstraint {
    path: LitStr,
    value: LitStr,
}

trait SchemaConstraintSource {
    fn non_empty(&self) -> &[LitStr];
    fn non_empty_if_present(&self) -> &[LitStr];
    fn minimums(&self) -> &[PathValueConstraint];
    fn maximums(&self) -> &[PathValueConstraint];
    fn exclusive_minimums(&self) -> &[PathValueConstraint];
    fn exclusive_maximums(&self) -> &[PathValueConstraint];
    fn min_items(&self) -> &[PathUsizeConstraint];
    fn max_items(&self) -> &[PathUsizeConstraint];
    fn min_properties(&self) -> &[PathUsizeConstraint];
    fn max_properties(&self) -> &[PathUsizeConstraint];
    fn min_chars(&self) -> &[PathUsizeConstraint];
    fn max_chars(&self) -> &[PathUsizeConstraint];
    fn formats(&self) -> &[PathStringConstraint];
    fn patterns(&self) -> &[PathStringConstraint];
    fn choices(&self) -> &[PathValuesConstraint];
    fn input_field_metadata(&self) -> &[PluginInputFieldMetadata] {
        &[]
    }
}

trait SchemaRelationSource {
    fn exactly_one_of(&self) -> &[Vec<LitStr>];
    fn at_least_one_of(&self) -> &[Vec<LitStr>];
    fn requires(&self) -> &[PathPairConstraint];
    fn conflicts_with(&self) -> &[PathPairConstraint];
    fn required_unless_present(&self) -> &[PathPairConstraint];
    fn forbid_substrings(&self) -> &[PathStringsConstraint];
    fn distinct_trimmed(&self) -> &[LitStr];
    fn distinct_trimmed_within(&self) -> &[PathPairConstraint];
}

impl SchemaConstraintSource for ToolSpecConfig {
    fn non_empty(&self) -> &[LitStr] {
        &self.non_empty
    }

    fn non_empty_if_present(&self) -> &[LitStr] {
        &self.non_empty_if_present
    }

    fn minimums(&self) -> &[PathValueConstraint] {
        &self.minimums
    }

    fn maximums(&self) -> &[PathValueConstraint] {
        &self.maximums
    }

    fn exclusive_minimums(&self) -> &[PathValueConstraint] {
        &self.exclusive_minimums
    }

    fn exclusive_maximums(&self) -> &[PathValueConstraint] {
        &self.exclusive_maximums
    }

    fn min_items(&self) -> &[PathUsizeConstraint] {
        &self.min_items
    }

    fn max_items(&self) -> &[PathUsizeConstraint] {
        &self.max_items
    }

    fn min_properties(&self) -> &[PathUsizeConstraint] {
        &self.min_properties
    }

    fn max_properties(&self) -> &[PathUsizeConstraint] {
        &self.max_properties
    }

    fn min_chars(&self) -> &[PathUsizeConstraint] {
        &self.min_chars
    }

    fn max_chars(&self) -> &[PathUsizeConstraint] {
        &self.max_chars
    }

    fn formats(&self) -> &[PathStringConstraint] {
        &self.formats
    }

    fn patterns(&self) -> &[PathStringConstraint] {
        &self.patterns
    }

    fn choices(&self) -> &[PathValuesConstraint] {
        &self.choices
    }

    fn input_field_metadata(&self) -> &[PluginInputFieldMetadata] {
        &self.input_field_metadata
    }
}

impl SchemaRelationSource for ToolSpecConfig {
    fn exactly_one_of(&self) -> &[Vec<LitStr>] {
        &self.exactly_one_of
    }

    fn at_least_one_of(&self) -> &[Vec<LitStr>] {
        &self.at_least_one_of
    }

    fn requires(&self) -> &[PathPairConstraint] {
        &self.requires
    }

    fn conflicts_with(&self) -> &[PathPairConstraint] {
        &self.conflicts_with
    }

    fn required_unless_present(&self) -> &[PathPairConstraint] {
        &self.required_unless_present
    }

    fn forbid_substrings(&self) -> &[PathStringsConstraint] {
        &self.forbid_substrings
    }

    fn distinct_trimmed(&self) -> &[LitStr] {
        &self.distinct_trimmed
    }

    fn distinct_trimmed_within(&self) -> &[PathPairConstraint] {
        &self.distinct_trimmed_within
    }
}

impl SchemaConstraintSource for ToolInputConfig {
    fn non_empty(&self) -> &[LitStr] {
        &self.non_empty
    }

    fn non_empty_if_present(&self) -> &[LitStr] {
        &self.non_empty_if_present
    }

    fn minimums(&self) -> &[PathValueConstraint] {
        &self.minimums
    }

    fn maximums(&self) -> &[PathValueConstraint] {
        &self.maximums
    }

    fn exclusive_minimums(&self) -> &[PathValueConstraint] {
        &self.exclusive_minimums
    }

    fn exclusive_maximums(&self) -> &[PathValueConstraint] {
        &self.exclusive_maximums
    }

    fn min_items(&self) -> &[PathUsizeConstraint] {
        &self.min_items
    }

    fn max_items(&self) -> &[PathUsizeConstraint] {
        &self.max_items
    }

    fn min_properties(&self) -> &[PathUsizeConstraint] {
        &self.min_properties
    }

    fn max_properties(&self) -> &[PathUsizeConstraint] {
        &self.max_properties
    }

    fn min_chars(&self) -> &[PathUsizeConstraint] {
        &self.min_chars
    }

    fn max_chars(&self) -> &[PathUsizeConstraint] {
        &self.max_chars
    }

    fn formats(&self) -> &[PathStringConstraint] {
        &self.formats
    }

    fn patterns(&self) -> &[PathStringConstraint] {
        &self.patterns
    }

    fn choices(&self) -> &[PathValuesConstraint] {
        &self.choices
    }

    fn input_field_metadata(&self) -> &[PluginInputFieldMetadata] {
        &self.input_field_metadata
    }
}

impl SchemaRelationSource for ToolInputConfig {
    fn exactly_one_of(&self) -> &[Vec<LitStr>] {
        &self.exactly_one_of
    }

    fn at_least_one_of(&self) -> &[Vec<LitStr>] {
        &self.at_least_one_of
    }

    fn requires(&self) -> &[PathPairConstraint] {
        &self.requires
    }

    fn conflicts_with(&self) -> &[PathPairConstraint] {
        &self.conflicts_with
    }

    fn required_unless_present(&self) -> &[PathPairConstraint] {
        &self.required_unless_present
    }

    fn forbid_substrings(&self) -> &[PathStringsConstraint] {
        &self.forbid_substrings
    }

    fn distinct_trimmed(&self) -> &[LitStr] {
        &self.distinct_trimmed
    }

    fn distinct_trimmed_within(&self) -> &[PathPairConstraint] {
        &self.distinct_trimmed_within
    }
}

impl SchemaConstraintSource for ToolInputVariantConfig {
    fn non_empty(&self) -> &[LitStr] {
        &self.non_empty
    }

    fn non_empty_if_present(&self) -> &[LitStr] {
        &self.non_empty_if_present
    }

    fn minimums(&self) -> &[PathValueConstraint] {
        &self.minimums
    }

    fn maximums(&self) -> &[PathValueConstraint] {
        &self.maximums
    }

    fn exclusive_minimums(&self) -> &[PathValueConstraint] {
        &self.exclusive_minimums
    }

    fn exclusive_maximums(&self) -> &[PathValueConstraint] {
        &self.exclusive_maximums
    }

    fn min_items(&self) -> &[PathUsizeConstraint] {
        &self.min_items
    }

    fn max_items(&self) -> &[PathUsizeConstraint] {
        &self.max_items
    }

    fn min_properties(&self) -> &[PathUsizeConstraint] {
        &self.min_properties
    }

    fn max_properties(&self) -> &[PathUsizeConstraint] {
        &self.max_properties
    }

    fn min_chars(&self) -> &[PathUsizeConstraint] {
        &self.min_chars
    }

    fn max_chars(&self) -> &[PathUsizeConstraint] {
        &self.max_chars
    }

    fn formats(&self) -> &[PathStringConstraint] {
        &self.formats
    }

    fn patterns(&self) -> &[PathStringConstraint] {
        &self.patterns
    }

    fn choices(&self) -> &[PathValuesConstraint] {
        &self.choices
    }

    fn input_field_metadata(&self) -> &[PluginInputFieldMetadata] {
        &self.input_field_metadata
    }
}

impl SchemaRelationSource for ToolInputVariantConfig {
    fn exactly_one_of(&self) -> &[Vec<LitStr>] {
        &self.exactly_one_of
    }

    fn at_least_one_of(&self) -> &[Vec<LitStr>] {
        &self.at_least_one_of
    }

    fn requires(&self) -> &[PathPairConstraint] {
        &self.requires
    }

    fn conflicts_with(&self) -> &[PathPairConstraint] {
        &self.conflicts_with
    }

    fn required_unless_present(&self) -> &[PathPairConstraint] {
        &self.required_unless_present
    }

    fn forbid_substrings(&self) -> &[PathStringsConstraint] {
        &self.forbid_substrings
    }

    fn distinct_trimmed(&self) -> &[LitStr] {
        &self.distinct_trimmed
    }

    fn distinct_trimmed_within(&self) -> &[PathPairConstraint] {
        &self.distinct_trimmed_within
    }
}

fn expand_plugin_command_definition(
    command: &PluginCommandPlan,
) -> Result<proc_macro2::TokenStream> {
    let id = &command.id;
    let title = &command.title;
    let description = &command.description;
    let category = &command.category;
    let slash = option_lit_str_expr(command.slash.as_ref());
    let aliases = &command.aliases;
    let usage = expand_plugin_command_usage_expr(command)?;
    let location = &command.location;
    let input_schema = match &command.handler {
        PluginCommandHandlerPlan::Method { input, .. } => match input {
            PluginCommandInputPlan::Typed { ty, .. } => {
                quote! { Some(<#ty as ::agena_plugin_sdk::ToolInput>::input_schema()) }
            }
            PluginCommandInputPlan::Generated { input_model, .. } => {
                let schema = expand_plugin_tool_input_schema(input_model)?;
                quote! { Some(#schema) }
            }
            PluginCommandInputPlan::None | PluginCommandInputPlan::Raw { .. } => quote! { None },
        },
        PluginCommandHandlerPlan::InvokeTool { input_model, .. } => {
            let schema = expand_plugin_tool_input_schema(input_model)?;
            quote! { Some(#schema) }
        }
    };
    let action = command.action.as_ref().map_or_else(
        || match &command.handler {
            PluginCommandHandlerPlan::Method { .. } => quote! {
                ::agena_plugin_sdk::PluginUiAction::InvokeCommand {
                    command: #id.to_string(),
                    input: None,
                }
            },
            PluginCommandHandlerPlan::InvokeTool {
                tool,
                submit_output_as_prompt,
                ..
            } => quote! {
                ::agena_plugin_sdk::PluginUiAction::InvokeTool {
                    tool: #tool.to_string(),
                    input: None,
                    submit_output_as_prompt: #submit_output_as_prompt,
                }
            },
        },
        |action| quote! { #action },
    );
    let handler = match &command.handler {
        PluginCommandHandlerPlan::Method { .. } => quote! { Some(#id.to_string()) },
        PluginCommandHandlerPlan::InvokeTool { .. } => quote! { Some(#id.to_string()) },
    };
    Ok(quote! {
        manifest.commands.push(::agena_plugin_sdk::PluginCommandDefinition {
            id: #id.to_string(),
            title: #title.to_string(),
            description: #description.to_string(),
            category: #category.to_string(),
            slash: #slash,
            aliases: vec![#(#aliases.to_string()),*],
            usage: #usage,
            location: #location.to_string(),
            input_schema: #input_schema,
            handler: #handler,
            action: #action,
        });
    })
}

fn option_lit_str_expr(value: Option<&LitStr>) -> proc_macro2::TokenStream {
    value
        .map(|value| quote! { Some(#value.to_string()) })
        .unwrap_or_else(|| quote! { None })
}

fn path_permission_kind_expr(kind: PluginPathPermissionKind) -> proc_macro2::TokenStream {
    match kind {
        PluginPathPermissionKind::Read => quote! { ::agena_plugin_sdk::PathKind::Read },
        PluginPathPermissionKind::Write => quote! { ::agena_plugin_sdk::PathKind::Write },
    }
}

fn expand_input_path_specs(specs: &[PluginInputPathSpec]) -> proc_macro2::TokenStream {
    if specs.is_empty() {
        return quote! { ::std::vec::Vec::new() };
    }
    let items = specs.iter().map(|spec| {
        let jsonpath = &spec.jsonpath;
        let kind = path_permission_kind_expr(spec.kind);
        let fallback = option_lit_str_expr(spec.fallback.as_ref());
        let optional = spec.optional;
        quote! {
            ::agena_plugin_sdk::InputPathSpec {
                jsonpath: #jsonpath.to_string(),
                kind: #kind,
                fallback: #fallback,
                optional: #optional,
            }
        }
    });
    quote! { vec![#(#items),*] }
}

fn expand_input_network_specs(specs: &[PluginInputNetworkSpec]) -> proc_macro2::TokenStream {
    if specs.is_empty() {
        return quote! { ::std::vec::Vec::new() };
    }
    let items = specs.iter().map(|spec| {
        let jsonpath = &spec.jsonpath;
        let fallback = option_lit_str_expr(spec.fallback.as_ref());
        let optional = spec.optional;
        quote! {
            ::agena_plugin_sdk::InputNetworkSpec {
                jsonpath: #jsonpath.to_string(),
                fallback: #fallback,
                optional: #optional,
            }
        }
    });
    quote! { vec![#(#items),*] }
}

fn expand_input_tags(
    paths: &[PluginInputPathSpec],
    networks: &[PluginInputNetworkSpec],
) -> proc_macro2::TokenStream {
    let mut tags = Vec::new();
    for path in paths {
        tags.push(match path.kind {
            PluginPathPermissionKind::Read => {
                quote! { ::agena_plugin_sdk::ToolTag::FilesystemRead }
            }
            PluginPathPermissionKind::Write => {
                quote! { ::agena_plugin_sdk::ToolTag::FilesystemWrite }
            }
        });
    }
    for network in networks {
        tags.push(quote! { ::agena_plugin_sdk::ToolTag::Network });
        match network.semantic {
            PluginNetworkSemantic::Internet => {
                tags.push(quote! { ::agena_plugin_sdk::ToolTag::Internet });
            }
            PluginNetworkSemantic::Private => {
                tags.push(quote! { ::agena_plugin_sdk::ToolTag::PrivateNetwork });
            }
            PluginNetworkSemantic::Network
            | PluginNetworkSemantic::Url
            | PluginNetworkSemantic::Host => {}
        }
    }
    if tags.is_empty() {
        quote! { ::std::vec::Vec::new() }
    } else {
        quote! { vec![#(#tags),*] }
    }
}

fn struct_flatten_shape_types(data: &Data) -> Result<Vec<Type>> {
    let Data::Struct(data_struct) = data else {
        return Ok(Vec::new());
    };
    data_struct
        .fields
        .iter()
        .filter_map(|field| flatten_shape_type(field).transpose())
        .collect()
}

fn enum_flatten_shape_types(data: &Data) -> Result<Vec<Type>> {
    let Data::Enum(data_enum) = data else {
        return Ok(Vec::new());
    };
    data_enum
        .variants
        .iter()
        .flat_map(|variant| {
            variant
                .fields
                .iter()
                .filter_map(|field| flatten_shape_type(field).transpose())
        })
        .collect()
}

fn struct_nested_shape_fields(
    attrs: &[Attribute],
    data: &Data,
) -> Result<Vec<NestedInputShapeField>> {
    let Data::Struct(data_struct) = data else {
        return Ok(Vec::new());
    };
    let rename_rule = serde_rename_all_rule(attrs)?;
    data_struct
        .fields
        .iter()
        .filter_map(|field| nested_input_shape_field(field, rename_rule).transpose())
        .collect()
}

fn enum_nested_shape_fields(
    attrs: &[Attribute],
    data: &Data,
) -> Result<Vec<NestedInputShapeField>> {
    let Data::Enum(data_enum) = data else {
        return Ok(Vec::new());
    };
    let enum_field_rule = serde_rename_all_fields_rule(attrs)?;
    let mut fields = Vec::new();
    for variant in &data_enum.variants {
        let variant_field_rule = serde_rename_all_rule(&variant.attrs)?.or(enum_field_rule);
        for field in &variant.fields {
            if let Some(field) = nested_input_shape_field(field, variant_field_rule)? {
                fields.push(field);
            }
        }
    }
    Ok(fields)
}

fn expand_input_paths_expr(
    attrs: &[Attribute],
    data: &Data,
    paths: &[PluginInputPathSpec],
) -> Result<proc_macro2::TokenStream> {
    let own = expand_input_path_specs(paths);
    let struct_flatten_shapes = struct_flatten_shape_types(data)?;
    let enum_flatten_shapes = enum_flatten_shape_types(data)?;
    let struct_nested_shapes = struct_nested_shape_fields(attrs, data)?;
    let enum_nested_shapes = enum_nested_shape_fields(attrs, data)?;
    if struct_flatten_shapes.is_empty()
        && enum_flatten_shapes.is_empty()
        && struct_nested_shapes.is_empty()
        && enum_nested_shapes.is_empty()
    {
        return Ok(own);
    }
    let struct_nested_path_expr = expand_nested_shape_path_specs_expr(&struct_nested_shapes, false);
    let enum_nested_path_expr = expand_nested_shape_path_specs_expr(&enum_nested_shapes, true);
    Ok(quote! {{
        let mut __items = #own;
        #(
            __items.extend(<#struct_flatten_shapes as ::agena_plugin_sdk::ToolInput>::input_paths());
        )*
        #(
            __items.extend(
                <#enum_flatten_shapes as ::agena_plugin_sdk::ToolInput>::input_paths()
                    .into_iter()
                    .map(|mut __spec| {
                        __spec.optional = true;
                        __spec
                    })
            );
        )*
        #struct_nested_path_expr
        #enum_nested_path_expr
        __items
    }})
}

fn expand_input_networks_expr(
    attrs: &[Attribute],
    data: &Data,
    networks: &[PluginInputNetworkSpec],
) -> Result<proc_macro2::TokenStream> {
    let own = expand_input_network_specs(networks);
    let struct_flatten_shapes = struct_flatten_shape_types(data)?;
    let enum_flatten_shapes = enum_flatten_shape_types(data)?;
    let struct_nested_shapes = struct_nested_shape_fields(attrs, data)?;
    let enum_nested_shapes = enum_nested_shape_fields(attrs, data)?;
    if struct_flatten_shapes.is_empty()
        && enum_flatten_shapes.is_empty()
        && struct_nested_shapes.is_empty()
        && enum_nested_shapes.is_empty()
    {
        return Ok(own);
    }
    let struct_nested_network_expr =
        expand_nested_shape_network_specs_expr(&struct_nested_shapes, false);
    let enum_nested_network_expr =
        expand_nested_shape_network_specs_expr(&enum_nested_shapes, true);
    Ok(quote! {{
        let mut __items = #own;
        #(
            __items.extend(<#struct_flatten_shapes as ::agena_plugin_sdk::ToolInput>::input_networks());
        )*
        #(
            __items.extend(
                <#enum_flatten_shapes as ::agena_plugin_sdk::ToolInput>::input_networks()
                    .into_iter()
                    .map(|mut __spec| {
                        __spec.optional = true;
                        __spec
                    })
            );
        )*
        #struct_nested_network_expr
        #enum_nested_network_expr
        __items
    }})
}

fn expand_input_tags_expr(
    attrs: &[Attribute],
    data: &Data,
    paths: &[PluginInputPathSpec],
    networks: &[PluginInputNetworkSpec],
) -> Result<proc_macro2::TokenStream> {
    let own = expand_input_tags(paths, networks);
    let struct_flatten_shapes = struct_flatten_shape_types(data)?;
    let enum_flatten_shapes = enum_flatten_shape_types(data)?;
    let struct_nested_shapes = struct_nested_shape_fields(attrs, data)?;
    let enum_nested_shapes = enum_nested_shape_fields(attrs, data)?;
    if struct_flatten_shapes.is_empty()
        && enum_flatten_shapes.is_empty()
        && struct_nested_shapes.is_empty()
        && enum_nested_shapes.is_empty()
    {
        return Ok(own);
    }
    let struct_nested_tag_exprs = struct_nested_shapes.iter().map(|field| {
        let ty = &field.spec.inner_ty;
        quote! {
            __items.extend(<#ty as ::agena_plugin_sdk::ToolInput>::input_tags());
        }
    });
    let enum_nested_tag_exprs = enum_nested_shapes.iter().map(|field| {
        let ty = &field.spec.inner_ty;
        quote! {
            __items.extend(<#ty as ::agena_plugin_sdk::ToolInput>::input_tags());
        }
    });
    Ok(quote! {{
        let mut __items = #own;
        #(
            __items.extend(<#struct_flatten_shapes as ::agena_plugin_sdk::ToolInput>::input_tags());
        )*
        #(
            __items.extend(<#enum_flatten_shapes as ::agena_plugin_sdk::ToolInput>::input_tags());
        )*
        #(#struct_nested_tag_exprs)*
        #(#enum_nested_tag_exprs)*
        __items
    }})
}

fn expand_nested_shape_schema_normalize_expr(
    nested_shapes: &[NestedInputShapeField],
) -> proc_macro2::TokenStream {
    if nested_shapes.is_empty() {
        return quote! {};
    }
    let exprs = nested_shapes.iter().map(|field| {
        let path = &field.normalize_path;
        let ty = &field.spec.inner_ty;
        quote! {
            ::agena_plugin_sdk::macro_support::normalize_nested_input_path(
                &mut input,
                #path,
                &<#ty as ::agena_plugin_sdk::ToolInput>::input_schema(),
            );
        }
    });
    quote! { #(#exprs)* }
}

fn expand_nested_shape_path_specs_expr(
    nested_shapes: &[NestedInputShapeField],
    variant_optional: bool,
) -> proc_macro2::TokenStream {
    if nested_shapes.is_empty() {
        return quote! {};
    }
    let field_exprs = nested_shapes.iter().map(|field| {
        let ty = &field.spec.inner_ty;
        let force_optional = field.spec.optional || variant_optional;
        let mut seen = BTreeSet::new();
        let prefixes = std::iter::once(&field.schema_path)
            .chain(field.schema_aliases.iter())
            .filter_map(|candidate| {
                let prefix = if field.spec.array {
                    format!("$.{}[*]", candidate.value())
                } else {
                    format!("$.{}", candidate.value())
                };
                if seen.insert(prefix.clone()) {
                    Some(LitStr::new(prefix.as_str(), candidate.span()))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        quote! {
            #(
                __items.extend(
                    <#ty as ::agena_plugin_sdk::ToolInput>::input_paths()
                        .into_iter()
                        .map(|mut __spec| {
                            if let Some(__jsonpath) = ::agena_plugin_sdk::macro_support::prefix_input_jsonpath(
                                #prefixes,
                                __spec.jsonpath.as_str(),
                            ) {
                                __spec.jsonpath = __jsonpath;
                            }
                            if #force_optional {
                                __spec.optional = true;
                            }
                            __spec
                        })
                );
            )*
        }
    });
    quote! { #(#field_exprs)* }
}

fn expand_nested_shape_network_specs_expr(
    nested_shapes: &[NestedInputShapeField],
    variant_optional: bool,
) -> proc_macro2::TokenStream {
    if nested_shapes.is_empty() {
        return quote! {};
    }
    let field_exprs = nested_shapes.iter().map(|field| {
        let ty = &field.spec.inner_ty;
        let force_optional = field.spec.optional || variant_optional;
        let mut seen = BTreeSet::new();
        let prefixes = std::iter::once(&field.schema_path)
            .chain(field.schema_aliases.iter())
            .filter_map(|candidate| {
                let prefix = if field.spec.array {
                    format!("$.{}[*]", candidate.value())
                } else {
                    format!("$.{}", candidate.value())
                };
                if seen.insert(prefix.clone()) {
                    Some(LitStr::new(prefix.as_str(), candidate.span()))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        quote! {
            #(
                __items.extend(
                    <#ty as ::agena_plugin_sdk::ToolInput>::input_networks()
                        .into_iter()
                        .map(|mut __spec| {
                            if let Some(__jsonpath) = ::agena_plugin_sdk::macro_support::prefix_input_jsonpath(
                                #prefixes,
                                __spec.jsonpath.as_str(),
                            ) {
                                __spec.jsonpath = __jsonpath;
                            }
                            if #force_optional {
                                __spec.optional = true;
                            }
                            __spec
                        })
                );
            )*
        }
    });
    quote! { #(#field_exprs)* }
}

fn expand_nested_shape_input_keys_expr(
    nested_shapes: &[NestedInputShapeField],
    path: &LitStr,
) -> proc_macro2::TokenStream {
    if nested_shapes.is_empty() {
        return quote! { ::std::vec::Vec::new() };
    }
    let field_exprs = nested_shapes.iter().map(|field| {
        let ty = &field.spec.inner_ty;
        let prefix = &field.normalize_path;
        let prefix_dot = LitStr::new(format!("{}.", prefix.value()).as_str(), prefix.span());
        let mut seen = BTreeSet::new();
        let prefixes = std::iter::once(&field.schema_path)
            .chain(field.schema_aliases.iter())
            .filter_map(|candidate| {
                let value = if field.spec.array {
                    format!("{}[]", candidate.value())
                } else {
                    candidate.value()
                };
                if seen.insert(value.clone()) {
                    Some(LitStr::new(value.as_str(), candidate.span()))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        quote! {
            if let Some(__tail) = #path.strip_prefix(#prefix_dot) {
                let __inner_keys =
                    ::agena_plugin_sdk::macro_support::flattened_input_keys_for_parse_path(
                        &<#ty as ::agena_plugin_sdk::ToolInput>::input_schema(),
                        __tail,
                    );
                #(
                    __keys.extend(
                        __inner_keys
                            .iter()
                            .map(|__inner| format!("{}.{__inner}", #prefixes)),
                    );
                )*
            }
        }
    });
    quote! {{
        let mut __keys = ::std::vec::Vec::new();
        #(#field_exprs)*
        __keys
    }}
}

fn expand_input_shape_resolved_path_expr(
    flatten_shapes: &[Type],
    nested_shapes: &[NestedInputShapeField],
    path: &LitStr,
) -> proc_macro2::TokenStream {
    if flatten_shapes.is_empty() && nested_shapes.is_empty() {
        return quote! { #path.to_string() };
    }
    let nested_expr = if nested_shapes.is_empty() {
        quote! {}
    } else {
        let exprs = nested_shapes.iter().map(|field| {
            let ty = &field.spec.inner_ty;
            let prefix = &field.normalize_path;
            let prefix_dot = LitStr::new(format!("{}.", prefix.value()).as_str(), prefix.span());
            quote! {
                if let Some(__tail) = __path.strip_prefix(#prefix_dot) {
                    let __resolved = ::agena_plugin_sdk::macro_support::resolve_input_constraint_path(
                        &<#ty as ::agena_plugin_sdk::ToolInput>::input_schema(),
                        __tail,
                    );
                    __path = format!("{}.{__resolved}", #prefix);
                }
            }
        });
        quote! { #(#exprs)* }
    };
    quote! {{
        let mut __path = #path.to_string();
        #nested_expr
        #(
            __path = ::agena_plugin_sdk::macro_support::resolve_input_constraint_path(
                &<#flatten_shapes as ::agena_plugin_sdk::ToolInput>::input_schema(),
                __path.as_str(),
            );
        )*
        __path
    }}
}

fn expand_flatten_shape_schema_normalize_expr(flatten_shapes: &[Type]) -> proc_macro2::TokenStream {
    if flatten_shapes.is_empty() {
        return quote! {};
    }
    quote! {
        #(
            ::agena_plugin_sdk::macro_support::normalize_flattened_input_object(
                &mut input,
                &<#flatten_shapes as ::agena_plugin_sdk::ToolInput>::input_schema(),
            );
        )*
    }
}

fn expand_flatten_shape_input_keys_expr(
    flatten_shapes: &[Type],
    path: &LitStr,
) -> proc_macro2::TokenStream {
    if flatten_shapes.is_empty() {
        return quote! { ::std::vec::Vec::new() };
    }
    quote! {{
        let mut __keys = ::std::vec::Vec::new();
        #(
            __keys.extend(
                ::agena_plugin_sdk::macro_support::flattened_input_keys_for_parse_path(
                    &<#flatten_shapes as ::agena_plugin_sdk::ToolInput>::input_schema(),
                    #path,
                ),
            );
        )*
        __keys
    }}
}

fn expand_input_example_expr(
    explicit_example: Option<&Expr>,
    metadata: &[PluginInputFieldMetadata],
) -> proc_macro2::TokenStream {
    if let Some(example) = explicit_example {
        return quote! { Some(::agena_plugin_sdk::serde_json::json!(#example)) };
    }
    let entries = metadata
        .iter()
        .filter_map(|field| {
            let example = field.example.as_ref()?;
            let path = &field.path;
            Some(quote! {
                __object.insert(
                    #path.to_string(),
                    ::agena_plugin_sdk::serde_json::json!(#example),
                );
            })
        })
        .collect::<Vec<_>>();
    if entries.is_empty() {
        quote! { ::agena_plugin_sdk::macro_support::example_value_from_schema(&Self::input_schema()) }
    } else {
        quote! {{
            let mut __object = ::agena_plugin_sdk::serde_json::Map::new();
            #(#entries)*
            Some(::agena_plugin_sdk::serde_json::Value::Object(__object))
        }}
    }
}

fn expand_input_root_example_schema_metadata_tokens(
    example: Option<&Expr>,
) -> proc_macro2::TokenStream {
    let Some(example) = example else {
        return quote! {};
    };
    quote! {
        ::agena_plugin_sdk::macro_support::set_schema_value_list_metadata(
            schema,
            "",
            "examples",
            &[::agena_plugin_sdk::serde_json::json!(#example)],
        );
    }
}

fn input_error_path_mappings(attrs: &[Attribute], data: &Data) -> Result<Vec<(LitStr, LitStr)>> {
    let Data::Struct(data_struct) = data else {
        return Ok(Vec::new());
    };
    let Fields::Named(fields) = &data_struct.fields else {
        return Ok(Vec::new());
    };
    let rename_rule = serde_rename_all_rule(attrs)?;
    field_error_path_mappings(&Fields::Named(fields.clone()), rename_rule)
}

fn field_error_path_mappings(
    fields: &Fields,
    rename_rule: Option<SerdeRenameRule>,
) -> Result<Vec<(LitStr, LitStr)>> {
    let Fields::Named(named) = fields else {
        return Ok(Vec::new());
    };
    let mut seen = BTreeSet::new();
    let mut mappings = Vec::new();
    for field in &named.named {
        let arg_config = parse_input_field_arg_attrs(field)?;
        let Some(names) = prepare_input_field_names(field, rename_rule, &arg_config)? else {
            continue;
        };
        if names.schema_path.value() == names.parse_path.value() {
            continue;
        }
        let key = (names.parse_path.value(), names.schema_path.value());
        if seen.insert(key.clone()) {
            mappings.push((
                LitStr::new(key.0.as_str(), names.parse_path.span()),
                LitStr::new(key.1.as_str(), names.schema_path.span()),
            ));
        }
    }
    Ok(mappings)
}

fn expand_input_shape_enum_parse_error_remap_expr(
    variants: &Punctuated<Variant, Token![,]>,
    enum_field_rule: Option<SerdeRenameRule>,
) -> Result<proc_macro2::TokenStream> {
    let mut arms = Vec::new();
    for variant in variants {
        let mappings = field_error_path_mappings(
            &variant.fields,
            serde_rename_all_rule(&variant.attrs)?.or(enum_field_rule),
        )?;
        if mappings.is_empty() {
            continue;
        }
        let config = normalized_input_variant_config(variant, enum_field_rule)?;
        let action = input_variant_action_name(variant, &config);
        let mapping_tokens = mappings
            .iter()
            .map(|(from, to)| quote! { (#from, #to) })
            .collect::<Vec<_>>();
        arms.push(quote! {
            Some(#action) => {
                ::agena_plugin_sdk::macro_support::remap_invalid_params_paths(
                    __macro_parse_result,
                    &[#(#mapping_tokens),*],
                )
            }
        });
    }
    if arms.is_empty() {
        Ok(quote! { __macro_parse_result })
    } else {
        Ok(quote! {
            match &input {
                ::agena_plugin_sdk::serde_json::Value::Object(object) => {
                    match object
                        .get("action")
                        .and_then(::agena_plugin_sdk::serde_json::Value::as_str)
                    {
                        #(#arms,)*
                        _ => __macro_parse_result,
                    }
                }
                _ => __macro_parse_result,
            }
        })
    }
}

fn expand_input_shape_enum_validate_error_remap_expr(
    variants: &Punctuated<Variant, Token![,]>,
    enum_field_rule: Option<SerdeRenameRule>,
) -> Result<proc_macro2::TokenStream> {
    let mut arms = Vec::new();
    for variant in variants {
        let mappings = field_error_path_mappings(
            &variant.fields,
            serde_rename_all_rule(&variant.attrs)?.or(enum_field_rule),
        )?;
        if mappings.is_empty() {
            continue;
        }
        let (_, ignore_pattern, _) = dispatch_variant_pattern_and_args(variant, false)?;
        let mapping_tokens = mappings
            .iter()
            .map(|(from, to)| quote! { (#from, #to) })
            .collect::<Vec<_>>();
        arms.push(quote! {
            #ignore_pattern => {
                ::agena_plugin_sdk::macro_support::remap_invalid_params_paths(
                    __macro_validate_result,
                    &[#(#mapping_tokens),*],
                )
            }
        });
    }
    if arms.is_empty() {
        Ok(quote! { __macro_validate_result })
    } else {
        Ok(quote! {
            match &parsed {
                #(#arms,)*
                _ => __macro_validate_result,
            }
        })
    }
}

fn expand_result_path_remap_expr(
    result_ident: &Ident,
    mappings: &[(LitStr, LitStr)],
) -> proc_macro2::TokenStream {
    if mappings.is_empty() {
        return quote! { #result_ident? };
    }
    let from = mappings.iter().map(|(from, _)| from).collect::<Vec<_>>();
    let to = mappings.iter().map(|(_, to)| to).collect::<Vec<_>>();
    quote! {
        ::agena_plugin_sdk::macro_support::remap_invalid_params_paths(
            #result_ident,
            &[#((#from, #to)),*],
        )?
    }
}

fn expand_input_root_default_insert_tokens(
    default: bool,
    default_expr: Option<&Expr>,
) -> proc_macro2::TokenStream {
    if let Some(default_expr) = default_expr {
        return quote! {
            if matches!(input, ::agena_plugin_sdk::serde_json::Value::Null) {
                input = ::agena_plugin_sdk::serde_json::to_value(#default_expr)
                    .map_err(|err| ::agena_plugin_sdk::PluginError::invalid_params(err.to_string()))?;
            }
        };
    }
    if default {
        return quote! {
            if matches!(input, ::agena_plugin_sdk::serde_json::Value::Null) {
                input = ::agena_plugin_sdk::serde_json::to_value(<Self as ::core::default::Default>::default())
                    .map_err(|err| ::agena_plugin_sdk::PluginError::invalid_params(err.to_string()))?;
            }
        };
    }
    quote! {}
}

fn expand_input_root_default_schema_metadata_tokens(
    default: bool,
    default_expr: Option<&Expr>,
) -> proc_macro2::TokenStream {
    if let Some(default_expr) = default_expr {
        return quote! {
            if let Ok(__default) = ::agena_plugin_sdk::serde_json::to_value(#default_expr) {
                ::agena_plugin_sdk::macro_support::set_schema_value_metadata(
                    schema,
                    "",
                    "default",
                    __default,
                );
            }
        };
    }
    if default {
        return quote! {
            if let Ok(__default) = ::agena_plugin_sdk::serde_json::to_value(
                <Self as ::core::default::Default>::default(),
            ) {
                ::agena_plugin_sdk::macro_support::set_schema_value_metadata(
                    schema,
                    "",
                    "default",
                    __default,
                );
            }
        };
    }
    quote! {}
}

fn expand_input_default_insert_tokens(
    defaults: &[PluginInputFieldDefaultSpec],
) -> proc_macro2::TokenStream {
    if defaults.is_empty() {
        return quote! {};
    }
    let inserts = defaults.iter().map(|default| {
        let path = &default.parse_path;
        let ty = &default.ty;
        let missing_expr = if default.aliases.is_empty() {
            quote! { !object.contains_key::<str>(#path) }
        } else {
            let aliases = &default.aliases;
            quote! {
                !object.contains_key::<str>(#path)
                    && ![#(#aliases),*]
                        .iter()
                        .any(|alias| object.contains_key::<str>(*alias))
            }
        };
        let default_value = if let Some(expr) = default.default_expr.as_ref() {
            quote! { ::agena_plugin_sdk::serde_json::json!(#expr) }
        } else {
            quote! {
                ::agena_plugin_sdk::serde_json::to_value(
                    <#ty as ::core::default::Default>::default(),
                )
                .map_err(|err| ::agena_plugin_sdk::PluginError::invalid_params(err.to_string()))?
            }
        };
        quote! {
            if #missing_expr {
                object.insert(#path.to_string(), #default_value);
            }
        }
    });
    quote! {
        match &mut input {
            ::agena_plugin_sdk::serde_json::Value::Object(object) => {
                #(#inserts)*
            }
            _ => {}
        }
    }
}

fn expand_input_alias_normalize_tokens(
    aliases: &[PluginInputFieldAliasSpec],
) -> proc_macro2::TokenStream {
    if aliases.is_empty() {
        return quote! {};
    }
    let moves = aliases.iter().map(|alias_spec| {
        let path = &alias_spec.path;
        let aliases = &alias_spec.aliases;
        quote! {
            if !object.contains_key::<str>(#path) {
                let mut __alias_key = None;
                for __candidate in [#(#aliases),*] {
                    if object.contains_key::<str>(__candidate) {
                        __alias_key = Some(__candidate);
                        break;
                    }
                }
                if let Some(__alias_key) = __alias_key {
                    if let Some(__alias_value) = object.remove::<str>(__alias_key) {
                        object.insert(#path.to_string(), __alias_value);
                    }
                }
            } else {
                for __alias_key in [#(#aliases),*] {
                    object.remove::<str>(__alias_key);
                }
            }
        }
    });
    quote! {
        match &mut input {
            ::agena_plugin_sdk::serde_json::Value::Object(object) => {
                #(#moves)*
            }
            _ => {}
        }
    }
}

fn expand_input_default_schema_metadata_tokens(
    defaults: &[PluginInputFieldDefaultSpec],
) -> proc_macro2::TokenStream {
    let calls = input_default_schema_metadata_calls("", defaults);
    quote! { #(#calls)* }
}

fn input_default_schema_metadata_calls(
    prefix: &str,
    defaults: &[PluginInputFieldDefaultSpec],
) -> Vec<proc_macro2::TokenStream> {
    defaults
        .iter()
        .map(|default| {
            let pointer = if prefix.is_empty() {
                format!(
                    "/properties/{}",
                    escape_json_pointer_segment(default.schema_path.value().as_str())
                )
            } else {
                format!(
                    "{prefix}/properties/{}",
                    escape_json_pointer_segment(default.schema_path.value().as_str())
                )
            };
            let pointer = LitStr::new(pointer.as_str(), default.schema_path.span());
            let ty = &default.ty;
            if let Some(expr) = default.default_expr.as_ref() {
                quote! {
                    ::agena_plugin_sdk::macro_support::set_schema_value_metadata(
                        schema,
                        #pointer,
                        "default",
                        ::agena_plugin_sdk::serde_json::json!(#expr),
                    );
                }
            } else {
                quote! {
                    if let Ok(__default) = ::agena_plugin_sdk::serde_json::to_value(
                        <#ty as ::core::default::Default>::default(),
                    ) {
                        ::agena_plugin_sdk::macro_support::set_schema_value_metadata(
                            schema,
                            #pointer,
                            "default",
                            __default,
                        );
                    }
                }
            }
        })
        .collect()
}

fn expand_plugin_tool_definition(
    model: &PluginGeneratedToolInput,
) -> Result<proc_macro2::TokenStream> {
    let spec = &model.spec;
    let flatten_shapes = generated_input_flatten_shape_types(&model.input_fields)?;
    let nested_shapes = generated_input_nested_shape_fields(&model.input_fields);
    let docs = model.docs.as_deref();
    let tool = spec.tool.as_ref().ok_or_else(|| {
        syn::Error::new(
            proc_macro2::Span::call_site(),
            "generated tool is missing a tool name",
        )
    })?;
    let summary = spec
        .summary
        .clone()
        .or_else(|| lit_str_from_text(doc_summary(docs).as_deref()))
        .ok_or_else(|| {
            syn::Error::new(
                proc_macro2::Span::call_site(),
                "generated tool is missing summary metadata or doc comments",
            )
        })?;
    let concurrency_safe = spec.concurrency_safe;
    let strict = spec.strict;
    let input_schema_expr = expand_plugin_tool_input_schema(model)?;
    let output_schema_expr = spec
        .output_ty
        .as_ref()
        .map(|ty| quote! { ::agena_plugin_sdk::macro_support::json_schema_for::<#ty>() })
        .unwrap_or_else(|| quote! { ::agena_plugin_sdk::serde_json::Value::Null });
    let help_expr = spec
        .help
        .as_ref()
        .cloned()
        .or_else(|| lit_str_from_text(docs))
        .map(|value| quote! { Some(#value.to_string()) })
        .unwrap_or_else(|| quote! { None });
    let before_help_expr = spec
        .before_help
        .as_ref()
        .map(|value| quote! { Some(#value.to_string()) })
        .unwrap_or_else(|| quote! { None });
    let after_help_expr = spec
        .after_help
        .as_ref()
        .map(|value| quote! { Some(#value.to_string()) })
        .unwrap_or_else(|| quote! { None });
    let examples_expr = if spec.examples.is_empty() {
        quote! { ::std::vec::Vec::new() }
    } else {
        let examples = &spec.examples;
        quote! { vec![#(#examples.to_string()),*] }
    };
    let display_preset = match spec.display.as_ref().map(LitStr::value).as_deref() {
        Some("brief") | Some("compact") => {
            Some(quote! { ::agena_plugin_sdk::manifest::ToolDisplayPreset::Compact })
        }
        Some("brief_detailed") => {
            Some(quote! { ::agena_plugin_sdk::manifest::ToolDisplayPreset::BriefDetailed })
        }
        Some("detailed") => {
            Some(quote! { ::agena_plugin_sdk::manifest::ToolDisplayPreset::Detailed })
        }
        Some(other) => {
            let invalid = spec.display.clone().expect("display was matched as Some");
            return Err(syn::Error::new_spanned(
                invalid,
                format!("unsupported tool display preset '{other}'"),
            ));
        }
        None => None,
    };
    let ui_display_override = match spec.ui_display.as_ref().map(LitStr::value).as_deref() {
        Some("brief") | Some("summary") => {
            Some(quote! { ::agena_plugin_sdk::UiTextDisplayMode::Summary })
        }
        Some("detailed") => Some(quote! { ::agena_plugin_sdk::UiTextDisplayMode::Detailed }),
        Some(other) => {
            let invalid = spec
                .ui_display
                .clone()
                .expect("ui_display was matched as Some");
            return Err(syn::Error::new_spanned(
                invalid,
                format!("unsupported ui display mode '{other}'"),
            ));
        }
        None => None,
    };
    let description_mode_override =
        match spec.description_mode.as_ref().map(LitStr::value).as_deref() {
            Some("brief") => Some(quote! { ::agena_plugin_sdk::ToolDescriptionMode::Brief }),
            Some("detailed") => Some(quote! { ::agena_plugin_sdk::ToolDescriptionMode::Detailed }),
            Some(other) => {
                let invalid = spec
                    .description_mode
                    .clone()
                    .expect("description_mode was matched as Some");
                return Err(syn::Error::new_spanned(
                    invalid,
                    format!("unsupported tool description mode '{other}'"),
                ));
            }
            None => None,
        };
    let ui_display_mode_override = match spec.ui_display_mode.as_ref().map(LitStr::value).as_deref()
    {
        Some("summary") => Some(quote! { ::agena_plugin_sdk::UiTextDisplayMode::Summary }),
        Some("detailed") => Some(quote! { ::agena_plugin_sdk::UiTextDisplayMode::Detailed }),
        Some(other) => {
            let invalid = spec
                .ui_display_mode
                .clone()
                .expect("ui_display_mode was matched as Some");
            return Err(syn::Error::new_spanned(
                invalid,
                format!("unsupported tool ui display mode '{other}'"),
            ));
        }
        None => None,
    };
    let description_mode_expr = if let Some(mode) = description_mode_override {
        quote! { Some(#mode) }
    } else if let Some(preset) = display_preset.as_ref() {
        quote! { Some(#preset.tool_description_mode()) }
    } else {
        quote! { None }
    };
    let ui_display_mode_expr = if let Some(mode) = ui_display_mode_override.or(ui_display_override)
    {
        quote! { Some(#mode) }
    } else if let Some(preset) = display_preset.as_ref() {
        quote! { Some(#preset.ui_display_mode()) }
    } else {
        quote! { None }
    };
    let tags_expr = if spec.tags.is_empty() {
        quote! { ::std::vec::Vec::new() }
    } else {
        let tags = &spec.tags;
        quote! { vec![#(#tags),*] }
    };
    let spec_input_paths_expr = expand_input_path_specs(&spec.input_paths);
    let spec_input_networks_expr = expand_input_network_specs(&spec.input_networks);
    let spec_input_tags_expr = expand_input_tags(&spec.input_paths, &spec.input_networks);
    let nested_paths_expr = expand_nested_shape_path_specs_expr(&nested_shapes, false);
    let nested_networks_expr = expand_nested_shape_network_specs_expr(&nested_shapes, false);
    let flatten_tag_exprs = flatten_shapes.iter().map(|ty| {
        quote! {
            __tags.extend(<#ty as ::agena_plugin_sdk::ToolInput>::input_tags());
        }
    });
    let nested_tag_exprs = nested_shapes.iter().map(|field| {
        let ty = &field.spec.inner_ty;
        quote! {
            __tags.extend(<#ty as ::agena_plugin_sdk::ToolInput>::input_tags());
        }
    });
    let input_paths_expr = if let Some(input_shape_ty) = spec.input_shape.as_ref() {
        quote! {{
            let mut __items = <#input_shape_ty as ::agena_plugin_sdk::ToolInput>::input_paths();
            __items.extend(#spec_input_paths_expr);
            __items
        }}
    } else if flatten_shapes.is_empty() && nested_shapes.is_empty() {
        spec_input_paths_expr
    } else {
        quote! {{
            let mut __items = #spec_input_paths_expr;
            #(
                __items.extend(<#flatten_shapes as ::agena_plugin_sdk::ToolInput>::input_paths());
            )*
            #nested_paths_expr
            __items
        }}
    };
    let input_networks_expr = if let Some(input_shape_ty) = spec.input_shape.as_ref() {
        quote! {{
            let mut __items = <#input_shape_ty as ::agena_plugin_sdk::ToolInput>::input_networks();
            __items.extend(#spec_input_networks_expr);
            __items
        }}
    } else if flatten_shapes.is_empty() && nested_shapes.is_empty() {
        spec_input_networks_expr
    } else {
        quote! {{
            let mut __items = #spec_input_networks_expr;
            #(
                __items.extend(<#flatten_shapes as ::agena_plugin_sdk::ToolInput>::input_networks());
            )*
            #nested_networks_expr
            __items
        }}
    };
    let tags_expr = if let Some(input_shape_ty) = spec.input_shape.as_ref() {
        quote! {{
            let mut __tags = #tags_expr;
            __tags.extend(#spec_input_tags_expr);
            __tags.extend(<#input_shape_ty as ::agena_plugin_sdk::ToolInput>::input_tags());
            ::agena_plugin_sdk::macro_support::dedupe_tool_tags(&mut __tags);
            __tags
        }}
    } else if flatten_shapes.is_empty() && nested_shapes.is_empty() {
        quote! {{
            let mut __tags = #tags_expr;
            __tags.extend(#spec_input_tags_expr);
            ::agena_plugin_sdk::macro_support::dedupe_tool_tags(&mut __tags);
            __tags
        }}
    } else {
        quote! {{
            let mut __tags = #tags_expr;
            __tags.extend(#spec_input_tags_expr);
            #(#flatten_tag_exprs)*
            #(#nested_tag_exprs)*
            ::agena_plugin_sdk::macro_support::dedupe_tool_tags(&mut __tags);
            __tags
        }}
    };
    let capabilities_expr = if spec.capabilities.is_empty() {
        quote! { ::std::vec::Vec::new() }
    } else {
        let capabilities = &spec.capabilities;
        quote! { vec![#(#capabilities),*] }
    };
    let streaming_expr = if spec.streaming {
        quote! { ::agena_plugin_sdk::ToolStreamingMode::Streaming }
    } else {
        quote! { ::agena_plugin_sdk::ToolStreamingMode::default() }
    };

    Ok(quote! {{
        let input_schema = #input_schema_expr;
        ::agena_plugin_sdk::ToolDefinition {
            name: #tool.to_string(),
            contract: ::agena_plugin_sdk::manifest::ToolContract {
                input_schema,
                output_schema: #output_schema_expr,
                strict: #strict,
            },
            model: ::agena_plugin_sdk::manifest::ToolModelSurface {
                examples: #examples_expr,
            },
            docs: ::agena_plugin_sdk::manifest::ToolDocs {
                before_help: #before_help_expr,
                after_help: #after_help_expr,
                summary: Some(#summary.to_string()),
                help: #help_expr,
            },
            runtime: ::agena_plugin_sdk::manifest::ToolRuntimePolicy {
                concurrency_safe: #concurrency_safe,
                streaming: #streaming_expr,
                result_policy: ::agena_plugin_sdk::ToolResultPolicy::default(),
            },
            permissions: ::agena_plugin_sdk::manifest::ToolPermissionContract {
                input_paths: #input_paths_expr,
                input_networks: #input_networks_expr,
                path_access: ::std::vec::Vec::new(),
                network_access: ::std::vec::Vec::new(),
                tags: #tags_expr,
            },
            display: ::agena_plugin_sdk::manifest::ToolDisplay {
                description_mode: #description_mode_expr,
                ui_display_mode: #ui_display_mode_expr,
            },
            capabilities: #capabilities_expr,
        }
    }})
}

fn expand_plugin_tool_input_schema(
    model: &PluginGeneratedToolInput,
) -> Result<proc_macro2::TokenStream> {
    let spec = &model.spec;
    let input_ty = &model.input_ty;
    let flatten_schema_calls = model
        .input_fields
        .iter()
        .enumerate()
        .filter(|(_, field)| field.flatten_shape)
        .map(|(index, field)| {
            let pointer = LitStr::new("", field.ident.span());
            let order = LitStr::new(format!("{index:06}").as_str(), field.ident.span());
            let ty = &field.ty;
            quote! {
                let mut overlay = <#ty as ::agena_plugin_sdk::ToolInput>::input_schema();
                ::agena_plugin_sdk::macro_support::prefix_schema_order_metadata(
                    &mut overlay,
                    #order,
                );
                ::agena_plugin_sdk::macro_support::merge_flattened_schema_at_pointer(
                    schema,
                    #pointer,
                    &overlay,
                );
            }
        })
        .collect::<Vec<_>>();
    let alias_calls = model
        .input_fields
        .iter()
        .filter(|field| !field.flatten_shape && !field.aliases.is_empty())
        .map(|field| {
            let pointer = LitStr::new(
                format!("/properties/{}", field.wire_name.value()).as_str(),
                field.wire_name.span(),
            );
            let aliases = field.aliases.iter().collect::<Vec<_>>();
            quote! {
                ::agena_plugin_sdk::macro_support::set_schema_string_list_metadata(
                    schema,
                    #pointer,
                    "x-agena-aliases",
                    &[#(#aliases),*],
                );
            }
        })
        .collect::<Vec<_>>();
    let nested_schema_calls = model
        .input_fields
        .iter()
        .enumerate()
        .filter_map(|(index, field)| {
            let spec = field
                .nested_shape
                .then(|| nested_input_shape_spec_from_type(&field.ty))
                .flatten()?;
            let pointer = if spec.array {
                format!("/properties/{}/items", field.wire_name.value())
            } else {
                format!("/properties/{}", field.wire_name.value())
            };
            let pointer = LitStr::new(pointer.as_str(), field.wire_name.span());
            let order = LitStr::new(format!("{index:06}").as_str(), field.wire_name.span());
            let inner_ty = &spec.inner_ty;
            Some(quote! {
                let mut overlay = <#inner_ty as ::agena_plugin_sdk::ToolInput>::input_schema();
                ::agena_plugin_sdk::macro_support::prefix_schema_order_metadata(
                    &mut overlay,
                    #order,
                );
                ::agena_plugin_sdk::macro_support::merge_schema_overlay_at_pointer(
                    schema,
                    #pointer,
                    &overlay,
                );
            })
        })
        .collect::<Vec<_>>();
    let metadata_calls = tool_spec_schema_metadata_calls(spec)?;
    let order_calls = model
        .input_fields
        .iter()
        .enumerate()
        .filter(|(_, field)| !field.flatten_shape)
        .map(|(index, field)| {
            let pointer = LitStr::new(
                format!("/properties/{}", field.wire_name.value()).as_str(),
                field.wire_name.span(),
            );
            let order = LitStr::new(format!("{index:06}").as_str(), field.wire_name.span());
            quote! {
                ::agena_plugin_sdk::macro_support::set_schema_string_metadata(
                    schema,
                    #pointer,
                    "x-agena-order",
                    #order,
                );
            }
        })
        .collect::<Vec<_>>();
    let default_calls = model
        .input_fields
        .iter()
        .filter(|field| !field.flatten_shape)
        .filter_map(|field| {
            let pointer = LitStr::new(
                format!("/properties/{}", field.wire_name.value()).as_str(),
                field.wire_name.span(),
            );
            let ty = &field.ty;
            if let Some(expr) = field.default_expr.as_ref() {
                Some(quote! {
                    ::agena_plugin_sdk::macro_support::set_schema_value_metadata(
                        schema,
                        #pointer,
                        "default",
                        ::agena_plugin_sdk::serde_json::json!(#expr),
                    );
                })
            } else if field.default {
                Some(quote! {
                    if let Ok(__default) = ::agena_plugin_sdk::serde_json::to_value(
                        <#ty as ::core::default::Default>::default(),
                    ) {
                        ::agena_plugin_sdk::macro_support::set_schema_value_metadata(
                            schema,
                            #pointer,
                            "default",
                            __default,
                        );
                    }
                })
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    let schema_source = if let Some(input_shape_ty) = spec.input_shape.as_ref() {
        quote! { <#input_shape_ty as ::agena_plugin_sdk::ToolInput>::input_schema() }
    } else {
        quote! { ::agena_plugin_sdk::macro_support::json_schema_for::<#input_ty>() }
    };
    Ok(quote! {{
        let mut schema = #schema_source;
        {
            let schema = &mut schema;
            #(#default_calls)*
            #(#alias_calls)*
            #(#flatten_schema_calls)*
            #(#nested_schema_calls)*
            #(#metadata_calls)*
            #(#order_calls)*
        }
        schema
    }})
}

fn expand_plugin_tool_parse_input(
    model: &PluginGeneratedToolInput,
    input_expr: proc_macro2::TokenStream,
    cache_label: &Ident,
) -> Result<proc_macro2::TokenStream> {
    let spec = &model.spec;
    let input_ty = &model.input_ty;
    let flatten_shapes = generated_input_flatten_shape_types(&model.input_fields)?;
    let nested_shapes = generated_input_nested_shape_fields(&model.input_fields);
    let input_aliases = generated_input_alias_specs(&model.input_fields);
    let input_alias_normalize_expr = expand_input_alias_normalize_tokens(&input_aliases);
    let flatten_shape_input_normalize_expr =
        expand_flatten_shape_schema_normalize_expr(&flatten_shapes);
    let nested_shape_input_normalize_expr =
        expand_nested_shape_schema_normalize_expr(&nested_shapes);
    let nested_shape_post_parse_expr = expand_generated_input_post_parse_tokens(model);
    let built_in_normalize_expr = built_in_normalization_tokens(
        quote! { &mut input },
        &spec.trim,
        &spec.trim_suffix,
        &flatten_shapes,
        &nested_shapes,
    );
    let built_in_post_parse_normalize_expr = built_in_post_parse_normalization_tokens(
        &spec.trim,
        &spec.trim_suffix,
        &flatten_shapes,
        &nested_shapes,
    );
    let normalize_expr = spec
        .normalize
        .as_ref()
        .map(|path| quote! { #path(input)? })
        .unwrap_or_else(|| quote! { input });
    let validate_expr = spec
        .validate
        .as_ref()
        .map(|path| quote! { #path(&parsed)?; })
        .unwrap_or_default();
    let built_in_validate_expr = built_in_validation_tokens(
        quote! { parsed },
        &spec.non_empty,
        &spec.non_empty_if_present,
        &spec.minimums,
        &spec.maximums,
        &spec.exclusive_minimums,
        &spec.exclusive_maximums,
        &spec.exactly_one_of,
        &spec.at_least_one_of,
        &spec.requires,
        &spec.conflicts_with,
        &spec.required_unless_present,
        &spec.forbid_substrings,
        &spec.distinct_trimmed,
        &spec.distinct_trimmed_within,
        &spec.min_items,
        &spec.max_items,
        &spec.min_properties,
        &spec.max_properties,
        &spec.min_chars,
        &spec.max_chars,
        &spec.formats,
        &spec.patterns,
        &spec.choices,
        &flatten_shapes,
        &nested_shapes,
    );

    if let Some(input_shape_ty) = spec.input_shape.as_ref() {
        return Ok(quote! {{
            let mut input = #input_expr;
            #input_alias_normalize_expr
            #nested_shape_input_normalize_expr
            #flatten_shape_input_normalize_expr
            #built_in_normalize_expr
            let input = #normalize_expr;
            let parsed = <#input_shape_ty as ::agena_plugin_sdk::ToolInput>::parse_input(input)?;
            let parsed = #built_in_post_parse_normalize_expr;
            let parsed = #nested_shape_post_parse_expr;
            #built_in_validate_expr
            #validate_expr
            parsed
        }});
    }

    let cache_label = sanitize_generated_ident_label(&cache_label.to_string()).to_ascii_uppercase();
    let schema_static = format_ident!("__AGENA_TOOL_INPUT_SCHEMA_{}", cache_label);
    let input_schema_expr = expand_plugin_tool_input_schema(model)?;
    Ok(quote! {{
        static #schema_static: ::std::sync::OnceLock<::agena_plugin_sdk::serde_json::Value> =
            ::std::sync::OnceLock::new();
        let mut input = #input_expr;
        #input_alias_normalize_expr
        #nested_shape_input_normalize_expr
        #flatten_shape_input_normalize_expr
        #built_in_normalize_expr
        let input = #normalize_expr;
        let schema = #schema_static.get_or_init(|| #input_schema_expr);
        let parsed = ::agena_plugin_sdk::macro_support::parse_typed_json_value_with_field_suggestions::<#input_ty>(
            input,
            schema,
            "field",
        )?;
        let parsed = #built_in_post_parse_normalize_expr;
        let parsed = #nested_shape_post_parse_expr;
        #built_in_validate_expr
        #validate_expr
        parsed
    }})
}

fn expand_input(input: DeriveInput) -> Result<proc_macro2::TokenStream> {
    let name = input.ident;
    let mut config = parse_input_config(&input.attrs)?;
    apply_input_field_arg_attrs(&mut config, &input.attrs, &input.data)?;
    if let Data::Struct(data_struct) = &input.data {
        let rename_rule = serde_rename_all_rule(&input.attrs)?;
        let (field_path_lookup, array_field_paths) =
            input_constraint_field_lookup(&data_struct.fields, rename_rule)?;
        resolve_constraint_lit_paths(&mut config.trim, &field_path_lookup);
        resolve_constraint_string_paths(&mut config.trim_suffix, &field_path_lookup);
        resolve_constraint_lit_paths(&mut config.non_empty, &field_path_lookup);
        resolve_constraint_lit_paths(&mut config.non_empty_if_present, &field_path_lookup);
        resolve_constraint_expr_paths(&mut config.minimums, &field_path_lookup);
        resolve_constraint_expr_paths(&mut config.maximums, &field_path_lookup);
        resolve_constraint_expr_paths(&mut config.exclusive_minimums, &field_path_lookup);
        resolve_constraint_expr_paths(&mut config.exclusive_maximums, &field_path_lookup);
        resolve_constraint_group_paths(&mut config.exactly_one_of, &field_path_lookup);
        resolve_constraint_group_paths(&mut config.at_least_one_of, &field_path_lookup);
        resolve_constraint_pair_paths(&mut config.requires, &field_path_lookup);
        resolve_constraint_pair_paths(&mut config.conflicts_with, &field_path_lookup);
        resolve_constraint_pair_paths(&mut config.required_unless_present, &field_path_lookup);
        resolve_constraint_strings_paths(&mut config.forbid_substrings, &field_path_lookup);
        resolve_constraint_lit_paths(&mut config.distinct_trimmed, &field_path_lookup);
        resolve_constraint_pair_paths(&mut config.distinct_trimmed_within, &field_path_lookup);
        resolve_constraint_usize_paths(&mut config.min_items, &field_path_lookup);
        resolve_constraint_usize_paths(&mut config.max_items, &field_path_lookup);
        resolve_constraint_usize_paths(&mut config.min_properties, &field_path_lookup);
        resolve_constraint_usize_paths(&mut config.max_properties, &field_path_lookup);
        resolve_constraint_usize_paths(&mut config.min_chars, &field_path_lookup);
        resolve_constraint_usize_paths(&mut config.max_chars, &field_path_lookup);
        resolve_constraint_string_paths(&mut config.formats, &field_path_lookup);
        resolve_constraint_string_paths(&mut config.patterns, &field_path_lookup);
        resolve_constraint_values_paths(&mut config.choices, &field_path_lookup);
        normalize_array_value_nested_path_constraints(
            &mut config.non_empty,
            &mut config.non_empty_if_present,
            &mut config.exactly_one_of,
            &mut config.at_least_one_of,
            &mut config.requires,
            &mut config.conflicts_with,
            &mut config.required_unless_present,
            &mut config.distinct_trimmed_within,
            &field_path_lookup,
            &array_field_paths,
        );
        normalize_array_value_constraints(
            &mut config.trim,
            &mut config.trim_suffix,
            &mut config.minimums,
            &mut config.maximums,
            &mut config.exclusive_minimums,
            &mut config.exclusive_maximums,
            &mut config.min_properties,
            &mut config.max_properties,
            &mut config.min_chars,
            &mut config.max_chars,
            &mut config.formats,
            &mut config.patterns,
            &mut config.choices,
            &mut config.forbid_substrings,
            &mut config.distinct_trimmed,
            &mut config.input_field_metadata,
            &field_path_lookup,
            &array_field_paths,
        );
    }
    let enum_field_rule = serde_rename_all_fields_rule(&input.attrs)?;
    if let Data::Enum(data_enum) = &input.data {
        for variant in &data_enum.variants {
            let mut variant_config = normalized_input_variant_config(variant, enum_field_rule)?;
            // Variant-local permission specs are conditional on the selected branch.
            // Once lifted onto the enum root for manifest/runtime permission extraction,
            // they must be treated as optional so inactive variants do not fail lookup.
            for spec in &mut variant_config.input_paths {
                spec.optional = true;
            }
            for spec in &mut variant_config.input_networks {
                spec.optional = true;
            }
            config.input_paths.extend(variant_config.input_paths);
            config.input_networks.extend(variant_config.input_networks);
        }
    }
    let schema_metadata_fn =
        expand_schema_metadata_fn(&input.attrs, &input.data, &config, |variant, prefix| {
            let config = normalized_input_variant_config(variant, enum_field_rule)?;
            let mut calls = constraint_schema_metadata_calls(prefix, &config)?;
            calls.extend(constraint_relation_metadata_calls(prefix, &config)?);
            calls.extend(input_default_schema_metadata_calls(
                prefix,
                &config.input_defaults,
            ));
            Ok(calls)
        })?;
    let flatten_shape_post_parse_expr =
        expand_flatten_shape_post_parse_tokens(&input.attrs, &input.data)?;
    let (
        enum_helper_fn,
        variant_parse_error_remap_expr,
        variant_post_parse_normalize_expr,
        variant_validate_arms,
        variant_validate_error_remap_expr,
    ) = match &input.data {
        Data::Enum(data_enum) => (
            expand_input_shape_enum_normalize_fn(&data_enum.variants, enum_field_rule)?,
            expand_input_shape_enum_parse_error_remap_expr(&data_enum.variants, enum_field_rule)?,
            expand_input_shape_enum_post_parse_normalize_expr(
                &data_enum.variants,
                enum_field_rule,
            )?,
            data_enum
                .variants
                .iter()
                .filter_map(|variant| {
                    expand_input_shape_variant_validation_arm(variant, enum_field_rule).transpose()
                })
                .collect::<Result<Vec<_>>>()?,
            expand_input_shape_enum_validate_error_remap_expr(
                &data_enum.variants,
                enum_field_rule,
            )?,
        ),
        Data::Struct(_) => (
            quote! {
                fn __macro_normalize_enum_input(
                    input: serde_json::Value,
                ) -> ::agena_plugin_sdk::Result<serde_json::Value> {
                    Ok(input)
                }
            },
            quote! { __macro_parse_result },
            quote! { parsed },
            Vec::new(),
            quote! { __macro_validate_result },
        ),
        _ => {
            return Err(syn::Error::new_spanned(
                name,
                "ToolInput can only be derived for enums or structs",
            ));
        }
    };

    let struct_flatten_shapes = struct_flatten_shape_types(&input.data)?;
    let struct_nested_shapes = struct_nested_shape_fields(&input.attrs, &input.data)?;
    let built_in_normalize_expr = built_in_normalization_tokens(
        quote! { &mut input },
        &config.trim,
        &config.trim_suffix,
        &struct_flatten_shapes,
        &struct_nested_shapes,
    );
    let built_in_post_parse_normalize_expr = built_in_post_parse_normalization_tokens(
        &config.trim,
        &config.trim_suffix,
        &struct_flatten_shapes,
        &struct_nested_shapes,
    );
    let normalize_expr = config
        .normalize
        .as_ref()
        .map(|path| quote! { #path(input)? })
        .unwrap_or_else(|| quote! { input });
    let validate_expr = config
        .validate
        .as_ref()
        .map(|path| quote! { #path(&parsed)?; })
        .unwrap_or_default();
    let built_in_validate_expr = built_in_validation_tokens(
        quote! { parsed },
        &config.non_empty,
        &config.non_empty_if_present,
        &config.minimums,
        &config.maximums,
        &config.exclusive_minimums,
        &config.exclusive_maximums,
        &config.exactly_one_of,
        &config.at_least_one_of,
        &config.requires,
        &config.conflicts_with,
        &config.required_unless_present,
        &config.forbid_substrings,
        &config.distinct_trimmed,
        &config.distinct_trimmed_within,
        &config.min_items,
        &config.max_items,
        &config.min_properties,
        &config.max_properties,
        &config.min_chars,
        &config.max_chars,
        &config.formats,
        &config.patterns,
        &config.choices,
        &struct_flatten_shapes,
        &struct_nested_shapes,
    );
    let dispatch_tool_invoke_fn = expand_input_dispatch_fn(&input.data, &config)?;
    let input_paths_expr = expand_input_paths_expr(&input.attrs, &input.data, &config.input_paths)?;
    let input_networks_expr =
        expand_input_networks_expr(&input.attrs, &input.data, &config.input_networks)?;
    let input_tags_expr = expand_input_tags_expr(
        &input.attrs,
        &input.data,
        &config.input_paths,
        &config.input_networks,
    )?;
    let input_example_expr =
        expand_input_example_expr(config.example.as_ref(), &config.input_field_metadata);
    let input_root_default_insert_expr =
        expand_input_root_default_insert_tokens(config.default, config.default_expr.as_ref());
    let input_alias_normalize_expr = expand_input_alias_normalize_tokens(&config.input_aliases);
    let input_default_insert_expr = expand_input_default_insert_tokens(&config.input_defaults);
    let nested_shape_input_normalize_expr =
        expand_nested_shape_schema_normalize_expr(&struct_nested_shapes);
    let flatten_shape_input_normalize_expr =
        expand_flatten_shape_schema_normalize_expr(&struct_flatten_shapes);
    let input_root_default_schema_metadata_expr = expand_input_root_default_schema_metadata_tokens(
        config.default,
        config.default_expr.as_ref(),
    );
    let input_default_schema_metadata_expr =
        expand_input_default_schema_metadata_tokens(&config.input_defaults);
    let input_root_example_schema_metadata_expr =
        expand_input_root_example_schema_metadata_tokens(config.example.as_ref());
    let input_error_path_mappings = input_error_path_mappings(&input.attrs, &input.data)?;
    let parse_error_remap_expr = expand_result_path_remap_expr(
        &format_ident!("__macro_parse_result"),
        &input_error_path_mappings,
    );
    let validate_error_remap_expr = expand_result_path_remap_expr(
        &format_ident!("__macro_validate_result"),
        &input_error_path_mappings,
    );

    Ok(quote! {
        impl #name {
            #enum_helper_fn
            #schema_metadata_fn
            #dispatch_tool_invoke_fn

            pub(crate) fn input_schema() -> serde_json::Value {
                static __AGENA_TOOL_INPUT_SCHEMA: ::std::sync::OnceLock<serde_json::Value> =
                    ::std::sync::OnceLock::new();
                __AGENA_TOOL_INPUT_SCHEMA.get_or_init(|| {
                    let mut schema = ::agena_plugin_sdk::macro_support::json_schema_for::<Self>();
                    Self::__macro_apply_schema_metadata(&mut schema);
                    {
                        let schema = &mut schema;
                        #input_root_default_schema_metadata_expr
                        #input_root_example_schema_metadata_expr
                        #input_default_schema_metadata_expr
                    }
                    schema
                }).clone()
            }

            pub(crate) fn input_paths() -> Vec<::agena_plugin_sdk::InputPathSpec> {
                #input_paths_expr
            }

            pub(crate) fn input_networks() -> Vec<::agena_plugin_sdk::InputNetworkSpec> {
                #input_networks_expr
            }

            pub(crate) fn input_tags() -> Vec<::agena_plugin_sdk::ToolTag> {
                #input_tags_expr
            }

            pub(crate) fn input_example() -> Option<::agena_plugin_sdk::serde_json::Value> {
                #input_example_expr
            }

            pub(crate) fn parse_input(
                input: serde_json::Value,
            ) -> ::agena_plugin_sdk::Result<Self> {
                let mut input = input;
                #input_root_default_insert_expr
                #input_alias_normalize_expr
                #input_default_insert_expr
                #nested_shape_input_normalize_expr
                #flatten_shape_input_normalize_expr
                #built_in_normalize_expr
                let input = #normalize_expr;
                let input = Self::__macro_normalize_enum_input(input)?;
                let schema = Self::input_schema();
                let __macro_parse_result = ::agena_plugin_sdk::macro_support::parse_typed_json_value_with_field_suggestions::<Self>(
                    input.clone(),
                    &schema,
                    "field",
                );
                let __macro_parse_result = #variant_parse_error_remap_expr;
                let parsed = #parse_error_remap_expr;
                let parsed = #built_in_post_parse_normalize_expr;
                let parsed = #variant_post_parse_normalize_expr;
                let parsed = #flatten_shape_post_parse_expr;
                let __macro_validate_result: ::agena_plugin_sdk::Result<()> = (|| {
                    match &parsed {
                        #(#variant_validate_arms)*
                        _ => {}
                    }
                    #built_in_validate_expr
                    #validate_expr
                    Ok(())
                })();
                let __macro_validate_result = #variant_validate_error_remap_expr;
                let () = #validate_error_remap_expr;
                Ok(parsed)
            }

            pub(crate) fn parse_json_str(
                input: &str,
            ) -> ::agena_plugin_sdk::Result<Self> {
                let input = ::agena_plugin_sdk::macro_support::parse_json_value_str(input)?;
                Self::parse_input(input)
            }
        }

        impl ::agena_plugin_sdk::ToolInput for #name {
            fn input_schema() -> serde_json::Value {
                Self::input_schema()
            }

            fn parse_input(input: serde_json::Value) -> ::agena_plugin_sdk::Result<Self> {
                Self::parse_input(input)
            }

            fn input_paths() -> Vec<::agena_plugin_sdk::InputPathSpec> {
                Self::input_paths()
            }

            fn input_networks() -> Vec<::agena_plugin_sdk::InputNetworkSpec> {
                Self::input_networks()
            }

            fn input_tags() -> Vec<::agena_plugin_sdk::ToolTag> {
                Self::input_tags()
            }

            fn input_example() -> Option<::agena_plugin_sdk::serde_json::Value> {
                Self::input_example()
            }

            fn parse_json_str(input: &str) -> ::agena_plugin_sdk::Result<Self> {
                Self::parse_json_str(input)
            }
        }
    })
}

fn streamify_invoke_output(call_expr: proc_macro2::TokenStream) -> proc_macro2::TokenStream {
    quote! {{
        let result = #call_expr?;
        sink.chunk(::agena_plugin_sdk::ToolStreamChunk {
            stream_id: sink.stream_id().to_string(),
            text_delta: Some(result.output_text.clone()),
            payload_delta: result.payload.clone(),
            metadata: result.metadata.clone(),
        })
        .await;
        Ok(::agena_plugin_sdk::ToolStreamEnd {
            stream_id: sink.stream_id().to_string(),
            title: result.title,
            output_text: result.output_text,
            payload: result.payload,
            metadata: result.metadata,
            attachments: result.attachments,
        })
    }}
}

fn dispatch_variant_pattern_and_args(
    variant: &Variant,
    handle_by_value: bool,
) -> Result<(
    proc_macro2::TokenStream,
    proc_macro2::TokenStream,
    Vec<proc_macro2::TokenStream>,
)> {
    let variant_name = &variant.ident;
    match &variant.fields {
        Fields::Unit => Ok((
            quote! { Self::#variant_name },
            quote! { Self::#variant_name },
            Vec::new(),
        )),
        Fields::Named(fields) => {
            let bindings = fields
                .named
                .iter()
                .map(|field| {
                    field.ident.clone().ok_or_else(|| {
                        syn::Error::new_spanned(field, "named dispatch field is missing identifier")
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            let args = bindings
                .iter()
                .map(|binding| {
                    if handle_by_value {
                        quote! { #binding }
                    } else {
                        quote! { &#binding }
                    }
                })
                .collect::<Vec<_>>();
            Ok((
                quote! { Self::#variant_name { #(#bindings),* } },
                quote! { Self::#variant_name { .. } },
                args,
            ))
        }
        Fields::Unnamed(fields) => {
            let bindings = fields
                .unnamed
                .iter()
                .enumerate()
                .map(|(index, _)| format_ident!("value_{index}"))
                .collect::<Vec<_>>();
            let args = bindings
                .iter()
                .map(|binding| {
                    if handle_by_value {
                        quote! { #binding }
                    } else {
                        quote! { &#binding }
                    }
                })
                .collect::<Vec<_>>();
            Ok((
                quote! { Self::#variant_name(#(#bindings),*) },
                quote! { Self::#variant_name(..) },
                args,
            ))
        }
    }
}

fn expand_input_dispatch_fn(
    data: &Data,
    config: &ToolInputConfig,
) -> Result<proc_macro2::TokenStream> {
    let receiver_ty = config.handler_receiver.as_ref();
    let struct_handle = config.handle.as_ref();
    let struct_handle_with_context = config.handle_with_context.as_ref();
    let struct_stream_handle = config.stream_handle.as_ref();
    let struct_stream_handle_with_context = config.stream_handle_with_context.as_ref();
    let struct_permission_paths_handle = config.permission_paths_handle.as_ref();
    let struct_permission_networks_handle = config.permission_networks_handle.as_ref();
    if receiver_ty.is_none() {
        if struct_handle.is_some()
            || struct_handle_with_context.is_some()
            || struct_stream_handle.is_some()
            || struct_stream_handle_with_context.is_some()
            || struct_permission_paths_handle.is_some()
            || struct_permission_networks_handle.is_some()
            || config.handle_field.is_some()
            || config.handle_by_value
        {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                "handle/handle_with_context/stream_handle/stream_handle_with_context/permission_paths_handle/permission_networks_handle/handle_field/handle_by_value require handler_receiver on the shape",
            ));
        }
    }

    match data {
        Data::Struct(_) => {
            if struct_handle.is_some() && struct_handle_with_context.is_some() {
                return Err(syn::Error::new(
                    proc_macro2::Span::call_site(),
                    "ToolInput structs cannot combine handle and handle_with_context",
                ));
            }
            if struct_stream_handle.is_some() && struct_stream_handle_with_context.is_some() {
                return Err(syn::Error::new(
                    proc_macro2::Span::call_site(),
                    "ToolInput structs cannot combine stream_handle and stream_handle_with_context",
                ));
            }
            let Some(receiver_ty) = receiver_ty else {
                if struct_handle.is_some()
                    || struct_handle_with_context.is_some()
                    || struct_stream_handle.is_some()
                    || struct_stream_handle_with_context.is_some()
                    || struct_permission_paths_handle.is_some()
                    || struct_permission_networks_handle.is_some()
                    || config.handle_by_value
                {
                    return Err(syn::Error::new(
                        proc_macro2::Span::call_site(),
                        "handle/handle_with_context/stream_handle/stream_handle_with_context/permission_paths_handle/permission_networks_handle/handle_by_value on a shape struct require handler_receiver",
                    ));
                }
                return Ok(quote! {});
            };
            let handle_by_value = config.handle_by_value;
            let arg_expr = if let Some(field) = config.handle_field.as_ref() {
                let field_ident = single_segment_ident(field, "handle_field")?;
                if handle_by_value {
                    quote! { parsed.#field_ident }
                } else {
                    quote! { &parsed.#field_ident }
                }
            } else if handle_by_value {
                quote! { parsed }
            } else {
                quote! { &parsed }
            };

            let plain_fn = if let Some(handle) = struct_handle {
                quote! {
                    pub async fn dispatch_tool_invoke(
                        self,
                        receiver: &#receiver_ty,
                    ) -> ::agena_plugin_sdk::Result<::agena_plugin_sdk::ToolInvokeOutput> {
                        let parsed = self;
                        #handle(receiver, #arg_expr).await
                    }
                }
            } else if struct_handle_with_context.is_some() {
                quote! {
                    pub async fn dispatch_tool_invoke(
                        self,
                        _receiver: &#receiver_ty,
                    ) -> ::agena_plugin_sdk::Result<::agena_plugin_sdk::ToolInvokeOutput> {
                        Err(::agena_plugin_sdk::PluginError::invalid_params(
                            "tool dispatch requires invoke context",
                        ))
                    }
                }
            } else {
                quote! {}
            };
            let context_fn = if let Some(handle) = struct_handle_with_context {
                quote! {
                    pub async fn dispatch_tool_invoke_with_context(
                        self,
                        receiver: &#receiver_ty,
                        context: &::agena_plugin_sdk::ToolInvokeContext<'_>,
                    ) -> ::agena_plugin_sdk::Result<::agena_plugin_sdk::ToolInvokeOutput> {
                        let parsed = self;
                        #handle(receiver, context, #arg_expr).await
                    }
                }
            } else if let Some(handle) = struct_handle {
                quote! {
                    pub async fn dispatch_tool_invoke_with_context(
                        self,
                        receiver: &#receiver_ty,
                        _context: &::agena_plugin_sdk::ToolInvokeContext<'_>,
                    ) -> ::agena_plugin_sdk::Result<::agena_plugin_sdk::ToolInvokeOutput> {
                        let parsed = self;
                        #handle(receiver, #arg_expr).await
                    }
                }
            } else {
                quote! {}
            };
            let plain_stream_fn = if let Some(handle) = struct_stream_handle {
                quote! {
                    pub async fn dispatch_tool_invoke_stream(
                        self,
                        receiver: &#receiver_ty,
                        sink: ::agena_plugin_sdk::ToolStreamSink,
                    ) -> ::agena_plugin_sdk::Result<::agena_plugin_sdk::ToolStreamEnd> {
                        let parsed = self;
                        #handle(receiver, sink, #arg_expr).await
                    }
                }
            } else if struct_stream_handle_with_context.is_some()
                || struct_handle_with_context.is_some()
            {
                quote! {
                    pub async fn dispatch_tool_invoke_stream(
                        self,
                        _receiver: &#receiver_ty,
                        _sink: ::agena_plugin_sdk::ToolStreamSink,
                    ) -> ::agena_plugin_sdk::Result<::agena_plugin_sdk::ToolStreamEnd> {
                        Err(::agena_plugin_sdk::PluginError::invalid_params(
                            "tool stream dispatch requires invoke context",
                        ))
                    }
                }
            } else if let Some(handle) = struct_handle {
                let call_expr = quote! { #handle(receiver, #arg_expr).await };
                let streamified = streamify_invoke_output(call_expr);
                quote! {
                    pub async fn dispatch_tool_invoke_stream(
                        self,
                        receiver: &#receiver_ty,
                        sink: ::agena_plugin_sdk::ToolStreamSink,
                    ) -> ::agena_plugin_sdk::Result<::agena_plugin_sdk::ToolStreamEnd> {
                        let parsed = self;
                        #streamified
                    }
                }
            } else {
                quote! {}
            };
            let context_stream_fn = if let Some(handle) = struct_stream_handle_with_context {
                quote! {
                    pub async fn dispatch_tool_invoke_stream_with_context(
                        self,
                        receiver: &#receiver_ty,
                        context: &::agena_plugin_sdk::ToolInvokeContext<'_>,
                        sink: ::agena_plugin_sdk::ToolStreamSink,
                    ) -> ::agena_plugin_sdk::Result<::agena_plugin_sdk::ToolStreamEnd> {
                        let parsed = self;
                        #handle(receiver, context, sink, #arg_expr).await
                    }
                }
            } else if let Some(handle) = struct_stream_handle {
                quote! {
                    pub async fn dispatch_tool_invoke_stream_with_context(
                        self,
                        receiver: &#receiver_ty,
                        _context: &::agena_plugin_sdk::ToolInvokeContext<'_>,
                        sink: ::agena_plugin_sdk::ToolStreamSink,
                    ) -> ::agena_plugin_sdk::Result<::agena_plugin_sdk::ToolStreamEnd> {
                        let parsed = self;
                        #handle(receiver, sink, #arg_expr).await
                    }
                }
            } else if let Some(handle) = struct_handle_with_context {
                let call_expr = quote! { #handle(receiver, context, #arg_expr).await };
                let streamified = streamify_invoke_output(call_expr);
                quote! {
                    pub async fn dispatch_tool_invoke_stream_with_context(
                        self,
                        receiver: &#receiver_ty,
                        context: &::agena_plugin_sdk::ToolInvokeContext<'_>,
                        sink: ::agena_plugin_sdk::ToolStreamSink,
                    ) -> ::agena_plugin_sdk::Result<::agena_plugin_sdk::ToolStreamEnd> {
                        let parsed = self;
                        #streamified
                    }
                }
            } else if let Some(handle) = struct_handle {
                let call_expr = quote! { #handle(receiver, #arg_expr).await };
                let streamified = streamify_invoke_output(call_expr);
                quote! {
                    pub async fn dispatch_tool_invoke_stream_with_context(
                        self,
                        receiver: &#receiver_ty,
                        _context: &::agena_plugin_sdk::ToolInvokeContext<'_>,
                        sink: ::agena_plugin_sdk::ToolStreamSink,
                    ) -> ::agena_plugin_sdk::Result<::agena_plugin_sdk::ToolStreamEnd> {
                        let parsed = self;
                        #streamified
                    }
                }
            } else {
                quote! {}
            };
            let permission_paths_fn = if let Some(handle) = struct_permission_paths_handle {
                quote! {
                    pub async fn dispatch_permission_paths(
                        self,
                        receiver: &#receiver_ty,
                    ) -> ::agena_plugin_sdk::Result<Vec<::agena_plugin_sdk::PathRequest>> {
                        let parsed = self;
                        #handle(receiver, #arg_expr).await
                    }
                }
            } else {
                quote! {
                    pub async fn dispatch_permission_paths(
                        self,
                        _receiver: &#receiver_ty,
                    ) -> ::agena_plugin_sdk::Result<Vec<::agena_plugin_sdk::PathRequest>> {
                        Ok(Vec::new())
                    }
                }
            };
            let permission_networks_fn = if let Some(handle) = struct_permission_networks_handle {
                quote! {
                    pub async fn dispatch_permission_networks(
                        self,
                        receiver: &#receiver_ty,
                    ) -> ::agena_plugin_sdk::Result<Vec<::agena_plugin_sdk::NetworkRequest>> {
                        let parsed = self;
                        #handle(receiver, #arg_expr).await
                    }
                }
            } else {
                quote! {
                    pub async fn dispatch_permission_networks(
                        self,
                        _receiver: &#receiver_ty,
                    ) -> ::agena_plugin_sdk::Result<Vec<::agena_plugin_sdk::NetworkRequest>> {
                        Ok(Vec::new())
                    }
                }
            };
            Ok(quote! {
                #plain_fn
                #context_fn
                #plain_stream_fn
                #context_stream_fn
                #permission_paths_fn
                #permission_networks_fn
            })
        }
        Data::Enum(data_enum) => {
            if receiver_ty.is_none() {
                let has_variant_handles = data_enum.variants.iter().any(|variant| {
                    parse_input_variant_config(variant)
                        .ok()
                        .map(|cfg| {
                            cfg.handle.is_some()
                                || cfg.handle_with_context.is_some()
                                || cfg.stream_handle.is_some()
                                || cfg.stream_handle_with_context.is_some()
                                || cfg.permission_paths_handle.is_some()
                                || cfg.permission_networks_handle.is_some()
                        })
                        .unwrap_or(false)
                });
                if has_variant_handles {
                    return Err(syn::Error::new(
                        proc_macro2::Span::call_site(),
                        "variant handle, handle_with_context, stream_handle, stream_handle_with_context, permission_paths_handle, or permission_networks_handle bindings require handler_receiver on the shape",
                    ));
                }
                return Ok(quote! {});
            }
            let receiver_ty = receiver_ty.expect("checked above");
            let mut plain_dispatch_arms = Vec::new();
            let mut context_dispatch_arms = Vec::new();
            let mut plain_stream_dispatch_arms = Vec::new();
            let mut context_stream_dispatch_arms = Vec::new();
            let mut permission_paths_dispatch_arms = Vec::new();
            let mut permission_networks_dispatch_arms = Vec::new();
            let mut saw_any_handle = false;
            let mut saw_context_handle = false;
            let mut can_generate_plain = true;
            let mut can_generate_context = true;
            let mut saw_any_stream_handle = false;
            let mut saw_context_stream_handle = false;
            let mut can_generate_plain_stream = true;
            let mut can_generate_context_stream = true;
            let mut saw_any_permission_paths_handle = false;
            let mut saw_any_permission_networks_handle = false;
            let mut saw_missing_permission_paths_handle = false;
            let mut saw_missing_permission_networks_handle = false;
            for variant in &data_enum.variants {
                let config = parse_input_variant_config(variant)?;
                if config.handle_by_value
                    && config.handle.is_none()
                    && config.handle_with_context.is_none()
                    && config.stream_handle.is_none()
                    && config.stream_handle_with_context.is_none()
                {
                    return Err(syn::Error::new_spanned(
                        variant,
                        "variant handle_by_value requires #[input(handle = path)], #[input(handle_with_context = path)], #[input(stream_handle = path)], or #[input(stream_handle_with_context = path)]",
                    ));
                }
                let plain_handle = config.handle.clone();
                let context_handle = config.handle_with_context.clone();
                let plain_stream_handle = config.stream_handle.clone();
                let context_stream_handle = config.stream_handle_with_context.clone();
                let permission_paths_handle = config.permission_paths_handle.clone();
                let permission_networks_handle = config.permission_networks_handle.clone();
                saw_any_handle |= plain_handle.is_some() || context_handle.is_some();
                saw_context_handle |= context_handle.is_some();
                if plain_handle.is_none() && context_handle.is_none() {
                    can_generate_plain = false;
                    can_generate_context = false;
                }
                saw_any_stream_handle |= plain_stream_handle.is_some()
                    || context_stream_handle.is_some()
                    || plain_handle.is_some()
                    || context_handle.is_some();
                saw_context_stream_handle |= context_stream_handle.is_some();
                if plain_stream_handle.is_none()
                    && context_stream_handle.is_none()
                    && plain_handle.is_none()
                    && context_handle.is_none()
                {
                    can_generate_plain_stream = false;
                    can_generate_context_stream = false;
                }
                saw_any_permission_paths_handle |= permission_paths_handle.is_some();
                saw_any_permission_networks_handle |= permission_networks_handle.is_some();
                if permission_paths_handle.is_none() {
                    saw_missing_permission_paths_handle = true;
                }
                if permission_networks_handle.is_none() {
                    saw_missing_permission_networks_handle = true;
                }
                let (bound_pattern, ignored_pattern, arg_exprs) =
                    dispatch_variant_pattern_and_args(variant, config.handle_by_value)?;
                if let Some(handle) = plain_handle.as_ref() {
                    plain_dispatch_arms.push(
                        quote! { #bound_pattern => #handle(receiver #(, #arg_exprs )*).await },
                    );
                } else if context_handle.is_some() {
                    plain_dispatch_arms.push(
                        quote! { #ignored_pattern => Err(::agena_plugin_sdk::PluginError::invalid_params(
                            "tool dispatch requires invoke context",
                        )) },
                    );
                }
                if can_generate_context {
                    if let Some(handle) = context_handle.as_ref() {
                        context_dispatch_arms.push(
                            quote! { #bound_pattern => #handle(receiver, context #(, #arg_exprs )*).await },
                        );
                    } else if let Some(handle) = plain_handle.as_ref() {
                        context_dispatch_arms.push(
                            quote! { #bound_pattern => #handle(receiver #(, #arg_exprs )*).await },
                        );
                    }
                }
                if let Some(handle) = plain_stream_handle.as_ref() {
                    plain_stream_dispatch_arms.push(
                        quote! { #bound_pattern => #handle(receiver, sink #(, #arg_exprs )*).await },
                    );
                } else if context_stream_handle.is_some() || context_handle.is_some() {
                    plain_stream_dispatch_arms.push(
                        quote! { #ignored_pattern => Err(::agena_plugin_sdk::PluginError::invalid_params(
                            "tool stream dispatch requires invoke context",
                        )) },
                    );
                } else if let Some(handle) = plain_handle.as_ref() {
                    let call_expr = streamify_invoke_output(
                        quote! { #handle(receiver #(, #arg_exprs )*).await },
                    );
                    plain_stream_dispatch_arms.push(quote! { #bound_pattern => #call_expr });
                }
                if can_generate_context_stream {
                    if let Some(handle) = context_stream_handle.as_ref() {
                        context_stream_dispatch_arms.push(
                            quote! { #bound_pattern => #handle(receiver, context, sink #(, #arg_exprs )*).await },
                        );
                    } else if let Some(handle) = plain_stream_handle.as_ref() {
                        context_stream_dispatch_arms.push(
                            quote! { #bound_pattern => #handle(receiver, sink #(, #arg_exprs )*).await },
                        );
                    } else if let Some(handle) = context_handle.as_ref() {
                        let call_expr = streamify_invoke_output(
                            quote! { #handle(receiver, context #(, #arg_exprs )*).await },
                        );
                        context_stream_dispatch_arms.push(quote! { #bound_pattern => #call_expr });
                    } else if let Some(handle) = plain_handle.as_ref() {
                        let call_expr = streamify_invoke_output(
                            quote! { #handle(receiver #(, #arg_exprs )*).await },
                        );
                        context_stream_dispatch_arms.push(quote! { #bound_pattern => #call_expr });
                    }
                }
                if let Some(handle) = permission_paths_handle.as_ref() {
                    permission_paths_dispatch_arms.push(
                        quote! { #bound_pattern => #handle(receiver #(, #arg_exprs )*).await },
                    );
                }
                if let Some(handle) = permission_networks_handle.as_ref() {
                    permission_networks_dispatch_arms.push(
                        quote! { #bound_pattern => #handle(receiver #(, #arg_exprs )*).await },
                    );
                }
            }
            if saw_any_handle && !can_generate_plain && !saw_context_handle {
                return Err(syn::Error::new(
                    proc_macro2::Span::call_site(),
                    "input dispatch requires #[input(handle = path)] on every variant",
                ));
            }
            if saw_context_handle && !can_generate_context {
                return Err(syn::Error::new(
                    proc_macro2::Span::call_site(),
                    "context-aware input dispatch requires #[input(handle = path)] or #[input(handle_with_context = path)] on every variant",
                ));
            }
            if saw_any_stream_handle && !can_generate_plain_stream && !saw_context_stream_handle {
                return Err(syn::Error::new(
                    proc_macro2::Span::call_site(),
                    "input stream dispatch requires #[input(stream_handle = path)], #[input(stream_handle_with_context = path)], #[input(handle = path)], or #[input(handle_with_context = path)] on every variant",
                ));
            }
            if saw_context_stream_handle && !can_generate_context_stream {
                return Err(syn::Error::new(
                    proc_macro2::Span::call_site(),
                    "context-aware input stream dispatch requires #[input(stream_handle = path)], #[input(stream_handle_with_context = path)], #[input(handle = path)], or #[input(handle_with_context = path)] on every variant",
                ));
            }
            if saw_any_permission_paths_handle && saw_missing_permission_paths_handle {
                return Err(syn::Error::new(
                    proc_macro2::Span::call_site(),
                    "input permission path dispatch requires #[input(permission_paths_handle = path)] on every variant",
                ));
            }
            if saw_any_permission_networks_handle && saw_missing_permission_networks_handle {
                return Err(syn::Error::new(
                    proc_macro2::Span::call_site(),
                    "input permission network dispatch requires #[input(permission_networks_handle = path)] on every variant",
                ));
            }
            let plain_fn = if can_generate_plain && !plain_dispatch_arms.is_empty() {
                quote! {
                    pub async fn dispatch_tool_invoke(
                        self,
                        receiver: &#receiver_ty,
                    ) -> ::agena_plugin_sdk::Result<::agena_plugin_sdk::ToolInvokeOutput> {
                        match self {
                            #(#plain_dispatch_arms),*
                        }
                    }
                }
            } else {
                quote! {}
            };
            let context_fn = if can_generate_context && !context_dispatch_arms.is_empty() {
                quote! {
                    pub async fn dispatch_tool_invoke_with_context(
                        self,
                        receiver: &#receiver_ty,
                        context: &::agena_plugin_sdk::ToolInvokeContext<'_>,
                    ) -> ::agena_plugin_sdk::Result<::agena_plugin_sdk::ToolInvokeOutput> {
                        match self {
                            #(#context_dispatch_arms),*
                        }
                    }
                }
            } else {
                quote! {}
            };
            let plain_stream_fn =
                if can_generate_plain_stream && !plain_stream_dispatch_arms.is_empty() {
                    quote! {
                        pub async fn dispatch_tool_invoke_stream(
                            self,
                            receiver: &#receiver_ty,
                            sink: ::agena_plugin_sdk::ToolStreamSink,
                        ) -> ::agena_plugin_sdk::Result<::agena_plugin_sdk::ToolStreamEnd> {
                            match self {
                                #(#plain_stream_dispatch_arms),*
                            }
                        }
                    }
                } else {
                    quote! {}
                };
            let context_stream_fn =
                if can_generate_context_stream && !context_stream_dispatch_arms.is_empty() {
                    quote! {
                        pub async fn dispatch_tool_invoke_stream_with_context(
                            self,
                            receiver: &#receiver_ty,
                            context: &::agena_plugin_sdk::ToolInvokeContext<'_>,
                            sink: ::agena_plugin_sdk::ToolStreamSink,
                        ) -> ::agena_plugin_sdk::Result<::agena_plugin_sdk::ToolStreamEnd> {
                            match self {
                                #(#context_stream_dispatch_arms),*
                            }
                        }
                    }
                } else {
                    quote! {}
                };
            let permission_paths_fn = if saw_any_permission_paths_handle {
                quote! {
                    pub async fn dispatch_permission_paths(
                        self,
                        receiver: &#receiver_ty,
                    ) -> ::agena_plugin_sdk::Result<Vec<::agena_plugin_sdk::PathRequest>> {
                        match self {
                            #(#permission_paths_dispatch_arms),*
                        }
                    }
                }
            } else {
                quote! {
                    pub async fn dispatch_permission_paths(
                        self,
                        _receiver: &#receiver_ty,
                    ) -> ::agena_plugin_sdk::Result<Vec<::agena_plugin_sdk::PathRequest>> {
                        Ok(Vec::new())
                    }
                }
            };
            let permission_networks_fn = if saw_any_permission_networks_handle {
                quote! {
                    pub async fn dispatch_permission_networks(
                        self,
                        receiver: &#receiver_ty,
                    ) -> ::agena_plugin_sdk::Result<Vec<::agena_plugin_sdk::NetworkRequest>> {
                        match self {
                            #(#permission_networks_dispatch_arms),*
                        }
                    }
                }
            } else {
                quote! {
                    pub async fn dispatch_permission_networks(
                        self,
                        _receiver: &#receiver_ty,
                    ) -> ::agena_plugin_sdk::Result<Vec<::agena_plugin_sdk::NetworkRequest>> {
                        Ok(Vec::new())
                    }
                }
            };
            Ok(quote! {
                #plain_fn
                #context_fn
                #plain_stream_fn
                #context_stream_fn
                #permission_paths_fn
                #permission_networks_fn
            })
        }
        Data::Union(_) => Ok(quote! {}),
    }
}

fn expand_schema_metadata_fn<C, F>(
    attrs: &[Attribute],
    data: &Data,
    constraints: &C,
    mut variant_constraint_metadata: F,
) -> Result<proc_macro2::TokenStream>
where
    C: SchemaConstraintSource + SchemaRelationSource,
    F: FnMut(&Variant, &str) -> Result<Vec<proc_macro2::TokenStream>>,
{
    let mut metadata_calls = Vec::new();
    match data {
        Data::Struct(data_struct) => {
            let rename_rule = serde_rename_all_rule(attrs)?;
            metadata_calls.extend(tool_input_struct_field_schema_metadata_calls(
                "",
                &data_struct.fields,
                rename_rule,
            )?);
            metadata_calls.extend(constraint_schema_metadata_calls("", constraints)?);
            metadata_calls.extend(constraint_relation_metadata_calls("", constraints)?);
        }
        Data::Enum(data_enum) => {
            let enum_field_rule = serde_rename_all_fields_rule(attrs)?;
            for (index, variant) in data_enum.variants.iter().enumerate() {
                if let Some(description) = doc_text(&variant.attrs).and_then(|text| {
                    let trimmed = text.trim().to_string();
                    (!trimmed.is_empty()).then_some(trimmed)
                }) {
                    let description = LitStr::new(&description, variant.ident.span());
                    for group in ["oneOf", "anyOf", "allOf"] {
                        let pointer =
                            LitStr::new(format!("/{group}/{index}").as_str(), variant.ident.span());
                        metadata_calls.push(quote! {
                            ::agena_plugin_sdk::macro_support::set_schema_metadata(
                                schema,
                                #pointer,
                                None,
                                Some(#description),
                            );
                        });
                    }
                }
                for group in ["oneOf", "anyOf", "allOf"] {
                    let prefix = format!("/{group}/{index}");
                    let variant_field_rule =
                        serde_rename_all_rule(&variant.attrs)?.or(enum_field_rule);
                    metadata_calls.extend(tool_input_struct_field_schema_metadata_calls(
                        prefix.as_str(),
                        &variant.fields,
                        variant_field_rule,
                    )?);
                    metadata_calls.extend(constraint_schema_metadata_calls(
                        prefix.as_str(),
                        constraints,
                    )?);
                    metadata_calls.extend(constraint_relation_metadata_calls(
                        prefix.as_str(),
                        constraints,
                    )?);
                    metadata_calls.extend(variant_constraint_metadata(variant, prefix.as_str())?);
                }
            }
        }
        Data::Union(_) => {}
    }

    Ok(quote! {
        fn __macro_apply_schema_metadata(schema: &mut serde_json::Value) {
            #(#metadata_calls)*
        }
    })
}

fn tool_spec_schema_metadata_calls(spec: &ToolSpecConfig) -> Result<Vec<proc_macro2::TokenStream>> {
    let mut metadata_calls = constraint_schema_metadata_calls("", spec)?;
    metadata_calls.extend(constraint_relation_metadata_calls("", spec)?);
    Ok(metadata_calls)
}

fn constraint_relation_metadata_calls<C: SchemaRelationSource + SchemaConstraintSource>(
    prefix: &str,
    constraints: &C,
) -> Result<Vec<proc_macro2::TokenStream>> {
    let mut labels = Vec::new();
    let display_path = |path: &LitStr| {
        schema_relation_display_path(path.value().as_str(), constraints.input_field_metadata())
    };
    for group in constraints.exactly_one_of() {
        if !group.is_empty() {
            let joined = group
                .iter()
                .map(|path| format!("`{}`", display_path(path)))
                .collect::<Vec<_>>()
                .join(", ");
            labels.push(format!("exactly_one_of: {joined}"));
        }
    }
    for group in constraints.at_least_one_of() {
        if !group.is_empty() {
            let joined = group
                .iter()
                .map(|path| format!("`{}`", display_path(path)))
                .collect::<Vec<_>>()
                .join(", ");
            labels.push(format!("at_least_one_of: {joined}"));
        }
    }
    for constraint in constraints.requires() {
        labels.push(format!(
            "requires `{}` -> `{}`",
            display_path(&constraint.left),
            display_path(&constraint.right)
        ));
    }
    for constraint in constraints.conflicts_with() {
        labels.push(format!(
            "conflicts_with `{}` x `{}`",
            display_path(&constraint.left),
            display_path(&constraint.right)
        ));
    }
    for constraint in constraints.required_unless_present() {
        labels.push(format!(
            "required_unless_present `{}` unless `{}` present",
            display_path(&constraint.left),
            display_path(&constraint.right)
        ));
    }
    for constraint in constraints.forbid_substrings() {
        let joined = constraint
            .values
            .iter()
            .map(|value| format!("\"{}\"", value.value()))
            .collect::<Vec<_>>()
            .join(", ");
        labels.push(format!(
            "forbid_substrings `{}`: {joined}",
            display_path(&constraint.path)
        ));
    }
    for path in constraints.distinct_trimmed() {
        labels.push(format!("distinct_trimmed `{}`", display_path(path)));
    }
    for constraint in constraints.distinct_trimmed_within() {
        labels.push(format!(
            "distinct_trimmed_within `{}` within `{}`",
            display_path(&constraint.left),
            display_path(&constraint.right)
        ));
    }
    if labels.is_empty() {
        return Ok(Vec::new());
    }
    let labels = labels
        .into_iter()
        .map(|label| LitStr::new(label.as_str(), proc_macro2::Span::call_site()))
        .collect::<Vec<_>>();
    let pointer = LitStr::new(prefix, proc_macro2::Span::call_site());
    Ok(vec![quote! {
        ::agena_plugin_sdk::macro_support::set_schema_string_list_metadata(
            schema,
            #pointer,
            "x-agena-relations",
            &[#(#labels),*],
        );
    }])
}

fn schema_relation_display_path(path: &str, metadata: &[PluginInputFieldMetadata]) -> String {
    if let Some(mapped) = metadata
        .iter()
        .find(|field| field.parse_path.value() == path)
        .map(|field| field.path.value())
    {
        return mapped;
    }
    let head_end = path.find('.').unwrap_or(path.len());
    let (head, tail) = path.split_at(head_end);
    let mut base = head;
    let mut suffix = String::new();
    while let Some(stripped) = base.strip_suffix("[]") {
        base = stripped;
        suffix.push_str("[]");
    }
    if let Some(mapped) = metadata
        .iter()
        .find(|field| field.parse_path.value() == base)
        .map(|field| field.path.value())
    {
        return format!("{mapped}{suffix}{tail}");
    }
    path.to_string()
}

fn constraint_schema_metadata_calls<C: SchemaConstraintSource>(
    prefix: &str,
    constraints: &C,
) -> Result<Vec<proc_macro2::TokenStream>> {
    let mut calls = Vec::new();
    let non_empty_metadata_parse_paths = constraints
        .input_field_metadata()
        .iter()
        .filter(|metadata| metadata.non_empty)
        .map(|metadata| metadata.parse_path.value())
        .collect::<BTreeSet<_>>();
    let minimum_metadata_parse_paths = constraints
        .input_field_metadata()
        .iter()
        .filter(|metadata| metadata.minimum.is_some())
        .map(|metadata| metadata.parse_path.value())
        .collect::<BTreeSet<_>>();
    let maximum_metadata_parse_paths = constraints
        .input_field_metadata()
        .iter()
        .filter(|metadata| metadata.maximum.is_some())
        .map(|metadata| metadata.parse_path.value())
        .collect::<BTreeSet<_>>();
    let exclusive_minimum_metadata_parse_paths = constraints
        .input_field_metadata()
        .iter()
        .filter(|metadata| metadata.exclusive_minimum.is_some())
        .map(|metadata| metadata.parse_path.value())
        .collect::<BTreeSet<_>>();
    let exclusive_maximum_metadata_parse_paths = constraints
        .input_field_metadata()
        .iter()
        .filter(|metadata| metadata.exclusive_maximum.is_some())
        .map(|metadata| metadata.parse_path.value())
        .collect::<BTreeSet<_>>();
    let item_minimum_metadata_parse_paths = constraints
        .input_field_metadata()
        .iter()
        .filter_map(|metadata| {
            metadata
                .item_minimum
                .as_ref()
                .map(|_| format!("{}[]", metadata.parse_path.value()))
        })
        .collect::<BTreeSet<_>>();
    let item_maximum_metadata_parse_paths = constraints
        .input_field_metadata()
        .iter()
        .filter_map(|metadata| {
            metadata
                .item_maximum
                .as_ref()
                .map(|_| format!("{}[]", metadata.parse_path.value()))
        })
        .collect::<BTreeSet<_>>();
    let item_exclusive_minimum_metadata_parse_paths = constraints
        .input_field_metadata()
        .iter()
        .filter_map(|metadata| {
            metadata
                .item_exclusive_minimum
                .as_ref()
                .map(|_| format!("{}[]", metadata.parse_path.value()))
        })
        .collect::<BTreeSet<_>>();
    let item_exclusive_maximum_metadata_parse_paths = constraints
        .input_field_metadata()
        .iter()
        .filter_map(|metadata| {
            metadata
                .item_exclusive_maximum
                .as_ref()
                .map(|_| format!("{}[]", metadata.parse_path.value()))
        })
        .collect::<BTreeSet<_>>();
    let min_items_metadata_parse_paths = constraints
        .input_field_metadata()
        .iter()
        .filter(|metadata| metadata.min_items.is_some())
        .map(|metadata| metadata.parse_path.value())
        .collect::<BTreeSet<_>>();
    let max_items_metadata_parse_paths = constraints
        .input_field_metadata()
        .iter()
        .filter(|metadata| metadata.max_items.is_some())
        .map(|metadata| metadata.parse_path.value())
        .collect::<BTreeSet<_>>();
    let min_properties_metadata_parse_paths = constraints
        .input_field_metadata()
        .iter()
        .filter(|metadata| metadata.min_properties.is_some())
        .map(|metadata| metadata.parse_path.value())
        .collect::<BTreeSet<_>>();
    let max_properties_metadata_parse_paths = constraints
        .input_field_metadata()
        .iter()
        .filter(|metadata| metadata.max_properties.is_some())
        .map(|metadata| metadata.parse_path.value())
        .collect::<BTreeSet<_>>();
    let item_min_properties_metadata_parse_paths = constraints
        .input_field_metadata()
        .iter()
        .filter_map(|metadata| {
            metadata
                .item_min_properties
                .map(|_| format!("{}[]", metadata.parse_path.value()))
        })
        .collect::<BTreeSet<_>>();
    let item_max_properties_metadata_parse_paths = constraints
        .input_field_metadata()
        .iter()
        .filter_map(|metadata| {
            metadata
                .item_max_properties
                .map(|_| format!("{}[]", metadata.parse_path.value()))
        })
        .collect::<BTreeSet<_>>();
    let min_chars_metadata_parse_paths = constraints
        .input_field_metadata()
        .iter()
        .filter(|metadata| metadata.min_chars.is_some())
        .map(|metadata| metadata.parse_path.value())
        .collect::<BTreeSet<_>>();
    let max_chars_metadata_parse_paths = constraints
        .input_field_metadata()
        .iter()
        .filter(|metadata| metadata.max_chars.is_some())
        .map(|metadata| metadata.parse_path.value())
        .collect::<BTreeSet<_>>();
    let item_min_chars_metadata_parse_paths = constraints
        .input_field_metadata()
        .iter()
        .filter_map(|metadata| {
            metadata
                .item_min_chars
                .map(|_| format!("{}[]", metadata.parse_path.value()))
        })
        .collect::<BTreeSet<_>>();
    let item_max_chars_metadata_parse_paths = constraints
        .input_field_metadata()
        .iter()
        .filter_map(|metadata| {
            metadata
                .item_max_chars
                .map(|_| format!("{}[]", metadata.parse_path.value()))
        })
        .collect::<BTreeSet<_>>();
    let format_metadata_parse_paths = constraints
        .input_field_metadata()
        .iter()
        .filter(|metadata| metadata.format.is_some())
        .map(|metadata| metadata.parse_path.value())
        .collect::<BTreeSet<_>>();
    let item_format_metadata_parse_paths = constraints
        .input_field_metadata()
        .iter()
        .filter_map(|metadata| {
            metadata
                .item_format
                .as_ref()
                .map(|_| format!("{}[]", metadata.parse_path.value()))
        })
        .collect::<BTreeSet<_>>();
    let pattern_metadata_parse_paths = constraints
        .input_field_metadata()
        .iter()
        .filter(|metadata| metadata.pattern.is_some())
        .map(|metadata| metadata.parse_path.value())
        .collect::<BTreeSet<_>>();
    let item_pattern_metadata_parse_paths = constraints
        .input_field_metadata()
        .iter()
        .filter_map(|metadata| {
            metadata
                .item_pattern
                .as_ref()
                .map(|_| format!("{}[]", metadata.parse_path.value()))
        })
        .collect::<BTreeSet<_>>();
    let item_choice_metadata_parse_paths = constraints
        .input_field_metadata()
        .iter()
        .filter_map(|metadata| {
            (!metadata.item_choices.is_empty())
                .then(|| format!("{}[]", metadata.parse_path.value()))
        })
        .collect::<BTreeSet<_>>();
    let choice_metadata_parse_paths = constraints
        .input_field_metadata()
        .iter()
        .filter(|metadata| !metadata.choices.is_empty())
        .map(|metadata| metadata.parse_path.value())
        .collect::<BTreeSet<_>>();
    calls.extend(
        constraints
            .input_field_metadata()
            .iter()
            .enumerate()
            .map(|(index, metadata)| {
                let pointer = schema_pointer_from_logical_path(prefix, &metadata.path.value())?;
                let pointer = LitStr::new(pointer.as_str(), metadata.path.span());
                let order = LitStr::new(format!("{index:06}").as_str(), metadata.path.span());
                let mut calls = Vec::new();
                calls.push(quote! {
                    ::agena_plugin_sdk::macro_support::set_schema_string_metadata(
                        schema,
                        #pointer,
                        "x-agena-order",
                        #order,
                    );
                });
                if let Some(description) = metadata.description.as_ref() {
                    calls.push(quote! {
                        ::agena_plugin_sdk::macro_support::set_schema_metadata(
                            schema,
                            #pointer,
                            None,
                            Some(#description),
                        );
                    });
                }
                if let Some(kind) = metadata.path_kind {
                    let label = LitStr::new(path_permission_kind_label(kind), metadata.path.span());
                    calls.push(quote! {
                        ::agena_plugin_sdk::macro_support::set_schema_string_metadata(
                            schema,
                            #pointer,
                            "x-agena-path",
                            #label,
                        );
                    });
                }
                if let Some(network) = metadata.network {
                    let label = LitStr::new(network_semantic_label(network), metadata.path.span());
                    calls.push(quote! {
                        ::agena_plugin_sdk::macro_support::set_schema_string_metadata(
                            schema,
                            #pointer,
                            "x-agena-network",
                            #label,
                        );
                    });
                }
                if metadata.non_empty {
                    calls.push(quote! {
                        ::agena_plugin_sdk::macro_support::set_schema_non_empty_metadata(
                            schema,
                            #pointer,
                        );
                    });
                }
                if metadata.item_non_empty || metadata.item_non_empty_if_present {
                    let item_pointer = LitStr::new(
                        format!("{}/items", pointer.value()).as_str(),
                        metadata.path.span(),
                    );
                    calls.push(quote! {
                        ::agena_plugin_sdk::macro_support::set_schema_non_empty_metadata(
                            schema,
                            #item_pointer,
                        );
                    });
                }
                if let Some(minimum) = metadata.minimum.as_ref() {
                    calls.push(quote! {
                        ::agena_plugin_sdk::macro_support::set_schema_number_metadata(
                            schema,
                            #pointer,
                            "minimum",
                            ::agena_plugin_sdk::serde_json::json!(#minimum),
                        );
                    });
                }
                if let Some(maximum) = metadata.maximum.as_ref() {
                    calls.push(quote! {
                        ::agena_plugin_sdk::macro_support::set_schema_number_metadata(
                            schema,
                            #pointer,
                            "maximum",
                            ::agena_plugin_sdk::serde_json::json!(#maximum),
                        );
                    });
                }
                if let Some(minimum) = metadata.exclusive_minimum.as_ref() {
                    calls.push(quote! {
                        ::agena_plugin_sdk::macro_support::set_schema_number_metadata(
                            schema,
                            #pointer,
                            "exclusiveMinimum",
                            ::agena_plugin_sdk::serde_json::json!(#minimum),
                        );
                    });
                }
                if let Some(maximum) = metadata.exclusive_maximum.as_ref() {
                    calls.push(quote! {
                        ::agena_plugin_sdk::macro_support::set_schema_number_metadata(
                            schema,
                            #pointer,
                            "exclusiveMaximum",
                            ::agena_plugin_sdk::serde_json::json!(#maximum),
                        );
                    });
                }
                if let Some(minimum) = metadata.item_minimum.as_ref() {
                    let item_pointer = LitStr::new(
                        format!("{}/items", pointer.value()).as_str(),
                        metadata.path.span(),
                    );
                    calls.push(quote! {
                        ::agena_plugin_sdk::macro_support::set_schema_number_metadata(
                            schema,
                            #item_pointer,
                            "minimum",
                            ::agena_plugin_sdk::serde_json::json!(#minimum),
                        );
                    });
                }
                if let Some(minimum) = metadata.item_exclusive_minimum.as_ref() {
                    let item_pointer = LitStr::new(
                        format!("{}/items", pointer.value()).as_str(),
                        metadata.path.span(),
                    );
                    calls.push(quote! {
                        ::agena_plugin_sdk::macro_support::set_schema_number_metadata(
                            schema,
                            #item_pointer,
                            "exclusiveMinimum",
                            ::agena_plugin_sdk::serde_json::json!(#minimum),
                        );
                    });
                }
                if let Some(maximum) = metadata.item_exclusive_maximum.as_ref() {
                    let item_pointer = LitStr::new(
                        format!("{}/items", pointer.value()).as_str(),
                        metadata.path.span(),
                    );
                    calls.push(quote! {
                        ::agena_plugin_sdk::macro_support::set_schema_number_metadata(
                            schema,
                            #item_pointer,
                            "exclusiveMaximum",
                            ::agena_plugin_sdk::serde_json::json!(#maximum),
                        );
                    });
                }
                if let Some(maximum) = metadata.item_maximum.as_ref() {
                    let item_pointer = LitStr::new(
                        format!("{}/items", pointer.value()).as_str(),
                        metadata.path.span(),
                    );
                    calls.push(quote! {
                        ::agena_plugin_sdk::macro_support::set_schema_number_metadata(
                            schema,
                            #item_pointer,
                            "maximum",
                            ::agena_plugin_sdk::serde_json::json!(#maximum),
                        );
                    });
                }
                if let Some(value) = metadata.min_items {
                    let value = value as u64;
                    calls.push(quote! {
                        ::agena_plugin_sdk::macro_support::set_schema_u64_metadata(
                            schema,
                            #pointer,
                            "minItems",
                            #value,
                        );
                    });
                }
                if let Some(value) = metadata.max_items {
                    let value = value as u64;
                    calls.push(quote! {
                        ::agena_plugin_sdk::macro_support::set_schema_u64_metadata(
                            schema,
                            #pointer,
                            "maxItems",
                            #value,
                        );
                    });
                }
                if let Some(value) = metadata.min_properties {
                    let value = value as u64;
                    calls.push(quote! {
                        ::agena_plugin_sdk::macro_support::set_schema_minimum_u64_metadata(
                            schema,
                            #pointer,
                            "minProperties",
                            #value,
                        );
                    });
                }
                if let Some(value) = metadata.max_properties {
                    let value = value as u64;
                    calls.push(quote! {
                        ::agena_plugin_sdk::macro_support::set_schema_u64_metadata(
                            schema,
                            #pointer,
                            "maxProperties",
                            #value,
                        );
                    });
                }
                if let Some(value) = metadata.item_min_properties {
                    let item_pointer = LitStr::new(
                        format!("{}/items", pointer.value()).as_str(),
                        metadata.path.span(),
                    );
                    let value = value as u64;
                    calls.push(quote! {
                        ::agena_plugin_sdk::macro_support::set_schema_minimum_u64_metadata(
                            schema,
                            #item_pointer,
                            "minProperties",
                            #value,
                        );
                    });
                }
                if let Some(value) = metadata.item_max_properties {
                    let item_pointer = LitStr::new(
                        format!("{}/items", pointer.value()).as_str(),
                        metadata.path.span(),
                    );
                    let value = value as u64;
                    calls.push(quote! {
                        ::agena_plugin_sdk::macro_support::set_schema_u64_metadata(
                            schema,
                            #item_pointer,
                            "maxProperties",
                            #value,
                        );
                    });
                }
                if let Some(value) = metadata.min_chars {
                    let value = value as u64;
                    calls.push(quote! {
                        ::agena_plugin_sdk::macro_support::set_schema_minimum_u64_metadata(
                            schema,
                            #pointer,
                            "minLength",
                            #value,
                        );
                    });
                }
                if let Some(value) = metadata.max_chars {
                    let value = value as u64;
                    calls.push(quote! {
                        ::agena_plugin_sdk::macro_support::set_schema_u64_metadata(
                            schema,
                            #pointer,
                            "maxLength",
                            #value,
                        );
                    });
                }
                if let Some(value) = metadata.item_min_chars {
                    let item_pointer = LitStr::new(
                        format!("{}/items", pointer.value()).as_str(),
                        metadata.path.span(),
                    );
                    let value = value as u64;
                    calls.push(quote! {
                        ::agena_plugin_sdk::macro_support::set_schema_minimum_u64_metadata(
                            schema,
                            #item_pointer,
                            "minLength",
                            #value,
                        );
                    });
                }
                if let Some(value) = metadata.item_max_chars {
                    let item_pointer = LitStr::new(
                        format!("{}/items", pointer.value()).as_str(),
                        metadata.path.span(),
                    );
                    let value = value as u64;
                    calls.push(quote! {
                        ::agena_plugin_sdk::macro_support::set_schema_u64_metadata(
                            schema,
                            #item_pointer,
                            "maxLength",
                            #value,
                        );
                    });
                }
                if let Some(format) = metadata.format.as_ref() {
                    calls.push(quote! {
                        ::agena_plugin_sdk::macro_support::set_schema_string_metadata(
                            schema,
                            #pointer,
                            "format",
                            #format,
                        );
                    });
                }
                if let Some(format) = metadata.item_format.as_ref() {
                    let item_pointer = LitStr::new(
                        format!("{}/items", pointer.value()).as_str(),
                        metadata.path.span(),
                    );
                    calls.push(quote! {
                        ::agena_plugin_sdk::macro_support::set_schema_string_metadata(
                            schema,
                            #item_pointer,
                            "format",
                            #format,
                        );
                    });
                }
                if let Some(pattern) = metadata.pattern.as_ref() {
                    calls.push(quote! {
                        ::agena_plugin_sdk::macro_support::set_schema_string_metadata(
                            schema,
                            #pointer,
                            "pattern",
                            #pattern,
                        );
                    });
                }
                if let Some(pattern) = metadata.item_pattern.as_ref() {
                    let item_pointer = LitStr::new(
                        format!("{}/items", pointer.value()).as_str(),
                        metadata.path.span(),
                    );
                    calls.push(quote! {
                        ::agena_plugin_sdk::macro_support::set_schema_string_metadata(
                            schema,
                            #item_pointer,
                            "pattern",
                            #pattern,
                        );
                    });
                }
                if !metadata.item_choices.is_empty() {
                    let item_pointer = LitStr::new(
                        format!("{}/items", pointer.value()).as_str(),
                        metadata.path.span(),
                    );
                    let values = metadata.item_choices.iter().collect::<Vec<_>>();
                    calls.push(quote! {
                        ::agena_plugin_sdk::macro_support::set_schema_value_list_metadata(
                            schema,
                            #item_pointer,
                            "enum",
                            &[#(::agena_plugin_sdk::serde_json::json!(#values)),*],
                        );
                    });
                }
                if let Some(picker) = metadata.picker {
                    let label = LitStr::new(picker_kind_label(picker), metadata.path.span());
                    calls.push(quote! {
                        ::agena_plugin_sdk::macro_support::set_schema_string_metadata(
                            schema,
                            #pointer,
                            "x-agena-picker",
                            #label,
                        );
                    });
                }
                if metadata.secret {
                    calls.push(quote! {
                        ::agena_plugin_sdk::macro_support::set_schema_bool_metadata(
                            schema,
                            #pointer,
                            "x-agena-secret",
                            true,
                        );
                    });
                }
                if let Some(example) = metadata.example.as_ref() {
                    calls.push(quote! {
                        ::agena_plugin_sdk::macro_support::set_schema_value_list_metadata(
                            schema,
                            #pointer,
                            "examples",
                            &[::agena_plugin_sdk::serde_json::json!(#example)],
                        );
                    });
                }
                if !metadata.choices.is_empty() {
                    let values = metadata.choices.iter().collect::<Vec<_>>();
                    calls.push(quote! {
                        ::agena_plugin_sdk::macro_support::set_schema_value_list_metadata(
                            schema,
                            #pointer,
                            "enum",
                            &[#(::agena_plugin_sdk::serde_json::json!(#values)),*],
                        );
                    });
                }
                if !metadata.aliases.is_empty() {
                    let aliases = metadata.aliases.iter().collect::<Vec<_>>();
                    calls.push(quote! {
                        ::agena_plugin_sdk::macro_support::set_schema_string_list_metadata(
                            schema,
                            #pointer,
                            "x-agena-aliases",
                            &[#(#aliases),*],
                        );
                    });
                }
                if metadata.path.value() != metadata.parse_path.value() {
                    let parse_path = &metadata.parse_path;
                    calls.push(quote! {
                        ::agena_plugin_sdk::macro_support::set_schema_string_metadata(
                            schema,
                            #pointer,
                            "x-agena-parse-name",
                            #parse_path,
                        );
                    });
                }
                Ok(quote! { #(#calls)* })
            })
            .collect::<Result<Vec<_>>>()?,
    );
    calls.extend(
        constraints
            .choices()
            .iter()
            .filter(|constraint| {
                !choice_metadata_parse_paths.contains(&constraint.path.value())
                    && !item_choice_metadata_parse_paths.contains(&constraint.path.value())
            })
            .map(|constraint| {
                let pointer = schema_pointer_from_logical_path(prefix, &constraint.path.value())?;
                let pointer = LitStr::new(pointer.as_str(), constraint.path.span());
                let values = &constraint.values;
                Ok(quote! {
                    ::agena_plugin_sdk::macro_support::set_schema_value_list_metadata(
                        schema,
                        #pointer,
                        "enum",
                        &[#(::agena_plugin_sdk::serde_json::json!(#values)),*],
                    );
                })
            })
            .collect::<Result<Vec<_>>>()?,
    );
    calls.extend(
        constraints
            .non_empty()
            .iter()
            .chain(constraints.non_empty_if_present().iter())
            .filter(|path| !non_empty_metadata_parse_paths.contains(&path.value()))
            .map(|path| {
                let pointer = schema_pointer_from_logical_path(prefix, &path.value())?;
                let pointer = LitStr::new(pointer.as_str(), path.span());
                Ok(quote! {
                    ::agena_plugin_sdk::macro_support::set_schema_non_empty_metadata(
                        schema,
                        #pointer,
                    );
                })
            })
            .collect::<Result<Vec<_>>>()?,
    );
    calls.extend(
        constraints
            .minimums()
            .iter()
            .filter(|constraint| {
                !minimum_metadata_parse_paths.contains(&constraint.path.value())
                    && !item_minimum_metadata_parse_paths.contains(&constraint.path.value())
            })
            .map(|constraint| {
                let pointer = schema_pointer_from_logical_path(prefix, &constraint.path.value())?;
                let pointer = LitStr::new(pointer.as_str(), constraint.path.span());
                let value = &constraint.value;
                Ok(quote! {
                    ::agena_plugin_sdk::macro_support::set_schema_number_metadata(
                        schema,
                        #pointer,
                        "minimum",
                        ::agena_plugin_sdk::serde_json::json!(#value),
                    );
                })
            })
            .collect::<Result<Vec<_>>>()?,
    );
    calls.extend(
        constraints
            .maximums()
            .iter()
            .filter(|constraint| {
                !maximum_metadata_parse_paths.contains(&constraint.path.value())
                    && !item_maximum_metadata_parse_paths.contains(&constraint.path.value())
            })
            .map(|constraint| {
                let pointer = schema_pointer_from_logical_path(prefix, &constraint.path.value())?;
                let pointer = LitStr::new(pointer.as_str(), constraint.path.span());
                let value = &constraint.value;
                Ok(quote! {
                    ::agena_plugin_sdk::macro_support::set_schema_number_metadata(
                        schema,
                        #pointer,
                        "maximum",
                        ::agena_plugin_sdk::serde_json::json!(#value),
                    );
                })
            })
            .collect::<Result<Vec<_>>>()?,
    );
    calls.extend(
        constraints
            .exclusive_minimums()
            .iter()
            .filter(|constraint| {
                !exclusive_minimum_metadata_parse_paths.contains(&constraint.path.value())
                    && !item_exclusive_minimum_metadata_parse_paths
                        .contains(&constraint.path.value())
            })
            .map(|constraint| {
                let pointer = schema_pointer_from_logical_path(prefix, &constraint.path.value())?;
                let pointer = LitStr::new(pointer.as_str(), constraint.path.span());
                let value = &constraint.value;
                Ok(quote! {
                    ::agena_plugin_sdk::macro_support::set_schema_number_metadata(
                        schema,
                        #pointer,
                        "exclusiveMinimum",
                        ::agena_plugin_sdk::serde_json::json!(#value),
                    );
                })
            })
            .collect::<Result<Vec<_>>>()?,
    );
    calls.extend(
        constraints
            .exclusive_maximums()
            .iter()
            .filter(|constraint| {
                !exclusive_maximum_metadata_parse_paths.contains(&constraint.path.value())
                    && !item_exclusive_maximum_metadata_parse_paths
                        .contains(&constraint.path.value())
            })
            .map(|constraint| {
                let pointer = schema_pointer_from_logical_path(prefix, &constraint.path.value())?;
                let pointer = LitStr::new(pointer.as_str(), constraint.path.span());
                let value = &constraint.value;
                Ok(quote! {
                    ::agena_plugin_sdk::macro_support::set_schema_number_metadata(
                        schema,
                        #pointer,
                        "exclusiveMaximum",
                        ::agena_plugin_sdk::serde_json::json!(#value),
                    );
                })
            })
            .collect::<Result<Vec<_>>>()?,
    );
    calls.extend(
        constraints
            .min_items()
            .iter()
            .filter(|constraint| !min_items_metadata_parse_paths.contains(&constraint.path.value()))
            .map(|constraint| {
                let pointer = schema_pointer_from_logical_path(prefix, &constraint.path.value())?;
                let pointer = LitStr::new(pointer.as_str(), constraint.path.span());
                let value = constraint.value as u64;
                Ok(quote! {
                    ::agena_plugin_sdk::macro_support::set_schema_u64_metadata(
                        schema,
                        #pointer,
                        "minItems",
                        #value,
                    );
                })
            })
            .collect::<Result<Vec<_>>>()?,
    );
    calls.extend(
        constraints
            .max_items()
            .iter()
            .filter(|constraint| !max_items_metadata_parse_paths.contains(&constraint.path.value()))
            .map(|constraint| {
                let pointer = schema_pointer_from_logical_path(prefix, &constraint.path.value())?;
                let pointer = LitStr::new(pointer.as_str(), constraint.path.span());
                let value = constraint.value as u64;
                Ok(quote! {
                    ::agena_plugin_sdk::macro_support::set_schema_u64_metadata(
                        schema,
                        #pointer,
                        "maxItems",
                        #value,
                    );
                })
            })
            .collect::<Result<Vec<_>>>()?,
    );
    calls.extend(
        constraints
            .min_properties()
            .iter()
            .filter(|constraint| {
                !min_properties_metadata_parse_paths.contains(&constraint.path.value())
                    && !item_min_properties_metadata_parse_paths.contains(&constraint.path.value())
            })
            .map(|constraint| {
                let pointer = schema_pointer_from_logical_path(prefix, &constraint.path.value())?;
                let pointer = LitStr::new(pointer.as_str(), constraint.path.span());
                let value = constraint.value as u64;
                Ok(quote! {
                    ::agena_plugin_sdk::macro_support::set_schema_minimum_u64_metadata(
                        schema,
                        #pointer,
                        "minProperties",
                        #value,
                    );
                })
            })
            .collect::<Result<Vec<_>>>()?,
    );
    calls.extend(
        constraints
            .max_properties()
            .iter()
            .filter(|constraint| {
                !max_properties_metadata_parse_paths.contains(&constraint.path.value())
                    && !item_max_properties_metadata_parse_paths.contains(&constraint.path.value())
            })
            .map(|constraint| {
                let pointer = schema_pointer_from_logical_path(prefix, &constraint.path.value())?;
                let pointer = LitStr::new(pointer.as_str(), constraint.path.span());
                let value = constraint.value as u64;
                Ok(quote! {
                    ::agena_plugin_sdk::macro_support::set_schema_u64_metadata(
                        schema,
                        #pointer,
                        "maxProperties",
                        #value,
                    );
                })
            })
            .collect::<Result<Vec<_>>>()?,
    );
    calls.extend(
        constraints
            .min_chars()
            .iter()
            .filter(|constraint| {
                !min_chars_metadata_parse_paths.contains(&constraint.path.value())
                    && !item_min_chars_metadata_parse_paths.contains(&constraint.path.value())
            })
            .map(|constraint| {
                let pointer = schema_pointer_from_logical_path(prefix, &constraint.path.value())?;
                let pointer = LitStr::new(pointer.as_str(), constraint.path.span());
                let value = constraint.value as u64;
                Ok(quote! {
                    ::agena_plugin_sdk::macro_support::set_schema_minimum_u64_metadata(
                        schema,
                        #pointer,
                        "minLength",
                        #value,
                    );
                })
            })
            .collect::<Result<Vec<_>>>()?,
    );
    calls.extend(
        constraints
            .max_chars()
            .iter()
            .filter(|constraint| {
                !max_chars_metadata_parse_paths.contains(&constraint.path.value())
                    && !item_max_chars_metadata_parse_paths.contains(&constraint.path.value())
            })
            .map(|constraint| {
                let pointer = schema_pointer_from_logical_path(prefix, &constraint.path.value())?;
                let pointer = LitStr::new(pointer.as_str(), constraint.path.span());
                let value = constraint.value as u64;
                Ok(quote! {
                    ::agena_plugin_sdk::macro_support::set_schema_u64_metadata(
                        schema,
                        #pointer,
                        "maxLength",
                        #value,
                    );
                })
            })
            .collect::<Result<Vec<_>>>()?,
    );
    calls.extend(
        constraints
            .formats()
            .iter()
            .filter(|constraint| {
                !format_metadata_parse_paths.contains(&constraint.path.value())
                    && !item_format_metadata_parse_paths.contains(&constraint.path.value())
            })
            .map(|constraint| {
                let pointer = schema_pointer_from_logical_path(prefix, &constraint.path.value())?;
                let pointer = LitStr::new(pointer.as_str(), constraint.path.span());
                let value = &constraint.value;
                Ok(quote! {
                    ::agena_plugin_sdk::macro_support::set_schema_string_metadata(
                        schema,
                        #pointer,
                        "format",
                        #value,
                    );
                })
            })
            .collect::<Result<Vec<_>>>()?,
    );
    calls.extend(
        constraints
            .patterns()
            .iter()
            .filter(|constraint| {
                !pattern_metadata_parse_paths.contains(&constraint.path.value())
                    && !item_pattern_metadata_parse_paths.contains(&constraint.path.value())
            })
            .map(|constraint| {
                let pointer = schema_pointer_from_logical_path(prefix, &constraint.path.value())?;
                let pointer = LitStr::new(pointer.as_str(), constraint.path.span());
                let value = &constraint.value;
                Ok(quote! {
                    ::agena_plugin_sdk::macro_support::set_schema_string_metadata(
                        schema,
                        #pointer,
                        "pattern",
                        #value,
                    );
                })
            })
            .collect::<Result<Vec<_>>>()?,
    );
    Ok(calls)
}

fn schema_pointer_from_logical_path(prefix: &str, path: &str) -> Result<String> {
    let mut pointer = prefix.to_string();
    if path.trim().is_empty() {
        return Ok(pointer);
    }
    for raw_segment in path.split('.') {
        let mut segment = raw_segment;
        let mut array_depth = 0usize;
        while let Some(stripped) = segment.strip_suffix("[]") {
            array_depth += 1;
            segment = stripped;
        }
        if !segment.is_empty() {
            pointer.push_str("/properties/");
            pointer.push_str(&escape_json_pointer_segment(segment));
        }
        for _ in 0..array_depth {
            pointer.push_str("/items");
        }
    }
    Ok(pointer)
}

fn escape_json_pointer_segment(segment: &str) -> String {
    segment.replace('~', "~0").replace('/', "~1")
}

fn append_constraint_path_suffix(path: &LitStr, suffix: &str) -> LitStr {
    LitStr::new(format!("{}{}", path.value(), suffix).as_str(), path.span())
}

fn resolve_known_input_constraint_path(
    path: &LitStr,
    field_path_lookup: &BTreeMap<String, LitStr>,
) -> LitStr {
    if let Some(resolved) = field_path_lookup.get(&path.value()) {
        return resolved.clone();
    }
    let value = path.value();
    let head_end = value.find('.').unwrap_or(value.len());
    let (head, tail) = value.split_at(head_end);
    let mut base = head;
    let mut suffix = String::new();
    while let Some(stripped) = base.strip_suffix("[]") {
        base = stripped;
        suffix.push_str("[]");
    }
    if let Some(resolved_head) = field_path_lookup.get(base) {
        return LitStr::new(
            format!("{}{}{}", resolved_head.value(), suffix, tail).as_str(),
            path.span(),
        );
    }
    path.clone()
}

fn resolve_known_constraint_path(
    path: &LitStr,
    field_path_lookup: Option<&BTreeMap<String, LitStr>>,
) -> LitStr {
    field_path_lookup
        .map(|lookup| resolve_known_input_constraint_path(path, lookup))
        .unwrap_or_else(|| path.clone())
}

fn normalize_array_value_constraint_path(
    path: &LitStr,
    field_path_lookup: &BTreeMap<String, LitStr>,
    array_field_paths: &BTreeSet<String>,
) -> LitStr {
    let resolved = resolve_known_input_constraint_path(path, field_path_lookup);
    let value = resolved.value();
    let head_end = value.find('.').unwrap_or(value.len());
    let (head, tail) = value.split_at(head_end);
    if head.ends_with("[]") || !array_field_paths.contains(head) {
        return resolved;
    }
    LitStr::new(format!("{head}[]{tail}").as_str(), path.span())
}

fn normalize_array_value_lit_paths(
    paths: &mut [LitStr],
    field_path_lookup: &BTreeMap<String, LitStr>,
    array_field_paths: &BTreeSet<String>,
) {
    for path in paths {
        *path = normalize_array_value_constraint_path(path, field_path_lookup, array_field_paths);
    }
}

fn resolve_constraint_lit_paths(
    paths: &mut [LitStr],
    field_path_lookup: &BTreeMap<String, LitStr>,
) {
    for path in paths {
        *path = resolve_known_input_constraint_path(path, field_path_lookup);
    }
}

fn resolve_constraint_string_paths(
    constraints: &mut [PathStringConstraint],
    field_path_lookup: &BTreeMap<String, LitStr>,
) {
    for constraint in constraints {
        constraint.path = resolve_known_input_constraint_path(&constraint.path, field_path_lookup);
    }
}

fn resolve_constraint_usize_paths(
    constraints: &mut [PathUsizeConstraint],
    field_path_lookup: &BTreeMap<String, LitStr>,
) {
    for constraint in constraints {
        constraint.path = resolve_known_input_constraint_path(&constraint.path, field_path_lookup);
    }
}

fn resolve_constraint_expr_paths(
    constraints: &mut [PathValueConstraint],
    field_path_lookup: &BTreeMap<String, LitStr>,
) {
    for constraint in constraints {
        constraint.path = resolve_known_input_constraint_path(&constraint.path, field_path_lookup);
    }
}

fn resolve_constraint_values_paths(
    constraints: &mut [PathValuesConstraint],
    field_path_lookup: &BTreeMap<String, LitStr>,
) {
    for constraint in constraints {
        constraint.path = resolve_known_input_constraint_path(&constraint.path, field_path_lookup);
    }
}

fn resolve_constraint_group_paths(
    groups: &mut [Vec<LitStr>],
    field_path_lookup: &BTreeMap<String, LitStr>,
) {
    for group in groups {
        resolve_constraint_lit_paths(group, field_path_lookup);
    }
}

fn resolve_constraint_pair_paths(
    constraints: &mut [PathPairConstraint],
    field_path_lookup: &BTreeMap<String, LitStr>,
) {
    for constraint in constraints {
        constraint.left = resolve_known_input_constraint_path(&constraint.left, field_path_lookup);
        constraint.right =
            resolve_known_input_constraint_path(&constraint.right, field_path_lookup);
    }
}

fn resolve_constraint_strings_paths(
    constraints: &mut [PathStringsConstraint],
    field_path_lookup: &BTreeMap<String, LitStr>,
) {
    for constraint in constraints {
        constraint.path = resolve_known_input_constraint_path(&constraint.path, field_path_lookup);
    }
}

fn normalize_array_value_string_constraints(
    constraints: &mut [PathStringConstraint],
    field_path_lookup: &BTreeMap<String, LitStr>,
    array_field_paths: &BTreeSet<String>,
) {
    for constraint in constraints {
        constraint.path = normalize_array_value_constraint_path(
            &constraint.path,
            field_path_lookup,
            array_field_paths,
        );
    }
}

fn normalize_array_value_usize_constraints(
    constraints: &mut [PathUsizeConstraint],
    field_path_lookup: &BTreeMap<String, LitStr>,
    array_field_paths: &BTreeSet<String>,
) {
    for constraint in constraints {
        constraint.path = normalize_array_value_constraint_path(
            &constraint.path,
            field_path_lookup,
            array_field_paths,
        );
    }
}

fn normalize_array_value_expr_constraints(
    constraints: &mut [PathValueConstraint],
    field_path_lookup: &BTreeMap<String, LitStr>,
    array_field_paths: &BTreeSet<String>,
) {
    for constraint in constraints {
        constraint.path = normalize_array_value_constraint_path(
            &constraint.path,
            field_path_lookup,
            array_field_paths,
        );
    }
}

fn normalize_array_value_group_constraints(
    groups: &mut [Vec<LitStr>],
    field_path_lookup: &BTreeMap<String, LitStr>,
    array_field_paths: &BTreeSet<String>,
) {
    for group in groups {
        normalize_array_value_lit_paths(group, field_path_lookup, array_field_paths);
    }
}

fn normalize_array_value_pair_constraints(
    constraints: &mut [PathPairConstraint],
    field_path_lookup: &BTreeMap<String, LitStr>,
    array_field_paths: &BTreeSet<String>,
) {
    for constraint in constraints {
        constraint.left = normalize_array_value_constraint_path(
            &constraint.left,
            field_path_lookup,
            array_field_paths,
        );
        constraint.right = normalize_array_value_constraint_path(
            &constraint.right,
            field_path_lookup,
            array_field_paths,
        );
    }
}

fn normalize_array_value_nested_path_constraints(
    non_empty: &mut [LitStr],
    non_empty_if_present: &mut [LitStr],
    exactly_one_of: &mut [Vec<LitStr>],
    at_least_one_of: &mut [Vec<LitStr>],
    requires: &mut [PathPairConstraint],
    conflicts_with: &mut [PathPairConstraint],
    required_unless_present: &mut [PathPairConstraint],
    distinct_trimmed_within: &mut [PathPairConstraint],
    field_path_lookup: &BTreeMap<String, LitStr>,
    array_field_paths: &BTreeSet<String>,
) {
    normalize_array_value_lit_paths(non_empty, field_path_lookup, array_field_paths);
    normalize_array_value_lit_paths(non_empty_if_present, field_path_lookup, array_field_paths);
    normalize_array_value_group_constraints(exactly_one_of, field_path_lookup, array_field_paths);
    normalize_array_value_group_constraints(at_least_one_of, field_path_lookup, array_field_paths);
    normalize_array_value_pair_constraints(requires, field_path_lookup, array_field_paths);
    normalize_array_value_pair_constraints(conflicts_with, field_path_lookup, array_field_paths);
    normalize_array_value_pair_constraints(
        required_unless_present,
        field_path_lookup,
        array_field_paths,
    );
    normalize_array_value_pair_constraints(
        distinct_trimmed_within,
        field_path_lookup,
        array_field_paths,
    );
}

fn normalize_array_value_values_constraints(
    constraints: &mut [PathValuesConstraint],
    field_path_lookup: &BTreeMap<String, LitStr>,
    array_field_paths: &BTreeSet<String>,
) {
    for constraint in constraints {
        constraint.path = normalize_array_value_constraint_path(
            &constraint.path,
            field_path_lookup,
            array_field_paths,
        );
    }
}

fn normalize_array_value_relation_constraints(
    forbid_substrings: &mut [PathStringsConstraint],
    distinct_trimmed: &mut [LitStr],
    field_path_lookup: &BTreeMap<String, LitStr>,
    array_field_paths: &BTreeSet<String>,
) {
    for constraint in forbid_substrings {
        constraint.path = normalize_array_value_constraint_path(
            &constraint.path,
            field_path_lookup,
            array_field_paths,
        );
    }
    normalize_array_value_lit_paths(distinct_trimmed, field_path_lookup, array_field_paths);
}

fn normalize_array_value_metadata(
    metadata: &mut [PluginInputFieldMetadata],
    field_path_lookup: &BTreeMap<String, LitStr>,
    array_field_paths: &BTreeSet<String>,
) {
    for field in metadata {
        let normalized = normalize_array_value_constraint_path(
            &field.parse_path,
            field_path_lookup,
            array_field_paths,
        );
        if normalized.value() == field.parse_path.value() {
            continue;
        }
        if let Some(value) = field.minimum.take()
            && field.item_minimum.is_none()
        {
            field.item_minimum = Some(value);
        }
        if let Some(value) = field.maximum.take()
            && field.item_maximum.is_none()
        {
            field.item_maximum = Some(value);
        }
        if let Some(value) = field.exclusive_minimum.take()
            && field.item_exclusive_minimum.is_none()
        {
            field.item_exclusive_minimum = Some(value);
        }
        if let Some(value) = field.exclusive_maximum.take()
            && field.item_exclusive_maximum.is_none()
        {
            field.item_exclusive_maximum = Some(value);
        }
        if let Some(value) = field.min_properties.take()
            && field.item_min_properties.is_none()
        {
            field.item_min_properties = Some(value);
        }
        if let Some(value) = field.max_properties.take()
            && field.item_max_properties.is_none()
        {
            field.item_max_properties = Some(value);
        }
        if let Some(value) = field.min_chars.take()
            && field.item_min_chars.is_none()
        {
            field.item_min_chars = Some(value);
        }
        if let Some(value) = field.max_chars.take()
            && field.item_max_chars.is_none()
        {
            field.item_max_chars = Some(value);
        }
        if let Some(value) = field.format.take()
            && field.item_format.is_none()
        {
            field.item_format = Some(value);
        }
        if let Some(value) = field.pattern.take()
            && field.item_pattern.is_none()
        {
            field.item_pattern = Some(value);
        }
        if !field.choices.is_empty() && field.item_choices.is_empty() {
            field.item_choices = std::mem::take(&mut field.choices);
        }
    }
}

fn normalize_array_value_constraints(
    trim: &mut [LitStr],
    trim_suffix: &mut [PathStringConstraint],
    minimums: &mut [PathValueConstraint],
    maximums: &mut [PathValueConstraint],
    exclusive_minimums: &mut [PathValueConstraint],
    exclusive_maximums: &mut [PathValueConstraint],
    min_properties: &mut [PathUsizeConstraint],
    max_properties: &mut [PathUsizeConstraint],
    min_chars: &mut [PathUsizeConstraint],
    max_chars: &mut [PathUsizeConstraint],
    formats: &mut [PathStringConstraint],
    patterns: &mut [PathStringConstraint],
    choices: &mut [PathValuesConstraint],
    forbid_substrings: &mut [PathStringsConstraint],
    distinct_trimmed: &mut [LitStr],
    input_field_metadata: &mut [PluginInputFieldMetadata],
    field_path_lookup: &BTreeMap<String, LitStr>,
    array_field_paths: &BTreeSet<String>,
) {
    normalize_array_value_lit_paths(trim, field_path_lookup, array_field_paths);
    normalize_array_value_string_constraints(trim_suffix, field_path_lookup, array_field_paths);
    normalize_array_value_expr_constraints(minimums, field_path_lookup, array_field_paths);
    normalize_array_value_expr_constraints(maximums, field_path_lookup, array_field_paths);
    normalize_array_value_expr_constraints(
        exclusive_minimums,
        field_path_lookup,
        array_field_paths,
    );
    normalize_array_value_expr_constraints(
        exclusive_maximums,
        field_path_lookup,
        array_field_paths,
    );
    normalize_array_value_usize_constraints(min_properties, field_path_lookup, array_field_paths);
    normalize_array_value_usize_constraints(max_properties, field_path_lookup, array_field_paths);
    normalize_array_value_usize_constraints(min_chars, field_path_lookup, array_field_paths);
    normalize_array_value_usize_constraints(max_chars, field_path_lookup, array_field_paths);
    normalize_array_value_string_constraints(formats, field_path_lookup, array_field_paths);
    normalize_array_value_string_constraints(patterns, field_path_lookup, array_field_paths);
    normalize_array_value_values_constraints(choices, field_path_lookup, array_field_paths);
    normalize_array_value_relation_constraints(
        forbid_substrings,
        distinct_trimmed,
        field_path_lookup,
        array_field_paths,
    );
    normalize_array_value_metadata(input_field_metadata, field_path_lookup, array_field_paths);
}

fn prefixed_constraint_group(
    current: &LitStr,
    peers: &[LitStr],
    field_path_lookup: Option<&BTreeMap<String, LitStr>>,
) -> Vec<LitStr> {
    let mut seen = BTreeSet::new();
    let mut group = Vec::new();
    if seen.insert(current.value()) {
        group.push(current.clone());
    }
    for peer in peers {
        let resolved = resolve_known_constraint_path(peer, field_path_lookup);
        if seen.insert(resolved.value()) {
            group.push(resolved);
        }
    }
    group
}

fn tool_input_struct_field_schema_metadata_calls(
    prefix: &str,
    fields: &Fields,
    rename_rule: Option<SerdeRenameRule>,
) -> Result<Vec<proc_macro2::TokenStream>> {
    let Fields::Named(named) = fields else {
        return Ok(Vec::new());
    };
    let mut calls = Vec::new();
    for (index, field) in named.named.iter().enumerate() {
        let order = LitStr::new(format!("{index:06}").as_str(), field.span());
        if let Some(flatten_shape_ty) = flatten_shape_type(field)? {
            let pointer = LitStr::new(prefix, field.span());
            calls.push(quote! {
                let mut overlay = <#flatten_shape_ty as ::agena_plugin_sdk::ToolInput>::input_schema();
                ::agena_plugin_sdk::macro_support::prefix_schema_order_metadata(
                    &mut overlay,
                    #order,
                );
                ::agena_plugin_sdk::macro_support::merge_flattened_schema_at_pointer(
                    schema,
                    #pointer,
                    &overlay,
                );
            });
            continue;
        }
        let arg_config = parse_input_field_arg_attrs(field)?;
        let Some(names) = prepare_input_field_names(field, rename_rule, &arg_config)? else {
            continue;
        };
        let nested_shape = nested_input_shape_spec(field)?;
        if names.schema_path.value() != names.parse_path.value() {
            let pointer = LitStr::new(prefix, field.span());
            let from = &names.parse_path;
            let to = &names.schema_path;
            calls.push(quote! {
                ::agena_plugin_sdk::macro_support::rename_schema_property(
                    schema,
                    #pointer,
                    #from,
                    #to,
                );
            });
        }
        if let Some(nested_shape) = nested_shape {
            let pointer = if prefix.is_empty() {
                if nested_shape.array {
                    format!("/properties/{}/items", names.schema_path.value())
                } else {
                    format!("/properties/{}", names.schema_path.value())
                }
            } else if nested_shape.array {
                format!("{prefix}/properties/{}/items", names.schema_path.value())
            } else {
                format!("{prefix}/properties/{}", names.schema_path.value())
            };
            let pointer = LitStr::new(pointer.as_str(), field.span());
            let inner_ty = &nested_shape.inner_ty;
            calls.push(quote! {
                let mut overlay = <#inner_ty as ::agena_plugin_sdk::ToolInput>::input_schema();
                ::agena_plugin_sdk::macro_support::prefix_schema_order_metadata(
                    &mut overlay,
                    #order,
                );
                ::agena_plugin_sdk::macro_support::merge_schema_overlay_at_pointer(
                    schema,
                    #pointer,
                    &overlay,
                );
            });
        }
        let description = doc_text(&field.attrs).and_then(|text| {
            let trimmed = text.trim().to_string();
            (!trimmed.is_empty()).then_some(trimmed)
        });
        let pointer = if prefix.is_empty() {
            format!("/properties/{}", names.schema_path.value())
        } else {
            format!("{prefix}/properties/{}", names.schema_path.value())
        };
        let pointer = LitStr::new(pointer.as_str(), field.span());
        calls.push(quote! {
            ::agena_plugin_sdk::macro_support::set_schema_string_metadata(
                schema,
                #pointer,
                "x-agena-order",
                #order,
            );
        });
        if let Some(description) = description {
            let description = LitStr::new(&description, field.span());
            calls.push(quote! {
                ::agena_plugin_sdk::macro_support::set_schema_metadata(
                    schema,
                    #pointer,
                    None,
                    Some(#description),
                );
            });
        }
        if !names.schema_aliases.is_empty() {
            let alias_values = names.schema_aliases.iter().collect::<Vec<_>>();
            calls.push(quote! {
                ::agena_plugin_sdk::macro_support::set_schema_string_list_metadata(
                    schema,
                    #pointer,
                    "x-agena-aliases",
                    &[#(#alias_values),*],
                );
            });
        }
    }
    Ok(calls)
}

#[derive(Clone, Copy)]
enum SerdeRenameRule {
    LowerCase,
    UpperCase,
    PascalCase,
    CamelCase,
    SnakeCase,
    ScreamingSnakeCase,
    KebabCase,
    ScreamingKebabCase,
}

impl SerdeRenameRule {
    fn parse(value: &LitStr) -> Result<Self> {
        match value.value().as_str() {
            "lowercase" => Ok(Self::LowerCase),
            "UPPERCASE" => Ok(Self::UpperCase),
            "PascalCase" => Ok(Self::PascalCase),
            "camelCase" => Ok(Self::CamelCase),
            "snake_case" => Ok(Self::SnakeCase),
            "SCREAMING_SNAKE_CASE" => Ok(Self::ScreamingSnakeCase),
            "kebab-case" => Ok(Self::KebabCase),
            "SCREAMING-KEBAB-CASE" => Ok(Self::ScreamingKebabCase),
            other => Err(syn::Error::new_spanned(
                value,
                format!("unsupported serde rename_all rule '{other}'"),
            )),
        }
    }

    fn apply(self, ident: &Ident) -> String {
        let snake = ident_to_snake_case(ident);
        match self {
            Self::LowerCase => snake.replace('_', ""),
            Self::UpperCase => snake.replace('_', "").to_ascii_uppercase(),
            Self::PascalCase => pascal_case_from_snake(&snake),
            Self::CamelCase => {
                let mut value = pascal_case_from_snake(&snake);
                if let Some(first) = value.get_mut(0..1) {
                    first.make_ascii_lowercase();
                }
                value
            }
            Self::SnakeCase => snake,
            Self::ScreamingSnakeCase => snake.to_ascii_uppercase(),
            Self::KebabCase => snake.replace('_', "-"),
            Self::ScreamingKebabCase => snake.replace('_', "-").to_ascii_uppercase(),
        }
    }
}

fn pascal_case_from_snake(snake: &str) -> String {
    let mut output = String::new();
    for segment in snake.split('_').filter(|segment| !segment.is_empty()) {
        let mut chars = segment.chars();
        if let Some(first) = chars.next() {
            output.extend(first.to_uppercase());
            output.extend(chars);
        }
    }
    output
}

fn serde_rename_all_rule(attrs: &[Attribute]) -> Result<Option<SerdeRenameRule>> {
    serde_rename_all_rule_for_key(attrs, "rename_all")
}

fn serde_rename_all_fields_rule(attrs: &[Attribute]) -> Result<Option<SerdeRenameRule>> {
    serde_rename_all_rule_for_key(attrs, "rename_all_fields")
}

fn serde_rename_all_rule_for_key(
    attrs: &[Attribute],
    key: &str,
) -> Result<Option<SerdeRenameRule>> {
    let mut rule = None;
    for attr in attrs {
        if !attr.path().is_ident("serde") {
            continue;
        }
        let metas = attr.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)?;
        for meta in metas {
            match meta {
                Meta::NameValue(value) if value.path.is_ident(key) => {
                    rule = Some(SerdeRenameRule::parse(&expr_lit_str(&value.value, key)?)?);
                }
                Meta::List(list) if list.path.is_ident(key) => {
                    if let Some(value) =
                        serde_rename_rule_list_value(list.tokens.clone(), key, "deserialize")?
                            .or(serde_rename_rule_list_value(list.tokens, key, "serialize")?)
                    {
                        rule = Some(SerdeRenameRule::parse(&value)?);
                    }
                }
                _ => {}
            }
        }
    }
    Ok(rule)
}

fn serde_rename_rule_list_value(
    tokens: proc_macro2::TokenStream,
    list_name: &str,
    key: &str,
) -> Result<Option<LitStr>> {
    let metas = Punctuated::<Meta, Token![,]>::parse_terminated.parse2(tokens)?;
    for meta in metas {
        if let Meta::NameValue(value) = meta
            && value.path.is_ident(key)
        {
            return Ok(Some(expr_lit_str(&value.value, list_name)?));
        }
    }
    Ok(None)
}

fn field_schema_property_name_with_rule(
    field: &Field,
    rename_rule: Option<SerdeRenameRule>,
) -> Result<Option<String>> {
    let Some(ident) = field.ident.as_ref() else {
        return Ok(None);
    };
    let mut serde_rename = None;
    let mut schema_rename = None;
    for attr in &field.attrs {
        if !attr.path().is_ident("serde") && !attr.path().is_ident("schemars") {
            continue;
        }
        let is_serde = attr.path().is_ident("serde");
        let metas = attr.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)?;
        for meta in metas {
            match meta {
                Meta::Path(path)
                    if is_serde
                        && (path.is_ident("skip") || path.is_ident("skip_deserializing")) =>
                {
                    return Ok(None);
                }
                Meta::Path(path) if !is_serde && path.is_ident("skip") => return Ok(None),
                Meta::Path(path) if path.is_ident("flatten") => return Ok(None),
                Meta::NameValue(value) => {
                    if value.path.is_ident("rename") {
                        let rename = expr_lit_str(&value.value, "rename")?.value();
                        if is_serde {
                            serde_rename = Some(rename);
                        } else {
                            schema_rename = Some(rename);
                        }
                    }
                }
                Meta::List(list) if list.path.is_ident("rename") => {
                    if let Some(rename) =
                        serde_rename_rule_list_value(list.tokens.clone(), "rename", "deserialize")?
                            .or(serde_rename_rule_list_value(
                                list.tokens,
                                "rename",
                                "serialize",
                            )?)
                    {
                        if is_serde {
                            serde_rename = Some(rename.value());
                        } else {
                            schema_rename = Some(rename.value());
                        }
                    }
                }
                _ => {}
            }
        }
    }
    Ok(Some(serde_rename.or(schema_rename).unwrap_or_else(|| {
        rename_rule.map_or_else(|| ident.to_string(), |rule| rule.apply(ident))
    })))
}

fn field_has_serde_default(field: &Field) -> Result<bool> {
    for attr in &field.attrs {
        if !attr.path().is_ident("serde") {
            continue;
        }
        let metas = attr.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)?;
        for meta in metas {
            match meta {
                Meta::Path(path) if path.is_ident("default") => return Ok(true),
                Meta::NameValue(value) if value.path.is_ident("default") => return Ok(true),
                _ => {}
            }
        }
    }
    Ok(false)
}

fn field_schema_aliases(field: &Field) -> Result<Vec<String>> {
    let mut aliases = Vec::new();
    for attr in &field.attrs {
        if !attr.path().is_ident("serde") {
            continue;
        }
        let metas = attr.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)?;
        for meta in metas {
            if let Meta::NameValue(value) = meta
                && value.path.is_ident("alias")
            {
                aliases.push(expr_lit_str(&value.value, "alias")?.value());
            }
        }
    }
    Ok(aliases)
}

fn named_field_object_insert_tokens<'a, I>(
    fields: I,
    flatten_error: &str,
    rename_rule: Option<SerdeRenameRule>,
) -> Result<Vec<proc_macro2::TokenStream>>
where
    I: IntoIterator<Item = (&'a Field, &'a syn::Ident)>,
{
    fields
        .into_iter()
        .map(|(field, binding)| {
            if field_is_flatten(field)? {
                Ok(quote! {
                    match serde_json::to_value(#binding)
                        .map_err(|err| ::agena_plugin_sdk::PluginError::invalid_params(err.to_string()))? {
                        serde_json::Value::Object(flattened) => {
                            object.extend(flattened);
                        }
                        _ => {
                            return Err(::agena_plugin_sdk::PluginError::invalid_params(
                                #flatten_error,
                            ));
                        }
                    }
                })
            } else {
                let Some(name) = field_schema_property_name_with_rule(field, rename_rule)? else {
                    return Err(syn::Error::new_spanned(
                        field,
                        "named field is missing serializable property name",
                    ));
                };
                let name = LitStr::new(&name, field.span());
                Ok(quote! {
                    object.insert(
                        #name.to_string(),
                        serde_json::to_value(#binding).map_err(|err| ::agena_plugin_sdk::PluginError::invalid_params(err.to_string()))?,
                    );
                })
            }
        })
        .collect()
}

fn flatten_shape_type(field: &Field) -> Result<Option<syn::Type>> {
    if !field_is_flatten(field)? {
        return Ok(None);
    }
    for attr in &field.attrs {
        if !attr.path().is_ident("input") {
            continue;
        }
        let metas = attr.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)?;
        for meta in metas {
            if let Meta::Path(path) = meta
                && path.is_ident("flatten_shape")
            {
                return Ok(Some(field.ty.clone()));
            }
        }
    }
    Ok(None)
}

fn nested_input_shape_post_parse_expr(
    target: proc_macro2::TokenStream,
    spec: &NestedInputShapeSpec,
    schema_path: Option<&LitStr>,
) -> proc_macro2::TokenStream {
    let ty = &spec.inner_ty;
    match (spec.optional, spec.array) {
        (false, false) => {
            if let Some(schema_path) = schema_path {
                quote! {{
                    let __inner_schema = <#ty as ::agena_plugin_sdk::ToolInput>::input_schema();
                    let __macro_parse_result = <#ty as ::agena_plugin_sdk::ToolInput>::parse_input(
                        serde_json::to_value(&#target)
                            .map_err(|err| ::agena_plugin_sdk::PluginError::invalid_params(err.to_string()))?,
                    );
                    let __mappings =
                        ::agena_plugin_sdk::macro_support::prefixed_input_error_path_mappings(
                            &__inner_schema,
                            #schema_path,
                        );
                    ::agena_plugin_sdk::macro_support::remap_invalid_params_paths_owned(
                        __macro_parse_result,
                        __mappings.as_slice(),
                    )?
                }}
            } else {
                quote! {
                    <#ty as ::agena_plugin_sdk::ToolInput>::parse_input(
                        serde_json::to_value(&#target)
                            .map_err(|err| ::agena_plugin_sdk::PluginError::invalid_params(err.to_string()))?,
                    )?
                }
            }
        }
        (true, false) => {
            if let Some(schema_path) = schema_path {
                quote! {
                    match &#target {
                        Some(value) => {
                            let __inner_schema = <#ty as ::agena_plugin_sdk::ToolInput>::input_schema();
                            let __macro_parse_result =
                                <#ty as ::agena_plugin_sdk::ToolInput>::parse_input(
                                    serde_json::to_value(value).map_err(|err| {
                                        ::agena_plugin_sdk::PluginError::invalid_params(err.to_string())
                                    })?,
                                );
                            let __mappings =
                                ::agena_plugin_sdk::macro_support::prefixed_input_error_path_mappings(
                                    &__inner_schema,
                                    #schema_path,
                                );
                            Some(
                                ::agena_plugin_sdk::macro_support::remap_invalid_params_paths_owned(
                                    __macro_parse_result,
                                    __mappings.as_slice(),
                                )?,
                            )
                        }
                        None => None,
                    }
                }
            } else {
                quote! {
                    match &#target {
                        Some(value) => Some(
                            <#ty as ::agena_plugin_sdk::ToolInput>::parse_input(
                                serde_json::to_value(value)
                                    .map_err(|err| ::agena_plugin_sdk::PluginError::invalid_params(err.to_string()))?,
                            )?,
                        ),
                        None => None,
                    }
                }
            }
        }
        (false, true) => {
            if let Some(schema_path) = schema_path {
                quote! {{
                    let __inner_schema = <#ty as ::agena_plugin_sdk::ToolInput>::input_schema();
                    #target
                        .iter()
                        .enumerate()
                        .map(|(__index, value)| {
                            let __macro_parse_result =
                                <#ty as ::agena_plugin_sdk::ToolInput>::parse_input(
                                    serde_json::to_value(value).map_err(|err| {
                                        ::agena_plugin_sdk::PluginError::invalid_params(err.to_string())
                                    })?,
                                );
                            let __prefix = format!("{}[{__index}]", #schema_path);
                            let __mappings =
                                ::agena_plugin_sdk::macro_support::prefixed_input_error_path_mappings(
                                    &__inner_schema,
                                    __prefix.as_str(),
                                );
                            ::agena_plugin_sdk::macro_support::remap_invalid_params_paths_owned(
                                __macro_parse_result,
                                __mappings.as_slice(),
                            )
                        })
                        .collect::<::agena_plugin_sdk::Result<Vec<_>>>()?
                }}
            } else {
                quote! {
                    #target
                        .iter()
                        .map(|value| {
                            <#ty as ::agena_plugin_sdk::ToolInput>::parse_input(
                                serde_json::to_value(value)
                                    .map_err(|err| ::agena_plugin_sdk::PluginError::invalid_params(err.to_string()))?,
                            )
                        })
                        .collect::<::agena_plugin_sdk::Result<Vec<_>>>()?
                }
            }
        }
        (true, true) => {
            if let Some(schema_path) = schema_path {
                quote! {
                    match &#target {
                        Some(values) => {
                            let __inner_schema = <#ty as ::agena_plugin_sdk::ToolInput>::input_schema();
                            Some(
                                values
                                    .iter()
                                    .enumerate()
                                    .map(|(__index, value)| {
                                        let __macro_parse_result =
                                            <#ty as ::agena_plugin_sdk::ToolInput>::parse_input(
                                                serde_json::to_value(value).map_err(|err| {
                                                    ::agena_plugin_sdk::PluginError::invalid_params(err.to_string())
                                                })?,
                                            );
                                        let __prefix = format!("{}[{__index}]", #schema_path);
                                        let __mappings =
                                            ::agena_plugin_sdk::macro_support::prefixed_input_error_path_mappings(
                                                &__inner_schema,
                                                __prefix.as_str(),
                                            );
                                        ::agena_plugin_sdk::macro_support::remap_invalid_params_paths_owned(
                                            __macro_parse_result,
                                            __mappings.as_slice(),
                                        )
                                    })
                                    .collect::<::agena_plugin_sdk::Result<Vec<_>>>()?,
                            )
                        }
                        None => None,
                    }
                }
            } else {
                quote! {
                    match &#target {
                        Some(values) => Some(
                            values
                                .iter()
                                .map(|value| {
                                    <#ty as ::agena_plugin_sdk::ToolInput>::parse_input(
                                        serde_json::to_value(value)
                                            .map_err(|err| ::agena_plugin_sdk::PluginError::invalid_params(err.to_string()))?,
                                    )
                                })
                                .collect::<::agena_plugin_sdk::Result<Vec<_>>>()?,
                        ),
                        None => None,
                    }
                }
            }
        }
    }
}

fn expand_flatten_shape_post_parse_tokens(
    attrs: &[Attribute],
    data: &Data,
) -> Result<proc_macro2::TokenStream> {
    match data {
        Data::Struct(data_struct) => {
            let rename_rule = serde_rename_all_rule(attrs)?;
            let updates = data_struct
                .fields
                .iter()
                .enumerate()
                .map(|(index, field)| {
                    let flatten_shape_ty = flatten_shape_type(field)?;
                    let nested_shape = nested_input_shape_spec(field)?;
                    let nested_shape_field = nested_input_shape_field(field, rename_rule)?;
                    let member = field
                        .ident
                        .clone()
                        .map(Member::Named)
                        .unwrap_or_else(|| Member::Unnamed(Index::from(index)));
                    Ok(
                        flatten_shape_ty
                            .map(|ty| {
                                quote! {
                                    parsed.#member = <#ty as ::agena_plugin_sdk::ToolInput>::parse_input(
                                        serde_json::to_value(&parsed.#member)
                                            .map_err(|err| ::agena_plugin_sdk::PluginError::invalid_params(err.to_string()))?,
                                    )?;
                                }
                            })
                            .or_else(|| {
                                nested_shape.as_ref().map(|spec| {
                                    let expr = nested_input_shape_post_parse_expr(
                                        quote! { parsed.#member },
                                        spec,
                                        nested_shape_field.as_ref().map(|field| &field.schema_path),
                                    );
                                    quote! {
                                        parsed.#member = #expr;
                                    }
                                })
                            }),
                    )
                })
                .collect::<Result<Vec<_>>>()?
                .into_iter()
                .flatten()
                .collect::<Vec<_>>();
            if updates.is_empty() {
                Ok(quote! { parsed })
            } else {
                Ok(quote! {{
                    let mut parsed = parsed;
                    #(#updates)*
                    parsed
                }})
            }
        }
        Data::Enum(data_enum) => {
            let enum_field_rule = serde_rename_all_fields_rule(attrs)?;
            let arms = data_enum
                .variants
                .iter()
                .map(|variant| {
                    expand_flatten_shape_variant_post_parse_arm(variant, enum_field_rule)
                })
                .collect::<Result<Vec<_>>>()?
                .into_iter()
                .flatten()
                .collect::<Vec<_>>();
            if arms.is_empty() {
                Ok(quote! { parsed })
            } else {
                Ok(quote! {
                    match parsed {
                        #(#arms,)*
                        other => other,
                    }
                })
            }
        }
        Data::Union(_) => Ok(quote! { parsed }),
    }
}

fn expand_generated_input_post_parse_tokens(
    model: &PluginGeneratedToolInput,
) -> proc_macro2::TokenStream {
    let updates = model
        .input_fields
        .iter()
        .filter_map(|field| {
            let ident = &field.ident;
            if field.flatten_shape {
                let ty = &field.ty;
                return Some(quote! {
                    parsed.#ident = <#ty as ::agena_plugin_sdk::ToolInput>::parse_input(
                        serde_json::to_value(&parsed.#ident)
                            .map_err(|err| ::agena_plugin_sdk::PluginError::invalid_params(err.to_string()))?,
                    )?;
                });
            }
            let spec = field
                .nested_shape
                .then(|| nested_input_shape_spec_from_type(&field.ty))
                .flatten()?;
            let expr = nested_input_shape_post_parse_expr(
                quote! { parsed.#ident },
                &spec,
                Some(&field.wire_name),
            );
            Some(quote! {
                parsed.#ident = #expr;
            })
        })
        .collect::<Vec<_>>();
    if updates.is_empty() {
        quote! { parsed }
    } else {
        quote! {{
            let mut parsed = parsed;
            #(#updates)*
            parsed
        }}
    }
}

fn expand_flatten_shape_variant_post_parse_arm(
    variant: &Variant,
    enum_field_rule: Option<SerdeRenameRule>,
) -> Result<Option<proc_macro2::TokenStream>> {
    let variant_field_rule = serde_rename_all_rule(&variant.attrs)?.or(enum_field_rule);
    match &variant.fields {
        Fields::Named(fields_named) => {
            let bindings = fields_named
                .named
                .iter()
                .map(|field| {
                    let ident = field
                        .ident
                        .clone()
                        .expect("named fields should have identifiers");
                    let flatten_shape_ty = flatten_shape_type(field)?;
                    let nested_shape = nested_input_shape_spec(field)?;
                    let nested_shape_field = nested_input_shape_field(field, variant_field_rule)?;
                    Ok((ident, flatten_shape_ty, nested_shape, nested_shape_field))
                })
                .collect::<Result<Vec<_>>>()?;
            if !bindings
                .iter()
                .any(|(_, flatten_shape_ty, nested_shape, _)| {
                    flatten_shape_ty.is_some() || nested_shape.is_some()
                })
            {
                return Ok(None);
            }
            let pattern_fields = bindings.iter().map(|(ident, _, _, _)| quote! { #ident });
            let normalize_bindings = bindings
                .iter()
                .filter_map(|(ident, flatten_shape_ty, nested_shape, nested_shape_field)| {
                    flatten_shape_ty.as_ref().map(|ty| {
                        quote! {
                            let #ident = <#ty as ::agena_plugin_sdk::ToolInput>::parse_input(
                                serde_json::to_value(&#ident)
                                    .map_err(|err| ::agena_plugin_sdk::PluginError::invalid_params(err.to_string()))?,
                            )?;
                        }
                    }).or_else(|| {
                        nested_shape.as_ref().map(|spec| {
                            let expr =
                                nested_input_shape_post_parse_expr(
                                    quote! { #ident },
                                    spec,
                                    nested_shape_field.as_ref().map(|field| &field.schema_path),
                                );
                            quote! {
                                let #ident = #expr;
                            }
                        })
                    })
                })
                .collect::<Vec<_>>();
            let rebuild_fields = bindings.iter().map(|(ident, _, _, _)| quote! { #ident });
            let variant_ident = &variant.ident;
            Ok(Some(quote! {
                Self::#variant_ident { #(#pattern_fields),* } => {
                    #(#normalize_bindings)*
                    Self::#variant_ident { #(#rebuild_fields),* }
                }
            }))
        }
        Fields::Unnamed(fields_unnamed) => {
            let bindings = fields_unnamed
                .unnamed
                .iter()
                .enumerate()
                .map(|(index, field)| {
                    let binding = format_ident!("__flatten_field_{index}");
                    let flatten_shape_ty = flatten_shape_type(field)?;
                    let nested_shape = nested_input_shape_spec(field)?;
                    let nested_shape_field = nested_input_shape_field(field, variant_field_rule)?;
                    Ok((binding, flatten_shape_ty, nested_shape, nested_shape_field))
                })
                .collect::<Result<Vec<_>>>()?;
            if !bindings
                .iter()
                .any(|(_, flatten_shape_ty, nested_shape, _)| {
                    flatten_shape_ty.is_some() || nested_shape.is_some()
                })
            {
                return Ok(None);
            }
            let pattern_fields = bindings
                .iter()
                .map(|(binding, _, _, _)| quote! { #binding });
            let normalize_bindings = bindings
                .iter()
                .filter_map(|(binding, flatten_shape_ty, nested_shape, nested_shape_field)| {
                    flatten_shape_ty.as_ref().map(|ty| {
                        quote! {
                            let #binding = <#ty as ::agena_plugin_sdk::ToolInput>::parse_input(
                                serde_json::to_value(&#binding)
                                    .map_err(|err| ::agena_plugin_sdk::PluginError::invalid_params(err.to_string()))?,
                            )?;
                        }
                    }).or_else(|| {
                        nested_shape.as_ref().map(|spec| {
                            let expr =
                                nested_input_shape_post_parse_expr(
                                    quote! { #binding },
                                    spec,
                                    nested_shape_field.as_ref().map(|field| &field.schema_path),
                                );
                            quote! {
                                let #binding = #expr;
                            }
                        })
                    })
                })
                .collect::<Vec<_>>();
            let rebuild_fields = bindings
                .iter()
                .map(|(binding, _, _, _)| quote! { #binding });
            let variant_ident = &variant.ident;
            Ok(Some(quote! {
                Self::#variant_ident(#(#pattern_fields),*) => {
                    #(#normalize_bindings)*
                    Self::#variant_ident(#(#rebuild_fields),*)
                }
            }))
        }
        Fields::Unit => Ok(None),
    }
}

fn field_is_flatten(field: &Field) -> Result<bool> {
    for attr in &field.attrs {
        if !attr.path().is_ident("serde") && !attr.path().is_ident("schemars") {
            continue;
        }
        let metas = attr.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)?;
        for meta in metas {
            if let Meta::Path(path) = meta
                && path.is_ident("flatten")
            {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

struct ToolInputVariantConfig {
    action: Option<LitStr>,
    validate: Option<Path>,
    handle: Option<Path>,
    handle_with_context: Option<Path>,
    stream_handle: Option<Path>,
    stream_handle_with_context: Option<Path>,
    permission_paths_handle: Option<Path>,
    permission_networks_handle: Option<Path>,
    handle_by_value: bool,
    trim: Vec<LitStr>,
    trim_suffix: Vec<PathStringConstraint>,
    non_empty: Vec<LitStr>,
    non_empty_if_present: Vec<LitStr>,
    minimums: Vec<PathValueConstraint>,
    maximums: Vec<PathValueConstraint>,
    exclusive_minimums: Vec<PathValueConstraint>,
    exclusive_maximums: Vec<PathValueConstraint>,
    exactly_one_of: Vec<Vec<LitStr>>,
    at_least_one_of: Vec<Vec<LitStr>>,
    requires: Vec<PathPairConstraint>,
    conflicts_with: Vec<PathPairConstraint>,
    required_unless_present: Vec<PathPairConstraint>,
    forbid_substrings: Vec<PathStringsConstraint>,
    distinct_trimmed: Vec<LitStr>,
    distinct_trimmed_within: Vec<PathPairConstraint>,
    min_items: Vec<PathUsizeConstraint>,
    max_items: Vec<PathUsizeConstraint>,
    min_properties: Vec<PathUsizeConstraint>,
    max_properties: Vec<PathUsizeConstraint>,
    min_chars: Vec<PathUsizeConstraint>,
    max_chars: Vec<PathUsizeConstraint>,
    formats: Vec<PathStringConstraint>,
    patterns: Vec<PathStringConstraint>,
    choices: Vec<PathValuesConstraint>,
    input_paths: Vec<PluginInputPathSpec>,
    input_networks: Vec<PluginInputNetworkSpec>,
    input_aliases: Vec<PluginInputFieldAliasSpec>,
    input_defaults: Vec<PluginInputFieldDefaultSpec>,
    input_field_metadata: Vec<PluginInputFieldMetadata>,
    default_when_empty: bool,
    infer_when_present: Vec<LitStr>,
    drop_keys: Vec<LitStr>,
}

struct ToolInputConfig {
    example: Option<Expr>,
    default: bool,
    default_expr: Option<Expr>,
    normalize: Option<Path>,
    validate: Option<Path>,
    handler_receiver: Option<Path>,
    handle: Option<Path>,
    handle_with_context: Option<Path>,
    stream_handle: Option<Path>,
    stream_handle_with_context: Option<Path>,
    permission_paths_handle: Option<Path>,
    permission_networks_handle: Option<Path>,
    handle_field: Option<Path>,
    handle_by_value: bool,
    trim: Vec<LitStr>,
    trim_suffix: Vec<PathStringConstraint>,
    non_empty: Vec<LitStr>,
    non_empty_if_present: Vec<LitStr>,
    minimums: Vec<PathValueConstraint>,
    maximums: Vec<PathValueConstraint>,
    exclusive_minimums: Vec<PathValueConstraint>,
    exclusive_maximums: Vec<PathValueConstraint>,
    exactly_one_of: Vec<Vec<LitStr>>,
    at_least_one_of: Vec<Vec<LitStr>>,
    requires: Vec<PathPairConstraint>,
    conflicts_with: Vec<PathPairConstraint>,
    required_unless_present: Vec<PathPairConstraint>,
    forbid_substrings: Vec<PathStringsConstraint>,
    distinct_trimmed: Vec<LitStr>,
    distinct_trimmed_within: Vec<PathPairConstraint>,
    min_items: Vec<PathUsizeConstraint>,
    max_items: Vec<PathUsizeConstraint>,
    min_properties: Vec<PathUsizeConstraint>,
    max_properties: Vec<PathUsizeConstraint>,
    min_chars: Vec<PathUsizeConstraint>,
    max_chars: Vec<PathUsizeConstraint>,
    formats: Vec<PathStringConstraint>,
    patterns: Vec<PathStringConstraint>,
    choices: Vec<PathValuesConstraint>,
    input_paths: Vec<PluginInputPathSpec>,
    input_networks: Vec<PluginInputNetworkSpec>,
    input_aliases: Vec<PluginInputFieldAliasSpec>,
    input_defaults: Vec<PluginInputFieldDefaultSpec>,
    input_field_metadata: Vec<PluginInputFieldMetadata>,
}

fn parse_input_config(attrs: &[Attribute]) -> Result<ToolInputConfig> {
    let mut example = None;
    let mut default = false;
    let mut default_expr = None;
    let mut normalize = None;
    let mut validate = None;
    let mut handler_receiver = None;
    let mut handle = None;
    let mut handle_with_context = None;
    let mut stream_handle = None;
    let mut stream_handle_with_context = None;
    let mut permission_paths_handle = None;
    let mut permission_networks_handle = None;
    let mut handle_field = None;
    let mut handle_by_value = false;
    let mut trim = Vec::new();
    let mut trim_suffix = Vec::new();
    let mut non_empty = Vec::new();
    let mut non_empty_if_present = Vec::new();
    let mut minimums = Vec::new();
    let mut maximums = Vec::new();
    let mut exclusive_minimums = Vec::new();
    let mut exclusive_maximums = Vec::new();
    let mut exactly_one_of = Vec::new();
    let mut at_least_one_of = Vec::new();
    let mut requires = Vec::new();
    let mut conflicts_with = Vec::new();
    let mut required_unless_present = Vec::new();
    let mut forbid_substrings = Vec::new();
    let mut distinct_trimmed = Vec::new();
    let mut distinct_trimmed_within = Vec::new();
    let mut min_items = Vec::new();
    let mut max_items = Vec::new();
    let mut min_properties = Vec::new();
    let mut max_properties = Vec::new();
    let mut min_chars = Vec::new();
    let mut max_chars = Vec::new();
    let mut formats = Vec::new();
    let mut patterns = Vec::new();
    let mut choices = Vec::new();
    let input_paths = Vec::new();
    let input_networks = Vec::new();
    let input_aliases = Vec::new();
    let input_defaults = Vec::new();
    let input_field_metadata = Vec::new();
    for attr in attrs {
        if !attr.path().is_ident("input") {
            continue;
        }
        let metas = attr.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)?;
        for meta in metas {
            match meta {
                Meta::NameValue(value) => {
                    let Some(ident) = value.path.get_ident() else {
                        return Err(syn::Error::new_spanned(value.path, "expected identifier"));
                    };
                    match ident.to_string().as_str() {
                        "example" => {
                            if example.replace(value.value).is_some() {
                                return Err(syn::Error::new_spanned(
                                    ident,
                                    "duplicate input example",
                                ));
                            }
                        }
                        "default" => {
                            if default || default_expr.replace(value.value).is_some() {
                                return Err(syn::Error::new_spanned(
                                    ident,
                                    "duplicate input default",
                                ));
                            }
                        }
                        "normalize" => normalize = Some(expr_path(&value.value, "normalize")?),
                        "validate" => validate = Some(expr_path(&value.value, "validate")?),
                        "handler_receiver" => {
                            handler_receiver = Some(expr_path(&value.value, "handler_receiver")?)
                        }
                        "handle" => handle = Some(expr_path(&value.value, "handle")?),
                        "handle_with_context" => {
                            handle_with_context =
                                Some(expr_path(&value.value, "handle_with_context")?)
                        }
                        "stream_handle" => {
                            stream_handle = Some(expr_path(&value.value, "stream_handle")?)
                        }
                        "stream_handle_with_context" => {
                            stream_handle_with_context =
                                Some(expr_path(&value.value, "stream_handle_with_context")?)
                        }
                        "permission_paths_handle" => {
                            permission_paths_handle =
                                Some(expr_path(&value.value, "permission_paths_handle")?)
                        }
                        "permission_networks_handle" => {
                            permission_networks_handle =
                                Some(expr_path(&value.value, "permission_networks_handle")?)
                        }
                        "handle_field" => {
                            handle_field = Some(expr_path(&value.value, "handle_field")?)
                        }
                        "handle_by_value" => {
                            handle_by_value = expr_lit_bool(&value.value, "handle_by_value")?
                        }
                        other => {
                            return Err(syn::Error::new_spanned(
                                ident,
                                format!("unsupported input attribute '{other}'"),
                            ));
                        }
                    }
                }
                Meta::List(list) => {
                    let Some(ident) = list.path.get_ident() else {
                        return Err(syn::Error::new_spanned(list.path, "expected identifier"));
                    };
                    match ident.to_string().as_str() {
                        "trim" => trim.extend(parse_lit_str_list(list.tokens)?),
                        "item_trim" => trim.extend(parse_item_lit_str_list(list.tokens)?),
                        "trim_suffix" => trim_suffix
                            .push(parse_path_lit_str_constraint(list.tokens, "trim_suffix")?),
                        "item_trim_suffix" => trim_suffix.push(parse_item_path_lit_str_constraint(
                            list.tokens,
                            "item_trim_suffix",
                        )?),
                        "non_empty" => non_empty.extend(parse_lit_str_list(list.tokens)?),
                        "item_non_empty" => non_empty.extend(parse_item_lit_str_list(list.tokens)?),
                        "non_empty_if_present" => {
                            non_empty_if_present.extend(parse_lit_str_list(list.tokens)?)
                        }
                        "item_non_empty_if_present" => {
                            non_empty_if_present.extend(parse_item_lit_str_list(list.tokens)?)
                        }
                        "minimum" => {
                            minimums.push(parse_path_expr_constraint(list.tokens, "minimum")?)
                        }
                        "maximum" => {
                            maximums.push(parse_path_expr_constraint(list.tokens, "maximum")?)
                        }
                        "exclusive_minimum" => exclusive_minimums.push(parse_path_expr_constraint(
                            list.tokens,
                            "exclusive_minimum",
                        )?),
                        "exclusive_maximum" => exclusive_maximums.push(parse_path_expr_constraint(
                            list.tokens,
                            "exclusive_maximum",
                        )?),
                        "exactly_one_of" => exactly_one_of.push(parse_lit_str_list(list.tokens)?),
                        "at_least_one_of" => at_least_one_of.push(parse_lit_str_list(list.tokens)?),
                        "requires" => {
                            requires.push(parse_path_pair_constraint(list.tokens, "requires")?)
                        }
                        "conflicts_with" => conflicts_with
                            .push(parse_path_pair_constraint(list.tokens, "conflicts_with")?),
                        "required_unless_present" => required_unless_present.push(
                            parse_path_pair_constraint(list.tokens, "required_unless_present")?,
                        ),
                        "forbid_substrings" => forbid_substrings.push(
                            parse_path_lit_str_list_constraint(list.tokens, "forbid_substrings")?,
                        ),
                        "distinct_trimmed" => {
                            distinct_trimmed.extend(parse_lit_str_list(list.tokens)?)
                        }
                        "distinct_trimmed_within" => distinct_trimmed_within.push(
                            parse_path_pair_constraint(list.tokens, "distinct_trimmed_within")?,
                        ),
                        "min_items" => {
                            min_items.push(parse_path_usize_constraint(list.tokens, "min_items")?)
                        }
                        "max_items" => {
                            max_items.push(parse_path_usize_constraint(list.tokens, "max_items")?)
                        }
                        "min_properties" => min_properties
                            .push(parse_path_usize_constraint(list.tokens, "min_properties")?),
                        "max_properties" => max_properties
                            .push(parse_path_usize_constraint(list.tokens, "max_properties")?),
                        "item_minimum" => minimums.push(parse_item_path_expr_constraint(
                            list.tokens,
                            "item_minimum",
                        )?),
                        "item_maximum" => maximums.push(parse_item_path_expr_constraint(
                            list.tokens,
                            "item_maximum",
                        )?),
                        "item_exclusive_minimum" => exclusive_minimums.push(
                            parse_item_path_expr_constraint(list.tokens, "item_exclusive_minimum")?,
                        ),
                        "item_exclusive_maximum" => exclusive_maximums.push(
                            parse_item_path_expr_constraint(list.tokens, "item_exclusive_maximum")?,
                        ),
                        "item_min_properties" => min_properties.push(
                            parse_item_path_usize_constraint(list.tokens, "item_min_properties")?,
                        ),
                        "item_max_properties" => max_properties.push(
                            parse_item_path_usize_constraint(list.tokens, "item_max_properties")?,
                        ),
                        "item_min_chars" => min_chars.push(parse_item_path_usize_constraint(
                            list.tokens,
                            "item_min_chars",
                        )?),
                        "item_max_chars" => max_chars.push(parse_item_path_usize_constraint(
                            list.tokens,
                            "item_max_chars",
                        )?),
                        "item_format" => formats.push(parse_item_path_format_constraint(
                            list.tokens,
                            "item_format",
                        )?),
                        "min_chars" => {
                            min_chars.push(parse_path_usize_constraint(list.tokens, "min_chars")?)
                        }
                        "max_chars" => {
                            max_chars.push(parse_path_usize_constraint(list.tokens, "max_chars")?)
                        }
                        "format" => {
                            formats.push(parse_path_format_constraint(list.tokens, "format")?)
                        }
                        "item_pattern" => patterns.push(parse_item_path_pattern_constraint(
                            list.tokens,
                            "item_pattern",
                        )?),
                        "item_choices" => choices.push(parse_item_path_expr_list_constraint(
                            list.tokens,
                            "item_choices",
                        )?),
                        "pattern" => patterns.push(parse_path_pattern_constraint(list.tokens)?),
                        "choices" => {
                            choices.push(parse_path_expr_list_constraint(list.tokens, "choices")?)
                        }
                        other => {
                            return Err(syn::Error::new_spanned(
                                ident,
                                format!("unsupported input list '{other}'"),
                            ));
                        }
                    }
                }
                Meta::Path(path) => {
                    if path.is_ident("default") {
                        if default || default_expr.is_some() {
                            return Err(syn::Error::new_spanned(path, "duplicate input default"));
                        }
                        default = true;
                    } else {
                        return Err(syn::Error::new_spanned(
                            path,
                            "unsupported bare input argument",
                        ));
                    }
                }
            }
        }
    }
    Ok(ToolInputConfig {
        example,
        default,
        default_expr,
        normalize,
        validate,
        handler_receiver,
        handle,
        handle_with_context,
        stream_handle,
        stream_handle_with_context,
        permission_paths_handle,
        permission_networks_handle,
        handle_field,
        handle_by_value,
        trim,
        trim_suffix,
        non_empty,
        non_empty_if_present,
        minimums,
        maximums,
        exclusive_minimums,
        exclusive_maximums,
        exactly_one_of,
        at_least_one_of,
        requires,
        conflicts_with,
        required_unless_present,
        forbid_substrings,
        distinct_trimmed,
        distinct_trimmed_within,
        min_items,
        max_items,
        min_properties,
        max_properties,
        min_chars,
        max_chars,
        formats,
        patterns,
        choices,
        input_paths,
        input_networks,
        input_aliases,
        input_defaults,
        input_field_metadata,
    })
}

fn expand_input_shape_enum_normalize_fn(
    variants: &Punctuated<Variant, Token![,]>,
    enum_field_rule: Option<SerdeRenameRule>,
) -> Result<proc_macro2::TokenStream> {
    struct EnumNormalizeVariant {
        action: LitStr,
        default_when_empty: bool,
        infer_when_present: Vec<LitStr>,
        drop_keys: Vec<LitStr>,
        trim: Vec<LitStr>,
        trim_suffix: Vec<PathStringConstraint>,
        input_aliases: Vec<PluginInputFieldAliasSpec>,
        input_defaults: Vec<PluginInputFieldDefaultSpec>,
        nested_shapes: Vec<NestedInputShapeField>,
        flatten_shapes: Vec<Type>,
    }

    let mut normalize_variants = Vec::new();
    let mut action_candidates = Vec::new();
    for variant in variants {
        let config = normalized_input_variant_config(variant, enum_field_rule)?;
        let variant_field_rule = serde_rename_all_rule(&variant.attrs)?.or(enum_field_rule);
        let nested_shapes = variant
            .fields
            .iter()
            .filter_map(|field| nested_input_shape_field(field, variant_field_rule).transpose())
            .collect::<Result<Vec<_>>>()?;
        let flatten_shapes = variant
            .fields
            .iter()
            .filter_map(|field| flatten_shape_type(field).transpose())
            .collect::<Result<Vec<_>>>()?;
        let action = input_variant_action_name(variant, &config);
        action_candidates.push(action.clone());
        if config.default_when_empty
            || !config.infer_when_present.is_empty()
            || !config.drop_keys.is_empty()
            || !config.trim.is_empty()
            || !config.trim_suffix.is_empty()
            || !config.input_aliases.is_empty()
            || !config.input_defaults.is_empty()
            || !nested_shapes.is_empty()
            || !flatten_shapes.is_empty()
        {
            normalize_variants.push(EnumNormalizeVariant {
                action: input_variant_action_name(variant, &config),
                default_when_empty: config.default_when_empty,
                infer_when_present: config.infer_when_present,
                drop_keys: config.drop_keys,
                trim: config.trim,
                trim_suffix: config.trim_suffix,
                input_aliases: config.input_aliases,
                input_defaults: config.input_defaults,
                nested_shapes,
                flatten_shapes,
            });
        }
    }

    if action_candidates.is_empty() {
        return Ok(quote! {
            fn __macro_normalize_enum_input(
                input: serde_json::Value,
            ) -> ::agena_plugin_sdk::Result<serde_json::Value> {
                Ok(input)
            }
        });
    }

    let default_actions = normalize_variants
        .iter()
        .filter(|variant| variant.default_when_empty)
        .map(|variant| variant.action.value())
        .collect::<Vec<_>>();
    if default_actions.len() > 1 {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            format!(
                "only one #[input(default_when_empty = true)] variant is allowed, found {}",
                default_actions.join(", ")
            ),
        ));
    }

    let default_empty_expr = normalize_variants
        .iter()
        .find(|variant| variant.default_when_empty)
        .map(|variant| {
            let action = &variant.action;
            quote! {
                if object.is_empty() {
                    object.insert(
                        "action".to_string(),
                        serde_json::Value::String(#action.to_string()),
                    );
                }
            }
        })
        .unwrap_or_default();

    let infer_match_exprs = normalize_variants
        .iter()
        .filter(|variant| !variant.infer_when_present.is_empty())
        .map(|variant| {
            let action = &variant.action;
            let static_keys = variant
                .infer_when_present
                .iter()
                .flat_map(|path| input_keys_for_parse_path(path, &variant.input_aliases))
                .collect::<Vec<_>>();
            let flattened_key_exprs = variant
                .infer_when_present
                .iter()
                .map(|path| expand_flatten_shape_input_keys_expr(&variant.flatten_shapes, path))
                .collect::<Vec<_>>();
            let nested_key_exprs = variant
                .infer_when_present
                .iter()
                .map(|path| expand_nested_shape_input_keys_expr(&variant.nested_shapes, path))
                .collect::<Vec<_>>();
            quote! {
                let mut __paths = vec![#(#static_keys.to_string()),*];
                #(
                    __paths.extend(#flattened_key_exprs);
                )*
                #(
                    __paths.extend(#nested_key_exprs);
                )*
                __paths.sort();
                __paths.dedup();
                let __input = serde_json::Value::Object(object.clone());
                if inferred_action.is_none()
                    && __paths.iter().any(|path| {
                        ::agena_plugin_sdk::macro_support::json_path_present(
                            &__input,
                            path.as_str(),
                        )
                    })
                {
                    inferred_action = Some(#action);
                }
            }
        });

    let drop_match_arms = normalize_variants
        .iter()
        .filter(|variant| !variant.drop_keys.is_empty())
        .map(|variant| {
            let action = &variant.action;
            let static_keys = variant
                .drop_keys
                .iter()
                .flat_map(|path| input_keys_for_parse_path(path, &variant.input_aliases))
                .collect::<Vec<_>>();
            let flattened_key_exprs = variant
                .drop_keys
                .iter()
                .map(|path| expand_flatten_shape_input_keys_expr(&variant.flatten_shapes, path))
                .collect::<Vec<_>>();
            let nested_key_exprs = variant
                .drop_keys
                .iter()
                .map(|path| expand_nested_shape_input_keys_expr(&variant.nested_shapes, path))
                .collect::<Vec<_>>();
            quote! {
                #action => {
                    let mut __paths = vec![#(#static_keys.to_string()),*];
                    #(
                        __paths.extend(#flattened_key_exprs);
                    )*
                    #(
                        __paths.extend(#nested_key_exprs);
                    )*
                    __paths.sort();
                    __paths.dedup();
                    let mut input = serde_json::Value::Object(object);
                    for path in __paths {
                        ::agena_plugin_sdk::macro_support::remove_json_path(
                            &mut input,
                            path.as_str(),
                        );
                    }
                    object = match input {
                        serde_json::Value::Object(object) => object,
                        other => {
                            return Err(::agena_plugin_sdk::PluginError::invalid_params(
                                format!(
                                    "enum input normalization expected object after drop_keys, found {}",
                                    other
                                ),
                            ));
                        }
                    };
                }
            }
        });

    let normalize_match_arms = normalize_variants
        .iter()
        .filter(|variant| {
            !variant.trim.is_empty()
                || !variant.trim_suffix.is_empty()
                || !variant.input_aliases.is_empty()
                || !variant.input_defaults.is_empty()
                || !variant.nested_shapes.is_empty()
                || !variant.flatten_shapes.is_empty()
        })
        .map(|variant| {
            let action = &variant.action;
            let alias_normalize_expr = expand_input_alias_normalize_tokens(&variant.input_aliases);
            let default_insert_expr = expand_input_default_insert_tokens(&variant.input_defaults);
            let nested_normalize_expr =
                expand_nested_shape_schema_normalize_expr(&variant.nested_shapes);
            let flatten_normalize_expr =
                expand_flatten_shape_schema_normalize_expr(&variant.flatten_shapes);
            let normalize_expr = built_in_normalization_tokens(
                quote! { &mut input },
                &variant.trim,
                &variant.trim_suffix,
                &variant.flatten_shapes,
                &variant.nested_shapes,
            );
            quote! {
                #action => {
                    let mut input = serde_json::Value::Object(object);
                    #alias_normalize_expr
                    #default_insert_expr
                    #nested_normalize_expr
                    #flatten_normalize_expr
                    #normalize_expr
                    return Ok(input);
                }
            }
        });

    Ok(quote! {
        fn __macro_normalize_enum_input(
            input: serde_json::Value,
        ) -> ::agena_plugin_sdk::Result<serde_json::Value> {
            let mut object = match input {
                serde_json::Value::Object(object) => object,
                other => return Ok(other),
            };
            let action_candidates = [#(#action_candidates),*];

            #default_empty_expr

            let action = match object.get("action").and_then(serde_json::Value::as_str) {
                Some(action) => match action {
                    other if action_candidates.iter().any(|candidate| *candidate == other) => other.to_string(),
                    other => {
                        let suggestions = ::agena_plugin_sdk::macro_support::suggest_name_candidates(
                            other,
                            action_candidates,
                            1,
                        );
                        let message = ::agena_plugin_sdk::macro_support::unknown_name_message(
                            "action",
                            other,
                            &suggestions,
                        );
                        return Err(::agena_plugin_sdk::PluginError::invalid_params(message));
                    }
                },
                None => {
                    let mut inferred_action: Option<&str> = None;
                    #(#infer_match_exprs)*
                    let Some(action) = inferred_action else {
                        return Ok(serde_json::Value::Object(object));
                    };
                    let action = action.to_string();
                    object.insert(
                        "action".to_string(),
                        serde_json::Value::String(action.clone()),
                    );
                    action
                }
            };

            match action.as_str() {
                #(#drop_match_arms)*
                _ => {}
            }

            match action.as_str() {
                #(#normalize_match_arms)*
                _ => {}
            }

            Ok(serde_json::Value::Object(object))
        }
    })
}

fn expand_input_shape_enum_post_parse_normalize_expr(
    variants: &Punctuated<Variant, Token![,]>,
    enum_field_rule: Option<SerdeRenameRule>,
) -> Result<proc_macro2::TokenStream> {
    let arms = variants
        .iter()
        .map(|variant| {
            expand_input_shape_variant_post_parse_normalize_arm(variant, enum_field_rule)
        })
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    if arms.is_empty() {
        Ok(quote! { parsed })
    } else {
        Ok(quote! {
            match parsed {
                #(#arms,)*
                other => other,
            }
        })
    }
}

fn expand_input_shape_variant_post_parse_normalize_arm(
    variant: &Variant,
    enum_field_rule: Option<SerdeRenameRule>,
) -> Result<Option<proc_macro2::TokenStream>> {
    let config = normalized_input_variant_config(variant, enum_field_rule)?;
    if config.trim.is_empty() && config.trim_suffix.is_empty() {
        return Ok(None);
    }
    let variant_name = &variant.ident;
    let variant_field_rule = serde_rename_all_rule(&variant.attrs)?.or(enum_field_rule);
    let nested_shapes = variant
        .fields
        .iter()
        .filter_map(|field| nested_input_shape_field(field, variant_field_rule).transpose())
        .collect::<Result<Vec<_>>>()?;
    let flatten_shapes = variant
        .fields
        .iter()
        .filter_map(|field| flatten_shape_type(field).transpose())
        .collect::<Result<Vec<_>>>()?;
    let normalize_expr = built_in_post_parse_normalization_tokens(
        &config.trim,
        &config.trim_suffix,
        &flatten_shapes,
        &nested_shapes,
    );
    match &variant.fields {
        Fields::Named(fields) => {
            let bindings = fields
                .named
                .iter()
                .map(|field| {
                    field.ident.clone().ok_or_else(|| {
                        syn::Error::new_spanned(
                            field,
                            "named tool input variant field is missing identifier",
                        )
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            Ok(Some(quote! {
                Self::#variant_name { #(#bindings),* } => {
                    let parsed = Self::#variant_name { #(#bindings),* };
                    #normalize_expr
                }
            }))
        }
        Fields::Unnamed(fields) => {
            let bindings = fields
                .unnamed
                .iter()
                .enumerate()
                .map(|(index, _)| format_ident!("value_{index}"))
                .collect::<Vec<_>>();
            Ok(Some(quote! {
                Self::#variant_name(#(#bindings),*) => {
                    let parsed = Self::#variant_name(#(#bindings),*);
                    #normalize_expr
                }
            }))
        }
        Fields::Unit => Ok(None),
    }
}

fn expand_input_shape_variant_validation_arm(
    variant: &Variant,
    enum_field_rule: Option<SerdeRenameRule>,
) -> Result<Option<proc_macro2::TokenStream>> {
    let config = normalized_input_variant_config(variant, enum_field_rule)?;
    let variant_field_rule = serde_rename_all_rule(&variant.attrs)?.or(enum_field_rule);
    let nested_shapes = variant
        .fields
        .iter()
        .filter_map(|field| nested_input_shape_field(field, variant_field_rule).transpose())
        .collect::<Result<Vec<_>>>()?;
    let flatten_shapes = variant
        .fields
        .iter()
        .filter_map(|field| flatten_shape_type(field).transpose())
        .collect::<Result<Vec<_>>>()?;
    let has_built_in_validation = !config.non_empty.is_empty()
        || !config.non_empty_if_present.is_empty()
        || !config.minimums.is_empty()
        || !config.maximums.is_empty()
        || !config.exclusive_minimums.is_empty()
        || !config.exclusive_maximums.is_empty()
        || !config.exactly_one_of.is_empty()
        || !config.at_least_one_of.is_empty()
        || !config.requires.is_empty()
        || !config.conflicts_with.is_empty()
        || !config.required_unless_present.is_empty()
        || !config.forbid_substrings.is_empty()
        || !config.distinct_trimmed.is_empty()
        || !config.distinct_trimmed_within.is_empty()
        || !config.min_items.is_empty()
        || !config.max_items.is_empty()
        || !config.min_properties.is_empty()
        || !config.max_properties.is_empty()
        || !config.min_chars.is_empty()
        || !config.max_chars.is_empty()
        || !config.formats.is_empty()
        || !config.patterns.is_empty()
        || !config.choices.is_empty();
    if config.validate.is_none() && !has_built_in_validation {
        return Ok(None);
    }

    let variant_name = &variant.ident;
    let built_in_validate_expr = built_in_validation_tokens(
        quote! { value },
        &config.non_empty,
        &config.non_empty_if_present,
        &config.minimums,
        &config.maximums,
        &config.exclusive_minimums,
        &config.exclusive_maximums,
        &config.exactly_one_of,
        &config.at_least_one_of,
        &config.requires,
        &config.conflicts_with,
        &config.required_unless_present,
        &config.forbid_substrings,
        &config.distinct_trimmed,
        &config.distinct_trimmed_within,
        &config.min_items,
        &config.max_items,
        &config.min_properties,
        &config.max_properties,
        &config.min_chars,
        &config.max_chars,
        &config.formats,
        &config.patterns,
        &config.choices,
        &flatten_shapes,
        &nested_shapes,
    );
    let arm = match &variant.fields {
        Fields::Named(fields) => {
            let bindings = fields
                .named
                .iter()
                .map(|field| {
                    field.ident.clone().ok_or_else(|| {
                        syn::Error::new_spanned(
                            field,
                            "named tool input variant field is missing identifier",
                        )
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            let field_inserts = named_field_object_insert_tokens(
                fields.named.iter().zip(bindings.iter()),
                "flattened tool input variant fields must serialize to objects",
                serde_rename_all_rule(&variant.attrs)?.or(enum_field_rule),
            )?;
            let validate_expr = config
                .validate
                .as_ref()
                .map(|path| quote! { #path(&value)?; })
                .unwrap_or_default();
            quote! {
                Self::#variant_name { #(#bindings),* } => {
                    let value = {
                        let mut object = serde_json::Map::new();
                        #(#field_inserts)*
                        serde_json::Value::Object(object)
                    };
                    #built_in_validate_expr
                    #validate_expr
                }
            }
        }
        Fields::Unnamed(fields) => {
            if fields.unnamed.len() == 1 {
                let validate_expr = config
                    .validate
                    .as_ref()
                    .map(|path| quote! { #path(value)?; })
                    .unwrap_or_default();
                quote! {
                    Self::#variant_name(value) => {
                        #built_in_validate_expr
                        #validate_expr
                    }
                }
            } else {
                let bindings = fields
                    .unnamed
                    .iter()
                    .enumerate()
                    .map(|(index, _)| format_ident!("value_{index}"))
                    .collect::<Vec<_>>();
                let validate_expr = config
                    .validate
                    .as_ref()
                    .map(|path| quote! { #path(&value)?; })
                    .unwrap_or_default();
                quote! {
                    Self::#variant_name(#(#bindings),*) => {
                        let value = serde_json::to_value((#(#bindings,)*))
                            .map_err(|err| ::agena_plugin_sdk::PluginError::invalid_params(err.to_string()))?;
                        #built_in_validate_expr
                        #validate_expr
                    }
                }
            }
        }
        Fields::Unit => {
            return Err(syn::Error::new_spanned(
                &variant.ident,
                "tool input variant validate hooks are not supported on unit variants",
            ));
        }
    };

    Ok(Some(arm))
}

fn parse_input_variant_config(variant: &Variant) -> Result<ToolInputVariantConfig> {
    let mut action = None;
    let mut validate = None;
    let mut handle = None;
    let mut handle_with_context = None;
    let mut stream_handle = None;
    let mut stream_handle_with_context = None;
    let mut permission_paths_handle = None;
    let mut permission_networks_handle = None;
    let mut handle_by_value = false;
    let mut trim = Vec::new();
    let mut trim_suffix = Vec::new();
    let mut non_empty = Vec::new();
    let mut non_empty_if_present = Vec::new();
    let mut minimums = Vec::new();
    let mut maximums = Vec::new();
    let mut exclusive_minimums = Vec::new();
    let mut exclusive_maximums = Vec::new();
    let mut exactly_one_of = Vec::new();
    let mut at_least_one_of = Vec::new();
    let mut requires = Vec::new();
    let mut conflicts_with = Vec::new();
    let mut required_unless_present = Vec::new();
    let mut forbid_substrings = Vec::new();
    let mut distinct_trimmed = Vec::new();
    let mut distinct_trimmed_within = Vec::new();
    let mut min_items = Vec::new();
    let mut max_items = Vec::new();
    let mut min_properties = Vec::new();
    let mut max_properties = Vec::new();
    let mut min_chars = Vec::new();
    let mut max_chars = Vec::new();
    let mut formats = Vec::new();
    let mut patterns = Vec::new();
    let mut choices = Vec::new();
    let mut default_when_empty = false;
    let mut infer_when_present = Vec::new();
    let mut drop_keys = Vec::new();
    for attr in &variant.attrs {
        if !attr.path().is_ident("input") {
            continue;
        }
        let metas = attr.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)?;
        for meta in metas {
            match meta {
                Meta::NameValue(value) => {
                    let Some(ident) = value.path.get_ident() else {
                        return Err(syn::Error::new_spanned(value.path, "expected identifier"));
                    };
                    match ident.to_string().as_str() {
                        "action" => action = Some(expr_lit_str(&value.value, "action")?),
                        "exec" => {
                            return Err(syn::Error::new_spanned(
                                ident,
                                "ToolInput uses `#[input(action = \"...\")]`; `exec` is only valid for generated tool routing",
                            ));
                        }
                        "validate" => validate = Some(expr_path(&value.value, "validate")?),
                        "handle" => handle = Some(expr_path(&value.value, "handle")?),
                        "handle_with_context" => {
                            handle_with_context =
                                Some(expr_path(&value.value, "handle_with_context")?)
                        }
                        "stream_handle" => {
                            stream_handle = Some(expr_path(&value.value, "stream_handle")?)
                        }
                        "stream_handle_with_context" => {
                            stream_handle_with_context =
                                Some(expr_path(&value.value, "stream_handle_with_context")?)
                        }
                        "permission_paths_handle" => {
                            permission_paths_handle =
                                Some(expr_path(&value.value, "permission_paths_handle")?)
                        }
                        "permission_networks_handle" => {
                            permission_networks_handle =
                                Some(expr_path(&value.value, "permission_networks_handle")?)
                        }
                        "handle_by_value" => {
                            handle_by_value = expr_lit_bool(&value.value, "handle_by_value")?
                        }
                        "default_when_empty" => {
                            default_when_empty = expr_lit_bool(&value.value, "default_when_empty")?
                        }
                        "map" => {
                            return Err(syn::Error::new_spanned(
                                ident,
                                "ToolInput does not support `map`; use `#[input(action = \"...\")]` to override the action name",
                            ));
                        }
                        other => {
                            return Err(syn::Error::new_spanned(
                                ident,
                                format!("unsupported input variant attribute '{other}'"),
                            ));
                        }
                    }
                }
                Meta::List(list) => {
                    let Some(ident) = list.path.get_ident() else {
                        return Err(syn::Error::new_spanned(list.path, "expected identifier"));
                    };
                    match ident.to_string().as_str() {
                        "trim" => trim.extend(parse_lit_str_list(list.tokens)?),
                        "item_trim" => trim.extend(parse_item_lit_str_list(list.tokens)?),
                        "trim_suffix" => trim_suffix
                            .push(parse_path_lit_str_constraint(list.tokens, "trim_suffix")?),
                        "item_trim_suffix" => trim_suffix.push(parse_item_path_lit_str_constraint(
                            list.tokens,
                            "item_trim_suffix",
                        )?),
                        "non_empty" => non_empty.extend(parse_lit_str_list(list.tokens)?),
                        "item_non_empty" => non_empty.extend(parse_item_lit_str_list(list.tokens)?),
                        "non_empty_if_present" => {
                            non_empty_if_present.extend(parse_lit_str_list(list.tokens)?)
                        }
                        "item_non_empty_if_present" => {
                            non_empty_if_present.extend(parse_item_lit_str_list(list.tokens)?)
                        }
                        "minimum" => {
                            minimums.push(parse_path_expr_constraint(list.tokens, "minimum")?)
                        }
                        "maximum" => {
                            maximums.push(parse_path_expr_constraint(list.tokens, "maximum")?)
                        }
                        "exclusive_minimum" => exclusive_minimums.push(parse_path_expr_constraint(
                            list.tokens,
                            "exclusive_minimum",
                        )?),
                        "exclusive_maximum" => exclusive_maximums.push(parse_path_expr_constraint(
                            list.tokens,
                            "exclusive_maximum",
                        )?),
                        "exactly_one_of" => exactly_one_of.push(parse_lit_str_list(list.tokens)?),
                        "at_least_one_of" => at_least_one_of.push(parse_lit_str_list(list.tokens)?),
                        "requires" => {
                            requires.push(parse_path_pair_constraint(list.tokens, "requires")?)
                        }
                        "conflicts_with" => conflicts_with
                            .push(parse_path_pair_constraint(list.tokens, "conflicts_with")?),
                        "required_unless_present" => required_unless_present.push(
                            parse_path_pair_constraint(list.tokens, "required_unless_present")?,
                        ),
                        "forbid_substrings" => forbid_substrings.push(
                            parse_path_lit_str_list_constraint(list.tokens, "forbid_substrings")?,
                        ),
                        "distinct_trimmed" => {
                            distinct_trimmed.extend(parse_lit_str_list(list.tokens)?)
                        }
                        "distinct_trimmed_within" => distinct_trimmed_within.push(
                            parse_path_pair_constraint(list.tokens, "distinct_trimmed_within")?,
                        ),
                        "min_items" => {
                            min_items.push(parse_path_usize_constraint(list.tokens, "min_items")?)
                        }
                        "max_items" => {
                            max_items.push(parse_path_usize_constraint(list.tokens, "max_items")?)
                        }
                        "min_properties" => min_properties
                            .push(parse_path_usize_constraint(list.tokens, "min_properties")?),
                        "max_properties" => max_properties
                            .push(parse_path_usize_constraint(list.tokens, "max_properties")?),
                        "item_minimum" => minimums.push(parse_item_path_expr_constraint(
                            list.tokens,
                            "item_minimum",
                        )?),
                        "item_maximum" => maximums.push(parse_item_path_expr_constraint(
                            list.tokens,
                            "item_maximum",
                        )?),
                        "item_exclusive_minimum" => exclusive_minimums.push(
                            parse_item_path_expr_constraint(list.tokens, "item_exclusive_minimum")?,
                        ),
                        "item_exclusive_maximum" => exclusive_maximums.push(
                            parse_item_path_expr_constraint(list.tokens, "item_exclusive_maximum")?,
                        ),
                        "item_min_properties" => min_properties.push(
                            parse_item_path_usize_constraint(list.tokens, "item_min_properties")?,
                        ),
                        "item_max_properties" => max_properties.push(
                            parse_item_path_usize_constraint(list.tokens, "item_max_properties")?,
                        ),
                        "item_min_chars" => min_chars.push(parse_item_path_usize_constraint(
                            list.tokens,
                            "item_min_chars",
                        )?),
                        "item_max_chars" => max_chars.push(parse_item_path_usize_constraint(
                            list.tokens,
                            "item_max_chars",
                        )?),
                        "item_format" => formats.push(parse_item_path_format_constraint(
                            list.tokens,
                            "item_format",
                        )?),
                        "min_chars" => {
                            min_chars.push(parse_path_usize_constraint(list.tokens, "min_chars")?)
                        }
                        "max_chars" => {
                            max_chars.push(parse_path_usize_constraint(list.tokens, "max_chars")?)
                        }
                        "format" => {
                            formats.push(parse_path_format_constraint(list.tokens, "format")?)
                        }
                        "item_pattern" => patterns.push(parse_item_path_pattern_constraint(
                            list.tokens,
                            "item_pattern",
                        )?),
                        "item_choices" => choices.push(parse_item_path_expr_list_constraint(
                            list.tokens,
                            "item_choices",
                        )?),
                        "pattern" => patterns.push(parse_path_pattern_constraint(list.tokens)?),
                        "choices" => {
                            choices.push(parse_path_expr_list_constraint(list.tokens, "choices")?)
                        }
                        "infer_when_present" => {
                            infer_when_present.extend(parse_lit_str_list(list.tokens)?)
                        }
                        "drop_keys" => drop_keys.extend(parse_lit_str_list(list.tokens)?),
                        other => {
                            return Err(syn::Error::new_spanned(
                                ident,
                                format!("unsupported input variant list '{other}'"),
                            ));
                        }
                    }
                }
                Meta::Path(path) => {
                    return Err(syn::Error::new_spanned(
                        path,
                        "unsupported bare input variant argument",
                    ));
                }
            }
        }
    }
    Ok(ToolInputVariantConfig {
        action,
        validate,
        handle,
        handle_with_context,
        stream_handle,
        stream_handle_with_context,
        permission_paths_handle,
        permission_networks_handle,
        handle_by_value,
        trim,
        trim_suffix,
        non_empty,
        non_empty_if_present,
        minimums,
        maximums,
        exclusive_minimums,
        exclusive_maximums,
        exactly_one_of,
        at_least_one_of,
        requires,
        conflicts_with,
        required_unless_present,
        forbid_substrings,
        distinct_trimmed,
        distinct_trimmed_within,
        min_items,
        max_items,
        min_properties,
        max_properties,
        min_chars,
        max_chars,
        formats,
        patterns,
        choices,
        input_paths: Vec::new(),
        input_networks: Vec::new(),
        input_aliases: Vec::new(),
        input_defaults: Vec::new(),
        input_field_metadata: Vec::new(),
        default_when_empty,
        infer_when_present,
        drop_keys,
    })
}

fn single_segment_ident(path: &Path, label: &str) -> Result<syn::Ident> {
    if path.leading_colon.is_none() && path.segments.len() == 1 {
        Ok(path.segments.first().expect("one segment").ident.clone())
    } else {
        Err(syn::Error::new_spanned(
            path,
            format!("{label} must be a single field identifier"),
        ))
    }
}

fn input_variant_action_name(variant: &Variant, config: &ToolInputVariantConfig) -> LitStr {
    config
        .action
        .clone()
        .unwrap_or_else(|| LitStr::new(&ident_to_snake_case(&variant.ident), variant.ident.span()))
}

fn default_tool_name(ident: &syn::Ident) -> String {
    let name = ident_to_snake_case(ident);
    ["invoke_", "dispatch_", "handle_"]
        .into_iter()
        .find_map(|prefix| name.strip_prefix(prefix).map(str::to_string))
        .unwrap_or(name)
}

fn default_command_id(ident: &syn::Ident) -> String {
    let name = ident_to_snake_case(ident);
    ["command_", "cmd_", "invoke_", "dispatch_", "handle_"]
        .into_iter()
        .find_map(|prefix| name.strip_prefix(prefix).map(str::to_string))
        .unwrap_or(name)
}

fn command_title_from_id(id: &str) -> String {
    id.split(['.', '_', '-'])
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            let Some(first) = chars.next() else {
                return String::new();
            };
            format!("{}{}", first.to_ascii_uppercase(), chars.as_str())
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn ident_to_snake_case(ident: &syn::Ident) -> String {
    let chars = ident.to_string().chars().collect::<Vec<_>>();
    let mut out = String::new();
    for (index, ch) in chars.iter().copied().enumerate() {
        if ch.is_ascii_uppercase() && index > 0 {
            let prev = chars[index - 1];
            let next = chars.get(index + 1).copied();
            if prev.is_ascii_lowercase()
                || prev.is_ascii_digit()
                || next.is_some_and(|next| next.is_ascii_lowercase())
            {
                out.push('_');
            }
        }
        out.push(ch.to_ascii_lowercase());
    }
    out
}

fn parse_path_lit_str_list_constraint(
    tokens: proc_macro2::TokenStream,
    attribute: &str,
) -> Result<PathStringsConstraint> {
    let items = Punctuated::<Expr, Token![,]>::parse_terminated.parse2(tokens)?;
    let mut iter = items.iter();
    let Some(first) = iter.next() else {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            format!("{attribute} requires a path string followed by one or more strings"),
        ));
    };
    let path = expr_lit_str(first, attribute)?;
    let values = iter
        .map(|item| expr_lit_str(item, attribute))
        .collect::<Result<Vec<_>>>()?;
    if values.is_empty() {
        return Err(syn::Error::new_spanned(
            path,
            format!("{attribute} requires at least one string value"),
        ));
    }
    Ok(PathStringsConstraint { path, values })
}

fn parse_path_expr_list_constraint(
    tokens: proc_macro2::TokenStream,
    attribute: &str,
) -> Result<PathValuesConstraint> {
    let items = Punctuated::<Expr, Token![,]>::parse_terminated.parse2(tokens)?;
    let mut iter = items.iter();
    let Some(first) = iter.next() else {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            format!("{attribute} requires a path string followed by one or more values"),
        ));
    };
    let path = expr_lit_str(first, attribute)?;
    let values = iter.cloned().collect::<Vec<_>>();
    if values.is_empty() {
        return Err(syn::Error::new_spanned(
            path,
            format!("{attribute} requires at least one value"),
        ));
    }
    Ok(PathValuesConstraint { path, values })
}

fn parse_path_expr_constraint(
    tokens: proc_macro2::TokenStream,
    attribute: &str,
) -> Result<PathValueConstraint> {
    let items = Punctuated::<Expr, Token![,]>::parse_terminated.parse2(tokens)?;
    let mut iter = items.iter();
    let Some(first) = iter.next() else {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            format!("{attribute} requires a path string and one value"),
        ));
    };
    let Some(second) = iter.next() else {
        return Err(syn::Error::new_spanned(
            first,
            format!("{attribute} requires a path string and one value"),
        ));
    };
    if iter.next().is_some() {
        return Err(syn::Error::new_spanned(
            second,
            format!("{attribute} accepts exactly two arguments"),
        ));
    }
    Ok(PathValueConstraint {
        path: expr_lit_str(first, attribute)?,
        value: second.clone(),
    })
}

fn parse_path_lit_str_constraint(
    tokens: proc_macro2::TokenStream,
    attribute: &str,
) -> Result<PathStringConstraint> {
    let items = Punctuated::<Expr, Token![,]>::parse_terminated.parse2(tokens)?;
    let mut iter = items.iter();
    let Some(first) = iter.next() else {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            format!("{attribute} requires a path string and one string value"),
        ));
    };
    let Some(second) = iter.next() else {
        return Err(syn::Error::new_spanned(
            first,
            format!("{attribute} requires a path string and one string value"),
        ));
    };
    if iter.next().is_some() {
        return Err(syn::Error::new_spanned(
            second,
            format!("{attribute} accepts exactly two string arguments"),
        ));
    }
    Ok(PathStringConstraint {
        path: expr_lit_str(first, attribute)?,
        value: expr_lit_str(second, attribute)?,
    })
}

fn parse_item_path_lit_str_constraint(
    tokens: proc_macro2::TokenStream,
    attribute: &str,
) -> Result<PathStringConstraint> {
    let mut constraint = parse_path_lit_str_constraint(tokens, attribute)?;
    constraint.path = append_constraint_path_suffix(&constraint.path, "[]");
    Ok(constraint)
}

fn parse_path_pattern_constraint(tokens: proc_macro2::TokenStream) -> Result<PathStringConstraint> {
    let constraint = parse_path_lit_str_constraint(tokens, "pattern")?;
    validate_pattern_lit(&constraint.value)?;
    Ok(constraint)
}

fn parse_path_format_constraint(
    tokens: proc_macro2::TokenStream,
    attribute: &str,
) -> Result<PathStringConstraint> {
    let mut constraint = parse_path_lit_str_constraint(tokens, attribute)?;
    constraint.value = validate_format_lit(&constraint.value)?;
    Ok(constraint)
}

fn parse_item_path_usize_constraint(
    tokens: proc_macro2::TokenStream,
    attribute: &str,
) -> Result<PathUsizeConstraint> {
    let mut constraint = parse_path_usize_constraint(tokens, attribute)?;
    constraint.path = append_constraint_path_suffix(&constraint.path, "[]");
    Ok(constraint)
}

fn parse_item_path_expr_constraint(
    tokens: proc_macro2::TokenStream,
    attribute: &str,
) -> Result<PathValueConstraint> {
    let mut constraint = parse_path_expr_constraint(tokens, attribute)?;
    constraint.path = append_constraint_path_suffix(&constraint.path, "[]");
    Ok(constraint)
}

fn parse_item_path_pattern_constraint(
    tokens: proc_macro2::TokenStream,
    attribute: &str,
) -> Result<PathStringConstraint> {
    let mut constraint = parse_path_lit_str_constraint(tokens, attribute)?;
    validate_pattern_lit(&constraint.value)?;
    constraint.path = append_constraint_path_suffix(&constraint.path, "[]");
    Ok(constraint)
}

fn parse_item_path_format_constraint(
    tokens: proc_macro2::TokenStream,
    attribute: &str,
) -> Result<PathStringConstraint> {
    let mut constraint = parse_path_lit_str_constraint(tokens, attribute)?;
    constraint.value = validate_format_lit(&constraint.value)?;
    constraint.path = append_constraint_path_suffix(&constraint.path, "[]");
    Ok(constraint)
}

fn parse_item_path_expr_list_constraint(
    tokens: proc_macro2::TokenStream,
    attribute: &str,
) -> Result<PathValuesConstraint> {
    let mut constraint = parse_path_expr_list_constraint(tokens, attribute)?;
    constraint.path = append_constraint_path_suffix(&constraint.path, "[]");
    Ok(constraint)
}

fn built_in_normalization_tokens(
    target: proc_macro2::TokenStream,
    trim: &[LitStr],
    trim_suffix: &[PathStringConstraint],
    flatten_shapes: &[Type],
    nested_shapes: &[NestedInputShapeField],
) -> proc_macro2::TokenStream {
    let trim_expr = if trim.is_empty() {
        quote! {}
    } else if flatten_shapes.is_empty() && nested_shapes.is_empty() {
        quote! {
            ::agena_plugin_sdk::macro_support::normalize_trim_paths(
                #target,
                &[#(#trim),*],
            );
        }
    } else {
        let resolved_paths = trim
            .iter()
            .map(|path| expand_input_shape_resolved_path_expr(flatten_shapes, nested_shapes, path));
        quote! {
            let __paths = vec![#(#resolved_paths),*];
            let __path_refs = __paths.iter().map(::std::string::String::as_str).collect::<Vec<_>>();
            ::agena_plugin_sdk::macro_support::normalize_trim_paths(#target, __path_refs.as_slice());
        }
    };
    let trim_suffix_exprs = trim_suffix.iter().map(|constraint| {
        let path = &constraint.path;
        let value = &constraint.value;
        if flatten_shapes.is_empty() && nested_shapes.is_empty() {
            quote! {
                ::agena_plugin_sdk::macro_support::normalize_trim_suffix_path(
                    #target,
                    #path,
                    #value,
                );
            }
        } else {
            let resolved_path =
                expand_input_shape_resolved_path_expr(flatten_shapes, nested_shapes, path);
            quote! {
                let __path = #resolved_path;
                ::agena_plugin_sdk::macro_support::normalize_trim_suffix_path(
                    #target,
                    __path.as_str(),
                    #value,
                );
            }
        }
    });
    quote! {
        #trim_expr
        #(#trim_suffix_exprs)*
    }
}

fn built_in_post_parse_normalization_tokens(
    trim: &[LitStr],
    trim_suffix: &[PathStringConstraint],
    flatten_shapes: &[Type],
    nested_shapes: &[NestedInputShapeField],
) -> proc_macro2::TokenStream {
    if trim.is_empty() && trim_suffix.is_empty() {
        quote! { parsed }
    } else {
        let normalize_expr = built_in_normalization_tokens(
            quote! { input },
            trim,
            trim_suffix,
            flatten_shapes,
            nested_shapes,
        );
        quote! {
            ::agena_plugin_sdk::macro_support::normalize_typed_json_value(&parsed, |input| {
                #normalize_expr
            })?
        }
    }
}

fn built_in_validation_tokens(
    target: proc_macro2::TokenStream,
    non_empty: &[LitStr],
    non_empty_if_present: &[LitStr],
    minimums: &[PathValueConstraint],
    maximums: &[PathValueConstraint],
    exclusive_minimums: &[PathValueConstraint],
    exclusive_maximums: &[PathValueConstraint],
    exactly_one_of: &[Vec<LitStr>],
    at_least_one_of: &[Vec<LitStr>],
    requires: &[PathPairConstraint],
    conflicts_with: &[PathPairConstraint],
    required_unless_present: &[PathPairConstraint],
    forbid_substrings: &[PathStringsConstraint],
    distinct_trimmed: &[LitStr],
    distinct_trimmed_within: &[PathPairConstraint],
    min_items: &[PathUsizeConstraint],
    max_items: &[PathUsizeConstraint],
    min_properties: &[PathUsizeConstraint],
    max_properties: &[PathUsizeConstraint],
    min_chars: &[PathUsizeConstraint],
    max_chars: &[PathUsizeConstraint],
    formats: &[PathStringConstraint],
    patterns: &[PathStringConstraint],
    choices: &[PathValuesConstraint],
    flatten_shapes: &[Type],
    nested_shapes: &[NestedInputShapeField],
) -> proc_macro2::TokenStream {
    let non_empty_expr = if non_empty.is_empty() {
        quote! {}
    } else if flatten_shapes.is_empty() && nested_shapes.is_empty() {
        quote! {
            ::agena_plugin_sdk::macro_support::validate_non_empty_paths(
                &#target,
                &[#(#non_empty),*],
            )?;
        }
    } else {
        let resolved_paths = non_empty
            .iter()
            .map(|path| expand_input_shape_resolved_path_expr(flatten_shapes, nested_shapes, path));
        quote! {
            let __paths = vec![#(#resolved_paths),*];
            let __path_refs = __paths.iter().map(::std::string::String::as_str).collect::<Vec<_>>();
            ::agena_plugin_sdk::macro_support::validate_non_empty_paths(
                &#target,
                __path_refs.as_slice(),
            )?;
        }
    };
    let non_empty_if_present_expr = if non_empty_if_present.is_empty() {
        quote! {}
    } else if flatten_shapes.is_empty() && nested_shapes.is_empty() {
        quote! {
            ::agena_plugin_sdk::macro_support::validate_non_empty_if_present_paths(
                &#target,
                &[#(#non_empty_if_present),*],
            )?;
        }
    } else {
        let resolved_paths = non_empty_if_present
            .iter()
            .map(|path| expand_input_shape_resolved_path_expr(flatten_shapes, nested_shapes, path));
        quote! {
            let __paths = vec![#(#resolved_paths),*];
            let __path_refs = __paths.iter().map(::std::string::String::as_str).collect::<Vec<_>>();
            ::agena_plugin_sdk::macro_support::validate_non_empty_if_present_paths(
                &#target,
                __path_refs.as_slice(),
            )?;
        }
    };
    let minimum_exprs = minimums.iter().map(|constraint| {
        let path = &constraint.path;
        let value = &constraint.value;
        if flatten_shapes.is_empty() && nested_shapes.is_empty() {
            quote! {
                ::agena_plugin_sdk::macro_support::validate_minimum_path(
                    &#target,
                    #path,
                    &::agena_plugin_sdk::serde_json::json!(#value),
                )?;
            }
        } else {
            let resolved_path =
                expand_input_shape_resolved_path_expr(flatten_shapes, nested_shapes, path);
            quote! {
                let __path = #resolved_path;
                ::agena_plugin_sdk::macro_support::validate_minimum_path(
                    &#target,
                    __path.as_str(),
                    &::agena_plugin_sdk::serde_json::json!(#value),
                )?;
            }
        }
    });
    let maximum_exprs = maximums.iter().map(|constraint| {
        let path = &constraint.path;
        let value = &constraint.value;
        if flatten_shapes.is_empty() && nested_shapes.is_empty() {
            quote! {
                ::agena_plugin_sdk::macro_support::validate_maximum_path(
                    &#target,
                    #path,
                    &::agena_plugin_sdk::serde_json::json!(#value),
                )?;
            }
        } else {
            let resolved_path =
                expand_input_shape_resolved_path_expr(flatten_shapes, nested_shapes, path);
            quote! {
                let __path = #resolved_path;
                ::agena_plugin_sdk::macro_support::validate_maximum_path(
                    &#target,
                    __path.as_str(),
                    &::agena_plugin_sdk::serde_json::json!(#value),
                )?;
            }
        }
    });
    let exclusive_minimum_exprs = exclusive_minimums.iter().map(|constraint| {
        let path = &constraint.path;
        let value = &constraint.value;
        if flatten_shapes.is_empty() && nested_shapes.is_empty() {
            quote! {
                ::agena_plugin_sdk::macro_support::validate_exclusive_minimum_path(
                    &#target,
                    #path,
                    &::agena_plugin_sdk::serde_json::json!(#value),
                )?;
            }
        } else {
            let resolved_path =
                expand_input_shape_resolved_path_expr(flatten_shapes, nested_shapes, path);
            quote! {
                let __path = #resolved_path;
                ::agena_plugin_sdk::macro_support::validate_exclusive_minimum_path(
                    &#target,
                    __path.as_str(),
                    &::agena_plugin_sdk::serde_json::json!(#value),
                )?;
            }
        }
    });
    let exclusive_maximum_exprs = exclusive_maximums.iter().map(|constraint| {
        let path = &constraint.path;
        let value = &constraint.value;
        if flatten_shapes.is_empty() && nested_shapes.is_empty() {
            quote! {
                ::agena_plugin_sdk::macro_support::validate_exclusive_maximum_path(
                    &#target,
                    #path,
                    &::agena_plugin_sdk::serde_json::json!(#value),
                )?;
            }
        } else {
            let resolved_path =
                expand_input_shape_resolved_path_expr(flatten_shapes, nested_shapes, path);
            quote! {
                let __path = #resolved_path;
                ::agena_plugin_sdk::macro_support::validate_exclusive_maximum_path(
                    &#target,
                    __path.as_str(),
                    &::agena_plugin_sdk::serde_json::json!(#value),
                )?;
            }
        }
    });
    let exactly_one_of_exprs = exactly_one_of.iter().map(|group| {
        if flatten_shapes.is_empty() && nested_shapes.is_empty() {
            quote! {
                ::agena_plugin_sdk::macro_support::validate_exactly_one_of_paths(
                    &#target,
                    &[#(#group),*],
                )?;
            }
        } else {
            let resolved_paths = group
                .iter()
                .map(|path| expand_input_shape_resolved_path_expr(flatten_shapes, nested_shapes, path));
            quote! {
                let __paths = vec![#(#resolved_paths),*];
                let __path_refs = __paths.iter().map(::std::string::String::as_str).collect::<Vec<_>>();
                ::agena_plugin_sdk::macro_support::validate_exactly_one_of_paths(
                    &#target,
                    __path_refs.as_slice(),
                )?;
            }
        }
    });
    let at_least_one_of_exprs = at_least_one_of.iter().map(|group| {
        if flatten_shapes.is_empty() && nested_shapes.is_empty() {
            quote! {
                ::agena_plugin_sdk::macro_support::validate_at_least_one_of_paths(
                    &#target,
                    &[#(#group),*],
                )?;
            }
        } else {
            let resolved_paths = group
                .iter()
                .map(|path| expand_input_shape_resolved_path_expr(flatten_shapes, nested_shapes, path));
            quote! {
                let __paths = vec![#(#resolved_paths),*];
                let __path_refs = __paths.iter().map(::std::string::String::as_str).collect::<Vec<_>>();
                ::agena_plugin_sdk::macro_support::validate_at_least_one_of_paths(
                    &#target,
                    __path_refs.as_slice(),
                )?;
            }
        }
    });
    let requires_exprs = requires.iter().map(|constraint| {
        let left = &constraint.left;
        let right = &constraint.right;
        if flatten_shapes.is_empty() && nested_shapes.is_empty() {
            quote! {
                ::agena_plugin_sdk::macro_support::validate_requires_path(
                    &#target,
                    #left,
                    #right,
                )?;
            }
        } else {
            let resolved_left =
                expand_input_shape_resolved_path_expr(flatten_shapes, nested_shapes, left);
            let resolved_right =
                expand_input_shape_resolved_path_expr(flatten_shapes, nested_shapes, right);
            quote! {
                let __left = #resolved_left;
                let __right = #resolved_right;
                ::agena_plugin_sdk::macro_support::validate_requires_path(
                    &#target,
                    __left.as_str(),
                    __right.as_str(),
                )?;
            }
        }
    });
    let conflicts_exprs = conflicts_with.iter().map(|constraint| {
        let left = &constraint.left;
        let right = &constraint.right;
        if flatten_shapes.is_empty() && nested_shapes.is_empty() {
            quote! {
                ::agena_plugin_sdk::macro_support::validate_conflicts_with_path(
                    &#target,
                    #left,
                    #right,
                )?;
            }
        } else {
            let resolved_left =
                expand_input_shape_resolved_path_expr(flatten_shapes, nested_shapes, left);
            let resolved_right =
                expand_input_shape_resolved_path_expr(flatten_shapes, nested_shapes, right);
            quote! {
                let __left = #resolved_left;
                let __right = #resolved_right;
                ::agena_plugin_sdk::macro_support::validate_conflicts_with_path(
                    &#target,
                    __left.as_str(),
                    __right.as_str(),
                )?;
            }
        }
    });
    let required_unless_exprs = required_unless_present.iter().map(|constraint| {
        let left = &constraint.left;
        let right = &constraint.right;
        if flatten_shapes.is_empty() && nested_shapes.is_empty() {
            quote! {
                ::agena_plugin_sdk::macro_support::validate_required_unless_present_path(
                    &#target,
                    #left,
                    #right,
                )?;
            }
        } else {
            let resolved_left =
                expand_input_shape_resolved_path_expr(flatten_shapes, nested_shapes, left);
            let resolved_right =
                expand_input_shape_resolved_path_expr(flatten_shapes, nested_shapes, right);
            quote! {
                let __left = #resolved_left;
                let __right = #resolved_right;
                ::agena_plugin_sdk::macro_support::validate_required_unless_present_path(
                    &#target,
                    __left.as_str(),
                    __right.as_str(),
                )?;
            }
        }
    });
    let forbid_substrings_exprs = forbid_substrings.iter().map(|constraint| {
        let path = &constraint.path;
        let values = &constraint.values;
        if flatten_shapes.is_empty() && nested_shapes.is_empty() {
            quote! {
                ::agena_plugin_sdk::macro_support::validate_forbid_substrings_path(
                    &#target,
                    #path,
                    &[#(#values),*],
                )?;
            }
        } else {
            let resolved_path =
                expand_input_shape_resolved_path_expr(flatten_shapes, nested_shapes, path);
            quote! {
                let __path = #resolved_path;
                ::agena_plugin_sdk::macro_support::validate_forbid_substrings_path(
                    &#target,
                    __path.as_str(),
                    &[#(#values),*],
                )?;
            }
        }
    });
    let distinct_trimmed_exprs = distinct_trimmed.iter().map(|path| {
        if flatten_shapes.is_empty() && nested_shapes.is_empty() {
            quote! {
                ::agena_plugin_sdk::macro_support::validate_distinct_trimmed_path(
                    &#target,
                    #path,
                )?;
            }
        } else {
            let resolved_path =
                expand_input_shape_resolved_path_expr(flatten_shapes, nested_shapes, path);
            quote! {
                let __path = #resolved_path;
                ::agena_plugin_sdk::macro_support::validate_distinct_trimmed_path(
                    &#target,
                    __path.as_str(),
                )?;
            }
        }
    });
    let distinct_trimmed_within_exprs = distinct_trimmed_within.iter().map(|constraint| {
        let path = &constraint.left;
        let scope = &constraint.right;
        if flatten_shapes.is_empty() && nested_shapes.is_empty() {
            quote! {
                ::agena_plugin_sdk::macro_support::validate_distinct_trimmed_within_path(
                    &#target,
                    #path,
                    #scope,
                )?;
            }
        } else {
            let resolved_path =
                expand_input_shape_resolved_path_expr(flatten_shapes, nested_shapes, path);
            let resolved_scope =
                expand_input_shape_resolved_path_expr(flatten_shapes, nested_shapes, scope);
            quote! {
                let __path = #resolved_path;
                let __scope = #resolved_scope;
                ::agena_plugin_sdk::macro_support::validate_distinct_trimmed_within_path(
                    &#target,
                    __path.as_str(),
                    __scope.as_str(),
                )?;
            }
        }
    });
    let min_items_exprs = min_items.iter().map(|constraint| {
        let path = &constraint.path;
        let value = constraint.value;
        if flatten_shapes.is_empty() && nested_shapes.is_empty() {
            quote! {
                ::agena_plugin_sdk::macro_support::validate_min_items_path(
                    &#target,
                    #path,
                    #value,
                )?;
            }
        } else {
            let resolved_path =
                expand_input_shape_resolved_path_expr(flatten_shapes, nested_shapes, path);
            quote! {
                let __path = #resolved_path;
                ::agena_plugin_sdk::macro_support::validate_min_items_path(
                    &#target,
                    __path.as_str(),
                    #value,
                )?;
            }
        }
    });
    let max_items_exprs = max_items.iter().map(|constraint| {
        let path = &constraint.path;
        let value = constraint.value;
        if flatten_shapes.is_empty() && nested_shapes.is_empty() {
            quote! {
                ::agena_plugin_sdk::macro_support::validate_max_items_path(
                    &#target,
                    #path,
                    #value,
                )?;
            }
        } else {
            let resolved_path =
                expand_input_shape_resolved_path_expr(flatten_shapes, nested_shapes, path);
            quote! {
                let __path = #resolved_path;
                ::agena_plugin_sdk::macro_support::validate_max_items_path(
                    &#target,
                    __path.as_str(),
                    #value,
                )?;
            }
        }
    });
    let min_properties_exprs = min_properties.iter().map(|constraint| {
        let path = &constraint.path;
        let value = constraint.value;
        if flatten_shapes.is_empty() && nested_shapes.is_empty() {
            quote! {
                ::agena_plugin_sdk::macro_support::validate_min_properties_path(
                    &#target,
                    #path,
                    #value,
                )?;
            }
        } else {
            let resolved_path =
                expand_input_shape_resolved_path_expr(flatten_shapes, nested_shapes, path);
            quote! {
                let __path = #resolved_path;
                ::agena_plugin_sdk::macro_support::validate_min_properties_path(
                    &#target,
                    __path.as_str(),
                    #value,
                )?;
            }
        }
    });
    let max_properties_exprs = max_properties.iter().map(|constraint| {
        let path = &constraint.path;
        let value = constraint.value;
        if flatten_shapes.is_empty() && nested_shapes.is_empty() {
            quote! {
                ::agena_plugin_sdk::macro_support::validate_max_properties_path(
                    &#target,
                    #path,
                    #value,
                )?;
            }
        } else {
            let resolved_path =
                expand_input_shape_resolved_path_expr(flatten_shapes, nested_shapes, path);
            quote! {
                let __path = #resolved_path;
                ::agena_plugin_sdk::macro_support::validate_max_properties_path(
                    &#target,
                    __path.as_str(),
                    #value,
                )?;
            }
        }
    });
    let min_chars_exprs = min_chars.iter().map(|constraint| {
        let path = &constraint.path;
        let value = constraint.value;
        if flatten_shapes.is_empty() && nested_shapes.is_empty() {
            quote! {
                ::agena_plugin_sdk::macro_support::validate_min_chars_path(
                    &#target,
                    #path,
                    #value,
                )?;
            }
        } else {
            let resolved_path =
                expand_input_shape_resolved_path_expr(flatten_shapes, nested_shapes, path);
            quote! {
                let __path = #resolved_path;
                ::agena_plugin_sdk::macro_support::validate_min_chars_path(
                    &#target,
                    __path.as_str(),
                    #value,
                )?;
            }
        }
    });
    let max_chars_exprs = max_chars.iter().map(|constraint| {
        let path = &constraint.path;
        let value = constraint.value;
        if flatten_shapes.is_empty() && nested_shapes.is_empty() {
            quote! {
                ::agena_plugin_sdk::macro_support::validate_max_chars_path(
                    &#target,
                    #path,
                    #value,
                )?;
            }
        } else {
            let resolved_path =
                expand_input_shape_resolved_path_expr(flatten_shapes, nested_shapes, path);
            quote! {
                let __path = #resolved_path;
                ::agena_plugin_sdk::macro_support::validate_max_chars_path(
                    &#target,
                    __path.as_str(),
                    #value,
                )?;
            }
        }
    });
    let format_exprs = formats.iter().map(|constraint| {
        let path = &constraint.path;
        let value = &constraint.value;
        if flatten_shapes.is_empty() && nested_shapes.is_empty() {
            quote! {
                ::agena_plugin_sdk::macro_support::validate_format_path(
                    &#target,
                    #path,
                    #value,
                )?;
            }
        } else {
            let resolved_path =
                expand_input_shape_resolved_path_expr(flatten_shapes, nested_shapes, path);
            quote! {
                let __path = #resolved_path;
                ::agena_plugin_sdk::macro_support::validate_format_path(
                    &#target,
                    __path.as_str(),
                    #value,
                )?;
            }
        }
    });
    let pattern_exprs = patterns.iter().map(|constraint| {
        let path = &constraint.path;
        let value = &constraint.value;
        if flatten_shapes.is_empty() && nested_shapes.is_empty() {
            quote! {
                ::agena_plugin_sdk::macro_support::validate_pattern_path(
                    &#target,
                    #path,
                    #value,
                )?;
            }
        } else {
            let resolved_path =
                expand_input_shape_resolved_path_expr(flatten_shapes, nested_shapes, path);
            quote! {
                let __path = #resolved_path;
                ::agena_plugin_sdk::macro_support::validate_pattern_path(
                    &#target,
                    __path.as_str(),
                    #value,
                )?;
            }
        }
    });
    let choices_exprs = choices.iter().map(|constraint| {
        let path = &constraint.path;
        let values = &constraint.values;
        if flatten_shapes.is_empty() && nested_shapes.is_empty() {
            quote! {
                ::agena_plugin_sdk::macro_support::validate_allowed_values_path(
                    &#target,
                    #path,
                    &[#(::agena_plugin_sdk::serde_json::json!(#values)),*],
                )?;
            }
        } else {
            let resolved_path =
                expand_input_shape_resolved_path_expr(flatten_shapes, nested_shapes, path);
            quote! {
                let __path = #resolved_path;
                ::agena_plugin_sdk::macro_support::validate_allowed_values_path(
                    &#target,
                    __path.as_str(),
                    &[#(::agena_plugin_sdk::serde_json::json!(#values)),*],
                )?;
            }
        }
    });
    quote! {
        #(#min_items_exprs)*
        #(#max_items_exprs)*
        #(#min_properties_exprs)*
        #(#max_properties_exprs)*
        #(#min_chars_exprs)*
        #(#max_chars_exprs)*
        #(#format_exprs)*
        #(#pattern_exprs)*
        #(#choices_exprs)*
        #non_empty_expr
        #non_empty_if_present_expr
        #(#minimum_exprs)*
        #(#maximum_exprs)*
        #(#exclusive_minimum_exprs)*
        #(#exclusive_maximum_exprs)*
        #(#forbid_substrings_exprs)*
        #(#distinct_trimmed_exprs)*
        #(#distinct_trimmed_within_exprs)*
        #(#exactly_one_of_exprs)*
        #(#at_least_one_of_exprs)*
        #(#requires_exprs)*
        #(#conflicts_exprs)*
        #(#required_unless_exprs)*
    }
}

fn parse_expr_list(tokens: proc_macro2::TokenStream) -> Result<Vec<Expr>> {
    Punctuated::<Expr, Token![,]>::parse_terminated
        .parse2(tokens)
        .map(|items| items.into_iter().collect())
}

fn parse_lit_str_list(tokens: proc_macro2::TokenStream) -> Result<Vec<LitStr>> {
    let items = Punctuated::<Expr, Token![,]>::parse_terminated.parse2(tokens)?;
    items
        .iter()
        .map(|expr| expr_lit_str(expr, "path"))
        .collect()
}

fn parse_item_lit_str_list(tokens: proc_macro2::TokenStream) -> Result<Vec<LitStr>> {
    parse_lit_str_list(tokens).map(|items| {
        items
            .into_iter()
            .map(|path| append_constraint_path_suffix(&path, "[]"))
            .collect()
    })
}

fn parse_path_usize_constraint(
    tokens: proc_macro2::TokenStream,
    attribute: &str,
) -> Result<PathUsizeConstraint> {
    let items = Punctuated::<Expr, Token![,]>::parse_terminated.parse2(tokens)?;
    let mut iter = items.into_iter();
    let Some(path_expr) = iter.next() else {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            format!("{attribute} requires a path string and usize value"),
        ));
    };
    let Some(value_expr) = iter.next() else {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            format!("{attribute} requires a path string and usize value"),
        ));
    };
    if iter.next().is_some() {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            format!("{attribute} accepts exactly two arguments"),
        ));
    }
    Ok(PathUsizeConstraint {
        path: expr_lit_str(&path_expr, attribute)?,
        value: expr_lit_usize(&value_expr, attribute)?,
    })
}

fn parse_path_pair_constraint(
    tokens: proc_macro2::TokenStream,
    attribute: &str,
) -> Result<PathPairConstraint> {
    let items = Punctuated::<Expr, Token![,]>::parse_terminated.parse2(tokens)?;
    let mut iter = items.into_iter();
    let Some(left_expr) = iter.next() else {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            format!("{attribute} requires exactly two path strings"),
        ));
    };
    let Some(right_expr) = iter.next() else {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            format!("{attribute} requires exactly two path strings"),
        ));
    };
    if iter.next().is_some() {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            format!("{attribute} requires exactly two path strings"),
        ));
    }
    Ok(PathPairConstraint {
        left: expr_lit_str(&left_expr, attribute)?,
        right: expr_lit_str(&right_expr, attribute)?,
    })
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

fn expr_string_like(expr: &Expr, field: &str) -> Result<LitStr> {
    match expr {
        Expr::Lit(ExprLit {
            lit: Lit::Str(value),
            ..
        }) => Ok(value.clone()),
        Expr::Path(path) if path.qself.is_none() && path.path.segments.len() == 1 => Ok(
            LitStr::new(&path.path.segments[0].ident.to_string(), path.span()),
        ),
        _ => Err(syn::Error::new_spanned(
            expr,
            format!("{field} must be a string literal or bare identifier"),
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

fn expr_lit_usize(expr: &Expr, field: &str) -> Result<usize> {
    match expr {
        Expr::Lit(ExprLit {
            lit: Lit::Int(value),
            ..
        }) => value.base10_parse::<usize>().map_err(|err| {
            syn::Error::new_spanned(expr, format!("{field} must be a usize literal: {err}"))
        }),
        _ => Err(syn::Error::new_spanned(
            expr,
            format!("{field} must be a usize literal"),
        )),
    }
}

fn expr_array_values(expr: &Expr, field: &str) -> Result<Vec<Expr>> {
    let Expr::Array(array) = expr else {
        return Err(syn::Error::new_spanned(
            expr,
            format!("{field} must be an array literal"),
        ));
    };
    if array.elems.is_empty() {
        return Err(syn::Error::new_spanned(
            expr,
            format!("{field} must include at least one value"),
        ));
    }
    Ok(array.elems.iter().cloned().collect())
}

fn expr_array_lit_strs(expr: &Expr, field: &str) -> Result<Vec<LitStr>> {
    expr_array_values(expr, field)?
        .iter()
        .map(|item| expr_lit_str(item, field))
        .collect()
}

fn expr_lit_i32(expr: &Expr, field: &str) -> Result<i32> {
    match expr {
        Expr::Lit(ExprLit {
            lit: Lit::Int(value),
            ..
        }) => value.base10_parse::<i32>().map_err(|err| {
            syn::Error::new_spanned(expr, format!("{field} must be an i32 literal: {err}"))
        }),
        Expr::Unary(unary) if matches!(unary.op, syn::UnOp::Neg(_)) => {
            let Expr::Lit(ExprLit {
                lit: Lit::Int(value),
                ..
            }) = unary.expr.as_ref()
            else {
                return Err(syn::Error::new_spanned(
                    expr,
                    format!("{field} must be an i32 literal"),
                ));
            };
            let value = value.base10_parse::<i32>().map_err(|err| {
                syn::Error::new_spanned(expr, format!("{field} must be an i32 literal: {err}"))
            })?;
            value
                .checked_neg()
                .ok_or_else(|| syn::Error::new_spanned(expr, format!("{field} is below i32::MIN")))
        }
        _ => Err(syn::Error::new_spanned(
            expr,
            format!("{field} must be an i32 literal"),
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

fn doc_text(attrs: &[Attribute]) -> Option<String> {
    let mut lines = Vec::new();
    for attr in attrs {
        if !attr.path().is_ident("doc") {
            continue;
        }
        let Meta::NameValue(value) = &attr.meta else {
            continue;
        };
        if let Expr::Lit(ExprLit {
            lit: Lit::Str(value),
            ..
        }) = &value.value
        {
            lines.push(value.value().trim().to_string());
        }
    }
    if lines.is_empty() {
        return None;
    }
    Some(normalize_doc_lines(&lines))
}

fn normalize_doc_lines(lines: &[String]) -> String {
    let mut output = String::new();
    let mut previous_blank = false;
    for line in lines {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            if !output.is_empty() && !previous_blank {
                output.push('\n');
            }
            previous_blank = true;
            continue;
        }
        if !output.is_empty() {
            output.push('\n');
        }
        output.push_str(trimmed);
        previous_blank = false;
    }
    output.trim().to_string()
}

fn doc_summary(doc: Option<&str>) -> Option<String> {
    let doc = doc?.trim();
    if doc.is_empty() {
        return None;
    }
    let first_paragraph = doc.split("\n\n").next()?.trim();
    if first_paragraph.is_empty() {
        return None;
    }
    Some(
        first_paragraph
            .lines()
            .map(str::trim)
            .collect::<Vec<_>>()
            .join(" "),
    )
}

fn lit_str_from_text(text: Option<&str>) -> Option<LitStr> {
    text.map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| LitStr::new(value, proc_macro2::Span::call_site()))
}
