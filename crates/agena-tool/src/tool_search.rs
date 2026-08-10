//! Fast fuzzy searching for the small tool and plugin catalogs.

use nucleo_matcher::pattern::{AtomKind, CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// A document indexed for tool search.
pub struct ToolSearchDocument {
    pub id: String,
    pub name: String,
    pub description: String,
    pub tags: Vec<String>,
    pub plugin_id: Option<String>,
    searchable_text: String,
}

impl ToolSearchDocument {
    pub fn new(
        name: String,
        description: String,
        tags: Vec<String>,
        plugin_id: Option<String>,
    ) -> Self {
        let searchable_text = format!(
            "{} {} {} {}",
            name,
            tags.join(" "),
            plugin_id.as_deref().unwrap_or(""),
            description,
        );
        Self {
            id: name.clone(),
            name,
            description,
            tags,
            plugin_id,
            searchable_text,
        }
    }
}

struct SearchCandidate<'a> {
    document: &'a ToolSearchDocument,
}

impl AsRef<str> for SearchCandidate<'_> {
    fn as_ref(&self) -> &str {
        self.document.searchable_text.as_str()
    }
}

/// Rank a bounded in-memory catalog with Nucleo's battle-tested fuzzy matcher.
///
/// Tool catalogs are normally only tens or hundreds of entries, so building a
/// full-text index per request costs much more than matching the candidates
/// directly. Nucleo handles Unicode normalization, case folding, tokenized
/// query patterns, typo-like subsequences, and deterministic score ordering.
pub fn search_tools(
    documents: &[ToolSearchDocument],
    query: &str,
    limit: usize,
) -> Vec<ToolSearchDocument> {
    let query = query.trim();
    if documents.is_empty() || query.is_empty() || limit == 0 {
        return Vec::new();
    }

    let normalized_query = query
        .chars()
        .map(|character| {
            if character.is_alphanumeric() || character.is_whitespace() {
                character
            } else {
                ' '
            }
        })
        .collect::<String>();
    let pattern = Pattern::new(
        normalized_query.as_str(),
        CaseMatching::Ignore,
        Normalization::Smart,
        AtomKind::Fuzzy,
    );
    let candidates = documents
        .iter()
        .map(|document| SearchCandidate { document });
    let mut matcher = Matcher::new(Config::DEFAULT.match_paths());
    pattern
        .match_list(candidates, &mut matcher)
        .into_iter()
        .take(limit)
        .map(|(candidate, _score)| candidate.document.clone())
        .collect()
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

        let results = search_tools(&docs, "applypatch", 5);

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

        let results = search_tools(&docs, "rn", 5);

        assert_eq!(
            results.first().map(|doc| doc.name.as_str()),
            Some("agena.shell/run")
        );
    }

    #[test]
    fn search_matches_multiple_description_words() {
        let docs = vec![
            doc(
                "agena.activities/list",
                "List current background activities and their status",
                &["discovery"],
            ),
            doc("agena.fs/read", "Read a workspace file", &["filesystem"]),
        ];

        let results = search_tools(&docs, "background status", 5);

        assert_eq!(
            results.first().map(|doc| doc.name.as_str()),
            Some("agena.activities/list")
        );
    }

    #[test]
    fn search_treats_tool_name_punctuation_as_token_boundaries() {
        let docs = vec![
            doc("shell.run", "Run one shell process", &["shell"]),
            doc("shell.logs", "Read process logs", &["shell"]),
        ];

        let results = search_tools(&docs, "process.run", 3);

        assert_eq!(
            results.first().map(|doc| doc.name.as_str()),
            Some("shell.run")
        );
    }

    #[test]
    fn search_honors_result_limit() {
        let docs = vec![
            doc("one", "background task one", &[]),
            doc("two", "background task two", &[]),
            doc("three", "background task three", &[]),
        ];

        assert_eq!(search_tools(&docs, "background", 2).len(), 2);
    }
}
