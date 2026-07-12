use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use text_splitter::MarkdownSplitter;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FetchedPage {
    pub url: String,
    pub canonical_url: String,
    pub title: String,
    pub markdown: String,
    pub content_type: String,
    pub status: u16,
    pub truncated: bool,
    #[serde(default)]
    pub rendered: bool,
    pub raw_html_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub etag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_modified: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StoredDocument {
    pub id: String,
    pub url: String,
    pub canonical_url: String,
    pub title: String,
    pub markdown: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub chunks: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub chunk_hashes: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<String>,
    pub content_type: String,
    pub status: u16,
    pub truncated: bool,
    #[serde(default)]
    pub rendered: bool,
    pub hash: String,
    pub raw_html_hash: String,
    pub markdown_hash: String,
    pub simhash: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub etag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_modified: Option<String>,
    pub depth: u32,
    pub fetched_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CrawlDocumentSummary {
    pub id: String,
    pub url: String,
    pub title: String,
    pub depth: u32,
    pub fetched_at: DateTime<Utc>,
    pub chunk_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CrawlSearchHit {
    pub id: String,
    pub url: String,
    pub title: String,
    pub chunk_index: u32,
    pub preview: String,
    pub score: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lexical_score: Option<f32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub match_sources: Vec<String>,
}

impl StoredDocument {
    pub fn from_fetched_page(page: FetchedPage, depth: u32, max_chunk_chars: usize) -> Self {
        let markdown_hash = blake3::hash(page.markdown.as_bytes()).to_hex().to_string();
        let hash = markdown_hash.clone();
        let id = blake3::hash(page.canonical_url.as_bytes())
            .to_hex()
            .to_string();
        let chunks = chunk_markdown(page.markdown.as_str(), max_chunk_chars);
        let chunk_hashes = chunks
            .iter()
            .map(|chunk| blake3::hash(chunk.as_bytes()).to_hex().to_string())
            .collect::<Vec<_>>();
        Self {
            id,
            url: page.url,
            canonical_url: page.canonical_url,
            title: page.title,
            markdown: page.markdown.clone(),
            chunks,
            chunk_hashes,
            links: page.links,
            content_type: page.content_type,
            status: page.status,
            truncated: page.truncated,
            rendered: page.rendered,
            hash,
            raw_html_hash: page.raw_html_hash,
            markdown_hash,
            simhash: simhash::simhash(page.markdown.as_str()),
            etag: page.etag,
            last_modified: page.last_modified,
            depth,
            fetched_at: Utc::now(),
        }
    }

    pub fn summary(&self) -> CrawlDocumentSummary {
        CrawlDocumentSummary {
            id: self.id.clone(),
            url: self.canonical_url.clone(),
            title: self.title.clone(),
            depth: self.depth,
            fetched_at: self.fetched_at,
            chunk_count: self.chunks.len(),
        }
    }
}

pub fn chunk_markdown(markdown: &str, max_chunk_chars: usize) -> Vec<String> {
    let limit = max_chunk_chars.max(400);
    let splitter = MarkdownSplitter::new(limit);
    let mut chunks = splitter
        .chunks(markdown)
        .map(str::trim)
        .filter(|chunk| !chunk.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();

    if chunks.is_empty() && !markdown.trim().is_empty() {
        chunks.push(preview_text(markdown.trim(), limit));
    }

    chunks
}

pub fn preview_text(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let mut end = 0usize;
    for (count, (idx, ch)) in text.char_indices().enumerate() {
        if count == max_chars {
            break;
        }
        end = idx + ch.len_utf8();
    }
    format!("{}…", text[..end].trim_end())
}
