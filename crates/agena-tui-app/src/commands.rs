//! Slash-invocation parsing and this client's view of the published command
//! catalog.
//!
//! Commands are declared once, by the server, as plugin commands and published
//! through the plugin surface catalog — the `agena.commands` plugin declares the
//! built-ins every client renders. A client therefore owns only two things:
//! splitting `/<name> <args>` as a user types it, and keeping the catalog
//! entries it can actually run.
//!
//! A `CommandTarget::Client` command names an action in
//! [`ClientCommandAction`]; this client renders the ones it can spell and drops
//! the rest, so a command another client owns never appears here. Every piece of
//! user-facing text — slash, aliases, usage, summary, examples — comes from the
//! declaration rather than from a table in this crate.

use agena_api::client_command::ClientCommandAction;
use agena_plugin_host::sdk::{CommandDocs, CommandTarget};
use agena_plugin_host::{CommandCatalogItem, PluginSurfaceCatalog};
use agena_tui::i18n::I18n;

/// Split a composer line into the command name and its raw argument text.
///
/// `None` when the line is not a slash invocation at all, so the caller treats
/// it as an ordinary message. `//text` is the composer's escape for a literal
/// leading slash and is never a command, and a bare `/` names nothing.
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

/// A `Client`-targeted command this client knows how to run.
///
/// The declaration travels with it, so a caller reads the slash spelling, the
/// usage line and the summary from the same place the user does.
#[derive(Debug, Clone)]
pub(crate) struct ClientCommand {
    entry: CommandCatalogItem,
    action: ClientCommandAction,
}

impl ClientCommand {
    /// The local action to dispatch on.
    pub(crate) fn action(&self) -> ClientCommandAction {
        self.action
    }

    /// The declaration's identifier, used as the palette key.
    pub(crate) fn id(&self) -> &str {
        self.entry.command.id.as_str()
    }

    /// The name a user types after `/`.
    pub(crate) fn name(&self) -> String {
        self.entry
            .command
            .slash
            .as_deref()
            .map(|slash| slash.trim().trim_start_matches('/'))
            .filter(|name| !name.is_empty())
            .unwrap_or(self.entry.command.id.as_str())
            .to_string()
    }

    pub(crate) fn aliases(&self) -> &[String] {
        &self.entry.command.aliases
    }

    pub(crate) fn docs(&self) -> &CommandDocs {
        &self.entry.command.docs
    }

    /// Whether the declaration takes arguments at all. A command that documents
    /// no usage takes none, so arguments typed after it are a usage error worth
    /// showing rather than silently dropping.
    pub(crate) fn takes_arguments(&self) -> bool {
        self.docs().usage.is_some()
    }

    /// Whether an argument is required before the command can run. Only a
    /// required argument (`<name>`) gates execution; an optional one (`[name]`)
    /// is composed in the composer as usual.
    pub(crate) fn requires_arguments(&self) -> bool {
        self.docs()
            .usage
            .as_deref()
            .is_some_and(|usage| usage.trim_start().starts_with('<'))
    }

    /// `/name <usage>` as a user would type it, for the usage warning.
    pub(crate) fn invocation(&self) -> String {
        match self.docs().usage.as_deref() {
            Some(usage) => format!("/{} {usage}", self.name()),
            None => format!("/{}", self.name()),
        }
    }

    /// Compact label for the action-oriented command palette.
    ///
    /// Optional slash-command arguments remain available when typing in the
    /// composer, but they are implementation details for commands whose default
    /// action already opens an interactive surface. Commands that cannot run
    /// without text keep only their required argument in the label.
    pub(crate) fn palette_invocation(&self) -> String {
        if !self.requires_arguments() {
            return format!("/{}", self.name());
        }
        let usage = self.docs().usage.as_deref().unwrap_or_default();
        let required = usage
            .split_once(" [")
            .map(|(required, _)| required)
            .unwrap_or(usage);
        format!("/{} {required}", self.name())
    }

    /// Whether the command is offered in a client's command palette.
    fn shows_in_palette(&self) -> bool {
        let discoverability = &self.entry.command.discoverability;
        discoverability.catalog && discoverability.palette
    }

