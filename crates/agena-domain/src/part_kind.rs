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
/// Kind of a message part.
pub enum PartKind {
    Text,
    Activity,
}

#[cfg(test)]
mod tests {
    use super::PartKind;

    #[test]
    fn uses_stable_wire_names() {
        assert_eq!(
            serde_json::to_string(&PartKind::Activity).expect("serialize activity kind"),
            "\"activity\""
        );
    }
}
