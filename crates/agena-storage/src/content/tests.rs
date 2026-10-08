use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use agena_domain::{CommandOutputStream, ContentInput};
use tokio::sync::Semaphore;

use super::*;

struct ObservedBackend {
    memory: MemoryContentBackend,
    commits: AtomicUsize,
    reads: AtomicUsize,
    fail_next: AtomicBool,
    fail_permanently: AtomicBool,
    fail_after_publish: AtomicBool,
    hold: AtomicBool,
    entered: Notify,
    release: Semaphore,
}

impl Default for ObservedBackend {
    fn default() -> Self {
        Self {
            memory: MemoryContentBackend::default(),
            commits: AtomicUsize::new(0),
            reads: AtomicUsize::new(0),
            fail_next: AtomicBool::new(false),
            fail_permanently: AtomicBool::new(false),
            fail_after_publish: AtomicBool::new(false),
            hold: AtomicBool::new(false),
            entered: Notify::new(),
            release: Semaphore::new(0),
        }
    }
}

#[async_trait]
impl ContentBackend for ObservedBackend {
    async fn restore(&self, archive: &ContentArchive) -> Result<(), StoreError> {
        self.memory.restore(archive).await
    }
    async fn prune(
        &self,
        protected: &std::collections::HashSet<ContentId>,
    ) -> Result<usize, StoreError> {
        self.memory.prune(protected).await
    }
    async fn delete(&self, id: ContentId) -> Result<(), StoreError> {
        self.memory.delete(id).await
    }
    async fn commit(
        &self,
        resource: ContentResource,
        chunks: &[Arc<ContentChunk>],
    ) -> Result<ContentResource, StoreError> {
        self.commits.fetch_add(1, Ordering::SeqCst);
        if resource.cursor.sequence > 0 {
            if self.hold.swap(false, Ordering::SeqCst) {
                self.entered.notify_one();
                self.release
                    .acquire()
                    .await
                    .expect("release commit")
                    .forget();
            }
            if self.fail_permanently.load(Ordering::SeqCst)
                || self.fail_next.swap(false, Ordering::SeqCst)
            {
                return Err(StoreError::Io("injected commit failure".into()));
            }
        }
        let committed = self.memory.commit(resource, chunks).await?;
        if self.fail_after_publish.swap(false, Ordering::SeqCst) {
            return Err(StoreError::Io("injected post-publication failure".into()));
        }
        Ok(committed)
    }

    async fn read(
        &self,
        id: ContentId,
        after: Option<ContentCursor>,
        max_bytes: usize,
    ) -> Result<ContentPage, StoreError> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        self.memory.read(id, after, max_bytes).await
    }
}

fn quiet_config() -> ContentConfig {
    ContentConfig {
        flush_interval: Duration::from_secs(60),
        flush_bytes: 512 * 1024,
        ..Default::default()
    }
}

fn progress(completed: u64) -> ContentInput {
    ContentInput::Structured {
        event: agena_domain::DocumentMutation::Progress {
            block_id: "scan".into(),
            phase: "scan".into(),
            completed,
            total: Some(100),
            unit: Some("files".into()),
        },
    }
}

#[tokio::test]
async fn document_bootstrap_reads_the_durable_checkpoint_when_live_memory_has_only_mutations() {
    let backend = Arc::new(ObservedBackend::default());
    let hub = ContentHub::new(
        backend.clone(),
        ContentConfig {
            memory_bytes: 1536,
            pending_bytes: 1536,
            chunk_bytes: 1024,
            ..quiet_config()
        },
    );
    let writer = hub.open(1, 2, ContentKind::Document).await.unwrap();
    writer.append(progress(1)).await.unwrap();
    for _ in 0..4 {
        writer
            .append(ContentInput::Log {
                stream: agena_domain::CommandOutputStream::Stdout,
                text: "x".repeat(900),
            })
            .await
            .unwrap();
    }
    writer.0.0.flush().await.unwrap();
    writer.append(progress(2)).await.unwrap();
    assert!(
        !writer
            .0
            .0
            .state
            .lock()
            .unwrap()
            .records
            .iter()
            .any(|chunk| matches!(chunk.payload, ContentPayload::StructuredSnapshot { .. }))
    );
    let before = backend.reads.load(Ordering::SeqCst);
    let page = hub
        .read(writer.resource().resource_id, None, 8192)
        .await
        .unwrap();
    assert_eq!(backend.reads.load(Ordering::SeqCst), before + 1);
    assert!(matches!(
        page.chunks[0].payload,
        ContentPayload::StructuredSnapshot { .. }
    ));
    assert!(matches!(
        page.chunks.last().unwrap().payload,
        ContentPayload::Structured { .. }
    ));
    assert_eq!(page.next_cursor, writer.resource().cursor);
    assert!(!page.gap);
    assert!(!page.has_more);
    writer.finish().await.unwrap();
}

