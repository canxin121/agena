use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use fastembed::{
    EmbeddingModel, InitOptions, RerankInitOptions, RerankerModel, TextEmbedding, TextRerank,
};
use serde::{Deserialize, Serialize};

use crate::{
    CrawlError, CrawlSearchHit, CrawlStore, StoredDocument, preview_text,
    vector_index::cosine_similarity,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingServiceConfig {
    pub embedding_model: String,
    pub reranker_model: String,
    pub cache_dir: PathBuf,
    pub enable_rerank: bool,
}

#[derive(Debug, Clone)]
pub struct HybridSearchOptions {
    pub lexical_limit: usize,
    pub vector_limit: usize,
    pub min_vector_query_chars: usize,
    pub rrf_k: usize,
    pub rerank_limit: usize,
}

pub struct EmbeddingService {
    embedding_model_name: String,
    text_model: Mutex<TextEmbedding>,
    rerank_model: Option<Mutex<TextRerank>>,
}

#[derive(Clone)]
struct VectorCandidate {
    hit: CrawlSearchHit,
    chunk_text: String,
}

impl EmbeddingService {
    pub fn new(config: &EmbeddingServiceConfig) -> Result<Self, CrawlError> {
        let embedding_model = config
            .embedding_model
            .parse::<EmbeddingModel>()
            .map_err(CrawlError::InvalidInput)?;
        let text_model = TextEmbedding::try_new(
            InitOptions::new(embedding_model)
                .with_cache_dir(config.cache_dir.clone())
                .with_show_download_progress(false),
        )?;

        let rerank_model = if config.enable_rerank {
            let reranker_model = config
                .reranker_model
                .parse::<RerankerModel>()
                .map_err(CrawlError::InvalidInput)?;
            Some(Mutex::new(TextRerank::try_new(
                RerankInitOptions::new(reranker_model)
                    .with_cache_dir(config.cache_dir.clone())
                    .with_show_download_progress(false),
            )?))
        } else {
            None
        };

        Ok(Self {
            embedding_model_name: config.embedding_model.clone(),
            text_model: Mutex::new(text_model),
            rerank_model,
        })
    }

    pub fn embed_document(&self, document: &mut StoredDocument) -> Result<(), CrawlError> {
        if document.chunks.is_empty() {
            document.chunks = vec![document.markdown.clone()];
        }
        let inputs = document
            .chunks
            .iter()
            .map(|chunk| format!("passage: {chunk}"))
            .collect::<Vec<_>>();
        let mut model = self
            .text_model
            .lock()
            .map_err(|_| CrawlError::InvalidInput("embedding model mutex poisoned".to_string()))?;
        let vectors = model.embed(inputs, None)?;
        document.embedding_model = Some(self.embedding_model_name.clone());
        document.embedding_dimension = vectors.first().map(|vector| vector.len() as u32);
        document.chunk_embeddings = vectors;
        Ok(())
    }

    pub fn embedding_model_name(&self) -> &str {
        self.embedding_model_name.as_str()
    }

    pub fn embed_query(&self, query: &str) -> Result<Vec<f32>, CrawlError> {
        let mut model = self
            .text_model
            .lock()
            .map_err(|_| CrawlError::InvalidInput("embedding model mutex poisoned".to_string()))?;
        Ok(model
            .embed(vec![format!("query: {query}")], None)?
            .into_iter()
            .next()
            .unwrap_or_default())
    }

    pub fn vector_search(
        &self,
        query: &str,
        documents: &[StoredDocument],
        limit: usize,
    ) -> Result<Vec<CrawlSearchHit>, CrawlError> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let query_vector = self.embed_query(query)?;
        self.vector_search_with_query(&query_vector, documents, limit)
    }

    pub fn vector_search_with_query(
        &self,
        query_vector: &[f32],
        documents: &[StoredDocument],
        limit: usize,
    ) -> Result<Vec<CrawlSearchHit>, CrawlError> {
        let mut hits = Vec::new();
        for document in documents {
            let mut best: Option<(usize, f32)> = None;
            for (idx, vector) in document.chunk_embeddings.iter().enumerate() {
                if vector.is_empty() || idx >= document.chunks.len() {
                    continue;
                }
                let score = cosine_similarity(&query_vector, vector);
                if best.is_none_or(|(_, current)| score > current) {
                    best = Some((idx, score));
                }
            }
            let Some((chunk_index, score)) = best else {
                continue;
            };
            let preview = preview_text(document.chunks[chunk_index].as_str(), 320);
            hits.push(CrawlSearchHit {
                id: document.id.clone(),
                url: document.canonical_url.clone(),
                title: document.title.clone(),
                chunk_index: chunk_index as u32,
                preview,
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

    pub fn rerank(
        &self,
        query: &str,
        documents: &[String],
        limit: usize,
    ) -> Result<Vec<(usize, f32)>, CrawlError> {
        let Some(model) = &self.rerank_model else {
            return Ok(Vec::new());
        };
        let docs = documents.iter().map(String::as_str).collect::<Vec<_>>();
        let mut model = model
            .lock()
            .map_err(|_| CrawlError::InvalidInput("rerank model mutex poisoned".to_string()))?;
        let results = model.rerank(query, &docs, false, None)?;
        Ok(results
            .into_iter()
            .take(limit)
            .map(|result| (result.index, result.score))
            .collect())
    }
}

pub fn hybrid_search_documents(
    store: &CrawlStore,
    query: &str,
    limit: usize,
    options: &HybridSearchOptions,
    embedding_service: Option<&EmbeddingService>,
) -> Result<Vec<CrawlSearchHit>, CrawlError> {
    let lexical_hits = store.search(query, options.lexical_limit)?;
    let vector_hits = if let Some(service) = embedding_service {
        if query.chars().count() >= options.min_vector_query_chars {
            let query_vector = service.embed_query(query)?;
            let hits = store.vector_search(
                &query_vector,
                Some(service.embedding_model_name()),
                options.vector_limit,
            )?;
            if hits.is_empty() {
                let documents = store.list_documents()?;
                service.vector_search_with_query(&query_vector, &documents, options.vector_limit)?
            } else {
                hits
            }
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };

    if vector_hits.is_empty() {
        return Ok(lexical_hits.into_iter().take(limit).collect());
    }

    let documents = store
        .list_documents()?
        .into_iter()
        .map(|document| (document.id.clone(), document))
        .collect::<HashMap<_, _>>();
    let mut merged = HashMap::<String, VectorCandidate>::new();
    let rrf_k = options.rrf_k.max(1) as f32;

    for (rank, hit) in lexical_hits.into_iter().enumerate() {
        let score = 1.0 / (rrf_k + rank as f32 + 1.0);
        upsert_candidate(
            &mut merged,
            hit,
            score,
            Some(score),
            None,
            &documents,
            "lexical",
        );
    }
    for (rank, hit) in vector_hits.into_iter().enumerate() {
        let score = 1.0 / (rrf_k + rank as f32 + 1.0);
        upsert_candidate(
            &mut merged,
            hit,
            score,
            None,
            Some(score),
            &documents,
            "vector",
        );
    }

    let mut candidates = merged.into_values().collect::<Vec<_>>();
    candidates.sort_by(|left, right| right.hit.score.total_cmp(&left.hit.score));
    if let Some(service) = embedding_service
        && options.rerank_limit > 0
        && !candidates.is_empty()
    {
        let rerank_docs = candidates
            .iter()
            .take(options.rerank_limit.min(candidates.len()))
            .map(|candidate| candidate.chunk_text.clone())
            .collect::<Vec<_>>();
        let reranked = service.rerank(query, &rerank_docs, rerank_docs.len())?;
        let mut rerank_scores = HashMap::new();
        for (position, score) in reranked {
            rerank_scores.insert(position, score);
        }
        for (index, candidate) in candidates.iter_mut().enumerate() {
            let Some(score) = rerank_scores.get(&index).copied() else {
                continue;
            };
            candidate.hit.rerank_score = Some(score);
            candidate.hit.score += score;
            push_match_source(&mut candidate.hit, "rerank");
        }
        candidates.sort_by(|left, right| right.hit.score.total_cmp(&left.hit.score));
    }

    Ok(candidates
        .into_iter()
        .map(|candidate| candidate.hit)
        .take(limit)
        .collect())
}

fn upsert_candidate(
    merged: &mut HashMap<String, VectorCandidate>,
    hit: CrawlSearchHit,
    delta: f32,
    lexical_score: Option<f32>,
    vector_score: Option<f32>,
    documents: &HashMap<String, StoredDocument>,
    source: &str,
) {
    let chunk_text = documents
        .get(hit.id.as_str())
        .and_then(|document| document.chunks.get(hit.chunk_index as usize))
        .cloned()
        .unwrap_or_else(|| hit.preview.clone());
    merged
        .entry(hit.id.clone())
        .and_modify(|candidate| {
            candidate.hit.score += delta;
            if lexical_score.is_some() {
                candidate.hit.lexical_score = lexical_score;
            }
            if vector_score.is_some() {
                candidate.hit.vector_score = vector_score;
            }
            push_match_source(&mut candidate.hit, source);
        })
        .or_insert_with(|| {
            let mut hit = hit;
            hit.score = delta;
            hit.lexical_score = lexical_score;
            hit.vector_score = vector_score;
            hit.rerank_score = None;
            hit.match_sources = vec![source.to_string()];
            VectorCandidate { hit, chunk_text }
        });
}

fn push_match_source(hit: &mut CrawlSearchHit, source: &str) {
    if !hit.match_sources.iter().any(|value| value == source) {
        hit.match_sources.push(source.to_string());
    }
}
