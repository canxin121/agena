use serde::{Deserialize, Deserializer, Serialize, Serializer};

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
/// arbitrary strings, so the SDK keeps `String`; the coercion to this enum is
/// lenient). `Review` drives the TUI's single-choice plan-review dialog;
/// `AskUser` is the default for anything without an explicit kind.
///
/// Serializes to the plain string (`"review"` / `"ask_user"` / the custom
/// value) so the wire and stored-content shapes are byte-identical to the
/// previous free string; unknown or empty values deserialize leniently.
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
        // Accept `null`/absent (via the field's serde default) and any string;
        // unknown strings are preserved as `Custom`.
        let kind = Option::<String>::deserialize(deserializer)?;
        Ok(kind.map(UserInputKind::from).unwrap_or_default())
    }
}

/// Category of an interactive request that is awaiting a reply.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PendingInteractiveRequestKind {
    Permission,
    UserInput,
}

#[cfg(test)]
mod tests {
    use super::{PendingInteractiveRequestKind, UserInputReplyKind};

    #[test]
    fn reply_kind_has_stable_wire_spelling() {
        assert_eq!(
            serde_json::to_string(&UserInputReplyKind::Timeout).unwrap(),
            "\"timeout\""
        );
    }

    #[test]
    fn pending_request_kind_has_stable_wire_spelling() {
        assert_eq!(
            serde_json::to_string(&PendingInteractiveRequestKind::UserInput).unwrap(),
            "\"user_input\""
        );
    }
}