#[tokio::test]
async fn document_checkpoints_stamp_the_chain_and_reject_updates_atomically() {
    let hub = ContentHub::new(Arc::new(MemoryContentBackend::default()), quiet_config());
    let writer = hub.open(1, 2, ContentKind::Document).await.unwrap();
    for completed in 0..34 {
        writer.append(progress(completed)).await.unwrap();
    }
    let before = writer.resource();
    assert!(
        writer
            .append(ContentInput::Structured {
                event: agena_domain::DocumentMutation::AppendRows {
                    block_id: "scan".into(),
                    rows: vec![]
                }
            })
            .await
            .is_err()
    );
    assert_eq!(writer.resource(), before);
    let resource = writer.finish().await.unwrap();
    let archive = hub.export_snapshot(resource.resource_id).await.unwrap();
    assert!(matches!(
        archive.chunks[0].payload,
        ContentPayload::StructuredSnapshot { .. }
    ));
    assert!(matches!(
        archive.chunks[32].payload,
        ContentPayload::StructuredSnapshot { .. }
    ));
    assert!(
        matches!(&archive.chunks[33].payload, ContentPayload::Structured { base_cursor, .. } if base_cursor.sequence == 33)
    );
    archive.validate().unwrap();
    assert_eq!(
        hub.0.resident_bytes.load(Ordering::Acquire),
        0,
        "the document state releases its own resident reservation"
    );
    let page = hub.read(resource.resource_id, None, 65536).await.unwrap();
    assert_eq!(
        page.chunks[0].cursor.sequence, 33,
        "new observers start from a reconstructible checkpoint"
    );
}

#[tokio::test]
async fn waiting_document_updates_are_reduced_again_after_other_writers_advance() {
    let backend = Arc::new(ObservedBackend::default());
    let hub = ContentHub::new(
        backend.clone(),
        ContentConfig {
            memory_bytes: 4096,
            pending_bytes: 4096,
            max_resident_bytes: 4096,
            chunk_bytes: 2048,
            flush_bytes: 4096,
            ..quiet_config()
        },
    );
    let writer = hub.open(1, 2, ContentKind::Structured).await.unwrap();
    writer
        .append(ContentInput::Structured {
            event: agena_domain::DocumentMutation::Insert {
                after: None,
                block: agena_domain::ViewBlock::Text {
                    id: Some("body".into()),
                    text: "x".repeat(1000),
                },
            },
        })
        .await
        .unwrap();
    backend.hold.store(true, Ordering::SeqCst);
    let pending_writer = writer.clone();
    let pending = tokio::spawn(async move {
        pending_writer
            .append(ContentInput::Structured {
                event: agena_domain::DocumentMutation::AppendText {
                    block_id: "body".into(),
                    text: "a".repeat(1000),
                },
            })
            .await
    });
    tokio::time::timeout(Duration::from_secs(2), backend.entered.notified())
        .await
        .unwrap();
    assert!(!pending.is_finished());
    writer
        .append(ContentInput::Structured {
            event: agena_domain::DocumentMutation::AppendText {
                block_id: "body".into(),
                text: "B".into(),
            },
        })
        .await
        .unwrap();
    backend.release.add_permits(1);
    tokio::time::timeout(Duration::from_secs(2), pending)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let document = writer.0.0.state.lock().unwrap().document.clone().unwrap();
    let agena_domain::ViewBlock::Text { text, .. } = &document.blocks[0] else {
        panic!("text block");
    };
    assert_eq!(text, &format!("{}B{}", "x".repeat(1000), "a".repeat(1000)));
    writer.finish().await.unwrap();
}

