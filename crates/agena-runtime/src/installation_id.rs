use std::io;
use std::path::Path;

use tokio::fs;
use uuid::Uuid;

const INSTALLATION_ID_FILENAME: &str = "installation_id";

/// Resolve the stable UUID assigned to this local Agena installation.
pub async fn resolve_installation_id() -> io::Result<String> {
    resolve_installation_id_in(&crate::agena_home_dir()).await
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
}
