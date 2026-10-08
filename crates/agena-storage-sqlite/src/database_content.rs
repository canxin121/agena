//! Bind content files to the database identity, independent of workspaces.

use crate::{FileContentBackend, FileContentConfig};
use agena_domain::{ContentChunk, ContentCursor, ContentId, ContentPage, ContentResource};
use agena_storage::{
    content::{ContentArchive, ContentBackend, MemoryContentBackend},
    store::StoreError,
};
use async_trait::async_trait;
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::OnceCell;

pub struct DatabaseContentBackend {
    database: Arc<DatabaseConnection>,
    backend: OnceCell<Arc<dyn ContentBackend>>,
}

impl DatabaseContentBackend {
    pub fn new(database: Arc<DatabaseConnection>) -> Self {
        Self {
            database,
            backend: OnceCell::new(),
        }
    }

    async fn backend(&self) -> Result<&Arc<dyn ContentBackend>, StoreError> {
        self.backend
            .get_or_try_init(|| async {
                let rows = self
                    .database
                    .query_all(Statement::from_string(
                        DbBackend::Sqlite,
                        "PRAGMA database_list".to_owned(),
                    ))
                    .await
                    .map_err(|error| StoreError::Io(error.to_string()))?;
                for row in rows {
                    let name: String = row
                        .try_get("", "name")
                        .map_err(|error| StoreError::Io(error.to_string()))?;
                    if name != "main" {
                        continue;
                    }
                    let file: String = row
                        .try_get("", "file")
                        .map_err(|error| StoreError::Io(error.to_string()))?;
                    if !file.is_empty() {
                        let path = PathBuf::from(file);
                        // Appending instead of replacing the extension isolates
                        // sibling databases with the same file stem.
                        let mut root = path.into_os_string();
                        root.push(".content");
                        return Ok(Arc::new(FileContentBackend::new(
                            PathBuf::from(root),
                            FileContentConfig::default(),
                        )) as Arc<dyn ContentBackend>);
                    }
                }
                Ok(Arc::new(MemoryContentBackend::default()) as Arc<dyn ContentBackend>)
            })
            .await
    }
}

#[async_trait]
impl ContentBackend for DatabaseContentBackend {
    async fn restore(&self, archive: &ContentArchive) -> Result<(), StoreError> {
        self.backend().await?.restore(archive).await
    }
    async fn prune(
        &self,
        protected: &std::collections::HashSet<ContentId>,
    ) -> Result<usize, StoreError> {
        self.backend().await?.prune(protected).await
    }
    async fn delete(&self, id: ContentId) -> Result<(), StoreError> {
        self.backend().await?.delete(id).await
    }
    async fn commit(
        &self,
        resource: ContentResource,
        chunks: &[Arc<ContentChunk>],
    ) -> Result<ContentResource, StoreError> {
        self.backend().await?.commit(resource, chunks).await
    }
    async fn read(
        &self,
        id: ContentId,
        after: Option<ContentCursor>,
        max_bytes: usize,
    ) -> Result<ContentPage, StoreError> {
        self.backend().await?.read(id, after, max_bytes).await
    }
    async fn describe(&self, id: ContentId) -> Result<ContentResource, StoreError> {
        self.backend().await?.describe(id).await
    }
}
