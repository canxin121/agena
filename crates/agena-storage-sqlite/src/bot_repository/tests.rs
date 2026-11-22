use super::*;
use crate::connection::KernelDatabase;

const RED: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGP4z8AAAAMBAQDJ/pLvAAAAAElFTkSuQmCC";
const BLUE: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGNgYPgPAAEDAQAIicLsAAAAAElFTkSuQmCC";

fn bot(name: &str, avatar: Option<&str>) -> BotRecord {
    BotRecord {
        id: Uuid::new_v4(),
        name: name.into(),
        name_folded: name.to_lowercase(),
        instructions: String::new(),
        prompt_ids: Vec::new(),
        publish_prompts: false,
        avatar: avatar.map(str::to_owned),
        revision: 1,
        archived: false,
        created_at_ms: 1,
        updated_at_ms: 1,
    }
}

#[tokio::test]
async fn avatar_and_revision_commit_atomically_and_survive_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let url = format!(
        "sqlite://{}?mode=rwc",
        directory.path().join("bots.sqlite").display()
    );
    let database = KernelDatabase::open(&url).await.unwrap();
    let repository = SqliteBotRepository::new(database.writer.clone());
    let original = repository.create(&bot("Avatar", Some(RED))).await.unwrap();
    assert_eq!(
        repository.list(false).await.unwrap(),
        [BotWithLocalSession {
            record: original.clone(),
            local_session_id: None
        }]
    );

    let invalid = bot("Invalid", Some("not-a-thumbnail"));
    assert!(repository.create(&invalid).await.is_err());
    assert_eq!(repository.count().await.unwrap(), 1);
    assert!(matches!(
        repository.get(invalid.id).await,
        Err(BotRepositoryError::NotFound)
    ));

    let updated = repository
        .update(
            &BotRecord {
                name: "New name".into(),
                name_folded: "new name".into(),
                avatar: Some(BLUE.into()),
                prompt_ids: vec!["first".into(), "second".into()],
                publish_prompts: true,
                updated_at_ms: 2,
                ..original.clone()
            },
            original.revision,
        )
        .await
        .unwrap();
    assert_eq!(updated.revision, 2);
    assert_eq!(updated.prompt_ids, ["first", "second"]);
    assert!(updated.publish_prompts);

    let stale = BotRecord {
        avatar: None,
        ..original.clone()
    };
    assert!(matches!(
        repository.update(&stale, original.revision).await,
        Err(BotRepositoryError::Conflict)
    ));
    assert_eq!(repository.get(original.id).await.unwrap(), updated);

    // A failed avatar write must roll back the metadata and revision update too.
    let invalid = BotRecord {
        name: "Must roll back".into(),
        name_folded: "must roll back".into(),
        avatar: Some("not-a-thumbnail".into()),
        updated_at_ms: 3,
        ..updated.clone()
    };
    assert!(repository.update(&invalid, updated.revision).await.is_err());
    assert_eq!(repository.get(original.id).await.unwrap(), updated);
    drop(repository);
    database.close().await.unwrap();

    let database = KernelDatabase::open(&url).await.unwrap();
    let repository = SqliteBotRepository::new(database.writer.clone());
    assert_eq!(repository.get(original.id).await.unwrap(), updated);
    let cleared = repository
        .update(
            &BotRecord {
                avatar: None,
                updated_at_ms: 3,
                ..updated.clone()
            },
            updated.revision,
        )
        .await
        .unwrap();
    assert_eq!(cleared.avatar, None);
    assert_eq!(
        repository.list(false).await.unwrap(),
        [BotWithLocalSession {
            record: cleared.clone(),
            local_session_id: None
        }]
    );

    let restored = repository
        .update(
            &BotRecord {
                avatar: Some(RED.into()),
                updated_at_ms: 4,
                ..cleared.clone()
            },
            cleared.revision,
        )
        .await
        .unwrap();
    let archived = repository
        .archive(restored.id, restored.revision, 5)
        .await
        .unwrap();
    assert_eq!(archived.avatar.as_deref(), Some(RED));
    assert!(archived.archived);
    assert!(repository.list(false).await.unwrap().is_empty());
    assert_eq!(
        repository.list(true).await.unwrap(),
        [BotWithLocalSession {
            record: archived,
            local_session_id: None
        }]
    );
    drop(repository);
    database.close().await.unwrap();
}
