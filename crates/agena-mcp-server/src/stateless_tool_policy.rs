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
    "agena.tasks",
    "agena.cron",
    "agena.monitor",
    "agena.snapshot",
    "agena.report",
    // Agena's own control/knowledge plane is intentionally separate from the
    // focused computer-control surface. MCP already provides tool discovery,
    // and nesting MCP through MCP creates a confused-deputy/recursion seam.
    "agena.settings",
    "agena.memory",
    "agena.skills",
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
    pub task: bool,
}

/// Return whether a runtime tool is meaningful on a stateless MCP transport.
///
/// This is a surface policy, not a read-only sandbox. Direct
/// workspace tools such as shell and filesystem writes remain eligible; OAuth
/// and Agena's permission contracts govern their authority separately.
pub fn is_stateless_mcp_tool_exposed(tool: StatelessMcpToolMetadata<'_>) -> bool {
    !tool.plugin_id.trim().is_empty()
        && !tool.interactive
        && !tool.task
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

#[cfg(test)]
mod tests {
    use super::{
        HIDDEN_STATELESS_MCP_PLUGIN_IDS, StatelessMcpToolMetadata, is_stateless_mcp_tool_exposed,
    };

    fn tool<'a>(
        name: &'a str,
        plugin_id: &'a str,
        interactive: bool,
        task: bool,
    ) -> StatelessMcpToolMetadata<'a> {
        StatelessMcpToolMetadata {
            name,
            plugin_id,
            interactive,
            task,
        }
    }

    #[test]
    fn direct_workspace_tools_remain_exposed() {
        for candidate in [
            tool("shell.run", "agena.shell", false, false),
            tool("shell.write", "agena.shell", false, false),
            tool("shell.resize", "agena.shell", false, false),
            tool("shell.signal", "agena.shell", false, false),
            tool("fs.write", "agena.fs", false, false),
            tool("code.search_ast", "agena.code", false, false),
            tool("lsp.definition", "agena.lsp", false, false),
            tool("notebook.edit_cell", "agena.notebook", false, false),
            tool("web.fetch", "agena.web", false, false),
            tool("third_party.execute", "vendor.plugin", false, false),
        ] {
            assert!(
                is_stateless_mcp_tool_exposed(candidate),
                "{} should remain exposed",
                candidate.name
            );
        }
    }

    #[test]
    fn interactive_and_task_tools_are_hidden_independently_of_plugin() {
        assert!(!is_stateless_mcp_tool_exposed(tool(
            "vendor.ask",
            "vendor.plugin",
            true,
            false,
        )));
        assert!(!is_stateless_mcp_tool_exposed(tool(
            "vendor.run_task",
            "vendor.plugin",
            false,
            true,
        )));
    }

    #[test]
    fn every_internal_plugin_is_hidden_by_current_plugin_identity() {
        for plugin_id in HIDDEN_STATELESS_MCP_PLUGIN_IDS {
            assert!(!is_stateless_mcp_tool_exposed(tool(
                "unrelated.name",
                plugin_id,
                false,
                false,
            )));
        }
    }

    #[test]
    fn browser_lifecycle_is_hidden_without_hiding_web_fetch() {
        assert!(!is_stateless_mcp_tool_exposed(tool(
            "web.browser_open",
            "agena.web",
            false,
            false,
        )));
        assert!(!is_stateless_mcp_tool_exposed(tool(
            "agena.web.browser_wait",
            "agena.web",
            false,
            false,
        )));
        assert!(is_stateless_mcp_tool_exposed(tool(
            "web.fetch",
            "agena.web",
            false,
            false,
        )));
    }

    #[test]
    fn similarly_named_unrelated_tools_are_not_hidden() {
        for name in ["sessionary.lookup", "planning.inspect", "memory_bank.read"] {
            assert!(is_stateless_mcp_tool_exposed(tool(
                name,
                "agena.utility",
                false,
                false,
            )));
        }
    }

    #[test]
    fn missing_plugin_identity_is_not_exposed() {
        assert!(!is_stateless_mcp_tool_exposed(tool(
            "third_party.execute",
            "",
            false,
            false,
        )));
    }
}
