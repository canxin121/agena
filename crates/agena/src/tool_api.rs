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
}

impl ToolApiFunction {
    pub const ALL: [Self; 5] = [Self::List, Self::Search, Self::Help, Self::Tags, Self::Call];

    /// The exact function name advertised to and accepted from providers.
    pub const fn function_name(self) -> &'static str {
        match self {
            Self::List => "tools_list",
            Self::Search => "tools_search",
            Self::Help => "tools_help",
            Self::Tags => "tools_tags",
            Self::Call => "tools_call",
        }
    }

    /// The stable internal registry key of the function's local handler.
    pub const fn handler_name(self) -> &'static str {
        match self {
            Self::List => "agena.tools.list",
            Self::Search => "agena.tools.search",
            Self::Help => "agena.tools.help",
            Self::Tags => "agena.tools.tags",
            Self::Call => "agena.tools.call",
        }
    }

    /// Compact legacy name of the local API handler. API handlers are not
    /// execution tools, but recognizing this form lets Agena explain a
    /// misrouted call without accepting it as an alias.
    pub const fn compact_handler_name(self) -> &'static str {
        match self {
            Self::List => "tools.list",
            Self::Search => "tools.search",
            Self::Help => "tools.help",
            Self::Tags => "tools.tags",
            Self::Call => "tools.call",
        }
    }

    pub fn from_function_name(name: &str) -> Option<Self> {
        match name {
            "tools_list" => Some(Self::List),
            "tools_search" => Some(Self::Search),
            "tools_help" => Some(Self::Help),
            "tools_tags" => Some(Self::Tags),
            "tools_call" => Some(Self::Call),
            _ => None,
        }
    }

    /// Resolve a persisted internal handler key. This is intentionally exact:
    /// it exists for durable history and migrations, not as a provider alias.
    pub fn from_handler_name(name: &str) -> Option<Self> {
        match name {
            "agena.tools.list" => Some(Self::List),
            "agena.tools.search" => Some(Self::Search),
            "agena.tools.help" => Some(Self::Help),
            "agena.tools.tags" => Some(Self::Tags),
            "agena.tools.call" => Some(Self::Call),
            _ => None,
        }
    }

    /// Classify the compact legacy name of a Tool API handler. This is used to
    /// reject API self-calls and is never a provider-name alias.
    pub fn from_compact_handler_name(name: &str) -> Option<Self> {
        match name {
            "tools.list" => Some(Self::List),
            "tools.search" => Some(Self::Search),
            "tools.help" => Some(Self::Help),
            "tools.tags" => Some(Self::Tags),
            "tools.call" => Some(Self::Call),
            _ => None,
        }
    }

    /// Resolve every persisted or provider-facing spelling of a Tool API
    /// function for display-only use. Protocol validation continues to use
    /// the exact resolvers above.
    pub fn from_display_name(name: &str) -> Option<Self> {
        Self::from_handler_name(name)
            .or_else(|| Self::from_compact_handler_name(name))
            .or_else(|| Self::from_function_name(name))
    }

    pub fn from_handler_parts(namespace: &str, plugin_name: &str, tool_name: &str) -> Option<Self> {
        if namespace != "agena" || plugin_name != "tools" {
            return None;
        }
        match tool_name {
            "list" => Some(Self::List),
            "search" => Some(Self::Search),
            "help" => Some(Self::Help),
            "tags" => Some(Self::Tags),
            "call" => Some(Self::Call),
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
    fn protocol_and_handler_names_round_trip_without_aliases() {
        for function in ToolApiFunction::ALL {
            assert_eq!(
                ToolApiFunction::from_function_name(function.function_name()),
                Some(function)
            );
            assert_eq!(
                ToolApiFunction::from_handler_name(function.handler_name()),
                Some(function)
            );
            assert_eq!(
                ToolApiFunction::from_compact_handler_name(function.compact_handler_name()),
                Some(function)
            );
        }

        assert_eq!(ToolApiFunction::from_function_name("tools.help"), None);
        assert_eq!(
            ToolApiFunction::from_function_name("agena.tools.help"),
            None
        );
        assert_eq!(ToolApiFunction::from_handler_name("tools_help"), None);
        assert_eq!(ToolApiFunction::from_function_name(" tools_help"), None);
        assert_eq!(ToolApiFunction::from_function_name("tools_help "), None);
        assert_eq!(
            ToolApiFunction::from_handler_name(" agena.tools.help"),
            None
        );
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
