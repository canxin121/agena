use syn::parse::{Parse, ParseStream};
use syn::{Attribute, Expr, Ident, LitStr, Meta, Result, Token, Type};

#[derive(Clone)]
pub(crate) struct PluginToolPlan {
    pub(crate) tool: LitStr,
    pub(crate) input_model: PluginGeneratedToolInput,
    pub(crate) invoke: PluginToolInvokeHandler,
    pub(crate) stream: Option<PluginToolStreamHandler>,
    pub(crate) permissions: PluginToolPermissionHandlers,
    pub(crate) command: Option<PluginToolCommandConfig>,
}

#[derive(Clone)]
pub(crate) struct PluginToolInvokeHandler {
    pub(crate) method: Ident,
    pub(crate) output: PluginToolOutputPlan,
    pub(crate) is_async: bool,
    pub(crate) context: Option<PluginContextArg>,
    pub(crate) input: PluginCallInput,
}

#[derive(Clone)]
pub(crate) struct PluginToolOutputPlan {
    pub(crate) ty: Option<Type>,
    pub(crate) returns_result: bool,
}

#[derive(Clone)]
pub(crate) struct PluginToolStreamHandler {
    pub(crate) method: Ident,
    pub(crate) is_async: bool,
    pub(crate) sink_first: bool,
    pub(crate) context: Option<PluginContextArg>,
    pub(crate) input: PluginCallInput,
}

#[derive(Clone)]
pub(crate) struct PluginToolStreamSignature {
    pub(crate) method: Ident,
    pub(crate) is_async: bool,
    pub(crate) sink_first: bool,
}

#[derive(Clone, Default)]
pub(crate) struct PluginToolPermissionHandlers {
    pub(crate) path_rules: Vec<PluginToolPathPermissionRule>,
    pub(crate) network_rules: Vec<PluginToolNetworkPermissionRule>,
}

impl PluginToolPermissionHandlers {
    pub(crate) fn has_path_permissions(&self) -> bool {
        !self.path_rules.is_empty()
    }

    pub(crate) fn has_network_permissions(&self) -> bool {
        !self.network_rules.is_empty()
    }
}

#[derive(Clone)]
pub(crate) enum PluginToolPathPermissionRule {
    Read(Expr),
    Reads(Expr),
    Write(Expr),
    Writes(Expr),
    Requests(Expr),
}

#[derive(Clone)]
pub(crate) enum PluginToolNetworkPermissionRule {
    Connect(Expr),
    Connects(Expr),
    Requests(Expr),
}

#[derive(Clone, Copy)]
pub(crate) struct PluginContextArg {
    pub(crate) first: bool,
    pub(crate) by_ref: bool,
}

#[derive(Clone)]
pub(crate) enum PluginCallInput {
    Wrapped { by_ref: bool },
    Fields(Vec<Ident>),
}

#[derive(Clone)]
pub(crate) struct PluginGeneratedToolInput {
    pub(crate) input_ident: Option<Ident>,
    pub(crate) input_fields: Vec<PluginGeneratedInputField>,
    pub(crate) input_ty: Type,
    pub(crate) spec: crate::tool_spec_support::ToolSpecConfig,
    pub(crate) docs: Option<String>,
}

#[derive(Clone)]
pub(crate) struct PluginGeneratedInputField {
    pub(crate) ident: Ident,
    pub(crate) wire_name: LitStr,
    pub(crate) aliases: Vec<LitStr>,
    pub(crate) ty: Type,
    pub(crate) default: bool,
    pub(crate) default_expr: Option<Expr>,
    pub(crate) flatten_shape: bool,
    pub(crate) nested_shape: bool,
}

#[derive(Clone)]
pub(crate) struct PluginCommandPlan {
    pub(crate) id: LitStr,
    pub(crate) title: LitStr,
    pub(crate) description: LitStr,
    pub(crate) category: LitStr,
    pub(crate) slash: Option<LitStr>,
    pub(crate) aliases: Vec<LitStr>,
    pub(crate) usage: Option<LitStr>,
    pub(crate) location: LitStr,
    pub(crate) action: Option<Expr>,
    pub(crate) handler: PluginCommandHandlerPlan,
}

#[derive(Clone)]
pub(crate) enum PluginCommandHandlerPlan {
    Method {
        method: Ident,
        input: PluginCommandInputPlan,
        context: Option<PluginContextArg>,
        is_async: bool,
    },
    InvokeTool {
        tool: LitStr,
        input_model: Box<PluginGeneratedToolInput>,
        submit_output_as_prompt: bool,
    },
}

#[derive(Clone)]
pub(crate) enum PluginCommandInputPlan {
    None,
    Raw {
        by_ref: bool,
    },
    Typed {
        ty: Box<Type>,
        by_ref: bool,
    },
    Generated {
        input_model: Box<PluginGeneratedToolInput>,
        input: PluginCallInput,
    },
}

#[derive(Clone)]
pub(crate) struct PluginCommandMethodShape {
    pub(crate) input: PluginCommandInputPlan,
    pub(crate) context: Option<PluginContextArg>,
}

#[derive(Clone, Default)]
pub(crate) struct PluginToolCommandConfig {
    pub(crate) id: Option<LitStr>,
    pub(crate) title: Option<LitStr>,
    pub(crate) description: Option<LitStr>,
    pub(crate) category: Option<LitStr>,
    pub(crate) slash: Option<LitStr>,
    pub(crate) aliases: Vec<LitStr>,
    pub(crate) usage: Option<LitStr>,
    pub(crate) location: Option<LitStr>,
    pub(crate) submit_output_as_prompt: bool,
}

