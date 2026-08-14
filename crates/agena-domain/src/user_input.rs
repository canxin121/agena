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

/// Origin of an interactive user-input request: the runtime's own host
/// `ask_user` (`Host`) vs a third-party/tool `interaction.ask` (`Plugin`).
/// This is the typed replacement for the historical `host-input:` request-id
/// prefix, which remains only an opaque correlation id.
///
/// Serializes to the plain string (`"host"` / `"plugin"`) so the wire and
/// stored-content shapes are byte-identical to the previous string protocol;
/// unknown or empty values deserialize leniently to `Plugin` — the safe
/// default for third-party/tool asks.
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

impl From<String> for UserInputSource {
    fn from(source: String) -> Self {
        match source.as_str() {
            "host" => Self::Host,
            _ => Self::Plugin,
        }
    }
}

impl From<&str> for UserInputSource {
    fn from(source: &str) -> Self {
        Self::from(source.to_owned())
    }
}

impl Serialize for UserInputSource {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for UserInputSource {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // Accept `null`/absent (via the field's serde default) and any string;
        // unknown strings fall back to the safe `Plugin` default.
        let source = Option::<String>::deserialize(deserializer)?;
        Ok(source.map(UserInputSource::from).unwrap_or_default())
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
    use super::{
        PendingInteractiveRequestKind, UserInputKind, UserInputReplyKind, UserInputSource,
    };
    use crate::UserInputRequest;
    use serde_json::json;

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

    #[test]
    fn user_input_kind_serializes_to_the_plain_string() {
        assert_eq!(
            serde_json::to_string(&UserInputKind::Review).unwrap(),
            "\"review\""
        );
        assert_eq!(
            serde_json::to_string(&UserInputKind::AskUser).unwrap(),
            "\"ask_user\""
        );
        assert_eq!(
            serde_json::to_string(&UserInputKind::Custom("x".into())).unwrap(),
            "\"x\""
        );
    }

    #[test]
    fn user_input_kind_deserializes_leniently() {
        assert_eq!(
            serde_json::from_value::<UserInputKind>(json!("review")).unwrap(),
            UserInputKind::Review
        );
        assert_eq!(
            serde_json::from_value::<UserInputKind>(json!("ask_user")).unwrap(),
            UserInputKind::AskUser
        );
        assert_eq!(
            serde_json::from_value::<UserInputKind>(json!("weird")).unwrap(),
            UserInputKind::Custom("weird".into())
        );
        // Empty, null, and absent (field default) all fall back to AskUser.
        assert_eq!(
            serde_json::from_value::<UserInputKind>(json!("")).unwrap(),
            UserInputKind::AskUser
        );
        assert_eq!(
            serde_json::from_value::<UserInputKind>(serde_json::Value::Null).unwrap(),
            UserInputKind::AskUser
        );
        let request: UserInputRequest = serde_json::from_value(
            json!({"request_id": "r1", "created_at": "2026-01-01T00:00:00Z"}),
        )
        .unwrap();
        assert_eq!(request.kind, UserInputKind::AskUser);
    }

    #[test]
    fn user_input_source_serializes_to_the_plain_string() {
        assert_eq!(
            serde_json::to_string(&UserInputSource::Host).unwrap(),
            "\"host\""
        );
        assert_eq!(
            serde_json::to_string(&UserInputSource::Plugin).unwrap(),
            "\"plugin\""
        );
    }

    #[test]
    fn user_input_source_deserializes_leniently() {
        assert_eq!(
            serde_json::from_value::<UserInputSource>(json!("host")).unwrap(),
            UserInputSource::Host
        );
        assert_eq!(
            serde_json::from_value::<UserInputSource>(json!("plugin")).unwrap(),
            UserInputSource::Plugin
        );
        // Unknown, empty, and null (field default) all fall back to Plugin,
        // the safe default for third-party/tool asks.
        assert_eq!(
            serde_json::from_value::<UserInputSource>(json!("weird")).unwrap(),
            UserInputSource::Plugin
        );
        assert_eq!(
            serde_json::from_value::<UserInputSource>(json!("")).unwrap(),
            UserInputSource::Plugin
        );
        assert_eq!(
            serde_json::from_value::<UserInputSource>(serde_json::Value::Null).unwrap(),
            UserInputSource::Plugin
        );
    }

    #[test]
    fn user_input_source_defaults_to_plugin_on_legacy_rows() {
        let request: UserInputRequest = serde_json::from_value(
            json!({"request_id": "r1", "created_at": "2026-01-01T00:00:00Z"}),
        )
        .unwrap();
        assert_eq!(request.source, UserInputSource::Plugin);
    }

    #[test]
    fn user_input_request_round_trips_preserving_kind() {
        let request = UserInputRequest {
            request_id: "r1".to_owned(),
            session_id: Some(7),
            title: "Approve?".to_owned(),
            body_markdown: "## Proposed Plan".to_owned(),
            kind: UserInputKind::Review,
            source: UserInputSource::Host,
            auto_resolution_ms: None,
            presented_at: None,
            questions: Vec::new(),
            created_at: chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
                .unwrap()
                .with_timezone(&chrono::Utc),
        };
        let value = serde_json::to_value(&request).unwrap();
        assert_eq!(value["kind"], json!("review"));
        assert_eq!(value["source"], json!("host"));
        let back: UserInputRequest = serde_json::from_value(value).unwrap();
        assert_eq!(back.kind, UserInputKind::Review);
        assert_eq!(back.source, UserInputSource::Host);
        assert_eq!(back.request_id, "r1");
        assert_eq!(back, request);
    }
}
