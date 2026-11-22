use agena_domain::{SessionOwner, SessionSurface};
use agena_storage::{WorkspaceListQuery, kernel::*};
use agena_storage_sqlite::{SqliteKernel, open_kernel_runtime};
use sea_orm::TransactionTrait;
use std::time::Duration;

async fn conversation(rt: &SqliteKernel, workspace: i64, bot: bool) -> SessionId {
    rt.ports
        .sessions
        .create_configured(
            RequestId::new(),
            workspace,
            if bot {
                "Bot conversation"
            } else {
                "Project conversation"
            }
            .into(),
            SessionConfiguration {
                owner: bot.then(|| SessionOwner {
                    bot: uuid::Uuid::new_v4(),
                    surface: SessionSurface::Group,
                    scope_key: None,
                }),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .value
        .id
}

#[tokio::test]
async fn project_catalog_hides_bot_directories_before_search_and_pagination() {
    let rt = open_kernel_runtime("sqlite::memory:").await.unwrap();
    let empty = rt.workspaces.ensure_id("/projects/empty").await.unwrap();
    let mixed = rt.workspaces.ensure_id("/projects/mixed").await.unwrap();
    conversation(&rt, mixed, false).await;
    conversation(&rt, mixed, true).await;
    let private = rt
        .workspaces
        .ensure_id("/projects/bot-private")
        .await
        .unwrap();
    let bot = conversation(&rt, private, true).await;
    let stats = rt
        .ports
        .reader
        .workspace_statistics(&[empty, mixed, private])
        .await
        .unwrap();
    assert_eq!(stats[&empty].total, 0);
    assert_eq!(stats[&mixed].total, 1);
    assert_eq!(stats[&mixed].roots, 1);
    assert_eq!(stats[&private].total, 0);

    let page = rt
        .workspaces
        .list(WorkspaceListQuery {
            limit: 1,
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(page.len(), 1);
    assert_eq!(page[0].id, mixed);
    let next = rt
        .workspaces
        .list(WorkspaceListQuery {
            before_id: Some(page[0].id),
            before_updated_at_ms: Some(page[0].updated_at_ms),
            limit: 1,
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(next.iter().map(|row| row.id).collect::<Vec<_>>(), [empty]);
    let offset = rt
        .workspaces
        .list(WorkspaceListQuery {
            offset: 1,
            limit: 1,
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(offset, next);
    let search = rt
        .workspaces
        .list(WorkspaceListQuery {
            search: Some("bot-private".into()),
            limit: 10,
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(search.is_empty());

    // The same workspace remains available to the Bot's execution tools.
    assert_eq!(
        rt.workspaces.get(private).await.unwrap().unwrap().id,
        private
    );
    assert_eq!(
        rt.workspaces.path_by_id(private).await.unwrap().as_deref(),
        Some("/projects/bot-private")
    );
    assert_eq!(
        rt.workspaces
            .ensure_id("/projects/bot-private")
            .await
            .unwrap(),
        private
    );
    assert_eq!(
        rt.ports.reader.summary(bot).await.unwrap().workspace_id,
        Some(private)
    );
}

#[tokio::test]
async fn file_workspace_and_instance_reads_complete_with_writer_occupied() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("instance.db");
    let rt = open_kernel_runtime(&format!("sqlite://{}?mode=rwc", path.display()))
        .await
        .unwrap();
    let workspace = rt.workspaces.ensure_id("/projects/demo").await.unwrap();
    rt.instance_state
        .set_payload("test", "{}".into())
        .await
        .unwrap();

    let writer = rt.database.begin().await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        assert_eq!(
            rt.workspaces.lookup_id("/projects/demo").await.unwrap(),
            Some(workspace)
        );
        assert_eq!(
            rt.workspaces.get(workspace).await.unwrap().unwrap().path,
            "/projects/demo"
        );
        assert_eq!(
            rt.instance_state
                .get_payload("test")
                .await
                .unwrap()
                .as_deref(),
            Some("{}")
        );
    })
    .await
    .expect("repository reads must not wait for the writer connection");
    writer.rollback().await.unwrap();
    rt.ports.shutdown().await.unwrap();
}

#[tokio::test]
async fn catalog_ownership_survives_deletion_and_preserves_project_history() {
    let rt = open_kernel_runtime("sqlite::memory:").await.unwrap();
    let private = rt.workspaces.ensure_id("/bot-workspace").await.unwrap();
    let bot = conversation(&rt, private, true).await;
    rt.ports
        .sessions
        .delete(RequestId::new(), bot)
        .await
        .unwrap();
    let rows = rt
        .workspaces
        .list(WorkspaceListQuery {
            limit: 10,
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(
        rows.is_empty(),
        "a deleted Bot must not leave an empty project behind"
    );

    let project = conversation(&rt, private, false).await;
    let rows = rt
        .workspaces
        .list(WorkspaceListQuery {
            limit: 10,
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(rows.iter().map(|row| row.id).collect::<Vec<_>>(), [private]);
    rt.ports
        .sessions
        .delete(RequestId::new(), project)
        .await
        .unwrap();
    let rows = rt
        .workspaces
        .list(WorkspaceListQuery {
            limit: 10,
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(rows.iter().map(|row| row.id).collect::<Vec<_>>(), [private]);
}
