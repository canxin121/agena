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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unicode_continuations_reconstruct_exact_content_and_validate_bounds() {
        let original = "## 中文\n\n 🦀 e\u{301}  \n```rust\n    x\n```";
        let mut all = String::new();
        let mut offset = 0;
        loop {
            let slice = page_slice(original, offset, 3).unwrap();
            all.push_str(&slice.markdown);
            match slice.next_offset {
                Some(next) => offset = next,
                None => break,
            }
        }
        assert_eq!(all, original);
        assert!(page_slice(original, original.chars().count() + 1, 3).is_err());
        assert!(page_slice(original, 0, 0).is_err());
        assert!(page_slice(original, 0, 24_001).is_err());
        assert_eq!(page_slice("", 0, 3).unwrap().next_offset, None);
    }

    #[tokio::test]
    async fn refreshed_content_does_not_mutate_old_snapshot_or_provenance() {
        let snapshots = PageSnapshots::default();
        let url = url::Url::parse("https://example.test/").unwrap();
        let page = crate::extract_page_from_body(
            &url,
            &url,
            "text/plain",
            200,
            false,
            false,
            "old",
            None,
            None,
        );
        let old = snapshots.insert(page.clone()).await.unwrap();
        let mut next = page.clone();
        next.markdown = "new".into();
        let new = snapshots.insert(next).await.unwrap();
        assert_ne!(old, new);
        let mut elsewhere = page;
        elsewhere.final_url = "https://elsewhere.test/".into();
        assert_ne!(old, snapshots.insert(elsewhere).await.unwrap());
        assert_eq!(snapshots.get(&old).await.unwrap().markdown, "old");
        assert!(snapshots.get("unknown").await.is_err());
    }
}
