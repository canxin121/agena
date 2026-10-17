use agena_domain::{ContentInput, ContentKind, ContentPayload, ContentRange};
use agena_storage::content::{ContentConfig, ContentHub};

use super::*;

fn quiet_config() -> ContentConfig {
    ContentConfig {
        flush_interval: std::time::Duration::from_secs(60),
        flush_bytes: 512 * 1024,
        ..Default::default()
    }
}

fn text(value: &str) -> ContentInput {
    ContentInput::Text { text: value.into() }
}

async fn seed_store_above_one_gib(root: &Path) -> ContentArchive {
    let backend = Arc::new(FileContentBackend::new(root, Default::default()));
    let hub = ContentHub::new(backend.clone(), quiet_config());
    let writer = hub.open(7, 9, ContentKind::Text).await.unwrap();
    writer.append(text("historical output")).await.unwrap();
    let resource = writer.finish().await.unwrap();
    let archive = hub.export_snapshot(resource.resource_id).await.unwrap();
    // The former quota counted every file in a resource directory. Sparse
    // padding reproduces that usage without allocating or writing 1 GiB.
    let padding = tokio::fs::File::create(
        backend
            .directory(resource.resource_id)
            .join("retained-size.fixture"),
    )
    .await
    .unwrap();
    padding.set_len(1024 * 1024 * 1024).await.unwrap();
    archive
}

#[tokio::test]
async fn store_above_one_gib_allows_concurrent_sessions_and_reopening() {
    let root = tempfile::tempdir().unwrap();
    let history = seed_store_above_one_gib(root.path()).await;
    let hub = ContentHub::new(
        Arc::new(FileContentBackend::new(root.path(), Default::default())),
        quiet_config(),
    );
    let (first, second) = tokio::try_join!(
        hub.open(8, 10, ContentKind::Text),
        hub.open(9, 11, ContentKind::Log),
    )
    .unwrap();
    let first_payload = text("new model output");
    let second_payload = ContentInput::Log {
        stream: agena_domain::CommandOutputStream::Stdout,
        text: "new tool output".into(),
    };
    tokio::try_join!(
        first.append(first_payload.clone()),
        second.append(second_payload.clone()),
    )
    .unwrap();
    let (first, second) = tokio::try_join!(
        first.finalize(ContentState::Complete),
        second.finalize(ContentState::Complete),
    )
    .unwrap();
    let reopened = FileContentBackend::new(root.path(), Default::default());
    for (resource, payload) in [(first, first_payload), (second, second_payload)] {
        assert_eq!(resource.state, ContentState::Complete);
        assert_eq!(resource.dropped_bytes, 0);
        assert!(resource.capture_error.is_none());
        let page = reopened
            .read(resource.resource_id, None, 1024)
            .await
            .unwrap();
        assert_eq!(page.resource, resource);
        assert_eq!(page.chunks.len(), 1);
        assert_eq!(page.chunks[0].payload, payload.into_payload(None).unwrap());
        assert!(!page.gap);
    }
    let page = reopened
        .read(history.resource.resource_id, None, 1024)
        .await
        .unwrap();
    assert_eq!(page.resource, history.resource);
    assert_eq!(page.chunks, history.chunks);
}

#[tokio::test]
async fn store_above_one_gib_allows_archive_restore() {
    let root = tempfile::tempdir().unwrap();
    let mut archive = seed_store_above_one_gib(root.path()).await;
    let id = ContentId::new();
    archive.remap(id, 8, 10);
    let backend = FileContentBackend::new(root.path(), Default::default());
    backend.restore(&archive).await.unwrap();
    let reopened = FileContentBackend::new(root.path(), Default::default());
    let page = reopened.read(id, None, 1024).await.unwrap();
    assert_eq!(page.resource, archive.resource);
    assert_eq!(page.chunks, archive.chunks);
    assert!(!page.gap);
}

