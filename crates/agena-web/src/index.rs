use std::collections::HashSet;
use std::fs;
use std::path::Path;

use tantivy::collector::TopDocs;
use tantivy::query::QueryParser;
use tantivy::schema::{
    Field, IndexRecordOption, STORED, STRING, Schema, TextFieldIndexing, TextOptions, Value,
};
use tantivy::tokenizer::{LowerCaser, NgramTokenizer, SimpleTokenizer, TextAnalyzer};
use tantivy::{DocAddress, Index, ReloadPolicy, TantivyDocument};

use crate::{CrawlError, CrawlSearchHit, StoredDocument, preview_text};

const NGRAM_TOKENIZER: &str = "crawl_ngram";

#[derive(Clone, Copy)]
struct SearchFields {
    doc_id: Field,
    url: Field,
    title: Field,
    chunk_index: Field,
    preview: Field,
    searchable_text: Field,
    searchable_ngrams: Field,
}

pub fn rebuild_search_index(
    index_dir: &Path,
    documents: &[StoredDocument],
) -> Result<(), CrawlError> {
    if index_dir.exists() {
        fs::remove_dir_all(index_dir)?;
    }
    fs::create_dir_all(index_dir)?;
    let (schema, fields) = build_schema();
    let index = Index::create_in_dir(index_dir, schema)?;
    register_tokenizers(&index)?;
    let mut writer = index.writer(20_000_000)?;

    for document in documents {
        let chunks = if document.chunks.is_empty() {
            vec![document.markdown.clone()]
        } else {
            document.chunks.clone()
        };
        for (chunk_index, chunk) in chunks.into_iter().enumerate() {
            let mut stored = TantivyDocument::default();
            stored.add_text(fields.doc_id, document.id.clone());
            stored.add_text(fields.url, document.canonical_url.clone());
            stored.add_text(fields.title, document.title.clone());
            stored.add_u64(fields.chunk_index, chunk_index as u64);
            stored.add_text(fields.preview, preview_text(chunk.as_str(), 320));
            let searchable_text =
                format!("{} {} {}", document.title, document.canonical_url, chunk);
            stored.add_text(fields.searchable_text, searchable_text.clone());
            stored.add_text(fields.searchable_ngrams, searchable_text);
            writer.add_document(stored)?;
        }
    }

    writer.commit()?;
    Ok(())
}

pub fn search_documents(
    index_dir: &Path,
    query: &str,
    limit: usize,
) -> Result<Vec<CrawlSearchHit>, CrawlError> {
    if !index_dir.exists() || limit == 0 {
        return Ok(Vec::new());
    }

    let (_schema, fields) = build_schema();
    let index = Index::open_in_dir(index_dir)?;
    register_tokenizers(&index)?;
    let reader = index
        .reader_builder()
        .reload_policy(ReloadPolicy::Manual)
        .try_into()?;
    let searcher = reader.searcher();
    let mut parser = QueryParser::for_index(
        &index,
        vec![
            fields.title,
            fields.searchable_text,
            fields.searchable_ngrams,
        ],
    );
    parser.set_field_boost(fields.title, 3.0);
    parser.set_field_boost(fields.searchable_text, 1.0);
    parser.set_field_boost(fields.searchable_ngrams, 0.6);
    let (parsed_query, errors) = parser.parse_query_lenient(query);
    if !errors.is_empty() {
        tracing::debug!(
            target: "agena::web",
            "crawl query parsed leniently for '{query}': {:?}",
            errors
        );
    }

    let collector = TopDocs::with_limit(limit.saturating_mul(4).max(limit)).order_by_score();
    let top_docs: Vec<(f32, DocAddress)> = searcher.search(&parsed_query, &collector)?;
    let mut seen = HashSet::new();
    let mut hits = Vec::new();

    for (score, address) in top_docs {
        let doc = searcher.doc::<TantivyDocument>(address)?;
        let hit = document_from_hit(&doc, &fields, score);
        if seen.insert(hit.id.clone()) {
            hits.push(hit);
        }
        if hits.len() >= limit {
            break;
        }
    }

    Ok(hits)
}