pub(crate) struct PluginInherentMethodAttrs {
    pub(crate) tools: Vec<PluginToolPlan>,
    pub(crate) hooks: Vec<crate::plugin_hooks::PluginHookPlan>,
    pub(crate) commands: Vec<PluginCommandPlan>,
}

pub(crate) struct PluginCommandAttrArgs {
    pub(crate) slash: Option<LitStr>,
    pub(crate) metas: Vec<Meta>,
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

pub(crate) fn plugin_attr_has_explicit_args(attr: &Attribute) -> bool {
    match &attr.meta {
        Meta::Path(_) => false,
        Meta::List(list) => !list.tokens.is_empty(),
        Meta::NameValue(_) => true,
    }
}

pub(crate) struct PluginToolAttrConfig {
    pub(crate) spec: crate::tool_spec_support::ToolSpecConfig,
    pub(crate) stream_method: Option<Ident>,
    pub(crate) permission_path_rules: Vec<PluginToolPathPermissionRule>,
    pub(crate) permission_network_rules: Vec<PluginToolNetworkPermissionRule>,
    pub(crate) command: Option<PluginToolCommandConfig>,
}

pub(crate) struct PluginToolMethodShape {
    pub(crate) input_model: PluginGeneratedToolInput,
    pub(crate) context: Option<PluginContextArg>,
    pub(crate) call_input: PluginCallInput,
    pub(crate) stream_arg_types: Vec<Type>,
    pub(crate) stream_method: Option<Ident>,
}

pub(crate) struct PluginMethodInfo {
    pub(crate) ident: Ident,
    pub(crate) is_async: bool,
    pub(crate) typed_args: Vec<Type>,
    pub(crate) shared_receiver: bool,
}

#[derive(Default)]
pub(crate) struct PluginArgConfig {
    pub(crate) default: bool,
    pub(crate) default_expr: Option<Expr>,
    pub(crate) description: Option<LitStr>,
    pub(crate) trim: bool,
    pub(crate) item_trim: bool,
    pub(crate) non_empty: bool,
    pub(crate) item_non_empty: bool,
    pub(crate) non_empty_if_present: bool,
    pub(crate) item_non_empty_if_present: bool,
    pub(crate) distinct_trimmed: bool,
    pub(crate) trim_suffix: Option<LitStr>,
    pub(crate) item_trim_suffix: Option<LitStr>,
    pub(crate) minimum: Option<Expr>,
    pub(crate) maximum: Option<Expr>,
    pub(crate) exclusive_minimum: Option<Expr>,
    pub(crate) exclusive_maximum: Option<Expr>,
    pub(crate) min_items: Option<usize>,
    pub(crate) max_items: Option<usize>,
    pub(crate) min_properties: Option<usize>,
    pub(crate) max_properties: Option<usize>,
    pub(crate) item_minimum: Option<Expr>,
    pub(crate) item_maximum: Option<Expr>,
    pub(crate) item_exclusive_minimum: Option<Expr>,
    pub(crate) item_exclusive_maximum: Option<Expr>,
    pub(crate) item_min_properties: Option<usize>,
    pub(crate) item_max_properties: Option<usize>,
    pub(crate) min_chars: Option<usize>,
    pub(crate) max_chars: Option<usize>,
    pub(crate) item_min_chars: Option<usize>,
    pub(crate) item_max_chars: Option<usize>,
    pub(crate) format: Option<LitStr>,
    pub(crate) item_format: Option<LitStr>,
    pub(crate) pattern: Option<LitStr>,
    pub(crate) item_pattern: Option<LitStr>,
    pub(crate) choices: Option<Vec<Expr>>,
    pub(crate) item_choices: Option<Vec<Expr>>,
    pub(crate) exactly_one_of: Vec<LitStr>,
    pub(crate) at_least_one_of: Vec<LitStr>,
    pub(crate) requires: Vec<LitStr>,
    pub(crate) conflicts_with: Vec<LitStr>,
    pub(crate) required_unless_present: Vec<LitStr>,
    pub(crate) forbid_substrings: Vec<LitStr>,
    pub(crate) distinct_trimmed_within: Vec<LitStr>,
    pub(crate) path: Option<PluginPathPermissionKind>,
    pub(crate) network: Option<PluginNetworkSemantic>,
    pub(crate) optional: bool,
    pub(crate) flatten_shape: bool,
    pub(crate) nested_shape: bool,
    pub(crate) jsonpath: Option<LitStr>,
    pub(crate) fallback: Option<LitStr>,
    pub(crate) name: Option<LitStr>,
    pub(crate) aliases: Vec<LitStr>,
    pub(crate) example: Option<Expr>,
    pub(crate) secret: bool,
    pub(crate) picker: Option<PluginPickerKind>,
}

#[derive(Clone, Copy)]
pub(crate) enum PluginPathPermissionKind {
    Read,
    Write,
}

#[derive(Clone, Copy)]
pub(crate) enum PluginNetworkSemantic {
    Network,
    Url,
    Host,
    Internet,
    Private,
}

#[derive(Clone, Copy)]
pub(crate) enum PluginPickerKind {
    File,
    Dir,
}

#[derive(Clone)]
pub(crate) struct PluginInputPathSpec {
    pub(crate) jsonpath: LitStr,
    pub(crate) kind: PluginPathPermissionKind,
    pub(crate) fallback: Option<LitStr>,
    pub(crate) optional: bool,
}

#[derive(Clone)]
pub(crate) struct PluginInputNetworkSpec {
    pub(crate) jsonpath: LitStr,
    pub(crate) fallback: Option<LitStr>,
    pub(crate) optional: bool,
    pub(crate) semantic: PluginNetworkSemantic,
}