    /// Whether the command is recognized when typed as `/name` in a composer.
    fn recognizes_slash(&self) -> bool {
        let discoverability = &self.entry.command.discoverability;
        discoverability.catalog && discoverability.slash
    }

    fn matches_name(&self, name: &str) -> bool {
        self.name().eq_ignore_ascii_case(name)
            || self
                .aliases()
                .iter()
                .any(|alias| alias.eq_ignore_ascii_case(name))
    }

    fn name_matches_query(&self, query: &str) -> (bool, bool) {
        let name = self.name().to_ascii_lowercase();
        let exact = name == query || self.aliases().iter().any(|alias| alias == query);
        let prefix =
            name.starts_with(query) || self.aliases().iter().any(|alias| alias.starts_with(query));
        (exact, prefix)
    }
}

/// Every declared command, reduced to the ones this client can run.
///
/// `catalog` is `None` before the first plugin surface snapshot arrives; the
/// client then renders no commands at all rather than inventing a local list.
pub(crate) fn client_commands(catalog: Option<&PluginSurfaceCatalog>) -> Vec<ClientCommand> {
    let Some(catalog) = catalog else {
        return Vec::new();
    };
    catalog.commands.iter().filter_map(client_command).collect()
}

/// The commands a palette offers, in catalog order.
pub(crate) fn client_palette_commands(
    catalog: Option<&PluginSurfaceCatalog>,
) -> Vec<ClientCommand> {
    client_commands(catalog)
        .into_iter()
        .filter(ClientCommand::shows_in_palette)
        .collect()
}

/// Resolve the command that `/name` names, if this client declares one.
pub(crate) fn find_client_command(
    catalog: Option<&PluginSurfaceCatalog>,
    name: &str,
) -> Option<ClientCommand> {
    let name = name.trim().trim_start_matches('/');
    if name.is_empty() {
        return None;
    }
    client_commands(catalog)
        .into_iter()
        .find(|command| command.recognizes_slash() && command.matches_name(name))
}

/// Composer suggestions for a partial name: exact matches first, then prefixes.
pub(crate) fn client_command_suggestions(
    catalog: Option<&PluginSurfaceCatalog>,
    query: &str,
) -> Vec<ClientCommand> {
    let query = query.trim().to_ascii_lowercase();
    let commands = client_commands(catalog)
        .into_iter()
        .filter(ClientCommand::recognizes_slash)
        .collect::<Vec<_>>();
    if query.is_empty() {
        return commands;
    }

    let mut exact = Vec::new();
    let mut prefix = Vec::new();
    for command in commands {
        let (is_exact, is_prefix) = command.name_matches_query(query.as_str());
        if is_exact {
            exact.push(command);
        } else if is_prefix {
            prefix.push(command);
        }
    }
    exact.extend(prefix);
    exact
}

/// The one-line description a client shows for a command: its localized
/// summary key when the catalog answers for it, else the literal summary, else
/// the declaration's id so a row is never blank.
pub(crate) fn docs_summary(i18n: &I18n, docs: &CommandDocs, fallback: &str) -> String {
    if let Some(text) = docs
        .summary_key
        .as_deref()
        .and_then(|key| i18n.try_text(key))
    {
        return text;
    }
    if let Some(summary) = docs
        .summary
        .as_deref()
        .map(str::trim)
        .filter(|summary| !summary.is_empty())
    {
        return summary.to_string();
    }
    fallback.to_string()
}

