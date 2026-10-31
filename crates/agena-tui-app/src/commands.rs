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
use agena_plugin_sdk::inspection::{CommandCatalogItem, PluginSurfaceCatalog};
use agena_plugin_sdk::{CommandDocs, CommandTarget};
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
        .find(|command| command.matches_name(name))
}

/// Composer suggestions for a partial name: exact matches first, then prefixes.
pub(crate) fn client_command_suggestions(
    catalog: Option<&PluginSurfaceCatalog>,
    query: &str,
) -> Vec<ClientCommand> {
    let query = query.trim().to_ascii_lowercase();
    let commands = client_commands(catalog);
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