#[tokio::test]
async fn memory_retention_preserves_document_bases_and_describes_sparse_ranges() {
    let backend = Arc::new(MemoryContentBackend {
        retained_bytes: 1200,
        ..Default::default()
    });
    let hub = ContentHub::new(backend.clone(), quiet_config());
    let writer = hub.open(1, 2, ContentKind::Document).await.unwrap();
    writer.append(progress(1)).await.unwrap();
    for _ in 0..20 {
        writer
            .append(ContentInput::Log {
                stream: CommandOutputStream::Stdout,
                text: "x".repeat(500),
            })
            .await
            .unwrap();
    }
    let resource = writer.finish().await.unwrap();
    assert_eq!(
        resource.retained_ranges[0],
        ContentRange { first: 1, last: 1 }
    );
    assert!(resource.retained_ranges.len() > 1);
    let page = backend
        .read(resource.resource_id, None, 65536)
        .await
        .unwrap();
    assert!(matches!(
        page.chunks[0].payload,
        ContentPayload::StructuredSnapshot { .. }
    ));
    assert!(page.gap);
    hub.export_snapshot(resource.resource_id)
        .await
        .unwrap()
        .validate()
        .unwrap();
}

#[tokio::test]
async fn restore_cannot_bypass_the_memory_retention_budget() {
    let source = ContentHub::in_memory();
    let writer = source.open(1, 2, ContentKind::Text).await.unwrap();
    writer.append(text(&"x".repeat(1024))).await.unwrap();
    let id = writer.finish().await.unwrap().resource_id;
    let archive = source.export_snapshot(id).await.unwrap();
    let destination = MemoryContentBackend {
        retained_bytes: 100,
        ..Default::default()
    };
    assert!(matches!(
        destination.restore(&archive).await,
        Err(StoreError::Constraint(_))
    ));
    assert!(matches!(
        destination.describe(id).await,
        Err(StoreError::NotFound(_))
    ));
}

#[tokio::test]
async fn completed_sources_release_slots_and_bytes_even_when_writers_are_retained() {
    let hub = ContentHub::new(
        Arc::new(MemoryContentBackend::default()),
        ContentConfig {
            max_active_sources: 1,
            ..quiet_config()
        },
    );
    let writer = hub.open(1, 2, ContentKind::Text).await.unwrap();
    assert!(hub.open(1, 3, ContentKind::Text).await.is_err());
    writer.append(text("retained descriptor")).await.unwrap();
    assert!(hub.0.resident_bytes.load(Ordering::Acquire) > 0);
    writer.finish().await.unwrap();
    assert_eq!(hub.0.resident_bytes.load(Ordering::Acquire), 0);
    assert_eq!(hub.0.slots.available_permits(), 1);
    let next = hub.open(1, 3, ContentKind::Text).await.unwrap();
    assert!(writer.append(text("sealed")).await.is_err());
    next.finish().await.unwrap();
    drop(writer);
    assert_eq!(hub.0.resident_bytes.load(Ordering::Acquire), 0);
}

#[tokio::test]
async fn global_pressure_reclaims_committed_replay_without_losing_pending_records() {
    let hub = ContentHub::new(
        Arc::new(MemoryContentBackend::default()),
        ContentConfig {
            max_resident_bytes: 1024,
            memory_bytes: 1024,
            pending_bytes: 1024,
            chunk_bytes: 512,
            ..quiet_config()
        },
    );
    let first = hub.open(1, 2, ContentKind::Text).await.unwrap();
    let second = hub.open(1, 3, ContentKind::Text).await.unwrap();
    for source in [&first, &second] {
        for _ in 0..3 {
            source.capture(text(&"x".repeat(100))).unwrap();
        }
    }
    assert_eq!(hub.0.resident_bytes.load(Ordering::Acquire), 984);
    assert!(first.capture(text("no room")).is_err());
    first.0.0.commit_batch().await.unwrap();
    second.capture(text(&"z".repeat(100))).unwrap();
    assert!(hub.0.resident_bytes.load(Ordering::Acquire) <= 1024);
    let page = hub
        .read(
            first.resource().resource_id,
            Some(beginning(&first.resource())),
            1024,
        )
        .await
        .unwrap();
    assert_eq!(page.chunks.len(), 3);
    assert!(!page.gap);
    first.finish().await.unwrap();
    second.finish().await.unwrap();
    assert_eq!(hub.0.resident_bytes.load(Ordering::Acquire), 0);
}

