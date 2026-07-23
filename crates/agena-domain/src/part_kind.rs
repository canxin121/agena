use serde::{Deserialize, Serialize};
use strum::{AsRefStr, Display, EnumString};

/// Semantic category of a message part.
///
/// Storage and presentation layers may choose their own representation, but
/// the category itself is part of the stable message model.
#[derive(
    Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, AsRefStr, Display, EnumString,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum PartKind {
    Text,
    Reasoning,
    Operation,
    Attachment,
    Request,
    Error,
}

#[cfg(test)]
mod tests {
    use super::PartKind;

    #[test]
    fn uses_stable_wire_names() {
        assert_eq!(
            serde_json::to_string(&PartKind::Operation).expect("serialize part kind"),
            "\"operation\""
        );
    }
}
