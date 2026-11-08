use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use agena_storage::MemoryDir;
use serde::{Deserialize, Serialize};
use tantivy::{
    DocAddress, Index, ReloadPolicy, TantivyDocument, Term,
    collector::TopDocs,
    directory::MmapDirectory,
    query::{BooleanQuery, BoostQuery, Occur, PhraseQuery, Query, TermQuery},
    schema::{
        Field, IndexRecordOption, STORED, STRING, Schema, TextFieldIndexing, TextOptions, Value,
    },
    tokenizer::{
        LowerCaser, NgramTokenizer, RemoveLongFilter, TextAnalyzer, Token, TokenStream, Tokenizer,
    },
};

use super::{
    MAX_SEARCH_RESULTS, MemoryIndexError, MemorySearchDocument, MemorySearchHit, validate_documents,
};
use crate::retrieval::{LiteralQuery, MAX_WORD_BYTES, WordSpans, cjk_text, lowercase, sort_hits};

const NGRAM_TOKENIZER: &str = "memory_ngram";
// Version 3 adds searchable path tokens and a case-folded exact path term.
// Changing the token policy rebuilds old documents once without changing schema.
const INDEX_VERSION: u32 = 3;

#[derive(Debug, Serialize, Deserialize)]
struct IndexManifest {
    version: u32,
    documents: BTreeMap<String, String>,
}

impl IndexManifest {
    fn from_documents(documents: &[MemorySearchDocument]) -> Result<Self, MemoryIndexError> {
        validate_documents(documents)?;
        let documents = documents
            .iter()
            .map(|document| {
                Ok((
                    document.id.clone(),
                    blake3::hash(&serde_json::to_vec(document)?)
                        .to_hex()
                        .to_string(),
                ))
            })
            .collect::<Result<_, MemoryIndexError>>()?;
        Ok(Self {
            version: INDEX_VERSION,
            documents,
        })
    }

    fn parse(payload: Option<&str>) -> Option<Self> {
        let manifest: Self = serde_json::from_str(payload?).ok()?;
        (manifest.version == INDEX_VERSION).then_some(manifest)
    }
}

#[derive(Clone)]
/// Full-text index over memory documents.
pub struct MemoryIndex {
    dir: PathBuf,
}

impl MemoryIndex {
    pub fn for_workspace(workspace_root: &Path) -> Self {
        Self {
            dir: MemoryDir::from_workspace(workspace_root).index_dir(),
        }
    }