#[tokio::test]
async fn many_capture_losses_share_one_timer_commit() {
    let backend = Arc::new(ObservedBackend::default());
    let hub = ContentHub::new(
        backend.clone(),
        ContentConfig {
            flush_interval: Duration::from_millis(20),
            ..quiet_config()
        },
    );
    let writer = hub.open(1, 2, ContentKind::Log).await.unwrap();
    for _ in 0..1000 {
        writer.record_loss(1, &"capture unavailable");
    }
    assert_eq!(backend.commits.load(Ordering::Acquire), 1);
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            let persisted = backend
                .memory
                .describe(writer.resource().resource_id)
                .await
                .unwrap();
            if persisted.dropped_bytes == 1000 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(backend.commits.load(Ordering::Acquire), 2);
    writer.finish().await.unwrap();
}

#[tokio::test]
async fn empty_reclaimed_replay_reads_committed_history_instead_of_advancing_past_it() {
    let backend = Arc::new(ObservedBackend::default());
    let hub = ContentHub::new(
        backend.clone(),
        ContentConfig {
            max_resident_bytes: 1024,
            memory_bytes: 1024,
            pending_bytes: 1024,
            chunk_bytes: 512,
            ..quiet_config()
        },
    );
    let writer = hub.open(1, 2, ContentKind::Text).await.unwrap();
    writer.append(text("committed history")).await.unwrap();
    writer.0.0.commit_batch().await.unwrap();
    hub.0.reclaim_committed(1024);
    assert!(writer.0.0.state.lock().unwrap().records.is_empty());
    for after in [None, Some(beginning(&writer.resource()))] {
        let page = hub
            .read(writer.resource().resource_id, after, 1024)
            .await
            .unwrap();
        assert_eq!(page.chunks.len(), 1);
        assert_eq!(
            page.chunks[0].payload,
            text("committed history").into_payload(None).unwrap()
        );
        assert!(!page.gap);
        assert_eq!(page.next_cursor, writer.resource().cursor);
    }
    assert_eq!(backend.reads.load(Ordering::Acquire), 2);
    writer.finish().await.unwrap();
}

#[tokio::test]
async fn terminal_patch_requires_the_last_frame_and_its_geometry() {
    let hub = ContentHub::in_memory();
    let writer = hub.open(1, 2, ContentKind::Terminal).await.unwrap();
    let screen = agena_domain::TerminalSnapshot {
        rows: 2,
        cols: 4,
        cursor_row: 0,
        cursor_col: 0,
        cursor_visible: true,
        alternate_screen: false,
        bracketed_paste: false,
        application_cursor: false,
        cells: vec![],
    };
    let first = writer
        .append(ContentInput::Terminal {
            screen: screen.clone(),
        })
        .await
        .unwrap();
    writer
        .append(ContentInput::Log {
            stream: CommandOutputStream::Stdout,
            text: "raw".into(),
        })
        .await
        .unwrap();
    let patch = ContentInput::TerminalPatch {
        base_cursor: first,
        screen: screen.clone(),
        rows_changed: vec![0],
    };
    writer.append(patch.clone()).await.unwrap();
    assert!(
        writer.append(patch).await.is_err(),
        "stale patch cannot corrupt the current screen"
    );
    let mut resized = screen;
    resized.cols = 5;
    assert!(
        writer
            .append(ContentInput::TerminalPatch {
                base_cursor: writer.resource().cursor,
                screen: resized,
                rows_changed: vec![0],
            })
            .await
            .is_err()
    );
    writer.finish().await.unwrap();
}

fn text(value: &str) -> ContentInput {
    ContentInput::Text { text: value.into() }
}

fn beginning(resource: &ContentResource) -> ContentCursor {
    ContentCursor {
        sequence: 0,
        ..resource.cursor
    }
}