#[tokio::test]
async fn restarting_recovers_unpublished_files_before_the_next_commit() {
    let root = tempfile::tempdir().unwrap();
    let backend = Arc::new(FileContentBackend::new(root.path(), Default::default()));
    let hub = ContentHub::new(backend.clone(), quiet_config());
    let writer = hub.open(7, 9, ContentKind::Text).await.unwrap();
    let resource = writer.resource();
    let directory = backend.directory(resource.resource_id);
    tokio::fs::write(
        FileContentBackend::segment_path(&directory, 20),
        vec![b'x'; 4096],
    )
    .await
    .unwrap();
    tokio::fs::write(directory.join("manifest-next.json"), vec![b'x'; 4096])
        .await
        .unwrap();
    let reopened = FileContentBackend::new(root.path(), Default::default());
    let mut completed = resource;
    completed.state = ContentState::Complete;
    reopened.commit(completed, &[]).await.unwrap();
    assert!(!FileContentBackend::segment_path(&directory, 20).exists());
    assert!(!directory.join("manifest-next.json").exists());
    let mut entries = tokio::fs::read_dir(directory).await.unwrap();
    assert_eq!(
        entries.next_entry().await.unwrap().unwrap().file_name(),
        "manifest.json"
    );
    assert!(entries.next_entry().await.unwrap().is_none());
}

#[tokio::test]
async fn document_checkpoint_chain_and_logs_reopen_under_segment_retention() {
    let root = tempfile::tempdir().unwrap();
    let config = FileContentConfig {
        segment_bytes: 200,
        retained_segments: 2,
        ..Default::default()
    };
    let backend = Arc::new(FileContentBackend::new(root.path(), config.clone()));
    let hub = ContentHub::new(backend, quiet_config());
    let writer = hub.open(7, 9, ContentKind::Document).await.unwrap();
    for completed in 0..40 {
        writer
            .append(ContentInput::Structured {
                event: agena_domain::DocumentMutation::Progress {
                    block_id: "scan".into(),
                    phase: "scan".into(),
                    completed,
                    total: Some(40),
                    unit: None,
                },
            })
            .await
            .unwrap();
        writer
            .append(ContentInput::Log {
                stream: agena_domain::CommandOutputStream::Stdout,
                text: format!("{completed}{}", "x".repeat(100)),
            })
            .await
            .unwrap();
    }
    let resource = writer.finish().await.unwrap();
    let archive = hub.export_snapshot(resource.resource_id).await.unwrap();
    archive.validate().unwrap();
    let reopened = FileContentBackend::new(root.path(), config);
    let page = reopened
        .read(resource.resource_id, None, 65536)
        .await
        .unwrap();
    assert!(matches!(
        page.chunks[0].payload,
        ContentPayload::StructuredSnapshot { .. }
    ));
    let mut document = agena_domain::ContentDocument::default();
    let mut cursor = None;
    for chunk in page.chunks {
        match chunk.payload {
            ContentPayload::StructuredSnapshot { document: snapshot } => {
                document = snapshot;
                cursor = Some(chunk.cursor);
            }
            ContentPayload::Structured { base_cursor, event } => {
                assert_eq!(Some(base_cursor), cursor);
                document = document.updated(&event).unwrap();
                cursor = Some(chunk.cursor);
            }
            _ => {}
        }
    }
    assert!(matches!(
        &document.blocks[0],
        agena_domain::ViewBlock::Progress { completed: 39, .. }
    ));
    let imported = tempfile::tempdir().unwrap();
    FileContentBackend::new(
        imported.path(),
        FileContentConfig {
            segment_bytes: 200,
            retained_segments: 2,
            ..Default::default()
        },
    )
    .restore(&archive)
    .await
    .unwrap();
}

