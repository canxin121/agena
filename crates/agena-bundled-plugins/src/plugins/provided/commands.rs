//! The built-in slash commands shared by Agena clients.

use agena_plugin_host::sdk::{
    ApplicableSessions, CommandDefinition, CommandDocs, CommandTarget, Plugin, PluginManifest,
    PluginManifestTranslation, SettingsContract, ToolTag,
};

pub(crate) const COMMANDS_PLUGIN_ID: &str = "agena.commands";

pub(crate) struct CommandsPlugin;

/// `action` is the client-side vocabulary name; `slash` is what a user types.
/// `usage` is the argument shape shown in the palette (empty when the command
/// takes none), and `summary_key` is the Fluent / vue-i18n key for its one-line
/// description. Which conversation kinds offer the command is not stored here:
/// [`built_in_applies_to`] owns that one policy table, so a command's scope is
/// read in one place instead of beside every entry.
struct BuiltInCommand {
    action: &'static str,
    slash: &'static str,
    aliases: &'static [&'static str],
    usage: Option<&'static str>,
    summary_key: &'static str,
}

/// Palette order is part of the contract: both clients render this sequence
/// for a blank query, so a new built-in belongs at the position a user would
/// look for it rather than wherever it was appended.
const BUILT_IN_COMMANDS: &[BuiltInCommand] = &[
    BuiltInCommand {
        action: "help",
        slash: "/help",
        aliases: &["?"],
        usage: None,
        summary_key: "command-help-summary",
    },
    BuiltInCommand {
        action: "commands",
        slash: "/commands",
        aliases: &["palette"],
        usage: None,
        summary_key: "command-commands-summary",
    },
    BuiltInCommand {
        action: "new",
        slash: "/new",
        // `/clear` is its own command now, with its own meaning: it resets the
        // current conversation instead of starting another one. The old
        // `clear` alias here would have made that name ambiguous, and
        // registration refuses two declarations that overlap.
        aliases: &[],
        usage: None,
        summary_key: "command-new-summary",
    },
    BuiltInCommand {
        action: "sessions",
        slash: "/sessions",
        aliases: &[],
        usage: None,
        summary_key: "command-sessions-summary",
    },
    BuiltInCommand {
        action: "hub",
        slash: "/hub",
        aliases: &[],
        usage: None,
        summary_key: "command-hub-summary",
    },
    BuiltInCommand {
        action: "bot",
        slash: "/bot",
        aliases: &[],
        usage: None,
        summary_key: "command-bot-summary",
    },
    BuiltInCommand {
        action: "lineage",
        slash: "/lineage",
        aliases: &["branch-history", "branches"],
        usage: None,
        summary_key: "command-lineage-summary",
    },
    BuiltInCommand {
        action: "rewind",
        slash: "/rewind",
        aliases: &["backtrack"],
        usage: Some("[message_id] [in-place]"),
        summary_key: "command-rewind-summary",
    },
    BuiltInCommand {
        action: "rename",
        slash: "/rename",
        aliases: &["title"],
        usage: None,
        summary_key: "command-rename-summary",
    },
    BuiltInCommand {
        action: "favorite",
        slash: "/favorite",
        aliases: &["star"],
        usage: None,
        summary_key: "command-favorite-summary",
    },
    BuiltInCommand {
        action: "timeline",
        slash: "/timeline",
        aliases: &["events"],
        usage: None,
        summary_key: "command-timeline-summary",
    },
    BuiltInCommand {
        action: "settings",
        slash: "/settings",
        aliases: &["config"],
        usage: None,
        summary_key: "command-settings-summary",
    },
    BuiltInCommand {
        action: "model",
        slash: "/model",
        aliases: &[],
        usage: None,
        summary_key: "command-model-summary",
    },
    BuiltInCommand {
        action: "commit",
        slash: "/commit",
        aliases: &[],
        usage: Some("<message>"),
        summary_key: "command-commit-summary",
    },
    BuiltInCommand {
        action: "pr",
        slash: "/pr",
        aliases: &[],
        usage: Some("<title> [--body <text>] [--base <branch>] [--head <branch>]"),
        summary_key: "command-pr-summary",
    },
    BuiltInCommand {
        action: "export",
        slash: "/export",
        aliases: &["save"],
        usage: Some("[path]"),
        summary_key: "command-export-summary",
    },
    BuiltInCommand {
        action: "pager",
        slash: "/pager",
        aliases: &["view", "less"],
        usage: None,
        summary_key: "command-pager-summary",
    },
    BuiltInCommand {
        action: "continue",
        slash: "/continue",
        aliases: &["resume-run"],
        usage: None,
        summary_key: "command-continue-summary",
    },
    BuiltInCommand {
        action: "compact",
        slash: "/compact",
        aliases: &["compress", "summarize"],
        usage: None,
        summary_key: "command-compact-summary",
    },
    BuiltInCommand {
        action: "clear",
        slash: "/clear",
        aliases: &["reset"],
        usage: None,
        summary_key: "command-clear-summary",
    },
    BuiltInCommand {
        action: "attach",
        slash: "/attach",
        aliases: &["file"],
        usage: None,
        summary_key: "command-attach-summary",
    },
    BuiltInCommand {
        action: "editor",
        slash: "/editor",
        aliases: &["edit"],
        usage: None,
        summary_key: "command-editor-summary",
    },
    BuiltInCommand {
        action: "image",
        slash: "/image",
        aliases: &[],
        usage: None,
        summary_key: "command-image-summary",
    },
    BuiltInCommand {
        action: "copy",
        slash: "/copy",
        aliases: &["yank"],
        usage: None,
        summary_key: "command-copy-summary",
    },
    BuiltInCommand {
        action: "copy-message",
        slash: "/copy-message",
        aliases: &["copy-last", "copy-assistant"],
        usage: None,
        summary_key: "command-copy-message-summary",
    },
    BuiltInCommand {
        action: "copy-visible",
        slash: "/copy-visible",
        aliases: &[],
        usage: None,
        summary_key: "command-copy-visible-summary",
    },
    BuiltInCommand {
        action: "fork",
        slash: "/fork",
        aliases: &["branch"],
        usage: None,
        summary_key: "command-fork-summary",
    },
    BuiltInCommand {
        action: "children",
        slash: "/children",
        aliases: &["child"],
        usage: None,
        summary_key: "command-children-summary",
    },
    BuiltInCommand {
        action: "parent",
        slash: "/parent",
        aliases: &[],
        usage: None,
        summary_key: "command-parent-summary",
    },
    BuiltInCommand {
        action: "diagnostics",
        slash: "/diagnostics",
        aliases: &["feedback"],
        usage: None,
        summary_key: "command-diagnostics-summary",
    },
    BuiltInCommand {
        action: "status",
        slash: "/status",
        aliases: &[],
        usage: None,
        summary_key: "command-status-summary",
    },
    BuiltInCommand {
        action: "usage",
        slash: "/usage",
        aliases: &["stats", "analytics"],
        usage: None,
        summary_key: "command-usage-summary",
    },
    BuiltInCommand {
        action: "schedules",
        slash: "/schedules",
        aliases: &[],
        usage: None,
        summary_key: "command-schedules-summary",
    },
    BuiltInCommand {
        action: "queue",
        slash: "/queue",
        aliases: &[],
        usage: None,
        summary_key: "command-queue-summary",
    },
    BuiltInCommand {
        action: "background",
        slash: "/background",
        aliases: &[],
        usage: None,
        summary_key: "command-background-summary",
    },
    BuiltInCommand {
        action: "inline-ui",
        slash: "/ui",
        aliases: &["inline-ui"],
        usage: Some("[reference]"),
        summary_key: "command-inline-ui-summary",
    },
    BuiltInCommand {
        action: "integrations",
        slash: "/integrations",
        aliases: &["connections"],
        usage: None,
        summary_key: "command-integrations-summary",
    },
    BuiltInCommand {
        action: "assistant",
        slash: "/assistant",
        aliases: &["companion"],
        usage: None,
        summary_key: "command-assistant-summary",
    },
    BuiltInCommand {
        action: "plan",
        slash: "/plan",
        aliases: &["plan-view", "show-plan"],
        usage: None,
        summary_key: "command-plan-summary",
    },
    BuiltInCommand {
        action: "side",
        slash: "/side",
        aliases: &["aside"],
        usage: Some("[question]"),
        summary_key: "command-side-summary",
    },
    BuiltInCommand {
        action: "btw",
        slash: "/btw",
        aliases: &[],
        usage: Some("[question]"),
        summary_key: "command-btw-summary",
    },
];

