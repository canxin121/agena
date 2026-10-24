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

fn plugin_id_is_hidden(plugin_id: &str) -> bool {
    HIDDEN_STATELESS_MCP_PLUGIN_IDS.contains(&plugin_id)
}

fn tool_has_known_interactive_name(name: &str) -> bool {
    KNOWN_INTERACTIVE_TOOL_NAMES.contains(&name)
        || KNOWN_INTERACTIVE_TOOL_PREFIXES
            .iter()
            .any(|prefix| name.starts_with(prefix))
}
