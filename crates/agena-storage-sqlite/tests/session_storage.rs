use std::{path::Path, time::Duration};

use agena_domain::{CommandOutputStream, ComposerDocument, ComposerNode};
use agena_storage::kernel::*;
use agena_storage_layout::StorageLayout;
use agena_storage_sqlite::{SqliteKernel, open_kernel_runtime};
use sea_orm::sqlx;
use serde_json::json;

struct Fixture {
    _directory: tempfile::TempDir,
    layout: StorageLayout,
    url: String,
    runtime: SqliteKernel,
}

impl Fixture {
    async fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let layout = StorageLayout::new(directory.path());
        let url = format!("sqlite://{}?mode=rwc", layout.database_path().display());
        let runtime = open_kernel_runtime(&url).await.unwrap();
        Self {
            _directory: directory,
            layout,
            url,
            runtime,
        }
    }

    async fn create(&self, name: &str) -> SessionId {
        let workspace = self
            .runtime
            .workspaces
            .ensure_id("/storage-project")
            .await
            .unwrap();
        self.runtime
            .ports
            .sessions
            .create(RequestId::new(), workspace, name.into())
            .await
            .unwrap()
            .value
            .id
    }

    fn write(&self, parent: &Path, name: &str, body: &[u8]) {
        std::fs::create_dir_all(parent).unwrap();
        std::fs::write(parent.join(name), body).unwrap();
    }

    async fn claim(&self, session: SessionId) -> AttemptHandle {
        let execution = self
            .runtime
            .ports
            .sessions
            .send(
                RequestId::new(),
                session,
                ComposerDocument(vec![ComposerNode::Text {
                    text: "处理测试数据".into(),
                }]),
            )
            .await
            .unwrap()
            .value;
        self.runtime
            .ports
            .executor
            .claim_next(RequestId::new(), session, execution.id)
            .await
            .unwrap()
            .value
    }

    async fn start_tool(&self, session: SessionId) -> (AttemptHandle, RoundHandle, ToolHandle) {
        let attempt = self.claim(session).await;
        let round = attempt
            .begin_current_round(RequestId::new())
            .await
            .unwrap()
            .value;
        let tool = round
            .begin_tool(
                RequestId::new(),
                ToolStart {
                    call_id: "test-call".into(),
                    name: "test.command".into(),
                    input: json!({"text": "运行任务"}),
                },
            )
            .await
            .unwrap()
            .value;
        (attempt, round, tool)
    }
}

