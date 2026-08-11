use std::io;
use std::path::Path;

use uuid::Uuid;

const INSTALLATION_ID_FILENAME: &str = "installation_id";
static INSTALLATION_ID_FILE_WORKERS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(4);

/// Resolve the stable UUID assigned to this local Agena installation.
pub async fn resolve_installation_id() -> io::Result<String> {
    resolve_installation_id_in(&agena_runtime_tools::agena_home_dir()).await
}

async fn resolve_installation_id_in(base_dir: &Path) -> io::Result<String> {
    tokio::fs::create_dir_all(base_dir).await?;
    let path = base_dir.join(INSTALLATION_ID_FILENAME);
    let worker_permit = INSTALLATION_ID_FILE_WORKERS
        .acquire()
        .await
        .expect("the static installation-id semaphore is never closed");
    tokio::task::spawn_blocking(move || {
        let _worker_permit = worker_permit;
        agena_runtime_tools::with_file_mutation_locks(std::slice::from_ref(&path), || {
            match std::fs::read_to_string(&path) {
                Ok(contents) => {
                    let trimmed = contents.trim();
                    if !trimmed.is_empty()
                        && let Ok(existing) = Uuid::parse_str(trimmed)
                    {
                        return Ok(existing.to_string());
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }

            let installation_id = Uuid::new_v4().to_string();
            if path.exists() {
                agena_runtime_tools::atomic_replace_file(&path, installation_id.as_bytes())?;
            } else {
                match agena_runtime_tools::atomic_create_file(
                    &path,
                    installation_id.as_bytes(),
                    None,
                ) {
                    Ok(()) => {}
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                        let winner = std::fs::read_to_string(&path)?;
                        if let Ok(existing) = Uuid::parse_str(winner.trim()) {
                            return Ok(existing.to_string());
                        }
                        return Err(error);
                    }
                    Err(error) => return Err(error),
                }
            }
            Ok(installation_id)
        })?
    })
    .await
    .map_err(|error| io::Error::other(format!("installation-id worker failed: {error}")))?
}

#[cfg(test)]
mod tests {
    use super::resolve_installation_id_in;

    #[tokio::test]
    async fn reuses_valid_id_and_replaces_invalid_contents() {
        let directory = tempfile::tempdir().expect("create installation-id directory");
        let first = resolve_installation_id_in(directory.path())
            .await
            .expect("create installation id");
        let second = resolve_installation_id_in(directory.path())
            .await
            .expect("reuse installation id");
        assert_eq!(first, second);

        tokio::fs::write(directory.path().join("installation_id"), "not-a-uuid")
            .await
            .expect("write invalid installation id");
        let replacement = resolve_installation_id_in(directory.path())
            .await
            .expect("replace invalid installation id");
        assert_ne!(replacement, "not-a-uuid");
        assert!(uuid::Uuid::parse_str(&replacement).is_ok());
    }

    #[tokio::test]
    async fn concurrent_initialization_returns_one_stable_id() {
        let directory = tempfile::tempdir().expect("create installation-id directory");
        let (first, second) = tokio::join!(
            resolve_installation_id_in(directory.path()),
            resolve_installation_id_in(directory.path()),
        );
        assert_eq!(first.expect("first id"), second.expect("second id"));
    }
}
