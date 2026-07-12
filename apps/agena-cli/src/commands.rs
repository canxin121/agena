#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandId {
    Help,
    Commands,
    New,
    Sessions,
    Lineage,
    Rewind,
    Rename,
    Timeline,
    Plugins,
    Settings,
    Permissions,
    Model,
    Review,
    Snapshot,
    Commit,
    Pr,
    Export,
    Memory,
    Pager,
    Continue,
    Compact,
    UserInput,
    Allow,
    AllowAlways,
    Deny,
    DenyAlways,
    Attach,
    Download,
    Editor,
    Image,
    Copy,
    CopyMessage,
    CopyVisible,
    Fork,
    Children,
    Parent,
    Diagnostics,
    Status,
    Usage,
    Btw,
    Queue,
}

#[derive(Debug, Clone, Copy)]
pub struct CommandSpec {
    pub id: CommandId,
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    pub arguments: &'static str,
    pub summary_key: &'static str,
}

impl CommandSpec {
    pub fn invocation(self) -> String {
        if self.arguments.is_empty() {
            format!("/{}", self.name)
        } else {
            format!("/{} {}", self.name, self.arguments)
        }
    }
}

#[derive(Debug, Clone)]
pub struct ParsedCommand {
    pub spec: &'static CommandSpec,
    pub args: String,
}

pub const COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        id: CommandId::Help,
        name: "help",
        aliases: &["?"],
        arguments: "",
        summary_key: "command-help-summary",
    },
    CommandSpec {
        id: CommandId::Commands,
        name: "commands",
        aliases: &["palette"],
        arguments: "",
        summary_key: "command-commands-summary",
    },
    CommandSpec {
        id: CommandId::New,
        name: "new",
        aliases: &["clear"],
        arguments: "",
        summary_key: "command-new-summary",
    },
    CommandSpec {
        id: CommandId::Sessions,
        name: "sessions",
        aliases: &[],
        arguments: "[query|all|roots|subtree]",
        summary_key: "command-sessions-summary",
    },
    CommandSpec {
        id: CommandId::Lineage,
        name: "lineage",
        aliases: &["branch-history", "branches"],
        arguments: "",
        summary_key: "command-lineage-summary",
    },
    CommandSpec {
        id: CommandId::Rewind,
        name: "rewind",
        aliases: &["backtrack"],
        arguments: "",
        summary_key: "command-rewind-summary",
    },
    CommandSpec {
        id: CommandId::Rename,
        name: "rename",
        aliases: &["title"],
        arguments: "[title]",
        summary_key: "command-rename-summary",
    },
    CommandSpec {
        id: CommandId::Timeline,
        name: "timeline",
        aliases: &["events"],
        arguments: "[limit]",
        summary_key: "command-timeline-summary",
    },
    CommandSpec {
        id: CommandId::Plugins,
        name: "plugins",
        aliases: &["plugin"],
        arguments: "[query]",
        summary_key: "command-plugins-summary",
    },
    CommandSpec {
        id: CommandId::Settings,
        name: "settings",
        aliases: &["config"],
        arguments: "[query]",
        summary_key: "command-settings-summary",
    },
    CommandSpec {
        id: CommandId::Permissions,
        name: "permissions",
        aliases: &["permission"],
        arguments: "[session|workspace|global|effective|new|list]",
        summary_key: "command-permissions-summary",
    },
    CommandSpec {
        id: CommandId::Model,
        name: "model",
        aliases: &[],
        arguments: "",
        summary_key: "command-model-summary",
    },
    CommandSpec {
        id: CommandId::Review,
        name: "review",
        aliases: &[],
        arguments: "[focus]",
        summary_key: "command-review-summary",
    },
    CommandSpec {
        id: CommandId::Snapshot,
        name: "snapshot",
        aliases: &[],
        arguments: "[query]",
        summary_key: "command-snapshot-summary",
    },
    CommandSpec {
        id: CommandId::Commit,
        name: "commit",
        aliases: &[],
        arguments: "<message>",
        summary_key: "command-commit-summary",
    },
    CommandSpec {
        id: CommandId::Pr,
        name: "pr",
        aliases: &[],
        arguments: "<title> [--body <text>] [--base <branch>] [--head <branch>]",
        summary_key: "command-pr-summary",
    },
    CommandSpec {
        id: CommandId::Export,
        name: "export",
        aliases: &["save"],
        arguments: "[path]",
        summary_key: "command-export-summary",
    },
    CommandSpec {
        id: CommandId::Memory,
        name: "memory",
        aliases: &["mem"],
        arguments: "[list|edit [name]|forget <name>]",
        summary_key: "command-memory-summary",
    },
    CommandSpec {
        id: CommandId::Pager,
        name: "pager",
        aliases: &["view", "less"],
        arguments: "",
        summary_key: "command-pager-summary",
    },
    CommandSpec {
        id: CommandId::Continue,
        name: "continue",
        aliases: &["resume-run"],
        arguments: "",
        summary_key: "command-continue-summary",
    },
    CommandSpec {
        id: CommandId::Compact,
        name: "compact",
        aliases: &["compress", "summarize"],
        arguments: "",
        summary_key: "command-compact-summary",
    },
    CommandSpec {
        id: CommandId::UserInput,
        name: "user-input",
        aliases: &["reply"],
        arguments: "",
        summary_key: "command-user-input-summary",
    },
    CommandSpec {
        id: CommandId::Allow,
        name: "allow",
        aliases: &[],
        arguments: "",
        summary_key: "command-allow-summary",
    },
    CommandSpec {
        id: CommandId::AllowAlways,
        name: "allow-always",
        aliases: &[],
        arguments: "",
        summary_key: "command-allow-always-summary",
    },
    CommandSpec {
        id: CommandId::Deny,
        name: "deny",
        aliases: &[],
        arguments: "",
        summary_key: "command-deny-summary",
    },
    CommandSpec {
        id: CommandId::DenyAlways,
        name: "deny-always",
        aliases: &[],
        arguments: "",
        summary_key: "command-deny-always-summary",
    },
    CommandSpec {
        id: CommandId::Attach,
        name: "attach",
        aliases: &["file"],
        arguments: "[local-path ...]",
        summary_key: "command-attach-summary",
    },
    CommandSpec {
        id: CommandId::Download,
        name: "download",
        aliases: &["dl"],
        arguments: "<workspace-path>",
        summary_key: "command-download-summary",
    },
    CommandSpec {
        id: CommandId::Editor,
        name: "editor",
        aliases: &["edit"],
        arguments: "",
        summary_key: "command-editor-summary",
    },
    CommandSpec {
        id: CommandId::Image,
        name: "image",
        aliases: &["paste-image"],
        arguments: "[local-path ...]",
        summary_key: "command-image-summary",
    },
    CommandSpec {
        id: CommandId::Copy,
        name: "copy",
        aliases: &["yank"],
        arguments: "",
        summary_key: "command-copy-summary",
    },
    CommandSpec {
        id: CommandId::CopyMessage,
        name: "copy-message",
        aliases: &["copy-last", "copy-assistant"],
        arguments: "",
        summary_key: "command-copy-message-summary",
    },
    CommandSpec {
        id: CommandId::CopyVisible,
        name: "copy-visible",
        aliases: &[],
        arguments: "",
        summary_key: "command-copy-visible-summary",
    },
    CommandSpec {
        id: CommandId::Fork,
        name: "fork",
        aliases: &["branch"],
        arguments: "",
        summary_key: "command-fork-summary",
    },
    CommandSpec {
        id: CommandId::Children,
        name: "children",
        aliases: &["child"],
        arguments: "",
        summary_key: "command-children-summary",
    },
    CommandSpec {
        id: CommandId::Parent,
        name: "parent",
        aliases: &[],
        arguments: "",
        summary_key: "command-parent-summary",
    },
    CommandSpec {
        id: CommandId::Diagnostics,
        name: "diagnostics",
        aliases: &["feedback"],
        arguments: "",
        summary_key: "command-diagnostics-summary",
    },
    CommandSpec {
        id: CommandId::Status,
        name: "status",
        aliases: &[],
        arguments: "",
        summary_key: "command-status-summary",
    },
    CommandSpec {
        id: CommandId::Usage,
        name: "usage",
        aliases: &["stats", "analytics"],
        arguments: "[today|yesterday|7d|14d|30d|90d|month|year|all]",
        summary_key: "command-usage-summary",
    },
    CommandSpec {
        id: CommandId::Btw,
        name: "btw",
        aliases: &["aside", "side"],
        arguments: "<question>",
        summary_key: "command-btw-summary",
    },
    CommandSpec {
        id: CommandId::Queue,
        name: "queue",
        aliases: &["q"],
        arguments: "[list|clear|pop]",
        summary_key: "command-queue-summary",
    },
];

