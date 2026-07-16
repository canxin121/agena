use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use tantivy::collector::TopDocs;
use tantivy::query::QueryParser;
use tantivy::schema::{
    Field, IndexRecordOption, STORED, STRING, Schema, TextFieldIndexing, TextOptions, Value,
};
use tantivy::tokenizer::{LowerCaser, NgramTokenizer, SimpleTokenizer, TextAnalyzer};
use tantivy::{DocAddress, Index, ReloadPolicy, TantivyDocument};
use thiserror::Error;

const NGRAM_TOKENIZER: &str = "tool_search_ngram";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct ToolSearchDocument {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) tags: Vec<String>,
    pub(crate) plugin_id: Option<String>,
    searchable_text: String,
    #[serde(skip_serializing, skip_deserializing)]
    searchable_ngrams: String,
}

impl ToolSearchDocument {
    pub(crate) fn new(
        name: String,
        description: String,
        tags: Vec<String>,
        plugin_id: Option<String>,
    ) -> Self {
        let searchable_text = format!(
            "{} {} {} {}",
            name,
            description,
            plugin_id.as_deref().unwrap_or(""),
            tags.join(" ")
        );
        Self {
            id: name.clone(),
            name,
            description,
            tags,
            plugin_id,
            searchable_ngrams: normalize_search_text(&searchable_text),
            searchable_text,
        }
    }
}

#[derive(Debug, Error)]
pub(crate) enum ToolSearchError {
    #[error("tantivy error: {0}")]
    Tantivy(#[from] tantivy::TantivyError),
}

pub(crate) fn search_tools(
    documents: &[ToolSearchDocument],
    query: &str,
    limit: usize,
) -> Result<Vec<ToolSearchDocument>, ToolSearchError> {
    if documents.is_empty() || limit == 0 {
        return Ok(Vec::new());
    }

    let normalized_query = normalize_search_text(query);
    if normalized_query.trim().is_empty() {
        return Ok(Vec::new());
    }

    let (schema, fields) = build_schema();
    let index = Index::create_in_ram(schema);
    register_tokenizers(&index)?;
    let mut writer = index.writer(15_000_000)?;
    for document in documents {
        let mut stored = TantivyDocument::new();
        stored.add_text(fields.id, document.id.clone());
        stored.add_text(fields.name, document.name.clone());
        stored.add_text(fields.description, document.description.clone());
        if let Some(plugin_id) = document.plugin_id.as_deref() {
            stored.add_text(fields.plugin_id, plugin_id);
        }
        for tag in &document.tags {
            stored.add_text(fields.tags, tag);
        }
        stored.add_text(fields.searchable_text, document.searchable_text.clone());
        stored.add_text(fields.searchable_ngrams, document.searchable_ngrams.clone());
        writer.add_document(stored)?;
    }
    writer.commit()?;

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
            fields.tags,
            fields.plugin_id,
            fields.searchable_text,
            fields.searchable_ngrams,
        ],
    );
    parser.set_field_boost(fields.name, 4.0);
    parser.set_field_boost(fields.tags, 2.5);
    parser.set_field_boost(fields.description, 2.0);
    parser.set_field_boost(fields.plugin_id, 1.2);
    parser.set_field_boost(fields.searchable_text, 1.0);
    parser.set_field_boost(fields.searchable_ngrams, 0.8);
    let (parsed_query, errors) = parser.parse_query_lenient(&normalized_query);
    if !errors.is_empty() {
        tracing::debug!(
            target: "agena::tool_search",
            "tool query parsed leniently for '{query}': {:?}",
            errors
        );
    }
    let collector = TopDocs::with_limit(limit).order_by_score();
    let top_docs: Vec<(f32, DocAddress)> = searcher.search(&parsed_query, &collector)?;
    let mut results = Vec::with_capacity(limit);
    let mut seen_ids = HashSet::new();
    for (_score, address) in top_docs {
        let doc = searcher.doc::<TantivyDocument>(address)?;
        let document = document_from_hit(&doc, &fields);
        if seen_ids.insert(document.id.clone()) {
            results.push(document);
        }
    }
    let fallback_limit = limit.saturating_mul(3).max(limit);
    for document in fallback_search(documents, &normalized_query, fallback_limit) {
        if seen_ids.insert(document.id.clone()) {
            results.push(document);
        }
        if results.len() >= limit {
            break;
        }
    }
    if results.is_empty() {
        return Ok(results);
    }
    results.truncate(limit);
    Ok(results)
}

