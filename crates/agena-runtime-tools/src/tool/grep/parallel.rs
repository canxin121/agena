//! Sorted discovery with small, ordered batches. File lengths reserve the byte
//! budget before workers start; changed/growing files invalidate their snapshot.
//! A dedicated persistent Rayon pool has at most four workers process-wide;
//! each invocation holds at most sixteen bounded file results per batch.
use super::*;
use grep_regex::RegexMatcher;
use rayon::prelude::*;
use std::{path::PathBuf, sync::OnceLock};

const BATCH_FILES: usize = 16;
static WORKERS: OnceLock<Result<rayon::ThreadPool, String>> = OnceLock::new();

fn workers() -> Result<&'static rayon::ThreadPool, ToolError> {
    WORKERS
        .get_or_init(|| {
            rayon::ThreadPoolBuilder::new()
                .num_threads(4)
                .thread_name(|index| format!("agena-grep-{index}"))
                .build()
                .map_err(|error| error.to_string())
        })
        .as_ref()
        .map_err(|error| ToolError::plugin(format!("cannot start grep workers: {error}")))
}

pub(super) struct Plan<'a> {
    pub base: &'a Path,
    pub workspace: &'a Path,
    pub base_is_file: bool,
    pub input: &'a GrepToolInput,
    pub matcher: &'a RegexMatcher,
    pub include: GlobSet,
    pub exclude: GlobSet,
    pub include_ignored: bool,
    pub cancel: Option<&'a CancellationToken>,
    pub limits: &'a SearchLimits,
}

struct WorkItem {
    path: PathBuf,
    display_path: String,
    length: u64,
}

struct Collected {
    records: Vec<GrepRecord>,
    stats: SearchStats,
    returned: usize,
    bytes: usize,
}

impl Plan<'_> {
    fn next_file(
        &self,
        walker: &mut ignore::Walk,
        stats: &mut SearchStats,
        queued: usize,
        deadline: Instant,
    ) -> Result<Option<WorkItem>, ToolError> {
        loop {
            if self.cancel.is_some_and(CancellationToken::is_cancelled) {
                return Err(ToolError::Cancelled);
            }
            if Instant::now() >= deadline {
                stats.stop_reason = Some(StopReason::Deadline);
                return Ok(None);
            }
            let Some(entry) = walker.next() else {
                return Ok(None);
            };
            if stats.visited_entries >= self.limits.max_visited_entries {
                stats.stop_reason = Some(StopReason::Entries);
                return Ok(None);
            }
            stats.visited_entries += 1;
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    stats.skipped_io_errors += 1;
                    tracing::debug!(%error, "grep skipped unreadable entry");
                    continue;
                }
            };
            if !entry.file_type().is_some_and(|kind| kind.is_file()) {
                continue;
            }
            let relative = if self.base_is_file {
                entry
                    .path()
                    .file_name()
                    .map(Path::new)
                    .unwrap_or(entry.path())
            } else {
                entry.path().strip_prefix(self.base).unwrap_or(entry.path())
            };
            let relative = normalize_path_for_display(relative);
            if (!self.include.is_empty() && !self.include.is_match(&relative))
                || self.exclude.is_match(&relative)
            {
                continue;
            }
            if stats.searched_files + queued >= self.limits.max_searched_files {
                stats.stop_reason = Some(StopReason::Files);
                return Ok(None);
            }
            let metadata = match entry.metadata() {
                Ok(metadata) => metadata,
                Err(error) => {
                    stats.skipped_io_errors += 1;
                    tracing::debug!(%error, "grep cannot reserve file bytes");
                    continue;
                }
            };
            if metadata.len() > self.limits.max_file_bytes {
                stats.searched_files += 1;
                stats.skipped_large_files += 1;
                continue;
            }
            return Ok(Some(WorkItem {
                path: entry.path().into(),
                display_path: normalize_path_for_display(
                    entry
                        .path()
                        .strip_prefix(self.workspace)
                        .unwrap_or(entry.path()),
                ),
                length: metadata.len(),
            }));
        }
    }

    fn scan_chunk(
        &self,
        jobs: &[WorkItem],
        results: usize,
        output: usize,
        deadline: Instant,
    ) -> Result<Vec<scan::FileResult>, ToolError> {
        let mut searcher = SearcherBuilder::new()
            .line_number(true)
            .before_context(self.input.before_context as usize)
            .after_context(self.input.after_context as usize)
            .binary_detection(BinaryDetection::quit(b'\0'))
            .heap_limit(Some(SEARCHER_HEAP_BYTES))
            .build();
        jobs.iter()
            .map(|job| {
                scan::search_file(
                    &mut searcher,
                    self.matcher,
                    scan::FileRequest {
                        path: &job.path,
                        display_path: &job.display_path,
                        mode: self.input.mode,
                        remaining_results: results,
                        remaining_output: output,
                        remaining_bytes: job.length,
                        max_file_bytes: self.limits.max_file_bytes,
                        expected_len: Some(job.length),
                        deadline,
                        cancel: self.cancel,
                    },
                )
            })
            .collect()
    }

    fn scan_batch(
        &self,
        jobs: &[WorkItem],
        results: usize,
        output: usize,
        deadline: Instant,
    ) -> Result<Vec<scan::FileResult>, ToolError> {
        let workers = self.limits.parallelism.clamp(1, 4).min(jobs.len());
        if workers <= 1 {
            return self.scan_chunk(jobs, results, output, deadline);
        }
        let pool = self::workers()?;
        // Indexed parallel chunks preserve discovery order. No unbounded
        // queue or per-batch OS threads, including concurrent tool calls.
        let chunks: Result<Vec<_>, ToolError> = pool.install(|| {
            jobs.par_chunks(jobs.len().div_ceil(workers))
                .map(|chunk| self.scan_chunk(chunk, results, output, deadline))
                .collect()
        });
        Ok(chunks?.into_iter().flatten().collect())
    }
}

