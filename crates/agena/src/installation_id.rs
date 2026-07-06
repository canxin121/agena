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
