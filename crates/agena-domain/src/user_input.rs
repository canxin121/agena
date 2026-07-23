use serde::{Deserialize, Serialize};

/// How an interactive user-input request was resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UserInputReplyKind {
    Submit,
    Cancel,
    Timeout,
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
