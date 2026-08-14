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
///
/// When the query names a complete tool (`monitor.start`), the exact-name
/// match is hoisted ahead of every fuzzy match: a tool the caller already
/// identified by name must land on the first page regardless of how many
/// loosely-matching tools dilute the fuzzy ranking, or the caller concludes —
/// wrongly — that the tool does not exist.
pub fn search_tools(
    documents: &[ToolSearchDocument],
    query: &str,
    limit: usize,
) -> Vec<ToolSearchDocument> {
    let query = query.trim();
    if documents.is_empty() || query.is_empty() || limit == 0 {
        return Vec::new();
    }

    let normalized_query = normalize_search_text(query);
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
    let mut matched = pattern
        .match_list(candidates, &mut matcher)
        .into_iter()
        .map(|(candidate, _score)| candidate.document.clone())
        .collect::<Vec<_>>();

    // Hoist exact tool-name hits above the fuzzy ranking.
    let mut exact = Vec::new();
    matched.retain(|document| {
        if normalize_search_text(&document.name) == normalized_query {
            exact.push(document.clone());
            false
        } else {
            true
        }
    });
    exact.extend(matched);
    exact.into_iter().take(limit).collect()
}

/// The search space's punctuation-to-space normalization, so an exact-name
/// comparison and the fuzzy query normalization stay in the same form.
/// Case-insensitive and whitespace-collapsed: `monitor.start`, `Monitor.Start`
/// and `monitor . start` all compare equal.
fn normalize_search_text(text: &str) -> String {
    text.chars()
        .map(|character| {
            if character.is_alphanumeric() || character.is_whitespace() {
                character
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
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

    #[test]
    fn exact_tool_name_ranks_first_even_when_many_tools_dilute_the_match() {
        // A catalog where dozens of tools mention "monitor" loosely, so a pure
        // fuzzy ranking buries the exact-name hit past the first page.
        let mut docs = Vec::new();
        for index in 0..30 {
            docs.push(doc(
                &format!("service.monitor{}", index),
                "monitoring heartbeat metrics stream watch log tail",
                &["monitor"],
            ));
        }
        docs.push(doc("monitor.start", "Start a continuous background monitor", &["monitor"]));
        docs.push(doc("monitor.stop", "Stop one background monitor", &["monitor"]));
        docs.push(doc(
            "session.rename",
            "Rename the current session",
            &["session"],
        ));

        let results = search_tools(&docs, "monitor.start", 8);
        assert_eq!(
            results.first().map(|doc| doc.name.as_str()),
            Some("monitor.start"),
            "the exact tool name must land on the first page"
        );
        assert!(results.contains(&doc("monitor.stop", "Stop one background monitor", &["monitor"])));
        // Non-matching tools never appear, exact hoist or not.
        assert!(!results
            .iter()
            .any(|doc| doc.name == "session.rename"));
    }

    #[test]
    fn exact_name_match_is_case_and_punctuation_insensitive() {
        let docs = vec![
            doc("monitor.start", "Start a continuous background monitor", &[]),
            doc("monitor.state", "Report monitor state", &[]),
        ];

        for query in ["Monitor.Start", "monitor . start", "MONITOR.START"] {
            let results = search_tools(&docs, query, 2);
            assert_eq!(
                results.first().map(|doc| doc.name.as_str()),
                Some("monitor.start"),
                "query {query:?} must hoist the exact-name hit"
            );
        }
    }

    #[test]
    fn exact_hoist_only_promotes_a_tool_with_the_query_as_its_full_name() {
        let docs = vec![
            doc("monitor.start", "Start a continuous background monitor", &[]),
            doc("monitor.startup", "Inspect process startup", &[]),
            doc("shell.run", "Run a shell command", &[]),
        ];

        // `monitor.startup` is a different tool: its name is not the query, so
        // the exact-name hoist must not claim it; fuzzy ranking still leads
        // with the closest name.
        let results = search_tools(&docs, "monitor.start", 3);
        assert_eq!(
            results.first().map(|doc| doc.name.as_str()),
            Some("monitor.start")
        );
        // The hoist only reorders; it never invents a result that does not
        // match at all.
        assert!(!results
            .iter()
            .any(|doc| doc.name == "shell.run"));
    }
}
