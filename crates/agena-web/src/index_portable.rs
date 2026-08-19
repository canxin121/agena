use std::collections::HashMap;
use std::fs;
use std::path::Path;

use crate::{CrawlError, CrawlSearchHit, StoredDocument, preview_text};

const INDEX_FILE: &str = "portable-index.json";

pub fn rebuild_search_index(
    index_dir: &Path,
    documents: &[StoredDocument],
) -> Result<(), CrawlError> {
    if index_dir.exists() {
        fs::remove_dir_all(index_dir)?;
    }
    fs::create_dir_all(index_dir)?;
    fs::write(index_dir.join(INDEX_FILE), serde_json::to_vec(documents)?)?;
    Ok(())
}

pub fn search_documents(
    index_dir: &Path,
    query: &str,
    limit: usize,
) -> Result<Vec<CrawlSearchHit>, CrawlError> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    let path = index_dir.join(INDEX_FILE);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let documents: Vec<StoredDocument> = serde_json::from_slice(&fs::read(path)?)?;
    let terms = query_terms(query);
    if terms.is_empty() {
        return Ok(Vec::new());
    }

    let mut best_by_document = HashMap::<String, CrawlSearchHit>::new();
    for document in documents {
        let chunks = if document.chunks.is_empty() {
            vec![document.markdown.as_str()]
        } else {
            document.chunks.iter().map(String::as_str).collect()
        };
        for (chunk_index, chunk) in chunks.into_iter().enumerate() {
            let score = chunk_score(&document, chunk, &terms);
            if score <= 0.0 {
                continue;
            }
            let hit = CrawlSearchHit {
                id: document.id.clone(),
                url: document.canonical_url.clone(),
                title: document.title.clone(),
                chunk_index: chunk_index as u32,
                preview: preview_text(chunk, 320),
                score,
                lexical_score: Some(score),
                match_sources: vec!["portable-lexical".to_string()],
            };
            best_by_document
                .entry(document.id.clone())
                .and_modify(|existing| {
                    if hit.score > existing.score {
                        *existing = hit.clone();
                    }
                })
                .or_insert(hit);
        }
    }

    let mut hits = best_by_document.into_values().collect::<Vec<_>>();
    hits.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| left.title.cmp(&right.title))
            .then_with(|| left.id.cmp(&right.id))
    });
    hits.truncate(limit);
    Ok(hits)
}

fn query_terms(query: &str) -> Vec<String> {
    query
        .split(|character: char| !character.is_alphanumeric())
        .filter(|term| !term.is_empty())
        .map(str::to_lowercase)
        .collect()
}

fn chunk_score(document: &StoredDocument, chunk: &str, terms: &[String]) -> f32 {
    let title = document.title.to_lowercase();
    let url = document.canonical_url.to_lowercase();
    let chunk = chunk.to_lowercase();
    terms
        .iter()
        .map(|term| {
            f32::from(title.contains(term)) * 5.0
                + f32::from(url.contains(term)) * 1.5
                + f32::from(chunk.contains(term)) * 2.0
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::{chunk_score, query_terms};
    use crate::StoredDocument;

    #[test]
    fn portable_index_scores_matching_chunks() {
        let document = StoredDocument {
            id: "1".into(),
            url: "https://example.test".into(),
            canonical_url: "https://example.test".into(),
            title: "Release matrix".into(),
            markdown: "Build every architecture".into(),
            chunks: Vec::new(),
            chunk_hashes: Vec::new(),
            links: Vec::new(),
            content_type: "text/markdown".into(),
            status: 200,
            truncated: false,
            rendered: false,
            hash: String::new(),
            raw_html_hash: String::new(),
            markdown_hash: String::new(),
            simhash: 0,
            etag: None,
            last_modified: None,
            depth: 0,
            fetched_at: Utc::now(),
        };
        assert!(chunk_score(&document, &document.markdown, &query_terms("architecture")) > 0.0);
        assert_eq!(
            chunk_score(&document, &document.markdown, &query_terms("absent")),
            0.0
        );
    }
}
