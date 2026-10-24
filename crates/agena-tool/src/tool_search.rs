//! Precision-first searching for the small tool and plugin catalogs.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// A document indexed for tool search.
pub struct ToolSearchDocument {
    pub id: String,
    pub name: String,
    pub description: String,
    pub tags: Vec<String>,
    pub plugin_id: Option<String>,
}

impl ToolSearchDocument {
    pub fn new(
        name: String,
        description: String,
        tags: Vec<String>,
        plugin_id: Option<String>,
    ) -> Self {
        Self {
            id: name.clone(),
            name,
            description,
            tags,
            plugin_id,
        }
    }
}

#[derive(Debug)]
struct SearchField {
    normalized: String,
    tokens: Vec<String>,
}

impl SearchField {
    fn new(text: &str) -> Self {
        let normalized = normalize_search_text(text);
        Self {
            tokens: normalized.split_whitespace().map(str::to_owned).collect(),
            normalized,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct TermMatch {
    quality: u16,
    field_weight: u16,
    anchored: bool,
}

/// Rank a bounded in-memory catalog with precision-first lexical matching.
///
/// A previous implementation ran an unrestricted fuzzy subsequence matcher
/// over one concatenated name/tag/plugin/description string. With a large
/// `limit`, weak matches whose letters merely appeared in unrelated words
/// leaked into the response. Discovery must be safe to act on, so this search
/// now filters before it ranks:
///
/// - a complete tool name returns only complete-name matches;
/// - every short query (one or two meaningful terms) must be fully covered;
/// - longer natural-language queries may omit a modifier, but never an action
///   such as `stop`, `delete`, `search`, or `write`;
/// - typo tolerance is bounded by edit distance and cannot span words;
/// - names and tags rank above description-only matches.
///
/// Returning fewer results, including zero, is intentional. `limit` caps
/// relevant results; it never asks the searcher to fill the page with noise.
pub fn search_tools(
    documents: &[ToolSearchDocument],
    query: &str,
    limit: usize,
) -> Vec<ToolSearchDocument> {
    let query = query.trim();
    if documents.is_empty() || query.is_empty() || limit == 0 {
        return Vec::new();
    }

    // A whitespace-separated list of explicit tool identifiers is a batch
    // lookup, not a single natural-language phrase whose terms must all occur
    // on one document. Keep ordinary prose on the ranked path below, but
    // recognize identifier-shaped chunks and union their exact matches.
    let identifier_chunks = query.split_whitespace().collect::<Vec<_>>();
    if identifier_chunks.len() > 1
        && identifier_chunks
            .iter()
            .all(|chunk| chunk.chars().any(|character| !character.is_alphanumeric()))
    {
        let mut matches = Vec::new();
        for chunk in identifier_chunks {
            for document in search_tools(documents, chunk, limit) {
                if !matches
                    .iter()
                    .any(|matched: &ToolSearchDocument| matched.id == document.id)
                {
                    matches.push(document);
                }
            }
        }
        return matches.into_iter().take(limit).collect();
    }

    let normalized_query = normalize_search_text(query);
    let exact = documents
        .iter()
        .filter(|document| normalize_search_text(&document.name) == normalized_query)
        .take(limit)
        .cloned()
        .collect::<Vec<_>>();
    if !exact.is_empty() {
        return exact;
    }

    let query_terms = meaningful_query_terms(normalized_query.as_str());
    if query_terms.is_empty() {
        return Vec::new();
    }

    let mut ranked = documents
        .iter()
        .filter_map(|document| {
            relevance_score(document, normalized_query.as_str(), query_terms.as_slice())
                .map(|score| (score, document))
        })
        .collect::<Vec<_>>();
    ranked.sort_by(|(left_score, left), (right_score, right)| {
        right_score
            .cmp(left_score)
            .then_with(|| left.name.cmp(&right.name))
    });
    ranked
        .into_iter()
        .take(limit)
        .map(|(_, document)| document.clone())
        .collect()
}

fn relevance_score(
    document: &ToolSearchDocument,
    normalized_query: &str,
    query_terms: &[String],
) -> Option<i64> {
    let name = SearchField::new(document.name.as_str());
    let description = SearchField::new(document.description.as_str());
    let tags = SearchField::new(document.tags.join(" ").as_str());
    let plugin = SearchField::new(document.plugin_id.as_deref().unwrap_or(""));
    let compact_query = normalized_query.replace(' ', "");

    let name_phrase = !normalized_query.is_empty() && name.normalized.contains(normalized_query);
    let description_phrase =
        !normalized_query.is_empty() && description.normalized.contains(normalized_query);
    let compact_name_match = compact_query.chars().count() >= 3
        && compact_query_matches_name_tokens(compact_query.as_str(), name.tokens.as_slice());

    let mut matches = Vec::with_capacity(query_terms.len());
    let mut matched_action_terms = 0usize;
    let action_term_count = query_terms
        .iter()
        .filter(|term| is_action_term(term.as_str()))
        .count();
    for term in query_terms {
        let matched = best_term_match(
            term,
            &name,
            &tags,
            &plugin,
            &description,
            compact_name_match && query_terms.len() == 1,
        );
        if is_action_term(term.as_str()) && matched.is_some_and(|matched| matched.quality >= 82) {
            matched_action_terms = matched_action_terms.saturating_add(1);
        }
        matches.push(matched);
    }

    // An unmatched action changes what a tool does, not merely how narrowly
    // the user described it. For example, `monitor.start` must not be returned
    // for "stop background monitor" just because two nouns overlap.
    if matched_action_terms < action_term_count {
        return None;
    }

    let matched = matches.iter().flatten().count();
    let strong = matches
        .iter()
        .flatten()
        .filter(|matched| matched.quality >= 88)
        .count();
    let anchored = matches.iter().flatten().any(|matched| matched.anchored);
    let accepted = match query_terms.len() {
        0 => false,
        1 => matches[0].is_some_and(|matched| {
            matched.quality >= 88 || (matched.anchored && matched.quality >= 72)
        }),
        2 => matched == 2 && strong >= 1,
        3 => matched >= 2 && strong >= 2 && anchored,
        term_count => {
            let required = term_count.saturating_mul(3).div_ceil(4);
            matched >= required && strong >= 2 && (anchored || description_phrase)
        }
    };
    if !accepted {
        return None;
    }

    let unmatched = query_terms.len().saturating_sub(matched);
    let mut score = matches
        .iter()
        .flatten()
        .map(|matched| i64::from(matched.quality) * 10 + i64::from(matched.field_weight))
        .sum::<i64>();
    score += i64::try_from(matched)
        .unwrap_or(i64::MAX)
        .saturating_mul(180);
    score += i64::try_from(strong).unwrap_or(i64::MAX).saturating_mul(90);
    score -= i64::try_from(unmatched)
        .unwrap_or(i64::MAX)
        .saturating_mul(260);
    if name_phrase {
        score += 450;
    }
    if compact_name_match {
        score += 400;
    }
    if description_phrase {
        score += 180;
    }
    if anchored {
        score += 120;
    }
    Some(score)
}

fn compact_query_matches_name_tokens(query: &str, name_tokens: &[String]) -> bool {
    for start in 0..name_tokens.len() {
        let mut compact = String::new();
        for token in &name_tokens[start..] {
            compact.push_str(token);
            if compact == query {
                return true;
            }
            if compact.len() >= query.len() {
                break;
            }
        }
    }
    false
}

fn best_term_match(
    term: &str,
    name: &SearchField,
    tags: &SearchField,
    plugin: &SearchField,
    description: &SearchField,
    compact_name_match: bool,
) -> Option<TermMatch> {
    let mut best = compact_name_match.then_some(TermMatch {
        quality: 96,
        field_weight: 80,
        anchored: true,
    });
    for (field, field_weight, anchored) in [
        (name, 70, true),
        (tags, 50, true),
        (plugin, 40, true),
        (description, 0, false),
    ] {
        for candidate in &field.tokens {
            let Some(quality) = token_match_quality(term, candidate.as_str()) else {
                continue;
            };
            let matched = TermMatch {
                quality,
                field_weight,
                anchored,
            };
            if best.is_none_or(|current| {
                (matched.quality, matched.field_weight) > (current.quality, current.field_weight)
            }) {
                best = Some(matched);
            }
        }
    }
    best
}

fn token_match_quality(query: &str, candidate: &str) -> Option<u16> {
    if query == candidate {
        return Some(100);
    }
    let query_len = query.chars().count();
    let candidate_len = candidate.chars().count();
    if query_len >= 3 && is_inflectional_extension(query, candidate) {
        return Some(92);
    }
    if candidate_len >= 2 && is_inflectional_extension(candidate, query) {
        return Some(88);
    }
    if query_len.min(candidate_len) >= 6
        && query_len.abs_diff(candidate_len) <= 5
        && common_prefix_len(query, candidate) >= 5
    {
        return Some(86);
    }

    let length_delta = query_len.abs_diff(candidate_len);
    let max_distance = match query_len.max(candidate_len) {
        0..=1 => 0,
        2..=6 => 1,
        7..=12 => 2,
        _ => 3,
    };
    if length_delta > max_distance {
        return None;
    }
    let distance = edit_distance(query, candidate);
    if distance > max_distance {
        return None;
    }
    if distance > 0
        && (query.chars().next() != candidate.chars().next()
            || query.chars().last() != candidate.chars().last())
    {
        return None;
    }
    Some(
        84_u16.saturating_sub(
            u16::try_from(distance)
                .unwrap_or(u16::MAX)
                .saturating_mul(6),
        ),
    )
}

fn is_inflectional_extension(base: &str, extended: &str) -> bool {
    if extended.strip_prefix(base).is_some_and(is_word_suffix) {
        return true;
    }
    if let Some(last) = base.chars().last() {
        let doubled_ing = format!("{last}ing");
        let doubled_ed = format!("{last}ed");
        if extended
            .strip_prefix(base)
            .is_some_and(|suffix| suffix == doubled_ing.as_str() || suffix == doubled_ed.as_str())
        {
            return true;
        }
    }
    base.strip_suffix('e').is_some_and(|stem| {
        extended
            .strip_prefix(stem)
            .is_some_and(|suffix| matches!(suffix, "ing" | "ion" | "ions"))
    })
}

fn is_word_suffix(suffix: &str) -> bool {
    matches!(
        suffix,
        "d" | "ed" | "er" | "ers" | "es" | "ing" | "ion" | "ions" | "ly" | "ment" | "ments" | "s"
    )
}

fn common_prefix_len(left: &str, right: &str) -> usize {
    left.chars()
        .zip(right.chars())
        .take_while(|(left, right)| left == right)
        .count()
}

fn meaningful_query_terms(normalized_query: &str) -> Vec<String> {
    let mut terms = Vec::new();
    for term in normalized_query.split_whitespace() {
        if is_query_filler(term) {
            continue;
        }
        if !terms.iter().any(|existing| existing == term) {
            terms.push(term.to_owned());
        }
    }
    terms
}

fn is_query_filler(term: &str) -> bool {
    matches!(
        term,
        "a" | "an"
            | "and"
            | "available"
            | "current"
            | "do"
            | "for"
            | "in"
            | "latest"
            | "my"
            | "of"
            | "on"
            | "one"
            | "or"
            | "please"
            | "the"
            | "this"
            | "to"
            | "with"
    )
}

fn is_action_term(term: &str) -> bool {
    ACTION_TERMS
        .iter()
        .any(|action| term == *action || is_inflectional_extension(action, term))
}

const ACTION_TERMS: &[&str] = &[
    "analyze",
    "apply",
    "cancel",
    "check",
    "close",
    "convert",
    "copy",
    "create",
    "delete",
    "disable",
    "download",
    "edit",
    "enable",
    "execute",
    "fetch",
    "find",
    "generate",
    "inspect",
    "install",
    "list",
    "move",
    "open",
    "patch",
    "pause",
    "publish",
    "query",
    "read",
    "remove",
    "rename",
    "render",
    "resume",
    "run",
    "search",
    "send",
    "start",
    "stop",
    "subscribe",
    "uninstall",
    "update",
    "upload",
    "wait",
    "watch",
    "write",
];

fn edit_distance(left: &str, right: &str) -> usize {
    let left_chars = left.chars().collect::<Vec<_>>();
    let right_chars = right.chars().collect::<Vec<_>>();
    let mut previous = (0..=right_chars.len()).collect::<Vec<_>>();
    let mut current = vec![0; right_chars.len() + 1];
    for (left_index, left_character) in left_chars.iter().enumerate() {
        current[0] = left_index + 1;
        for (right_index, right_character) in right_chars.iter().enumerate() {
            let replace = previous[right_index] + usize::from(left_character != right_character);
            let insert = current[right_index] + 1;
            let delete = previous[right_index + 1] + 1;
            current[right_index + 1] = replace.min(insert.min(delete));
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[right_chars.len()]
}

/// Normalize punctuation to token boundaries for exact-name and lexical
/// comparisons. Case-insensitive and whitespace-collapsed:
/// `monitor.start`, `Monitor.Start`, and `monitor . start` compare equally.
fn normalize_search_text(text: &str) -> String {
    text.chars()
        .flat_map(char::to_lowercase)
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
}
