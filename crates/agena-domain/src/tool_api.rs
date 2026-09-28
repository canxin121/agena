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
    #[serde(rename = "tools_search")]
    Search,
    #[serde(rename = "tools_help")]
    Help,
    #[serde(rename = "tools_tags")]
    Tags,
    #[serde(rename = "tools_call")]
    Call,
    #[serde(rename = "plugins_list")]
    PluginsList,
    #[serde(rename = "plugins_search")]
    PluginsSearch,
    #[serde(rename = "plugins_tags")]
    PluginsTags,
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
    pub const ALL: [Self; 8] = [
        Self::List,
        Self::Search,
        Self::Help,
        Self::Tags,
        Self::Call,
        Self::PluginsList,
        Self::PluginsSearch,
        Self::PluginsTags,
    ];

    /// The exact function name advertised to and accepted from providers.
    pub const fn function_name(self) -> &'static str {
        match self {
            Self::List => "tools_list",
            Self::Search => "tools_search",
            Self::Help => "tools_help",
            Self::Tags => "tools_tags",
            Self::Call => "tools_call",
            Self::PluginsList => "plugins_list",
            Self::PluginsSearch => "plugins_search",
            Self::PluginsTags => "plugins_tags",
        }
    }

    pub fn from_function_name(name: &str) -> Option<Self> {
        match name {
            "tools_list" => Some(Self::List),
            "tools_search" => Some(Self::Search),
            "tools_help" => Some(Self::Help),
            "tools_tags" => Some(Self::Tags),
            "tools_call" => Some(Self::Call),
            "plugins_list" => Some(Self::PluginsList),
            "plugins_search" => Some(Self::PluginsSearch),
            "plugins_tags" => Some(Self::PluginsTags),
            _ => None,
        }
    }
}

impl std::fmt::Display for ToolApiFunction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.function_name())
    }
}

#[cfg(test)]
mod tests {
    use super::ToolApiFunction;

    #[test]
    fn control_function_names_are_closed_and_disjoint() {
        use super::{CONTROL_FUNCTION_NAMES, is_control_function_name};
        // The `const fn` must agree with the constant it documents.
        for name in CONTROL_FUNCTION_NAMES {
            assert!(is_control_function_name(name), "{name} must be recognized");
            // Never a gateway function name, and never an execution tool name
            // (whose canonical names always contain a dot).
            assert!(ToolApiFunction::from_function_name(name).is_none());
            assert!(!name.contains('.'));
        }
        assert!(is_control_function_name("approve_action"));
        assert!(is_control_function_name("block_action"));
        for other in ["", "approve", "approve_actions", "tools_call", "fs.read"] {
            assert!(!is_control_function_name(other), "{other} must not match");
        }
    }

    #[test]
    fn protocol_names_round_trip_without_aliases() {
        for function in ToolApiFunction::ALL {
            assert_eq!(
                ToolApiFunction::from_function_name(function.function_name()),
                Some(function)
            );
        }

        assert_eq!(ToolApiFunction::from_function_name("tools.help"), None);
        assert_eq!(
            ToolApiFunction::from_function_name("agena.tools.help"),
            None
        );
        assert_eq!(ToolApiFunction::from_function_name(" tools_help"), None);
        assert_eq!(ToolApiFunction::from_function_name("tools_help "), None);
    }

    #[test]
    fn every_protocol_name_is_provider_safe() {
        for function in ToolApiFunction::ALL {
            assert!(
                function
                    .function_name()
                    .bytes()
                    .all(|byte| { byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-' })
            );
        }
    }
}