#[tokio::test]
async fn failed_publication_recovers_unpublished_files_before_retrying() {
    let root = tempfile::tempdir().unwrap();
    let backend = FileContentBackend::new(root.path(), Default::default());
    let id = ContentId::new();
    let cursor = ContentCursor {
        epoch: id.0,
        sequence: 0,
    };
    let mut resource = ContentResource {
        resource_id: id,
        owner_session_id: 7,
        part_id: 9,
        kind: ContentKind::Text,
        state: ContentState::Active,
        cursor,
        committed_cursor: cursor,
        total_bytes: 0,
        dropped_bytes: 0,
        retained_ranges: Vec::new(),
        capture_error: None,
    };
    backend.commit(resource.clone(), &[]).await.unwrap();
    let directory = backend.directory(id);
    let before = tokio::fs::read(directory.join("manifest.json"))
        .await
        .unwrap();
    // Reject manifest publication after record bytes have reached disk.
    let obstruction = directory.join("manifest-next.json");
    tokio::fs::create_dir(&obstruction).await.unwrap();
    resource.cursor.sequence = 1;
    resource.committed_cursor = resource.cursor;
    resource.total_bytes = "retry this batch".len() as u64;
    resource.state = ContentState::Complete;
    let chunks = [Arc::new(ContentChunk {
        captured_at_ms: 1000,
        cursor: resource.cursor,
        payload: text("retry this batch").into_payload(None).unwrap(),
    })];
    assert!(matches!(
        backend.commit(resource.clone(), &chunks).await,
        Err(StoreError::Io(_))
    ));
    assert_eq!(
        tokio::fs::read(directory.join("manifest.json"))
            .await
            .unwrap(),
        before
    );
    tokio::fs::remove_dir(obstruction).await.unwrap();
    let unpublished = FileContentBackend::segment_path(&directory, 20);
    tokio::fs::write(&unpublished, b"incomplete batch")
        .await
        .unwrap();
    let resource = backend.commit(resource, &chunks).await.unwrap();
    assert_eq!(resource.state, ContentState::Complete);
    assert_eq!(resource.cursor.sequence, 1);
    assert!(!unpublished.exists());
    let page = backend
        .read(resource.resource_id, None, 1024)
        .await
        .unwrap();
    assert_eq!(page.chunks.len(), 1);
    assert_eq!(
        page.chunks[0].payload,
        text("retry this batch").into_payload(None).unwrap()
    );
    assert!(!page.gap);
}

#[tokio::test]
async fn resource_retention_failure_still_allows_a_loss_descriptor_to_be_sealed() {
    let root = tempfile::tempdir().unwrap();
    let backend = Arc::new(FileContentBackend::new(
        root.path(),
        FileContentConfig {
            max_retained_bytes: 1000,
            ..Default::default()
        },
    ));
    let hub = ContentHub::new(backend.clone(), quiet_config());
    let writer = hub.open(7, 9, ContentKind::Log).await.unwrap();
    writer
        .append(ContentInput::Log {
            stream: agena_domain::CommandOutputStream::Stderr,
            text: "x".repeat(1500),
        })
        .await
        .unwrap();
    let resource = writer.finalize(ContentState::Complete).await.unwrap();
    assert_eq!(resource.state, ContentState::Interrupted);
    assert_eq!(resource.dropped_bytes, 1500);
    assert_eq!(resource.cursor.sequence, 1);
    assert_eq!(resource.committed_cursor, resource.cursor);
    assert!(resource.capture_error.is_some());
    assert!(hub.subscribe(resource.resource_id).is_none());
    let reopened = ContentHub::new(
        Arc::new(FileContentBackend::new(root.path(), Default::default())),
        quiet_config(),
    );
    let page = reopened
        .read(
            resource.resource_id,
            Some(ContentCursor {
                sequence: 0,
                ..resource.cursor
            }),
            1024,
        )
        .await
        .unwrap();
    assert_eq!(page.resource, resource);
    assert!(page.gap);
    assert!(!page.has_more);
    assert!(page.chunks.is_empty());
    let archive = reopened
        .export_snapshot(resource.resource_id)
        .await
        .unwrap();
    archive.validate().unwrap();
}

