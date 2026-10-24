use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error as DeError};

/// How an interactive user-input request was resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UserInputReplyKind {
    Submit,
    Cancel,
    Timeout,
}

/// Display kind of a user-input request, canonicalized from the free-form
/// plugin `kind` string at the runtime boundary (third-party plugins may send
/// arbitrary non-empty strings, so the SDK keeps `String`; unknown values are
/// preserved as `Custom`). `Review` drives the TUI's single-choice plan-review
/// dialog; `AskUser` is the explicit default chosen by the runtime.
///
/// Serializes to the plain string (`"review"` / `"ask_user"` / the custom
/// value). The decoder accepts one required non-empty string.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum UserInputKind {
    Review,
    #[default]
    AskUser,
    Custom(String),
}

impl UserInputKind {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Review => "review",
            Self::AskUser => "ask_user",
            Self::Custom(kind) => kind,
        }
    }
}

impl From<String> for UserInputKind {
    fn from(kind: String) -> Self {
        match kind.as_str() {
            "review" => Self::Review,
            "" | "ask_user" => Self::AskUser,
            _ => Self::Custom(kind),
        }
    }
}

impl From<&str> for UserInputKind {
    fn from(kind: &str) -> Self {
        Self::from(kind.to_owned())
    }
}

impl Serialize for UserInputKind {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for UserInputKind {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let kind = String::deserialize(deserializer)?;
        if kind.is_empty() {
            return Err(D::Error::invalid_value(
                serde::de::Unexpected::Str(&kind),
                &"a non-empty user-input kind",
            ));
        }
        Ok(UserInputKind::from(kind))
    }
}

/// Origin of an interactive user-input request: the runtime's own host
/// `ask_user` (`Host`) vs a third-party/tool `interaction.ask` (`Plugin`).
///
/// Serializes to the plain string (`"host"` / `"plugin"`). The decoder
/// accepts only those two canonical values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UserInputSource {
    Host,
    #[default]
    Plugin,
}

impl UserInputSource {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Host => "host",
            Self::Plugin => "plugin",
        }
    }
}

impl Serialize for UserInputSource {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for UserInputSource {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        match String::deserialize(deserializer)?.as_str() {
            "host" => Ok(Self::Host),
            "plugin" => Ok(Self::Plugin),
            other => Err(D::Error::unknown_variant(other, &["host", "plugin"])),
        }
    }
}

/// Category of an interactive request that is awaiting a reply.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PendingInteractiveRequestKind {
    Permission,
    UserInput,
}
