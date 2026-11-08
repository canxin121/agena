use std::{
    collections::BTreeMap,
    fs, io,
    path::{Path, PathBuf},
};

use agena_storage_layout::StorageLayout;
use serde::{Deserialize, Serialize};
use tantivy::{
    DocAddress, Index, IndexReader, ReloadPolicy, TantivyDocument, TantivyError, Term,
    collector::TopDocs,
    directory::error::{OpenDirectoryError, OpenReadError},
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

const NGRAM_TOKENIZER: &str = "memory_cjk";
const WORD_TOKENIZER: &str = "memory_words";
const CACHE_NAMESPACE: &str = "typed-lexical-v1";
const CACHE_FORMAT: &str = "agena.memory.typed-lexical/v1";
const CACHE_LOCK: &str = ".index.lock";

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct IndexManifest {
    format: String,
    documents: BTreeMap<String, String>,
}

impl IndexManifest {
    fn from_documents(documents: &[MemorySearchDocument]) -> Result<Self, MemoryIndexError> {
        validate_documents(documents)?;
        let documents = documents
            .iter()
            .map(|document| Ok((document.id.clone(), document_hash(document)?)))
            .collect::<Result<_, MemoryIndexError>>()?;
        Ok(Self {
            format: CACHE_FORMAT.to_owned(),
            documents,
        })
    }

    fn read_current(payload: Option<&str>) -> Option<Self> {
        let manifest: Self = serde_json::from_str(payload?).ok()?;
        (manifest.format == CACHE_FORMAT).then_some(manifest)
    }
}

struct CacheSnapshot {
    index: Index,
    reader: IndexReader,
    manifest: IndexManifest,
}

/// Rebuildable full-text cache. Source authorization remains with the caller.
#[derive(Clone)]
pub struct MemoryIndex {
    dir: PathBuf,
}

impl MemoryIndex {
    pub fn for_workspace(workspace_root: &Path) -> Self {
        Self::at_cache_root(StorageLayout::current().memory_index_dir(workspace_root))
    }

    fn at_cache_root(root: PathBuf) -> Self {
        Self {
            dir: root.join(CACHE_NAMESPACE),
        }
    }

    #[cfg(test)]
    pub(crate) fn for_test(root: PathBuf) -> Self {
        Self::at_cache_root(root)
    }

    pub(crate) fn data_dir(&self) -> PathBuf {
        self.dir.join("index")
    }

    /// Synchronize the complete authorized source snapshot. Missing records are
    /// deleted and changed records are replaced; untouched records keep segments.
    ///
    /// A missing or invalid cache is built solely from these supplied records.
    pub fn replace_documents(
        &self,
        documents: &[MemorySearchDocument],
    ) -> Result<(), MemoryIndexError> {
        let manifest = IndexManifest::from_documents(documents)?;
        let _lock = self.lock_cache(true)?.ok_or_else(|| {
            MemoryIndexError::InvalidCache("cache lock was not created".to_owned())
        })?;
        let (schema, fields) = build_schema();
        if let Some(snapshot) = self.open_current(&schema)? {
            if snapshot.manifest == manifest {
                return Ok(());
            }
            commit_documents(
                &snapshot.index,
                &fields,
                documents,
                &manifest,
                Some(&snapshot.manifest),
            )?;
        } else {
            self.rebuild(&schema, &fields, documents, &manifest)?;
        }
        Ok(())
    }

    /// Replace the cache solely with the caller's current authorized source snapshot.
    /// Use after lazy stored-payload corruption, even when the manifest is unchanged.
    pub fn rebuild_documents(
        &self,
        documents: &[MemorySearchDocument],
    ) -> Result<(), MemoryIndexError> {
        let manifest = IndexManifest::from_documents(documents)?;
        let _lock = self.lock_cache(true)?.ok_or_else(|| {
            MemoryIndexError::InvalidCache("cache lock was not created".to_owned())
        })?;
        let (schema, fields) = build_schema();
        self.rebuild(&schema, &fields, documents, &manifest)
    }

    /// Search literal text with scores, matched terms, and bounded source passages.
    /// An absent/invalid index or an empty/punctuation-only query returns no hits.
    /// Corrupt stored payloads return InvalidCache; the caller can rebuild from
    /// its current authorized source snapshot before retrying once.
    pub fn search(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<MemorySearchHit>, MemoryIndexError> {
        let query = LiteralQuery::new(query);
        if limit == 0 || query.is_empty() {
            return Ok(Vec::new());
        }
        let Some(_lock) = self.lock_cache(false)? else {
            return Ok(Vec::new());
        };
        let (schema, fields) = build_schema();
        let Some(snapshot) = self.open_current(&schema)? else {
            return Ok(Vec::new());
        };
        let limit = limit.min(MAX_SEARCH_RESULTS);
        let searcher = snapshot.reader.searcher();
        let parsed_query = literal_query(&query, &fields);
        let collector = TopDocs::with_limit(limit).order_by_score();
        let top_docs: Vec<(f32, DocAddress)> = searcher
            .search(&parsed_query, &collector)
            .map_err(search_cache_error)?;
        let mut hits = Vec::with_capacity(top_docs.len());
        for (score, address) in top_docs {
            let stored = searcher
                .doc::<TantivyDocument>(address)
                .map_err(search_cache_error)?;
            let encoded = stored
                .get_first(fields.document)
                .and_then(|value| value.as_str())
                .ok_or_else(|| {
                    MemoryIndexError::InvalidCache("record payload is missing".to_owned())
                })?;
            let document: MemorySearchDocument =
                serde_json::from_str(encoded).map_err(|error| {
                    MemoryIndexError::InvalidCache(format!("record payload is invalid: {error}"))
                })?;
            let hash = document_hash(&document)?;
            if snapshot.manifest.documents.get(&document.id) != Some(&hash) {
                return Err(MemoryIndexError::InvalidCache(
                    "record payload does not match its current manifest".to_owned(),
                ));
            }
            hits.push(query.hit(document, score));
        }
        sort_hits(&mut hits);
        hits.truncate(limit);
        Ok(hits)
    }

    fn lock_cache(&self, write: bool) -> Result<Option<fs::File>, MemoryIndexError> {
        if write {
            agena_storage_layout::secure_directory(&self.dir)?;
        }
        match fs::symlink_metadata(&self.dir) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
            Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => {}
            Ok(_) => {
                return Err(MemoryIndexError::InvalidCache(
                    "cache must be a regular directory".to_owned(),
                ));
            }
        }
        let path = self.dir.join(CACHE_LOCK);
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == io::ErrorKind::NotFound && !write => return Ok(None),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
            Ok(meta) if meta.is_file() && !meta.file_type().is_symlink() => {}
            Ok(_) => {
                return Err(MemoryIndexError::InvalidCache(
                    "cache lock must be a regular file".to_owned(),
                ));
            }
        }
        let mut options = fs::OpenOptions::new();
        options.read(true).write(write).create(write);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
        }
        let lock = options.open(&path)?;
        if !lock.metadata()?.is_file() {
            return Err(MemoryIndexError::InvalidCache(
                "cache lock must be a regular file".to_owned(),
            ));
        }
        if write {
            agena_storage_layout::secure_file(&path)?;
            lock.lock()?;
        } else {
            lock.lock_shared()?;
        }
        Ok(Some(lock))
    }

    fn open_current(&self, schema: &Schema) -> Result<Option<CacheSnapshot>, MemoryIndexError> {
        let dir = self.data_dir();
        match fs::symlink_metadata(&dir) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
            Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => {}
            Ok(_) => {
                return Err(MemoryIndexError::InvalidCache(
                    "index must be a regular directory".to_owned(),
                ));
            }
        }
        let open = || -> Result<Option<CacheSnapshot>, TantivyError> {
            let index = Index::open_in_dir(&dir)?;
            if index.schema() != *schema {
                return Ok(None);
            }
            let metadata = index.load_metas()?;
            let Some(manifest) = IndexManifest::read_current(metadata.payload.as_deref()) else {
                return Ok(None);
            };
            register_tokenizers(&index)?;
            let reader = index
                .reader_builder()
                .reload_policy(ReloadPolicy::Manual)
                .try_into()?;
            Ok(Some(CacheSnapshot {
                index,
                reader,
                manifest,
            }))
        };
        match open() {
            Ok(snapshot) => Ok(snapshot),
            Err(error) if invalid_cache_error(&error) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    fn rebuild(
        &self,
        schema: &Schema,
        fields: &MemoryIndexFields,
        documents: &[MemorySearchDocument],
        manifest: &IndexManifest,
    ) -> Result<(), MemoryIndexError> {
        // Build completely before replacing the cache. The stable lock outside
        // the index directory protects publication across threads and processes.
        let staged = tempfile::Builder::new()
            .prefix(".rebuild-")
            .tempdir_in(&self.dir)?;
        {
            let index = Index::create_in_dir(staged.path(), schema.clone())?;
            register_tokenizers(&index)?;
            commit_documents(&index, fields, documents, manifest, None)?;
        }
        let target = self.data_dir();
        match fs::symlink_metadata(&target) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
            Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => {
                fs::remove_dir_all(&target)?
            }
            Ok(_) => {
                return Err(MemoryIndexError::InvalidCache(
                    "index must be a regular directory".to_owned(),
                ));
            }
        }
        fs::rename(staged.path(), &target)?;
        #[cfg(unix)]
        fs::File::open(&self.dir)?.sync_all()?;
        Ok(())
    }
}