#[tokio::test]
async fn restore_respects_retention_and_publishes_nothing_on_rejection() {
    let source = ContentHub::in_memory();
    let writer = source.open(7, 9, ContentKind::Text).await.unwrap();
    for _ in 0..5 {
        writer.append(text(&"x".repeat(120))).await.unwrap();
    }
    let id = writer.finish().await.unwrap().resource_id;
    let archive = source.export_snapshot(id).await.unwrap();
    let root = tempfile::tempdir().unwrap();
    let backend = FileContentBackend::new(
        root.path(),
        FileContentConfig {
            segment_bytes: 200,
            retained_segments: 2,
            cached_manifests: 2,
            ..Default::default()
        },
    );
    assert!(matches!(
        backend.restore(&archive).await,
        Err(StoreError::Constraint(_))
    ));
    assert!(!backend.directory(id).exists());
    assert!(!root.path().join(format!(".import-{id}")).exists());
    assert!(matches!(
        backend.describe(id).await,
        Err(StoreError::NotFound(_))
    ));
    backend.delete(id).await.unwrap();
}

#[tokio::test]
async fn mixed_document_events_survive_reopening() {
    let root = tempfile::tempdir().unwrap();
    let hub = ContentHub::new(
        Arc::new(FileContentBackend::new(root.path(), Default::default())),
        quiet_config(),
    );
    let writer = hub.open(7, 9, ContentKind::Document).await.unwrap();
    let events = vec![
        text("searching"),
        ContentInput::Log {
            stream: agena_domain::CommandOutputStream::Stderr,
            text: "warning\n".into(),
        },
        ContentInput::Structured {
            event: agena_domain::DocumentMutation::Progress {
                block_id: "scan".into(),
                phase: "searching".into(),
                completed: 2,
                total: Some(3),
                unit: Some("sources".into()),
            },
        },
    ];
    for event in &events {
        writer.append(event.clone()).await.unwrap();
    }
    let complete = writer.finish().await.unwrap();
    let reopened = FileContentBackend::new(root.path(), Default::default());
    let page = reopened
        .read(
            complete.resource_id,
            Some(ContentCursor {
                sequence: 0,
                ..complete.cursor
            }),
            1024,
        )
        .await
        .unwrap();
    assert_eq!(page.resource, complete);
    assert_eq!(
        page.chunks
            .into_iter()
            .map(|chunk| chunk.payload)
            .collect::<Vec<_>>(),
        vec![
            events[0].clone().into_payload(None).unwrap(),
            events[1].clone().into_payload(None).unwrap(),
            ContentPayload::StructuredSnapshot {
                document: agena_domain::ContentDocument::default()
                    .updated(match &events[2] {
                        ContentInput::Structured { event } => event,
                        _ => unreachable!(),
                    })
                    .unwrap(),
            }
        ]
    );
}

#[tokio::test]
async fn cache_eviction_preserves_the_gate_of_an_in_use_manifest() {
    let root = tempfile::tempdir().unwrap();
    let backend = Arc::new(FileContentBackend::new(
        root.path(),
        FileContentConfig {
            cached_manifests: 1,
            ..Default::default()
        },
    ));
    let hub = ContentHub::new(backend.clone(), quiet_config());
    let first = hub.open(7, 9, ContentKind::Text).await.unwrap();
    let held = backend
        .manifest(first.resource().resource_id, None)
        .await
        .unwrap();
    let second = hub.open(7, 10, ContentKind::Text).await.unwrap();
    let reloaded = backend
        .manifest(first.resource().resource_id, None)
        .await
        .unwrap();
    assert!(Arc::ptr_eq(&held, &reloaded));
    first.append(text("after eviction")).await.unwrap();
    let complete = first.finish().await.unwrap();
    assert_eq!(held.lock().await.manifest.resource, complete);
    second.finish().await.unwrap();
}

