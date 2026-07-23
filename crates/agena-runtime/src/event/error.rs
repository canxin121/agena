use thiserror::Error;

#[derive(Debug, Error)]
pub enum BusError {
    #[error(transparent)]
    Store(#[from] agena_storage::EventStoreError),
}

#[derive(Debug, Error)]
pub enum PublishError {
    #[error(transparent)]
    Bus(#[from] BusError),
    #[error(transparent)]
    Store(#[from] agena_storage::EventStoreError),
}
