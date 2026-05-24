use meilisearch_sdk::client::Client;
use meilisearch_sdk::errors::{Error as MeiliError, ErrorCode};
use serde::Serialize;
use serde::de::DeserializeOwned;
use thiserror::Error;

#[derive(Debug, Error)]
pub(crate) enum SearchBackendError {
    #[error("missing Meilisearch URL")]
    MissingUrl,
    #[error("invalid Meilisearch client config: {0}")]
    Client(String),
    #[error(transparent)]
    Backend(#[from] MeiliError),
}

#[derive(Debug, Clone)]
pub(crate) struct MeiliConnection {
    client: Client,
}

impl MeiliConnection {
    pub(crate) fn new(url: &str, api_key: Option<&str>) -> Result<Self, SearchBackendError> {
        let trimmed = url.trim();
        if trimmed.is_empty() {
            return Err(SearchBackendError::MissingUrl);
        }
        let client = Client::new(trimmed, api_key)
            .map_err(|err| SearchBackendError::Client(err.to_string()))?;
        Ok(Self { client })
    }

    pub(crate) async fn replace_documents<T>(
        &self,
        index_uid: &str,
        primary_key: Option<&str>,
        documents: &[T],
    ) -> Result<(), SearchBackendError>
    where
        T: Serialize + Send + Sync,
    {
        self.ensure_index(index_uid, primary_key).await?;
        let index = self.client.index(index_uid);
        index
            .delete_all_documents()
            .await?
            .wait_for_completion(&self.client, None, None)
            .await?;
        if documents.is_empty() {
            return Ok(());
        }
        index
            .add_documents(documents, primary_key)
            .await?
            .wait_for_completion(&self.client, None, None)
            .await?;
        Ok(())
    }

    pub(crate) async fn search<T>(
        &self,
        index_uid: &str,
        query: &str,
        limit: usize,
    ) -> Result<meilisearch_sdk::search::SearchResults<T>, SearchBackendError>
    where
        T: DeserializeOwned + Send + Sync + 'static,
    {
        let results = self
            .client
            .index(index_uid)
            .search()
            .with_query(query)
            .with_limit(limit)
            .execute::<T>()
            .await?;
        Ok(results)
    }

    fn client(&self) -> &Client {
        &self.client
    }

    async fn ensure_index(
        &self,
        index_uid: &str,
        primary_key: Option<&str>,
    ) -> Result<(), SearchBackendError> {
        match self.client.create_index(index_uid, primary_key).await {
            Ok(task) => {
                task.wait_for_completion(self.client(), None, None).await?;
                Ok(())
            }
            Err(MeiliError::Meilisearch(error))
                if error.error_code == ErrorCode::IndexAlreadyExists =>
            {
                Ok(())
            }
            Err(err) => Err(SearchBackendError::Backend(err)),
        }
    }
}