pub fn find_command(name: &str) -> Option<&'static CommandSpec> {
    let normalized = name.trim().to_ascii_lowercase();
    COMMANDS.iter().find(|spec| {
        spec.name == normalized || spec.aliases.iter().any(|alias| *alias == normalized)
    })
}

pub fn parse_invocation(input: &str) -> Option<(&str, &str)> {
    let trimmed = input.trim();
    if !trimmed.starts_with('/') || trimmed.starts_with("//") {
        return None;
    }
    let content = trimmed[1..].trim_start();
    if content.is_empty() {
        return None;
    }

    let mut parts = content.splitn(2, char::is_whitespace);
    let name = parts.next()?;
    let args = parts.next().unwrap_or("").trim();
    Some((name, args))
}

pub fn parse_command(input: &str) -> Option<ParsedCommand> {
    let (name, args) = parse_invocation(input)?;
    let spec = find_command(name)?;
    Some(ParsedCommand {
        spec,
        args: args.to_string(),
    })
}

pub fn command_suggestions_for_prefix(query: &str) -> Vec<&'static CommandSpec> {
    let query = query.trim().to_ascii_lowercase();
    if query.is_empty() {
        return COMMANDS.iter().collect();
    }

    let mut exact = Vec::new();
    let mut prefix = Vec::new();
    for spec in COMMANDS {
        if command_name_exact_match(spec, query.as_str()) {
            exact.push(spec);
        } else if command_name_prefix_match(spec, query.as_str()) {
            prefix.push(spec);
        }
    }
    exact.extend(prefix);
    exact
}

fn command_name_exact_match(spec: &CommandSpec, query: &str) -> bool {
    spec.name == query || spec.aliases.contains(&query)
}

fn command_name_prefix_match(spec: &CommandSpec, query: &str) -> bool {
    spec.name.starts_with(query) || spec.aliases.iter().any(|alias| alias.starts_with(query))
}

#[cfg(test)]
mod tests {
    use super::{CommandId, parse_command};

    #[test]
    fn parses_terminal_download_command_and_alias() {
        let command = parse_command("/download artifacts/build.zip").expect("download command");
        assert_eq!(command.spec.id, CommandId::Download);
        assert_eq!(command.args, "artifacts/build.zip");

        let alias = parse_command("/dl notes.txt").expect("download alias");
        assert_eq!(alias.spec.id, CommandId::Download);
        assert_eq!(alias.args, "notes.txt");
    }
}
