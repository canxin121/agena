use serde::{Deserialize, Serialize};

/// The fixed Tool API that Agena exposes through a model provider's
/// function-calling protocol.
///
/// Execution tools such as `session.rename` and internal tool keys such as
/// `agena.session.rename` are deliberately not represented by this type. A
/// model passes an execution tool's name to `tools_help` or `tools_call`; it
/// never uses that tool name as a provider function name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ToolApiFunction {
    #[serde(rename = "tools_list")]
    List,
    #[serde(rename = "tools_help")]
    Help,
    #[serde(rename = "tools_call")]
    Call,
    #[serde(rename = "plugins_list")]
    PluginsList,
}

/// Well-known field name inside [`ToolApiCall::arguments`] that carries a
/// corrective shape diagnostic when a provider's `tools_call` arguments could
/// not be interpreted as the required `{ tool, input }` object (for example
/// the arguments were a JSON-encoded string, a non-object value, or malformed
/// JSON). The session processor stamps this field onto the gateway invocation
/// and the executor surfaces its text in the rejection instead of a generic
/// missing-`tool` message.
pub const TOOLS_CALL_ARGUMENTS_DIAGNOSTIC_FIELD: &str = "__tools_call_arguments_diagnostic";

/// Function the automatic-approval model calls to submit an allow verdict.
/// See [`CONTROL_FUNCTION_NAMES`].
pub const APPROVE_ACTION_FUNCTION: &str = "approve_action";

/// Function the automatic-approval model calls to submit a block verdict.
/// See [`CONTROL_FUNCTION_NAMES`].
pub const BLOCK_ACTION_FUNCTION: &str = "block_action";

/// Provider-facing function names that Agena declares outside the Tool API
/// gateway because they carry a *decision* back to the runtime instead of
/// naming an execution tool.
///
/// A control function's call is read and dropped by the component that
/// declared it; it is never routed to the tool executor and never becomes a
/// transcript operation. That is the whole point of the shape: the decision
/// lives in the tool *name*, and the arguments are constrained by a schema, so
/// a model submits a verdict structurally instead of as prose a parser has to
/// guess at.
///
/// Membership is deliberately tiny and closed. Adding a name here widens what
/// can be advertised to a provider as a direct function call, which is exactly
/// what the Tool API gateway exists to prevent — an execution tool must reach
/// a provider only through `tools_call`, never as its own function name.
pub const CONTROL_FUNCTION_NAMES: [&str; 2] = [APPROVE_ACTION_FUNCTION, BLOCK_ACTION_FUNCTION];

/// Whether `name` is one of Agena's runtime-owned [`CONTROL_FUNCTION_NAMES`].
///
/// Names are dot-free and disjoint from every Tool API gateway function name,
/// so a control function can never be confused with a gateway call or with an
/// execution tool (whose canonical names all contain a dot).
pub const fn is_control_function_name(name: &str) -> bool {
    // A byte match keeps this `const`; a unit test pins it against
    // `CONTROL_FUNCTION_NAMES` so the two cannot drift apart.
    matches!(name.as_bytes(), b"approve_action" | b"block_action")
}

impl ToolApiFunction {
    /// All Tool API function kinds.
    pub const ALL: [Self; 4] = [Self::List, Self::Help, Self::Call, Self::PluginsList];

    /// The exact function name advertised to and accepted from providers.
    pub const fn function_name(self) -> &'static str {
        match self {
            Self::List => "tools_list",
            Self::Help => "tools_help",
            Self::Call => "tools_call",
            Self::PluginsList => "plugins_list",
        }
    }

    pub fn from_function_name(name: &str) -> Option<Self> {
        match name {
            "tools_list" => Some(Self::List),
            "tools_help" => Some(Self::Help),
            "tools_call" => Some(Self::Call),
            "plugins_list" => Some(Self::PluginsList),
            _ => None,
        }
    }
}

impl std::fmt::Display for ToolApiFunction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.function_name())
    }
}
