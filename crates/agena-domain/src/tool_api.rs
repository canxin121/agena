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

impl ToolApiFunction {
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
