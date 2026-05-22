use std::path::{Path, PathBuf};

pub(crate) const SETTINGS_FILE: &str = "studio-settings.json";
pub(crate) const LEGACY_SETTINGS_FILE: &str = "settings.json";
pub(crate) const TERMINAL_UI_STATE_FILE: &str = "terminal-ui-state.json";
pub(crate) const LEGACY_TERMINAL_UI_STATE_FILE: &str = "terminal.state.json";
pub(crate) const TERMINAL_SESSION_REGISTRY_FILE: &str = "session-registry.json";
pub(crate) const LEGACY_TERMINAL_SESSION_REGISTRY_FILE: &str = "sessions.json";

// Agena Studio state is stored in a single SQLite database.
pub(crate) const STUDIO_DB_FILE: &str = "agena-studio.db";

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

fn existing_path_priority(path: &Path) -> u8 {
    let Ok(meta) = std::fs::metadata(path) else {
        return PATH_PRIORITY_MISSING;
    };

    if meta.is_file() {
        return if meta.len() > 0 {
            PATH_PRIORITY_NON_EMPTY_FILE
        } else {
            PATH_PRIORITY_EMPTY_FILE
        };
    }

    if meta.is_dir() {
        return match std::fs::read_dir(path) {
            Ok(mut entries) => {
                if entries.next().is_some() {
                    PATH_PRIORITY_NON_EMPTY_DIR
                } else {
                    PATH_PRIORITY_EMPTY_DIR
                }
            }
            Err(_) => PATH_PRIORITY_EMPTY_DIR,
        };
    }

    PATH_PRIORITY_EMPTY_FILE
}

fn select_existing_path(candidates: Vec<PathBuf>) -> PathBuf {
    let mut best_priority = PATH_PRIORITY_MISSING;
    let mut best: Option<PathBuf> = None;

    for path in &candidates {
        let priority = existing_path_priority(path);
        if priority <= best_priority {
            continue;
        }
        best_priority = priority;
        best = Some(path.clone());
        if priority == PATH_PRIORITY_NON_EMPTY_FILE {
            break;
        }
    }

    best.unwrap_or_else(|| candidates.into_iter().next().unwrap_or_default())
}

pub(crate) fn studio_data_dir_candidates() -> Vec<PathBuf> {
    if let Ok(dir) = std::env::var("AGENA_STUDIO_DATA_DIR")
        && !dir.trim().is_empty()
    {
        return vec![PathBuf::from(dir)];
    }

    let mut candidates = Vec::<PathBuf>::new();
    if let Some(home) = crate::path_utils::home_dir_path() {
        candidates.push(home.join(".config").join("agena-studio"));
    }

    candidates.push(crate::path_utils::config_home_dir().join("agena-studio"));

    if let Ok(dir) = std::env::var("APPDATA") {
        let trimmed = dir.trim();
        if !trimmed.is_empty() {
            candidates.push(PathBuf::from(trimmed).join("agena-studio"));
        }
    }

    dedupe_paths(candidates)
}

pub(crate) fn studio_settings_path_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::<PathBuf>::new();
    for root in studio_data_dir_candidates() {
        candidates.push(root.join(SETTINGS_FILE));
        candidates.push(root.join(LEGACY_SETTINGS_FILE));
    }
    dedupe_paths(candidates)
}

pub(crate) fn studio_settings_path() -> PathBuf {
    select_existing_path(studio_settings_path_candidates())
}

pub(crate) fn studio_db_path_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::<PathBuf>::new();
    for root in studio_data_dir_candidates() {
        // Prefer the current DB name.
        candidates.push(root.join(STUDIO_DB_FILE));
    }
    dedupe_paths(candidates)
}

pub(crate) fn studio_db_path() -> PathBuf {
    select_existing_path(studio_db_path_candidates())
}

pub(crate) fn terminal_ui_state_path_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::<PathBuf>::new();
    for root in studio_data_dir_candidates() {
        let ui_dir = root.join("ui");
        candidates.push(ui_dir.join(TERMINAL_UI_STATE_FILE));
        candidates.push(ui_dir.join(LEGACY_TERMINAL_UI_STATE_FILE));
    }
    dedupe_paths(candidates)
}

pub(crate) fn terminal_ui_state_path() -> PathBuf {
    select_existing_path(terminal_ui_state_path_candidates())
}

pub(crate) fn terminal_session_registry_path_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::<PathBuf>::new();
    for root in studio_data_dir_candidates() {
        let terminal_dir = root.join("terminal");
        candidates.push(terminal_dir.join(TERMINAL_SESSION_REGISTRY_FILE));
        candidates.push(terminal_dir.join(LEGACY_TERMINAL_SESSION_REGISTRY_FILE));
    }
    dedupe_paths(candidates)
}

pub(crate) fn terminal_session_registry_path() -> PathBuf {
    select_existing_path(terminal_session_registry_path_candidates())
}