fn built_in_commands() -> Vec<CommandDefinition> {
    BUILT_IN_COMMANDS
        .iter()
        .map(|built_in| CommandDefinition {
            // Command identity follows its primary spelling; the client action
            // may also be an alias, as with /ui and /inline-ui.
            id: built_in.slash.trim_start_matches('/').to_string(),
            title: built_in_summary_key_title(built_in.summary_key),
            group: GROUP.to_string(),
            category: Some(CATEGORY.to_string()),
            slash: Some(built_in.slash.to_string()),
            aliases: built_in
                .aliases
                .iter()
                .map(|alias| (*alias).to_string())
                .collect(),
            applies_to: built_in_applies_to(built_in.action),
            // Built-in commands document themselves through the client's
            // message catalog, so only the key is declared here. Usage, where
            // the command takes arguments, is the one literal: it is a shape,
            // not prose, and both clients render it verbatim.
            docs: CommandDocs {
                usage: built_in.usage.map(str::to_string),
                summary_key: Some(built_in.summary_key.to_string()),
                ..CommandDocs::default()
            },
            input: SettingsContract::empty_object("No input", ""),
            target: CommandTarget::Client {
                action: built_in.action.to_string(),
            },
        })
        .collect()
}

/// The title a client shows before it has resolved the summary key. Deriving
/// it from the key keeps the fallback honest instead of inventing display copy
/// in the declaration.
fn built_in_summary_key_title(summary_key: &str) -> String {
    summary_key
        .trim_start_matches("command-")
        .trim_end_matches("-summary")
        .to_string()
}

