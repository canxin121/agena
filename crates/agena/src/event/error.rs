use thiserror::Error;

#[derive(Debug, Error)]
pub enum EventStoreError {
    #[error("event store backend error: {0}")]
    Backend(#[from] sea_orm::DbErr),
    #[error("event with seq_global={0} already exists")]
    DuplicateSeq(i64),
    #[error("invalid range: {0}")]
    InvalidRange(String),
    #[error(transparent)]
    Serde(#[from] serde_json::Error),
}

#[derive(Debug, Error)]
pub enum BusError {
    #[error("bus shut down")]
    Closed,
    #[error(transparent)]
    Store(#[from] EventStoreError),
}

#[derive(Debug, Error)]
pub enum PublishError {
    #[error(transparent)]
    Bus(#[from] BusError),
    #[error(transparent)]
    Store(#[from] EventStoreError),
}
