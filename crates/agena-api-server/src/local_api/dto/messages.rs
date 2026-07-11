pub use agena_api::resource::{MessageResource, PartLoadMode};

#[derive(Debug, Clone, Deserialize, Default)]
pub struct MessageListQuery {
    #[serde(flatten)]
    pub pagination: CursorPaginationQuery,
    #[serde(default)]
    pub parts: PartLoadMode,
}
use super::{CursorPaginationQuery, Deserialize};
