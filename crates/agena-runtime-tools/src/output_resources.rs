//! Owner-scoped recovery of captured tool output. Bounded runtime cache, not
//! durable transcript storage. Expired/evicted data is never reconstructed.
use crate::TerminalOwner;
use serde::Serialize;
use std::{
    collections::VecDeque,
    path::Path,
    sync::{Arc, LazyLock, Mutex},
    time::{Duration, Instant},
};
const MAX_ENTRY: usize = 8 * 1024 * 1024;
const MAX_TOTAL: usize = 64 * 1024 * 1024;
const MAX_ENTRIES: usize = 256;
const TTL: Duration = Duration::from_secs(3600);
#[derive(Clone)]
struct Entry {
    id: String,
    owner: TerminalOwner,
    text: Arc<str>,
    original_bytes: usize,
    created: Instant,
}
#[derive(Default)]
struct Cache {
    entries: VecDeque<Entry>,
    bytes: usize,
}
static CACHE: LazyLock<Mutex<Cache>> = LazyLock::new(|| Mutex::new(Cache::default()));
#[derive(Debug, Serialize)]
pub struct OutputRead {
    pub output_id: String,
    pub text: String,
    pub next_offset: Option<usize>,
    pub captured_bytes: usize,
    pub original_bytes: usize,
    pub capture_truncated: bool,
}
#[derive(Debug, Serialize)]
pub struct OutputMatch {
    pub offset: usize,
    pub preview: String,
}
#[derive(Debug, Serialize)]
pub struct OutputSearch {
    pub output_id: String,
    pub matches: Vec<OutputMatch>,
    pub next_offset: Option<usize>,
    pub capture_truncated: bool,
}
fn boundary(text: &str, end: usize) -> usize {
    let mut end = end.min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    end
}
fn owner(root: &Path, session: i64) -> Result<TerminalOwner, String> {
    Ok(TerminalOwner {
        workspace: root.canonicalize().map_err(|e| e.to_string())?,
        session_id: Some(session),
    })
}
fn prune(cache: &mut Cache) {
    cache.entries.retain(|entry| entry.created.elapsed() <= TTL);
    cache.bytes = cache.entries.iter().map(|entry| entry.text.len()).sum();
}

pub fn capture(root: &Path, session: i64, text: &str) -> Result<String, String> {
    let owner = owner(root, session)?;
    let length = boundary(text, MAX_ENTRY);
    let entry = Entry {
        id: format!("out_{}", uuid::Uuid::new_v4().simple()),
        owner,
        text: Arc::from(&text[..length]),
        original_bytes: text.len(),
        created: Instant::now(),
    };
    let id = entry.id.clone();
    let mut cache = CACHE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    prune(&mut cache);
    while cache.entries.len() >= MAX_ENTRIES || cache.bytes + entry.text.len() > MAX_TOTAL {
        if let Some(old) = cache.entries.pop_front() {
            cache.bytes -= old.text.len();
        } else {
            break;
        }
    }
    cache.bytes += entry.text.len();
    cache.entries.push_back(entry);
    Ok(id)
}
fn lookup(root: &Path, session: i64, id: &str) -> Result<Entry, String> {
    let owner = owner(root, session)?;
    let mut cache = CACHE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    prune(&mut cache);
    cache.entries.iter().find(|entry|entry.id==id && entry.owner==owner).cloned().ok_or_else(||"output not found, not owned, or expired/evicted; retained transcript may still exist but this cache cannot recover it".into())
}
pub fn read(
    root: &Path,
    session: i64,
    id: &str,
    offset: usize,
    limit: usize,
) -> Result<OutputRead, String> {
    if !(1..=16000).contains(&limit) {
        return Err("limit must be 1..16000 bytes".into());
    }
    let entry = lookup(root, session, id)?;
    if offset > entry.text.len() || !entry.text.is_char_boundary(offset) {
        return Err("offset is outside capture or not at a UTF-8 boundary".into());
    }
    let mut end = boundary(&entry.text, offset.saturating_add(limit));
    if end == offset && offset < entry.text.len() {
        end = offset + entry.text[offset..].chars().next().unwrap().len_utf8();
    }
    Ok(OutputRead {
        output_id: id.into(),
        text: entry.text[offset..end].into(),
        next_offset: (end < entry.text.len()).then_some(end),
        captured_bytes: entry.text.len(),
        original_bytes: entry.original_bytes,
        capture_truncated: entry.original_bytes > entry.text.len(),
    })
}
pub fn search(
    root: &Path,
    session: i64,
    id: &str,
    pattern: &str,
    offset: usize,
    limit: usize,
) -> Result<OutputSearch, String> {
    if pattern.is_empty() || pattern.len() > 4096 || !(1..=100).contains(&limit) {
        return Err("nonempty literal pattern (<=4096 bytes) and limit 1..100 required".into());
    }
    let entry = lookup(root, session, id)?;
    if offset > entry.text.len() || !entry.text.is_char_boundary(offset) {
        return Err("invalid UTF-8 byte offset".into());
    }
    let mut matches = Vec::new();
    let mut cursor = offset;
    let mut more = None;
    while let Some(found) = entry.text[cursor..].find(pattern) {
        let position = cursor + found;
        if matches.len() == limit {
            more = Some(position);
            break;
        }
        let start = entry.text[..position]
            .rfind('\n')
            .map(|index| index + 1)
            .unwrap_or(0);
        let end = entry.text[position..]
            .find('\n')
            .map(|index| position + index)
            .unwrap_or(entry.text.len());
        let end = boundary(&entry.text, end.min(position + 2048));
        let start = start.max(boundary(&entry.text, position.saturating_sub(128)));
        matches.push(OutputMatch {
            offset: position,
            preview: entry.text[start..end].into(),
        });
        cursor = position + pattern.len();
        if cursor >= entry.text.len() {
            break;
        }
    }
    Ok(OutputSearch {
        output_id: id.into(),
        matches,
        next_offset: more,
        capture_truncated: entry.text.len() < entry.original_bytes,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn middle_errors_can_be_searched_and_read_but_not_by_other_owners() {
        let dir = tempfile::tempdir().unwrap();
        let text = format!(
            "{}\nUNIQUE_MIDDLE_ERROR\n{}",
            "head\n".repeat(5000),
            "tail\n".repeat(5000)
        );
        let id = capture(dir.path(), 41, &text).unwrap();
        let hits = search(dir.path(), 41, &id, "UNIQUE_MIDDLE_ERROR", 0, 10).unwrap();
        assert_eq!(hits.matches.len(), 1);
        assert!(
            read(dir.path(), 41, &id, hits.matches[0].offset, 200)
                .unwrap()
                .text
                .starts_with("UNIQUE_MIDDLE_ERROR")
        );
        assert!(read(dir.path(), 42, &id, 0, 100).is_err());
        assert!(search(dir.path(), 42, &id, "ERROR", 0, 10).is_err());
    }
    #[test]
    fn truncation_and_utf8_boundaries_are_explicit() {
        let dir = tempfile::tempdir().unwrap();
        let text = "中".repeat(MAX_ENTRY / 3 + 100);
        let id = capture(dir.path(), 41, &text).unwrap();
        let page = read(dir.path(), 41, &id, 0, 1).unwrap();
        assert_eq!(page.text, "中");
        assert!(page.capture_truncated);
        assert!(read(dir.path(), 41, &id, 1, 10).is_err());
    }
}
