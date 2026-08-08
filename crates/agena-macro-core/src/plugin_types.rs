//! Shared plugin type aliases used by generated code.

use syn::parse::{Parse, ParseStream};
use syn::{Attribute, Expr, Ident, LitStr, Meta, Result, Token, Type};

#[derive(Clone)]
pub struct PluginToolPlan {
    pub tool: LitStr,
    pub input_model: PluginGeneratedToolInput,
    pub invoke: PluginToolInvokeHandler,
    pub stream: Option<PluginToolStreamHandler>,
    pub permissions: PluginToolPermissionHandlers,
    pub command: Option<PluginToolCommandConfig>,
}

#[derive(Clone)]
pub struct PluginToolInvokeHandler {
    pub method: Ident,
    pub output: PluginToolOutputPlan,
    pub is_async: bool,
    pub context: Option<PluginContextArg>,
    pub input: PluginCallInput,
}

#[derive(Clone)]
pub struct PluginToolOutputPlan {
    pub ty: Option<Type>,
    pub returns_result: bool,
}

#[derive(Clone)]
pub struct PluginToolStreamHandler {
    pub method: Ident,
    pub is_async: bool,
    pub sink_first: bool,
    pub context: Option<PluginContextArg>,
    pub input: PluginCallInput,
}

#[derive(Clone)]
pub struct PluginToolStreamSignature {
    pub method: Ident,
    pub is_async: bool,
    pub sink_first: bool,
}

#[derive(Clone, Default)]
pub struct PluginToolPermissionHandlers {
    pub path_rules: Vec<PluginToolPathPermissionRule>,
    pub network_rules: Vec<PluginToolNetworkPermissionRule>,
}

impl PluginToolPermissionHandlers {
    pub fn has_path_permissions(&self) -> bool {
        !self.path_rules.is_empty()
    }

    pub fn has_network_permissions(&self) -> bool {
        !self.network_rules.is_empty()
    }
}

#[derive(Clone)]
pub enum PluginToolPathPermissionRule {
    Read(Expr),
    Reads(Expr),
    Write(Expr),
    Writes(Expr),
    Requests(Expr),
}

#[derive(Clone)]
pub enum PluginToolNetworkPermissionRule {
    Connect(Expr),
    Connects(Expr),
    Requests(Expr),
}

#[derive(Clone, Copy)]
pub struct PluginContextArg {
    pub first: bool,
    pub by_ref: bool,
}

#[derive(Clone)]
pub enum PluginCallInput {
    Wrapped { by_ref: bool },
    Fields(Vec<Ident>),
}

#[derive(Clone)]
pub struct PluginGeneratedToolInput {
    pub input_ident: Option<Ident>,
    pub input_fields: Vec<PluginGeneratedInputField>,
    pub input_ty: Type,
    pub spec: crate::tool_spec_support::ToolSpecConfig,
    pub docs: Option<String>,
}

#[derive(Clone)]
pub struct PluginGeneratedInputField {
    pub ident: Ident,
    pub wire_name: LitStr,
    pub aliases: Vec<LitStr>,
    pub ty: Type,
    pub default: bool,
    pub default_expr: Option<Expr>,
    pub flatten_shape: bool,
    pub nested_shape: bool,
}

#[derive(Clone)]
pub struct PluginCommandPlan {
    pub id: LitStr,
    pub title: LitStr,
    pub description: LitStr,
    pub category: LitStr,
    pub slash: Option<LitStr>,
    pub aliases: Vec<LitStr>,
    pub usage: Option<LitStr>,
    pub location: LitStr,
    pub action: Option<Expr>,
    pub handler: PluginCommandHandlerPlan,
}

