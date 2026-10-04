use super::{GrepMode, GrepRecord, MAX_LINE_BYTES, StopReason, ToolError};
use grep_regex::RegexMatcher;
use grep_searcher::{Searcher, Sink, SinkContext, SinkMatch};
use std::{
    io::{self, Read},
    path::Path,
    time::Instant,
};
use tokio_util::sync::CancellationToken;

pub(super) struct FileRequest<'a> {
    pub path: &'a Path,
    pub display_path: &'a str,
    pub mode: GrepMode,
    pub remaining_results: usize,
    pub remaining_output: usize,
    pub remaining_bytes: u64,
    pub max_file_bytes: u64,
    pub deadline: Instant,
    pub cancel: Option<&'a CancellationToken>,
}
#[derive(Debug, Default)]
pub(super) struct FileResult {
    pub records: Vec<GrepRecord>,
    pub returned_matches: usize,
    pub output_bytes: usize,
    pub bytes_read: u64,
    pub shortened_lines: usize,
    pub too_large: bool,
    pub io_error: bool,
    pub changed: bool,
    pub binary: bool,
    pub stop_reason: Option<StopReason>,
}
struct BoundedReader<'a> {
    file: std::fs::File,
    request: &'a FileRequest<'a>,
    bytes_read: u64,
    hit_limit: bool,
}
impl Read for BoundedReader<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        check_active(self.request)?;
        if buf.is_empty() {
            return Ok(0);
        }
        let budget = self
            .request
            .remaining_bytes
            .min(self.request.max_file_bytes);
        let remaining = budget.saturating_sub(self.bytes_read);
        // One probe byte distinguishes exact EOF from growth beyond a budget.
        let length = buf
            .len()
            .min(64 * 1024)
            .min(remaining.saturating_add(1) as usize);
        let read = self.file.read(&mut buf[..length])?;
        self.bytes_read += (read as u64).min(remaining);
        if read as u64 > remaining {
            self.hit_limit = true;
            return Err(io::Error::other("grep byte budget exceeded"));
        }
        Ok(read)
    }
}
fn check_active(request: &FileRequest<'_>) -> io::Result<()> {
    if request.cancel.is_some_and(CancellationToken::is_cancelled) {
        // Interrupted is retried by the searcher, so use a terminal error kind.
        return Err(io::Error::other("grep cancelled"));
    }
    if Instant::now() >= request.deadline {
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "grep deadline reached",
        ));
    }
    Ok(())
}
struct Collector<'a> {
    request: &'a FileRequest<'a>,
    result: FileResult,
    matching_lines: u64,
    last_match_line: Option<u64>,
}
impl Collector<'_> {
    fn push(&mut self, record: GrepRecord) -> bool {
        // Budget serialized bytes including escaping.
        let bytes = serde_json::to_vec(&record)
            .expect("grep record serializes")
            .len()
            + 1;
        if self.result.output_bytes + bytes > self.request.remaining_output {
            self.result.stop_reason = Some(StopReason::Output);
            return false;
        }
        self.result.output_bytes += bytes;
        if matches!(
            record,
            GrepRecord::Line {
                text_truncated: true,
                ..
            }
        ) {
            self.result.shortened_lines += 1;
        }
        self.result.records.push(record);
        true
    }
    fn line(&mut self, number: u64, bytes: &[u8], matched: bool) -> bool {
        let bytes = bytes.strip_suffix(b"\n").unwrap_or(bytes);
        let bytes = bytes.strip_suffix(b"\r").unwrap_or(bytes);
        let prefix = &bytes[..bytes.len().min(MAX_LINE_BYTES)];
        let prefix = match std::str::from_utf8(prefix) {
            Err(error) if error.error_len().is_none() && prefix.len() < bytes.len() => {
                &prefix[..error.valid_up_to()]
            }
            _ => prefix,
        };
        let text = String::from_utf8_lossy(prefix);
        let mut end = text.len().min(MAX_LINE_BYTES);
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        self.push(GrepRecord::Line {
            path: self.request.display_path.to_owned(),
            line: number,
            text: text[..end].to_owned(),
            matched,
            text_truncated: prefix.len() < bytes.len() || end < text.len(),
        })
    }
}
impl Sink for Collector<'_> {
    type Error = io::Error;
    fn matched(&mut self, _: &Searcher, mat: &SinkMatch<'_>) -> io::Result<bool> {
        check_active(self.request)?;
        if self.request.remaining_results == 0
            || (self.request.mode == GrepMode::Content
                && self.result.returned_matches >= self.request.remaining_results)
        {
            self.result.stop_reason = Some(StopReason::Results);
            return Ok(false);
        }
        self.matching_lines += 1;
        match self.request.mode {
            GrepMode::Content => {
                let line = mat.line_number().expect("line numbers enabled");
                self.last_match_line = Some(line);
                if !self.line(line, mat.bytes(), true) {
                    return Ok(false);
                }
                self.result.returned_matches += 1;
                Ok(true)
            }
            GrepMode::Files => {
                if self.push(GrepRecord::File {
                    path: self.request.display_path.into(),
                }) {
                    self.result.returned_matches = 1;
                }
                Ok(false)
            }
            GrepMode::Count => Ok(true),
        }
    }
    fn context(&mut self, searcher: &Searcher, context: &SinkContext<'_>) -> io::Result<bool> {
        check_active(self.request)?;
        let number = context.line_number().expect("line numbers enabled");
        // No orphan before-context for an unreturned additional match.
        if self.result.returned_matches >= self.request.remaining_results
            && self
                .last_match_line
                .is_none_or(|last| number > last + searcher.after_context() as u64)
        {
            return Ok(true);
        }
        Ok(self.line(number, context.bytes(), false))
    }
    fn binary_data(&mut self, _: &Searcher, _: u64) -> io::Result<bool> {
        self.result.binary = true;
        Ok(false)
    }
}

