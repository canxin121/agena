use crate::ProviderError;

use agena_provider::AuthData;

static AUTH_STORE_WORKERS: agena_async::BlockingPool = agena_async::BlockingPool::new(8);

pub(crate) async fn run_auth_store_operation<T: Send + 'static>(
    operation: impl FnOnce() -> Result<T, ProviderError> + Send + 'static,
) -> Result<T, ProviderError> {
    AUTH_STORE_WORKERS.run(operation).await.map_err(|error| {
        ProviderError::Config(format!("credential store worker failed: {error}"))
    })?
}

/// Persistence for provider authentication data.
pub trait AuthStore: Send + Sync {
    fn get(&self, provider_id: &str) -> Result<Option<AuthData>, ProviderError>;
    fn set(&self, provider_id: &str, auth: AuthData) -> Result<(), ProviderError>;
    fn remove(&self, provider_id: &str) -> Result<(), ProviderError>;
}
