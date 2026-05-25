use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};

use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};
use serde::{Deserialize, Serialize};

use crate::{CrawlError, CrawlSearchHit, StoredDocument, preview_text};

const VECTOR_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("crawl_chunk_vectors");

static OPEN_DATABASES: LazyLock<Mutex<HashMap<PathBuf, Arc<Database>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub struct CrawlVectorIndex {
    db: Arc<Database>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct VectorChunkRecord {
    doc_id: String,
    url: String,
    title: String,
    chunk_index: u32,
    preview: String,
    chunk_hash: String,
    embedding_model: Option<String>,
    embedding_dimension: u32,
    vector: Vec<f32>,
}

impl CrawlVectorIndex {
    pub fn open(path: &Path) -> Result<Self, CrawlError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut open = OPEN_DATABASES
            .lock()
            .map_err(|_| CrawlError::InvalidInput("vector database mutex poisoned".to_string()))?;
        let db = if let Some(existing) = open.get(path) {
            Arc::clone(existing)
        } else {
            let created = Arc::new(Database::create(path)?);
            open.insert(path.to_path_buf(), Arc::clone(&created));
            created
        };
        Ok(Self { db })
    }

    pub fn replace_document(&self, document: &StoredDocument) -> Result<usize, CrawlError> {
        let write_txn = self.db.begin_write()?;
        {
            let mut table = write_txn.open_table(VECTOR_TABLE)?;
            remove_document_rows(&mut table, document.id.as_str())?;

            let mut inserted = 0usize;
            for record in records_from_document(document) {
                let key = chunk_key(document.id.as_str(), record.chunk_index);
                let value = serde_json::to_vec(&record)?;
                table.insert(key.as_str(), value.as_slice())?;
                inserted += 1;
            }
            tracing::debug!(
                target: "agena::crawl",
                doc_id = %document.id,
                inserted,
                "updated crawl vector index"
            );
        }
        write_txn.commit()?;
        Ok(document.chunk_embeddings.len())
    }

    pub fn rebuild(&self, documents: &[StoredDocument]) -> Result<usize, CrawlError> {
        let write_txn = self.db.begin_write()?;
        let mut inserted = 0usize;
        {
            let mut table = write_txn.open_table(VECTOR_TABLE)?;
            let keys = table
                .iter()?
                .map(|entry| entry.map(|(key, _)| key.value().to_string()))
                .collect::<Result<Vec<_>, _>>()?;
            for key in keys {
                table.remove(key.as_str())?;
            }
            for document in documents {
                for record in records_from_document(document) {
                    let key = chunk_key(document.id.as_str(), record.chunk_index);
                    let value = serde_json::to_vec(&record)?;
                    table.insert(key.as_str(), value.as_slice())?;
                    inserted += 1;
                }
            }
        }
        write_txn.commit()?;
        Ok(inserted)
    }

    pub fn search(
        &self,
        query_vector: &[f32],
        embedding_model: Option<&str>,
        limit: usize,
    ) -> Result<Vec<CrawlSearchHit>, CrawlError> {
        if query_vector.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }

        let read_txn = self.db.begin_read()?;
        let table = match read_txn.open_table(VECTOR_TABLE) {
            Ok(table) => table,
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(Vec::new()),
            Err(err) => return Err(err.into()),
        };
        let mut hits = Vec::new();
        for entry in table.iter()? {
            let (_, value) = entry?;
            let record = serde_json::from_slice::<VectorChunkRecord>(value.value())?;
            if let Some(expected_model) = embedding_model
                && record.embedding_model.as_deref() != Some(expected_model)
            {
                continue;
            }
            if record.vector.len() != query_vector.len() {
                continue;
            }
            let score = cosine_similarity(query_vector, &record.vector);
            hits.push(CrawlSearchHit {
                id: record.doc_id,
                url: record.url,
                title: record.title,
                chunk_index: record.chunk_index,
                preview: record.preview,
                score,
                lexical_score: None,
                vector_score: Some(score),
                rerank_score: None,
                match_sources: vec!["vector".to_string()],
            });
        }

        hits.sort_by(|left, right| right.score.total_cmp(&left.score));
        hits.truncate(limit);
        Ok(hits)
    }
}

fn remove_document_rows(
    table: &mut redb::Table<&str, &[u8]>,
    document_id: &str,
) -> Result<(), CrawlError> {
    let prefix = format!("{document_id}:");
    let keys = table
        .iter()?
        .filter_map(|entry| match entry {
            Ok((key, _)) if key.value().starts_with(prefix.as_str()) => {
                Some(Ok(key.value().to_string()))
            }
            Ok(_) => None,
            Err(err) => Some(Err(err)),
        })
        .collect::<Result<Vec<_>, _>>()?;
    for key in keys {
        table.remove(key.as_str())?;
    }
    Ok(())
}

fn records_from_document(document: &StoredDocument) -> Vec<VectorChunkRecord> {
    document
        .chunk_embeddings
        .iter()
        .enumerate()
        .filter_map(|(chunk_index, vector)| {
            if vector.is_empty() || chunk_index >= document.chunks.len() {
                return None;
            }
            Some(VectorChunkRecord {
                doc_id: document.id.clone(),
                url: document.canonical_url.clone(),
                title: document.title.clone(),
                chunk_index: chunk_index as u32,
                preview: preview_text(document.chunks[chunk_index].as_str(), 320),
                chunk_hash: document
                    .chunk_hashes
                    .get(chunk_index)
                    .cloned()
                    .unwrap_or_default(),
                embedding_model: document.embedding_model.clone(),
                embedding_dimension: vector.len() as u32,
                vector: vector.clone(),
            })
        })
        .collect()
}

fn chunk_key(document_id: &str, chunk_index: u32) -> String {
    format!("{document_id}:{chunk_index:08}")
}

pub fn cosine_similarity(left: &[f32], right: &[f32]) -> f32 {
    if left.is_empty() || right.is_empty() || left.len() != right.len() {
        return 0.0;
    }
    let mut dot = 0.0f32;
    let mut left_norm = 0.0f32;
    let mut right_norm = 0.0f32;
    for (left, right) in left.iter().zip(right.iter()) {
        dot += left * right;
        left_norm += left * left;
        right_norm += right * right;
    }
    if left_norm == 0.0 || right_norm == 0.0 {
        return 0.0;
    }
    dot / (left_norm.sqrt() * right_norm.sqrt())
}
