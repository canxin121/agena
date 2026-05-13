use std::collections::HashMap;

use crate::error::AppError;

use super::AuthData;

pub trait AuthStore: Send + Sync {
    fn all(&self) -> Result<HashMap<String, AuthData>, AppError>;
    fn get(&self, provider_id: &str) -> Result<Option<AuthData>, AppError>;
    fn set(&self, provider_id: &str, auth: AuthData) -> Result<(), AppError>;
    fn remove(&self, provider_id: &str) -> Result<(), AppError>;
}