#[derive(Clone, Copy)]
struct ToolSearchFields {
    id: Field,
    name: Field,
    description: Field,
    tags: Field,
    plugin_id: Field,
    searchable_text: Field,
    searchable_ngrams: Field,
}

fn build_schema() -> (Schema, ToolSearchFields) {
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
    let tags = builder.add_text_field("tags", indexed_text.clone());
    let plugin_id = builder.add_text_field("plugin_id", indexed_text.clone());
    let searchable_text = builder.add_text_field("searchable_text", indexed_text);
    let searchable_ngrams = builder.add_text_field("searchable_ngrams", ngram_text);
    let schema = builder.build();
    (
        schema,
        ToolSearchFields {
            id,
            name,
            description,
            tags,
            plugin_id,
            searchable_text,
            searchable_ngrams,
        },
    )
}

fn register_tokenizers(index: &Index) -> Result<(), ToolSearchError> {
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

fn document_from_hit(doc: &TantivyDocument, fields: &ToolSearchFields) -> ToolSearchDocument {
    ToolSearchDocument {
        id: first_text(doc, fields.id),
        name: first_text(doc, fields.name),
        description: first_text(doc, fields.description),
        tags: all_text(doc, fields.tags),
        plugin_id: optional_text(doc, fields.plugin_id),
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

fn all_text(doc: &TantivyDocument, field: Field) -> Vec<String> {
    doc.get_all(field)
        .filter_map(|value| value.as_str().map(ToOwned::to_owned))
        .collect()
}

fn normalize_search_text(value: &str) -> String {
    value
        .trim()
        .to_ascii_lowercase()
        .replace(['/', '.', '_', '-'], " ")
}

fn fallback_search(
    documents: &[ToolSearchDocument],
    normalized_query: &str,
    limit: usize,
) -> Vec<ToolSearchDocument> {
    let tokens = normalized_tokens(normalized_query);
    let mut ranked = documents
        .iter()
        .filter_map(|document| {
            let score = fallback_score(document, normalized_query, tokens.as_slice());
            (score > 0).then_some((score, document))
        })
        .collect::<Vec<_>>();
    ranked.sort_by(|(left_score, left_tool), (right_score, right_tool)| {
        right_score
            .cmp(left_score)
            .then_with(|| left_tool.name.cmp(&right_tool.name))
    });
    ranked
        .into_iter()
        .take(limit)
        .map(|(_, document)| document.clone())
        .collect()
}

fn fallback_score(document: &ToolSearchDocument, normalized_query: &str, tokens: &[String]) -> i32 {
    let normalized_name = normalize_search_text(&document.name);
    let normalized_description = normalize_search_text(&document.description);
    let normalized_plugin_id = document
        .plugin_id
        .as_deref()
        .map(normalize_search_text)
        .unwrap_or_default();
    let normalized_tags = document
        .tags
        .iter()
        .map(|tag| normalize_search_text(tag))
        .collect::<Vec<_>>();
    let compact_query = compact_search_text(normalized_query);
    let compact_name = compact_search_text(&normalized_name);
    let compact_plugin_id = compact_search_text(&normalized_plugin_id);
    let compact_tags = normalized_tags
        .iter()
        .map(|tag| compact_search_text(tag))
        .collect::<Vec<_>>();

    let mut score = 0;

    if normalized_name == normalized_query {
        score += 100;
    } else if normalized_name.contains(normalized_query) {
        score += 45;
    }

    if normalized_tags
        .iter()
        .any(|tag| tag == normalized_query || tag.contains(normalized_query))
    {
        score += 24;
    }

    if normalized_plugin_id.contains(normalized_query) {
        score += 12;
    }

    if normalized_description.contains(normalized_query) {
        score += 20;
    }
    if compact_name.starts_with(compact_query.as_str()) {
        score += 36;
    } else if compact_name.contains(compact_query.as_str()) {
        score += 18;
    }
    if compact_plugin_id.starts_with(compact_query.as_str()) {
        score += 14;
    }
    if compact_tags
        .iter()
        .any(|tag| tag.starts_with(compact_query.as_str()) || tag.contains(compact_query.as_str()))
    {
        score += 16;
    }
    if is_subsequence(compact_query.as_str(), compact_name.as_str()) {
        score += 14;
    }
    let name_distance = bounded_edit_distance(compact_query.as_str(), compact_name.as_str(), 2);
    if name_distance == Some(1) {
        score += 28;
    } else if name_distance == Some(2) {
        score += 14;
    }

    for token in tokens {
        let compact_token = compact_search_text(token);
        if normalized_name.contains(token) {
            score += 12;
        }
        if normalized_tags.iter().any(|tag| tag.contains(token)) {
            score += 6;
        }
        if normalized_plugin_id.contains(token) {
            score += 5;
        }
        if normalized_description.contains(token) {
            score += 5;
        }
        if compact_name.starts_with(compact_token.as_str()) {
            score += 10;
        } else if compact_name.contains(compact_token.as_str()) {
            score += 6;
        }
        if compact_tags
            .iter()
            .any(|tag| tag.contains(compact_token.as_str()))
        {
            score += 5;
        }
        if is_subsequence(compact_token.as_str(), compact_name.as_str()) {
            score += 4;
        }
    }

    score
}

fn normalized_tokens(value: &str) -> Vec<String> {
    value
        .split_whitespace()
        .filter(|token| token.len() >= 2)
        .map(str::to_string)
        .collect()
}

fn compact_search_text(value: &str) -> String {
    value.chars().filter(|ch| !ch.is_whitespace()).collect()
}

fn is_subsequence(needle: &str, haystack: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    let mut needle_chars = needle.chars();
    let mut current = needle_chars.next();
    for haystack_char in haystack.chars() {
        if Some(haystack_char) == current {
            current = needle_chars.next();
            if current.is_none() {
                return true;
            }
        }
    }
    false
}

fn bounded_edit_distance(left: &str, right: &str, max_distance: usize) -> Option<usize> {
    let left_chars = left.chars().collect::<Vec<_>>();
    let right_chars = right.chars().collect::<Vec<_>>();
    if left_chars.is_empty() {
        return Some(right_chars.len()).filter(|distance| *distance <= max_distance);
    }
    if right_chars.is_empty() {
        return Some(left_chars.len()).filter(|distance| *distance <= max_distance);
    }
    let len_diff = left_chars.len().abs_diff(right_chars.len());
    if len_diff > max_distance {
        return None;
    }

    let mut previous = (0..=right_chars.len()).collect::<Vec<_>>();
    let mut current = vec![0; right_chars.len() + 1];
    for (i, left_char) in left_chars.iter().enumerate() {
        current[0] = i + 1;
        let mut row_min = current[0];
        for (j, right_char) in right_chars.iter().enumerate() {
            let substitution_cost = usize::from(left_char != right_char);
            current[j + 1] = (previous[j + 1] + 1)
                .min(current[j] + 1)
                .min(previous[j] + substitution_cost);
            row_min = row_min.min(current[j + 1]);
        }
        if row_min > max_distance {
            return None;
        }
        std::mem::swap(&mut previous, &mut current);
    }
    Some(previous[right_chars.len()]).filter(|distance| *distance <= max_distance)
}

#[cfg(test)]
mod tests {
    use super::{ToolSearchDocument, search_tools};

    fn doc(name: &str, description: &str, tags: &[&str]) -> ToolSearchDocument {
        ToolSearchDocument::new(
            name.to_string(),
            description.to_string(),
            tags.iter().map(|tag| tag.to_string()).collect(),
            None,
        )
    }

    #[test]
    fn search_matches_compact_tool_names() {
        let docs = vec![
            doc(
                "agena.fs/apply_patch",
                "Apply a patch to files",
                &["mutating"],
            ),
            doc("agena.fs/read", "Read a file", &["read_only"]),
        ];

        let results = search_tools(&docs, "applypatch", 5).expect("compact search should succeed");

        assert_eq!(
            results.first().map(|doc| doc.name.as_str()),
            Some("agena.fs/apply_patch")
        );
    }

    #[test]
    fn search_matches_small_typos() {
        let docs = vec![
            doc("agena.shell/run", "Run a foreground command", &["shell"]),
            doc("agena.shell/logs", "Read process logs", &["read_only"]),
        ];

        let results = search_tools(&docs, "rn", 5).expect("fuzzy search should succeed");

        assert_eq!(
            results.first().map(|doc| doc.name.as_str()),
            Some("agena.shell/run")
        );
    }
}
