use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tantivy::collector::TopDocs;
use tantivy::query::QueryParser;
use tantivy::schema::{
    Field, IndexRecordOption, STORED, STRING, Schema, TextFieldIndexing, TextOptions, Value,
};
use tantivy::tokenizer::{LowerCaser, NgramTokenizer, SimpleTokenizer, TextAnalyzer};
use tantivy::{DocAddress, Index, ReloadPolicy, TantivyDocument};
use thiserror::Error;

use crate::memory::MemoryDir;

const NGRAM_TOKENIZER: &str = "memory_ngram";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct MemorySearchDocument {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) memory_type: Option<String>,
    pub(crate) body: String,
    pub(crate) path: String,
    pub(crate) searchable_text: String,
    #[serde(skip_serializing, skip_deserializing)]
    searchable_ngrams: String,
}

impl MemorySearchDocument {
    pub(crate) fn new(
        id: String,
        name: String,
        description: String,
        memory_type: Option<String>,
        body: String,
        path: String,
    ) -> Self {
        let searchable_text = format!(
            "{} {} {} {}",
            name,
            description,
            memory_type.as_deref().unwrap_or(""),
            body
        );
        Self {
            id,
            name,
            description,
            memory_type,
            body,
            path,
            searchable_ngrams: searchable_text.clone(),
            searchable_text,
        }
    }
}

#[derive(Debug, Error)]
pub(crate) enum MemoryIndexError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("tantivy error: {0}")]
    Tantivy(#[from] tantivy::TantivyError),
}

#[derive(Clone)]
pub(crate) struct MemoryIndex {
    dir: PathBuf,
}

impl MemoryIndex {
    pub(crate) fn for_workspace(workspace_root: &Path) -> Self {
        let dir = MemoryDir::from_workspace(workspace_root).index_dir();
        Self { dir }
    }

    pub(crate) fn replace_documents(
        &self,
        documents: &[MemorySearchDocument],
    ) -> Result<(), MemoryIndexError> {
        if self.dir.exists() {
            fs::remove_dir_all(&self.dir)?;
        }
        fs::create_dir_all(&self.dir)?;
        let (schema, fields) = build_schema();
        let index = Index::create_in_dir(&self.dir, schema)?;
        register_tokenizers(&index)?;
        let mut writer = index.writer(15_000_000)?;
        for document in documents {
            let mut stored = TantivyDocument::new();
            stored.add_text(fields.id, document.id.clone());
            stored.add_text(fields.name, document.name.clone());
            stored.add_text(fields.description, document.description.clone());
            if let Some(memory_type) = document.memory_type.as_deref() {
                stored.add_text(fields.memory_type, memory_type);
            }
            stored.add_text(fields.body, document.body.clone());
            stored.add_text(fields.path, document.path.clone());
            stored.add_text(fields.searchable_text, document.searchable_text.clone());
            stored.add_text(fields.searchable_ngrams, document.searchable_ngrams.clone());
            writer.add_document(stored)?;
        }
        writer.commit()?;
        Ok(())
    }

    pub(crate) fn search(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<MemorySearchDocument>, MemoryIndexError> {
        let (_schema, fields) = build_schema();
        let index = Index::open_in_dir(&self.dir)?;
        register_tokenizers(&index)?;
        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::Manual)
            .try_into()?;
        let searcher = reader.searcher();
        let mut parser = QueryParser::for_index(
            &index,
            vec![
                fields.name,
                fields.description,
                fields.searchable_text,
                fields.searchable_ngrams,
            ],
        );
        parser.set_field_boost(fields.name, 3.0);
        parser.set_field_boost(fields.description, 2.0);
        parser.set_field_boost(fields.searchable_text, 1.0);
        parser.set_field_boost(fields.searchable_ngrams, 0.6);
        let (parsed_query, errors) = parser.parse_query_lenient(query);
        if !errors.is_empty() {
            tracing::debug!(
                target: "agena::memory",
                "memory query parsed leniently for '{query}': {:?}",
                errors
            );
        }
        let collector = TopDocs::with_limit(limit).order_by_score();
        let top_docs: Vec<(f32, DocAddress)> = searcher.search(&parsed_query, &collector)?;
        let mut results = Vec::with_capacity(top_docs.len());
        for (_score, address) in top_docs {
            let doc = searcher.doc::<TantivyDocument>(address)?;
            results.push(document_from_hit(&doc, &fields));
        }
        Ok(results)
    }
}

#[derive(Clone, Copy)]
struct MemoryIndexFields {
    id: Field,
    name: Field,
    description: Field,
    memory_type: Field,
    body: Field,
    path: Field,
    searchable_text: Field,
    searchable_ngrams: Field,
}

fn build_schema() -> (Schema, MemoryIndexFields) {
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
    let id = builder.add_text_field("id", STRING | STORED);
    let name = builder.add_text_field("name", indexed_text.clone());
    let description = builder.add_text_field("description", indexed_text.clone());
    let memory_type = builder.add_text_field("memory_type", indexed_text.clone());
    let body = builder.add_text_field("body", STORED);
    let path = builder.add_text_field("path", STRING | STORED);
    let searchable_text = builder.add_text_field("searchable_text", indexed_text);
    let searchable_ngrams = builder.add_text_field("searchable_ngrams", ngram_text);
    let schema = builder.build();
    (
        schema,
        MemoryIndexFields {
            id,
            name,
            description,
            memory_type,
            body,
            path,
            searchable_text,
            searchable_ngrams,
        },
    )
}

fn register_tokenizers(index: &Index) -> Result<(), MemoryIndexError> {
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

fn document_from_hit(doc: &TantivyDocument, fields: &MemoryIndexFields) -> MemorySearchDocument {
    MemorySearchDocument {
        id: first_text(doc, fields.id),
        name: first_text(doc, fields.name),
        description: first_text(doc, fields.description),
        memory_type: optional_text(doc, fields.memory_type),
        body: first_text(doc, fields.body),
        path: first_text(doc, fields.path),
        searchable_text: first_text(doc, fields.searchable_text),
        searchable_ngrams: String::new(),
    }
}

fn first_text(doc: &TantivyDocument, field: Field) -> String {
    optional_text(doc, field).unwrap_or_default()
}

fn optional_text(doc: &TantivyDocument, field: Field) -> Option<String> {
    doc.get_first(field)
        .and_then(|value| value.as_str())
        .map(ToOwned::to_owned)
}
