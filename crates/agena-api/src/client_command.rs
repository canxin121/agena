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
    Attach => "attach", "/attach";
    Editor => "editor", "/editor";
    Image => "image", "/image";
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
    Queue => "queue", "/queue";
    Background => "background", "/background";
    Plan => "plan", "/plan";
    InlineUi => "inline-ui", "/ui";
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
