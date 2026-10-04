//! The client-side command vocabulary.
//!
//! Built-in commands are declared once, by the server, as ordinary plugin
//! commands whose target is [`agena_plugin_sdk::CommandTarget::Client`]. The
//! declaration is published through the plugin surface catalog, so every
//! client sees the same names, aliases, usage lines and descriptions.
//!
//! A `Client` target only names an *action*; running it is the rendering
//! client's own job. [`ClientCommandAction`] is the closed vocabulary those
//! action names draw from: a client keeps the catalog commands whose action it
//! can spell here and drops the rest, which keeps one client's palette from
//! offering a surface another client does not have. The enum is deliberately
//! the only command list on the client side — slashes, aliases, summaries,
//! usage and examples all come from the published catalog.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

macro_rules! client_command_actions {
    ($($variant:ident => $action:literal, $slash:literal;)*) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        /// A built-in command action a client can run locally.
        pub enum ClientCommandAction {
            $($variant,)*
        }

        impl ClientCommandAction {
            /// Every action this client understands, in declared order.
            pub const ALL: &'static [ClientCommandAction] = &[$(Self::$variant,)*];

            /// The stable action name a `Client` command's target carries.
            pub const fn action(self) -> &'static str {
                match self {
                    $(Self::$variant => $action,)*
                }
            }

            /// The `/name` spelling this action's command is published under.
            ///
            /// Clients never resolve commands through this — the catalog owns
            /// the spelling. It is here so a build-time check can prove the
            /// published declaration and this vocabulary still describe one
            /// command, catching a rename that reached only one of them.
            pub const fn slash(self) -> &'static str {
                match self {
                    $(Self::$variant => $slash,)*
                }
            }

            /// Resolve a published `Client` target's action name.
            pub fn from_action(value: &str) -> Option<Self> {
                match value {
                    $($action => Some(Self::$variant),)*
                    _ => None,
                }
            }
        }
    };
}

client_command_actions! {
    Help => "help", "/help";
    Commands => "commands", "/commands";
    New => "new", "/new";
    Sessions => "sessions", "/sessions";
    Hub => "hub", "/hub";
    Lineage => "lineage", "/lineage";
    Rewind => "rewind", "/rewind";
    Rename => "rename", "/rename";
    Favorite => "favorite", "/favorite";
    Timeline => "timeline", "/timeline";
    Settings => "settings", "/settings";
    Model => "model", "/model";
    Commit => "commit", "/commit";
    Pr => "pr", "/pr";
    Export => "export", "/export";
    Pager => "pager", "/pager";
    Continue => "continue", "/continue";
    Compact => "compact", "/compact";
    UserInput => "user-input", "/user-input";
    Allow => "allow", "/allow";
    AllowAlways => "allow-always", "/allow-always";
    Deny => "deny", "/deny";
    DenyAlways => "deny-always", "/deny-always";
    Attach => "attach", "/attach";
    Download => "download", "/download";
    Editor => "editor", "/editor";
    Image => "image", "/image";
    Paste => "paste", "/paste";
    Copy => "copy", "/copy";
    CopyMessage => "copy-message", "/copy-message";
    CopyVisible => "copy-visible", "/copy-visible";
    Fork => "fork", "/fork";
    Children => "children", "/children";
    Parent => "parent", "/parent";
    Diagnostics => "diagnostics", "/diagnostics";
    Status => "status", "/status";
    Usage => "usage", "/usage";
    Activities => "activities", "/activities";
    Background => "background", "/background";
    Plan => "plan", "/plan";
    Side => "side", "/side";
    Btw => "btw", "/btw";
}

impl Serialize for ClientCommandAction {
    /// The action name is the wire spelling, so serialization goes through
    /// [`ClientCommandAction::action`] rather than a second rename table.
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.action())
    }
}

impl<'de> Deserialize<'de> for ClientCommandAction {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let action = String::deserialize(deserializer)?;
        Self::from_action(action.as_str()).ok_or_else(|| {
            serde::de::Error::custom(format!("unknown client command action `{action}`"))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::ClientCommandAction;

    #[test]
    fn the_vocabulary_has_no_duplicate_action_or_slash() {
        let actions = ClientCommandAction::ALL
            .iter()
            .map(|action| action.action())
            .collect::<std::collections::BTreeSet<_>>();
        let slashes = ClientCommandAction::ALL
            .iter()
            .map(|action| action.slash())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(actions.len(), ClientCommandAction::ALL.len());
        assert_eq!(slashes.len(), ClientCommandAction::ALL.len());
    }

    #[test]
    fn every_action_is_a_stable_identifier_and_round_trips_through_the_wire() {
        for action in ClientCommandAction::ALL {
            assert!(!action.action().is_empty());
            assert!(
                action
                    .action()
                    .chars()
                    .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-'),
                "{} is not a stable action name",
                action.action()
            );
            assert_eq!(
                ClientCommandAction::from_action(action.action()),
                Some(*action)
            );
            let encoded = serde_json::to_string(action).expect("serialize action");
            assert_eq!(encoded, format!("\"{}\"", action.action()));
            let decoded: ClientCommandAction =
                serde_json::from_str(encoded.as_str()).expect("deserialize action");
            assert_eq!(decoded, *action);
        }
        assert!(serde_json::from_str::<ClientCommandAction>("\"not-a-command\"").is_err());
    }

    #[test]
    fn every_slash_is_one_leading_slash_and_a_nonempty_name() {
        for action in ClientCommandAction::ALL {
            let name = action
                .slash()
                .strip_prefix('/')
                .expect("a slash spelling starts with `/`");
            assert!(!name.is_empty(), "{} has an empty slash", action.action());
            assert!(
                !name.contains('/'),
                "{} must not contain a second `/`",
                action.slash()
            );
            assert_eq!(name, name.trim());
        }
    }
}
