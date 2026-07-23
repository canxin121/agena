use agena_provider::{ModelCatalogDocument, ModelCatalogSnapshot, ModelCatalogSnapshotSourceKind};
use agena_storage::ModelCatalogCacheRecord;
use chrono::{DateTime, Utc};

/// Error while translating the storage port's opaque cache record into the
/// runtime/provider catalog snapshot values.
#[derive(Debug, thiserror::Error)]
pub enum ModelCatalogCacheCodecError {
    #[error("invalid model catalog cache source: {0}")]
    Source(String),
    #[error(transparent)]
    Serde(#[from] serde_json::Error),
}

/// Decodes a storage-port cache record without depending on a core error type.
pub fn model_catalog_snapshot_from_cache_record(
    record: &ModelCatalogCacheRecord,
) -> Result<ModelCatalogSnapshot, ModelCatalogCacheCodecError> {
    let source = ModelCatalogSnapshotSourceKind::from_persisted(record.source.as_str())
        .map_err(ModelCatalogCacheCodecError::Source)?;
    let official = serde_json::from_value(record.document.clone())?;
    Ok(ModelCatalogSnapshot {
        last_refresh_at: DateTime::<Utc>::from_timestamp_millis(record.fetched_at_unix_ms),
        last_successful_source: Some(source),
        last_error: None,
        official,
    })
}

/// Encodes the generated catalog document for the storage repository port.
pub fn model_catalog_cache_record_from_document(
    fetched_at_unix_ms: i64,
    source: ModelCatalogSnapshotSourceKind,
    document: &ModelCatalogDocument,
) -> Result<ModelCatalogCacheRecord, ModelCatalogCacheCodecError> {
    Ok(ModelCatalogCacheRecord {
        fetched_at_unix_ms,
        source: source.as_persisted().to_owned(),
        document: serde_json::to_value(document)?,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        model_catalog_cache_record_from_document, model_catalog_snapshot_from_cache_record,
    };
    use agena_provider::{ModelCatalogDocument, ModelCatalogSnapshotSourceKind};
    use std::collections::BTreeMap;

    #[test]
    fn cache_codec_round_trips_provider_catalog_values() {
        let document = ModelCatalogDocument {
            models: BTreeMap::new(),
        };
        let record = model_catalog_cache_record_from_document(
            123,
            ModelCatalogSnapshotSourceKind::Cache,
            &document,
        )
        .expect("encode record");
        let snapshot = model_catalog_snapshot_from_cache_record(&record).expect("decode record");
        assert_eq!(snapshot.last_refresh_at.unwrap().timestamp_millis(), 123);
        assert_eq!(
            snapshot.last_successful_source,
            Some(ModelCatalogSnapshotSourceKind::Cache)
        );
        assert_eq!(snapshot.official, document);
    }
}