/// Which conversation kinds a built-in command is offered to.
///
/// The list below names the built-ins that cannot mean anything in the
/// operator Bot. `/new` starts a project conversation, and `/commit` and `/pr`
/// act on a project's Git state: the Bot has no working directory by design,
/// so offering them there would promise an action the runtime cannot perform.
/// The client hides them from the Bot palette, and the server refuses a direct
/// dispatch.
///
/// Everything else stays available in both kinds — session navigation,
/// transcript, model, export, settings, and the commands that open another
/// surface all mean the same thing in either conversation. A new built-in
/// therefore defaults to both kinds; add it here only when it genuinely needs
/// a project directory.
const PROJECT_ONLY_BUILT_INS: &[&str] = &["new", "commit", "pr"];

fn built_in_applies_to(action: &str) -> ApplicableSessions {
    if PROJECT_ONLY_BUILT_INS.contains(&action) {
        ApplicableSessions::project()
    } else {
        ApplicableSessions::all()
    }
}

/// Both clients group built-ins together; `category` distinguishes them from
/// plugin-provided commands in the same group.
const GROUP: &str = "Built-in";
const CATEGORY: &str = "Client";

impl Plugin for CommandsPlugin {
    fn manifest(&self) -> PluginManifest {
        let mut manifest = PluginManifest::new("agena", "commands", env!("CARGO_PKG_VERSION"));
        manifest.display_name = Some("Commands".into());
        manifest.icon = Some(crate::plugin_metadata::icon(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/plugin-icons/commands.svg"
        ))));
        manifest.authors = vec!["canxin121".into()];
        manifest.tags = vec![ToolTag::Discovery, ToolTag::Custom("commands".into())];
        manifest.summary =
            Some("Declare the built-in slash commands shared by Agena clients.".into());
        manifest.help =
            Some("Open /commands to see the actions available in the current conversation.".into());
        for (language, summary) in [
            ("zh-CN", "声明 Agena 客户端共用的内置斜杠命令。"),
            ("zh-TW", "宣告 Agena 用戶端共用的內建斜線命令。"),
            (
                "ja-JP",
                "Agena クライアント共通の組み込みスラッシュコマンドを宣言します。",
            ),
            (
                "ko-KR",
                "Agena 클라이언트가 공유하는 기본 슬래시 명령을 선언합니다.",
            ),
            (
                "fr-FR",
                "Déclare les commandes intégrées communes aux clients Agena.",
            ),
            (
                "de-DE",
                "Deklariert die integrierten Slash-Befehle für Agena-Clients.",
            ),
            (
                "es-ES",
                "Declara los comandos integrados que comparten los clientes de Agena.",
            ),
            (
                "hi-IN",
                "Agena क्लाइंट के साझा अंतर्निहित स्लैश कमांड घोषित करता है।",
            ),
            (
                "ar-SA",
                "يعرّف أوامر الشرطة المائلة المضمّنة المشتركة بين عملاء Agena.",
            ),
            (
                "pt-BR",
                "Declara os comandos de barra integrados aos clientes do Agena.",
            ),
        ] {
            manifest.translations.insert(
                language.into(),
                PluginManifestTranslation {
                    summary: Some(summary.into()),
                    help: (language == "zh-CN")
                        .then(|| "使用 /commands 查看当前会话可用的操作。".into()),
                    ..Default::default()
                },
            );
        }
        manifest.commands = built_in_commands();
        manifest
    }
}

