use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("database error: {0}")]
    Database(#[from] sea_orm::DbErr),
    #[error("serde json error: {0}")]
    SerdeJson(#[from] serde_json::Error),
    #[error("invalid role value in storage: {0}")]
    InvalidRole(String),
    #[error("internal error: {0}")]
    Internal(String),
}
