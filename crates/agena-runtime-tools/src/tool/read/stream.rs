//! Bounded-memory line paging. The size of the whole file does not determine
//! whether a small page can be read; CPU/scan/output budgets remain explicit.
use super::*;
use std::{
    io::BufRead,
    time::{Duration, Instant},
};
const MAX_SCAN_BYTES: usize = 256 * 1024 * 1024;
const MAX_PAGE_BYTES: usize = 64 * 1024;
const PREFIX_BYTES: usize = MAX_LINE_CHARS * 4 + 4;

pub(super) struct Page {
    pub preview: String,
    pub returned: usize,
    pub next_offset: Option<usize>,
    pub total_lines: Option<usize>,
    pub truncated_lines: bool,
    pub scanned_bytes: usize,
    pub source_bytes: u64,
    pub modified_ns: Option<u128>,
}
pub(super) fn read_page(
    path: &std::path::Path,
    offset: usize,
    limit: usize,
    cancel: Option<&tokio_util::sync::CancellationToken>,
) -> Result<Page, ToolError> {
    let file = fs::File::open(path)?;
    let before = file.metadata()?;
    if !before.is_file() {
        return Err(ToolError::invalid_input(
            "read target must be a regular file",
        ));
    }
    let mut reader = std::io::BufReader::with_capacity(64 * 1024, file);
    let mut prefix = Vec::with_capacity(PREFIX_BYTES);
    let mut line = 1usize;
    let mut line_bytes = 0usize;
    let mut preview = String::new();
    let mut returned = 0usize;
    let mut scanned = 0usize;
    let mut clipped = false;
    let deadline = Instant::now() + Duration::from_secs(5);
    let (next, total) = loop {
        if cancel.is_some_and(|token| token.is_cancelled()) {
            return Err(ToolError::Cancelled);
        }
        if Instant::now() >= deadline || scanned >= MAX_SCAN_BYTES {
            return Err(ToolError::invalid_input(
                "requested line range exceeds the 256 MiB / 5 second scan budget; narrow the range or use a byte-oriented tool",
            ));
        }
        let buffer = reader.fill_buf()?;
        let eof = buffer.is_empty();
        if eof && line_bytes == 0 {
            break (None, Some(line - 1));
        }
        let length = buffer
            .iter()
            .position(|byte| *byte == b'\n')
            .map(|i| i + 1)
            .unwrap_or(buffer.len())
            .min(MAX_SCAN_BYTES - scanned);
        let complete = eof || buffer.get(length.wrapping_sub(1)) == Some(&b'\n');
        if line >= offset {
            prefix.extend_from_slice(&buffer[..length.min(PREFIX_BYTES - prefix.len())]);
        }
        scanned += length;
        line_bytes += length;
        reader.consume(length);
        if !complete {
            continue;
        }
        if line >= offset {
            let text = match std::str::from_utf8(&prefix) {
                Ok(text) => text,
                Err(error) if line_bytes > prefix.len() && error.error_len().is_none() => {
                    std::str::from_utf8(&prefix[..error.valid_up_to()])
                        .expect("verified UTF-8 prefix")
                }
                Err(_) => {
                    return Err(ToolError::invalid_input(
                        "selected file range is not UTF-8; use attachment mode for binary content",
                    ));
                }
            };
            let text = text
                .strip_suffix('\n')
                .unwrap_or(text)
                .strip_suffix('\r')
                .unwrap_or(text.strip_suffix('\n').unwrap_or(text));
            let shortened = truncate_line_chars(text);
            let line_clipped = line_bytes > prefix.len() || text.chars().count() > MAX_LINE_CHARS;
            let row = format!(
                "{line}: {shortened}{}",
                if line_bytes > prefix.len() && !shortened.ends_with('…') {
                    "…"
                } else {
                    ""
                }
            );
            if !preview.is_empty() && preview.len() + row.len() + 1 > MAX_PAGE_BYTES {
                break (Some(line), None);
            }
            if !preview.is_empty() {
                preview.push('\n');
            }
            preview.push_str(&row);
            returned += 1;
            clipped |= line_clipped;
            if returned >= limit.min(2000) {
                let more = !reader.fill_buf()?.is_empty();
                break (more.then_some(line + 1), (!more).then_some(line));
            }
        }
        line += 1;
        line_bytes = 0;
        prefix.clear();
        if eof {
            break (None, Some(line - 1));
        }
    };
    if returned == 0 && total.is_some_and(|count| count > 0 && offset > count) {
        return Err(ToolError::invalid_input(format!(
            "read offset {offset} exceeds file line count {}",
            total.unwrap()
        )));
    }
    let after = reader.get_ref().metadata()?;
    if before.len() != after.len() || before.modified().ok() != after.modified().ok() {
        return Err(ToolError::invalid_input(
            "file changed during read; retry for a consistent page",
        ));
    }
    if let Some(next) = next {
        preview.push_str(&format!(
            "\n[More content: continue fs.read with offset={next}.]"
        ));
    }
    if clipped {
        preview.push_str(
            "\n[Long lines were shortened in this preview; this is not the complete byte content.]",
        );
    }
    Ok(Page {
        preview,
        returned,
        next_offset: next,
        total_lines: total,
        truncated_lines: clipped,
        scanned_bytes: scanned,
        source_bytes: before.len(),
        modified_ns: before
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duration| duration.as_nanos()),
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    #[test]
    fn small_page_from_large_file_does_not_load_the_whole_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("large.txt");
        let mut file = fs::File::create(&path).unwrap();
        file.write_all(b"hello\nworld\n").unwrap();
        file.set_len(64 * 1024 * 1024).unwrap();
        let page = read_page(&path, 1, 1, None).unwrap();
        assert!(page.preview.starts_with("1: hello"));
        assert_eq!(page.next_offset, Some(2));
        assert_eq!(page.total_lines, None);
        assert!(page.scanned_bytes < 100);
    }
    #[test]
    fn huge_line_and_unicode_are_bounded_and_can_continue() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("long.txt");
        fs::write(&path, format!("{}\nnext\n", "中😀".repeat(100000))).unwrap();
        let page = read_page(&path, 1, 1, None).unwrap();
        assert!(page.truncated_lines);
        assert!(page.preview.len() < 10000);
        assert_eq!(page.next_offset, Some(2));
        let next = read_page(&path, 2, 5, None).unwrap();
        assert_eq!(next.preview, "2: next");
        assert_eq!(next.total_lines, Some(2));
    }
    #[test]
    fn cancelled_read_does_not_scan() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("file.txt");
        fs::write(&path, "line\n").unwrap();
        let cancel = tokio_util::sync::CancellationToken::new();
        cancel.cancel();
        assert!(matches!(
            read_page(&path, 1, 1, Some(&cancel)),
            Err(ToolError::Cancelled)
        ));
    }
}
