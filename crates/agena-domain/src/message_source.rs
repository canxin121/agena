use serde::{Deserialize, Serialize};
use strum::{AsRefStr, Display, EnumIter, EnumString};

/// Origin of a persisted conversation message.
#[derive(
    Debug,
    Clone,
    Copy,
    Serialize,
    Deserialize,
    PartialEq,
    Eq,
    Hash,
    AsRefStr,
    Display,
    EnumString,
    EnumIter,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum MessageSource {
    User,
    Assistant,
    System,
}

#[cfg(test)]
mod tests {
    use super::MessageSource;

    #[test]
    fn source_serialization_is_stable() {
        assert_eq!(
            serde_json::to_string(&MessageSource::Assistant).unwrap(),
            "\"assistant\""
        );
        assert_eq!(
            "user".parse::<MessageSource>().unwrap(),
            MessageSource::User
        );
    }
}