fn document_hash(document: &MemorySearchDocument) -> Result<String, MemoryIndexError> {
    Ok(blake3::hash(&serde_json::to_vec(document)?)
        .to_hex()
        .to_string())
}

fn search_cache_error(error: TantivyError) -> MemoryIndexError {
    if invalid_cache_error(&error) {
        MemoryIndexError::InvalidCache(error.to_string())
    } else {
        error.into()
    }
}

fn invalid_cache_error(error: &TantivyError) -> bool {
    let invalid_io = |error: &io::Error| {
        matches!(
            error.kind(),
            io::ErrorKind::NotFound | io::ErrorKind::InvalidData | io::ErrorKind::UnexpectedEof
        )
    };
    match error {
        TantivyError::DataCorruption(_)
        | TantivyError::IncompatibleIndex(_)
        | TantivyError::SchemaError(_)
        | TantivyError::DeserializeError(_)
        | TantivyError::OpenDirectoryError(OpenDirectoryError::DoesNotExist(_))
        | TantivyError::OpenReadError(
            OpenReadError::FileDoesNotExist(_) | OpenReadError::IncompatibleIndex(_),
        ) => true,
        TantivyError::IoError(error) => invalid_io(error),
        TantivyError::OpenReadError(OpenReadError::IoError { io_error, .. }) => {
            invalid_io(io_error)
        }
        _ => false,
    }
}

