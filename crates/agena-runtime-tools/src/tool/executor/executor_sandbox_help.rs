//! Present the effective execution mode beside the shell contract.

use super::{RegisteredTool, ToolExecutor};

const MODE_PREFIX: &str = "Execution resource mode: ";

impl ToolExecutor {
    pub(super) fn with_resource_mode_help(
        &self,
        tools: Vec<RegisteredTool>,
    ) -> Vec<RegisteredTool> {
        let mode = if self.sandbox_enabled() {
            "MXC is enabled. Omit reads, writes and network. The sandbox enforces configured path and network permissions, including authority ceilings; command keyword prompts for declarations are disabled."
        } else if self.resource_guard_enabled() {
            "MXC is disabled and resource_guard is enabled. Declare actual reads, writes and network effects. The resource guard checks declarations and prompts for missing declarations before direct execution."
        } else {
            "MXC and resource_guard are disabled. Omit reads, writes and network. Tool and command approval rules still apply."
        };

        tools
            .into_iter()
            .map(|tool| {
                if !matches!(
                    (tool.namespace(), tool.plugin_name(), tool.tool_name()),
                    (
                        "agena",
                        "shell",
                        "exec" | "spawn" | "watch" | "open" | "write"
                    )
                ) {
                    return tool;
                }
                let mut definition = tool.definition().clone();
                definition.docs.help = Some(mode_help(definition.docs.help.as_deref(), mode));
                tool.with_definition(definition).unwrap_or(tool)
            })
            .collect()
    }
}

fn mode_help(help: Option<&str>, mode: &str) -> String {
    let help = help.unwrap_or_default();
    // A trusted invocation can select a different mode after the session
    // catalog was captured. Replace its previous notice instead of retaining
    // stale advice or adding another copy on each catalog read.
    let help = if help.starts_with(MODE_PREFIX) {
        help.split_once("\n\n").map_or("", |(_, help)| help)
    } else {
        help
    };
    if help.is_empty() {
        format!("{MODE_PREFIX}{mode}")
    } else {
        format!("{MODE_PREFIX}{mode}\n\n{help}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changing_invocation_mode_replaces_cached_advice_and_preserves_tool_help() {
        let isolated = mode_help(Some("Keep exact terminal input."), "MXC is enabled.");
        let direct = mode_help(Some(&isolated), "MXC is disabled.");
        assert_eq!(
            direct,
            "Execution resource mode: MXC is disabled.\n\nKeep exact terminal input."
        );
        assert_eq!(mode_help(Some(&direct), "MXC is disabled."), direct);
    }
}
