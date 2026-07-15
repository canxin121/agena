use serde::{Deserialize, Serialize};

/// The complete set of Agena client functions exposed through a model
/// provider's tool/function-calling protocol.
///
/// Catalog targets such as `session.rename` and internal registry keys such as
/// `agena.session.rename` are deliberately not represented by this type. They
/// are data carried inside gateway-function payloads and must never be used as
/// provider function names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GatewayFunction {
    ToolsList,
    ToolsSearch,
    ToolsHelp,
    ToolsTags,
    ToolsCall,
}

impl GatewayFunction {
    pub const ALL: [Self; 5] = [
        Self::ToolsList,
        Self::ToolsSearch,
        Self::ToolsHelp,
        Self::ToolsTags,
        Self::ToolsCall,
    ];

    /// The exact function name advertised to and accepted from providers.
    pub const fn protocol_name(self) -> &'static str {
        match self {
            Self::ToolsList => "tools_list",
            Self::ToolsSearch => "tools_search",
            Self::ToolsHelp => "tools_help",
            Self::ToolsTags => "tools_tags",
            Self::ToolsCall => "tools_call",
        }
    }

    /// The stable internal registry key of the function's local handler.
    pub const fn handler_name(self) -> &'static str {
        match self {
            Self::ToolsList => "agena.tools.list",
            Self::ToolsSearch => "agena.tools.search",
            Self::ToolsHelp => "agena.tools.help",
            Self::ToolsTags => "agena.tools.tags",
            Self::ToolsCall => "agena.tools.call",
        }
    }

    /// Compact catalog address of the local handler. Gateway handlers are not
    /// valid `tools_call` targets, but recognizing this form lets the gateway
    /// reject that misuse without conflating it with a provider function name.
    pub const fn catalog_target_name(self) -> &'static str {
        match self {
            Self::ToolsList => "tools.list",
            Self::ToolsSearch => "tools.search",
            Self::ToolsHelp => "tools.help",
            Self::ToolsTags => "tools.tags",
            Self::ToolsCall => "tools.call",
        }
    }

    pub fn from_protocol_name(name: &str) -> Option<Self> {
        match name {
            "tools_list" => Some(Self::ToolsList),
            "tools_search" => Some(Self::ToolsSearch),
            "tools_help" => Some(Self::ToolsHelp),
            "tools_tags" => Some(Self::ToolsTags),
            "tools_call" => Some(Self::ToolsCall),
            _ => None,
        }
    }

    /// Resolve a persisted internal handler key. This is intentionally exact:
    /// it exists for durable history and migrations, not as a provider alias.
    pub fn from_handler_name(name: &str) -> Option<Self> {
        match name {
            "agena.tools.list" => Some(Self::ToolsList),
            "agena.tools.search" => Some(Self::ToolsSearch),
            "agena.tools.help" => Some(Self::ToolsHelp),
            "agena.tools.tags" => Some(Self::ToolsTags),
            "agena.tools.call" => Some(Self::ToolsCall),
            _ => None,
        }
    }

    /// Classify the compact internal address of a gateway handler. This is
    /// used to reject gateway self-calls and is never a provider-name alias.
    pub fn from_catalog_target_name(name: &str) -> Option<Self> {
        match name {
            "tools.list" => Some(Self::ToolsList),
            "tools.search" => Some(Self::ToolsSearch),
            "tools.help" => Some(Self::ToolsHelp),
            "tools.tags" => Some(Self::ToolsTags),
            "tools.call" => Some(Self::ToolsCall),
            _ => None,
        }
    }

    pub fn from_handler_parts(namespace: &str, plugin_name: &str, tool_name: &str) -> Option<Self> {
        if namespace != "agena" || plugin_name != "tools" {
            return None;
        }
        match tool_name {
            "list" => Some(Self::ToolsList),
            "search" => Some(Self::ToolsSearch),
            "help" => Some(Self::ToolsHelp),
            "tags" => Some(Self::ToolsTags),
            "call" => Some(Self::ToolsCall),
            _ => None,
        }
    }
}

impl std::fmt::Display for GatewayFunction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.protocol_name())
    }
}

#[cfg(test)]
mod tests {
    use super::GatewayFunction;

    #[test]
    fn protocol_and_handler_names_round_trip_without_aliases() {
        for function in GatewayFunction::ALL {
            assert_eq!(
                GatewayFunction::from_protocol_name(function.protocol_name()),
                Some(function)
            );
            assert_eq!(
                GatewayFunction::from_handler_name(function.handler_name()),
                Some(function)
            );
            assert_eq!(
                GatewayFunction::from_catalog_target_name(function.catalog_target_name()),
                Some(function)
            );
        }

        assert_eq!(GatewayFunction::from_protocol_name("tools.help"), None);
        assert_eq!(
            GatewayFunction::from_protocol_name("agena.tools.help"),
            None
        );
        assert_eq!(GatewayFunction::from_handler_name("tools_help"), None);
        assert_eq!(GatewayFunction::from_protocol_name(" tools_help"), None);
        assert_eq!(GatewayFunction::from_protocol_name("tools_help "), None);
        assert_eq!(
            GatewayFunction::from_handler_name(" agena.tools.help"),
            None
        );
    }

    #[test]
    fn every_protocol_name_is_provider_safe() {
        for function in GatewayFunction::ALL {
            assert!(
                function
                    .protocol_name()
                    .bytes()
                    .all(|byte| { byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-' })
            );
        }
    }
}