async fn wait_for_state(receiver: &mut watch::Receiver<ContentResource>, desired: ContentState) {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if receiver.borrow_and_update().state == desired {
                break;
            }
            receiver
                .changed()
                .await
                .expect("content worker remains live");
        }
    })
    .await
    .expect("source reaches terminal state");
}

#[tokio::test]
async fn live_appends_do_not_read_or_write_the_backend() {
    let backend = Arc::new(ObservedBackend::default());
    let hub = ContentHub::new(backend.clone(), quiet_config());
    let writer = hub.open(1, 2, ContentKind::Log).await.unwrap();
    writer
        .append(ContentInput::Log {
            stream: CommandOutputStream::Stdout,
            text: "  中\n\npartial".into(),
        })
        .await
        .unwrap();
    writer
        .append(ContentInput::Log {
            stream: CommandOutputStream::Stderr,
            text: "\rwarning".into(),
        })
        .await
        .unwrap();
    let resource = writer.resource();
    let page = hub
        .read(resource.resource_id, Some(beginning(&resource)), 1024)
        .await
        .unwrap();
    assert_eq!(page.chunks.len(), 2);
    assert_eq!(
        page.chunks[0].payload,
        ContentPayload::Log {
            stream: CommandOutputStream::Stdout,
            text: "  中\n\npartial".into(),
        }
    );
    assert_eq!(
        page.chunks[1].payload,
        ContentPayload::Log {
            stream: CommandOutputStream::Stderr,
            text: "\rwarning".into(),
        }
    );
    assert_eq!(resource.committed_cursor.sequence, 0);
    assert_eq!(resource.cursor.sequence, 2);
    assert_eq!(
        backend.commits.load(Ordering::SeqCst),
        1,
        "only resource creation commits"
    );
    assert_eq!(backend.reads.load(Ordering::SeqCst), 0);
    writer.finish().await.unwrap();
}

#[tokio::test]
async fn subscribe_before_bootstrap_covers_racing_appends() {
    let hub = ContentHub::in_memory();
    let writer = hub.open(1, 2, ContentKind::Text).await.unwrap();
    let id = writer.resource().resource_id;
    let mut receiver = hub.subscribe(id).unwrap();
    writer.append(text("first")).await.unwrap();
    let bootstrap = hub.read(id, None, 1024).await.unwrap();
    assert_eq!(receiver.borrow_and_update().cursor, bootstrap.next_cursor);
    writer.append(text("second")).await.unwrap();
    receiver.changed().await.unwrap();
    let page = hub
        .read(id, Some(bootstrap.next_cursor), 1024)
        .await
        .unwrap();
    assert_eq!(page.chunks.len(), 1);
    assert_eq!(
        page.chunks[0].payload,
        text("second").into_payload(None).unwrap()
    );
    writer.finish().await.unwrap();
}

#[tokio::test]
async fn quiet_last_chunk_is_committed_by_the_timer() {
    let backend = Arc::new(ObservedBackend::default());
    let hub = ContentHub::new(
        backend.clone(),
        ContentConfig {
            flush_interval: Duration::from_millis(10),
            ..quiet_config()
        },
    );
    let writer = hub.open(1, 2, ContentKind::Text).await.unwrap();
    let mut receiver = hub.subscribe(writer.resource().resource_id).unwrap();
    writer.append(text("last before a pause")).await.unwrap();
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if receiver.borrow_and_update().committed_cursor.sequence == 1 {
                break;
            }
            receiver.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    assert_eq!(backend.commits.load(Ordering::SeqCst), 2);
    writer.finish().await.unwrap();
}

#[tokio::test]
async fn completion_commits_the_tail_and_seals_the_source() {
    let hub = ContentHub::in_memory();
    let writer = hub.open(1, 2, ContentKind::Text).await.unwrap();
    writer.append(text("one")).await.unwrap();
    writer.append(text("two")).await.unwrap();
    let completed = writer.finish().await.unwrap();
    assert_eq!(completed.state, ContentState::Complete);
    assert_eq!(completed.committed_cursor, completed.cursor);
    assert!(writer.append(text("late")).await.is_err());
    assert!(
        hub.subscribe(completed.resource_id).is_none(),
        "completed resources leave the active registry"
    );
    let page = hub
        .read(completed.resource_id, Some(beginning(&completed)), 1024)
        .await
        .unwrap();
    assert_eq!(page.chunks.len(), 2);
    assert_eq!(page.resource.state, ContentState::Complete);
}

