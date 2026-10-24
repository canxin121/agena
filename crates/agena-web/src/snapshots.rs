//! Bounded immutable page snapshots for continuation without a second fetch.
use std::{sync::Arc, time::Duration};

use moka::{future::Cache, policy::EvictionPolicy};
use serde::Serialize;

use crate::{CrawlError, FetchedPage};

pub struct PageSnapshots {
    pages: Cache<String, Arc<FetchedPage>>,
}

impl Default for PageSnapshots {
    fn default() -> Self {
        Self {
            pages: Cache::builder()
                .time_to_live(Duration::from_secs(15 * 60))
                .max_capacity(32 * 1024 * 1024)
                .eviction_policy(EvictionPolicy::lru())
                .weigher(|_: &String, page: &Arc<FetchedPage>| {
                    let bytes = page.markdown.len()
                        + page.title.len()
                        + page.url.len()
                        + page.final_url.len()
                        + page.canonical_url.len()
                        + page.links.iter().map(|s| s.len() + 24).sum::<usize>()
                        + page.warnings.iter().map(String::len).sum::<usize>()
                        + 1024;
                    bytes.clamp(128 * 1024, u32::MAX as usize) as u32
                })
                .build(),
        }
    }
}

impl PageSnapshots {
    pub async fn insert(&self, page: FetchedPage) -> Result<String, CrawlError> {
        // Include provenance/status as well as text: identical text from two
        // origins must never overwrite the permission boundary of a snapshot.
        let bytes = serde_json::to_vec(&page)?;
        let id = blake3::hash(&bytes).to_hex().to_string();
        self.pages.insert(id.clone(), Arc::new(page)).await;
        Ok(id)
    }

    /// The caller must reauthorize both requested and final URLs before
    /// exposing any snapshot content, even though this read makes no requests.
    pub async fn get(&self, id: &str) -> Result<Arc<FetchedPage>, CrawlError> {
        self.pages.get(id).await.ok_or_else(|| CrawlError::NotFound(
            "Page snapshot expired or was evicted; fetch the URL or query the crawl index again.".into()
        ))
    }
}

#[derive(Debug, Serialize)]
pub struct PageSlice {
    pub markdown: String,
    pub offset: usize,
    pub returned_chars: usize,
    pub total_chars: usize,
    pub next_offset: Option<usize>,
}

/// Offsets count Unicode scalar values, not UTF-8 bytes. No trimming or
/// ellipsis is inserted, so consecutive slices reproduce the exact snapshot.
pub fn page_slice(
    markdown: &str,
    offset: usize,
    max_chars: usize,
) -> Result<PageSlice, CrawlError> {
    if !(1..=24_000).contains(&max_chars) {
        return Err(CrawlError::InvalidInput(
            "max_chars must be between 1 and 24000".into(),
        ));
    }
    let total_chars = markdown.chars().count();
    if offset > total_chars {
        return Err(CrawlError::InvalidInput(format!(
            "offset exceeds the {total_chars} available characters"
        )));
    }
    let text: String = markdown.chars().skip(offset).take(max_chars).collect();
    let returned_chars = text.chars().count();
    let end = offset + returned_chars;
    Ok(PageSlice {
        markdown: text,
        offset,
        returned_chars,
        total_chars,
        next_offset: (end < total_chars).then_some(end),
    })
}
