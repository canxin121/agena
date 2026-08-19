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
    "agena.openai",
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
    pub plugin_id: Option<&'a str>,
    pub interactive: bool,
    pub task: bool,
}

/// Return whether a runtime tool is meaningful on a stateless MCP transport.
///
/// This is a compatibility/surface policy, not a read-only sandbox. Direct
/// workspace tools such as shell and filesystem writes remain eligible; OAuth
/// and Agena's permission contracts govern their authority separately.
pub fn is_stateless_mcp_tool_exposed(tool: StatelessMcpToolMetadata<'_>) -> bool {
    !tool.interactive
        && !tool.task
        && !tool_uses_hidden_plugin(tool.name, tool.plugin_id)
        && !tool_has_known_interactive_name(tool.name)
}

fn tool_uses_hidden_plugin(name: &str, plugin_id: Option<&str>) -> bool {
    if plugin_id.is_some_and(plugin_id_is_hidden) {
        return true;
    }

    // Older Agena servers may omit plugin_id. Compact and canonical tool names
    // still carry the plugin namespace, so retain a conservative fallback.
    HIDDEN_STATELESS_MCP_PLUGIN_IDS.iter().any(|hidden_id| {
        let compact_id = hidden_id.strip_prefix("agena.").unwrap_or(hidden_id);
        name_belongs_to_plugin(name, compact_id) || name_belongs_to_plugin(name, hidden_id)
    })
}

fn plugin_id_is_hidden(plugin_id: &str) -> bool {
    HIDDEN_STATELESS_MCP_PLUGIN_IDS.iter().any(|hidden_id| {
        plugin_id == *hidden_id
            || plugin_id == hidden_id.strip_prefix("agena.").unwrap_or(hidden_id)
    })
}

fn name_belongs_to_plugin(name: &str, plugin_id: &str) -> bool {
    name == plugin_id
        || name
            .strip_prefix(plugin_id)
            .is_some_and(|suffix| suffix.starts_with('.'))
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
        plugin_id: Option<&'a str>,
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
            tool("shell.run", Some("agena.shell"), false, false),
            tool("fs.write", Some("agena.fs"), false, false),
            tool("code.search_ast", Some("agena.code"), false, false),
            tool("lsp.definition", Some("agena.lsp"), false, false),
            tool("notebook.edit_cell", Some("agena.notebook"), false, false),
            tool("web.fetch", Some("agena.web"), false, false),
            tool("third_party.execute", Some("vendor.plugin"), false, false),
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
            Some("vendor.plugin"),
            true,
            false,
        )));
        assert!(!is_stateless_mcp_tool_exposed(tool(
            "vendor.run_task",
            Some("vendor.plugin"),
            false,
            true,
        )));
    }

    #[test]
    fn every_internal_plugin_is_hidden_with_or_without_plugin_metadata() {
        for plugin_id in HIDDEN_STATELESS_MCP_PLUGIN_IDS {
            let compact_id = plugin_id.strip_prefix("agena.").unwrap_or(plugin_id);
            assert!(!is_stateless_mcp_tool_exposed(tool(
                "unrelated.name",
                Some(plugin_id),
                false,
                false,
            )));
            assert!(!is_stateless_mcp_tool_exposed(tool(
                &format!("{compact_id}.tool"),
                None,
                false,
                false,
            )));
            assert!(!is_stateless_mcp_tool_exposed(tool(
                &format!("{plugin_id}.tool"),
                None,
                false,
                false,
            )));
        }
    }

    #[test]
    fn browser_lifecycle_is_hidden_without_hiding_web_fetch() {
        assert!(!is_stateless_mcp_tool_exposed(tool(
            "web.browser_open",
            Some("agena.web"),
            false,
            false,
        )));
        assert!(!is_stateless_mcp_tool_exposed(tool(
            "agena.web.browser_wait",
            None,
            false,
            false,
        )));
        assert!(is_stateless_mcp_tool_exposed(tool(
            "web.fetch",
            Some("agena.web"),
            false,
            false,
        )));
    }

    #[test]
    fn similarly_named_unrelated_tools_are_not_hidden() {
        for name in ["sessionary.lookup", "planning.inspect", "memory_bank.read"] {
            assert!(is_stateless_mcp_tool_exposed(tool(
                name,
                Some("agena.utility"),
                false,
                false,
            )));
        }
    }
}
