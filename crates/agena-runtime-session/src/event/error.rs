use thiserror::Error;

#[derive(Debug, Error)]
/// Error from the event bus.
pub enum BusError {
    #[error(transparent)]
    Store(#[from] agena_storage::EventStoreError),
}

#[derive(Debug, Error)]
/// Error publishing an event to the bus.
pub enum PublishError {
    #[error(transparent)]
    Bus(#[from] BusError),
    #[error(transparent)]
    Store(#[from] agena_storage::EventStoreError),
}
