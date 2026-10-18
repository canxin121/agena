//! Real process ownership tests; every database belongs to this fixture.

use std::{
    io::{BufRead, Write},
    process::{Child, Command, Stdio},
    sync::Arc,
    time::Duration,
};

use agena_storage_sqlite::connect_owned_database;
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};

struct ChildOwner(Child);
impl Drop for ChildOwner {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn owner_is_released_after_process_death_without_removing_the_lock_file() {
    const CHILD_PATH: &str = "AGENA_KERNEL_OWNER_TEST_DATABASE";
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    if let Ok(url) = std::env::var(CHILD_PATH) {
        runtime.block_on(async {
            let database = connect_owned_database(&url, log::LevelFilter::Off)
                .await
                .unwrap();
            database
                .execute(Statement::from_string(
                    DatabaseBackend::Sqlite,
                    "CREATE TABLE owner_evidence (value TEXT NOT NULL)",
                ))
                .await
                .unwrap();
            database
                .execute(Statement::from_string(
                    DatabaseBackend::Sqlite,
                    "INSERT INTO owner_evidence VALUES ('committed-before-kill')",
                ))
                .await
                .unwrap();
            println!("KERNEL_OWNER_READY");
            std::io::stdout().flush().unwrap();
            // The parent kills this process only after observing the committed
            // boundary. No timer is used to guess whether ownership exists.
            loop {
                std::thread::park();
            }
        });
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("owned.db");
    let url = format!("sqlite://{}?mode=rwc", path.display());
    let mut child = ChildOwner(
        Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg("owner_is_released_after_process_death_without_removing_the_lock_file")
            .arg("--nocapture")
            .env(CHILD_PATH, &url)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap(),
    );
    let stdout = child.0.stdout.take().unwrap();
    let (sender, receiver) = std::sync::mpsc::channel();
    let reader = std::thread::spawn(move || {
        for line in std::io::BufReader::new(stdout).lines() {
            if line.unwrap().contains("KERNEL_OWNER_READY") {
                let _ = sender.send(());
                break;
            }
        }
    });
    receiver
        .recv_timeout(Duration::from_secs(15))
        .expect("child committed and holds ownership");
    reader.join().unwrap();
    runtime.block_on(async {
        let error = connect_owned_database(&url, log::LevelFilter::Off)
            .await
            .err()
            .unwrap();
        assert!(error.to_string().contains("already has a service owner"));
    });
    child.0.kill().unwrap();
    child.0.wait().unwrap();
    runtime.block_on(async {
        let database = connect_owned_database(&url, log::LevelFilter::Off)
            .await
            .unwrap();
        let value: String = database
            .query_one(Statement::from_string(
                DatabaseBackend::Sqlite,
                "SELECT value FROM owner_evidence",
            ))
            .await
            .unwrap()
            .unwrap()
            .try_get("", "value")
            .unwrap();
        assert_eq!(value, "committed-before-kill");
        database.close().await.unwrap();
    });
    assert!(dir.path().join("owned.db.owner-lock").exists());
}

#[tokio::test]
async fn canonical_aliases_and_pool_clones_retain_one_owner() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("owned.db");
    let url = format!("sqlite://{}?mode=rwc", path.display());
    let database = Arc::new(
        connect_owned_database(&url, log::LevelFilter::Off)
            .await
            .unwrap(),
    );
    let retained_pool = database.get_sqlite_connection_pool().clone();
    let other_url = format!("sqlite://{}/./owned.db?mode=rwc", dir.path().display());
    assert!(
        connect_owned_database(&other_url, log::LevelFilter::Off)
            .await
            .is_err()
    );
    #[cfg(unix)]
    {
        let alias = dir.path().join("alias.db");
        std::os::unix::fs::symlink(&path, &alias).unwrap();
        let alias_url = format!("sqlite://{}?mode=rwc", alias.display());
        assert!(
            connect_owned_database(&alias_url, log::LevelFilter::Off)
                .await
                .is_err()
        );
    }
    database.close_by_ref().await.unwrap();
    drop(database);
    assert!(
        connect_owned_database(&url, log::LevelFilter::Off)
            .await
            .is_err(),
        "a retained pool clone still owns the database"
    );
    drop(retained_pool);
    let reopened = connect_owned_database(&url, log::LevelFilter::Off)
        .await
        .unwrap();
    reopened.close().await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn hard_link_aliases_are_rejected_instead_of_creating_separate_wal_owners() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("owned.db");
    std::fs::File::create(&path).unwrap();
    let alias = dir.path().join("hard-link.db");
    std::fs::hard_link(&path, &alias).unwrap();
    for path in [&path, &alias] {
        let url = format!("sqlite://{}?mode=rwc", path.display());
        let error = connect_owned_database(&url, log::LevelFilter::Off)
            .await
            .err()
            .unwrap();
        assert!(
            error
                .to_string()
                .contains("hard-linked databases are unsupported")
        );
    }
}