async fn settled(storage: &SessionStorage, session: SessionId) -> StorageOperation {
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let job = storage.operation(session).await.unwrap().unwrap();
            if job.state != StorageOperationState::Running {
                assert_eq!(job.state, StorageOperationState::Completed, "{job:?}");
                return job;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("cleanup did not reach a terminal state")
}

async fn confirmation(
    storage: &SessionStorage,
    session: SessionId,
    selection: StorageSelection,
) -> (StorageCleanupPreview, ConfirmStorageCleanup) {
    let preview = storage.preview(session, selection.clone()).await.unwrap();
    let confirmation = ConfirmStorageCleanup {
        token: preview.token.clone(),
        selection,
    };
    (preview, confirmation)
}

async fn detail(storage: &SessionStorage, session: SessionId) -> SessionStorageDetail {
    storage.detail(session, Default::default()).await.unwrap()
}

#[tokio::test]
async fn inventory_previews_and_resumes_scoped_file_cleanup() {
    let fixture = Fixture::new().await;
    let session = fixture.create("Files for demo").await;
    let other = fixture.create("Other").await;
    let cache = fixture.layout.session_web_cache_dir(session.get());
    let uploads = fixture.layout.session_uploads_dir(session.get());
    let other_uploads = fixture.layout.session_uploads_dir(other.get());
    fixture.write(&cache, "metadata.redb", b"1234");
    fixture.write(&cache.join("documents"), "a.txt", b"cached");
    fixture.write(&uploads, "draft.txt", b"draft");
    fixture.write(&other_uploads, "other.txt", b"another conversation");
    fixture.write(
        &fixture.layout.artifacts_dir(Path::new("/storage-project")),
        "legacy.png",
        b"shared file",
    );

    let storage = fixture.runtime.ports.storage();
    let list = storage
        .list(SessionStorageQuery {
            search: Some("FILES".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(list.total, 1);
    assert_eq!(list.items[0].session_id, session);
    assert_eq!(list.items[0].bytes.cache, 10);
    assert_eq!(list.items[0].bytes.files, 5);
    assert_eq!(list.shared.workspace_files, 11);

    let first = storage
        .detail(
            session,
            SessionStorageDetailQuery {
                file_offset: Some(0),
                file_limit: Some(1),
            },
        )
        .await
        .unwrap();
    let second = storage
        .detail(
            session,
            SessionStorageDetailQuery {
                file_offset: Some(1),
                file_limit: Some(1),
            },
        )
        .await
        .unwrap();
    assert_eq!(first.file_count, 3);
    assert_ne!(first.files[0].id, second.files[0].id);

    let selection = StorageSelection {
        cache: true,
        ..Default::default()
    };
    let snapshot = fixture
        .runtime
        .ports
        .reader
        .history_snapshot(session)
        .await
        .unwrap();
    assert_eq!(
        storage
            .preview(session, selection.clone())
            .await
            .unwrap()
            .protection,
        Some(StorageProtection::Reading)
    );
    drop(snapshot);
    tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            if storage
                .preview(session, selection.clone())
                .await
                .unwrap()
                .protection
                .is_none()
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    })
    .await
    .unwrap();

    let (preview, stale) = confirmation(&storage, session, selection.clone()).await;
    assert_eq!(preview.file_bytes, 10);
    fixture.write(&cache, "metadata.redb", b"updated cache");
    assert!(matches!(
        storage.confirm(session, stale).await,
        Err(KernelError::Conflict(_))
    ));
    assert!(cache.join("documents/a.txt").is_file());

    let (preview, confirm) = confirmation(&storage, session, selection).await;
    assert_eq!(preview.file_bytes, 19);
    let job = storage.confirm(session, confirm.clone()).await.unwrap();
    assert_eq!(job.state, StorageOperationState::Running);
    fixture.runtime.ports.clone().shutdown().await.unwrap();
    let Fixture {
        _directory,
        url,
        runtime,
        ..
    } = fixture;
    drop((storage, runtime));
    let reopened = open_kernel_runtime(&url).await.unwrap();
    let storage = reopened.ports.storage();

    let job = settled(&storage, session).await;
    assert_eq!(job.file_bytes_removed, 19);
    assert_eq!(job.files_removed, 2);
    assert!(!cache.exists());
    assert_eq!(storage.confirm(session, confirm).await.unwrap().id, job.id);
    assert_eq!(detail(&storage, session).await.summary.bytes.cache, 0);
    assert!(other_uploads.join("other.txt").is_file());

    let file = detail(&storage, session).await.files.pop().unwrap();
    let (_, confirm) = confirmation(
        &storage,
        session,
        StorageSelection {
            files: vec![file.id],
            ..Default::default()
        },
    )
    .await;
    storage.confirm(session, confirm).await.unwrap();
    assert_eq!(settled(&storage, session).await.file_bytes_removed, 5);
    assert!(!uploads.join("draft.txt").exists());
    reopened.ports.clone().shutdown().await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn file_inventory_ignores_links_and_a_swapped_directory_cannot_remove_external_files() {
    use std::os::unix::fs::symlink;

    let fixture = Fixture::new().await;
    let session = fixture.create("Assets").await;
    let uploads = fixture.layout.session_uploads_dir(session.get());
    let outside = fixture.layout.root().join("outside");
    fixture.write(&uploads, "draft.txt", b"safe to remove");
    fixture.write(&outside, "draft.txt", b"keep this");
    symlink(outside.join("draft.txt"), uploads.join("link")).unwrap();
    let storage = fixture.runtime.ports.storage();
    let owned = detail(&storage, session).await.files;
    assert_eq!(owned.len(), 1);
    let selection = StorageSelection {
        files: vec![owned[0].id.clone()],
        ..Default::default()
    };
    let (_, confirm) = confirmation(&storage, session, selection).await;
    std::fs::rename(&uploads, fixture.layout.root().join("original-uploads")).unwrap();
    symlink(&outside, &uploads).unwrap();

    assert!(matches!(
        storage.confirm(session, confirm).await,
        Err(KernelError::Conflict(_))
    ));
    assert_eq!(detail(&storage, session).await.file_count, 0);
    assert_eq!(
        std::fs::read(outside.join("draft.txt")).unwrap(),
        b"keep this"
    );
    fixture.runtime.ports.clone().shutdown().await.unwrap();
}

#[tokio::test]
async fn purge_keeps_shared_history_then_reclaims_payloads_and_database_file_bytes() {
    let fixture = Fixture::new().await;
    let parent = fixture.create("Parent").await;
    let (attempt, round, tool) = fixture.start_tool(parent).await;
    let entry = fixture
        .runtime
        .ports
        .reader
        .tool(parent, tool.id())
        .await
        .unwrap()
        .entry_id;
    let log = tool.begin_log(RequestId::new()).await.unwrap().value;
    let frozen_output = "frozen output".repeat(6_000);
    log.append(CommandOutputStream::Stdout, &frozen_output)
        .await
        .unwrap();
    log.finish(RequestId::new()).await.unwrap();

    let bytes = vec![42u8; 3 * 1024 * 1024];
    let blob = fixture
        .runtime
        .ports
        .sessions
        .prepare_blob(
            RequestId::new(),
            parent,
            bytes.len() as u64,
            &mut bytes.as_slice(),
        )
        .await
        .unwrap();
    let blob_id = blob.descriptor().id;
    tool.complete_with_blobs(
        RequestId::new(),
        ToolResult {
            phase: ToolPhase::Succeeded,
            value: json!({"blob": blob_id}),
        },
        &[blob],
    )
    .await
    .unwrap();
    round.finish(RequestId::new(), None, true).await.unwrap();
    attempt
        .finish(RequestId::new(), ExecutionPhase::Succeeded)
        .await
        .unwrap();
    let child = fixture
        .runtime
        .ports
        .sessions
        .fork(RequestId::new(), parent, entry)
        .await
        .unwrap()
        .value
        .id;
    let nested = fixture
        .runtime
        .ports
        .sessions
        .fork(RequestId::new(), child, entry)
        .await
        .unwrap()
        .value
        .id;
    let storage = fixture.runtime.ports.storage();
    assert_eq!(
        detail(&storage, parent).await.summary.bytes.media,
        bytes.len() as u64
    );
    assert_eq!(detail(&storage, child).await.summary.bytes.media, 0);
    let pool = fixture.runtime.database.get_sqlite_connection_pool();
    let shared_head: (i64, Option<i64>) = sqlx::query_as(
        "SELECT head_owner_session_id,head_bytes FROM agena_branch_sources WHERE session_id=?",
    )
    .bind(nested.get())
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(shared_head, (child.get(), None));
    let purge = StorageSelection {
        purge: true,
        ..Default::default()
    };

    let reader = fixture
        .runtime
        .ports
        .reader
        .open_blob(parent, blob_id)
        .await
        .unwrap();
    assert_eq!(
        storage
            .preview(parent, purge.clone())
            .await
            .unwrap()
            .protection,
        Some(StorageProtection::Reading)
    );
    drop(reader);
    let (preview, confirm) = confirmation(&storage, parent, purge.clone()).await;
    assert_eq!(preview.protection, None);
    assert!(preview.keeps_shared_history);
    assert_eq!(preview.database_bytes, 0);
    storage.confirm(parent, confirm).await.unwrap();
    assert!(settled(&storage, parent).await.keeps_shared_history);
    let inherited = fixture
        .runtime
        .ports
        .reader
        .open_blob(child, blob_id)
        .await
        .unwrap();
    assert_eq!(inherited.range(0, 32).await.unwrap().bytes, bytes[..32]);
    let inherited_log = fixture
        .runtime
        .ports
        .reader
        .event_page(child, log.id(), None, 16)
        .await
        .unwrap();
    assert_eq!(inherited_log.records.len(), 1);
    assert!(matches!(
        &inherited_log.records[0].payload,
        EventPayload::Log { text, .. } if text == &frozen_output
    ));
    drop((inherited, inherited_log));

    let (preview, confirm) = confirmation(&storage, child, purge.clone()).await;
    assert!(preview.keeps_shared_history);
    storage.confirm(child, confirm).await.unwrap();
    settled(&storage, child).await;
    let page = fixture
        .runtime
        .ports
        .reader
        .event_page(nested, log.id(), None, 16)
        .await
        .unwrap();
    assert_eq!(page.records.len(), 1);
    drop(page);

    let (_, confirm) = confirmation(&storage, nested, purge.clone()).await;
    storage.confirm(nested, confirm).await.unwrap();
    settled(&storage, nested).await;
    let remaining_heads: i64 =
        sqlx::query_scalar("SELECT count(*) FROM agena_branch_sources WHERE session_id IN (?,?)")
            .bind(child.get())
            .bind(nested.get())
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(remaining_heads, 0);

    // An independent delivery can still reference the origin's entry even
    // when no branch remains. This must guard the log and blob before any batch.
    let live = fixture.create("Delivery owner").await;
    sqlx::query("INSERT INTO agena_deliveries (id,session_id,business_key,payload_json,state,result_entry_id) VALUES (?,?,?,'{}','consumed',?)")
        .bind(agena_domain::CommandId::new().to_string()).bind(live.get()).bind("cross-owner-result")
        .bind(entry.get()).execute(pool).await.unwrap();
    let (preview, confirm) = confirmation(&storage, parent, purge.clone()).await;
    assert!(preview.keeps_shared_history);
    storage.confirm(parent, confirm).await.unwrap();
    settled(&storage, parent).await;
    assert_eq!(
        detail(&storage, parent).await.summary.bytes.media,
        bytes.len() as u64
    );
    sqlx::query("DELETE FROM agena_deliveries WHERE business_key='cross-owner-result'")
        .execute(pool)
        .await
        .unwrap();

    let (preview, confirm) = confirmation(&storage, parent, purge).await;
    assert!(!preview.keeps_shared_history);
    assert!(preview.database_bytes >= bytes.len() as u64);
    storage.confirm(parent, confirm).await.unwrap();
    let job = settled(&storage, parent).await;
    assert_eq!(job.database_bytes_removed, preview.database_bytes);
    assert!(
        sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(pool)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(detail(&storage, parent).await.summary.bytes.media, 0);
    let remaining_pieces: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM agena_content_block_pieces")
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(remaining_pieces, 0);
    let current = storage
        .list(SessionStorageQuery {
            refresh: Some(true),
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(
        current
            .items
            .iter()
            .all(|row| ![parent, child, nested].contains(&row.session_id))
    );
    assert!(current.database.reusable_bytes >= 2 * 1024 * 1024);
    let compacted = storage.reclaim_database().await.unwrap();
    assert!(compacted.released_bytes >= 2 * 1024 * 1024);
    assert_eq!(compacted.database.reusable_bytes, 0);
    assert!(
        compacted.database.file_bytes < current.database.file_bytes + current.database.wal_bytes
    );
    fixture.runtime.ports.clone().shutdown().await.unwrap();
}

#[tokio::test]
async fn purge_reclaims_frozen_prompt_blocks_after_their_last_round() {
    let fixture = Fixture::new().await;
    let origin = fixture.create("Prompt source").await;
    let (attempt, source, tool) = fixture.start_tool(origin).await;
    let tool_entry = fixture
        .runtime
        .ports
        .reader
        .tool(origin, tool.id())
        .await
        .unwrap()
        .entry_id;
    tool.complete(
        RequestId::new(),
        ToolResult {
            phase: ToolPhase::Succeeded,
            value: json!({"result": "retained"}),
        },
    )
    .await
    .unwrap();
    let provider_state = json!({"replay": "retained"});
    source
        .finish(RequestId::new(), Some(provider_state.clone()), true)
        .await
        .unwrap();
    attempt
        .finish(RequestId::new(), ExecutionPhase::Succeeded)
        .await
        .unwrap();

    let mut consumers = Vec::new();
    for _ in 0..2 {
        let session = fixture
            .runtime
            .ports
            .sessions
            .fork(RequestId::new(), origin, tool_entry)
            .await
            .unwrap()
            .value
            .id;
        let attempt = fixture.claim(session).await;
        let round = attempt
            .begin_round(RequestId::new(), vec![tool_entry])
            .await
            .unwrap()
            .value;
        round.finish(RequestId::new(), None, true).await.unwrap();
        attempt
            .finish(RequestId::new(), ExecutionPhase::Succeeded)
            .await
            .unwrap();
        consumers.push((session, round.id()));
    }
    let [(first, first_round), (last, last_round)] = consumers.try_into().unwrap();
    let pool = fixture.runtime.database.get_sqlite_connection_pool();
    let (input_block, replay_block): (i64, i64) = sqlx::query_as(
        "SELECT l.block_id,p.block_id FROM agena_round_input_blocks l
         JOIN agena_round_replay_blocks p ON p.round_id=l.round_id
         WHERE l.round_id=?",
    )
    .bind(first_round.get())
    .fetch_one(pool)
    .await
    .unwrap();
    let other: (i64, i64) = sqlx::query_as(
        "SELECT l.block_id,p.block_id FROM agena_round_input_blocks l
         JOIN agena_round_replay_blocks p ON p.round_id=l.round_id
         WHERE l.round_id=?",
    )
    .bind(last_round.get())
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(other, (input_block, replay_block));

    let storage = fixture.runtime.ports.storage();
    let purge = StorageSelection {
        purge: true,
        ..Default::default()
    };
    let (preview, confirm) = confirmation(&storage, first, purge.clone()).await;
    assert!(!preview.keeps_shared_history);
    storage.confirm(first, confirm).await.unwrap();
    settled(&storage, first).await;
    let prompt = fixture
        .runtime
        .ports
        .reader
        .prompt_of(last_round)
        .await
        .unwrap();
    assert_eq!(prompt.entries[0].entry.id, tool_entry);
    assert_eq!(
        prompt.replays[0].provider_state.as_ref(),
        Some(&provider_state)
    );
    drop(prompt);

    let (_, confirm) = confirmation(&storage, last, purge).await;
    storage.confirm(last, confirm).await.unwrap();
    settled(&storage, last).await;
    let blocks_remain: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM agena_prompt_input_blocks WHERE id=?)
          OR EXISTS(SELECT 1 FROM agena_prompt_replay_blocks WHERE id=?)",
    )
    .bind(input_block)
    .bind(replay_block)
    .fetch_one(pool)
    .await
    .unwrap();
    assert!(!blocks_remain);
    assert!(
        sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(pool)
            .await
            .unwrap()
            .is_empty()
    );
    fixture.runtime.ports.clone().shutdown().await.unwrap();
}

#[tokio::test]
async fn recording_cleanup_keeps_the_tail_and_tool_results() {
    let fixture = Fixture::new().await;
    let session = fixture.create("Long command").await;
    let (attempt, round, tool) = fixture.start_tool(session).await;
    let storage = fixture.runtime.ports.storage();
    let selection = StorageSelection {
        recordings: true,
        ..Default::default()
    };
    assert_eq!(
        storage
            .preview(session, selection.clone())
            .await
            .unwrap()
            .protection,
        Some(StorageProtection::Running)
    );
    let log = tool.begin_log(RequestId::new()).await.unwrap().value;
    log.append(CommandOutputStream::Stdout, &"a".repeat(72 * 1024))
        .await
        .unwrap();
    let early = "a".repeat(4096);
    for _ in 0..179 {
        log.append(CommandOutputStream::Stdout, &early)
            .await
            .unwrap();
    }
    let pool = fixture.runtime.database.get_sqlite_connection_pool();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let committed: i64 =
                sqlx::query_scalar("SELECT committed_head FROM agena_content_streams WHERE id=?")
                    .bind(log.id().to_string())
                    .fetch_one(pool)
                    .await
                    .unwrap();
            if committed == 180 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    })
    .await
    .unwrap();
    let split_bytes: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(b.body_bytes),0) FROM agena_content_streams c
         JOIN agena_content_blocks b ON b.source_key=c.source_key WHERE c.id=? AND b.body IS NULL",
    )
    .bind(log.id().to_string())
    .fetch_one(pool)
    .await
    .unwrap();
    assert!(split_bytes > 0);
    for _ in 0..150 {
        log.append(CommandOutputStream::Stdout, "recent output")
            .await
            .unwrap();
    }
    log.finish(RequestId::new()).await.unwrap();
    tool.complete(
        RequestId::new(),
        ToolResult {
            phase: ToolPhase::Succeeded,
            value: json!({"answer": 7}),
        },
    )
    .await
    .unwrap();
    round.finish(RequestId::new(), None, true).await.unwrap();
    attempt
        .finish(RequestId::new(), ExecutionPhase::Succeeded)
        .await
        .unwrap();

    let (preview, confirm) = confirmation(&storage, session, selection).await;
    assert!(preview.database_bytes > 512 * 1024);
    storage.confirm(session, confirm).await.unwrap();
    assert_eq!(
        settled(&storage, session).await.database_bytes_removed,
        preview.database_bytes
    );
    let retained_pieces: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM agena_content_streams c
         JOIN agena_content_block_pieces p ON p.source_key=c.source_key WHERE c.id=?",
    )
    .bind(log.id().to_string())
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(retained_pieces, 0);
    let reader = &fixture.runtime.ports.reader;
    let boundary = 330 - 128;
    assert!(matches!(
        reader.event_page(session, log.id(), None, 256).await,
        Err(KernelError::ContentGap { retained_start, .. }) if retained_start == boundary
    ));
    let cursor = EventCursor {
        content: log.id(),
        epoch: log.epoch(),
        sequence: boundary,
    };
    let page = reader
        .event_page(session, log.id(), Some(cursor), 256)
        .await
        .unwrap();
    assert_eq!(page.stream.retained_start, boundary);
    assert!(
        matches!(page.records.last().map(|event| &event.payload), Some(EventPayload::Log { text, .. }) if text == "recent output")
    );
    let tool_view = fixture
        .runtime
        .ports
        .reader
        .tool(session, tool.id())
        .await
        .unwrap();
    assert_eq!(tool_view.result, Some(json!({"answer": 7})));
    fixture.runtime.ports.clone().shutdown().await.unwrap();
}

