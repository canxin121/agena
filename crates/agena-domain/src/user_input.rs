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
/// value). The decoder accepts one required non-empty string and never treats
/// `null` or a missing field as a historical request.
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
/// This is the typed replacement for the historical `host-input:` request-id
/// prefix, which remains only an opaque correlation id.
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
    fn user_input_kind_deserializes_canonical_values() {
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
        assert!(serde_json::from_value::<UserInputKind>(json!("")).is_err());
        assert!(serde_json::from_value::<UserInputKind>(serde_json::Value::Null).is_err());
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
    fn user_input_source_deserializes_canonical_values() {
        assert_eq!(
            serde_json::from_value::<UserInputSource>(json!("host")).unwrap(),
            UserInputSource::Host
        );
        assert_eq!(
            serde_json::from_value::<UserInputSource>(json!("plugin")).unwrap(),
            UserInputSource::Plugin
        );
        assert!(serde_json::from_value::<UserInputSource>(json!("weird")).is_err());
        assert!(serde_json::from_value::<UserInputSource>(json!("")).is_err());
        assert!(serde_json::from_value::<UserInputSource>(serde_json::Value::Null).is_err());
    }

    #[test]
    fn user_input_request_requires_kind_and_source() {
        let value = json!({
            "request_id": "r1",
            "created_at": "2026-01-01T00:00:00Z"
        });
        assert!(serde_json::from_value::<UserInputRequest>(value).is_err());
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