#[tokio::test]
async fn finalization_under_permanent_pressure_records_loss_and_releases_the_live_source() {
    let backend = Arc::new(MemoryContentBackend {
        retained_bytes: 128,
        ..Default::default()
    });
    let hub = ContentHub::new(backend, quiet_config());
    let writer = hub.open(1, 2, ContentKind::Text).await.unwrap();
    writer.append(text(&"x".repeat(512))).await.unwrap();
    let cursor = writer.resource().cursor;
    let resource = writer.finalize(ContentState::Complete).await.unwrap();
    assert_eq!(resource.state, ContentState::Interrupted);
    assert_eq!(resource.cursor, cursor);
    assert_eq!(resource.committed_cursor, cursor);
    assert_eq!(resource.dropped_bytes, 512);
    assert!(resource.capture_error.is_some());
    assert!(resource.retained_ranges.is_empty());
    assert!(hub.subscribe(resource.resource_id).is_none());
    assert_eq!(hub.0.resident_bytes.load(Ordering::Acquire), 0);
    let page = hub
        .read(resource.resource_id, Some(beginning(&resource)), 1024)
        .await
        .unwrap();
    assert!(page.gap);
    assert!(!page.has_more);
    assert_eq!(page.next_cursor, cursor);
    assert_eq!(page.resource, resource);
    let archive = hub.export_snapshot(resource.resource_id).await.unwrap();
    archive.validate().unwrap();
    let other = ContentHub::in_memory();
    other.restore(&archive).await.unwrap();
    assert_eq!(
        other.describe(resource.resource_id).await.unwrap(),
        resource
    );
}

#[tokio::test]
async fn cancelling_a_finish_waiter_does_not_discard_its_batch() {
    let backend = Arc::new(ObservedBackend::default());
    let hub = ContentHub::new(backend.clone(), quiet_config());
    let writer = hub.open(1, 2, ContentKind::Text).await.unwrap();
    let id = writer.resource().resource_id;
    let mut receiver = hub.subscribe(id).unwrap();
    writer
        .append(text("must survive cancellation"))
        .await
        .unwrap();
    backend.hold.store(true, Ordering::SeqCst);
    let waiter = writer.clone();
    let task = tokio::spawn(async move { waiter.finish().await });
    tokio::time::timeout(Duration::from_secs(1), backend.entered.notified())
        .await
        .unwrap();
    task.abort();
    let _ = task.await;
    backend.release.add_permits(1);
    wait_for_state(&mut receiver, ContentState::Complete).await;
    let page = hub
        .read(id, Some(beginning(&writer.resource())), 1024)
        .await
        .unwrap();
    assert_eq!(
        page.chunks[0].payload,
        text("must survive cancellation")
            .into_payload(None)
            .unwrap()
    );
    assert_eq!(page.resource.committed_cursor.sequence, 1);
}

#[tokio::test]
async fn failed_commit_retains_output_for_a_retry() {
    let backend = Arc::new(ObservedBackend::default());
    let hub = ContentHub::new(backend.clone(), quiet_config());
    let writer = hub.open(1, 2, ContentKind::Text).await.unwrap();
    writer.append(text("retained on failure")).await.unwrap();
    backend.fail_next.store(true, Ordering::SeqCst);
    // Call the batch directly to isolate retry state from the independent
    // completion worker.
    assert!(writer.0.0.commit_batch().await.is_err());
    assert_eq!(writer.resource().committed_cursor.sequence, 0);
    let page = hub
        .read(writer.resource().resource_id, None, 1024)
        .await
        .unwrap();
    assert_eq!(
        page.chunks[0].payload,
        text("retained on failure").into_payload(None).unwrap()
    );
    let completed = writer.finish().await.unwrap();
    assert_eq!(completed.committed_cursor.sequence, 1);
    assert!(completed.capture_error.is_none());
}