#[derive(Clone)]
pub enum PluginCommandHandlerPlan {
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
pub enum PluginCommandInputPlan {
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
pub struct PluginCommandMethodShape {
    pub input: PluginCommandInputPlan,
    pub context: Option<PluginContextArg>,
}

#[derive(Clone, Default)]
pub struct PluginToolCommandConfig {
    pub id: Option<LitStr>,
    pub title: Option<LitStr>,
    pub description: Option<LitStr>,
    pub category: Option<LitStr>,
    pub slash: Option<LitStr>,
    pub aliases: Vec<LitStr>,
    pub usage: Option<LitStr>,
    pub location: Option<LitStr>,
    pub submit_output_as_prompt: bool,
}

pub struct PluginInherentMethodAttrs {
    pub tools: Vec<PluginToolPlan>,
    pub hooks: Vec<crate::plugin_hooks::PluginHookPlan>,
    pub commands: Vec<PluginCommandPlan>,
}

pub struct PluginCommandAttrArgs {
    pub slash: Option<LitStr>,
    pub metas: Vec<Meta>,
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

pub fn plugin_attr_has_explicit_args(attr: &Attribute) -> bool {
    match &attr.meta {
        Meta::Path(_) => false,
        Meta::List(list) => !list.tokens.is_empty(),
        Meta::NameValue(_) => true,
    }
}

pub struct PluginToolAttrConfig {
    pub spec: crate::tool_spec_support::ToolSpecConfig,
    pub stream_method: Option<Ident>,
    pub permission_path_rules: Vec<PluginToolPathPermissionRule>,
    pub permission_network_rules: Vec<PluginToolNetworkPermissionRule>,
    pub command: Option<PluginToolCommandConfig>,
}

pub struct PluginToolMethodShape {
    pub input_model: PluginGeneratedToolInput,
    pub context: Option<PluginContextArg>,
    pub call_input: PluginCallInput,
    pub stream_arg_types: Vec<Type>,
    pub stream_method: Option<Ident>,
}

pub struct PluginMethodInfo {
    pub ident: Ident,
    pub is_async: bool,
    pub typed_args: Vec<Type>,
    pub shared_receiver: bool,
}

#[derive(Default)]
pub struct PluginArgConfig {
    pub default: bool,
    pub default_expr: Option<Expr>,
    pub description: Option<LitStr>,
    pub trim: bool,
    pub item_trim: bool,
    pub non_empty: bool,
    pub item_non_empty: bool,
    pub non_empty_if_present: bool,
    pub item_non_empty_if_present: bool,
    pub distinct_trimmed: bool,
    pub trim_suffix: Option<LitStr>,
    pub item_trim_suffix: Option<LitStr>,
    pub minimum: Option<Expr>,
    pub maximum: Option<Expr>,
    pub exclusive_minimum: Option<Expr>,
    pub exclusive_maximum: Option<Expr>,
    pub min_items: Option<usize>,
    pub max_items: Option<usize>,
    pub min_properties: Option<usize>,
    pub max_properties: Option<usize>,
    pub item_minimum: Option<Expr>,
    pub item_maximum: Option<Expr>,
    pub item_exclusive_minimum: Option<Expr>,
    pub item_exclusive_maximum: Option<Expr>,
    pub item_min_properties: Option<usize>,
    pub item_max_properties: Option<usize>,
    pub min_chars: Option<usize>,
    pub max_chars: Option<usize>,
    pub item_min_chars: Option<usize>,
    pub item_max_chars: Option<usize>,
    pub format: Option<LitStr>,
    pub item_format: Option<LitStr>,
    pub pattern: Option<LitStr>,
    pub item_pattern: Option<LitStr>,
    pub choices: Option<Vec<Expr>>,
    pub item_choices: Option<Vec<Expr>>,
    pub exactly_one_of: Vec<LitStr>,
    pub at_least_one_of: Vec<LitStr>,
    pub requires: Vec<LitStr>,
    pub conflicts_with: Vec<LitStr>,
    pub required_unless_present: Vec<LitStr>,
    pub forbid_substrings: Vec<LitStr>,
    pub distinct_trimmed_within: Vec<LitStr>,
    pub path: Option<PluginPathPermissionKind>,
    pub network: Option<PluginNetworkSemantic>,
    pub optional: bool,
    pub flatten_shape: bool,
    pub nested_shape: bool,
    pub jsonpath: Option<LitStr>,
    pub fallback: Option<LitStr>,
    pub name: Option<LitStr>,
    pub aliases: Vec<LitStr>,
    pub example: Option<Expr>,
    pub secret: bool,
    pub picker: Option<PluginPickerKind>,
}

#[derive(Clone, Copy)]
pub enum PluginPathPermissionKind {
    Read,
    Write,
}

#[derive(Clone, Copy)]
pub enum PluginNetworkSemantic {
    Network,
    Url,
    Host,
    Internet,
    Private,
}

#[derive(Clone, Copy)]
pub enum PluginPickerKind {
    File,
    Dir,
}

#[derive(Clone)]
pub struct PluginInputPathSpec {
    pub jsonpath: LitStr,
    pub kind: PluginPathPermissionKind,
    pub fallback: Option<LitStr>,
    pub optional: bool,
}

#[derive(Clone)]
pub struct PluginInputNetworkSpec {
    pub jsonpath: LitStr,
    pub fallback: Option<LitStr>,
    pub optional: bool,
    pub semantic: PluginNetworkSemantic,
}