fn commit_documents(
    index: &Index,
    fields: &MemoryIndexFields,
    documents: &[MemorySearchDocument],
    manifest: &IndexManifest,
    previous: Option<&IndexManifest>,
) -> Result<(), MemoryIndexError> {
    let mut writer = index.writer(15_000_000)?;
    if let Some(previous) = previous {
        for (id, hash) in &previous.documents {
            if manifest.documents.get(id) != Some(hash) {
                writer.delete_term(Term::from_field_text(fields.id, id));
            }
        }
    }
    for document in documents {
        let unchanged = previous.is_some_and(|previous| {
            previous.documents.get(&document.id) == manifest.documents.get(&document.id)
        });
        if !unchanged {
            writer.add_document(to_index_document(document, fields)?)?;
        }
    }
    let payload = serde_json::to_string(manifest)?;
    let mut commit = writer.prepare_commit()?;
    commit.set_payload(&payload);
    commit.commit()?;
    writer.wait_merging_threads()?;
    Ok(())
}

fn literal_query(query: &LiteralQuery, fields: &MemoryIndexFields) -> BooleanQuery {
    let mut clauses = Vec::new();
    for word in &query.words {
        for (field, boost) in [
            (fields.name, 3.0),
            (fields.description, 2.0),
            (fields.body, 1.0),
            (fields.path, 1.5),
        ] {
            clauses.push(boosted_term(field, word, boost));
        }
    }
    // Independent CJK terms avoid treating an entire natural-language question
    // as a phrase. ASCII words are never indexed as short arbitrary n-grams.
    for gram in &query.cjk_ngrams {
        clauses.push(boosted_term(
            fields.cjk,
            gram,
            0.6 * (gram.chars().count() - 1) as f32,
        ));
    }
    for literal in &query.literals {
        for (field, boost) in [
            (fields.name, 5.0),
            (fields.description, 4.0),
            (fields.body, 3.0),
            (fields.path, 4.0),
        ] {
            let terms = WordSpans::new(literal)
                .map(|span| Term::from_field_text(field, &literal[span]))
                .collect::<Vec<_>>();
            if terms.len() >= 2 {
                clauses.push((
                    Occur::Should,
                    Box::new(BoostQuery::new(Box::new(PhraseQuery::new(terms)), boost))
                        as Box<dyn Query>,
                ));
            }
        }
        clauses.push(boosted_term(fields.exact_path, literal, 5.0));
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
    document: Field,
    name: Field,
    description: Field,
    body: Field,
    path: Field,
    exact_path: Field,
    cjk: Field,
}

fn build_schema() -> (Schema, MemoryIndexFields) {
    let mut builder = Schema::builder();
    let words = TextOptions::default().set_indexing_options(
        TextFieldIndexing::default()
            .set_tokenizer(WORD_TOKENIZER)
            .set_index_option(IndexRecordOption::WithFreqsAndPositions),
    );
    let ngrams = TextOptions::default().set_indexing_options(
        TextFieldIndexing::default()
            .set_tokenizer(NGRAM_TOKENIZER)
            .set_index_option(IndexRecordOption::WithFreqs),
    );
    let fields = MemoryIndexFields {
        id: builder.add_text_field("id", STRING),
        document: builder.add_text_field("document", STORED),
        name: builder.add_text_field("name", words.clone()),
        description: builder.add_text_field("description", words.clone()),
        body: builder.add_text_field("body", words.clone()),
        path: builder.add_text_field("path", words),
        exact_path: builder.add_text_field("exact_path", STRING),
        cjk: builder.add_text_field("cjk", ngrams),
    };
    (builder.build(), fields)
}

fn register_tokenizers(index: &Index) -> Result<(), TantivyError> {
    let ngrams = TextAnalyzer::builder(NgramTokenizer::new(2, 4, false).map_err(|error| {
        TantivyError::InvalidArgument(format!("invalid CJK n-gram tokenizer: {error}"))
    })?)
    .filter(LowerCaser)
    .build();
    let words = TextAnalyzer::builder(ScriptTokenizer::default())
        .filter(LowerCaser)
        .filter(RemoveLongFilter::limit(MAX_WORD_BYTES + 1))
        .build();
    index.tokenizers().register(WORD_TOKENIZER, words);
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
) -> Result<TantivyDocument, MemoryIndexError> {
    let mut stored = TantivyDocument::new();
    stored.add_text(fields.id, &document.id);
    stored.add_text(fields.document, serde_json::to_string(document)?);
    stored.add_text(fields.name, &document.name);
    stored.add_text(fields.description, &document.description);
    stored.add_text(fields.body, &document.body);
    stored.add_text(fields.path, &document.path);
    stored.add_text(fields.exact_path, lowercase(&document.path));
    for text in [
        &document.name,
        &document.description,
        &document.body,
        &document.path,
    ] {
        stored.add_text(fields.cjk, cjk_text(text));
    }
    Ok(stored)
}