pub(super) fn search_file(
    searcher: &mut Searcher,
    matcher: &RegexMatcher,
    request: FileRequest<'_>,
) -> Result<FileResult, ToolError> {
    if request.cancel.is_some_and(CancellationToken::is_cancelled) {
        return Err(ToolError::Cancelled);
    }
    if Instant::now() >= request.deadline {
        return Ok(FileResult {
            stop_reason: Some(StopReason::Deadline),
            ..FileResult::default()
        });
    }
    // Nonblocking open closes the FIFO race. Explicit targets are resolved by
    // the executor; directory discovery never follows final symlinks.
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW);
    }
    let file = match options.open(request.path) {
        Ok(file) => file,
        Err(error) => {
            tracing::debug!(path = %request.path.display(), %error, "grep cannot open file");
            return Ok(FileResult {
                io_error: true,
                ..FileResult::default()
            });
        }
    };
    let before = match file.metadata() {
        Ok(metadata) => metadata,
        Err(error) => {
            tracing::debug!(path = %request.path.display(), %error, "grep cannot inspect opened file");
            return Ok(FileResult {
                io_error: true,
                ..FileResult::default()
            });
        }
    };
    if !before.is_file() {
        return Ok(FileResult {
            io_error: true,
            ..FileResult::default()
        });
    }
    if before.len() > request.max_file_bytes {
        return Ok(FileResult {
            too_large: true,
            ..FileResult::default()
        });
    }
    if before.len() > request.remaining_bytes {
        return Ok(FileResult {
            stop_reason: Some(StopReason::Bytes),
            ..FileResult::default()
        });
    }
    let mut reader = BoundedReader {
        file,
        request: &request,
        bytes_read: 0,
        hit_limit: false,
    };
    let mut sink = Collector {
        request: &request,
        result: FileResult::default(),
        matching_lines: 0,
        last_match_line: None,
    };
    let outcome = searcher.search_reader(matcher, &mut reader, &mut sink);
    if request.cancel.is_some_and(CancellationToken::is_cancelled) {
        return Err(ToolError::Cancelled);
    }
    let after = reader.file.metadata();
    if let Err(error) = &after {
        tracing::debug!(path = %request.path.display(), %error, "grep cannot recheck opened file");
        sink.result.io_error = true;
    }
    let changed = after.as_ref().is_ok_and(|after| {
        before.len() != after.len() || before.modified().ok() != after.modified().ok()
    });
    if sink.result.binary || changed {
        return Ok(FileResult {
            bytes_read: reader.bytes_read,
            binary: sink.result.binary,
            changed,
            ..FileResult::default()
        });
    }
    if outcome.is_err() {
        if Instant::now() >= request.deadline {
            sink.result.stop_reason = Some(StopReason::Deadline);
        } else if reader.hit_limit {
            if request.remaining_bytes <= request.max_file_bytes {
                sink.result.stop_reason = Some(StopReason::Bytes);
            } else {
                sink.result.too_large = true;
            }
        } else {
            tracing::debug!(path = %request.path.display(), error = ?outcome.err(), "grep file scan incomplete");
            sink.result.io_error = true;
        }
    }
    if request.mode == GrepMode::Count && sink.matching_lines > 0 {
        let complete =
            sink.result.stop_reason.is_none() && !sink.result.io_error && !sink.result.too_large;
        if sink.push(GrepRecord::Count {
            path: request.display_path.into(),
            count: sink.matching_lines,
            complete,
        }) {
            sink.result.returned_matches = 1;
        }
    }
    sink.result.bytes_read = reader.bytes_read;
    Ok(sink.result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    fn request(path: &Path) -> FileRequest<'_> {
        FileRequest {
            path,
            display_path: "fixture",
            mode: GrepMode::Content,
            remaining_results: 500,
            remaining_output: 256 * 1024,
            remaining_bytes: 4,
            max_file_bytes: 4,
            deadline: Instant::now() + std::time::Duration::from_secs(5),
            cancel: None,
        }
    }
    #[test]
    fn reader_checks_cancellation_between_reads_and_detects_growth() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("file");
        std::fs::write(&path, b"abcd").unwrap();
        let cancel = CancellationToken::new();
        let mut req = request(&path);
        req.cancel = Some(&cancel);
        let mut reader = BoundedReader {
            file: std::fs::File::open(&path).unwrap(),
            request: &req,
            bytes_read: 0,
            hit_limit: false,
        };
        let mut buffer = [0; 2];
        assert_eq!(reader.read(&mut buffer).unwrap(), 2);
        cancel.cancel();
        assert!(reader.read(&mut buffer).is_err());
        assert_eq!(reader.bytes_read, 2);
        let req = request(&path);
        let mut reader = BoundedReader {
            file: std::fs::File::open(&path).unwrap(),
            request: &req,
            bytes_read: 0,
            hit_limit: false,
        };
        let mut buffer = [0; 8];
        assert_eq!(reader.read(&mut buffer).unwrap(), 4);
        std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(b"growth")
            .unwrap();
        assert!(reader.read(&mut buffer).is_err());
        assert!(reader.hit_limit);
        assert_eq!(reader.bytes_read, 4);
    }
    #[cfg(unix)]
    #[test]
    fn special_file_open_does_not_wait_for_a_fifo_writer() {
        use std::os::unix::ffi::OsStrExt;
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("pipe");
        let name = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        let mut searcher = grep_searcher::SearcherBuilder::new()
            .line_number(true)
            .build();
        let matcher = grep_regex::RegexMatcher::new("hit").unwrap();
        let started = Instant::now();
        let result = search_file(&mut searcher, &matcher, request(&path)).unwrap();
        assert!(result.io_error);
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
    }
}
