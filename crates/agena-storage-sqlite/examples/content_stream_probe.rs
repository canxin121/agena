//! Reproducible source-to-observer measurements over the production SQLite
//! facade and content files. This measures neither network nor UI paint.
use std::error::Error;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use std::time::{Duration, Instant};

use agena_domain::{
    ContentChunk, ContentCursor, ContentId, ContentInput, ContentKind, ContentPage,
    ContentResource, ContentState, SessionRelationKind,
};
use agena_storage::WorkspaceRepository;
use agena_storage::content::{ContentArchive, ContentBackend};
use agena_storage::store::{
    NewPart, NewSession, PartDelta, PartRole, PartState, SessionFacade, SessionStore, StoreError,
};
use agena_storage_sqlite::{
    DatabaseContentBackend, SeaWorkspaceRepository, SqliteEngine, initialize_schema,
};
use sea_orm::Database;
use serde_json::json;

const RECORDS: usize = 1000;

struct MeasuredBackend {
    inner: DatabaseContentBackend,
    commits: AtomicUsize,
    records: AtomicUsize,
}

#[async_trait::async_trait]
impl ContentBackend for MeasuredBackend {
    async fn commit(
        &self,
        resource: ContentResource,
        chunks: &[Arc<ContentChunk>],
    ) -> Result<ContentResource, StoreError> {
        let result = self.inner.commit(resource, chunks).await?;
        self.commits.fetch_add(1, Ordering::Relaxed);
        self.records.fetch_add(chunks.len(), Ordering::Relaxed);
        Ok(result)
    }
    async fn describe(&self, id: ContentId) -> Result<ContentResource, StoreError> {
        self.inner.describe(id).await
    }
    async fn read(
        &self,
        id: ContentId,
        after: Option<ContentCursor>,
        max_bytes: usize,
    ) -> Result<ContentPage, StoreError> {
        self.inner.read(id, after, max_bytes).await
    }
    async fn restore(&self, archive: &ContentArchive) -> Result<(), StoreError> {
        self.inner.restore(archive).await
    }
    async fn prune(
        &self,
        protected: &std::collections::HashSet<ContentId>,
    ) -> Result<usize, StoreError> {
        self.inner.prune(protected).await
    }
    async fn delete(&self, id: ContentId) -> Result<(), StoreError> {
        self.inner.delete(id).await
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    for observers in [1, 8] {
        measure(observers).await?;
    }
    Ok(())
}

async fn measure(observers: usize) -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let database_path = directory.path().join("probe.db");
    let mut connection =
        Database::connect(format!("sqlite://{}?mode=rwc", database_path.display())).await?;
    let sql_count = Arc::new(AtomicUsize::new(0));
    connection.set_metric_callback({
        let sql_count = sql_count.clone();
        move |_| {
            sql_count.fetch_add(1, Ordering::Relaxed);
        }
    });
    let db = Arc::new(connection);
    initialize_schema(&db).await?;
    let workspace_id = SeaWorkspaceRepository::new(db.clone())
        .ensure_id("/probe/workspace")
        .await?;
    let backend = Arc::new(MeasuredBackend {
        inner: DatabaseContentBackend::new(db.clone()),
        commits: AtomicUsize::new(0),
        records: AtomicUsize::new(0),
    });
    let contents = agena_storage::content::ContentHub::new(backend.clone(), Default::default());
    let store = SessionFacade::new(SqliteEngine::new(db), 32).with_contents(contents);
    let session_id = store
        .create_session(NewSession {
            workspace_id,
            parent_id: None,
            relation_kind: SessionRelationKind::Root,
            cutoff_part_id: None,
            title: "content probe".into(),
            task_id: None,
            config_json: None,
            provider_anchors_json: None,
        })
        .await?
        .id;
    let run = store
        .submit_user_run(
            session_id,
            vec![NewPart {
                state: PartState::InProgress,
                ..NewPart::pending("text", PartRole::Assistant, json!({"text":""}))
            }],
            None,
        )
        .await?;
    let part_id = run.parts[1].part_id;
    let writer = store
        .contents()
        .open(session_id, part_id, ContentKind::Text)
        .await?;
    let reference = writer.resource().reference();
    store
        .update_part(
            session_id,
            part_id,
            PartDelta {
                content: Some(json!({"text":"", "resources":[reference]})),
                ..Default::default()
            },
        )
        .await?;
    let hub = store.contents().clone();
    let starts = Arc::new(Mutex::new(Vec::<Instant>::with_capacity(RECORDS)));
    let mut readers = Vec::new();
    for _ in 0..observers {
        let mut changes = hub
            .subscribe(reference.resource_id)
            .expect("active resource");
        let hub = hub.clone();
        let starts = starts.clone();
        let epoch = writer.resource().cursor.epoch;
        readers.push(tokio::spawn(async move {
            let mut cursor = ContentCursor { epoch, sequence: 0 };
            let mut latencies = Vec::new();
            loop {
                let page = hub
                    .read(reference.resource_id, Some(cursor), 64 * 1024)
                    .await
                    .expect("observer page");
                assert!(!page.gap);
                let observed = Instant::now();
                for chunk in &page.chunks {
                    latencies.push(
                        observed
                            .duration_since(
                                starts.lock().unwrap()[chunk.cursor.sequence as usize - 1],
                            )
                            .as_secs_f64()
                            * 1000.0,
                    );
                }
                cursor = page.next_cursor;
                if page.resource.state != ContentState::Active && !page.has_more {
                    break;
                }
                if !page.has_more {
                    changes.changed().await.expect("source update");
                }
            }
            assert_eq!(cursor.sequence, RECORDS as u64);
            assert_eq!(latencies.len(), RECORDS);
            latencies
        }));
    }
    // Admission, Part attachment and backend identity discovery are setup.
    // Count every SQL operation through final resource flush after this point.
    sql_count.store(0, Ordering::Relaxed);
    backend.commits.store(0, Ordering::Relaxed);
    backend.records.store(0, Ordering::Relaxed);
    let mut peak_resident_bytes = hub.resident_bytes();
    let start = Instant::now();
    for index in 0..RECORDS {
        starts.lock().unwrap().push(Instant::now());
        writer
            .append(ContentInput::Text {
                text: format!("{index:04} {}\n", "x".repeat(1018)),
            })
            .await?;
        peak_resident_bytes = peak_resident_bytes.max(hub.resident_bytes());
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    let sealed = writer.finalize(ContentState::Complete).await?;
    let mut latencies = Vec::new();
    for reader in readers {
        latencies.extend(reader.await?);
    }
    let stream_sql = sql_count.load(Ordering::Relaxed);
    assert_eq!(
        stream_sql, 0,
        "streaming and observer reads must issue no SQL"
    );
    assert_eq!(sealed.cursor, sealed.committed_cursor);
    let committed_records = backend.records.load(Ordering::Relaxed);
    assert_eq!(committed_records, RECORDS);
    latencies.sort_unstable_by(f64::total_cmp);
    let segment_root = directory
        .path()
        .join("probe.db.content")
        .join(reference.resource_id.to_string());
    let segments = std::fs::read_dir(segment_root)?
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("records-"))
        .count();
    println!(
        "{}",
        json!({
            "scope":"SQLite facade / content files / ContentHub observers",
            "observers":observers, "source_records":RECORDS, "observer_samples":latencies.len(),
            "sql_during_stream_and_finalize":stream_sql, "retained_segment_files":segments,
            "backend_commits":backend.commits.load(Ordering::Relaxed), "committed_records":committed_records,
            "peak_accounted_live_bytes":peak_resident_bytes,
            "elapsed_ms":start.elapsed().as_secs_f64()*1000.0,
            "observer_p50_ms":latencies[latencies.len()/2],
            "observer_p95_ms":latencies[latencies.len()*95/100],
            "observer_max_ms":latencies[latencies.len()-1]
        })
    );
    Ok(())
}