fn client_command(entry: &CommandCatalogItem) -> Option<ClientCommand> {
    if !entry.command.discoverability.catalog {
        return None;
    }
    let CommandTarget::Client { action } = &entry.command.target else {
        return None;
    };
    // A `Client` command another client owns is not ours to render; dropping it
    // here is what keeps one client's palette out of another's.
    let action = ClientCommandAction::from_action(action.as_str())?;
    Some(ClientCommand {
        entry: entry.clone(),
        action,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        client_command_suggestions, client_commands, client_palette_commands, docs_summary,
        find_client_command, parse_invocation,
    };
    use agena_plugin_host::sdk::{
        CommandDefinition, CommandDiscoverability, CommandDocs, CommandTarget, SettingsContract,
    };
    use agena_plugin_host::{CommandCatalogItem, PluginKey, PluginSurfaceCatalog};
    use agena_tui::i18n::I18n;

    fn catalog_item(
        id: &str,
        slash: Option<&str>,
        aliases: &[&str],
        usage: Option<&str>,
        discoverability: CommandDiscoverability,
        target: CommandTarget,
    ) -> CommandCatalogItem {
        CommandCatalogItem {
            plugin_id: "agena.commands"
                .parse::<PluginKey>()
                .expect("valid plugin id"),
            accepts_empty_input: true,
            default_input: serde_json::json!({}),
            command: CommandDefinition {
                id: id.to_string(),
                title: id.to_string(),
                group: "Built-in".to_string(),
                category: Some("Client".to_string()),
                slash: slash.map(str::to_string),
                aliases: aliases.iter().map(|alias| (*alias).to_string()).collect(),
                docs: CommandDocs {
                    usage: usage.map(str::to_string),
                    summary_key: Some(format!("command-{id}-summary")),
                    ..CommandDocs::default()
                },
                input: SettingsContract::empty_object("No input", ""),
                discoverability,
                target,
            },
        }
    }

    fn client_target(action: &str) -> CommandTarget {
        CommandTarget::Client {
            action: action.to_string(),
        }
    }

    fn catalog(commands: Vec<CommandCatalogItem>) -> PluginSurfaceCatalog {
        PluginSurfaceCatalog {
            commands,
            terminal: Default::default(),
        }
    }

    #[test]
    fn parse_invocation_splits_a_slash_name_from_its_arguments() {
        assert_eq!(
            parse_invocation("/pr fix the build"),
            Some(("pr", "fix the build"))
        );
        assert_eq!(parse_invocation("  /side  "), Some(("side", "")));
        assert_eq!(
            parse_invocation("/download   a/b.zip"),
            Some(("download", "a/b.zip"))
        );
    }

    #[test]
    fn parse_invocation_rejects_lines_that_are_not_commands() {
        assert_eq!(parse_invocation("hello"), None);
        // `//` is the composer's escape for a literal leading slash.
        assert_eq!(parse_invocation("//not a command"), None);
        assert_eq!(parse_invocation("/"), None);
        assert_eq!(parse_invocation("   "), None);
    }

    #[test]
    fn only_client_targeted_commands_the_client_can_spell_are_rendered() {
        let catalog = catalog(vec![
            catalog_item(
                "help",
                Some("/help"),
                &["?"],
                None,
                CommandDiscoverability::default(),
                client_target("help"),
            ),
            // Declared for another client: this build has no such action.
            catalog_item(
                "not-ours",
                Some("/not-ours"),
                &[],
                None,
                CommandDiscoverability::default(),
                client_target("an-action-only-the-web-client-has"),
            ),
            // Server-owned targets are not client commands at all.
            catalog_item(
                "remote",
                Some("/remote"),
                &[],
                None,
                CommandDiscoverability::default(),
                CommandTarget::Method {
                    handler: "remote.run".to_string(),
                },
            ),
        ]);

        let commands = client_commands(Some(&catalog));
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].id(), "help");
        assert_eq!(commands[0].name(), "help");
        assert!(find_client_command(Some(&catalog), "?").is_some());
        assert!(find_client_command(Some(&catalog), "not-ours").is_none());
        assert!(find_client_command(Some(&catalog), "remote").is_none());
    }

    #[test]
    fn palette_and_slash_discoverability_are_read_separately() {
        let slash_only = CommandDiscoverability {
            palette: false,
            ..CommandDiscoverability::default()
        };
        let palette_only = CommandDiscoverability {
            slash: false,
            ..CommandDiscoverability::default()
        };
        let catalog = catalog(vec![
            catalog_item(
                "slash-only",
                Some("/slash-only"),
                &[],
                None,
                slash_only,
                client_target("help"),
            ),
            catalog_item(
                "palette-only",
                Some("/palette-only"),
                &[],
                None,
                palette_only,
                client_target("commands"),
            ),
        ]);

        let palette = client_palette_commands(Some(&catalog));
        assert_eq!(
            palette
                .iter()
                .map(|command| command.id())
                .collect::<Vec<_>>(),
            vec!["palette-only"]
        );
        assert!(find_client_command(Some(&catalog), "slash-only").is_some());
        assert!(find_client_command(Some(&catalog), "palette-only").is_none());
    }

    #[test]
    fn usage_drives_argument_requirements_and_palette_labels() {
        let catalog = catalog(vec![
            catalog_item(
                "pr",
                Some("/pr"),
                &[],
                Some("<title> [--body <text>] [--base <branch>]"),
                CommandDiscoverability::default(),
                client_target("pr"),
            ),
            catalog_item(
                "export",
                Some("/export"),
                &[],
                Some("[path]"),
                CommandDiscoverability::default(),
                client_target("export"),
            ),
            catalog_item(
                "sessions",
                Some("/sessions"),
                &[],
                None,
                CommandDiscoverability::default(),
                client_target("sessions"),
            ),
        ]);
        let find =
            |name: &str| find_client_command(Some(&catalog), name).expect("declared command");

        let pr = find("pr");
        assert!(pr.requires_arguments());
        assert!(pr.takes_arguments());
        assert_eq!(pr.palette_invocation(), "/pr <title>");
        assert_eq!(
            pr.invocation(),
            "/pr <title> [--body <text>] [--base <branch>]"
        );

        // An optional argument runs without one, so the palette keeps the bare name.
        let export = find("export");
        assert!(!export.requires_arguments());
        assert!(export.takes_arguments());
        assert_eq!(export.palette_invocation(), "/export");

        let sessions = find("sessions");
        assert!(!sessions.takes_arguments());
        assert_eq!(sessions.palette_invocation(), "/sessions");
        assert_eq!(sessions.invocation(), "/sessions");
    }

    #[test]
    fn suggestions_put_exact_name_and_alias_matches_before_prefixes() {
        let catalog = catalog(vec![
            catalog_item(
                "sessions",
                Some("/sessions"),
                &[],
                None,
                CommandDiscoverability::default(),
                client_target("sessions"),
            ),
            catalog_item(
                "side",
                Some("/side"),
                &["btw"],
                None,
                CommandDiscoverability::default(),
                client_target("side"),
            ),
            catalog_item(
                "settings",
                Some("/settings"),
                &["config"],
                None,
                CommandDiscoverability::default(),
                client_target("settings"),
            ),
        ]);

        let ids = |query: &str| {
            client_command_suggestions(Some(&catalog), query)
                .iter()
                .map(|command| command.id().to_string())
                .collect::<Vec<_>>()
        };

        assert_eq!(ids(""), vec!["sessions", "side", "settings"]);
        assert_eq!(ids("se"), vec!["sessions", "settings"]);
        assert_eq!(ids("btw"), vec!["side"]);
        assert!(ids("nothing-matches").is_empty());
    }

    #[test]
    fn summary_prefers_the_message_key_then_the_literal_then_the_id() {
        let i18n = I18n::english();

        let keyed = CommandDocs {
            summary_key: Some("command-side-summary".to_string()),
            ..CommandDocs::default()
        };
        assert_eq!(
            docs_summary(&i18n, &keyed, "side"),
            i18n.text("command-side-summary")
        );

        // A key no locale carries falls through to the literal summary.
        let missing_key = CommandDocs {
            summary_key: Some("command-not-translated-anywhere-summary".to_string()),
            summary: Some("Run an untranslated command.".to_string()),
            ..CommandDocs::default()
        };
        assert_eq!(
            docs_summary(&i18n, &missing_key, "example"),
            "Run an untranslated command."
        );

        let bare = CommandDocs::default();
        assert_eq!(docs_summary(&i18n, &bare, "example.run"), "example.run");
    }

    #[test]
    fn an_empty_catalog_offers_nothing_rather_than_a_local_fallback() {
        assert!(client_commands(None).is_empty());
        assert!(client_palette_commands(None).is_empty());
        assert!(find_client_command(None, "help").is_none());
        assert!(client_command_suggestions(None, "").is_empty());
    }
}