#[tokio::test]
async fn capture_loss_is_committed_without_another_chunk_and_survives_recovery() {
    let backend = Arc::new(ObservedBackend::default());
    let hub = ContentHub::new(backend.clone(), quiet_config());
    let writer = hub.open(1, 2, ContentKind::Text).await.unwrap();
    writer.record_loss(13, &"capture unavailable");
    writer.0.0.commit_batch().await.unwrap();
    let persisted = backend
        .memory
        .describe(writer.resource().resource_id)
        .await
        .unwrap();
    assert_eq!(persisted.dropped_bytes, 13);
    assert_eq!(persisted.total_bytes, 13);
    assert_eq!(
        persisted.capture_error.as_deref(),
        Some("capture unavailable")
    );
    writer.append(text("recovered")).await.unwrap();
    let complete = writer.finish().await.unwrap();
    assert_eq!(complete.state, ContentState::Interrupted);
    assert_eq!(complete.total_bytes, 22);
    assert_eq!(complete.dropped_bytes, 13);
    assert_eq!(complete.committed_cursor, complete.cursor);
    writer.record_loss(100, &"late callback");
    assert_eq!(writer.resource(), complete, "sealed facts are immutable");
}

#[tokio::test]
async fn abandoning_the_last_writer_marks_content_interrupted() {
    let hub = ContentHub::in_memory();
    let writer = hub.open(1, 2, ContentKind::Text).await.unwrap();
    writer.append(text("partial output")).await.unwrap();
    let id = writer.resource().resource_id;
    let mut receiver = hub.subscribe(id).unwrap();
    drop(writer);
    wait_for_state(&mut receiver, ContentState::Interrupted).await;
    let page = hub.read(id, None, 1024).await.unwrap();
    assert_eq!(page.resource.state, ContentState::Interrupted);
    assert_eq!(
        page.chunks[0].payload,
        text("partial output").into_payload(None).unwrap()
    );
}

#[tokio::test]
async fn abandoning_under_permanent_pressure_seals_loss_without_retrying_forever() {
    let backend = Arc::new(MemoryContentBackend {
        retained_bytes: 128,
        ..Default::default()
    });
    let hub = ContentHub::new(backend, quiet_config());
    let writer = hub.open(1, 2, ContentKind::Text).await.unwrap();
    writer.append(text(&"x".repeat(512))).await.unwrap();
    let id = writer.resource().resource_id;
    let mut receiver = hub.subscribe(id).unwrap();
    drop(writer);
    wait_for_state(&mut receiver, ContentState::Interrupted).await;
    let page = hub
        .read(
            id,
            Some(ContentCursor {
                epoch: id.0,
                sequence: 0,
            }),
            1024,
        )
        .await
        .unwrap();
    assert!(page.gap);
    assert_eq!(page.resource.dropped_bytes, 512);
    assert_eq!(page.resource.cursor.sequence, 1);
    assert!(hub.subscribe(id).is_none());
    assert_eq!(hub.0.resident_bytes.load(Ordering::Acquire), 0);
}

#[tokio::test]
async fn abandoned_source_releases_memory_even_when_loss_descriptor_cannot_be_written() {
    let backend = Arc::new(ObservedBackend::default());
    let hub = ContentHub::new(backend.clone(), quiet_config());
    let writer = hub.open(1, 2, ContentKind::Text).await.unwrap();
    writer.append(text("unpersistable tail")).await.unwrap();
    let id = writer.resource().resource_id;
    let mut receiver = hub.subscribe(id).unwrap();
    backend.fail_permanently.store(true, Ordering::SeqCst);
    drop(writer);
    wait_for_state(&mut receiver, ContentState::Interrupted).await;
    assert!(receiver.borrow().capture_error.is_some());
    assert!(hub.subscribe(id).is_none());
    assert_eq!(hub.0.resident_bytes.load(Ordering::Acquire), 0);
    assert_eq!(backend.commits.load(Ordering::SeqCst), 4);
}

#[tokio::test]
async fn finalization_reconciles_a_published_batch_before_declaring_loss() {
    let backend = Arc::new(ObservedBackend::default());
    let hub = ContentHub::new(backend.clone(), quiet_config());
    let writer = hub.open(1, 2, ContentKind::Text).await.unwrap();
    writer
        .append(text("published before sync failure"))
        .await
        .unwrap();
    backend.fail_after_publish.store(true, Ordering::SeqCst);
    let resource = writer.finalize(ContentState::Complete).await.unwrap();
    assert_eq!(resource.state, ContentState::Complete);
    assert_eq!(resource.dropped_bytes, 0);
    assert!(resource.capture_error.is_none());
    let page = hub.read(resource.resource_id, None, 1024).await.unwrap();
    assert_eq!(page.chunks.len(), 1);
    assert_eq!(page.resource, resource);
}

