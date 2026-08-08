use thiserror::Error;

#[derive(Debug, Error)]
/// Error from the web crawl subsystem.
pub enum CrawlError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("url error: {0}")]
    Url(#[from] url::ParseError),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("tantivy error: {0}")]
    Tantivy(#[from] tantivy::TantivyError),
    #[error("metadata error: {0}")]
    Redb(#[from] redb::Error),
    #[error("storage error: {0}")]
    RedbStorage(#[from] redb::StorageError),
    #[error("transaction error: {0}")]
    RedbTransaction(#[from] redb::TransactionError),
    #[error("table error: {0}")]
    RedbTable(#[from] redb::TableError),
    #[error("database error: {0}")]
    RedbDatabase(#[from] redb::DatabaseError),
    #[error("commit error: {0}")]
    RedbCommit(#[from] redb::CommitError),
    #[error("{0}")]
    InvalidInput(String),
    #[error("{0} not found")]
    NotFound(String),
}