#[cfg(test)]
mod bot_command_tests {
    use super::*;

    #[test]
    fn public_catalog_omits_retired_task_commands_and_keeps_runtime_controls() {
        let commands = built_in_commands();
        for name in ["task", "tasks", "task-board", "activities"] {
            assert!(commands.iter().all(|command| {
                command.slash.as_deref() != Some(format!("/{name}").as_str())
                    && !command.aliases.iter().any(|alias| alias == name)
                    && command.target
                        != CommandTarget::Client {
                            action: name.into(),
                        }
            }));
        }
        for slash in ["/bot", "/schedules", "/background", "/queue"] {
            assert!(
                commands
                    .iter()
                    .any(|command| command.slash.as_deref() == Some(slash))
            );
        }
    }

    #[test]
    fn bot_is_a_standalone_client_command() {
        let bot = built_in_commands()
            .into_iter()
            .find(|c| c.slash.as_deref() == Some("/bot"))
            .unwrap();
        assert_eq!(
            bot.target,
            CommandTarget::Client {
                action: "bot".into()
            }
        );
        assert_eq!(bot.docs.summary_key.as_deref(), Some("command-bot-summary"));
        assert!(bot.aliases.is_empty());
        let assistant = BUILT_IN_COMMANDS
            .iter()
            .find(|c| c.action == "assistant")
            .unwrap();
        assert!(!assistant.aliases.contains(&"bot"));
    }

    /// The policy table is the only place a built-in's scope lives, so the
    /// whole catalog must be free of two declarations that answer to one name
    /// in the same conversation. `agena.commands` declares `/clear` and `/new`
    /// itself; an alias on either that collides with the other would now fail
    /// registration, and this test states the intent instead of letting it
    /// surface as a load-time block.
    #[test]
    fn built_in_names_are_unambiguous_within_one_conversation_kind() {
        let commands = built_in_commands();
        for (index, left) in commands.iter().enumerate() {
            left.validate()
                .unwrap_or_else(|error| panic!("`{}` is invalid: {error}", left.id));
            for right in commands.iter().skip(index + 1) {
                assert_eq!(
                    left.conflicting_name(right),
                    None,
                    "`{}` and `{}` both answer to one slash",
                    left.id,
                    right.id
                );
            }
        }
    }

    /// `/new`, `/commit` and `/pr` act on a project working directory. The
    /// Bot has none by design, so those rows must not reach its palette.
    #[test]
    fn project_only_built_ins_declare_the_project_kind() {
        for command in built_in_commands() {
            let action = match &command.target {
                CommandTarget::Client { action } => action.as_str(),
                other => panic!("built-in `{}` has target {other:?}", command.id),
            };
            let expected = if PROJECT_ONLY_BUILT_INS.contains(&action) {
                ApplicableSessions::project()
            } else {
                ApplicableSessions::all()
            };
            assert_eq!(command.applies_to, expected, "`/{action}`");
        }
    }
}