#[tokio::test]
async fn bot_totals_include_owned_sessions_and_binding_storage_once() {
    let fixture = Fixture::new().await;
    let id = uuid::Uuid::new_v4();
    let profile = agena_storage::BotRecord {
        id,
        name: "Research Bot".into(),
        name_folded: "research bot".into(),
        instructions: "整理资料".into(),
        agent_profile: None,
        prompt_ids: vec![],
        publish_prompts: false,
        avatar: None,
        revision: 1,
        archived: false,
        created_at_ms: 1,
        updated_at_ms: 1,
    };
    fixture.runtime.bots.create(&profile).await.unwrap();
    let session = fixture
        .runtime
        .ports
        .sessions
        .ensure_local_bot_session(RequestId::new(), id, "Research Bot session".into())
        .await
        .unwrap()
        .value
        .id;
    let project = fixture.create("Project session").await;
    fixture.write(
        &fixture.layout.session_artifacts_dir(session.get()),
        "image.png",
        b"image",
    );
    fixture.write(
        &fixture.layout.session_uploads_dir(project.get()),
        "unrelated.txt",
        b"another owner",
    );
    let binding = uuid::Uuid::new_v4().to_string();
    let pool = fixture.runtime.database.get_sqlite_connection_pool();
    sqlx::query("INSERT INTO agena_integration_bindings (id,owner_bot_id,source_session_id,request_key,revision,state,next_poll_ms,binding_json) VALUES (?,?,?,'binding',1,'enabled',0,'{}')")
        .bind(&binding).bind(id.to_string()).bind(session.get())
        .execute(pool).await.unwrap();
    let event = json!({"text": "频道输入"}).to_string();
    let note = "已处理";
    let inbox_id: i64 = sqlx::query_scalar(
        "INSERT INTO agena_integration_inbox
         (binding_id,event_id,epoch,state,received_at_ms,is_access,note)
         VALUES (?,'input',1,'handled',1,0,?) RETURNING id",
    )
    .bind(&binding)
    .bind(note)
    .fetch_one(pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO agena_integration_event_payloads VALUES (?,?)")
        .bind(inbox_id)
        .bind(&event)
        .execute(pool)
        .await
        .unwrap();
    let route = json!({"destination": "频道"}).to_string();
    let text = "频道输出";
    let sent: i64 = sqlx::query_scalar(
        "INSERT INTO agena_integration_outbox
         (id,binding_id,state,attempts,next_attempt_ms,created_at_ms,route_json)
         VALUES ('output',?,'sent',1,0,2,?) RETURNING delivery_key",
    )
    .bind(&binding)
    .bind(&route)
    .fetch_one(pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO agena_integration_delivery_text VALUES (?,?)")
        .bind(sent)
        .bind(text)
        .execute(pool)
        .await
        .unwrap();
    let error = "失去授权";
    sqlx::query(
        "INSERT INTO agena_integration_outbox
         (id,binding_id,state,attempts,next_attempt_ms,created_at_ms,error)
         VALUES ('empty',?,'skipped',0,0,3,?)",
    )
    .bind(&binding)
    .bind(error)
    .execute(pool)
    .await
    .unwrap();
    let data = fixture.layout.root().join("channel-scopes").join(&binding);
    fixture.write(&data, "context.txt", b"channel context");

    let storage = fixture.runtime.ports.storage();
    let inventory = storage
        .list(SessionStorageQuery {
            bot: Some(id.to_string()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(inventory.total, 1);
    assert_eq!(inventory.items[0].session_id, session);
    assert_eq!(inventory.bots[0].session_count, 1);
    assert_eq!(inventory.bots[0].session_bytes, inventory.bytes);
    assert_eq!(
        inventory.bots[0].profile_bytes,
        profile.instructions.len() as u64
    );
    assert_eq!(
        inventory.bots[0].channel_bytes,
        (2 + event.len() + note.len() + route.len() + text.len() + error.len()) as u64,
    );
    assert_eq!(inventory.bots[0].channel_files, 15);
    fixture.runtime.ports.clone().shutdown().await.unwrap();
}