#[tokio::test]
async fn sparse_archive_restores_into_an_independent_backend_with_loss_and_ranges() {
    let root = tempfile::tempdir().unwrap();
    let hub = ContentHub::new(
        Arc::new(FileContentBackend::new(
            root.path(),
            FileContentConfig {
                segment_bytes: 200,
                retained_segments: 3,
                cached_manifests: 2,
                ..Default::default()
            },
        )),
        quiet_config(),
    );
    let writer = hub.open(7, 9, ContentKind::Text).await.unwrap();
    for index in 0..6 {
        writer
            .append(text(&format!("{index}{}", "x".repeat(119))))
            .await
            .unwrap();
    }
    writer.record_loss(7, &"source loss");
    let original = writer.finish().await.unwrap();
    let mut archive = hub.export_snapshot(original.resource_id).await.unwrap();
    assert_eq!(
        archive
            .chunks
            .iter()
            .map(|chunk| chunk.cursor.sequence)
            .collect::<Vec<_>>(),
        vec![1, 5, 6]
    );
    let destination = tempfile::tempdir().unwrap();
    let id = ContentId::new();
    archive.remap(id, 11, 15);
    let backend = FileContentBackend::new(destination.path(), Default::default());
    backend.restore(&archive).await.unwrap();
    assert!(
        backend.restore(&archive).await.is_err(),
        "a restore cannot overwrite existing ownership"
    );
    drop(backend);
    let reopened = FileContentBackend::new(destination.path(), Default::default());
    let page = reopened
        .read(
            id,
            Some(ContentCursor {
                sequence: 0,
                ..archive.resource.cursor
            }),
            1024,
        )
        .await
        .unwrap();
    assert_eq!(page.resource, archive.resource);
    assert_eq!(page.chunks, archive.chunks);
    assert!(page.gap);
    assert_eq!(page.resource.dropped_bytes, 7);
    assert_eq!(page.resource.retained_ranges, original.retained_ranges);
}

#[tokio::test]
async fn committed_content_survives_reopening_with_a_new_hub() {
    let root = tempfile::tempdir().unwrap();
    let backend = Arc::new(FileContentBackend::new(root.path(), Default::default()));
    let hub = ContentHub::new(backend, quiet_config());
    let writer = hub.open(7, 9, ContentKind::Text).await.unwrap();
    writer.append(text("  中文\n\n")).await.unwrap();
    writer.append(text("partial line")).await.unwrap();
    let complete = writer.finish().await.unwrap();
    let reopened = ContentHub::new(
        Arc::new(FileContentBackend::new(root.path(), Default::default())),
        quiet_config(),
    );
    let page = reopened
        .read(
            complete.resource_id,
            Some(ContentCursor {
                sequence: 0,
                ..complete.cursor
            }),
            1024,
        )
        .await
        .unwrap();
    assert_eq!(page.resource, complete);
    assert_eq!(
        page.chunks
            .iter()
            .map(|chunk| chunk.payload.clone())
            .collect::<Vec<_>>(),
        vec![
            text("  中文\n\n").into_payload(None).unwrap(),
            text("partial line").into_payload(None).unwrap()
        ]
    );
    assert!(!page.gap);
    assert!(!page.has_more);
}

#[tokio::test]
async fn retention_preserves_startup_and_recent_segments_with_explicit_ranges() {
    let root = tempfile::tempdir().unwrap();
    let backend = Arc::new(FileContentBackend::new(
        root.path(),
        FileContentConfig {
            segment_bytes: 200,
            retained_segments: 3,
            cached_manifests: 2,
            ..Default::default()
        },
    ));
    let hub = ContentHub::new(backend, quiet_config());
    let writer = hub.open(7, 9, ContentKind::Text).await.unwrap();
    for index in 0..5 {
        writer
            .append(text(&format!("{index}{}", "x".repeat(119))))
            .await
            .unwrap();
    }
    let complete = writer.finish().await.unwrap();
    let page = hub
        .read(
            complete.resource_id,
            Some(ContentCursor {
                sequence: 0,
                ..complete.cursor
            }),
            1024,
        )
        .await
        .unwrap();
    assert_eq!(
        page.resource.retained_ranges,
        vec![
            ContentRange { first: 1, last: 1 },
            ContentRange { first: 4, last: 5 }
        ]
    );
    assert_eq!(
        page.chunks
            .iter()
            .map(|chunk| chunk.cursor.sequence)
            .collect::<Vec<_>>(),
        vec![1, 4, 5]
    );
    assert!(page.gap);
    let tail = hub.read(complete.resource_id, None, 120).await.unwrap();
    assert_eq!(tail.chunks.len(), 1);
    assert_eq!(tail.chunks[0].cursor.sequence, 5);
    assert!(!tail.gap);
}