#[tokio::test]
async fn wrong_epoch_and_future_cursors_are_rejected() {
    let hub = ContentHub::in_memory();
    let writer = hub.open(1, 2, ContentKind::Text).await.unwrap();
    let resource = writer.resource();
    assert!(matches!(
        hub.read(
            resource.resource_id,
            Some(ContentCursor {
                epoch: ContentId::new().0,
                sequence: 0,
            }),
            1024
        )
        .await,
        Err(StoreError::Conflict(_))
    ));
    assert!(matches!(
        hub.read(
            resource.resource_id,
            Some(ContentCursor {
                sequence: 1,
                ..resource.cursor
            }),
            1024
        )
        .await,
        Err(StoreError::Conflict(_))
    ));
    assert!(
        writer
            .append(ContentInput::Log {
                stream: CommandOutputStream::Stdout,
                text: "wrong kind".into()
            })
            .await
            .is_err()
    );
    writer.finish().await.unwrap();
}

#[tokio::test]
async fn a_slow_reader_recovers_evicted_memory_from_storage() {
    let backend = Arc::new(ObservedBackend::default());
    let hub = ContentHub::new(
        backend.clone(),
        ContentConfig {
            memory_bytes: 1024,
            pending_bytes: 1024,
            chunk_bytes: 512,
            flush_bytes: 1,
            flush_interval: Duration::from_secs(60),
            ..Default::default()
        },
    );
    let writer = hub.open(1, 2, ContentKind::Text).await.unwrap();
    let start = beginning(&writer.resource());
    for _ in 0..20 {
        writer.append(text(&"界".repeat(30))).await.unwrap();
        writer.0.0.flush().await.unwrap();
    }
    assert!(writer.0.0.state.lock().unwrap().records.len() < 20);
    let page = hub
        .read(writer.resource().resource_id, Some(start), 4096)
        .await
        .unwrap();
    assert_eq!(page.chunks.len(), 20);
    assert!(!page.gap);
    assert!(backend.reads.load(Ordering::SeqCst) > 0);
    writer.finish().await.unwrap();
}

#[tokio::test]
async fn retention_loss_is_an_explicit_gap() {
    let backend = Arc::new(MemoryContentBackend {
        retained_bytes: 200,
        ..Default::default()
    });
    let hub = ContentHub::new(backend, quiet_config());
    let writer = hub.open(1, 2, ContentKind::Text).await.unwrap();
    let start = beginning(&writer.resource());
    for _ in 0..3 {
        writer.append(text(&"x".repeat(100))).await.unwrap();
    }
    let completed = writer.finish().await.unwrap();
    let page = hub
        .read(completed.resource_id, Some(start), 1024)
        .await
        .unwrap();
    assert!(page.gap);
    assert_eq!(page.chunks.len(), 1);
    assert_eq!(page.chunks[0].cursor.sequence, 3);
    assert_eq!(
        page.resource.retained_ranges,
        vec![ContentRange { first: 3, last: 3 }]
    );
}

#[tokio::test]
async fn bounded_pages_preserve_whole_utf8_records() {
    let hub = ContentHub::in_memory();
    let writer = hub.open(1, 2, ContentKind::Text).await.unwrap();
    let start = beginning(&writer.resource());
    writer.append(text("中")).await.unwrap();
    writer.append(text("文")).await.unwrap();
    let first = hub
        .read(writer.resource().resource_id, Some(start), 3)
        .await
        .unwrap();
    assert_eq!(first.chunks.len(), 1);
    assert!(first.has_more);
    let second = hub
        .read(writer.resource().resource_id, Some(first.next_cursor), 3)
        .await
        .unwrap();
    assert_eq!(
        second.chunks[0].payload,
        text("文").into_payload(None).unwrap()
    );
    assert!(!second.has_more);
    writer.finish().await.unwrap();
}
