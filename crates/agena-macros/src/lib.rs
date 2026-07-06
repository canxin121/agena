use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::ext::IdentExt;
use syn::parse::Parser;
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::{
    Attribute, Data, DeriveInput, Expr, ExprLit, ExprPath, Field, Fields, FnArg, Ident, ImplItem,
    ImplItemFn, Index, ItemImpl, Lit, LitBool, LitStr, Member, Meta, MetaList, MetaNameValue, Path,
    PathArguments, Result, Token, Type, Variant, parse_macro_input, parse_quote,
};

#[proc_macro_attribute]
pub fn plugin(attr: TokenStream, item: TokenStream) -> TokenStream {
    let attr = parse_macro_input!(attr as proc_macro2::TokenStream);
    let item = parse_macro_input!(item as ItemImpl);
    match expand_plugin_impl_attr(attr, item) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

#[proc_macro_derive(StaticToolSurface, attributes(tool_surface, tool_command, tool))]
pub fn derive_static_tool_surface(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match expand_static_tool_surface(input) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

#[proc_macro_derive(ToolSuite, attributes(tool_suite, tool_subcommands, tool))]
pub fn derive_tool_suite(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match expand_tool_suite(input) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

#[proc_macro_derive(ToolInputShape, attributes(tool_input, tool_args, tool))]
pub fn derive_tool_input_shape(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match expand_tool_input_shape(input) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

#[proc_macro_derive(ToolCommand, attributes(tool_surface, tool_command, tool))]
pub fn derive_tool_command(input: TokenStream) -> TokenStream {
    derive_static_tool_surface(input)
}

#[proc_macro_derive(ToolSubcommands, attributes(tool_suite, tool_subcommands, tool))]
pub fn derive_tool_subcommands(input: TokenStream) -> TokenStream {
    derive_tool_suite(input)
}

#[proc_macro_derive(ToolArgs, attributes(tool_input, tool_args, tool))]
pub fn derive_tool_args(input: TokenStream) -> TokenStream {
    derive_tool_input_shape(input)
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
            "#[plugin(...)] only supports inherent impl blocks; write `#[async_trait] impl Plugin for Type` manually for dynamic plugins",
        ));
    }

    if attr.is_empty() {
        return Err(syn::Error::new_spanned(
            &item.self_ty,
            "#[plugin(...)] inherent impls require id/version/description metadata",
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
                ::agena_plugin_sdk::macro_support::json_schema_for_with_default(
                    <#config_ty as ::core::default::Default>::default(),
                )
            }
        }
        PluginConfigDefault::Expr(default) => {
            quote! { ::agena_plugin_sdk::macro_support::json_schema_for_with_default(#default) }
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

#[derive(Default)]
struct PluginImplConfig {
    id: Option<Expr>,
    version: Option<Expr>,
    description: Option<Expr>,
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
    commands: Option<Expr>,
    plugin_capabilities_expr: Option<Expr>,
    plugin_capabilities: Vec<Expr>,
    explicit_hooks: Option<Expr>,
    export: Option<Ident>,
    export_bind: Option<Expr>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PluginToolBindingKind {
    Surface,
    Suite,
}

#[derive(Clone)]
struct PluginToolBinding {
    method: Ident,
    ty: Type,
    kind: PluginToolBindingKind,
    is_async: bool,
    context: Option<PluginContextArg>,
}

#[derive(Clone)]
struct PluginStreamToolBinding {
    method: Ident,
    ty: Type,
    is_async: bool,
    sink_first: bool,
    kind: PluginToolBindingKind,
    for_method: Option<Ident>,
}

#[derive(Clone)]
struct PluginPermissionBinding {
    method: Ident,
    ty: Type,
    is_async: bool,
    kind: PluginToolBindingKind,
}

#[derive(Clone, Copy)]
struct PluginContextArg {
    first: bool,
    by_ref: bool,
}

enum PluginPermissionTarget {
    Paths,
    Networks,
}

struct PluginPermissionAttr {
    target: PluginPermissionTarget,
    kind: PluginToolBindingKind,
}

#[derive(Clone)]
struct PluginHookBinding {
    method: Ident,
    hook: PluginHookKind,
    is_async: bool,
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
    let mut tool_bindings = Vec::new();
    let mut stream_bindings = Vec::new();
    let mut permission_path_bindings = Vec::new();
    let mut permission_network_bindings = Vec::new();
    let mut hook_bindings = Vec::new();

    for impl_item in &mut item.items {
        let ImplItem::Fn(method) = impl_item else {
            continue;
        };
        let attrs = parse_plugin_inherent_method_attrs(method)?;
        tool_bindings.extend(attrs.tools);
        stream_bindings.extend(attrs.stream_tools);
        permission_path_bindings.extend(attrs.permission_paths);
        permission_network_bindings.extend(attrs.permission_networks);
        hook_bindings.extend(attrs.hooks);
    }

    resolve_plugin_stream_bindings(&tool_bindings, &mut stream_bindings)?;
    reject_duplicate_hook_bindings(&hook_bindings)?;

    let manifest_method = expand_plugin_layer_manifest(
        &config,
        &self_ty,
        docs.as_deref(),
        &tool_bindings,
        &stream_bindings,
        &permission_path_bindings,
        &permission_network_bindings,
        &hook_bindings,
    )?;
    let tool_invoke_method = (!tool_bindings.is_empty())
        .then(|| expand_plugin_layer_tool_invoke(&self_ty, &tool_bindings))
        .transpose()?;
    let stream_method = (!stream_bindings.is_empty())
        .then(|| expand_plugin_layer_tool_stream(&self_ty, &stream_bindings))
        .transpose()?;
    let permission_paths_method = (!permission_path_bindings.is_empty())
        .then(|| expand_plugin_layer_permission_paths(&self_ty, &permission_path_bindings))
        .transpose()?;
    let permission_networks_method = (!permission_network_bindings.is_empty())
        .then(|| expand_plugin_layer_permission_networks(&self_ty, &permission_network_bindings))
        .transpose()?;
    let init_binding = hook_bindings
        .iter()
        .find(|binding| binding.hook == PluginHookKind::Init);
    let init_method =
        (config.config_field.is_some() || config.config_store || init_binding.is_some())
            .then(|| expand_plugin_layer_init_method(&config, &self_ty, init_binding))
            .transpose()?;
    let hook_methods = hook_bindings
        .iter()
        .filter(|binding| binding.hook != PluginHookKind::Init)
        .map(|binding| expand_plugin_layer_hook_method(&self_ty, binding))
        .collect::<Result<Vec<_>>>()?;
    let generics = &item.generics;
    let export = expand_plugin_layer_export(&config, &self_ty, generics)?;
    let (impl_generics, _ty_generics, where_clause) = generics.split_for_impl();
    Ok(quote! {
        #item

        #[::agena_plugin_sdk::async_trait]
        impl #impl_generics ::agena_plugin_sdk::Plugin for #self_ty #where_clause {
            #manifest_method
            #tool_invoke_method
            #stream_method
            #permission_paths_method
            #permission_networks_method
            #init_method
            #(#hook_methods)*
        }

        #export
    })
}

fn parse_plugin_impl_config(attr: proc_macro2::TokenStream) -> Result<PluginImplConfig> {
    let metas = Punctuated::<Meta, Token![,]>::parse_terminated.parse2(attr)?;
    let mut config = PluginImplConfig::default();
    for meta in metas {
        match meta {
            Meta::NameValue(value) => apply_plugin_impl_name_value(&mut config, value)?,
            Meta::List(list) => apply_plugin_impl_list(&mut config, list)?,
            Meta::Path(path) => {
                if path.is_ident("config") {
                    config.config_store = true;
                    config.config_schema_store = true;
                    continue;
                }
                if path.is_ident("config_store") {
                    config.config_store = true;
                    config.config_schema_store = true;
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
        ("id", config.id.is_some()),
        ("version", config.version.is_some()),
    ] {
        if !present {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                format!("#[plugin(...)] requires `{label} = ...`"),
            ));
        }
    }
    if config.config_field.is_some() && config.config_schema_type.is_none() {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            "#[plugin(..., config_field = field)] requires `config = Type` or `config_schema_type = Type`",
        ));
    }
    if config.config_store && config.config_schema_type.is_none() {
        config.config_schema_store = true;
    }
    if config.config_schema_default.is_some()
        && config.config_schema_type.is_none()
        && config.config_schema_store
    {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            "put derived config defaults on the field, e.g. `#[config(default)]`; `config_default = ...` requires `config = Type` or `config_schema_type = Type`",
        ));
    }
    Ok(config)
}

fn apply_plugin_impl_name_value(config: &mut PluginImplConfig, value: MetaNameValue) -> Result<()> {
    let Some(ident) = value.path.get_ident() else {
        return Err(syn::Error::new_spanned(value.path, "expected identifier"));
    };
    match ident.to_string().as_str() {
        "id" => config.id = Some(value.value),
        "version" => config.version = Some(value.value),
        "description" | "long_about" => config.description = Some(value.value),
        "summary" | "about" => config.summary = Some(value.value),
        "help" | "long_help" => config.help = Some(value.value),
        "config" => {
            config.config_schema_type = Some(expr_as_type(value.value)?);
            config.config_store = true;
        }
        "config_schema" => config.config_schema = Some(value.value),
        "config_schema_type" => config.config_schema_type = Some(expr_as_type(value.value)?),
        "config_default" => config.config_schema_default = Some(value.value),
        "config_schema_default" => config.config_schema_default = Some(value.value),
        "config_field" => config.config_field = Some(expr_path_ident(value.value, "config_field")?),
        "config_store" => config.config_store = expr_bool(value.value, "config_store")?,
        "display" => config.display = Some(expr_path_ident(value.value, "display")?),
        "ui_display" => config.ui_display = Some(expr_path_ident(value.value, "ui_display")?),
        "tool_description_mode" => config.tool_description_mode = Some(value.value),
        "ui_display_mode" => config.ui_display_mode = Some(value.value),
        "commands" => config.commands = Some(value.value),
        "plugin_capabilities" => config.plugin_capabilities_expr = Some(value.value),
        "hooks" => config.explicit_hooks = Some(value.value),
        "export" => config.export = Some(expr_path_ident(value.value, "export")?),
        "bind" | "export_bind" | "http_bind" => config.export_bind = Some(value.value),
        other => {
            return Err(syn::Error::new_spanned(
                ident,
                format!("unsupported plugin argument '{other}'"),
            ));
        }
    }
    Ok(())
}

fn apply_plugin_impl_list(config: &mut PluginImplConfig, list: MetaList) -> Result<()> {
    let Some(ident) = list.path.get_ident() else {
        return Err(syn::Error::new_spanned(list.path, "expected identifier"));
    };
    match ident.to_string().as_str() {
        "plugin_capabilities" => config
            .plugin_capabilities
            .extend(parse_expr_list(list.tokens)?),
        other => {
            return Err(syn::Error::new_spanned(
                ident,
                format!("unsupported plugin list '{other}'"),
            ));
        }
    }
    Ok(())
}

fn expr_as_type(expr: Expr) -> Result<Type> {
    match expr {
        Expr::Path(path) => {
            let path = path.path;
            Ok(parse_quote!(#path))
        }
        other => Err(syn::Error::new_spanned(
            other,
            "expected a type path, such as `MyConfig`",
        )),
    }
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

#[derive(Default)]
struct PluginInherentMethodAttrs {
    tools: Vec<PluginToolBinding>,
    stream_tools: Vec<PluginStreamToolBinding>,
    permission_paths: Vec<PluginPermissionBinding>,
    permission_networks: Vec<PluginPermissionBinding>,
    hooks: Vec<PluginHookBinding>,
}

fn parse_plugin_inherent_method_attrs(
    method: &mut ImplItemFn,
) -> Result<PluginInherentMethodAttrs> {
    let mut out = PluginInherentMethodAttrs::default();
    let mut kept_attrs = Vec::new();
    let method_ident = method.sig.ident.clone();
    let is_async = method.sig.asyncness.is_some();
    let attrs = std::mem::take(&mut method.attrs);
    for attr in attrs {
        if attr.path().is_ident("tool") {
            ensure_plugin_method_shared_receiver(method, "#[tool] methods")?;
            let (ty, context) = plugin_method_tool_input_type_and_context(&attr, method)?;
            out.tools.push(PluginToolBinding {
                method: method_ident.clone(),
                ty,
                kind: PluginToolBindingKind::Surface,
                is_async,
                context,
            });
        } else if attr.path().is_ident("tool_suite") {
            ensure_plugin_method_shared_receiver(method, "#[tool_suite] methods")?;
            let (ty, context) = plugin_method_tool_input_type_and_context(&attr, method)?;
            out.tools.push(PluginToolBinding {
                method: method_ident.clone(),
                ty,
                kind: PluginToolBindingKind::Suite,
                is_async,
                context,
            });
        } else if attr.path().is_ident("tool_stream") {
            ensure_plugin_method_shared_receiver(method, "#[tool_stream] methods")?;
            let sink_first = stream_sink_is_first_arg(method)?;
            out.stream_tools.push(PluginStreamToolBinding {
                method: method_ident.clone(),
                ty: plugin_method_stream_input_type(&attr, method, sink_first)?,
                is_async,
                sink_first,
                kind: PluginToolBindingKind::Surface,
                for_method: None,
            });
        } else if attr.path().is_ident("tool_suite_stream") {
            ensure_plugin_method_shared_receiver(method, "#[tool_suite_stream] methods")?;
            let sink_first = stream_sink_is_first_arg(method)?;
            out.stream_tools.push(PluginStreamToolBinding {
                method: method_ident.clone(),
                ty: plugin_method_stream_input_type(&attr, method, sink_first)?,
                is_async,
                sink_first,
                kind: PluginToolBindingKind::Suite,
                for_method: None,
            });
        } else if attr.path().is_ident("stream") {
            ensure_plugin_method_shared_receiver(method, "#[stream] methods")?;
            let sink_first = stream_sink_is_first_arg(method)?;
            out.stream_tools.push(PluginStreamToolBinding {
                method: method_ident.clone(),
                ty: plugin_method_inferred_stream_input_type(method, sink_first)?,
                is_async,
                sink_first,
                kind: PluginToolBindingKind::Surface,
                for_method: parse_plugin_stream_attr(&attr)?,
            });
        } else if attr.path().is_ident("permission_paths") {
            ensure_plugin_method_shared_receiver(method, "#[permission_paths] methods")?;
            ensure_plugin_method_typed_arg_count(method, 1, "#[permission_paths] methods")?;
            out.permission_paths.push(PluginPermissionBinding {
                method: method_ident.clone(),
                ty: plugin_method_single_input_type(&attr, method)?,
                kind: PluginToolBindingKind::Surface,
                is_async,
            });
        } else if attr.path().is_ident("permission_paths_suite") {
            ensure_plugin_method_shared_receiver(method, "#[permission_paths_suite] methods")?;
            ensure_plugin_method_typed_arg_count(method, 1, "#[permission_paths_suite] methods")?;
            out.permission_paths.push(PluginPermissionBinding {
                method: method_ident.clone(),
                ty: plugin_method_single_input_type(&attr, method)?,
                kind: PluginToolBindingKind::Suite,
                is_async,
            });
        } else if attr.path().is_ident("permission_networks") {
            ensure_plugin_method_shared_receiver(method, "#[permission_networks] methods")?;
            ensure_plugin_method_typed_arg_count(method, 1, "#[permission_networks] methods")?;
            out.permission_networks.push(PluginPermissionBinding {
                method: method_ident.clone(),
                ty: plugin_method_single_input_type(&attr, method)?,
                kind: PluginToolBindingKind::Surface,
                is_async,
            });
        } else if attr.path().is_ident("permission_networks_suite") {
            ensure_plugin_method_shared_receiver(method, "#[permission_networks_suite] methods")?;
            ensure_plugin_method_typed_arg_count(
                method,
                1,
                "#[permission_networks_suite] methods",
            )?;
            out.permission_networks.push(PluginPermissionBinding {
                method: method_ident.clone(),
                ty: plugin_method_single_input_type(&attr, method)?,
                kind: PluginToolBindingKind::Suite,
                is_async,
            });
        } else if attr.path().is_ident("permission") {
            ensure_plugin_method_shared_receiver(method, "#[permission] methods")?;
            ensure_plugin_method_typed_arg_count(method, 1, "#[permission] methods")?;
            let permission = parse_plugin_permission_attr(&attr)?;
            let binding = PluginPermissionBinding {
                method: method_ident.clone(),
                ty: plugin_method_inferred_single_input_type(method)?,
                kind: permission.kind,
                is_async,
            };
            match permission.target {
                PluginPermissionTarget::Paths => out.permission_paths.push(binding),
                PluginPermissionTarget::Networks => out.permission_networks.push(binding),
            }
        } else if attr.path().is_ident("hook") {
            ensure_plugin_method_shared_receiver(method, "#[hook] methods")?;
            let hook = parse_plugin_hook_attr(&attr, &method_ident)?;
            ensure_plugin_method_typed_arg_count(
                method,
                plugin_hook_arg_count(hook),
                &format!("#[hook({})] methods", plugin_hook_name(hook)),
            )?;
            out.hooks.push(PluginHookBinding {
                method: method_ident.clone(),
                hook,
                is_async,
            });
        } else {
            kept_attrs.push(attr);
        }
    }
    method.attrs = kept_attrs;
    Ok(out)
}

fn plugin_attr_has_explicit_args(attr: &Attribute) -> bool {
    match &attr.meta {
        Meta::Path(_) => false,
        Meta::List(list) => !list.tokens.is_empty(),
        Meta::NameValue(_) => true,
    }
}

fn plugin_method_single_input_type(attr: &Attribute, method: &ImplItemFn) -> Result<Type> {
    if plugin_attr_has_explicit_args(attr) {
        return attr.parse_args::<Type>();
    }
    plugin_method_inferred_single_input_type(method)
}

fn plugin_method_tool_input_type_and_context(
    attr: &Attribute,
    method: &ImplItemFn,
) -> Result<(Type, Option<PluginContextArg>)> {
    let args = typed_arg_types(method);
    match args.len() {
        1 => Ok((plugin_method_single_input_type(attr, method)?, None)),
        2 => {
            let first_context = type_is_tool_invoke_context(&args[0]);
            let second_context = type_is_tool_invoke_context(&args[1]);
            match (first_context, second_context) {
                (true, false) => {
                    let ty = if plugin_attr_has_explicit_args(attr) {
                        attr.parse_args::<Type>()?
                    } else {
                        args[1].clone()
                    };
                    Ok((
                        ty,
                        Some(PluginContextArg {
                            first: true,
                            by_ref: type_is_reference(&args[0]),
                        }),
                    ))
                }
                (false, true) => {
                    let ty = if plugin_attr_has_explicit_args(attr) {
                        attr.parse_args::<Type>()?
                    } else {
                        args[0].clone()
                    };
                    Ok((
                        ty,
                        Some(PluginContextArg {
                            first: false,
                            by_ref: type_is_reference(&args[1]),
                        }),
                    ))
                }
                _ => Err(syn::Error::new_spanned(
                    &method.sig,
                    "#[tool] methods must take input, or input plus ToolInvokeContext",
                )),
            }
        }
        _ => Err(syn::Error::new_spanned(
            &method.sig,
            "#[tool] methods must take input, or input plus ToolInvokeContext",
        )),
    }
}

fn plugin_method_inferred_single_input_type(method: &ImplItemFn) -> Result<Type> {
    typed_arg_types(method).into_iter().next().ok_or_else(|| {
        syn::Error::new_spanned(
            &method.sig,
            "cannot infer plugin method input type without a typed argument",
        )
    })
}

fn plugin_method_stream_input_type(
    attr: &Attribute,
    method: &ImplItemFn,
    sink_first: bool,
) -> Result<Type> {
    if plugin_attr_has_explicit_args(attr) {
        return attr.parse_args::<Type>();
    }
    plugin_method_inferred_stream_input_type(method, sink_first)
}

fn plugin_method_inferred_stream_input_type(method: &ImplItemFn, sink_first: bool) -> Result<Type> {
    let args = typed_arg_types(method);
    let index = usize::from(sink_first);
    args.get(index).cloned().ok_or_else(|| {
        syn::Error::new_spanned(
            &method.sig,
            "cannot infer plugin stream input type without an input argument",
        )
    })
}

fn parse_plugin_stream_attr(attr: &Attribute) -> Result<Option<Ident>> {
    if !plugin_attr_has_explicit_args(attr) {
        return Ok(None);
    }
    let parsed = attr.parse_args::<PluginStreamAttr>()?;
    Ok(parsed.for_method)
}

struct PluginStreamAttr {
    for_method: Option<Ident>,
}

impl Parse for PluginStreamAttr {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let mut for_method = None;
        while !input.is_empty() {
            let key = input.call(Ident::parse_any)?;
            match key.to_string().as_str() {
                "for" | "tool" => {
                    let _: Token![=] = input.parse()?;
                    let value = input.call(Ident::parse_any)?;
                    if for_method.replace(value).is_some() {
                        return Err(syn::Error::new_spanned(
                            key,
                            "duplicate stream target option",
                        ));
                    }
                }
                other => {
                    return Err(syn::Error::new_spanned(
                        key,
                        format!("unsupported stream option '{other}'; expected `for = method`"),
                    ));
                }
            }
            if input.peek(Token![,]) {
                let _: Token![,] = input.parse()?;
            }
        }
        Ok(Self { for_method })
    }
}

fn parse_plugin_permission_attr(attr: &Attribute) -> Result<PluginPermissionAttr> {
    attr.parse_args::<PluginPermissionAttr>()
}

impl Parse for PluginPermissionAttr {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let mut target = None;
        let mut kind = PluginToolBindingKind::Surface;
        while !input.is_empty() {
            let ident = input.call(Ident::parse_any)?;
            match ident.to_string().as_str() {
                "paths" => {
                    if target.replace(PluginPermissionTarget::Paths).is_some() {
                        return Err(syn::Error::new_spanned(
                            ident,
                            "duplicate permission target",
                        ));
                    }
                }
                "networks" => {
                    if target.replace(PluginPermissionTarget::Networks).is_some() {
                        return Err(syn::Error::new_spanned(
                            ident,
                            "duplicate permission target",
                        ));
                    }
                }
                "surface" => kind = PluginToolBindingKind::Surface,
                "suite" => kind = PluginToolBindingKind::Suite,
                other => {
                    return Err(syn::Error::new_spanned(
                        ident,
                        format!(
                            "unsupported permission option '{other}'; expected paths, networks, surface, or suite"
                        ),
                    ));
                }
            }
            if input.peek(Token![,]) {
                let _: Token![,] = input.parse()?;
            }
        }
        let target = target.ok_or_else(|| {
            syn::Error::new(
                proc_macro2::Span::call_site(),
                "#[permission(...)] requires `paths` or `networks`",
            )
        })?;
        Ok(Self { target, kind })
    }
}

fn ensure_plugin_method_shared_receiver(method: &ImplItemFn, label: &str) -> Result<()> {
    match method.sig.inputs.first() {
        Some(FnArg::Receiver(receiver))
            if receiver.reference.is_some() && receiver.mutability.is_none() =>
        {
            Ok(())
        }
        _ => Err(syn::Error::new_spanned(
            &method.sig,
            format!("{label} must be inherent methods with `&self` receiver"),
        )),
    }
}

fn ensure_plugin_method_typed_arg_count(
    method: &ImplItemFn,
    expected: usize,
    label: &str,
) -> Result<()> {
    let count = typed_arg_types(method).len();
    if count != expected {
        return Err(syn::Error::new_spanned(
            &method.sig,
            format!("{label} must take exactly {expected} typed argument(s) after `&self`"),
        ));
    }
    Ok(())
}

fn stream_sink_is_first_arg(method: &ImplItemFn) -> Result<bool> {
    let args = typed_arg_types(method);
    if args.len() != 2 {
        return Err(syn::Error::new_spanned(
            &method.sig,
            "#[tool_stream] methods must take exactly two typed arguments after `&self`: input and ToolStreamSink",
        ));
    }
    let first_is_sink = type_last_segment_is(&args[0], "ToolStreamSink");
    let second_is_sink = type_last_segment_is(&args[1], "ToolStreamSink");
    match (first_is_sink, second_is_sink) {
        (true, false) => Ok(true),
        (false, true) => Ok(false),
        _ => Err(syn::Error::new_spanned(
            &method.sig,
            "#[tool_stream] methods must include exactly one ToolStreamSink argument",
        )),
    }
}

fn typed_arg_types(method: &ImplItemFn) -> Vec<Type> {
    method
        .sig
        .inputs
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

fn type_is_tool_invoke_context(ty: &Type) -> bool {
    type_last_segment_is(ty, "ToolInvokeContext")
}

fn type_is_reference(ty: &Type) -> bool {
    matches!(ty, Type::Reference(_))
}

fn parse_plugin_hook_attr(attr: &Attribute, method_ident: &Ident) -> Result<PluginHookKind> {
    if plugin_attr_has_explicit_args(attr) {
        let ident = attr.parse_args::<Ident>()?;
        plugin_hook_kind_from_ident(&ident)
    } else {
        plugin_hook_kind_from_ident(method_ident)
    }
}

fn plugin_hook_kind_from_ident(ident: &Ident) -> Result<PluginHookKind> {
    match ident.to_string().as_str() {
        "init" => Ok(PluginHookKind::Init),
        "shutdown" => Ok(PluginHookKind::Shutdown),
        "tool_execute_before" => Ok(PluginHookKind::ToolExecuteBefore),
        "tool_execute_after" => Ok(PluginHookKind::ToolExecuteAfter),
        "tool_execute_failure" => Ok(PluginHookKind::ToolExecuteFailure),
        "tool_definition" => Ok(PluginHookKind::ToolDefinition),
        "chat_message" => Ok(PluginHookKind::ChatMessage),
        "chat_params" => Ok(PluginHookKind::ChatParams),
        "chat_headers" => Ok(PluginHookKind::ChatHeaders),
        "chat_system_transform" => Ok(PluginHookKind::ChatSystemTransform),
        "chat_messages_transform" => Ok(PluginHookKind::ChatMessagesTransform),
        "event" => Ok(PluginHookKind::Event),
        "auth" => Ok(PluginHookKind::Auth),
        "provider_list" => Ok(PluginHookKind::ProviderList),
        "permission_ask" => Ok(PluginHookKind::PermissionAsk),
        "notification" => Ok(PluginHookKind::Notification),
        "command_execute_before" => Ok(PluginHookKind::CommandExecuteBefore),
        "command_execute_after" => Ok(PluginHookKind::CommandExecuteAfter),
        "shell_env" => Ok(PluginHookKind::ShellEnv),
        "pre_run" => Ok(PluginHookKind::PreRun),
        "post_run" => Ok(PluginHookKind::PostRun),
        "session_start" => Ok(PluginHookKind::SessionStart),
        "session_end" => Ok(PluginHookKind::SessionEnd),
        "user_prompt_submit" => Ok(PluginHookKind::UserPromptSubmit),
        "agent_stop" => Ok(PluginHookKind::AgentStop),
        "config_resolved" => Ok(PluginHookKind::ConfigResolved),
        other => Err(syn::Error::new_spanned(
            ident,
            format!("unsupported plugin hook '{other}'"),
        )),
    }
}

fn plugin_hook_arg_count(hook: PluginHookKind) -> usize {
    match hook {
        PluginHookKind::Shutdown => 0,
        PluginHookKind::Init => 2,
        PluginHookKind::ToolExecuteBefore
        | PluginHookKind::ToolExecuteAfter
        | PluginHookKind::ToolExecuteFailure
        | PluginHookKind::ToolDefinition
        | PluginHookKind::ChatMessage
        | PluginHookKind::ChatParams
        | PluginHookKind::ChatHeaders
        | PluginHookKind::ChatSystemTransform
        | PluginHookKind::ChatMessagesTransform
        | PluginHookKind::Event
        | PluginHookKind::Auth
        | PluginHookKind::ProviderList
        | PluginHookKind::PermissionAsk
        | PluginHookKind::Notification
        | PluginHookKind::CommandExecuteBefore
        | PluginHookKind::CommandExecuteAfter
        | PluginHookKind::ShellEnv
        | PluginHookKind::PreRun
        | PluginHookKind::PostRun
        | PluginHookKind::SessionStart
        | PluginHookKind::SessionEnd
        | PluginHookKind::UserPromptSubmit
        | PluginHookKind::AgentStop
        | PluginHookKind::ConfigResolved => 1,
    }
}

fn plugin_hook_name(hook: PluginHookKind) -> &'static str {
    match hook {
        PluginHookKind::Init => "init",
        PluginHookKind::Shutdown => "shutdown",
        PluginHookKind::ToolExecuteBefore => "tool_execute_before",
        PluginHookKind::ToolExecuteAfter => "tool_execute_after",
        PluginHookKind::ToolExecuteFailure => "tool_execute_failure",
        PluginHookKind::ToolDefinition => "tool_definition",
        PluginHookKind::ChatMessage => "chat_message",
        PluginHookKind::ChatParams => "chat_params",
        PluginHookKind::ChatHeaders => "chat_headers",
        PluginHookKind::ChatSystemTransform => "chat_system_transform",
        PluginHookKind::ChatMessagesTransform => "chat_messages_transform",
        PluginHookKind::Event => "event",
        PluginHookKind::Auth => "auth",
        PluginHookKind::ProviderList => "provider_list",
        PluginHookKind::PermissionAsk => "permission_ask",
        PluginHookKind::Notification => "notification",
        PluginHookKind::CommandExecuteBefore => "command_execute_before",
        PluginHookKind::CommandExecuteAfter => "command_execute_after",
        PluginHookKind::ShellEnv => "shell_env",
        PluginHookKind::PreRun => "pre_run",
        PluginHookKind::PostRun => "post_run",
        PluginHookKind::SessionStart => "session_start",
        PluginHookKind::SessionEnd => "session_end",
        PluginHookKind::UserPromptSubmit => "user_prompt_submit",
        PluginHookKind::AgentStop => "agent_stop",
        PluginHookKind::ConfigResolved => "config_resolved",
    }
}

fn reject_duplicate_hook_bindings(hooks: &[PluginHookBinding]) -> Result<()> {
    for (index, hook) in hooks.iter().enumerate() {
        if hooks
            .iter()
            .skip(index + 1)
            .any(|other| other.hook == hook.hook)
        {
            return Err(syn::Error::new_spanned(
                &hook.method,
                "duplicate #[hook] binding for the same plugin hook",
            ));
        }
    }
    Ok(())
}

fn resolve_plugin_stream_bindings(
    tools: &[PluginToolBinding],
    streams: &mut [PluginStreamToolBinding],
) -> Result<()> {
    for stream in streams {
        let Some(for_method) = stream.for_method.as_ref() else {
            continue;
        };
        let Some(tool) = tools.iter().find(|tool| &tool.method == for_method) else {
            return Err(syn::Error::new_spanned(
                for_method,
                "stream target must reference a #[tool] or #[tool_suite] method in the same #[plugin] impl",
            ));
        };
        let tool_ty = &tool.ty;
        let stream_ty = &stream.ty;
        if quote!(#tool_ty).to_string() != quote!(#stream_ty).to_string() {
            return Err(syn::Error::new_spanned(
                &stream.method,
                "stream input type must match the referenced tool method input type",
            ));
        }
        stream.kind = tool.kind;
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
    docs: Option<&str>,
    tools: &[PluginToolBinding],
    streams: &[PluginStreamToolBinding],
    permission_paths: &[PluginPermissionBinding],
    permission_networks: &[PluginPermissionBinding],
    hooks: &[PluginHookBinding],
) -> Result<proc_macro2::TokenStream> {
    let id = config.id.as_ref().expect("plugin id validated");
    let version = config.version.as_ref().expect("plugin version validated");
    let description = if let Some(description) = config.description.as_ref() {
        quote! { #description }
    } else if let Some(description) = lit_str_from_text(docs) {
        quote! { #description }
    } else {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            "#[plugin(...)] requires `description = ...` or doc comments on the impl block",
        ));
    };
    let hooks_expr = plugin_layer_hooks_expr(
        config.explicit_hooks.as_ref(),
        tools,
        streams,
        permission_paths,
        permission_networks,
        hooks,
    );

    let config_schema_chain = expand_plugin_layer_config_schema_chain(
        config.config_schema_type.as_ref(),
        config,
        self_ty,
    )?;
    let config_schema_value_chain = config
        .config_schema
        .as_ref()
        .map(|schema| quote! { builder = builder.config_schema(#schema); })
        .unwrap_or_default();
    let display_chain = config
        .display
        .as_ref()
        .map(|display| {
            match display.to_string().as_str() {
                "brief" | "compact" => {
                    quote! { builder = builder.display(::agena_plugin_sdk::ToolDisplayPreset::Compact); }
                }
                "brief_detailed" => {
                    quote! { builder = builder.display(::agena_plugin_sdk::ToolDisplayPreset::BriefDetailed); }
                }
                "detailed" => {
                    quote! { builder = builder.display(::agena_plugin_sdk::ToolDisplayPreset::Detailed); }
                }
                _ => quote! { compile_error!("unsupported plugin display mode"); },
            }
        })
        .unwrap_or_default();
    let ui_display_chain = config
        .ui_display
        .as_ref()
        .map(|display| {
            match display.to_string().as_str() {
                "brief" | "summary" => {
                    quote! { builder = builder.ui_display_mode(::agena_plugin_sdk::UiTextDisplayMode::Summary); }
                }
                "detailed" => {
                    quote! { builder = builder.ui_display_mode(::agena_plugin_sdk::UiTextDisplayMode::Detailed); }
                }
                _ => quote! { compile_error!("unsupported plugin UI display mode"); },
            }
        })
        .unwrap_or_default();
    let summary_chain = if let Some(summary) = config.summary.as_ref() {
        quote! { builder = builder.summary(#summary); }
    } else if let Some(summary) = lit_str_from_text(doc_summary(docs).as_deref()) {
        quote! { builder = builder.summary(#summary); }
    } else {
        quote! {}
    };
    let help_chain = if let Some(help) = config.help.as_ref() {
        quote! { builder = builder.help(#help); }
    } else if let Some(help) = lit_str_from_text(docs) {
        quote! { builder = builder.help(#help); }
    } else {
        quote! {}
    };
    let tool_description_mode_chain = config
        .tool_description_mode
        .as_ref()
        .map(|mode| quote! { builder = builder.tool_description_mode(#mode); })
        .unwrap_or_default();
    let ui_display_mode_chain = config
        .ui_display_mode
        .as_ref()
        .map(|mode| quote! { builder = builder.ui_display_mode(#mode); })
        .unwrap_or_default();
    let commands_chain = config
        .commands
        .as_ref()
        .map(|commands| quote! { builder = builder.commands(#commands); })
        .unwrap_or_default();
    let plugin_capabilities_expr_chain = config
        .plugin_capabilities_expr
        .as_ref()
        .map(|capabilities| quote! { builder = builder.plugin_capabilities(#capabilities); })
        .unwrap_or_default();
    let plugin_capability_chains = config
        .plugin_capabilities
        .iter()
        .map(|capability| quote! { builder = builder.plugin_capability(#capability); })
        .collect::<Vec<_>>();
    let (surface_types, suite_types) = unique_manifest_tool_types(tools, streams);
    let surface_chains = surface_types
        .iter()
        .map(|ty| quote! { builder = builder.tool_surface::<#ty>(); })
        .collect::<Vec<_>>();
    let suite_chains = suite_types
        .iter()
        .map(|ty| quote! { builder = builder.tool_suite::<#ty>(); })
        .collect::<Vec<_>>();

    Ok(quote! {
        fn manifest(&self) -> ::agena_plugin_sdk::PluginManifest {
            let mut builder = ::agena_plugin_sdk::PluginManifest::builder_from_full_name(#id, #version)
                .description(#description)
                .hooks(#hooks_expr)
                .config_schema(::agena_plugin_sdk::macro_support::empty_config_schema());
            #config_schema_chain
            #config_schema_value_chain
            #display_chain
            #ui_display_chain
            #summary_chain
            #help_chain
            #tool_description_mode_chain
            #ui_display_mode_chain
            #commands_chain
            #plugin_capabilities_expr_chain
            #(#plugin_capability_chains)*
            #(#surface_chains)*
            #(#suite_chains)*
            builder.build()
        }
    })
}

fn expand_plugin_layer_config_schema_chain(
    config_schema_type: Option<&Type>,
    config: &PluginImplConfig,
    self_ty: &Type,
) -> Result<proc_macro2::TokenStream> {
    let Some(ty) = config_schema_type else {
        if config.config_schema_store {
            return Ok(quote! {
                builder = builder.config_schema(
                    <#self_ty as ::agena_plugin_sdk::plugin::PluginConfigStoreAccess>::plugin_config_schema(),
                );
            });
        }
        return Ok(quote! {});
    };
    let Some(default) = config.config_schema_default.as_ref() else {
        return Ok(quote! {
            builder = builder.config_schema(::agena_plugin_sdk::macro_support::json_schema_for::<#ty>());
        });
    };
    if expr_is_ident(default, "default") {
        Ok(quote! {
            builder = builder.config_schema(
                ::agena_plugin_sdk::macro_support::json_schema_for_with_default(
                    <#ty as ::core::default::Default>::default(),
                ),
            );
        })
    } else {
        Ok(quote! {
            builder = builder.config_schema(
                ::agena_plugin_sdk::macro_support::json_schema_for_with_default(#default),
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

fn unique_manifest_tool_types(
    tools: &[PluginToolBinding],
    streams: &[PluginStreamToolBinding],
) -> (Vec<Type>, Vec<Type>) {
    let mut surface_types = Vec::new();
    let mut surface_keys = Vec::new();
    let mut suite_types = Vec::new();
    let mut suite_keys = Vec::new();
    for binding in tools {
        match binding.kind {
            PluginToolBindingKind::Surface => {
                push_unique_type(&mut surface_types, &mut surface_keys, &binding.ty)
            }
            PluginToolBindingKind::Suite => {
                push_unique_type(&mut suite_types, &mut suite_keys, &binding.ty)
            }
        }
    }
    for binding in streams {
        match binding.kind {
            PluginToolBindingKind::Surface => {
                push_unique_type(&mut surface_types, &mut surface_keys, &binding.ty)
            }
            PluginToolBindingKind::Suite => {
                push_unique_type(&mut suite_types, &mut suite_keys, &binding.ty)
            }
        }
    }
    (surface_types, suite_types)
}

fn push_unique_type(types: &mut Vec<Type>, keys: &mut Vec<String>, ty: &Type) {
    let key = quote!(#ty).to_string();
    if !keys.contains(&key) {
        keys.push(key);
        types.push(ty.clone());
    }
}

fn plugin_layer_hooks_expr(
    explicit_hooks: Option<&Expr>,
    tools: &[PluginToolBinding],
    streams: &[PluginStreamToolBinding],
    _permission_paths: &[PluginPermissionBinding],
    _permission_networks: &[PluginPermissionBinding],
    hooks: &[PluginHookBinding],
) -> proc_macro2::TokenStream {
    let mut terms = Vec::new();
    if let Some(explicit) = explicit_hooks {
        terms.push(quote! { #explicit });
    }
    if !tools.is_empty() {
        terms.push(quote! { ::agena_plugin_sdk::HookSubscription::TOOL_INVOKE });
    }
    if !streams.is_empty() {
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
    bindings: &[PluginToolBinding],
) -> Result<proc_macro2::TokenStream> {
    let branches = bindings
        .iter()
        .map(expand_plugin_layer_tool_invoke_branch)
        .collect::<Vec<_>>();

    Ok(quote! {
        async fn tool_invoke(
            &self,
            input: ::agena_plugin_sdk::ToolInvokeInput,
        ) -> ::agena_plugin_sdk::Result<::agena_plugin_sdk::ToolInvokeOutput> {
            let __tool_name = input.tool_name.clone();
            let __context = input.context();
            #(#branches)*
            Err(::agena_plugin_sdk::PluginError::not_implemented(format!(
                "tool_invoke({})",
                __tool_name
            )))
        }
    })
}

fn expand_plugin_layer_tool_invoke_branch(binding: &PluginToolBinding) -> proc_macro2::TokenStream {
    let ty = &binding.ty;
    let call_args = plugin_layer_tool_call_args(binding.context);
    let call = plugin_layer_tool_method_call(&binding.method, binding.is_async, &call_args);
    match binding.kind {
        PluginToolBindingKind::Surface => quote! {
            {
                let __definitions = <#ty as ::agena_plugin_sdk::ToolSurface>::tool_definitions();
                if __definitions.iter().any(|__definition| {
                    __definition.name.as_str() == __tool_name.as_str()
                }) {
                    let __parsed = <#ty as ::agena_plugin_sdk::ToolSurface>::parse_tool(
                        __tool_name.as_str(),
                        input.input.clone(),
                    )?;
                    return #call;
                }
            }
        },
        PluginToolBindingKind::Suite => quote! {
            {
                let __definitions = <#ty as ::agena_plugin_sdk::ToolSuiteSurface>::tool_definitions();
                if __definitions.iter().any(|__definition| {
                    __definition.name.as_str() == __tool_name.as_str()
                }) {
                    let __parsed = <#ty as ::agena_plugin_sdk::ToolSuiteSurface>::parse_tool(
                        __tool_name.as_str(),
                        input.input.clone(),
                    )?;
                    return #call;
                }
            }
        },
    }
}

fn expand_plugin_layer_tool_stream(
    _self_ty: &Type,
    bindings: &[PluginStreamToolBinding],
) -> Result<proc_macro2::TokenStream> {
    let branches = bindings
        .iter()
        .map(expand_plugin_layer_tool_stream_branch)
        .collect::<Vec<_>>();

    Ok(quote! {
        async fn tool_invoke_stream(
            &self,
            input: ::agena_plugin_sdk::ToolInvokeInput,
            sink: ::agena_plugin_sdk::ToolStreamSink,
        ) -> ::agena_plugin_sdk::Result<::agena_plugin_sdk::ToolStreamEnd> {
            let __tool_name = input.tool_name.clone();
            #(#branches)*

            let __stream_id = sink.stream_id().to_string();
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
    binding: &PluginStreamToolBinding,
) -> proc_macro2::TokenStream {
    let ty = &binding.ty;
    let args = if binding.sink_first {
        vec![quote! { sink }, quote! { __parsed }]
    } else {
        vec![quote! { __parsed }, quote! { sink }]
    };
    let call = plugin_layer_stream_method_call(&binding.method, binding.is_async, &args);
    match binding.kind {
        PluginToolBindingKind::Surface => quote! {
            {
                let __definitions = <#ty as ::agena_plugin_sdk::ToolSurface>::tool_definitions();
                if __definitions.iter().any(|__definition| {
                    __definition.name.as_str() == __tool_name.as_str()
                }) {
                    let __parsed = <#ty as ::agena_plugin_sdk::ToolSurface>::parse_tool(
                        __tool_name.as_str(),
                        input.input,
                    )?;
                    return #call;
                }
            }
        },
        PluginToolBindingKind::Suite => quote! {
            {
                let __definitions = <#ty as ::agena_plugin_sdk::ToolSuiteSurface>::tool_definitions();
                if __definitions.iter().any(|__definition| {
                    __definition.name.as_str() == __tool_name.as_str()
                }) {
                    let __parsed = <#ty as ::agena_plugin_sdk::ToolSuiteSurface>::parse_tool(
                        __tool_name.as_str(),
                        input.input,
                    )?;
                    return #call;
                }
            }
        },
    }
}

fn expand_plugin_layer_permission_paths(
    _self_ty: &Type,
    bindings: &[PluginPermissionBinding],
) -> Result<proc_macro2::TokenStream> {
    let branches = bindings
        .iter()
        .map(|binding| expand_plugin_layer_permission_branch(binding, true))
        .collect::<Vec<_>>();

    Ok(quote! {
        async fn permission_paths(
            &self,
            tool: &str,
            input: &::agena_plugin_sdk::serde_json::Value,
        ) -> ::agena_plugin_sdk::Result<Vec<::agena_plugin_sdk::PathRequest>> {
            #(#branches)*
            Ok(Vec::new())
        }
    })
}

fn expand_plugin_layer_permission_networks(
    _self_ty: &Type,
    bindings: &[PluginPermissionBinding],
) -> Result<proc_macro2::TokenStream> {
    let branches = bindings
        .iter()
        .map(|binding| expand_plugin_layer_permission_branch(binding, false))
        .collect::<Vec<_>>();

    Ok(quote! {
        async fn permission_networks(
            &self,
            tool: &str,
            input: &::agena_plugin_sdk::serde_json::Value,
        ) -> ::agena_plugin_sdk::Result<Vec<::agena_plugin_sdk::NetworkRequest>> {
            #(#branches)*
            Ok(Vec::new())
        }
    })
}

fn expand_plugin_layer_permission_branch(
    binding: &PluginPermissionBinding,
    paths: bool,
) -> proc_macro2::TokenStream {
    let ty = &binding.ty;
    let call = plugin_layer_permission_method_call(
        &binding.method,
        binding.is_async,
        &[quote! { __parsed }],
        paths,
    );
    match binding.kind {
        PluginToolBindingKind::Surface => quote! {
            {
                let __definitions = <#ty as ::agena_plugin_sdk::ToolSurface>::tool_definitions();
                if __definitions.iter().any(|__definition| {
                    __definition.name.as_str() == tool
                }) {
                    let __parsed = <#ty as ::agena_plugin_sdk::ToolSurface>::parse_tool(
                        tool,
                        input.clone(),
                    )?;
                    return #call;
                }
            }
        },
        PluginToolBindingKind::Suite => quote! {
            {
                let __definitions = <#ty as ::agena_plugin_sdk::ToolSuiteSurface>::tool_definitions();
                if __definitions.iter().any(|__definition| {
                    __definition.name.as_str() == tool
                }) {
                    let __parsed = <#ty as ::agena_plugin_sdk::ToolSuiteSurface>::parse_tool(
                        tool,
                        input.clone(),
                    )?;
                    return #call;
                }
            }
        },
    }
}

fn expand_plugin_layer_init_method(
    config: &PluginImplConfig,
    self_ty: &Type,
    binding: Option<&PluginHookBinding>,
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
    config
        .id
        .as_ref()
        .and_then(|expr| match expr {
            Expr::Lit(ExprLit {
                lit: Lit::Str(value),
                ..
            }) => Some(value.value()),
            _ => None,
        })
        .unwrap_or_else(|| "plugin".to_string())
}

fn expand_plugin_layer_hook_method(
    _self_ty: &Type,
    binding: &PluginHookBinding,
) -> Result<proc_macro2::TokenStream> {
    let method = &binding.method;
    let is_async = binding.is_async;
    let tokens = match binding.hook {
        PluginHookKind::Init => {
            let call =
                plugin_layer_method_call(method, is_async, &[quote! { ctx }, quote! { host }]);
            quote! {
                async fn init(
                    &self,
                    ctx: ::agena_plugin_sdk::InitContext,
                    host: ::std::sync::Arc<dyn ::agena_plugin_sdk::HostClient>,
                ) -> ::agena_plugin_sdk::Result<::agena_plugin_sdk::InitOutcome> {
                    #call
                }
            }
        }
        PluginHookKind::Shutdown => {
            let call = plugin_layer_method_call(method, is_async, &[]);
            quote! {
                async fn shutdown(&self) -> ::agena_plugin_sdk::Result<()> {
                    ::agena_plugin_sdk::IntoHookOutput::<()>::into_hook_output(#call)
                }
            }
        }
        PluginHookKind::ToolExecuteBefore => expand_plugin_layer_single_arg_hook(
            "tool_execute_before",
            quote! { ::agena_plugin_sdk::ToolBeforeInput },
            quote! { Option<::agena_plugin_sdk::ToolBeforePatch> },
            method,
            is_async,
        ),
        PluginHookKind::ToolExecuteAfter => expand_plugin_layer_single_arg_hook(
            "tool_execute_after",
            quote! { ::agena_plugin_sdk::ToolAfterInput },
            quote! { Option<::agena_plugin_sdk::ToolAfterPatch> },
            method,
            is_async,
        ),
        PluginHookKind::ToolExecuteFailure => expand_plugin_layer_single_arg_hook(
            "tool_execute_failure",
            quote! { ::agena_plugin_sdk::ToolFailureInput },
            quote! { () },
            method,
            is_async,
        ),
        PluginHookKind::ToolDefinition => expand_plugin_layer_single_arg_hook(
            "tool_definition",
            quote! { ::agena_plugin_sdk::ToolDefinitionInput },
            quote! { Option<::agena_plugin_sdk::ToolDefinitionPatch> },
            method,
            is_async,
        ),
        PluginHookKind::ChatMessage => expand_plugin_layer_single_arg_hook(
            "chat_message",
            quote! { ::agena_plugin_sdk::ChatMessageInput },
            quote! { Option<::agena_plugin_sdk::ChatMessagePatch> },
            method,
            is_async,
        ),
        PluginHookKind::ChatParams => expand_plugin_layer_single_arg_hook(
            "chat_params",
            quote! { ::agena_plugin_sdk::ChatParamsInput },
            quote! { Option<::agena_plugin_sdk::ChatParamsPatch> },
            method,
            is_async,
        ),
        PluginHookKind::ChatHeaders => expand_plugin_layer_single_arg_hook(
            "chat_headers",
            quote! { ::agena_plugin_sdk::ChatHeadersInput },
            quote! { Option<::agena_plugin_sdk::ChatHeadersPatch> },
            method,
            is_async,
        ),
        PluginHookKind::ChatSystemTransform => expand_plugin_layer_single_arg_hook(
            "chat_system_transform",
            quote! { ::agena_plugin_sdk::ChatSystemTransformInput },
            quote! { Option<::agena_plugin_sdk::ChatSystemTransformPatch> },
            method,
            is_async,
        ),
        PluginHookKind::ChatMessagesTransform => expand_plugin_layer_single_arg_hook(
            "chat_messages_transform",
            quote! { ::agena_plugin_sdk::ChatMessagesTransformInput },
            quote! { Option<::agena_plugin_sdk::ChatMessagesTransformPatch> },
            method,
            is_async,
        ),
        PluginHookKind::Event => expand_plugin_layer_single_arg_hook(
            "event",
            quote! { ::agena_plugin_sdk::EventEnvelope },
            quote! { () },
            method,
            is_async,
        ),
        PluginHookKind::Auth => expand_plugin_layer_single_arg_hook(
            "auth",
            quote! { ::agena_plugin_sdk::AuthInput },
            quote! { Option<::agena_plugin_sdk::AuthOutput> },
            method,
            is_async,
        ),
        PluginHookKind::ProviderList => expand_plugin_layer_single_arg_hook(
            "provider_list",
            quote! { ::agena_plugin_sdk::ProviderListInput },
            quote! { Option<::agena_plugin_sdk::ProviderListPatch> },
            method,
            is_async,
        ),
        PluginHookKind::PermissionAsk => expand_plugin_layer_single_arg_hook(
            "permission_ask",
            quote! { ::agena_plugin_sdk::PermissionAskInput },
            quote! { Option<::agena_plugin_sdk::PermissionAskDecision> },
            method,
            is_async,
        ),
        PluginHookKind::Notification => expand_plugin_layer_single_arg_hook(
            "notification",
            quote! { ::agena_plugin_sdk::NotificationInput },
            quote! { () },
            method,
            is_async,
        ),
        PluginHookKind::CommandExecuteBefore => expand_plugin_layer_single_arg_hook(
            "command_execute_before",
            quote! { ::agena_plugin_sdk::CommandBeforeInput },
            quote! { Option<::agena_plugin_sdk::CommandBeforeResponse> },
            method,
            is_async,
        ),
        PluginHookKind::CommandExecuteAfter => expand_plugin_layer_single_arg_hook(
            "command_execute_after",
            quote! { ::agena_plugin_sdk::CommandAfterInput },
            quote! { Option<::agena_plugin_sdk::CommandAfterPatch> },
            method,
            is_async,
        ),
        PluginHookKind::ShellEnv => expand_plugin_layer_single_arg_hook(
            "shell_env",
            quote! { ::agena_plugin_sdk::ShellEnvInput },
            quote! { Option<::agena_plugin_sdk::ShellEnvPatch> },
            method,
            is_async,
        ),
        PluginHookKind::PreRun => expand_plugin_layer_single_arg_hook(
            "pre_run",
            quote! { ::agena_plugin_sdk::PreRunInput },
            quote! { () },
            method,
            is_async,
        ),
        PluginHookKind::PostRun => expand_plugin_layer_single_arg_hook(
            "post_run",
            quote! { ::agena_plugin_sdk::PostRunInput },
            quote! { () },
            method,
            is_async,
        ),
        PluginHookKind::SessionStart => expand_plugin_layer_single_arg_hook(
            "session_start",
            quote! { ::agena_plugin_sdk::SessionStartInput },
            quote! { Option<::agena_plugin_sdk::SessionStartPatch> },
            method,
            is_async,
        ),
        PluginHookKind::SessionEnd => expand_plugin_layer_single_arg_hook(
            "session_end",
            quote! { ::agena_plugin_sdk::SessionEndInput },
            quote! { () },
            method,
            is_async,
        ),
        PluginHookKind::UserPromptSubmit => expand_plugin_layer_single_arg_hook(
            "user_prompt_submit",
            quote! { ::agena_plugin_sdk::UserPromptSubmitInput },
            quote! { Option<::agena_plugin_sdk::UserPromptSubmitPatch> },
            method,
            is_async,
        ),
        PluginHookKind::AgentStop => expand_plugin_layer_single_arg_hook(
            "agent_stop",
            quote! { ::agena_plugin_sdk::AgentStopInput },
            quote! { Option<::agena_plugin_sdk::AgentStopPatch> },
            method,
            is_async,
        ),
        PluginHookKind::ConfigResolved => expand_plugin_layer_single_arg_hook(
            "config_resolved",
            quote! { ::agena_plugin_sdk::ConfigInput },
            quote! { Option<::agena_plugin_sdk::ConfigPatch> },
            method,
            is_async,
        ),
    };
    Ok(tokens)
}

fn expand_plugin_layer_single_arg_hook(
    trait_method: &str,
    input_ty: proc_macro2::TokenStream,
    output_ty: proc_macro2::TokenStream,
    method: &Ident,
    is_async: bool,
) -> proc_macro2::TokenStream {
    let trait_method = format_ident!("{trait_method}");
    let call = plugin_layer_method_call(method, is_async, &[quote! { input }]);
    quote! {
        async fn #trait_method(
            &self,
            input: #input_ty,
        ) -> ::agena_plugin_sdk::Result<#output_ty> {
            ::agena_plugin_sdk::IntoHookOutput::<#output_ty>::into_hook_output(#call)
        }
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
) -> proc_macro2::TokenStream {
    let call = plugin_layer_method_call(method, is_async, args);
    quote! {
        ::agena_plugin_sdk::IntoToolInvokeOutput::into_tool_invoke_output(#call)
    }
}

fn plugin_layer_tool_call_args(context: Option<PluginContextArg>) -> Vec<proc_macro2::TokenStream> {
    let Some(context) = context else {
        return vec![quote! { __parsed }];
    };
    let context_arg = if context.by_ref {
        quote! { &__context }
    } else {
        quote! { __context }
    };
    if context.first {
        vec![context_arg, quote! { __parsed }]
    } else {
        vec![quote! { __parsed }, context_arg]
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

fn plugin_layer_permission_method_call(
    method: &Ident,
    is_async: bool,
    args: &[proc_macro2::TokenStream],
    paths: bool,
) -> proc_macro2::TokenStream {
    let call = plugin_layer_method_call(method, is_async, args);
    if paths {
        quote! { ::agena_plugin_sdk::IntoPathRequests::into_path_requests(#call) }
    } else {
        quote! { ::agena_plugin_sdk::IntoNetworkRequests::into_network_requests(#call) }
    }
}

#[derive(Clone, Default)]
struct SurfaceConfig {
    tool: Option<LitStr>,
    aliases: Vec<LitStr>,
    description: Option<LitStr>,
    before_help: Option<LitStr>,
    after_help: Option<LitStr>,
    summary: Option<LitStr>,
    help: Option<LitStr>,
    examples: Vec<LitStr>,
    handler_receiver: Option<Path>,
    handle: Option<Path>,
    handle_with_context: Option<Path>,
    stream_handle: Option<Path>,
    stream_handle_with_context: Option<Path>,
    permission_paths_handle: Option<Path>,
    permission_networks_handle: Option<Path>,
    handle_field: Option<Path>,
    handle_by_value: Option<bool>,
    normalize: Option<Path>,
    validate: Option<Path>,
    trim: Vec<LitStr>,
    trim_suffix: Vec<PathStringConstraint>,
    non_empty: Vec<LitStr>,
    non_empty_if_present: Vec<LitStr>,
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
    max_chars: Vec<PathUsizeConstraint>,
    display: Option<LitStr>,
    ui_display: Option<LitStr>,
    description_mode: Option<LitStr>,
    ui_display_mode: Option<LitStr>,
    tags: Vec<Expr>,
    capabilities: Vec<Expr>,
    concurrency_safe: Option<bool>,
    strict: Option<bool>,
    streaming: Option<LitStr>,
}

enum VariantMapping {
    Exec(LitStr),
    Map(Path),
}

struct ToolVariantConfig {
    mapping: VariantMapping,
    route: Option<LitStr>,
    route_action: Option<LitStr>,
    handle: Option<Path>,
    handle_with_context: Option<Path>,
    stream_handle: Option<Path>,
    stream_handle_with_context: Option<Path>,
    permission_paths_handle: Option<Path>,
    permission_networks_handle: Option<Path>,
    handle_by_value: bool,
    field: Option<Path>,
    convert: Option<Path>,
    shape: Option<Path>,
    validate: Option<Path>,
    non_empty: Vec<LitStr>,
    non_empty_if_present: Vec<LitStr>,
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
    max_chars: Vec<PathUsizeConstraint>,
    default_when_empty: bool,
    infer_when_present: Vec<LitStr>,
    drop_keys: Vec<LitStr>,
    action_aliases: Vec<LitStr>,
    action_alias_defaults: Vec<ActionAliasDefaultConfig>,
}

struct ActionAliasDefaultConfig {
    alias: LitStr,
    defaults: Vec<(LitStr, Expr)>,
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
struct PathStringConstraint {
    path: LitStr,
    value: LitStr,
}

trait SchemaConstraintSource {
    fn non_empty(&self) -> &[LitStr];
    fn non_empty_if_present(&self) -> &[LitStr];
    fn min_items(&self) -> &[PathUsizeConstraint];
    fn max_items(&self) -> &[PathUsizeConstraint];
    fn max_chars(&self) -> &[PathUsizeConstraint];
}

trait SchemaRelationSource {
    fn exactly_one_of(&self) -> &[Vec<LitStr>];
    fn at_least_one_of(&self) -> &[Vec<LitStr>];
    fn requires(&self) -> &[PathPairConstraint];
    fn conflicts_with(&self) -> &[PathPairConstraint];
    fn required_unless_present(&self) -> &[PathPairConstraint];
    fn distinct_trimmed_within(&self) -> &[PathPairConstraint];
}

impl SchemaConstraintSource for SurfaceConfig {
    fn non_empty(&self) -> &[LitStr] {
        &self.non_empty
    }

    fn non_empty_if_present(&self) -> &[LitStr] {
        &self.non_empty_if_present
    }

    fn min_items(&self) -> &[PathUsizeConstraint] {
        &self.min_items
    }

    fn max_items(&self) -> &[PathUsizeConstraint] {
        &self.max_items
    }

    fn max_chars(&self) -> &[PathUsizeConstraint] {
        &self.max_chars
    }
}

impl SchemaRelationSource for SurfaceConfig {
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

    fn distinct_trimmed_within(&self) -> &[PathPairConstraint] {
        &self.distinct_trimmed_within
    }
}

impl SchemaConstraintSource for ToolInputShapeConfig {
    fn non_empty(&self) -> &[LitStr] {
        &self.non_empty
    }

    fn non_empty_if_present(&self) -> &[LitStr] {
        &self.non_empty_if_present
    }

    fn min_items(&self) -> &[PathUsizeConstraint] {
        &self.min_items
    }

    fn max_items(&self) -> &[PathUsizeConstraint] {
        &self.max_items
    }

    fn max_chars(&self) -> &[PathUsizeConstraint] {
        &self.max_chars
    }
}

impl SchemaRelationSource for ToolInputShapeConfig {
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

    fn distinct_trimmed_within(&self) -> &[PathPairConstraint] {
        &self.distinct_trimmed_within
    }
}

impl SchemaConstraintSource for ToolVariantConfig {
    fn non_empty(&self) -> &[LitStr] {
        &self.non_empty
    }

    fn non_empty_if_present(&self) -> &[LitStr] {
        &self.non_empty_if_present
    }

    fn min_items(&self) -> &[PathUsizeConstraint] {
        &self.min_items
    }

    fn max_items(&self) -> &[PathUsizeConstraint] {
        &self.max_items
    }

    fn max_chars(&self) -> &[PathUsizeConstraint] {
        &self.max_chars
    }
}

impl SchemaRelationSource for ToolVariantConfig {
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

    fn min_items(&self) -> &[PathUsizeConstraint] {
        &self.min_items
    }

    fn max_items(&self) -> &[PathUsizeConstraint] {
        &self.max_items
    }

    fn max_chars(&self) -> &[PathUsizeConstraint] {
        &self.max_chars
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

    fn distinct_trimmed_within(&self) -> &[PathPairConstraint] {
        &self.distinct_trimmed_within
    }
}

fn expand_static_tool_surface(input: DeriveInput) -> Result<proc_macro2::TokenStream> {
    let name = input.ident;
    let docs = doc_text(&input.attrs);
    let surface = parse_surface_config(&input.attrs)?;
    let surface_for_dispatch = surface.clone();
    let schema_metadata_fn =
        expand_schema_metadata_fn(&input.data, &surface, |variant, prefix| {
            let config = parse_tool_variant_config(variant)?;
            let mut calls = constraint_schema_metadata_calls(prefix, &config)?;
            calls.extend(constraint_relation_metadata_calls(prefix, &config)?);
            Ok(calls)
        })?;
    let tool = surface.tool.ok_or_else(|| {
        syn::Error::new_spanned(
            &name,
            "missing #[tool_surface(tool = \"...\")] or #[tool_command(tool = \"...\")]",
        )
    })?;
    let description = surface
        .description
        .or_else(|| lit_str_from_text(docs.as_deref()))
        .ok_or_else(|| {
            syn::Error::new_spanned(
                &name,
                "missing #[tool_surface(description = \"...\")] / #[tool_command(description = \"...\")] or doc comments",
            )
        })?;
    let concurrency_safe = surface.concurrency_safe.unwrap_or(false);
    let strict = surface.strict.unwrap_or(false);
    let built_in_normalize_expr =
        built_in_normalization_tokens(quote! { &mut input }, &surface.trim, &surface.trim_suffix);
    let built_in_post_parse_normalize_expr =
        built_in_post_parse_normalization_tokens(&surface.trim, &surface.trim_suffix);
    let normalize_expr = surface
        .normalize
        .as_ref()
        .map(|path| quote! { #path(input)? })
        .unwrap_or_else(|| quote! { input });
    let validate_expr = surface
        .validate
        .as_ref()
        .map(|path| quote! { #path(&parsed)?; })
        .unwrap_or_default();
    let built_in_validate_expr = built_in_validation_tokens(
        quote! { parsed },
        &surface.non_empty,
        &surface.non_empty_if_present,
        &surface.exactly_one_of,
        &surface.at_least_one_of,
        &surface.requires,
        &surface.conflicts_with,
        &surface.required_unless_present,
        &surface.forbid_substrings,
        &surface.distinct_trimmed,
        &surface.distinct_trimmed_within,
        &surface.min_items,
        &surface.max_items,
        &surface.max_chars,
    );

    let summary_chain = surface
        .summary
        .or_else(|| lit_str_from_text(doc_summary(docs.as_deref()).as_deref()))
        .map(|value| quote! { .summary(#value) })
        .unwrap_or_default();
    let help_chain = surface
        .help
        .or_else(|| lit_str_from_text(docs.as_deref()))
        .map(|value| quote! { .help(#value) })
        .unwrap_or_default();
    let before_help_chain = surface
        .before_help
        .map(|value| quote! { .before_help(#value) })
        .unwrap_or_default();
    let after_help_chain = surface
        .after_help
        .map(|value| quote! { .after_help(#value) })
        .unwrap_or_default();
    let examples_chain = if surface.examples.is_empty() {
        quote! {}
    } else {
        let examples = surface.examples;
        quote! { .examples([#(#examples),*]) }
    };
    let aliases_chain = quote! {};
    let display_chain = match surface.display.as_ref().map(LitStr::value).as_deref() {
        Some("brief") => quote! { .display(::agena_plugin_sdk::ToolDisplayPreset::Compact) },
        Some("compact") => {
            quote! { .display(::agena_plugin_sdk::ToolDisplayPreset::Compact) }
        }
        Some("brief_detailed") => {
            quote! { .display(::agena_plugin_sdk::ToolDisplayPreset::BriefDetailed) }
        }
        Some("detailed") => {
            quote! { .display(::agena_plugin_sdk::ToolDisplayPreset::Detailed) }
        }
        Some(other) => {
            let invalid = surface
                .display
                .clone()
                .expect("display was matched as Some");
            return Err(syn::Error::new_spanned(
                invalid,
                format!("unsupported tool display preset '{other}'"),
            ));
        }
        None => quote! {},
    };
    let ui_display_chain = match surface.ui_display.as_ref().map(LitStr::value).as_deref() {
        Some("brief") => {
            quote! { .ui_display_mode(::agena_plugin_sdk::UiTextDisplayMode::Summary) }
        }
        Some("summary") => {
            quote! { .ui_display_mode(::agena_plugin_sdk::UiTextDisplayMode::Summary) }
        }
        Some("detailed") => {
            quote! { .ui_display_mode(::agena_plugin_sdk::UiTextDisplayMode::Detailed) }
        }
        Some(other) => {
            let invalid = surface
                .ui_display
                .clone()
                .expect("ui_display was matched as Some");
            return Err(syn::Error::new_spanned(
                invalid,
                format!("unsupported ui display mode '{other}'"),
            ));
        }
        None => quote! {},
    };
    let description_mode_chain = match surface
        .description_mode
        .as_ref()
        .map(LitStr::value)
        .as_deref()
    {
        Some("brief") => {
            quote! { .description_mode(::agena_plugin_sdk::ToolDescriptionMode::Brief) }
        }
        Some("detailed") => {
            quote! { .description_mode(::agena_plugin_sdk::ToolDescriptionMode::Detailed) }
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
            quote! { .ui_display_mode(::agena_plugin_sdk::UiTextDisplayMode::Summary) }
        }
        Some("detailed") => {
            quote! { .ui_display_mode(::agena_plugin_sdk::UiTextDisplayMode::Detailed) }
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
        let tags = surface.tags.clone();
        quote! { .tags([#(#tags),*]) }
    };
    let capabilities_chain = if surface.capabilities.is_empty() {
        quote! {}
    } else {
        let capabilities = surface.capabilities.clone();
        quote! { .capabilities([#(#capabilities),*]) }
    };
    let streaming_chain = match surface.streaming.as_ref().map(LitStr::value).as_deref() {
        Some("streaming") => {
            quote! { .streaming(::agena_plugin_sdk::ToolStreamingMode::Streaming) }
        }
        Some("buffered") | None => quote! {},
        Some(other) => {
            return Err(syn::Error::new_spanned(
                surface
                    .streaming
                    .clone()
                    .expect("streaming was matched as Some"),
                format!("unsupported tool streaming mode '{other}'"),
            ));
        }
    };
    let strict_chain = if strict {
        quote! { .strict(true) }
    } else {
        quote! {}
    };

    let tool_name_fn = quote! {
        pub(crate) fn tool_name() -> &'static str {
            #tool
        }
    };
    let input_schema_fn = quote! {
        pub(crate) fn input_schema() -> serde_json::Value {
            let mut schema = ::agena_plugin_sdk::macro_support::json_schema_for::<Self>();
            Self::__macro_apply_schema_metadata(&mut schema);
            schema
        }
    };

    let parse_json_str_fn = quote! {
        pub(crate) fn parse_json_str(
            input: &str,
        ) -> ::agena_plugin_sdk::Result<Self> {
            let input = ::agena_plugin_sdk::macro_support::parse_json_value_str(input)?;
            Self::parse_input(input)
        }
    };

    let make_tool_definition_fn = quote! {
        fn __macro_tool_definition(
            name: impl Into<String>,
            schema: serde_json::Value,
        ) -> ::agena_plugin_sdk::ToolDefinition {
            ::agena_plugin_sdk::ToolDefinition::new(
                name,
                schema,
            )
            .description(#description)
            #aliases_chain
            #before_help_chain
            #summary_chain
            #help_chain
            #after_help_chain
            #examples_chain
            #display_chain
            #ui_display_chain
            #description_mode_chain
            #ui_display_mode_chain
            #tags_chain
            #capabilities_chain
            .concurrency_safe(#concurrency_safe)
            #streaming_chain
            #strict_chain
        }
    };

    let tool_definition_fn = quote! {
        pub(crate) fn tool_definition() -> ::agena_plugin_sdk::ToolDefinition {
            Self::__macro_tool_definition(#tool, Self::input_schema())
        }
    };

    let dispatch_tool_invoke_fn =
        expand_static_surface_dispatch_fn(&input.data, &surface_for_dispatch)?;
    let flatten_shape_post_parse_expr = expand_flatten_shape_post_parse_tokens(&input.data)?;
    let tool_alias_match_arms = surface
        .aliases
        .iter()
        .map(|alias| quote! { | #alias })
        .collect::<Vec<_>>();
    let resolve_tool_exec_match_arms = match &input.data {
        Data::Enum(data_enum) => data_enum
            .variants
            .iter()
            .filter_map(|variant| {
                let config = parse_tool_variant_config(variant).ok()?;
                let VariantMapping::Exec(tool_name) = config.mapping else {
                    return None;
                };
                Some(quote! { #tool_name => {} })
            })
            .collect::<Vec<_>>(),
        _ => Vec::new(),
    };

    let mut enum_helper_fn = quote! {};
    let tool_definitions_body = match &input.data {
        Data::Enum(data_enum) => {
            let variant_definition_pushes = data_enum
                .variants
                .iter()
                .filter_map(|variant| {
                    let config = parse_tool_variant_config(variant).ok()?;
                    let VariantMapping::Exec(tool_name) = config.mapping else {
                        return None;
                    };
                    let action = LitStr::new(
                        ident_to_snake_case(&variant.ident).as_str(),
                        variant.ident.span(),
                    );
                    Some(quote! {
                        let schema = ::agena_plugin_sdk::macro_support::schema_for_discriminated_variant(
                            &input_schema,
                            "action",
                            #action,
                        )
                        .unwrap_or_else(|| input_schema.clone());
                        definitions.push(Self::__macro_tool_definition(#tool_name, schema));
                    })
                })
                .collect::<Vec<_>>();
            if variant_definition_pushes.is_empty() {
                quote! {
                    vec![Self::tool_definition()]
                }
            } else {
                quote! {
                    let input_schema = Self::input_schema();
                    let mut definitions = Vec::new();
                    #(#variant_definition_pushes)*
                    definitions
                }
            }
        }
        _ => quote! {
            vec![Self::tool_definition()]
        },
    };

    let tool_definitions_fn = quote! {
        pub(crate) fn tool_definitions() -> Vec<::agena_plugin_sdk::ToolDefinition> {
            #tool_definitions_body
        }
    };

    let (parse_input_body, parse_tool_body, resolve_tool_body) = match input.data {
        Data::Enum(data_enum) => {
            enum_helper_fn = expand_enum_input_normalize_fn(&data_enum.variants)?;
            let match_arms = data_enum
                .variants
                .iter()
                .map(expand_variant_arm)
                .collect::<Result<Vec<_>>>()?;
            let parse_tool_exec_arms = data_enum
                .variants
                .iter()
                .filter_map(|variant| {
                    let config = parse_tool_variant_config(variant).ok()?;
                    let VariantMapping::Exec(tool_name) = config.mapping else {
                        return None;
                    };
                    let prefixed_tool_name = LitStr::new(
                        format!("{}.{}", tool.value(), tool_name.value()).as_str(),
                        tool_name.span(),
                    );
                    let action = LitStr::new(
                        ident_to_snake_case(&variant.ident).as_str(),
                        variant.ident.span(),
                    );
                    Some(quote! {
                        #tool_name | #prefixed_tool_name => {
                            let mut input = input;
                            match &mut input {
                                serde_json::Value::Object(object) => {
                                    object.insert(
                                        "action".to_string(),
                                        serde_json::Value::String(#action.to_string()),
                                    );
                                    Self::parse_input(input)
                                }
                                _ => Err(::agena_plugin_sdk::PluginError::invalid_params(
                                    "tool route payload must be an object",
                                )),
                            }
                        }
                    })
                })
                .collect::<Vec<_>>();
            let validate_arms = data_enum
                .variants
                .iter()
                .filter_map(|variant| expand_variant_validation_arm(variant).transpose())
                .collect::<Result<Vec<_>>>()?;
            (
                quote! {
                    let mut input = input;
                    #built_in_normalize_expr
                    let input = #normalize_expr;
                    let input = Self::__macro_normalize_enum_input(input)?;
                    let schema = Self::input_schema();
                    let parsed = ::agena_plugin_sdk::macro_support::parse_typed_json_value_with_field_suggestions::<Self>(
                        input,
                        &schema,
                        "field",
                    )?;
                    let parsed = #built_in_post_parse_normalize_expr;
                    let parsed = #flatten_shape_post_parse_expr;
                    match &parsed {
                        #(#validate_arms)*
                        _ => {}
                    }
                    #built_in_validate_expr
                    #validate_expr
                    Ok(parsed)
                },
                quote! {
                    match tool {
                        #tool #(#tool_alias_match_arms)* => Self::parse_input(input),
                        #(#parse_tool_exec_arms,)*
                        other => Err(::agena_plugin_sdk::PluginError::invalid_params(format!(
                            "unknown {} tool '{other}'",
                            #tool
                        ))),
                    }
                },
                quote! {
                    let parsed = Self::parse_tool(tool, input)?;

                    match parsed {
                        #(#match_arms),*
                    }
                },
            )
        }
        Data::Struct(_) => (
            quote! {
                    let mut input = input;
                    #built_in_normalize_expr
                    let input = #normalize_expr;
                    let schema = Self::input_schema();
                    let parsed = ::agena_plugin_sdk::macro_support::parse_typed_json_value_with_field_suggestions::<Self>(
                        input,
                        &schema,
                        "field",
                    )?;
                    let parsed = #built_in_post_parse_normalize_expr;
                    let parsed = #flatten_shape_post_parse_expr;
                    #built_in_validate_expr
                    #validate_expr
                    Ok(parsed)
            },
            quote! {
                match tool {
                    #tool #(#tool_alias_match_arms)* => Self::parse_input(input),
                    other => Err(::agena_plugin_sdk::PluginError::invalid_params(format!(
                        "unknown {} tool '{other}'",
                        #tool
                    ))),
                }
            },
            quote! {
                let parsed = Self::parse_input(input)?;
                Ok((
                    #tool.to_string(),
                    serde_json::to_value(parsed)
                        .map_err(|err| ::agena_plugin_sdk::PluginError::invalid_params(err.to_string()))?,
                ))
            },
        ),
        _ => {
            return Err(syn::Error::new_spanned(
                name,
                "StaticToolSurface can only be derived for enums or structs",
            ));
        }
    };

    let parse_input_fn = quote! {
        pub(crate) fn parse_input(
            input: serde_json::Value,
        ) -> ::agena_plugin_sdk::Result<Self> {
            #parse_input_body
        }
    };
    let parse_tool_fn = quote! {
        pub(crate) fn parse_tool(
            tool: &str,
            input: serde_json::Value,
        ) -> ::agena_plugin_sdk::Result<Self> {
            #parse_tool_body
        }
    };

    Ok(quote! {
        impl #name {
            #enum_helper_fn
            #schema_metadata_fn

            #tool_name_fn

            #input_schema_fn

            #parse_input_fn

            #parse_tool_fn

            #parse_json_str_fn

            #make_tool_definition_fn

            #tool_definition_fn

            #tool_definitions_fn

            #dispatch_tool_invoke_fn

            pub(crate) fn resolve_tool(
                tool: &str,
                input: serde_json::Value,
            ) -> ::agena_plugin_sdk::Result<(String, serde_json::Value)> {
                match tool {
                    #tool #(#tool_alias_match_arms)* => {}
                    #(#resolve_tool_exec_match_arms,)*
                    other => {
                        return Err(::agena_plugin_sdk::PluginError::invalid_params(format!(
                            "unknown {} tool '{other}'",
                            #tool
                        )));
                    }
                }

                #resolve_tool_body
            }

            pub(crate) fn resolve_json_str(
                tool: &str,
                input: &str,
            ) -> ::agena_plugin_sdk::Result<(String, serde_json::Value)> {
                let value = ::agena_plugin_sdk::macro_support::parse_json_value_str(input)?;
                Self::resolve_tool(tool, value)
            }
        }

        impl ::agena_plugin_sdk::ToolSurface for #name {
            fn tool_name() -> &'static str {
                Self::tool_name()
            }

            fn tool_definition() -> ::agena_plugin_sdk::ToolDefinition {
                Self::tool_definition()
            }

            fn tool_definitions() -> Vec<::agena_plugin_sdk::ToolDefinition> {
                Self::tool_definitions()
            }

            fn parse_input(input: serde_json::Value) -> ::agena_plugin_sdk::Result<Self> {
                Self::parse_input(input)
            }

            fn parse_tool(
                tool: &str,
                input: serde_json::Value,
            ) -> ::agena_plugin_sdk::Result<Self> {
                Self::parse_tool(tool, input)
            }

            fn parse_json_str(input: &str) -> ::agena_plugin_sdk::Result<Self> {
                Self::parse_json_str(input)
            }

            fn resolve_tool(
                tool: &str,
                input: serde_json::Value,
            ) -> ::agena_plugin_sdk::Result<(String, serde_json::Value)> {
                Self::resolve_tool(tool, input)
            }

            fn resolve_json_str(
                tool: &str,
                input: &str,
            ) -> ::agena_plugin_sdk::Result<(String, serde_json::Value)> {
                Self::resolve_json_str(tool, input)
            }
        }
    })
}

struct ToolSuiteVariantConfig {
    parse: Option<Path>,
    resolve: Option<Path>,
    route: Option<LitStr>,
    route_action: Option<LitStr>,
    field: Option<Path>,
    convert: Option<Path>,
    shape: Option<Path>,
}

#[derive(Default)]
struct ToolSuiteConfig {
    handler_receiver: Option<Path>,
}

fn expand_tool_suite(input: DeriveInput) -> Result<proc_macro2::TokenStream> {
    let name = input.ident;
    let suite = parse_tool_suite_config(&input.attrs)?;
    let Data::Enum(data_enum) = input.data else {
        return Err(syn::Error::new_spanned(
            name,
            "ToolSuite can only be derived for enums",
        ));
    };

    let variants = data_enum
        .variants
        .iter()
        .map(parse_tool_suite_variant)
        .collect::<Result<Vec<_>>>()?;

    let definition_pushes = variants.iter().map(|variant| {
        let ty = &variant.ty;
        quote! {
            definitions.extend(<#ty as ::agena_plugin_sdk::ToolSurface>::tool_definitions());
        }
    });
    let parse_arms = variants.iter().map(|variant| {
        let ident = &variant.ident;
        let ty = &variant.ty;
        let parse_expr = variant
            .config
            .parse
            .as_ref()
            .map(|path| quote! { #path(input) })
            .unwrap_or_else(
                || quote! { <#ty as ::agena_plugin_sdk::ToolSurface>::parse_tool(tool, input) },
            );
        quote! {
            let __definitions = <#ty as ::agena_plugin_sdk::ToolSurface>::tool_definitions();
            if __definitions.iter().any(|__definition| tool == __definition.name) {
                return Ok(Self::#ident(#parse_expr?));
            }
        }
    });
    let resolve_arms = variants
        .iter()
        .map(|variant| -> Result<proc_macro2::TokenStream> {
        let ty = &variant.ty;
        let parse_expr = variant
            .config
            .parse
            .as_ref()
            .map(|path| quote! { #path(input) })
            .unwrap_or_else(
                || quote! { <#ty as ::agena_plugin_sdk::ToolSurface>::parse_input(input) },
            );
        let resolve_expr = if let Some(path) = variant.config.resolve.as_ref() {
            quote! { #path(tool, input) }
        } else if let Some(route) = variant.config.route.as_ref() {
            let raw_payload_expr = if let Some(path) = variant.config.convert.as_ref() {
                quote! {
                    serde_json::to_value(#path(parsed)?)
                        .map_err(|err| ::agena_plugin_sdk::PluginError::invalid_params(err.to_string()))?
                }
            } else if let Some(field) = variant.config.field.as_ref() {
                let field_ident = single_segment_ident(field, "field")?;
                quote! {
                    serde_json::to_value(parsed.#field_ident)
                        .map_err(|err| ::agena_plugin_sdk::PluginError::invalid_params(err.to_string()))?
                }
            } else {
                quote! {
                    serde_json::to_value(parsed)
                        .map_err(|err| ::agena_plugin_sdk::PluginError::invalid_params(err.to_string()))?
                }
            };
            let action_injected_payload_expr = if let Some(route_action) =
                variant.config.route_action.as_ref()
            {
                quote! {{
                    let mut routed_value = #raw_payload_expr;
                    match &mut routed_value {
                        serde_json::Value::Object(object) => {
                            object.insert(
                                "action".to_string(),
                                serde_json::Value::String(#route_action.to_string()),
                            );
                            routed_value
                        }
                        _ => {
                            return Err(::agena_plugin_sdk::PluginError::invalid_params(
                                "route_action requires an object-shaped routed payload",
                            ));
                        }
                    }
                }}
            } else {
                raw_payload_expr
            };
            let payload_expr = if let Some(shape) = variant.config.shape.as_ref() {
                quote! {{
                    let routed_value = #action_injected_payload_expr;
                    let routed_input = <#shape as ::agena_plugin_sdk::ToolInputShape>::parse_input(routed_value)?;
                    serde_json::to_value(routed_input)
                        .map_err(|err| ::agena_plugin_sdk::PluginError::invalid_params(err.to_string()))?
                }}
            } else {
                action_injected_payload_expr
            };
            quote! {{
                let parsed = #parse_expr?;
                let routed_input = #payload_expr;
                Ok((#route.to_string(), routed_input))
            }}
        } else {
            quote! { <#ty as ::agena_plugin_sdk::ToolSurface>::resolve_tool(tool, input) }
        };
        Ok(quote! {
            let __definitions = <#ty as ::agena_plugin_sdk::ToolSurface>::tool_definitions();
            if __definitions.iter().any(|__definition| tool == __definition.name) {
                return #resolve_expr;
            }
        })
    })
    .collect::<Result<Vec<_>>>()?;

    let dispatch_tool_invoke_fn = if let Some(receiver_ty) = suite.handler_receiver {
        let dispatch_arms = variants
            .iter()
            .map(|variant| {
                let ident = &variant.ident;
                quote! {
                    Self::#ident(inner) => inner.dispatch_tool_invoke(receiver).await
                }
            })
            .collect::<Vec<_>>();
        let dispatch_context_arms = variants
            .iter()
            .map(|variant| {
                let ident = &variant.ident;
                quote! {
                    Self::#ident(inner) => inner.dispatch_tool_invoke_with_context(receiver, context).await
                }
            })
            .collect::<Vec<_>>();
        let dispatch_stream_arms = variants
            .iter()
            .map(|variant| {
                let ident = &variant.ident;
                quote! {
                    Self::#ident(inner) => inner.dispatch_tool_invoke_stream(receiver, sink).await
                }
            })
            .collect::<Vec<_>>();
        let dispatch_stream_context_arms = variants
            .iter()
            .map(|variant| {
                let ident = &variant.ident;
                quote! {
                    Self::#ident(inner) => inner.dispatch_tool_invoke_stream_with_context(receiver, context, sink).await
                }
            })
            .collect::<Vec<_>>();
        let dispatch_permission_paths_arms = variants
            .iter()
            .map(|variant| {
                let ident = &variant.ident;
                quote! {
                    Self::#ident(inner) => inner.dispatch_permission_paths(receiver).await
                }
            })
            .collect::<Vec<_>>();
        let dispatch_permission_networks_arms = variants
            .iter()
            .map(|variant| {
                let ident = &variant.ident;
                quote! {
                    Self::#ident(inner) => inner.dispatch_permission_networks(receiver).await
                }
            })
            .collect::<Vec<_>>();
        quote! {
            pub async fn dispatch_tool_invoke(
                self,
                receiver: &#receiver_ty,
            ) -> ::agena_plugin_sdk::Result<::agena_plugin_sdk::ToolInvokeOutput> {
                match self {
                    #(#dispatch_arms),*
                }
            }

            pub async fn dispatch_tool_invoke_with_context(
                self,
                receiver: &#receiver_ty,
                context: &::agena_plugin_sdk::ToolInvokeContext<'_>,
            ) -> ::agena_plugin_sdk::Result<::agena_plugin_sdk::ToolInvokeOutput> {
                match self {
                    #(#dispatch_context_arms),*
                }
            }

            pub async fn dispatch_tool_invoke_stream(
                self,
                receiver: &#receiver_ty,
                sink: ::agena_plugin_sdk::ToolStreamSink,
            ) -> ::agena_plugin_sdk::Result<::agena_plugin_sdk::ToolStreamEnd> {
                match self {
                    #(#dispatch_stream_arms),*
                }
            }

            pub async fn dispatch_tool_invoke_stream_with_context(
                self,
                receiver: &#receiver_ty,
                context: &::agena_plugin_sdk::ToolInvokeContext<'_>,
                sink: ::agena_plugin_sdk::ToolStreamSink,
            ) -> ::agena_plugin_sdk::Result<::agena_plugin_sdk::ToolStreamEnd> {
                match self {
                    #(#dispatch_stream_context_arms),*
                }
            }

            pub async fn dispatch_permission_paths(
                self,
                receiver: &#receiver_ty,
            ) -> ::agena_plugin_sdk::Result<Vec<::agena_plugin_sdk::PathRequest>> {
                match self {
                    #(#dispatch_permission_paths_arms),*
                }
            }

            pub async fn dispatch_permission_networks(
                self,
                receiver: &#receiver_ty,
            ) -> ::agena_plugin_sdk::Result<Vec<::agena_plugin_sdk::NetworkRequest>> {
                match self {
                    #(#dispatch_permission_networks_arms),*
                }
            }
        }
    } else {
        quote! {}
    };

    Ok(quote! {
        impl #name {
            pub(crate) fn tool_definitions() -> Vec<::agena_plugin_sdk::ToolDefinition> {
                let mut definitions = Vec::new();
                #(#definition_pushes)*
                definitions
            }

            pub(crate) fn parse_tool(
                tool: &str,
                input: serde_json::Value,
            ) -> ::agena_plugin_sdk::Result<Self> {
                #(#parse_arms)*
                Err(::agena_plugin_sdk::PluginError::invalid_params(format!(
                    "unknown tool '{tool}'"
                )))
            }

            pub(crate) fn resolve_tool(
                tool: &str,
                input: serde_json::Value,
            ) -> ::agena_plugin_sdk::Result<(String, serde_json::Value)> {
                #(#resolve_arms)*
                Err(::agena_plugin_sdk::PluginError::invalid_params(format!(
                    "unknown tool '{tool}'"
                )))
            }

            pub(crate) fn parse_tool_json_str(
                tool: &str,
                input: &str,
            ) -> ::agena_plugin_sdk::Result<Self> {
                let value = ::agena_plugin_sdk::macro_support::parse_json_value_str(input)?;
                Self::parse_tool(tool, value)
            }

            pub(crate) fn resolve_tool_json_str(
                tool: &str,
                input: &str,
            ) -> ::agena_plugin_sdk::Result<(String, serde_json::Value)> {
                let value = ::agena_plugin_sdk::macro_support::parse_json_value_str(input)?;
                Self::resolve_tool(tool, value)
            }

            #dispatch_tool_invoke_fn
        }

        impl ::agena_plugin_sdk::ToolSuiteSurface for #name {
            fn tool_definitions() -> Vec<::agena_plugin_sdk::ToolDefinition> {
                Self::tool_definitions()
            }

            fn parse_tool(
                tool: &str,
                input: serde_json::Value,
            ) -> ::agena_plugin_sdk::Result<Self> {
                Self::parse_tool(tool, input)
            }

            fn parse_tool_json_str(
                tool: &str,
                input: &str,
            ) -> ::agena_plugin_sdk::Result<Self> {
                Self::parse_tool_json_str(tool, input)
            }

            fn resolve_tool(
                tool: &str,
                input: serde_json::Value,
            ) -> ::agena_plugin_sdk::Result<(String, serde_json::Value)> {
                Self::resolve_tool(tool, input)
            }

            fn resolve_tool_json_str(
                tool: &str,
                input: &str,
            ) -> ::agena_plugin_sdk::Result<(String, serde_json::Value)> {
                Self::resolve_tool_json_str(tool, input)
            }
        }
    })
}

fn expand_tool_input_shape(input: DeriveInput) -> Result<proc_macro2::TokenStream> {
    let name = input.ident;
    let config = parse_tool_input_shape_config(&input.attrs)?;
    let schema_metadata_fn = expand_schema_metadata_fn(&input.data, &config, |variant, prefix| {
        let config = parse_tool_input_variant_config(variant)?;
        let mut calls = constraint_schema_metadata_calls(prefix, &config)?;
        calls.extend(constraint_relation_metadata_calls(prefix, &config)?);
        Ok(calls)
    })?;
    let flatten_shape_post_parse_expr = expand_flatten_shape_post_parse_tokens(&input.data)?;
    let (enum_helper_fn, variant_validate_arms) = match &input.data {
        Data::Enum(data_enum) => (
            expand_input_shape_enum_normalize_fn(&data_enum.variants)?,
            data_enum
                .variants
                .iter()
                .filter_map(|variant| {
                    expand_input_shape_variant_validation_arm(variant).transpose()
                })
                .collect::<Result<Vec<_>>>()?,
        ),
        Data::Struct(_) => (
            quote! {
                fn __macro_normalize_enum_input(
                    input: serde_json::Value,
                ) -> ::agena_plugin_sdk::Result<serde_json::Value> {
                    Ok(input)
                }
            },
            Vec::new(),
        ),
        _ => {
            return Err(syn::Error::new_spanned(
                name,
                "ToolInputShape can only be derived for enums or structs",
            ));
        }
    };

    let built_in_normalize_expr =
        built_in_normalization_tokens(quote! { &mut input }, &config.trim, &config.trim_suffix);
    let built_in_post_parse_normalize_expr =
        built_in_post_parse_normalization_tokens(&config.trim, &config.trim_suffix);
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
        &config.max_chars,
    );
    let dispatch_tool_invoke_fn = expand_tool_input_shape_dispatch_fn(&input.data, &config)?;

    Ok(quote! {
        impl #name {
            #enum_helper_fn
            #schema_metadata_fn
            #dispatch_tool_invoke_fn

            pub(crate) fn input_schema() -> serde_json::Value {
                let mut schema = ::agena_plugin_sdk::macro_support::json_schema_for::<Self>();
                Self::__macro_apply_schema_metadata(&mut schema);
                schema
            }

            pub(crate) fn parse_input(
                input: serde_json::Value,
            ) -> ::agena_plugin_sdk::Result<Self> {
                let mut input = input;
                #built_in_normalize_expr
                let input = #normalize_expr;
                let input = Self::__macro_normalize_enum_input(input)?;
                let schema = Self::input_schema();
                let parsed = ::agena_plugin_sdk::macro_support::parse_typed_json_value_with_field_suggestions::<Self>(
                    input,
                    &schema,
                    "field",
                )?;
                let parsed = #built_in_post_parse_normalize_expr;
                let parsed = #flatten_shape_post_parse_expr;
                match &parsed {
                    #(#variant_validate_arms)*
                    _ => {}
                }
                #built_in_validate_expr
                #validate_expr
                Ok(parsed)
            }

            pub(crate) fn parse_json_str(
                input: &str,
            ) -> ::agena_plugin_sdk::Result<Self> {
                let input = ::agena_plugin_sdk::macro_support::parse_json_value_str(input)?;
                Self::parse_input(input)
            }
        }

        impl ::agena_plugin_sdk::ToolInputShape for #name {
            fn input_schema() -> serde_json::Value {
                Self::input_schema()
            }

            fn parse_input(input: serde_json::Value) -> ::agena_plugin_sdk::Result<Self> {
                Self::parse_input(input)
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

fn expand_tool_input_shape_dispatch_fn(
    data: &Data,
    config: &ToolInputShapeConfig,
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
            || config.handle_by_value == Some(true)
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
                    "ToolInputShape structs cannot combine handle and handle_with_context",
                ));
            }
            if struct_stream_handle.is_some() && struct_stream_handle_with_context.is_some() {
                return Err(syn::Error::new(
                    proc_macro2::Span::call_site(),
                    "ToolInputShape structs cannot combine stream_handle and stream_handle_with_context",
                ));
            }
            let Some(receiver_ty) = receiver_ty else {
                if struct_handle.is_some()
                    || struct_handle_with_context.is_some()
                    || struct_stream_handle.is_some()
                    || struct_stream_handle_with_context.is_some()
                    || struct_permission_paths_handle.is_some()
                    || struct_permission_networks_handle.is_some()
                    || config.handle_by_value == Some(true)
                {
                    return Err(syn::Error::new(
                        proc_macro2::Span::call_site(),
                        "handle/handle_with_context/stream_handle/stream_handle_with_context/permission_paths_handle/permission_networks_handle/handle_by_value on a shape struct require handler_receiver",
                    ));
                }
                return Ok(quote! {});
            };
            let handle_by_value = config.handle_by_value.unwrap_or(false);
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
                    parse_tool_input_variant_config(variant)
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
                let config = parse_tool_input_variant_config(variant)?;
                if config.handle_by_value
                    && config.handle.is_none()
                    && config.handle_with_context.is_none()
                    && config.stream_handle.is_none()
                    && config.stream_handle_with_context.is_none()
                {
                    return Err(syn::Error::new_spanned(
                        variant,
                        "variant handle_by_value requires #[tool(handle = path)], #[tool(handle_with_context = path)], #[tool(stream_handle = path)], or #[tool(stream_handle_with_context = path)]",
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
                    "shape dispatch requires #[tool(handle = path)] on every variant",
                ));
            }
            if saw_context_handle && !can_generate_context {
                return Err(syn::Error::new(
                    proc_macro2::Span::call_site(),
                    "context-aware shape dispatch requires #[tool(handle = path)] or #[tool(handle_with_context = path)] on every variant",
                ));
            }
            if saw_any_stream_handle && !can_generate_plain_stream && !saw_context_stream_handle {
                return Err(syn::Error::new(
                    proc_macro2::Span::call_site(),
                    "shape stream dispatch requires #[tool(stream_handle = path)], #[tool(stream_handle_with_context = path)], #[tool(handle = path)], or #[tool(handle_with_context = path)] on every variant",
                ));
            }
            if saw_context_stream_handle && !can_generate_context_stream {
                return Err(syn::Error::new(
                    proc_macro2::Span::call_site(),
                    "context-aware shape stream dispatch requires #[tool(stream_handle = path)], #[tool(stream_handle_with_context = path)], #[tool(handle = path)], or #[tool(handle_with_context = path)] on every variant",
                ));
            }
            if saw_any_permission_paths_handle && saw_missing_permission_paths_handle {
                return Err(syn::Error::new(
                    proc_macro2::Span::call_site(),
                    "shape permission path dispatch requires #[tool(permission_paths_handle = path)] on every variant",
                ));
            }
            if saw_any_permission_networks_handle && saw_missing_permission_networks_handle {
                return Err(syn::Error::new(
                    proc_macro2::Span::call_site(),
                    "shape permission network dispatch requires #[tool(permission_networks_handle = path)] on every variant",
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
            metadata_calls.extend(struct_field_schema_metadata_calls("", &data_struct.fields)?);
            metadata_calls.extend(constraint_schema_metadata_calls("", constraints)?);
            metadata_calls.extend(constraint_relation_metadata_calls("", constraints)?);
        }
        Data::Enum(data_enum) => {
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
                let action_aliases = variant_action_aliases(variant)?;
                if !action_aliases.is_empty() {
                    let alias_values = action_aliases
                        .iter()
                        .map(|alias| LitStr::new(alias, variant.ident.span()))
                        .collect::<Vec<_>>();
                    for group in ["oneOf", "anyOf", "allOf"] {
                        let pointer =
                            LitStr::new(format!("/{group}/{index}").as_str(), variant.ident.span());
                        metadata_calls.push(quote! {
                            ::agena_plugin_sdk::macro_support::set_schema_string_list_metadata(
                                schema,
                                #pointer,
                                "x-agena-aliases",
                                &[#(#alias_values),*],
                            );
                        });
                    }
                }
                for group in ["oneOf", "anyOf", "allOf"] {
                    let prefix = format!("/{group}/{index}");
                    metadata_calls.extend(struct_field_schema_metadata_calls(
                        prefix.as_str(),
                        &variant.fields,
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

fn constraint_relation_metadata_calls<C: SchemaRelationSource>(
    prefix: &str,
    constraints: &C,
) -> Result<Vec<proc_macro2::TokenStream>> {
    let mut labels = Vec::new();
    for group in constraints.exactly_one_of() {
        if !group.is_empty() {
            let joined = group
                .iter()
                .map(|path| format!("`{}`", path.value()))
                .collect::<Vec<_>>()
                .join(", ");
            labels.push(format!("exactly_one_of: {joined}"));
        }
    }
    for group in constraints.at_least_one_of() {
        if !group.is_empty() {
            let joined = group
                .iter()
                .map(|path| format!("`{}`", path.value()))
                .collect::<Vec<_>>()
                .join(", ");
            labels.push(format!("at_least_one_of: {joined}"));
        }
    }
    for constraint in constraints.requires() {
        labels.push(format!(
            "requires `{}` -> `{}`",
            constraint.left.value(),
            constraint.right.value()
        ));
    }
    for constraint in constraints.conflicts_with() {
        labels.push(format!(
            "conflicts_with `{}` x `{}`",
            constraint.left.value(),
            constraint.right.value()
        ));
    }
    for constraint in constraints.required_unless_present() {
        labels.push(format!(
            "required_unless_present `{}` unless `{}` present",
            constraint.left.value(),
            constraint.right.value()
        ));
    }
    for constraint in constraints.distinct_trimmed_within() {
        labels.push(format!(
            "distinct_trimmed_within `{}` within `{}`",
            constraint.left.value(),
            constraint.right.value()
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

fn constraint_schema_metadata_calls<C: SchemaConstraintSource>(
    prefix: &str,
    constraints: &C,
) -> Result<Vec<proc_macro2::TokenStream>> {
    let mut calls = Vec::new();
    calls.extend(
        constraints
            .non_empty()
            .iter()
            .chain(constraints.non_empty_if_present().iter())
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
            .min_items()
            .iter()
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
            .max_chars()
            .iter()
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

fn struct_field_schema_metadata_calls(
    prefix: &str,
    fields: &Fields,
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
                let mut overlay = <#flatten_shape_ty as ::agena_plugin_sdk::ToolInputShape>::input_schema();
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
        let Some(property_name) = field_schema_property_name(field)? else {
            continue;
        };
        let aliases = field_schema_aliases(field)?;
        let description = doc_text(&field.attrs).and_then(|text| {
            let trimmed = text.trim().to_string();
            (!trimmed.is_empty()).then_some(trimmed)
        });
        if description.is_none() && aliases.is_empty() {
            continue;
        }
        let pointer = if prefix.is_empty() {
            format!("/properties/{property_name}")
        } else {
            format!("{prefix}/properties/{property_name}")
        };
        let pointer = LitStr::new(pointer.as_str(), field.span());
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
        calls.push(quote! {
            ::agena_plugin_sdk::macro_support::set_schema_string_metadata(
                schema,
                #pointer,
                "x-agena-order",
                #order,
            );
        });
        if !aliases.is_empty() {
            let alias_values = aliases
                .iter()
                .map(|alias| LitStr::new(alias, field.span()))
                .collect::<Vec<_>>();
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

fn field_schema_property_name(field: &Field) -> Result<Option<String>> {
    let Some(ident) = field.ident.as_ref() else {
        return Ok(None);
    };
    let mut rename = None;
    for attr in &field.attrs {
        if !attr.path().is_ident("serde") && !attr.path().is_ident("schemars") {
            continue;
        }
        let metas = attr.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)?;
        for meta in metas {
            match meta {
                Meta::Path(path) if path.is_ident("flatten") => return Ok(None),
                Meta::NameValue(value) => {
                    if value.path.is_ident("rename") {
                        rename = Some(expr_lit_str(&value.value, "rename")?.value());
                    }
                }
                _ => {}
            }
        }
    }
    Ok(Some(rename.unwrap_or_else(|| ident.to_string())))
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
                let Some(name) = field_schema_property_name(field)? else {
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

fn variant_action_aliases(variant: &Variant) -> Result<Vec<String>> {
    let mut aliases = Vec::new();
    for attr in &variant.attrs {
        if !attr.path().is_ident("tool") {
            continue;
        }
        let metas = attr.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)?;
        for meta in metas {
            if let Meta::List(list) = meta {
                let Some(ident) = list.path.get_ident() else {
                    continue;
                };
                match ident.to_string().as_str() {
                    "action_alias" => {
                        aliases.extend(
                            parse_lit_str_list(list.tokens)?
                                .into_iter()
                                .map(|alias| alias.value()),
                        );
                    }
                    "action_alias_default" => {
                        aliases.push(parse_action_alias_default(list.tokens)?.alias.value());
                    }
                    _ => {}
                }
            }
        }
    }
    Ok(aliases)
}

fn flatten_shape_type(field: &Field) -> Result<Option<syn::Type>> {
    if !field_is_flatten(field)? {
        return Ok(None);
    }
    for attr in &field.attrs {
        if !attr.path().is_ident("tool") {
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

fn expand_flatten_shape_post_parse_tokens(data: &Data) -> Result<proc_macro2::TokenStream> {
    match data {
        Data::Struct(data_struct) => {
            let updates = data_struct
                .fields
                .iter()
                .enumerate()
                .map(|(index, field)| {
                    let flatten_shape_ty = flatten_shape_type(field)?;
                    let member = field
                        .ident
                        .clone()
                        .map(Member::Named)
                        .unwrap_or_else(|| Member::Unnamed(Index::from(index)));
                    Ok(flatten_shape_ty.map(|ty| (member, ty)))
                })
                .collect::<Result<Vec<_>>>()?
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .into_iter()
                .map(|(member, ty)| {
                    quote! {
                        parsed.#member = <#ty as ::agena_plugin_sdk::ToolInputShape>::parse_input(
                            serde_json::to_value(&parsed.#member)
                                .map_err(|err| ::agena_plugin_sdk::PluginError::invalid_params(err.to_string()))?,
                        )?;
                    }
                })
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
            let arms = data_enum
                .variants
                .iter()
                .map(expand_flatten_shape_variant_post_parse_arm)
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

fn expand_flatten_shape_variant_post_parse_arm(
    variant: &Variant,
) -> Result<Option<proc_macro2::TokenStream>> {
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
                    Ok((ident, flatten_shape_ty))
                })
                .collect::<Result<Vec<_>>>()?;
            if !bindings
                .iter()
                .any(|(_, flatten_shape_ty)| flatten_shape_ty.is_some())
            {
                return Ok(None);
            }
            let pattern_fields = bindings.iter().map(|(ident, _)| quote! { #ident });
            let normalize_bindings = bindings
                .iter()
                .filter_map(|(ident, flatten_shape_ty)| {
                    flatten_shape_ty.as_ref().map(|ty| {
                        quote! {
                            let #ident = <#ty as ::agena_plugin_sdk::ToolInputShape>::parse_input(
                                serde_json::to_value(&#ident)
                                    .map_err(|err| ::agena_plugin_sdk::PluginError::invalid_params(err.to_string()))?,
                            )?;
                        }
                    })
                })
                .collect::<Vec<_>>();
            let rebuild_fields = bindings.iter().map(|(ident, _)| quote! { #ident });
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
                    Ok((binding, flatten_shape_ty))
                })
                .collect::<Result<Vec<_>>>()?;
            if !bindings
                .iter()
                .any(|(_, flatten_shape_ty)| flatten_shape_ty.is_some())
            {
                return Ok(None);
            }
            let pattern_fields = bindings.iter().map(|(binding, _)| quote! { #binding });
            let normalize_bindings = bindings
                .iter()
                .filter_map(|(binding, flatten_shape_ty)| {
                    flatten_shape_ty.as_ref().map(|ty| {
                        quote! {
                            let #binding = <#ty as ::agena_plugin_sdk::ToolInputShape>::parse_input(
                                serde_json::to_value(&#binding)
                                    .map_err(|err| ::agena_plugin_sdk::PluginError::invalid_params(err.to_string()))?,
                            )?;
                        }
                    })
                })
                .collect::<Vec<_>>();
            let rebuild_fields = bindings.iter().map(|(binding, _)| quote! { #binding });
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

struct ToolSuiteVariant {
    ident: syn::Ident,
    ty: syn::Type,
    config: ToolSuiteVariantConfig,
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
    non_empty: Vec<LitStr>,
    non_empty_if_present: Vec<LitStr>,
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
    max_chars: Vec<PathUsizeConstraint>,
    default_when_empty: bool,
    infer_when_present: Vec<LitStr>,
    drop_keys: Vec<LitStr>,
    action_aliases: Vec<LitStr>,
    action_alias_defaults: Vec<ActionAliasDefaultConfig>,
}

#[derive(Default)]
struct ToolInputShapeConfig {
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
    handle_by_value: Option<bool>,
    trim: Vec<LitStr>,
    trim_suffix: Vec<PathStringConstraint>,
    non_empty: Vec<LitStr>,
    non_empty_if_present: Vec<LitStr>,
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
    max_chars: Vec<PathUsizeConstraint>,
}

fn parse_tool_input_shape_config(attrs: &[Attribute]) -> Result<ToolInputShapeConfig> {
    let mut config = ToolInputShapeConfig::default();
    for attr in attrs {
        if !attr.path().is_ident("tool_input") && !attr.path().is_ident("tool_args") {
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
                        "normalize" => {
                            config.normalize = Some(expr_path(&value.value, "normalize")?)
                        }
                        "validate" => config.validate = Some(expr_path(&value.value, "validate")?),
                        "handler_receiver" => {
                            config.handler_receiver =
                                Some(expr_path(&value.value, "handler_receiver")?)
                        }
                        "handle" => config.handle = Some(expr_path(&value.value, "handle")?),
                        "handle_with_context" => {
                            config.handle_with_context =
                                Some(expr_path(&value.value, "handle_with_context")?)
                        }
                        "stream_handle" => {
                            config.stream_handle = Some(expr_path(&value.value, "stream_handle")?)
                        }
                        "stream_handle_with_context" => {
                            config.stream_handle_with_context =
                                Some(expr_path(&value.value, "stream_handle_with_context")?)
                        }
                        "permission_paths_handle" => {
                            config.permission_paths_handle =
                                Some(expr_path(&value.value, "permission_paths_handle")?)
                        }
                        "permission_networks_handle" => {
                            config.permission_networks_handle =
                                Some(expr_path(&value.value, "permission_networks_handle")?)
                        }
                        "handle_field" => {
                            config.handle_field = Some(expr_path(&value.value, "handle_field")?)
                        }
                        "handle_by_value" => {
                            config.handle_by_value =
                                Some(expr_lit_bool(&value.value, "handle_by_value")?)
                        }
                        other => {
                            return Err(syn::Error::new_spanned(
                                ident,
                                format!("unsupported tool args attribute '{other}'"),
                            ));
                        }
                    }
                }
                Meta::List(list) => {
                    let Some(ident) = list.path.get_ident() else {
                        return Err(syn::Error::new_spanned(list.path, "expected identifier"));
                    };
                    match ident.to_string().as_str() {
                        "trim" => config.trim.extend(parse_lit_str_list(list.tokens)?),
                        "trim_suffix" => config
                            .trim_suffix
                            .push(parse_path_lit_str_constraint(list.tokens, "trim_suffix")?),
                        "non_empty" => config.non_empty.extend(parse_lit_str_list(list.tokens)?),
                        "non_empty_if_present" => config
                            .non_empty_if_present
                            .extend(parse_lit_str_list(list.tokens)?),
                        "exactly_one_of" => {
                            config.exactly_one_of.push(parse_lit_str_list(list.tokens)?)
                        }
                        "at_least_one_of" => config
                            .at_least_one_of
                            .push(parse_lit_str_list(list.tokens)?),
                        "requires" => config
                            .requires
                            .push(parse_path_pair_constraint(list.tokens, "requires")?),
                        "conflicts_with" => config
                            .conflicts_with
                            .push(parse_path_pair_constraint(list.tokens, "conflicts_with")?),
                        "required_unless_present" => {
                            config
                                .required_unless_present
                                .push(parse_path_pair_constraint(
                                    list.tokens,
                                    "required_unless_present",
                                )?)
                        }
                        "forbid_substrings" => {
                            config
                                .forbid_substrings
                                .push(parse_path_lit_str_list_constraint(
                                    list.tokens,
                                    "forbid_substrings",
                                )?)
                        }
                        "distinct_trimmed" => config
                            .distinct_trimmed
                            .extend(parse_lit_str_list(list.tokens)?),
                        "distinct_trimmed_within" => {
                            config
                                .distinct_trimmed_within
                                .push(parse_path_pair_constraint(
                                    list.tokens,
                                    "distinct_trimmed_within",
                                )?)
                        }
                        "min_items" => config
                            .min_items
                            .push(parse_path_usize_constraint(list.tokens, "min_items")?),
                        "max_items" => config
                            .max_items
                            .push(parse_path_usize_constraint(list.tokens, "max_items")?),
                        "max_chars" => config
                            .max_chars
                            .push(parse_path_usize_constraint(list.tokens, "max_chars")?),
                        other => {
                            return Err(syn::Error::new_spanned(
                                ident,
                                format!("unsupported tool args list '{other}'"),
                            ));
                        }
                    }
                }
                Meta::Path(path) => {
                    return Err(syn::Error::new_spanned(
                        path,
                        "unsupported bare tool args argument",
                    ));
                }
            }
        }
    }
    Ok(config)
}

fn parse_tool_suite_config(attrs: &[Attribute]) -> Result<ToolSuiteConfig> {
    let mut config = ToolSuiteConfig::default();
    for attr in attrs {
        if !attr.path().is_ident("tool_suite") && !attr.path().is_ident("tool_subcommands") {
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
                        "handler_receiver" => {
                            config.handler_receiver =
                                Some(expr_path(&value.value, "handler_receiver")?)
                        }
                        other => {
                            return Err(syn::Error::new_spanned(
                                ident,
                                format!("unsupported tool suite attribute '{other}'"),
                            ));
                        }
                    }
                }
                Meta::List(list) => {
                    return Err(syn::Error::new_spanned(
                        list,
                        "unsupported tool suite list argument",
                    ));
                }
                Meta::Path(path) => {
                    return Err(syn::Error::new_spanned(
                        path,
                        "unsupported bare tool suite argument",
                    ));
                }
            }
        }
    }
    Ok(config)
}

fn expand_input_shape_enum_normalize_fn(
    variants: &Punctuated<Variant, Token![,]>,
) -> Result<proc_macro2::TokenStream> {
    struct EnumNormalizeVariant {
        action: LitStr,
        default_when_empty: bool,
        infer_when_present: Vec<LitStr>,
        drop_keys: Vec<LitStr>,
        action_aliases: Vec<LitStr>,
        action_alias_defaults: Vec<ActionAliasDefaultConfig>,
    }

    let mut normalize_variants = Vec::new();
    let mut action_candidates = Vec::new();
    for variant in variants {
        let config = parse_tool_input_variant_config(variant)?;
        let action = tool_input_variant_action_name(variant, &config);
        action_candidates.push(action.clone());
        action_candidates.extend(
            variant_action_aliases(variant)?
                .into_iter()
                .map(|alias| LitStr::new(alias.as_str(), variant.ident.span())),
        );
        if config.default_when_empty
            || !config.infer_when_present.is_empty()
            || !config.drop_keys.is_empty()
            || !config.action_aliases.is_empty()
            || !config.action_alias_defaults.is_empty()
        {
            normalize_variants.push(EnumNormalizeVariant {
                action: tool_input_variant_action_name(variant, &config),
                default_when_empty: config.default_when_empty,
                infer_when_present: config.infer_when_present,
                drop_keys: config.drop_keys,
                action_aliases: config.action_aliases,
                action_alias_defaults: config.action_alias_defaults,
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
                "only one #[tool(default_when_empty = true)] variant is allowed, found {}",
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

    let action_alias_match_arms = normalize_variants.iter().flat_map(|variant| {
        let action = &variant.action;
        let simple_aliases = variant.action_aliases.iter().map(|alias| {
            quote! {
                #alias => {
                    object.insert(
                        "action".to_string(),
                        serde_json::Value::String(#action.to_string()),
                    );
                    #action.to_string()
                }
            }
        });
        let alias_defaults = variant.action_alias_defaults.iter().map(|alias_default| {
            let alias = &alias_default.alias;
            let defaults = alias_default.defaults.iter().map(|(field, value)| {
                quote! {
                    if !object.contains_key(#field) {
                        object.insert(#field.to_string(), serde_json::json!(#value));
                    }
                }
            });
            quote! {
                #alias => {
                    object.insert(
                        "action".to_string(),
                        serde_json::Value::String(#action.to_string()),
                    );
                    #(#defaults)*
                    #action.to_string()
                }
            }
        });
        simple_aliases.chain(alias_defaults).collect::<Vec<_>>()
    });

    let infer_match_exprs = normalize_variants
        .iter()
        .filter(|variant| !variant.infer_when_present.is_empty())
        .map(|variant| {
            let action = &variant.action;
            let keys = &variant.infer_when_present;
            quote! {
                if inferred_action.is_none() && [#(#keys),*].iter().any(|key| object.contains_key(*key)) {
                    inferred_action = Some(#action);
                }
            }
        });

    let drop_match_arms = normalize_variants
        .iter()
        .filter(|variant| !variant.drop_keys.is_empty())
        .map(|variant| {
            let action = &variant.action;
            let keys = &variant.drop_keys;
            quote! {
                #action => {
                    for key in [#(#keys),*] {
                        object.remove(key);
                    }
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
                    #(#action_alias_match_arms,)*
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

            Ok(serde_json::Value::Object(object))
        }
    })
}

fn expand_input_shape_variant_validation_arm(
    variant: &Variant,
) -> Result<Option<proc_macro2::TokenStream>> {
    let config = parse_tool_input_variant_config(variant)?;
    let has_built_in_validation = !config.non_empty.is_empty()
        || !config.non_empty_if_present.is_empty()
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
        || !config.max_chars.is_empty();
    if config.validate.is_none() && !has_built_in_validation {
        return Ok(None);
    }

    let variant_name = &variant.ident;
    let built_in_validate_expr = built_in_validation_tokens(
        quote! { value },
        &config.non_empty,
        &config.non_empty_if_present,
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
        &config.max_chars,
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

fn parse_tool_input_variant_config(variant: &Variant) -> Result<ToolInputVariantConfig> {
    let mut action = None;
    let mut validate = None;
    let mut handle = None;
    let mut handle_with_context = None;
    let mut stream_handle = None;
    let mut stream_handle_with_context = None;
    let mut permission_paths_handle = None;
    let mut permission_networks_handle = None;
    let mut handle_by_value = false;
    let mut non_empty = Vec::new();
    let mut non_empty_if_present = Vec::new();
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
    let mut max_chars = Vec::new();
    let mut default_when_empty = false;
    let mut infer_when_present = Vec::new();
    let mut drop_keys = Vec::new();
    let mut action_aliases = Vec::new();
    let mut action_alias_defaults = Vec::new();
    for attr in &variant.attrs {
        if !attr.path().is_ident("tool") {
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
                        "exec" => action = Some(expr_lit_str(&value.value, "exec")?),
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
                                "ToolInputShape does not support #[tool(map = ...)]",
                            ));
                        }
                        other => {
                            return Err(syn::Error::new_spanned(
                                ident,
                                format!(
                                    "unsupported tool variant attribute '{other}' for ToolInputShape"
                                ),
                            ));
                        }
                    }
                }
                Meta::List(list) => {
                    let Some(ident) = list.path.get_ident() else {
                        return Err(syn::Error::new_spanned(list.path, "expected identifier"));
                    };
                    match ident.to_string().as_str() {
                        "non_empty" => non_empty.extend(parse_lit_str_list(list.tokens)?),
                        "non_empty_if_present" => {
                            non_empty_if_present.extend(parse_lit_str_list(list.tokens)?)
                        }
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
                        "max_chars" => {
                            max_chars.push(parse_path_usize_constraint(list.tokens, "max_chars")?)
                        }
                        "infer_when_present" => {
                            infer_when_present.extend(parse_lit_str_list(list.tokens)?)
                        }
                        "drop_keys" => drop_keys.extend(parse_lit_str_list(list.tokens)?),
                        "action_alias" => action_aliases.extend(parse_lit_str_list(list.tokens)?),
                        "action_alias_default" => {
                            action_alias_defaults.push(parse_action_alias_default(list.tokens)?)
                        }
                        other => {
                            return Err(syn::Error::new_spanned(
                                ident,
                                format!(
                                    "unsupported tool variant list '{other}' for ToolInputShape"
                                ),
                            ));
                        }
                    }
                }
                Meta::Path(path) => {
                    return Err(syn::Error::new_spanned(
                        path,
                        "unsupported bare tool argument",
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
        non_empty,
        non_empty_if_present,
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
        max_chars,
        default_when_empty,
        infer_when_present,
        drop_keys,
        action_aliases,
        action_alias_defaults,
    })
}

fn parse_tool_suite_variant(variant: &Variant) -> Result<ToolSuiteVariant> {
    let ty = match &variant.fields {
        Fields::Unnamed(fields) if fields.unnamed.len() == 1 => {
            fields.unnamed.first().expect("one field").ty.clone()
        }
        _ => {
            return Err(syn::Error::new_spanned(
                variant,
                "ToolSuite variants must have exactly one unnamed field",
            ));
        }
    };
    let config = parse_tool_suite_variant_config(&variant.attrs)?;
    Ok(ToolSuiteVariant {
        ident: variant.ident.clone(),
        ty,
        config,
    })
}

fn parse_tool_suite_variant_config(attrs: &[Attribute]) -> Result<ToolSuiteVariantConfig> {
    let mut config = ToolSuiteVariantConfig {
        parse: None,
        resolve: None,
        route: None,
        route_action: None,
        field: None,
        convert: None,
        shape: None,
    };
    for attr in attrs {
        if !attr.path().is_ident("tool") {
            continue;
        }
        let metas = attr.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)?;
        for meta in metas {
            let Meta::NameValue(value) = meta else {
                return Err(syn::Error::new_spanned(
                    attr,
                    "tool attributes on ToolSuite variants must use name = value syntax",
                ));
            };
            let Some(ident) = value.path.get_ident() else {
                return Err(syn::Error::new_spanned(value.path, "expected identifier"));
            };
            match ident.to_string().as_str() {
                "parse" => config.parse = Some(expr_path(&value.value, "parse")?),
                "resolve" => config.resolve = Some(expr_path(&value.value, "resolve")?),
                "route" => config.route = Some(expr_lit_str(&value.value, "route")?),
                "route_action" => {
                    config.route_action = Some(expr_lit_str(&value.value, "route_action")?)
                }
                "field" => config.field = Some(expr_path(&value.value, "field")?),
                "convert" => config.convert = Some(expr_path(&value.value, "convert")?),
                "shape" => config.shape = Some(expr_path(&value.value, "shape")?),
                other => {
                    return Err(syn::Error::new_spanned(
                        ident,
                        format!("unsupported ToolSuite tool attribute '{other}'"),
                    ));
                }
            }
        }
    }
    if config.resolve.is_some()
        && (config.route.is_some()
            || config.route_action.is_some()
            || config.field.is_some()
            || config.convert.is_some()
            || config.shape.is_some())
    {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            "ToolSuite variants cannot combine resolve with route/route_action/field/convert/shape",
        ));
    }
    if config.route.is_none()
        && (config.route_action.is_some()
            || config.field.is_some()
            || config.convert.is_some()
            || config.shape.is_some())
    {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            "ToolSuite variants using route_action, field, convert, or shape must also declare route",
        ));
    }
    if config.field.is_some() && config.convert.is_some() {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            "ToolSuite variants cannot combine field and convert",
        ));
    }
    Ok(config)
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

fn expand_variant_arm(variant: &Variant) -> Result<proc_macro2::TokenStream> {
    let config = parse_tool_variant_config(variant)?;
    let variant_name = &variant.ident;
    let (pattern, value_expr) = single_field_pattern_and_value(variant)?;

    Ok(match config.mapping {
        VariantMapping::Exec(tool_name) => {
            let route_expr = if let Some(route) = config.route.as_ref() {
                let raw_payload_expr = if let Some(path) = config.convert.as_ref() {
                    quote! {
                        serde_json::to_value(#path(value)?)
                            .map_err(|err| ::agena_plugin_sdk::PluginError::invalid_params(err.to_string()))?
                    }
                } else if let Some(field) = config.field.as_ref() {
                    let field_ident = single_segment_ident(field, "field")?;
                    quote! {
                        serde_json::to_value(value.#field_ident)
                            .map_err(|err| ::agena_plugin_sdk::PluginError::invalid_params(err.to_string()))?
                    }
                } else {
                    value_expr.clone()
                };
                let action_injected_payload_expr =
                    if let Some(route_action) = config.route_action.as_ref() {
                        quote! {{
                            let mut routed_value = #raw_payload_expr;
                            match &mut routed_value {
                                serde_json::Value::Object(object) => {
                                    object.insert(
                                        "action".to_string(),
                                        serde_json::Value::String(#route_action.to_string()),
                                    );
                                    routed_value
                                }
                                _ => {
                                    return Err(::agena_plugin_sdk::PluginError::invalid_params(
                                        "route_action requires an object-shaped routed payload",
                                    ));
                                }
                            }
                        }}
                    } else {
                        raw_payload_expr
                    };
                let payload_expr = if let Some(shape) = config.shape.as_ref() {
                    quote! {{
                        let routed_value = #action_injected_payload_expr;
                        let routed_input = <#shape as ::agena_plugin_sdk::ToolInputShape>::parse_input(routed_value)?;
                        serde_json::to_value(routed_input)
                            .map_err(|err| ::agena_plugin_sdk::PluginError::invalid_params(err.to_string()))?
                    }}
                } else {
                    action_injected_payload_expr
                };
                quote! {
                    Ok((
                        #route.to_string(),
                        #payload_expr,
                    ))
                }
            } else {
                quote! {
                    Ok((
                        #tool_name.to_string(),
                        #value_expr,
                    ))
                }
            };
            quote! {
                Self::#variant_name #pattern => #route_expr
            }
        }
        VariantMapping::Map(path) => quote! {
            Self::#variant_name #pattern => #path(value)
        },
    })
}

fn expand_enum_input_normalize_fn(
    variants: &Punctuated<Variant, Token![,]>,
) -> Result<proc_macro2::TokenStream> {
    struct EnumNormalizeVariant {
        action: LitStr,
        default_when_empty: bool,
        infer_when_present: Vec<LitStr>,
        drop_keys: Vec<LitStr>,
        action_aliases: Vec<LitStr>,
        action_alias_defaults: Vec<ActionAliasDefaultConfig>,
    }

    let mut normalize_variants = Vec::new();
    let mut action_candidates = Vec::new();
    for variant in variants {
        let config = parse_tool_variant_config(variant)?;
        action_candidates.push(LitStr::new(
            &ident_to_snake_case(&variant.ident),
            variant.ident.span(),
        ));
        action_candidates.extend(
            variant_action_aliases(variant)?
                .into_iter()
                .map(|alias| LitStr::new(alias.as_str(), variant.ident.span())),
        );
        if config.default_when_empty
            || !config.infer_when_present.is_empty()
            || !config.drop_keys.is_empty()
            || !config.action_aliases.is_empty()
            || !config.action_alias_defaults.is_empty()
        {
            let VariantMapping::Exec(action) = config.mapping else {
                return Err(syn::Error::new_spanned(
                    &variant.ident,
                    "default_when_empty, infer_when_present, drop_keys, and action aliases require #[tool(exec = \"...\")]",
                ));
            };
            normalize_variants.push(EnumNormalizeVariant {
                action,
                default_when_empty: config.default_when_empty,
                infer_when_present: config.infer_when_present,
                drop_keys: config.drop_keys,
                action_aliases: config.action_aliases,
                action_alias_defaults: config.action_alias_defaults,
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
                "only one #[tool(default_when_empty = true)] variant is allowed, found {}",
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

    let action_alias_match_arms = normalize_variants.iter().flat_map(|variant| {
        let action = &variant.action;
        let simple_aliases = variant.action_aliases.iter().map(|alias| {
            quote! {
                #alias => {
                    object.insert(
                        "action".to_string(),
                        serde_json::Value::String(#action.to_string()),
                    );
                    #action.to_string()
                }
            }
        });
        let alias_defaults = variant.action_alias_defaults.iter().map(|alias_default| {
            let alias = &alias_default.alias;
            let defaults = alias_default.defaults.iter().map(|(field, value)| {
                quote! {
                    if !object.contains_key(#field) {
                        object.insert(#field.to_string(), serde_json::json!(#value));
                    }
                }
            });
            quote! {
                #alias => {
                    object.insert(
                        "action".to_string(),
                        serde_json::Value::String(#action.to_string()),
                    );
                    #(#defaults)*
                    #action.to_string()
                }
            }
        });
        simple_aliases.chain(alias_defaults).collect::<Vec<_>>()
    });

    let infer_match_exprs = normalize_variants
        .iter()
        .filter(|variant| !variant.infer_when_present.is_empty())
        .map(|variant| {
            let action = &variant.action;
            let keys = &variant.infer_when_present;
            quote! {
                if inferred_action.is_none() && [#(#keys),*].iter().any(|key| object.contains_key(*key)) {
                    inferred_action = Some(#action);
                }
            }
        });

    let drop_match_arms = normalize_variants
        .iter()
        .filter(|variant| !variant.drop_keys.is_empty())
        .map(|variant| {
            let action = &variant.action;
            let keys = &variant.drop_keys;
            quote! {
                #action => {
                    for key in [#(#keys),*] {
                        object.remove(key);
                    }
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
                    #(#action_alias_match_arms,)*
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

            Ok(serde_json::Value::Object(object))
        }
    })
}

fn expand_variant_validation_arm(variant: &Variant) -> Result<Option<proc_macro2::TokenStream>> {
    let config = parse_tool_variant_config(variant)?;
    let has_built_in_validation = !config.non_empty.is_empty()
        || !config.non_empty_if_present.is_empty()
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
        || !config.max_chars.is_empty();
    if config.validate.is_none() && !has_built_in_validation {
        return Ok(None);
    }
    let variant_name = &variant.ident;
    let built_in_validate_expr = built_in_validation_tokens(
        quote! { value },
        &config.non_empty,
        &config.non_empty_if_present,
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
        &config.max_chars,
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
                            "named tool variant field is missing identifier",
                        )
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            let field_inserts = named_field_object_insert_tokens(
                fields.named.iter().zip(bindings.iter()),
                "flattened tool variant fields must serialize to objects",
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
                "tool variant validate hooks are not supported on unit variants",
            ));
        }
    };
    Ok(Some(arm))
}

fn single_field_pattern_and_value(
    variant: &Variant,
) -> Result<(proc_macro2::TokenStream, proc_macro2::TokenStream)> {
    match &variant.fields {
        Fields::Named(fields) => {
            let bindings = fields
                .named
                .iter()
                .map(|field| {
                    field.ident.clone().ok_or_else(|| {
                        syn::Error::new_spanned(
                            field,
                            "named tool variant field is missing identifier",
                        )
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            let field_inserts = named_field_object_insert_tokens(
                fields.named.iter().zip(bindings.iter()),
                "flattened tool variant fields must serialize to objects",
            )?;
            Ok((
                quote! {{ #(#bindings),* }},
                quote! {{
                    let mut object = serde_json::Map::new();
                    #(#field_inserts)*
                    serde_json::Value::Object(object)
                }},
            ))
        }
        Fields::Unnamed(fields) => {
            if fields.unnamed.len() == 1 {
                Ok((
                    quote! {(value)},
                    quote! {
                        serde_json::to_value(value)
                            .map_err(|err| ::agena_plugin_sdk::PluginError::invalid_params(err.to_string()))?
                    },
                ))
            } else {
                let bindings = fields
                    .unnamed
                    .iter()
                    .enumerate()
                    .map(|(index, _)| format_ident!("value_{index}"))
                    .collect::<Vec<_>>();
                Ok((
                    quote! {(#(#bindings),*)},
                    quote! {
                        serde_json::to_value((#(#bindings,)*))
                            .map_err(|err| ::agena_plugin_sdk::PluginError::invalid_params(err.to_string()))?
                    },
                ))
            }
        }
        Fields::Unit => Ok((
            quote! {},
            quote! { serde_json::Value::Object(serde_json::Map::new()) },
        )),
    }
}

fn expand_static_surface_dispatch_fn(
    data: &Data,
    surface: &SurfaceConfig,
) -> Result<proc_macro2::TokenStream> {
    let receiver_ty = surface.handler_receiver.as_ref();
    let struct_handle = surface.handle.as_ref();
    let struct_handle_with_context = surface.handle_with_context.as_ref();
    let struct_stream_handle = surface.stream_handle.as_ref();
    let struct_stream_handle_with_context = surface.stream_handle_with_context.as_ref();
    let struct_permission_paths_handle = surface.permission_paths_handle.as_ref();
    let struct_permission_networks_handle = surface.permission_networks_handle.as_ref();
    if receiver_ty.is_none() {
        if struct_handle.is_some()
            || struct_handle_with_context.is_some()
            || struct_stream_handle.is_some()
            || struct_stream_handle_with_context.is_some()
            || struct_permission_paths_handle.is_some()
            || struct_permission_networks_handle.is_some()
            || surface.handle_field.is_some()
            || surface.handle_by_value == Some(true)
        {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                "handle/handle_with_context/stream_handle/stream_handle_with_context/permission_paths_handle/permission_networks_handle/handle_field/handle_by_value require handler_receiver on the surface",
            ));
        }
    }

    match data {
        Data::Struct(_) => {
            if struct_handle.is_some() && struct_handle_with_context.is_some() {
                return Err(syn::Error::new(
                    proc_macro2::Span::call_site(),
                    "StaticToolSurface structs cannot combine handle and handle_with_context",
                ));
            }
            if struct_stream_handle.is_some() && struct_stream_handle_with_context.is_some() {
                return Err(syn::Error::new(
                    proc_macro2::Span::call_site(),
                    "StaticToolSurface structs cannot combine stream_handle and stream_handle_with_context",
                ));
            }
            let Some(receiver_ty) = receiver_ty else {
                if struct_handle.is_some()
                    || struct_handle_with_context.is_some()
                    || struct_stream_handle.is_some()
                    || struct_stream_handle_with_context.is_some()
                    || struct_permission_paths_handle.is_some()
                    || struct_permission_networks_handle.is_some()
                    || surface.handle_by_value == Some(true)
                {
                    return Err(syn::Error::new(
                        proc_macro2::Span::call_site(),
                        "handle/handle_with_context/stream_handle/stream_handle_with_context/permission_paths_handle/permission_networks_handle/handle_by_value on a struct surface require handler_receiver",
                    ));
                }
                return Ok(quote! {});
            };
            let handle_by_value = surface.handle_by_value.unwrap_or(false);
            let arg_expr = if let Some(field) = surface.handle_field.as_ref() {
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
                    parse_tool_variant_config(variant)
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
                        "variant handle, handle_with_context, stream_handle, stream_handle_with_context, permission_paths_handle, or permission_networks_handle bindings require handler_receiver on the surface",
                    ));
                }
                return Ok(quote! {});
            }
            let receiver_ty = receiver_ty.expect("checked above");
            let mut plain_dispatch_arms = Vec::new();
            let mut context_dispatch_arms = Vec::new();
            let mut plain_stream_dispatch_arms = Vec::new();
            let mut context_stream_dispatch_arms = Vec::new();
            let mut saw_any_handle = false;
            let mut saw_context_handle = false;
            let mut can_generate_plain = true;
            let mut can_generate_context = true;
            let mut saw_any_stream_handle = false;
            let mut saw_context_stream_handle = false;
            let mut can_generate_plain_stream = true;
            let mut can_generate_context_stream = true;
            let mut permission_paths_dispatch_arms = Vec::<proc_macro2::TokenStream>::new();
            let mut permission_networks_dispatch_arms = Vec::<proc_macro2::TokenStream>::new();
            let mut saw_any_permission_paths_handle = false;
            let mut saw_any_permission_networks_handle = false;
            let mut saw_missing_permission_paths_handle = false;
            let mut saw_missing_permission_networks_handle = false;
            for variant in &data_enum.variants {
                let config = parse_tool_variant_config(variant)?;
                if config.handle_by_value
                    && config.handle.is_none()
                    && config.handle_with_context.is_none()
                    && config.stream_handle.is_none()
                    && config.stream_handle_with_context.is_none()
                {
                    return Err(syn::Error::new_spanned(
                        variant,
                        "variant handle_by_value requires #[tool(handle = path)], #[tool(handle_with_context = path)], #[tool(stream_handle = path)], or #[tool(stream_handle_with_context = path)]",
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
                }
                if plain_handle.is_none() && context_handle.is_none() {
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
                    "surface dispatch requires #[tool(handle = path)] on every variant",
                ));
            }
            if saw_context_handle && !can_generate_context {
                return Err(syn::Error::new(
                    proc_macro2::Span::call_site(),
                    "context-aware surface dispatch requires #[tool(handle = path)] or #[tool(handle_with_context = path)] on every variant",
                ));
            }
            if saw_any_stream_handle && !can_generate_plain_stream && !saw_context_stream_handle {
                return Err(syn::Error::new(
                    proc_macro2::Span::call_site(),
                    "surface stream dispatch requires #[tool(stream_handle = path)], #[tool(stream_handle_with_context = path)], #[tool(handle = path)], or #[tool(handle_with_context = path)] on every variant",
                ));
            }
            if saw_context_stream_handle && !can_generate_context_stream {
                return Err(syn::Error::new(
                    proc_macro2::Span::call_site(),
                    "context-aware surface stream dispatch requires #[tool(stream_handle = path)], #[tool(stream_handle_with_context = path)], #[tool(handle = path)], or #[tool(handle_with_context = path)] on every variant",
                ));
            }
            if saw_any_permission_paths_handle && saw_missing_permission_paths_handle {
                return Err(syn::Error::new(
                    proc_macro2::Span::call_site(),
                    "surface permission path dispatch requires #[tool(permission_paths_handle = path)] on every variant",
                ));
            }
            if saw_any_permission_networks_handle && saw_missing_permission_networks_handle {
                return Err(syn::Error::new(
                    proc_macro2::Span::call_site(),
                    "surface permission network dispatch requires #[tool(permission_networks_handle = path)] on every variant",
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
            let permission_paths_fn = if permission_paths_dispatch_arms.is_empty() {
                quote! {
                    pub async fn dispatch_permission_paths(
                        self,
                        _receiver: &#receiver_ty,
                    ) -> ::agena_plugin_sdk::Result<Vec<::agena_plugin_sdk::PathRequest>> {
                        Ok(Vec::new())
                    }
                }
            } else {
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
            };
            let permission_networks_fn = if permission_networks_dispatch_arms.is_empty() {
                quote! {
                    pub async fn dispatch_permission_networks(
                        self,
                        _receiver: &#receiver_ty,
                    ) -> ::agena_plugin_sdk::Result<Vec<::agena_plugin_sdk::NetworkRequest>> {
                        Ok(Vec::new())
                    }
                }
            } else {
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

fn parse_surface_config(attrs: &[Attribute]) -> Result<SurfaceConfig> {
    let mut config = SurfaceConfig::default();
    for attr in attrs {
        if !attr.path().is_ident("tool_surface") && !attr.path().is_ident("tool_command") {
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
                        "unsupported bare tool command argument",
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
        "alias" => {
            return Err(syn::Error::new_spanned(
                value.path,
                "tool aliases were removed; use the canonical tool name",
            ));
        }
        "visible_alias" => {
            return Err(syn::Error::new_spanned(
                value.path,
                "tool aliases were removed; use the canonical tool name",
            ));
        }
        "description" => config.description = Some(expr_lit_str(&value.value, "description")?),
        "long_about" => config.description = Some(expr_lit_str(&value.value, "long_about")?),
        "summary" => config.summary = Some(expr_lit_str(&value.value, "summary")?),
        "about" => config.summary = Some(expr_lit_str(&value.value, "about")?),
        "help" => config.help = Some(expr_lit_str(&value.value, "help")?),
        "long_help" => config.help = Some(expr_lit_str(&value.value, "long_help")?),
        "after_help" => config.after_help = Some(expr_lit_str(&value.value, "after_help")?),
        "after_long_help" => {
            config.after_help = Some(expr_lit_str(&value.value, "after_long_help")?)
        }
        "before_help" => config.before_help = Some(expr_lit_str(&value.value, "before_help")?),
        "before_long_help" => {
            config.before_help = Some(expr_lit_str(&value.value, "before_long_help")?)
        }
        "handler_receiver" => {
            config.handler_receiver = Some(expr_path(&value.value, "handler_receiver")?)
        }
        "handle" => config.handle = Some(expr_path(&value.value, "handle")?),
        "handle_with_context" => {
            config.handle_with_context = Some(expr_path(&value.value, "handle_with_context")?)
        }
        "stream_handle" => config.stream_handle = Some(expr_path(&value.value, "stream_handle")?),
        "stream_handle_with_context" => {
            config.stream_handle_with_context =
                Some(expr_path(&value.value, "stream_handle_with_context")?)
        }
        "permission_paths_handle" => {
            config.permission_paths_handle =
                Some(expr_path(&value.value, "permission_paths_handle")?)
        }
        "permission_networks_handle" => {
            config.permission_networks_handle =
                Some(expr_path(&value.value, "permission_networks_handle")?)
        }
        "handle_field" => config.handle_field = Some(expr_path(&value.value, "handle_field")?),
        "handle_by_value" => {
            config.handle_by_value = Some(expr_lit_bool(&value.value, "handle_by_value")?)
        }
        "normalize" => config.normalize = Some(expr_path(&value.value, "normalize")?),
        "validate" => config.validate = Some(expr_path(&value.value, "validate")?),
        "display" => config.display = Some(expr_string_like(&value.value, "display")?),
        "ui_display" => config.ui_display = Some(expr_string_like(&value.value, "ui_display")?),
        "description_mode" => {
            config.description_mode = Some(expr_string_like(&value.value, "description_mode")?)
        }
        "ui_display_mode" => {
            config.ui_display_mode = Some(expr_string_like(&value.value, "ui_display_mode")?)
        }
        "concurrency_safe" => {
            config.concurrency_safe = Some(expr_lit_bool(&value.value, "concurrency_safe")?)
        }
        "strict" => config.strict = Some(expr_lit_bool(&value.value, "strict")?),
        "streaming" => config.streaming = Some(expr_string_like(&value.value, "streaming")?),
        other => {
            return Err(syn::Error::new_spanned(
                ident,
                format!("unsupported tool command argument '{other}'"),
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
        "trim" => {
            config.trim.extend(parse_lit_str_list(list.tokens)?);
        }
        "trim_suffix" => {
            config
                .trim_suffix
                .push(parse_path_lit_str_constraint(list.tokens, "trim_suffix")?);
        }
        "non_empty" => {
            config.non_empty.extend(parse_lit_str_list(list.tokens)?);
        }
        "non_empty_if_present" => {
            config
                .non_empty_if_present
                .extend(parse_lit_str_list(list.tokens)?);
        }
        "exactly_one_of" => {
            config.exactly_one_of.push(parse_lit_str_list(list.tokens)?);
        }
        "at_least_one_of" => {
            config
                .at_least_one_of
                .push(parse_lit_str_list(list.tokens)?);
        }
        "examples" => {
            config.examples.extend(parse_lit_str_list(list.tokens)?);
        }
        "aliases" | "visible_aliases" => {
            return Err(syn::Error::new_spanned(
                list.path,
                "tool aliases were removed; use the canonical tool name",
            ));
        }
        "requires" => {
            config
                .requires
                .push(parse_path_pair_constraint(list.tokens, "requires")?);
        }
        "conflicts_with" => {
            config
                .conflicts_with
                .push(parse_path_pair_constraint(list.tokens, "conflicts_with")?);
        }
        "required_unless_present" => {
            config
                .required_unless_present
                .push(parse_path_pair_constraint(
                    list.tokens,
                    "required_unless_present",
                )?);
        }
        "forbid_substrings" => {
            config
                .forbid_substrings
                .push(parse_path_lit_str_list_constraint(
                    list.tokens,
                    "forbid_substrings",
                )?);
        }
        "distinct_trimmed" => {
            config
                .distinct_trimmed
                .extend(parse_lit_str_list(list.tokens)?);
        }
        "distinct_trimmed_within" => {
            config
                .distinct_trimmed_within
                .push(parse_path_pair_constraint(
                    list.tokens,
                    "distinct_trimmed_within",
                )?);
        }
        "min_items" => {
            config
                .min_items
                .push(parse_path_usize_constraint(list.tokens, "min_items")?);
        }
        "max_items" => {
            config
                .max_items
                .push(parse_path_usize_constraint(list.tokens, "max_items")?);
        }
        "max_chars" => {
            config
                .max_chars
                .push(parse_path_usize_constraint(list.tokens, "max_chars")?);
        }
        "tags" => {
            config.tags = parse_expr_list(list.tokens)?;
        }
        "capabilities" => {
            config.capabilities = parse_expr_list(list.tokens)?;
        }
        other => {
            return Err(syn::Error::new_spanned(
                ident,
                format!("unsupported tool command list '{other}'"),
            ));
        }
    }
    Ok(())
}

fn parse_tool_variant_config(variant: &Variant) -> Result<ToolVariantConfig> {
    let mut mapping = None;
    let mut route = None;
    let mut route_action = None;
    let mut handle = None;
    let mut handle_with_context = None;
    let mut stream_handle = None;
    let mut stream_handle_with_context = None;
    let mut permission_paths_handle = None;
    let mut permission_networks_handle = None;
    let mut handle_by_value = false;
    let mut field = None;
    let mut convert = None;
    let mut shape = None;
    let mut validate = None;
    let mut non_empty = Vec::new();
    let mut non_empty_if_present = Vec::new();
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
    let mut max_chars = Vec::new();
    let mut default_when_empty = false;
    let mut infer_when_present = Vec::new();
    let mut drop_keys = Vec::new();
    let mut action_aliases = Vec::new();
    let mut action_alias_defaults = Vec::new();
    for attr in &variant.attrs {
        if !attr.path().is_ident("tool") {
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
                        "exec" => {
                            mapping =
                                Some(VariantMapping::Exec(expr_lit_str(&value.value, "exec")?))
                        }
                        "map" => {
                            mapping = Some(VariantMapping::Map(expr_path(&value.value, "map")?))
                        }
                        "route" => route = Some(expr_lit_str(&value.value, "route")?),
                        "route_action" => {
                            route_action = Some(expr_lit_str(&value.value, "route_action")?)
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
                        "handle_by_value" => {
                            handle_by_value = expr_lit_bool(&value.value, "handle_by_value")?
                        }
                        "field" => field = Some(expr_path(&value.value, "field")?),
                        "convert" => convert = Some(expr_path(&value.value, "convert")?),
                        "shape" => shape = Some(expr_path(&value.value, "shape")?),
                        "validate" => validate = Some(expr_path(&value.value, "validate")?),
                        "default_when_empty" => {
                            default_when_empty = expr_lit_bool(&value.value, "default_when_empty")?
                        }
                        other => {
                            return Err(syn::Error::new_spanned(
                                ident,
                                format!("unsupported tool attribute '{other}'"),
                            ));
                        }
                    }
                }
                Meta::List(list) => {
                    let Some(ident) = list.path.get_ident() else {
                        return Err(syn::Error::new_spanned(list.path, "expected identifier"));
                    };
                    match ident.to_string().as_str() {
                        "non_empty" => non_empty.extend(parse_lit_str_list(list.tokens)?),
                        "non_empty_if_present" => {
                            non_empty_if_present.extend(parse_lit_str_list(list.tokens)?)
                        }
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
                        "max_chars" => {
                            max_chars.push(parse_path_usize_constraint(list.tokens, "max_chars")?)
                        }
                        "infer_when_present" => {
                            infer_when_present.extend(parse_lit_str_list(list.tokens)?)
                        }
                        "drop_keys" => drop_keys.extend(parse_lit_str_list(list.tokens)?),
                        "action_alias" => action_aliases.extend(parse_lit_str_list(list.tokens)?),
                        "action_alias_default" => {
                            action_alias_defaults.push(parse_action_alias_default(list.tokens)?)
                        }
                        other => {
                            return Err(syn::Error::new_spanned(
                                ident,
                                format!("unsupported tool list '{other}'"),
                            ));
                        }
                    }
                }
                Meta::Path(path) => {
                    return Err(syn::Error::new_spanned(
                        path,
                        "unsupported bare tool argument",
                    ));
                }
            }
        }
    }
    let Some(mapping) = mapping else {
        return Err(syn::Error::new_spanned(
            variant,
            "missing #[tool(exec = \"...\")] or #[tool(map = path)] on variant",
        ));
    };
    if matches!(mapping, VariantMapping::Map(_))
        && (route.is_some()
            || route_action.is_some()
            || field.is_some()
            || convert.is_some()
            || shape.is_some())
    {
        return Err(syn::Error::new_spanned(
            variant,
            "StaticToolSurface variants using #[tool(map = ...)] cannot combine route/route_action/field/convert/shape",
        ));
    }
    if route.is_none()
        && (route_action.is_some() || field.is_some() || convert.is_some() || shape.is_some())
    {
        return Err(syn::Error::new_spanned(
            variant,
            "StaticToolSurface variants using route_action, field, convert, or shape must also declare route",
        ));
    }
    if field.is_some() && convert.is_some() {
        return Err(syn::Error::new_spanned(
            variant,
            "StaticToolSurface variants cannot combine field and convert",
        ));
    }
    if handle.is_some() && handle_with_context.is_some() {
        return Err(syn::Error::new_spanned(
            variant,
            "StaticToolSurface variants cannot combine handle and handle_with_context",
        ));
    }
    Ok(ToolVariantConfig {
        mapping,
        route,
        route_action,
        handle,
        handle_with_context,
        stream_handle,
        stream_handle_with_context,
        permission_paths_handle,
        permission_networks_handle,
        handle_by_value,
        field,
        convert,
        shape,
        validate,
        non_empty,
        non_empty_if_present,
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
        max_chars,
        default_when_empty,
        infer_when_present,
        drop_keys,
        action_aliases,
        action_alias_defaults,
    })
}

fn parse_action_alias_default(
    tokens: proc_macro2::TokenStream,
) -> Result<ActionAliasDefaultConfig> {
    let items = Punctuated::<Expr, Token![,]>::parse_terminated.parse2(tokens)?;
    let mut iter = items.into_iter();
    let Some(first) = iter.next() else {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            "action_alias_default requires at least one alias string",
        ));
    };
    let alias = expr_lit_str(&first, "action alias")?;
    let mut defaults = Vec::new();
    for expr in iter {
        let Expr::Assign(assign) = expr else {
            return Err(syn::Error::new_spanned(
                expr,
                "action_alias_default defaults must use field = value syntax",
            ));
        };
        let left = assign.left.as_ref();
        let Expr::Path(path) = left else {
            return Err(syn::Error::new_spanned(
                left,
                "action_alias_default field must be an identifier",
            ));
        };
        let Some(field_ident) = path.path.get_ident() else {
            return Err(syn::Error::new_spanned(
                &path.path,
                "action_alias_default field must be an identifier",
            ));
        };
        defaults.push((
            LitStr::new(&field_ident.to_string(), field_ident.span()),
            *assign.right,
        ));
    }
    Ok(ActionAliasDefaultConfig { alias, defaults })
}

fn tool_input_variant_action_name(variant: &Variant, config: &ToolInputVariantConfig) -> LitStr {
    config
        .action
        .clone()
        .unwrap_or_else(|| LitStr::new(&ident_to_snake_case(&variant.ident), variant.ident.span()))
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

fn built_in_normalization_tokens(
    target: proc_macro2::TokenStream,
    trim: &[LitStr],
    trim_suffix: &[PathStringConstraint],
) -> proc_macro2::TokenStream {
    let trim_expr = if trim.is_empty() {
        quote! {}
    } else {
        quote! {
            ::agena_plugin_sdk::macro_support::normalize_trim_paths(
                #target,
                &[#(#trim),*],
            );
        }
    };
    let trim_suffix_exprs = trim_suffix.iter().map(|constraint| {
        let path = &constraint.path;
        let value = &constraint.value;
        quote! {
            ::agena_plugin_sdk::macro_support::normalize_trim_suffix_path(
                #target,
                #path,
                #value,
            );
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
) -> proc_macro2::TokenStream {
    if trim.is_empty() && trim_suffix.is_empty() {
        quote! { parsed }
    } else {
        let normalize_expr = built_in_normalization_tokens(quote! { input }, trim, trim_suffix);
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
    max_chars: &[PathUsizeConstraint],
) -> proc_macro2::TokenStream {
    let non_empty_expr = if non_empty.is_empty() {
        quote! {}
    } else {
        quote! {
            ::agena_plugin_sdk::macro_support::validate_non_empty_paths(
                &#target,
                &[#(#non_empty),*],
            )?;
        }
    };
    let non_empty_if_present_expr = if non_empty_if_present.is_empty() {
        quote! {}
    } else {
        quote! {
            ::agena_plugin_sdk::macro_support::validate_non_empty_if_present_paths(
                &#target,
                &[#(#non_empty_if_present),*],
            )?;
        }
    };
    let exactly_one_of_exprs = exactly_one_of.iter().map(|group| {
        quote! {
            ::agena_plugin_sdk::macro_support::validate_exactly_one_of_paths(
                &#target,
                &[#(#group),*],
            )?;
        }
    });
    let at_least_one_of_exprs = at_least_one_of.iter().map(|group| {
        quote! {
            ::agena_plugin_sdk::macro_support::validate_at_least_one_of_paths(
                &#target,
                &[#(#group),*],
            )?;
        }
    });
    let requires_exprs = requires.iter().map(|constraint| {
        let left = &constraint.left;
        let right = &constraint.right;
        quote! {
            ::agena_plugin_sdk::macro_support::validate_requires_path(
                &#target,
                #left,
                #right,
            )?;
        }
    });
    let conflicts_exprs = conflicts_with.iter().map(|constraint| {
        let left = &constraint.left;
        let right = &constraint.right;
        quote! {
            ::agena_plugin_sdk::macro_support::validate_conflicts_with_path(
                &#target,
                #left,
                #right,
            )?;
        }
    });
    let required_unless_exprs = required_unless_present.iter().map(|constraint| {
        let left = &constraint.left;
        let right = &constraint.right;
        quote! {
            ::agena_plugin_sdk::macro_support::validate_required_unless_present_path(
                &#target,
                #left,
                #right,
            )?;
        }
    });
    let forbid_substrings_exprs = forbid_substrings.iter().map(|constraint| {
        let path = &constraint.path;
        let values = &constraint.values;
        quote! {
            ::agena_plugin_sdk::macro_support::validate_forbid_substrings_path(
                &#target,
                #path,
                &[#(#values),*],
            )?;
        }
    });
    let distinct_trimmed_exprs = distinct_trimmed.iter().map(|path| {
        quote! {
            ::agena_plugin_sdk::macro_support::validate_distinct_trimmed_path(
                &#target,
                #path,
            )?;
        }
    });
    let distinct_trimmed_within_exprs = distinct_trimmed_within.iter().map(|constraint| {
        let path = &constraint.left;
        let scope = &constraint.right;
        quote! {
            ::agena_plugin_sdk::macro_support::validate_distinct_trimmed_within_path(
                &#target,
                #path,
                #scope,
            )?;
        }
    });
    let min_items_exprs = min_items.iter().map(|constraint| {
        let path = &constraint.path;
        let value = constraint.value;
        quote! {
            ::agena_plugin_sdk::macro_support::validate_min_items_path(
                &#target,
                #path,
                #value,
            )?;
        }
    });
    let max_items_exprs = max_items.iter().map(|constraint| {
        let path = &constraint.path;
        let value = constraint.value;
        quote! {
            ::agena_plugin_sdk::macro_support::validate_max_items_path(
                &#target,
                #path,
                #value,
            )?;
        }
    });
    let max_chars_exprs = max_chars.iter().map(|constraint| {
        let path = &constraint.path;
        let value = constraint.value;
        quote! {
            ::agena_plugin_sdk::macro_support::validate_max_chars_path(
                &#target,
                #path,
                #value,
            )?;
        }
    });
    quote! {
        #(#min_items_exprs)*
        #(#max_items_exprs)*
        #(#max_chars_exprs)*
        #non_empty_expr
        #non_empty_if_present_expr
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