#[tokio::test]
async fn retry_truncates_an_unpublished_file_tail_without_duplicating_records() {
    let root = tempfile::tempdir().unwrap();
    let backend = Arc::new(FileContentBackend::new(root.path(), Default::default()));
    let hub = ContentHub::new(backend.clone(), quiet_config());
    let writer = hub.open(7, 9, ContentKind::Text).await.unwrap();
    writer.append(text("first")).await.unwrap();
    let mut resource = writer.resource();
    resource.committed_cursor = resource.cursor;
    let first = Arc::new(ContentChunk {
        captured_at_ms: 1000,
        cursor: resource.cursor,
        payload: text("first").into_payload(None).unwrap(),
    });
    backend.commit(resource.clone(), &[first]).await.unwrap();
    let directory = backend.directory(resource.resource_id);
    let path = FileContentBackend::segment_path(&directory, 1);
    let mut file = tokio::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .await
        .unwrap();
    file.write_all(b"unpublished and incomplete JSON")
        .await
        .unwrap();
    file.sync_data().await.unwrap();
    drop(file);
    writer.append(text("second")).await.unwrap();
    let complete = writer.finish().await.unwrap();
    let page = hub
        .read(
            complete.resource_id,
            Some(ContentCursor {
                sequence: 0,
                ..complete.cursor
            }),
            1024,
        )
        .await
        .unwrap();
    assert_eq!(page.chunks.len(), 2);
    assert_eq!(
        page.chunks[0].payload,
        text("first").into_payload(None).unwrap()
    );
    assert_eq!(
        page.chunks[1].payload,
        text("second").into_payload(None).unwrap()
    );
    assert!(!page.gap);
}

#[tokio::test]
async fn missing_committed_bytes_are_reported_as_an_error() {
    let root = tempfile::tempdir().unwrap();
    let backend = Arc::new(FileContentBackend::new(root.path(), Default::default()));
    let hub = ContentHub::new(backend.clone(), quiet_config());
    let writer = hub.open(7, 9, ContentKind::Text).await.unwrap();
    writer.append(text("committed output")).await.unwrap();
    let complete = writer.finish().await.unwrap();
    let path = FileContentBackend::segment_path(&backend.directory(complete.resource_id), 1);
    tokio::fs::OpenOptions::new()
        .write(true)
        .open(path)
        .await
        .unwrap()
        .set_len(1)
        .await
        .unwrap();
    assert!(matches!(
        hub.read(complete.resource_id, None, 1024).await,
        Err(StoreError::Io(_))
    ));
}

#[tokio::test]
async fn current_owner_absence_is_interruption_not_a_live_history_claim() {
    let root = tempfile::tempdir().unwrap();
    let backend = Arc::new(FileContentBackend::new(root.path(), Default::default()));
    let hub = ContentHub::new(backend.clone(), quiet_config());
    let writer = hub.open(7, 9, ContentKind::Text).await.unwrap();
    writer.append(text("before restart")).await.unwrap();
    let mut resource = writer.resource();
    resource.committed_cursor = resource.cursor;
    backend
        .commit(
            resource.clone(),
            &[Arc::new(ContentChunk {
                captured_at_ms: 1000,
                cursor: resource.cursor,
                payload: text("before restart").into_payload(None).unwrap(),
            })],
        )
        .await
        .unwrap();
    let reopened = ContentHub::new(
        Arc::new(FileContentBackend::new(root.path(), Default::default())),
        quiet_config(),
    );
    let page = reopened
        .read(resource.resource_id, None, 1024)
        .await
        .unwrap();
    assert_eq!(page.resource.state, ContentState::Interrupted);
    assert_eq!(
        page.chunks[0].payload,
        text("before restart").into_payload(None).unwrap()
    );
    writer.finish().await.unwrap();
}
