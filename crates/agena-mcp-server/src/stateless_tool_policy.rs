//! Shared tool-exposure policy for MCP transports that invoke Agena tools
//! without an Agena conversation/session.

/// Bundled plugins that are not part of the direct computer/workspace control
/// surface exposed to stateless MCP clients.
///
/// Keep this list shared by the HTTP ChatGPT connector and the CLI stdio
/// bridge. Duplicating the policy previously let the two transports drift.
const HIDDEN_STATELESS_MCP_PLUGIN_IDS: &[&str] = &[
    // Provider/developer adapters would recursively invoke another model or
    // expose internal schema experiments rather than operate the workspace.
    "agena.chatgpt",
    "agena.gemini",
    "agena.claude",
    "agena.schema_lab",
    // These plugins depend on Agena session lifecycle, user interaction, UI
    // effects, subagents, or session notifications that stateless MCP cannot
    // deliver correctly.
    "agena.interaction",
    "agena.session",
    "agena.plan",
    "agena.agent",
    "agena.cron",
    "agena.monitor",
    "agena.report",
    // Agena's own control/knowledge plane is intentionally separate from the
    // focused computer-control surface. MCP already provides tool discovery,
    // and nesting MCP through MCP creates a confused-deputy/recursion seam.
    "agena.settings",
    "agena.memory",
    "agena.commands",
    "agena.tools",
    "agena.mcp",
];

const KNOWN_INTERACTIVE_TOOL_NAMES: &[&str] = &[
    "interaction.ask",
    "interaction.notify",
    "prompt.ask",
    "prompt.notify",
];

// `agena.web` also owns useful stateless fetch/search/crawl tools, so only its
// managed-browser lifecycle is filtered rather than hiding the entire plugin.
const KNOWN_INTERACTIVE_TOOL_PREFIXES: &[&str] = &["web.browser_", "agena.web.browser_"];

/// Transport-relevant metadata used to decide whether one runtime tool belongs
/// on Agena's stateless MCP surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatelessMcpToolMetadata<'a> {
    pub name: &'a str,
    pub plugin_id: &'a str,
    pub interactive: bool,
    pub agent: bool,
}

/// Return whether a runtime tool is meaningful on a stateless MCP transport.
///
/// This is a surface policy, not a read-only sandbox. Direct
/// workspace tools such as shell and filesystem writes remain eligible; OAuth
/// and Agena's permission policy governs their authority separately.
pub fn is_stateless_mcp_tool_exposed(tool: StatelessMcpToolMetadata<'_>) -> bool {
    !tool.plugin_id.trim().is_empty()
        && !tool.interactive
        && !tool.agent
        && !plugin_id_is_hidden(tool.plugin_id)
        && !tool_has_known_interactive_name(tool.name)
}

/// Authorization requirement of one exposed MCP tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpToolAccess {
    /// Anonymous clients may invoke the tool without an access token.
    Anonymous,
    /// A valid OAuth access token is required.
    OAuth,
}

/// Tools that are explicitly safe to invoke anonymously.
///
/// Deliberately empty: on the stateless surface "read-only" is not the same as
/// "non-sensitive" (workspace reads expose private data), so anonymity is a
/// property each tool owner has to claim for its own tool. Every transport that
/// serves this policy therefore protects the whole exposed catalog by default
/// while still supporting anonymous tools as a first-class capability.
const ANONYMOUS_MCP_TOOL_ALLOWLIST: &[&str] = &[];

/// Return the authorization requirement of one exposed MCP tool.
///
/// This is the single place where "does this call need a token" is answered;
/// no transport offers a deployment-wide authentication mode any more.
pub fn mcp_tool_access(tool: StatelessMcpToolMetadata<'_>) -> McpToolAccess {
    if ANONYMOUS_MCP_TOOL_ALLOWLIST.contains(&tool.name) {
        McpToolAccess::Anonymous
    } else {
        McpToolAccess::OAuth
    }
}

fn plugin_id_is_hidden(plugin_id: &str) -> bool {
    HIDDEN_STATELESS_MCP_PLUGIN_IDS.contains(&plugin_id)
}

fn tool_has_known_interactive_name(name: &str) -> bool {
    KNOWN_INTERACTIVE_TOOL_NAMES.contains(&name)
        || KNOWN_INTERACTIVE_TOOL_PREFIXES
            .iter()
            .any(|prefix| name.starts_with(prefix))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metadata<'a>(name: &'a str, plugin_id: &'a str) -> StatelessMcpToolMetadata<'a> {
        StatelessMcpToolMetadata {
            name,
            plugin_id,
            interactive: false,
            agent: false,
        }
    }

    #[test]
    fn workspace_tools_stay_exposed() {
        assert!(is_stateless_mcp_tool_exposed(metadata(
            "fs.read", "agena.fs"
        )));
        assert!(is_stateless_mcp_tool_exposed(metadata(
            "shell.exec",
            "agena.shell"
        )));
    }

    #[test]
    fn provider_session_and_control_plugins_are_hidden() {
        for plugin in [
            "agena.chatgpt",
            "agena.session",
            "agena.interaction",
            "agena.settings",
            "agena.mcp",
        ] {
            assert!(
                !is_stateless_mcp_tool_exposed(metadata("some.tool", plugin)),
                "{plugin} must not be exposed on a stateless transport"
            );
        }
        assert!(!is_stateless_mcp_tool_exposed(StatelessMcpToolMetadata {
            name: "interaction.ask",
            plugin_id: "agena.core",
            interactive: false,
            agent: false,
        }));
    }

    #[test]
    fn every_exposed_tool_requires_a_token_until_it_opts_out() {
        assert_eq!(
            mcp_tool_access(metadata("fs.read", "agena.fs")),
            McpToolAccess::OAuth
        );
        assert_eq!(
            mcp_tool_access(metadata("shell.exec", "agena.shell")),
            McpToolAccess::OAuth
        );
    }
}
