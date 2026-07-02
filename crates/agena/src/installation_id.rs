use std::io;
use std::path::Path;

use tokio::fs;
use uuid::Uuid;

const INSTALLATION_ID_FILENAME: &str = "installation_id";

pub(crate) async fn resolve_installation_id() -> io::Result<String> {
    resolve_installation_id_in(&crate::project_paths::agena_home_dir()).await
}

async fn resolve_installation_id_in(base_dir: &Path) -> io::Result<String> {
    fs::create_dir_all(base_dir).await?;
    let path = base_dir.join(INSTALLATION_ID_FILENAME);

    match fs::read_to_string(&path).await {
        Ok(contents) => {
            let trimmed = contents.trim();
            if !trimmed.is_empty()
                && let Ok(existing) = Uuid::parse_str(trimmed)
            {
                return Ok(existing.to_string());
            }
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(err) => return Err(err),
    }

    let installation_id = Uuid::new_v4().to_string();
    fs::write(&path, installation_id.as_bytes()).await?;
    Ok(installation_id)
}

#[cfg(test)]
mod tests {
    use super::INSTALLATION_ID_FILENAME;
    use super::resolve_installation_id_in;
    use tempfile::TempDir;
    use uuid::Uuid;

    #[tokio::test]
    async fn resolve_installation_id_generates_and_persists_uuid() {
        let tempdir = TempDir::new().expect("create temp dir");
        let resolved = resolve_installation_id_in(tempdir.path())
            .await
            .expect("resolve installation id");

        assert!(Uuid::parse_str(&resolved).is_ok());
        assert_eq!(
            std::fs::read_to_string(tempdir.path().join(INSTALLATION_ID_FILENAME))
                .expect("read installation id"),
            resolved
        );
    }

    #[tokio::test]
    async fn resolve_installation_id_reuses_existing_uuid() {
        let tempdir = TempDir::new().expect("create temp dir");
        let existing = Uuid::new_v4().to_string().to_uppercase();
        std::fs::write(
            tempdir.path().join(INSTALLATION_ID_FILENAME),
            existing.clone(),
        )
        .expect("write installation id");

        let resolved = resolve_installation_id_in(tempdir.path())
            .await
            .expect("resolve installation id");

        assert_eq!(
            resolved,
            Uuid::parse_str(&existing)
                .expect("parse existing installation id")
                .to_string()
        );
    }

    #[tokio::test]
    async fn resolve_installation_id_rewrites_invalid_contents() {
        let tempdir = TempDir::new().expect("create temp dir");
        std::fs::write(tempdir.path().join(INSTALLATION_ID_FILENAME), "invalid")
            .expect("write invalid installation id");

        let resolved = resolve_installation_id_in(tempdir.path())
            .await
            .expect("resolve installation id");

        assert!(Uuid::parse_str(&resolved).is_ok());
        assert_eq!(
            std::fs::read_to_string(tempdir.path().join(INSTALLATION_ID_FILENAME))
                .expect("read installation id"),
            resolved
        );
    }
}
