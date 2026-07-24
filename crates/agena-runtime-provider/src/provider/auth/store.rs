use crate::ProviderError;

use agena_provider::AuthData;

pub trait AuthStore: Send + Sync {
    fn get(&self, provider_id: &str) -> Result<Option<AuthData>, ProviderError>;
    fn set(&self, provider_id: &str, auth: AuthData) -> Result<(), ProviderError>;
    fn remove(&self, provider_id: &str) -> Result<(), ProviderError>;
}
