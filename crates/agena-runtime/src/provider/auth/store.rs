use crate::error::AppError;

use agena_provider::AuthData;

pub trait AuthStore: Send + Sync {
    fn get(&self, provider_id: &str) -> Result<Option<AuthData>, AppError>;
    fn set(&self, provider_id: &str, auth: AuthData) -> Result<(), AppError>;
    fn remove(&self, provider_id: &str) -> Result<(), AppError>;
}
