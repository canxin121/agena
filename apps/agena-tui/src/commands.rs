#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandId {
    Help,
    Commands,
    New,
    Sessions,
    Resume,
    Lineage,
    Rewind,
    Search,
    Find,
    Rename,
    Timeline,
    Plugins,
    Mcp,
    Lsp,
    Skills,
    Runtime,
    Cost,
    Review,
    Permissions,
    Config,
    Worktree,
    Git,
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
    Editor,
    Image,
    Copy,
    CopyVisible,
    Providers,
    Provider,
    ProviderConfig,
    Models,
    Model,
    Variant,
    Temperature,
    MaxOutput,
    System,
    Fork,
    Children,
    Parent,
    Diagnostics,
    Status,
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
        arguments: "[all|roots|subtree]",
        summary_key: "command-sessions-summary",
    },
    CommandSpec {
        id: CommandId::Resume,
        name: "resume",
        aliases: &["switch", "recent"],
        arguments: "",
        summary_key: "command-resume-summary",
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
        id: CommandId::Search,
        name: "search",
        aliases: &[],
        arguments: "[query]",
        summary_key: "command-search-summary",
    },
    CommandSpec {
        id: CommandId::Find,
        name: "find",
        aliases: &[],
        arguments: "[query]",
        summary_key: "command-find-summary",
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
        id: CommandId::Mcp,
        name: "mcp",
        aliases: &[],
        arguments: "[query]",
        summary_key: "command-mcp-summary",
    },
    CommandSpec {
        id: CommandId::Lsp,
        name: "lsp",
        aliases: &[],
        arguments: "[query]",
        summary_key: "command-lsp-summary",
    },
    CommandSpec {
        id: CommandId::Skills,
        name: "skills",
        aliases: &["skill"],
        arguments: "[query]",
        summary_key: "command-skills-summary",
    },
    CommandSpec {
        id: CommandId::Runtime,
        name: "runtime",
        aliases: &["operator"],
        arguments: "[query]",
        summary_key: "command-runtime-summary",
    },
    CommandSpec {
        id: CommandId::Cost,
        name: "cost",
        aliases: &["usage"],
        arguments: "",
        summary_key: "command-cost-summary",
    },
    CommandSpec {
        id: CommandId::Review,
        name: "review",
        aliases: &[],
        arguments: "[focus]",
        summary_key: "command-review-summary",
    },
    CommandSpec {
        id: CommandId::Permissions,
        name: "permissions",
        aliases: &["perm"],
        arguments: "[query]",
        summary_key: "command-permissions-summary",
    },
    CommandSpec {
        id: CommandId::Config,
        name: "config",
        aliases: &[],
        arguments: "[query]",
        summary_key: "command-config-summary",
    },
    CommandSpec {
        id: CommandId::Worktree,
        name: "worktree",
        aliases: &["wt"],
        arguments: "[query]",
        summary_key: "command-worktree-summary",
    },
    CommandSpec {
        id: CommandId::Git,
        name: "git",
        aliases: &["repo"],
        arguments: "[query]",
        summary_key: "command-git-summary",
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
        aliases: &["summarize"],
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
        arguments: "",
        summary_key: "command-attach-summary",
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
        arguments: "",
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
        id: CommandId::CopyVisible,
        name: "copy-visible",
        aliases: &[],
        arguments: "",
        summary_key: "command-copy-visible-summary",
    },
    CommandSpec {
        id: CommandId::Providers,
        name: "providers",
        aliases: &[],
        arguments: "",
        summary_key: "command-providers-summary",
    },
    CommandSpec {
        id: CommandId::Provider,
        name: "provider",
        aliases: &[],
        arguments: "[provider_id|clear]",
        summary_key: "command-provider-summary",
    },
    CommandSpec {
        id: CommandId::ProviderConfig,
        name: "provider-config",
        aliases: &["providers-config", "provider-manager"],
        arguments: "[provider_id]",
        summary_key: "command-provider-config-summary",
    },
    CommandSpec {
        id: CommandId::Models,
        name: "models",
        aliases: &[],
        arguments: "[provider_id]",
        summary_key: "command-models-summary",
    },
    CommandSpec {
        id: CommandId::Model,
        name: "model",
        aliases: &[],
        arguments: "[provider_id/model_id|model_id|clear]",
        summary_key: "command-model-summary",
    },
    CommandSpec {
        id: CommandId::Variant,
        name: "variant",
        aliases: &["reasoning"],
        arguments: "<name|clear>",
        summary_key: "command-variant-summary",
    },
    CommandSpec {
        id: CommandId::Temperature,
        name: "temperature",
        aliases: &["temp"],
        arguments: "<number|clear>",
        summary_key: "command-temperature-summary",
    },
    CommandSpec {
        id: CommandId::MaxOutput,
        name: "max-output",
        aliases: &["max-tokens"],
        arguments: "<number|clear>",
        summary_key: "command-max-output-summary",
    },
    CommandSpec {
        id: CommandId::System,
        name: "system",
        aliases: &[],
        arguments: "<text|clear>",
        summary_key: "command-system-summary",
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

pub fn command_matches_query(spec: &CommandSpec, query: &str) -> bool {
    let query = query.trim().to_ascii_lowercase();
    if query.is_empty() {
        return true;
    }

    spec.name.contains(query.as_str())
        || query.contains(spec.name)
        || spec
            .aliases
            .iter()
            .any(|alias| alias.contains(query.as_str()) || query.contains(alias))
        || spec.arguments.to_ascii_lowercase().contains(query.as_str())
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
    spec.name == query || spec.aliases.iter().any(|alias| *alias == query)
}

fn command_name_prefix_match(spec: &CommandSpec, query: &str) -> bool {
    spec.name.starts_with(query) || spec.aliases.iter().any(|alias| alias.starts_with(query))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_command_supports_aliases() {
        let parsed = parse_command("/temp 0.2").expect("command should parse");
        assert_eq!(parsed.spec.id, CommandId::Temperature);
        assert_eq!(parsed.args, "0.2");

        let parsed = parse_command("/reasoning high").expect("command should parse");
        assert_eq!(parsed.spec.id, CommandId::Variant);
        assert_eq!(parsed.args, "high");
    }

    #[test]
    fn parse_invocation_preserves_unknown_command_names() {
        let (name, args) = parse_invocation("/custom run this").expect("invocation should parse");
        assert_eq!(name, "custom");
        assert_eq!(args, "run this");
    }

    #[test]
    fn parse_command_ignores_literal_double_slash() {
        assert!(parse_command("//not-a-command").is_none());
    }

    #[test]
    fn command_matches_query_uses_aliases_and_arguments() {
        let spec = find_command("model").expect("model command should exist");
        assert!(command_matches_query(spec, "models"));
        assert!(command_matches_query(spec, "provider_id"));
    }

    #[test]
    fn provider_config_command_matches_aliases() {
        let spec = find_command("provider-config").expect("provider-config command should exist");
        assert!(command_matches_query(spec, "providers-config"));
        assert!(command_matches_query(spec, "provider-manager"));
    }

    #[test]
    fn command_suggestions_match_names_and_alias_prefixes() {
        let names = command_suggestions_for_prefix("re")
            .into_iter()
            .map(|spec| spec.name)
            .collect::<Vec<_>>();
        assert!(names.contains(&"resume"));
        assert!(names.contains(&"rewind"));
        assert!(names.contains(&"review"));

        let alias_names = command_suggestions_for_prefix("temp")
            .into_iter()
            .map(|spec| spec.name)
            .collect::<Vec<_>>();
        assert_eq!(alias_names.first(), Some(&"temperature"));
    }

    #[test]
    fn command_suggestions_do_not_match_arguments() {
        let names = command_suggestions_for_prefix("number")
            .into_iter()
            .map(|spec| spec.name)
            .collect::<Vec<_>>();
        assert!(names.is_empty());
    }

    #[test]
    fn parse_command_supports_new_timeline_command() {
        let parsed = parse_command("/events 80").expect("timeline command should parse");
        assert_eq!(parsed.spec.id, CommandId::Timeline);
        assert_eq!(parsed.args, "80");
    }

    #[test]
    fn parse_command_supports_compact_alias() {
        let parsed = parse_command("/summarize").expect("compact command should parse");
        assert_eq!(parsed.spec.id, CommandId::Compact);
        assert_eq!(parsed.args, "");
    }

    #[test]
    fn parse_command_supports_plugin_inspector_command() {
        let parsed = parse_command("/plugin failed").expect("plugin command should parse");
        assert_eq!(parsed.spec.id, CommandId::Plugins);
        assert_eq!(parsed.args, "failed");
    }

    #[test]
    fn parse_command_supports_runtime_operator_commands() {
        assert_eq!(
            parse_command("/mcp")
                .expect("mcp command should parse")
                .spec
                .id,
            CommandId::Mcp
        );
        assert_eq!(
            parse_command("/lsp")
                .expect("lsp command should parse")
                .spec
                .id,
            CommandId::Lsp
        );
        assert_eq!(
            parse_command("/skill search")
                .expect("skills command should parse")
                .spec
                .id,
            CommandId::Skills
        );
        assert_eq!(
            parse_command("/operator")
                .expect("runtime command should parse")
                .spec
                .id,
            CommandId::Runtime
        );
    }

    #[test]
    fn parse_command_supports_workflow_commands() {
        assert_eq!(
            parse_command("/cost")
                .expect("cost command should parse")
                .spec
                .id,
            CommandId::Cost
        );
        assert_eq!(
            parse_command("/review auth flow")
                .expect("review command should parse")
                .spec
                .id,
            CommandId::Review
        );
        assert_eq!(
            parse_command("/perm")
                .expect("permissions command should parse")
                .spec
                .id,
            CommandId::Permissions
        );
        assert_eq!(
            parse_command("/config")
                .expect("config command should parse")
                .spec
                .id,
            CommandId::Config
        );
        assert_eq!(
            parse_command("/wt")
                .expect("worktree command should parse")
                .spec
                .id,
            CommandId::Worktree
        );
        assert_eq!(
            parse_command("/repo")
                .expect("git command should parse")
                .spec
                .id,
            CommandId::Git
        );
        assert_eq!(
            parse_command("/commit ship it")
                .expect("commit command should parse")
                .spec
                .id,
            CommandId::Commit
        );
        assert_eq!(
            parse_command("/pr ship it")
                .expect("pr command should parse")
                .spec
                .id,
            CommandId::Pr
        );
    }

    #[test]
    fn parse_command_supports_memory_command() {
        let parsed = parse_command("/mem forget user_role").expect("memory command should parse");
        assert_eq!(parsed.spec.id, CommandId::Memory);
        assert_eq!(parsed.args, "forget user_role");
    }

    #[test]
    fn parse_command_supports_pager_alias() {
        let parsed = parse_command("/view").expect("pager alias should parse");
        assert_eq!(parsed.spec.id, CommandId::Pager);
        assert_eq!(parsed.args, "");
    }

    #[test]
    fn parse_command_supports_resume_session_picker() {
        let parsed = parse_command("/recent").expect("resume alias should parse");
        assert_eq!(parsed.spec.id, CommandId::Resume);
        assert_eq!(parsed.args, "");
    }

    #[test]
    fn parse_command_supports_lineage_picker() {
        let parsed = parse_command("/branches").expect("lineage alias should parse");
        assert_eq!(parsed.spec.id, CommandId::Lineage);
        assert_eq!(parsed.args, "");
    }

    #[test]
    fn parse_command_supports_rewind_picker() {
        let parsed = parse_command("/backtrack").expect("rewind alias should parse");
        assert_eq!(parsed.spec.id, CommandId::Rewind);
        assert_eq!(parsed.args, "");
    }

    #[test]
    fn parse_command_supports_session_view_args() {
        let parsed = parse_command("/sessions subtree").expect("sessions command should parse");
        assert_eq!(parsed.spec.id, CommandId::Sessions);
        assert_eq!(parsed.args, "subtree");
    }
}