fn build_schema() -> (Schema, SearchFields) {
    let mut builder = Schema::builder();
    let indexed_text = TextOptions::default().set_stored().set_indexing_options(
        TextFieldIndexing::default()
            .set_tokenizer("default")
            .set_index_option(IndexRecordOption::WithFreqsAndPositions),
    );
    let ngram_text = TextOptions::default().set_indexing_options(
        TextFieldIndexing::default()
            .set_tokenizer(NGRAM_TOKENIZER)
            .set_index_option(IndexRecordOption::WithFreqsAndPositions),
    );
    let doc_id = builder.add_text_field("doc_id", STRING | STORED);
    let url = builder.add_text_field("url", STRING | STORED);
    let title = builder.add_text_field("title", indexed_text.clone());
    let chunk_index = builder.add_u64_field("chunk_index", STORED);
    let preview = builder.add_text_field("preview", STORED);
    let searchable_text = builder.add_text_field("searchable_text", indexed_text);
    let searchable_ngrams = builder.add_text_field("searchable_ngrams", ngram_text);
    let schema = builder.build();
    (
        schema,
        SearchFields {
            doc_id,
            url,
            title,
            chunk_index,
            preview,
            searchable_text,
            searchable_ngrams,
        },
    )
}

fn register_tokenizers(index: &Index) -> Result<(), CrawlError> {
    let ngrams = TextAnalyzer::builder(
        NgramTokenizer::new(2, 4, false)
            .map_err(|err| tantivy::TantivyError::InvalidArgument(err.to_string()))?,
    )
    .filter(LowerCaser)
    .build();
    let simple = TextAnalyzer::builder(SimpleTokenizer::default())
        .filter(LowerCaser)
        .build();
    index.tokenizers().register("default", simple);
    index.tokenizers().register(NGRAM_TOKENIZER, ngrams);
    Ok(())
}

fn document_from_hit(doc: &TantivyDocument, fields: &SearchFields, score: f32) -> CrawlSearchHit {
    CrawlSearchHit {
        id: first_text(doc, fields.doc_id),
        url: first_text(doc, fields.url),
        title: first_text(doc, fields.title),
        chunk_index: doc
            .get_first(fields.chunk_index)
            .and_then(|value| value.as_u64())
            .unwrap_or_default() as u32,
        preview: first_text(doc, fields.preview),
        score,
        lexical_score: Some(score),
        match_sources: vec!["lexical".to_string()],
    }
}

fn first_text(doc: &TantivyDocument, field: Field) -> String {
    doc.get_first(field)
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .to_string()
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use crate::{StoredDocument, rebuild_search_index, search_documents};

    #[test]
    fn search_supports_cjk_queries() {
        let temp = tempdir().expect("tempdir");
        let doc = StoredDocument {
            id: "doc-1".to_string(),
            url: "https://example.com".to_string(),
            canonical_url: "https://example.com/".to_string(),
            title: "发布说明".to_string(),
            markdown: "这是 agena crawl 的中文文档。".to_string(),
            chunks: vec!["这是 agena crawl 的中文文档。".to_string()],
            links: Vec::new(),
            content_type: "text/html".to_string(),
            status: 200,
            truncated: false,
            rendered: false,
            raw_html_hash: "raw-hash".to_string(),
            markdown_hash: "hash".to_string(),
            simhash: 1,
            etag: None,
            last_modified: None,
            chunk_hashes: vec!["chunk-hash".to_string()],
            hash: "hash".to_string(),
            depth: 0,
            fetched_at: chrono::Utc::now(),
        };
        rebuild_search_index(temp.path(), &[doc]).expect("index builds");
        let hits = search_documents(temp.path(), "中文文档", 5).expect("search succeeds");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title, "发布说明");
    }
}