impl Collected {
    fn merge(&mut self, result: scan::FileResult, plan: &Plan<'_>) {
        let mut last_match = None;
        self.stats.skipped_large_files += usize::from(result.too_large);
        self.stats.skipped_io_errors += usize::from(result.io_error);
        self.stats.skipped_changed_files += usize::from(result.changed);
        self.stats.binary_files += usize::from(result.binary);
        for record in result.records {
            let is_match = !matches!(record, GrepRecord::Line { matched: false, .. });
            if is_match && self.returned >= plan.limits.max_results {
                self.stats.stop_reason = Some(StopReason::Results);
                return;
            }
            if let GrepRecord::Line { line, matched, .. } = &record {
                if *matched {
                    last_match = Some(*line);
                }
                // Skip before-context belonging only to an unreturned next match.
                if !matched
                    && self.returned >= plan.limits.max_results
                    && last_match
                        .is_none_or(|last| *line > last + u64::from(plan.input.after_context))
                {
                    continue;
                }
            }
            let bytes = serde_json::to_vec(&record)
                .expect("grep record serializes")
                .len()
                + 1;
            if self.bytes + bytes > plan.limits.max_output_bytes {
                self.stats.stop_reason = Some(StopReason::Output);
                return;
            }
            self.bytes += bytes;
            self.returned += usize::from(is_match);
            self.stats.shortened_lines += usize::from(matches!(
                record,
                GrepRecord::Line {
                    text_truncated: true,
                    ..
                }
            ));
            self.records.push(record);
        }
        self.stats.stop_reason = result.stop_reason;
    }
}

pub(super) fn collect(plan: Plan<'_>) -> Result<SearchResult, ToolError> {
    let deadline = Instant::now() + plan.limits.max_duration;
    let mut walker = walk_builder(plan.base, plan.include_ignored).build();
    let mut collected = Collected {
        records: Vec::new(),
        stats: SearchStats::default(),
        returned: 0,
        bytes: 0,
    };
    let mut pending = None;
    loop {
        let mut jobs = Vec::new();
        let mut reserved = 0;
        let mut exhausted = false;
        while jobs.len() < BATCH_FILES {
            let job = if let Some(job) = pending.take() {
                Some(job)
            } else {
                plan.next_file(&mut walker, &mut collected.stats, jobs.len(), deadline)?
            };
            let Some(job) = job else {
                exhausted = true;
                break;
            };
            if job.length > plan.limits.max_total_bytes - collected.stats.searched_bytes - reserved
            {
                if jobs.is_empty() {
                    collected.stats.stop_reason = Some(StopReason::Bytes);
                }
                pending = Some(job);
                break;
            }
            reserved += job.length;
            jobs.push(job);
        }
        if jobs.is_empty() {
            break;
        }
        // Discovery may have stopped after the queued prefix. Finish that prefix
        // before reporting its boundary, unless a result/output boundary precedes it.
        let discovery_stop = collected.stats.stop_reason.take();
        let files = plan.scan_batch(
            &jobs,
            plan.limits.max_results - collected.returned,
            plan.limits.max_output_bytes - collected.bytes,
            deadline,
        )?;
        // Include speculative work in the resource accounting, even if earlier
        // results fill the output limit. All jobs reserved disjoint byte budgets.
        collected.stats.searched_files += files.len();
        collected.stats.searched_bytes += files.iter().map(|file| file.bytes_read).sum::<u64>();
        for file in files {
            collected.merge(file, &plan);
            if collected.stats.stop_reason.is_some() {
                break;
            }
        }
        if collected.stats.stop_reason.is_some() {
            break;
        }
        collected.stats.stop_reason = discovery_stop;
        if exhausted || collected.stats.stop_reason.is_some() {
            break;
        }
    }
    Ok(SearchResult {
        records: collected.records,
        stats: collected.stats,
        returned_matches: collected.returned,
    })
}