    #[cfg(test)]
    pub(crate) fn for_test(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// Synchronize the complete caller-authorized set. Missing IDs are deleted,
    /// changed IDs are replaced, and unchanged documents keep their segments.
    pub fn replace_documents(
        &self,
        documents: &[MemorySearchDocument],
    ) -> Result<(), MemoryIndexError> {
        let manifest = IndexManifest::from_documents(documents)?;
        let payload = serde_json::to_string(&manifest)?;
        agena_storage_layout::secure_directory(&self.dir)?;
        let (schema, fields) = build_schema();
        let directory = MmapDirectory::open(&self.dir).map_err(tantivy::TantivyError::from)?;
        let index = Index::open_or_create(directory, schema)?;
        register_tokenizers(&index)?;
        if index.load_metas()?.payload.as_deref() == Some(payload.as_str()) {
            return Ok(());
        }
        let mut writer = index.writer(15_000_000)?;
        // Read again after acquiring Tantivy's exclusive writer lock. The
        // manifest and segment changes are committed atomically in one payload.
        let metadata = index.load_metas()?;
        if metadata.payload.as_deref() == Some(payload.as_str()) {
            return Ok(());
        }
        let previous = IndexManifest::parse(metadata.payload.as_deref());
        if let Some(previous) = &previous {
            for (id, old_hash) in &previous.documents {
                if manifest.documents.get(id) != Some(old_hash) {
                    writer.delete_term(Term::from_field_text(fields.id, id));
                }
            }
        } else {
            // Migrate earlier fingerprint/indexed-token policies on first sync.
            writer.delete_all_documents()?;
        }
        for document in documents {
            let unchanged = previous.as_ref().is_some_and(|previous| {
                previous.documents.get(&document.id) == manifest.documents.get(&document.id)
            });
            if !unchanged {
                writer.add_document(to_index_document(document, &fields))?;
            }
        }
        let mut commit = writer.prepare_commit()?;
        commit.set_payload(&payload);
        commit.commit()?;
        Ok(())
    }

    /// Search literal text, returning documents for existing callers.
    pub fn search(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<MemorySearchDocument>, MemoryIndexError> {
        Ok(self
            .search_with_details(query, limit)?
            .into_iter()
            .map(|hit| hit.document)
            .collect())
    }

    /// Search with ranking evidence and a relevant, source-addressable passage.
    /// Empty/punctuation-only queries and indexes not yet created return no hits.
    pub fn search_with_details(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<MemorySearchHit>, MemoryIndexError> {
        let query = LiteralQuery::new(query);
        if limit == 0 || query.is_empty() || !self.dir.join("meta.json").exists() {
            return Ok(Vec::new());
        }
        let limit = limit.min(MAX_SEARCH_RESULTS);
        let (_schema, fields) = build_schema();
        let index = Index::open_in_dir(&self.dir)?;
        register_tokenizers(&index)?;
        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::Manual)
            .try_into()?;
        let searcher = reader.searcher();
        let parsed_query = literal_query(&query, &fields);
        let collector = TopDocs::with_limit(limit).order_by_score();
        let top_docs: Vec<(f32, DocAddress)> = searcher.search(&parsed_query, &collector)?;
        let mut hits = Vec::with_capacity(top_docs.len());
        for (score, address) in top_docs {
            let document = document_from_hit(&searcher.doc::<TantivyDocument>(address)?, &fields);
            hits.push(query.hit(document, score));
        }
        sort_hits(&mut hits);
        hits.truncate(limit);
        Ok(hits)
    }
}

fn literal_query(query: &LiteralQuery, fields: &MemoryIndexFields) -> BooleanQuery {
    let mut clauses = Vec::new();
    for word in &query.words {
        for (field, boost) in [
            (fields.name, 3.0),
            (fields.description, 2.0),
            (fields.searchable_text, 1.0),
        ] {
            clauses.push(boosted_term(field, word, boost));
        }
    }
    // Independent terms are essential here: QueryParser turns an n-gram token
    // stream into a PhraseQuery, requiring the entire natural-language question.
    // Restricting this field to CJK also prevents "we" from matching "weather".
    for gram in &query.cjk_ngrams {
        clauses.push(boosted_term(
            fields.searchable_ngrams,
            gram,
            0.6 * (gram.chars().count() - 1) as f32,
        ));
    }
    for literal in &query.literals {
        for (field, boost) in [
            (fields.name, 5.0),
            (fields.description, 4.0),
            (fields.searchable_text, 3.0),
        ] {
            let terms = WordSpans::new(literal)
                .map(|span| Term::from_field_text(field, &literal[span]))
                .collect::<Vec<_>>();
            if terms.len() >= 2 {
                let phrase = PhraseQuery::new(terms);
                clauses.push((
                    Occur::Should,
                    Box::new(BoostQuery::new(Box::new(phrase), boost)) as Box<dyn Query>,
                ));
            }
        }
        clauses.push(boosted_term(fields.path, literal, 5.0));
    }
    BooleanQuery::new(clauses)
}

fn boosted_term(field: Field, text: &str, boost: f32) -> (Occur, Box<dyn Query>) {
    let query = TermQuery::new(
        Term::from_field_text(field, text),
        IndexRecordOption::WithFreqs,
    );
    (
        Occur::Should,
        Box::new(BoostQuery::new(Box::new(query), boost)),
    )
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

// Retain the v1 schema so old on-disk caches can be migrated transactionally.
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
    (
        builder.build(),
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
    let ngrams = TextAnalyzer::builder(NgramTokenizer::new(2, 4, false).map_err(|error| {
        tantivy::TantivyError::InvalidArgument(format!(
            "failed to create the 2-4 character memory-index n-gram tokenizer: {error}"
        ))
    })?)
    .filter(LowerCaser)
    .build();
    let words = TextAnalyzer::builder(ScriptTokenizer::default())
        .filter(LowerCaser)
        .filter(RemoveLongFilter::limit(MAX_WORD_BYTES + 1))
        .build();
    index.tokenizers().register("default", words);
    index.tokenizers().register(NGRAM_TOKENIZER, ngrams);
    Ok(())
}

#[derive(Clone, Default)]
struct ScriptTokenizer {
    token: Token,
}

struct ScriptTokenStream<'a> {
    text: &'a str,
    spans: WordSpans<'a>,
    token: &'a mut Token,
}

impl Tokenizer for ScriptTokenizer {
    type TokenStream<'a> = ScriptTokenStream<'a>;

    fn token_stream<'a>(&'a mut self, text: &'a str) -> Self::TokenStream<'a> {
        self.token.reset();
        ScriptTokenStream {
            text,
            spans: WordSpans::new(text),
            token: &mut self.token,
        }
    }
}

impl TokenStream for ScriptTokenStream<'_> {
    fn advance(&mut self) -> bool {
        let Some(span) = self.spans.next() else {
            return false;
        };
        self.token.text.clear();
        self.token.text.push_str(&self.text[span.clone()]);
        self.token.offset_from = span.start;
        self.token.offset_to = span.end;
        self.token.position = self.token.position.wrapping_add(1);
        true
    }

    fn token(&self) -> &Token {
        self.token
    }
    fn token_mut(&mut self) -> &mut Token {
        self.token
    }
}

fn to_index_document(
    document: &MemorySearchDocument,
    fields: &MemoryIndexFields,
) -> TantivyDocument {
    let mut stored = TantivyDocument::new();
    stored.add_text(fields.id, &document.id);
    stored.add_text(fields.name, &document.name);
    stored.add_text(fields.description, &document.description);
    if let Some(memory_type) = &document.memory_type {
        stored.add_text(fields.memory_type, memory_type);
    }
    stored.add_text(fields.body, &document.body);
    stored.add_text(fields.path, &document.path);
    let normalized_path = lowercase(&document.path);
    if normalized_path != document.path {
        // Keep the original first stored value for source navigation, while
        // matching the same case-folding policy as literal query terms.
        stored.add_text(fields.path, normalized_path);
    }
    stored.add_text(fields.searchable_text, &document.searchable_text);
    // A separate field value prevents phrases from crossing the content/path
    // boundary. The original first value still round-trips through search().
    stored.add_text(fields.searchable_text, &document.path);
    stored.add_text(
        fields.searchable_ngrams,
        cjk_text(&document.searchable_text),
    );
    stored.add_text(fields.searchable_ngrams, cjk_text(&document.path));
    stored
}

fn document_from_hit(
    document: &TantivyDocument,
    fields: &MemoryIndexFields,
) -> MemorySearchDocument {
    let searchable_text = first_text(document, fields.searchable_text);
    MemorySearchDocument {
        id: first_text(document, fields.id),
        name: first_text(document, fields.name),
        description: first_text(document, fields.description),
        memory_type: optional_text(document, fields.memory_type),
        body: first_text(document, fields.body),
        path: first_text(document, fields.path),
        searchable_ngrams: searchable_text.clone(),
        searchable_text,
    }
}

fn first_text(document: &TantivyDocument, field: Field) -> String {
    optional_text(document, field).unwrap_or_default()
}

fn optional_text(document: &TantivyDocument, field: Field) -> Option<String> {
    document
        .get_first(field)
        .and_then(|value| value.as_str())
        .map(ToOwned::to_owned)
}
