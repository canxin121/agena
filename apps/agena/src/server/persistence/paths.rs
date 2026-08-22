use std::path::{Path, PathBuf};

// Agena server state is stored in a single SQLite database.
pub(crate) const SERVER_DB_FILE: &str = "agena.db";

fn dedupe_paths(candidates: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut out = Vec::<PathBuf>::new();
    for path in candidates {
        if out.iter().any(|existing| existing == &path) {
            continue;
        }
        out.push(path);
    }
    out
}

const PATH_PRIORITY_MISSING: u8 = 0;
const PATH_PRIORITY_EMPTY_DIR: u8 = 1;
const PATH_PRIORITY_EMPTY_FILE: u8 = 2;
const PATH_PRIORITY_NON_EMPTY_DIR: u8 = 3;
const PATH_PRIORITY_NON_EMPTY_FILE: u8 = 4;

fn existing_path_priority(path: &Path) -> Result<u8, String> {
    let meta = match std::fs::metadata(path) {
        Ok(meta) => meta,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(PATH_PRIORITY_MISSING);
        }
        Err(error) => {
            return Err(agena_failure::diagnostic::format_error_chain_with_context(
                format!(
                    "failed to inspect server-state path candidate {}",
                    path.display()
                ),
                &error,
            ));
        }
    };

    if meta.is_file() {
        return Ok(if meta.len() > 0 {
            PATH_PRIORITY_NON_EMPTY_FILE
        } else {
            PATH_PRIORITY_EMPTY_FILE
        });
    }

    if meta.is_dir() {
        let mut entries = std::fs::read_dir(path).map_err(|error| {
            agena_failure::diagnostic::format_error_chain_with_context(
                format!(
                    "failed to read server-state directory candidate {}",
                    path.display()
                ),
                &error,
            )
        })?;
        return match entries.next() {
            Some(Ok(_)) => Ok(PATH_PRIORITY_NON_EMPTY_DIR),
            None => Ok(PATH_PRIORITY_EMPTY_DIR),
            Some(Err(error)) => Err(agena_failure::diagnostic::format_error_chain_with_context(
                format!(
                    "failed to inspect an entry in server-state directory candidate {}",
                    path.display()
                ),
                &error,
            )),
        };
    }

    Ok(PATH_PRIORITY_EMPTY_FILE)
}

fn select_existing_path(candidates: Vec<PathBuf>) -> Result<PathBuf, String> {
    let mut best_priority = PATH_PRIORITY_MISSING;
    let mut best: Option<PathBuf> = None;

    for path in &candidates {
        let priority = existing_path_priority(path)?;
        if priority <= best_priority {
            continue;
        }
        best_priority = priority;
        best = Some(path.clone());
        if priority == PATH_PRIORITY_NON_EMPTY_FILE {
            break;
        }
    }

    Ok(best.unwrap_or_else(|| candidates.into_iter().next().unwrap_or_default()))
}

pub(crate) fn server_data_dir_candidates() -> Vec<PathBuf> {
    if let Ok(dir) = std::env::var("AGENA_SERVER_DATA_DIR")
        && !dir.trim().is_empty()
    {
        return vec![PathBuf::from(dir)];
    }

    let mut candidates = Vec::<PathBuf>::new();
    if let Some(home) = crate::server::path_utils::home_dir_path() {
        candidates.push(home.join(".config").join("agena"));
    }

    candidates.push(crate::server::path_utils::config_home_dir().join("agena"));

    if let Ok(dir) = std::env::var("APPDATA") {
        let trimmed = dir.trim();
        if !trimmed.is_empty() {
            candidates.push(PathBuf::from(trimmed).join("agena"));
        }
    }

    dedupe_paths(candidates)
}

pub(crate) fn server_state_db_path_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::<PathBuf>::new();
    for root in server_data_dir_candidates() {
        // Prefer the current DB name.
        candidates.push(root.join(SERVER_DB_FILE));
    }
    dedupe_paths(candidates)
}

pub(crate) fn server_state_db_path() -> Result<PathBuf, String> {
    select_existing_path(server_state_db_path_candidates())
}
